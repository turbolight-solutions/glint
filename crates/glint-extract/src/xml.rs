use std::ops::Range;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::block::BlockKind;
use crate::block::{Origin, SectionRef};
use crate::classify::{block_kind, looks_like_asn1};
use crate::sink::{Candidate, Marking, Sink};
use crate::source_map::Assembler;

/// What an `<sourcecode>` or `<artwork>` element holds, judged by its `type` attribute.
enum Typing {
    Asn1 { exact: bool },
    Other,
    Missing,
}

fn typing(type_attr: Option<&str>) -> Typing {
    match type_attr.map(str::trim) {
        None | Some("") => Typing::Missing,
        Some("asn.1") => Typing::Asn1 { exact: true },
        Some(other)
            if other.eq_ignore_ascii_case("asn.1") || other.eq_ignore_ascii_case("asn1") =>
        {
            Typing::Asn1 { exact: false }
        }
        Some(_) => Typing::Other,
    }
}

struct Frame {
    name: String,
    is_section: bool,
}

const MAX_SECTION_DEPTH: usize = 32;

#[derive(Clone)]
struct SectionInfo {
    number: Option<String>,
    appendix: bool,
    title: Option<String>,
}

struct Capture {
    element: String,
    type_attr: Option<String>,
    content_start: usize,
    tag_start: usize,
    section: Option<SectionInfo>,
}

#[derive(Default)]
struct Numbering {
    counters: Vec<u32>,
    in_back: bool,
}

impl Numbering {
    fn reset(&mut self, in_back: bool) {
        self.counters.clear();
        self.in_back = in_back;
    }

    /// Assigns the next number at `depth` (0 for a top-level section); `None` when an ancestor is unnumbered.
    fn next(&mut self, depth: usize, parent: Option<&SectionInfo>) -> Option<String> {
        if depth >= MAX_SECTION_DEPTH || parent.is_some_and(|p| p.number.is_none()) {
            return None;
        }
        self.counters.resize(depth.saturating_add(1), 0);
        let slot = self.counters.get_mut(depth)?;
        *slot = slot.saturating_add(1);
        let parts: Vec<String> = self
            .counters
            .iter()
            .enumerate()
            .map(|(level, count)| {
                if level == 0 && self.in_back {
                    appendix_letter(*count)
                } else {
                    count.to_string()
                }
            })
            .collect();
        Some(parts.join("."))
    }
}

fn appendix_letter(count: u32) -> String {
    let index = count.saturating_sub(1);
    char::from_u32(u32::from(b'A').saturating_add(index))
        .filter(char::is_ascii_uppercase)
        .map_or_else(|| count.to_string(), String::from)
}

fn position(reader: &Reader<&[u8]>) -> usize {
    usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
}

fn local_name(start: &BytesStart<'_>) -> String {
    start.local_name().as_ref().to_owned()
}

fn attribute(start: &BytesStart<'_>, name: &str) -> Option<String> {
    start
        .attributes()
        .with_checks(false)
        .filter_map(Result::ok)
        .find(|attr| attr.key.local_name().as_ref() == name)
        .map(|attr| decode_entities(&attr.value))
}

