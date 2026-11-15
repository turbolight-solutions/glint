//! Where the `<sourcecode>` and `<artwork>` elements of an RFCXML document are in its text rendering.
//!
//! An RFC from 8650 on is published as RFCXML (`rfcN.xml`) and as `xml2rfc`'s rendering of it (`rfcN.txt`).
//! `xml2rfc` copies the content of `<sourcecode>` and `<artwork>` into the text verbatim, entities decoded, adding only
//! a uniform left indentation (and dropping blank lines around the content and whitespace at line ends). So the k-th
//! element of the XML is the k-th block of the text, found in order, each after the one before it. Matching by block
//! rather than by text keeps a definition an RFC repeats (a module and its inline copy) on its own copy.
//!
//! Inside a `<blockquote>`, `xml2rfc` also draws a bar: each line of the block starts with spaces, a `|` at a fixed
//! column, then spaces (RFC 8951 section 4 quotes RFC 7030's text this way), and a blank line is the bar alone.
//!
//! An Internet-Draft's text is paginated: a block can run across a page break, the page's footer line
//! (`Author  Expires <date>  [Page N]`), a form feed (on a line of its own or in front of the header), the next page's
//! header line (`Internet-Draft  <title>  <Month Year>`), and the blank lines `xml2rfc` puts around them, recognised as
//! plain-text extraction recognises page furniture ([`crate::text::page_line`]). Such a break is skipped between two
//! lines of a block, and a blank line of the content next to it may be missing from the text; nothing else is
//! skipped, so a break cannot hide a line of text that is not the element's. RFCs from 8650 on are not paginated.

use std::ops::Range;

use crate::text::{PageLine, page_line};
use crate::xml::CodeElement;

/// Where an element's content is in the text: the `len` lines from the 0-based line `start`, but for the page breaks
/// in `breaks`, are its lines, each after a margin of `indent` bytes more than in the element: spaces, with a `|` at
/// byte `bar` in a quoted block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBlock {
    pub start: usize,
    pub len: usize,
    pub indent: usize,
    pub bar: Option<usize>,
    /// The page breaks between the block's lines, as ranges of lines: a footer, a form feed, a header and the blank
    /// lines around them. Empty in a text that is not paginated.
    pub breaks: Vec<Range<usize>>,
}

/// The margin the text puts before each line of a block: `indent` bytes of spaces, with a `|` at byte `bar` when the
/// block is quoted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Margin {
    pub indent: usize,
    pub bar: Option<usize>,
}

impl Margin {
    /// The margin read off a prefix (`"   "`, or `"   |     "`), or `None` when it is not one.
    fn parse(prefix: &str) -> Option<Margin> {
        let bytes = prefix.as_bytes();
        let bars: Vec<usize> = (0..bytes.len())
            .filter(|&i| bytes.get(i) == Some(&b'|'))
            .collect();
        let only_spaces_else = bytes
            .iter()
            .enumerate()
            .all(|(i, &b)| b == b' ' || bars.first() == Some(&i));
        match bars.as_slice() {
            [] | [_] if only_spaces_else => Some(Margin {
                indent: bytes.len(),
                bar: bars.first().copied(),
            }),
            _ => None,
        }
    }

    /// How many leading bytes of `line` (a line of the text) are this margin: `indent` for a line of the block, all
    /// of a blank quoted line (`   |  `), up to the first byte that differs otherwise.
    pub fn skip(&self, line: &str) -> usize {
        let bytes = line.as_bytes();
        let mut n = 0usize;
        while n < self.indent {
            let expected = if self.bar == Some(n) { b'|' } else { b' ' };
            match bytes.get(n) {
                Some(&b) if b == expected => n = n.saturating_add(1),
                _ => break,
            }
        }
        n
    }

    /// The margin as text, for a line of the block with text.
    pub fn text(&self) -> String {
        (0..self.indent)
            .map(|i| if self.bar == Some(i) { '|' } else { ' ' })
            .collect()
    }

    /// A blank line of the block: empty, or the bar alone in a quoted block.
    pub fn blank(&self) -> String {
        self.bar.map_or(String::new(), |bar| {
            let mut line = " ".repeat(bar);
            line.push('|');
            line
        })
    }

