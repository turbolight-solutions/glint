mod common;
use common::*;

fn dump(src: &str) -> String {
    format!("{:#?}", assert_clean(src).syntax())
}

#[test]
fn extension_additions_and_version_groups() {
    insta::assert_snapshot!(dump(&module(
        "T ::= SEQUENCE { a INTEGER, ..., [[ 2: b BOOLEAN OPTIONAL, c NULL DEFAULT NULL ]], ..., d INTEGER }"
    )));
}

#[test]
fn import_assigned_identifier_resolution() {
    insta::assert_snapshot!(dump(&module(
        "IMPORTS a, B FROM X { 1 2 } WITH SUCCESSORS c FROM Y d FROM Z e{} FROM W;"
    )));
}

#[test]
fn subtype_constraint_with_set_arithmetic() {
    insta::assert_snapshot!(dump(&module(
        "T ::= INTEGER (1..10 | 20 ^ (MIN..<50) EXCEPT 5, ... ! 7)"
    )));
}

#[test]
fn table_constraint_with_component_relation() {
    insta::assert_snapshot!(dump(&module(
        "T ::= SEQUENCE { id OBJECT-CLASS.&id ({Set}), val OBJECT-CLASS.&Type ({Set}{@id}) }"
    )));
}

#[test]
fn ambiguous_assignments_keep_their_right_hand_sides_neutral() {
    insta::assert_snapshot!(dump(&module(
        "V INTEGER ::= { 1 | 2 }\nobj MY-CLASS ::= { &id 1, &Type BOOLEAN }\nSet MY-CLASS ::= { obj1 | obj2, ... }\nn INTEGER ::= 5"
    )));
}

#[test]
fn parameterized_definition_and_use() {
    insta::assert_snapshot!(dump(&module(
        "List{Elem, INTEGER:max} ::= SEQUENCE (SIZE (1..max)) OF Elem\nU ::= List{ BOOLEAN, 8 }"
    )));
}
