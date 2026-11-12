mod common;
use common::*;
use glint_extract::{BlockKind, Format};

const APPENDIX_A: &str = include_str!("fixtures/text/rfc5280-appendix-a.txt");
const EXPLICIT: &str =
    include_str!("../../glint-syntax/tests/fixtures/rfc5280/PKIX1Explicit88.asn");
const IMPLICIT: &str =
    include_str!("../../glint-syntax/tests/fixtures/rfc5280/PKIX1Implicit88.asn");

#[test]
fn rfc5280_appendix_a_yields_exactly_the_two_modules() {
    let extraction = run(APPENDIX_A, Format::Text);
    let blocks = modules(&extraction);
    assert_eq!(
        module_names(&blocks),
        ["PKIX1Explicit88", "PKIX1Implicit88"]
    );
    assert!(blocks.iter().all(|b| b.kind == BlockKind::Module));
    assert!(extraction.blocks.len() == 2, "unexpected extra blocks");
    assert!(extraction.notes.is_empty(), "{:?}", extraction.notes);
}

#[test]
fn rfc5280_modules_match_the_hand_copied_fixtures_token_for_token() {
    let extraction = run(APPENDIX_A, Format::Text);
    let blocks = modules(&extraction);
    let expected = [EXPLICIT, IMPLICIT];
    for (block, expected) in blocks.iter().zip(expected) {
        assert_eq!(tokens(&block.text), tokens(expected));
    }
}

#[test]
fn page_headers_footers_and_form_feeds_are_removed_from_modules() {
    let extraction = run(APPENDIX_A, Format::Text);
    for block in modules(&extraction) {
        let original = &APPENDIX_A[block.origin.span.clone()];
        assert!(
            original.contains("[Page "),
            "fixture has no page break in the module"
        );
        assert!(
            original.contains('\u{0C}'),
            "fixture has no form feed in the module"
        );
        assert!(!block.text.contains("[Page "), "footer left in the block");
        assert!(
            !block.text.contains('\u{0C}'),
            "form feed left in the block"
        );
        assert!(
            !block.text.lines().any(|l| l.starts_with("RFC 5280")),
            "running header left in the block"
        );
    }
}

#[test]
fn origins_name_the_appendix_and_its_title() {
    let extraction = run(APPENDIX_A, Format::Text);
    let blocks = modules(&extraction);
    assert_eq!(
        blocks.iter().map(|b| section_label(b)).collect::<Vec<_>>(),
        [
            Some("Appendix A.1".to_owned()),
            Some("Appendix A.2".to_owned())
        ]
    );
    assert_eq!(
        blocks.first().and_then(|b| b.origin.title.as_deref()),
        Some("Explicitly Tagged Module, 1988 Syntax")
    );
}

#[test]
fn a_block_start_maps_to_the_module_name_in_the_original() {
    let extraction = run(APPENDIX_A, Format::Text);
    let block = extraction.blocks.first().unwrap();
    let range = block.original_range(0..15).unwrap();
    assert_eq!(&APPENDIX_A[range], "PKIX1Explicit88");
}

#[test]
fn the_whole_rfc_5280_text_yields_the_same_two_modules_when_cached() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cache/rfc5280.txt");
    let Ok(source) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: {} is not present", path.display());
        return;
    };
    let extraction = run(&source, Format::Text);
    let blocks = modules(&extraction);
    assert_eq!(
        module_names(&blocks),
        ["PKIX1Explicit88", "PKIX1Implicit88"]
    );
    for (block, expected) in blocks.iter().zip([EXPLICIT, IMPLICIT]) {
        assert_eq!(tokens(&block.text), tokens(expected));
    }
}

const DRAFT: &str = "\
Internet-Draft                  Example                    November 2026


1.  Introduction

   Some prose that mentions DEFINITIONS in passing.

2.  Module

   <CODE BEGINS>
   Marked DEFINITIONS ::= BEGIN
     MarkedType ::= INTEGER
   END
   <CODE ENDS>

3.  Appendix-like

Unmarked { 1 2 3 }

DEFINITIONS IMPLICIT TAGS ::=

BEGIN

Part1 ::= SEQUENCE {
    a INTEGER,

Smith                    Expires 1 May 2027                    [Page 3]
\u{0C}
Internet-Draft                  Example                    November 2026

    b BOOLEAN }

