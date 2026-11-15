#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

//! The decoder against corpus modules (Codegen and Decoder Design, Milestones: 2): a certificate decoded with the
//! RFC 5912 modules goes through the `ExtensionSet` table; decoded with the RFC 5280 modules, through the OID hint
//! table built from RFC 5912's object sets, marked heuristic.

use std::path::Path;

use glint_check::{CorpusOracle, lower_user_type};
use glint_corpus::{
    Corpus, CorpusData, Exports, ModuleRecord, ModuleSource, OidHint, ParseStatus, SourceKind,
    SyntaxEra,
};
use glint_decode::{Decoded, NoOracle, SchemaState, Tlv, decode, render_text, sniff};
use glint_ir::IrSet;
use glint_sema::{DraftSource, Options, analyze};
use glint_syntax::{ParseOptions, parse_source_with};

const RFC5912: [&str; 7] = [
    "AlgorithmInformation-2009",
    "PKIX-CommonTypes-2009",
    "PKIX-X400Address-2009",
    "PKIX1-PSS-OAEP-Algorithms-2009",
    "PKIXAlgs-2009",
    "PKIX1Explicit-2009",
    "PKIX1Implicit-2009",
];

fn read(path: &str) -> String {
    let full = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

fn rfc5912_texts() -> Vec<String> {
    RFC5912
        .iter()
        .map(|name| read(&format!("../glint-sema/tests/fixtures/rfc5912/{name}.asn")))
        .collect()
}

fn rfc5280_texts() -> Vec<String> {
    ["PKIX1Explicit88", "PKIX1Implicit88"]
        .iter()
        .map(|name| {
            read(&format!(
                "../glint-syntax/tests/fixtures/rfc5280/{name}.asn"
            ))
        })
        .collect()
}

fn certificate() -> Vec<u8> {
    let pem = read("tests/fixtures/decode/ec-ca.pem");
    sniff(pem.as_bytes()).remove(0).bytes
}

/// A corpus record of an RFC 5912 module, with its module identifier so that imports by OID resolve.
fn record(name: &str, text: &str) -> ModuleRecord {
    let (empty, options) = (Corpus::default(), Options::default());
    let analysis = analyze(
        vec![DraftSource {
            parse: parse_source_with(text, ParseOptions::corpus()),
            text: text.to_owned(),
        }],
        &empty,
        &options,
    );
    let oid = analysis
        .graph
        .module(analysis.drafts[0])
        .unwrap()
        .oid
        .clone();
    ModuleRecord {
        name: name.to_owned(),
        oid,
        source: ModuleSource::rfc(5912, None, SourceKind::Extracted),
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Y2002,
        text: text.to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    }
}

/// A corpus of the RFC 5912 modules with the hint table corpus-build would give it.
fn rfc5912_corpus() -> Corpus {
    let modules: Vec<ModuleRecord> = RFC5912
        .iter()
        .zip(rfc5912_texts())
        .map(|(name, text)| record(name, &text))
        .collect();
    let plain = Corpus::from_data(CorpusData {
        modules: modules.clone(),
        ..CorpusData::default()
    });
    let mut hints = Vec::new();
    for module in &modules {
        let options = Options {
            home_document: Some("rfc5912".to_owned()),
            ..Options::default()
        };
        let analysis = analyze(
            vec![DraftSource {
                parse: parse_source_with(&module.text, ParseOptions::corpus()),
                text: module.text.clone(),
            }],
            &plain,
            &options,
        );
        let id = analysis.drafts[0];
        for row in glint_sema::ir::oid_hints(&analysis.graph, id) {
            hints.push(OidHint {
                oid: row.oid.parse().unwrap(),
                class: row.class,
                field: row.field,
                document: "rfc5912".to_owned(),
                module: module.name.clone(),
                set: row.set,
                ty: row.ty,
            });
        }
    }
    Corpus::from_data(CorpusData {
        modules,
        oid_hints: hints,
        ..CorpusData::default()
    })
}

fn find<'a>(decoded: &'a Decoded, pred: &dyn Fn(&Tlv) -> bool) -> Vec<&'a Tlv> {
    decoded
        .tlvs
        .iter()
        .flat_map(|t| t.walk())
        .filter(|t| pred(t))
        .collect()
}

