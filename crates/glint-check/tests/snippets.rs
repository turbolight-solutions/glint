#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! The synthetic mismatch suite for snippet consistency (design spec, M5).

use glint_check::{CheckOptions, Config, Report, check_document};
use glint_corpus::{
    Bundle, Corpus, EmptyCorpus, Exports, ModuleRecord, ModuleSource, ParseStatus, SourceKind,
    SyntaxEra,
};
use glint_extract::Format;

const SNIPPET_CODES: [&str; 6] = ["E300", "W301", "W302", "I303", "W304", "W310"];

/// A Markdown draft: the module in one block, the snippet in another.
fn draft(header: &str, body: &str, snippet: &str) -> String {
    format!(
        "# Draft\n\n## 1. The module\n\n```asn.1\nM DEFINITIONS {header} ::= BEGIN\n{body}\nEND\n```\n\n## 2. Text\n\n```asn.1\n{snippet}\n```\n"
    )
}

fn run(text: &str) -> Report {
    check_document(
        "d.md",
        text,
        Format::Markdown,
        &EmptyCorpus::default(),
        &CheckOptions::default(),
    )
}

fn snippet_findings(report: &Report) -> Vec<(&'static str, String)> {
    report
        .diagnostics
        .iter()
        .filter(|d| SNIPPET_CODES.contains(&d.code))
        .map(|d| (d.code, d.message.clone()))
        .collect()
}

fn findings(header: &str, body: &str, snippet: &str) -> Vec<(&'static str, String)> {
    snippet_findings(&run(&draft(header, body, snippet)))
}

fn only<'a>(findings: &'a [(&'static str, String)]) -> (&'static str, &'a str) {
    assert_eq!(findings.len(), 1, "{findings:?}");
    (findings[0].0, findings[0].1.as_str())
}

const CERT: &str = "Cert ::= SEQUENCE {
    version [0] EXPLICIT Version DEFAULT v1,
    serial INTEGER,
    issuer Name OPTIONAL,
    id OBJECT IDENTIFIER,
    names SEQUENCE SIZE (1..MAX) OF UTF8String (SIZE (1..64))
}
Version ::= INTEGER { v1(0), v2(1) }
Name ::= UTF8String";

#[test]
fn identical_text_has_no_findings() {
    let snippet = "Cert ::= SEQUENCE {
    version [0] EXPLICIT Version DEFAULT v1, -- a comment
    serial INTEGER,
    issuer Name OPTIONAL,
    id OBJECT IDENTIFIER,
    names SEQUENCE SIZE (1..MAX) OF UTF8String (SIZE (1..64))
}";
    assert_eq!(findings("", CERT, snippet), []);
}

