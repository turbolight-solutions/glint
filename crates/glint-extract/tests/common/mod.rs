#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use glint_extract::{Block, BlockKind, Extraction, Format, extract};
use glint_syntax::lex;

/// What a character of an extracted block may stand for in the original document.
fn stands_for(original: &str, c: char) -> bool {
    if original.chars().eq([c]) {
        return true;
    }
    if c == '\n' && original == "\r\n" {
        return true;
    }
    let Some(reference) = original.strip_prefix('&').and_then(|r| r.strip_suffix(';')) else {
        return false;
    };
    match reference {
        "lt" => c == '<',
        "gt" => c == '>',
        "amp" => c == '&',
        "apos" => c == '\'',
        "quot" => c == '"',
        numeric => {
            numeric
                .strip_prefix('#')
                .and_then(|digits| match digits.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => digits.parse().ok(),
                })
                .and_then(char::from_u32)
                == Some(c)
        }
    }
}

/// Every character of every block must map to the same character (or the reference that stands for it) in the
/// original, in increasing order.
pub fn assert_round_trip(source: &str, extraction: &Extraction) {
    for block in &extraction.blocks {
        let mut previous_end = 0;
        for (offset, c) in block.text.char_indices() {
            let range = offset..offset + c.len_utf8();
            let original = block
                .original_range(range.clone())
                .unwrap_or_else(|| panic!("no mapping for offset {offset} in {:?}", block.origin));
            let slice = source.get(original.clone()).unwrap_or_else(|| {
                panic!("mapped range {original:?} is not on character boundaries")
            });
            assert!(
                stands_for(slice, c),
                "block char {c:?} at {offset} maps to {slice:?} at {original:?}"
            );
            assert!(
                original.start >= previous_end,
                "mapping goes backwards at {offset}"
            );
            previous_end = original.end;
        }
    }
}

pub fn run(source: &str, format: Format) -> Extraction {
    let extraction = extract(source, format);
    assert_round_trip(source, &extraction);
    extraction
}

pub fn modules(extraction: &Extraction) -> Vec<&Block> {
    extraction
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Module | BlockKind::ModuleList))
        .collect()
}

pub fn fragments(extraction: &Extraction) -> Vec<&Block> {
    extraction
        .blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Fragment)
        .collect()
}

pub fn codes(extraction: &Extraction) -> Vec<&'static str> {
    extraction.notes.iter().map(|n| n.code).collect()
}

/// The significant tokens of ASN.1 text, so two texts can be compared ignoring layout and comments.
pub fn tokens(text: &str) -> Vec<String> {
    lex(text)
        .0
        .into_iter()
        .filter(|t| !t.kind.is_trivia())
        .map(|t| text[usize::from(t.range.start())..usize::from(t.range.end())].to_owned())
        .collect()
}

pub fn module_names(blocks: &[&Block]) -> Vec<String> {
    blocks
        .iter()
        .flat_map(|b| glint_extract::module_slices(b))
        .map(|s| s.name.unwrap_or_default())
        .collect()
}

pub fn section_label(block: &Block) -> Option<String> {
    block.origin.section.as_ref().map(ToString::to_string)
}
