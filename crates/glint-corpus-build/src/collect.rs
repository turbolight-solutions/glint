//! Pulls the ASN.1 modules out of one RFC or Internet-Draft.

use std::path::Path;

use glint_corpus::{DocumentBlock, ModuleSource, SourceKind};
use glint_errata::store::{rfc_source, rfc_text};
use glint_errata::{LineRange, SeriesFailure, SourceFormat, final_hunk_lines};
use glint_extract::{BlockKind, Format, declared_blocks, extract, is_smi, module_slices};

use crate::errata::{Series, Step, apply};
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
    /// The document as read, with its errata applied, for `documents.bin`; for an RFC or draft whose source is XML, its
    /// published text (an RFC's with the errata's text patches) when [`RfcExtraction::blocks`] could be found in it.
    pub text: String,
    /// `xml` or `text`.
    pub format: &'static str,
    /// For the text of an RFC or draft whose source is XML: the blocks in `text` of the elements XML extraction takes as
    /// ASN.1.
    pub blocks: Option<Vec<DocumentBlock>>,
    /// The errata applied, in order, each with the lines of its hunks' new sides in `text`.
    pub errata_applied: Vec<(u32, Vec<LineRange>)>,
    /// Whether `errata_applied` are the text patches of an RFC whose source is XML.
    pub text_patches: bool,
    /// For the text of an RFC or draft whose source is XML: the XML (an RFC's corrected: the published XML with the
    /// errata's XML patches), which `documents.bin` keeps as the document's second view.
    pub xml: Option<String>,
    /// With `text_patches`: the errata applied to the XML, each with the lines of its hunks' new sides in `xml`.
    pub xml_errata_applied: Vec<(u32, Vec<LineRange>)>,
    /// Why the RFC's errata were not applied, when they were not: it is extracted as published.
    pub errata_failure: Option<SeriesFailure>,
    /// The PEM blocks of the document as read.
    pub pem: glint_extract::artifacts::PemScan,
    /// Why a document with a module, whose source is XML, is kept as its XML in `documents.bin` instead of its text
    /// ("kept as RFCXML for the web UI").
    pub kept_as_xml: Option<String>,
}

/// The modules of RFC `rfc`, from its RFCXML source when it has one, else its text, with `series` (its errata
/// patches) applied first. Each module names the errata that changed it. For an RFC whose source is XML, the text
/// kept for `documents.bin` is its published `rfcN.txt` with `text_series` (the errata's text patches) applied, when
/// the XML's ASN.1 blocks can all be found in it ([`with_text`]).
pub fn extract_rfc(
    dir: &Path,
    rfc: u32,
    series: Option<&Series>,
    text_series: Option<&Series>,
) -> Option<RfcExtraction> {
    let (path, patch_format) = rfc_source(dir, rfc)?;
    let format = match patch_format {
        SourceFormat::Xml => Format::Xml,
        SourceFormat::Text => Format::Text,
    };
    let published = read_lossy(&path)?;
    let source = |section| ModuleSource::rfc(rfc, section, SourceKind::Extracted);
    let mut result = match series {
        None => extract_text(published, format, source),
        Some(series) => match apply(rfc, patch_format, &published, series) {
            Err(failure) => {
                let mut result = extract_text(published, format, source);
                result.errata_failure = Some(failure);
                result
            }
            Ok(steps) => {
                let provenance = provenance(&published, &steps, format);
                let placements: Vec<_> = steps.iter().map(|s| s.placements.clone()).collect();
                let hunks = final_hunk_lines(&placements);
                let corrected = steps.last().map_or(published, |step| step.text.clone());
                let mut result = extract_text(corrected, format, source);
                for (candidate, errata) in result.candidates.iter_mut().zip(provenance) {
                    candidate.source.errata = errata;
                }
                result.errata_applied = steps.iter().map(|s| s.errata_id).zip(hunks).collect();
                result
            }
        },
    };
    // Only the text of an RFC with a module goes into documents.bin.
    if format == Format::Xml && result.candidates.iter().any(|c| !c.smi) {
        result.kept_as_xml = with_text(dir, rfc, &mut result, text_series).err();
    }
    Some(result)
}

