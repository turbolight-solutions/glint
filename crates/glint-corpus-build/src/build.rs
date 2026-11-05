//! The build pipeline: extract, apply overrides, parse and check twice, index, emit.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use glint_corpus::{
    AllowedOid, BODY_RFC, CORPUS_FILE, Corpus, CorpusData, CorpusInfo, DOCUMENTS_FILE,
    DocumentRecord, Documents, Exports, ModuleRecord, ModuleSource, Oid, OidAssignment, OidLabels,
    ParseStatus, Pointer, SourceKind, build_dir, latest_path,
};
use glint_diag::Severity;
use glint_sema::{
    AssignedId, DraftSource, Evaluator, ExportsModel, ModuleModel, Options, SourceId, analyze,
    syntax_era,
};
use glint_syntax::{ParseOptions, ast, parse_source_with};
use rayon::prelude::*;
use sha2::{Digest, Sha256};

use crate::Settings;
use crate::collect::{Candidate, extract_rfc};
use crate::index::{RfcMeta, read_index};
use crate::overrides::{
    Override, check_errata, check_extraction_tier, read_allowlist, read_overrides,
};
use crate::report::{AuditLine, FindingLine, ModuleOutcome, ReportData, render_report};
use crate::sources::{Defined, Fetched, Manifest, Need, fetch, read_manifest, select};

const RFC_NOTICE: &str = "Modules extracted from RFCs are code components under the IETF Trust's Revised BSD License (BCP 78, BCP 79).";

