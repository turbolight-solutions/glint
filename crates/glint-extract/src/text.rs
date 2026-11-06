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
    toc_dots: Regex,
    definitions: Regex,
    header_rest: Regex,
    end_line: Regex,
    assignment: Regex,
}

static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(|| {
    Some(Patterns {
        footer: Regex::new(r"\[Page \d+\]\s*$").ok()?,
        header: Regex::new(r"^(?:RFC \d+|Internet-Draft)\s{2,}\S.*\S\s{2,}[A-Z][a-z]+ \d{4}\s*$").ok()?,
        heading_number: Regex::new(r"^(\d+(?:\.\d+)*)\.?\s+([A-Z].*\S)\s*$").ok()?,
        heading_appendix: Regex::new(r"^Appendix\s+([A-Z](?:\.\d+)*)\.?\s+(\S.*\S)\s*$").ok()?,
        heading_sub_appendix: Regex::new(r"^([A-Z](?:\.\d+)+)\.?\s+([A-Z].*\S)\s*$").ok()?,
        toc_dots: Regex::new(r"\.{4,}").ok()?,
        definitions: Regex::new(r"\bDEFINITIONS\b").ok()?,
        header_rest: Regex::new(
            r"^\s*(?:(?:EXPLICIT|IMPLICIT|AUTOMATIC)\s+TAGS)?\s*(?:EXTENSIBILITY\s+IMPLIED)?\s*(?:::=)?\s*(?:BEGIN)?\s*$",
        )
        .ok()?,
        end_line: Regex::new(r"^\s*END\s*$").ok()?,
        assignment: Regex::new(r"^\s*(?:[A-Za-z][\w-]*\s*::=|[a-z][\w-]*\s+[A-Z][^:]*::=)").ok()?,
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
    /// object identifier come before it, possibly in an earlier paragraph.
    fn module_start(&self, index: usize, prefix: &str) -> usize {
        if !prefix.trim().is_empty() {
            let mut start = index;
            let mut depth = brace_balance(prefix);
            while depth > 0 && start > 0 {
                let Some(previous) = start.checked_sub(1) else {
                    break;
                };
                if self.class(previous) != Class::Content {
                    break;
                }
                start = previous;
                depth = depth.saturating_add(brace_balance(self.content(start)));
            }
            return start;
        }
        let mut previous = self.previous_content(index);
        while let Some(line) = previous {
            if !strip_comment(self.content(line)).trim().is_empty() {
                break;
            }
            previous = self.previous_content(line);
        }
        let Some(last) = previous else {
            return index;
        };
        let mut first = last;
        while first > 0 && last.saturating_sub(first) < 8 {
            let Some(previous) = first.checked_sub(1) else {
                break;
            };
            if self.class(previous) != Class::Content || self.is_end_line(previous) {
                break;
            }
            first = previous;
        }
        let paragraph: Vec<&str> = (first..=last)
            .map(|i| strip_comment(self.content(i)).trim())
            .collect();
        let starts_with_name = paragraph
            .first()
            .is_some_and(|line| line.starts_with(|c: char| c.is_ascii_uppercase()));
        let ends_properly = paragraph
            .last()
            .is_some_and(|line| line.ends_with('}') || !line.contains(char::is_whitespace));
        let not_end = !self.is_end_line(first);
        let free = (first..=last).all(|i| self.is_free_content(i));
        if starts_with_name && ends_properly && not_end && free {
            first
        } else {
            index
        }
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
