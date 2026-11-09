mod common;
use common::*;
use glint_diag::Severity;
use glint_syntax::{ParseOptions, parse_source_with};

fn plain(body: &str) -> String {
    format!("M DEFINITIONS ::= BEGIN {body} END")
}

fn automatic(body: &str) -> String {
    format!("M DEFINITIONS AUTOMATIC TAGS ::= BEGIN {body} END")
}

fn implicit(body: &str) -> String {
    format!("M DEFINITIONS IMPLICIT TAGS ::= BEGIN {body} END")
}

fn tag_codes(text: &str) -> Vec<&'static str> {
    codes(&check(text))
}

// X.680 29.9 example 1
#[test]
fn choice_inside_choice_with_distinct_tags_is_legal() {
    let text = plain(
        "A ::= CHOICE { b B, c NULL }
         B ::= CHOICE { d [0] NULL, e [1] NULL }",
    );
    assert!(tag_codes(&text).is_empty());
}

// X.680 29.9 example 2
#[test]
fn two_choices_with_disjoint_tags_are_legal() {
    let text = plain(
        "A ::= CHOICE { b B, c C }
         B ::= CHOICE { d [0] NULL, e [1] NULL }
         C ::= CHOICE { f [2] NULL, g [3] NULL }",
    );
    assert!(tag_codes(&text).is_empty());
}

// X.680 29.9 example 3
#[test]
fn two_choices_with_the_same_tags_clash() {
    let text = plain(
        "A ::= CHOICE { b B, c C }
         B ::= CHOICE { d [0] NULL, e [1] NULL }
         C ::= CHOICE { f [0] NULL, g [1] NULL }",
    );
    assert_eq!(tag_codes(&text), ["E120"]);
}

#[test]
fn automatic_tagging_removes_the_clash_of_29_9_example_3() {
    let text = automatic(
        "A ::= CHOICE { b B, c C }
         B ::= CHOICE { d [0] NULL, e [1] NULL }
         C ::= CHOICE { f [0] NULL, g [1] NULL }",
    );
    assert!(tag_codes(&text).is_empty());
}

// X.680 52.8.1
#[test]
fn extensible_choice_inside_a_set_is_legal_when_the_set_is_not_extensible() {
    let text = plain(
        "A ::= SET { a A2, b CHOICE { c C, d D, ... } }
         A2 ::= SEQUENCE { x INTEGER }
         C ::= [0] INTEGER
         D ::= [1] BOOLEAN",
    );
    assert!(tag_codes(&text).is_empty());
}

// X.680 52.8.2
#[test]
fn extensible_choice_next_to_an_extension_marker_clashes() {
    let text = plain(
        "A ::= SET { a A2, b CHOICE { c C, ... }, ... }
         A2 ::= SEQUENCE { x INTEGER }
         C ::= [0] INTEGER",
    );
    assert_eq!(tag_codes(&text), ["E120"]);
}

// X.680 52.8.3
#[test]
fn two_extensible_choices_in_a_set_clash() {
    let text = plain(
        "A ::= SET { a A2, b CHOICE { c C, ... }, d CHOICE { e E, ... } }
         A2 ::= SEQUENCE { x INTEGER }
         C ::= [0] INTEGER
         E ::= [1] INTEGER",
    );
    assert_eq!(tag_codes(&text), ["E120"]);
}

#[test]
fn extensibility_implied_makes_choices_extensible() {
    let text = "M DEFINITIONS EXTENSIBILITY IMPLIED ::= BEGIN
        A ::= SET { b CHOICE { c [0] INTEGER }, d CHOICE { e [1] INTEGER } }
        END";
    assert!(tag_codes(text).iter().all(|c| *c == "E120"));
    assert!(!tag_codes(text).is_empty());
}

#[test]
fn optional_component_with_the_tag_of_the_next_one_clashes() {
    let text = plain("T ::= SEQUENCE { a INTEGER OPTIONAL, b INTEGER }");
    assert_eq!(tag_codes(&text), ["E121"]);
}

#[test]
fn optional_component_with_a_different_tag_is_legal() {
    let text = plain("T ::= SEQUENCE { a INTEGER OPTIONAL, b BOOLEAN }");
    assert!(tag_codes(&text).is_empty());
}

#[test]
fn a_run_of_defaults_must_be_distinct_up_to_the_next_mandatory_component() {
    let text = plain("T ::= SEQUENCE { a INTEGER DEFAULT 1, b INTEGER DEFAULT 2, c OCTET STRING }");
    assert_eq!(tag_codes(&text), ["E121"]);
}