fn lower(texts: &[String], corpus: &Corpus, module: &str, ty: &str) -> IrSet {
    lower_user_type(texts, corpus, module, ty).unwrap()
}

#[test]
fn rfc5912_certificate_decodes_every_extension_through_its_table() {
    let corpus = Corpus::default();
    let set = lower(
        &rfc5912_texts(),
        &corpus,
        "PKIX1Explicit-2009",
        "Certificate",
    );
    let decoded = decode(
        &certificate(),
        Some((&set, set.root)),
        &CorpusOracle::new(&corpus),
    );
    assert!(
        decoded.all_findings().is_empty(),
        "{:?}",
        decoded.all_findings()
    );
    let values = find(&decoded, &|t| t.field.as_deref() == Some("extnValue"));
    assert_eq!(values.len(), 6);
    for value in values {
        let [inner] = value.children.as_slice() else {
            panic!("extnValue at {} not decoded", value.offset);
        };
        assert_eq!(inner.schema, SchemaState::Matched);
        assert!(inner.type_name.is_some(), "{inner:?}");
    }
    insta::assert_snapshot!(render_text(&decoded));
}

#[test]
fn rfc5280_certificate_decodes_extensions_through_hints() {
    let corpus = rfc5912_corpus();
    let set = lower(&rfc5280_texts(), &corpus, "PKIX1Explicit88", "Certificate");
    let decoded = decode(
        &certificate(),
        Some((&set, set.root)),
        &CorpusOracle::new(&corpus),
    );
    assert!(
        decoded.all_findings().is_empty(),
        "{:?}",
        decoded.all_findings()
    );
    let values = find(&decoded, &|t| t.field.as_deref() == Some("extnValue"));
    assert_eq!(values.len(), 6);
    for value in values {
        let [inner] = value.children.as_slice() else {
            panic!("extnValue at {} not decoded", value.offset);
        };
        assert_eq!(
            inner.schema,
            SchemaState::Hinted {
                from: "rfc5912#PKIX1Implicit-2009".to_owned()
            }
        );
    }
    insta::assert_snapshot!(render_text(&decoded));
}

#[test]
fn without_hints_1988_extensions_stay_octet_strings() {
    let corpus = Corpus::default();
    let set = lower(&rfc5280_texts(), &corpus, "PKIX1Explicit88", "Certificate");
    let decoded = decode(&certificate(), Some((&set, set.root)), &NoOracle);
    let values = find(&decoded, &|t| t.field.as_deref() == Some("extnValue"));
    assert!(values.iter().all(|v| v.children.is_empty()));
}

#[test]
fn a_broken_component_is_decoded_without_a_schema_and_its_siblings_still_are() {
    let corpus = Corpus::default();
    let set = lower(
        &rfc5912_texts(),
        &corpus,
        "PKIX1Explicit-2009",
        "Certificate",
    );
    let mut bytes = certificate();
    // the version INTEGER (offset 10) becomes a BOOLEAN
    assert_eq!(bytes[10], 0x02);
    bytes[10] = 0x01;
    let decoded = decode(&bytes, Some((&set, set.root)), &NoOracle);
    let codes: Vec<&str> = decoded.all_findings().iter().map(|f| f.code).collect();
    // the stray BOOLEAN is also not DER (TRUE written as 02)
    assert_eq!(codes, ["W402", "E403"]);
    let broken = find(&decoded, &|t| t.offset == 10)[0];
    assert_eq!(broken.schema, SchemaState::Unmatched);
    let serial = find(&decoded, &|t| t.field.as_deref() == Some("serialNumber"));
    assert_eq!(serial[0].schema, SchemaState::Matched);
}

#[test]
fn ber_edge_cases_without_a_schema() {
    // indefinite-length SEQUENCE with a constructed OCTET STRING and a long-form length, then trailing data
    let bytes = [
        0x30, 0x80, //
        0x24, 0x80, 0x04, 0x01, 0xaa, 0x04, 0x01, 0xbb, 0x00, 0x00, //
        0x02, 0x81, 0x01, 0x05, //
        0x01, 0x01, 0x01, //
        0x00, 0x00, //
        0x05, 0x00,
    ];
    let decoded = decode(&bytes, None, &NoOracle);
    insta::assert_snapshot!(render_text(&decoded));
}

