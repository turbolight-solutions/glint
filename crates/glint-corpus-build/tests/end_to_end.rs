#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use glint_check::{CheckOptions, check_document};
use glint_corpus::{
    CORPUS_FILE, Corpus, CorpusData, CorpusProvider, DOCUMENTS_FILE, Documents, Pointer, build_dir,
    latest_path,
};
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
                "[[override]]\ntier = \"correction\"\ndocument = \"rfc5280\"\nmodule = \"{module}\"\njustification = \"test\"\n"
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
    let corpus = Corpus::from_data(load_data(&dir.join("out")));
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
fn the_build_writes_digests_and_the_document_texts_under_one_build_id() {
    let built = build("layout", None);
    let out = built.dir.join("out");
    let pointer = Pointer::from_json(&std::fs::read(out.join(latest_path())).unwrap()).unwrap();
    assert!(
        pointer.build.starts_with("2026-11-03."),
        "{}",
        pointer.build
    );
    let dir = out.join(build_dir(&pointer.build));
    for file in [CORPUS_FILE, DOCUMENTS_FILE] {
        let digest = std::fs::read_to_string(dir.join(format!("{file}.sha256"))).unwrap();
        assert!(digest.ends_with(&format!("  {file}\n")), "{digest}");
    }
    let documents = Documents::read(std::fs::read(dir.join(DOCUMENTS_FILE)).unwrap()).unwrap();
    documents.require_build(&pointer.build).unwrap();
    let (text, format) = documents.get("rfc8692").unwrap();
    assert_eq!(format, "xml");
    assert!(text.contains("PKIXAlgsForSHAKE-2019"));
    assert_eq!(documents.get("rfc5280").unwrap().1, "text");
    let indexed: Vec<_> = built
        .corpus
        .documents()
        .iter()
        .map(|d| (d.id.as_str(), d.text_format.as_deref()))
        .collect();
    assert!(indexed.contains(&("rfc8692", Some("xml"))), "{indexed:?}");

    let again = build("layout-again", None);
    assert_eq!(
        again.corpus.info().build,
        built.corpus.info().build,
        "the same content gets the same build id"
    );
    let _ = std::fs::remove_dir_all(built.dir);
    let _ = std::fs::remove_dir_all(again.dir);
}

