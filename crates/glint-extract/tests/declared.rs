//! Extraction from an RFC's or draft's published text with the ASN.1 blocks its RFCXML marks matches extraction from
//! the XML.

mod common;
use common::*;
use glint_extract::{Format, declared_blocks, display_lines, extract, extract_declared};

const DOCUMENTS: [(&str, &str, &str); 5] = [
    (
        "rfc8692",
        include_str!("fixtures/xml/rfc8692.xml"),
        include_str!("fixtures/xml/rfc8692.txt"),
    ),
    // Two `<sourcecode>` elements without a `type`: taken by their content, with W202.
    (
        "rfc9336",
        include_str!("fixtures/xml/rfc9336.xml"),
        include_str!("fixtures/xml/rfc9336.txt"),
    ),
    // `type="ASN.1"`: W203.
    (
        "rfc9598",
        include_str!("fixtures/xml/rfc9598.xml"),
        include_str!("fixtures/xml/rfc9598.txt"),
    ),
    // A `<sourcecode>` inside a `<blockquote>`: the text draws it behind a `|` bar (section 4).
    (
        "rfc8951",
        include_str!("fixtures/xml/rfc8951.xml"),
        include_str!("fixtures/xml/rfc8951.txt"),
    ),
    // A paginated draft whose module, an untyped `<sourcecode>` (W202), runs across four page breaks; its XML has CRLF
    // line breaks.
    (
        "draft-ietf-lamps-frodokem-certificates-00",
        include_str!("fixtures/xml/draft-ietf-lamps-frodokem-certificates-00.xml"),
        include_str!("fixtures/xml/draft-ietf-lamps-frodokem-certificates-00.txt"),
    ),
];