#[test]
fn mandatory_components_may_share_a_tag() {
    let text = plain("T ::= SEQUENCE { a INTEGER, b INTEGER }");
    assert!(tag_codes(&text).is_empty());
}

#[test]
fn explicit_tags_separate_optional_components() {
    let text = plain("T ::= SEQUENCE { a [0] INTEGER OPTIONAL, b [1] INTEGER }");
    assert!(tag_codes(&text).is_empty());
}

#[test]
fn tag_of_an_untagged_choice_counts_every_alternative() {
    let text = plain("T ::= SEQUENCE { a CHOICE { x INTEGER, y BOOLEAN } OPTIONAL, b BOOLEAN }");
    assert_eq!(tag_codes(&text), ["E121"]);
}

// X.680 25.7
#[test]
fn extension_addition_must_differ_from_the_following_root_component() {
    let text = plain("T ::= SEQUENCE { a INTEGER, ..., b INTEGER OPTIONAL, ..., c INTEGER }");
    assert_eq!(tag_codes(&text), ["E121"]);
}

#[test]
fn extension_addition_with_another_tag_is_legal() {
    let text = plain("T ::= SEQUENCE { a INTEGER, ..., b BOOLEAN OPTIONAL, ..., c INTEGER }");
    assert!(tag_codes(&text).is_empty());
}

// X.680 52.7: the conceptually added element
#[test]
fn optional_extensible_choice_before_the_extension_marker_clashes() {
    let text = plain("T ::= SEQUENCE { a CHOICE { x [0] INTEGER, ... } OPTIONAL, ... }");
    assert_eq!(tag_codes(&text), ["E121"]);
}

// X.681 14.2 b
#[test]
fn untagged_open_type_clashes_with_other_components_of_a_set() {
    let text = plain("T ::= SET { a TYPE-IDENTIFIER.&Type, b INTEGER }");
    assert_eq!(tag_codes(&text), ["E120"]);
}

#[test]
fn tagged_open_type_is_legal() {
    let text = plain("T ::= SET { a [0] TYPE-IDENTIFIER.&Type, b INTEGER }");
    assert!(tag_codes(&text).is_empty());
}

#[test]
fn automatic_tagging_covers_optional_runs() {
    let text = automatic("T ::= SEQUENCE { a INTEGER OPTIONAL, b INTEGER }");
    assert!(tag_codes(&text).is_empty());
}

// X.680 25.3
#[test]
fn one_textual_tag_switches_automatic_tagging_off() {
    let text = automatic("T ::= SEQUENCE { a INTEGER OPTIONAL, b INTEGER, c [5] BOOLEAN }");
    assert_eq!(tag_codes(&text), ["E121"]);
}

// X.680 25.9
#[test]
fn tagged_extension_addition_under_automatic_tags_is_an_error() {
    let text = automatic("T ::= SEQUENCE { a INTEGER, ..., b [0] INTEGER OPTIONAL }");
    assert_eq!(tag_codes(&text), ["E124"]);
}

#[test]
fn tagged_extension_addition_is_fine_when_the_root_is_tagged() {
    let text = automatic("T ::= SEQUENCE { a [0] INTEGER, ..., b [1] INTEGER OPTIONAL }");
    assert!(tag_codes(&text).is_empty());
}

// X.680 31.2.9
#[test]
fn implicit_tag_on_a_choice_is_an_error() {
    let text = plain("T ::= [1] IMPLICIT CHOICE { a INTEGER, b BOOLEAN }");
    assert_eq!(tag_codes(&text), ["E122"]);
}

#[test]
fn implicit_tag_on_a_reference_to_a_choice_is_an_error() {
    let text = plain("C ::= CHOICE { a INTEGER, b BOOLEAN } T ::= [1] IMPLICIT C");
    assert_eq!(tag_codes(&text), ["E122"]);
}

#[test]
fn implicit_tag_on_a_tagged_choice_is_legal() {
    let text = plain("C ::= [2] CHOICE { a INTEGER, b BOOLEAN } T ::= [1] IMPLICIT C");
    assert!(tag_codes(&text).is_empty());
}

#[test]
fn bare_tag_on_a_choice_under_implicit_tags_is_made_explicit_silently() {
    let text = implicit("T ::= [1] CHOICE { a INTEGER, b BOOLEAN }");
    assert!(tag_codes(&text).is_empty());
}

