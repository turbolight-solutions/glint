//! The build pipeline: extract, apply overrides, parse and check twice, index, emit.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use glint_corpus::{
    Bundle, Corpus, CorpusInfo, Exports, ModuleRecord, ModuleSource, OidAssignment, ParseStatus,
    SourceKind, write_bundle,
};
use glint_diag::Severity;
use glint_sema::{
    DraftSource, Evaluator, ExportsModel, ModuleModel, Options, SourceId, analyze, syntax_era,
};
use glint_syntax::{ParseOptions, ast, parse_source_with};
use rayon::prelude::*;
use sha2::{Digest, Sha256};

use crate::Settings;
use crate::collect::{Candidate, extract_rfc};
use crate::index::{RfcMeta, read_index};
use crate::itu::{fetch_needed, read_manifest as read_itu_manifest};
use crate::overrides::{Override, check_errata, check_extraction_tier, read_overrides};
use crate::report::{FindingLine, ModuleOutcome, ReportData, render_report};

const NOTICES: &[&str] = &[
    "Modules extracted from RFCs are code components under the IETF Trust's Revised BSD License (BCP 78, BCP 79).",
    "Modules from ITU-T Recommendations are the property of ITU-T and are included with credit to ITU-T.",
];

pub fn run(settings: &Settings) -> Result<String, String> {
    let numbers = rfc_numbers(&settings.rfc_dir, &settings.only);
    let meta = read_index(&settings.rfc_dir.join("rfc-index.xml")).unwrap_or_default();
    let overrides = match &settings.overrides {
        Some(dir) => read_overrides(dir)?,
        None => Vec::new(),
    };

    let extracted: Vec<_> = numbers
        .par_iter()
        .filter_map(|&rfc| extract_rfc(&settings.rfc_dir, rfc).map(|e| (rfc, e)))
        .collect();

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut skipped_smi: Vec<(u32, String)> = Vec::new();
    let mut extraction_notes: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut rfcs_with_modules: HashSet<u32> = HashSet::new();
    for (rfc, extraction) in extracted {
        if !extraction.notes.is_empty() {
            extraction_notes.insert(rfc, extraction.notes.clone());
        }
        for candidate in extraction.candidates {
            if candidate.smi {
                skipped_smi.push((rfc, candidate.name));
            } else {
                rfcs_with_modules.insert(rfc);
                candidates.push(candidate);
            }
        }
    }

    let mut kinds: HashMap<(u32, String), SourceKind> = HashMap::new();
    let mut applied: Vec<Override> = Vec::new();
    if let Some(errata) = &settings.errata {
        check_errata(&overrides, errata)?;
    }
    for item in overrides {
        check_extraction_tier(
            &item,
            candidates
                .iter()
                .find(|c| c.rfc == item.rfc && c.name == item.module)
                .map(|c| c.text.as_str()),
        )?;
        let key = (item.rfc, item.module.clone());
        let section = candidates
            .iter()
            .find(|c| c.rfc == item.rfc && c.name == item.module)
            .and_then(|c| c.section.clone());
        candidates.retain(|c| !(c.rfc == item.rfc && c.name == item.module));
        candidates.push(Candidate {
            rfc: item.rfc,
            name: item.module.clone(),
            section,
            text: item.text.trim().to_owned(),
            smi: false,
        });
        kinds.insert(
            key,
            SourceKind::Overridden {
                tier: item.tier,
                errata_ids: item.errata_ids.clone(),
            },
        );
        applied.push(item);
    }

    let mut seen: HashSet<(u32, String)> = HashSet::new();
    candidates.retain(|c| seen.insert((c.rfc, c.name.clone())));
    candidates.sort_by(|a, b| (a.rfc, &a.name).cmp(&(b.rfc, &b.name)));

    let mut records: Vec<ModuleRecord> = candidates
        .par_iter()
        .filter_map(|c| {
            let kind = kinds
                .get(&(c.rfc, c.name.clone()))
                .cloned()
                .unwrap_or(SourceKind::Extracted);
            let input = RecordInput {
                name: &c.name,
                text: &c.text,
                rfc: Some(c.rfc),
                section: c.section.clone(),
            };
            make_record(&input, kind, meta.get(&c.rfc))
        })
        .collect();
    records.sort_by(|a, b| (a.source.rfc, &a.name).cmp(&(b.source.rfc, &b.name)));
    let unreadable = candidates.len().saturating_sub(records.len());
    let mut itu_problems: Vec<String> = Vec::new();
    if let Some(manifest_path) = &settings.itu_manifest {
        let manifest = read_itu_manifest(manifest_path)?;
        for _ in 0..4 {
            let needed = unresolved_imports(&records);
            let already: HashSet<String> = records.iter().map(|r| r.name.clone()).collect();
            let (fetched, problems) =
                fetch_needed(&manifest, &settings.itu_cache, &needed, &already);
            itu_problems.extend(problems);
            if fetched.is_empty() {
                break;
            }
            for item in fetched {
                let input = RecordInput {
                    name: &item.name,
                    text: item.text.trim(),
                    rfc: None,
                    section: None,
                };
                if let Some(record) = make_record(&input, item.kind, None) {
                    records.push(record);
                }
            }
        }
        itu_problems.sort();
        itu_problems.dedup();
    }
    records.sort_by(|a, b| (a.source.rfc, &a.name).cmp(&(b.source.rfc, &b.name)));

    let corpus = Corpus::from_bundle(Bundle {
        info: CorpusInfo {
            version: settings.date.clone(),
            notices: NOTICES.iter().map(|s| (*s).to_owned()).collect(),
        },
        modules: records.clone(),
        oid_assignments: Vec::new(),
    });

    let outcomes: Vec<(ModuleOutcome, Vec<OidAssignment>)> = records
        .par_iter()
        .map(|record| check_record(record, &corpus))
        .collect();

    let mut oid_assignments: Vec<OidAssignment> = Vec::new();
    let mut outcome_list = Vec::new();
    for (record, (outcome, assignments)) in records.iter_mut().zip(outcomes) {
        record.parse_status = outcome.status;
        oid_assignments.extend(assignments);
        outcome_list.push(outcome);
    }
    oid_assignments.sort_by(|a, b| (&a.oid, &a.module, &a.name).cmp(&(&b.oid, &b.module, &b.name)));
    oid_assignments.dedup();

    let data = ReportData {
        date: settings.date.clone(),
        rfcs_scanned: numbers.len(),
        rfcs_with_modules: rfcs_with_modules.len(),
        records: &records,
        outcomes: &outcome_list,
        skipped_smi: &skipped_smi,
        unreadable,
        extraction_notes: &extraction_notes,
        itu_problems: &itu_problems,
        overrides: &applied,
        oid_assignments: oid_assignments.len(),
    };
    let report = render_report(&data);
    if let Some(path) = &settings.dump_findings {
        let mut dump = String::new();
        for (record, outcome) in data.records.iter().zip(data.outcomes) {
            for finding in &outcome.findings {
                let _ = writeln!(
                    dump,
                    "{}\t{}\t{}\t{}\t{}",
                    record.source.label(),
                    record.name,
                    finding.code,
                    finding.line,
                    finding.message
                );
            }
        }
        std::fs::write(path, dump).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let rfcs_with_modules_count = data.rfcs_with_modules;

    let bundle = Bundle {
        info: CorpusInfo {
            version: settings.date.clone(),
            notices: NOTICES.iter().map(|s| (*s).to_owned()).collect(),
        },
        modules: records,
        oid_assignments,
    };
    std::fs::create_dir_all(&settings.out)
        .map_err(|e| format!("{}: {e}", settings.out.display()))?;
    let bundle_path = settings
        .out
        .join(format!("corpus-{}.bundle", settings.date));
    let mut file = std::fs::File::create(&bundle_path)
        .map_err(|e| format!("{}: {e}", bundle_path.display()))?;
    write_bundle(&bundle, &mut file).map_err(|e| e.to_string())?;
    std::fs::write(settings.out.join("report.md"), &report)
        .map_err(|e| format!("report.md: {e}"))?;

    let mut summary = String::new();
    let _ = write!(
        summary,
        "{}: {} modules from {} RFCs, {} OID assignments",
        bundle_path.display(),
        bundle.modules.len(),
        rfcs_with_modules_count,
        bundle.oid_assignments.len()
    );
    Ok(summary)
}

fn rfc_numbers(dir: &Path, only: &[u32]) -> Vec<u32> {
    let mut numbers: Vec<u32> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let stem = name.strip_prefix("rfc")?;
            let digits = stem
                .strip_suffix(".txt")
                .or_else(|| stem.strip_suffix(".xml"))?;
            digits.parse::<u32>().ok()
        })
        .filter(|n| only.is_empty() || only.contains(n))
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

