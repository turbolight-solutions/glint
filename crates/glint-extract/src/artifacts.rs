//! Finds the PEM blocks of a document (Codegen and Decoder Design, Example artifacts: Extraction).
//!
//! A block runs from `-----BEGIN <label>-----` to `-----END <label>-----`. In a plain-text document the page furniture
//! inside it is dropped; in RFCXML each `<sourcecode>` and `<artwork>` element is scanned on its own, so a block that
//! spans elements is not joined. RFC 8792 folding (`\` and `\\`) is undone, RFC 1421 headers (`Proc-Type: ...`) are
//! skipped, and a block is kept only when its base64 decodes and its octets are complete BER; any other block gets a
//! [`PemFinding`]. OpenPGP armor (RFC 4880, `-----BEGIN PGP ...-----`) looks like PEM but is not DER: it is counted
//! and left out.

use std::ops::Range;

use glint_decode::{NoOracle, base64, decode};

use crate::Format;
use crate::block::SectionRef;
use crate::lines::split_lines;
use crate::text::{headings, page_line};
use crate::xml::located_elements;

/// One PEM block whose octets are BER.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PemBlock {
    /// 1-based position among the document's blocks, broken ones included and OpenPGP armor left out: the `n` of
    /// the artifact id `rfc9500-n`.
    pub index: usize,
    pub label: String,
    pub der: Vec<u8>,
    pub place: Place,
}

/// Where a block is in its document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Place {
    pub section: Option<SectionRef>,
    /// The section's title.
    pub title: Option<String>,
    /// The figure caption: the `<figure>`'s name in RFCXML, a `Figure N: ...` line after the block in text.
    pub caption: Option<String>,
    /// 1-based line of the `BEGIN` line in the document.
    pub line: u32,
    /// From the start of the `BEGIN` line to the end of the `END` line (or of the last line read).
    pub span: Range<usize>,
}

/// Why a PEM block was not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PemProblem {
    /// No `END` line before the next `BEGIN`, a section heading or the end of the text.
    Unterminated,
    /// The `END` line names another label.
    LabelMismatch,
    /// The body is not base64 (often an example elided with `...`).
    Base64,
    /// The octets are empty, or not one complete BER encoding.
    NotBer,
}

/// A PEM block that was not kept, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PemFinding {
    pub index: usize,
    pub label: String,
    pub problem: PemProblem,
    /// One line, no trailing period.
    pub message: String,
    pub place: Place,
}

/// The PEM blocks of one document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PemScan {
    pub blocks: Vec<PemBlock>,
    pub findings: Vec<PemFinding>,
    /// OpenPGP armor blocks: label and line.
    pub openpgp: Vec<(String, u32)>,
}

/// One line as the scanner reads it: its text (without the line break) and its offset in the document.
struct ScanLine<'a> {
    text: &'a str,
    offset: usize,
    /// A section heading of a plain-text document: a block does not run past one.
    heading: bool,
}