    /// Whether `text_line` (trailing whitespace removed) is `line` (a display line of the element) behind this margin.
    fn holds(&self, text_line: &str, line: &str) -> bool {
        if line.is_empty() {
            return text_line == self.blank();
        }
        text_line.len() == self.indent.saturating_add(line.len())
            && self.skip(text_line) == self.indent
            && text_line.get(self.indent..) == Some(line)
    }
}

impl TextBlock {
    pub fn margin(&self) -> Margin {
        Margin {
            indent: self.indent,
            bar: self.bar,
        }
    }

    /// The line after the block.
    pub fn end(&self) -> usize {
        self.start.saturating_add(self.len)
    }
}

/// A line of an element's content as `xml2rfc` writes it into the text, before its indentation: tabs expanded to
/// 8-column stops, no-break spaces as spaces, and no whitespace at the end.
fn display_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut column = 0usize;
    for c in line.chars() {
        match c {
            '\t' => {
                let next = column.saturating_add(8).saturating_sub(column % 8);
                out.extend(std::iter::repeat_n(' ', next.saturating_sub(column)));
                column = next;
            }
            '\u{a0}' => {
                out.push(' ');
                column = column.saturating_add(1);
            }
            c => {
                out.push(c);
                column = column.saturating_add(1);
            }
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// The lines of an element's content as the text shows them (tabs expanded, no-break spaces as spaces, no whitespace
/// at line ends), without the blank lines before the first and after the last line with text.
pub fn display_lines(content: &str) -> Vec<String> {
    let lines: Vec<String> = content.split('\n').map(display_line).collect();
    let first = lines.iter().position(|l| !l.is_empty());
    let last = lines.iter().rposition(|l| !l.is_empty());
    match (first, last) {
        (Some(first), Some(last)) => lines.get(first..=last).unwrap_or_default().to_vec(),
        _ => Vec::new(),
    }
}

/// Whether `xml2rfc` shows the element in the text: SVG artwork is left out.
pub fn is_rendered(element: &CodeElement) -> bool {
    !element
        .type_attr
        .as_deref()
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("svg"))
}

/// The margin the text adds when its lines from `at` are `lines` (display lines, the first one not blank), or `None`
/// when they are not. `text` is the text's lines, with or without their line breaks.
pub fn block_margin(text: &[&str], at: usize, lines: &[String]) -> Option<Margin> {
    let window = text.get(at..at.checked_add(lines.len())?)?;
    let (first_text, first_line) = (window.first()?.trim_end(), lines.first()?);
    let margin = Margin::parse(first_text.strip_suffix(first_line.as_str())?)?;
    window
        .iter()
        .zip(lines.iter())
        .skip(1)
        .all(|(text_line, line)| margin.holds(text_line.trim_end(), line))
        .then_some(margin)
}

/// A text line as page furniture: without its line break, but with its form feed.
fn furniture(text: &[&str], at: usize) -> Option<PageLine> {
    page_line(text.get(at)?.trim_end_matches(['\n', '\r']))
}

/// Whether line `at` of the text is blank: whitespace only, and no form feed.
fn is_blank(text: &[&str], at: usize) -> bool {
    text.get(at)
        .is_some_and(|line| line.trim().is_empty() && !line.contains('\u{0C}'))
}

/// The line after the page break that starts at line `at` of the text, or `None` when none does: blank lines, the
/// footer, the form feed alone and the header (or the header behind the form feed), blank lines.
fn page_break(text: &[&str], at: usize) -> Option<usize> {
    let mut at = at;
    while is_blank(text, at) {
        at = at.saturating_add(1);
    }
    if furniture(text, at)? != PageLine::Footer {
        return None;
    }
    at = at.saturating_add(1);
    match furniture(text, at)? {
        PageLine::Header { after_feed: true } => {}
        PageLine::Feed => {
            at = at.saturating_add(1);
            if furniture(text, at)? != (PageLine::Header { after_feed: false }) {
                return None;
            }
        }
        PageLine::Footer | PageLine::Header { after_feed: false } => return None,
    }
    at = at.saturating_add(1);
    while is_blank(text, at) {
        at = at.saturating_add(1);
    }
    Some(at)
}

/// The block that `lines` (display lines, the first one not blank) make when they start at line `at` of the text, or
/// `None` when they do not. Between two lines, the text may have a page break ([`page_break`]), next to which the
/// blank lines of `lines` may be missing; nothing else may come between them. `text` is the text's lines, with or
/// without their line breaks.
pub fn match_block(text: &[&str], at: usize, lines: &[String]) -> Option<TextBlock> {
    let (first_text, first_line) = (text.get(at)?.trim_end(), lines.first()?);
    let margin = Margin::parse(first_text.strip_suffix(first_line.as_str())?)?;
    let mut breaks = Vec::new();
    let mut next = at.saturating_add(1);
    let mut k = 1usize;
    while let Some(line) = lines.get(k) {
        let holds = |at: usize, line: &str| {
            text.get(at).is_some_and(|text_line| {
                !text_line.contains('\u{0C}') && margin.holds(text_line.trim_end(), line)
            })
        };
        if holds(next, line) {
            next = next.saturating_add(1);
            k = k.saturating_add(1);
            continue;
        }
        let after = page_break(text, next)?;
        breaks.push(next..after);
        next = after;
        while lines.get(k).is_some_and(String::is_empty) && !holds(next, "") {
            k = k.saturating_add(1);
        }
    }
    Some(TextBlock {
        start: at,
        len: next.saturating_sub(at),
        indent: margin.indent,
        bar: margin.bar,
        breaks,
    })
}

/// Each element's block in the text, in order: every shown element with content must be found, after the block of
/// the element before it ([`match_block`]). Elements the text does not show (SVG, empty) get `None`. `Err(k)`: element
/// `k` (0-based) is not found.
pub fn locate_blocks(
    elements: &[CodeElement],
    text: &[&str],
) -> Result<Vec<Option<TextBlock>>, usize> {
    let mut cursor = 0usize;
    let mut found = Vec::with_capacity(elements.len());
    for (k, element) in elements.iter().enumerate() {
        let lines = display_lines(&element.text);
        if lines.is_empty() || !is_rendered(element) {
            found.push(None);
            continue;
        }
        let block = (cursor..text.len())
            .find_map(|at| match_block(text, at, &lines))
            .ok_or(k)?;
        cursor = block.end();
        found.push(Some(block));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        display_lines(text)
    }

    #[test]
    fn a_plain_block_is_found_behind_its_indentation() {
        let text = ["intro", "   A ::= INTEGER", "", "   B ::= BOOLEAN", "after"];
        let margin = block_margin(&text, 1, &lines("A ::= INTEGER\n\nB ::= BOOLEAN\n"));
        assert_eq!(
            margin,
            Some(Margin {
                indent: 3,
                bar: None
            })
        );
    }

    #[test]
    fn a_quoted_block_is_found_behind_the_bar() {
        // As xml2rfc draws a <sourcecode> inside a <blockquote> (RFC 8951 section 4).
        let text = [
            "   |  follows:",
            "   |  ",
            "   |     A ::= SEQUENCE {",
            "   |       b  INTEGER }",
            "   |  ",
            "   |     C ::= BOOLEAN",
            "   |  ",
        ];
        let element = lines("A ::= SEQUENCE {\n  b  INTEGER }\n\nC ::= BOOLEAN\n");
        let margin = block_margin(&text, 2, &element).unwrap();
        assert_eq!(
            margin,
            Margin {
                indent: 9,
                bar: Some(3)
            }
        );
        assert_eq!(margin.skip("   |     A ::= SEQUENCE {"), 9);
        assert_eq!(margin.skip("   |  "), 6);
        assert_eq!(margin.text(), "   |     ");
        assert_eq!(margin.blank(), "   |");
        // The same lines without the bar, or with the bar in another column, are not the block.
        assert_eq!(block_margin(&text, 1, &element), None);
        let shifted = ["  |      A ::= SEQUENCE {", "   |       b  INTEGER }"];
        assert_eq!(
            block_margin(&shifted, 0, &lines("A ::= SEQUENCE {\n  b  INTEGER }")),
            None
        );
    }

    /// A page break as `xml2rfc` paginates a draft: blank lines, the footer, the form feed, the header, blank lines.
    const BREAK: [&str; 7] = [
        "",
        "",
        "Smyslov                   Expires 30 April 2027                 [Page 7]",
        "\u{c}",
        "Internet-Draft     Algorithm Identifiers for FrodoKEM       October 2026",
        "",
        "",
    ];

    fn with_break(before: &[&str], page_break: &[&str], after: &[&str]) -> Vec<String> {
        before
            .iter()
            .chain(page_break)
            .chain(after)
            .map(|line| format!("{line}\n"))
            .collect()
    }

    fn locate(text: &[String], content: &str) -> Option<TextBlock> {
        let element = CodeElement {
            element: "sourcecode".to_owned(),
            type_attr: Some("asn.1".to_owned()),
            content: 0..content.len(),
            text: content.to_owned(),
        };
        let text: Vec<&str> = text.iter().map(String::as_str).collect();
        locate_blocks(&[element], &text).ok()?.pop()?
    }

    #[test]
    fn a_block_runs_across_a_page_break() {
        let text = with_break(
            &["intro", "   A ::= SEQUENCE {"],
            &BREAK,
            &["     b  INTEGER }", "after"],
        );
        let block = locate(&text, "A ::= SEQUENCE {\n  b  INTEGER }\n").unwrap();
        assert_eq!((block.start, block.len, block.indent), (1, 9, 3));
        assert_eq!(block.breaks, vec![Range { start: 2, end: 9 }]);
    }

    #[test]
    fn the_form_feed_may_start_the_header_line_and_blank_lines_may_be_missing() {
        let page_break = [
            "Author                    Expires 30 April 2027                 [Page 7]",
            "\u{c}Internet-Draft                   Title                   October 2026",
        ];
        let text = with_break(&["   A ::= INTEGER"], &page_break, &["   B ::= BOOLEAN"]);
        let block = locate(&text, "A ::= INTEGER\n\nB ::= BOOLEAN\n").unwrap();
        assert_eq!((block.start, block.len), (0, 4));
        assert_eq!(block.breaks, vec![Range { start: 1, end: 3 }]);
        // A blank line of the content kept before the break is the block's own.
        let text = with_break(&["   A ::= INTEGER"], &BREAK, &["   B ::= BOOLEAN"]);
        let block = locate(&text, "A ::= INTEGER\n\nB ::= BOOLEAN\n").unwrap();
        assert_eq!(block.breaks, vec![Range { start: 2, end: 8 }]);
    }

    #[test]
    fn a_page_break_does_not_swallow_text() {
        let content = "A ::= SEQUENCE {\n  b  INTEGER }\n";
        let found = |before: &[&str], page_break: &[&str], after: &[&str]| {
            locate(&with_break(before, page_break, after), content)
        };
        let (first, last) = (["   A ::= SEQUENCE {"], ["     b  INTEGER }"]);
        assert!(found(&first, &BREAK, &last).is_some());
        // A line of prose between the block's lines, before the break, after it, or instead of the header.
        let mut prose = BREAK.to_vec();
        prose.insert(1, "   Prose.");
        assert!(found(&first, &prose, &last).is_none());
        let mut prose = BREAK.to_vec();
        prose.insert(6, "   Prose.");
        assert!(found(&first, &prose, &last).is_none());
        let mut no_header = BREAK.to_vec();
        no_header[4] = "   Prose.";
        assert!(found(&first, &no_header, &last).is_none());
        // A footer, form feed or header alone is not a page break, nor are blank lines.
        for alone in [&BREAK[2..3], &BREAK[3..4], &BREAK[4..5], &BREAK[..2]] {
            assert!(found(&first, alone, &last).is_none(), "{alone:?}");
        }
        // A block does not start or end at a break: its first line is found after it, and its end is its last line.
        let text = with_break(&[], &BREAK, &["   A ::= SEQUENCE {", "     b  INTEGER }"]);
        let block = locate(&text, content).unwrap();
        assert_eq!((block.start, block.len), (7, 2));
        assert!(block.breaks.is_empty());
    }

    #[test]
    fn a_quoted_block_runs_across_a_page_break() {
        let text = with_break(
            &["   |     A ::= SEQUENCE {", "   |"],
            &BREAK,
            &["   |       b  INTEGER }"],
        );
        let block = locate(&text, "A ::= SEQUENCE {\n\n  b  INTEGER }\n").unwrap();
        assert_eq!((block.start, block.len, block.bar), (0, 10, Some(3)));
        assert_eq!(block.breaks, vec![Range { start: 2, end: 9 }]);
    }

    #[test]
    fn a_margin_is_spaces_with_at_most_one_bar() {
        assert_eq!(
            Margin::parse("    "),
            Some(Margin {
                indent: 4,
                bar: None
            })
        );
        assert_eq!(
            Margin::parse("  |  "),
            Some(Margin {
                indent: 5,
                bar: Some(2)
            })
        );
        assert_eq!(Margin::parse(" | | "), None);
        assert_eq!(Margin::parse("  x "), None);
    }
}
