#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use glint_check::{CheckOptions, check_document};
use glint_corpus::{Corpus, CorpusProvider, read_bundle};
use glint_extract::Format;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../glint-extract/tests/fixtures")
        .join(name)
}

struct Built {
    dir: PathBuf,
    corpus: Corpus,
    report: String,
}

fn build(tag: &str, with_override: Option<(&str, &str)>) -> Built {
    let dir = std::env::temp_dir().join(format!("glint-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::copy(
        fixture("text/rfc5280-appendix-a.txt"),
        rfcs.join("rfc5280.txt"),
    )
    .unwrap();
    std::fs::copy(fixture("xml/rfc8692.xml"), rfcs.join("rfc8692.xml")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_corpus-build"));
    command
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-03"]);
    if let Some((module, text)) = with_override {
        let over = dir.join("overrides/rfc5280");
        std::fs::create_dir_all(&over).unwrap();
        std::fs::write(over.join(format!("{module}.asn")), text).unwrap();
        std::fs::write(
            over.join("meta.toml"),
            format!(
                "[[override]]\ntier = \"correction\"\nrfc = 5280\nmodule = \"{module}\"\njustification = \"test\"\n"
            ),
        )
        .unwrap();
        command.arg("--overrides").arg(dir.join("overrides"));
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut file = std::fs::File::open(dir.join("out/corpus-2026-11-03.bundle")).unwrap();
    let corpus = Corpus::from_bundle(read_bundle(&mut file).unwrap());
    let report = std::fs::read_to_string(dir.join("out/report.md")).unwrap();
    Built {
        dir,
        corpus,
        report,
    }
}

const DRAFT: &str = "DraftModule { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7) id-mod(0) 999 }
DEFINITIONS EXPLICIT TAGS ::= BEGIN
IMPORTS
  Certificate, AlgorithmIdentifier, id-pe
    FROM PKIX1Explicit88 { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7) id-mod(0) id-pkix1-explicit(18) };
id-pe-mine OBJECT IDENTIFIER ::= { id-pe 1 }
id-pe-fresh OBJECT IDENTIFIER ::= { id-pe 99 }
Mine ::= SEQUENCE { cert Certificate, alg AlgorithmIdentifier }
END
";

#[test]
fn bundle_has_the_modules_and_the_report_records_a_baseline() {
    let built = build("baseline", None);
    let names: Vec<_> = built
        .corpus
        .modules()
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    for expected in [
        "PKIX1Explicit88",
        "PKIX1Implicit88",
        "PKIXAlgsForSHAKE-2019",
    ] {
        assert!(names.contains(&expected), "{names:?}");
    }
    assert!(built.report.contains("Parse rate"), "{}", built.report);
    assert!(
        built.report.contains("| Modules in the bundle | 3 |"),
        "{}",
        built.report
    );
    let _ = std::fs::remove_dir_all(built.dir);
}

#[test]
fn a_draft_importing_pkix_resolves_with_no_user_files() {
    let built = build("resolve", None);
    let report = check_document(
        "draft.asn",
        DRAFT,
        Format::Bare,
        &built.corpus,
        &CheckOptions::default(),
    );
    let codes: Vec<_> = report.diagnostics.iter().map(|d| d.code).collect();
    assert!(!codes.contains(&"W100"), "{codes:?}");
    assert!(!codes.contains(&"E101"), "{codes:?}");
    assert!(!codes.contains(&"E110"), "{codes:?}");
    let import = &report.modules[0].imports[0];
    assert_eq!(
        import.resolved_to.as_deref(),
        Some("rfc5280#PKIX1Explicit88")
    );
    let _ = std::fs::remove_dir_all(built.dir);
}

#[test]
fn an_oid_assigned_in_rfc_5280_triggers_w140() {
    let built = build("w140", None);
    let report = check_document(
        "draft.asn",
        DRAFT,
        Format::Bare,
        &built.corpus,
        &CheckOptions::default(),
    );
    let collisions: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == "W140")
        .collect();
    assert_eq!(
        collisions.len(),
        1,
        "{:?}",
        report
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    assert!(
        collisions[0].message.contains("RFC 5280"),
        "{}",
        collisions[0].message
    );
    let _ = std::fs::remove_dir_all(built.dir);
}

#[test]
fn an_override_replaces_the_extracted_module_and_is_reported() {
    let text =
        "PKIX1Implicit88 { 1 3 6 1 5 5 7 0 19 } DEFINITIONS ::= BEGIN Replaced ::= INTEGER END";
    let built = build("override", Some(("PKIX1Implicit88", text)));
    let record = built
        .corpus
        .modules()
        .iter()
        .find(|m| m.name == "PKIX1Implicit88")
        .unwrap();
    assert!(record.text.contains("Replaced"));
    assert!(built.report.contains("correction"), "{}", built.report);
    let _ = std::fs::remove_dir_all(built.dir);
}
