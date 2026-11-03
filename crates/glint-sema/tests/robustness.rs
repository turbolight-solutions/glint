//! The semantic checks must survive any text the parser accepts, including truncated and damaged modules.
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

fn fixtures() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rfc5912");
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect()
}

fn analyze_text(text: &str) {
    let empty = corpus(&[] as &[Rec<'_>]);
    let options = Options::default();
    let source = DraftSource {
        parse: parse_source_with(text, ParseOptions::corpus()),
        text: text.to_owned(),
    };
    let analysis = analyze(vec![source], &empty, &options);
    let _ = analysis.findings.len();
}

fn boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[test]
fn truncated_modules_do_not_panic() {
    for text in fixtures().iter().take(6) {
        let step = (text.len() / 40).max(1);
        let mut at = 0;
        while at < text.len() {
            analyze_text(&text[..boundary(text, at)]);
            at += step;
        }
    }
}

#[test]
fn damaged_modules_do_not_panic() {
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for text in fixtures().iter().take(8) {
        for _ in 0..12 {
            let mut damaged = text.clone();
            for _ in 0..6 {
                let start = boundary(&damaged, (next() as usize) % damaged.len().max(1));
                let end = boundary(&damaged, start + 1 + (next() as usize) % 12);
                damaged.replace_range(start..end, "");
            }
            analyze_text(&damaged);
        }
    }
}

#[test]
fn pathological_class_and_object_text_does_not_panic() {
    for text in [
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &a &a.&a } W ::= C O C ::= { } END",
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &a C OPTIONAL } WITH SYNTAX { [[[ &a ]]] ]] } o C ::= { [ } END",
        "M DEFINITIONS ::= BEGIN C ::= CLASS { & } WITH SYNTAX { } S C ::= { | | ... , ... } END",
        "M DEFINITIONS ::= BEGIN T {A, B : c, } ::= A U ::= T { { { } } } v T ::= T.&x.&y.&z END",
        "M DEFINITIONS ::= BEGIN A ::= B B ::= A c A ::= { c } d B ::= d.&x END",
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER } T ::= SEQUENCE { a C.&id ({S}{@}), b C.&id ({S}{@...}) } END",
    ] {
        analyze_text(text);
    }
}
