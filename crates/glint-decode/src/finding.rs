//! What the decoder reports about an encoding, in the diagnostics scheme (`glint explain E400`).

use glint_diag::{Severity, code_info};
use serde::Serialize;

/// One finding on a TLV, or on the input as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecodeFinding {
    pub code: &'static str,
    pub severity: Severity,
    /// One line, no trailing period.
    pub message: String,
}

impl DecodeFinding {
    /// A finding with the severity registered for `code`.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: code_info(code).map_or(Severity::Error, |info| info.severity),
            message: message.into(),
        }
    }

    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
}

/// The identifier, length or nesting of the octets is broken (E400).
pub(crate) fn malformed(message: impl Into<String>) -> DecodeFinding {
    DecodeFinding::new("E400", message)
}

/// The contents are not a valid encoding of a value of the type (E401).
pub(crate) fn invalid(message: impl Into<String>) -> DecodeFinding {
    DecodeFinding::new("E401", message)
}

/// Valid BER that DER forbids (W402).
pub(crate) fn der(message: impl Into<String>) -> DecodeFinding {
    DecodeFinding::new("W402", message)
}
