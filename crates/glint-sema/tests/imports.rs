#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod common;
use common::*;
use glint_sema::Options;

const BASE: &str = "Base { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN
    Name ::= INTEGER
    Other ::= BOOLEAN
    id-base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 }
    END";

fn base_corpus() -> glint_corpus::Corpus {
    corpus(&[Rec {
        name: "Base",
        oid: Some("1.3.6.1.4.1.99.1"),
        rfc: 1000,
        text: BASE,
    }])
}

fn run(text: &str) -> Vec<&'static str> {
    without_notes_codes(check_with(text, &base_corpus(), &Options::default()))
}

fn without_notes_codes(findings: Vec<glint_sema::Finding>) -> Vec<&'static str> {
    codes(&without_notes(findings))
}

#[test]
fn an_import_from_the_corpus_resolves_without_files() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 };
        T ::= SEQUENCE { n Name } END";
    assert!(run(text).is_empty());
}

// X.680 13.12
#[test]
fn import_without_the_definitive_identifier_is_warned_about() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base;
        T ::= SEQUENCE { n Name } END";
    assert_eq!(run(text), ["W106"]);
}

// W100
#[test]
fn unknown_module_is_opaque_and_reported_once() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS A, B FROM Nowhere;
        T ::= SEQUENCE { a A, b B } END";
    assert_eq!(run(text), ["W100"]);
}

// X.680 13.16 a
#[test]
fn symbol_missing_from_the_target_module() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Missing FROM Base { 1 3 6 1 4 1 99 1 };
        T ::= SEQUENCE { n Missing } END";
    assert_eq!(run(text), ["E101"]);
}

// X.680 13.16 b
#[test]
fn symbol_not_exported_by_the_target_module() {
    let base = "Base { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN EXPORTS Name; Name ::= INTEGER Hidden ::= BOOLEAN END";
    let corpus = corpus(&[Rec {
        name: "Base",
        oid: Some("1.3.6.1.4.1.99.1"),
        rfc: 1000,
        text: base,
    }]);
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Hidden FROM Base { 1 3 6 1 4 1 99 1 };
        T ::= SEQUENCE { n Hidden } END";
    let findings = without_notes(check_with(text, &corpus, &Options::default()));
    assert_eq!(codes(&findings), ["E101"]);
}

// X.680 13.10, 13.11
#[test]
fn oid_is_definitive_over_the_module_name() {
    let text = "Renamed DEFINITIONS ::= BEGIN END";
    let _ = text;
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Different { 1 3 6 1 4 1 99 1 };
        T ::= SEQUENCE { n Name } END";
    assert!(run(text).is_empty());
}

// W102
#[test]
fn name_match_with_a_different_oid_warns() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base { 1 3 6 1 4 1 99 2 };
        T ::= SEQUENCE { n Name } END";
    assert_eq!(run(text), ["W102"]);
}

// X.680 13.16 e
#[test]
fn importing_the_same_module_twice_is_an_error() {
    let text = "M DEFINITIONS ::= BEGIN
        IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 } Other FROM Base { 1 3 6 1 4 1 99 1 };
        T ::= SEQUENCE { n Name, o Other } END";
    assert_eq!(run(text), ["E104"]);
}

#[test]
fn a_module_cannot_import_itself() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS X FROM M; T ::= SEQUENCE { x X } END";
    assert!(run(text).contains(&"E104"));
}

// design spec: several candidates
#[test]
fn several_candidates_are_reported_as_a_note() {
    let corpus = corpus(&[
        Rec {
            name: "Base",
            oid: None,
            rfc: 1000,
            text: BASE,
        },
        Rec {
            name: "Base",
            oid: None,
            rfc: 2000,
            text: "Base DEFINITIONS ::= BEGIN Name ::= INTEGER END",
        },
    ]);
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base; T ::= SEQUENCE { n Name } END";
    let findings = check_with(text, &corpus, &Options::default());
    assert!(has(&findings, "I103"));
}

#[test]
fn a_pin_overrides_the_choice() {
    let corpus = corpus(&[
        Rec {
            name: "Base",
            oid: None,
            rfc: 1000,
            text: "Base DEFINITIONS ::= BEGIN Old ::= INTEGER END",
        },
        Rec {
            name: "Base",
            oid: None,
            rfc: 2000,
            text: "Base DEFINITIONS ::= BEGIN New ::= INTEGER END",
        },
    ]);
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Old FROM Base; T ::= SEQUENCE { n Old } END";
    let mut options = Options::default();
    assert!(has(&check_with(text, &corpus, &options), "E101"));
    options.pins.insert("Base".to_owned(), "rfc1000".to_owned());
    assert!(!has(&check_with(text, &corpus, &options), "E101"));
}

/// One RFC may define two modules with the same name and different OIDs (RFC 4998 does).
fn twin_corpus() -> glint_corpus::Corpus {
    corpus(&[
        Rec {
            name: "Twin",
            oid: Some("1.2.3.1"),
            rfc: 1000,
            text: "Twin { 1 2 3 1 } DEFINITIONS ::= BEGIN Old ::= INTEGER END",
        },
        Rec {
            name: "Twin",
            oid: Some("1.2.3.2"),
            rfc: 1000,
            text: "Twin { 1 2 3 2 } DEFINITIONS ::= BEGIN New ::= INTEGER END",
        },
    ])
}

