mod common;
use common::*;
use glint_syntax::SyntaxKind;

// ---- clause 13: modules -----------------------------------------------------------------------

#[test]
fn x680_13_1_module_header_with_oid_and_defaults() {
    let parse = assert_clean(
        "Foo { iso(1) identified-organization(3) 6 1 } DEFINITIONS XER INSTRUCTIONS AUTOMATIC TAGS EXTENSIBILITY IMPLIED ::= BEGIN END",
    );
    assert_eq!(count(&parse, SyntaxKind::EncodingReferenceDefault), 1);
    assert_eq!(count(&parse, SyntaxKind::TagDefault), 1);
    assert_eq!(count(&parse, SyntaxKind::ExtensionDefault), 1);
}

#[test]
fn x680_13_1_definitive_oid_and_iri() {
    assert_clean("Foo { 1 2 3 } \"/Example/Foo\" DEFINITIONS ::= BEGIN END");
}

#[test]
fn x680_13_1_empty_module_and_all_tag_defaults() {
    for tags in ["", "EXPLICIT TAGS", "IMPLICIT TAGS", "AUTOMATIC TAGS"] {
        assert_clean(&format!("Foo DEFINITIONS {tags} ::= BEGIN END"));
    }
}

#[test]
fn x680_13_1_several_modules_in_one_block() {
    let parse = assert_clean(
        "A DEFINITIONS ::= BEGIN X ::= INTEGER END\nB DEFINITIONS ::= BEGIN Y ::= BOOLEAN END",
    );
    assert_eq!(count(&parse, SyntaxKind::ModuleDefinition), 2);
}

#[test]
fn x680_13_1_note_2_module_body_without_a_header() {
    let parse = assert_clean("IMPORTS X FROM Y;\nA ::= INTEGER\nb A ::= 1");
    assert_eq!(count(&parse, SyntaxKind::ModuleDefinition), 0);
    assert_eq!(count(&parse, SyntaxKind::TypeAssignment), 1);
    assert_eq!(count(&parse, SyntaxKind::ValueAssignment), 1);
}

#[test]
fn x680_13_1_module_identifier_may_close_on_a_continuation_line_with_definitions() {
    // The ITU-T modules are written this way (X.411 MTSAbstractService): the line holding the last arcs also
    // closes the group and carries `DEFINITIONS ... ::=`, which must not read as a new assignment.
    let parse = assert_clean(
        "MTSAbstractService {joint-iso-itu-t mhs(6) mts(3) modules(0)\n  mts-abstract-service(1) version-1999(1)} DEFINITIONS IMPLICIT TAGS ::=\nBEGIN\nX ::= INTEGER\nEND",
    );
    assert_eq!(count(&parse, SyntaxKind::ModuleDefinition), 1);
    assert_eq!(count(&parse, SyntaxKind::TypeAssignment), 1);
}

#[test]
fn x680_13_1_exports_forms() {
    clean_module("EXPORTS ALL;");
    clean_module("EXPORTS;");
    clean_module("EXPORTS A, b, C{};\nA ::= INTEGER");
}

#[test]
fn x680_13_1_imports_forms() {
    clean_module("IMPORTS;");
    clean_module("IMPORTS A, b FROM X { 1 2 } C FROM Y;");
    clean_module("IMPORTS A{}, b FROM X;");
}

#[test]
fn x680_13_9_assigned_identifier_resolution() {
    let parse = clean_module("IMPORTS a FROM X id-x b, C FROM Y;");
    assert_eq!(count(&parse, SyntaxKind::AssignedIdentifier), 1);

    let parse = clean_module("IMPORTS a FROM X b FROM Y;");
    assert_eq!(count(&parse, SyntaxKind::AssignedIdentifier), 0);

    let parse = clean_module("IMPORTS a FROM X b{} FROM Y;");
    assert_eq!(count(&parse, SyntaxKind::AssignedIdentifier), 0);

    let parse = clean_module("IMPORTS a FROM X Other.id-x;");
    assert_eq!(count(&parse, SyntaxKind::AssignedIdentifier), 1);

    let parse = clean_module("IMPORTS a FROM X { 1 2 } b FROM Y;");
    assert_eq!(count(&parse, SyntaxKind::AssignedIdentifier), 1);
}

