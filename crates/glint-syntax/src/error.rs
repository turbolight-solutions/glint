use rowan::TextRange;

/// How serious the parser considers a finding. Consumers may remap it through configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A lexical or syntax finding with a stable diagnostic code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub range: TextRange,
}

impl SyntaxError {
    /// Creates a finding whose severity follows the code prefix (`E`, `W` or `I`).
    pub fn new(code: &'static str, message: impl Into<String>, range: TextRange) -> Self {
        let severity = match code.as_bytes().first() {
            Some(b'W') => Severity::Warning,
            Some(b'I') => Severity::Info,
            _ => Severity::Error,
        };
        Self {
            code,
            severity,
            message: message.into(),
            range,
        }
    }

    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
}