#[test]
fn implicit_tag_on_an_open_type_is_an_error() {
    let text = plain("T ::= SEQUENCE { a [0] IMPLICIT TYPE-IDENTIFIER.&Type }");
    assert_eq!(tag_codes(&text), ["E122"]);
}

// X.680 31.2.5
#[test]
fn universal_tag_in_a_user_module_is_an_error() {
    let text = plain("T ::= [UNIVERSAL 5] IMPLICIT INTEGER");
    assert_eq!(tag_codes(&text), ["E123"]);
}

fn pkix1_explicit_88(body: &str) -> String {
    format!(
        "PKIX1Explicit88 {{ iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5) \
         pkix(7) id-mod(0) id-pkix1-explicit(18) }} DEFINITIONS EXPLICIT TAGS ::= BEGIN {body} END"
    )
}

fn e123(findings: &[glint_sema::Finding]) -> Vec<&glint_sema::Finding> {
    findings.iter().filter(|f| f.code == "E123").collect()
}

// X.680 31.2.5; X.208 has no UniversalString, BMPString or UTF8String (RFC 5280's PKIX1Explicit88)
#[test]
fn a_published_1988_definition_of_a_later_built_in_string_type_is_not_reported() {
    for (name, tag) in [
        ("UniversalString", 28),
        ("BMPString", 30),
        ("UTF8String", 12),
    ] {
        let findings = check(&pkix1_explicit_88(&format!(
            "{name} ::= [UNIVERSAL {tag}] IMPLICIT OCTET STRING"
        )));
        assert!(e123(&findings).is_empty(), "{name}: {findings:?}");
    }
}

#[test]
fn every_listed_module_may_define_the_string_types() {
    for (module, arcs, _) in glint_sema::LEGACY_STRING_MODULES {
        let oid: Vec<String> = arcs.iter().map(u128::to_string).collect();
        let text = format!(
            "{module} {{ {} }} DEFINITIONS ::= BEGIN \
             UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING END",
            oid.join(" ")
        );
        let findings = check(&text);
        assert!(e123(&findings).is_empty(), "{module}: {findings:?}");
    }
}

#[test]
fn a_1988_string_type_definition_in_any_other_module_is_an_error() {
    let body = "UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING";
    for text in [
        // no module identifier
        plain(body),
        // a module that is not listed
        format!("New-2026 {{ 1 3 6 1 5 5 7 0 999 }} DEFINITIONS ::= BEGIN {body} END"),
        // a listed name with another identifier
        format!("PKIX1Explicit88 {{ 1 3 6 1 5 5 7 0 999 }} DEFINITIONS ::= BEGIN {body} END"),
        // a listed identifier under another name
        format!("Other {{ 1 3 6 1 5 5 7 0 18 }} DEFINITIONS ::= BEGIN {body} END"),
    ] {
        let findings = check(&text);
        let e123 = e123(&findings);
        assert_eq!(e123.len(), 1, "{text}: {findings:?}");
        assert_eq!(e123[0].severity, Severity::Error, "{text}");
        assert!(
            e123[0]
                .help
                .iter()
                .any(|h| h.contains("built-in UTF8String")),
            "{text}: {:?}",
            e123[0].help
        );
    }
}

// X.680 12.2.2: the name is a reserved word only in editions later than the module's
#[test]
fn the_reserved_word_warning_is_dropped_only_for_a_listed_definition() {
    let body = "UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING \
                BMPString ::= [UNIVERSAL 30] IMPLICIT INTEGER";
    let e011 = |text: &str| {
        let parse = parse_source_with(text, ParseOptions::corpus());
        let kept = glint_sema::without_legacy_name_errors(parse.errors(), &parse.syntax());
        kept.iter()
            .filter(|e| e.code == "E011")
            .map(|e| text[usize::from(e.range.start())..usize::from(e.range.end())].to_owned())
            .collect::<Vec<_>>()
    };
    // the BMPString definition is not the 1988 form, so its name is still reported
    assert_eq!(e011(&pkix1_explicit_88(body)), ["BMPString"]);
    assert_eq!(e011(&plain(body)), ["UTF8String", "BMPString"]);
}

