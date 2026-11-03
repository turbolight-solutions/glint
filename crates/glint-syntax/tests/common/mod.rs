#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use glint_syntax::{Parse, SyntaxKind, parse_assignment_list, parse_source};

pub fn parse(src: &str) -> Parse {
    let parse = parse_source(src);
    assert_eq!(
        parse.syntax().text().to_string(),
        src,
        "tree is not lossless"
    );
    parse
}

pub fn parse_fragment(src: &str) -> Parse {
    let parse = parse_assignment_list(src);
    assert_eq!(
        parse.syntax().text().to_string(),
        src,
        "tree is not lossless"
    );
    parse
}

pub fn assert_clean(src: &str) -> Parse {
    let parse = parse(src);
    assert!(
        parse.errors().is_empty(),
        "unexpected errors in:\n{src}\n{:#?}",
        parse.errors()
    );
    assert!(
        parse
            .syntax()
            .descendants()
            .all(|n| n.kind() != SyntaxKind::ErrorNode),
        "error node in tree for:\n{src}"
    );
    parse
}

pub fn codes(src: &str) -> Vec<&'static str> {
    parse(src).errors().iter().map(|e| e.code).collect()
}

pub fn module(body: &str) -> String {
    format!("M DEFINITIONS ::= BEGIN\n{body}\nEND\n")
}

pub fn clean_module(body: &str) -> Parse {
    assert_clean(&module(body))
}

pub fn clean_type(ty: &str) -> Parse {
    clean_module(&format!("T ::= {ty}"))
}

pub fn clean_value(governor: &str, value: &str) -> Parse {
    clean_module(&format!("v {governor} ::= {value}"))
}

pub fn module_codes(body: &str) -> Vec<&'static str> {
    codes(&module(body))
}

pub fn count(parse: &Parse, kind: SyntaxKind) -> usize {
    parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == kind)
        .count()
}
