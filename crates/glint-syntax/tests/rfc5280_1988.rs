mod common;
use common::*;
use glint_syntax::{ParseOptions, Severity, SyntaxKind, parse_source_with};

const EXPLICIT: &str = include_str!("fixtures/rfc5280/PKIX1Explicit88.asn");
const IMPLICIT: &str = include_str!("fixtures/rfc5280/PKIX1Implicit88.asn");

fn assert_clean_corpus(src: &str) -> glint_syntax::Parse {
    let parse = parse_source_with(src, ParseOptions::corpus());
    assert_eq!(parse.syntax().text().to_string(), src);
    let errors: Vec<_> = parse
        .errors()
        .iter()
        .filter(|e| e.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    parse
}

fn assignment_names(parse: &glint_syntax::Parse) -> Vec<String> {
    parse
        .syntax()
        .descendants()
        .filter(|n| {
            matches!(
                n.kind(),
                SyntaxKind::TypeAssignment
                    | SyntaxKind::ValueAssignment
                    | SyntaxKind::ValueSetAssignment
            )
        })
        .filter_map(|n| n.first_token().map(|t| t.text().to_owned()))
        .collect()
}

#[test]
fn pkix1_explicit88_parses_with_zero_errors() {
    let parse = assert_clean_corpus(EXPLICIT);
    assert_eq!(count(&parse, SyntaxKind::ModuleDefinition), 1);
    let names = assignment_names(&parse);
    for expected in [
        "Certificate",
        "TBSCertificate",
        "AlgorithmIdentifier",
        "Extension",
        "Name",
        "id-pkix",
        "ub-name",
    ] {
        assert!(names.iter().any(|n| n == expected), "missing {expected}");
    }
    assert!(
        count(&parse, SyntaxKind::AnyType) >= 1,
        "ANY DEFINED BY must parse as legacy ANY"
    );
}

#[test]
fn pkix1_implicit88_parses_with_zero_errors() {
    let parse = assert_clean_corpus(IMPLICIT);
    assert_eq!(count(&parse, SyntaxKind::ModuleDefinition), 1);
    let names = assignment_names(&parse);
    for expected in [
        "AuthorityKeyIdentifier",
        "KeyUsage",
        "GeneralName",
        "BasicConstraints",
    ] {
        assert!(names.iter().any(|n| n == expected), "missing {expected}");
    }
}

#[test]
fn both_modules_in_one_block() {
    let both = format!("{EXPLICIT}\n{IMPLICIT}");
    let parse = assert_clean_corpus(&both);
    assert_eq!(count(&parse, SyntaxKind::ModuleDefinition), 2);
}

#[test]
fn imports_of_the_implicit_module_parse_with_assigned_identifiers() {
    let parse = assert_clean_corpus(IMPLICIT);
    assert!(count(&parse, SyntaxKind::SymbolsFromModule) >= 1);
    assert!(count(&parse, SyntaxKind::AssignedIdentifier) >= 1);
}

#[test]
fn later_edition_names_defined_by_the_1988_module_are_warnings_in_corpus_mode() {
    let parse = parse_source_with(EXPLICIT, ParseOptions::corpus());
    let warnings: Vec<_> = parse.errors().iter().filter(|e| e.code == "E011").collect();
    assert!(!warnings.is_empty());
    assert!(warnings.iter().all(|e| e.severity == Severity::Warning));
}

#[test]
fn the_same_text_is_an_error_when_checked_as_a_draft() {
    let parse = glint_syntax::parse_source(EXPLICIT);
    assert!(
        parse
            .errors()
            .iter()
            .any(|e| e.code == "E011" && e.severity == Severity::Error)
    );
}

#[test]
fn shadowed_names_parse_as_defined_types() {
    let parse = assert_clean_corpus(EXPLICIT);
    let uses = parse
        .syntax()
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::DefinedType && n.text() == "UTF8String")
        .count();
    assert!(
        uses >= 1,
        "uses of the module's own UTF8String must be DefinedType"
    );
}