#[test]
fn ber_edge_cases_with_a_schema() {
    let module = "M DEFINITIONS AUTOMATIC TAGS ::= BEGIN
  T ::= SEQUENCE {
    a OCTET STRING (SIZE (2)),
    b BOOLEAN DEFAULT FALSE,
    c SET OF INTEGER,
    d PrintableString
  }
END"
    .to_owned();
    let set = lower(&[module], &Corpus::default(), "M", "T");
    let bytes = [
        0x30, 0x80, //
        0xa0, 0x80, 0x04, 0x01, 0xaa, 0x04, 0x01, 0xbb, 0x00, 0x00, // a: constructed
        0x81, 0x01, 0x00, // b: DEFAULT encoded
        0xa2, 0x06, 0x02, 0x01, 0x07, 0x02, 0x01, 0x03, // c: out of order
        0x83, 0x01, 0x40, // d: '@' not printable
        0x00, 0x00,
    ];
    let decoded = decode(&bytes, Some((&set, set.root)), &NoOracle);
    insta::assert_snapshot!(render_text(&decoded));
}

const SIGNED: &str = "Signed DEFINITIONS ::= BEGIN
ALG ::= CLASS { &id OBJECT IDENTIFIER UNIQUE, &Params OPTIONAL, &Value OPTIONAL }
    WITH SYNTAX { IDENTIFIER &id [PARAMS &Params] [VALUE &Value] }
a-plain ALG ::= { IDENTIFIER { 1 2 3 } PARAMS NULL }
a-valued ALG ::= { IDENTIFIER { 1 2 4 } VALUE INTEGER }
Algs ALG ::= { a-plain | a-valued, ... }
Signed ::= SEQUENCE {
    data INTEGER,
    algorithmIdentifier SEQUENCE {
        algorithm ALG.&id({Algs}),
        parameters ALG.&Params({Algs}{@algorithmIdentifier.algorithm}) OPTIONAL },
    signature BIT STRING (CONTAINING ALG.&Value({Algs}{@algorithmIdentifier.algorithm})) }
END
";

/// RFC 5912 `SIGNED`: `parameters` is keyed by a path into the record that holds it, still being decoded; and a
/// signature whose algorithm object sets no `&Value` holds bits, which are not decoded as a TLV.
#[test]
fn signed_keys_parameters_by_an_open_record_and_leaves_plain_signatures_alone() {
    let set = lower_user_type(
        &[SIGNED.to_owned()],
        &Corpus::from_data(CorpusData::default()),
        "Signed",
        "Signed",
    )
    .unwrap();
    // Algorithm 1.2.3 (NULL parameters, no &Value); the signature's bits would read as a broken TLV.
    let plain = [
        0x30, 0x11, 0x02, 0x01, 0x05, 0x30, 0x06, 0x06, 0x02, 0x2A, 0x03, 0x05, 0x00, 0x03, 0x04,
        0x00, 0x30, 0x7F, 0x01,
    ];
    let decoded = decode(&plain, Some((&set, set.root)), &NoOracle);
    assert!(
        decoded.all_findings().is_empty(),
        "{}",
        render_text(&decoded)
    );
    let top = &decoded.tlvs[0];
    let parameters = &top.children[1].children[1];
    assert_eq!(parameters.field.as_deref(), Some("parameters"));
    assert_eq!(parameters.schema, SchemaState::Matched);
    assert!(top.children[2].children.is_empty());
    // Algorithm 1.2.4: the signature holds an INTEGER.
    let valued = [
        0x30, 0x0F, 0x02, 0x01, 0x05, 0x30, 0x04, 0x06, 0x02, 0x2A, 0x04, 0x03, 0x04, 0x00, 0x02,
        0x01, 0x07,
    ];
    let decoded = decode(&valued, Some((&set, set.root)), &NoOracle);
    assert!(
        decoded.all_findings().is_empty(),
        "{}",
        render_text(&decoded)
    );
    assert_eq!(decoded.tlvs[0].children[2].children.len(), 1);
}
