mod common;
use common::*;
use glint_sema::Options;

fn run(text: &str) -> Vec<&'static str> {
    codes(&without_notes(check(text)))
}

fn two(a: &str, b: &str) -> Vec<&'static str> {
    let corpus = corpus(&[Rec {
        name: "A",
        oid: None,
        rfc: 1,
        text: a,
    }]);
    codes(&without_notes(check_with(b, &corpus, &Options::default())))
}

// X.680 14.4
#[test]
fn undefined_type_reference() {
    assert_eq!(
        run("M DEFINITIONS ::= BEGIN T ::= SEQUENCE { a Nope } END"),
        ["E110"]
    );
}

#[test]
fn defined_type_reference_is_fine() {
    assert!(run("M DEFINITIONS ::= BEGIN T ::= SEQUENCE { a U } U ::= INTEGER END").is_empty());
}

#[test]
fn undefined_value_reference_in_a_default() {
    assert_eq!(
        run("M DEFINITIONS ::= BEGIN T ::= SEQUENCE { a INTEGER DEFAULT nope } END"),
        ["E110"]
    );
}

#[test]
fn named_number_in_a_default_is_not_a_reference() {
    let text = "M DEFINITIONS ::= BEGIN
        V ::= INTEGER { one(1), two(2) }
        T ::= SEQUENCE { a V DEFAULT one } END";
    assert!(run(text).is_empty());
}

#[test]
fn enumeration_item_in_a_default_is_not_a_reference() {
    let text = "M DEFINITIONS ::= BEGIN
        V ::= ENUMERATED { red, green }
        T ::= SEQUENCE { a V DEFAULT red } END";
    assert!(run(text).is_empty());
}

#[test]
fn dummy_references_are_in_scope_inside_parameterized_assignments() {
    let text = "M DEFINITIONS ::= BEGIN Pair{Elem} ::= SEQUENCE { a Elem, b Elem } END";
    assert!(run(text).is_empty());
}

// X.680 14.7: external references need the module in IMPORTS
#[test]
fn external_reference_to_a_module_not_in_imports() {
    assert_eq!(
        run("M DEFINITIONS ::= BEGIN T ::= SEQUENCE { a Other.Thing } END"),
        ["E110"]
    );
}

// X.680 13.16 c
#[test]
fn external_reference_to_a_symbol_not_imported_from_that_module() {
    let a = "A DEFINITIONS ::= BEGIN X ::= INTEGER Y ::= BOOLEAN END";
    let b = "M DEFINITIONS ::= BEGIN IMPORTS X FROM A; T ::= SEQUENCE { a A.Y, b X } END";
    assert_eq!(two(a, b), ["E110"]);
}

#[test]
fn external_reference_to_an_imported_symbol_is_fine() {
    let a = "A DEFINITIONS ::= BEGIN X ::= INTEGER END";
    let b = "M DEFINITIONS ::= BEGIN IMPORTS X FROM A; T ::= SEQUENCE { a A.X } END";
    assert!(two(a, b).iter().all(|c| *c != "E110"));
}

// X.680 13.21
#[test]
fn two_assignments_with_the_same_name() {
    assert_eq!(
        run("M DEFINITIONS ::= BEGIN T ::= INTEGER T ::= BOOLEAN END"),
        ["E111"]
    );
}

// X.683 8.2
#[test]
fn parameterized_and_plain_assignments_share_the_name_space() {
    assert_eq!(
        run("M DEFINITIONS ::= BEGIN T ::= INTEGER T{X} ::= SEQUENCE { a X } END"),
        ["E111"]
    );
}

#[test]
fn unused_import() {
    let a = "A DEFINITIONS ::= BEGIN X ::= INTEGER Y ::= BOOLEAN END";
    let b = "M DEFINITIONS ::= BEGIN IMPORTS X, Y FROM A; T ::= SEQUENCE { a X } END";
    assert_eq!(two(a, b), ["W112"]);
}

// X.680 13.20
#[test]
fn symbol_imported_from_two_modules_must_be_qualified() {
    let a = "A DEFINITIONS ::= BEGIN X ::= INTEGER END";
    let b = "B DEFINITIONS ::= BEGIN X ::= BOOLEAN END";
    let corpus = corpus(&[
        Rec {
            name: "A",
            oid: None,
            rfc: 1,
            text: a,
        },
        Rec {
            name: "B",
            oid: None,
            rfc: 2,
            text: b,
        },
    ]);
    let unqualified =
        "M DEFINITIONS ::= BEGIN IMPORTS X FROM A X FROM B; T ::= SEQUENCE { a X } END";
    let findings = without_notes(check_with(unqualified, &corpus, &Options::default()));
    assert_eq!(codes(&findings), ["E114"]);
    let qualified =
        "M DEFINITIONS ::= BEGIN IMPORTS X FROM A X FROM B; T ::= SEQUENCE { a A.X, b B.X } END";
    let findings = without_notes(check_with(qualified, &corpus, &Options::default()));
    assert!(findings.is_empty(), "{:?}", codes(&findings));
}

#[test]
fn symbol_defined_locally_and_imported_must_be_qualified() {
    let a = "A DEFINITIONS ::= BEGIN X ::= INTEGER END";
    let b = "M DEFINITIONS ::= BEGIN IMPORTS X FROM A; X ::= BOOLEAN T ::= SEQUENCE { a X } END";
    assert_eq!(two(a, b), ["E114"]);
}

#[test]
fn macro_notation_assignments_are_not_checked() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS OBJECT-TYPE FROM RFC-1212;
        x OBJECT-TYPE SYNTAX Mystery ACCESS read-only ::= { y 1 }
        END";
    let findings = without_notes(check(text));
    assert!(
        findings.iter().all(|f| f.code != "E110"),
        "{:?}",
        codes(&findings)
    );
}
