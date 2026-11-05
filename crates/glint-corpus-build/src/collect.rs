//! Pulls the ASN.1 modules out of one RFC or Internet-Draft.

use std::path::Path;

use glint_corpus::{ModuleSource, SourceKind};
use glint_extract::{BlockKind, Format, extract, module_slices};
use regex::Regex;

use crate::index::DraftEntry;

/// Modules written in SMI use macros of their own and are checked by `smilint`, not by Glint.
fn smi_pattern() -> Option<Regex> {
    Regex::new(
        r"\b(OBJECT-TYPE|MODULE-IDENTITY|NOTIFICATION-TYPE|OBJECT-IDENTITY|TEXTUAL-CONVENTION|MODULE-COMPLIANCE|OBJECT-GROUP|NOTIFICATION-GROUP|AGENT-CAPABILITIES|TRAP-TYPE)\b|FROM\s+(SNMPv2-SMI|RFC1155-SMI|RFC1065-SMI|SNMPv2-TC|SNMPv2-CONF|RFC-1212|RFC-1215)\b",
    )
    .ok()
}

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
    /// The document as read, for `documents.bin`.
    pub text: String,
    /// `xml` or `text`.
    pub format: &'static str,
}

/// RFCs from 8650 on have an RFCXML v3 source; earlier ones are read as text.
const FIRST_XML_RFC: u32 = 8650;

pub fn extract_rfc(dir: &Path, rfc: u32) -> Option<RfcExtraction> {
    let xml = dir.join(format!("rfc{rfc}.xml"));
    let txt = dir.join(format!("rfc{rfc}.txt"));
    let (path, format) = if rfc >= FIRST_XML_RFC && xml.exists() {
        (xml, Format::Xml)
    } else if txt.exists() {
        (txt, Format::Text)
    } else {
        return None;
    };
    extract_file(&path, format, |section| {
        ModuleSource::rfc(rfc, section, SourceKind::Extracted)
    })
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
    extract_file(&path, format, |section| {
        ModuleSource::draft(&draft.id, &draft.date, section, SourceKind::Extracted)
    })
}

fn extract_file(
    path: &Path,
    format: Format,
    source: impl Fn(Option<String>) -> ModuleSource,
) -> Option<RfcExtraction> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let extraction = extract(&text, format);
    let smi = smi_pattern();
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
            let smi_module = smi.as_ref().is_some_and(|re| re.is_match(&body));
            result.candidates.push(Candidate {
                name,
                text: body,
                smi: smi_module,
                source: source(block.origin.section.as_ref().map(ToString::to_string)),
            });
        }
    }
    result.text = text;
    Some(result)
}