#[test]
fn x680_13_16_f_with_successors_and_descendants() {
    clean_module("IMPORTS A FROM X { 1 2 3 } WITH SUCCESSORS B FROM Y { 1 2 } WITH DESCENDANTS;");
}

#[test]
fn x680_13_16_f_with_successors_needs_an_assigned_identifier() {
    assert_eq!(
        module_codes("IMPORTS A FROM X WITH SUCCESSORS;"),
        vec!["E108"]
    );
}

#[test]
fn x680_54_encoding_control_sections_are_opaque() {
    let src = "A DEFINITIONS ::= BEGIN X ::= INTEGER\nENCODING-CONTROL XER GLOBAL-DEFAULTS MODIFIED-ENCODINGS\nENCODING-CONTROL PER whatever { a b } END";
    assert_clean(src);
}

#[test]
fn x680_54_unknown_encoding_control_reference_is_w136() {
    let found = codes("A DEFINITIONS ::= BEGIN ENCODING-CONTROL ZZZ stuff END");
    assert_eq!(found, vec!["W136"]);
}

// ---- clauses 16-17: assignments and references ------------------------------------------------

#[test]
fn x680_16_1_type_assignment() {
    clean_module("A ::= INTEGER");
}

#[test]
fn x680_16_2_value_assignment() {
    clean_module("a INTEGER ::= 5\nb BOOLEAN ::= TRUE\nc UTF8String ::= \"x\"");
}

#[test]
fn x680_16_6_value_set_assignment_is_kept_ambiguous() {
    let parse = clean_module("V INTEGER ::= { 1 | 2 }");
    assert_eq!(count(&parse, SyntaxKind::ValueSetAssignment), 1);
    assert_eq!(count(&parse, SyntaxKind::AmbiguousRhs), 1);
}

#[test]
fn x681_9_1_class_vs_type_assignment_shape_is_ambiguous_until_resolved() {
    let parse = clean_module("MY-CLASS ::= CLASS { &id INTEGER UNIQUE }\nOther ::= MY-CLASS");
    assert_eq!(count(&parse, SyntaxKind::ObjectClassDef), 1);
    assert_eq!(count(&parse, SyntaxKind::TypeAssignment), 2);
}

#[test]
fn x681_11_1_object_assignment_has_ambiguous_rhs() {
    let parse = clean_module("obj MY-CLASS ::= { &id 1 }");
    assert_eq!(count(&parse, SyntaxKind::AmbiguousRhs), 1);
}

#[test]
fn x681_9_2_class_with_syntax() {
    clean_module(
        "C ::= CLASS { &id OBJECT IDENTIFIER UNIQUE, &Type, &value &Type OPTIONAL } WITH SYNTAX { ID &id TYPE &Type [VALUE &value] }",
    );
}

#[test]
fn x683_8_1_parameterized_assignments() {
    clean_module("List{ElementType} ::= SEQUENCE OF ElementType");
    clean_module("Bounded{INTEGER:n} ::= INTEGER (0..n)");
    clean_module("pv{Type, Type:x} Type ::= x");
    clean_module("Vs{INTEGER:lo} INTEGER ::= { lo | 5 }");
}

#[test]
fn x683_9_2_parameterized_uses_split_actuals_at_top_level_commas() {
    let parse = clean_module("A ::= List{ SEQUENCE OF Foo, { 1, 2 }, bar }");
    assert_eq!(count(&parse, SyntaxKind::ActualParameter), 3);
}

