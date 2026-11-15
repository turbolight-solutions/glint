//! Checking an RFC's or draft's published text with the ASN.1 blocks its RFCXML marks gives the diagnostics that
//! checking the XML gives, at the same characters (a draft's page breaks aside), except the findings about the XML's
//! marking, which the text's check keeps apart.

#![allow(clippy::unwrap_used, clippy::panic)]

use glint_check::{CheckOptions, check_document, check_extraction};
use glint_corpus::EmptyCorpus;
use glint_extract::{Format, declared_blocks, extract_declared};

const DOCUMENTS: [(&str, &str, &str); 5] = [
    (
        "rfc8692",
        include_str!("../../glint-extract/tests/fixtures/xml/rfc8692.xml"),
        include_str!("../../glint-extract/tests/fixtures/xml/rfc8692.txt"),
    ),
    (
        "rfc9336",
        include_str!("../../glint-extract/tests/fixtures/xml/rfc9336.xml"),
        include_str!("../../glint-extract/tests/fixtures/xml/rfc9336.txt"),
    ),
    (
        "rfc9598",
        include_str!("../../glint-extract/tests/fixtures/xml/rfc9598.xml"),
        include_str!("../../glint-extract/tests/fixtures/xml/rfc9598.txt"),
    ),
    // A `<sourcecode>` inside a `<blockquote>`: the text draws it behind a `|` bar (section 4).
    (
        "rfc8951",
        include_str!("../../glint-extract/tests/fixtures/xml/rfc8951.xml"),
        include_str!("../../glint-extract/tests/fixtures/xml/rfc8951.txt"),
    ),
    // A paginated draft whose module runs across page breaks.
    (
        "draft-ietf-lamps-frodokem-certificates-00",
        include_str!(
            "../../glint-extract/tests/fixtures/xml/draft-ietf-lamps-frodokem-certificates-00.xml"
        ),
        include_str!(
            "../../glint-extract/tests/fixtures/xml/draft-ietf-lamps-frodokem-certificates-00.txt"
        ),
    ),
];

/// The findings about the RFCXML marking of the ASN.1 elements.
const MARKUP: [&str; 5] = ["W201", "W202", "W203", "I200", "I201"];

/// The characters of a span, as a reader sees them: entities decoded, whitespace runs (the text's indentation) as one
/// space.
fn shown(text: &str) -> String {
    let decoded = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn diagnostics_of_the_text_with_declared_blocks_are_those_of_the_xml() {
    let corpus = EmptyCorpus::default();
    let options = CheckOptions::default();
    let mut markup_seen = 0;
    for (name, xml, text) in DOCUMENTS {
        let from_xml = check_document(name, xml, Format::Xml, &corpus, &options);
        let blocks = declared_blocks(xml, text).unwrap();
        let from_text = check_extraction(
            name,
            text,
            extract_declared(text, &blocks),
            &corpus,
            &options,
        );
        let key = |d: &glint_diag::Diagnostic| (d.code, d.severity, d.message.clone());
        let (xml_markup, xml_rest): (Vec<_>, Vec<_>) = from_xml
            .diagnostics
            .iter()
            .partition(|d| MARKUP.contains(&d.code));
        assert!(from_xml.markup_findings.is_empty(), "{name}");
        let keys = |list: &[&glint_diag::Diagnostic]| {
            let mut keys: Vec<_> = list.iter().map(|d| key(d)).collect();
            keys.sort_by(|a, b| (a.0, &a.2).cmp(&(b.0, &b.2)));
            keys
        };
        let text_rest: Vec<_> = from_text.diagnostics.iter().collect();
        let text_markup: Vec<_> = from_text.markup_findings.iter().collect();
        assert!(!xml_rest.is_empty(), "{name}");
        assert_eq!(keys(&xml_rest), keys(&text_rest), "{name}");
        // The markup findings are not among the text's diagnostics, but kept apart: the same ones, as many.
        assert!(
            text_rest.iter().all(|d| !MARKUP.contains(&d.code)),
            "{name}"
        );
        assert_eq!(keys(&xml_markup), keys(&text_markup), "{name}");
        assert_eq!(xml_markup.len(), from_text.markup_findings.len(), "{name}");
        markup_seen += xml_markup.len();

        // Both lists are sorted by position, and the blocks are in the same order in both documents.
        for (x, t) in xml_rest.iter().zip(&text_rest) {
            assert_eq!(key(x), key(t), "{name}");
            if x.primary.file != from_xml.document {
                continue;
            }
            let in_xml = xml.get(x.primary.span.clone()).unwrap();
            // The page breaks a span of the text runs across are not the XML's.
            let span = t.primary.span.clone();
            let in_text: String = text
                .get(span.clone())
                .unwrap()
                .char_indices()
                .filter(|(at, _)| {
                    !blocks
                        .iter()
                        .flat_map(|b| &b.breaks)
                        .any(|b| b.contains(&(span.start + at)))
                })
                .map(|(_, c)| c)
                .collect();
            let in_text = in_text.as_str();
            if in_xml.trim_start().starts_with('<') {
                // A finding about a whole element: in the text, about its block.
                assert!(shown(in_xml).contains(&shown(in_text)), "{name} {}", x.code);
            } else {
                assert_eq!(shown(in_xml), shown(in_text), "{name} {}", x.code);
            }
        }
        let modules = |r: &glint_check::Report| -> Vec<(String, Option<String>, Option<String>)> {
            r.modules
                .iter()
                .map(|m| (m.name.clone(), m.oid.clone(), m.section.clone()))
                .collect()
        };
        assert_eq!(modules(&from_xml), modules(&from_text), "{name}");
    }
    assert!(
        markup_seen > 0,
        "the fixtures have untyped and misspelt elements"
    );
}