/// What every block of one run of lines shares, and how to place a block found in it.
trait Locate {
    fn place(&self, first: usize, last: usize, lines: &[ScanLine<'_>]) -> Place;
}

fn begin_label(text: &str) -> Option<&str> {
    text.trim()
        .strip_prefix("-----BEGIN ")
        .and_then(|rest| rest.strip_suffix("-----"))
}

fn end_label(text: &str) -> Option<&str> {
    text.trim()
        .strip_prefix("-----END ")
        .and_then(|rest| rest.strip_suffix("-----"))
}

/// Scans `lines` for PEM blocks, numbering them from `scan`'s count so far.
fn scan_lines(lines: &[ScanLine<'_>], locate: &dyn Locate, scan: &mut PemScan) {
    let mut i = 0;
    while i < lines.len() {
        let Some(label) = lines.get(i).and_then(|l| begin_label(l.text)) else {
            i = i.saturating_add(1);
            continue;
        };
        let label = label.to_owned();
        let first = i;
        let mut j = i.saturating_add(1);
        let mut end: Option<String> = None;
        while let Some(line) = lines.get(j) {
            if let Some(found) = end_label(line.text) {
                end = Some(found.to_owned());
                break;
            }
            if begin_label(line.text).is_some() || line.heading {
                break;
            }
            j = j.saturating_add(1);
        }
        if label.starts_with("PGP ") {
            let line = locate.place(first, first, lines).line;
            scan.openpgp.push((label, line));
            i = if end.is_some() {
                j.saturating_add(1)
            } else {
                j
            };
            continue;
        }
        let index = scan
            .blocks
            .len()
            .saturating_add(scan.findings.len())
            .saturating_add(1);
        let Some(end) = end else {
            let last = j.saturating_sub(1).max(first);
            scan.findings.push(PemFinding {
                index,
                problem: PemProblem::Unterminated,
                message: format!("PEM block `{label}` has no `-----END {label}-----` line"),
                label,
                place: locate.place(first, last, lines),
            });
            i = j;
            continue;
        };
        let place = locate.place(first, j, lines);
        i = j.saturating_add(1);
        if end != label {
            scan.findings.push(PemFinding {
                index,
                problem: PemProblem::LabelMismatch,
                message: format!("PEM block `{label}` ends with `-----END {end}-----`"),
                label,
                place,
            });
            continue;
        }
        let body = lines
            .get(first.saturating_add(1)..j)
            .unwrap_or_default()
            .iter()
            .map(|l| l.text);
        match read_body(body) {
            Ok(der) => scan.blocks.push(PemBlock {
                index,
                label,
                der,
                place,
            }),
            Err((problem, message)) => scan.findings.push(PemFinding {
                index,
                label,
                problem,
                message,
                place,
            }),
        }
    }
}

/// The octets of a PEM body: RFC 8792 folding undone, RFC 1421 headers skipped, base64 decoded, checked to be BER.
fn read_body<'a>(lines: impl Iterator<Item = &'a str>) -> Result<Vec<u8>, (PemProblem, String)> {
    let mut text = String::new();
    let mut folded = false;
    for line in lines {
        let mut line = line.trim();
        // RFC 8792 Section 7.2: the continuation of a `\\` fold starts with a `\`.
        if folded {
            line = line.strip_prefix('\\').unwrap_or(line);
        }
        folded = line.ends_with('\\');
        line = line.strip_suffix('\\').unwrap_or(line);
        // RFC 1421 encapsulated headers (RFC 7468 Section 2 says parsers may ignore them).
        if !folded && line.contains(':') {
            continue;
        }
        text.push_str(line);
    }
    if text.contains("...") || text.contains('…') {
        return Err((
            PemProblem::Base64,
            "PEM body is elided (`...`), not base64".to_owned(),
        ));
    }
    let Some(der) = base64(text.as_bytes()) else {
        let bad = text
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_')))
            .map_or_else(String::new, |c| format!(": `{}`", c.escape_default()));
        return Err((PemProblem::Base64, format!("PEM body is not base64{bad}")));
    };
    if der.is_empty() {
        return Err((PemProblem::NotBer, "PEM body is empty".to_owned()));
    }
    let decoded = decode(&der, None, &NoOracle);
    if let Some(finding) = decoded
        .all_findings()
        .into_iter()
        .find(|f| matches!(f.code, "E400" | "E409"))
    {
        return Err((
            PemProblem::NotBer,
            format!("PEM body is not one BER encoding: {}", finding.message),
        ));
    }
    Ok(der)
}

fn line_number(source: &str, offset: usize) -> u32 {
    let before = source.get(..offset).unwrap_or(source);
    let count = before.bytes().filter(|&b| b == b'\n').count();
    u32::try_from(count.saturating_add(1)).unwrap_or(u32::MAX)
}

/// The PEM blocks of `source`, read as `format`. Markdown and bare files are scanned as plain text.
pub fn pem_blocks(source: &str, format: Format) -> PemScan {
    let mut scan = PemScan::default();
    if !source.contains("-----BEGIN ") {
        return scan;
    }
    match format {
        Format::Xml => scan_xml(source, &mut scan),
        Format::Text | Format::Markdown | Format::Bare => scan_text(source, &mut scan),
    }
    scan
}

struct TextLocate<'a> {
    source: &'a str,
    sections: Vec<(usize, SectionRef, String)>,
    /// For each scanned line, its index among the document's lines.
    numbers: Vec<usize>,
}

impl Locate for TextLocate<'_> {
    fn place(&self, first: usize, last: usize, lines: &[ScanLine<'_>]) -> Place {
        let start = lines.get(first).map_or(0, |l| l.offset);
        let end = lines
            .get(last)
            .map_or(start, |l| l.offset.saturating_add(l.text.len()));
        let document_line = self.numbers.get(first).copied().unwrap_or(0);
        let section = self
            .sections
            .iter()
            .rev()
            .find(|(line, _, _)| *line <= document_line);
        Place {
            section: section.map(|(_, s, _)| s.clone()),
            title: section.map(|(_, _, t)| t.clone()),
            caption: caption_after(lines, last),
            line: line_number(self.source, start),
            span: start..end,
        }
    }
}

/// The caption of a `Figure N: caption` line that is the first text after line `last`.
fn caption_after(lines: &[ScanLine<'_>], last: usize) -> Option<String> {
    let text = lines
        .iter()
        .skip(last.saturating_add(1))
        .map(|line| line.text.trim())
        .find(|text| !text.is_empty())?;
    let caption = text.strip_prefix("Figure ")?.split_once(':')?.1.trim();
    (!caption.is_empty()).then(|| caption.to_owned())
}

fn scan_text(source: &str, scan: &mut PemScan) {
    let all = split_lines(source);
    let sections = headings(source, &all);
    let heading_lines: std::collections::HashSet<usize> =
        sections.iter().map(|(line, _, _)| *line).collect();
    let mut lines = Vec::new();
    let mut numbers = Vec::new();
    for (number, line) in all.iter().enumerate() {
        let text = line.content(source);
        if page_line(text).is_some() {
            continue;
        }
        lines.push(ScanLine {
            text,
            offset: line.start,
            heading: heading_lines.contains(&number),
        });
        numbers.push(number);
    }
    let locate = TextLocate {
        source,
        sections,
        numbers,
    };
    scan_lines(&lines, &locate, scan);
}

struct ElementLocate<'a> {
    source: &'a str,
    section: Option<SectionRef>,
    title: Option<String>,
    caption: Option<String>,
}

impl Locate for ElementLocate<'_> {
    fn place(&self, first: usize, last: usize, lines: &[ScanLine<'_>]) -> Place {
        let start = lines.get(first).map_or(0, |l| l.offset);
        let end = lines
            .get(last)
            .map_or(start, |l| l.offset.saturating_add(l.text.len()));
        Place {
            section: self.section.clone(),
            title: self.title.clone(),
            caption: self.caption.clone(),
            line: line_number(self.source, start),
            span: start..end,
        }
    }
}

fn scan_xml(source: &str, scan: &mut PemScan) {
    for element in located_elements(source) {
        if !element.text.contains("-----BEGIN ") {
            continue;
        }
        let lines: Vec<ScanLine<'_>> = split_lines(&element.text)
            .iter()
            .map(|line| ScanLine {
                text: line.content(&element.text),
                offset: element.map.map_start(line.start).unwrap_or(0),
                heading: false,
            })
            .collect();
        let locate = ElementLocate {
            source,
            section: element.section.clone(),
            title: element.title.clone(),
            caption: element.caption.clone(),
        };
        scan_lines(&lines, &locate, scan);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn folded_lines_are_joined() {
        let body = ["MAMC\\", "  AQU=", "MAMC\\", "   \\AQU="];
        assert_eq!(
            read_body(body[..2].iter().copied()).unwrap(),
            [0x30, 0x03, 0x02, 0x01, 0x05]
        );
        assert_eq!(
            read_body(body[2..].iter().copied()).unwrap(),
            [0x30, 0x03, 0x02, 0x01, 0x05]
        );
    }

    #[test]
    fn headers_are_skipped_and_garbage_reported() {
        assert!(read_body(["Proc-Type: 4,ENCRYPTED", "", "MAMCAQU="].into_iter()).is_ok());
        assert_eq!(
            read_body(["MAMC", "..."].into_iter()).unwrap_err().0,
            PemProblem::Base64
        );
        assert_eq!(
            read_body(["MAUCAQU="].into_iter()).unwrap_err().0,
            PemProblem::NotBer
        );
    }
}
