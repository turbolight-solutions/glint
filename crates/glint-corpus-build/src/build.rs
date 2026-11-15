//! The build pipeline: extract, apply overrides, parse and check twice, index, emit.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use glint_corpus::{
    ARTIFACTS_FILE, AllowedOid, AppliedErratum, BODY_RFC, CORPUS_FILE, Corpus, CorpusData,
    CorpusInfo, DOCUMENTS_FILE, DocumentRecord, DocumentText, Documents, ErrataFailure,
    ErratumPatch, Exports, HunkLines, ModuleRecord, ModuleSource, Oid, OidAssignment, OidHint,
    OidLabels, ParseStatus, Pointer, SourceKind, SyntaxEra, build_dir, latest_path,
};
use glint_diag::Severity;
use glint_errata::LineRange;
use glint_sema::{
    AssignedId, DraftSource, Evaluator, ExportsModel, ModuleModel, Options, Origin, SourceId,
    analyze, syntax_era,
};
use glint_syntax::{ParseOptions, ast, parse_source_with};
use rayon::prelude::*;
use sha2::{Digest, Sha256};

use crate::Settings;
use crate::artifacts::{ScannedDocument, expected, identify_all};
use crate::collect::{Candidate, RfcExtraction, extract_draft, extract_rfc};
use crate::errata::{Errata, Series, read_errata};
use crate::iana::read_iana;
use crate::index::{RfcMeta, read_drafts, read_index};
use crate::overrides::{
    Override, check_extraction_tier, errata_documents, read_allowlist, read_overrides,
};
use crate::report::{
    AuditLine, FindingLine, HintConflict, ModuleOutcome, ReportData, render_report,
};
use crate::sources::{Defined, Fetched, Manifest, Need, fetch, read_manifest, select};

