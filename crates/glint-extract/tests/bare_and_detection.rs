mod common;
use std::path::Path;

use common::*;
use glint_extract::{BlockKind, Format, detect_format};

const MODULE: &str = include_str!("../../glint-syntax/tests/fixtures/rfc5280/PKIX1Implicit88.asn");
const APPENDIX_A: &str = include_str!("fixtures/text/rfc5280-appendix-a.txt");
const RFC8692: &str = include_str!("fixtures/xml/rfc8692.xml");

#[test]
fn a_bare_file_is_one_block_with_an_identity_map() {
    let extraction = run(MODULE, Format::Bare);
    let block = extraction.blocks.first().unwrap();
    assert_eq!(extraction.blocks.len(), 1);
    assert_eq!(block.text, MODULE);
    assert_eq!(block.kind, BlockKind::Module);
    assert_eq!(block.original_range(5..9), Some(5..9));
}

#[test]
fn a_bare_file_that_is_not_asn1_is_reported() {
    let extraction = run("just words\n", Format::Bare);
    assert!(extraction.blocks.is_empty());
    assert_eq!(codes(&extraction), ["W201"]);
}

#[test]
fn extensions_decide_when_present() {
    assert_eq!(detect_format(Some(Path::new("a.xml")), ""), Format::Xml);
    assert_eq!(detect_format(Some(Path::new("a.MD")), ""), Format::Markdown);
    assert_eq!(detect_format(Some(Path::new("a.txt")), ""), Format::Text);
    assert_eq!(detect_format(Some(Path::new("a.asn")), ""), Format::Bare);
}

#[test]
fn content_decides_without_an_extension() {
    assert_eq!(detect_format(None, RFC8692), Format::Xml);
    assert_eq!(detect_format(None, APPENDIX_A), Format::Text);
    assert_eq!(detect_format(None, MODULE), Format::Bare);
    assert_eq!(detect_format(None, "# Title\n\ntext\n"), Format::Markdown);
    assert_eq!(
        detect_format(None, "---\ntitle: x\n---\n"),
        Format::Markdown
    );
    assert_eq!(
        detect_format(None, "Internet-Draft   X   May 2026\n\n   Hello\n"),
        Format::Text
    );
}
