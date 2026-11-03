#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use glint_check::{CheckOptions, Config, check_document};
use glint_corpus::{
    Bundle, Corpus, EmptyCorpus, Exports, ModuleRecord, ModuleSource, ParseStatus, SourceKind,
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
        source: ModuleSource {
            rfc: Some(1000),
            section: None,
            kind: SourceKind::Extracted,
        },
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: "Base DEFINITIONS ::= BEGIN A ::= INTEGER A ::= BOOLEAN END".to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    };
    let corpus = Corpus::from_bundle(Bundle {
        modules: vec![record],
        ..Bundle::default()
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
        source: ModuleSource {
            rfc: Some(rfc),
            section: None,
            kind: SourceKind::Extracted,
        },
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: format!("Base DEFINITIONS ::= BEGIN {body} END"),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    };
    let corpus = Corpus::from_bundle(Bundle {
        modules: vec![make(1, "Old ::= INTEGER"), make(2, "New ::= INTEGER")],
        ..Bundle::default()
    });
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Old FROM Base; T ::= Old END";
    let options = CheckOptions {
        pins: vec![("Base".to_owned(), "rfc1".to_owned())],
        ..CheckOptions::default()
    };
    let report = check_document("d.asn", text, Format::Bare, &corpus, &options);
    assert!(!codes(&report).contains(&"E101"), "{:?}", codes(&report));
}
