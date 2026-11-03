mod common;
use common::*;

#[test]
fn clean_module_has_no_findings() {
    let findings = check(
        "M DEFINITIONS ::= BEGIN
         T ::= SEQUENCE { a INTEGER, b BOOLEAN OPTIONAL }
         v INTEGER ::= 5
         END",
    );
    assert!(findings.is_empty(), "{:?}", codes(&findings));
}

#[test]
fn undefined_type_is_e110() {
    let findings = check("M DEFINITIONS ::= BEGIN T ::= SEQUENCE { a Missing } END");
    assert_eq!(codes(&findings), ["E110"]);
}
