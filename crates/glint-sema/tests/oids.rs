#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod common;
use common::*;
use glint_corpus::{AllowedOid, Corpus, CorpusData, IanaModuleId, OidAssignment, OidLabels};
use glint_sema::Options;

fn run(text: &str) -> Vec<&'static str> {
    codes(&check(text))
}

fn module(body: &str) -> String {
    format!("M DEFINITIONS ::= BEGIN {body} END")
}

// X.660 via X.680 32.11
#[test]
fn oid_with_one_arc_is_an_error() {
    assert_eq!(run(&module("o OBJECT IDENTIFIER ::= { 1 }")), ["E130"]);
}

#[test]
fn first_arc_above_two_is_an_error() {
    assert_eq!(run(&module("o OBJECT IDENTIFIER ::= { 3 1 }")), ["E130"]);
}

#[test]
fn second_arc_of_forty_under_iso_is_an_error() {
    assert_eq!(run(&module("o OBJECT IDENTIFIER ::= { 1 40 }")), ["E130"]);
}

#[test]
fn large_second_arc_under_joint_is_legal() {
    assert!(run(&module("o OBJECT IDENTIFIER ::= { 2 999 }")).is_empty());
}

#[test]
fn registered_names_are_legal_forms() {
    let text = module(
        "o OBJECT IDENTIFIER ::= { iso identified-organization 6 1 }
         p OBJECT IDENTIFIER ::= { joint-iso-itu-t 5 }
         q OBJECT IDENTIFIER ::= { iso(1) member-body(2) us(840) 1 }",
    );
    assert!(run(&text).is_empty());
}

// X.680 32.7
#[test]
fn bare_name_that_x660_does_not_register() {
    assert_eq!(
        run(&module(
            "o OBJECT IDENTIFIER ::= { iso identified-organization dod 1 }"
        )),
        ["E134"]
    );
}

#[test]
fn bare_name_registered_elsewhere_is_not_registered_here() {
    assert_eq!(
        run(&module("o OBJECT IDENTIFIER ::= { iso recommendation 1 }")),
        ["E134"]
    );
}

#[test]
fn oid_built_on_a_local_value() {
    let text = module(
        "base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 }
         child OBJECT IDENTIFIER ::= { base 5 }
         leaf OBJECT IDENTIFIER ::= { child leafarc(2) }",
    );
    assert!(run(&text).is_empty());
}

#[test]
fn undefined_parent_value() {
    assert_eq!(
        run(&module("o OBJECT IDENTIFIER ::= { missing 5 }")),
        ["E110"]
    );
}

#[test]
fn integer_value_in_the_first_position_is_not_an_oid() {
    let text = module("n INTEGER ::= 5 o OBJECT IDENTIFIER ::= { n 5 }");
    assert_eq!(run(&text), ["E131"]);
}

#[test]
fn number_form_can_be_an_integer_value() {
    let text = module("n INTEGER ::= 5 o OBJECT IDENTIFIER ::= { 1 3 big(n) }");
    assert!(run(&text).is_empty());
}

// I141
#[test]
fn placeholder_arcs_are_noted() {
    let findings = check(&module(
        "base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 } o OBJECT IDENTIFIER ::= { base id-TBD }",
    ));
    assert!(has(&findings, "I141"), "{:?}", codes(&findings));
    assert!(!has(&findings, "E110"));
}

fn corpus_with_assignment(oid: &str, name: &str) -> Corpus {
    Corpus::from_data(CorpusData {
        oid_assignments: vec![OidAssignment {
            oid: oid.parse().unwrap(),
            name: name.to_owned(),
            module: "PKIX1Explicit88".to_owned(),
            document: "rfc5280".to_owned(),
        }],
        ..CorpusData::default()
    })
}

