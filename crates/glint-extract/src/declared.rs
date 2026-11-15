//! Extraction from the text rendering of an RFCXML document, with the ASN.1 blocks its XML marks.
//!
//! A reader of an RFC from 8650 on, or of a draft written in RFCXML, wants the published text (`rfcN.txt`, a draft's
//! paginated `.txt`), but the XML says which blocks are ASN.1 (`<sourcecode type="asn.1">`). [`declared_blocks`] finds,
//! in the text, the block of every element that XML extraction takes as an ASN.1 candidate ([`crate::rendered`]), with
//! the page breaks inside it; [`extract_declared`] extracts from the text with those blocks, as XML extraction would
//! from the XML, with source maps into the text.

use std::ops::Range;

use crate::block::{Origin, SectionRef};
use crate::rendered::{Margin, locate_blocks};
use crate::sink::{Candidate, Sink};
use crate::source_map::Assembler;
use crate::xml::{self, Captured, HeaderJoiner, code_elements};
use crate::{Extraction, Format, finish};

/// The block of an RFCXML element in the document's text rendering, which XML extraction takes as an ASN.1
/// candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredBlock {
    /// Byte range of the block's lines in the text: from the start of its first line to the end of its last, line
    /// break included.
    pub range: Range<usize>,
    /// The margin the text puts before each line of the element's content: `indent` bytes of spaces, with a `|` at
    /// byte `bar` when the element is quoted.
    pub indent: usize,
    pub bar: Option<usize>,
    /// Byte ranges in `range` of the page breaks between the block's lines (footer, form feed, header and the blank
    /// lines around them), which are not the element's content; empty in a text that is not paginated.
    pub breaks: Vec<Range<usize>>,
    /// The element's `type` attribute: `asn.1`, another spelling of it (`W203`), or none (taken by its content,
    /// `W202`).
    pub type_attr: Option<String>,
    pub section: Option<SectionRef>,
    pub title: Option<String>,
}

/// The blocks, in `text`, of the elements of `xml` that XML extraction takes as ASN.1 candidates, in document order;
/// `text` is the rendering of `xml`. Fails, saying why, unless every element of the XML is found in the text (see
/// [`crate::rendered`]) and the XML extraction has no finding the text could not carry (a document that is not
/// well-formed, an element with no content or an external `src`).
pub fn declared_blocks(xml: &str, text: &str) -> Result<Vec<DeclaredBlock>, String> {
    let mut sink = Sink::default();
    let mut captured: Vec<(usize, Option<String>, Origin)> = Vec::new();
    xml::walk(xml, &mut sink, |capture, element| {
        captured.push((
            capture.content_start,
            capture.type_attr.clone(),
            element.candidate.origin.clone(),
        ));
    });
    if let Some(note) = sink
        .notes
        .iter()
        .find(|n| matches!(n.code, "E203" | "I200"))
    {
        return Err(format!("XML extraction: {}: {}", note.code, note.message));
    }
    let elements = code_elements(xml);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let located = locate_blocks(&elements, &lines).map_err(|k| {
        format!(
            "<sourcecode>/<artwork> element {} of the XML is not found in the text after the block before it",
            k.saturating_add(1)
        )
    })?;
    let mut line_starts = Vec::with_capacity(lines.len().saturating_add(1));
    let mut at = 0usize;
    for line in &lines {
        line_starts.push(at);
        at = at.saturating_add(line.len());
    }
    line_starts.push(at);
    captured
        .into_iter()
        .map(|(content_start, type_attr, origin)| {
            let k = elements
                .iter()
                .position(|e| e.content.start == content_start)
                .ok_or_else(|| {
                    format!("an ASN.1 element at XML byte {content_start} is not listed")
                })?;
            let block = located.get(k).cloned().flatten().ok_or_else(|| {
                format!(
                    "<sourcecode>/<artwork> element {} of the XML is not shown in the text",
                    k.saturating_add(1)
                )
            })?;
            let (Some(&start), Some(&end)) =
                (line_starts.get(block.start), line_starts.get(block.end()))
            else {
                return Err(format!(
                    "element {} lies outside the text",
                    k.saturating_add(1)
                ));
            };
            let byte = |line: usize| line_starts.get(line).copied().unwrap_or(end);
            Ok(DeclaredBlock {
                range: start..end,
                indent: block.indent,
                bar: block.bar,
                breaks: block
                    .breaks
                    .iter()
                    .map(|lines| byte(lines.start)..byte(lines.end))
                    .collect(),
                type_attr,
                section: origin.section,
                title: origin.title,
            })
        })
        .collect()
}

/// The ASN.1 of `text` in `blocks` (from [`declared_blocks`]), taken as XML extraction takes the elements they come
/// from: by their `type`, header-only blocks joined with the next. Each block's lines lose the block's indentation,
/// and the lines of its page breaks are left out; the source maps point into `text`. A block that does not lie in
/// `text` is left out.
pub fn extract_declared(text: &str, blocks: &[DeclaredBlock]) -> Extraction {
    let mut sink = Sink::default();
    let mut joiner = HeaderJoiner::default();
    for block in blocks {
        let Some(content) = text.get(block.range.clone()) else {
            continue;
        };
        let span = block.range.clone();
        let Some(marking) = xml::marking(block.type_attr.as_deref(), &span, &mut sink) else {
            continue;
        };
        let mut assembler = Assembler::default();
        let mut at = block.range.start;
        for line in content.split_inclusive('\n') {
            if block.breaks.iter().any(|b| b.contains(&at)) {
                at = at.saturating_add(line.len());
                continue;
            }
            let indent = Margin {
                indent: block.indent,
                bar: block.bar,
            }
            .skip(line);
            assembler.push_verbatim(
                line.get(indent..).unwrap_or_default(),
                at.saturating_add(indent),
            );
            at = at.saturating_add(line.len());
        }
        let (text, map) = assembler.finish();
        let origin = Origin {
            section: block.section.clone(),
            title: block.title.clone(),
            span,
        };
        joiner.push(
            Captured {
                candidate: Candidate { text, map, origin },
                marking,
            },
            &mut sink,
        );
    }
    joiner.finish(&mut sink);
    let mut extraction = finish(Format::Text, sink);
    extraction.blocks_declared = true;
    extraction
}
