//! Diagnostic types and renderers shared by every Glint front end.

mod codes;
mod explain;
mod line_index;
mod render;
mod sarif;

use std::ops::Range;

use serde::Serialize;

pub use codes::{CodeInfo, all_codes, code_info};
pub use explain::{Example, examples, explain, explained_codes};
pub use line_index::LineIndex;
pub use render::{Files, Versions, render_human, render_json};
pub use sarif::{SarifInput, render_sarif};

/// Identifies a file in a [`Files`] set: the draft, or a corpus module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct FileId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A place in a file that a diagnostic points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    pub file: FileId,
    /// Byte offsets in the original file.
    pub span: Range<usize>,
    /// 1-based line and column.
    pub line_col: (u32, u32),
    pub section: Option<String>,
    pub text: Option<String>,
}

impl Label {
    /// Builds a label for `span`, computing its line and column from `index`.
    pub fn new(file: FileId, span: Range<usize>, index: &LineIndex) -> Self {
        let line_col = index.line_col(span.start);
        Self {
            file,
            span,
            line_col,
            section: None,
            text: None,
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_section(mut self, section: impl Into<String>) -> Self {
        self.section = Some(section.into());
        self
    }
}

/// One finding. Every renderer prints exactly this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    /// One line, no trailing period.
    pub message: String,
    pub primary: Label,
    pub secondary: Vec<Label>,
    pub notes: Vec<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    /// Creates a diagnostic with the default severity registered for `code`.
    pub fn new(code: &'static str, message: impl Into<String>, primary: Label) -> Self {
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

    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_secondary(mut self, label: Label) -> Self {
        self.secondary.push(label);
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
}
