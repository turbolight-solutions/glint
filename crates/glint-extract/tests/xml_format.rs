mod common;
use common::*;
use glint_extract::{BlockKind, Format};
use glint_syntax::{ParseOptions, Severity, parse_source_with};

const RFC8692: &str = include_str!("fixtures/xml/rfc8692.xml");

#[test]
fn v3_rfc_yields_the_expected_module_and_snippets() {
    let extraction = run(RFC8692, Format::Xml);
    assert_eq!(
        module_names(&modules(&extraction)),
        ["PKIXAlgsForSHAKE-2019"]
    );
    let kinds: Vec<_> = extraction.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        [
            BlockKind::Fragment,
            BlockKind::Fragment,
            BlockKind::Fragment,
            BlockKind::Module
        ]
    );
    assert!(extraction.notes.is_empty(), "{:?}", extraction.notes);
}

#[test]
fn v3_origins_use_rfc_numbering() {
    let extraction = run(RFC8692, Format::Xml);
    let labels: Vec<_> = extraction.blocks.iter().map(section_label).collect();
    assert_eq!(
        labels,
        [
            Some("Section 3".to_owned()),
            Some("Section 3".to_owned()),
            Some("Section 4.1".to_owned()),
            Some("Appendix A".to_owned()),
        ]
    );
    let module = extraction.blocks.last().unwrap();
    assert_eq!(module.origin.title.as_deref(), Some("ASN.1 Module"));
}

#[test]
fn v3_module_has_entities_decoded_and_parses_without_errors() {
    let extraction = run(RFC8692, Format::Xml);
    let module = extraction.blocks.last().unwrap();
    assert!(module.text.contains("&smimeCaps"));
    assert!(!module.text.contains("&amp;"));
    let parse = parse_source_with(&module.text, ParseOptions::corpus());
    let errors: Vec<_> = parse
        .errors()
        .iter()
        .filter(|e| e.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn v3_module_starts_where_the_sourcecode_content_starts() {
    let extraction = run(RFC8692, Format::Xml);
    let module = extraction.blocks.last().unwrap();
    let name_at = module.text.find("PKIXAlgsForSHAKE-2019").unwrap();
    let original = module.original_range(name_at..name_at + 21).unwrap();
    assert_eq!(&RFC8692[original], "PKIXAlgsForSHAKE-2019");
}

#[test]
fn entities_and_cdata_change_offsets_but_map_back() {
    let source = "\
<rfc><middle><section><name>Mod</name>
<sourcecode type=\"asn.1\">M DEFINITIONS ::= BEGIN
  T ::= INTEGER (1..10) -- a &lt; b &amp;&amp; c &#x3E; d &#62;
  <![CDATA[U ::= SEQUENCE { a <INTEGER> }]]>
END</sourcecode></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    let block = extraction.blocks.first().unwrap();
    assert!(block.text.contains("-- a < b && c > d >\n"));
    assert!(block.text.contains("U ::= SEQUENCE { a <INTEGER> }"));
    assert_eq!(block.kind, BlockKind::Module);
}

#[test]
fn v2_artwork_typed_asn1_is_a_declared_block() {
    let source = "\
<?xml version=\"1.0\"?>
<!DOCTYPE rfc SYSTEM \"rfc2629.dtd\" [
<!ENTITY RFC2119 SYSTEM \"http://xml.resource.org/public/rfc/bibxml/reference.RFC.2119.xml\">
]>
<rfc ipr=\"trust200902\" docName=\"draft-example-00\">
<front><title>Example</title></front>
<middle>
<section title=\"Intro\"><t>See &RFC2119;.</t></section>
<section title=\"Syntax\">
<figure><artwork type=\"asn.1\"><![CDATA[
Foo DEFINITIONS ::= BEGIN
  Bar ::= BOOLEAN
END
]]></artwork></figure>
</section>
</middle>
<back>
<section title=\"Appendix Module\">
<artwork>
Baz DEFINITIONS ::= BEGIN
  Qux ::= NULL