#[test]
fn x680_legacy_macro_definition_is_opaque() {
    let parse = clean_module(
        "OBJECT-TYPE MACRO ::=\nBEGIN\n  TYPE NOTATION ::= \"SYNTAX\" type(Syntax)\n  VALUE NOTATION ::= value(VALUE OBJECT IDENTIFIER)\nEND\nA ::= INTEGER",
    );
    assert_eq!(count(&parse, SyntaxKind::MacroDefinition), 1);
    assert_eq!(count(&parse, SyntaxKind::TypeAssignment), 1);
}

#[test]
fn x680_legacy_macro_use_is_opaque_and_ends_with_its_value() {
    let parse = clean_module(
        "sysDescr OBJECT-TYPE\n  SYNTAX OCTET STRING\n  ACCESS read-only\n  STATUS mandatory\n  DESCRIPTION \"text\"\n  ::= { system 1 }\nA ::= INTEGER",
    );
    assert_eq!(count(&parse, SyntaxKind::MacroUse), 1);
    assert_eq!(count(&parse, SyntaxKind::TypeAssignment), 1);
}

#[test]
fn x680_legacy_any_is_a_type_when_not_defined_or_imported() {
    let parse = clean_module(
        "A ::= SEQUENCE { alg OBJECT IDENTIFIER, params ANY DEFINED BY alg OPTIONAL, x ANY }",
    );
    assert_eq!(count(&parse, SyntaxKind::AnyType), 2);
}

#[test]
fn x680_legacy_any_defined_in_the_module_is_an_ordinary_name() {
    let parse = clean_module("ANY ::= INTEGER\nB ::= SEQUENCE { a ANY }");
    assert_eq!(count(&parse, SyntaxKind::AnyType), 0);
}

// ---- clauses 18-48: types ---------------------------------------------------------------------

#[test]
fn x680_18_to_24_simple_built_in_types() {
    for ty in [
        "BOOLEAN",
        "NULL",
        "REAL",
        "INTEGER",
        "INTEGER { a(1), b(-2), c(other-value), d(Mod.value) }",
        "BIT STRING",
        "BIT STRING { first(0), second(1) }",
        "OCTET STRING",
        "OBJECT IDENTIFIER",
        "RELATIVE-OID",
        "OID-IRI",
        "RELATIVE-OID-IRI",
        "EMBEDDED PDV",
        "EXTERNAL",
        "CHARACTER STRING",
        "TIME",
        "DATE",
        "TIME-OF-DAY",
        "DATE-TIME",
        "DURATION",
    ] {
        clean_type(ty);
    }
}

#[test]
fn x680_40_to_41_character_string_and_useful_types() {
    for ty in [
        "BMPString",
        "GeneralString",
        "GraphicString",
        "IA5String",
        "ISO646String",
        "NumericString",
        "PrintableString",
        "TeletexString",
        "T61String",
        "UniversalString",
        "UTF8String",
        "VideotexString",
        "VisibleString",
        "GeneralizedTime",
        "UTCTime",
        "ObjectDescriptor",
    ] {
        clean_type(ty);
    }
}

#[test]
fn x680_20_1_enumerated_with_extension_and_exception() {
    clean_type("ENUMERATED { a, b(5), ... }");
    clean_type("ENUMERATED { a, b, ... ! 3, c(7), d }");
    clean_type("ENUMERATED { a }");
}

#[test]
fn x680_25_1_sequence_forms() {
    clean_type("SEQUENCE { }");
    clean_type("SEQUENCE { ... }");
    clean_type("SEQUENCE { a INTEGER, b BOOLEAN OPTIONAL, c NULL DEFAULT NULL }");
    clean_type("SEQUENCE { a INTEGER, ... }");
    clean_type("SEQUENCE { a INTEGER, ..., b BOOLEAN }");
    clean_type("SEQUENCE { a INTEGER, ..., b BOOLEAN, ..., c NULL }");
    clean_type("SEQUENCE { ..., a INTEGER, ... }");
    clean_type(
        "SEQUENCE { a INTEGER, ... ! INTEGER : 5, [[ 2: b BOOLEAN, c NULL ]], [[ d NULL ]], e NULL, ... }",
    );
}

