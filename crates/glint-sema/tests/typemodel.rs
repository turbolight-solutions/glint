#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use std::path::Path;

use common::{Rec, corpus};
use glint_sema::typemodel::{TypeNode, resolve_module, resolve_type};
use glint_sema::{DraftSource, Options, analyze};
use glint_syntax::{ParseOptions, parse_source_with};

fn read(dir: &str, name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir).join(name);
    std::fs::read_to_string(&path).unwrap()
}

fn sources(texts: &[String], options: ParseOptions) -> Vec<DraftSource> {
    texts
        .iter()
        .map(|text| DraftSource {
            parse: parse_source_with(text, options),
            text: text.clone(),
        })
        .collect()
}

fn model(texts: &[String], module: &str, ty: &str) -> TypeNode {
    let options = Options {
        oid_collisions: false,
        policy_checks: false,
        resolution_notes: false,
        ..Options::default()
    };
    let empty = corpus(&[] as &[Rec<'_>]);
    let analysis = analyze(sources(texts, ParseOptions::corpus()), &empty, &options);
    let id = analysis
        .graph
        .modules
        .iter()
        .find(|m| m.model.name == module)
        .unwrap()
        .id;
    resolve_type(&analysis.graph, id, ty).unwrap()
}

fn json(node: &TypeNode) -> String {
    serde_json::to_string_pretty(node).unwrap()
}

fn tag_of(node: &TypeNode) -> String {
    node.tags
        .iter()
        .map(|t| format!("{} {}", t.class, t.number))
        .collect::<Vec<_>>()
        .join(", ")
}

fn component<'a>(node: &'a TypeNode, name: &str) -> &'a TypeNode {
    &node
        .components
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no component {name}"))
        .ty
}

const M1: &str = "M1 DEFINITIONS AUTOMATIC TAGS ::= BEGIN
EXPORTS T1;
T1 ::= SET { f1 INTEGER, f2 BOOLEAN }
END";

// X.683 9.8: tagging environment of the actual parameter, first module
#[test]
fn x683_9_8_t3_actual_keeps_its_own_automatic_tags() {
    let m2 = "M2 DEFINITIONS EXPLICIT TAGS ::= BEGIN
IMPORTS T1 FROM M1;
T3 ::= T2{T1}
T2{X} ::= SEQUENCE { a INTEGER, b X }
END";
    let t3 = model(&[M1.to_owned(), m2.to_owned()], "M2", "T3");
    let instantiated = t3.instantiation.as_deref().unwrap();
    let a = component(instantiated, "a");
    assert_eq!(tag_of(a), "UNIVERSAL 2");
    assert!(a.mode.is_none());
    let b = component(instantiated, "b");
    assert_eq!(tag_of(b), "UNIVERSAL 17");
    assert!(b.mode.is_none());
    let t1 = b.expanded.as_deref().unwrap();
    let f1 = component(t1, "f1");
    assert_eq!(tag_of(f1), "CONTEXT 0");
    assert_eq!(f1.mode, Some(glint_sema::typemodel::TaggingMode::Implicit));
    let f2 = component(t1, "f2");
    assert_eq!(tag_of(f2), "CONTEXT 1");
    assert_eq!(f2.mode, Some(glint_sema::typemodel::TaggingMode::Implicit));
}

// X.683 9.8: the dummy reference is always explicitly tagged under AUTOMATIC TAGS
#[test]
fn x683_9_8_t5_dummy_is_explicitly_tagged() {
    let m3 = "M3 DEFINITIONS AUTOMATIC TAGS ::= BEGIN
IMPORTS T1 FROM M1;
T5 ::= T4{T1}
T4{Y} ::= SEQUENCE { a INTEGER, b Y }
END";
    let t5 = model(&[M1.to_owned(), m3.to_owned()], "M3", "T5");
    let instantiated = t5.instantiation.as_deref().unwrap();
    let a = component(instantiated, "a");
    assert_eq!(tag_of(a), "CONTEXT 0");
    assert_eq!(a.mode, Some(glint_sema::typemodel::TaggingMode::Implicit));
    let b = component(instantiated, "b");
    assert_eq!(tag_of(b), "CONTEXT 1");
    assert_eq!(b.mode, Some(glint_sema::typemodel::TaggingMode::Explicit));
    let t1 = b.expanded.as_deref().unwrap();
    assert_eq!(tag_of(component(t1, "f1")), "CONTEXT 0");
    assert_eq!(tag_of(component(t1, "f2")), "CONTEXT 1");
}

#[test]
fn recursive_parameterized_definitions_stay_finite() {
    let text = "M DEFINITIONS EXPLICIT TAGS ::= BEGIN
List1 { E } ::= SEQUENCE { elem E, next List1 { E } OPTIONAL }
IntegerList1 ::= List1 { INTEGER }
END"
    .to_owned();
    let node = model(&[text], "M", "IntegerList1");
    let inst = node.instantiation.as_deref().unwrap();
    let next = component(inst, "next");
    assert_eq!(next.reference.as_deref(), Some("List1"));
    assert_eq!(next.instantiation.as_deref().unwrap().kind, "Recursive");
}

