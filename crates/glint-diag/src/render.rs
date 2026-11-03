use std::io::Write;

use ariadne::{Color, Config, IndexType, Label as AriadneLabel, Report, ReportKind};
use serde::Serialize;

use crate::{Diagnostic, FileId, LineIndex, Severity};

const JSON_SCHEMA_VERSION: u32 = 1;

/// The files that diagnostics refer to, by [`FileId`].
#[derive(Debug, Default)]
pub struct Files {
    entries: Vec<(String, String, LineIndex)>,
}

impl Files {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an ASN.1 file and returns its id together with a line index for building labels.
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> (FileId, LineIndex) {
        self.add_with(name.into(), text.into(), LineIndex::new)
    }

    /// Adds a draft or RFC, whose lines are counted as editors count them (form feeds do not end a line).
    pub fn add_document(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> (FileId, LineIndex) {
        self.add_with(name.into(), text.into(), LineIndex::for_document)
    }

    fn add_with(
        &mut self,
        name: String,
        text: String,
        index: fn(&str) -> LineIndex,
    ) -> (FileId, LineIndex) {
        let index = index(&text);
        let id = FileId(u32::try_from(self.entries.len()).unwrap_or(u32::MAX));
        self.entries.push((name, text, index.clone()));
        (id, index)
    }

    /// Every file with its id, in the order added.
    pub fn iter(&self) -> impl Iterator<Item = (FileId, &str)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, (name, _, _))| (FileId(u32::try_from(i).unwrap_or(u32::MAX)), name.as_str()))
    }

    pub fn name(&self, id: FileId) -> &str {
        self.entry(id)
            .map_or("<unknown>", |(name, _, _)| name.as_str())
    }

    /// The line index of a file, with the line rules it was added under.
    pub fn line_index(&self, id: FileId) -> Option<&LineIndex> {
        self.entry(id).map(|(_, _, index)| index)
    }

    fn entry(&self, id: FileId) -> Option<&(String, String, LineIndex)> {
        self.entries.get(usize::try_from(id.0).ok()?)
    }
}

fn report_kind(severity: Severity) -> ReportKind<'static> {
    match severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
        Severity::Info => ReportKind::Advice,
    }
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Error => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Info => Color::Cyan,
    }
}

/// Renders diagnostics with source excerpts, in the human format.
pub fn render_human(diagnostics: &[Diagnostic], files: &Files, color: bool) -> String {
    let mut out = Vec::new();
    for diagnostic in diagnostics {
        let _ = write_human(&mut out, diagnostic, files, color);
    }
    String::from_utf8_lossy(&out).into_owned()
}

type Span = (String, std::ops::Range<usize>);

fn write_human(
    out: &mut Vec<u8>,
    diagnostic: &Diagnostic,
    files: &Files,
    color: bool,
) -> std::io::Result<()> {
    let primary_name = files.name(diagnostic.primary.file).to_owned();
    let span: Span = (primary_name, diagnostic.primary.span.clone());
    let mut builder = Report::<Span>::build(report_kind(diagnostic.severity), span.clone())
        .with_code(diagnostic.code)
        .with_message(&diagnostic.message)
        .with_config(
            Config::default()
                .with_color(color)
                .with_index_type(IndexType::Byte),
        );

    let mut primary_label = AriadneLabel::new(span).with_color(severity_color(diagnostic.severity));
    primary_label =
        primary_label.with_message(diagnostic.primary.text.as_deref().unwrap_or("here"));
    builder = builder.with_label(primary_label);

    for secondary in &diagnostic.secondary {
        let span: Span = (
            files.name(secondary.file).to_owned(),
            secondary.span.clone(),
        );
        let mut label = AriadneLabel::new(span).with_color(Color::Blue);
        if let Some(text) = &secondary.text {
            label = label.with_message(text);
        }
        builder = builder.with_label(label);
    }
    for note in &diagnostic.notes {
        builder = builder.with_note(note);
    }
    if let Some(help) = &diagnostic.help {
        builder = builder.with_help(help);
    }

    let mut cache = ariadne::sources(
        files
            .entries
            .iter()
            .map(|(name, text, _)| (name.clone(), text.clone())),
    );
    let report = builder.finish();
    if report.write(&mut cache, &mut *out).is_err() {
        let _ = writeln!(out, "{}: {}", diagnostic.code, diagnostic.message);
    }
    Ok(())
}

#[derive(Serialize)]
struct JsonReport<'a> {
    schema_version: u32,
    diagnostics: &'a [Diagnostic],
}

/// Renders diagnostics as JSON with a `schema_version` field.
pub fn render_json(diagnostics: &[Diagnostic]) -> String {
    let report = JsonReport {
        schema_version: JSON_SCHEMA_VERSION,
        diagnostics,
    };
    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Diagnostic, Label};

    fn sample() -> (Files, Vec<Diagnostic>) {
        let mut files = Files::new();
        let (id, index) = files.add("draft.asn", "A ::= SEQUENCE { a INTEGER b BOOLEAN }\n");
        let label = Label::new(id, 28..29, &index).with_text("expected `,`");
        let diagnostic =
            Diagnostic::new("E001", "expected `}`, found `b`", label).with_help("add a comma");
        (files, vec![diagnostic])
    }

    #[test]
    fn human_output_shows_code_message_location_and_help() {
        let (files, diagnostics) = sample();
        let out = render_human(&diagnostics, &files, false);
        assert!(out.contains("E001"), "{out}");
        assert!(out.contains("expected `}`, found `b`"), "{out}");
        assert!(out.contains("draft.asn:1:29"), "{out}");
        assert!(out.contains("add a comma"), "{out}");
    }

    #[test]
    fn json_output_carries_a_schema_version() {
        let (_, diagnostics) = sample();
        let value: serde_json::Value = serde_json::from_str(&render_json(&diagnostics)).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["diagnostics"][0]["code"], "E001");
        assert_eq!(value["diagnostics"][0]["severity"], "error");
        assert_eq!(
            value["diagnostics"][0]["primary"]["line_col"],
            serde_json::json!([1, 29])
        );
    }

    #[test]
    fn registered_codes_supply_their_default_severity() {
        let mut files = Files::new();
        let (id, index) = files.add("x", "");
        let warning = Diagnostic::new("W136", "m", Label::new(id, 0..0, &index));
        assert_eq!(warning.severity, crate::Severity::Warning);
    }
}
