#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use glint_corpus::{
    CorpusData, CorpusInfo, Exports, ModuleRecord, ModuleSource, OidAssignment, ParseStatus,
    SourceKind, SyntaxEra,
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
    let dir = std::env::temp_dir().join(format!("glint-decode-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(mut command: Command, stdin: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

const MODULE: &str = "Algs { 1 3 6 1 4 1 99 2 } DEFINITIONS AUTOMATIC TAGS ::= BEGIN
id-alg OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 3 }
Pair ::= SEQUENCE { id OBJECT IDENTIFIER, n INTEGER (0..9) }
END
";

/// A corpus holding `Algs` (RFC 1000) and the name of its OID.
fn corpus_file(dir: &Path) -> PathBuf {
    let data = CorpusData {
        info: CorpusInfo {
            build: "2026-11-08.0123abcd".to_owned(),
            notices: Vec::new(),
        },
        modules: vec![ModuleRecord {
            name: "Algs".to_owned(),
            oid: Some("1.3.6.1.4.1.99.2".parse().unwrap()),
            source: ModuleSource::rfc(1000, None, SourceKind::Extracted),
            rfc_status: None,
            obsoleted_by: Vec::new(),
            syntax_era: SyntaxEra::Neutral,
            text: MODULE.to_owned(),
            exports: Exports::All,
            parse_status: ParseStatus::Clean,
            sha256: String::new(),
        }],
        oid_assignments: vec![OidAssignment {
            oid: "1.3.6.1.4.1.99.3".parse().unwrap(),
            name: "id-alg".to_owned(),
            module: "Algs".to_owned(),
            document: "rfc1000".to_owned(),
        }],
        ..CorpusData::default()
    };
    let path = dir.join("corpus.json");
    std::fs::write(&path, data.to_json()).unwrap();
    path
}

/// `Pair { id id-alg, n 5 }`.
const PAIR: [u8; 14] = [
    0x30, 0x0c, 0x80, 0x07, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x63, 0x03, 0x81, 0x01, 0x05,
];

#[test]
fn decodes_der_from_stdin_without_a_schema() {
    let output = run(
        {
            let mut c = glint();
            c.arg("decode");
            c
        },
        &[0x30, 0x03, 0x02, 0x01, 0x05],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("cons: SEQUENCE"), "{text}");
    assert!(text.contains("prim:  INTEGER"), "{text}");
}

// X.690 10.2, 10.3, 11.6 without a schema. A SET cannot be told from a SET OF there, so either order is DER.
#[test]
fn der_rules_apply_without_a_schema() {
    let decode = |bytes: &[u8], strict: bool| {
        let mut command = glint();
        command.arg("decode");
        if strict {
            command.arg("--strict-der");
        }
        run(command, bytes)
    };
    let constructed_string = [0x24, 0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x02, 0x03, 0x04];
    let unsorted_set = [0x31, 0x06, 0x02, 0x01, 0x02, 0x02, 0x01, 0x01];
    for (bytes, expected) in [
        (
            &constructed_string[..],
            "W402 warning: a string in the constructed form",
        ),
        (
            &unsorted_set[..],
            "W402 warning: the elements of a SET are in neither tag order",
        ),
    ] {
        assert!(decode(bytes, false).status.success(), "{bytes:02x?}");
        let output = decode(bytes, true);
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(1), "{bytes:02x?}\n{text}");
        assert!(text.contains(expected), "{text}");
    }
    // [1] then [0] constructed: encoding order only (a SET OF); [0] constructed then [1]: tag order only (a SET)
    for bytes in [
        &[0x31, 0x05, 0x81, 0x01, 0x00, 0xa0, 0x00][..],
        &[0x31, 0x05, 0xa0, 0x00, 0x81, 0x01, 0x00][..],
    ] {
        let output = decode(bytes, true);
        assert!(
            output.status.success(),
            "{bytes:02x?}\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

// X.690 8.1.2.5: the universal types that have only one form keep it without a schema too (as inside an open type
// that no object set resolves).
#[test]
fn universal_types_in_the_wrong_form_are_errors_without_a_schema() {
    let decode = |bytes: &[u8]| {
        let mut command = glint();
        command.arg("decode");
        run(command, bytes)
    };
    for (bytes, expected) in [
        (&[0x21, 0x03, 0x01, 0x01, 0xff][..], "BOOLEAN"),
        (&[0x22, 0x03, 0x02, 0x01, 0x05][..], "INTEGER"),
        (&[0x25, 0x00][..], "NULL"),
        (
            &[0x26, 0x04, 0x06, 0x02, 0x2a, 0x03][..],
            "OBJECT IDENTIFIER",
        ),
        (&[0x30, 0x05, 0x22, 0x03, 0x02, 0x01, 0x05][..], "INTEGER"),
    ] {
        let output = decode(bytes);
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(1), "{bytes:02x?}\n{text}");
        assert!(
            text.contains(&format!(
                "E401 error: a {expected} is encoded in the primitive form"
            )),
            "{text}"
        );
    }
    for (bytes, expected) in [(&[0x10, 0x00][..], "SEQUENCE"), (&[0x11, 0x00][..], "SET")] {
        let output = decode(bytes);
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(1), "{bytes:02x?}\n{text}");
        assert!(
            text.contains(&format!(
                "E401 error: a {expected} is encoded in the constructed form"
            )),
            "{text}"
        );
    }
    // a string may be constructed in BER
    assert!(decode(&[0x24, 0x03, 0x04, 0x01, 0x05]).status.success());
}

#[test]
fn decodes_a_corpus_type_with_oid_names() {
    let dir = scratch("corpus");
    let corpus = corpus_file(&dir);
    let input = dir.join("pair.der");
    std::fs::write(&input, PAIR).unwrap();
    let output = glint()
        .args(["decode", "rfc1000#Algs", "Pair"])
        .arg(&input)
        .arg("--corpus")
        .arg(&corpus)
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("# as rfc1000#Algs.Pair"), "{text}");
    assert!(text.contains("id: 1.3.6.1.4.1.99.3 id-alg"), "{text}");
    assert!(text.contains("n: 5"), "{text}");
}

