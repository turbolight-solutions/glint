#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let (tokens, _) = glint_syntax::lex(text);
    let rebuilt: String = tokens
        .iter()
        .map(|t| &text[usize::from(t.range.start())..usize::from(t.range.end())])
        .collect();
    assert_eq!(rebuilt, text);
});