const RFC_NOTICE: &str = "Modules extracted from RFCs and Internet-Drafts are code components under the IETF Trust's Revised BSD License (BCP 78, BCP 79).";

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

    let errata = match &settings.errata_dir {
        Some(dir) => read_errata(dir, settings.errata.as_deref())?,
        None => Errata::default(),
    };
    let extracted: Vec<_> = numbers
        .par_iter()
        .filter_map(|&rfc| {
            extract_rfc(
                &settings.rfc_dir,
                rfc,
                errata.series.get(&rfc),
                errata.text_series.get(&rfc),
            )
            .map(|e| (rfc, e))
        })
        .collect();

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut skipped_smi: Vec<(u32, String)> = Vec::new();
    // Every module skipped as SMI, RFC or draft, by name and OID: imports of them are resolved against this.
    let mut smi_modules: Vec<(String, Option<Oid>)> = Vec::new();
    let mut extraction_notes: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut rfcs_with_modules: HashSet<u32> = HashSet::new();
    // The text of every document with a module, by document id, for documents.bin.
    let mut texts_by_id: BTreeMap<String, DocumentText> = BTreeMap::new();
    let mut errata_applied: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut errata_failed: BTreeMap<u32, glint_errata::SeriesFailure> = BTreeMap::new();
    // The errata of every RFC with a module, by document id, for its document record.
    let mut errata_by_id: BTreeMap<String, DocumentErrata> = BTreeMap::new();
    // The PEM blocks of every document that has one, for artifacts.json.
    let mut scanned: Vec<ScannedDocument> = Vec::new();
    for (rfc, mut extraction) in extracted {
        let pem = std::mem::take(&mut extraction.pem);
        if pem != glint_extract::artifacts::PemScan::default() {
            scanned.push(ScannedDocument {
                document: format!("rfc{rfc}"),
                scan: pem,
                obsoleted: meta.get(&rfc).is_some_and(|m| !m.obsoleted_by.is_empty()),
            });
        }
        if !extraction.errata_applied.is_empty() {
            errata_applied.insert(
                rfc,
                extraction
                    .errata_applied
                    .iter()
                    .map(|(id, _)| *id)
                    .collect(),
            );
        }
        if let Some(failure) = extraction.errata_failure.clone() {
            errata_failed.insert(rfc, failure);
        }
        if let Some(reason) = &extraction.kept_as_xml {
            extraction.notes.push(kept_as_xml_note(reason));
        }
        if !extraction.notes.is_empty() {
            extraction_notes.insert(rfc, extraction.notes.clone());
        }
        let candidates_of_rfc = std::mem::take(&mut extraction.candidates);
        for candidate in candidates_of_rfc {
            if candidate.smi {
                smi_modules.push((
                    candidate.name.clone(),
                    module_oid(&candidate.text, &candidate.name),
                ));
                skipped_smi.push((rfc, candidate.name));
            } else {
                rfcs_with_modules.insert(rfc);
                candidates.push(candidate);
            }
        }
        if rfcs_with_modules.contains(&rfc) {
            errata_by_id.insert(
                format!("rfc{rfc}"),
                document_errata(rfc, &extraction, &errata),
            );
            let id = format!("rfc{rfc}");
            texts_by_id.insert(id.clone(), document_text(id, extraction));
        }
    }

    let drafts = match &settings.drafts_dir {
        Some(dir) if settings.only.is_empty() => read_drafts(&dir.join("all_id2.txt"))?,
        _ => Vec::new(),
    };
    let draft_dates: HashMap<String, (String, Option<String>)> = drafts
        .iter()
        .map(|d| (d.id.clone(), (d.date.clone(), d.iesg_state.clone())))
        .collect();
    let extracted_drafts: Vec<_> = match &settings.drafts_dir {
        Some(dir) => drafts
            .par_iter()
            .filter_map(|draft| extract_draft(dir, draft).map(|e| (draft.id.clone(), e)))
            .collect(),
        None => Vec::new(),
    };
    let mut drafts_with_modules = 0_usize;
    // Why a draft with a module is kept as RFCXML for the web UI, by draft.
    let mut draft_notes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, mut extraction) in extracted_drafts {
        let pem = std::mem::take(&mut extraction.pem);
        if pem != glint_extract::artifacts::PemScan::default() {
            scanned.push(ScannedDocument {
                document: id.clone(),
                scan: pem,
                obsoleted: false,
            });
        }
        let (smi, modules): (Vec<Candidate>, Vec<Candidate>) =
            std::mem::take(&mut extraction.candidates)
                .into_iter()
                .partition(|c| c.smi);
        smi_modules.extend(
            smi.iter()
                .map(|c| (c.name.clone(), module_oid(&c.text, &c.name))),
        );
        if !modules.is_empty() {
            drafts_with_modules = drafts_with_modules.saturating_add(1);
            candidates.extend(modules);
            if let Some(reason) = &extraction.kept_as_xml {
                draft_notes.insert(id.clone(), vec![kept_as_xml_note(reason)]);
            }
            texts_by_id.insert(id.clone(), document_text(id, extraction));
        }
    }

    let manifest = match &settings.sources {
        Some(path) => read_manifest(path)?,
        None => Manifest::default(),
    };
    // A listed module that cannot be bundled fails the build: the data would otherwise be published without it.
    let mut standalone_problems: Vec<String> = Vec::new();
    let (always, problems) = fetch(&manifest.always(), &settings.cache);
    standalone_problems.extend(problems);
    for item in always {
        candidates.push(standalone_candidate(item));
    }

    let iana_module_ids = match &settings.iana {
        Some(path) => {
            let errata = match &settings.errata {
                Some(errata) => errata_documents(errata)?,
                None => HashMap::new(),
            };
            read_iana(path, &errata)?
        }
        None => Vec::new(),
    };
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
    let (mut records, mut source_problems) = one_per_identity(made.into_iter().flatten().collect());
    let mut fetched_oids: HashSet<Oid> = HashSet::new();
    // A fetched module can import another listed module, so fetch until a round adds nothing. Each round fetches
    // only entries whose OID the corpus lacks, so there are at most as many rounds as entries.
    for _ in 0..=manifest.module.len() {
        let mut defined = defined_by(&records);
        defined.oids.extend(fetched_oids.iter().cloned());
        let needs = unresolved_imports(&records, &defined);
        let wanted = select(&manifest.module, &needs, &defined);
        let (fetched, problems) = fetch(&wanted, &settings.cache);
        standalone_problems.extend(problems);
        if fetched.is_empty() {
            break;
        }
        let mut batch: Vec<Candidate> = Vec::new();
        for item in fetched {
            fetched_oids.extend(item.oid.clone());
            batch.push(standalone_candidate(item));
        }
        apply_overrides(&mut batch, &mut pending, &mut applied)?;
        for candidate in &batch {
            match make_record(candidate, None) {
                Some(made) => records.push(made.record),
                None => {
                    unreadable = unreadable.saturating_add(1);
                    standalone_problems.push(format!(
                        "could not read {} in {}",
                        candidate.name, candidate.source.document
                    ));
                }
            }
        }
    }
    // A module that imports a skipped SMI module, where nothing in the corpus satisfies the import, is written in
    // that module's macros: it is skipped too. RFC 1303's FourBSD-ISODE uses the MODULE-CONFORMANCE macro of its
    // RFC-1303, which imports RFC1155-SMI.
    let mut emptied: HashSet<String> = HashSet::new();
    for record in skip_smi_importers(&mut records, &mut smi_modules) {
        if let Some(rfc) = record.source.rfc_number() {
            skipped_smi.push((rfc, record.name.clone()));
        }
        emptied.insert(record.source.document.clone());
    }
    skipped_smi.sort_by_key(|(rfc, _)| *rfc);
    for record in &records {
        emptied.remove(&record.source.document);
    }
    for id in &emptied {
        texts_by_id.remove(id);
        if let Some(rfc) = glint_corpus::rfc_number(id) {
            rfcs_with_modules.remove(&rfc);
        } else {
            drafts_with_modules = drafts_with_modules.saturating_sub(1);
        }
    }
    if !standalone_problems.is_empty() {
        standalone_problems.sort();
        standalone_problems.dedup();
        return Err(format!(
            "listed modules could not be bundled:\n  {}",
            standalone_problems.join("\n  ")
        ));
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

    let imports: Vec<Vec<usize>> = outcomes.iter().map(|c| c.imports.clone()).collect();
    let unmaintained = unmaintained(&records, &imports);
    let mut oid_assignments: Vec<OidAssignment> = Vec::new();
    let mut label_triples: Vec<(Oid, String, String)> = Vec::new();
    let mut hint_rows: Vec<(usize, OidHint)> = Vec::new();
    let mut outcome_list = Vec::new();
    for (index, (record, checked)) in records.iter_mut().zip(outcomes).enumerate() {
        record.parse_status = checked.outcome.status;
        oid_assignments.extend(checked.assignments);
        hint_rows.extend(checked.hints.into_iter().map(|hint| (index, hint)));
        // Drafts may still change: they are no evidence of how an arc is labelled.
        if record.source.is_draft() {
            outcome_list.push(checked.outcome);
            continue;
        }
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
    let (oid_hints, hint_conflicts) = hint_table(&records, hint_rows);

    // Every module against the label table, leaving out its own document's evidence: what a user checking the
    // document would be told.
    let audited = Corpus::from_data(CorpusData {
        info: info.clone(),
        modules: records.clone(),
        oid_assignments: oid_assignments.clone(),
        oid_nodes: labels.nodes(),
        oid_allowlist: labels.allowlist().to_vec(),
        iana_module_ids: iana_module_ids.clone(),
        ..CorpusData::default()
    });
    let audit: Vec<AuditLine> = records
        .par_iter()
        .flat_map(|record| audit_record(record, &audited))
        .collect();

    let documents = document_index(&records, &meta, &draft_dates, &texts_by_id, &errata_by_id);
    let texts: Vec<DocumentText> = texts_by_id.into_values().collect();
    let mut corpus_data = CorpusData {
        info,
        documents,
        modules: records,
        oid_assignments,
        oid_nodes: labels.nodes(),
        oid_allowlist: labels.allowlist().to_vec(),
        iana_module_ids,
        oid_hints,
        ..CorpusData::default()
    };
    // Every example identified against the corpus as published.
    let mut artifacts = identify_all(&Corpus::from_data(corpus_data.clone()), &scanned);
    let openpgp_blocks = scanned.iter().map(|d| d.scan.openpgp.len()).sum();
    // The build id names the content: the same data built twice on one day gets the same id.
    let mut hasher = Sha256::new();
    hasher.update(corpus_data.to_json());
    hasher.update(Documents::write("", &texts));
    hasher.update(artifacts.to_json());
    let hash: String = hex(&hasher.finalize()).chars().take(8).collect();
    let build = format!("{}.{hash}", settings.date);
    corpus_data.info.build = build.clone();
    artifacts.build = build.clone();

    let data = ReportData {
        build: build.clone(),
        rfcs_scanned: numbers.len(),
        rfcs_with_modules: rfcs_with_modules.len(),
        drafts_scanned: drafts.len(),
        drafts_with_modules,
        records: &corpus_data.modules,
        outcomes: &outcome_list,
        unmaintained: &unmaintained,
        skipped_smi: &skipped_smi,
        unreadable,
        extraction_notes: &extraction_notes,
        draft_notes: &draft_notes,
        source_problems: &source_problems,
        overrides: &applied,
        errata_applied: &errata_applied,
        errata_failed: &errata_failed,
        oid_assignments: corpus_data.oid_assignments.len(),
        oid_nodes: corpus_data.oid_nodes.len(),
        allowlist: &corpus_data.oid_allowlist,
        label_audit: &audit,
        iana_module_ids: corpus_data.iana_module_ids.len(),
        oid_hints: corpus_data.oid_hints.len(),
        hint_conflicts: &hint_conflicts,
        artifacts: &artifacts,
        openpgp_blocks,
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
    write_with_digest(
        &dir,
        DOCUMENTS_FILE,
        &gzip(&Documents::write(&build, &texts))?,
    )?;
    write_with_digest(&dir, ARTIFACTS_FILE, &artifacts.to_json())?;
    std::fs::write(
        settings.out.join("artifacts.expected"),
        expected(&artifacts),
    )
    .map_err(|e| format!("artifacts.expected: {e}"))?;
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
        "{}: build {build}, {} modules from {} RFCs and {} other documents, {} OID assignments, {} document texts, {} example artifacts",
        dir.display(),
        corpus_data.modules.len(),
        rfc_documents,
        corpus_data.documents.len().saturating_sub(rfc_documents),
        corpus_data.oid_assignments.len(),
        texts.len(),
        artifacts.artifacts.len()
    );
    Ok(summary)
}

/// Gzips `bytes` the same way every time: the header has no time and no file name.
fn gzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write as _;
    let mut encoder = flate2::GzBuilder::new().write(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(bytes)
        .and_then(|()| encoder.finish())
        .map_err(|e| format!("{DOCUMENTS_FILE}: {e}"))
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

fn kept_as_xml_note(reason: &str) -> String {
    format!("kept as RFCXML for the web UI: {reason}")
}

fn document_text(id: String, extraction: RfcExtraction) -> DocumentText {
    DocumentText {
        id,
        format: extraction.format.to_owned(),
        text: extraction.text,
        blocks: extraction.blocks,
        xml: extraction.xml,
    }
}

/// An RFC's applied errata, and why they were not applied when they were not.
type DocumentErrata = (Vec<AppliedErratum>, Option<ErrataFailure>);

/// The errata of RFC `rfc` for its document record: each applied erratum with its ledger row (who decided, the note),
/// its patch and where its hunks are in the corrected text; for the text of an RFC whose source is XML, also its XML
/// patch and where those hunks are in the corrected XML stored beside the text.
fn document_errata(rfc: u32, extraction: &RfcExtraction, errata: &Errata) -> DocumentErrata {
    let patches_of = |series: &BTreeMap<u32, Series>| -> HashMap<u32, String> {
        match series.get(&rfc) {
            Some(Ok(list)) => list.iter().cloned().collect(),
            _ => HashMap::new(),
        }
    };
    let side = |patches: &HashMap<u32, String>, id: u32, ranges: &[LineRange]| ErratumPatch {
        patch: patches.get(&id).cloned().unwrap_or_default(),
        hunks: ranges
            .iter()
            .map(|r| HunkLines {
                start_line: r.start,
                end_line: r.end,
            })
            .collect(),
    };
    let xml_patches = patches_of(&errata.series);
    let text_patches = if extraction.text_patches {
        patches_of(&errata.text_series)
    } else {
        HashMap::new()
    };
    let shown_patches = if extraction.text_patches {
        &text_patches
    } else {
        &xml_patches
    };
    // Only for the text of an RFC whose source is XML: the lines of each erratum in the corrected XML beside it.
    let xml_ranges: HashMap<u32, &[LineRange]> = extraction
        .xml_errata_applied
        .iter()
        .map(|(id, ranges)| (*id, ranges.as_slice()))
        .collect();
    let applied = extraction
        .errata_applied
        .iter()
        .map(|(id, ranges)| {
            let row = errata.rows.get(&(rfc, *id));
            let ErratumPatch { patch, hunks } = side(shown_patches, *id, ranges);
            AppliedErratum {
                id: *id,
                by: row.map(|r| r.by.to_string()).unwrap_or_default(),
                note: row.map(|r| r.note.clone()).unwrap_or_default(),
                patch,
                hunks,
                xml: xml_ranges
                    .get(id)
                    .map(|ranges| side(&xml_patches, *id, ranges)),
            }
        })
        .collect();
    let failure = extraction.errata_failure.as_ref().map(|f| ErrataFailure {
        errata_id: f.errata_id,
        reason: f.reason.clone(),
    });
    (applied, failure)
}

/// One entry per document that publishes a module of the corpus, in the order of the modules.
fn document_index(
    records: &[ModuleRecord],
    meta: &HashMap<u32, RfcMeta>,
    drafts: &HashMap<String, (String, Option<String>)>,
    texts: &BTreeMap<String, DocumentText>,
    errata: &BTreeMap<String, DocumentErrata>,
) -> Vec<DocumentRecord> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut documents = Vec::new();
    for record in records {
        let id = record.source.document.as_str();
        if !seen.insert(id) {
            continue;
        }
        let rfc_meta = record.source.rfc_number().and_then(|n| meta.get(&n));
        let draft = drafts.get(id);
        documents.push(DocumentRecord {
            id: id.to_owned(),
            body: record.source.body.clone(),
            status: rfc_meta
                .and_then(|m| m.status.clone())
                .or_else(|| draft.and_then(|(_, state)| state.clone())),
            obsoleted_by: rfc_meta.map(|m| m.obsoleted_by.clone()).unwrap_or_default(),
            date: rfc_meta
                .and_then(|m| m.date.clone())
                .or_else(|| draft.map(|(date, _)| date.clone())),
            text_format: texts.get(id).map(|t| t.format.clone()),
            errata: errata
                .get(id)
                .map(|(applied, _)| applied.clone())
                .unwrap_or_default(),
            errata_failure: errata.get(id).and_then(|(_, failure)| failure.clone()),
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

/// A downloaded module as a candidate. `fetch` has checked that it is the module the manifest lists.
fn standalone_candidate(item: Fetched) -> Candidate {
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
        if let Some(corrected) = replaced
            .iter()
            .filter_map(|&i| candidates.get(i))
            .find(|c| !c.source.errata.is_empty())
        {
            let ids: Vec<String> = corrected.source.errata.iter().map(u32::to_string).collect();
            return Err(format!(
                "override of {} in {} replaces a module that errata {} correct; change the errata patches instead",
                item.module,
                glint_corpus::document_title(&item.document),
                ids.join(", ")
            ));
        }
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

/// Keeps one record per identity, preferring the one that parses without syntax errors, then among those the
/// longest, then the first. A document may quote a module's header with a placeholder body before the full module
/// (RFC 4120 Section 5 and Appendix A), and both parse; among copies with errors, length says nothing.
///
/// A module shown twice with the same text is nothing to report. One shown twice with different text (a header
/// stub beside the full module, or two modules that really are different) is named, so a curator can tell
/// which was kept.
fn one_per_identity(mut made: Vec<Made>) -> (Vec<ModuleRecord>, Vec<String>) {
    made.sort_by(|a, b| {
        identity(&a.record)
            .cmp(&identity(&b.record))
            .then(b.syntax_clean.cmp(&a.syntax_clean))
            .then(clean_len(b).cmp(&clean_len(a)))
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

/// The text length of a record that parses cleanly, else 0.
fn clean_len(made: &Made) -> usize {
    if made.syntax_clean {
        made.record.text.len()
    } else {
        0
    }
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
    module_identity(text, name).and_then(|(_, oid)| oid)
}

/// The module called `name` in `text`, or else its first module: the module's name and its definitive identifier,
/// `None` when the header has none (13.1 allows an empty "DefinitiveIdentification"). `None` when `text` has no
/// module.
pub(crate) fn module_identity(text: &str, name: &str) -> Option<(String, Option<Oid>)> {
    let parse = parse_source_with(text, ParseOptions::corpus());
    let file = ast::SourceFile::cast(parse.syntax())?;
    let module = file
        .modules()
        .find(|m| m.name().is_some_and(|n| n.text() == name))
        .or_else(|| file.modules().next())?;
    let model = ModuleModel::build(&module)?;
    let found = module.name()?.text().to_string();
    Some((
        found,
        model.oid_group.as_ref().and_then(Evaluator::literal_oid),
    ))
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
    records
        .iter()
        .flat_map(imports_of)
        .filter(|need| !satisfied(need, defined))
        .collect()
}

/// Whether the corpus satisfies `need`: by OID when the import has a literal one, otherwise by name.
fn satisfied(need: &Need, defined: &Defined) -> bool {
    match &need.oid {
        Some(oid) => defined.oids.contains(oid),
        None => defined.names.contains(&need.name),
    }
}

/// The modules `record` imports, each with the literal OID its `FROM` clause gives, if any.
fn imports_of(record: &ModuleRecord) -> Vec<Need> {
    let parse = parse_source_with(&record.text, ParseOptions::corpus());
    let Some(file) = ast::SourceFile::cast(parse.syntax()) else {
        return Vec::new();
    };
    let mut needs = Vec::new();
    for module in file.modules() {
        if let Some(model) = ModuleModel::build(&module) {
            for clause in &model.imports {
                let oid = match &clause.assigned {
                    Some((AssignedId::Oid { group }, _)) => Evaluator::literal_oid(group),
                    _ => None,
                };
                needs.push(Need {
                    name: clause.module_name.clone(),
                    oid,
                });
            }
        }
    }
    needs
}

/// Removes from `records`, and returns, each module with an import that nothing in the corpus satisfies and that
/// names a module of `smi`: by OID when both the import and that module have one, otherwise by name. Such a module
/// is written in the SMI module's macros. A removed module is added to `smi`, and the search repeats until it
/// removes nothing. An import the corpus satisfies is never followed to an SMI module of the same name: RFC 3416's
/// SNMPv2-PDU is ASN.1, those of RFC 1448 and 1905 are SMI.
fn skip_smi_importers(
    records: &mut Vec<ModuleRecord>,
    smi: &mut Vec<(String, Option<Oid>)>,
) -> Vec<ModuleRecord> {
    let mut removed = Vec::new();
    loop {
        let defined = defined_by(records);
        let names_smi = |need: &Need| {
            smi.iter().any(|(name, oid)| match (&need.oid, oid) {
                (Some(wanted), Some(oid)) => wanted == oid,
                _ => *name == need.name,
            })
        };
        let (skip, keep): (Vec<ModuleRecord>, Vec<ModuleRecord>) =
            std::mem::take(records).into_iter().partition(|record| {
                imports_of(record)
                    .iter()
                    .any(|need| !satisfied(need, &defined) && names_smi(need))
            });
        *records = keep;
        if skip.is_empty() {
            return removed;
        }
        smi.extend(skip.iter().map(|r| (r.name.clone(), r.oid.clone())));
        removed.extend(skip);
    }
}

fn line_of(text: &str, offset: usize) -> u32 {
    let newlines = text.get(..offset).map_or(0, |p| p.matches('\n').count());
    u32::try_from(newlines.saturating_add(1)).unwrap_or(u32::MAX)
}

/// What checking one corpus module gives.
struct Checked {
    outcome: ModuleOutcome,
    assignments: Vec<OidAssignment>,
    /// The module's rows for the OID hint table: published 2002-syntax modules only.
    hints: Vec<OidHint>,
    /// The labels the module writes, with the OIDs of their arcs: its evidence for the label table.
    labels: Vec<(Oid, String)>,
    /// The corpus modules its imports resolve to, as indexes into the records.
    imports: Vec<usize>,
}

/// The OID hint table: per OID, class and field, the row of the module ranked first by import resolution (a current RFC
/// before an obsoleted one, then the more recent document), and every OID, class and field whose modules disagree on
/// the type.
fn hint_table(
    records: &[ModuleRecord],
    rows: Vec<(usize, OidHint)>,
) -> (Vec<OidHint>, Vec<HintConflict>) {
    let mut grouped: BTreeMap<(Oid, String, String), Vec<(usize, OidHint)>> = BTreeMap::new();
    for (index, hint) in rows {
        grouped
            .entry((hint.oid.clone(), hint.class.clone(), hint.field.clone()))
            .or_default()
            .push((index, hint));
    }
    let mut table = Vec::new();
    let mut conflicts = Vec::new();
    for ((oid, class, field), mut candidates) in grouped {
        candidates.sort_by(|(a, x), (b, y)| {
            let rank = |i: &usize| records.get(*i).map(ModuleRecord::rank);
            rank(b)
                .cmp(&rank(a))
                .then_with(|| (&x.module, &x.set).cmp(&(&y.module, &y.set)))
        });
        let Some((_, winner)) = candidates.first() else {
            continue;
        };
        let losers: Vec<String> = candidates
            .iter()
            .filter(|(_, h)| h.ty != winner.ty)
            .map(|(_, h)| format!("{} `{}` in {}#{}", h.ty, h.set, h.document, h.module))
            .collect();
        if !losers.is_empty() {
            conflicts.push(HintConflict {
                oid: oid.to_string(),
                field: format!("{class}.{field}"),
                kept: format!(
                    "{} `{}` in {}#{}",
                    winner.ty, winner.set, winner.document, winner.module
                ),
                others: losers,
            });
        }
        table.push(winner.clone());
    }
    (table, conflicts)
}

/// Which modules are unmaintained: a module of an obsoleted RFC that no maintained module of another document
/// imports, directly or through other modules. Every other module (a current RFC's, a draft's, a module of another
/// standards body) is maintained. `imports[i]` are the records module `i` imports.
fn unmaintained(records: &[ModuleRecord], imports: &[Vec<usize>]) -> Vec<bool> {
    let mut maintained: Vec<bool> = records
        .iter()
        .map(|r| r.source.is_draft() || r.source.body != BODY_RFC || r.obsoleted_by.is_empty())
        .collect();
    let mut changed = true;
    while changed {
        changed = false;
        for (importer, targets) in imports.iter().enumerate() {
            if !maintained.get(importer).copied().unwrap_or(false) {
                continue;
            }
            let document = records.get(importer).map(|r| &r.source.document);
            for &target in targets {
                let other = records.get(target).map(|r| &r.source.document) != document;
                if other
                    && let Some(slot) = maintained.get_mut(target)
                    && !*slot
                {
                    *slot = true;
                    changed = true;
                }
            }
        }
    }
    maintained.into_iter().map(|m| !m).collect()
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
    let imports = analysis
        .drafts
        .first()
        .and_then(|&module| analysis.graph.module(module))
        .map(|module| {
            module
                .imports
                .iter()
                .filter_map(|import| analysis.graph.module(import.target?))
                .filter_map(|target| match target.origin {
                    Origin::Corpus { record } => Some(record),
                    Origin::Draft | Origin::Builtin => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let hints = match analysis.drafts.first() {
        Some(&module) if record.syntax_era == SyntaxEra::Y2002 && !record.source.is_draft() => {
            glint_sema::ir::oid_hints(&analysis.graph, module)
                .into_iter()
                .filter_map(|row| {
                    Some(OidHint {
                        oid: row.oid.parse().ok()?,
                        class: row.class,
                        field: row.field,
                        document: record.source.document.clone(),
                        module: record.name.clone(),
                        set: row.set,
                        ty: row.ty,
                    })
                })
                .collect()
        }
        _ => Vec::new(),
    };
    syntax.extend(findings);
    Checked {
        outcome: ModuleOutcome {
            status,
            findings: syntax,
        },
        assignments,
        hints,
        labels,
        imports,
    }
}

/// The label table and registry findings (W142, W143, W144, W146) for one corpus module, its own document's evidence left out.
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
        .filter(|f| matches!(f.code, "W142" | "W143" | "W144" | "W146"))
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