#[test]
fn x680_25_5_components_of() {
    let parse = clean_type("SEQUENCE { COMPONENTS OF Base, extra INTEGER }");
    assert_eq!(count(&parse, SyntaxKind::ComponentsOf), 1);
}

#[test]
fn x680_26_1_sequence_of_and_named_element() {
    clean_type("SEQUENCE OF INTEGER");
    clean_type("SEQUENCE OF item INTEGER");
    clean_type("SET OF Foo");
    clean_type("SET OF item Foo");
    clean_type("SEQUENCE OF SEQUENCE OF INTEGER");
}

#[test]
fn x680_49_5_sequence_of_with_size_constraint_forms() {
    clean_type("SEQUENCE SIZE (1..MAX) OF INTEGER");
    clean_type("SEQUENCE (SIZE (1..MAX)) OF INTEGER");
    clean_type("SET SIZE (2) OF item Foo");
    clean_type("SEQUENCE (SIZE (1..4)) OF INTEGER (0..9)");
}

#[test]
fn x680_27_1_set_type() {
    clean_type("SET { a [0] INTEGER, b [1] BOOLEAN OPTIONAL, ... }");
}

#[test]
fn x680_29_1_choice_type() {
    clean_type("CHOICE { a INTEGER, b BOOLEAN }");
    clean_type("CHOICE { a INTEGER, ..., [[ b BOOLEAN ]], c NULL }");
}

#[test]
fn x680_29_choice_alternative_cannot_be_optional() {
    assert_eq!(
        module_codes("T ::= CHOICE { a INTEGER OPTIONAL }"),
        vec!["E001"]
    );
}

#[test]
fn x680_30_1_selection_type() {
    let parse = clean_type("SEQUENCE { x a < Choice }");
    assert_eq!(count(&parse, SyntaxKind::SelectionType), 1);
}

#[test]
fn x680_31_tagged_types() {
    clean_type("[0] INTEGER");
    clean_type("[0] IMPLICIT INTEGER");
    clean_type("[APPLICATION 3] EXPLICIT INTEGER");
    clean_type("[UNIVERSAL 3] INTEGER");
    clean_type("[PRIVATE 42] OCTET STRING");
    clean_type("[some-number] INTEGER");
    clean_type("[Mod.some-number] INTEGER");
    clean_type("[TAG: 5] INTEGER");
    clean_type("[0] [1] INTEGER");
}

#[test]
fn x680_31_3_encoding_prefixed_types_are_opaque() {
    let parse = clean_type("[XER: NAME AS \"x\" ] INTEGER");
    assert_eq!(count(&parse, SyntaxKind::EncodingPrefixedType), 1);
    assert_eq!(
        codes(&module("T ::= [ZZZ: foo [bar] ] INTEGER")),
        vec!["W136"]
    );
}

#[test]
fn x680_instance_of_and_class_field_types() {
    clean_type("INSTANCE OF TYPE-IDENTIFIER");
    clean_type("TYPE-IDENTIFIER.&Type");
    clean_type("SEQUENCE { id MY-CLASS.&id, value MY-CLASS.&Type, other Mod.CLASS.&Type }");
    clean_type("SEQUENCE { t obj.&Type, v Mod.Type }");
    clean_type("MY-CLASS.&Field.&Sub");
}

#[test]
fn x680_17_1_defined_and_external_types() {
    clean_type("Foo");
    clean_type("Module.Foo");
    clean_type("Foo (SIZE (1..2))");
}

#[test]
fn x680_reserved_word_in_type_position_is_an_error() {
    assert_eq!(module_codes("T ::= END").first(), Some(&"E001"));
}

// ---- values -----------------------------------------------------------------------------------