pub fn run(settings: &Settings) -> Result<String, String> {
    let numbers = rfc_numbers(&settings.rfc_dir, &settings.only);
    let meta = read_index(&settings.rfc_dir.join("rfc-index.xml")).unwrap_or_default();
    let overrides = match &settings.overrides {
        Some(dir) => read_overrides(dir)?,
        None => Vec::new(),
    };
    let allowlist = match &settings.overrides {
        Some(dir) => read_allowlist(dir)?,
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
    let mut rfc_texts: BTreeMap<u32, (String, &'static str)> = BTreeMap::new();
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
        if rfcs_with_modules.contains(&rfc) {
            rfc_texts.insert(rfc, (extraction.text, extraction.format));
        }
    }

    let manifest = match &settings.sources {
        Some(path) => read_manifest(path)?,
        None => Manifest::default(),
    };
    let mut source_problems: Vec<String> = Vec::new();
    let (always, problems) = fetch(&manifest.always(), &settings.cache);
    source_problems.extend(problems);
    for item in always {
        candidates.push(standalone_candidate(item, &mut source_problems));
    }

    if let Some(errata) = &settings.errata {
        check_errata(&overrides, errata)?;
    }
    let mut pending = overrides;
    let mut applied: Vec<Override> = Vec::new();
    apply_overrides(&mut candidates, &mut pending, &mut applied)?;

    candidates
        .sort_by(|a, b| (source_key(&a.source), &a.name).cmp(&(source_key(&b.source), &b.name)));

    // A module is identified by its document, name and OID. An RFC may define two modules with one name and
    // different OIDs (RFC 4998 does), and may show the same module twice, as a header stub and in full.
    let made: Vec<Option<Made>> = candidates
        .par_iter()
        .map(|c| make_record(c, meta_of(&meta, c)))
        .collect();
    let mut unreadable = made.iter().filter(|m| m.is_none()).count();
    let (mut records, collapsed) = one_per_identity(made.into_iter().flatten().collect());
    source_problems.extend(collapsed);
    let mut fetched_oids: HashSet<Oid> = HashSet::new();
    for _ in 0..4 {
        let mut defined = defined_by(&records);
        defined.oids.extend(fetched_oids.iter().cloned());
        let needs = unresolved_imports(&records, &defined);
        let wanted = select(&manifest.module, &needs, &defined);
        let (fetched, problems) = fetch(&wanted, &settings.cache);
        source_problems.extend(problems);
        if fetched.is_empty() {
            break;
        }
        let mut batch: Vec<Candidate> = Vec::new();
        for item in fetched {
            fetched_oids.insert(item.oid.clone());
            batch.push(standalone_candidate(item, &mut source_problems));
        }
        apply_overrides(&mut batch, &mut pending, &mut applied)?;
        for candidate in &batch {
            match make_record(candidate, None) {
                Some(made) => records.push(made.record),
                None => {
                    unreadable = unreadable.saturating_add(1);
                    source_problems.push(format!(
                        "could not read {} in {}",
                        candidate.name, candidate.source.document
                    ));
                }
            }
        }
    }
    source_problems.sort();
    source_problems.dedup();
    if settings.only.is_empty()
        && let Some(item) = pending.first()
    {
        return Err(format!(
            "override of {} in {} matches no module of the corpus",
            item.module,
            glint_corpus::document_title(&item.document)
        ));
    }
    records.sort_by(|a, b| identity(a).cmp(&identity(b)));

    let info = CorpusInfo {
        build: String::new(),
        notices: notices(&manifest, &records),
    };
    let corpus = Corpus::from_data(CorpusData {
        info: info.clone(),
        modules: records.clone(),
        ..CorpusData::default()
    });

    let outcomes: Vec<Checked> = records
        .par_iter()
        .map(|record| check_record(record, &corpus, &allowlist))
        .collect();

    let mut oid_assignments: Vec<OidAssignment> = Vec::new();
    let mut label_triples: Vec<(Oid, String, String)> = Vec::new();
    let mut outcome_list = Vec::new();
    for (record, checked) in records.iter_mut().zip(outcomes) {
        record.parse_status = checked.outcome.status;
        oid_assignments.extend(checked.assignments);
        label_triples.extend(
            checked
                .labels
                .into_iter()
                .map(|(oid, label)| (oid, label, record.source.document.clone())),
        );
        outcome_list.push(checked.outcome);
    }
    oid_assignments.sort_by(|a, b| (&a.oid, &a.module, &a.name).cmp(&(&b.oid, &b.module, &b.name)));
    oid_assignments.dedup();
    let labels = OidLabels::collect(label_triples, allowlist);

    // Every module against the label table, leaving out its own document's evidence: what a user checking the
    // document would be told.
    let audited = Corpus::from_data(CorpusData {
        info: info.clone(),
        modules: records.clone(),
        oid_assignments: oid_assignments.clone(),
        oid_nodes: labels.nodes(),
        oid_allowlist: labels.allowlist().to_vec(),
        ..CorpusData::default()
    });
    let audit: Vec<AuditLine> = records
        .par_iter()
        .flat_map(|record| audit_record(record, &audited))
        .collect();

    let documents = document_index(&records, &meta, &rfc_texts);
    let texts: Vec<(String, String, String)> = rfc_texts
        .into_iter()
        .map(|(rfc, (text, format))| (format!("rfc{rfc}"), format.to_owned(), text))
        .collect();
    let mut corpus_data = CorpusData {
        info,
        documents,
        modules: records,
        oid_assignments,
        oid_nodes: labels.nodes(),
        oid_allowlist: labels.allowlist().to_vec(),
        ..CorpusData::default()
    };
    // The build id names the content: the same data built twice on one day gets the same id.
    let mut hasher = Sha256::new();
    hasher.update(corpus_data.to_json());
    hasher.update(Documents::write("", &texts));
    let hash: String = hex(&hasher.finalize()).chars().take(8).collect();
    let build = format!("{}.{hash}", settings.date);
    corpus_data.info.build = build.clone();

    let data = ReportData {
        build: build.clone(),
        rfcs_scanned: numbers.len(),
        rfcs_with_modules: rfcs_with_modules.len(),
        records: &corpus_data.modules,
        outcomes: &outcome_list,
        skipped_smi: &skipped_smi,
        unreadable,
        extraction_notes: &extraction_notes,
        source_problems: &source_problems,
        overrides: &applied,
        oid_assignments: corpus_data.oid_assignments.len(),
        oid_nodes: corpus_data.oid_nodes.len(),
        allowlist: &corpus_data.oid_allowlist,
        label_audit: &audit,
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

    let dir = settings.out.join(build_dir(&build));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    write_with_digest(&dir, CORPUS_FILE, &corpus_data.to_json())?;
    write_with_digest(&dir, DOCUMENTS_FILE, &Documents::write(&build, &texts))?;
    let pointer = Pointer {
        schema: corpus_data.schema,
        build: build.clone(),
    };
    let latest = settings.out.join(latest_path());
    std::fs::write(&latest, pointer.to_json()).map_err(|e| format!("{}: {e}", latest.display()))?;
    std::fs::write(settings.out.join("report.md"), &report)
        .map_err(|e| format!("report.md: {e}"))?;

    let rfc_documents = corpus_data
        .documents
        .iter()
        .filter(|d| d.body == BODY_RFC)
        .count();
    let mut summary = String::new();
    let _ = write!(
        summary,
        "{}: build {build}, {} modules from {} RFCs and {} other documents, {} OID assignments, {} document texts",
        dir.display(),
        corpus_data.modules.len(),
        rfc_documents,
        corpus_data.documents.len().saturating_sub(rfc_documents),
        corpus_data.oid_assignments.len(),
        texts.len()
    );
    Ok(summary)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Writes `name` in `dir` and `name.sha256` beside it, in the format of `sha256sum`.
fn write_with_digest(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let digest = format!("{}  {name}\n", hex(&Sha256::digest(bytes)));
    let digest_path = dir.join(format!("{name}.sha256"));
    std::fs::write(&digest_path, digest).map_err(|e| format!("{}: {e}", digest_path.display()))
}

/// One entry per document that publishes a module of the corpus, in the order of the modules.
fn document_index(
    records: &[ModuleRecord],
    meta: &HashMap<u32, RfcMeta>,
    texts: &BTreeMap<u32, (String, &'static str)>,
) -> Vec<DocumentRecord> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut documents = Vec::new();
    for record in records {
        if !seen.insert(record.source.document.as_str()) {
            continue;
        }
        let rfc = record.source.rfc_number();
        let rfc_meta = rfc.and_then(|n| meta.get(&n));
        documents.push(DocumentRecord {
            id: record.source.document.clone(),
            body: record.source.body.clone(),
            status: rfc_meta.and_then(|m| m.status.clone()),
            obsoleted_by: rfc_meta.map(|m| m.obsoleted_by.clone()).unwrap_or_default(),
            date: rfc_meta.and_then(|m| m.date.clone()),
            text_format: rfc
                .and_then(|n| texts.get(&n))
                .map(|(_, format)| (*format).to_owned()),
        });
    }
    documents
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

/// A record and whether its text parsed without syntax errors.
struct Made {
    record: ModuleRecord,
    syntax_clean: bool,
}

fn make_record(candidate: &Candidate, meta: Option<&RfcMeta>) -> Option<Made> {
    let (name, text) = (candidate.name.as_str(), candidate.text.as_str());
    let parse = parse_source_with(text, ParseOptions::corpus());
    let file = ast::SourceFile::cast(parse.syntax())?;
    let module = file
        .modules()
        .find(|m| m.name().is_some_and(|n| n.text() == name))
        .or_else(|| file.modules().next())?;
    let model = ModuleModel::build(&module)?;
    let oid = model.oid_group.as_ref().and_then(Evaluator::literal_oid);
    let exports = match &model.exports {
        ExportsModel::Absent | ExportsModel::All => Exports::All,
        ExportsModel::List(list) => Exports::List(list.iter().map(|(n, _)| n.clone()).collect()),
    };
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let sha256 = hex(&hasher.finalize());
    let syntax_clean = parse
        .errors()
        .iter()
        .all(|e| e.severity != glint_syntax::Severity::Error);
    let record = ModuleRecord {
        name: name.to_owned(),
        oid,
        source: candidate.source.clone(),
        rfc_status: meta.and_then(|m| m.status.clone()),
        obsoleted_by: meta.map(|m| m.obsoleted_by.clone()).unwrap_or_default(),
        syntax_era: syntax_era(&module),
        text: text.to_owned(),
        exports,
        parse_status: ParseStatus::Clean,
        sha256,
    };
    Some(Made {
        record,
        syntax_clean,
    })
}

/// Orders documents: by body, then by recency within the body, then by id.
fn source_key(source: &ModuleSource) -> (&str, Option<u32>, &str) {
    (&source.body, source.recency(), &source.document)
}

/// What identifies a module: its document, name and definitive identifier.
fn identity(record: &ModuleRecord) -> ((&str, Option<u32>, &str), &str, &Option<Oid>) {
    (source_key(&record.source), &record.name, &record.oid)
}

fn meta_of<'a>(meta: &'a HashMap<u32, RfcMeta>, candidate: &Candidate) -> Option<&'a RfcMeta> {
    candidate
        .source
        .rfc_number()
        .and_then(|number| meta.get(&number))
}

/// A downloaded module as a candidate. Reports when its definitive identifier is not the one the manifest lists.
fn standalone_candidate(item: Fetched, problems: &mut Vec<String>) -> Candidate {
    let found = module_oid(&item.text, &item.name);
    if found.as_ref() != Some(&item.oid) {
        problems.push(format!(
            "{}: the manifest says {} but the module's identifier is {}",
            item.name,
            item.oid,
            found.map_or_else(|| "absent".to_owned(), |oid| oid.to_string())
        ));
    }
    Candidate {
        name: item.name,
        text: item.text.trim().to_owned(),
        smi: false,
        source: item.source,
    }
}

/// The licence notices of the bodies that `records` come from.
fn notices(manifest: &Manifest, records: &[ModuleRecord]) -> Vec<String> {
    let mut notices = vec![RFC_NOTICE.to_owned()];
    for body in &manifest.body {
        if records.iter().any(|r| r.source.body == body.id) {
            notices.push(body.notice.clone());
        }
    }
    notices
}

/// Replaces the candidates that each pending override names, moving the override to `applied`. An override that
/// replaces nothing stays pending: its module may arrive with a later batch of downloads.
fn apply_overrides(
    candidates: &mut Vec<Candidate>,
    pending: &mut Vec<Override>,
    applied: &mut Vec<Override>,
) -> Result<(), String> {
    for item in std::mem::take(pending) {
        let replaced = replaced_by(candidates, &item)?;
        let extracted = replaced
            .iter()
            .filter_map(|&i| candidates.get(i))
            .min_by_key(|c| !parses_cleanly(&c.text));
        let Some(extracted) = extracted else {
            pending.push(item);
            continue;
        };
        check_extraction_tier(&item, Some(extracted.text.as_str()))?;
        let source = ModuleSource {
            kind: SourceKind::Overridden {
                tier: item.tier,
                errata_ids: item.errata_ids.clone(),
            },
            ..extracted.source.clone()
        };
        *candidates = std::mem::take(candidates)
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !replaced.contains(index))
            .map(|(_, candidate)| candidate)
            .collect();
        candidates.push(Candidate {
            name: item.module.clone(),
            text: item.text.trim().to_owned(),
            smi: false,
            source,
        });
        applied.push(item);
    }
    Ok(())
}

/// Keeps one record per identity, preferring the one that parses without syntax errors, then the first.
///
/// A module shown twice with the same text is nothing to report. One shown twice with different text (a header
/// stub beside the full module, or two modules that really are different) is named, so a curator can tell
/// which was kept.
fn one_per_identity(mut made: Vec<Made>) -> (Vec<ModuleRecord>, Vec<String>) {
    made.sort_by(|a, b| {
        identity(&a.record)
            .cmp(&identity(&b.record))
            .then(b.syntax_clean.cmp(&a.syntax_clean))
    });
    let mut kept: Vec<Made> = Vec::with_capacity(made.len());
    let mut notes = Vec::new();
    for later in made {
        match kept.last() {
            Some(first) if identity(&first.record) == identity(&later.record) => {
                if first.record.text != later.record.text {
                    notes.push(duplicate_note(&first.record, &later.record));
                }
            }
            _ => kept.push(later),
        }
    }
    (kept.into_iter().map(|m| m.record).collect(), notes)
}

fn duplicate_note(kept: &ModuleRecord, dropped: &ModuleRecord) -> String {
    let place = |record: &ModuleRecord| match &record.source.section {
        Some(section) => format!("{} {section}", record.source.document),
        None => record.source.document.clone(),
    };
    let oid = kept
        .oid
        .as_ref()
        .map_or_else(String::new, |oid| format!(" ({oid})"));
    format!(
        "{}{oid} is defined twice in {} with different text; kept {} ({} lines), dropped {} ({} lines)",
        kept.name,
        kept.source.document,
        place(kept),
        kept.text.lines().count(),
        place(dropped),
        dropped.text.lines().count()
    )
}

fn parses_cleanly(text: &str) -> bool {
    parse_source_with(text, ParseOptions::corpus())
        .errors()
        .iter()
        .all(|e| e.severity != glint_syntax::Severity::Error)
}

/// The definitive identifier of the module called `name` in `text`.
fn module_oid(text: &str, name: &str) -> Option<Oid> {
    let parse = parse_source_with(text, ParseOptions::corpus());
    let file = ast::SourceFile::cast(parse.syntax())?;
    let module = file
        .modules()
        .find(|m| m.name().is_some_and(|n| n.text() == name))
        .or_else(|| file.modules().next())?;
    let model = ModuleModel::build(&module)?;
    model.oid_group.as_ref().and_then(Evaluator::literal_oid)
}

/// Indexes of the candidates an override replaces: those of its document and module name, narrowed to its OID when
/// it has one. Without an OID the document must define the name with at most one OID.
fn replaced_by(candidates: &[Candidate], item: &Override) -> Result<Vec<usize>, String> {
    let same_name: Vec<(usize, Option<Oid>)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            c.source.document.eq_ignore_ascii_case(&item.document) && c.name == item.module
        })
        .map(|(i, c)| (i, module_oid(&c.text, &c.name)))
        .collect();
    if let Some(oid) = &item.oid {
        return Ok(same_name
            .into_iter()
            .filter(|(_, found)| found.as_ref() == Some(oid))
            .map(|(i, _)| i)
            .collect());
    }
    let mut oids: Vec<&Option<Oid>> = same_name.iter().map(|(_, oid)| oid).collect();
    oids.sort();
    oids.dedup();
    if oids.len() > 1 {
        return Err(format!(
            "override of {} in {} is ambiguous: the document defines modules of that name with {} different OIDs; add `oid` to the entry",
            item.module,
            glint_corpus::document_title(&item.document),
            oids.len()
        ));
    }
    Ok(same_name.into_iter().map(|(i, _)| i).collect())
}