// W140
#[test]
fn oid_already_assigned_in_the_corpus_to_another_name() {
    let corpus = corpus_with_assignment("1.3.6.1.5.5.7.1.1", "id-pe-authorityInfoAccess");
    let text = module(
        "id-pkix OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7) }
         id-pe-mine OBJECT IDENTIFIER ::= { id-pkix 1 1 }",
    );
    let findings = check_with(&text, &corpus, &Options::default());
    assert_eq!(codes(&findings), ["W140"]);
    assert!(findings[0].message.contains("RFC 5280"));
}

#[test]
fn same_name_for_the_same_oid_is_not_a_collision() {
    let corpus = corpus_with_assignment("1.3.6.1.5.5.7.1.1", "id-pe-authorityInfoAccess");
    let text = module(
        "id-pkix OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7) }
         id-pe-authorityInfoAccess OBJECT IDENTIFIER ::= { id-pkix 1 1 }",
    );
    assert!(check_with(&text, &corpus, &Options::default()).is_empty());
}

#[test]
fn collisions_are_not_checked_for_corpus_modules() {
    let corpus = corpus_with_assignment("1.3.6.1.5.5.7.1.1", "id-pe-authorityInfoAccess");
    let text = module("o OBJECT IDENTIFIER ::= { 1 3 6 1 5 5 7 1 1 }");
    let options = Options {
        oid_collisions: false,
        ..Options::default()
    };
    assert!(check_with(&text, &corpus, &options).is_empty());
}

#[test]
fn module_identifier_already_held_by_another_module() {
    let corpus = corpus(&[Rec {
        name: "Existing",
        oid: Some("1.3.6.1.4.1.99.1"),
        rfc: 1,
        text: "Existing { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN END",
    }]);
    let text = "Mine { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN END";
    let findings = check_with(text, &corpus, &Options::default());
    assert_eq!(codes(&findings), ["W140"]);
}

#[test]
fn oid_in_a_module_header_is_validated() {
    assert_eq!(run("Mine { 1 } DEFINITIONS ::= BEGIN END"), ["E130"]);
}

/// A corpus whose label table has `dod(6)` under 1.3 and `id-aa(2)` under smime(16) in three RFCs, `dod(4)` in RFC
/// 2797, and holdInstruction on the allowlist.
fn labelled_corpus() -> Corpus {
    let mut triples = Vec::new();
    for doc in ["rfc5280", "rfc5652", "rfc5912"] {
        for (oid, label) in [
            ("1.2", "member-body"),
            ("1.2.840", "us"),
            ("1.2.840.113549", "rsadsi"),
            ("1.2.840.113549.1", "pkcs"),
            ("1.2.840.113549.1.9", "pkcs-9"),
            ("1.2.840.113549.1.9.16", "smime"),
            ("1.2.840.113549.1.9.16.2", "id-aa"),
            ("1.3", "identified-organization"),
            ("1.3.6", "dod"),
            ("1.3.6.1", "internet"),
        ] {
            triples.push((oid.parse().unwrap(), label.to_owned(), doc.to_owned()));
        }
    }
    triples.push((
        "1.3.4".parse().unwrap(),
        "dod".to_owned(),
        "rfc2797".to_owned(),
    ));
    let labels = OidLabels::collect(
        triples,
        vec![AllowedOid {
            oid: "2.2.840.10040.2".parse().unwrap(),
            reason: "deployed".to_owned(),
        }],
    );
    Corpus::from_data(CorpusData {
        oid_nodes: labels.nodes(),
        oid_allowlist: labels.allowlist().to_vec(),
        ..CorpusData::default()
    })
}

// W142
#[test]
fn a_label_with_another_number_under_the_same_parent() {
    let text = module(
        "o OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(4) internet(1) 5 }",
    );
    let findings = check_with(&text, &labelled_corpus(), &Options::default());
    assert_eq!(codes(&findings), ["W142"]);
    assert!(
        findings[0].message.contains("`dod(4)`"),
        "{}",
        findings[0].message
    );
    assert!(findings[0].help.as_deref().unwrap().contains("dod(6)"));
    let right = module(
        "o OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(6) internet(1) 5 }",
    );
    assert!(check_with(&right, &labelled_corpus(), &Options::default()).is_empty());
}

