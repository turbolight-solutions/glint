#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use glint_corpus::{
    CorpusData, CorpusInfo, DocumentRecord, Documents, Exports, ModuleRecord, ModuleSource,
    OidAssignment, OidLabels, ParseStatus, SourceKind, SyntaxEra,
};
use glint_wasm::{CheckRequest, DocumentRequest, Service, complete};

const BUILD: &str = "2026-11-05.0123abcd";

const BASE: &str =
    "Base { iso(1) identified-organization(3) dod(6) internet(1) private(4) enterprise(1) 99 1 }
DEFINITIONS ::= BEGIN
  Name ::= INTEGER
  Other ::= BOOLEAN
  id-base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 }
  id-one OBJECT IDENTIFIER ::= { id-base 1 }
END";

const DRAFT_TEXT: &str = "Internet-Draft                                                 Oct 2026

Appendix A.  ASN.1 Module

   Mine DEFINITIONS ::= BEGIN
   IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 };
   T ::= SEQUENCE { n Name }
   END
";

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
    let labels = OidLabels::collect(
        [
            ("1.3", "identified-organization"),
            ("1.3.6", "dod"),
            ("1.3.6.1", "internet"),
            ("1.3.6.1.4", "private"),
            ("1.3.6.1.4.1", "enterprise"),
        ]
        .into_iter()
        .map(|(oid, label)| (oid.parse().unwrap(), label.to_owned(), "rfc1000".to_owned())),
        Vec::new(),
    );
    let data = CorpusData {
        info: CorpusInfo {
            build: BUILD.to_owned(),
            notices: Vec::new(),
        },
        documents: vec![DocumentRecord {
            id: "draft-ietf-x-mine-02".to_owned(),
            body: "draft".to_owned(),
            status: None,
            obsoleted_by: Vec::new(),
            date: Some("2026-10-01".to_owned()),
            text_format: Some("text".to_owned()),
        }],
        modules: vec![module],
        oid_assignments: vec![
            OidAssignment {
                oid: "1.3.6.1.4.1.99".parse().unwrap(),
                name: "id-base".to_owned(),
                module: "Base".to_owned(),
                document: "rfc1000".to_owned(),
            },
            OidAssignment {
                oid: "1.3.6.1.4.1.99.1".parse().unwrap(),
                name: "id-one".to_owned(),
                module: "Base".to_owned(),
                document: "rfc1000".to_owned(),
            },
        ],
        oid_nodes: labels.nodes(),
        ..CorpusData::default()
    };
    let mut service = Service::default();
    service.load_corpus(&data.to_json()).unwrap();
    service
}