#[test]
fn constraint_violations_fail_and_der_notes_fail_only_when_strict() {
    let dir = scratch("exit");
    let asn1 = dir.join("algs.asn");
    std::fs::write(&asn1, MODULE).unwrap();
    let decode = |bytes: &[u8], strict: bool| {
        let mut command = glint();
        command
            .args(["decode", "Algs", "Pair", "-", "--file"])
            .arg(&asn1);
        if strict {
            command.arg("--strict-der");
        }
        run(command, bytes)
    };
    assert!(decode(&PAIR, false).status.success());

    let mut out_of_range = PAIR;
    out_of_range[13] = 0x0a;
    let output = decode(&out_of_range, false);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("E407"));

    // the same value with a long-form length: BER, not DER
    let mut long = vec![0x30, 0x81, 0x0c];
    long.extend_from_slice(&PAIR[2..]);
    assert!(decode(&long, false).status.success());
    assert_eq!(decode(&long, true).status.code(), Some(1));
}

/// One object sets `&Params`, the other leaves it unset (an AlgorithmIdentifier's `PARAMS ARE absent`).
const PARAMS_MODULE: &str = "P DEFINITIONS ::= BEGIN
  ALG ::= CLASS { &id OBJECT IDENTIFIER UNIQUE, &Params OPTIONAL } WITH SYNTAX { ID &id [PARAMS &Params] }
  alg-int ALG ::= { ID { 1 2 3 } PARAMS INTEGER }
  alg-none ALG ::= { ID { 1 2 4 } }
  Algs ALG ::= { alg-int | alg-none, ... }
  AlgId ::= SEQUENCE { id ALG.&id({Algs}), params ALG.&Params({Algs}{@id}) OPTIONAL }
END
";

// X.682 10.20 and 10.6 a with X.681 13.4: the key selects the object's row, whose &Params cell is empty when the
// object does not set the field, so no value of the component satisfies the constraint, extensible set or not.
#[test]
fn a_value_for_a_field_the_object_does_not_set_is_an_error() {
    let dir = scratch("params");
    let asn1 = dir.join("p.asn");
    std::fs::write(&asn1, PARAMS_MODULE).unwrap();
    let decode = |bytes: &[u8]| {
        let mut command = glint();
        command
            .args(["decode", "P", "AlgId", "-", "--file"])
            .arg(&asn1);
        run(command, bytes)
    };
    // alg-int with an INTEGER, alg-none without parameters
    assert!(
        decode(&[0x30, 0x07, 0x06, 0x02, 0x2a, 0x03, 0x02, 0x01, 0x05])
            .status
            .success()
    );
    assert!(
        decode(&[0x30, 0x04, 0x06, 0x02, 0x2a, 0x04])
            .status
            .success()
    );
    // alg-none with NULL parameters
    let output = decode(&[0x30, 0x06, 0x06, 0x02, 0x2a, 0x04, 0x05, 0x00]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("W408 error: the object of Algs keyed by 1.2.4 does not set &Params, so this component must be absent"),
        "{text}"
    );
}

