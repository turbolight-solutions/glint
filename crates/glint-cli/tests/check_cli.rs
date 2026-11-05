#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::process::Command;

use glint_corpus::{
    CorpusData, CorpusInfo, Exports, ModuleRecord, ModuleSource, OidAssignment, ParseStatus,
    SourceKind, SyntaxEra,
};

const BUILD: &str = "2026-11-05.0123abcd";

/// `glint` with no corpus data unless a test gives some: no `GLINT_CORPUS`, and an empty cache.
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
    let dir = std::env::temp_dir().join(format!("glint-check-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn corpus_file(dir: &std::path::Path) -> PathBuf {
    let record = ModuleRecord {
        name: "Base".to_owned(),
        oid: Some("1.3.6.1.4.1.99.1".parse().unwrap()),
        source: ModuleSource::rfc(1000, Some("Appendix A".to_owned()), SourceKind::Extracted),
        rfc_status: Some("proposed standard".to_owned()),
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: "Base { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN Name ::= INTEGER id-base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 } END".to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: "abc".to_owned(),
    };
    let data = CorpusData {
        info: CorpusInfo {
            build: BUILD.to_owned(),
            notices: Vec::new(),
        },
        modules: vec![record],
        oid_assignments: vec![OidAssignment {
            oid: "1.3.6.1.4.1.99".parse().unwrap(),
            name: "id-base".to_owned(),
            module: "Base".to_owned(),
            document: "rfc1000".to_owned(),
        }],
        ..CorpusData::default()
    };
    let path = dir.join("corpus.json");
    std::fs::write(&path, data.to_json()).unwrap();
    path
}

#[test]
fn clean_document_exits_zero() {
    let dir = scratch("clean");
    let corpus = corpus_file(&dir);
    let file = dir.join("ok.asn");
    std::fs::write(&file, "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 }; T ::= SEQUENCE { n Name } END").unwrap();
    let output = glint()
        .arg("check")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn errors_exit_one_and_print_the_code() {
    let dir = scratch("errors");
    let corpus = corpus_file(&dir);
    let file = dir.join("bad.asn");
    std::fs::write(
        &file,
        "M DEFINITIONS ::= BEGIN T ::= SEQUENCE { n Missing } END",
    )
    .unwrap();
    let output = glint()
        .args(["check", "--color", "never", "--corpus"])
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("E110"));
}

#[test]
fn warnings_fail_only_with_deny_warnings() {
    let dir = scratch("warn");
    let corpus = corpus_file(&dir);
    let file = dir.join("warn.asn");
    std::fs::write(
        &file,
        "M DEFINITIONS ::= BEGIN IMPORTS X FROM Nowhere; T ::= X END",
    )
    .unwrap();
    let plain = glint()
        .arg("check")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(plain.status.code(), Some(0));
    let strict = glint()
        .args(["check", "--deny-warnings", "--corpus"])
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(strict.status.code(), Some(1));
}

#[test]
fn json_output_goes_to_stdout() {
    let dir = scratch("json");
    let corpus = corpus_file(&dir);
    let file = dir.join("bad.asn");
    std::fs::write(
        &file,
        "M DEFINITIONS ::= BEGIN T ::= SEQUENCE { n Missing } END",
    )
    .unwrap();
    let output = glint()
        .args(["check", "--format", "json", "--corpus"])
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.to_string().contains("E110"));
}