fn decode_entities(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        let (before, from_amp) = rest.split_at(at);
        out.push_str(before);
        match entity_at(from_amp) {
            Some((decoded, length)) => {
                out.push(decoded);
                rest = from_amp.get(length..).unwrap_or_default();
            }
            None => {
                out.push('&');
                rest = from_amp.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// Decodes the predefined or numeric character reference at the start of `text`; returns the character and the
/// length of the reference.
fn entity_at(text: &str) -> Option<(char, usize)> {
    let end = text.find(';')?;
    let name = text.get(1..end)?;
    let decoded = match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((decoded, end.saturating_add(1)))
}

/// Builds the text of an element from its raw content: character references are decoded, CDATA sections are
/// unwrapped, and comments and nested tags are dropped.
fn decode_content(source: &str, content: Range<usize>) -> Assembler {
    let mut assembler = Assembler::default();
    let mut at = content.start;
    while at < content.end {
        let Some(rest) = source.get(at..content.end) else {
            break;
        };
        if let Some(inner) = rest.strip_prefix("<![CDATA[") {
            let body_start = at.saturating_add("<![CDATA[".len());
            let body = inner
                .find("]]>")
                .map_or(inner, |i| inner.get(..i).unwrap_or(inner));
            assembler.push_verbatim(body, body_start);
            let consumed = "<![CDATA[".len().saturating_add(body.len());
            at = at.saturating_add(consumed);
            if inner
                .get(body.len()..)
                .is_some_and(|tail| tail.starts_with("]]>"))
            {
                at = at.saturating_add(3);
            }
        } else if rest.starts_with("<!--") {
            let length = rest.find("-->").map_or(rest.len(), |i| i.saturating_add(3));
            at = at.saturating_add(length);
        } else if rest.starts_with('<') {
            let length = rest.find('>').map_or(rest.len(), |i| i.saturating_add(1));
            at = at.saturating_add(length);
        } else if rest.starts_with('&') {
            match entity_at(rest) {
                Some((decoded, length)) => {
                    let mut buffer = [0_u8; 4];
                    assembler.push_mapped(
                        decoded.encode_utf8(&mut buffer),
                        at..at.saturating_add(length),
                    );
                    at = at.saturating_add(length);
                }
                None => {
                    assembler.push_verbatim("&", at);
                    at = at.saturating_add(1);
                }
            }
        } else {
            let length = rest.find(['<', '&']).unwrap_or(rest.len());
            assembler.push_verbatim(rest.get(..length).unwrap_or_default(), at);
            at = at.saturating_add(length);
        }
    }
    assembler
}

/// Finds ASN.1 in `<sourcecode>` (RFCXML v3) and `<artwork>` (RFCXML v2 and v3) elements.
pub(crate) fn extract(source: &str, sink: &mut Sink) {
    let mut reader = Reader::from_str(source);
    let config = reader.config_mut();
    config.check_end_names = false;
    config.allow_unmatched_ends = true;
    config.allow_dangling_amp = true;

    let mut stack: Vec<Frame> = Vec::new();
    let mut sections: Vec<SectionInfo> = Vec::new();
    let mut numbering = Numbering::default();
    let mut capture: Option<Capture> = None;
    let mut name_text: Option<String> = None;

    loop {
        let before = position(&reader);
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(error) => {
                let at = position(&reader).min(source.len());
                sink.note(
                    "E203",
                    format!("document is not well-formed XML: {error}"),
                    at..at,
                );
                break;
            }
        };
        let after = position(&reader);
        match event {
            Event::Eof => break,
            Event::Start(start) => {
                let name = local_name(&start);
                if capture.is_none() {
                    match name.as_str() {
                        "back" => numbering.reset(true),
                        "middle" => numbering.reset(false),
                        _ => {}
                    }
                    if matches!(name.as_str(), "sourcecode" | "artwork") {
                        capture = Some(Capture {
                            type_attr: attribute(&start, "type"),
                            content_start: after,
                            tag_start: before,
                            section: sections.last().cloned(),
                            element: name.clone(),
                        });
                    } else if name == "name" && stack.last().is_some_and(|f| f.is_section) {
                        name_text = Some(String::new());
                    }
                }
                let is_section = name == "section";
                if is_section {
                    sections.push(section_info(&start, &sections, &mut numbering));
                }
                stack.push(Frame { name, is_section });
            }
            Event::Empty(start) => {
                let name = local_name(&start);
                if capture.is_none()
                    && matches!(name.as_str(), "sourcecode" | "artwork")
                    && attribute(&start, "src").is_some()
                    && matches!(
                        typing(attribute(&start, "type").as_deref()),
                        Typing::Asn1 { .. }
                    )
                {
                    sink.note(
                        "I200",
                        "ASN.1 block refers to an external file (`src`); skipped",
                        before..after,
                    );
                }
            }
            Event::Text(text) => {
                if let Some(title) = name_text.as_mut() {
                    title.push_str(&decode_entities(&text));
                }
            }
            Event::End(end) => {
                let name = end.local_name().as_ref().to_owned();
                if capture.as_ref().is_some_and(|c| c.element == name)
                    && let Some(done) = capture.take()
                {
                    finish_capture(source, &done, before, after, sink);
                }
                if name == "name"
                    && let Some(title) = name_text.take()
                {
                    set_section_title(&stack, &mut sections, title);
                }
                close_element(&name, &mut stack, &mut sections);
            }
            _ => {}
        }
    }
    if let Some(unfinished) = capture {
        sink.note(
            "I200",
            "ASN.1 block is not closed before the end of the document; skipped",
            unfinished.tag_start..source.len(),
        );
    }
}

fn section_info(
    start: &BytesStart<'_>,
    sections: &[SectionInfo],
    numbering: &mut Numbering,
) -> SectionInfo {
    let numbered = attribute(start, "numbered").is_none_or(|value| value != "false");
    let number = if numbered {
        numbering.next(sections.len(), sections.last())
    } else {
        None
    };
    SectionInfo {
        number,
        appendix: numbering.in_back,
        title: attribute(start, "title"),
    }
}

fn set_section_title(stack: &[Frame], sections: &mut [SectionInfo], title: String) {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let named_section_is_open = stack
        .iter()
        .rev()
        .nth(1)
        .is_some_and(|frame| frame.is_section);
    if named_section_is_open && let Some(info) = sections.last_mut() {
        info.title = Some(title);
    }
}

fn close_element(name: &str, stack: &mut Vec<Frame>, sections: &mut Vec<SectionInfo>) {
    let Some(index) = stack.iter().rposition(|frame| frame.name == name) else {
        return;
    };
    let closed_sections = stack
        .iter()
        .skip(index)
        .filter(|frame| frame.is_section)
        .count();
    stack.truncate(index);
    sections.truncate(sections.len().saturating_sub(closed_sections));
}

fn finish_capture(
    source: &str,
    capture: &Capture,
    end_start: usize,
    end_after: usize,
    sink: &mut Sink,
) {
    let content = capture.content_start..end_start.max(capture.content_start);
    let span = capture.tag_start..end_after;
    let marking = match typing(capture.type_attr.as_deref()) {
        Typing::Asn1 { exact } => {
            if !exact {
                sink.note(
                    "W203",
                    format!(
                        "non-standard type `{}` for ASN.1; use `asn.1`",
                        capture.type_attr.as_deref().unwrap_or_default()
                    ),
                    span.clone(),
                );
            }
            Marking::Declared
        }
        Typing::Other => return,
        Typing::Missing => Marking::Guessed { warn: true },
    };

    let (text, map) = decode_content(source, content).finish();
    if capture.element == "artwork" && marking != Marking::Declared {
        let is_module = looks_like_asn1(&text) && block_kind(&text) != BlockKind::Fragment;
        if !is_module {
            return;
        }
    }
    let origin = Origin {
        section: capture.section.as_ref().and_then(|info| {
            info.number.clone().map(|number| SectionRef {
                number,
                appendix: info.appendix,
            })
        }),
        title: capture.section.as_ref().and_then(|info| info.title.clone()),
        span,
    };
    sink.accept(Candidate { text, map, origin }, marking);
}
