use std::ops::Range;

pub use glint_syntax::Severity;

/// A finding made while extracting, with a stable diagnostic code and a span in the original document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractNote {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub span: Range<usize>,
}

impl ExtractNote {
    /// Creates a finding whose severity follows the code prefix (`E`, `W` or `I`).
    pub fn new(code: &'static str, message: impl Into<String>, span: Range<usize>) -> Self {
        let severity = match code.as_bytes().first() {
            Some(b'W') => Severity::Warning,
            Some(b'I') => Severity::Info,
            _ => Severity::Error,
        };
        Self {
            code,
            severity,
            message: message.into(),
            span,
        }
    }
}
