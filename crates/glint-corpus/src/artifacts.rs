//! `artifacts.json`: the PEM examples of every RFC and active draft, with the type each was identified as (Codegen and
//! Decoder Design, Example artifacts: Data file).

use serde::{Deserialize, Serialize};

use crate::data::{DataError, check_schema};

pub const ARTIFACTS_FILE: &str = "artifacts.json";

/// How an example's type was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// A root the PEM label suggests.
    Label,
    /// A type that decodes the octets, tried without the label's help.
    Decoded,
    /// No candidate fits.
    Unknown,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Label => "label",
            Self::Decoded => "decoded",
            Self::Unknown => "unknown",
        }
    }
}

/// One PEM example whose octets are BER.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// `rfc9500-4`: the document and the block's position among its PEM blocks.
    pub id: String,
    pub document: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub appendix: bool,
    /// The section's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// 1-based line of the `BEGIN` line in the document's source (its RFCXML when it has one).
    pub line: u32,
    /// The label as written.
    pub label: String,
    /// SHA-256 of the octets, hex.
    pub sha256: String,
    /// The octets, base64.
    pub der: String,
    /// The type it was identified as (`rfc5912#PKIX1Explicit-2009.Certificate`); for `unknown`, the closest
    /// candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub confidence: Confidence,
    /// The codes of the decode's findings as `root`, each once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    /// The first error of the decode as `root`, for an example with errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_error: Option<String>,
    /// The document is an obsoleted RFC.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub obsoleted: bool,
}

impl Artifact {
    /// Whether the decode as its type has errors: with the decoder correct, an erratum candidate.
    pub fn has_errors(&self) -> bool {
        self.first_error.is_some()
    }
}

/// A PEM block that is not kept: not terminated, not base64, not BER.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokenArtifact {
    pub id: String,
    pub document: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub appendix: bool,
    pub line: u32,
    pub label: String,
    pub message: String,
}

/// The contents of `artifacts.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactsData {
    pub schema: u32,
    pub build: String,
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub broken: Vec<BrokenArtifact>,
}

impl Default for ArtifactsData {
    fn default() -> Self {
        Self {
            schema: crate::SCHEMA,
            build: String::new(),
            artifacts: Vec::new(),
            broken: Vec::new(),
        }
    }
}

impl ArtifactsData {
    /// Reads `artifacts.json`, refusing data of another schema.
    pub fn from_json(json: &[u8]) -> Result<Self, DataError> {
        check_schema(json)?;
        serde_json::from_slice(json).map_err(|e| DataError::Damaged(e.to_string()))
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Refuses a file of another build than `expected`, the corpus's.
    pub fn require_build(&self, expected: &str) -> Result<(), DataError> {
        if self.build == expected {
            Ok(())
        } else {
            Err(DataError::BuildMismatch {
                file: ARTIFACTS_FILE,
                found: self.build.clone(),
                expected: expected.to_owned(),
            })
        }
    }

    pub fn get(&self, id: &str) -> Option<&Artifact> {
        self.artifacts
            .iter()
            .find(|a| a.id.eq_ignore_ascii_case(id))
    }
}
