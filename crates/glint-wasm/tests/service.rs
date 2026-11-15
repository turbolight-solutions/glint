#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use glint_corpus::{
    AppliedErratum, CorpusData, CorpusInfo, DocumentBlock, DocumentRecord, DocumentText, Documents,
    ErratumPatch, Exports, HunkLines, ModuleRecord, ModuleSource, OidAssignment, OidLabels,
    ParseStatus, SourceKind, SyntaxEra,
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

fn data() -> CorpusData {
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
    CorpusData {
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
            errata: Vec::new(),
            errata_failure: None,
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
    }
}

fn service() -> Service {
    let mut service = Service::default();
    service.load_corpus(&data().to_json()).unwrap();
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

fn text(id: &str, content: &str) -> DocumentText {
    DocumentText {
        id: id.to_owned(),
        format: "text".to_owned(),
        text: content.to_owned(),
        blocks: None,
        xml: None,
    }
}

fn codes(response: &serde_json::Value) -> Vec<String> {
    response["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_text_of_an_xml_rfc_is_checked_with_the_blocks_its_xml_marks() {
    let content = "1.  Introduction\n\n   Prose.\n\nAppendix A.  ASN.1 Module\n\n   Mine DEFINITIONS ::= BEGIN\n   IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 };\n   Mine ::= SEQUENCE { a Name }\n   END\n";
    let start = content.find("   Mine DEFINITIONS").unwrap();
    let block = DocumentBlock {
        start,
        end: content.len(),
        indent: 3,
        bar: None,
        breaks: Vec::new(),
        type_attr: None,
        section: Some("A".to_owned()),
        appendix: true,
        title: Some("ASN.1 Module".to_owned()),
    };
    let mut service = service();
    let texts = vec![DocumentText {
        blocks: Some(vec![block]),
        ..text("rfc9000", content)
    }];
    service
        .load_documents(Documents::write(BUILD, &texts))
        .unwrap();
    let request: DocumentRequest = serde_json::from_str(r#"{"rfc": 9000}"#).unwrap();
    let response = json(&service.check_document(request).unwrap());
    assert_eq!(response["document"]["format"], "text");
    // No XML stored beside the text: no other view.
    assert!(response["document"].get("other_format").is_none());
    assert!(response["document"].get("xml_source").is_none());
    assert_eq!(response["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(response["blocks"][0]["section"], "Appendix A");
    assert_eq!(response["modules"][0]["name"], "Mine");
    assert_eq!(response["modules"][0]["start"], start);
    // The block has no `type` in the XML: XML extraction took it by its content and says so, about the XML: the
    // text's check counts that finding instead of listing it.
    assert!(!codes(&response).contains(&"W202".to_owned()), "{response}");
    assert_eq!(
        response["document"]["markup_findings"],
        serde_json::json!({ "errors": 0, "warnings": 1, "infos": 0 })
    );

    // Pasted, the same text is read by the text heuristics, which do not ask for RFCXML marking, and has no XML
    // source.
    let pasted: CheckRequest =
        serde_json::from_value(serde_json::json!({ "content": content, "name": "rfc9000" }))
            .unwrap();
    let response = json(&service.check(pasted).unwrap());
    assert!(response["document"].get("xml_source").is_none());
    assert!(response["document"].get("markup_findings").is_none());
    assert_eq!(response["modules"][0]["name"], "Mine");
    assert!(!codes(&response).contains(&"W202".to_owned()), "{response}");
}

const RFC9336_XML: &str = include_str!("../../glint-extract/tests/fixtures/xml/rfc9336.xml");
const RFC9336_TXT: &str = include_str!("../../glint-extract/tests/fixtures/xml/rfc9336.txt");

/// RFC 9336 as corpus-build stores it: its text with the blocks its XML marks, and its XML beside it; with an
/// erratum whose text and XML patches each have their own lines.
fn rfc9336() -> Service {
    let blocks = glint_extract::declared_blocks(RFC9336_XML, RFC9336_TXT)
        .unwrap()
        .into_iter()
        .map(|b| DocumentBlock {
            start: b.range.start,
            end: b.range.end,
            indent: b.indent,
            bar: b.bar,
            breaks: b.breaks.into_iter().map(|r| [r.start, r.end]).collect(),
            type_attr: b.type_attr,
            appendix: b.section.as_ref().is_some_and(|s| s.appendix),
            section: b.section.map(|s| s.number),
            title: b.title,
        })
        .collect();
    let mut data = data();
    data.documents.push(DocumentRecord {
        id: "rfc9336".to_owned(),
        body: "rfc".to_owned(),
        status: None,
        obsoleted_by: Vec::new(),
        date: None,
        text_format: Some("text".to_owned()),
        errata: vec![AppliedErratum {
            id: 4321,
            by: "claude".to_owned(),
            note: "a correction".to_owned(),
            patch: "--- a/rfc9336.txt\n".to_owned(),
            hunks: vec![HunkLines {
                start_line: 10,
                end_line: 11,
            }],
            xml: Some(ErratumPatch {
                patch: "--- a/rfc9336.xml\n".to_owned(),
                hunks: vec![HunkLines {
                    start_line: 20,
                    end_line: 21,
                }],
            }),
        }],
        errata_failure: None,
    });
    let mut service = Service::default();
    service.load_corpus(&data.to_json()).unwrap();
    let texts = vec![
        DocumentText {
            blocks: Some(blocks),
            xml: Some(RFC9336_XML.to_owned()),
            ..text("rfc9336", RFC9336_TXT)
        },
        text("draft-ietf-x-mine-02", DRAFT_TEXT),
    ];
    service
        .load_documents(Documents::write(BUILD, &texts))
        .unwrap();
    service
}

#[test]
fn an_xml_rfc_opens_as_its_text_or_its_xml_and_each_names_the_other() {
    let service = rfc9336();
    let open = |request: serde_json::Value| {
        json(
            &service
                .check_document(serde_json::from_value(request).unwrap())
                .unwrap(),
        )
    };
    let text = open(serde_json::json!({ "rfc": 9336 }));
    assert_eq!(
        text,
        open(serde_json::json!({ "rfc": 9336, "format": "text" }))
    );
    let xml = open(serde_json::json!({ "rfc": 9336, "format": "xml" }));

    let url = "https://www.rfc-editor.org/rfc/rfc9336.xml";
    assert_eq!(text["document"]["name"], "rfc9336");
    assert_eq!(text["document"]["format"], "text");
    assert_eq!(text["document"]["content"], RFC9336_TXT);
    assert_eq!(text["document"]["other_format"], "xml");
    assert_eq!(text["document"]["xml_source"], url);
    assert_eq!(xml["document"]["name"], "rfc9336.xml");
    assert_eq!(xml["document"]["format"], "xml");
    assert_eq!(xml["document"]["content"], RFC9336_XML);
    assert_eq!(xml["document"]["other_format"], "text");
    assert_eq!(xml["document"]["xml_source"], url);
    assert!(xml["document"].get("markup_findings").is_none());

    // The two untyped `<sourcecode>` elements: W202 on the XML's elements, a count in the text.
    let w202 = |r: &serde_json::Value| codes(r).iter().filter(|c| *c == "W202").count();
    assert_eq!(w202(&xml), 2, "{xml}");
    assert_eq!(w202(&text), 0, "{text}");
    assert_eq!(
        text["document"]["markup_findings"],
        serde_json::json!({ "errors": 0, "warnings": 2, "infos": 0 })
    );
    // Everything else is the same in both views, with the same modules.
    let others = |r: &serde_json::Value| -> Vec<(String, String, String)> {
        let mut list: Vec<_> = r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["code"] != "W202")
            .map(|d| {
                let field = |k: &str| d[k].as_str().unwrap().to_owned();
                (field("code"), field("severity"), field("message"))
            })
            .collect();
        list.sort();
        list
    };
    assert_eq!(others(&text), others(&xml));
    let names = |r: &serde_json::Value| -> Vec<String> {
        r["modules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(!names(&text).is_empty());
    assert_eq!(names(&text), names(&xml));
    // The XML view's positions are in the XML.
    let start = xml["modules"][0]["start"].as_u64().unwrap();
    assert!(RFC9336_XML[..usize::try_from(start).unwrap()].contains("<sourcecode"));

    // Each view's errata are its own patches and lines.
    let erratum = |r: &serde_json::Value| r["document"]["errata"][0].clone();
    assert_eq!(erratum(&text)["patch"], "--- a/rfc9336.txt\n");
    assert_eq!(
        erratum(&text)["hunks"],
        serde_json::json!([{"start_line": 10, "end_line": 11}])
    );
    assert!(erratum(&text).get("xml").is_none());
    assert_eq!(erratum(&xml)["patch"], "--- a/rfc9336.xml\n");
    assert_eq!(
        erratum(&xml)["hunks"],
        serde_json::json!([{"start_line": 20, "end_line": 21}])
    );
    assert_eq!(erratum(&xml)["id"], 4321);
    assert!(erratum(&xml).get("xml").is_none());
}

const FRODOKEM: &str = "draft-ietf-lamps-frodokem-certificates-00";
const FRODOKEM_XML: &str = include_str!(
    "../../glint-extract/tests/fixtures/xml/draft-ietf-lamps-frodokem-certificates-00.xml"
);
const FRODOKEM_TXT: &str = include_str!(
    "../../glint-extract/tests/fixtures/xml/draft-ietf-lamps-frodokem-certificates-00.txt"
);

/// A draft as corpus-build stores it: its paginated text with the blocks its XML marks and their page breaks, and its
/// XML beside it.
fn frodokem() -> Service {
    let blocks: Vec<DocumentBlock> = glint_extract::declared_blocks(FRODOKEM_XML, FRODOKEM_TXT)
        .unwrap()
        .into_iter()
        .map(|b| DocumentBlock {
            start: b.range.start,
            end: b.range.end,
            indent: b.indent,
            bar: b.bar,
            breaks: b.breaks.into_iter().map(|r| [r.start, r.end]).collect(),
            type_attr: b.type_attr,
            appendix: b.section.as_ref().is_some_and(|s| s.appendix),
            section: b.section.map(|s| s.number),
            title: b.title,
        })
        .collect();
    assert!(blocks.iter().any(|b| !b.breaks.is_empty()));
    let mut data = data();
    data.documents.push(DocumentRecord {
        id: FRODOKEM.to_owned(),
        body: "draft".to_owned(),
        status: None,
        obsoleted_by: Vec::new(),
        date: Some("2026-10-27".to_owned()),
        text_format: Some("text".to_owned()),
        errata: Vec::new(),
        errata_failure: None,
    });
    let mut service = Service::default();
    service.load_corpus(&data.to_json()).unwrap();
    let texts = vec![DocumentText {
        blocks: Some(blocks),
        xml: Some(FRODOKEM_XML.to_owned()),
        ..text(FRODOKEM, FRODOKEM_TXT)
    }];
    service
        .load_documents(Documents::write(BUILD, &texts))
        .unwrap();
    service
}

#[test]
fn an_xml_draft_opens_as_its_paginated_text_or_its_xml() {
    let service = frodokem();
    let open = |request: serde_json::Value| {
        json(
            &service
                .check_document(serde_json::from_value(request).unwrap())
                .unwrap(),
        )
    };
    let name = "draft-ietf-lamps-frodokem-certificates";
    let text = open(serde_json::json!({ "draft": name }));
    assert_eq!(
        text,
        open(serde_json::json!({ "draft": FRODOKEM, "format": "text" }))
    );
    let xml = open(serde_json::json!({ "draft": name, "rev": "00", "format": "xml" }));
    assert_eq!(
        xml,
        open(serde_json::json!({ "draft": FRODOKEM, "format": "xml" }))
    );

    let url = "https://www.ietf.org/archive/id/draft-ietf-lamps-frodokem-certificates-00.xml";
    assert_eq!(text["document"]["name"], FRODOKEM);
    assert_eq!(text["document"]["format"], "text");
    assert_eq!(text["document"]["content"], FRODOKEM_TXT);
    assert_eq!(text["document"]["other_format"], "xml");
    assert_eq!(text["document"]["xml_source"], url);
    assert_eq!(xml["document"]["name"], format!("{FRODOKEM}.xml"));
    assert_eq!(xml["document"]["format"], "xml");
    assert_eq!(xml["document"]["content"], FRODOKEM_XML);
    assert_eq!(xml["document"]["other_format"], "text");
    assert_eq!(xml["document"]["xml_source"], url);
    assert_eq!(text["document"]["errata"], serde_json::json!([]));

    // The untyped module element: W202 on the XML, counted in the text.
    let w202 = |r: &serde_json::Value| codes(r).iter().filter(|c| *c == "W202").count();
    assert_eq!(w202(&xml), 1, "{xml}");
    assert_eq!(w202(&text), 0, "{text}");
    assert_eq!(
        text["document"]["markup_findings"],
        serde_json::json!({ "errors": 0, "warnings": 1, "infos": 0 })
    );
    // Everything else is the same in both views, and every position in the text is outside its page breaks.
    let others = |r: &serde_json::Value| -> Vec<(String, String)> {
        let mut list: Vec<_> = r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["code"] != "W202")
            .map(|d| {
                let field = |k: &str| d[k].as_str().unwrap().to_owned();
                (field("code"), field("message"))
            })
            .collect();
        list.sort();
        list
    };
    assert!(!others(&text).is_empty());
    assert_eq!(others(&text), others(&xml));
    let module = &text["modules"][0];
    assert_eq!(module["name"], "X509-FRODOKEM-2026");
    assert_eq!(module["name"], xml["modules"][0]["name"]);
    let (start, end) = (
        usize::try_from(module["start"].as_u64().unwrap()).unwrap(),
        usize::try_from(module["end"].as_u64().unwrap()).unwrap(),
    );
    let span = &FRODOKEM_TXT[start..end];
    assert!(span.contains("   X509-FRODOKEM-2026\n") && span.contains("\n   END\n"));
    // The module runs across the page breaks, which the check left out.
    assert_eq!(span.matches("[Page ").count(), 4);
}

#[test]
fn a_document_without_a_second_view_refuses_it() {
    let service = rfc9336();
    let ask = |request: serde_json::Value| {
        service
            .check_document(serde_json::from_value(request).unwrap())
            .err()
            .unwrap()
    };
    let error = ask(serde_json::json!({ "draft": "draft-ietf-x-mine", "format": "xml" }));
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("as text only"), "{}", error.message);
    let bad: Result<DocumentRequest, _> =
        serde_json::from_value(serde_json::json!({ "rfc": 9336, "format": "markdown" }));
    assert!(bad.is_err());
    // A document has no other view unless the corpus has it.
    let draft = json(
        &service
            .check_document(serde_json::from_str(r#"{"draft": "draft-ietf-x-mine"}"#).unwrap())
            .unwrap(),
    );
    assert!(draft["document"].get("other_format").is_none());
    assert!(draft["document"].get("markup_findings").is_none());
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
    let texts = vec![text("draft-ietf-x-mine-02", DRAFT_TEXT)];
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
fn a_document_checked_by_name_carries_its_errata() {
    let mut data = data();
    let patch = "--- a/rfc1000.txt\n+++ b/rfc1000.txt\n@@ -3 +3 @@\n-  Name ::= BOOLEAN\n+  Name ::= INTEGER\n";
    data.documents.push(DocumentRecord {
        id: "rfc1000".to_owned(),
        body: "rfc".to_owned(),
        status: None,
        obsoleted_by: Vec::new(),
        date: None,
        text_format: Some("text".to_owned()),
        errata: vec![AppliedErratum {
            id: 1234,
            by: "human".to_owned(),
            note: "Name is an INTEGER".to_owned(),
            patch: patch.to_owned(),
            hunks: vec![HunkLines {
                start_line: 3,
                end_line: 3,
            }],
            xml: None,
        }],
        errata_failure: None,
    });
    let mut service = Service::default();
    service.load_corpus(&data.to_json()).unwrap();
    let texts = vec![
        text("rfc1000", BASE),
        text("draft-ietf-x-mine-02", DRAFT_TEXT),
    ];
    service
        .load_documents(Documents::write(BUILD, &texts))
        .unwrap();
    let rfc: DocumentRequest = serde_json::from_str(r#"{"rfc": 1000}"#).unwrap();
    let document = json(&service.check_document(rfc).unwrap())["document"].clone();
    assert_eq!(
        document["errata"],
        serde_json::json!([{
            "id": 1234,
            "by": "human",
            "note": "Name is an INTEGER",
            "patch": patch,
            "hunks": [{"start_line": 3, "end_line": 3}],
        }])
    );
    assert!(document.get("errata_failure").is_none());

    // A document without errata says so; pasted text has no errata at all.
    let draft: DocumentRequest = serde_json::from_str(r#"{"draft": "draft-ietf-x-mine"}"#).unwrap();
    let document = json(&service.check_document(draft).unwrap())["document"].clone();
    assert_eq!(document["errata"], serde_json::json!([]));
    let pasted: CheckRequest =
        serde_json::from_str(r#"{"content": "M DEFINITIONS ::= BEGIN END"}"#).unwrap();
    let document = json(&service.check(pasted).unwrap())["document"].clone();
    assert!(document.get("errata").is_none());
}

#[test]
fn corpus_queries_answer_like_the_old_api() {
    let service = service();
    let page = json(&service.modules("bas", 0, None));
    assert_eq!(page["total"], 1);
    assert_eq!(page["modules"][0]["name"], "Base");
    for query in ["rfc1000", "1000", "RFC1000#ba", "rfc1000#Base"] {
        assert_eq!(json(&service.modules(query, 0, None))["total"], 1, "{query}");
    }
    assert_eq!(json(&service.modules("rfc1001", 0, None))["total"], 0);
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