END

4.  Snippets

   Here is a type:

      Cert ::= SEQUENCE {
          version  INTEGER,

          serial   INTEGER }

   and a value:

      id-foo OBJECT IDENTIFIER ::= { 1 2 3 }

   End of prose.
";

#[test]
fn drafts_yield_marked_unmarked_and_fragment_blocks_in_order() {
    let extraction = run(DRAFT, Format::Text);
    let kinds: Vec<_> = extraction.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        [
            BlockKind::Module,
            BlockKind::Module,
            BlockKind::Fragment,
            BlockKind::Fragment
        ]
    );
    assert_eq!(module_names(&modules(&extraction)), ["Marked", "Unmarked"]);
    assert!(extraction.notes.is_empty(), "{:?}", extraction.notes);
}

#[test]
fn marked_blocks_are_dedented_and_lose_their_markers() {
    let extraction = run(DRAFT, Format::Text);
    let block = extraction.blocks.first().unwrap();
    assert!(block.text.starts_with("Marked DEFINITIONS"));
    assert!(!block.text.contains("<CODE"));
    assert!(block.text.contains("\n  MarkedType"));
}

#[test]
fn a_page_break_inside_a_module_leaves_no_trace() {
    let extraction = run(DRAFT, Format::Text);
    let block = extraction.blocks.get(1).unwrap();
    assert!(!block.text.contains("Expires"));
    assert!(!block.text.contains("Internet-Draft"));
    assert_eq!(
        tokens(&block.text),
        tokens(
            "Unmarked { 1 2 3 } DEFINITIONS IMPLICIT TAGS ::= BEGIN Part1 ::= SEQUENCE { a INTEGER, b BOOLEAN } END"
        )
    );
    assert_eq!(section_label(block).as_deref(), Some("Section 3"));
}

#[test]
fn a_snippet_run_is_one_fragment_with_its_section() {
    let extraction = run(DRAFT, Format::Text);
    let fragment = extraction.blocks.get(2).unwrap();
    assert_eq!(section_label(fragment).as_deref(), Some("Section 4"));
    assert_eq!(fragment.origin.title.as_deref(), Some("Snippets"));
    assert!(fragment.text.starts_with("Cert ::= SEQUENCE {"));
    assert!(fragment.text.contains("serial   INTEGER }"));
    assert!(!fragment.text.contains("id-foo"));
}

#[test]
fn crlf_documents_round_trip() {
    let crlf = DRAFT.replace('\n', "\r\n");
    let extraction = run(&crlf, Format::Text);
    assert_eq!(extraction.blocks.len(), 4);
    assert!(extraction.blocks.iter().all(|b| !b.text.contains('\r')));
}

#[test]
fn lone_cr_line_breaks_are_kept_for_the_asn1_lexer() {
    let source = "M DEFINITIONS ::= BEGIN\rT ::= INTEGER\rEND\r";
    let extraction = run(source, Format::Text);
    assert_eq!(modules(&extraction).len(), 1);
}

#[test]
fn a_module_without_end_stops_at_the_next_heading() {
    let source = "\
1.  First

M DEFINITIONS ::= BEGIN
T ::= INTEGER

2.  Second

   Plain prose.
";
    let extraction = run(source, Format::Text);
    let block = extraction.blocks.first().unwrap();
    assert!(block.text.contains("T ::= INTEGER"));
    assert!(!block.text.contains("Plain prose"));
}

#[test]
fn consecutive_modules_without_blank_separation_are_split() {
    let source = "\
A { 1 2 } DEFINITIONS ::= BEGIN
END
B DEFINITIONS ::= BEGIN
END
";
    let extraction = run(source, Format::Text);
    assert_eq!(module_names(&modules(&extraction)), ["A", "B"]);
}

#[test]
fn prose_mentioning_the_keyword_is_not_a_module() {
    let source = "   The DEFINITIONS keyword starts a module.\n";
    let extraction = run(source, Format::Text);
    assert!(extraction.blocks.is_empty());
}

#[test]
fn table_of_contents_lines_are_not_headings() {
    let source = "\
   1.  Introduction .......................................... 3

   Foo ::= INTEGER
";
    let extraction = run(source, Format::Text);
    let block = extraction.blocks.first().unwrap();
    assert!(block.origin.section.is_none());
}