fn defined_by(records: &[ModuleRecord]) -> Defined {
    Defined {
        names: records.iter().map(|r| r.name.clone()).collect(),
        oids: records.iter().filter_map(|r| r.oid.clone()).collect(),
    }
}

/// Imports of `records` that nothing in the corpus satisfies: by OID when the import has a literal one, otherwise
/// by name.
fn unresolved_imports(records: &[ModuleRecord], defined: &Defined) -> HashSet<Need> {
    let mut missing = HashSet::new();
    for record in records {
        let parse = parse_source_with(&record.text, ParseOptions::corpus());
        let Some(file) = ast::SourceFile::cast(parse.syntax()) else {
            continue;
        };
        for module in file.modules() {
            if let Some(model) = ModuleModel::build(&module) {
                for clause in &model.imports {
                    let oid = match &clause.assigned {
                        Some((AssignedId::Oid { group }, _)) => Evaluator::literal_oid(group),
                        _ => None,
                    };
                    let satisfied = match &oid {
                        Some(oid) => defined.oids.contains(oid),
                        None => defined.names.contains(&clause.module_name),
                    };
                    if !satisfied {
                        missing.insert(Need {
                            name: clause.module_name.clone(),
                            oid,
                        });
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

/// What checking one corpus module gives.
struct Checked {
    outcome: ModuleOutcome,
    assignments: Vec<OidAssignment>,
    /// The labels the module writes, with the OIDs of their arcs: its evidence for the label table.
    labels: Vec<(Oid, String)>,
}

fn check_record(record: &ModuleRecord, corpus: &Corpus, allowlist: &[AllowedOid]) -> Checked {
    let options = Options {
        oid_collisions: false,
        oid_labels: false,
        policy_checks: false,
        resolution_notes: false,
        home_document: Some(record.source.document.clone()),
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
                    document: record.source.document.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let labels = analysis
        .drafts
        .first()
        .map(|&module| {
            evaluator
                .oid_values(module)
                .into_iter()
                .filter(|value| {
                    // A wrong but deployed OID is no evidence, and its labels would be checked against nothing.
                    let oid = value
                        .oid
                        .as_ref()
                        .or_else(|| value.labels.last().map(|l| &l.oid));
                    !oid.is_some_and(|oid| allowlist.iter().any(|a| a.oid.is_prefix_of(oid)))
                })
                .flat_map(|value| value.labels)
                .map(|label| (label.oid, label.name))
                .collect()
        })
        .unwrap_or_default();
    syntax.extend(findings);
    Checked {
        outcome: ModuleOutcome {
            status,
            findings: syntax,
        },
        assignments,
        labels,
    }
}

/// The label table findings (W142, W143, W144) for one corpus module, its own document's evidence left out.
fn audit_record(record: &ModuleRecord, corpus: &Corpus) -> Vec<AuditLine> {
    let options = Options {
        oid_collisions: false,
        policy_checks: false,
        resolution_notes: false,
        home_document: Some(record.source.document.clone()),
        own_document: Some(record.source.document.clone()),
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
    analysis
        .findings
        .iter()
        .filter(|f| f.primary.source == SourceId(0))
        .filter(|f| matches!(f.code, "W142" | "W143" | "W144"))
        .map(|finding| AuditLine {
            document: record.source.document.clone(),
            module: record.name.clone(),
            finding: FindingLine {
                code: finding.code.to_owned(),
                severity: finding.severity,
                line: line_of(&record.text, usize::from(finding.primary.range.start())),
                message: finding.message.clone(),
            },
            notes: finding.notes.clone(),
        })
        .collect()
}
