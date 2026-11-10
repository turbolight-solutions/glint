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
    let dir = std::env::temp_dir().join(format!("glint-closure-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn corpus_file(dir: &Path) -> PathBuf {
    let record = ModuleRecord {
        name: "Base".to_owned(),
        oid: Some("1.3.6.1.4.1.99.1".parse().unwrap()),
        source: ModuleSource::rfc(1000, None, SourceKind::Extracted),
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: "Base { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN Name ::= INTEGER END".to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    };
    let data = CorpusData {
        info: CorpusInfo {
            build: "2026-11-08.0123abcd".to_owned(),
            notices: Vec::new(),
        },
        modules: vec![record],
        ..CorpusData::default()
    };
    let path = dir.join("corpus.json");
    std::fs::write(&path, data.to_json()).unwrap();
    path
}

const TOP: &str = "Top { 1 3 6 1 4 1 99 3 } DEFINITIONS ::= BEGIN
IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 };
Thing ::= SEQUENCE { name Name }
END
";

/// The names in `dir`, sorted.
fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn closure_writes_one_file_per_module_with_the_order_and_sources() {
    let dir = scratch("ok");
    let corpus = corpus_file(&dir);
    let top = dir.join("top.asn");
    std::fs::write(&top, TOP).unwrap();
    let out = dir.join("out");
    let output = glint()
        .arg("closure")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&top)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_eq!(stderr, "");
    assert_eq!(entries(&out), ["combined.asn", "modules", "modules.tsv"]);
    assert_eq!(entries(&out.join("modules")), ["Base.asn", "Top.asn"]);
    let table = std::fs::read_to_string(out.join("modules.tsv")).unwrap();
    assert_eq!(
        table,
        format!(
            "Base\tdependency\trfc1000#Base\nTop\tinput\t{}\n",
            top.display()
        )
    );
    assert_eq!(
        std::fs::read_to_string(out.join("modules/Top.asn")).unwrap(),
        TOP
    );
    let combined = std::fs::read_to_string(out.join("combined.asn")).unwrap();
    assert!(combined.contains("Base { 1 3 6 1 4 1 99 1 } DEFINITIONS"));
    assert!(combined.ends_with(TOP), "{combined}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_dependency_with_the_name_of_an_input_is_written_under_a_name_of_its_own() {
    let dir = scratch("same-name");
    let corpus = corpus_file(&dir);
    let base = dir.join("base.asn");
    std::fs::write(
        &base,
        "Base { 1 3 6 1 4 1 99 2 } DEFINITIONS ::= BEGIN\nOther ::= BOOLEAN\nEND\n",
    )
    .unwrap();
    let top = dir.join("top.asn");
    std::fs::write(
        &top,
        "Top { 1 3 6 1 4 1 99 3 } DEFINITIONS ::= BEGIN\nIMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 }\n\
         Other FROM Base { 1 3 6 1 4 1 99 2 };\nThing ::= SEQUENCE { name Name, other Other }\nEND\n",
    )
    .unwrap();
    let out = dir.join("out");
    let output = glint()
        .arg("closure")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&base)
        .arg(&top)
        .arg("--rename")
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains(
            "rfc1000#Base: renamed to Base-rfc1000, since the set needs another module named Base"
        ),
        "{stderr}"
    );
    assert_eq!(
        entries(&out.join("modules")),
        ["Base-rfc1000.asn", "Base.asn", "Top.asn"]
    );
    let table = std::fs::read_to_string(out.join("modules.tsv")).unwrap();
    assert!(
        table.contains("Base-rfc1000\tdependency\trfc1000#Base\n"),
        "{table}"
    );
    let top = std::fs::read_to_string(out.join("modules/Top.asn")).unwrap();
    assert!(
        top.contains("FROM Base-rfc1000 { 1 3 6 1 4 1 99 1 }"),
        "{top}"
    );
    assert!(top.contains("FROM Base { 1 3 6 1 4 1 99 2 }"), "{top}");
    let dependency = std::fs::read_to_string(out.join("modules/Base-rfc1000.asn")).unwrap();
    assert!(
        dependency.starts_with("Base-rfc1000 { 1 3 6 1 4 1 99 1 }"),
        "{dependency}"
    );
    let combined = std::fs::read_to_string(out.join("combined.asn")).unwrap();
    assert!(
        combined.contains(
            "-- Modules renamed, since the set needs two modules of their name:\n--   rfc1000#Base: Base -> Base-rfc1000"
        ),
        "{combined}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn without_rename_two_modules_of_one_name_exit_one_and_say_how_to_fix_it() {
    let dir = scratch("same-name-error");
    let corpus = corpus_file(&dir);
    let base = dir.join("base.asn");
    std::fs::write(
        &base,
        "Base { 1 3 6 1 4 1 99 2 } DEFINITIONS ::= BEGIN\nOther ::= BOOLEAN\nEND\n",
    )
    .unwrap();
    let top = dir.join("top.asn");
    std::fs::write(
        &top,
        "Top { 1 3 6 1 4 1 99 3 } DEFINITIONS ::= BEGIN\nIMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 }\n\
         Other FROM Base { 1 3 6 1 4 1 99 2 };\nThing ::= SEQUENCE { name Name, other Other }\nEND\n",
    )
    .unwrap();
    let out = dir.join("out");
    let output = glint()
        .arg("closure")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&base)
        .arg(&top)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "glint: error: the set needs 2 modules named Base ({}, rfc1000#Base), and a compiler that finds \
             modules by name takes one; --rename gives each module a name of its own",
            base.display()
        )),
        "{stderr}"
    );
    assert!(!out.exists(), "nothing is written");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn closure_starts_from_a_corpus_module() {
    let dir = scratch("corpus-ref");
    let corpus = corpus_file(&dir);
    let out = dir.join("out");
    let output = glint()
        .arg("closure")
        .arg("--corpus")
        .arg(&corpus)
        .args(["--module", "rfc1000#Base", "-o"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(output.status.success());
    let table = std::fs::read_to_string(out.join("modules.tsv")).unwrap();
    assert_eq!(table, "Base\tinput\trfc1000#Base\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_incomplete_set_exits_one_with_one_line_and_writes_nothing() {
    let dir = scratch("missing");
    let corpus = corpus_file(&dir);
    let top = dir.join("top.asn");
    std::fs::write(
        &top,
        "Top DEFINITIONS ::= BEGIN IMPORTS X FROM Gone { 1 9 9 }; Y ::= X END\n",
    )
    .unwrap();
    let out = dir.join("out");
    let output = glint()
        .arg("closure")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&top)
        .args(["--module", "rfc2000#Nothing", "-o"])
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let errors: Vec<&str> = stderr
        .lines()
        .filter(|l| l.starts_with("glint: error:"))
        .collect();
    assert_eq!(errors.len(), 1, "{stderr}");
    assert!(
        errors[0].contains("rfc2000#Nothing: not in the corpus"),
        "{stderr}"
    );
    assert!(!out.exists(), "nothing is written");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_import_is_named_with_its_identifier() {
    let dir = scratch("gone");
    let corpus = corpus_file(&dir);
    let top = dir.join("top.asn");
    std::fs::write(
        &top,
        "Top DEFINITIONS ::= BEGIN IMPORTS X FROM Gone { 1 9 9 }; Y ::= X END\n",
    )
    .unwrap();
    let output = glint()
        .arg("closure")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&top)
        .arg("-o")
        .arg(dir.join("out"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("glint: error: Gone (not found; FROM { 1 9 9 }; imported by Top)"),
        "{stderr}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_directory_that_is_not_empty_is_refused() {
    let dir = scratch("busy");
    let corpus = corpus_file(&dir);
    let top = dir.join("top.asn");
    std::fs::write(&top, TOP).unwrap();
    let output = glint()
        .arg("closure")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&top)
        .arg("-o")
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not empty"));
    std::fs::remove_dir_all(&dir).unwrap();
}