END
</artwork>
</section>
</back>
</rfc>";
    let extraction = run(source, Format::Xml);
    assert_eq!(module_names(&modules(&extraction)), ["Foo", "Baz"]);
    assert_eq!(codes(&extraction), ["W202"]);
    let labels: Vec<_> = extraction
        .blocks
        .iter()
        .map(|b| (section_label(b), b.origin.title.clone()))
        .collect();
    assert_eq!(
        labels,
        [
            (Some("Section 2".to_owned()), Some("Syntax".to_owned())),
            (
                Some("Appendix A".to_owned()),
                Some("Appendix Module".to_owned())
            ),
        ]
    );
}

#[test]
fn untyped_artwork_without_a_module_header_is_ignored() {
    let source = "<rfc><middle><section><artwork>\n  +---+\n  | A ::= B |\n  +---+\n</artwork></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert!(extraction.blocks.is_empty());
    assert!(extraction.notes.is_empty());
}

#[test]
fn other_types_are_ignored() {
    let source = "<rfc><middle><section><sourcecode type=\"abnf\">foo ::= bar</sourcecode><sourcecode type=\"json\">{}</sourcecode></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert!(extraction.blocks.is_empty());
    assert!(extraction.notes.is_empty());
}

#[test]
fn type_spelling_variants_are_accepted_with_a_warning() {
    let source = "<rfc><middle><section>\
<sourcecode type=\"asn1\">A ::= INTEGER</sourcecode>\
<sourcecode type=\"ASN.1\">B ::= INTEGER</sourcecode>\
<sourcecode type=\"asn.1\">C ::= INTEGER</sourcecode>\
</section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert_eq!(extraction.blocks.len(), 3);
    assert_eq!(codes(&extraction), ["W203", "W203"]);
}

#[test]
fn untyped_sourcecode_that_looks_like_asn1_warns() {
    let source =
        "<rfc><middle><section><sourcecode>A ::= INTEGER</sourcecode></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert_eq!(extraction.blocks.len(), 1);
    assert_eq!(codes(&extraction), ["W202"]);
}

#[test]
fn typed_block_that_is_not_asn1_is_reported_and_skipped() {
    let source = "<rfc><middle><section><sourcecode type=\"asn.1\">30 82 01 0a 02 82</sourcecode></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert!(extraction.blocks.is_empty());
    assert_eq!(codes(&extraction), ["W201"]);
}

#[test]
fn external_sources_are_reported_and_skipped() {
    let source = "<rfc><middle><section><sourcecode type=\"asn.1\" src=\"module.asn\"/></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert!(extraction.blocks.is_empty());
    assert_eq!(codes(&extraction), ["I200"]);
}

#[test]
fn empty_typed_blocks_are_reported_and_skipped() {
    let source = "<rfc><middle><section><sourcecode type=\"asn.1\">  \n </sourcecode></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert!(extraction.blocks.is_empty());
    assert_eq!(codes(&extraction), ["I200"]);
}

#[test]
fn code_markers_inside_sourcecode_are_removed() {
    let source = "<rfc><middle><section><sourcecode type=\"asn.1\" markers=\"true\">\n<CODE BEGINS>\nA ::= INTEGER\n<CODE ENDS>\n</sourcecode></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    let block = extraction.blocks.first().unwrap();
    assert!(!block.text.contains("CODE"));
    assert_eq!(block.text.trim(), "A ::= INTEGER");
}

#[test]
fn unnumbered_sections_do_not_consume_numbers() {
    let source = "<rfc><middle>\
<section numbered=\"false\"><name>Note</name></section>\
<section><name>One</name></section>\
<section><name>Two</name>\
<section><name>Two A</name><sourcecode type=\"asn.1\">A ::= INTEGER</sourcecode></section>\
</section></middle>\
<back><section><name>Ack</name></section><section><name>Mod</name><section><name>Sub</name>\
<sourcecode type=\"asn.1\">B ::= INTEGER</sourcecode></section></section></back></rfc>";
    let extraction = run(source, Format::Xml);
    let labels: Vec<_> = extraction.blocks.iter().map(section_label).collect();
    assert_eq!(
        labels,
        [
            Some("Section 2.1".to_owned()),
            Some("Appendix B.1".to_owned())
        ]
    );
    assert_eq!(
        extraction
            .blocks
            .first()
            .and_then(|b| b.origin.title.as_deref()),
        Some("Two A")
    );
}

