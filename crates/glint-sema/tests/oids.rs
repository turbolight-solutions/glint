#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod common;
use common::*;
use glint_corpus::{Bundle, Corpus, OidAssignment};
use glint_sema::Options;

fn run(text: &str) -> Vec<&'static str> {
    codes(&check(text))
}

fn module(body: &str) -> String {
    format!("M DEFINITIONS ::= BEGIN {body} END")
}

// X.660 via X.680 32.11
#[test]
fn oid_with_one_arc_is_an_error() {
    assert_eq!(run(&module("o OBJECT IDENTIFIER ::= { 1 }")), ["E130"]);
}

#[test]
fn first_arc_above_two_is_an_error() {
    assert_eq!(run(&module("o OBJECT IDENTIFIER ::= { 3 1 }")), ["E130"]);
}

#[test]
fn second_arc_of_forty_under_iso_is_an_error() {
    assert_eq!(run(&module("o OBJECT IDENTIFIER ::= { 1 40 }")), ["E130"]);
}

#[test]
fn large_second_arc_under_joint_is_legal() {
    assert!(run(&module("o OBJECT IDENTIFIER ::= { 2 999 }")).is_empty());
}

#[test]
fn registered_names_are_legal_forms() {
    let text = module(
        "o OBJECT IDENTIFIER ::= { iso identified-organization 6 1 }
         p OBJECT IDENTIFIER ::= { joint-iso-itu-t 5 }
         q OBJECT IDENTIFIER ::= { iso(1) member-body(2) us(840) 1 }",
    );
    assert!(run(&text).is_empty());
}

// X.680 32.7
#[test]
fn bare_name_that_x660_does_not_register() {
    assert_eq!(
        run(&module(
            "o OBJECT IDENTIFIER ::= { iso identified-organization dod 1 }"
        )),
        ["E134"]
    );
}

#[test]
fn bare_name_registered_elsewhere_is_not_registered_here() {
    assert_eq!(
        run(&module("o OBJECT IDENTIFIER ::= { iso recommendation 1 }")),
        ["E134"]
    );
}

#[test]
fn oid_built_on_a_local_value() {
    let text = module(
        "base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 }
         child OBJECT IDENTIFIER ::= { base 5 }
         leaf OBJECT IDENTIFIER ::= { child leafarc(2) }",
    );
    assert!(run(&text).is_empty());
}

#[test]
fn undefined_parent_value() {
    assert_eq!(
        run(&module("o OBJECT IDENTIFIER ::= { missing 5 }")),
        ["E110"]
    );
}

#[test]
fn integer_value_in_the_first_position_is_not_an_oid() {
    let text = module("n INTEGER ::= 5 o OBJECT IDENTIFIER ::= { n 5 }");
    assert_eq!(run(&text), ["E131"]);
}

#[test]
fn number_form_can_be_an_integer_value() {
    let text = module("n INTEGER ::= 5 o OBJECT IDENTIFIER ::= { 1 3 big(n) }");
    assert!(run(&text).is_empty());
}

// I141
#[test]
fn placeholder_arcs_are_noted() {
    let findings = check(&module(
        "base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 } o OBJECT IDENTIFIER ::= { base id-TBD }",
    ));
    assert!(has(&findings, "I141"), "{:?}", codes(&findings));
    assert!(!has(&findings, "E110"));
}

fn corpus_with_assignment(oid: &str, name: &str) -> Corpus {
    Corpus::from_bundle(Bundle {
        oid_assignments: vec![OidAssignment {
            oid: oid.parse().unwrap(),
            name: name.to_owned(),
            module: "PKIX1Explicit88".to_owned(),
            document: "rfc5280".to_owned(),
        }],
        ..Bundle::default()
    })
}

// W140
#[test]
fn oid_already_assigned_in_the_corpus_to_another_name() {
    let corpus = corpus_with_assignment("1.3.6.1.5.5.7.1.1", "id-pe-authorityInfoAccess");
    let text = module(
        "id-pkix OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7) }
         id-pe-mine OBJECT IDENTIFIER ::= { id-pkix 1 1 }",
    );
    let findings = check_with(&text, &corpus, &Options::default());
    assert_eq!(codes(&findings), ["W140"]);
    assert!(findings[0].message.contains("RFC 5280"));
}

#[test]
fn same_name_for_the_same_oid_is_not_a_collision() {
    let corpus = corpus_with_assignment("1.3.6.1.5.5.7.1.1", "id-pe-authorityInfoAccess");
    let text = module(
        "id-pkix OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7) }
         id-pe-authorityInfoAccess OBJECT IDENTIFIER ::= { id-pkix 1 1 }",
    );
    assert!(check_with(&text, &corpus, &Options::default()).is_empty());
}

#[test]
fn collisions_are_not_checked_for_corpus_modules() {
    let corpus = corpus_with_assignment("1.3.6.1.5.5.7.1.1", "id-pe-authorityInfoAccess");
    let text = module("o OBJECT IDENTIFIER ::= { 1 3 6 1 5 5 7 1 1 }");
    let options = Options {
        oid_collisions: false,
        ..Options::default()
    };
    assert!(check_with(&text, &corpus, &options).is_empty());
}

#[test]
fn module_identifier_already_held_by_another_module() {
    let corpus = corpus(&[Rec {
        name: "Existing",
        oid: Some("1.3.6.1.4.1.99.1"),
        rfc: 1,
        text: "Existing { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN END",
    }]);
    let text = "Mine { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN END";
    let findings = check_with(text, &corpus, &Options::default());
    assert_eq!(codes(&findings), ["W140"]);
}

#[test]
fn oid_in_a_module_header_is_validated() {
    assert_eq!(run("Mine { 1 } DEFINITIONS ::= BEGIN END"), ["E130"]);
}