/// Replaces the XML of `result` (an RFC whose source is XML, as extracted) with the RFC's published text plus the
/// text patches of the errata applied to the XML, and the blocks of the XML's ASN.1 elements in it; the XML and its
/// errata's lines in it are kept beside the text (`xml`, `xml_errata_applied`). Leaves `result`
/// as it is, saying why, when the mirror has no `rfcN.txt`, the text patches do not apply, or the blocks are not all
/// found in the text (an `xml2rfc` that re-wrapped artwork, or a text patch that disagrees with the XML patch).
fn with_text(
    dir: &Path,
    rfc: u32,
    result: &mut RfcExtraction,
    text_series: Option<&Series>,
) -> Result<(), String> {
    let path = rfc_text(dir, rfc).ok_or_else(|| format!("no rfc{rfc}.txt"))?;
    let published = read_lossy(&path).ok_or_else(|| format!("rfc{rfc}.txt is not readable"))?;
    let applied: Vec<u32> = result.errata_applied.iter().map(|(id, _)| *id).collect();
    let (text, hunks) = if applied.is_empty() {
        (published, Vec::new())
    } else {
        let series = text_series.ok_or_else(|| "the errata have no text patches".to_owned())?;
        let steps = apply(rfc, SourceFormat::Text, &published, series).map_err(|f| {
            format!(
                "the text patch of erratum {} does not apply ({})",
                f.errata_id, f.reason
            )
        })?;
        let ids: Vec<u32> = steps.iter().map(|s| s.errata_id).collect();
        if ids != applied {
            return Err(format!(
                "the text patches are of errata {ids:?}, the XML patches of {applied:?}"
            ));
        }
        let placements: Vec<_> = steps.iter().map(|s| s.placements.clone()).collect();
        let hunks = final_hunk_lines(&placements);
        let text = steps.last().map_or(published, |step| step.text.clone());
        (text, hunks)
    };
    shown_as_text(result, text)?;
    if !applied.is_empty() {
        let text_errata = applied.into_iter().zip(hunks).collect();
        result.xml_errata_applied = std::mem::replace(&mut result.errata_applied, text_errata);
        result.text_patches = true;
    }
    Ok(())
}

/// Replaces the XML of `result` (a document whose source is XML, as extracted) with `text`, its rendering, and the
/// blocks in it of the XML's ASN.1 elements, keeping the XML beside it (`xml`). Leaves `result` as it is, saying
/// why, when the blocks are not all found in the text.
fn shown_as_text(result: &mut RfcExtraction, text: String) -> Result<(), String> {
    let blocks = declared_blocks(&result.text, &text)?;
    result.blocks = Some(blocks.into_iter().map(document_block).collect());
    result.xml = Some(std::mem::replace(&mut result.text, text));
    result.format = "text";
    Ok(())
}

fn document_block(block: glint_extract::DeclaredBlock) -> DocumentBlock {
    DocumentBlock {
        start: block.range.start,
        end: block.range.end,
        indent: block.indent,
        bar: block.bar,
        breaks: block.breaks.into_iter().map(|b| [b.start, b.end]).collect(),
        type_attr: block.type_attr,
        appendix: block.section.as_ref().is_some_and(|s| s.appendix),
        section: block.section.map(|s| s.number),
        title: block.title,
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
fn provenance(published: &str, steps: &[Step], format: Format) -> Vec<Vec<u32>> {
    let mut previous = modules_of(published, format);
    let mut errata: Vec<Vec<u32>> = vec![Vec::new(); previous.len()];
    for Step {
        errata_id: id,
        text,
        ..
    } in steps
    {
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

/// The modules of an active draft, from its RFCXML source when the mirror has one, else its text. The text kept for
/// `documents.bin` of a draft read from XML is its published `.txt` (paginated), when the mirror has it and the XML's
/// ASN.1 blocks can all be found in it; drafts have no errata.
pub fn extract_draft(dir: &Path, draft: &DraftEntry) -> Option<RfcExtraction> {
    let xml = dir.join(format!("{}.xml", draft.id));
    let txt = dir.join(format!("{}.txt", draft.id));
    let (path, format) = if xml.exists() {
        (xml, Format::Xml)
    } else if txt.exists() {
        (txt.clone(), Format::Text)
    } else {
        return None;
    };
    let text = read_lossy(&path)?;
    let mut result = extract_text(text, format, |section| {
        ModuleSource::draft(&draft.id, &draft.date, section, SourceKind::Extracted)
    });
    // Only the text of a draft with a module goes into documents.bin.
    if format == Format::Xml && result.candidates.iter().any(|c| !c.smi) {
        let shown = match read_lossy(&txt) {
            Some(text) => shown_as_text(&mut result, text),
            None => Err(format!("no {}.txt", draft.id)),
        };
        result.kept_as_xml = shown.err();
    }
    Some(result)
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
    result.pem = glint_extract::artifacts::pem_blocks(&text, format);
    result.text = text;
    result
}