fn json<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn pasted_text_is_checked_and_the_answer_names_code_and_data() {
    let service = service();
    let request: CheckRequest = serde_json::from_str(
        r#"{"content": "M DEFINITIONS ::= BEGIN IMPORTS Missing FROM Base { 1 3 6 1 4 1 99 1 }; END", "name": "m.asn"}"#,
    )
    .unwrap();
    let response = json(&service.check(request).unwrap());
    assert_eq!(response["data_build"], BUILD);
    assert_eq!(response["glint_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(response["document"]["format"], "bare");
    let codes: Vec<_> = response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect();
    assert!(codes.contains(&"E101".to_owned()), "{codes:?}");
}

#[test]
fn documents_are_checked_by_name_once_their_texts_are_loaded() {
    let mut service = service();
    let request = || -> DocumentRequest {
        serde_json::from_str(r#"{"draft": "draft-ietf-x-mine"}"#).unwrap()
    };
    let error = service.check_document(request()).err().unwrap();
    assert_eq!(error.code, "documents_loading");

    let wrong_build = Documents::write("2026-11-04.0123abcd", &[]);
    assert_eq!(
        service.load_documents(wrong_build).err().unwrap().code,
        "data_build"
    );
    let texts = vec![(
        "draft-ietf-x-mine-02".to_owned(),
        "text".to_owned(),
        DRAFT_TEXT.to_owned(),
    )];
    service
        .load_documents(Documents::write(BUILD, &texts))
        .unwrap();
    let response = json(&service.check_document(request()).unwrap());
    assert_eq!(response["document"]["name"], "draft-ietf-x-mine-02");
    assert_eq!(response["document"]["content"], DRAFT_TEXT);
    assert_eq!(response["modules"][0]["name"], "Mine");

    let old: DocumentRequest =
        serde_json::from_str(r#"{"draft": "draft-ietf-x-mine", "rev": "01"}"#).unwrap();
    let error = service.check_document(old).err().unwrap();
    assert!(
        error.message.contains("draft-ietf-x-mine-02"),
        "{}",
        error.message
    );
    let absent: DocumentRequest = serde_json::from_str(r#"{"rfc": 5280}"#).unwrap();
    assert_eq!(
        service.check_document(absent).err().unwrap().code,
        "not_found"
    );
}

#[test]
fn corpus_queries_answer_like_the_old_api() {
    let service = service();
    let page = json(&service.modules("bas", 0, None));
    assert_eq!(page["total"], 1);
    assert_eq!(page["modules"][0]["name"], "Base");
    let detail = json(&service.module("rfc1000", "Base", None).unwrap());
    assert!(detail["text"].as_str().unwrap().contains("id-one"));
    let oid = json(&service.oid("1.3.6", &[]).unwrap());
    assert_eq!(oid["labels"][0]["label"], "dod");
    let info = json(&service.corpus_info());
    assert_eq!(info["build"], BUILD);
    assert_eq!(info["modules"], 1);
}

#[test]
fn oid_lookup_checks_the_names_written_for_its_arcs() {
    let service = service();
    let names = |list: &[&str]| list.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
    let notes = |oid: &str, written: &[&str]| {
        json(&service.oid(oid, &names(written)).unwrap())["label_notes"].clone()
    };
    assert_eq!(
        notes(
            "1.3.6.1",
            &["iso", "identified-organization", "dod", "internet"]
        ),
        serde_json::json!([])
    );
    assert_eq!(notes("1.3.6.1", &["", "", "", ""]), serde_json::json!([]));
    assert_eq!(
        notes("1.3.6.1", &["iso", "", "dad", "internet"]),
        serde_json::json!(["The corpus calls 1.3.6 dod, not dad."])
    );
}

/// The completions with the cursor at `|` in `text`.
fn labels_at(service: &Service, text: &str) -> (usize, Vec<String>) {
    let position = text.find('|').unwrap();
    let text = text.replace('|', "");
    let result = complete(service, &text, Some("m.asn"), None, position);
    (
        result.from,
        result.items.into_iter().map(|i| i.label).collect(),
    )
}

#[test]
fn completion_offers_modules_after_from_and_their_exports_before_it() {
    let service = service();
    let (from, items) = labels_at(
        &service,
        "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Ba| ; END",
    );
    assert_eq!(from, 42);
    assert_eq!(items, ["Base"]);
    let result = complete(
        &service,
        "M DEFINITIONS ::= BEGIN IMPORTS Name FROM  ; END",
        None,
        None,
        42,
    );
    assert_eq!(
        result.items[0].apply.as_deref(),
        Some(
            "Base { 1 identified-organization(3) dod(6) internet(1) private(4) enterprise(1) 99 1 }"
        ),
        "the module's identifier, with the labels documents write"
    );

    let (_, items) = labels_at(
        &service,
        "M DEFINITIONS ::= BEGIN IMPORTS Name, | FROM Base; END",
    );
    assert!(items.contains(&"Other".to_owned()), "{items:?}");
    assert!(items.contains(&"id-base".to_owned()), "{items:?}");
    assert!(
        !items.contains(&"Name".to_owned()),
        "already imported: {items:?}"
    );
}

#[test]
fn completion_offers_the_arcs_below_an_oid() {
    let service = service();
    let (_, items) = labels_at(
        &service,
        "M DEFINITIONS ::= BEGIN o OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) | } END",
    );
    assert_eq!(items, ["dod(6)"]);
    let (_, items) = labels_at(
        &service,
        "M DEFINITIONS ::= BEGIN IMPORTS id-base FROM Base { 1 3 6 1 4 1 99 1 }; o OBJECT IDENTIFIER ::= { id-base | } END",
    );
    assert_eq!(items, ["id-one(1)"]);
    let (_, items) = labels_at(
        &service,
        "M DEFINITIONS ::= BEGIN T ::= SEQUENCE { a | } END",
    );
    assert!(items.contains(&"INTEGER".to_owned()));
    assert!(items.contains(&"T".to_owned()));
}

#[test]
fn completion_offers_the_names_of_a_module_still_being_written() {
    let service = service();
    for text in [
        "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 }; U ::= INTEGER\n T ::= |",
        "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 }; U ::= INTEGER\n T ::= SEQUENCE {\n a |\n",
    ] {
        let (_, items) = labels_at(&service, text);
        for name in ["U", "T", "Name", "INTEGER"] {
            assert!(
                items.contains(&name.to_owned()),
                "{name} in {text:?}: {items:?}"
            );
        }
    }
    let (_, items) = labels_at(&service, "M DEFINITIONS ::= BEGIN U ::= INTEGER END\n|");
    assert!(
        !items.contains(&"U".to_owned()),
        "after the END of a module: {items:?}"
    );
}

#[test]
fn completion_in_a_snippet_offers_the_names_of_the_documents_modules() {
    let service = service();
    let text = "Internet-Draft                                                 Oct 2026

1.  Introduction

   The extension is:

   Pair ::= SEQUENCE {
        first   Th|,
        second  Name }

Appendix A.  ASN.1 Module

   Mine DEFINITIONS ::= BEGIN
   IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 };
   Thing ::= INTEGER
   Pair ::= SEQUENCE { first Thing, second Name }
   END
";
    let position = text.find('|').unwrap();
    let result = complete(
        &service,
        &text.replace('|', ""),
        Some("d.txt"),
        None,
        position,
    );
    let detail = |label: &str| {
        result
            .items
            .iter()
            .find(|i| i.label == label)
            .and_then(|i| i.detail.clone())
    };
    assert_eq!(
        detail("Thing").as_deref(),
        Some("Mine"),
        "{:?}",
        result.items
    );
    assert_eq!(detail("Name").as_deref(), Some("from Base"));
    assert_eq!(detail("Pair").as_deref(), Some("this snippet"));
    assert!(result.items.iter().any(|i| i.label == "INTEGER"));
}