#[test]
fn json_and_pem_blocks() {
    let pem = "-----BEGIN FIRST-----\nMAMCAQU=\n-----END FIRST-----\n-----BEGIN SECOND-----\nBQA=\n-----END SECOND-----\n";
    let output = run(
        {
            let mut c = glint();
            c.args(["decode", "--json"]);
            c
        },
        pem.as_bytes(),
    );
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let blocks = json.as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["label"], "FIRST");
    assert_eq!(blocks[0]["encoding"], "pem");
    assert_eq!(blocks[0]["tlvs"][0]["children"][0]["value"]["decimal"], "5");
    assert_eq!(blocks[1]["tlvs"][0]["tag"]["number"], 5);
}

/// `artifacts.json` beside the corpus of `corpus_file`, with `PAIR` as `rfc1000-1`.
fn artifacts_file(dir: &Path) {
    let data = glint_corpus::ArtifactsData {
        build: "2026-11-08.0123abcd".to_owned(),
        artifacts: vec![glint_corpus::Artifact {
            id: "rfc1000-1".to_owned(),
            document: "rfc1000".to_owned(),
            section: Some("3".to_owned()),
            appendix: false,
            title: None,
            caption: None,
            line: 10,
            label: "PAIR".to_owned(),
            sha256: String::new(),
            der: glint_decode::base64_encode(&PAIR),
            root: Some("rfc1000#Algs.Pair".to_owned()),
            confidence: glint_corpus::Confidence::Decoded,
            findings: Vec::new(),
            first_error: None,
            obsoleted: false,
        }],
        ..glint_corpus::ArtifactsData::default()
    };
    std::fs::write(dir.join("artifacts.json"), data.to_json()).unwrap();
}

#[test]
fn artifacts_are_listed_written_and_decoded_as_their_type() {
    let dir = scratch("artifacts");
    let corpus = corpus_file(&dir);
    artifacts_file(&dir);
    let listed = glint()
        .args(["artifacts", "list", "--corpus"])
        .arg(&corpus)
        .output()
        .unwrap();
    let text = String::from_utf8(listed.stdout).unwrap();
    assert!(text.contains("rfc1000-1"), "{text}");
    assert!(text.contains("rfc1000#Algs.Pair"), "{text}");

    let der = glint()
        .args([
            "artifacts",
            "get",
            "rfc1000-1",
            "--format",
            "der",
            "--corpus",
        ])
        .arg(&corpus)
        .output()
        .unwrap();
    assert_eq!(der.stdout, PAIR);
    // A label the label map does not know is written as base64.
    let base64 = glint()
        .args(["artifacts", "get", "rfc1000-1", "--corpus"])
        .arg(&corpus)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(base64.stdout).unwrap().trim(),
        glint_decode::base64_encode(&PAIR)
    );

    let decoded = glint()
        .args(["decode", "--artifact", "rfc1000-1", "--corpus"])
        .arg(&corpus)
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    let text = String::from_utf8(decoded.stdout).unwrap();
    assert!(text.contains("as rfc1000#Algs.Pair"), "{text}");
    assert!(text.contains("id-alg"), "{text}");
}

#[test]
fn artifacts_extract_scans_a_local_document() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../glint-extract/tests/fixtures/pem/paginated.txt");
    let output = glint()
        .args(["artifacts", "extract"])
        .arg(&fixture)
        .output()
        .unwrap();
    // Broken blocks make the exit code 1.
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("CERTIFICATE              line 16, Section 2.1: 536 octets"),
        "{text}"
    );
    assert!(text.contains("elided"), "{text}");
    assert!(text.contains("OpenPGP armor"), "{text}");
}
