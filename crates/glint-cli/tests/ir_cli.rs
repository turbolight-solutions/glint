#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use glint_corpus::{
    CorpusData, CorpusInfo, Exports, ModuleRecord, ModuleSource, ParseStatus, SourceKind, SyntaxEra,
};

fn glint() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_glint"));
    command.env_remove("GLINT_CORPUS");
    command.env(
        "GLINT_CACHE_DIR",
        std::env::temp_dir().join(format!("glint-no-cache-{}", std::process::id())),
    );
    command
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glint-ir-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const BASE: &str = "Base { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN
Name ::= UTF8String (SIZE (1..64))
END
";

/// A corpus holding `Base`, published in RFC 1000 and RFC 2000.
fn corpus_file(dir: &Path) -> PathBuf {
    let record = |rfc| ModuleRecord {
        name: "Base".to_owned(),
        oid: Some("1.3.6.1.4.1.99.1".parse().unwrap()),
        source: ModuleSource::rfc(rfc, None, SourceKind::Extracted),
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: BASE.to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    };
    let data = CorpusData {
        info: CorpusInfo {
            build: "2026-11-08.0123abcd".to_owned(),
            notices: Vec::new(),
        },
        modules: vec![record(1000), record(2000)],
        ..CorpusData::default()
    };
    let path = dir.join("corpus.json");
    std::fs::write(&path, data.to_json()).unwrap();
    path
}

const TOP: &str = "Top { 1 3 6 1 4 1 99 3 } DEFINITIONS AUTOMATIC TAGS ::= BEGIN
IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 };
Thing ::= SEQUENCE { name Name, count INTEGER (0..255) }
END
";

#[test]
fn prints_the_ir_of_user_asn1_with_corpus_imports() {
    let dir = scratch("files");
    let corpus = corpus_file(&dir);
    let top = dir.join("top.asn");
    std::fs::write(&top, TOP).unwrap();
    let output = glint()
        .args(["ir", "Top", "Thing"])
        .arg(&top)
        .arg("--corpus")
        .arg(&corpus)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("#0 Thing (Top)"), "{text}");
    assert!(
        text.contains("name: [0] IMPLICIT auto -> #1 Name"),
        "{text}"
    );
    assert!(text.contains("#1 Name (Base, rfc2000)"), "{text}");
    assert!(
        text.contains("UTF8String (SIZE (1..64))  { size=1..64 per: none }"),
        "{text}"
    );
    assert!(!text.contains("notes"), "{text}");
}

#[test]
fn prints_json() {
    let dir = scratch("json");
    let corpus = corpus_file(&dir);
    let top = dir.join("top.asn");
    std::fs::write(&top, TOP).unwrap();
    let output = glint()
        .args(["ir", "Top", "Thing", "--json"])
        .arg(&top)
        .arg("--corpus")
        .arg(&corpus)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["types"][0]["name"], "Thing");
    assert_eq!(json["types"][1]["origin"]["document"], "rfc2000");
}

#[test]
fn takes_a_corpus_module_by_document_and_name() {
    let dir = scratch("corpus");
    let corpus = corpus_file(&dir);
    let output = glint()
        .args(["ir", "rfc2000#Base", "Name", "--corpus"])
        .arg(&corpus)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("#0 Name (Base, rfc2000)"), "{text}");
    // a bare name two documents define is ambiguous
    let output = glint()
        .args(["ir", "Base", "Name", "--corpus"])
        .arg(&corpus)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("rfc1000#Base, rfc2000#Base"), "{stderr}");
}

#[test]
fn works_without_a_corpus_for_user_asn1() {
    let dir = scratch("no-corpus");
    let top = dir.join("top.asn");
    std::fs::write(&top, TOP).unwrap();
    let output = glint()
        .args(["ir", "Top", "Thing"])
        .arg(&top)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("imports from the corpus stay unresolved"),
        "{stderr}"
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("`Name`: the defining module is not available"),
        "{text}"
    );
}

#[test]
fn reports_an_unknown_type_or_module() {
    let dir = scratch("unknown");
    let top = dir.join("top.asn");
    std::fs::write(&top, TOP).unwrap();
    let corpus = corpus_file(&dir);
    for (module, ty, message) in [
        ("Top", "Nope", "`Top` defines no type `Nope`"),
        ("Other", "Thing", "no module `Other` in the given files"),
    ] {
        let output = glint()
            .args(["ir", module, ty])
            .arg(&top)
            .arg("--corpus")
            .arg(&corpus)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{stderr}");
    }
}
