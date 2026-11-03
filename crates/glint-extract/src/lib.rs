//! Extracts ASN.1 from Internet-Draft and RFC sources.
//!
//! Every supported format (RFCXML v2 and v3, kramdown-rfc and mmark Markdown, plain text, and bare `.asn` files)
//! yields a list of [`Block`]s. Each block carries a [`SourceMap`] that translates offsets in the block back to
//! offsets in the original document, so later stages can report positions the author recognizes.
//!
//! The crate does no I/O: callers pass the document text.

mod block;
mod classify;
mod lines;
mod markdown;
mod modules;
mod notes;
mod sink;
mod source_map;
mod text;
mod xml;

use std::ops::Range;
use std::path::Path;

pub use block::{Block, BlockKind, Origin, SectionRef};
pub use classify::{block_kind, looks_like_asn1};
pub use modules::{ModuleSlice, module_slices};
pub use notes::{ExtractNote, Severity};
pub use source_map::{Assembler, Mapped, SourceMap};

use sink::{Candidate, Marking, Sink};

/// The kind of document being extracted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// RFCXML v2 or v3.
    Xml,
    /// kramdown-rfc or mmark Markdown.
    Markdown,
    /// A plain-text draft or RFC.
    Text,
    /// A file that is ASN.1 and nothing else.
    Bare,
}

/// The blocks and findings from one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    pub format: Format,
    pub blocks: Vec<Block>,
    pub notes: Vec<ExtractNote>,
}

/// Extracts the ASN.1 blocks of `source`, read as `format`.
pub fn extract(source: &str, format: Format) -> Extraction {
    let mut sink = Sink::default();
    match format {
        Format::Xml => xml::extract(source, &mut sink),
        Format::Markdown => markdown::extract(source, &mut sink),
        Format::Text => text::extract(source, &mut sink),
        Format::Bare => extract_bare(source, &mut sink),
    }
    Extraction {
        format,
        blocks: sink.blocks,
        notes: sink.notes,
    }
}

fn extract_bare(source: &str, sink: &mut Sink) {
    let candidate = Candidate {
        text: source.to_owned(),
        map: SourceMap::identity(source.len()),
        origin: Origin {
            span: Range {
                start: 0,
                end: source.len(),
            },
            ..Origin::default()
        },
    };
    sink.accept(candidate, Marking::Declared);
}

/// Chooses a [`Format`] from the file name's extension when it is conclusive, otherwise from the content.
pub fn detect_format(name: Option<&Path>, source: &str) -> Format {
    let extension = name
        .and_then(Path::extension)
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("xml") => return Format::Xml,
        Some("md" | "markdown" | "mkd" | "mdown") => return Format::Markdown,
        Some("asn" | "asn1") => return Format::Bare,
        Some("txt") => return Format::Text,
        _ => {}
    }
    sniff(source)
}

fn sniff(source: &str) -> Format {
    let head = source.trim_start_matches('\u{FEFF}').trim_start();
    if head.starts_with("<?xml") || head.starts_with("<rfc") || head.starts_with("<!DOCTYPE rfc") {
        return Format::Xml;
    }
    if source.contains('\u{0C}') {
        return Format::Text;
    }
    if head.starts_with("---\n")
        || head.starts_with("---\r\n")
        || source.lines().any(|line| {
            let line = line.trim_end();
            line.starts_with("```") || line.starts_with("~~~") || line.starts_with("# ")
        })
    {
        return Format::Markdown;
    }
    let first_line_is_code = source
        .lines()
        .find(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--")
        })
        .is_some_and(|line| !line.starts_with([' ', '\t']));
    if first_line_is_code
        && looks_like_asn1(source)
        && !source.lines().any(|l| l.contains("[Page "))
    {
        Format::Bare
    } else {
        Format::Text
    }
}
