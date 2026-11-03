mod common;
use common::*;
use glint_syntax::{ParseOptions, PrefixKind, SyntaxKind, parse_prefix};

fn dump(src: &str) -> String {
    format!("{:#?}", assert_clean(src).syntax())
}

fn kinds(src: &str, kind: SyntaxKind) -> usize {
    assert_clean(src)
        .syntax()
        .descendants()
        .filter(|n| n.kind() == kind)
        .count()
}

// X.681 9.3-9.12
#[test]
fn class_definition_with_all_field_shapes() {
    insta::assert_snapshot!(dump(&module(
        "C ::= CLASS {
           &Type OPTIONAL,
           &Other DEFAULT INTEGER,
           &fixed INTEGER UNIQUE,
           &withDefault BOOLEAN DEFAULT TRUE,
           &variable &Type OPTIONAL,
           &Set INTEGER DEFAULT { 1 | 2 },
           &obj D OPTIONAL,
           &Objs D DEFAULT { a },
           &chain &obj.&Type
         } WITH SYNTAX { ID &fixed [TYPE &Type] }"
    )));
}

#[test]
fn every_field_spec_is_a_node() {
    assert_eq!(
        kinds(
            &module("C ::= CLASS { &a INTEGER, &B, &c &B OPTIONAL, &D E DEFAULT { x } }"),
            SyntaxKind::FieldSpec
        ),
        4
    );
}

#[test]
fn class_body_errors_are_reported_and_recovered() {
    let src = module("C ::= CLASS { &a INTEGER, ???, &b BOOLEAN }\nD ::= INTEGER");
    let parse = parse(&src);
    assert!(!parse.errors().is_empty());
    assert_eq!(
        parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::TypeAssignment)
            .count(),
        2
    );
}

// X.681 11.7, X.683 9.5: a setting or an actual parameter is read once its kind is known
#[test]
fn parse_prefix_reads_a_type_and_says_where_it_ends() {
    let text = "xxxxxxxx PARAMS TYPE NULL ARE required }";
    let start = text.find("NULL").unwrap();
    let limit = text.find('}').unwrap();
    let result = parse_prefix(
        text,
        start,
        limit,
        PrefixKind::Type,
        ParseOptions::default(),
    );
    assert!(result.ok);
    assert_eq!(usize::from(result.end), start + "NULL".len());
}

#[test]
fn parse_prefix_keeps_original_offsets() {
    let text = "{ a INTEGER (1..5) b }";
    let start = text.find("INTEGER").unwrap();
    let result = parse_prefix(
        text,
        start,
        text.len().saturating_sub(1),
        PrefixKind::Type,
        ParseOptions::default(),
    );
    assert!(result.ok);
    assert_eq!(usize::from(result.end), text.find(" b").unwrap());
    let node = result.parse.syntax();
    let first = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.text() == "INTEGER")
        .unwrap();
    assert_eq!(usize::from(first.text_range().start()), start);
}

#[test]
fn parse_prefix_reads_a_value() {
    let text = "IDENTIFIED BY id-x PARAMS";
    let start = text.find("id-x").unwrap();
    let result = parse_prefix(
        text,
        start,
        text.len(),
        PrefixKind::Value,
        ParseOptions::default(),
    );
    assert!(result.ok);
    assert_eq!(usize::from(result.end), start + "id-x".len());
}

#[test]
fn parse_prefix_reports_a_bad_type() {
    let text = "}";
    let result = parse_prefix(
        text,
        0,
        text.len(),
        PrefixKind::Type,
        ParseOptions::default(),
    );
    assert!(!result.ok);
}
