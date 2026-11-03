mod common;
use common::*;
use glint_extract::{BlockKind, Format};

const DRAFT: &str = "\
---
title: Example Module
docname: draft-example-00
category: std
---

--- abstract

An abstract with `code`.

--- middle

# Introduction

Text.

# Syntax

## The Type

~~~ asn.1
Foo ::= SEQUENCE {
    a INTEGER,
    b BOOLEAN OPTIONAL }
~~~

~~~ json
{ \"not\": \"asn.1\" }
~~~

--- back

# ASN.1 Module

```asn1
Example-Module DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Foo ::= INTEGER
END
```

# Acknowledgements {numbered=\"false\"}

Thanks.

## Not numbered either {numbered=\"false\"}

# Second Appendix

    Indented DEFINITIONS ::= BEGIN
      T ::= NULL
    END
";

#[test]
fn fenced_blocks_with_asn1_info_strings_are_extracted() {
    let extraction = run(DRAFT, Format::Markdown);
    assert_eq!(extraction.blocks.len(), 3);
    let kinds: Vec<_> = extraction.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        [BlockKind::Fragment, BlockKind::Module, BlockKind::Module]
    );
    assert_eq!(
        module_names(&modules(&extraction)),
        ["Example-Module", "Indented"]
    );
}

#[test]
fn other_languages_and_front_matter_are_ignored() {
    let extraction = run(DRAFT, Format::Markdown);
    assert!(extraction.blocks.iter().all(|b| !b.text.contains("not")));
    assert!(
        extraction
            .blocks
            .iter()
            .all(|b| !b.text.contains("docname"))
    );
}

#[test]
fn sections_are_numbered_like_xml2rfc_would() {
    let extraction = run(DRAFT, Format::Markdown);
    let labels: Vec<_> = extraction
        .blocks
        .iter()
        .map(|b| (section_label(b), b.origin.title.clone()))
        .collect();
    assert_eq!(
        labels,
        [
            (Some("Section 2.1".to_owned()), Some("The Type".to_owned())),
            (
                Some("Appendix A".to_owned()),
                Some("ASN.1 Module".to_owned())
            ),
            (
                Some("Appendix B".to_owned()),
                Some("Second Appendix".to_owned())
            ),
        ]
    );
}

#[test]
fn info_string_variants_and_unmarked_code_get_the_right_notes() {
    let extraction = run(DRAFT, Format::Markdown);
    assert_eq!(codes(&extraction), ["W202"]);
    let source =
        "```ASN.1\nA ::= INTEGER\n```\n\n```asn1\nB ::= INTEGER\n```\n\n```\nC ::= INTEGER\n```\n";
    let extraction = run(source, Format::Markdown);
    assert_eq!(extraction.blocks.len(), 3);
    assert_eq!(codes(&extraction), ["W203", "W202"]);
}

#[test]
fn an_indented_block_that_is_not_asn1_is_ignored_silently() {
    let source = "Text.\n\n    int main(void) { return 0; }\n";
    let extraction = run(source, Format::Markdown);
    assert!(extraction.blocks.is_empty());
    assert!(extraction.notes.is_empty());
}

#[test]
fn blocks_inside_lists_and_quotes_map_back_past_the_container_prefix() {
    let source = "\
- item

  ```asn.1
  Foo ::= SEQUENCE {
      a INTEGER }
  ```

> ```asn.1
> Bar ::= INTEGER
> ```
";
    let extraction = run(source, Format::Markdown);
    assert_eq!(extraction.blocks.len(), 2);
    let first = extraction.blocks.first().unwrap();
    let at = first.text.find("Foo").unwrap();
    let original = first.original_range(at..at + 3).unwrap();
    assert_eq!(&source[original], "Foo");
    let second = extraction.blocks.get(1).unwrap();
    let at = second.text.find("Bar").unwrap();
    let original = second.original_range(at..at + 3).unwrap();
    assert_eq!(&source[original], "Bar");
}

#[test]
fn mmark_back_matter_marker_starts_appendices() {
    let source = "# Body\n\n{backmatter}\n\n# Module\n\n~~~ asn.1\nA ::= INTEGER\n~~~\n";
    let extraction = run(source, Format::Markdown);
    let block = extraction.blocks.first().unwrap();
    assert_eq!(section_label(block).as_deref(), Some("Appendix A"));
}

#[test]
fn code_markers_inside_fences_are_removed() {
    let source = "```asn.1\n<CODE BEGINS>\nA ::= INTEGER\n<CODE ENDS>\n```\n";
    let extraction = run(source, Format::Markdown);
    assert_eq!(extraction.blocks.first().unwrap().text, "A ::= INTEGER\n");
}

#[test]
fn crlf_markdown_round_trips() {
    let crlf = DRAFT.replace('\n', "\r\n");
    let extraction = run(&crlf, Format::Markdown);
    assert_eq!(extraction.blocks.len(), 3);
}
