//! `report.md`: the curator's to-do list and the parse-rate baseline.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glint_corpus::{ModuleRecord, OverrideTier, ParseStatus, SourceKind, SyntaxEra};
use glint_diag::Severity;

use crate::overrides::Override;

#[derive(Debug, Clone)]
pub struct FindingLine {
    pub code: String,
    pub severity: Severity,
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ModuleOutcome {
    pub status: ParseStatus,
    pub findings: Vec<FindingLine>,
}

pub struct ReportData<'a> {
    pub build: String,
    pub rfcs_scanned: usize,
    pub rfcs_with_modules: usize,
    pub records: &'a [ModuleRecord],
    pub outcomes: &'a [ModuleOutcome],
    pub skipped_smi: &'a [(u32, String)],
    pub unreadable: usize,
    pub extraction_notes: &'a BTreeMap<u32, Vec<String>>,
    pub source_problems: &'a [String],
    pub overrides: &'a [Override],
    pub oid_assignments: usize,
}

fn status_name(status: ParseStatus) -> &'static str {
    match status {
        ParseStatus::Clean => "clean",
        ParseStatus::Warnings => "warnings",
        ParseStatus::Errors => "errors",
        ParseStatus::Unparseable => "unparseable",
    }
}

fn era_name(era: SyntaxEra) -> &'static str {
    match era {
        SyntaxEra::Y1988 => "1988",
        SyntaxEra::Y1988Macro => "1988-macro",
        SyntaxEra::Y2002 => "2002+",
        SyntaxEra::Neutral => "neutral",
    }
}

fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "n/a".to_owned();
    }
    let tenths = part.saturating_mul(1000).checked_div(whole).unwrap_or(0);
    format!(
        "{}.{}%",
        tenths.checked_div(10).unwrap_or(0),
        tenths.checked_rem(10).unwrap_or(0)
    )
}

pub fn render_report(data: &ReportData<'_>) -> String {
    let mut out = String::new();
    let total = data.records.len();
    let count = |status: ParseStatus| data.outcomes.iter().filter(|o| o.status == status).count();
    let clean = count(ParseStatus::Clean);
    let warnings = count(ParseStatus::Warnings);
    let errors = count(ParseStatus::Errors);
    let unparseable = count(ParseStatus::Unparseable);
    let parsed = total.saturating_sub(unparseable);

    let _ = writeln!(out, "# Corpus report {}\n", data.build);
    let _ = writeln!(out, "## Baseline\n");
    let _ = writeln!(out, "| Metric | Value |\n| --- | --- |");
    let _ = writeln!(out, "| RFCs scanned | {} |", data.rfcs_scanned);
    let _ = writeln!(
        out,
        "| RFCs with at least one ASN.1 module | {} |",
        data.rfcs_with_modules
    );
    let _ = writeln!(out, "| Modules in the corpus | {total} |");
    let _ = writeln!(
        out,
        "| Parse rate (modules that parse without syntax errors) | {parsed} / {total} ({}) |",
        percent(parsed, total)
    );
    let _ = writeln!(out, "| Clean | {clean} ({}) |", percent(clean, total));
    let _ = writeln!(out, "| Warnings only | {warnings} |");
    let _ = writeln!(out, "| Errors (parses, fails a check) | {errors} |");
    let _ = writeln!(out, "| Unparseable | {unparseable} |");
    let _ = writeln!(
        out,
        "| SMI MIB modules skipped | {} |",
        data.skipped_smi.len()
    );
    let _ = writeln!(
        out,
        "| Candidates that could not be read as a module | {} |",
        data.unreadable
    );
    let _ = writeln!(
        out,
        "| OID assignments indexed | {} |",
        data.oid_assignments
    );
    let _ = writeln!(out);

    let mut by_code: BTreeMap<&str, usize> = BTreeMap::new();
    for outcome in data.outcomes {
        for finding in &outcome.findings {
            if finding.severity != Severity::Info {
                let slot = by_code.entry(finding.code.as_str()).or_default();
                *slot = slot.saturating_add(1);
            }
        }
    }
    let _ = writeln!(out, "## Findings by code\n");
    let _ = writeln!(out, "| Code | Count |\n| --- | --- |");
    let mut sorted: Vec<_> = by_code.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (code, n) in sorted {
        let _ = writeln!(out, "| `{code}` | {n} |");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Overrides\n");
    if data.overrides.is_empty() {
        let _ = writeln!(out, "None.\n");
    } else {
        let _ = writeln!(
            out,
            "| Document | Module | Tier | Errata | Justification |\n| --- | --- | --- | --- | --- |"
        );
        for item in data.overrides {
            let tier = match item.tier {
                OverrideTier::Extraction => "extraction",
                OverrideTier::ErrataVerified => "errata-verified",
                OverrideTier::Correction => "correction",
            };
            let oid = item.oid.as_ref().map_or(String::new(), |o| format!(" {o}"));
            let _ = writeln!(
                out,
                "| {} | `{}`{oid} | {tier} | {} | {} |",
                item.document,
                item.module,
                item.errata_ids.join(", "),
                item.justification.as_deref().unwrap_or("")
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Modules needing attention\n");
    let _ = writeln!(
        out,
        "Every module that is not clean, with up to five findings each.\n"
    );
    for (record, outcome) in data.records.iter().zip(data.outcomes) {
        if outcome.status == ParseStatus::Clean {
            continue;
        }
        let oid = record
            .oid
            .as_ref()
            .map_or(String::new(), |o| format!(" {o}"));
        let _ = writeln!(
            out,
            "### {} `{}`{oid} — {}\n",
            record.source.label(),
            record.name,
            status_name(outcome.status)
        );
        for finding in outcome
            .findings
            .iter()
            .filter(|f| f.severity != Severity::Info)
            .take(5)
        {
            let _ = writeln!(
                out,
                "- `{}` line {}: {}",
                finding.code, finding.line, finding.message
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## All modules\n");
    let _ = writeln!(
        out,
        "| Document | Module | OID | Era | Status | Source |\n| --- | --- | --- | --- | --- | --- |"
    );
    for (record, outcome) in data.records.iter().zip(data.outcomes) {
        let source = match &record.source.kind {
            SourceKind::Extracted => "extracted".to_owned(),
            SourceKind::Overridden { tier, .. } => format!("override ({tier:?})"),
            SourceKind::Standalone => record.source.body.clone(),
        };
        let _ = writeln!(
            out,
            "| {} | `{}` | {} | {} | {} | {source} |",
            record.source.document,
            record.name,
            record
                .oid
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
            era_name(record.syntax_era),
            status_name(outcome.status),
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Skipped SMI modules\n");
    for (rfc, name) in data.skipped_smi {
        let _ = writeln!(out, "- RFC {rfc}: `{name}`");
    }
    let _ = writeln!(out);

    if !data.source_problems.is_empty() {
        let _ = writeln!(out, "## Source problems\n");
        for problem in data.source_problems {
            let _ = writeln!(out, "- {problem}");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Extraction notes\n");
    for (rfc, notes) in data.extraction_notes {
        for note in notes {
            let _ = writeln!(out, "- RFC {rfc}: {note}");
        }
    }
    out
}
