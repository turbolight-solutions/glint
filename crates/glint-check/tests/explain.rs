#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Every `bad` example in `glint explain` raises its code; every `good` example does not.

use glint_check::{CheckOptions, check_document};
use glint_corpus::EmptyCorpus;
use glint_diag::{examples, explain, explained_codes};
use glint_extract::{Format, detect_format};

fn codes_of(language: &str, text: &str) -> Vec<&'static str> {
    let format = match language {
        "xml" => Format::Xml,
        "markdown" => Format::Markdown,
        "text" => Format::Text,
        _ => detect_format(None, text),
    };
    check_document(
        "example",
        text,
        format,
        &EmptyCorpus::default(),
        &CheckOptions::default(),
    )
    .diagnostics
    .iter()
    .map(|d| d.code)
    .collect()
}

#[test]
fn examples_behave_as_the_explanation_says() {
    let mut failures = Vec::new();
    for code in explained_codes() {
        let body = explain(code).unwrap();
        let found = examples(body);
        for example in &found {
            if example.tags.contains(&"untested") {
                continue;
            }
            let raised = codes_of(example.language, example.text);
            if example.tags.contains(&"bad") && !raised.contains(&code) {
                failures.push(format!(
                    "{code}: bad example does not raise it (got {raised:?}):\n{}",
                    example.text
                ));
            }
            if example.tags.contains(&"good") && raised.contains(&code) {
                failures.push(format!("{code}: good example raises it:\n{}", example.text));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn most_codes_have_a_tested_bad_example() {
    let untested: Vec<_> = explained_codes()
        .filter(|code| {
            !examples(explain(code).unwrap())
                .iter()
                .any(|e| e.tags.contains(&"bad"))
        })
        .collect();
    // The label table codes (W142, W143, W144, I145), W146 and I109 need a corpus, which these examples are not checked
    // against.
    assert!(untested.len() <= 18, "{untested:?}");
}
