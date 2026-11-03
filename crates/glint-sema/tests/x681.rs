mod common;

use common::{check, codes, has};

const PRELUDE: &str = "ATTR ::= CLASS {
  &id OBJECT IDENTIFIER UNIQUE,
  &Type OPTIONAL,
  &count INTEGER DEFAULT 1
} WITH SYNTAX { [TYPE &Type] [COUNT &count] IDENTIFIED BY &id }
";

fn module(body: &str) -> String {
    format!("M DEFINITIONS EXPLICIT TAGS ::= BEGIN\n{PRELUDE}\n{body}\nEND")
}

fn run(body: &str) -> Vec<&'static str> {
    codes(&check(&module(body)))
}

fn clean(body: &str) {
    let found = run(body);
    assert!(found.is_empty(), "unexpected findings: {found:?}");
}

// X.681 9.13
#[test]
fn e153_duplicate_field_names() {
    let found = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER, &id BOOLEAN } END",
    ));
    assert!(found.contains(&"E153"));
}

// X.681 9.6
#[test]
fn e153_unique_with_default() {
    let bad = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER UNIQUE DEFAULT 1 } END",
    ));
    assert!(bad.contains(&"E153"));
    let good = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER UNIQUE } END",
    ));
    assert!(!good.contains(&"E153"));
}

// X.681 9.8 a, b
#[test]
fn e153_variable_type_field_optionality() {
    let bad = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &T OPTIONAL, &v &T } END",
    ));
    assert!(bad.contains(&"E153"));
    let good = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &T OPTIONAL, &v &T OPTIONAL } END",
    ));
    assert!(!good.contains(&"E153"));
}

// X.681 9.8: the FieldName must name a type field
#[test]
fn e153_variable_type_field_must_name_a_type_field() {
    let bad = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &a INTEGER, &v &a } END",
    ));
    assert!(bad.contains(&"E153") || bad.contains(&"E001"));
}

// X.681 9.15
#[test]
fn e153_recursive_class_needs_an_optional_link() {
    let bad = codes(&check(
        "M DEFINITIONS ::= BEGIN OP ::= CLASS { &Linked OP, &code INTEGER UNIQUE } END",
    ));
    assert!(bad.contains(&"E153"));
    let good = codes(&check(
        "M DEFINITIONS ::= BEGIN OP ::= CLASS { &Linked OP OPTIONAL, &code INTEGER UNIQUE } END",
    ));
    assert!(!good.contains(&"E153"));
}

// X.681 9.16 and 10.13: the OPERATION and ERROR classes
#[test]
fn x681_9_16_and_11_11_examples_are_clean() {
    clean(
        "OPERATION ::= CLASS {
   &ArgumentType OPTIONAL,
   &ResultType OPTIONAL,
   &Errors ERROR OPTIONAL,
   &Linked OPERATION OPTIONAL,
   &resultReturned BOOLEAN DEFAULT TRUE,
   &operationCode INTEGER UNIQUE
} WITH SYNTAX {
   [ARGUMENT &ArgumentType]
   [RESULT &ResultType]
   [RETURN RESULT &resultReturned]
   [ERRORS &Errors]
   [LINKED &Linked]
   CODE &operationCode
}
ERROR ::= CLASS {
   &ParameterType OPTIONAL,
   &errorCode INTEGER UNIQUE
} WITH SYNTAX { [PARAMETER &ParameterType] CODE &errorCode }
Matrix ::= SEQUENCE OF INTEGER
invertMatrix OPERATION ::= {
   ARGUMENT Matrix
   RESULT Matrix
   ERRORS {determinantIsZero}
   CODE 7
}
determinantIsZero ERROR ::= { CODE 1 }
addMatrices OPERATION ::= { ARGUMENT Matrix RESULT Matrix CODE 8 }
MatrixOperations OPERATION ::= { invertMatrix | addMatrices }",
    );
}

// X.681 11.10: default syntax
#[test]
fn x681_11_10_default_syntax_example_is_clean() {
    let text = "M DEFINITIONS EXPLICIT TAGS ::= BEGIN
OPERATION ::= CLASS {
   &ArgumentType OPTIONAL,
   &Errors ERROR OPTIONAL,
   &operationCode INTEGER UNIQUE
}
ERROR ::= CLASS { &errorCode INTEGER UNIQUE }
Matrix ::= SEQUENCE OF INTEGER
invertMatrix OPERATION ::= {
   &ArgumentType Matrix,
   &Errors {determinantIsZero},
   &operationCode 7
}
determinantIsZero ERROR ::= { &errorCode 1 }
END";
    assert!(codes(&check(text)).is_empty());
}

