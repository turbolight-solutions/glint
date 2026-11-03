use std::fmt;
use std::ops::Range;

use crate::source_map::SourceMap;

/// What an extracted block contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// Exactly one module definition.
    Module,
    /// Several module definitions.
    ModuleList,
    /// Assignments or other ASN.1 text outside a module header.
    Fragment,
}

/// A section of the document, as the author would cite it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SectionRef {
    /// `4.1`, or `A.1` for an appendix.
    pub number: String,
    pub appendix: bool,
}

impl fmt::Display for SectionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = if self.appendix { "Appendix" } else { "Section" };
        write!(f, "{word} {}", self.number)
    }
}

/// Where a block came from in the original document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Origin {
    pub section: Option<SectionRef>,
    pub title: Option<String>,
    /// Byte range of the whole block in the original document.
    pub span: Range<usize>,
}

/// One piece of ASN.1 pulled out of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub text: String,
    pub kind: BlockKind,
    pub source_map: SourceMap,
    pub origin: Origin,
}

impl Block {
    /// Translates a byte range of [`Block::text`] to a byte range of the original document.
    pub fn original_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        self.source_map.map_range(range)
    }
}
