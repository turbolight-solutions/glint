use std::ops::Range;

use glint_syntax::{ParseOptions, ast, parse_source_with};

use crate::block::Block;

/// One module definition inside a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleSlice {
    pub name: Option<String>,
    /// Byte range of the definition within [`Block::text`].
    pub range: Range<usize>,
}

/// Locates the module definitions in `block`, in order. A block of kind `Fragment` has none.
pub fn module_slices(block: &Block) -> Vec<ModuleSlice> {
    let parse = parse_source_with(&block.text, ParseOptions::corpus());
    let Some(file) = ast::SourceFile::cast(parse.syntax()) else {
        return Vec::new();
    };
    file.modules()
        .map(|module| {
            let range = module.syntax().text_range();
            ModuleSlice {
                name: module.name().map(|token| token.text().to_owned()),
                range: usize::from(range.start())..usize::from(range.end()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Format, extract};

    #[test]
    fn a_module_list_is_split_by_module() {
        let source = "A DEFINITIONS ::= BEGIN END\n\nB DEFINITIONS ::= BEGIN END\n";
        let extraction = extract(source, Format::Bare);
        let slices = extraction
            .blocks
            .first()
            .map(module_slices)
            .unwrap_or_default();
        let names: Vec<_> = slices.iter().map(|s| s.name.as_deref()).collect();
        assert_eq!(names, [Some("A"), Some("B")]);
    }
}
