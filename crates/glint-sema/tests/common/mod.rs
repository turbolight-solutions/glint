#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use glint_corpus::{
    Bundle, Corpus, Exports, ModuleRecord, ModuleSource, Oid, ParseStatus, SourceKind, SyntaxEra,
};
use glint_sema::{DraftSource, Finding, Options, analyze};
use glint_syntax::{ParseOptions, parse_source_with};

pub struct Rec<'a> {
    pub name: &'a str,
    pub oid: Option<&'a str>,
    pub rfc: u32,
    pub text: &'a str,
}

pub fn corpus(records: &[Rec<'_>]) -> Corpus {
    let modules = records
        .iter()
        .map(|r| ModuleRecord {
            name: r.name.to_owned(),
            oid: r.oid.map(|o| o.parse::<Oid>().unwrap()),
            source: ModuleSource {
                rfc: Some(r.rfc),
                section: None,
                kind: SourceKind::Extracted,
            },
            rfc_status: None,
            obsoleted_by: Vec::new(),
            syntax_era: SyntaxEra::Neutral,
            text: r.text.to_owned(),
            exports: Exports::All,
            parse_status: ParseStatus::Clean,
            sha256: String::new(),
        })
        .collect();
    Corpus::from_bundle(Bundle {
        modules,
        ..Bundle::default()
    })
}

pub fn check_with(text: &str, corpus: &Corpus, options: &Options) -> Vec<Finding> {
    let parse = parse_source_with(text, ParseOptions::default());
    let source = DraftSource {
        text: text.to_owned(),
        parse,
    };
    analyze(vec![source], corpus, options).findings
}

pub fn check(text: &str) -> Vec<Finding> {
    check_with(text, &corpus(&[]), &Options::default())
}

pub fn codes(findings: &[Finding]) -> Vec<&'static str> {
    findings.iter().map(|f| f.code).collect()
}

pub fn has(findings: &[Finding], code: &str) -> bool {
    findings.iter().any(|f| f.code == code)
}

pub fn without_notes(findings: Vec<Finding>) -> Vec<Finding> {
    findings
        .into_iter()
        .filter(|f| !f.code.starts_with('I'))
        .collect()
}
