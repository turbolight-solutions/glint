#![no_main]

use glint_syntax::ParseOptions;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    for options in [ParseOptions::default(), ParseOptions::corpus()] {
        let parse = glint_syntax::parse_source_with(text, options);
        assert_eq!(parse.syntax().text().to_string(), text);
    }
    let parse = glint_syntax::parse_assignment_list(text);
    assert_eq!(parse.syntax().text().to_string(), text);
});