#[test]
fn missing_file_exits_two() {
    let output = glint()
        .args(["check", "/nonexistent/file.asn"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn config_file_is_found_above_the_input() {
    let dir = scratch("config");
    let corpus = corpus_file(&dir);
    std::fs::write(dir.join("glint.toml"), "[severity]\nE110 = \"off\"\n").unwrap();
    let file = dir.join("bad.asn");
    std::fs::write(
        &file,
        "M DEFINITIONS ::= BEGIN T ::= SEQUENCE { n Missing } END",
    )
    .unwrap();
    let output = glint()
        .arg("check")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn pin_flag_is_validated() {
    let dir = scratch("pin");
    let file = dir.join("x.asn");
    std::fs::write(&file, "M DEFINITIONS ::= BEGIN END").unwrap();
    let output = glint()
        .args(["check", "--pin", "nonsense"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn corpus_list_show_and_oid() {
    let dir = scratch("corpus");
    let corpus = corpus_file(&dir);
    let list = glint()
        .args(["corpus", "--corpus"])
        .arg(&corpus)
        .arg("list")
        .output()
        .unwrap();
    let text = String::from_utf8(list.stdout).unwrap();
    assert!(
        text.contains("rfc1000") && text.contains("Base") && text.contains("1.3.6.1.4.1.99.1"),
        "{text}"
    );
    let filtered = glint()
        .args(["corpus", "--corpus"])
        .arg(&corpus)
        .args(["list", "--filter", "zzz"])
        .output()
        .unwrap();
    assert!(filtered.stdout.is_empty());
    let show = glint()
        .args(["corpus", "--corpus"])
        .arg(&corpus)
        .args(["show", "rfc1000"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8(show.stdout)
            .unwrap()
            .contains("DEFINITIONS ::= BEGIN")
    );
    let oid = glint()
        .args(["corpus", "--corpus"])
        .arg(&corpus)
        .args(["oid", "1.3.6.1.4.1.99.5"])
        .output()
        .unwrap();
    let text = String::from_utf8(oid.stdout).unwrap();
    assert!(text.contains("id-base"), "{text}");
}

#[test]
fn sarif_output_goes_to_stdout_with_rules_and_regions() {
    let dir = scratch("sarif");
    let corpus = corpus_file(&dir);
    let file = dir.join("bad.asn");
    std::fs::write(
        &file,
        "M DEFINITIONS ::= BEGIN T ::= SEQUENCE { n Missing } END",
    )
    .unwrap();
    let output = glint()
        .args(["check", "--format", "sarif", "--corpus"])
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sarif["version"], "2.1.0");
    let result = &sarif["runs"][0]["results"][0];
    assert_eq!(result["ruleId"], "E110");
    assert_eq!(result["level"], "error");
    assert_eq!(
        result["locations"][0]["physicalLocation"]["region"]["startLine"],
        1
    );
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["rules"][0]["id"], "E110");
    assert!(
        sarif["runs"][0]["tool"]["driver"]["rules"][0]["help"]["markdown"]
            .as_str()
            .unwrap()
            .contains("not defined")
    );
}

#[test]
fn sarif_with_several_files_is_one_run() {
    let dir = scratch("sarif-many");
    let corpus = corpus_file(&dir);
    let one = dir.join("one.asn");
    let two = dir.join("two.asn");
    for file in [&one, &two] {
        std::fs::write(
            file,
            "M DEFINITIONS ::= BEGIN T ::= SEQUENCE { n Missing } END",
        )
        .unwrap();
    }
    let output = glint()
        .args(["check", "--format", "sarif", "--corpus"])
        .arg(&corpus)
        .arg(&one)
        .arg(&two)
        .output()
        .unwrap();
    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sarif["runs"].as_array().unwrap().len(), 1);
    assert_eq!(sarif["runs"][0]["results"].as_array().unwrap().len(), 2);
}

#[test]
fn check_needs_a_file_or_a_draft() {
    let output = glint().arg("check").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_bad_draft_name_exits_two_without_the_network() {
    let output = glint()
        .args(["check", "--draft", "not-a-draft"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("draft name"));
}

#[test]
fn explain_describes_a_code_with_examples() {
    let output = glint().args(["explain", "e110"]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("E110 (error)"), "{text}");
    assert!(text.contains("basis: 14.4"), "{text}");
    assert!(text.contains("T ::= SEQUENCE { n Missing }"), "{text}");
}

#[test]
fn explain_without_a_code_lists_them_and_an_unknown_code_fails() {
    let list = glint().arg("explain").output().unwrap();
    let text = String::from_utf8(list.stdout).unwrap();
    assert!(text.contains("E300") && text.contains("W100"));
    let unknown = glint().args(["explain", "E999"]).output().unwrap();
    assert_eq!(unknown.status.code(), Some(2));
}

#[test]
fn corpus_update_installs_data_that_later_runs_use() {
    let dir = scratch("update");
    let source = corpus_file(&dir);
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("corpus.bundle"), "old").unwrap();
    let update = glint()
        .env("GLINT_CACHE_DIR", &cache)
        .args(["corpus", "update", "--from"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        update.status.success(),
        "{}",
        String::from_utf8_lossy(&update.stderr)
    );
    assert!(cache.join("corpus.json").is_file());
    assert!(
        !cache.join("corpus.bundle").exists(),
        "the cache file of old versions is removed"
    );
    let list = glint()
        .env("GLINT_CACHE_DIR", &cache)
        .args(["corpus", "list"])
        .output()
        .unwrap();
    assert!(String::from_utf8(list.stdout).unwrap().contains("Base"));
}

#[test]
fn corpus_update_refuses_a_file_that_is_not_corpus_data() {
    let dir = scratch("update-bad");
    let junk = dir.join("junk.json");
    std::fs::write(&junk, "nope").unwrap();
    let cache = dir.join("cache");
    let update = glint()
        .env("GLINT_CACHE_DIR", &cache)
        .args(["corpus", "update", "--from"])
        .arg(&junk)
        .output()
        .unwrap();
    assert_eq!(update.status.code(), Some(2));
    assert!(!cache.join("corpus.json").exists());
}

#[test]
fn without_data_check_fails_and_says_how_to_get_it() {
    let dir = scratch("no-data");
    let file = dir.join("ok.asn");
    std::fs::write(&file, "M DEFINITIONS ::= BEGIN T ::= INTEGER END").unwrap();
    let output = glint().arg("check").arg(&file).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("glint corpus update"), "{stderr}");
}

#[test]
fn every_output_names_the_code_version_and_the_data_build() {
    let dir = scratch("versions");
    let corpus = corpus_file(&dir);
    let file = dir.join("ok.asn");
    std::fs::write(&file, "M DEFINITIONS ::= BEGIN T ::= INTEGER END").unwrap();
    let human = glint()
        .arg("check")
        .arg("--corpus")
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&human.stderr);
    let line = format!("glint {}, data {BUILD}", env!("CARGO_PKG_VERSION"));
    assert!(stderr.contains(&line), "{stderr}");
    let json = glint()
        .args(["check", "--format", "json", "--corpus"])
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(json["data_build"], BUILD);
    assert_eq!(json["glint_version"], env!("CARGO_PKG_VERSION"));
    let sarif = glint()
        .args(["check", "--format", "sarif", "--corpus"])
        .arg(&corpus)
        .arg(&file)
        .output()
        .unwrap();
    let sarif: serde_json::Value = serde_json::from_slice(&sarif.stdout).unwrap();
    assert_eq!(sarif["runs"][0]["properties"]["glintDataBuild"], BUILD);
}
