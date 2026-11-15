#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use glint_corpus::{
    Artifact, ArtifactsData, BrokenArtifact, Confidence, CorpusData, CorpusInfo, Exports,
    ModuleRecord, ModuleSource, ParseStatus, SourceKind, SyntaxEra,
};
use glint_wasm::{ArtifactQuery, DecodeRequest, SchemaSpec, Service};

const BUILD: &str = "2026-11-15.0123abcd";

const BASE: &str =
    "Base { iso(1) identified-organization(3) dod(6) internet(1) private(4) enterprise(1) 99 1 }
DEFINITIONS ::= BEGIN
  Name ::= INTEGER
  Pair ::= SEQUENCE { n Name, b BOOLEAN }
  Wrap { T } ::= SEQUENCE { t T }
  id-base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 }
END";

/// `Pair { n 5, b TRUE }`.
const PAIR: [u8; 8] = [0x30, 0x06, 0x02, 0x01, 0x05, 0x01, 0x01, 0xff];

fn service() -> Service {
    let module = ModuleRecord {
        name: "Base".to_owned(),
        oid: Some("1.3.6.1.4.1.99.1".parse().unwrap()),
        source: ModuleSource::rfc(1000, None, SourceKind::Extracted),
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: BASE.to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    };
    let data = CorpusData {
        info: CorpusInfo {
            build: BUILD.to_owned(),
            notices: Vec::new(),
        },
        modules: vec![module],
        ..CorpusData::default()
    };
    let mut service = Service::default();
    service.load_corpus(&data.to_json()).unwrap();
    service
}

fn json<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}

fn request(text: &str) -> DecodeRequest {
    serde_json::from_str(text).unwrap()
}