#[test]
fn a_document_is_not_evidence_about_itself() {
    let text = module("o OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(4) 1 }");
    let as_2797 = Options {
        own_document: Some("rfc2797".to_owned()),
        ..Options::default()
    };
    assert_eq!(
        codes(&check_with(&text, &labelled_corpus(), &as_2797)),
        ["W142"]
    );
    let right = module("o OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(6) 1 }");
    let as_5280 = Options {
        own_document: Some("rfc5280".to_owned()),
        ..Options::default()
    };
    assert!(check_with(&right, &labelled_corpus(), &as_5280).is_empty());
}

// W143
#[test]
fn a_label_under_another_parent() {
    let text = module(
        "id-aa-multipleSignatures OBJECT IDENTIFIER ::= { iso(1) member-body(2) us(840) rsadsi(113549) pkcs(1) pkcs-9(9) id-aa(2) 51 }",
    );
    let findings = check_with(&text, &labelled_corpus(), &Options::default());
    assert_eq!(codes(&findings), ["W143"]);
    assert!(
        findings[0].message.contains("1.2.840.113549.1.9.16"),
        "{}",
        findings[0].message
    );
}

#[test]
fn label_checks_skip_allowlisted_oids_and_can_be_turned_off() {
    let hold = module(
        "holdInstruction OBJECT IDENTIFIER ::= { joint-iso-itu-t(2) member-body(2) us(840) x9cm(10040) 2 }",
    );
    assert!(check_with(&hold, &labelled_corpus(), &Options::default()).is_empty());
    let wrong = module("o OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(4) 1 }");
    let off = Options {
        oid_labels: false,
        ..Options::default()
    };
    assert!(check_with(&wrong, &labelled_corpus(), &off).is_empty());
}

// I145
#[test]
fn another_name_for_an_arc_is_reported_only_when_asked() {
    let text = module("o OBJECT IDENTIFIER ::= { iso(1) org(3) 6 }");
    assert!(check_with(&text, &labelled_corpus(), &Options::default()).is_empty());
    let options = Options {
        oid_synonyms: true,
        ..Options::default()
    };
    let findings = check_with(&text, &labelled_corpus(), &options);
    assert_eq!(codes(&findings), ["I145"]);
    assert!(findings[0].message.contains("identified-organization"));
}

// W144
#[test]
fn an_import_identifier_of_another_module() {
    let corpus = corpus(&[
        Rec {
            name: "Explicit",
            oid: Some("1.3.6.1.5.5.7.0.18"),
            rfc: 5280,
            text: "Explicit { 1 3 6 1 5 5 7 0 18 } DEFINITIONS ::= BEGIN Certificate ::= INTEGER END",
        },
        Rec {
            name: "Implicit",
            oid: Some("1.3.6.1.5.5.7.0.19"),
            rfc: 5280,
            text: "Implicit { 1 3 6 1 5 5 7 0 19 } DEFINITIONS ::= BEGIN Certificate ::= INTEGER END",
        },
    ]);
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Certificate FROM Explicit { 1 3 6 1 5 5 7 0 19 }; T ::= Certificate END";
    let findings = without_notes(check_with(text, &corpus, &Options::default()));
    assert_eq!(codes(&findings), ["W144"]);
    assert!(findings[0].message.contains("1.3.6.1.5.5.7.0.18"));
}

#[test]
fn upper_case_placeholders_are_noted_not_malformed() {
    let findings = check(&module(
        "base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 } a OBJECT IDENTIFIER ::= { base id-a(TBD1) } b OBJECT IDENTIFIER ::= { base TBD } c OBJECT IDENTIFIER ::= { base id-c(MODNUM) }",
    ));
    assert_eq!(codes(&findings), ["I141", "I141", "I141"]);
    let header = check("M { 1 3 6 1 4 1 99 id-mod(0) id-mod-m(TBD) } DEFINITIONS ::= BEGIN END");
    assert_eq!(codes(&header), ["I141"]);
    assert_eq!(
        run(&module("o OBJECT IDENTIFIER ::= { 1 3 Foo }")),
        ["E131"]
    );
}

