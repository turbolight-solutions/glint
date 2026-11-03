use serde::{Deserialize, Serialize};

use crate::Oid;

/// Which grammar family a module belongs to (design spec, "Module record").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyntaxEra {
    /// Uses `ANY`.
    #[serde(rename = "1988")]
    Y1988,
    /// Defines or uses `MACRO` notation.
    #[serde(rename = "1988-macro")]
    Y1988Macro,
    /// Uses X.681 constructs.
    #[serde(rename = "2002+")]
    Y2002,
    Neutral,
}

/// The result of parsing and checking a corpus module with the full corpus available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParseStatus {
    Clean,
    Warnings,
    Errors,
    Unparseable,
}

/// How a module's text was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverrideTier {
    Extraction,
    ErrataVerified,
    Correction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceKind {
    Extracted,
    Overridden {
        tier: OverrideTier,
        errata_ids: Vec<String>,
    },
    Itu {
        url: String,
        recommendation: String,
        edition: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleSource {
    pub rfc: Option<u32>,
    pub section: Option<String>,
    pub kind: SourceKind,
}

impl ModuleSource {
    /// `rfc5280`, or the Recommendation for ITU-T modules.
    pub fn label(&self) -> String {
        match (&self.kind, self.rfc) {
            (SourceKind::Itu { recommendation, .. }, _) => recommendation.clone(),
            (_, Some(rfc)) => format!("rfc{rfc}"),
            _ => "unknown".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Exports {
    /// No `EXPORTS` clause, or `EXPORTS ALL`.
    All,
    List(Vec<String>),
}

/// One module of the corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleRecord {
    pub name: String,
    pub oid: Option<Oid>,
    pub source: ModuleSource,
    pub rfc_status: Option<String>,
    pub obsoleted_by: Vec<u32>,
    pub syntax_era: SyntaxEra,
    pub text: String,
    pub exports: Exports,
    pub parse_status: ParseStatus,
    pub sha256: String,
}

impl ModuleRecord {
    /// `rfc5280#PKIX1Explicit88`, the file name used in diagnostics.
    pub fn label(&self) -> String {
        format!("{}#{}", self.source.label(), self.name)
    }
}

/// A value assignment of type `OBJECT IDENTIFIER`, evaluated to dotted form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidAssignment {
    pub oid: Oid,
    pub name: String,
    pub module: String,
    pub rfc: Option<u32>,
}

/// Identifies a corpus build.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CorpusInfo {
    /// `YYYY-MM-DD` of the build.
    pub version: String,
    pub notices: Vec<String>,
}

/// The serialized corpus.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Bundle {
    pub info: CorpusInfo,
    pub modules: Vec<ModuleRecord>,
    pub oid_assignments: Vec<OidAssignment>,
}
