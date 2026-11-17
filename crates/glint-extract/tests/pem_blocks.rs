#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

//! PEM extraction for the example artifacts (Codegen and Decoder Design, Example artifacts: Extraction): page breaks
//! inside a block, RFC 8792 folding, indentation, RFCXML elements, and blocks that are not usable.

use std::path::Path;

use glint_extract::Format;
use glint_extract::artifacts::{PemProblem, pem_blocks};

fn read(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pem")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn certificate() -> Vec<u8> {
    let pem = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../glint-check/tests/fixtures/decode/ec-ca.pem"),
    )
    .unwrap();
    glint_decode::sniff(pem.as_bytes()).unwrap().remove(0).bytes
}

#[test]
fn text_blocks_lose_page_furniture_folding_and_indentation() {
    let scan = pem_blocks(&read("paginated.txt"), Format::Text);
    let cert = certificate();
    assert_eq!(scan.blocks.len(), 3, "{:?}", scan.findings);
    for block in &scan.blocks {
        assert_eq!(block.label, "CERTIFICATE");
        assert_eq!(block.der, cert, "block {}", block.index);
    }
    let first = &scan.blocks[0].place;
    assert_eq!(first.section.as_ref().unwrap().to_string(), "Section 2.1");
    assert_eq!(first.title.as_deref(), Some("A Certificate"));
    assert_eq!(first.caption.as_deref(), Some("An EC CA certificate"));
    assert_eq!(first.line, 16);
    assert_eq!(
        scan.blocks
            .iter()
            .map(|b| b.place.section.as_ref().unwrap().number.as_str())
            .collect::<Vec<_>>(),
        ["2.1", "2.2", "2.2"]
    );
}

#[test]
fn broken_blocks_are_findings_and_openpgp_armor_is_left_out() {
    let scan = pem_blocks(&read("paginated.txt"), Format::Text);
    let problems: Vec<(usize, &str, PemProblem)> = scan
        .findings
        .iter()
        .map(|f| (f.index, f.label.as_str(), f.problem))
        .collect();
    assert_eq!(
        problems,
        [
            (4, "CERTIFICATE", PemProblem::Base64),
            (5, "PUBLIC KEY", PemProblem::LabelMismatch),
            (6, "PRIVATE KEY", PemProblem::NotBer),
            (7, "CERTIFICATE", PemProblem::Unterminated),
        ]
    );
    assert!(scan.findings[0].message.contains("elided"));
    assert_eq!(scan.openpgp.len(), 1);
    assert_eq!(scan.openpgp[0].0, "PGP MESSAGE");
}

#[test]
fn xml_elements_are_scanned_one_by_one() {
    let source = read("draft.xml");
    let scan = pem_blocks(&source, Format::Xml);
    // The block split across two elements is not joined: its first half has no END.
    assert_eq!(scan.blocks.len(), 1, "{:?}", scan.findings);
    assert_eq!(scan.blocks[0].der, certificate());
    assert_eq!(scan.blocks[0].index, 2);
    assert_eq!(
        scan.blocks[0].place.section.as_ref().unwrap().to_string(),
        "Section 2"
    );
    assert_eq!(scan.findings.len(), 1);
    assert_eq!(scan.findings[0].problem, PemProblem::Unterminated);
    assert_eq!(
        scan.findings[0].place.caption.as_deref(),
        Some("An EC CA certificate")
    );
    let line = scan.blocks[0].place.line as usize;
    assert_eq!(
        source.lines().nth(line - 1),
        Some("-----BEGIN CERTIFICATE-----")
    );
}