const SMIME_49: &str = "1.2.840.113549.1.9.16.0.49";
const PKIX_49: &str = "1.3.6.1.5.5.7.0.49";

fn iana(oid: &str, description: &str, documents: &[&str], errata: &[&str]) -> IanaModuleId {
    IanaModuleId {
        oid: oid.parse().unwrap(),
        description: description.to_owned(),
        registry: "SMI Security for S/MIME Module Identifier (1.2.840.113549.1.9.16.0)".to_owned(),
        documents: documents.iter().map(|d| (*d).to_owned()).collect(),
        errata: errata.iter().map(|e| (*e).to_owned()).collect(),
    }
}

/// RFC 5912 and RFC 6210 as erratum 9255 finds them, with the IANA entries of both arcs.
fn registry_corpus() -> Corpus {
    let modules = corpus(&[
        Rec {
            name: "AttributeCertificateVersion1-2009",
            oid: Some(SMIME_49),
            rfc: 5912,
            text: "AttributeCertificateVersion1-2009 { 1 2 840 113549 1 9 16 0 49 } DEFINITIONS ::= BEGIN AttributeCertificateV1 ::= INTEGER END",
        },
        Rec {
            name: "MD5-HASH-EXPERIMENT",
            oid: Some("1.2.840.113549.1.9.16.0.999"),
            rfc: 6210,
            text: "MD5-HASH-EXPERIMENT { 1 2 840 113549 1 9 16 0 999 } DEFINITIONS ::= BEGIN END",
        },
        Rec {
            name: "PKIX1Explicit88",
            oid: Some("1.3.6.1.5.5.7.0.18"),
            rfc: 5280,
            text: "PKIX1Explicit88 { 1 3 6 1 5 5 7 0 18 } DEFINITIONS ::= BEGIN END",
        },
    ]);
    let mut data = modules.into_data();
    data.iana_module_ids = vec![
        iana(
            SMIME_49,
            "id-mod-MD5-XOR-EXPERIMENT",
            &["rfc6210"],
            &["3866"],
        ),
        iana(PKIX_49, "id-mod-v1AttrCert-02", &["rfc5912"], &[]),
        iana(
            "1.3.6.1.5.5.7.0.18",
            "id-mod-pkix1-explicit",
            &["rfc5280"],
            &[],
        ),
        iana(
            "1.3.6.1.5.5.7.0.20",
            "id-mod-qualified-cert-88",
            &["rfc3039"],
            &[],
        ),
        iana("1.3.6.1.5.5.7.0.21", "id-mod-foo", &["rfc5280"], &[]),
    ];
    Corpus::from_data(data)
}

fn as_document(document: &str) -> Options {
    Options {
        own_document: Some(document.to_owned()),
        ..Options::default()
    }
}

// W146
#[test]
fn a_module_identifier_that_iana_registers_for_another_module() {
    let text = "AttributeCertificateVersion1-2009
        { iso(1) member-body(2) us(840) rsadsi(113549) pkcs(1) pkcs-9(9) smime(16) modules(0) id-mod-v1AttrCert-02(49) }
        DEFINITIONS ::= BEGIN END";
    let findings = check_with(text, &registry_corpus(), &as_document("rfc5912"));
    assert_eq!(codes(&findings), ["W146"]);
    let message = &findings[0].message;
    assert!(message.contains("`id-mod-MD5-XOR-EXPERIMENT`"), "{message}");
    assert!(message.contains("RFC 6210 (erratum 3866)"), "{message}");

    let fixed = "AttributeCertificateVersion1-2009
        { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7) id-mod(0)
          id-mod-v1AttrCert-02(49) }
        DEFINITIONS ::= BEGIN END";
    assert!(check_with(fixed, &registry_corpus(), &as_document("rfc5912")).is_empty());
}