#[test]
fn x680_values_built_in_notation() {
    clean_value("BOOLEAN", "TRUE");
    clean_value("INTEGER", "-12");
    clean_value("REAL", "1.5e-3");
    clean_value("REAL", "PLUS-INFINITY");
    clean_value("REAL", "{ mantissa 314, base 10, exponent -2 }");
    clean_value("BIT STRING", "{ first, second }");
    clean_value("OCTET STRING", "'FF'H");
    clean_value("OBJECT IDENTIFIER", "{ iso(1) member-body(2) 840 113549 }");
    clean_value("OBJECT IDENTIFIER", "{ id-pkix 1 }");
    clean_value("Seq", "{ a 1, b TRUE, c { d 1 } }");
    clean_value("Ch", "alt : 5");
    clean_value("Ch", "alt : { a 1 }");
    clean_value("Foo", "Mod.value");
    clean_value("INTEGER", "other-value");
    clean_value("UTF8String", "\"text\"");
    clean_value("REAL", "NOT-A-NUMBER");
}

#[test]
fn x681_14_open_type_value_has_a_type_prefix() {
    clean_value("OpenType", "INTEGER : 5");
    clean_value("OpenType", "Foo : { a 1 }");
}

// ---- constraints ------------------------------------------------------------------------------

#[test]
fn x680_51_subtype_elements() {
    for constraint in [
        "(5)",
        "(1..10)",
        "(1<..10)",
        "(1..<10)",
        "(1<..<10)",
        "(MIN..MAX)",
        "(-5..5)",
        "(a | b | 3)",
        "(SIZE (1..4))",
        "(FROM (\"a\"..\"z\"))",
        "(INCLUDES Foo)",
        "(Foo)",
        "(PATTERN \"[a-z]+\")",
        "(SETTINGS \"Basic=Date\")",
        "(WITH COMPONENT (SIZE (1..2)))",
        "(WITH COMPONENTS { a (1..2), b ABSENT, c PRESENT, d OPTIONAL })",
        "(WITH COMPONENTS { ..., a PRESENT })",
        "(TRUE)",
        "(Mod.value)",
    ] {
        clean_type(&format!("INTEGER {constraint}"));
    }
}

#[test]
fn x680_50_set_arithmetic() {
    for constraint in [
        "(1..5 | 10..15)",
        "(1..5 UNION 10..15)",
        "((1..20) ^ (5..30))",
        "(1..20 INTERSECTION 5..30)",
        "((1..20) EXCEPT 5)",
        "(ALL EXCEPT 5)",
        "(1..10, ...)",
        "(1..10, ..., 20..30)",
        "((1..10), ..., (20..30))",
        "(1..5 | 10..15 ^ 12 EXCEPT 13)",
    ] {
        clean_type(&format!("INTEGER {constraint}"));
    }
}

#[test]
fn x680_50_union_binds_looser_than_intersection() {
    let parse = clean_type("INTEGER (1 | 2 ^ 3)");
    assert_eq!(count(&parse, SyntaxKind::UnionSet), 1);
    assert_eq!(count(&parse, SyntaxKind::IntersectionSet), 1);
}

#[test]
fn x680_49_6_exception_spec_in_constraint() {
    clean_type("INTEGER (1..10, ... ! 5)");
    clean_type("INTEGER (1..10, ... ! Foo : 5)");
}

#[test]
fn x680_51_constraints_can_stack() {
    let parse = clean_type("IA5String (SIZE (1..8)) (FROM (\"a\"..\"z\"))");
    assert_eq!(count(&parse, SyntaxKind::ConstrainedType), 2);
}

#[test]
fn x682_8_1_general_constraints() {
    clean_type("OCTET STRING (CONTAINING Foo)");
    clean_type("OCTET STRING (ENCODED BY { 2 1 1 })");
    clean_type("OCTET STRING (CONTAINING Foo ENCODED BY { 2 1 1 })");
    clean_type("INTEGER (CONSTRAINED BY { -- details -- Foo })");
}

