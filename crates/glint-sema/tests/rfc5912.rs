//! M4 acceptance: the RFC 5912 modules parse and check clean when read together as one document.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use std::path::Path;

use common::{Rec, corpus};
use glint_sema::{DraftSource, Options, analyze};
use glint_syntax::{ParseOptions, parse_source_with};

fn modules() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rfc5912");
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| {
            (
                p.file_stem().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn rfc_5912_modules_parse_without_syntax_errors() {
    for (name, text) in modules() {
        let parse = parse_source_with(&text, ParseOptions::corpus());
        let errors: Vec<_> = parse
            .errors()
            .iter()
            .filter(|e| e.severity == glint_syntax::Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
    }
}

#[test]
fn rfc_5912_modules_check_with_no_findings_besides_unused_imports() {
    let sources: Vec<DraftSource> = modules()
        .into_iter()
        .map(|(_, text)| DraftSource {
            parse: parse_source_with(&text, ParseOptions::corpus()),
            text,
        })
        .collect();
    let options = Options {
        oid_collisions: false,
        policy_checks: false,
        resolution_notes: false,
        ..Options::default()
    };
    let empty = corpus(&[] as &[Rec<'_>]);
    let analysis = analyze(sources, &empty, &options);
    let unexpected: Vec<String> = analysis
        .findings
        .iter()
        .filter(|f| f.code != "W112" && f.code != "W100" && f.code != "W102")
        .map(|f| format!("{} {}", f.code, f.message))
        .collect();
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}
