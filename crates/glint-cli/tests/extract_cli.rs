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
