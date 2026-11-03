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
