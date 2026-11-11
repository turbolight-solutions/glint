//! Pulls the ASN.1 modules out of one RFC or Internet-Draft.

use std::path::Path;

use glint_corpus::{ModuleSource, SourceKind};
use glint_errata::{SeriesFailure, SourceFormat};
use glint_extract::{BlockKind, Format, extract, is_smi, module_slices};

use crate::errata::{Series, apply};
use crate::index::DraftEntry;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub text: String,
    pub smi: bool,
    pub source: ModuleSource,
}

#[derive(Debug, Default)]
pub struct RfcExtraction {
    pub candidates: Vec<Candidate>,
    pub notes: Vec<String>,
    /// The document as read, with its errata applied, for `documents.bin`.
    pub text: String,
    /// `xml` or `text`.
    pub format: &'static str,
    /// The errata applied, in order.
    pub errata_applied: Vec<u32>,
    /// Why the RFC's errata were not applied, when they were not: it is extracted as published.
    pub errata_failure: Option<SeriesFailure>,
}

/// RFCs from 8650 on have an RFCXML v3 source; earlier ones are read as text.
const FIRST_XML_RFC: u32 = 8650;

/// The modules of RFC `rfc`, from its RFCXML source when it has one, else its text, with `series` (its errata
/// patches) applied first. Each module names the errata that changed it.
pub fn extract_rfc(dir: &Path, rfc: u32, series: Option<&Series>) -> Option<RfcExtraction> {
    let xml = dir.join(format!("rfc{rfc}.xml"));
    let txt = dir.join(format!("rfc{rfc}.txt"));
    let (path, format) = if rfc >= FIRST_XML_RFC && xml.exists() {
        (xml, Format::Xml)
    } else if txt.exists() {
        (txt, Format::Text)
    } else {
        return None;
    };
    let published = read_lossy(&path)?;
    let source = |section| ModuleSource::rfc(rfc, section, SourceKind::Extracted);
    let Some(series) = series else {
        return Some(extract_text(published, format, source));
    };
    let patch_format = if format == Format::Xml {
        SourceFormat::Xml
    } else {
        SourceFormat::Text
    };
    match apply(rfc, patch_format, &published, series) {
        Err(failure) => {
            let mut result = extract_text(published, format, source);
            result.errata_failure = Some(failure);
            Some(result)
        }
        Ok(steps) => {
            let provenance = provenance(&published, &steps, format);
            let corrected = steps.last().map_or(published, |(_, text)| text.clone());
            let mut result = extract_text(corrected, format, source);
            for (candidate, errata) in result.candidates.iter_mut().zip(provenance) {
                candidate.source.errata = errata;
            }
            result.errata_applied = steps.iter().map(|(id, _)| *id).collect();
            Some(result)
        }
    }
}

/// The modules of `text` as (name, text), in extraction order.
fn modules_of(text: &str, format: Format) -> Vec<(String, String)> {
    extract_text(text.to_owned(), format, |section| {
        ModuleSource::rfc(0, section, SourceKind::Extracted)
    })
    .candidates
    .into_iter()
    .map(|c| (c.name, c.text))
    .collect()
}

/// For each module of the last text of `steps`, the errata whose patch changed it. A module is followed from one text
/// to the next by name, or by position when the patch renamed it (RFC 7508 erratum 5875 corrects a module name) and
/// the number of modules stayed the same; a module a patch adds or changes is attributed to that erratum.
fn provenance(published: &str, steps: &[(u32, String)], format: Format) -> Vec<Vec<u32>> {
    let mut previous = modules_of(published, format);
    let mut errata: Vec<Vec<u32>> = vec![Vec::new(); previous.len()];
    for (id, text) in steps {
        let next = modules_of(text, format);
        let mut used = vec![false; previous.len()];
        let mut next_errata = Vec::with_capacity(next.len());
        for (index, (name, body)) in next.iter().enumerate() {
            let free = |j: &usize| used.get(*j) == Some(&false);
            let by_name = previous
                .iter()
                .enumerate()
                .position(|(j, (n, _))| n == name && free(&j));
            let by_position = (next.len() == previous.len())
                .then_some(index)
                .filter(|j| free(j));
            let matched = by_name.or(by_position);
            let mut ids = matched
                .and_then(|j| errata.get(j).cloned())
                .unwrap_or_default();
            if let Some(slot) = matched.and_then(|j| used.get_mut(j)) {
                *slot = true;
            }
            let unchanged = matched
                .and_then(|j| previous.get(j))
                .is_some_and(|(_, old)| old == body);
            if !unchanged && !ids.contains(id) {
                ids.push(*id);
            }
            next_errata.push(ids);
        }
        previous = next;
        errata = next_errata;
    }
    errata
}

/// The modules of an active draft, from its RFCXML source when the mirror has one, else its text.
pub fn extract_draft(dir: &Path, draft: &DraftEntry) -> Option<RfcExtraction> {
    let xml = dir.join(format!("{}.xml", draft.id));
    let txt = dir.join(format!("{}.txt", draft.id));
    let (path, format) = if xml.exists() {
        (xml, Format::Xml)
    } else if txt.exists() {
        (txt, Format::Text)
    } else {
        return None;
    };
    let text = read_lossy(&path)?;
    Some(extract_text(text, format, |section| {
        ModuleSource::draft(&draft.id, &draft.date, section, SourceKind::Extracted)
    }))
}

fn read_lossy(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn extract_text(
    text: String,
    format: Format,
    source: impl Fn(Option<String>) -> ModuleSource,
) -> RfcExtraction {
    let extraction = extract(&text, format);
    let mut result = RfcExtraction {
        format: if format == Format::Xml { "xml" } else { "text" },
        ..RfcExtraction::default()
    };
    for note in &extraction.notes {
        result
            .notes
            .push(format!("{}: {}", note.code, note.message));
    }
    for block in &extraction.blocks {
        if block.kind == BlockKind::Fragment {
            continue;
        }
        for slice in module_slices(block) {
            let Some(name) = slice.name else { continue };
            let Some(body) = block.text.get(slice.range) else {
                continue;
            };
            let body = body.trim().to_owned();
            let smi_module = is_smi(&body);
            result.candidates.push(Candidate {
                name,
                text: body,
                smi: smi_module,
                source: source(block.origin.section.as_ref().map(ToString::to_string)),
            });
        }
    }
    result.text = text;
    result
}