#[test]
fn a_renamed_component_is_an_error_naming_both() {
    let snippet = CERT.replace("serial INTEGER", "serialNumber INTEGER");
    let snippet = snippet.split("Version ::=").next().unwrap().to_owned();
    let found = findings("", CERT, &snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(
        message.contains("serialNumber") && message.contains("serial"),
        "{message}"
    );
}

#[test]
fn a_changed_tag_is_an_error() {
    let snippet = CERT
        .replace("[0] EXPLICIT", "[1] EXPLICIT")
        .split("Version ::=")
        .next()
        .unwrap()
        .to_owned();
    let found = findings("", CERT, &snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(
        message.contains("tag is `[1]`") && message.contains("`[0]`"),
        "{message}"
    );
}

#[test]
fn a_dropped_optional_is_an_error_saying_what_changed() {
    let snippet = CERT
        .replace("issuer Name OPTIONAL", "issuer Name")
        .split("Version ::=")
        .next()
        .unwrap()
        .to_owned();
    let found = findings("", CERT, &snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(
        message.contains("component `issuer` is `mandatory` here but `OPTIONAL` in the module"),
        "{message}"
    );
}

#[test]
fn a_changed_default_is_an_error() {
    let snippet = CERT
        .replace("DEFAULT v1", "DEFAULT v2")
        .split("Version ::=")
        .next()
        .unwrap()
        .to_owned();
    let found = findings("", CERT, &snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(
        message.contains("`DEFAULT v2` here but `DEFAULT v1`"),
        "{message}"
    );
}

#[test]
fn a_changed_constraint_is_an_error() {
    let snippet = CERT
        .replace("SIZE (1..64)", "SIZE (1..32)")
        .split("Version ::=")
        .next()
        .unwrap()
        .to_owned();
    let found = findings("", CERT, &snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(message.contains("constraint"), "{message}");
}

#[test]
fn elided_components_without_partial_are_an_error() {
    let snippet = "Cert ::= SEQUENCE {
    version [0] EXPLICIT Version DEFAULT v1,
    id OBJECT IDENTIFIER
}";
    let found = findings("", CERT, snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(message.contains("missing here"), "{message}");
}

#[test]
fn the_partial_directive_allows_a_subset_in_order() {
    let snippet = "-- glint: partial
Cert ::= SEQUENCE {
    version [0] EXPLICIT Version DEFAULT v1,
    id OBJECT IDENTIFIER
}";
    assert_eq!(findings("", CERT, snippet), []);
}

#[test]
fn a_partial_snippet_in_the_wrong_order_is_still_an_error() {
    let snippet = "-- glint: partial
Cert ::= SEQUENCE {
    id OBJECT IDENTIFIER,
    version [0] EXPLICIT Version DEFAULT v1
}";
    let found = findings("", CERT, snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(message.contains("out of order"), "{message}");
}

#[test]
fn a_partial_snippet_cannot_invent_components() {
    let snippet = "-- glint: partial
Cert ::= SEQUENCE {
    version [0] EXPLICIT Version DEFAULT v1,
    extra BOOLEAN
}";
    let found = findings("", CERT, snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(
        message.contains("`extra` is not in the module"),
        "{message}"
    );
}

#[test]
fn an_elision_comment_makes_the_enclosing_construct_partial() {
    let snippet = "Cert ::= SEQUENCE {
    version [0] EXPLICIT Version DEFAULT v1,
    -- (snip)
    id OBJECT IDENTIFIER
}";
    assert_eq!(findings("", CERT, snippet), []);
}

#[test]
fn elision_markers_come_from_the_configuration() {
    let snippet = "Cert ::= SEQUENCE {
    version [0] EXPLICIT Version DEFAULT v1,
    -- and so on
    id OBJECT IDENTIFIER
}";
    let text = draft("", CERT, snippet);
    let strict = run(&text);
    assert_eq!(snippet_findings(&strict)[0].0, "E300");
    let config = Config::from_toml("[snippets]\nelision_comments = [\"-- and so on\"]\n").unwrap();
    let options = CheckOptions {
        config,
        ..CheckOptions::default()
    };
    let report = check_document(
        "d.md",
        &text,
        Format::Markdown,
        &EmptyCorpus::default(),
        &options,
    );
    assert_eq!(snippet_findings(&report), []);
}

#[test]
fn an_elision_comment_only_affects_its_own_construct() {
    let snippet = "Cert ::= SEQUENCE {
    version [0] EXPLICIT Version DEFAULT v1,
    serial INTEGER,
    issuer Name OPTIONAL,
    id OBJECT IDENTIFIER,
    names SEQUENCE SIZE (1..MAX) OF UTF8String (SIZE (1..64))
}
Version ::= INTEGER { v1(0), -- ...
    v3(2) }";
    let found = findings("", CERT, snippet);
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(message.contains("v3(2)"), "{message}");
}

#[test]
fn reordered_alternatives_are_a_warning() {
    let found = findings(
        "",
        "C ::= CHOICE { a INTEGER, b BOOLEAN, c NULL }",
        "C ::= CHOICE { c NULL, a INTEGER, b BOOLEAN }",
    );
    assert_eq!(only(&found).0, "W302");
}

#[test]
fn reordered_set_components_are_a_warning() {
    let found = findings(
        "",
        "S ::= SET { a INTEGER, b BOOLEAN }",
        "S ::= SET { b BOOLEAN, a INTEGER }",
    );
    assert_eq!(only(&found).0, "W302");
}

#[test]
fn reordered_sequence_components_are_an_error() {
    let found = findings(
        "",
        "S ::= SEQUENCE { a INTEGER, b BOOLEAN }",
        "S ::= SEQUENCE { b BOOLEAN, a INTEGER }",
    );
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(message.contains("different order"), "{message}");
}

#[test]
fn an_oid_written_with_names_instead_of_numbers_is_a_warning() {
    let found = findings(
        "",
        "id-a OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) 6 1 }",
        "id-a OBJECT IDENTIFIER ::= { 1 3 6 1 }",
    );
    assert_eq!(only(&found).0, "W302");
    let wrong = findings(
        "",
        "id-a OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) 6 1 }",
        "id-a OBJECT IDENTIFIER ::= { 1 3 6 2 }",
    );
    assert_eq!(only(&wrong).0, "E300");
}

#[test]
fn an_oid_built_on_a_defined_value_is_compared_by_number() {
    let body = "id-a OBJECT IDENTIFIER ::= { 1 3 6 1 }\nid-b OBJECT IDENTIFIER ::= { id-a 4 }";
    let found = findings("", body, "id-b OBJECT IDENTIFIER ::= { 1 3 6 1 4 }");
    assert_eq!(only(&found).0, "W302");
}

#[test]
fn spelling_out_implicit_under_implicit_tags_is_a_warning() {
    let body = "T ::= SEQUENCE { a [0] INTEGER, b [1] IMPLICIT BOOLEAN }";
    let snippet = "T ::= SEQUENCE { a [0] IMPLICIT INTEGER, b [1] BOOLEAN }";
    let found = findings("IMPLICIT TAGS", body, snippet);
    assert_eq!(only(&found).0, "W302");
    let different = findings("EXPLICIT TAGS", body, snippet);
    assert_eq!(only(&different).0, "E300");
}

#[test]
fn explicit_and_implicit_are_different_when_the_default_is_explicit() {
    let found = findings(
        "EXPLICIT TAGS",
        "T ::= SEQUENCE { a [0] EXPLICIT INTEGER }",
        "T ::= SEQUENCE { a [0] IMPLICIT INTEGER }",
    );
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(message.contains("tagging"), "{message}");
}

#[test]
fn the_two_sequence_of_size_forms_are_a_warning() {
    let found = findings(
        "",
        "L ::= SEQUENCE SIZE (1..MAX) OF INTEGER",
        "L ::= SEQUENCE (SIZE (1..MAX)) OF INTEGER",
    );
    assert_eq!(only(&found).0, "W302");
}

#[test]
fn enumeration_numbers_and_order_do_not_matter_to_the_normal_form() {
    let found = findings(
        "",
        "E ::= ENUMERATED { b, a, c(5), d }",
        "E ::= ENUMERATED { a(1), b(0), d(2), c(5) }",
    );
    assert_eq!(only(&found).0, "W302");
    let changed = findings(
        "",
        "E ::= ENUMERATED { b, a, c(5), d }",
        "E ::= ENUMERATED { a(1), b(0), d(3), c(5) }",
    );
    assert_eq!(only(&changed).0, "E300");
}

#[test]
fn integer_named_numbers_are_compared_by_name_and_value() {
    let found = findings(
        "",
        "V ::= INTEGER { v2(1), v1(0) }",
        "V ::= INTEGER { v1(0), v2(1) }",
    );
    assert_eq!(only(&found).0, "W302");
    let changed = findings(
        "",
        "V ::= INTEGER { v2(1), v1(0) }",
        "V ::= INTEGER { v1(0), v2(2) }",
    );
    assert_eq!(only(&changed).0, "E300");
}

#[test]
fn extension_parts_move_to_the_end() {
    let found = findings(
        "",
        "S ::= SEQUENCE { a INTEGER, ..., b BOOLEAN, ..., c NULL }",
        "S ::= SEQUENCE { a INTEGER, c NULL, ..., b BOOLEAN }",
    );
    assert_eq!(only(&found).0, "W302");
}

#[test]
fn extensibility_implied_adds_the_marker() {
    let found = findings(
        "EXTENSIBILITY IMPLIED",
        "S ::= SEQUENCE { a INTEGER, ... }",
        "S ::= SEQUENCE { a INTEGER }",
    );
    assert_eq!(only(&found).0, "W302");
    let without = findings(
        "",
        "S ::= SEQUENCE { a INTEGER, ... }",
        "S ::= SEQUENCE { a INTEGER }",
    );
    assert_eq!(only(&without).0, "E300");
}

#[test]
fn components_of_is_expanded() {
    let body =
        "Base ::= SEQUENCE { a INTEGER, b BOOLEAN }\nD ::= SEQUENCE { COMPONENTS OF Base, c NULL }";
    let found = findings("", body, "D ::= SEQUENCE { a INTEGER, b BOOLEAN, c NULL }");
    assert_eq!(only(&found).0, "W302");
}

#[test]
fn automatic_tagging_makes_tags_explicit_in_the_normal_form() {
    let body = "T ::= SEQUENCE { a INTEGER, b BOOLEAN }";
    let spelled = "T ::= SEQUENCE { a [0] IMPLICIT INTEGER, b [1] IMPLICIT BOOLEAN }";
    let found = findings("AUTOMATIC TAGS", body, spelled);
    assert_eq!(only(&found).0, "W302");
    let swapped = "T ::= SEQUENCE { a [1] IMPLICIT INTEGER, b [0] IMPLICIT BOOLEAN }";
    let different = findings("AUTOMATIC TAGS", body, swapped);
    assert_eq!(only(&different).0, "E300");
}

#[test]
fn under_automatic_tagging_the_order_of_alternatives_is_part_of_the_meaning() {
    let found = findings(
        "AUTOMATIC TAGS",
        "C ::= CHOICE { a INTEGER, b BOOLEAN }",
        "C ::= CHOICE { b BOOLEAN, a INTEGER }",
    );
    assert_eq!(only(&found).0, "E300");
}

#[test]
fn a_value_set_or_type_defined_as_something_else_is_an_error() {
    let found = findings("", "T ::= INTEGER\nv T ::= 5", "v INTEGER ::= 5");
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(
        message.contains("`T`") || message.contains("INTEGER"),
        "{message}"
    );
    let kind = findings("", "X ::= INTEGER", "X INTEGER ::= { 1 }");
    assert!(only(&kind).1.contains("value set assignment"), "{kind:?}");
}

#[test]
fn a_name_that_is_in_no_module_is_a_warning() {
    let found = findings("", CERT, "Stranger ::= INTEGER");
    let (code, message) = only(&found);
    assert_eq!(code, "W301");
    assert!(message.contains("Stranger"));
}

#[test]
fn matching_one_of_several_modules_passes_and_says_which() {
    let text = "# Draft\n\n```asn.1\nA DEFINITIONS ::= BEGIN T ::= INTEGER END\n```\n\n```asn.1\nB DEFINITIONS ::= BEGIN T ::= BOOLEAN END\n```\n\nText:\n\n```asn.1\nT ::= BOOLEAN\n```\n";
    let found = snippet_findings(&run(text));
    let (code, message) = only(&found);
    assert_eq!(code, "I303");
    assert!(message.contains('B'), "{message}");
}

#[test]
fn matching_none_of_several_modules_reports_the_closest() {
    let text = "# Draft\n\n```asn.1\nA DEFINITIONS ::= BEGIN T ::= SEQUENCE { a INTEGER, b BOOLEAN } END\n```\n\n```asn.1\nB DEFINITIONS ::= BEGIN T ::= SEQUENCE { a INTEGER, b NULL } END\n```\n\nText:\n\n```asn.1\nT ::= SEQUENCE { a INTEGER, b OCTET STRING }\n```\n";
    let found = snippet_findings(&run(text));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "E300");
}

#[test]
fn the_ignore_directive_skips_the_block() {
    let found = findings(
        "",
        CERT,
        "-- glint: ignore\nCert ::= INTEGER\nStranger ::= INTEGER",
    );
    assert_eq!(found, []);
}

#[test]
fn the_module_directive_compares_only_against_that_module() {
    let text = "# Draft\n\n```asn.1\nA DEFINITIONS ::= BEGIN T ::= INTEGER END\n```\n\n```asn.1\nB DEFINITIONS ::= BEGIN T ::= BOOLEAN END\n```\n\nText:\n\n```asn.1\n-- glint: module=A\nT ::= BOOLEAN\n```\n";
    let found = snippet_findings(&run(text));
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(message.contains("in A"), "{message}");
}

#[test]
fn a_directive_for_a_missing_module_or_an_unknown_directive_is_a_warning() {
    let found = findings("", CERT, "-- glint: module=Nowhere\nCert ::= INTEGER");
    assert_eq!(only(&found).0, "W304");
    let unknown = findings("", "T ::= INTEGER", "-- glint: frobnicate\nT ::= INTEGER");
    assert_eq!(only(&unknown).0, "W304");
}

#[test]
fn syntax_errors_in_a_snippet_are_reported_and_that_assignment_is_not_compared() {
    let report = run(&draft(
        "",
        "T ::= INTEGER",
        "T ::= SEQUENCE { a INTEGER,, }",
    ));
    assert!(report.diagnostics.iter().any(|d| d.code == "E001"));
    assert_eq!(snippet_findings(&report), []);
}

#[test]
fn snippets_can_be_switched_off() {
    let text = draft("", "T ::= INTEGER", "T ::= BOOLEAN");
    let options = CheckOptions {
        no_snippets: true,
        ..CheckOptions::default()
    };
    let report = check_document(
        "d.md",
        &text,
        Format::Markdown,
        &EmptyCorpus::default(),
        &options,
    );
    assert_eq!(snippet_findings(&report), []);
}

#[test]
fn the_label_points_into_the_snippet_and_the_module() {
    let text = draft(
        "",
        "T ::= SEQUENCE { a INTEGER }",
        "T ::= SEQUENCE { a BOOLEAN }",
    );
    let report = run(&text);
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "E300")
        .unwrap();
    assert_eq!(&text[diagnostic.primary.span.clone()], "BOOLEAN");
    let secondary = &diagnostic.secondary[0];
    assert_eq!(&text[secondary.span.clone()], "INTEGER");
}

fn record(name: &str, rfc: u32, body: &str) -> ModuleRecord {
    ModuleRecord {
        name: name.to_owned(),
        oid: None,
        source: ModuleSource::rfc(rfc, None, SourceKind::Extracted),
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: format!("{name} DEFINITIONS ::= BEGIN {body} END"),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    }
}

#[test]
fn a_snippet_is_checked_against_a_corpus_module_the_draft_imports() {
    let corpus = Corpus::from_bundle(Bundle {
        modules: vec![record(
            "Base",
            1000,
            "Item ::= SEQUENCE { a INTEGER, b BOOLEAN }",
        )],
        ..Bundle::default()
    });
    let module = "M DEFINITIONS ::= BEGIN IMPORTS Item FROM Base; T ::= Item END";
    let make =
        |snippet: &str| format!("# D\n\n```asn.1\n{module}\n```\n\n```asn.1\n{snippet}\n```\n");
    let good = make("Item ::= SEQUENCE { a INTEGER, b BOOLEAN }");
    let report = check_document(
        "d.md",
        &good,
        Format::Markdown,
        &corpus,
        &CheckOptions::default(),
    );
    assert_eq!(snippet_findings(&report), []);
    let bad = make("Item ::= SEQUENCE { a INTEGER, b NULL }");
    let report = check_document(
        "d.md",
        &bad,
        Format::Markdown,
        &corpus,
        &CheckOptions::default(),
    );
    let found = snippet_findings(&report);
    assert_eq!(only(&found).0, "E300");
    assert!(found[0].1.contains("rfc1000#Base"), "{found:?}");
    let label = &report
        .diagnostics
        .iter()
        .find(|d| d.code == "E300")
        .unwrap()
        .secondary[0];
    assert_ne!(label.file, report.document);
}

#[test]
fn a_snippet_is_checked_against_any_corpus_module_when_nothing_closer_defines_it() {
    let corpus = Corpus::from_bundle(Bundle {
        modules: vec![record("Far", 2000, "Quoted ::= INTEGER (0..10)")],
        ..Bundle::default()
    });
    let text = draft("", "T ::= INTEGER", "Quoted ::= INTEGER (0..11)");
    let report = check_document(
        "d.md",
        &text,
        Format::Markdown,
        &corpus,
        &CheckOptions::default(),
    );
    let found = snippet_findings(&report);
    assert_eq!(only(&found).0, "E300");
    let same = draft("", "T ::= INTEGER", "Quoted ::= INTEGER (0..10)");
    let report = check_document(
        "d.md",
        &same,
        Format::Markdown,
        &corpus,
        &CheckOptions::default(),
    );
    assert_eq!(snippet_findings(&report), []);
}

#[test]
fn an_oid_that_cannot_be_resolved_in_the_module_is_not_an_error() {
    let found = findings(
        "",
        "id-a OBJECT IDENTIFIER ::= { iso(1) 3 6 1 5 5 7 20 }",
        "id-a OBJECT IDENTIFIER ::= { id-pkix 20 }",
    );
    assert_eq!(only(&found).0, "W302");
}

#[test]
fn a_parameterized_definition_quoted_without_its_parameters_is_an_error() {
    let found = findings(
        "",
        "List { T } ::= SEQUENCE OF T",
        "List ::= SEQUENCE OF INTEGER",
    );
    let (code, message) = only(&found);
    assert_eq!(code, "E300");
    assert!(message.contains("parameters"), "{message}");
}

#[test]
fn the_reverse_check_reports_types_the_text_never_mentions() {
    let text = "# Draft\n\n```asn.1\nM DEFINITIONS ::= BEGIN Shown ::= INTEGER Mentioned ::= INTEGER Hidden ::= INTEGER END\n```\n\nThe Mentioned type is simple.\n\n```asn.1\nShown ::= INTEGER\n```\n";
    let quiet = run(text);
    assert!(!quiet.diagnostics.iter().any(|d| d.code == "W310"));
    let config = Config::from_toml("[snippets]\nreverse_check = true\n").unwrap();
    let options = CheckOptions {
        config,
        ..CheckOptions::default()
    };
    let report = check_document(
        "d.md",
        text,
        Format::Markdown,
        &EmptyCorpus::default(),
        &options,
    );
    let found = snippet_findings(&report);
    let (code, message) = only(&found);
    assert_eq!(code, "W310");
    assert!(message.contains("Hidden"), "{message}");
}

#[test]
fn syntax_errors_in_unmarked_text_fragments_are_warnings() {
    let text = "Title\n\n1. Module\n\n   M DEFINITIONS ::= BEGIN\n     T ::= INTEGER\n   END\n\n2. Example\n\n   Looks like this:\n\n   Foo ::= SEQUENCE {\n     a INTEGER,,\n   }\n\n   End.\n";
    let report = check_document(
        "d.txt",
        text,
        Format::Text,
        &EmptyCorpus::default(),
        &CheckOptions::default(),
    );
    let error = report
        .diagnostics
        .iter()
        .find(|d| d.code == "E001")
        .unwrap();
    assert_eq!(error.severity, glint_diag::Severity::Warning);
}

#[test]
fn corpus_modules_with_errors_are_not_used_for_the_whole_corpus_search() {
    let mut broken = record("Far", 2000, "Quoted ::= INTEGER (0..10)");
    broken.parse_status = ParseStatus::Errors;
    let corpus = Corpus::from_bundle(Bundle {
        modules: vec![broken],
        ..Bundle::default()
    });
    let text = draft("", "T ::= INTEGER", "Quoted ::= INTEGER (0..11)");
    let report = check_document(
        "d.md",
        &text,
        Format::Markdown,
        &corpus,
        &CheckOptions::default(),
    );
    assert_eq!(only(&snippet_findings(&report)).0, "W301");
}

#[test]
fn a_same_named_but_unrelated_corpus_definition_is_not_a_match() {
    let corpus = Corpus::from_bundle(Bundle {
        modules: vec![record("Far", 2000, "Quoted ::= SEQUENCE { a INTEGER }")],
        ..Bundle::default()
    });
    let text = draft("", "T ::= INTEGER", "Quoted ::= INTEGER");
    let report = check_document(
        "d.md",
        &text,
        Format::Markdown,
        &corpus,
        &CheckOptions::default(),
    );
    assert_eq!(only(&snippet_findings(&report)).0, "W301");
    let related = draft("", "T ::= INTEGER", "Quoted ::= SEQUENCE { a BOOLEAN }");
    let report = check_document(
        "d.md",
        &related,
        Format::Markdown,
        &corpus,
        &CheckOptions::default(),
    );
    assert_eq!(only(&snippet_findings(&report)).0, "E300");
}

#[test]
fn damaged_snippets_never_panic() {
    let full = CERT.to_owned()
        + "\nC ::= CHOICE { a [0] INTEGER, ..., b [[ 2: NULL ]] }\nE ::= ENUMERATED { x, y(5), ..., z }\nid OBJECT IDENTIFIER ::= { iso(1) 3 6 }\nL{T} ::= SEQUENCE (SIZE (1..4)) OF T";
    for end in (0..full.len()).filter(|&i| full.is_char_boundary(i)) {
        let snippet = &full[..end];
        let text = draft("AUTOMATIC TAGS", &full, snippet);
        let _ = run(&text);
        let text = draft("", &full, &format!("-- glint: partial\n{snippet}"));
        let _ = run(&text);
    }
    for cut in (0..full.len())
        .step_by(7)
        .filter(|&i| full.is_char_boundary(i))
    {
        let mut damaged = full.clone();
        damaged.replace_range(cut..cut + 1, "}");
        let _ = run(&draft("IMPLICIT TAGS", &full, &damaged));
    }
}

#[test]
fn a_missing_name_is_only_info_when_the_draft_imports_from_an_unavailable_module() {
    let text = "# D\n\n```asn.1\nM DEFINITIONS ::= BEGIN IMPORTS X FROM Elsewhere; T ::= X END\n```\n\n```asn.1\nStranger ::= INTEGER\n```\n";
    let report = run(text);
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == "W301")
        .unwrap();
    assert_eq!(diagnostic.severity, glint_diag::Severity::Info);
    assert!(
        diagnostic.message.contains("Elsewhere"),
        "{}",
        diagnostic.message
    );
}