#[test]
fn x682_10_table_constraints() {
    clean_type("MY-CLASS.&Type ({ObjectSet})");
    clean_type("MY-CLASS.&Type ({ObjectSet}{@id})");
    clean_type("MY-CLASS.&Type ({ObjectSet}{@.id, @..up, @...errorId, @a.b})");
    let parse = clean_type("MY-CLASS.&Type ({ObjectSet})");
    assert_eq!(count(&parse, SyntaxKind::TableConstraint), 1);
}

// ---- recovery and robustness ------------------------------------------------------------------

#[test]
fn a_typo_in_one_assignment_does_not_hide_later_errors() {
    let src = module(
        "A ::= SEQUENCE { a INTEGER b BOOLEAN }\nB ::= INTEGER (1..\nC ::= BOOLEAN\nD ::= SEQUENCE { , }",
    );
    let errors = parse(&src).errors().len();
    assert!(
        errors >= 3,
        "expected one error per broken assignment, got {errors}"
    );
}

#[test]
fn recovery_keeps_later_good_assignments_in_the_tree() {
    let parse = parse(&module(
        "A ::= SEQUENCE { a INTEGER b BOOLEAN }\nB ::= INTEGER\nC ::= BOOLEAN",
    ));
    let names: Vec<String> = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::TypeAssignment)
        .filter_map(|n| n.first_token().map(|t| t.text().to_owned()))
        .collect();
    assert!(
        names.contains(&"B".to_owned()) && names.contains(&"C".to_owned()),
        "{names:?}"
    );
}

#[test]
fn unclosed_brace_is_reported_once_and_stops_at_the_next_assignment() {
    let found = module_codes("A ::= INTEGER { a(1)\nB ::= BOOLEAN");
    assert!(!found.is_empty() && found.len() <= 2, "{found:?}");
}

#[test]
fn unclosed_brace_still_stops_at_a_parameterized_assignment() {
    // `B{T} ::=` has balanced braces before its `::=`; it is still the start of the next assignment.
    let parse = parse(&module("A ::= INTEGER { a(1)\nB{T} ::= BOOLEAN"));
    assert_eq!(count(&parse, SyntaxKind::TypeAssignment), 2);
}

#[test]
fn missing_end_is_an_error() {
    assert_eq!(codes("M DEFINITIONS ::= BEGIN A ::= INTEGER"), vec!["E001"]);
}

#[test]
fn garbage_between_modules_is_reported() {
    assert!(!codes("M DEFINITIONS ::= BEGIN END ??? N DEFINITIONS ::= BEGIN END").is_empty());
}

#[test]
fn deeply_nested_input_is_bounded() {
    let nested = format!("T ::= {}INTEGER{}", "SEQUENCE OF ".repeat(5000), "");
    let found = codes(&module(&nested));
    assert!(found.contains(&"E001"));
    let braces = format!("v INTEGER ::= {}{}", "{".repeat(5000), "}".repeat(5000));
    let _ = codes(&module(&braces));
}

#[test]
fn empty_and_whitespace_only_input() {
    assert_clean("");
    assert_clean("  \n\t ");
    assert_clean("-- only a comment");
}

#[test]
fn x680_16_2_xml_value_assignment_is_opaque_and_w137() {
    let src = module(
        "a INTEGER ::= <INTEGER>5</INTEGER>\nb SEQUENCE { x INTEGER } ::= <SEQUENCE><x>1</x></SEQUENCE>\nc NULL ::= <NULL/>\nd BOOLEAN ::= TRUE",
    );
    let parse = parse(&src);
    assert_eq!(
        parse.errors().iter().map(|e| e.code).collect::<Vec<_>>(),
        vec!["W137"; 3]
    );
    assert_eq!(count(&parse, SyntaxKind::XmlValue), 3);
    assert_eq!(count(&parse, SyntaxKind::ValueAssignment), 4);
}