struct RecordInput<'a> {
    name: &'a str,
    text: &'a str,
    rfc: Option<u32>,
    section: Option<String>,
}

fn make_record(
    input: &RecordInput<'_>,
    kind: SourceKind,
    meta: Option<&RfcMeta>,
) -> Option<ModuleRecord> {
    let parse = parse_source_with(input.text, ParseOptions::corpus());
    let file = ast::SourceFile::cast(parse.syntax())?;
    let module = file
        .modules()
        .find(|m| m.name().is_some_and(|n| n.text() == input.name))
        .or_else(|| file.modules().next())?;
    let model = ModuleModel::build(&module)?;
    let oid = model.oid_group.as_ref().and_then(Evaluator::literal_oid);
    let exports = match &model.exports {
        ExportsModel::Absent | ExportsModel::All => Exports::All,
        ExportsModel::List(list) => Exports::List(list.iter().map(|(n, _)| n.clone()).collect()),
    };
    let mut hasher = Sha256::new();
    hasher.update(input.text.as_bytes());
    let sha256: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(ModuleRecord {
        name: input.name.to_owned(),
        oid,
        source: ModuleSource {
            rfc: input.rfc,
            section: input.section.clone(),
            kind,
        },
        rfc_status: meta.and_then(|m| m.status.clone()),
        obsoleted_by: meta.map(|m| m.obsoleted_by.clone()).unwrap_or_default(),
        syntax_era: syntax_era(&module),
        text: input.text.to_owned(),
        exports,
        parse_status: ParseStatus::Clean,
        sha256,
    })
}

