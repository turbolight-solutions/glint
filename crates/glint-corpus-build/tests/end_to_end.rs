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
    ARTIFACTS_FILE, ArtifactsData, CORPUS_FILE, Confidence, Corpus, CorpusData, CorpusProvider,
    DOCUMENTS_FILE, Documents, Pointer, build_dir, latest_path,
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
    // PEM examples, and no module.
    std::fs::copy(fixture("pem/paginated.txt"), rfcs.join("rfc9999.txt")).unwrap();
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
    let mut unpacked = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(std::fs::File::open(dir.join(DOCUMENTS_FILE)).unwrap()),
        &mut unpacked,
    )
    .unwrap();
    let documents = Documents::read(unpacked).unwrap();
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
    // Without its published text, an XML RFC's corrected XML is what the web UI shows.
    assert!(
        built
            .report
            .contains("RFC 8692: kept as RFCXML for the web UI: no rfc8692.txt"),
        "{}",
        built.report
    );

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

/// Runs corpus-build over the RFCs in `dir/rfc` with the manifest `dir/sources.toml` and the cache `dir/cache`.
fn build_sources_in(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_corpus-build"));
    command
        .arg("--rfc-dir")
        .arg(dir.join("rfc"))
        .arg("--sources")
        .arg(dir.join("sources.toml"))
        .arg("--cache")
        .arg(dir.join("cache"))
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-04"]);
    command
}

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
    let mut command = build_sources_in(&dir);
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
    let output = build_sources_in(&dir).output().unwrap();
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
fn a_listed_module_without_an_oid_satisfies_an_import_by_name() {
    // X.680 13.1 allows a module header without an OID; T.101's Videotex-Coding-Attributes has none, and the
    // modules that use it import it by name alone.
    let dir = std::env::temp_dir().join(format!("glint-src-no-oid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rfc")).unwrap();
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::write(
        dir.join("rfc/rfc9997.txt"),
        IMPORTING_RFC.replace("FROM Standalone { 1 2 3 4 }", "FROM NoOid"),
    )
    .unwrap();
    std::fs::write(
        dir.join("cache/TS-102-941-NoOid.asn"),
        "NoOid DEFINITIONS ::= BEGIN\nShared ::= INTEGER\nEND\n",
    )
    .unwrap();
    let manifest = "[[body]]\nid = \"etsi\"\nnotice = \"ETSI credit\"\n\n[[module]]\nbody = \"etsi\"\n\
        document = \"TS 102 941\"\nname = \"NoOid\"\nurl = \"https://example.test/NoOid.asn\"\n";
    std::fs::write(dir.join("sources.toml"), manifest).unwrap();
    let output = build_sources_in(&dir).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    let bundled = &corpus.modules()[corpus.by_name("NoOid")[0]];
    assert_eq!(bundled.oid, None);
    assert_eq!(bundled.source.document, "TS 102 941");
    let importer = &corpus.modules()[corpus.by_name("Importer")[0]];
    assert_eq!(
        importer.parse_status,
        glint_corpus::ParseStatus::Clean,
        "the import by name is satisfied"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cached_page_that_is_not_the_module_fails_the_build() {
    // An HTML page cached as the module (an entry that pointed at a rendering, or an error page served with status
    // 200) is not used: it is downloaded again, and when that fails too the build fails before writing anything.
    let dir = std::env::temp_dir().join(format!("glint-src-html-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rfc")).unwrap();
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::write(dir.join("rfc/rfc9997.txt"), IMPORTING_RFC).unwrap();
    std::fs::write(
        dir.join("cache/Standalone-1.2.3.4.asn"),
        "<HTML><BODY><PRE>Standalone { 1 2 3 4 } <B>DEFINITIONS</B> ::= <B>BEGIN</B></PRE></BODY></HTML>\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("sources.toml"),
        MANIFEST.replace("include = \"always\"", ""),
    )
    .unwrap();
    let output = build_sources_in(&dir).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains("Standalone 1.2.3.4 from https://example.test/Standalone.asn"),
        "{stderr}"
    );
    assert!(
        !dir.join("out").join(latest_path()).exists(),
        "nothing is published"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_body_with_no_modules_in_the_corpus_adds_no_notice() {
    // Nothing imports `Elsewhere`, `Unused` or `Pinned` once they are on demand, so no ETSI module is bundled.
    let manifest = MANIFEST
        .replace("include = \"always\"", "")
        .replace("name = \"Standalone\"", "name = \"Elsewhere\"")
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
    // The mirror has no text of the draft: the web UI gets its XML.
    let report = std::fs::read_to_string(dir.join("out/report.md")).unwrap();
    assert!(
        report.contains(
            "- draft-ietf-lamps-shake-01: kept as RFCXML for the web UI: no draft-ietf-lamps-shake-01.txt"
        ),
        "{report}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

const FRODOKEM: &str = "draft-ietf-lamps-frodokem-certificates-00";

/// The FrodoKEM draft fixture's text.
fn frodokem_text() -> String {
    std::fs::read_to_string(fixture(&format!("xml/{FRODOKEM}.txt"))).unwrap()
}

/// Builds a corpus of RFC 5280 and the drafts `drafts`, each the FrodoKEM draft fixture's XML under another id, with
/// the text the mirror has of it, if any.
fn build_drafts(tag: &str, drafts: &[(&str, Option<String>)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glint-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (rfcs, mirror) = (dir.join("rfc"), dir.join("drafts"));
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::create_dir_all(&mirror).unwrap();
    std::fs::copy(
        fixture("text/rfc5280-appendix-a.txt"),
        rfcs.join("rfc5280.txt"),
    )
    .unwrap();
    let mut index = String::from("# index\n");
    for (id, text) in drafts {
        std::fs::copy(
            fixture(&format!("xml/{FRODOKEM}.xml")),
            mirror.join(format!("{id}.xml")),
        )
        .unwrap();
        if let Some(text) = text {
            std::fs::write(mirror.join(format!("{id}.txt")), text).unwrap();
        }
        index.push_str(&format!(
            "{id}\t-1\tActive\tI-D Exists\t\t\t2026-10-27\tlamps\n"
        ));
    }
    std::fs::write(mirror.join("all_id2.txt"), index).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corpus-build"))
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--drafts-dir")
        .arg(&mirror)
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
    dir
}

#[test]
fn a_draft_read_from_xml_is_kept_as_its_paginated_text_with_the_xml_beside_it() {
    let dir = build_drafts("draft-text", &[(FRODOKEM, Some(frodokem_text()))]);
    let out = dir.join("out");
    let data = load_data(&out);
    let texts = documents(&out);
    let xml = std::fs::read_to_string(fixture(&format!("xml/{FRODOKEM}.xml"))).unwrap();
    let txt = frodokem_text();

    // documents.bin: the published text, with the blocks of the XML's ASN.1 elements and their page breaks; the XML
    // beside it.
    assert_eq!(texts.get(FRODOKEM), Some((txt.as_str(), "text")));
    assert_eq!(texts.xml(FRODOKEM), Some(xml.as_str()));
    let record = data.documents.iter().find(|d| d.id == FRODOKEM).unwrap();
    assert_eq!(record.text_format.as_deref(), Some("text"));
    assert!(record.errata.is_empty());
    let blocks = texts.entry(FRODOKEM).unwrap().blocks.clone().unwrap();
    let [block] = blocks.as_slice() else {
        panic!("{blocks:?}")
    };
    assert!(txt[block.start..block.end].starts_with("   <CODE BEGINS>\n   X509-FRODOKEM-2026\n"));
    assert_eq!(block.breaks.len(), 4);
    for &[start, end] in &block.breaks {
        assert!(txt[start..end].contains("[Page "), "{:?}", &txt[start..end]);
    }
    let report = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(!report.contains("kept as RFCXML"), "{report}");

    // The modules are those of the XML, exactly as when the mirror has no text of the draft.
    let xml_only = build_drafts("draft-xml-only", &[(FRODOKEM, None)]);
    let modules = |data: &CorpusData| {
        serde_json::to_string(
            &data
                .modules
                .iter()
                .filter(|m| m.source.document == FRODOKEM)
                .collect::<Vec<_>>(),
        )
        .unwrap()
    };
    let before = load_data(&xml_only.join("out"));
    assert!(!modules(&data).is_empty() && modules(&data) != "[]");
    assert_eq!(modules(&data), modules(&before));
    assert_eq!(
        documents(&xml_only.join("out")).get(FRODOKEM),
        Some((xml.as_str(), "xml"))
    );
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(xml_only);
}

#[test]
fn a_draft_whose_blocks_are_not_all_in_its_text_is_kept_as_xml() {
    // The module's first line, re-wrapped.
    let rewrapped =
        frodokem_text().replacen("   X509-FRODOKEM-2026\n", "   X509-\n   FRODOKEM-2026\n", 1);
    let id = "draft-ietf-lamps-rewrapped-00";
    let dir = build_drafts("draft-rewrapped", &[(id, Some(rewrapped))]);
    let out = dir.join("out");
    let texts = documents(&out);
    let (_, format) = texts.get(id).unwrap();
    assert_eq!(format, "xml");
    assert!(texts.entry(id).unwrap().blocks.is_none() && texts.xml(id).is_none());
    let report = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(
        report.contains(&format!(
            "- {id}: kept as RFCXML for the web UI: <sourcecode>/<artwork> element"
        )),
        "{report}"
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

/// An SMI module (it imports RFC1155-SMI) and a module that uses it, as RFC 1303 has `RFC-1303` and its usage
/// example `FourBSD-ISODE`.
const SMI_RFC: &str = "\
3.1.  Definitions

   Macros DEFINITIONS ::= BEGIN
   IMPORTS ObjectName FROM RFC1155-SMI;
   Thing ::= ObjectName
   END

3.4.  Usage Example

   Example DEFINITIONS ::= BEGIN
   IMPORTS Thing FROM Macros;
   Used ::= Thing
   END
";

/// A module that imports the usage example, so it is skipped only once the example is.
const SMI_USER_RFC: &str = "\
Appendix A.  Module

   Deeper DEFINITIONS ::= BEGIN
   IMPORTS Used FROM Example;
   Again ::= Used
   END
";

/// An SMI module whose name an ASN.1 module also has, as RFC 1448 and 1905 have an SMI `SNMPv2-PDU` and RFC 3416
/// an ASN.1 one.
const SMI_TWIN_RFC: &str = "\
Appendix A.  Module

   Shared DEFINITIONS ::= BEGIN
   IMPORTS ObjectName FROM SNMPv2-SMI;
   Value ::= ObjectName
   END
";

const ASN1_TWIN_RFC: &str = "\
Appendix A.  Modules

   Shared DEFINITIONS ::= BEGIN
   Value ::= INTEGER
   END

   KeptImporter DEFINITIONS ::= BEGIN
   IMPORTS Value FROM Shared;
   Kept ::= Value
   END
";

#[test]
fn a_module_that_needs_a_skipped_smi_module_is_skipped_too() {
    let dir = std::env::temp_dir().join(format!("glint-smi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    std::fs::create_dir_all(&rfcs).unwrap();
    for (rfc, text) in [
        (9991, SMI_RFC),
        (9992, SMI_USER_RFC),
        (9993, SMI_TWIN_RFC),
        (9994, ASN1_TWIN_RFC),
    ] {
        std::fs::write(rfcs.join(format!("rfc{rfc}.txt")), text).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_corpus-build"))
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-10"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus = read_corpus(&dir);
    for name in ["Macros", "Example", "Deeper"] {
        assert!(corpus.by_name(name).is_empty(), "{name} is skipped");
    }
    // The import of Shared is satisfied by the ASN.1 module, so it never reaches the SMI one.
    assert_eq!(corpus.by_name("Shared").len(), 1);
    assert_eq!(corpus.by_name("KeptImporter").len(), 1);
    let documents: Vec<&str> = corpus.documents().iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        documents,
        ["rfc9994"],
        "documents left with no module are not listed"
    );

    let report = std::fs::read_to_string(dir.join("out/report.md")).unwrap();
    let skipped = report
        .split("## Skipped SMI modules")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .unwrap();
    for line in [
        "- RFC 9991: `Macros`",
        "- RFC 9991: `Example`",
        "- RFC 9992: `Deeper`",
        "- RFC 9993: `Shared`",
    ] {
        assert!(skipped.contains(line), "{line} in {skipped}");
    }
    assert!(!skipped.contains("KeptImporter"), "{skipped}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Errata patched into the fixture RFCs: 9001 corrects RFC 5280's `ub-name` (text), 9002 the inline `Certificate`
/// of RFC 8692 (XML) and 9003 a comment in RFC 8692's module.
struct ErrataFixture {
    dir: PathBuf,
    rfcs: PathBuf,
    errata: PathBuf,
}

fn errata_fixture(tag: &str) -> ErrataFixture {
    use glint_errata::{Decider, Decision, Row, Rule, Source, SourceFormat, make_patch, store};
    let dir = std::env::temp_dir().join(format!("glint-e2e-errata-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    std::fs::create_dir_all(&rfcs).unwrap();
    std::fs::copy(
        fixture("text/rfc5280-appendix-a.txt"),
        rfcs.join("rfc5280.txt"),
    )
    .unwrap();
    std::fs::copy(fixture("xml/rfc8692.xml"), rfcs.join("rfc8692.xml")).unwrap();
    let text = std::fs::read_to_string(rfcs.join("rfc5280.txt")).unwrap();
    let xml = std::fs::read_to_string(rfcs.join("rfc8692.xml")).unwrap();
    let errata = dir.join("errata");
    let txt_source = Source {
        rfc: 5280,
        format: SourceFormat::Text,
    };
    let xml_source = Source {
        rfc: 8692,
        format: SourceFormat::Xml,
    };
    let p9001 = make_patch(
        txt_source,
        &text,
        &text.replace("ub-name INTEGER ::= 32768", "ub-name INTEGER ::= 32769"),
    )
    .unwrap();
    let inline = xml.replace(
        "      signatureValue       BIT STRING  }",
        "      signatureValue       BIT STRING (SIZE (1..MAX)) }",
    );
    let p9002 = make_patch(xml_source, &xml, &inline).unwrap();
    let p9003 = make_patch(
        xml_source,
        &inline,
        &inline.replace("-- EXPORTS ALL;", "-- EXPORTS ALL; corrected"),
    )
    .unwrap();
    for (rfc, id, patch) in [
        (5280, 9001, &p9001),
        (8692, 9002, &p9002),
        (8692, 9003, &p9003),
    ] {
        store::write_patch(&errata, rfc, id, patch).unwrap();
    }
    let row = |rfc, errata_id| Row {
        rfc,
        errata_id,
        decision: Decision::Patched,
        by: Decider::Claude,
        fingerprint: "0123456789abcdef".to_owned(),
        rule: Rule::Asn1,
        note: "test".to_owned(),
    };
    store::write_ledger(
        &errata,
        &[row(5280, 9001), row(8692, 9002), row(8692, 9003)],
    )
    .unwrap();
    ErrataFixture { dir, rfcs, errata }
}

fn corpus_build(fixture: &ErrataFixture, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_corpus-build"))
        .arg("--rfc-dir")
        .arg(&fixture.rfcs)
        .arg("--out")
        .arg(fixture.dir.join("out"))
        .args(["--date", "2026-11-11"])
        .args(args)
        .output()
        .unwrap()
}

fn errata_args(fixture: &ErrataFixture) -> Vec<String> {
    vec![
        "--errata-dir".to_owned(),
        fixture.errata.display().to_string(),
    ]
}

fn documents(out: &Path) -> Documents {
    let pointer = Pointer::from_json(&std::fs::read(out.join(latest_path())).unwrap()).unwrap();
    let file =
        std::fs::File::open(out.join(build_dir(&pointer.build)).join(DOCUMENTS_FILE)).unwrap();
    let mut unpacked = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(file), &mut unpacked).unwrap();
    Documents::read(unpacked).unwrap()
}

fn run_ok(fixture: &ErrataFixture, args: &[String]) {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = corpus_build(fixture, &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn errata_are_applied_to_text_and_xml_before_extraction() {
    let fixture = errata_fixture("applied");
    run_ok(&fixture, &errata_args(&fixture));
    let data = load_data(&fixture.dir.join("out"));
    let corrected: Vec<(&str, &str, Vec<u32>)> = data
        .modules
        .iter()
        .filter(|m| !m.source.errata.is_empty())
        .map(|m| {
            (
                m.source.document.as_str(),
                m.name.as_str(),
                m.source.errata.clone(),
            )
        })
        .collect();
    assert_eq!(
        corrected,
        [
            ("rfc5280", "PKIX1Explicit88", vec![9001]),
            ("rfc8692", "PKIXAlgsForSHAKE-2019", vec![9003])
        ],
        "the inline Certificate of erratum 9002 is in no module"
    );
    let explicit = data
        .modules
        .iter()
        .find(|m| m.name == "PKIX1Explicit88")
        .unwrap();
    assert!(explicit.text.contains("ub-name INTEGER ::= 32769"));
    let documents = documents(&fixture.dir.join("out"));
    assert!(
        documents
            .get("rfc8692")
            .unwrap()
            .0
            .contains("BIT STRING (SIZE (1..MAX)) }")
    );
    let report = std::fs::read_to_string(fixture.dir.join("out/report.md")).unwrap();
    assert!(
        report.contains("| 5280 | 9001 | `PKIX1Explicit88` |"),
        "{report}"
    );
    assert!(
        report.contains("| 8692 | 9002, 9003 | `PKIXAlgsForSHAKE-2019` |"),
        "{report}"
    );
    let _ = std::fs::remove_dir_all(fixture.dir);
}

#[test]
fn documents_list_their_errata_with_the_lines_of_each_hunk() {
    use glint_errata::{Source, SourceFormat, parse_patch};
    let fixture = errata_fixture("hunks");
    run_ok(&fixture, &errata_args(&fixture));
    let out = fixture.dir.join("out");
    let data = load_data(&out);
    let texts = documents(&out);
    let mut seen = Vec::new();
    for (rfc, format) in [(5280, SourceFormat::Text), (8692, SourceFormat::Xml)] {
        let id = format!("rfc{rfc}");
        let record = data.documents.iter().find(|d| d.id == id).unwrap();
        assert!(record.errata_failure.is_none());
        let text = texts.get(&id).unwrap().0;
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        for erratum in &record.errata {
            seen.push(erratum.id);
            assert_eq!(
                (erratum.by.as_str(), erratum.note.as_str()),
                ("claude", "test")
            );
            let stored = std::fs::read_to_string(glint_errata::store::patch_path(
                &fixture.errata,
                rfc,
                erratum.id,
            ))
            .unwrap();
            assert_eq!(erratum.patch, stored);
            let hunks = parse_patch(&erratum.patch, Source { rfc, format }).unwrap();
            assert_eq!(hunks.len(), erratum.hunks.len());
            for (hunk, range) in hunks.iter().zip(&erratum.hunks) {
                let shown = &lines[range.start_line - 1..range.end_line];
                assert_eq!(shown, hunk.new, "RFC {rfc} erratum {}", erratum.id);
            }
        }
    }
    assert_eq!(seen, [9001, 9002, 9003]);
    let _ = std::fs::remove_dir_all(fixture.dir);
}

#[test]
fn no_errata_builds_the_rfcs_as_published() {
    let fixture = errata_fixture("published");
    let mut args = errata_args(&fixture);
    args.push("--no-errata".to_owned());
    run_ok(&fixture, &args);
    let data = load_data(&fixture.dir.join("out"));
    assert!(data.modules.iter().all(|m| m.source.errata.is_empty()));
    assert!(
        data.modules
            .iter()
            .any(|m| m.text.contains("ub-name INTEGER ::= 32768"))
    );
    let _ = std::fs::remove_dir_all(fixture.dir);
}

#[test]
fn a_series_that_does_not_apply_leaves_the_rfc_as_published() {
    let fixture = errata_fixture("fails");
    let path = glint_errata::store::patch_path(&fixture.errata, 8692, 9003);
    let broken = std::fs::read_to_string(&path)
        .unwrap()
        .replace("-- EXPORTS ALL;\n", "-- EXPORT NONE;\n");
    std::fs::write(&path, broken).unwrap();
    run_ok(&fixture, &errata_args(&fixture));
    let data = load_data(&fixture.dir.join("out"));
    assert!(
        data.modules
            .iter()
            .filter(|m| m.source.document == "rfc8692")
            .all(|m| m.source.errata.is_empty())
    );
    let document = data.documents.iter().find(|d| d.id == "rfc8692").unwrap();
    assert!(document.errata.is_empty());
    assert!(
        document
            .errata_failure
            .as_ref()
            .is_some_and(|f| f.errata_id == 9003 && f.reason.contains("not in the text")),
        "{:?}",
        document.errata_failure
    );
    let report = std::fs::read_to_string(fixture.dir.join("out/report.md")).unwrap();
    assert!(
        report
            .contains("- RFC 8692 is built as published: the patch of erratum 9003 does not apply"),
        "{report}"
    );
    assert!(report.contains("| 5280 | 9001 |"), "{report}");
    let _ = std::fs::remove_dir_all(fixture.dir);
}

#[test]
fn a_patch_for_the_other_format_is_not_applied() {
    let fixture = errata_fixture("format");
    let path = glint_errata::store::patch_path(&fixture.errata, 5280, 9001);
    let as_xml = std::fs::read_to_string(&path)
        .unwrap()
        .replace("rfc5280.txt", "rfc5280.xml");
    std::fs::write(&path, as_xml).unwrap();
    run_ok(&fixture, &errata_args(&fixture));
    let report = std::fs::read_to_string(fixture.dir.join("out/report.md")).unwrap();
    assert!(
        report.contains("RFC 5280 is built as published"),
        "{report}"
    );
    assert!(report.contains("not against rfc5280.txt"), "{report}");
    let _ = std::fs::remove_dir_all(fixture.dir);
}

/// Adds RFC 8692's published text to an errata fixture, with the text patches of errata 9002 and 9003 mirrored from
/// their XML patches, as `glint errata record` makes them.
fn with_published_text(errata_fixture: &ErrataFixture) {
    use glint_errata::{Source, SourceFormat, apply_patch, mirror_patch, store};
    std::fs::copy(
        fixture("xml/rfc8692.txt"),
        errata_fixture.rfcs.join("rfc8692.txt"),
    )
    .unwrap();
    let mut xml = std::fs::read_to_string(errata_fixture.rfcs.join("rfc8692.xml")).unwrap();
    let mut text = std::fs::read_to_string(errata_fixture.rfcs.join("rfc8692.txt")).unwrap();
    let source = Source {
        rfc: 8692,
        format: SourceFormat::Xml,
    };
    for id in [9002, 9003] {
        let patch =
            store::read_patch(&store::patch_path(&errata_fixture.errata, 8692, id)).unwrap();
        let after = apply_patch(&xml, &patch, source).unwrap();
        let text_patch = mirror_patch(8692, &xml, &after, &text).unwrap();
        store::write_text_patch(&errata_fixture.errata, 8692, id, &text_patch).unwrap();
        text = apply_patch(
            &text,
            &text_patch,
            Source {
                format: SourceFormat::Text,
                ..source
            },
        )
        .unwrap();
        xml = after;
    }
}

#[test]
fn an_xml_rfc_is_kept_as_its_corrected_text_with_the_blocks_its_xml_marks() {
    use glint_errata::{Source, SourceFormat, parse_patch};
    let fixture = errata_fixture("text-view");
    with_published_text(&fixture);
    run_ok(&fixture, &errata_args(&fixture));
    let out = fixture.dir.join("out");
    let data = load_data(&out);
    let texts = documents(&out);

    // The modules are still extracted from the corrected XML.
    let module = data
        .modules
        .iter()
        .find(|m| m.name == "PKIXAlgsForSHAKE-2019")
        .unwrap();
    assert_eq!(module.source.errata, [9003]);
    assert!(module.text.contains("-- EXPORTS ALL; corrected"));

    // documents.bin has the published text with the text patches, and the blocks of the XML's ASN.1 elements.
    let (text, format) = texts.get("rfc8692").unwrap();
    assert_eq!(format, "text");
    assert!(!text.contains("<sourcecode"));
    assert!(text.contains("      signatureValue       BIT STRING (SIZE (1..MAX)) }"));
    assert!(text.contains("-- EXPORTS ALL; corrected"));
    let record = data.documents.iter().find(|d| d.id == "rfc8692").unwrap();
    assert_eq!(record.text_format.as_deref(), Some("text"));
    let blocks = texts.entry("rfc8692").unwrap().blocks.clone().unwrap();
    let sections: Vec<(Option<&str>, bool)> = blocks
        .iter()
        .map(|b| (b.section.as_deref(), b.appendix))
        .collect();
    assert_eq!(
        sections,
        [
            (Some("3"), false),
            (Some("3"), false),
            (Some("4.1"), false),
            (Some("A"), true)
        ]
    );
    let module_block = text.get(blocks[3].start..blocks[3].end).unwrap();
    assert!(
        module_block.starts_with("   PKIXAlgsForSHAKE-2019 {"),
        "{module_block}"
    );
    assert!(module_block.contains("-- EXPORTS ALL; corrected"));
    assert!(
        blocks
            .iter()
            .all(|b| b.type_attr.as_deref() == Some("asn.1") && b.indent == 3)
    );

    // Beside the text, the corrected XML: the published XML with the XML patches, as extraction read it.
    let xml = texts.xml("rfc8692").unwrap();
    assert!(xml.contains("<sourcecode"));
    assert!(xml.contains("      signatureValue       BIT STRING (SIZE (1..MAX)) }"));
    assert!(xml.contains("-- EXPORTS ALL; corrected"));
    let xml_lines: Vec<&str> = xml.split_inclusive('\n').collect();

    // The errata carry the text patches, and their lines are in the text.
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut seen = Vec::new();
    for erratum in &record.errata {
        seen.push(erratum.id);
        let stored = std::fs::read_to_string(glint_errata::store::text_patch_path(
            &fixture.errata,
            8692,
            erratum.id,
        ))
        .unwrap();
        assert_eq!(erratum.patch, stored);
        let source = Source {
            rfc: 8692,
            format: SourceFormat::Text,
        };
        let hunks = parse_patch(&erratum.patch, source).unwrap();
        assert_eq!(hunks.len(), erratum.hunks.len());
        for (hunk, range) in hunks.iter().zip(&erratum.hunks) {
            assert_eq!(&lines[range.start_line - 1..range.end_line], hunk.new);
        }
        // And the XML patches, with their lines in the corrected XML stored beside the text.
        let xml_side = erratum.xml.as_ref().unwrap();
        let stored = std::fs::read_to_string(glint_errata::store::patch_path(
            &fixture.errata,
            8692,
            erratum.id,
        ))
        .unwrap();
        assert_eq!(xml_side.patch, stored);
        let source = Source {
            rfc: 8692,
            format: SourceFormat::Xml,
        };
        let hunks = parse_patch(&xml_side.patch, source).unwrap();
        assert_eq!(hunks.len(), xml_side.hunks.len());
        for (hunk, range) in hunks.iter().zip(&xml_side.hunks) {
            assert_eq!(&xml_lines[range.start_line - 1..range.end_line], hunk.new);
        }
    }
    assert_eq!(seen, [9002, 9003]);
    let report = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(!report.contains("kept as RFCXML"), "{report}");
    let _ = std::fs::remove_dir_all(fixture.dir);
}

#[test]
fn an_xml_rfc_whose_text_patches_do_not_apply_is_kept_as_xml() {
    let fixture = errata_fixture("text-patch-fails");
    with_published_text(&fixture);
    let path = glint_errata::store::text_patch_path(&fixture.errata, 8692, 9003);
    let broken = std::fs::read_to_string(&path)
        .unwrap()
        .replace("-- EXPORTS ALL;\n", "-- EXPORT NONE;\n");
    std::fs::write(&path, broken).unwrap();
    run_ok(&fixture, &errata_args(&fixture));
    let out = fixture.dir.join("out");
    let data = load_data(&out);
    // The corpus is built from the XML and its patches as before.
    let module = data
        .modules
        .iter()
        .find(|m| m.name == "PKIXAlgsForSHAKE-2019")
        .unwrap();
    assert_eq!(module.source.errata, [9003]);
    // The web UI gets the corrected XML, with the XML patches.
    let texts = documents(&out);
    let (text, format) = texts.get("rfc8692").unwrap();
    assert_eq!(format, "xml");
    assert!(text.contains("-- EXPORTS ALL; corrected"));
    assert!(texts.entry("rfc8692").unwrap().blocks.is_none());
    assert!(texts.xml("rfc8692").is_none(), "no second view");
    let record = data.documents.iter().find(|d| d.id == "rfc8692").unwrap();
    assert!(
        record
            .errata
            .iter()
            .all(|e| e.patch.starts_with("--- a/rfc8692.xml") && e.xml.is_none())
    );
    let report = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(
        report.contains(
            "RFC 8692: kept as RFCXML for the web UI: the text patch of erratum 9003 does not apply"
        ),
        "{report}"
    );
    let _ = std::fs::remove_dir_all(fixture.dir);
}

/// Breaks an errata fixture in one way.
type Damage = fn(&ErrataFixture);

#[test]
fn malformed_errata_data_fails_the_build() {
    let cases: [(&str, Damage, &str); 3] = [
        (
            "orphan",
            |f| {
                std::fs::write(glint_errata::store::patch_path(&f.errata, 5280, 9999), "x").unwrap()
            },
            "without a `patched` ledger row",
        ),
        (
            "ledger",
            |f| std::fs::write(f.errata.join("ledger.tsv"), "rfc\terrata_id\n").unwrap(),
            "the header must be",
        ),
        (
            "added-line",
            |f| {
                let path = glint_errata::store::patch_path(&f.errata, 5280, 9001);
                let patch = std::fs::read_to_string(&path)
                    .unwrap()
                    .replace("32769", "32769\u{202e}");
                std::fs::write(path, patch).unwrap();
            },
            "adds a control, bidi or zero-width character",
        ),
    ];
    for (tag, damage, message) in cases {
        let fixture = errata_fixture(tag);
        damage(&fixture);
        let args = errata_args(&fixture);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = corpus_build(&fixture, &args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains(message),
            "{tag}: {stderr}"
        );
        let _ = std::fs::remove_dir_all(fixture.dir);
    }
}

#[test]
fn patched_errata_must_be_verified_errata_of_their_rfc() {
    let fixture = errata_fixture("verified");
    let feed = fixture.dir.join("errata.json");
    std::fs::write(
        &feed,
        r#"[{"errata_id":"9001","doc-id":"RFC5280","errata_status_code":"Verified"},
            {"errata_id":"9002","doc-id":"RFC8692","errata_status_code":"Reported"},
            {"errata_id":"9003","doc-id":"RFC5280","errata_status_code":"Verified"}]"#,
    )
    .unwrap();
    let mut args = errata_args(&fixture);
    args.extend(["--errata".to_owned(), feed.display().to_string()]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = corpus_build(&fixture, &args);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains("RFC 8692 erratum 9002 has status Reported, not Verified"),
        "{stderr}"
    );
    assert!(
        stderr.contains("erratum 9003 is of RFC 5280, not RFC 8692"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(fixture.dir);
}

#[test]
fn an_override_of_a_module_errata_correct_is_refused() {
    let fixture = errata_fixture("override");
    let over = fixture.dir.join("overrides/rfc5280");
    std::fs::create_dir_all(&over).unwrap();
    std::fs::write(
        over.join("PKIX1Explicit88.asn"),
        "PKIX1Explicit88 { 1 3 6 1 5 5 7 0 18 } DEFINITIONS ::= BEGIN END",
    )
    .unwrap();
    std::fs::write(
        over.join("meta.toml"),
        "[[override]]\ntier = \"correction\"\ndocument = \"rfc5280\"\nmodule = \"PKIX1Explicit88\"\njustification = \"t\"\n",
    )
    .unwrap();
    let mut args = errata_args(&fixture);
    args.extend([
        "--overrides".to_owned(),
        fixture.dir.join("overrides").display().to_string(),
    ]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = corpus_build(&fixture, &args);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains("replaces a module that errata 9001 correct"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(fixture.dir);
}

/// A plain-text RFC holding one module, with its header OID `{ 1 3 6 1 4 1 99999 <n> }`.
fn module_rfc(dir: &Path, n: u32, name: &str, body: &str) {
    std::fs::write(
        dir.join(format!("rfc{n}.txt")),
        format!(
            "\n\nRequest for Comments: {n}\n\n1.  Module\n\n   {name} {{ 1 3 6 1 4 1 99999 {n} }}\n   DEFINITIONS ::= BEGIN\n{body}\n   END\n"
        ),
    )
    .unwrap();
}

#[test]
fn modules_of_obsoleted_rfcs_that_nothing_maintained_imports_are_unmaintained() {
    let dir = std::env::temp_dir().join(format!("glint-e2e-unmaintained-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rfcs = dir.join("rfc");
    std::fs::create_dir_all(&rfcs).unwrap();
    // 101 (obsoleted) is imported by 200 (current): maintained. 102 (obsoleted) is imported by nothing, and 103
    // (obsoleted) only by 102: both unmaintained.
    module_rfc(&rfcs, 101, "Kept", "   A ::= INTEGER");
    module_rfc(
        &rfcs,
        102,
        "Dropped",
        "   IMPORTS C FROM Inner { 1 3 6 1 4 1 99999 103 };\n   B ::= Undefined1",
    );
    module_rfc(&rfcs, 103, "Inner", "   C ::= Undefined2");
    module_rfc(
        &rfcs,
        200,
        "Current",
        "   IMPORTS A FROM Kept { 1 3 6 1 4 1 99999 101 };\n   D ::= A",
    );
    let entry = |n: u32, obsoleted_by: Option<u32>| {
        let by = obsoleted_by.map_or(String::new(), |b| {
            format!("<obsoleted-by><doc-id>RFC{b}</doc-id></obsoleted-by>")
        });
        format!(
            "<rfc-entry><doc-id>RFC{n}</doc-id><date><month>May</month><year>2000</year></date>{by}\
             <current-status>PROPOSED STANDARD</current-status></rfc-entry>"
        )
    };
    std::fs::write(
        rfcs.join("rfc-index.xml"),
        format!(
            "<rfc-index>{}{}{}{}</rfc-index>",
            entry(101, Some(300)),
            entry(102, Some(300)),
            entry(103, Some(300)),
            entry(200, None)
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corpus-build"))
        .arg("--rfc-dir")
        .arg(&rfcs)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--date", "2026-11-14"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = std::fs::read_to_string(dir.join("out/report.md")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        report.contains(
            "| Unmaintained (obsoleted RFC, imported by no maintained module of another document) | 2 |"
        ),
        "{report}"
    );
    assert!(report.contains("`Dropped`"), "{report}");
    let heading = |name: &str| {
        report
            .lines()
            .find(|l| l.starts_with("### ") && l.contains(&format!("`{name}`")))
            .unwrap_or_default()
            .to_owned()
    };
    assert!(
        heading("Dropped").ends_with("(unmaintained: obsoleted by RFC 300)"),
        "{report}"
    );
    assert!(
        heading("Inner").ends_with("(unmaintained: obsoleted by RFC 300)"),
        "{report}"
    );
    assert!(!heading("Kept").contains("unmaintained"), "{report}");
    // Each unmaintained module has one E110, left out of the maintained totals.
    assert!(report.contains("| `E110` | 2 | 0 |"), "{report}");
}

#[test]
fn every_pem_example_is_identified_and_published_with_the_build() {
    let built = build("artifacts", None);
    let out = built.dir.join("out");
    let pointer = Pointer::from_json(&std::fs::read(out.join(latest_path())).unwrap()).unwrap();
    let dir = out.join(build_dir(&pointer.build));
    let digest = std::fs::read_to_string(dir.join(format!("{ARTIFACTS_FILE}.sha256"))).unwrap();
    assert!(
        digest.ends_with(&format!("  {ARTIFACTS_FILE}\n")),
        "{digest}"
    );
    let data = ArtifactsData::from_json(&std::fs::read(dir.join(ARTIFACTS_FILE)).unwrap()).unwrap();
    data.require_build(&pointer.build).unwrap();
    let ids: Vec<&str> = data.artifacts.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["rfc9999-1", "rfc9999-2", "rfc9999-3"]);
    // The corpus has only RFC 5280's modules: the label's second root.
    let first = &data.artifacts[0];
    assert_eq!(
        first.root.as_deref(),
        Some("rfc5280#PKIX1Explicit88.Certificate")
    );
    assert_eq!(first.confidence, Confidence::Label);
    assert_eq!(first.section.as_deref(), Some("2.1"));
    assert_eq!(first.caption.as_deref(), Some("An EC CA certificate"));
    assert!(!first.has_errors(), "{first:?}");
    assert_eq!(data.broken.len(), 4);
    assert!(
        built.report.contains("## Example artifacts"),
        "{}",
        built.report
    );
    assert!(
        built.report.contains(
            "3 PEM examples in 1 documents; 4 broken blocks; 1 OpenPGP armor blocks left out."
        ),
        "{}",
        built.report
    );
    let expected = std::fs::read_to_string(out.join("artifacts.expected")).unwrap();
    assert!(
        expected.contains("rfc9999-1\trfc5280#PKIX1Explicit88.Certificate\tlabel\t"),
        "{expected}"
    );
    assert!(expected.contains("rfc9999-4\tbroken\t-\t-"), "{expected}");
}
