use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::block::{Origin, SectionRef};
use crate::lines::{Line, split_lines};
use crate::sink::{Candidate, Marking, Sink};
use crate::source_map::Assembler;

struct Patterns {
    footer: Regex,
    header: Regex,
    heading_number: Regex,
    heading_appendix: Regex,
    heading_sub_appendix: Regex,
    heading_top_appendix: Regex,
    toc_dots: Regex,
    definitions: Regex,
    header_rest: Regex,
    end_line: Regex,
    assignment: Regex,
    module_reference: Regex,
}

static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(|| {
    Some(Patterns {
        footer: Regex::new(r"\[Page \d+\]\s*$").ok()?,
        // The title between the document name and the date may be missing (RFC 5544).
        header: Regex::new(r"^(?:RFC \d+|Internet-Draft)(?:\s{2,}\S.*\S)?\s{2,}[A-Z][a-z]+ \d{4}\s*$").ok()?,
        heading_number: Regex::new(r"^(\d+(?:\.\d+)*)\.?\s+([A-Z].*\S)\s*$").ok()?,
        heading_appendix: Regex::new(r"^Appendix\s+([A-Z](?:\.\d+)*)\.?\s+(\S.*\S)\s*$").ok()?,
        heading_sub_appendix: Regex::new(r"^([A-Z](?:\.\d+)+)\.?\s+([A-Z].*\S)\s*$").ok()?,
        // `A.  ASN.1 module` (RFC 4120): an appendix without the word "Appendix"; the dot is required, so that a
        // line starting with a one-letter word is not taken for one.
        heading_top_appendix: Regex::new(r"^([A-Z])\.\s+([A-Z].*\S)\s*$").ok()?,
        toc_dots: Regex::new(r"\.{4,}").ok()?,
        definitions: Regex::new(r"\bDEFINITIONS\b").ok()?,
        header_rest: Regex::new(
            r"^\s*(?:(?:EXPLICIT|IMPLICIT|AUTOMATIC)\s+TAGS)?\s*(?:EXTENSIBILITY\s+IMPLIED)?\s*(?:::=)?\s*(?:BEGIN)?\s*$",
        )
        .ok()?,
        end_line: Regex::new(r"^\s*END\s*$").ok()?,
        assignment: Regex::new(r"^\s*(?:[A-Za-z][\w-]*\s*::=|[a-z][\w-]*\s+[A-Z][^:]*::=)").ok()?,
        // X.680 12.5 modulereference: the characters of a typereference (12.2)
        module_reference: Regex::new(r"^[A-Z][A-Za-z0-9]*(?:-[A-Za-z0-9]+)*$").ok()?,
    })
});

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Blank,
    Furniture,
    Content,
}

/// First and last line index of a block, both inclusive.
#[derive(Clone, Copy)]
struct Found {
    first: usize,
    last: usize,
}

struct Document<'a> {
    source: &'a str,
    patterns: &'a Patterns,
    lines: Vec<Line>,
    classes: Vec<Class>,
    consumed: Vec<bool>,
}

fn strip_comment(text: &str) -> &str {
    text.find("--")
        .and_then(|at| text.get(..at))
        .unwrap_or(text)
}

/// Whether `text` is code followed by a `--` comment on the same line.
fn has_trailing_comment(text: &str) -> bool {
    text.contains("--") && !strip_comment(text).trim().is_empty()
}

fn indent_of(text: &str) -> usize {
    text.bytes()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count()
}

fn brace_balance(text: &str) -> i32 {
    strip_comment(text).chars().fold(0_i32, |depth, c| match c {
        '{' => depth.saturating_add(1),
        '}' => depth.saturating_sub(1),
        _ => depth,
    })
}