#[test]
fn malformed_xml_keeps_what_was_read_and_reports_e203() {
    let source = "<rfc><middle><section><sourcecode type=\"asn.1\">A ::= INTEGER</sourcecode><p>&</section>\u{0}<<<";
    let extraction = run(source, Format::Xml);
    assert_eq!(extraction.blocks.len(), 1);
    assert!(codes(&extraction).contains(&"E203"));
}

#[test]
fn an_unclosed_block_is_reported() {
    let source = "<rfc><middle><section><sourcecode type=\"asn.1\">A ::= INTEGER";
    let extraction = run(source, Format::Xml);
    assert!(extraction.blocks.is_empty());
    assert!(codes(&extraction).contains(&"I200"));
}

#[test]
fn multibyte_text_maps_correctly() {
    let source = "<rfc><middle><section><sourcecode type=\"asn.1\">-- é &lt; ü\nA ::= INTEGER</sourcecode></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert_eq!(extraction.blocks.len(), 1);
}

#[test]
fn a_module_header_in_its_own_sourcecode_joins_the_next_one() {
    // RFC 9286 Appendix A puts the name and identifier in one element and `DEFINITIONS ... END` in the next.
    let source = r#"<rfc><back><section><name>ASN.1 Module</name>
<sourcecode type="asn.1">
    RPKIManifest { iso(1) member-body(2) us(840) rsadsi(113549)
                   pkcs(1) pkcs9(9) smime(16) mod(0) 60 }
</sourcecode>
<sourcecode type="asn.1">
DEFINITIONS EXPLICIT TAGS ::=
   BEGIN
   Manifest ::= SEQUENCE { a INTEGER }
   END
</sourcecode>
</section></back></rfc>"#;
    let extraction = run(source, Format::Xml);
    assert_eq!(module_names(&modules(&extraction)), ["RPKIManifest"]);
    assert_eq!(extraction.blocks.len(), 1);
    assert!(extraction.notes.is_empty(), "{:?}", extraction.notes);
    let block = &extraction.blocks[0];
    let original = &source[block.origin.span.clone()];
    assert!(original.starts_with("<sourcecode") && original.ends_with("</sourcecode>"));
}

#[test]
fn a_header_only_sourcecode_with_nothing_after_it_is_still_reported() {
    let source = r#"<rfc><middle><section><sourcecode type="asn.1">
    Lonely { 1 2 3 }
</sourcecode></section><section><sourcecode type="asn.1">
DEFINITIONS ::= BEGIN END
</sourcecode></section></middle></rfc>"#;
    let extraction = run(source, Format::Xml);
    assert_eq!(
        codes(&extraction),
        ["W201"],
        "the header is not joined across sections"
    );
}

#[test]
fn a_grammar_typed_as_asn1_is_reported_and_skipped() {
    let source = "<rfc><middle><section><sourcecode type=\"asn.1\">\nlabel ::= INTEGER | &lt;null&gt;\nterms ::= term | terms , term\n</sourcecode></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert!(extraction.blocks.is_empty());
    assert_eq!(codes(&extraction), ["I201"]);
}

#[test]
fn an_untyped_grammar_is_skipped_without_a_warning() {
    let source = "<rfc><middle><section><artwork>\naddress ::= \"(\" addr_name SPACE addr_host \")\"\naddr_host ::= nstring\n</artwork></section></middle></rfc>";
    let extraction = run(source, Format::Xml);
    assert!(extraction.blocks.is_empty());
    assert!(codes(&extraction).is_empty());
}
