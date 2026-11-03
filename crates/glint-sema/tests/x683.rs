mod common;

use common::{check, codes};

fn run(body: &str) -> Vec<&'static str> {
    codes(&check(&format!(
        "M DEFINITIONS EXPLICIT TAGS ::= BEGIN\n{body}\nEND"
    )))
}

// X.683 8.5 example
#[test]
fn x683_8_5_parameterized_class_example_is_clean() {
    let found = run("PARAMETERIZED-OBJECT-CLASS { TypeParam, INTEGER:valueParam, INTEGER:ValueSetParam } ::= CLASS {
  &valueField1 TypeParam,
  &valueField2 INTEGER DEFAULT valueParam,
  &valueField3 INTEGER (ValueSetParam),
  &ValueSetField INTEGER DEFAULT { ValueSetParam }
}
MY-OBJECT-CLASS ::= PARAMETERIZED-OBJECT-CLASS { BIT STRING, 123, {4 | 5 | 6} }");
    assert!(found.is_empty(), "{found:?}");
}

// X.683 Annex A
#[test]
fn x683_annex_a_examples_are_clean() {
    let found = run("SIGNED { ToBeSigned } ::= SEQUENCE { authenticated-data ToBeSigned, authenticator BIT STRING }
OrderInformation ::= INTEGER
OPTIONALLY-SIGNED {ToBeSigned} ::= CHOICE {
  unsigned-data [0] ToBeSigned,
  signed-data [1] SIGNED { ToBeSigned } }
A ::= SIGNED { OrderInformation }
List1 { ElementTypeParam } ::= SEQUENCE { elem ElementTypeParam, next List1 { ElementTypeParam } OPTIONAL }
IntegerList1 ::= List1 { INTEGER }
genericBirthdayGreeting { IA5String : name } IA5String ::= { \"Happy birthday, \", name, \"!!\" }
greeting1 IA5String ::= genericBirthdayGreeting { \"John\" }
QuestList1 {IA5String : extraQuest} IA5String ::= { \"Jack\" | \"John\" | extraQuest }
QuestList2 {IA5String : ExtraQuests} IA5String ::= { \"Jack\" | \"John\" | ExtraQuests }
SetOfQuests1 IA5String ::= { QuestList1 { \"Jill\" } }
SetOfQuests2 IA5String ::= { QuestList2 { {\"Jill\"} } }
GENERIC-ERROR { ErrorCodeType, ErrorCodeType : ValidErrorCodes } ::= CLASS { &errorCode ValidErrorCodes }");
    assert!(found.is_empty(), "{found:?}");
}

// X.683 8.3
#[test]
fn e161_parameter_list_errors() {
    assert!(run("T { value } ::= INTEGER (value)").contains(&"E161"));
    assert!(run("v INTEGER ::= 1\nT { v : p } ::= INTEGER (p)").contains(&"E161"));
    assert!(
        run(
            "T { INTEGER : a, a : b } ::= INTEGER (a | b)\nU { X : a, a : b } ::= SEQUENCE { c X }"
        )
        .contains(&"E161")
    );
    let good = run("T { INTEGER : value } ::= INTEGER (value)");
    assert!(good.is_empty(), "{good:?}");
}

// X.683 9.6
#[test]
fn e160_actual_parameter_lists() {
    assert!(run("T { A, B } ::= SEQUENCE { a A, b B }\nU ::= T { INTEGER }").contains(&"E160"));
    assert!(run("T { A } ::= SEQUENCE { a A }\nU ::= T").contains(&"E160"));
    assert!(run("T ::= SEQUENCE { a INTEGER }\nU ::= T { INTEGER }").contains(&"E160"));
    assert!(run("T { A } ::= SEQUENCE { a A }\nU ::= T { 5 }").contains(&"E160"));
    assert!(run("T { INTEGER : n } ::= INTEGER (n)\nU ::= T { INTEGER }").contains(&"E160"));
    let good =
        run("T { A, INTEGER : n } ::= SEQUENCE { a A, b INTEGER (n) }\nU ::= T { BOOLEAN, 5 }");
    assert!(good.is_empty(), "{good:?}");
}

// X.683 8.5
#[test]
fn e162_inconsistent_use_of_a_dummy() {
    let found = run("T { X } ::= SEQUENCE { a X, b X.&id }\nC ::= CLASS { &id INTEGER }");
    assert!(found.contains(&"E162"), "{found:?}");
}

// X.683 8.6
#[test]
fn e163_unused_dummy() {
    assert!(run("T { A, B } ::= SEQUENCE { a A }").contains(&"E163"));
}

// X.683 8.6-8.8
#[test]
fn e164_illegal_recursion() {
    assert!(run("v { INTEGER : n } INTEGER ::= v { n }").contains(&"E164"));
    assert!(run("T { A } ::= SEQUENCE { a A, next T { A } }").contains(&"E164"));
    // X.683 A.3: List2 grows without end
    let growing = run(
        "List2 { ElementTypeParam } ::= SEQUENCE { elem ElementTypeParam, next List2 { [0] ElementTypeParam } OPTIONAL }",
    );
    assert!(growing.contains(&"E164"), "{growing:?}");
    let fine = run("List1 { E } ::= SEQUENCE { elem E, next List1 { E } OPTIONAL }");
    assert!(fine.is_empty(), "{fine:?}");
}

// X.683 8.9, 8.11
#[test]
fn e165_governor_depends_on_a_dummy() {
    let governed = run("Foo { INTEGER : n } ::= INTEGER (n)
T { INTEGER : a, Foo { a } : b } ::= SEQUENCE { x INTEGER (a), y INTEGER (b) }");
    assert!(governed.contains(&"E165"), "{governed:?}");
    let itself = run("T { T : x } ::= SEQUENCE { y INTEGER (x) }");
    assert!(itself.contains(&"E165"), "{itself:?}");
}

// X.683 8.10
#[test]
fn e166_right_hand_side_is_only_a_dummy() {
    assert!(run("T { A } ::= A").contains(&"E166"));
}

// X.683 8.12
#[test]
fn e167_actual_value_incompatible_with_its_governor() {
    assert!(
        run("T { INTEGER : n } ::= SEQUENCE { a INTEGER (n) }\nU ::= T { \"text\" }")
            .contains(&"E167")
    );
    let good = run("T { INTEGER : n } ::= SEQUENCE { a INTEGER (n) }\nU ::= T { 5 }");
    assert!(good.is_empty(), "{good:?}");
}

// X.683 10
#[test]
fn x683_10_abstract_syntax_example_is_clean() {
    let found = run("MESSAGE-PARAMETERS ::= CLASS {
  &maximum-priority-level INTEGER,
  &maximum-message-buffer-size INTEGER
} WITH SYNTAX { THE MAXIMUM PRIORITY LEVEL IS &maximum-priority-level THE MAXIMUM MESSAGE BUFFER SIZE IS &maximum-message-buffer-size }
Message-PDU { MESSAGE-PARAMETERS : param } ::= SEQUENCE {
  priority-level INTEGER (0..param.&maximum-priority-level),
  message BMPString (SIZE (0..param.&maximum-message-buffer-size)) }
message-Abstract-Syntax { MESSAGE-PARAMETERS : param } ABSTRACT-SYNTAX ::= {
  Message-PDU { param } IDENTIFIED BY { joint-iso-itu-t example(999) 0 } }
my-message-parameters MESSAGE-PARAMETERS ::= {
  THE MAXIMUM PRIORITY LEVEL IS 10 THE MAXIMUM MESSAGE BUFFER SIZE IS 2000 }
my-message-Abstract-Syntax ABSTRACT-SYNTAX ::= message-Abstract-Syntax { my-message-parameters }");
    assert!(found.iter().all(|c| *c == "W169"), "{found:?}");
}

// X.683 10.2
#[test]
fn e168_abstract_syntax_parameter_outside_a_constraint() {
    let bad = run("PDU { INTEGER : bound } ::= SEQUENCE { a INTEGER (0..10), b INTEGER DEFAULT bound }
yyy-Abstract-Syntax { INTEGER : bound } ABSTRACT-SYNTAX ::= { PDU { bound } IDENTIFIED BY { 1 2 3 } }");
    assert!(bad.contains(&"E168"), "{bad:?}");
}

// X.683 10.3-10.4
#[test]
fn w169_variable_constraint_without_an_exception_spec() {
    let bare = run("PDU { INTEGER : bound } ::= SEQUENCE { a INTEGER (0..bound) }
yyy-Abstract-Syntax { INTEGER : bound } ABSTRACT-SYNTAX ::= { PDU { bound } IDENTIFIED BY { 1 2 3 } }");
    assert!(bare.contains(&"W169"), "{bare:?}");
    let with_exception = run("Err ::= ENUMERATED { bad }
PDU { INTEGER : bound } ::= SEQUENCE { a INTEGER (0..bound ! Err : bad) }
yyy-Abstract-Syntax { INTEGER : bound } ABSTRACT-SYNTAX ::= { PDU { bound } IDENTIFIED BY { 1 2 3 } }");
    assert!(!with_exception.contains(&"W169"), "{with_exception:?}");
}

// policy codes
#[test]
fn policy_codes() {
    let macro_text = "M DEFINITIONS ::= BEGIN OP MACRO ::= BEGIN TYPE NOTATION ::= \"x\" END END";
    assert!(codes(&check(macro_text)).contains(&"E172"));
    assert!(run("FOO ::= INTEGER").contains(&"W171"));
    assert!(!run("FOO ::= CLASS { &id INTEGER }").contains(&"W171"));
    let mixed = run("T ::= SEQUENCE { a ANY }\nC ::= CLASS { &id INTEGER }");
    assert!(mixed.contains(&"W170"), "{mixed:?}");
}
