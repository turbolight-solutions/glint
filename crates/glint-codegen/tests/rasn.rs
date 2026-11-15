#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

//! The rasn backend (Codegen and Decoder Design, Testing and Milestones: 5).
//!
//! The listing snapshot runs everywhere. The differential test needs cargo and the rasn crate, so it is ignored by
//! default (`cargo test -p glint-codegen -- --ignored`, as CI runs it): it generates the code of every fixture root,
//! compiles it in a scratch crate, decodes every vector with it and with `glint-decode`, and requires the two trees
//! to be equal, every DER vector to re-encode byte for byte, and every vector the decoder finds errors in to be
//! rejected. With `GLINT_CODEGEN_DATA` naming a directory of corpus data (`corpus.json` and `artifacts.json`), every
//! published example identified by its label or by decoding is a vector too, decoded as the root it was identified as.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use glint_check::{lower_corpus_root, lower_user_type};
use glint_codegen::{Options, backend, generate};
use glint_corpus::{ArtifactsData, Confidence, Corpus, CorpusData};
use glint_decode::{Decoded, NoOracle, Rendered, SchemaState, Tlv, decode, sniff};
use glint_ir::{IrSet, TagMode};
use serde_json::{Value, json};

fn manifest(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn read(path: &str) -> String {
    let full = manifest(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

const SEMA: &str = "../glint-sema/tests/fixtures";
const RFC5280: &str = "../glint-syntax/tests/fixtures/rfc5280";

const RFC5912: [&str; 7] = [
    "rfc5912/AlgorithmInformation-2009",
    "rfc5912/PKIX-CommonTypes-2009",
    "rfc5912/PKIX-X400Address-2009",
    "rfc5912/PKIX1-PSS-OAEP-Algorithms-2009",
    "rfc5912/PKIXAlgs-2009",
    "rfc5912/PKIX1Explicit-2009",
    "rfc5912/PKIX1Implicit-2009",
];

const CMS2009: [&str; 5] = [
    "rfc5912/AttributeCertificateVersion1-2009",
    "rfc5912/PKIXAttributeCertificate-2009",
    "rfc5911/CryptographicMessageSyntaxAlgorithms-2009",
    "rfc5911/SecureMimeMessageV3dot1-2009",
    "rfc5911/CryptographicMessageSyntax-2009",
];

/// What generated code must do with a vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// What the decoder does: decode it to the same tree when the decoder finds no error, reject it otherwise.
    Oracle,
    /// Reject it, though the decoder does not: it breaks a presence constraint, which the decoder does not check.
    Reject,
}

/// A root type and the vectors decoded as it.
struct Root {
    name: String,
    set: IrSet,
    vectors: Vec<(String, Vec<u8>, Expect)>,
    /// Statements run against the generated module `$m`, returning the failures as a `Vec<String>`.
    extra: Option<&'static str>,
}

fn sema(paths: &[&str]) -> Vec<String> {
    paths
        .iter()
        .map(|p| read(&format!("{SEMA}/{p}.asn")))
        .collect()
}

fn rfc5280() -> Vec<String> {
    ["PKIX1Explicit88", "PKIX1Implicit88"]
        .iter()
        .map(|n| read(&format!("{RFC5280}/{n}.asn")))
        .collect()
}

fn vector(path: &str) -> (String, Vec<u8>, Expect) {
    let text = read(path);
    let input = sniff(text.as_bytes()).remove(0);
    let name = path.rsplit('/').next().unwrap_or(path).to_owned();
    (name, input.bytes, Expect::Oracle)
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// Vectors of `tests/fixtures/Codegen-Test.asn`: a valid `Top`, and one change of it per constraint.
const SYNTHETIC: &[(&str, Expect, &str)] = &[
    (
        "valid",
        Expect::Oracle,
        "3055301C02010516026162800206C082026869A306020101020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "valid-minimal",
        Expect::Oracle,
        "3015300702010516026162300A3008020101A003020107",
    ),
    (
        "valid-choice-n",
        Expect::Oracle,
        "3054301B02010516026162800206C081012AA306020101020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "unknown-extension",
        Expect::Oracle,
        "3058301F02010516026162800206C082026869A306020101020103A40380010189010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "default-encoded",
        Expect::Oracle,
        "3055301C02010516026162800206C082026869A306020101020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0101",
    ),
    (
        "value-out-of-range",
        Expect::Oracle,
        "3055301C02010A16026162800206C082026869A306020101020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "size-too-long",
        Expect::Oracle,
        "3058301F02010516056162636465800206C082026869A306020101020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "bits-wrong-size",
        Expect::Oracle,
        "3055301C02010516026162800205A082026869A306020101020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "alphabet",
        Expect::Oracle,
        "3055301C02010516026162800206C082024869A306020101020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "element-value",
        Expect::Oracle,
        "3052301902010516026162800206C082026869A303020104A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "list-too-long",
        Expect::Oracle,
        "3058301F02010516026162800206C082026869A309020101020102020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "no-holders",
        Expect::Oracle,
        "3032301C02010516026162800206C082026869A306020101020103A4038001013000300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "key-not-in-set",
        Expect::Oracle,
        "303C301C02010516026162800206C082026869A306020101020103A403800101300A3008020109A003020101300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "open-type-mismatch",
        Expect::Oracle,
        "303C301C02010516026162800206C082026869A306020101020103A403800101300A3008020102A003020101300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "presence",
        Expect::Reject,
        "3052301902010516026162800206C082026869A306020101020103A40030233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
    (
        "contained-record-invalid",
        Expect::Oracle,
        "3041301C02010516026162800206C082026869A306020101020103A403800101300F300D020103A008300602016316017A300B0C04726F6F74A0030C016CA5030A0102",
    ),
];

/// Values built by hand that `glint_rt::encode` must refuse, and one it must take.
const SYNTHETIC_ENCODE: &str = r#"
    let valid = unhex("3055301C02010516026162800206C082026869A306020101020103A40380010130233008020101A0030201073008020102A0030101FF300D020103A008300602010016017A300B0C04726F6F74A0030C016CA5030A0102");
    let mut failures: Vec<String> = Vec::new();
    let top = $m::glint_rt::decode::<$m::Top>(&valid).unwrap();
    if $m::glint_rt::encode(&top).is_err() {
        failures.push("a decoded value does not encode".to_owned());
    }
    let mut bad = top.clone();
    bad.record.small = 10;
    if $m::glint_rt::encode(&bad).is_ok() {
        failures.push("encoded a value outside (0..9)".to_owned());
    }
    let mut bad = top.clone();
    bad.holders[0].id = rasn::types::Integer::from(2);
    if $m::glint_rt::encode(&bad).is_ok() {
        failures.push("encoded an open type whose key names another type".to_owned());
    }
    let mut bad = top.clone();
    bad.record.pair = Some($m::codegen_test::Pair { x: None, y: None });
    if $m::glint_rt::encode(&bad).is_ok() {
        failures.push("encoded a value that breaks WITH COMPONENTS".to_owned());
    }
    failures
"#;

fn lower(texts: Vec<String>, module: &str, ty: &str) -> IrSet {
    lower_user_type(&texts, &Corpus::default(), module, ty)
        .unwrap_or_else(|e| panic!("{module}.{ty}: {e}"))
}

/// The fixture roots of the design's Testing section, with their checked-in vectors.
fn fixture_roots() -> Vec<Root> {
    let certificate = "../glint-check/tests/fixtures/decode/ec-ca.pem";
    let mut cms = sema(&RFC5912);
    cms.extend(sema(&CMS2009));
    let mut roots = vec![
        Root {
            name: "rfc5912-certificate".into(),
            set: lower(sema(&RFC5912), "PKIX1Explicit-2009", "Certificate"),
            vectors: vec![vector(certificate)],
            extra: None,
        },
        Root {
            name: "rfc5280-certificate".into(),
            set: lower(rfc5280(), "PKIX1Explicit88", "Certificate"),
            vectors: vec![vector(certificate)],
            extra: None,
        },
        Root {
            name: "codegen-test".into(),
            set: lower(
                vec![read("tests/fixtures/Codegen-Test.asn")],
                "Codegen-Test",
                "Top",
            ),
            vectors: SYNTHETIC
                .iter()
                .map(|(name, expect, hex)| ((*name).to_owned(), unhex(hex), *expect))
                .collect(),
            extra: Some(SYNTHETIC_ENCODE),
        },
    ];
    let mut texts = cms.clone();
    texts.extend(sema(&["rfc6268/CryptographicMessageSyntax-2010"]));
    roots.push(Root {
        name: "rfc6268-content-info".into(),
        set: lower(texts, "CryptographicMessageSyntax-2010", "ContentInfo"),
        vectors: vec![
            vector("tests/fixtures/vectors/content-info-rfc7468-6.pem"),
            vector("tests/fixtures/vectors/content-info-rfc9882-1.pem"),
        ],
        extra: None,
    });
    let mut texts = rfc5280();
    texts.extend(sema(&[
        "rfc5652/AttributeCertificateVersion1",
        "rfc5652/CryptographicMessageSyntax2004",
    ]));
    roots.push(Root {
        name: "rfc5652-content-info".into(),
        set: lower(texts, "CryptographicMessageSyntax2004", "ContentInfo"),
        vectors: vec![
            vector("tests/fixtures/vectors/content-info-rfc7468-6.pem"),
            vector("tests/fixtures/vectors/content-info-rfc9882-1.pem"),
        ],
        extra: None,
    });
    let mut texts = cms;
    texts.extend(sema(&["rfc5958/AsymmetricKeyPackageModuleV1"]));
    roots.push(Root {
        name: "rfc5958-one-asymmetric-key".into(),
        set: lower(texts, "AsymmetricKeyPackageModuleV1", "OneAsymmetricKey"),
        vectors: vec![
            vector("tests/fixtures/vectors/one-asymmetric-key-rfc8410-2.pem"),
            vector("tests/fixtures/vectors/one-asymmetric-key-rfc8410-7.pem"),
            vector("tests/fixtures/vectors/one-asymmetric-key-rfc8410-8.pem"),
        ],
        extra: None,
    });
    let mut texts = rfc5280();
    texts.extend(sema(&[
        "rfc3369/CryptographicMessageSyntax",
        "rfc4210/PKIXCMP",
        "rfc3161/PKIXTSP",
    ]));
    roots.push(Root {
        name: "rfc3161-time-stamp-resp".into(),
        set: lower(texts, "PKIXTSP", "TimeStampResp"),
        vectors: Vec::new(),
        extra: None,
    });
    let mut texts = sema(&RFC5912);
    texts.extend(sema(&["rfc6960/OCSP-2013-08"]));
    roots.push(Root {
        name: "rfc6960-ocsp-response".into(),
        set: lower(texts, "OCSP-2013-08", "OCSPResponse"),
        vectors: Vec::new(),
        extra: None,
    });
    roots
}

/// One root per type the published examples were identified as, with those examples.
fn artifact_roots(dir: &Path) -> Vec<Root> {
    let corpus = CorpusData::from_json(&std::fs::read(dir.join("corpus.json")).unwrap()).unwrap();
    let corpus = Corpus::from_data(corpus);
    let artifacts =
        ArtifactsData::from_json(&std::fs::read(dir.join("artifacts.json")).unwrap()).unwrap();
    let mut by_root: BTreeMap<String, Vec<(String, Vec<u8>, Expect)>> = BTreeMap::new();
    for a in &artifacts.artifacts {
        if a.confidence == Confidence::Unknown {
            continue;
        }
        let Some(root) = &a.root else { continue };
        let Some(bytes) = glint_decode::base64(a.der.as_bytes()) else {
            continue;
        };
        by_root
            .entry(root.clone())
            .or_default()
            .push((a.id.clone(), bytes, Expect::Oracle));
    }
    by_root
        .into_iter()
        .filter_map(|(root, vectors)| match lower_corpus_root(&corpus, &root) {
            Ok(set) => Some(Root {
                name: root,
                set,
                vectors,
                extra: None,
            }),
            Err(e) => {
                eprintln!("skipping {root}: {e}");
                None
            }
        })
        .collect()
}

// ---- the decoder's tree, normalized ------------------------------------------------------------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn leaf(value: &Rendered) -> String {
    match value {
        Rendered::Boolean { value } => value.to_string(),
        Rendered::Integer { decimal, .. } | Rendered::Enumerated { decimal, .. } => decimal.clone(),
        Rendered::Null => "NULL".to_owned(),
        Rendered::BitString { unused, hex, .. } => format!("{unused}:{hex}"),
        Rendered::OctetString { hex } | Rendered::Hex { hex } => hex.clone(),
        Rendered::Oid { dotted, .. } | Rendered::RelativeOid { dotted } => dotted.clone(),
        Rendered::Time { text, .. } | Rendered::String { text } | Rendered::Real { text } => {
            text.clone()
        }
    }
}

fn labeled(mut node: Value, label: Option<&str>) -> Value {
    if let Some(label) = label {
        let inner = node.get("label").and_then(Value::as_str).map(str::to_owned);
        node["label"] = json!(match inner {
            Some(inner) => format!("{label}.{inner}"),
            None => label.to_owned(),
        });
    }
    node
}

/// The tree generated code shows for the value of `tlv` (`glint_rt::Tree::to_json`): explicit tags are transparent,
/// a component encoded with its `DEFAULT` value and an unknown extension addition are left out (generated code
/// does not keep them), and a TLV decoded without a schema is raw.
fn normalize(tlv: &Tlv, bytes: &[u8]) -> Value {
    let label = tlv.field.as_deref();
    if tlv.schema == SchemaState::Unmatched {
        return labeled(json!({ "raw": hex(&bytes[tlv.offset..tlv.end]) }), label);
    }
    if tlv.tagging == Some(TagMode::Explicit)
        && let [inner] = tlv.children.as_slice()
    {
        return labeled(normalize(inner, bytes), label);
    }
    let node = match (&tlv.value, tlv.children.as_slice()) {
        (Some(_), [inner, ..]) => json!({ "contains": normalize(inner, bytes) }),
        (Some(value), []) => json!({ "leaf": leaf(value) }),
        (None, children) => {
            let items: Vec<Value> = children
                .iter()
                .filter(|c| {
                    !c.findings.iter().any(|f| {
                        f.code == "I406"
                            || (f.code == "W402" && f.message.contains("DEFAULT value"))
                    })
                })
                .map(|c| normalize(c, bytes))
                .collect();
            json!({ "list": items })
        }
    };
    labeled(node, label)
}

fn is_der(decoded: &Decoded) -> bool {
    !decoded.all_findings().iter().any(|f| f.code == "W402")
}

fn node(
    offset: usize,
    end: usize,
    field: &str,
    value: Option<Rendered>,
    children: Vec<Tlv>,
) -> Tlv {
    Tlv {
        offset,
        header_len: 2,
        len: glint_decode::Length::Definite(end - offset - 2),
        end,
        tag: None,
        tagging: None,
        field: Some(field.to_owned()),
        type_name: None,
        value,
        children,
        findings: Vec::new(),
        schema: SchemaState::Matched,
    }
}

#[test]
fn normalized_trees_drop_explicit_tags_and_join_choice_labels() {
    // [4] EXPLICIT around a CHOICE: the labels of the component and of the alternative join
    let bytes = [0xa4, 0x03, 0x02, 0x01, 0x05];
    let five = Rendered::Integer {
        decimal: "5".into(),
        hex: None,
        name: None,
    };
    let mut outer = node(
        0,
        5,
        "directoryName",
        None,
        vec![node(2, 5, "number", Some(five), Vec::new())],
    );
    outer.tagging = Some(TagMode::Explicit);
    assert_eq!(
        normalize(&outer, &bytes),
        json!({ "label": "directoryName.number", "leaf": "5" })
    );
    outer.schema = SchemaState::Unmatched;
    assert_eq!(
        normalize(&outer, &bytes),
        json!({ "label": "directoryName", "raw": "A403020105" })
    );
}

// ---- listing ------------------------------------------------------------------------------------

/// What the backend writes for each fixture root: its files and their sizes, and its findings.
#[test]
fn rasn_listing_of_the_fixture_roots() {
    let rasn = backend("rasn").unwrap();
    let mut listing = String::new();
    for root in fixture_roots() {
        let output = generate(&root.set, rasn, &Options::default());
        let _ = writeln!(listing, "{}", root.name);
        for file in &output.files {
            if file.path != "glint_rt.rs" {
                let _ = writeln!(
                    listing,
                    "  {} ({} lines)",
                    file.path,
                    file.contents.lines().count()
                );
            }
        }
        for f in &output.findings {
            let _ = writeln!(listing, "  {} {}: {}", f.code, f.at, f.message);
        }
    }
    insta::assert_snapshot!(listing);
}

/// The whole output for the synthetic module, so a change to the shape of generated code shows in review.
#[test]
fn rasn_output_of_the_synthetic_module() {
    let set = lower(
        vec![read("tests/fixtures/Codegen-Test.asn")],
        "Codegen-Test",
        "Top",
    );
    let output = generate(&set, backend("rasn").unwrap(), &Options::default());
    let module = output
        .files
        .iter()
        .find(|f| f.path == "codegen_test.rs")
        .unwrap();
    insta::assert_snapshot!(module.contents);
}

// ---- the differential test ----------------------------------------------------------------------

/// The name of the root type the generated `mod.rs` re-exports.
fn root_type(files: &[glint_codegen::File]) -> String {
    let module = files.iter().find(|f| f.path == "mod.rs").unwrap();
    let line = module
        .contents
        .lines()
        .find(|l| l.starts_with("pub use self::"))
        .unwrap();
    line.trim_end_matches(';')
        .rsplit("::")
        .next()
        .unwrap()
        .to_owned()
}

const HARNESS_MAIN: &str = r#"
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn quote(s: &str) -> String {
    format!("{s:?}").replace("\\'", "'")
}

macro_rules! run {
    ($m:ident, $t:ident, $bytes:expr) => {{
        use $m::glint_rt::Glint as _;
        match $m::glint_rt::decode::<$m::$t>($bytes) {
            Ok(value) => {
                let tree = value.tree().to_json();
                match $m::glint_rt::encode(&value) {
                    Ok(der) => format!("{{\"tree\":{tree},\"der\":\"{}\"}}", hex(&der)),
                    Err(e) => format!("{{\"tree\":{tree},\"encode\":{}}}", quote(&e.to_string())),
                }
            }
            Err(e) => format!("{{\"error\":{}}}", quote(&e.to_string())),
        }
    }};
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    for line in std::io::stdin().lines() {
        let line = line.unwrap();
        if line == "extra" {
            let failures: Vec<String> = extra().iter().map(|f| quote(f)).collect();
            println!("{{\"extra\":[{}]}}", failures.join(","));
            continue;
        }
        let (root, data) = line.split_once(' ').unwrap();
        let bytes = unhex(data);
        let root: usize = root.parse().unwrap();
        let out = std::panic::catch_unwind(|| dispatch(root, &bytes))
            .unwrap_or_else(|_| "{\"panic\":true}".to_owned());
        println!("{out}");
    }
}
"#;

struct Failure {
    root: String,
    vector: String,
    what: String,
}

#[test]
#[ignore = "compiles generated code with cargo and the rasn crate; run with --ignored"]
fn rasn_output_compiles_and_matches_the_decoder() {
    let mut roots = fixture_roots();
    if let Some(dir) = std::env::var_os("GLINT_CODEGEN_DATA") {
        roots.extend(artifact_roots(Path::new(&dir)));
    }
    let rasn = backend("rasn").unwrap();
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("codegen-rasn");
    let src = work.join("src");
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        work.join("Cargo.toml"),
        format!(
            "[package]\nname = \"codegen-rasn\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
             [dependencies]\nrasn = \"={}\"\n\n[workspace]\n",
            rasn.meta().library_version
        ),
    )
    .unwrap();
    let mut main = String::new();
    let mut extra_calls: Vec<String> = Vec::new();
    let mut dispatch =
        String::from("fn dispatch(root: usize, bytes: &[u8]) -> String {\n    match root {\n");
    for (i, root) in roots.iter().enumerate() {
        let output = generate(&root.set, rasn, &Options::default());
        let dir = src.join(format!("r{i}"));
        std::fs::create_dir_all(&dir).unwrap();
        for file in &output.files {
            std::fs::write(dir.join(&file.path), &file.contents).unwrap();
        }
        let _ = writeln!(main, "#[allow(warnings)]\nmod r{i}; // {}", root.name);
        let _ = writeln!(
            dispatch,
            "        {i} => run!(r{i}, {}, bytes),",
            root_type(&output.files)
        );
        if let Some(code) = root.extra {
            let _ = writeln!(
                main,
                "fn extra_r{i}() -> Vec<String> {{{}}}",
                code.replace("$m", &format!("r{i}"))
            );
            extra_calls.push(format!("extra_r{i}()"));
        }
    }
    dispatch.push_str("        _ => unreachable!(),\n    }\n}\n");
    let _ = writeln!(
        dispatch,
        "fn extra() -> Vec<String> {{\n    let mut all = Vec::new();\n{}    all\n}}",
        extra_calls
            .iter()
            .map(|c| format!("    all.extend({c});\n"))
            .collect::<String>()
    );
    std::fs::write(
        src.join("main.rs"),
        format!("{main}{HARNESS_MAIN}\n{dispatch}"),
    )
    .unwrap();

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let build = Command::new(&cargo)
        .args(["build", "--quiet", "--manifest-path"])
        .arg(work.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", work.join("target"))
        .status()
        .unwrap();
    assert!(build.success(), "the generated code does not compile");

    let mut input = String::new();
    for (i, root) in roots.iter().enumerate() {
        for (_, bytes, _) in &root.vectors {
            let _ = writeln!(input, "{i} {}", hex(bytes));
        }
    }
    input.push_str("extra\n");
    let mut child = Command::new(work.join("target/debug/codegen-rasn"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // written from a thread, so that neither side waits on a full pipe
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        use std::io::Write as _;
        stdin.write_all(input.as_bytes()).unwrap();
    });
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    let mut results = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into_iter();

    let mut failures = Vec::new();
    let mut counts = BTreeMap::<&str, usize>::new();
    for root in &roots {
        for (name, bytes, expect) in &root.vectors {
            let result: Value = serde_json::from_str(&results.next().unwrap()).unwrap();
            let decoded = decode(bytes, Some((&root.set, root.set.root)), &NoOracle);
            let fail = |what: String| Failure {
                root: root.name.clone(),
                vector: name.clone(),
                what,
            };
            let valid = !decoded.failed(false) && *expect == Expect::Oracle;
            if result.get("panic").is_some() {
                failures.push(fail("generated code panicked".into()));
                continue;
            }
            match (valid, result.get("tree")) {
                (true, None) => failures.push(fail(format!(
                    "rejected a vector the decoder accepts: {}",
                    result["error"]
                ))),
                (false, Some(_)) => failures.push(fail(format!(
                    "accepted a vector the decoder rejects: {}",
                    decoded
                        .all_findings()
                        .iter()
                        .filter(|f| f.severity == glint_diag::Severity::Error)
                        .map(|f| format!("{} {}", f.code, f.message))
                        .collect::<Vec<_>>()
                        .join("; ")
                ))),
                (false, None) => *counts.entry("rejected, as the decoder").or_default() += 1,
                (true, Some(tree)) => {
                    if !is_der(&decoded) {
                        eprintln!("not compared (BER): {} {name}", root.name);
                        *counts.entry("decoded (BER, not compared)").or_default() += 1;
                        continue;
                    }
                    let expected = normalize(&decoded.tlvs[0], bytes);
                    if *tree != expected {
                        failures.push(fail(format!(
                            "trees differ\n    generated: {tree}\n    decoder:   {expected}"
                        )));
                        continue;
                    }
                    match result.get("der").and_then(Value::as_str) {
                        Some(der) if der == hex(bytes) => {
                            *counts
                                .entry("decoded, equal trees, round trip")
                                .or_default() += 1;
                        }
                        Some(der) => failures.push(fail(format!(
                            "re-encodes differently\n    input:  {}\n    output: {der}",
                            hex(bytes)
                        ))),
                        None => {
                            failures.push(fail(format!("cannot re-encode: {}", result["encode"])))
                        }
                    }
                }
            }
        }
    }
    let extra: Value = serde_json::from_str(&results.next().unwrap()).unwrap();
    for message in extra["extra"].as_array().unwrap() {
        failures.push(Failure {
            root: "codegen-test".into(),
            vector: "encode".into(),
            what: message.as_str().unwrap_or_default().to_owned(),
        });
    }
    eprintln!("{} roots compiled; {counts:?}", roots.len());
    for f in &failures {
        eprintln!("FAIL {} {}: {}", f.root, f.vector, f.what);
    }
    assert!(failures.is_empty(), "{} vectors failed", failures.len());
}