#[test]
fn a_brace_on_its_own_line_continues_a_fragment() {
    let text = "Title\n\n1. Messages\n\n   Arguments:\n\n   Submit ::= SEQUENCE\n   {\n     a INTEGER,\n     b BOOLEAN\n   }\n\n   Next paragraph.\n";
    let extraction = run(text, Format::Text);
    let fragment = extraction.blocks.first().unwrap();
    assert_eq!(fragment.kind, BlockKind::Fragment);
    assert!(fragment.text.contains("b BOOLEAN"), "{:?}", fragment.text);
    assert!(
        fragment.text.trim_end().ends_with('}'),
        "{:?}",
        fragment.text
    );
}

#[test]
fn a_constraint_after_the_closing_brace_continues_a_fragment() {
    // RFC 8226, Section 8: the constraint sits at the snippet's own indent,
    // after comments that follow the closing brace.
    let text = "\
8.  JWT Claim Constraints Syntax

   The JWT Claim Constraints certificate extension has the following
   syntax:

     JWTClaimConstraints ::= SEQUENCE {
       mustInclude [0] JWTClaimNames OPTIONAL,
       permittedValues [1] JWTClaimPermittedValuesList OPTIONAL }
         -- If the claim name is present, the claim MUST contain one of
         -- the listed values.
     ( WITH COMPONENTS { ..., mustInclude PRESENT } |
       WITH COMPONENTS { ..., permittedValues PRESENT } )

     JWTClaimNames ::= SEQUENCE SIZE (1..MAX) OF JWTClaimName

   (Prose in parentheses after the snippet is not part of it.)
";
    let extraction = run(text, Format::Text);
    assert_eq!(extraction.blocks.len(), 1, "{:?}", extraction.blocks);
    let fragment = extraction.blocks.first().unwrap();
    assert_eq!(fragment.kind, BlockKind::Fragment);
    assert!(
        fragment
            .text
            .contains("WITH COMPONENTS { ..., permittedValues PRESENT } )"),
        "{:?}",
        fragment.text
    );
    assert!(
        fragment.text.contains("JWTClaimNames ::="),
        "{:?}",
        fragment.text
    );
    assert!(!fragment.text.contains("Prose"), "{:?}", fragment.text);
}

#[test]
fn a_wrapped_trailing_comment_in_parentheses_ends_a_fragment() {
    // RFC 2560, Section 4.2.1: the end-of-line comment wraps onto a line
    // that starts with `(`, which is prose, not a constraint.
    let text = "\
4.2.1  ASN.1 Specification of the OCSP Response

   KeyHash ::= OCTET STRING -- SHA-1 hash of responder's public key
   (excluding the tag and length fields)

   SingleResponse ::= SEQUENCE {
      certID                       CertID }
";
    let extraction = run(text, Format::Text);
    assert!(
        extraction
            .blocks
            .iter()
            .all(|b| !b.text.contains("excluding")),
        "{:?}",
        extraction.blocks
    );
}

#[test]
fn a_closing_brace_before_definitions_keeps_the_header_lines_above() {
    // RFC 4120 Section 5: the name and the opening brace on one line, the arcs below, `} DEFINITIONS` last.
    let source = "\
5.  Specifications

   KerberosV5Spec2 {
           iso(1) identified-organization(3) dod(6) internet(1)
           security(5) kerberosV5(2) modules(4) krb5spec2(2)
   } DEFINITIONS EXPLICIT TAGS ::= BEGIN

   Int32 ::= INTEGER

   END
";
    let extraction = run(source, Format::Text);
    let found = modules(&extraction);
    assert_eq!(module_names(&found), ["KerberosV5Spec2"]);
    assert!(
        found[0].text.starts_with("KerberosV5Spec2 {"),
        "{}",
        found[0].text
    );
}

#[test]
fn a_name_and_an_identifier_in_paragraphs_of_their_own_start_the_module() {
    // RFC 5751 Appendix A and RFC 5055 Section 8.
    let source = "\
   The module follows.

   SecureMimeMessageV3dot1

     { iso(1) member-body(2) us(840) rsadsi(113549)
            pkcs(1) pkcs-9(9) smime(16) modules(0) msg-v3dot1(21) }

   DEFINITIONS IMPLICIT TAGS ::=

   BEGIN

   Version ::= INTEGER

   END
";
    let extraction = run(source, Format::Text);
    let found = modules(&extraction);
    assert_eq!(module_names(&found), ["SecureMimeMessageV3dot1"]);
    assert!(found[0].text.starts_with("SecureMimeMessageV3dot1"));
    assert!(!found[0].text.contains("module follows"));
}

