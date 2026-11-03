use glint_diag::{Severity, code_info};
use glint_syntax::TextRange;

/// Identifies one source text: a block of the draft, or a corpus module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub u32);

/// A place inside a source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loc {
    pub source: SourceId,
    pub range: TextRange,
}

/// A diagnostic before it is mapped to document coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub primary: Loc,
    pub secondary: Vec<(Loc, String)>,
    pub notes: Vec<String>,
    pub help: Option<String>,
}

impl Finding {
    /// A finding at `primary` with the default severity registered for `code`.
    pub fn new(code: &'static str, message: impl Into<String>, primary: Loc) -> Self {
        let severity = code_info(code).map_or(Severity::Error, |info| info.severity);
        Self {
            code,
            severity,
            message: message.into(),
            primary,
            secondary: Vec::new(),
            notes: Vec::new(),
            help: None,
        }
    }

    pub fn with_secondary(mut self, loc: Loc, text: impl Into<String>) -> Self {
        self.secondary.push((loc, text.into()));
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
}
