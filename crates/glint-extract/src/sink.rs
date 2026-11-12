use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::block::{Block, BlockKind, Origin};
use crate::classify::{block_kind, is_inline_asn1, looks_like_asn1};
use crate::lines::split_lines;
use crate::notes::ExtractNote;
use crate::source_map::{Assembler, Mapped, SourceMap};

static MARKER_LINE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\s*<CODE (?:BEGINS|ENDS)>").ok());

/// How a block was identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Marking {
    /// The author declared the content to be ASN.1.
    Declared,
    /// Found by looking at the content; `warn` asks for `W202` when it is accepted.
    Guessed { warn: bool },
}

/// Text found in the document, before it is checked and turned into a [`Block`].
pub(crate) struct Candidate {
    pub text: String,
    pub map: SourceMap,
    pub origin: Origin,
}

/// Collects the blocks and findings of one extraction.
#[derive(Default)]
pub(crate) struct Sink {
    pub blocks: Vec<Block>,
    pub notes: Vec<ExtractNote>,
}

impl Sink {
    pub fn note(&mut self, code: &'static str, message: impl Into<String>, span: Range<usize>) {
        self.notes.push(ExtractNote::new(code, message, span));
    }

    /// Strips `<CODE BEGINS>` and `<CODE ENDS>` lines, then keeps the candidate if it is ASN.1.
    pub fn accept(&mut self, candidate: Candidate, marking: Marking) {
        let Candidate { text, map, origin } = candidate;
        let (text, map) = strip_marker_lines(text, map);
        let span = origin.span.clone();

        if text.trim().is_empty() {
            if marking == Marking::Declared {
                self.note("I200", "empty ASN.1 block skipped", span);
            }
            return;
        }
        let looks = looks_like_asn1(&text);
        let kind = block_kind(&text);
        let other_notation = looks && kind == BlockKind::Fragment && !is_inline_asn1(&text);
        match marking {
            Marking::Declared if !looks => {
                self.note(
                    "W201",
                    "block is typed as ASN.1 but does not look like ASN.1; skipped",
                    span,
                );
                return;
            }
            Marking::Declared if other_notation => {
                self.note(
                    "I201",
                    "block is typed as ASN.1 but is a grammar or SMI (check MIBs with smilint); skipped",
                    span,
                );
                return;
            }
            Marking::Guessed { .. } if !looks || other_notation => return,
            Marking::Guessed { warn: true } => self.note(
                "W202",
                "text looks like ASN.1 but is not marked as `sourcecode` of type `asn.1`",
                span,
            ),
            _ => {}
        }
        self.blocks.push(Block {
            text,
            kind,
            source_map: map,
            origin,
        });
    }
}

fn strip_marker_lines(text: String, map: SourceMap) -> (String, SourceMap) {
    let Some(marker) = MARKER_LINE.as_ref() else {
        return (text, map);
    };
    let lines = split_lines(&text);
    if !lines
        .iter()
        .any(|line| marker.is_match(line.content(&text)))
    {
        return (text, map);
    }
    let source = Mapped {
        text: &text,
        map: &map,
    };
    let mut assembler = Assembler::default();
    for line in &lines {
        if !marker.is_match(line.content(&text)) {
            assembler.push_slice(source, line.start..line.end);
        }
    }
    assembler.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(text: &str) -> Candidate {
        Candidate {
            text: text.to_owned(),
            map: SourceMap::identity(text.len()),
            origin: Origin {
                span: 0..text.len(),
                ..Origin::default()
            },
        }
    }

    #[test]
    fn marker_lines_are_removed_and_the_map_follows() {
        let source = "<CODE BEGINS>\nFoo ::= INTEGER\n<CODE ENDS>\n";
        let mut sink = Sink::default();
        sink.accept(candidate(source), Marking::Declared);
        let block = sink.blocks.first();
        assert_eq!(block.map(|b| b.text.as_str()), Some("Foo ::= INTEGER\n"));
        assert_eq!(block.and_then(|b| b.original_range(0..3)), Some(14..17));
        assert_eq!(block.map(|b| b.kind), Some(BlockKind::Fragment));
    }

    #[test]
    fn declared_non_asn1_is_reported_and_skipped() {
        let mut sink = Sink::default();
        sink.accept(candidate("30 82 01 0a"), Marking::Declared);
        assert!(sink.blocks.is_empty());
        assert_eq!(sink.notes.first().map(|n| n.code), Some("W201"));
    }

    #[test]
    fn guessed_asn1_warns_only_when_asked() {
        let mut sink = Sink::default();
        sink.accept(
            candidate("Foo ::= INTEGER"),
            Marking::Guessed { warn: true },
        );
        sink.accept(
            candidate("Bar ::= INTEGER"),
            Marking::Guessed { warn: false },
        );
        assert_eq!(sink.blocks.len(), 2);
        assert_eq!(sink.notes.len(), 1);
        assert_eq!(sink.notes.first().map(|n| n.code), Some("W202"));
    }
}