impl<'a> Document<'a> {
    fn content(&self, index: usize) -> &'a str {
        self.lines
            .get(index)
            .map_or("", |line| line.content(self.source))
    }

    fn class(&self, index: usize) -> Class {
        self.classes.get(index).copied().unwrap_or(Class::Blank)
    }

    fn is_free_content(&self, index: usize) -> bool {
        self.class(index) == Class::Content && !self.consumed.get(index).copied().unwrap_or(true)
    }

    fn next_content(&self, from: usize) -> Option<usize> {
        (from..self.lines.len()).find(|&i| self.class(i) == Class::Content)
    }

    fn previous_content(&self, before: usize) -> Option<usize> {
        (0..before).rev().find(|&i| self.class(i) == Class::Content)
    }

    fn mark_consumed(&mut self, found: Found) {
        for flag in self
            .consumed
            .iter_mut()
            .skip(found.first)
            .take(found.last.saturating_sub(found.first).saturating_add(1))
        {
            *flag = true;
        }
    }

    fn is_heading(&self, index: usize) -> bool {
        let content = self.content(index);
        !content.starts_with([' ', '\t']) && heading_of(self.patterns, content).is_some()
    }

    /// The line where the module whose `DEFINITIONS` keyword is on line `index` begins: the module reference and
    /// object identifier (X.680 13.1) come before it, on the same line, on the lines just above, or in earlier
    /// paragraphs, as in `Name {` / arcs / `} DEFINITIONS` (RFC 4120) or `Name` / blank / `{ arcs }` / blank /
    /// `DEFINITIONS` (RFC 5751). `index` itself when no header is found, so the block starts at the keyword.
    fn module_start(&self, index: usize, prefix: &str) -> usize {
        if prefix.trim().is_empty() {
            return self
                .previous_code(index)
                .and_then(|last| self.header_start(last, 0))
                .unwrap_or(index);
        }
        let depth = brace_balance(prefix);
        if depth >= 0 {
            // `Name { arcs } DEFINITIONS` or `Name DEFINITIONS`: the whole header is on this line.
            return index;
        }
        self.header_start(index, depth).unwrap_or(index)
    }

    /// The previous content line before `before` that holds code, skipping lines that are only a comment.
    fn previous_code(&self, before: usize) -> Option<usize> {
        let mut previous = self.previous_content(before);
        while let Some(line) = previous {
            if !strip_comment(self.content(line)).trim().is_empty() {
                return Some(line);
            }
            previous = self.previous_content(line);
        }
        None
    }

    /// The first line of a module header whose last line before `DEFINITIONS` is `last`. A negative `depth` is
    /// the brace balance of the text before `DEFINITIONS` on `last` (`} DEFINITIONS`); otherwise `last` is a line
    /// of its own above the keyword. Walks back to the `{` that opens the object identifier, across blank lines
    /// and page furniture, then to the module reference: the text before that `{`, or a lone reference on the
    /// previous line. The layouts, as RFCs write them:
    /// - `Name {` / arcs / `} DEFINITIONS ...` (RFC 4120 Section 5);
    /// - `Name { arcs` / `arcs }` / blank / `DEFINITIONS ...` (RFC 5280 Appendix A.1);
    /// - `Name` / blank / `{ arcs }` / blank / `DEFINITIONS ...` (RFC 5751 Appendix A, RFC 5055 Section 8);
    /// - `Name` / blank / `DEFINITIONS ...`, with no identifier (RFC 3163 Appendix A.1).
    fn header_start(&self, last: usize, depth: i32) -> Option<usize> {
        /// Lines an object identifier may span; RFC headers use at most five.
        const MAX_OID_LINES: usize = 12;
        let mut depth = if depth < 0 {
            depth
        } else {
            let code = strip_comment(self.content(last)).trim();
            if self.patterns.module_reference.is_match(code) {
                // `Name` alone, then `DEFINITIONS` with no object identifier.
                return self.usable_header(last, last);
            }
            if !code.ends_with('}') {
                return None;
            }
            brace_balance(code)
        };
        let mut line = last;
        let mut spanned = 1_usize;
        while depth < 0 {
            if spanned >= MAX_OID_LINES {
                return None;
            }
            line = self.previous_content(line)?;
            spanned = spanned.saturating_add(1);
            if self.is_end_line(line) || self.is_heading(line) {
                return None;
            }
            depth = depth.saturating_add(brace_balance(self.content(line)));
        }
        let opening = strip_comment(self.content(line));
        let before_brace = opening.split('{').next().unwrap_or_default().trim();
        // The reference is the last word: RFC 2156 writes `MIXER Definitions { ... }`, which the parser reads as
        // a module named `Definitions`.
        let reference = before_brace.split_whitespace().last().unwrap_or_default();
        if before_brace.starts_with(|c: char| c.is_ascii_uppercase())
            && self.patterns.module_reference.is_match(reference)
        {
            return self.usable_header(line, last);
        }
        if !before_brace.is_empty() {
            return None;
        }
        let name = self.previous_code(line)?;
        let reference = strip_comment(self.content(name)).trim();
        if self.patterns.module_reference.is_match(reference) {
            self.usable_header(name, last)
        } else {
            None
        }
    }

    /// `first` when the header lines `first..=last` are free content, and `first` is not an `END` or a heading.
    fn usable_header(&self, first: usize, last: usize) -> Option<usize> {
        let free = (first..=last)
            .filter(|&i| self.class(i) == Class::Content)
            .all(|i| self.is_free_content(i));
        (free && !self.is_end_line(first) && !self.is_heading(first)).then_some(first)
    }

    fn is_end_line(&self, index: usize) -> bool {
        self.patterns
            .end_line
            .is_match(strip_comment(self.content(index)))
    }

    /// The `DEFINITIONS` header on this line, if any: the text before the keyword.
    fn header_prefix(&self, index: usize) -> Option<&'a str> {
        if !self.is_free_content(index) {
            return None;
        }
        let code = strip_comment(self.content(index));
        let found = self.patterns.definitions.find(code)?;
        let rest = code.get(found.end()..)?;
        self.patterns
            .header_rest
            .is_match(rest)
            .then(|| code.get(..found.start()))
            .flatten()
    }

    fn find_modules(&mut self, found: &mut Vec<Found>) {
        let mut index = 0;
        while index < self.lines.len() {
            let Some(prefix) = self.header_prefix(index) else {
                index = index.saturating_add(1);
                continue;
            };
            let first = self.module_start(index, prefix);
            let last = self.module_end(index);
            let block = Found { first, last };
            self.mark_consumed(block);
            found.push(block);
            index = last.saturating_add(1);
        }
    }

    fn module_end(&self, header_line: usize) -> usize {
        let mut last = header_line;
        let mut index = header_line.saturating_add(1);
        while index < self.lines.len() {
            match self.class(index) {
                Class::Furniture | Class::Blank => {}
                Class::Content => {
                    if self.consumed.get(index).copied().unwrap_or(true) || self.is_heading(index) {
                        break;
                    }
                    if let Some(prefix) = self.header_prefix(index) {
                        let next_start = self.module_start(index, prefix);
                        return self
                            .previous_content(next_start)
                            .map_or(last, |previous| previous.max(last));
                    }
                    last = index;
                    if self.is_end_line(index) {
                        break;
                    }
                }
            }
            index = index.saturating_add(1);
        }
        last
    }

    fn find_code_markers(&mut self, found: &mut Vec<Found>) {
        let mut index = 0;
        while index < self.lines.len() {
            if !self
                .content(index)
                .trim_start()
                .starts_with("<CODE BEGINS>")
            {
                index = index.saturating_add(1);
                continue;
            }
            let ends = (index.saturating_add(1)..self.lines.len())
                .find(|&i| self.content(i).trim_start().starts_with("<CODE ENDS>"));
            let Some(ends) = ends else {
                index = index.saturating_add(1);
                continue;
            };
            self.mark_consumed(Found {
                first: index,
                last: ends,
            });
            let first = index.saturating_add(1);
            if let Some(last) = ends.checked_sub(1)
                && first <= last
            {
                found.push(Found { first, last });
            }
            index = ends.saturating_add(1);
        }
    }

    fn find_fragments(&mut self, found: &mut Vec<Found>) {
        let mut index = 0;
        while index < self.lines.len() {
            if self.is_free_content(index) && self.patterns.assignment.is_match(self.content(index))
            {
                let last = self.fragment_end(index);
                let block = Found { first: index, last };
                self.mark_consumed(block);
                found.push(block);
                index = last.saturating_add(1);
            } else {
                index = index.saturating_add(1);
            }
        }
    }

    fn fragment_end(&self, start: usize) -> usize {
        let base_indent = indent_of(self.content(start));
        let mut depth = brace_balance(self.content(start));
        let mut last = start;
        let mut index = start.saturating_add(1);
        while index < self.lines.len() {
            match self.class(index) {
                Class::Furniture => {}
                Class::Blank => {
                    if depth <= 0 {
                        let continues = self.next_content(index).is_some_and(|next| {
                            self.is_free_content(next)
                                && indent_of(self.content(next)) == base_indent
                                && self.patterns.assignment.is_match(self.content(next))
                        });
                        if !continues {
                            break;
                        }
                    }
                }
                Class::Content => {
                    if !self.is_free_content(index) {
                        break;
                    }
                    let content = self.content(index);
                    let indent = indent_of(content);
                    // At depth 0, a line at the block's own indent continues it
                    // when it starts the next assignment, opens the braces of
                    // this one, or adds a constraint after the closing brace,
                    // as in `( WITH COMPONENTS { ... } )` (RFC 8226). A `(`
                    // after code with a trailing comment is more likely the
                    // comment wrapped onto the next line (RFC 2560's
                    // `-- SHA-1 hash ...` / `(excluding the tag ...)`).
                    let opens_constraint = || {
                        content.trim_start().starts_with('(')
                            && !has_trailing_comment(self.content(last))
                    };
                    let belongs = if depth > 0 {
                        indent >= base_indent
                    } else {
                        indent > base_indent
                            || (indent == base_indent
                                && (self.patterns.assignment.is_match(content)
                                    || content.trim_start().starts_with('{')
                                    || opens_constraint()))
                    };
                    if !belongs {
                        break;
                    }
                    depth = depth.saturating_add(brace_balance(content));
                    last = index;
                }
            }
            index = index.saturating_add(1);
        }
        last
    }

    fn candidate(&self, found: Found, sections: &[(usize, SectionRef, String)]) -> Candidate {
        let included: Vec<usize> = (found.first..=found.last)
            .filter(|&i| self.class(i) != Class::Furniture)
            .collect();
        let trimmed: Vec<usize> = {
            let leading = included
                .iter()
                .position(|&i| self.class(i) == Class::Content)
                .unwrap_or(0);
            let trailing = included
                .iter()
                .rposition(|&i| self.class(i) == Class::Content)
                .map_or(0, |p| p.saturating_add(1));
            included
                .get(leading..trailing.max(leading))
                .unwrap_or_default()
                .to_vec()
        };
        let indent = trimmed
            .iter()
            .filter(|&&i| self.class(i) == Class::Content)
            .map(|&i| indent_of(self.content(i)))
            .min()
            .unwrap_or(0);

        let mut assembler = Assembler::default();
        for &i in &trimmed {
            let Some(line) = self.lines.get(i) else {
                continue;
            };
            let content = line.content(self.source);
            let skip = indent.min(indent_of(content));
            assembler.push_verbatim(
                content.get(skip..).unwrap_or_default(),
                line.start.saturating_add(skip),
            );
            match line.terminator(self.source) {
                "" => {}
                "\r\n" => assembler.push_mapped("\n", line.content_end..line.end),
                other => assembler.push_verbatim(other, line.content_end),
            }
        }
        let (text, map) = assembler.finish();

        let start = self.lines.get(found.first).map_or(0, |line| line.start);
        let end = self
            .lines
            .get(found.last)
            .map_or(start, |line| line.content_end);
        let section = sections
            .iter()
            .rev()
            .find(|(line, _, _)| *line <= found.first);
        Candidate {
            text,
            map,
            origin: Origin {
                section: section.map(|(_, section, _)| section.clone()),
                title: section.map(|(_, _, title)| title.clone()),
                span: Range { start, end },
            },
        }
    }
}