#[test]
fn a_draft_that_reuses_a_registered_module_identifier() {
    let text = "NewModule-2026 { 1 3 6 1 5 5 7 0 49 } DEFINITIONS ::= BEGIN END";
    let findings = check_with(
        text,
        &registry_corpus(),
        &as_document("draft-ietf-lamps-new-00"),
    );
    assert_eq!(codes(&findings), ["W146"]);
    assert!(
        findings[0].message.contains("for RFC 5912"),
        "{}",
        findings[0].message
    );
}

#[test]
fn an_entry_citing_a_later_edition_of_the_module_is_the_modules() {
    // IANA cites RFC 5280 for the module that RFC 3280 published first.
    let text = "PKIX1Explicit88 { 1 3 6 1 5 5 7 0 18 } DEFINITIONS ::= BEGIN END";
    assert!(check_with(text, &registry_corpus(), &as_document("rfc3280")).is_empty());
}

#[test]
fn an_entry_whose_label_matches_or_whose_documents_are_unknown_is_not_judged() {
    // RFC 5280 does not define `Foo`, but the label is the entry's, its `id-mod-` prefix aside.
    let labelled = "Foo { 1 3 6 1 5 5 7 0 foo(21) } DEFINITIONS ::= BEGIN END";
    assert!(check_with(labelled, &registry_corpus(), &as_document("rfc9999")).is_empty());
    let unlabelled = "Foo { 1 3 6 1 5 5 7 0 21 } DEFINITIONS ::= BEGIN END";
    assert_eq!(
        codes(&check_with(
            unlabelled,
            &registry_corpus(),
            &as_document("rfc9999")
        )),
        ["W146"]
    );
    let unknown = "Qualified { 1 3 6 1 5 5 7 0 20 } DEFINITIONS ::= BEGIN END";
    assert!(
        check_with(unknown, &registry_corpus(), &as_document("rfc9999")).is_empty(),
        "RFC 3039 is not in the corpus"
    );
    let unregistered = "Fresh { 1 3 6 1 5 5 7 0 500 } DEFINITIONS ::= BEGIN END";
    assert!(check_with(unregistered, &registry_corpus(), &as_document("rfc9999")).is_empty());
}

#[test]
fn registry_checks_are_off_with_the_label_checks() {
    let text = "NewModule-2026 { 1 3 6 1 5 5 7 0 49 } DEFINITIONS ::= BEGIN END";
    let options = Options {
        oid_labels: false,
        ..as_document("draft-ietf-lamps-new-00")
    };
    assert!(check_with(text, &registry_corpus(), &options).is_empty());
}

// W102 with the registry note
#[test]
fn an_import_with_the_registered_identifier_says_which_side_iana_takes() {
    let text = "M DEFINITIONS ::= BEGIN
        IMPORTS AttributeCertificateV1 FROM AttributeCertificateVersion1-2009
            { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) pkix(7) id-mod(0)
              id-mod-v1AttrCert-02(49) };
        T ::= AttributeCertificateV1
        END";
    let findings = without_notes(check_with(text, &registry_corpus(), &Options::default()));
    assert_eq!(codes(&findings), ["W102"]);
    let notes = findings[0].notes.join("\n");
    assert!(
        notes.contains("IANA registers 1.3.6.1.5.5.7.0.49 as `id-mod-v1AttrCert-02` for RFC 5912"),
        "{notes}"
    );
    assert!(notes.contains("`id-mod-MD5-XOR-EXPERIMENT`"), "{notes}");

    // Without the registry, W102 says only what X.680 says.
    let plain = corpus(&[Rec {
        name: "AttributeCertificateVersion1-2009",
        oid: Some(SMIME_49),
        rfc: 5912,
        text: "AttributeCertificateVersion1-2009 { 1 2 840 113549 1 9 16 0 49 } DEFINITIONS ::= BEGIN AttributeCertificateV1 ::= INTEGER END",
    }]);
    let findings = without_notes(check_with(text, &plain, &Options::default()));
    assert_eq!(codes(&findings), ["W102"]);
    assert_eq!(findings[0].notes.len(), 1);
}
