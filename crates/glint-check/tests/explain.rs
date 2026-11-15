#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

//! Every `bad` example in `glint explain` raises its code; every `good` example does not.

use glint_check::{CheckOptions, check_document, lower_user_type};
use glint_corpus::EmptyCorpus;
use glint_decode::{NoOracle, decode};
use glint_diag::{Example, examples, explain, explained_codes};
use glint_extract::{Format, detect_format};

/// The codes `glint decode` raises for a `der` example: decoded as the type its tags name (`M.T`) of the section's
/// `asn1 schema` module, or without a schema.
fn decoder_codes(example: &Example<'_>, found: &[Example<'_>]) -> Vec<&'static str> {
    let hex: String = example.text.split_whitespace().collect();
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let set = example.tags.iter().find(|t| t.contains('.')).map(|root| {
        let (module, ty) = root.split_once('.').unwrap();
        let schema = found
            .iter()
            .find(|e| e.language == "asn1" && e.tags.contains(&"schema"))
            .expect("a `der` example that names a type needs an `asn1 schema` example");
        lower_user_type(
            &[schema.text.to_owned()],
            &EmptyCorpus::default(),
            module,
            ty,
        )
        .unwrap()
    });
    let decoded = decode(&bytes, set.as_ref().map(|s| (s, s.root)), &NoOracle);
    decoded.all_findings().iter().map(|f| f.code).collect()
}

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
            // `codegen` examples are run through the code generators, in glint-codegen's tests
            if example.tags.contains(&"untested") || example.tags.contains(&"codegen") {
                continue;
            }
            let raised = if example.language == "der" {
                decoder_codes(example, &found)
            } else {
                codes_of(example.language, example.text)
            };
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