#[test]
fn an_import_with_an_oid_picks_the_module_of_that_oid_among_same_named_ones() {
    let corpus = twin_corpus();
    for (symbol, oid) in [("Old", "1 2 3 1"), ("New", "1 2 3 2")] {
        let text = format!(
            "M DEFINITIONS ::= BEGIN IMPORTS {symbol} FROM Twin {{ {oid} }}; T ::= SEQUENCE {{ n {symbol} }} END"
        );
        let findings = without_notes(check_with(&text, &corpus, &Options::default()));
        assert!(findings.is_empty(), "{symbol}: {:?}", codes(&findings));
    }
}

#[test]
fn a_pin_can_name_the_oid_of_one_of_several_same_named_modules() {
    let corpus = twin_corpus();
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Old FROM Twin; T ::= SEQUENCE { n Old } END";
    let mut options = Options::default();
    // Name only: the later edition (the larger OID) is preferred, which has no `Old`.
    assert!(has(&check_with(text, &corpus, &options), "E101"));
    // `rfc1000` cannot say which; the later edition is chosen again.
    options.pins.insert("Twin".to_owned(), "rfc1000".to_owned());
    assert!(has(&check_with(text, &corpus, &options), "E101"));
    options.pins.insert("Twin".to_owned(), "1.2.3.1".to_owned());
    assert!(!has(&check_with(text, &corpus, &options), "E101"));
}

// X.680 13.16 f
#[test]
fn with_successors_picks_the_greatest_incremented_oid() {
    let corpus = corpus(&[
        Rec {
            name: "V",
            oid: Some("1.2.3.1"),
            rfc: 1,
            text: "V DEFINITIONS ::= BEGIN A ::= INTEGER END",
        },
        Rec {
            name: "V",
            oid: Some("1.2.3.3"),
            rfc: 2,
            text: "V DEFINITIONS ::= BEGIN A ::= INTEGER B ::= INTEGER END",
        },
    ]);
    let text = "M DEFINITIONS ::= BEGIN IMPORTS B FROM V { 1 2 3 1 } WITH SUCCESSORS; T ::= SEQUENCE { b B } END";
    let findings = without_notes(check_with(text, &corpus, &Options::default()));
    assert!(findings.is_empty(), "{:?}", codes(&findings));
}

#[test]
fn with_descendants_picks_the_largest_oid_below() {
    let corpus = corpus(&[
        Rec {
            name: "V",
            oid: Some("1.2.3.1"),
            rfc: 1,
            text: "V DEFINITIONS ::= BEGIN A ::= INTEGER END",
        },
        Rec {
            name: "V",
            oid: Some("1.2.3.1.7"),
            rfc: 2,
            text: "V DEFINITIONS ::= BEGIN A ::= INTEGER B ::= INTEGER END",
        },
    ]);
    let text = "M DEFINITIONS ::= BEGIN IMPORTS B FROM V { 1 2 3 } WITH DESCENDANTS; T ::= SEQUENCE { b B } END";
    let findings = without_notes(check_with(text, &corpus, &Options::default()));
    assert!(findings.is_empty(), "{:?}", codes(&findings));
}

// X.680 13.9
#[test]
fn assigned_identifier_may_use_a_local_value() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base base-oid;
        base-oid OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 1 }
        T ::= SEQUENCE { n Name } END";
    assert!(run(text).is_empty());
}

#[test]
fn assigned_identifier_may_not_use_an_undefined_value() {
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base missing-oid;
        T ::= SEQUENCE { n Name } END";
    assert!(run(text).contains(&"E107"));
}

#[test]
fn module_in_the_same_draft_is_found_by_name() {
    let text = "A DEFINITIONS ::= BEGIN EXPORTS X; X ::= INTEGER END
        B DEFINITIONS ::= BEGIN IMPORTS X FROM A; T ::= SEQUENCE { x X } END";
    assert!(run(text).is_empty());
}

#[test]
fn module_in_the_same_draft_is_found_by_oid() {
    let text = "A { 1 2 3 4 } DEFINITIONS ::= BEGIN X ::= INTEGER END
        B DEFINITIONS ::= BEGIN IMPORTS X FROM Renamed { 1 2 3 4 }; T ::= SEQUENCE { x X } END";
    assert!(run(text).is_empty());
}

// X.680 13.16 a: a symbol the target itself imports can be re-imported once
#[test]
fn a_symbol_imported_once_by_the_target_can_be_reimported() {
    let text = "A DEFINITIONS ::= BEGIN IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 }; END
        B DEFINITIONS ::= BEGIN IMPORTS Name FROM A; T ::= SEQUENCE { n Name } END";
    assert!(run(text).iter().all(|c| *c != "E101"));
}

// I105
#[test]
fn curated_modules_are_announced_once() {
    use glint_corpus::{Bundle, Corpus, OverrideTier, SourceKind};
    let mut corpus_bundle = Bundle {
        modules: base_corpus().into_bundle().modules,
        ..Bundle::default()
    };
    corpus_bundle.modules[0].source.kind = SourceKind::Overridden {
        tier: OverrideTier::ErrataVerified,
        errata_ids: vec!["1234".to_owned()],
    };
    let corpus = Corpus::from_bundle(corpus_bundle);
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 }; T ::= SEQUENCE { n Name } END";
    let findings = check_with(text, &corpus, &Options::default());
    assert_eq!(codes(&findings), ["I105"]);
    assert!(findings[0].message.contains("errata 1234"));
}