// X.681 10.9, 10.12 a
#[test]
fn e154_syntax_list_rules() {
    let missing = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER, &T } WITH SYNTAX { ID &id } END",
    ));
    assert!(missing.contains(&"E154"));
    let twice = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER } WITH SYNTAX { ID &id AGAIN &id } END",
    ));
    assert!(twice.contains(&"E154"));
    let reserved = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER } WITH SYNTAX { SEQUENCE &id } END",
    ));
    assert!(reserved.contains(&"E154"));
    let empty_group = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER } WITH SYNTAX { ID &id [NOTHING] } END",
    ));
    assert!(empty_group.contains(&"E154"));
    let good = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &id INTEGER, &T OPTIONAL } WITH SYNTAX { ID &id [TYPE &T] } END",
    ));
    assert!(!good.contains(&"E154"));
}

// X.681 10.12 c
#[test]
fn e154_adjacent_groups_starting_with_the_same_literal() {
    let bad = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &a INTEGER OPTIONAL, &b INTEGER OPTIONAL, &id INTEGER }
         WITH SYNTAX { [X &a] [X &b] ID &id } END",
    ));
    assert!(bad.contains(&"E154"));
}

// X.681 10.5 Note 2
#[test]
fn double_brackets_in_a_syntax_list_are_two_brackets() {
    let found = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &a INTEGER OPTIONAL, &b INTEGER OPTIONAL, &id INTEGER }
         WITH SYNTAX { [A &a [[B &b]]] ID &id } END",
    ));
    assert!(!found.contains(&"E154"), "{found:?}");
}

// X.681 11.4-11.5, 11.7
#[test]
fn e152_object_does_not_match_its_class() {
    assert!(run("a ATTR ::= { TYPE INTEGER }").contains(&"E152"));
    assert!(run("a ATTR ::= { COUNT 3 IDENTIFIED BY {1 2} TYPE INTEGER }").contains(&"E152"));
    assert!(run("a ATTR ::= { IDENTIFIED BY {1 2}, }").contains(&"E152"));
    clean("a ATTR ::= { TYPE INTEGER COUNT 3 IDENTIFIED BY {1 2} }");
    clean("b ATTR ::= { IDENTIFIED BY {1 2} }");
}

#[test]
fn e152_default_syntax_errors() {
    let class = "C ::= CLASS { &id INTEGER UNIQUE, &T OPTIONAL }";
    let body = |objects: &str| format!("M DEFINITIONS ::= BEGIN\n{class}\n{objects}\nEND");
    assert!(codes(&check(&body("o C ::= { &T INTEGER }"))).contains(&"E152"));
    assert!(codes(&check(&body("o C ::= { &id 1, &id 2 }"))).contains(&"E152"));
    assert!(codes(&check(&body("o C ::= { &nope 1, &id 2 }"))).contains(&"E152"));
    assert!(codes(&check(&body("o C ::= { &id INTEGER }"))).contains(&"E152"));
    assert!(!codes(&check(&body("o C ::= { &id 1, &T INTEGER }"))).contains(&"E152"));
}

// X.681 10.12 d
#[test]
fn e152_literal_without_setting() {
    let found = run("a ATTR ::= { TYPE IDENTIFIED BY {1 2} }");
    assert!(found.contains(&"E152"));
}

// X.681 8.2, 11.3, 12.10
#[test]
fn e155_object_of_the_wrong_class() {
    let text = "M DEFINITIONS ::= BEGIN
A ::= CLASS { &id INTEGER UNIQUE }
B ::= CLASS { &id INTEGER UNIQUE }
a1 A ::= { &id 1 }
bad B ::= a1
S B ::= { a1 }
END";
    let found = codes(&check(text));
    assert_eq!(
        found.iter().filter(|c| **c == "E155").count(),
        2,
        "{found:?}"
    );
}

#[test]
fn e155_a_separate_identical_class_does_not_count() {
    let text = "M DEFINITIONS ::= BEGIN
A ::= CLASS { &id INTEGER UNIQUE }
B ::= CLASS { &id INTEGER UNIQUE }
a1 A ::= { &id 1 }
S B ::= { a1 }
END";
    assert!(codes(&check(text)).contains(&"E155"));
    let good = text.replace("S B ::= { a1 }", "S A ::= { a1 }");
    assert!(!codes(&check(&good)).contains(&"E155"));
}

#[test]
fn e155_a_name_that_is_not_an_object() {
    let found = run("v INTEGER ::= 5\nS ATTR ::= { v }");
    assert!(found.contains(&"E155"), "{found:?}");
}