/// Names of modules that records import but nothing in `records` defines.
fn unresolved_imports(records: &[ModuleRecord]) -> HashSet<String> {
    let defined: HashSet<&str> = records.iter().map(|r| r.name.as_str()).collect();
    let mut missing = HashSet::new();
    for record in records {
        let parse = parse_source_with(&record.text, ParseOptions::corpus());
        let Some(file) = ast::SourceFile::cast(parse.syntax()) else {
            continue;
        };
        for module in file.modules() {
            if let Some(model) = ModuleModel::build(&module) {
                for clause in &model.imports {
                    if !defined.contains(clause.module_name.as_str()) {
                        missing.insert(clause.module_name.clone());
                    }
                }
            }
        }
    }
    missing
}

fn line_of(text: &str, offset: usize) -> u32 {
    let newlines = text.get(..offset).map_or(0, |p| p.matches('\n').count());
    u32::try_from(newlines.saturating_add(1)).unwrap_or(u32::MAX)
}

fn check_record(record: &ModuleRecord, corpus: &Corpus) -> (ModuleOutcome, Vec<OidAssignment>) {
    let options = Options {
        oid_collisions: false,
        policy_checks: false,
        resolution_notes: false,
        home_rfc: record.source.rfc,
        ..Options::default()
    };
    let parse = parse_source_with(&record.text, ParseOptions::corpus());
    let analysis = analyze(
        vec![DraftSource {
            text: record.text.clone(),
            parse,
        }],
        corpus,
        &options,
    );
    let mut syntax = Vec::new();
    if let Some(unit) = analysis.graph.unit(SourceId(0)) {
        for error in &unit.syntax_errors {
            syntax.push(FindingLine {
                code: error.code.to_owned(),
                severity: match error.severity {
                    glint_syntax::Severity::Error => Severity::Error,
                    glint_syntax::Severity::Warning => Severity::Warning,
                    glint_syntax::Severity::Info => Severity::Info,
                },
                line: line_of(&unit.text, usize::from(error.range.start())),
                message: error.message.clone(),
            });
        }
    }
    let mut findings = Vec::new();
    for finding in &analysis.findings {
        if finding.primary.source != SourceId(0) {
            continue;
        }
        findings.push(FindingLine {
            code: finding.code.to_owned(),
            severity: finding.severity,
            line: line_of(&record.text, usize::from(finding.primary.range.start())),
            message: finding.message.clone(),
        });
    }
    let status = if syntax.iter().any(|f| f.severity == Severity::Error) {
        ParseStatus::Unparseable
    } else if findings.iter().any(|f| f.severity == Severity::Error) {
        ParseStatus::Errors
    } else if findings
        .iter()
        .chain(&syntax)
        .any(|f| f.severity == Severity::Warning)
    {
        ParseStatus::Warnings
    } else {
        ParseStatus::Clean
    };
    let evaluator = Evaluator::new(&analysis.graph);
    let assignments = analysis
        .drafts
        .first()
        .map(|&module| {
            evaluator
                .oid_assignments(module)
                .into_iter()
                .map(|(name, oid)| OidAssignment {
                    oid,
                    name,
                    module: record.name.clone(),
                    rfc: record.source.rfc,
                })
                .collect()
        })
        .unwrap_or_default();
    syntax.extend(findings);
    (
        ModuleOutcome {
            status,
            findings: syntax,
        },
        assignments,
    )
}