fn classify_line(patterns: &Patterns, content: &str) -> Class {
    let without_feed = content.trim_start_matches('\u{0C}');
    if without_feed.trim().is_empty() {
        return if content.len() == without_feed.len() {
            Class::Blank
        } else {
            Class::Furniture
        };
    }
    if patterns.footer.is_match(without_feed) || patterns.header.is_match(without_feed) {
        Class::Furniture
    } else {
        Class::Content
    }
}

fn heading_of(patterns: &Patterns, content: &str) -> Option<(SectionRef, String)> {
    if patterns.toc_dots.is_match(content) {
        return None;
    }
    let (captures, appendix) = if let Some(c) = patterns.heading_appendix.captures(content) {
        (c, true)
    } else if let Some(c) = patterns.heading_sub_appendix.captures(content) {
        (c, true)
    } else if let Some(c) = patterns.heading_top_appendix.captures(content) {
        (c, true)
    } else {
        (patterns.heading_number.captures(content)?, false)
    };
    Some((
        SectionRef {
            number: captures.get(1)?.as_str().to_owned(),
            appendix,
        },
        captures.get(2)?.as_str().to_owned(),
    ))
}

/// Finds ASN.1 in a plain-text draft or RFC: between `<CODE BEGINS>` and `<CODE ENDS>`, from a module header to
/// its `END`, and in runs of assignments. Page headers, footers and form feeds are dropped.
pub(crate) fn extract(source: &str, sink: &mut Sink) {
    let Some(patterns) = PATTERNS.as_ref() else {
        return;
    };
    let lines = split_lines(source);
    let classes: Vec<Class> = lines
        .iter()
        .map(|line| classify_line(patterns, line.content(source)))
        .collect();
    let consumed = vec![false; lines.len()];
    let mut document = Document {
        source,
        patterns,
        lines,
        classes,
        consumed,
    };

    let sections: Vec<(usize, SectionRef, String)> = (0..document.lines.len())
        .filter(|&i| document.class(i) == Class::Content)
        .filter_map(|i| {
            let content = document.content(i);
            if content.starts_with([' ', '\t']) {
                return None;
            }
            heading_of(patterns, content).map(|(section, title)| (i, section, title))
        })
        .collect();

    let mut found = Vec::new();
    document.find_code_markers(&mut found);
    document.find_modules(&mut found);
    document.find_fragments(&mut found);
    found.sort_by_key(|block| block.first);

    for block in found {
        let candidate = document.candidate(block, &sections);
        sink.accept(candidate, Marking::Guessed { warn: false });
    }
}
