use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::block::{Origin, SectionRef};
use crate::sink::{Candidate, Marking, Sink};
use crate::source_map::Assembler;

struct Code {
    info: Option<String>,
    pieces: Vec<(String, Range<usize>)>,
    span: Range<usize>,
}

struct Heading {
    level: HeadingLevel,
    numbered: bool,
    title: String,
}

#[derive(Default)]
struct Sections {
    counters: Vec<u32>,
    in_back: bool,
    current: Option<(SectionRef, String)>,
}

impl Sections {
    fn reset(&mut self, in_back: bool) {
        self.counters.clear();
        self.in_back = in_back;
        self.current = None;
    }

    fn enter(&mut self, heading: &Heading) {
        let depth = heading_depth(heading.level);
        if !heading.numbered {
            self.current = None;
            return;
        }
        self.counters.resize(depth.saturating_add(1), 0);
        if let Some(slot) = self.counters.get_mut(depth) {
            *slot = slot.saturating_add(1);
        }
        let number = self
            .counters
            .iter()
            .enumerate()
            .map(|(level, count)| {
                if level == 0 && self.in_back {
                    u32::from(b'A')
                        .checked_add(count.saturating_sub(1))
                        .and_then(char::from_u32)
                        .filter(char::is_ascii_uppercase)
                        .map_or_else(|| count.to_string(), String::from)
                } else {
                    count.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(".");
        self.current = Some((
            SectionRef {
                number,
                appendix: self.in_back,
            },
            heading.title.trim().to_owned(),
        ));
    }
}

fn heading_depth(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 0,
        HeadingLevel::H2 => 1,
        HeadingLevel::H3 => 2,
        HeadingLevel::H4 => 3,
        HeadingLevel::H5 => 4,
        HeadingLevel::H6 => 5,
    }
}

fn is_asn1_info(info: &str) -> Option<bool> {
    let word = info.split_whitespace().next()?;
    if word == "asn.1" || word == "asn1" {
        Some(true)
    } else if word.eq_ignore_ascii_case("asn.1") || word.eq_ignore_ascii_case("asn1") {
        Some(false)
    } else {
        None
    }
}

/// Finds ASN.1 in fenced code blocks with the info string `asn.1` or `asn1`, and in indented or untyped code
/// blocks that look like ASN.1.
pub(crate) fn extract(source: &str, sink: &mut Sink) {
    let options = Options::ENABLE_YAML_STYLE_METADATA_BLOCKS | Options::ENABLE_HEADING_ATTRIBUTES;
    let mut sections = Sections::default();
    let mut code: Option<Code> = None;
    let mut heading: Option<Heading> = None;

    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let info = match kind {
                    CodeBlockKind::Fenced(info) => Some(info.into_string()),
                    CodeBlockKind::Indented => None,
                };
                code = Some(Code {
                    info,
                    pieces: Vec::new(),
                    span: range,
                });
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(done) = code.take() {
                    finish_code(source, done, &sections, sink);
                }
            }
            Event::Start(Tag::Heading { level, attrs, .. }) => {
                let numbered = !attrs.iter().any(|(key, value)| {
                    key.as_ref() == "numbered"
                        && value
                            .as_ref()
                            .is_some_and(|v| v.trim_matches(['"', '\'']) == "false")
                });
                heading = Some(Heading {
                    level,
                    numbered,
                    title: String::new(),
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(done) = heading.take() {
                    sections.enter(&done);
                }
            }
            Event::Text(text) => {
                if let Some(open) = code.as_mut() {
                    open.pieces.push((text.into_string(), range));
                } else if let Some(open) = heading.as_mut() {
                    open.title.push_str(&text);
                } else {
                    match text.trim() {
                        "--- back" | "{backmatter}" => sections.reset(true),
                        "--- middle" | "{mainmatter}" => sections.reset(false),
                        _ => {}
                    }
                }
            }
            Event::Code(text) => {
                if let Some(open) = heading.as_mut() {
                    open.title.push_str(&text);
                }
            }
            _ => {}
        }
    }
}

fn finish_code(source: &str, code: Code, sections: &Sections, sink: &mut Sink) {
    let marking = match code.info.as_deref().map(str::trim) {
        None | Some("") => Marking::Guessed { warn: true },
        Some(info) => match is_asn1_info(info) {
            Some(true) => Marking::Declared,
            Some(false) => {
                sink.note(
                    "W203",
                    format!("non-standard info string `{info}` for ASN.1; use `asn.1`"),
                    code.span.clone(),
                );
                Marking::Declared
            }
            None => return,
        },
    };

    let mut assembler = Assembler::default();
    for (text, range) in &code.pieces {
        if source.get(range.clone()) == Some(text.as_str()) {
            assembler.push_verbatim(text, range.start);
        } else {
            assembler.push_mapped(text, range.clone());
        }
    }
    let (text, map) = assembler.finish();
    let origin = Origin {
        section: sections
            .current
            .as_ref()
            .map(|(section, _)| section.clone()),
        title: sections.current.as_ref().map(|(_, title)| title.clone()),
        span: code.span,
    };
    sink.accept(Candidate { text, map, origin }, marking);
}
