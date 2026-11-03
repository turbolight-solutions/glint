#![allow(clippy::arithmetic_side_effects)]
mod common;
use common::*;
use glint_extract::{Extraction, Format};

fn summary(source: &str, extraction: &Extraction) -> String {
    let mut out = String::new();
    for block in &extraction.blocks {
        let span = &block.origin.span;
        let first_line = block.text.lines().next().unwrap_or_default();
        out.push_str(&format!(
            "{:?} | {} | {:?} | bytes {}..{} | text {} bytes | first line {:?} | original starts {:?}\n",
            block.kind,
            section_label(block).unwrap_or_else(|| "-".to_owned()),
            block.origin.title.as_deref().unwrap_or("-"),
            span.start,
            span.end,
            block.text.len(),
            first_line,
            source.get(span.start..span.start + 20).unwrap_or_default(),
        ));
    }
    for note in &extraction.notes {
        out.push_str(&format!("{} {:?}\n", note.code, note.span));
    }
    out
}

#[test]
fn rfc8692_xml() {
    let source = include_str!("fixtures/xml/rfc8692.xml");
    insta::assert_snapshot!(summary(source, &run(source, Format::Xml)));
}

#[test]
fn rfc5280_appendix_a_text() {
    let source = include_str!("fixtures/text/rfc5280-appendix-a.txt");
    insta::assert_snapshot!(summary(source, &run(source, Format::Text)));
}
