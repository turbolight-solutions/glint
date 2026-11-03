#![cfg(feature = "net-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! The RFC 5280 blind test (design spec, "Blind test: RFC 5280").
//!
//! The document is checked as an author would check it: through the text extractor, against its own Appendix A
//! modules, with an empty corpus. Nothing in this test or in the checker is specific to RFC 5280.

use std::fmt::Write as _;
use std::path::PathBuf;

use glint_check::{CheckOptions, check_document};
use glint_corpus::EmptyCorpus;
use glint_extract::Format;
use sha2::{Digest, Sha256};

const URL: &str = "https://www.rfc-editor.org/rfc/rfc5280.txt";
const SHA256: &str = "a2f2628c0a83b873fc4786abd921f9b2c02395954b655d190bf16b831633345d";

fn cache_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cache")
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn rfc5280() -> String {
    let path = cache_dir().join("rfc5280.txt");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            let body = reqwest::blocking::get(URL)
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.bytes())
                .expect("download RFC 5280");
            std::fs::create_dir_all(cache_dir()).unwrap();
            std::fs::write(&path, &body).unwrap();
            body.to_vec()
        }
    };
    assert_eq!(digest(&bytes), SHA256, "RFC 5280 text changed");
    String::from_utf8(bytes).unwrap()
}

#[test]
fn a_section_4_snippet_diverges_from_appendix_a() {
    let text = rfc5280();
    let report = check_document(
        "rfc5280.txt",
        &text,
        Format::Text,
        &EmptyCorpus::default(),
        &CheckOptions::default(),
    );
    let mut lines = String::new();
    for d in report
        .diagnostics
        .iter()
        .filter(|d| matches!(d.code, "E300" | "W301" | "W302"))
    {
        let _ = writeln!(
            lines,
            "{} line {} ({}): {}",
            d.code,
            d.primary.line_col.0,
            d.primary.section.as_deref().unwrap_or("-"),
            d.message
        );
        for s in &d.secondary {
            let _ = writeln!(
                lines,
                "    -> line {} ({})",
                s.line_col.0,
                s.section.as_deref().unwrap_or("-")
            );
        }
    }
    std::fs::create_dir_all(cache_dir()).unwrap();
    std::fs::write(cache_dir().join("rfc5280-snippet-report.txt"), &lines).unwrap();
    insta::assert_snapshot!("rfc5280_snippet_report", lines);

    let found = report.diagnostics.iter().any(|d| {
        d.code == "E300"
            && d.primary
                .section
                .as_deref()
                .is_some_and(|s| s.starts_with("Section 4"))
            && d.secondary.iter().any(|s| {
                s.section
                    .as_deref()
                    .is_some_and(|s| s.starts_with("Appendix A"))
            })
    });
    assert!(found, "no E300 from Section 4 to Appendix A:\n{lines}");
}
