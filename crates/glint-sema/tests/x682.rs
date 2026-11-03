mod common;

use common::{check, codes};

const ERRORS: &str = "ERROR-CLASS ::= CLASS {
   &category PrintableString (SIZE(1)),
   &code INTEGER,
   &Type
} WITH SYNTAX { &category &code &Type }
ErrorSet ERROR-CLASS ::= {
   {\"A\" 1 INTEGER} |
   {\"A\" 2 REAL} |
   {\"B\" 1 CHARACTER STRING} |
   {\"B\" 2 GeneralString}
}
";

fn run(body: &str) -> Vec<&'static str> {
    codes(&check(&format!(
        "M DEFINITIONS EXPLICIT TAGS ::= BEGIN\n{ERRORS}\n{body}\nEND"
    )))
}

// X.682 clause 10, Example
#[test]
fn x682_clause_10_example_is_clean() {
    let found = run("ErrorReturn ::= SEQUENCE {
  errorCategory ERROR-CLASS.&category ({ErrorSet}) OPTIONAL,
  errors SEQUENCE OF SEQUENCE {
    errorCode ERROR-CLASS.&code ({ErrorSet}{@errorCategory}),
    errorInfo ERROR-CLASS.&Type ({ErrorSet}{@errorCategory,@.errorCode})
  } OPTIONAL
}");
    assert!(found.is_empty(), "{found:?}");
}

// X.682 10.10 b: `@...errorId`
#[test]
fn x682_10_10_example_with_three_dots_is_clean() {
    let found = run(
        "ERROR ::= CLASS { &severity INTEGER, &id INTEGER UNIQUE, &Type }
Errors ERROR ::= { {&severity 1, &id 1, &Type INTEGER} }
ErrorMessage ::= SEQUENCE {
  severity ERROR.&severity({Errors}),
  parameters SEQUENCE OF SEQUENCE {
    errorId ERROR.&id({Errors}),
    data SEQUENCE OF SEQUENCE {
      value ERROR.&Type({Errors}{@severity,@...errorId}),
      text VisibleString }}}",
    );
    assert!(found.is_empty(), "{found:?}");
}

// X.682 10.1
#[test]
fn e180_table_constraint_on_a_type_that_is_not_a_field_type() {
    let bad = run("T ::= SEQUENCE { a INTEGER, b INTEGER ({ErrorSet}{@a}) }");
    assert!(bad.contains(&"E180"), "{bad:?}");
}

// X.682 A.2
#[test]
fn e180_instance_of_takes_only_a_simple_table_constraint() {
    let bad = run("T ::= INSTANCE OF TYPE-IDENTIFIER (SIZE (1..2))");
    assert!(bad.contains(&"E180"), "{bad:?}");
    let good = run(
        "s TYPE-IDENTIFIER ::= { INTEGER IDENTIFIED BY {1 2} }\nSet TYPE-IDENTIFIER ::= { s }\nT ::= INSTANCE OF TYPE-IDENTIFIER ({Set})",
    );
    assert!(good.is_empty(), "{good:?}");
}

// X.682 10.4
#[test]
fn e181_constraining_set_of_the_wrong_class() {
    let bad = run("OTHER ::= CLASS { &x INTEGER UNIQUE }
OtherSet OTHER ::= { {&x 1} }
T ::= SEQUENCE { a ERROR-CLASS.&code ({OtherSet}) }");
    assert!(bad.contains(&"E181"), "{bad:?}");
}

// X.682 10.8-10.10
#[test]
fn e151_component_relation_paths() {
    let unknown = run(
        "T ::= SEQUENCE { a ERROR-CLASS.&category ({ErrorSet}), b ERROR-CLASS.&code ({ErrorSet}{@nope}) }",
    );
    assert!(unknown.contains(&"E151"), "{unknown:?}");
    let too_many = run(
        "T ::= SEQUENCE { a ERROR-CLASS.&category ({ErrorSet}), b ERROR-CLASS.&code ({ErrorSet}{@..a}) }",
    );
    assert!(too_many.contains(&"E151"), "{too_many:?}");
    let through_leaf = run(
        "T ::= SEQUENCE { a ERROR-CLASS.&category ({ErrorSet}), b ERROR-CLASS.&code ({ErrorSet}{@a.x}) }",
    );
    assert!(through_leaf.contains(&"E151"), "{through_leaf:?}");
    let good = run(
        "T ::= SEQUENCE { a ERROR-CLASS.&category ({ErrorSet}), b ERROR-CLASS.&code ({ErrorSet}{@a}), c ERROR-CLASS.&Type ({ErrorSet}{@.a, @b}) }",
    );
    assert!(good.is_empty(), "{good:?}");
}

// X.682 10.15
#[test]
fn e182_referenced_component_must_be_a_value_field_of_the_same_set() {
    let plain = run("T ::= SEQUENCE { a INTEGER, b ERROR-CLASS.&code ({ErrorSet}{@a}) }");
    assert!(plain.contains(&"E182"), "{plain:?}");
    let other_set = run("Other ERROR-CLASS ::= { {\"C\" 1 INTEGER} }
T ::= SEQUENCE { a ERROR-CLASS.&category ({Other}), b ERROR-CLASS.&code ({ErrorSet}{@a}) }");
    assert!(other_set.contains(&"E182"), "{other_set:?}");
    let type_field = run(
        "T ::= SEQUENCE { a ERROR-CLASS.&Type ({ErrorSet}), b ERROR-CLASS.&code ({ErrorSet}{@a}) }",
    );
    assert!(type_field.contains(&"E182"), "{type_field:?}");
}

// X.682 10.14
#[test]
fn e183_referenced_component_in_another_alternative() {
    let bad = run("T ::= CHOICE {
  one SEQUENCE { a ERROR-CLASS.&category ({ErrorSet}) },
  two SEQUENCE { b ERROR-CLASS.&code ({ErrorSet}{@one.a}) } }");
    assert!(bad.contains(&"E183"), "{bad:?}");
}

// X.682 11.3
#[test]
fn e184_contents_constraints() {
    assert!(run("T ::= INTEGER (CONTAINING BOOLEAN)").contains(&"E184"));
    assert!(run("T ::= BIT STRING { a(0) } (CONTAINING BOOLEAN)").contains(&"E184"));
    assert!(run("T ::= OCTET STRING (SIZE (2)) (CONTAINING BOOLEAN)").contains(&"E184"));
    assert!(
        run("Base ::= OCTET STRING (SIZE (2))\nT ::= Base (CONTAINING BOOLEAN)").contains(&"E184")
    );
    assert!(run("T ::= OCTET STRING (CONTAINING BOOLEAN ENCODED BY 5)").contains(&"E184"));
    let good = run(
        "T ::= OCTET STRING (CONTAINING BOOLEAN)\nU ::= BIT STRING (CONTAINING INTEGER ENCODED BY {2 1 1})",
    );
    assert!(good.is_empty(), "{good:?}");
}

// X.682 9.3
#[test]
fn e185_constrained_by_parameters() {
    assert!(run("v INTEGER ::= 5\nT ::= OCTET STRING (CONSTRAINED BY { v })").contains(&"E185"));
    assert!(
        run("v INTEGER ::= 5\nT ::= OCTET STRING (CONSTRAINED BY { v, INTEGER : v })")
            .contains(&"E185")
    );
    assert!(
        run("v INTEGER ::= 5\nT ::= OCTET STRING (CONSTRAINED BY { v : v })").contains(&"E185")
    );
    let good = run(
        "v INTEGER ::= 5\nT ::= OCTET STRING (CONSTRAINED BY { -- anything -- INTEGER : v, ErrorSet, ERROR-CLASS, BOOLEAN })",
    );
    assert!(good.is_empty(), "{good:?}");
}

// X.682 9.4
#[test]
fn x682_9_4_example_is_clean() {
    let found = run("ENCRYPTED {ToBeEnciphered} ::= BIT STRING
  (CONSTRAINED BY { -- must be the result of the encipherment of some BER-encoded value of -- ToBeEnciphered }
  ! Err : securityViolation)
Err ::= ENUMERATED {securityViolation}
SecurityParameters ::= INTEGER
U ::= ENCRYPTED{SecurityParameters}");
    assert!(found.is_empty(), "{found:?}");
}