#[test]
fn der_is_decoded_as_a_type_of_a_corpus_module() {
    let service = service();
    let response = service
        .decode(
            &request(r#"{"module": {"source": "rfc1000", "name": "Base"}, "type": "Pair"}"#),
            &PAIR,
        )
        .unwrap();
    let block = &json(&response)["blocks"][0];
    assert_eq!(block["encoding"], "der");
    assert_eq!(block["root"], "rfc1000#Base.Pair");
    assert_eq!(block["der"], "MAYCAQUBAf8=");
    let fields: Vec<_> = block["tlvs"][0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["field"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(fields, ["n", "b"]);
    assert_eq!(block["tlvs"][0]["children"][0]["value"]["decimal"], "5");
}

#[test]
fn base64_text_is_decoded_as_a_type_of_a_drafts_module_with_imports_from_the_corpus() {
    let service = service();
    // A draft in plain text, as the checker holds it: the module is extracted from it first.
    let draft = "Internet-Draft                                                 Nov 2026\n\nAppendix A.  ASN.1 Module\n\n   Mine DEFINITIONS ::= BEGIN\n   IMPORTS Pair FROM Base { 1 3 6 1 4 1 99 1 };\n   T ::= SEQUENCE OF Pair\n   END\n";
    let body = serde_json::json!({
        "module": { "source": "", "name": "Mine" },
        "document": { "content": draft, "name": "draft-x-mine-00.txt" },
        "type": "T",
    });
    let response = service
        .decode(&serde_json::from_value(body).unwrap(), b"MAgwBgIBBQEB/w==")
        .unwrap();
    let block = &json(&response)["blocks"][0];
    assert_eq!(block["encoding"], "base64");
    assert_eq!(block["root"], "Mine.T");
    assert_eq!(block["tlvs"][0]["children"][0]["type_name"], "Base.Pair");
}

#[test]
fn a_type_the_module_lacks_is_refused() {
    let service = service();
    let error = service
        .decode(
            &request(r#"{"module": {"source": "1000", "name": "Base"}, "type": "Missing"}"#),
            &PAIR,
        )
        .err()
        .unwrap();
    assert_eq!(error.code, "not_found");
}

#[test]
fn without_a_type_the_input_is_decoded_without_a_schema() {
    let service = service();
    let response = service.decode(&request("{}"), &PAIR).unwrap();
    let block = &json(&response)["blocks"][0];
    assert!(block.get("root").is_none());
    assert_eq!(block["tlvs"][0]["schema"]["state"], "unmatched");
}

#[test]
fn a_forced_input_format_that_does_not_fit_is_a_bad_request() {
    let service = service();
    let error = service
        .decode(&request(r#"{"input": "pem"}"#), &PAIR)
        .err()
        .unwrap();
    assert_eq!(error.code, "bad_request");
}

#[test]
fn a_module_offers_its_unparameterized_types() {
    let service = service();
    let spec: SchemaSpec =
        serde_json::from_str(r#"{"module": {"source": "rfc1000", "name": "Base"}}"#).unwrap();
    let response = json(&service.types(&spec).unwrap());
    assert_eq!(response["modules"][0]["name"], "Base");
    assert_eq!(
        response["modules"][0]["types"],
        serde_json::json!(["Name", "Pair"])
    );
    let spec: SchemaSpec = serde_json::from_value(serde_json::json!({
        "document": { "content": "A DEFINITIONS ::= BEGIN X ::= NULL END B DEFINITIONS ::= BEGIN Y ::= INTEGER END", "format": "bare" },
    }))
    .unwrap();
    let response = json(&service.types(&spec).unwrap());
    let names: Vec<_> = response["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["A", "B"]);
}

fn artifact(id: &str, label: &str, confidence: Confidence, first_error: Option<&str>) -> Artifact {
    Artifact {
        id: id.to_owned(),
        document: id.split('-').next().unwrap().to_owned(),
        section: Some("A.1".to_owned()),
        appendix: true,
        title: None,
        caption: None,
        line: 10,
        label: label.to_owned(),
        sha256: String::new(),
        der: "MAYCAQUBAf8=".to_owned(),
        root: Some("rfc1000#Base.Pair".to_owned()),
        confidence,
        findings: first_error
            .map(|_| vec!["E401".to_owned()])
            .unwrap_or_default(),
        first_error: first_error.map(str::to_owned),
        obsoleted: false,
    }
}

fn artifacts(build: &str) -> Vec<u8> {
    ArtifactsData {
        build: build.to_owned(),
        artifacts: vec![
            artifact("rfc1000-1", "X509 CERTIFICATE", Confidence::Label, None),
            artifact(
                "rfc1000-2",
                "PAIR",
                Confidence::Decoded,
                Some("E401 bad value"),
            ),
            artifact(
                "rfc2000-1",
                "PAIR",
                Confidence::Unknown,
                Some("E404 missing"),
            ),
        ],
        broken: vec![BrokenArtifact {
            id: "rfc2000-2".to_owned(),
            document: "rfc2000".to_owned(),
            section: None,
            appendix: false,
            line: 40,
            label: "PAIR".to_owned(),
            message: "the body is not base64".to_owned(),
        }],
        ..ArtifactsData::default()
    }
    .to_json()
}

fn ids(list: &serde_json::Value) -> Vec<String> {
    list["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn artifacts_of_another_build_are_refused() {
    let mut service = service();
    let error = service
        .load_artifacts(&artifacts("2026-11-14.ffffffff"))
        .err()
        .unwrap();
    assert_eq!(error.code, "data_build");
    assert!(!service.has_artifacts());
    assert_eq!(
        service
            .artifacts(&ArtifactQuery::default())
            .err()
            .unwrap()
            .code,
        "artifacts_loading"
    );
}

#[test]
fn artifacts_are_filtered_by_document_label_type_confidence_and_errors() {
    let mut service = service();
    let info = json(&service.load_artifacts(&artifacts(BUILD)).unwrap());
    assert_eq!(info["artifacts"], 3);
    assert_eq!(info["broken"], 1);
    let all = json(&service.artifacts(&ArtifactQuery::default()).unwrap());
    assert_eq!(ids(&all), ["rfc1000-1", "rfc1000-2", "rfc2000-1"]);
    assert_eq!(
        all["labels"],
        serde_json::json!(["PAIR", "X509 CERTIFICATE"])
    );
    assert_eq!(all["artifacts"][0]["size"], 8);
    assert!(all["artifacts"][0].get("der").is_none());
    let query = |text: &str| -> serde_json::Value {
        let query: ArtifactQuery = serde_json::from_str(text).unwrap();
        json(&service.artifacts(&query).unwrap())
    };
    let rfc2000 = query(r#"{"document": "RFC2000"}"#);
    assert_eq!(ids(&rfc2000), ["rfc2000-1"]);
    assert_eq!(rfc2000["broken"][0]["id"], "rfc2000-2");
    assert_eq!(
        ids(&query(r#"{"label": "pair"}"#)),
        ["rfc1000-2", "rfc2000-1"]
    );
    assert_eq!(ids(&query(r#"{"confidence": "unknown"}"#)), ["rfc2000-1"]);
    assert_eq!(ids(&query(r#"{"errors": true}"#)), ["rfc1000-2"]);
    assert_eq!(ids(&query(r#"{"type": "base.PAIR"}"#)).len(), 3);
    assert_eq!(ids(&query(r#"{"type": "Certificate"}"#)).len(), 0);
}

#[test]
fn an_artifact_downloads_as_pem_with_its_canonical_label_or_as_base64() {
    let mut service = service();
    service.load_artifacts(&artifacts(BUILD)).unwrap();
    let known = json(&service.artifact("rfc1000-1").unwrap());
    assert_eq!(known["canonical_label"], "CERTIFICATE");
    assert_eq!(
        known["text"],
        "-----BEGIN CERTIFICATE-----\nMAYCAQUBAf8=\n-----END CERTIFICATE-----\n"
    );
    assert_eq!(known["der"], "MAYCAQUBAf8=");
    let unknown = json(&service.artifact("RFC1000-2").unwrap());
    assert!(unknown.get("canonical_label").is_none());
    assert_eq!(unknown["text"], "MAYCAQUBAf8=\n");
    assert_eq!(unknown["first_error"], "E401 bad value");
    assert_eq!(service.artifact("rfc9-1").err().unwrap().code, "not_found");
}
