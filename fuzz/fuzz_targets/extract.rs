#![no_main]

use glint_extract::{Format, extract};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (u8, &str)| {
    let (selector, text) = input;
    let format = match selector % 4 {
        0 => Format::Xml,
        1 => Format::Markdown,
        2 => Format::Text,
        _ => Format::Bare,
    };
    let extraction = extract(text, format);
    for block in &extraction.blocks {
        for (offset, c) in block.text.char_indices() {
            let original = block
                .original_range(offset..offset + c.len_utf8())
                .expect("every character of a block maps back");
            assert!(text.get(original).is_some());
        }
    }
});