#[test]
fn corpus_has_the_modules_and_the_report_records_a_baseline() {
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
        built.report.contains("| Modules in the corpus | 3 |"),
        "{}",
        built.report
    );
    assert!(built.report.contains("## OID label audit"));
    let pkix: glint_corpus::Oid = "1.3.6.1.5.5.7".parse().unwrap();
    let labels = built.corpus.oid_labels().labels_of(&pkix);
    assert!(
        labels
            .iter()
            .any(|e| e.label == "pkix" && e.documents.contains(&"rfc5280".to_owned())),
        "{labels:?}"
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

/// RFC 4998 defines `ERS` twice, for 1988 and 1997 syntax, with different OIDs. Each appears as a header in the
/// body and in full in an appendix.
const TWIN_RFC: &str = "Request for Comments: 9998                                   Test Author
Category: Standards Track                                   January 2026


                       Twin Module Names

2.1.  Module for 1988 Syntax

   ERS { 1 3 6 1 5 5 11 0 2 1 }
   DEFINITIONS IMPLICIT TAGS ::=
   BEGIN

2.2.  Module for 1997 Syntax

   ERS { 1 3 6 1 5 5 11 0 1 1 }
   DEFINITIONS IMPLICIT TAGS ::=
   BEGIN

Appendix A.  Module with 1988 Syntax

   ERS { 1 3 6 1 5 5 11 0 2 1 }
   DEFINITIONS IMPLICIT TAGS ::=
   BEGIN

   Old ::= INTEGER

   END

Appendix B.  Module with 1997 Syntax

   ERS { 1 3 6 1 5 5 11 0 1 1 }
   DEFINITIONS IMPLICIT TAGS ::=
   BEGIN

   New ::= INTEGER

   END
";

const ERS_88: &str = "1.3.6.1.5.5.11.0.2.1";
const ERS_97: &str = "1.3.6.1.5.5.11.0.1.1";

fn build_twins(tag: &str, overrides: &[(&str, &str)]) -> (PathBuf, std::process::Output) {
    let dir = std::env::temp_dir().join(format!("glint-twin-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::write(rfcs.join("rfc9998.txt"), TWIN_RFC).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_corpus-build"));
    command
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-04"]);
    if !overrides.is_empty() {
        let over = dir.join("overrides/rfc9998");
        std::fs::create_dir_all(&over).unwrap();
        for (file, content) in overrides {
            std::fs::write(over.join(file), content).unwrap();
        }
        command.arg("--overrides").arg(dir.join("overrides"));
    }
    let output = command.output().unwrap();
    (dir, output)
}

fn read_corpus(dir: &Path) -> Corpus {
    Corpus::from_data(load_data(&dir.join("out")))
}

/// The data of the build that `latest.json` names.
fn load_data(out: &Path) -> CorpusData {
    let pointer = Pointer::from_json(&std::fs::read(out.join(latest_path())).unwrap()).unwrap();
    let json = std::fs::read(out.join(build_dir(&pointer.build)).join(CORPUS_FILE)).unwrap();
    let data = CorpusData::from_json(&json).unwrap();
    assert_eq!(data.info.build, pointer.build);
    data
}

#[test]
fn one_rfc_can_define_two_modules_with_the_same_name() {
    let (dir, output) = build_twins("both", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    let found = corpus.by_name("ERS");
    assert_eq!(found.len(), 2, "both modules are kept");
    for (oid, marker) in [(ERS_88, "Old ::="), (ERS_97, "New ::=")] {
        let index = corpus.by_oid(&oid.parse().unwrap()).unwrap();
        let module = &corpus.modules()[index];
        assert_eq!(module.name, "ERS");
        assert!(
            module.text.contains(marker),
            "the full module is kept over its header: {}",
            module.text
        );
        assert_eq!(module.parse_status, glint_corpus::ParseStatus::Clean);
    }
    let report = std::fs::read_to_string(dir.join("out/report.md")).unwrap();
    assert!(
        report.contains(ERS_88) && report.contains(ERS_97),
        "{report}"
    );
    assert!(
        !report.contains("is defined twice"),
        "the headers quoted in sections 2.1 and 2.2 are fragments, not second definitions: {report}"
    );
}

/// A module given twice with different bodies, the first a stub, as RFC 4120 Section 5 and Appendix A do.
const STUB_TWICE_RFC: &str =
    "Request for Comments: 9996                                   Test Author
Category: Standards Track                                   January 2026


                       Stub and Module

5.  Specifications

   Spec { 1 3 6 1 5 2 4 2 } DEFINITIONS EXPLICIT TAGS ::= BEGIN
   -- rest of definitions here
   END

Appendix A.  ASN.1 Module

   Spec { 1 3 6 1 5 2 4 2 } DEFINITIONS EXPLICIT TAGS ::= BEGIN
   Thing ::= INTEGER
   END
";

#[test]
fn a_module_defined_twice_with_different_text_is_named_in_the_report() {
    let dir = std::env::temp_dir().join(format!("glint-stub-twice-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::write(rfcs.join("rfc9996.txt"), STUB_TWICE_RFC).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corpus-build"))
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-04"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = std::fs::read_to_string(dir.join("out/report.md")).unwrap();
    assert!(
        report.contains("Spec (1.3.6.1.5.2.4.2) is defined twice in rfc9996 with different text"),
        "{report}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_override_names_the_oid_of_the_module_it_replaces() {
    let replacement = "ERS { 1 3 6 1 5 5 11 0 1 1 }\nDEFINITIONS IMPLICIT TAGS ::=\nBEGIN\nNewer ::= INTEGER\nEND\n";
    let (dir, output) = build_twins(
        "override-oid",
        &[
            (
                "meta.toml",
                &format!(
                    "[[override]]\ntier = \"correction\"\ndocument = \"rfc9998\"\nmodule = \"ERS\"\noid = \"{ERS_97}\"\njustification = \"test\"\n"
                ),
            ),
            (&format!("ERS-{ERS_97}.asn"), replacement),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    assert_eq!(corpus.by_name("ERS").len(), 2);
    let at = |oid: &str| &corpus.modules()[corpus.by_oid(&oid.parse().unwrap()).unwrap()];
    assert!(at(ERS_97).text.contains("Newer ::="));
    assert!(matches!(
        at(ERS_97).source.kind,
        glint_corpus::SourceKind::Overridden { .. }
    ));
    assert!(
        at(ERS_88).text.contains("Old ::="),
        "the other module is untouched"
    );
    assert_eq!(at(ERS_88).source.kind, glint_corpus::SourceKind::Extracted);
}

#[test]
fn an_override_that_cannot_tell_the_modules_apart_is_refused() {
    let (_, output) = build_twins(
        "override-ambiguous",
        &[
            (
                "meta.toml",
                "[[override]]\ntier = \"correction\"\ndocument = \"rfc9998\"\nmodule = \"ERS\"\njustification = \"test\"\n",
            ),
            (
                "ERS.asn",
                "ERS { 1 3 6 1 5 5 11 0 1 1 }\nDEFINITIONS ::= BEGIN\nEND\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ambiguous") && stderr.contains("oid"),
        "{stderr}"
    );
}

const IMPORTING_RFC: &str = "Appendix A.  Module

   Importer { 1 2 3 9 }
   DEFINITIONS IMPLICIT TAGS ::=
   BEGIN

   IMPORTS Shared FROM Standalone { 1 2 3 4 };

   Uses ::= SEQUENCE { shared Shared }

   END
";

const STANDALONE: &str = "Standalone { 1 2 3 4 }
DEFINITIONS ::= BEGIN
Shared ::= INTEGER
END
";

const UNUSED: &str = "Unused { 1 2 3 5 }
DEFINITIONS ::= BEGIN
Spare ::= INTEGER
END
";

const PINNED: &str = "Pinned { 1 2 3 6 }
DEFINITIONS ::= BEGIN
Fixed ::= INTEGER
END
";

const MANIFEST: &str = r#"
[[body]]
id = "etsi"
notice = "ETSI credit"

[[module]]
body = "etsi"
document = "TS 102 941"
edition = "v2.1.1"
name = "Standalone"
oid = "1.2.3.4"
url = "https://example.test/Standalone.asn"

[[module]]
body = "etsi"
document = "TS 102 941"
edition = "v2.1.1"
name = "Unused"
oid = "1.2.3.5"
url = "https://example.test/Unused.asn"

[[module]]
body = "etsi"
document = "TS 103 097"
name = "Pinned"
oid = "1.2.3.6"
url = "https://example.test/Pinned.asn"
include = "always"
"#;

/// Builds a corpus of one RFC plus the sources manifest, with the downloads already in the cache.
fn build_with_sources(
    tag: &str,
    manifest: &str,
    overrides: &[(&str, &str)],
) -> (PathBuf, std::process::Output) {
    let dir = std::env::temp_dir().join(format!("glint-src-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    let cache = dir.join("cache");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(rfcs.join("rfc9997.txt"), IMPORTING_RFC).unwrap();
    for (name, oid, text) in [
        ("Standalone", "1.2.3.4", STANDALONE),
        ("Unused", "1.2.3.5", UNUSED),
        ("Pinned", "1.2.3.6", PINNED),
    ] {
        std::fs::write(cache.join(format!("{name}-{oid}.asn")), text).unwrap();
    }
    std::fs::write(dir.join("sources.toml"), manifest).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_corpus-build"));
    command
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--sources")
        .arg(dir.join("sources.toml"))
        .arg("--cache")
        .arg(&cache)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-04"]);
    if !overrides.is_empty() {
        let over = dir.join("overrides/ts-102-941");
        std::fs::create_dir_all(&over).unwrap();
        for (file, content) in overrides {
            std::fs::write(over.join(file), content).unwrap();
        }
        command.arg("--overrides").arg(dir.join("overrides"));
    }
    let output = command.output().unwrap();
    (dir, output)
}

#[test]
fn a_chain_of_listed_modules_is_fetched_to_its_end() {
    // Standalone imports Link1, which imports Link2, ... Link5: each link is found only once the one before it is
    // fetched, so the chain takes six rounds.
    let dir = std::env::temp_dir().join(format!("glint-src-chain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    let cache = dir.join("cache");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(rfcs.join("rfc9997.txt"), IMPORTING_RFC).unwrap();
    let mut manifest = String::from("[[body]]\nid = \"etsi\"\nnotice = \"ETSI credit\"\n");
    let links = ["Standalone", "Link1", "Link2", "Link3", "Link4", "Link5"];
    for (i, name) in links.iter().enumerate() {
        let oid = if i == 0 {
            "1.2.3.4".to_owned()
        } else {
            format!("1.2.3.4{i}")
        };
        let import = match links.get(i + 1) {
            Some(next) => format!(
                "IMPORTS Next FROM {next} {{ 1 2 3 4{} }};\nShared ::= Next\n",
                i + 1
            ),
            None => "Shared ::= INTEGER\n".to_owned(),
        };
        let text = format!(
            "{name} {{ {} }}\nDEFINITIONS ::= BEGIN\n{import}Next ::= INTEGER\nEND\n",
            oid.replace('.', " ")
        );
        std::fs::write(cache.join(format!("{name}-{oid}.asn")), text).unwrap();
        manifest.push_str(&format!(
            "\n[[module]]\nbody = \"etsi\"\ndocument = \"TS 102 941\"\nname = \"{name}\"\noid = \"{oid}\"\nurl = \"https://example.test/{name}.asn\"\n"
        ));
    }
    std::fs::write(dir.join("sources.toml"), manifest).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corpus-build"))
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--sources")
        .arg(dir.join("sources.toml"))
        .arg("--cache")
        .arg(&cache)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-04"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    for name in links {
        assert_eq!(corpus.by_name(name).len(), 1, "{name} is bundled");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn standalone_modules_join_the_corpus_beside_the_rfc_modules() {
    let (dir, output) = build_with_sources("join", MANIFEST, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    let find = |name: &str| &corpus.modules()[corpus.by_name(name)[0]];
    let imported = find("Standalone");
    assert_eq!(imported.source.body, "etsi");
    assert_eq!(imported.source.document, "TS 102 941");
    assert_eq!(imported.source.edition.as_deref(), Some("v2.1.1"));
    assert_eq!(imported.source.kind, glint_corpus::SourceKind::Standalone);
    assert_eq!(imported.label(), "TS 102 941#Standalone");
    assert_eq!(
        imported.parse_status,
        glint_corpus::ParseStatus::Clean,
        "the RFC module's import is satisfied"
    );
    assert_eq!(find("Importer").source.document, "rfc9997");
    assert!(
        corpus.by_name("Unused").is_empty(),
        "a when-imported module nothing imports is left out"
    );
    assert_eq!(find("Pinned").source.document, "TS 103 097");
    let notices = &corpus.info().notices;
    assert!(notices.iter().any(|n| n == "ETSI credit"), "{notices:?}");
}

#[test]
fn a_body_with_no_modules_in_the_corpus_adds_no_notice() {
    // Nothing imports `Unused` or `Pinned` once they are on demand, so no ETSI module is bundled.
    let manifest = MANIFEST
        .replace("include = \"always\"", "")
        .replace("oid = \"1.2.3.4\"", "oid = \"1.2.3.7\"");
    let (dir, output) = build_with_sources("notice", &manifest, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    assert!(corpus.modules().iter().all(|m| m.source.body == "rfc"));
    assert!(!corpus.info().notices.iter().any(|n| n == "ETSI credit"));
}

#[test]
fn an_override_replaces_a_standalone_module() {
    let replacement = "Standalone { 1 2 3 4 }\nDEFINITIONS ::= BEGIN\nShared ::= BOOLEAN\nEND\n";
    let (dir, output) = build_with_sources(
        "override",
        MANIFEST,
        &[
            (
                "meta.toml",
                "[[override]]\ntier = \"correction\"\ndocument = \"TS 102 941\"\nmodule = \"Standalone\"\njustification = \"test\"\n",
            ),
            ("Standalone.asn", replacement),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    let module = &corpus.modules()[corpus.by_name("Standalone")[0]];
    assert!(module.text.contains("BOOLEAN"));
    assert!(matches!(
        module.source.kind,
        glint_corpus::SourceKind::Overridden { .. }
    ));
    assert_eq!(module.source.body, "etsi", "the document is kept");
    assert_eq!(
        module.source.url.as_deref(),
        Some("https://example.test/Standalone.asn")
    );
}

#[test]
fn an_override_that_matches_no_module_is_refused() {
    let (_, output) = build_with_sources(
        "override-dead",
        MANIFEST,
        &[
            (
                "meta.toml",
                "[[override]]\ntier = \"correction\"\ndocument = \"TS 102 941\"\nmodule = \"Unused\"\njustification = \"test\"\n",
            ),
            ("Unused.asn", UNUSED),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("matches no module"), "{stderr}");
}

#[test]
fn a_manifest_entry_needs_its_body_to_be_declared() {
    let manifest = MANIFEST.replace("[[body]]\nid = \"etsi\"", "[[body]]\nid = \"other\"");
    let (_, output) = build_with_sources("no-body", &manifest, &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("has no [[body]]"), "{stderr}");
}

#[test]
fn active_drafts_join_the_corpus_but_are_no_label_evidence() {
    let dir = std::env::temp_dir().join(format!("glint-e2e-drafts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    let drafts = dir.join("drafts");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::create_dir_all(&drafts).unwrap();
    std::fs::copy(
        fixture("text/rfc5280-appendix-a.txt"),
        rfcs.join("rfc5280.txt"),
    )
    .unwrap();
    std::fs::copy(
        fixture("xml/rfc8692.xml"),
        drafts.join("draft-ietf-lamps-shake-01.xml"),
    )
    .unwrap();
    std::fs::copy(
        fixture("xml/rfc8692.xml"),
        drafts.join("draft-expired-thing-04.xml"),
    )
    .unwrap();
    std::fs::write(
        drafts.join("all_id2.txt"),
        "# index\n\
         draft-ietf-lamps-shake-01\t-1\tActive\tIn Last Call\t\t\t2026-10-20\tlamps\n\
         draft-expired-thing-04\t-1\tExpired\t\t\t\t2020-01-01\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corpus-build"))
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--drafts-dir")
        .arg(&drafts)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-05"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    let draft = corpus
        .documents()
        .iter()
        .find(|d| d.id == "draft-ietf-lamps-shake-01")
        .expect("the active draft is indexed");
    assert_eq!(draft.body, "draft");
    assert_eq!(draft.date.as_deref(), Some("2026-10-20"));
    assert_eq!(draft.status.as_deref(), Some("In Last Call"));
    assert_eq!(draft.text_format.as_deref(), Some("xml"));
    assert!(
        corpus
            .documents()
            .iter()
            .all(|d| d.id != "draft-expired-thing-04")
    );
    let module = corpus
        .modules()
        .iter()
        .find(|m| m.source.document == "draft-ietf-lamps-shake-01")
        .expect("the draft's module is in the corpus");
    assert_eq!(module.source.edition.as_deref(), Some("2026-10-20"));
    let pkix: glint_corpus::Oid = "1.3.6.1.5.5.7".parse().unwrap();
    assert!(
        corpus
            .oid_labels()
            .labels_of(&pkix)
            .iter()
            .all(|e| !e.documents.iter().any(|d| d.starts_with("draft-"))),
        "drafts are no label evidence"
    );
    let _ = std::fs::remove_dir_all(dir);
}

const STUB_FIRST_RFC: &str = "\
5.  Specifications

   Spec { 1 3 6 1 5 2 4 2 } DEFINITIONS EXPLICIT TAGS ::= BEGIN

   -- rest of definitions here

   END

Appendix A.  ASN.1 Module

   Spec { 1 3 6 1 5 2 4 2 } DEFINITIONS EXPLICIT TAGS ::= BEGIN

   Int32 ::= INTEGER (-2147483648..2147483647)

   END
";

#[test]
fn a_full_module_is_kept_over_an_earlier_stub_that_also_parses() {
    // RFC 4120 shows its header with a placeholder body in Section 5 and the module in Appendix A.
    let dir = std::env::temp_dir().join(format!("glint-stub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::write(rfcs.join("rfc9997.txt"), STUB_FIRST_RFC).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corpus-build"))
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-08"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    let found = corpus.by_name("Spec");
    assert_eq!(found.len(), 1);
    let module = &corpus.modules()[found[0]];
    assert!(module.text.contains("Int32"), "{}", module.text);
    std::fs::remove_dir_all(&dir).unwrap();
}