#[test]
fn defaults_and_named_values_are_evaluated() {
    let text = "M DEFINITIONS EXPLICIT TAGS ::= BEGIN
Level ::= INTEGER { low(1), high(10) }
Mode ::= ENUMERATED { fast, slow, ... }
T ::= SEQUENCE {
  a INTEGER DEFAULT 5,
  b BOOLEAN DEFAULT TRUE,
  c OBJECT IDENTIFIER DEFAULT { 1 2 3 },
  d BIT STRING { x(0), y(1) } DEFAULT { y },
  e Mode DEFAULT slow,
  f Level (1..10),
  g OCTET STRING (SIZE (4..8)),
  ... }
END"
    .to_owned();
    let node = model(std::slice::from_ref(&text), "M", "T");
    let value = |name: &str| {
        node.components
            .iter()
            .find(|c| c.name == name)
            .unwrap()
            .default
            .clone()
    };
    use glint_sema::typemodel::DefaultValue::*;
    assert_eq!(value("a"), Some(Integer(5)));
    assert_eq!(value("b"), Some(Boolean(true)));
    assert_eq!(value("c"), Some(Oid("1.2.3".to_owned())));
    assert_eq!(value("d"), Some(Bits(vec!["y".to_owned()])));
    assert_eq!(value("e"), Some(Enumerated("slow".to_owned())));
    assert!(node.extensible);
    let g = component(&node, "g");
    assert_eq!(
        g.constraints[0].size,
        Some(("4".to_owned(), "8".to_owned()))
    );
    let level = model(std::slice::from_ref(&text), "M", "Level");
    assert_eq!(level.named_values.len(), 2);
    assert_eq!(level.named_values[1].value, Some(10));
    let mode = model(&[text], "M", "Mode");
    assert!(mode.extensible);
}

#[test]
fn module_model_lists_every_type() {
    let text = "M DEFINITIONS IMPLICIT TAGS ::= BEGIN
A ::= [APPLICATION 3] INTEGER
B ::= SEQUENCE OF A
C OP ::= ANY
END"
    .to_owned();
    let options = Options::default();
    let empty = corpus(&[] as &[Rec<'_>]);
    let analysis = analyze(sources(&[text], ParseOptions::corpus()), &empty, &options);
    let types = resolve_module(&analysis.graph, analysis.drafts[0]).unwrap();
    assert_eq!(types.tag_default, "IMPLICIT");
    assert_eq!(
        types
            .types
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    let a = &types.types[0].ty;
    assert_eq!(tag_of(a), "APPLICATION 3");
    assert_eq!(a.mode, Some(glint_sema::typemodel::TaggingMode::Implicit));
}

#[test]
fn instance_of_is_its_equivalent_sequence() {
    let text = "M DEFINITIONS EXPLICIT TAGS ::= BEGIN
T ::= INSTANCE OF TYPE-IDENTIFIER
END"
    .to_owned();
    let node = model(&[text], "M", "T");
    assert_eq!(node.kind, "SEQUENCE");
    assert_eq!(tag_of(&node), "UNIVERSAL 8");
    let value = component(&node, "value");
    assert_eq!(tag_of(value), "CONTEXT 0");
    assert_eq!(
        value.mode,
        Some(glint_sema::typemodel::TaggingMode::Explicit)
    );
}

fn rfc5280() -> Vec<String> {
    vec![
        read(
            "../glint-syntax/tests/fixtures/rfc5280",
            "PKIX1Explicit88.asn",
        ),
        read(
            "../glint-syntax/tests/fixtures/rfc5280",
            "PKIX1Implicit88.asn",
        ),
    ]
}

fn rfc5912() -> Vec<String> {
    [
        "AlgorithmInformation-2009",
        "PKIX-CommonTypes-2009",
        "PKIX-X400Address-2009",
        "PKIX1-PSS-OAEP-Algorithms-2009",
        "PKIXAlgs-2009",
        "PKIX1Explicit-2009",
        "PKIX1Implicit-2009",
    ]
    .iter()
    .map(|name| read("tests/fixtures/rfc5912", &format!("{name}.asn")))
    .collect()
}

#[test]
fn rfc_5280_validity_and_time() {
    insta::assert_snapshot!(
        "rfc5280_validity",
        json(&model(&rfc5280(), "PKIX1Explicit88", "Validity"))
    );
    insta::assert_snapshot!(
        "rfc5280_time",
        json(&model(&rfc5280(), "PKIX1Explicit88", "Time"))
    );
}

#[test]
fn rfc_5280_tbs_certificate() {
    insta::assert_snapshot!(
        "rfc5280_tbs_certificate",
        json(&model(&rfc5280(), "PKIX1Explicit88", "TBSCertificate"))
    );
}

#[test]
fn rfc_5280_algorithm_identifier_and_extension() {
    insta::assert_snapshot!(
        "rfc5280_algorithm_identifier",
        json(&model(&rfc5280(), "PKIX1Explicit88", "AlgorithmIdentifier"))
    );
    insta::assert_snapshot!(
        "rfc5280_extension",
        json(&model(&rfc5280(), "PKIX1Explicit88", "Extension"))
    );
}

#[test]
fn rfc_5912_extension_links_its_open_type_to_the_object_set() {
    let extension = model(&rfc5912(), "PKIX-CommonTypes-2009", "Extension");
    let value = component(&extension, "extnValue");
    let contents = value
        .constraints
        .iter()
        .find(|c| c.kind == "contents")
        .unwrap();
    assert!(contents.text.contains("&ExtnType"));
    let link = value.open_type.as_ref().unwrap();
    assert_eq!(link.class, "EXTENSION");
    assert_eq!(link.field, "&ExtnType");
    assert_eq!(link.referenced, ["@extnID"]);
    insta::assert_snapshot!("rfc5912_extension", json(&extension));
}

#[test]
fn rfc_5912_tbs_certificate_instantiates_extensions_per_use() {
    let tbs = model(&rfc5912(), "PKIX1Explicit-2009", "TBSCertificate");
    insta::assert_snapshot!("rfc5912_tbs_certificate", json(&tbs));
}

#[test]
fn rfc_5912_single_attribute() {
    insta::assert_snapshot!(
        "rfc5912_single_attribute",
        json(&model(
            &rfc5912(),
            "PKIX-CommonTypes-2009",
            "SingleAttribute"
        ))
    );
}
