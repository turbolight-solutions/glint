use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../glint-extract/tests/fixtures")
        .join(name)
}

fn glint() -> Command {
    Command::new(env!("CARGO_BIN_EXE_glint"))
}

#[test]
fn extract_lists_blocks_without_an_output_directory() {
    let output = glint()
        .arg("extract")
        .arg(fixture("text/rfc5280-appendix-a.txt"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout).unwrap();
    assert!(
        listing.contains("PKIX1Explicit88  Appendix A.1"),
        "{listing}"
    );
    assert!(
        listing.contains("PKIX1Implicit88  Appendix A.2"),
        "{listing}"
    );
}

#[test]
fn extract_writes_one_file_per_module() {
    let dir = std::env::temp_dir().join(format!("glint-extract-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let output = glint()
        .arg("extract")
        .arg(fixture("xml/rfc8692.xml"))
        .arg("-o")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    let module = std::fs::read_to_string(dir.join("PKIXAlgsForSHAKE-2019.asn")).unwrap();
    assert!(module.contains("&smimeCaps"));
    assert!(module.ends_with("END\n"));
    let files = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(files, 1, "fragments are only written with --fragments");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn extract_reports_findings_with_exit_status() {
    let dir = std::env::temp_dir().join(format!("glint-extract-cli-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.xml");
    std::fs::write(
        &path,
        "<rfc><middle><sourcecode type=\"asn.1\">30 82 01</sourcecode><<",
    )
    .unwrap();
    let output = glint().arg("extract").arg(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let findings = String::from_utf8(output.stderr).unwrap();
    assert!(findings.contains("W201"), "{findings}");
    assert!(findings.contains("E203"), "{findings}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn extract_lists_blocks_as_json_with_module_names_identifiers_and_lines() {
    let path = fixture("text/rfc5280-appendix-a.txt");
    let output = glint()
        .args(["extract", "--format", "json"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listing["schema_version"], 1);
    let modules: Vec<&serde_json::Value> = listing["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|block| block["modules"].as_array().unwrap())
        .collect();
    let names: Vec<&str> = modules
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["PKIX1Explicit88", "PKIX1Implicit88"]);
    assert_eq!(modules[0]["oid"], "1.3.6.1.5.5.7.0.18");
    assert_eq!(modules[1]["oid"], "1.3.6.1.5.5.7.0.19");
    let source = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    for module in &modules {
        let first = usize::try_from(module["first_line"].as_u64().unwrap()).unwrap();
        let last = usize::try_from(module["last_line"].as_u64().unwrap()).unwrap();
        let name = module["name"].as_str().unwrap();
        assert!(
            lines[first - 1].trim_start().starts_with(name),
            "{name} starts on line {first}"
        );
        assert_eq!(lines[last - 1].trim(), "END", "{name} ends on line {last}");
        assert!(module["file"].is_null(), "nothing is written without -o");
        assert!(
            module.get("text").is_none(),
            "the text is in the files -o writes"
        );
    }
}

#[test]
fn a_header_without_numeric_arcs_has_no_identifier_in_the_json_listing() {
    let dir = std::env::temp_dir().join(format!("glint-extract-json-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("draft.asn");
    std::fs::write(
        &path,
        "A { 1 2 id-mod-a(TBD) } DEFINITIONS ::= BEGIN END\n\nB DEFINITIONS ::= BEGIN END\n",
    )
    .unwrap();
    let output = glint()
        .args(["extract", "--format", "json"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let modules = listing["blocks"][0]["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 2);
    assert!(modules.iter().all(|m| m["oid"].is_null()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn json_with_an_output_directory_names_the_file_of_each_module() {
    let dir = std::env::temp_dir().join(format!("glint-extract-json-out-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("twins.asn");
    std::fs::write(
        &path,
        "  T { 1 2 3 } DEFINITIONS ::= BEGIN A ::= INTEGER END\n\n  T { 1 2 4 } DEFINITIONS ::= BEGIN B ::= BOOLEAN END\n",
    )
    .unwrap();
    let out = dir.join("out");
    let output = glint()
        .args(["extract", "--format", "json", "-o"])
        .arg(&out)
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let modules: Vec<&serde_json::Value> = listing["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|block| block["modules"].as_array().unwrap())
        .collect();
    let files: Vec<(&str, &str)> = modules
        .iter()
        .map(|m| (m["oid"].as_str().unwrap(), m["file"].as_str().unwrap()))
        .collect();
    assert_eq!(files, [("1.2.3", "T.asn"), ("1.2.4", "T-2.asn")]);
    assert_eq!(
        std::fs::read_to_string(out.join("T.asn")).unwrap(),
        "T { 1 2 3 } DEFINITIONS ::= BEGIN A ::= INTEGER END\n",
        "trimmed at both ends, as the corpus build records it"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("T-2.asn")).unwrap(),
        "T { 1 2 4 } DEFINITIONS ::= BEGIN B ::= BOOLEAN END\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn json_with_fragments_names_the_file_of_each_fragment() {
    let dir = std::env::temp_dir().join(format!("glint-extract-json-frag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let output = glint()
        .args(["extract", "--format", "json", "--fragments", "-o"])
        .arg(&dir)
        .arg(fixture("xml/rfc8692.xml"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let listing: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let blocks = listing["blocks"].as_array().unwrap();
    let fragments: Vec<&str> = blocks
        .iter()
        .filter(|block| block["modules"].as_array().unwrap().is_empty())
        .map(|block| block["file"].as_str().unwrap())
        .collect();
    assert!(!fragments.is_empty());
    for file in &fragments {
        assert!(dir.join(file).is_file(), "{file}");
    }
    assert!(
        blocks
            .iter()
            .filter(|block| !block["modules"].as_array().unwrap().is_empty())
            .all(|block| block["file"].is_null())
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