// X.681 9.7
#[test]
fn e156_two_objects_with_the_same_unique_value() {
    let bad = run("a ATTR ::= { IDENTIFIED BY {1 2} }
b ATTR ::= { IDENTIFIED BY {1 2} }
S ATTR ::= { a | b }");
    assert!(bad.contains(&"E156"), "{bad:?}");
    let named = run("id-x OBJECT IDENTIFIER ::= {1 2}
a ATTR ::= { IDENTIFIED BY id-x }
b ATTR ::= { IDENTIFIED BY {1 2} }
S ATTR ::= { a | b }");
    assert!(named.contains(&"E156"), "{named:?}");
    clean(
        "a ATTR ::= { IDENTIFIED BY {1 2} }
b ATTR ::= { IDENTIFIED BY {1 3} }
S ATTR ::= { a | b | a }",
    );
}

// X.681 11.2, 12.2
#[test]
fn e157_recursive_object_sets() {
    assert!(run("S ATTR ::= { S }").contains(&"E157"));
    assert!(run("A ATTR ::= { B }\nB ATTR ::= { A }").contains(&"E157"));
    clean("a ATTR ::= { IDENTIFIED BY {1 2} }\nA ATTR ::= { a }\nB ATTR ::= { A, ... }");
}

// X.681 12.3
#[test]
fn e158_empty_object_set() {
    assert!(run("S ATTR ::= { }").contains(&"E158"));
    clean("S ATTR ::= { ... }");
}

// X.681 14.2 a
#[test]
fn e159_open_type_as_the_type_of_a_value_field() {
    let bad = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &Open, &v C2.&Open } C2 ::= CLASS { &Open } END",
    ));
    assert!(bad.contains(&"E159"));
    let indirect = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &v T } T ::= SEQUENCE { a C2.&Open } C2 ::= CLASS { &Open } END",
    ));
    assert!(indirect.contains(&"E159"));
    let good = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &Open, &v INTEGER } END",
    ));
    assert!(!good.contains(&"E159"));
}

// X.681 14.1, 14.5, 9.14
#[test]
fn e150_field_references() {
    assert!(run("T ::= ATTR.&nope").contains(&"E150"));
    assert!(run("T ::= ATTR.&id.&more").contains(&"E150"));
    assert!(run("L ::= CLASS { &o ATTR OPTIONAL }\nT ::= L.&o").contains(&"E150"));
    clean("T ::= ATTR.&id\nU ::= ATTR.&Type");
}

// X.681 15.4, 15.5, 15.11
#[test]
fn e150_information_from_objects() {
    let class = "C ::= CLASS { &id INTEGER UNIQUE, &T, &Sub D OPTIONAL }
D ::= CLASS { &x INTEGER, &U }
o C ::= { &id 1, &T BOOLEAN }
S C ::= { o }";
    let body = |extra: &str| format!("M DEFINITIONS ::= BEGIN\n{class}\n{extra}\nEND");
    assert!(codes(&check(&body("T1 ::= o.&T"))).is_empty());
    assert!(codes(&check(&body("T2 ::= o.&id"))).contains(&"E150"));
    assert!(codes(&check(&body("T3 ::= S.&T"))).contains(&"E150"));
    assert!(codes(&check(&body("T4 ::= o.&nope"))).contains(&"E150"));
    assert!(codes(&check(&body("v INTEGER ::= o.&id"))).is_empty());
}

// X.681 Annex A, B
#[test]
fn built_in_classes_need_no_import() {
    clean(
        "t TYPE-IDENTIFIER ::= { INTEGER IDENTIFIED BY {1 2 3} }
S TYPE-IDENTIFIER ::= { t }
X ::= TYPE-IDENTIFIER.&Type
a ABSTRACT-SYNTAX ::= { INTEGER IDENTIFIED BY {1 2 4} }",
    );
}

#[test]
fn a_class_alias_is_the_same_class() {
    clean("CT ::= ATTR\no CT ::= { IDENTIFIED BY {1 2} }\nS ATTR ::= { o }");
}

#[test]
fn undefined_names_inside_objects_are_reported() {
    let found = run("a ATTR ::= { TYPE Nope IDENTIFIED BY nothing }");
    assert_eq!(
        found.iter().filter(|c| **c == "E110").count(),
        2,
        "{found:?}"
    );
}

#[test]
fn enumeration_identifiers_set_a_field_of_enumerated_type() {
    clean(
        "Mode ::= ENUMERATED { fast, slow }
C ::= CLASS { &id INTEGER UNIQUE, &mode Mode DEFAULT fast } WITH SYNTAX { ID &id [MODE &mode] }
o C ::= { ID 1 MODE slow }",
    );
}

#[test]
fn has_is_available() {
    assert!(!has(&check(&module("")), "E152"));
}

#[test]
fn undefined_names_inside_class_definitions_are_reported() {
    let found = codes(&check(
        "M DEFINITIONS ::= BEGIN C ::= CLASS { &a Nope, &B Other OPTIONAL } END",
    ));
    assert_eq!(
        found.iter().filter(|c| **c == "E110").count(),
        2,
        "{found:?}"
    );
}
