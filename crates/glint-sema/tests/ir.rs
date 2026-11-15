#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

//! The IR of the fixture roots (Codegen and Decoder Design, Testing: IR).

mod common;

use std::path::Path;

use common::{Rec, corpus};
use glint_ir::{Constraint, ElementSet, IrSet, Kind, Presence, Type, Value};
use glint_sema::{DraftSource, Options, analyze};
use glint_syntax::{ParseOptions, parse_source_with};

fn fixture(path: &str) -> String {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{path}.asn"));
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

fn rfc5280(path: &str) -> String {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../glint-syntax/tests/fixtures/rfc5280")
        .join(format!("{path}.asn"));
    std::fs::read_to_string(full).unwrap()
}

const RFC5912: [&str; 7] = [
    "rfc5912/AlgorithmInformation-2009",
    "rfc5912/PKIX-CommonTypes-2009",
    "rfc5912/PKIX-X400Address-2009",
    "rfc5912/PKIX1-PSS-OAEP-Algorithms-2009",
    "rfc5912/PKIXAlgs-2009",
    "rfc5912/PKIX1Explicit-2009",
    "rfc5912/PKIX1Implicit-2009",
];

fn lower_texts(texts: Vec<String>, module: &str, ty: &str) -> IrSet {
    let options = Options {
        oid_collisions: false,
        policy_checks: false,
        resolution_notes: false,
        ..Options::default()
    };
    let empty = corpus(&[] as &[Rec<'_>]);
    let sources = texts
        .iter()
        .map(|text| DraftSource {
            parse: parse_source_with(text, ParseOptions::corpus()),
            text: text.clone(),
        })
        .collect();
    let analysis = analyze(sources, &empty, &options);
    let id = analysis
        .graph
        .modules
        .iter()
        .find(|m| m.model.name == module)
        .unwrap_or_else(|| panic!("no module {module}"))
        .id;
    glint_sema::ir::lower(&analysis.graph, id, ty).unwrap()
}

fn lower(paths: &[&str], module: &str, ty: &str) -> IrSet {
    lower_texts(paths.iter().map(|p| fixture(p)).collect(), module, ty)
}

fn rfc5912_with(extra: &[&str]) -> Vec<&'static str> {
    let mut all: Vec<&'static str> = RFC5912.to_vec();
    for path in extra {
        all.push(Box::leak(path.to_string().into_boxed_str()));
    }
    all
}

// ---- walking the IR -------------------------------------------------------------------------

fn each_type<'a>(ir: &'a IrSet, f: &mut dyn FnMut(&'a Type)) {
    fn walk<'a>(ty: &'a Type, f: &mut dyn FnMut(&'a Type)) {
        f(ty);
        match &ty.kind {
            Kind::Sequence(r) | Kind::Set(r) | Kind::Choice(r) => {
                for c in &r.components {
                    walk(&c.ty, f);
                }
            }
            Kind::SequenceOf(c) | Kind::SetOf(c) => walk(&c.element, f),
            _ => {}
        }
        for c in &ty.constraints {
            constraint(c, f);
        }
    }
    fn constraint<'a>(c: &'a Constraint, f: &mut dyn FnMut(&'a Type)) {
        set(&c.root, f);
        if let Some(a) = &c.additions {
            set(a, f);
        }
    }
    fn set<'a>(e: &'a ElementSet, f: &mut dyn FnMut(&'a Type)) {
        match e {
            ElementSet::Union(items) | ElementSet::Intersection(items) => {
                for i in items {
                    set(i, f);
                }
            }
            ElementSet::Except(a, b) => {
                set(a, f);
                set(b, f);
            }
            ElementSet::Size(c) | ElementSet::From(c) | ElementSet::WithComponent(c) => {
                constraint(c, f);
            }
            ElementSet::ContainedSubtype(t) => walk(t, f),
            ElementSet::Contents { ty: Some(t), .. } => walk(t, f),
            _ => {}
        }
    }
    for def in &ir.types {
        walk(&def.ty, f);
    }
    for table in &ir.object_sets {
        for row in &table.rows {
            for ty in row.types.values() {
                walk(ty, f);
            }
        }
    }
}

fn opaque_constraints(set: &IrSet) -> Vec<String> {
    fn find(e: &ElementSet, out: &mut Vec<String>) {
        match e {
            ElementSet::Opaque(text) => out.push(text.clone()),
            ElementSet::Union(items) | ElementSet::Intersection(items) => {
                items.iter().for_each(|i| find(i, out));
            }
            ElementSet::Except(a, b) => {
                find(a, out);
                find(b, out);
            }
            ElementSet::Size(c) | ElementSet::From(c) | ElementSet::WithComponent(c) => {
                find(&c.root, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    each_type(set, &mut |ty| {
        for c in &ty.constraints {
            find(&c.root, &mut out);
            if let Some(a) = &c.additions {
                find(a, &mut out);
            }
        }
    });
    out
}

fn opaque_types(set: &IrSet) -> Vec<String> {
    let mut out = Vec::new();
    each_type(set, &mut |ty| {
        if let Kind::Opaque { reason } = &ty.kind {
            out.push(reason.clone());
        }
    });
    out
}

fn def<'a>(set: &'a IrSet, name: &str) -> &'a glint_ir::TypeDef {
    set.types
        .iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("no type {name}"))
}

fn component<'a>(set: &'a IrSet, ty: &'a Type, name: &str) -> &'a glint_ir::Component {
    match &set.resolve(ty).kind {
        Kind::Sequence(r) | Kind::Set(r) | Kind::Choice(r) => r
            .components
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no component {name}")),
        other => panic!("not a record: {other:?}"),
    }
}

// ---- acceptance -----------------------------------------------------------------------------

// Milestone 1: no opaque constraint in the RFC 5912 Certificate closure
#[test]
fn rfc5912_certificate_has_no_opaque_constraints() {
    let set = lower(&RFC5912, "PKIX1Explicit-2009", "Certificate");
    assert_eq!(opaque_constraints(&set), Vec::<String>::new());
    assert_eq!(opaque_types(&set), Vec::<String>::new());
    assert_eq!(set.notes, Vec::new());
}

#[test]
fn rfc5912_extensions_dispatch_through_the_extension_set() {
    let set = lower(&RFC5912, "PKIX1Explicit-2009", "Certificate");
    let extension = def(&set, "ExtensionCertExtensions");
    let value = component(&set, &extension.ty, "extnValue");
    let contents = value
        .ty
        .constraints
        .iter()
        .find_map(|c| match &c.root {
            ElementSet::Contents { ty: Some(ty), .. } => Some(ty),
            _ => None,
        })
        .expect("CONTAINING");
    let Kind::OpenType(open) = &contents.kind else {
        panic!("not an open type");
    };
    let table = open.table.as_ref().expect("table");
    assert_eq!(table.field, "&ExtnType");
    assert_eq!(table.relation, vec!["extnID".to_owned()]);
    let rows = &set.object_set(table.set).unwrap().rows;
    let key_usage = rows
        .iter()
        .find(|r| r.key_name.as_deref() == Some("id-ce-keyUsage"))
        .expect("keyUsage row");
    assert_eq!(key_usage.key, Some(Value::Oid("2.5.29.15".to_owned())));
    let ty = &key_usage.types["&ExtnType"];
    let Kind::Reference { target } = ty.kind else {
        panic!("not a reference");
    };
    assert_eq!(set.get(target).unwrap().name, "KeyUsage");
    assert!(
        set.oids
            .iter()
            .any(|o| o.name == "id-ce-keyUsage" && o.oid == "2.5.29.15")
    );
}

#[test]
fn rfc5912_parameterized_types_are_monomorphized_with_their_values() {
    let set = lower(&RFC5912, "PKIX1Explicit-2009", "Certificate");
    let directory = def(&set, "DirectoryStringUbName");
    assert_eq!(directory.origin.asn1_name, "DirectoryString");
    assert_eq!(directory.origin.actuals, ["ub-name"]);
    let teletex = component(&set, &directory.ty, "teletexString");
    let size = teletex.ty.summary.size.expect("size summary");
    assert_eq!((size.lo, size.hi), (Some(1), Some(32768)));
}

#[test]
fn rfc5912_version_default_is_its_number() {
    let set = lower(&RFC5912, "PKIX1Explicit-2009", "Certificate");
    let tbs = def(&set, "TBSCertificate");
    let version = component(&set, &tbs.ty, "version");
    assert_eq!(version.presence, Presence::Default(Value::Integer(0)));
}

// ---- snapshots ------------------------------------------------------------------------------

#[test]
fn snapshot_rfc5912_certificate() {
    let set = lower(&RFC5912, "PKIX1Explicit-2009", "Certificate");
    insta::assert_snapshot!(glint_ir::dump(&set));
}

#[test]
fn snapshot_rfc5280_certificate() {
    let set = lower_texts(
        vec![rfc5280("PKIX1Explicit88"), rfc5280("PKIX1Implicit88")],
        "PKIX1Explicit88",
        "Certificate",
    );
    assert_eq!(opaque_constraints(&set), Vec::<String>::new());
    insta::assert_snapshot!(glint_ir::dump(&set));
}

#[test]
fn snapshot_rfc6268_content_info() {
    let paths = rfc5912_with(&[
        "rfc5912/AttributeCertificateVersion1-2009",
        "rfc5912/PKIXAttributeCertificate-2009",
        "rfc5911/CryptographicMessageSyntaxAlgorithms-2009",
        "rfc5911/SecureMimeMessageV3dot1-2009",
        "rfc5911/CryptographicMessageSyntax-2009",
        "rfc6268/CryptographicMessageSyntax-2010",
    ]);
    let set = lower(&paths, "CryptographicMessageSyntax-2010", "ContentInfo");
    insta::assert_snapshot!(glint_ir::dump(&set));
}

#[test]
fn snapshot_rfc5652_content_info() {
    let mut texts = vec![rfc5280("PKIX1Explicit88"), rfc5280("PKIX1Implicit88")];
    texts.push(fixture("rfc5652/AttributeCertificateVersion1"));
    texts.push(fixture("rfc5652/CryptographicMessageSyntax2004"));
    let set = lower_texts(texts, "CryptographicMessageSyntax2004", "ContentInfo");
    insta::assert_snapshot!(glint_ir::dump(&set));
}

#[test]
fn snapshot_rfc5958_one_asymmetric_key() {
    let paths = rfc5912_with(&[
        "rfc5912/AttributeCertificateVersion1-2009",
        "rfc5912/PKIXAttributeCertificate-2009",
        "rfc5911/CryptographicMessageSyntaxAlgorithms-2009",
        "rfc5911/SecureMimeMessageV3dot1-2009",
        "rfc5911/CryptographicMessageSyntax-2009",
        "rfc5958/AsymmetricKeyPackageModuleV1",
    ]);
    let set = lower(&paths, "AsymmetricKeyPackageModuleV1", "OneAsymmetricKey");
    insta::assert_snapshot!(glint_ir::dump(&set));
}

#[test]
fn snapshot_rfc3161_time_stamp_resp() {
    let mut texts = vec![rfc5280("PKIX1Explicit88"), rfc5280("PKIX1Implicit88")];
    for path in [
        "rfc3369/CryptographicMessageSyntax",
        "rfc4210/PKIXCMP",
        "rfc3161/PKIXTSP",
    ] {
        texts.push(fixture(path));
    }
    let set = lower_texts(texts, "PKIXTSP", "TimeStampResp");
    insta::assert_snapshot!(glint_ir::dump(&set));
}

#[test]
fn snapshot_rfc6960_ocsp_response() {
    let paths = rfc5912_with(&["rfc6960/OCSP-2013-08"]);
    let set = lower(&paths, "OCSP-2013-08", "OCSPResponse");
    insta::assert_snapshot!(glint_ir::dump(&set));
}

// ---- unit cases -----------------------------------------------------------------------------

fn lower_one(text: &str, ty: &str) -> IrSet {
    let module = text.split_whitespace().next().unwrap().to_owned();
    lower_texts(vec![text.to_owned()], &module, ty)
}

#[test]
fn constraint_algebra_is_kept_and_summarized() {
    let set = lower_one(
        "M DEFINITIONS AUTOMATIC TAGS ::= BEGIN
T ::= SEQUENCE {
  a INTEGER (0..10 | 20<..<30, ...),
  b INTEGER ((1..100) EXCEPT 50),
  c IA5String (SIZE (1..8) ^ FROM (\"a\"..\"z\")),
  d PrintableString (SIZE (2)),
  e Small (5..MAX),
  f SEQUENCE SIZE (1..4) OF INTEGER,
  g S (WITH COMPONENTS { ..., x PRESENT }),
  h INTEGER (ALL EXCEPT 0)
}
Small ::= INTEGER (0..7)
S ::= SEQUENCE { x INTEGER OPTIONAL, y BOOLEAN OPTIONAL }
END",
        "T",
    );
    let t = def(&set, "T");
    let summary = |name: &str| component(&set, &t.ty, name).ty.summary.clone();
    let a = summary("a").value.unwrap();
    assert_eq!((a.lo, a.hi, a.extensible), (Some(0), Some(29), true));
    let b = summary("b").value.unwrap();
    assert_eq!((b.lo, b.hi), (Some(1), Some(100)));
    let c = summary("c");
    assert_eq!(c.size.map(|s| (s.lo, s.hi)), Some((Some(1), Some(8))));
    assert_eq!(c.alphabet.unwrap().ranges, vec![(97, 122)]);
    let d = summary("d").size.unwrap();
    assert_eq!((d.lo, d.hi), (Some(2), Some(2)));
    let e = summary("e").value.unwrap();
    assert_eq!((e.lo, e.hi), (Some(5), Some(7)));
    let f = summary("f").size.unwrap();
    assert_eq!((f.lo, f.hi), (Some(1), Some(4)));
    let g = &component(&set, &t.ty, "g").ty.constraints[0].root;
    let ElementSet::WithComponents { partial, items } = g else {
        panic!("not WITH COMPONENTS: {g:?}");
    };
    assert!(*partial);
    assert_eq!(
        items[0].presence,
        Some(glint_ir::PresenceConstraint::Present)
    );
    // the EXCEPT hole is not representable as one range; PER sees the whole first operand
    let h = summary("h");
    assert_eq!(h.value.map(|r| (r.lo, r.hi)), Some((None, None)));
    insta::assert_snapshot!(glint_ir::dump(&set));
}

#[test]
fn value_sets_constrain_the_types_that_use_them() {
    let set = lower_one(
        "M DEFINITIONS ::= BEGIN
T ::= INTEGER (Primes)
Primes INTEGER ::= { 2 | 3 | 5 | 7, ... }
U ::= Primes
END",
        "T",
    );
    let t = def(&set, "T");
    let value = t.ty.summary.value.unwrap();
    assert_eq!(
        (value.lo, value.hi, value.extensible),
        (Some(2), Some(7), false)
    );
    let set = lower_one(
        "M DEFINITIONS ::= BEGIN
Primes INTEGER ::= { 2 | 3 | 5 | 7, ... }
U ::= Primes
END",
        "U",
    );
    let primes = def(&set, "Primes");
    let value = primes.ty.summary.value.unwrap();
    assert_eq!(
        (value.lo, value.hi, value.extensible),
        (Some(2), Some(7), true)
    );
}

#[test]
fn recursive_types_are_references_to_themselves() {
    let set = lower_one(
        "M DEFINITIONS ::= BEGIN
Name ::= SEQUENCE { label UTF8String, next Name OPTIONAL }
END",
        "Name",
    );
    assert_eq!(set.types.len(), 1);
    let next = component(&set, &set.types[0].ty, "next");
    assert_eq!(next.ty.kind, Kind::Reference { target: set.root });
}

#[test]
fn enumerations_are_numbered_by_x680_20() {
    let set = lower_one(
        "M DEFINITIONS ::= BEGIN
E ::= ENUMERATED { a, b(0), c, ..., d, e(10), f }
END",
        "E",
    );
    let Kind::Enumerated {
        root,
        additions,
        extensible,
    } = &set.types[0].ty.kind
    else {
        panic!("not ENUMERATED");
    };
    let numbers = |items: &[glint_ir::NamedNumber]| {
        items
            .iter()
            .map(|n| (n.name.clone(), n.value))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        numbers(root),
        vec![("a".into(), 1), ("b".into(), 0), ("c".into(), 2)]
    );
    assert_eq!(
        numbers(additions),
        vec![("d".into(), 3), ("e".into(), 10), ("f".into(), 11)]
    );
    assert!(extensible);
}

#[test]
fn tags_follow_the_module_default_and_x680_31_2_7() {
    let set = lower_one(
        "M DEFINITIONS IMPLICIT TAGS ::= BEGIN
T ::= SEQUENCE {
  a [0] INTEGER,
  b [1] EXPLICIT INTEGER,
  c [2] C,
  d [APPLICATION 3] IMPLICIT [4] EXPLICIT BOOLEAN
}
C ::= CHOICE { x INTEGER, y BOOLEAN }
END",
        "T",
    );
    let t = def(&set, "T");
    let tags = |name: &str| {
        component(&set, &t.ty, name)
            .ty
            .tags
            .iter()
            .map(|l| format!("{:?} {} {:?}", l.tag.class, l.tag.number, l.mode))
            .collect::<Vec<_>>()
    };
    assert_eq!(tags("a"), ["Context 0 Implicit"]);
    assert_eq!(tags("b"), ["Context 1 Explicit"]);
    assert_eq!(tags("c"), ["Context 2 Explicit"]);
    assert_eq!(tags("d"), ["Application 3 Implicit", "Context 4 Explicit"]);
}

#[test]
fn the_ir_round_trips_through_json() {
    let set = lower(&RFC5912, "PKIX1Explicit-2009", "Certificate");
    let json = serde_json::to_string(&set).unwrap();
    let back: IrSet = serde_json::from_str(&json).unwrap();
    assert_eq!(back, set);
}

#[test]
fn instance_of_with_a_table_selects_the_value_type() {
    let set = lower_one(
        "M DEFINITIONS IMPLICIT TAGS ::= BEGIN
G ::= CHOICE { otherName [0] INSTANCE OF OTHER-NAME ({SupportedOtherNames}) }
OTHER-NAME ::= TYPE-IDENTIFIER
SupportedOtherNames OTHER-NAME ::= { on-upn, ... }
on-upn OTHER-NAME ::= { UTF8String IDENTIFIED BY { 1 3 6 1 4 1 311 20 2 3 } }
END",
        "G",
    );
    let other = component(&set, &set.types[0].ty, "otherName");
    let value = component(&set, &other.ty, "value");
    let Kind::OpenType(open) = &value.ty.kind else {
        panic!("not an open type");
    };
    assert_eq!(open.class.as_deref(), Some("OTHER-NAME"));
    let table = open.table.as_ref().expect("table");
    assert_eq!(table.relation, ["type-id"]);
    let rows = &set.object_set(table.set).unwrap().rows;
    assert_eq!(
        rows[0].key,
        Some(Value::Oid("1.3.6.1.4.1.311.20.2.3".to_owned()))
    );
    assert!(matches!(
        rows[0].types["&Type"].kind,
        Kind::String {
            string: glint_ir::StringKind::UTF8String
        }
    ));
}

#[test]
fn structured_defaults_are_evaluated() {
    let set = lower(&RFC5912, "PKIX1Explicit-2009", "Certificate");
    let pss = def(&set, "RSASSA-PSS-params");
    let hash = component(&set, &pss.ty, "hashAlgorithm");
    let Presence::Default(Value::Record(fields)) = &hash.presence else {
        panic!("not a record default: {:?}", hash.presence);
    };
    assert_eq!(
        fields[0],
        (
            "algorithm".to_owned(),
            Value::Oid("1.3.14.3.2.26".to_owned())
        )
    );
    assert_eq!(fields[1], ("parameters".to_owned(), Value::Null));
}
