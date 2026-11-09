#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use glint_check::{CheckOptions, Config, check_document};
use glint_corpus::{
    Corpus, CorpusData, EmptyCorpus, Exports, ModuleRecord, ModuleSource, ParseStatus, SourceKind,
    SyntaxEra,
};
use glint_diag::Severity;
use glint_extract::Format;

fn codes(report: &glint_check::Report) -> Vec<&'static str> {
    report.diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn a_text_document_reports_positions_in_the_document() {
    let text = "Title\n\n1. Intro\n\n   Some text.\n\n   M DEFINITIONS ::= BEGIN\n     T ::= SEQUENCE { a Missing }\n   END\n";
    let report = check_document(
        "d.txt",
        text,
        Format::Text,
        &EmptyCorpus::default(),
        &CheckOptions::default(),
    );
    assert_eq!(codes(&report), ["E110"]);
    let label = &report.diagnostics[0].primary;
    assert_eq!(label.line_col.0, 8);
    assert_eq!(&text[label.span.clone()], "Missing");
    assert_eq!(label.section.as_deref(), Some("Section 1"));
}

#[test]
fn syntax_errors_are_reported_at_the_document_position() {
    let text = "M DEFINITIONS ::= BEGIN\n T ::= SEQUENCE { a INTEGER,, }\nEND\n";
    let report = check_document(
        "d.asn",
        text,
        Format::Bare,
        &EmptyCorpus::default(),
        &CheckOptions::default(),
    );
    assert!(codes(&report).contains(&"E001"));
}

#[test]
fn severity_can_be_remapped_or_switched_off() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS X FROM Nowhere; END";
    let plain = check_document(
        "d.asn",
        text,
        Format::Bare,
        &EmptyCorpus::default(),
        &CheckOptions::default(),
    );
    assert_eq!(codes(&plain), ["W112", "W100"]);
    let config = Config::from_toml("[severity]\nW112 = \"off\"\nW100 = \"error\"\n").unwrap();
    let options = CheckOptions {
        config,
        ..CheckOptions::default()
    };
    let report = check_document(
        "d.asn",
        text,
        Format::Bare,
        &EmptyCorpus::default(),
        &options,
    );
    assert_eq!(codes(&report), ["W100"]);
    assert_eq!(report.diagnostics[0].severity, Severity::Error);
}

#[test]
fn unknown_configuration_keys_are_rejected() {
    assert!(Config::from_toml("[nonsense]\nx = 1\n").is_err());
}

#[test]
fn missing_symbols_in_a_corpus_module_are_reported() {
    let record = ModuleRecord {
        name: "Base".to_owned(),
        oid: None,
        source: ModuleSource::rfc(1000, None, SourceKind::Extracted),
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: "Base DEFINITIONS ::= BEGIN A ::= INTEGER A ::= BOOLEAN END".to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    };
    let corpus = Corpus::from_data(CorpusData {
        modules: vec![record],
        ..CorpusData::default()
    });
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Gone FROM Base; T ::= Gone END";
    let report = check_document(
        "d.asn",
        text,
        Format::Bare,
        &corpus,
        &CheckOptions::default(),
    );
    assert!(codes(&report).contains(&"E101"));
}

#[test]
fn pins_from_the_command_line_apply() {
    let make = |rfc: u32, body: &str| ModuleRecord {
        name: "Base".to_owned(),
        oid: None,
        source: ModuleSource::rfc(rfc, None, SourceKind::Extracted),
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: format!("Base DEFINITIONS ::= BEGIN {body} END"),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    };
    let corpus = Corpus::from_data(CorpusData {
        modules: vec![make(1, "Old ::= INTEGER"), make(2, "New ::= INTEGER")],
        ..CorpusData::default()
    });
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Old FROM Base; T ::= Old END";
    let options = CheckOptions {
        pins: vec![("Base".to_owned(), "rfc1".to_owned())],
        ..CheckOptions::default()
    };
    let report = check_document("d.asn", text, Format::Bare, &corpus, &options);
    assert!(!codes(&report).contains(&"E101"), "{:?}", codes(&report));
}

// X.680 12.2.2, 31.2.5; X.208 (1988) has no UTF8String
#[test]
fn a_published_1988_string_type_definition_is_not_reported() {
    let body = "DEFINITIONS IMPLICIT TAGS ::= BEGIN\n UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING\nEND\n";
    let check = |text: &str| {
        check_document(
            "d.asn",
            text,
            Format::Bare,
            &EmptyCorpus::default(),
            &CheckOptions::default(),
        )
    };
    let listed = check(&format!(
        "EnrollmentMessageSyntax {{ iso(1) identified-organization(3) dod(6) internet(1) security(5) \
         mechanisms(5) pkix(7) id-mod(0) id-mod-cmc2002(23) }}\n{body}"
    ));
    assert_eq!(codes(&listed), Vec::<&str>::new());
    let new = check(&format!("New-2026 {{ 1 3 6 1 5 5 7 0 999 }}\n{body}"));
    assert_eq!(codes(&new), ["E011", "E123"]);
    let e123 = new.diagnostics.iter().find(|d| d.code == "E123").unwrap();
    assert_eq!(e123.severity, Severity::Error);
}
