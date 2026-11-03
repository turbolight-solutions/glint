mod common;
use common::*;
use glint_extract::Format;
use proptest::prelude::*;

const FORMATS: [Format; 4] = [Format::Xml, Format::Markdown, Format::Text, Format::Bare];
const XML_SEED: &str = include_str!("fixtures/xml/rfc8692.xml");
const TEXT_SEED: &str = include_str!("fixtures/text/rfc5280-appendix-a.txt");

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    #[test]
    fn arbitrary_text_never_panics_and_maps_back(src in ".{0,400}") {
        for format in FORMATS {
            let _ = run(&src, format);
        }
    }

    #[test]
    fn document_flavoured_text_never_panics_and_maps_back(
        src in r"[A-Za-z0-9 \n\r\t\x0C{}()<>/:=,.&;!#~`\x22'-]{0,400}"
    ) {
        for format in FORMATS {
            let _ = run(&src, format);
        }
    }

    #[test]
    fn markup_flavoured_text_never_panics(
        parts in proptest::collection::vec(
            prop_oneof![
                Just("<sourcecode type=\"asn.1\">"),
                Just("</sourcecode>"),
                Just("<artwork>"),
                Just("</artwork>"),
                Just("<section>"),
                Just("</section>"),
                Just("<![CDATA["),
                Just("]]>"),
                Just("&lt;"),
                Just("&#x41;"),
                Just("&bogus;"),
                Just("A ::= INTEGER\n"),
                Just("M DEFINITIONS ::= BEGIN\n"),
                Just("END\n"),
                Just("```asn.1\n"),
                Just("```\n"),
                Just("<CODE BEGINS>\n"),
                Just("<CODE ENDS>\n"),
                Just("   [Page 3]\n\u{0C}\n"),
            ],
            0..40,
        )
    ) {
        let src: String = parts.concat();
        for format in FORMATS {
            let _ = run(&src, format);
        }
    }

    #[test]
    fn truncating_real_documents_never_panics(cut_xml in 0..XML_SEED.len(), cut_text in 0..TEXT_SEED.len()) {
        if let Some(prefix) = XML_SEED.get(..cut_xml) {
            let _ = run(prefix, Format::Xml);
        }
        if let Some(prefix) = TEXT_SEED.get(..cut_text) {
            let _ = run(prefix, Format::Text);
        }
    }
}

fn assert_bounded(text: &str, format: Format) {
    let started = std::time::Instant::now();
    let extraction = glint_extract::extract(text, format);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "{format:?} extraction took {:?}",
        started.elapsed()
    );
    drop(extraction);
}

#[test]
fn one_megabyte_documents_extract_in_bounded_time() {
    let xml = XML_SEED.repeat(1_000_000 / XML_SEED.len() + 1);
    assert_bounded(&xml, Format::Xml);
    let text = TEXT_SEED.repeat(1_000_000 / TEXT_SEED.len() + 1);
    assert_bounded(&text, Format::Text);
    assert_bounded(&text, Format::Markdown);
    assert_bounded(&text, Format::Bare);
}

#[test]
fn one_megabyte_of_pathological_input_extracts_in_bounded_time() {
    assert_bounded(&"<sourcecode type=\"asn.1\">".repeat(40_000), Format::Xml);
    assert_bounded(&"<section><name>x</name>".repeat(40_000), Format::Xml);
    assert_bounded(&"&amp;".repeat(200_000), Format::Xml);
    assert_bounded(&"M DEFINITIONS ::= BEGIN\n".repeat(40_000), Format::Text);
    assert_bounded(&"   A ::= SEQUENCE {\n".repeat(50_000), Format::Text);
    assert_bounded(&"\n\n   [Page 1]\n\u{0C}\n".repeat(50_000), Format::Text);
    assert_bounded(&"```asn.1\n".repeat(100_000), Format::Markdown);
    assert_bounded(&"- ".repeat(300_000), Format::Markdown);
}
