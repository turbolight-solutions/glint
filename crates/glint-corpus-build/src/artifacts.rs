//! The example artifacts (Codegen and Decoder Design, Example artifacts): the PEM blocks of every RFC and active
//! draft, each identified against the corpus just built, for `artifacts.json`, the report and
//! `corpus/artifacts.expected`.

use std::fmt::Write as _;

use glint_check::artifacts::{Candidate, Identifier, document_roots};
use glint_corpus::{Artifact, ArtifactsData, BrokenArtifact, Corpus};
use glint_decode::base64_encode;
use glint_diag::Severity;
use glint_extract::artifacts::PemScan;
use rayon::prelude::*;
use sha2::{Digest, Sha256};

/// The PEM blocks found in one document.
pub struct ScannedDocument {
    /// `rfc9500`, `draft-ietf-lamps-foo-03`.
    pub document: String,
    pub scan: PemScan,
    pub obsoleted: bool,
}

/// Identifies every block of `documents` against `corpus`. The build is left empty.
pub fn identify_all(corpus: &Corpus, documents: &[ScannedDocument]) -> ArtifactsData {
    let identified: Vec<(Vec<Artifact>, Vec<BrokenArtifact>)> = documents
        .par_iter()
        .map_init(
            || Identifier::new(corpus),
            |identifier, document| identify_document(identifier, corpus, document),
        )
        .collect();
    let mut data = ArtifactsData::default();
    for (artifacts, broken) in identified {
        data.artifacts.extend(artifacts);
        data.broken.extend(broken);
    }
    data
}

fn identify_document(
    identifier: &Identifier<'_>,
    corpus: &Corpus,
    document: &ScannedDocument,
) -> (Vec<Artifact>, Vec<BrokenArtifact>) {
    let own: Vec<Candidate> = if document.scan.blocks.is_empty() {
        Vec::new()
    } else {
        document_roots(corpus, &document.document)
    };
    let artifacts = document
        .scan
        .blocks
        .iter()
        .map(|block| {
            let identification = identifier.identify(&block.der, &block.label, &own);
            let first_error = identification
                .findings
                .iter()
                .find(|f| f.severity == Severity::Error)
                .map(|f| format!("{}: {}", f.code, f.message));
            Artifact {
                id: format!("{}-{}", document.document, block.index),
                document: document.document.clone(),
                section: block.place.section.as_ref().map(|s| s.number.clone()),
                appendix: block.place.section.as_ref().is_some_and(|s| s.appendix),
                title: block.place.title.clone(),
                caption: block.place.caption.clone(),
                line: block.place.line,
                label: block.label.clone(),
                sha256: hex(&Sha256::digest(&block.der)),
                der: base64_encode(&block.der),
                findings: identification
                    .codes()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                root: identification.root,
                confidence: identification.confidence,
                first_error,
                obsoleted: document.obsoleted,
            }
        })
        .collect();
    let broken = document
        .scan
        .findings
        .iter()
        .map(|finding| BrokenArtifact {
            id: format!("{}-{}", document.document, finding.index),
            document: document.document.clone(),
            section: finding.place.section.as_ref().map(|s| s.number.clone()),
            appendix: finding.place.section.as_ref().is_some_and(|s| s.appendix),
            line: finding.place.line,
            label: finding.label.clone(),
            message: finding.message.clone(),
        })
        .collect();
    (artifacts, broken)
}

/// The RFC number and block index of an artifact id of an RFC (`rfc9500-4`).
fn rfc_key(id: &str) -> Option<(u32, usize)> {
    let (document, index) = id.rsplit_once('-')?;
    Some((glint_corpus::rfc_number(document)?, index.parse().ok()?))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `corpus/artifacts.expected`: one line per artifact of an RFC (drafts change too often to be golden), `id`, root
/// type, confidence and finding codes, tab-separated, in id order.
pub fn expected(data: &ArtifactsData) -> String {
    let mut out = String::from(
        "# Example artifacts of the RFCs: id, type, confidence, decode findings. Written by corpus-build; see\n\
         # docs/Glint — Codegen and Decoder Design.md, Example artifacts: Corpus self-audit.\n",
    );
    let mut lines: Vec<(u32, usize, String)> = Vec::new();
    for artifact in &data.artifacts {
        let Some(key) = rfc_key(&artifact.id) else {
            continue;
        };
        let findings = if artifact.findings.is_empty() {
            "-".to_owned()
        } else {
            artifact.findings.join(",")
        };
        let line = format!(
            "{}\t{}\t{}\t{findings}",
            artifact.id,
            artifact.root.as_deref().unwrap_or("-"),
            artifact.confidence.as_str(),
        );
        lines.push((key.0, key.1, line));
    }
    for broken in &data.broken {
        if let Some(key) = rfc_key(&broken.id) {
            lines.push((key.0, key.1, format!("{}\tbroken\t-\t-", broken.id)));
        }
    }
    lines.sort();
    for (_, _, line) in lines {
        let _ = writeln!(out, "{line}");
    }
    out
}