#[test]
fn the_text_with_declared_blocks_yields_what_the_xml_yields() {
    for (name, xml, text) in DOCUMENTS {
        let from_xml = run(xml, Format::Xml);
        let blocks = declared_blocks(xml, text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let from_text = extract_declared(text, &blocks);
        assert_round_trip(text, &from_text);
        assert!(from_text.blocks_declared && !from_xml.blocks_declared);
        assert_eq!(from_text.format, Format::Text);
        assert_eq!(
            from_xml.blocks.len(),
            from_text.blocks.len(),
            "{name}: blocks"
        );
        for (x, t) in from_xml.blocks.iter().zip(&from_text.blocks) {
            assert_eq!(x.kind, t.kind, "{name}");
            assert_eq!(x.origin.section, t.origin.section, "{name}");
            assert_eq!(x.origin.title, t.origin.title, "{name}");
            // The same content, as the text shows it: indentation, blank lines around it and spaces at line ends
            // aside.
            assert_eq!(display_lines(&x.text), display_lines(&t.text), "{name}");
            // The block's span is its lines in the text, and holds the block.
            let span = t.origin.span.clone();
            assert!(span.start == 0 || text.get(..span.start).unwrap().ends_with('\n'));
            assert!(text.get(span.clone()).unwrap().ends_with('\n'), "{name}");
            let held = t.original_range(0..t.text.len()).unwrap();
            assert!(span.start <= held.start && held.end <= span.end, "{name}");
        }
        let notes = |e: &glint_extract::Extraction| -> Vec<(&str, String)> {
            e.notes
                .iter()
                .map(|n| (n.code, n.message.clone()))
                .collect()
        };
        assert_eq!(notes(&from_xml), notes(&from_text), "{name}: notes");
        // Every note of the text is about the XML's marking, which a check of the text keeps apart.
        assert!(
            from_text
                .notes
                .iter()
                .all(|n| ["W201", "W202", "W203", "I200", "I201"].contains(&n.code)),
            "{name}"
        );
    }
}

#[test]
fn a_draft_module_across_page_breaks_leaves_them_out() {
    let (name, xml, text) = DOCUMENTS[4];
    let blocks = declared_blocks(xml, text).unwrap();
    let [block] = blocks.as_slice() else {
        panic!("{name}: one block")
    };
    assert_eq!(block.breaks.len(), 4, "{name}");
    for page_break in &block.breaks {
        let lines = text.get(page_break.clone()).unwrap();
        assert!(block.range.start < page_break.start && page_break.end < block.range.end);
        assert!(lines.contains("[Page ") && lines.contains("\u{c}\nInternet-Draft "));
        assert!(lines.ends_with("\n\n") && text.get(..page_break.start).unwrap().ends_with('\n'));
    }
    let from_text = extract_declared(text, &blocks);
    let module = from_text.blocks.first().unwrap();
    assert!(!module.text.contains("[Page ") && !module.text.contains("Internet-Draft"));
    assert!(!module.text.contains('\u{c}'));
    // Every character maps outside the breaks (and to itself: `the_text_with_declared_blocks_yields_what_the_xml_yields`),
    // and the lines after each break are the module's.
    let mapped: Vec<usize> = module
        .text
        .char_indices()
        .map(|(at, c)| module.original_range(at..at + c.len_utf8()).unwrap().start)
        .collect();
    assert!(
        mapped
            .iter()
            .all(|at| !block.breaks.iter().any(|b| b.contains(at)))
    );
    for page_break in &block.breaks {
        assert!(mapped.contains(&(page_break.end + block.indent)), "{name}");
    }
}

#[test]
fn the_fixtures_exercise_untyped_and_misspelt_elements() {
    let codes = |xml: &str| -> Vec<&str> {
        extract(xml, Format::Xml)
            .notes
            .iter()
            .map(|n| n.code)
            .collect()
    };
    assert!(codes(DOCUMENTS[1].1).contains(&"W202"));
    assert!(codes(DOCUMENTS[2].1).contains(&"W203"));
}

#[test]
fn a_header_only_element_joins_the_next_in_the_text_too() {
    // RFC 9286 Appendix A puts a module's name and OID in one <sourcecode> and its DEFINITIONS in the next.
    let xml = "<rfc><back><section><name>ASN.1 Module</name>\n\
               <sourcecode type=\"asn.1\">\nM { 1 2 3 }\n</sourcecode>\n\
               <t>Prose.</t>\n\
               <sourcecode type=\"asn.1\">\nDEFINITIONS ::= BEGIN\nA ::= INTEGER\nEND\n</sourcecode>\n\
               </section></back></rfc>\n";
    let text = "Appendix A.  ASN.1 Module\n\n   M { 1 2 3 }\n\n   Prose.\n\n   DEFINITIONS ::= BEGIN\n   A ::= INTEGER\n   END\n";
    let from_xml = run(xml, Format::Xml);
    let blocks = declared_blocks(xml, text).unwrap();
    assert_eq!(blocks.len(), 2);
    let from_text = extract_declared(text, &blocks);
    assert_round_trip(text, &from_text);
    assert_eq!(module_names(&modules(&from_xml)), ["M"]);
    assert_eq!(module_names(&modules(&from_text)), ["M"]);
    let module = from_text.blocks.first().unwrap();
    assert_eq!(
        module.origin.span,
        text.find("   M {").unwrap()..text.len(),
        "the joined block spans both"
    );
}

#[test]
fn a_text_that_lacks_a_block_is_refused() {
    let (_, xml, text) = DOCUMENTS[0];
    let module_line = "   PKIXAlgsForSHAKE-2019 {";
    assert!(text.contains(module_line));
    // A line of the module re-wrapped, as some xml2rfc versions do with artwork.
    let damaged = text.replacen(module_line, "   PKIXAlgsForSHAKE-2019\n   {", 1);
    let error = declared_blocks(xml, &damaged).unwrap_err();
    assert!(error.contains("is not found in the text"), "{error}");
    // XML that is not well-formed cannot be followed.
    let broken = format!("{xml}<sourcecode type=\"asn.1\">A ::= INTEGER");
    assert!(declared_blocks(&broken, text).is_err());
}

#[test]
fn a_block_outside_the_text_is_left_out() {
    let (_, xml, text) = DOCUMENTS[0];
    let mut blocks = declared_blocks(xml, text).unwrap();
    let all = extract_declared(text, &blocks).blocks.len();
    if let Some(last) = blocks.last_mut() {
        last.range = text.len()..text.len() + 10;
    }
    assert_eq!(extract_declared(text, &blocks).blocks.len(), all - 1);
}
