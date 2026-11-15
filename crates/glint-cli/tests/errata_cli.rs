#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! `glint errata` over a mirror of two fixture RFCs: RFC 5280's appendix (text) and RFC 8692 (XML, with its published
//! text).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use glint_corpus::{Pointer, build_dir, latest_path};

struct Mirror {
    root: PathBuf,
    corpus_json: PathBuf,
}

impl Mirror {
    fn corpus(&self) -> PathBuf {
        self.root.join("corpus")
    }

    fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    fn errata(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_glint"))
            .env_remove("GLINT_CORPUS")
            .arg("errata")
            .arg("--corpus-dir")
            .arg(self.corpus())
            .arg("--cache-dir")
            .arg(self.cache())
            .arg("--corpus")
            .arg(&self.corpus_json)
            .args(args)
            .output()
            .unwrap()
    }

    /// `errata` run from the mirror's root with relative `--corpus-dir` and `--cache-dir`, as with the defaults.
    fn errata_relative(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_glint"))
            .env_remove("GLINT_CORPUS")
            .current_dir(&self.root)
            .args([
                "errata",
                "--corpus-dir",
                "corpus",
                "--cache-dir",
                "cache",
                "--corpus",
            ])
            .arg(&self.corpus_json)
            .args(args)
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.errata(args);
        assert!(
            output.status.success(),
            "{args:?}: {}{}",
            stdout(&output),
            stderr(&output)
        );
        stdout(&output)
    }

    fn work(&self, name: &str) -> PathBuf {
        self.cache().join("errata-work").join(name)
    }

    fn edit_work(&self, name: &str, from: &str, to: &str) {
        let path = self.work(name);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches(from).count(), 1, "{from}");
        std::fs::write(&path, text.replace(from, to)).unwrap();
    }

    fn ledger(&self) -> String {
        std::fs::read_to_string(self.corpus().join("errata/ledger.tsv")).unwrap_or_default()
    }

    fn patch(&self, rfc: u32, id: u32) -> Option<String> {
        std::fs::read_to_string(
            self.corpus()
                .join(format!("errata/patches/rfc{rfc}/{id}.patch")),
        )
        .ok()
    }

    fn text_patch(&self, rfc: u32, id: u32) -> Option<String> {
        std::fs::read_to_string(
            self.corpus()
                .join(format!("errata/patches/rfc{rfc}/{id}.txt.patch")),
        )
        .ok()
    }

    /// `record ... --decision patched`, expected to fail; its stderr.
    fn record_fails(&self, rfc: &str, id: &str) -> String {
        let output = self.errata(&["record", rfc, id, "--decision", "patched", "--note", "x"]);
        assert!(!output.status.success(), "{}", stdout(&output));
        stderr(&output)
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../glint-extract/tests/fixtures")
        .join(name)
}

const FEED: &str = r#"[
 {"errata_id":"9001","doc-id":"RFC5280","errata_status_code":"Verified","section":"Appendix A.1",
  "orig_text":"ub-name INTEGER ::= 32768","correct_text":"ub-name INTEGER ::= 32769",
  "notes":"Sam Submitter (sam@example.com) noticed it.","submitter_name":"Sam Submitter","verifier_name":null},
 {"errata_id":"9002","doc-id":"RFC5280","errata_status_code":"Verified","section":"Appendix A.1 (ask sam@example.com)",
  "orig_text":"ub-common-name INTEGER ::= 64","correct_text":"ub-common-name INTEGER ::= 65","notes":null},
 {"errata_id":"9003","doc-id":"RFC5280","errata_status_code":"Reported","section":"1",
  "orig_text":"ub-organization-name INTEGER ::= 64","correct_text":"x","notes":null},
 {"errata_id":"9004","doc-id":"RFC8692","errata_status_code":"Verified","section":"4.1",
  "orig_text":"signatureValue       BIT STRING  }","correct_text":"signatureValue BIT STRING (SIZE (1..MAX)) }","notes":null},
 {"errata_id":"9006","doc-id":"RFC8692","errata_status_code":"Verified","section":"4.1",
  "orig_text":"signatureAlgorithm   AlgorithmIdentifier,","correct_text":"signatureAlgorithm   AlgorithmIdentifier{{SigAlgs}},","notes":null}
]"#;

