mod common;
use common::*;

fn run(body: &str) -> Vec<&'static str> {
    codes(&check(&format!("M DEFINITIONS ::= BEGIN {body} END")))
}

// X.680 25.12, C.6
#[test]
fn matching_values_are_legal() {
    let body = "T ::= SEQUENCE {
        a INTEGER DEFAULT 5, b BOOLEAN DEFAULT TRUE, c OCTET STRING DEFAULT '00'H,
        d IA5String DEFAULT \"x\", e BIT STRING DEFAULT '1'B, f NULL DEFAULT NULL }";
    assert!(run(body).is_empty());
}

#[test]
fn boolean_default_that_is_a_number() {
    assert_eq!(run("T ::= SEQUENCE { a BOOLEAN DEFAULT 1 }"), ["E131"]);
}

#[test]
fn integer_default_that_is_a_string() {
    assert_eq!(run("T ::= SEQUENCE { a INTEGER DEFAULT \"x\" }"), ["E131"]);
}

#[test]
fn character_string_default_that_is_a_number() {
    assert_eq!(run("T ::= SEQUENCE { a IA5String DEFAULT 5 }"), ["E131"]);
}

#[test]
fn enumerated_default_with_an_unknown_item() {
    assert!(
        run("E ::= ENUMERATED { red, green } T ::= SEQUENCE { a E DEFAULT blue }")
            .contains(&"E110")
    );
}

#[test]
fn integer_value_assignment_with_the_wrong_kind_of_value() {
    assert_eq!(run("v INTEGER ::= TRUE"), ["E131"]);
}

#[test]
fn value_reference_of_the_wrong_type() {
    assert_eq!(run("b BOOLEAN ::= TRUE n INTEGER ::= b"), ["E131"]);
}

#[test]
fn value_reference_of_the_right_type() {
    assert!(run("a INTEGER ::= 3 n INTEGER ::= a").is_empty());
}

#[test]
fn sequence_value_with_known_components() {
    let body = "T ::= SEQUENCE { a INTEGER, b BOOLEAN OPTIONAL } v T ::= { a 1, b TRUE }";
    assert!(run(body).is_empty());
}

#[test]
fn sequence_value_with_an_unknown_component() {
    let body = "T ::= SEQUENCE { a INTEGER } v T ::= { a 1, z 2 }";
    assert_eq!(run(body), ["E131"]);
}

#[test]
fn sequence_value_missing_a_mandatory_component() {
    let body = "T ::= SEQUENCE { a INTEGER, b BOOLEAN } v T ::= { a 1 }";
    assert_eq!(run(body), ["E131"]);
}

#[test]
fn sequence_value_component_of_the_wrong_type() {
    let body = "T ::= SEQUENCE { a INTEGER } v T ::= { a TRUE }";
    assert_eq!(run(body), ["E131"]);
}

#[test]
fn choice_value() {
    let body = "C ::= CHOICE { a INTEGER, b BOOLEAN } v C ::= b : TRUE";
    assert!(run(body).is_empty());
    let bad = "C ::= CHOICE { a INTEGER, b BOOLEAN } v C ::= z : TRUE";
    assert_eq!(run(bad), ["E131"]);
}

#[test]
fn sequence_of_value() {
    let body = "T ::= SEQUENCE OF INTEGER v T ::= { 1, 2, 3 }";
    assert!(run(body).is_empty());
    assert_eq!(
        run("T ::= SEQUENCE OF INTEGER v T ::= { 1, TRUE }"),
        ["E131"]
    );
}

// X.680 50.7
#[test]
fn inverted_range() {
    assert_eq!(run("T ::= INTEGER (5..1)"), ["E132"]);
}

#[test]
fn empty_exclusive_range() {
    assert_eq!(run("T ::= INTEGER (1<..<2)"), ["E132"]);
}

#[test]
fn legal_ranges() {
    assert!(run("T ::= INTEGER (1..5 | 10..MAX) U ::= INTEGER (MIN..0)").is_empty());
}

// X.680 50.10, 50.11
#[test]
fn value_outside_the_parent_type() {
    assert_eq!(run("Foo ::= INTEGER (1..10) Bar ::= Foo (20)"), ["E132"]);
}

#[test]
fn value_inside_the_parent_type() {
    assert!(run("Foo ::= INTEGER (1..10) Bar ::= Foo (3..4)").is_empty());
}

// X.680 50.11 example
#[test]
fn extension_additions_of_the_parent_are_not_in_its_root() {
    assert_eq!(
        run("Foo ::= INTEGER (1..6, ..., 73..80) Bar ::= Foo (73)"),
        ["E132"]
    );
}

#[test]
fn size_bound_cannot_be_negative() {
    assert_eq!(run("T ::= OCTET STRING (SIZE (-1..4))"), ["E132"]);
}

#[test]
fn inverted_size_range() {
    assert_eq!(run("T ::= SEQUENCE (SIZE (5..2)) OF INTEGER"), ["E132"]);
}

// X.680 51.1 Tables 11-12
#[test]
fn size_constraint_on_an_integer() {
    assert_eq!(run("T ::= INTEGER (SIZE (1..4))"), ["E133"]);
}

#[test]
fn value_range_on_an_octet_string() {
    assert_eq!(run("T ::= OCTET STRING (1..4)"), ["E133"]);
}

#[test]
fn permitted_alphabet_on_an_integer() {
    assert_eq!(run("T ::= INTEGER (FROM (\"a\"))"), ["E133"]);
}

#[test]
fn applicable_constraints_are_legal() {
    let body =
        "A ::= OCTET STRING (SIZE (1..4)) B ::= IA5String (SIZE (1..8) ^ FROM (\"a\"..\"z\"))
        C ::= SEQUENCE (SIZE (1..MAX)) OF INTEGER D ::= BIT STRING (SIZE (8))";
    assert!(run(body).is_empty());
}

#[test]
fn constraint_through_a_type_reference() {
    assert_eq!(
        run("Base ::= INTEGER Derived ::= Base (SIZE (1..2))"),
        ["E133"]
    );
}

// X.680 49.6
#[test]
fn exception_without_an_extension_marker() {
    assert_eq!(run("T ::= INTEGER (1..5 ! 7)"), ["E135"]);
}

#[test]
fn exception_with_an_extension_marker() {
    assert!(run("T ::= INTEGER (1..5, ... ! 7)").is_empty());
}
