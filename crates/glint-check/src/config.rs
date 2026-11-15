use std::collections::BTreeMap;

use glint_diag::Severity;
use serde::Deserialize;

/// The contents of `glint.toml` (design spec, "Config file").
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub pins: BTreeMap<String, String>,
    pub severity: BTreeMap<String, SeveritySetting>,
    pub snippets: SnippetsConfig,
    pub oid: OidConfig,
    pub examples: ExamplesConfig,
}

/// The PEM examples of the document (Codegen and Decoder Design, Example artifacts: Examples in a checked draft).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExamplesConfig {
    /// Decode the PEM examples and report what is wrong with them (E410, W411, W412, I413, W415).
    pub check: bool,
    /// Name the type each example was identified as (I414).
    pub identified: bool,
}

impl Default for ExamplesConfig {
    fn default() -> Self {
        Self {
            check: true,
            identified: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SnippetsConfig {
    pub elision_comments: Vec<String>,
    pub reverse_check: bool,
}

impl Default for SnippetsConfig {
    fn default() -> Self {
        Self {
            elision_comments: vec!["-- (snip)".to_owned(), "-- ...".to_owned()],
            reverse_check: false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OidConfig {
    pub placeholders: Vec<String>,
    /// Report labels that differ from the corpus's name for the same arc (I145).
    pub synonym_labels: bool,
}

/// What a code is remapped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeveritySetting {
    Off,
    Error,
    Warning,
    Info,
}

impl SeveritySetting {
    pub fn severity(self) -> Option<Severity> {
        match self {
            Self::Off => None,
            Self::Error => Some(Severity::Error),
            Self::Warning => Some(Severity::Warning),
            Self::Info => Some(Severity::Info),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid configuration: {0}")]
pub struct ConfigError(String);

impl Config {
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError(e.to_string()))
    }
}