#[test]
fn another_universal_tag_in_a_listed_module_is_still_an_error() {
    for body in [
        "UTF8String ::= [UNIVERSAL 4] IMPLICIT OCTET STRING",
        "UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING (SIZE (1..10))",
        "BMPString ::= [UNIVERSAL 30] IMPLICIT INTEGER",
        "T ::= [UNIVERSAL 12] IMPLICIT OCTET STRING",
        "S ::= SEQUENCE { u [UNIVERSAL 12] IMPLICIT OCTET STRING }",
    ] {
        let findings = check(&pkix1_explicit_88(body));
        let e123 = e123(&findings);
        assert_eq!(e123.len(), 1, "{body}: {findings:?}");
        assert_eq!(e123[0].severity, Severity::Error, "{body}");
    }
}

// X.680 25.16
#[test]
fn version_number_below_two_is_an_error() {
    let text = plain("T ::= SEQUENCE { a INTEGER, ..., [[ 1: b INTEGER OPTIONAL ]] }");
    assert_eq!(tag_codes(&text), ["E125"]);
}

#[test]
fn version_numbers_must_increase() {
    let text = plain(
        "T ::= SEQUENCE { a INTEGER, ..., [[ 3: b BOOLEAN OPTIONAL ]], [[ 2: c NULL OPTIONAL ]] }",
    );
    assert_eq!(tag_codes(&text), ["E125"]);
}

#[test]
fn version_numbers_cannot_mix_with_ungrouped_additions() {
    let text =
        plain("T ::= SEQUENCE { a INTEGER, ..., [[ 2: b BOOLEAN OPTIONAL ]], c NULL OPTIONAL }");
    assert!(has(&check(&text), "E125"));
}

#[test]
fn increasing_version_numbers_are_legal() {
    let text = plain(
        "T ::= SEQUENCE { a INTEGER, ..., [[ 2: b BOOLEAN OPTIONAL ]], [[ 3: c NULL OPTIONAL ]] }",
    );
    assert!(tag_codes(&text).is_empty());
}

// X.680 25.5
#[test]
fn components_of_must_name_a_sequence() {
    let text = plain("U ::= SET { a INTEGER } T ::= SEQUENCE { COMPONENTS OF U, b BOOLEAN }");
    assert_eq!(tag_codes(&text), ["E126"]);
}

#[test]
fn components_of_a_sequence_is_legal_and_adds_its_components() {
    let text = plain("U ::= SEQUENCE { a INTEGER } T ::= SEQUENCE { COMPONENTS OF U, b BOOLEAN }");
    assert!(tag_codes(&text).is_empty());
}

// X.680 27.2
#[test]
fn components_of_in_a_set_must_name_a_set() {
    let text = plain("U ::= SEQUENCE { a INTEGER } T ::= SET { COMPONENTS OF U, b BOOLEAN }");
    assert_eq!(tag_codes(&text), ["E126"]);
}

// X.680 25.14
#[test]
fn duplicate_component_names_are_an_error() {
    let text = plain("T ::= SEQUENCE { a INTEGER, a BOOLEAN }");
    assert_eq!(tag_codes(&text), ["E113"]);
}

#[test]
fn duplicate_alternative_names_are_an_error() {
    let text = plain("T ::= CHOICE { a INTEGER, a BOOLEAN }");
    assert_eq!(tag_codes(&text), ["E113"]);
}

#[test]
fn components_of_that_repeats_a_name_is_an_error() {
    let text = plain("U ::= SEQUENCE { a INTEGER } T ::= SEQUENCE { a BOOLEAN, COMPONENTS OF U }");
    assert_eq!(tag_codes(&text), ["E113"]);
}

#[test]
fn components_of_is_expanded_before_tag_checks() {
    let text = plain(
        "U ::= SEQUENCE { a INTEGER OPTIONAL } T ::= SEQUENCE { COMPONENTS OF U, b INTEGER }",
    );
    assert_eq!(tag_codes(&text), ["E121"]);
}

// X.680 13.1 Note 4
#[test]
fn imported_types_keep_their_own_tag_environment() {
    let base =
        "Base DEFINITIONS AUTOMATIC TAGS ::= BEGIN Inner ::= CHOICE { x INTEGER, y BOOLEAN } END";
    let corpus = corpus(&[Rec {
        name: "Base",
        oid: None,
        rfc: 1,
        text: base,
    }]);
    let text = "M DEFINITIONS ::= BEGIN IMPORTS Inner FROM Base;
        T ::= SEQUENCE { a Inner OPTIONAL, b [0] INTEGER }
        END";
    let findings = check_with(text, &corpus, &glint_sema::Options::default());
    assert_eq!(codes(&findings), ["E121"]);
}