fn mirror(tag: &str) -> Mirror {
    let root = std::env::temp_dir().join(format!("glint-errata-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let rfcs = root.join("cache/rfc");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::create_dir_all(root.join("corpus/errata")).unwrap();
    std::fs::copy(
        fixture("text/rfc5280-appendix-a.txt"),
        rfcs.join("rfc5280.txt"),
    )
    .unwrap();
    std::fs::copy(fixture("xml/rfc8692.xml"), rfcs.join("rfc8692.xml")).unwrap();
    std::fs::copy(fixture("xml/rfc8692.txt"), rfcs.join("rfc8692.txt")).unwrap();
    std::fs::write(root.join("cache/errata.json"), FEED).unwrap();
    let out = root.join("built");
    glint_corpus_build::run(&glint_corpus_build::Settings {
        rfc_dir: rfcs,
        drafts_dir: None,
        overrides: None,
        out: out.clone(),
        date: "2026-11-11".to_owned(),
        only: Vec::new(),
        dump_findings: None,
        sources: None,
        cache: root.join("cache/files"),
        errata: None,
        errata_dir: None,
        iana: None,
    })
    .unwrap();
    let pointer = Pointer::from_json(&std::fs::read(out.join(latest_path())).unwrap()).unwrap();
    let corpus_json = out
        .join(build_dir(&pointer.build))
        .join(glint_corpus::CORPUS_FILE);
    Mirror { root, corpus_json }
}

fn patch_9001(m: &Mirror) {
    m.ok(&["edit", "5280", "9001"]);
    m.edit_work(
        "rfc5280-9001.txt",
        "ub-name INTEGER ::= 32768",
        "ub-name INTEGER ::= 32769",
    );
    m.ok(&[
        "record",
        "5280",
        "9001",
        "--decision",
        "patched",
        "--note",
        "ub-name in PKIX1Explicit88",
    ]);
}

#[test]
fn queue_lists_verified_errata_of_rfcs_with_asn1() {
    let m = mirror("queue");
    let out = m.ok(&["queue"]);
    assert!(
        out.starts_with("4 errata awaiting triage (2 RFCs in scope)"),
        "{out}"
    );
    for line in [
        "rfc5280 9001 new",
        "rfc5280 9002 new",
        "rfc8692 9004 new",
        "rfc8692 9006 new",
    ] {
        assert!(out.contains(line), "{line}: {out}");
    }
    assert!(
        !out.contains("9003"),
        "Reported errata are not triaged: {out}"
    );
    assert!(out.contains("Appendix A.1 (ask [email])"), "{out}");
    assert!(!out.contains("sam@example.com"), "{out}");
    let one = m.ok(&["queue", "--rfc", "8692"]);
    assert!(
        one.starts_with("2 errata awaiting triage for rfc8692"),
        "{one}"
    );
}

#[test]
fn show_redacts_names_and_places_the_erratum() {
    let m = mirror("show");
    let out = m.ok(&["show", "5280", "9001"]);
    assert!(out.contains("\"[name] ([email]) noticed it.\""), "{out}");
    assert!(
        !out.contains("Sam Submitter") && !out.contains("sam@example.com"),
        "{out}"
    );
    assert!(out.contains("in module PKIX1Explicit88"), "{out}");
    assert!(out.contains("\"ub-name INTEGER ::= 32768\""), "{out}");
    assert!(out.contains("9002 \"Verified\" untriaged"), "{out}");
    let inline = m.ok(&["show", "8692", "9004", "--no-excerpt"]);
    assert!(inline.contains("in inline ASN.1"), "{inline}");
    assert!(inline.contains("excerpt not shown"), "{inline}");
}

#[test]
fn list_shows_every_erratum_of_an_rfc_whatever_its_status() {
    let m = mirror("list");
    let out = m.ok(&["list", "5280"]);
    assert!(out.starts_with("rfc5280: 3 errata in the feed"), "{out}");
    assert!(
        out.contains("9001 \"Verified\" untriaged section \"Appendix A.1\""),
        "{out}"
    );
    assert!(
        out.contains("9003 \"Reported\" untriaged section \"1\""),
        "{out}"
    );
    assert!(!out.contains("9004"), "{out}");
    assert!(
        out.contains("9002 \"Verified\" untriaged section \"Appendix A.1 (ask [email])\""),
        "{out}"
    );
    assert!(!out.contains("sam@example.com"), "{out}");
    assert!(
        m.ok(&["list", "1"])
            .starts_with("rfc1: 0 errata in the feed")
    );
}

#[test]
fn edit_record_and_check() {
    let m = mirror("record");
    patch_9001(&m);
    let patch = m.patch(5280, 9001).unwrap();
    assert!(
        patch.starts_with("--- a/rfc5280.txt\n+++ b/rfc5280.txt\n"),
        "{patch}"
    );
    assert!(
        m.ledger().contains("5280\t9001\tpatched\tclaude\t"),
        "{}",
        m.ledger()
    );
    assert!(m.ledger().contains("\tasn1\tub-name in PKIX1Explicit88\n"));
    let check = m.ok(&["check"]);
    assert!(
        check.contains("check: 1 ledger rows, 1 patched, OK"),
        "{check}"
    );
    let queue = m.ok(&["queue"]);
    assert!(!queue.contains("9001"), "{queue}");
    m.ok(&[
        "record",
        "5280",
        "9002",
        "--decision",
        "not-applicable",
        "--note",
        "no change needed",
    ]);
    assert!(m.ledger().contains("5280\t9002\tnot-applicable"));
}

#[test]
fn record_refuses_what_it_must() {
    let m = mirror("refuse");
    let reported = m.errata(&["record", "5280", "9003", "--decision", "patched"]);
    assert!(
        stderr(&reported).contains("is not Verified"),
        "{}",
        stderr(&reported)
    );
    let named = m.errata(&[
        "record",
        "5280",
        "9001",
        "--decision",
        "not-applicable",
        "--note",
        "per Sam Submitter",
    ]);
    assert!(
        stderr(&named).contains("name of the erratum's submitter"),
        "{}",
        stderr(&named)
    );
    let no_work = m.errata(&["record", "5280", "9002", "--decision", "patched"]);
    assert!(
        stderr(&no_work).contains("run `edit` first"),
        "{}",
        stderr(&no_work)
    );
    let wrong_rfc = m.errata(&["show", "8692", "9001"]);
    assert!(
        stderr(&wrong_rfc).contains("is of rfc5280, not rfc8692"),
        "{}",
        stderr(&wrong_rfc)
    );
    assert_eq!(m.errata(&["queue", "--rfc", "١٢"]).status.code(), Some(2));
}

#[test]
fn a_lower_erratum_recorded_after_edit_makes_record_refuse_the_old_work_file() {
    let m = mirror("base");
    m.ok(&["edit", "5280", "9002"]);
    m.edit_work(
        "rfc5280-9002.txt",
        "ub-common-name INTEGER ::= 64",
        "ub-common-name INTEGER ::= 65",
    );
    patch_9001(&m);
    let stale = m.errata(&[
        "record",
        "5280",
        "9002",
        "--decision",
        "patched",
        "--note",
        "x",
    ]);
    assert!(
        stderr(&stale).contains("changed since `edit`"),
        "{}",
        stderr(&stale)
    );
    m.ok(&["edit", "5280", "9002", "--force"]);
    assert!(
        std::fs::read_to_string(m.work("rfc5280-9002.txt"))
            .unwrap()
            .contains("ub-name INTEGER ::= 32769")
    );
}

#[test]
fn backing_out_a_patch_rebases_the_higher_ones() {
    let m = mirror("backout");
    m.ok(&["edit", "5280", "9002"]);
    m.edit_work(
        "rfc5280-9002.txt",
        "ub-common-name INTEGER ::= 64",
        "ub-common-name INTEGER ::= 65",
    );
    m.ok(&[
        "record",
        "5280",
        "9002",
        "--decision",
        "patched",
        "--note",
        "x",
    ]);
    // 9001 is lower: recording it rebases 9002 onto it.
    m.ok(&["edit", "5280", "9001"]);
    m.edit_work(
        "rfc5280-9001.txt",
        "ub-name INTEGER ::= 32768\nub-common-name",
        "ub-name INTEGER ::= 32769\nub-common-name",
    );
    let out = m.ok(&[
        "record",
        "5280",
        "9001",
        "--decision",
        "patched",
        "--note",
        "x",
    ]);
    assert!(
        out.contains("rebased the patch of rfc5280 erratum 9002"),
        "{out}"
    );
    assert!(m.ok(&["check"]).contains("OK"));
    let out = m.ok(&[
        "record",
        "5280",
        "9001",
        "--decision",
        "not-applicable",
        "--note",
        "on second thought",
    ]);
    assert!(
        out.contains("removed the patch of rfc5280 erratum 9001"),
        "{out}"
    );
    assert!(
        out.contains("rebased the patch of rfc5280 erratum 9002"),
        "{out}"
    );
    assert!(m.patch(5280, 9001).is_none());
    assert!(m.ok(&["check"]).contains("1 patched, OK"));
}

#[test]
fn check_finds_broken_errata_data() {
    let m = mirror("check");
    patch_9001(&m);
    let path = m.corpus().join("errata/patches/rfc5280/9001.patch");
    let patch = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        patch.replace("-ub-name INTEGER ::= 32768", "-ub-name INTEGER ::= 99"),
    )
    .unwrap();
    std::fs::write(
        m.corpus().join("errata/patches/rfc5280/9002.patch"),
        "stray",
    )
    .unwrap();
    let output = m.errata(&["check"]);
    let out = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{out}");
    assert!(
        out.contains("9002.patch has no `patched` ledger row"),
        "{out}"
    );
    assert!(
        out.contains("the patch of erratum 9001 does not apply"),
        "{out}"
    );
}

#[test]
fn prove_compares_the_corpus_before_and_after_an_edit() {
    let m = mirror("prove");
    m.ok(&["edit", "5280", "9001"]);
    let work = m.work("rfc5280-9001.txt");
    m.edit_work(
        "rfc5280-9001.txt",
        "ub-name INTEGER ::= 32768",
        "ub-name INTEGER ::= 32769",
    );
    let out = m.ok(&["prove", work.to_str().unwrap()]);
    assert!(out.contains("no new errors or warnings"), "{out}");
    assert!(out.contains("PKIX1Explicit88: "), "{out}");
    m.edit_work(
        "rfc5280-9001.txt",
        "ub-name INTEGER ::= 32769",
        "ub-name INTEGER ::= undefined-value",
    );
    let broken = m.errata(&["prove", work.to_str().unwrap()]);
    assert_eq!(
        broken.status.code(),
        Some(1),
        "{}{}",
        stdout(&broken),
        stderr(&broken)
    );
    assert!(
        stdout(&broken).contains("loss rfc5280 PKIX1Explicit88"),
        "{}",
        stdout(&broken)
    );
}

#[test]
fn prove_sees_the_other_rfcs_with_relative_paths() {
    // prove links the mirror's other RFCs into its builds; with a relative --cache-dir (the default) the links must
    // still resolve, or the inline ASN.1 of RFC 8692 is compared with a corpus that lacks RFC 5280.
    let m = mirror("prove-relative");
    m.ok(&["edit", "8692", "9004"]);
    let work = m.work("rfc8692-9004.xml");
    m.edit_work(
        "rfc8692-9004.xml",
        "signatureValue       BIT STRING  }",
        "signatureValue       OCTET STRING  }",
    );
    let absolute = m.errata(&["prove", work.to_str().unwrap()]);
    let relative = m.errata_relative(&["prove", "cache/errata-work/rfc8692-9004.xml"]);
    assert_eq!(
        stdout(&relative),
        stdout(&absolute),
        "{}",
        stderr(&relative)
    );
    assert!(
        stdout(&relative).contains("(inline): 1 errors and warnings before"),
        "{}",
        stdout(&relative)
    );
}

#[test]
fn a_skipped_feed_entry_leaves_the_queue_once_decided() {
    let m = mirror("skipped");
    // A Verified erratum whose orig_text holds a bidi override: the feed parser skips it.
    let feed = FEED.replacen(
        "\n]",
        ",\n {\"errata_id\":\"9005\",\"doc-id\":\"RFC5280\",\"errata_status_code\":\"Verified\",\"section\":\"1\",\n  \"orig_text\":\"a\u{202e}b\",\"correct_text\":\"ab\",\"notes\":null}\n]",
        1,
    );
    std::fs::write(m.cache().join("errata.json"), feed).unwrap();
    let before = m.ok(&["queue"]);
    assert!(before.contains("feed entries skipped"), "{before}");
    assert!(before.contains("erratum 9005 (rfc5280)"), "{before}");
    m.ok(&[
        "record",
        "5280",
        "9005",
        "--decision",
        "not-applicable",
        "--by",
        "human",
        "--note",
        "unreadable feed entry",
    ]);
    let after = m.ok(&["queue"]);
    assert!(!after.contains("9005"), "{after}");
    assert!(!after.contains("feed entries skipped"), "{after}");
}

const SIGNATURE_VALUE: &str = "      signatureValue       BIT STRING  }";
const SIZED_VALUE: &str = "      signatureValue       BIT STRING (SIZE (1..MAX)) }";

/// Edits erratum 9004's XML work file: the inline `Certificate` of section 4.1 gets a SIZE constraint.
fn edit_9004(m: &Mirror) {
    m.ok(&["edit", "8692", "9004"]);
    m.edit_work("rfc8692-9004.xml", SIGNATURE_VALUE, SIZED_VALUE);
}

#[test]
fn an_xml_erratum_gets_a_text_patch_mirrored_from_its_xml_patch() {
    let m = mirror("mirror");
    edit_9004(&m);
    let published = std::fs::read_to_string(m.cache().join("rfc/rfc8692.txt")).unwrap();
    assert_eq!(
        std::fs::read_to_string(m.work("rfc8692-9004.txt")).unwrap(),
        published
    );
    assert!(m.work("rfc8692-9004.txt.base").exists());
    let out = m.ok(&[
        "record",
        "8692",
        "9004",
        "--decision",
        "patched",
        "--note",
        "x",
    ]);
    assert!(
        out.contains("9004.txt.patch (mirrored from the XML change)"),
        "{out}"
    );
    assert!(
        m.patch(8692, 9004)
            .unwrap()
            .starts_with("--- a/rfc8692.xml\n")
    );
    let text_patch = m.text_patch(8692, 9004).unwrap();
    // The text block is indented three more spaces than the XML's content.
    assert!(
        text_patch.starts_with("--- a/rfc8692.txt\n+++ b/rfc8692.txt\n"),
        "{text_patch}"
    );
    assert!(
        text_patch.contains(&format!("\n-   {SIGNATURE_VALUE}\n+   {SIZED_VALUE}\n")),
        "{text_patch}"
    );
    assert!(m.ok(&["check"]).contains("1 patched, OK"));
}

#[test]
fn an_xml_change_outside_the_blocks_needs_the_text_work_file() {
    let m = mirror("outside");
    edit_9004(&m);
    m.edit_work(
        "rfc8692-9004.xml",
        "a signatureValue attribute that contains the actual signature.",
        "a signatureValue attribute that holds the actual signature.",
    );
    let refused = m.record_fails("8692", "9004");
    assert!(
        refused.contains("outside <sourcecode> and <artwork> content")
            && refused.contains("Make the same correction in")
            && refused.contains("rfc8692-9004.txt"),
        "{refused}"
    );
    assert!(m.patch(8692, 9004).is_none() && m.ledger().is_empty());
    // The text work file with the prose corrected but not the block disagrees with the XML change.
    m.edit_work(
        "rfc8692-9004.txt",
        "signatureValue attribute that\n   contains the actual signature.",
        "signatureValue attribute that\n   holds the actual signature.",
    );
    let disagrees = m.record_fails("8692", "9004");
    assert!(
        disagrees.contains("the corrected text does not hold the corrected XML's blocks"),
        "{disagrees}"
    );
    m.edit_work(
        "rfc8692-9004.txt",
        &format!("   {SIGNATURE_VALUE}"),
        &format!("   {SIZED_VALUE}"),
    );
    let out = m.ok(&[
        "record",
        "8692",
        "9004",
        "--decision",
        "patched",
        "--note",
        "x",
    ]);
    assert!(out.contains("(from the text work file)"), "{out}");
    assert!(
        m.text_patch(8692, 9004)
            .unwrap()
            .contains("+   holds the actual")
    );
    assert!(m.ok(&["check"]).contains("1 patched, OK"));
}

#[test]
fn an_edited_text_work_file_must_make_the_mirrored_change() {
    let m = mirror("disagree");
    edit_9004(&m);
    m.edit_work(
        "rfc8692-9004.txt",
        &format!("   {SIGNATURE_VALUE}"),
        "         signatureValue       OCTET STRING  }",
    );
    let refused = m.record_fails("8692", "9004");
    assert!(
        refused.contains("does not make the change the XML patch makes"),
        "{refused}"
    );
    // The text work file of an XML RFC is not what `prove` compares.
    let prove = m.errata(&["prove", m.work("rfc8692-9004.txt").to_str().unwrap()]);
    assert!(
        stderr(&prove).contains("is the text work file of rfc8692"),
        "{}",
        stderr(&prove)
    );
}

#[test]
fn higher_text_patches_are_rebased_and_backed_out_with_their_xml_patches() {
    let m = mirror("text-rebase");
    m.ok(&["edit", "8692", "9006"]);
    m.edit_work(
        "rfc8692-9006.xml",
        "      signatureAlgorithm   AlgorithmIdentifier,",
        "      signatureAlgorithm   AlgorithmIdentifier{{SigAlgs}},",
    );
    m.ok(&[
        "record",
        "8692",
        "9006",
        "--decision",
        "patched",
        "--note",
        "x",
    ]);
    let higher = m.text_patch(8692, 9006).unwrap();
    // 9004 is lower and changes the next line: 9006's patches are rebased onto it.
    edit_9004(&m);
    let out = m.ok(&[
        "record",
        "8692",
        "9004",
        "--decision",
        "patched",
        "--note",
        "x",
    ]);
    assert!(
        out.contains("rebased the patch of rfc8692 erratum 9006"),
        "{out}"
    );
    assert!(
        out.contains("rebased the text patch of rfc8692 erratum 9006"),
        "{out}"
    );
    assert_ne!(m.text_patch(8692, 9006).unwrap(), higher);
    assert!(m.ok(&["check"]).contains("2 patched, OK"));
    let out = m.ok(&[
        "record",
        "8692",
        "9004",
        "--decision",
        "not-applicable",
        "--note",
        "on second thought",
    ]);
    assert!(
        out.contains("rebased the text patch of rfc8692 erratum 9006"),
        "{out}"
    );
    assert!(m.patch(8692, 9004).is_none() && m.text_patch(8692, 9004).is_none());
    assert_eq!(m.text_patch(8692, 9006).unwrap(), higher);
    assert!(m.ok(&["check"]).contains("1 patched, OK"));
}

#[test]
fn check_finds_missing_stray_and_wrong_text_patches() {
    let m = mirror("text-check");
    edit_9004(&m);
    m.ok(&[
        "record",
        "8692",
        "9004",
        "--decision",
        "patched",
        "--note",
        "x",
    ]);
    patch_9001(&m);
    let errata = m.corpus().join("errata/patches");
    let text_patch = errata.join("rfc8692/9004.txt.patch");
    let good = std::fs::read_to_string(&text_patch).unwrap();
    // A text patch that applies but makes another change than the XML patch.
    std::fs::write(
        &text_patch,
        good.replace("(SIZE (1..MAX))", "(SIZE (2..MAX))"),
    )
    .unwrap();
    std::fs::write(errata.join("rfc5280/9001.txt.patch"), "stray\n").unwrap();
    std::fs::write(errata.join("rfc8692/9006.txt.patch"), "stray\n").unwrap();
    let output = m.errata(&["check"]);
    let out = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{out}");
    for expected in [
        "rfc8692 erratum 9004: the text patch does not make the change the XML patch makes",
        "9006.txt.patch has no `patched` ledger row",
        "9001.txt.patch: rfc5280 is read as text",
    ] {
        assert!(out.contains(expected), "{expected}: {out}");
    }
    std::fs::remove_file(&text_patch).unwrap();
    let out = stdout(&m.errata(&["check"]));
    assert!(
        out.contains("rfc8692 erratum 9004 is patched but has no usable text patch"),
        "{out}"
    );
    let queue = m.ok(&["queue", "--rfc", "8692"]);
    assert!(queue.contains("rfc8692 9004 patch-fails"), "{queue}");
}