#[test]
fn a_header_after_a_heading_does_not_reach_into_the_prose_above() {
    // RFC 1142 Section 11.2.17: no space before the brace, and a heading right above.
    let source = "\
operation shall fail with the generation of the con
straintViolation specific error.;
11.2.17 ASN1 Modules
ISO10589-ISIS{tbd1}
DEFINITIONS ::= BEGIN
T ::= INTEGER
END
";
    let extraction = run(source, Format::Text);
    let found = modules(&extraction);
    assert_eq!(module_names(&found), ["ISO10589-ISIS"]);
    assert!(found[0].text.starts_with("ISO10589-ISIS{tbd1}"));
}

#[test]
fn a_running_header_without_a_title_is_page_furniture() {
    // RFC 5544 puts only the RFC number and the date in its running header.
    let source = "\
   M { 1 2 } DEFINITIONS ::= BEGIN

   A ::= INTEGER

Housley                     Standards Track                    [Page 8]
\u{0C}
RFC 5544                                                   February 2010


   B ::= BOOLEAN

   END
";
    let extraction = run(source, Format::Text);
    let found = modules(&extraction);
    assert_eq!(module_names(&found), ["M"]);
    assert!(!found[0].text.contains("RFC 5544"), "{}", found[0].text);
    assert!(found[0].text.contains("B ::= BOOLEAN"));
}

#[test]
fn an_appendix_heading_without_the_word_appendix_names_the_appendix() {
    // RFC 4120 titles its module appendix `A.  ASN.1 module`.
    let source = "\
11.  Acknowledgements

   Thanks.

A.  ASN.1 module

KerberosV5Spec2 { 1 3 6 1 5 2 4 2 } DEFINITIONS EXPLICIT TAGS ::= BEGIN
Int32 ::= INTEGER
END
";
    let extraction = run(source, Format::Text);
    let found = modules(&extraction);
    assert_eq!(module_names(&found), ["KerberosV5Spec2"]);
    assert_eq!(section_label(found[0]).as_deref(), Some("Appendix A"));
}

#[test]
fn a_grammar_is_not_extracted_but_asn1_next_to_it_is() {
    let source = "\
1.  Syntax

   address         ::= \"(\" addr_name SPACE addr_adl SPACE addr_mailbox
                       SPACE addr_host \")\"

   addr_adl        ::= nstring

2.  Signatures

   Dss-Sig-Value ::= SEQUENCE {
       r INTEGER,
       s INTEGER }
";
    let extraction = run(source, Format::Text);
    assert_eq!(extraction.blocks.len(), 1);
    assert!(extraction.blocks[0].text.starts_with("Dss-Sig-Value"));
    assert!(extraction.has_inline_asn1());
}

#[test]
fn the_snippets_of_a_mib_are_not_inline_asn1() {
    let snippet = "\
3.  The Interfaces Table

   IfEntry ::= SEQUENCE {
       ifIndex INTEGER,
       ifDescr OCTET STRING }

";
    let mib = "\
4.  Definitions

   IF-MIB DEFINITIONS ::= BEGIN
   IMPORTS OBJECT-TYPE, Counter32 FROM SNMPv2-SMI;

   ifInOctets OBJECT-TYPE
       SYNTAX      Counter32
       MAX-ACCESS  read-only
       STATUS      current
       DESCRIPTION \"The octets received.\"
       ::= { ifEntry 10 }

   END
";
    assert!(run(snippet, Format::Text).has_inline_asn1());
    let extraction = run(&format!("{snippet}{mib}"), Format::Text);
    assert!(!extraction.has_inline_asn1());
    assert_eq!(
        modules(&extraction).len(),
        1,
        "the MIB module itself is kept"
    );
}

#[test]
fn the_snippets_of_an_asn1_module_are_inline_asn1() {
    let source = "\
3.  Types

   T ::= INTEGER

4.  Module

   M { 1 2 3 } DEFINITIONS ::= BEGIN
   T ::= INTEGER
   END
";
    assert!(run(source, Format::Text).has_inline_asn1());
}
