//! `report.md`: the curator's to-do list and the parse-rate baseline.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glint_corpus::{AllowedOid, ModuleRecord, OverrideTier, ParseStatus, SourceKind, SyntaxEra};
use glint_diag::Severity;

use crate::overrides::Override;

#[derive(Debug, Clone)]
pub struct FindingLine {
    pub code: String,
    pub severity: Severity,
    pub line: u32,
    pub message: String,
}

/// A label table finding about a corpus module.
#[derive(Debug, Clone)]
pub struct AuditLine {
    pub document: String,
    pub module: String,
    pub finding: FindingLine,
    pub notes: Vec<String>,
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
    pub drafts_scanned: usize,
    pub drafts_with_modules: usize,
    pub records: &'a [ModuleRecord],
    pub outcomes: &'a [ModuleOutcome],
    /// By record: a module of an obsoleted RFC that no maintained module of another document imports. Its findings
    /// are left out of the totals of the maintained modules.
    pub unmaintained: &'a [bool],
    pub skipped_smi: &'a [(u32, String)],
    pub unreadable: usize,
    pub extraction_notes: &'a BTreeMap<u32, Vec<String>>,
    /// The notes on drafts with a module: why one is kept as RFCXML for the web UI.
    pub draft_notes: &'a BTreeMap<String, Vec<String>>,
    pub source_problems: &'a [String],
    pub overrides: &'a [Override],
    /// The errata applied, in order, by RFC.
    pub errata_applied: &'a BTreeMap<u32, Vec<u32>>,
    /// The RFCs built as published because their errata did not apply, and the erratum that failed.
    pub errata_failed: &'a BTreeMap<u32, glint_errata::SeriesFailure>,
    pub oid_assignments: usize,
    pub oid_nodes: usize,
    pub allowlist: &'a [AllowedOid],
    pub label_audit: &'a [AuditLine],
    /// Entries read from the IANA module identifier registries.
    pub iana_module_ids: usize,
    /// Rows of the decoder's OID hint table.
    pub oid_hints: usize,
    pub hint_conflicts: &'a [HintConflict],
}

/// An OID, class and field that two modules bind to different types; the hint table keeps one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintConflict {
    pub oid: String,
    /// `EXTENSION.&ExtnType`.
    pub field: String,
    pub kept: String,
    pub others: Vec<String>,
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
    let _ = writeln!(
        out,
        "| Active Internet-Drafts scanned | {} |",
        data.drafts_scanned
    );
    let _ = writeln!(
        out,
        "| Drafts with at least one ASN.1 module | {} |",
        data.drafts_with_modules
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
    let unmaintained = data.unmaintained.iter().filter(|&&u| u).count();
    let _ = writeln!(
        out,
        "| Unmaintained (obsoleted RFC, imported by no maintained module of another document) | {unmaintained} |"
    );
    let maintained_errors = data
        .outcomes
        .iter()
        .zip(data.unmaintained)
        .filter(|(o, u)| !**u && matches!(o.status, ParseStatus::Errors | ParseStatus::Unparseable))
        .count();
    let _ = writeln!(
        out,
        "| Maintained modules with errors (errors or unparseable) | {maintained_errors} |"
    );
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
    let _ = writeln!(out, "| OIDs with labels | {} |", data.oid_nodes);
    let _ = writeln!(
        out,
        "| IANA module identifier entries | {} |",
        data.iana_module_ids
    );
    let _ = writeln!(out, "| OID hint table rows | {} |", data.oid_hints);
    let _ = writeln!(
        out,
        "| Label table and registry findings (W142, W143, W144, W146) | {} |",
        data.label_audit.len()
    );
    let _ = writeln!(out);

    // By code: (all modules, maintained modules).
    let mut by_code: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (index, outcome) in data.outcomes.iter().enumerate() {
        let maintained = !data.unmaintained.get(index).copied().unwrap_or(false);
        for finding in &outcome.findings {
            if finding.severity != Severity::Info {
                let slot = by_code.entry(finding.code.as_str()).or_default();
                slot.0 = slot.0.saturating_add(1);
                if maintained {
                    slot.1 = slot.1.saturating_add(1);
                }
            }
        }
    }
    let _ = writeln!(out, "## Findings by code\n");
    let _ = writeln!(
        out,
        "Maintained: without the findings of unmaintained modules (obsoleted RFCs that no maintained module of another \
         document imports).\n"
    );
    let _ = writeln!(out, "| Code | All | Maintained |\n| --- | --- | --- |");
    let mut sorted: Vec<_> = by_code.into_iter().collect();
    sorted.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    let (mut all, mut maintained) = (0_usize, 0_usize);
    for (code, (n, m)) in sorted {
        all = all.saturating_add(n);
        maintained = maintained.saturating_add(m);
        let _ = writeln!(out, "| `{code}` | {n} | {m} |");
    }
    let _ = writeln!(out, "| Total | {all} | {maintained} |");
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

    let _ = writeln!(out, "## Errata\n");
    let _ = writeln!(
        out,
        "Verified RFC errata applied to the documents before extraction (`corpus/errata`).\n"
    );
    if data.errata_applied.is_empty() && data.errata_failed.is_empty() {
        let _ = writeln!(out, "None.\n");
    } else {
        if !data.errata_applied.is_empty() {
            let _ = writeln!(
                out,
                "| RFC | Errata | Modules corrected |\n| --- | --- | --- |"
            );
            for (rfc, ids) in data.errata_applied {
                let document = format!("rfc{rfc}");
                let modules: Vec<String> = data
                    .records
                    .iter()
                    .filter(|r| r.source.document == document && !r.source.errata.is_empty())
                    .map(|r| format!("`{}`", r.name))
                    .collect();
                let ids: Vec<String> = ids.iter().map(u32::to_string).collect();
                let _ = writeln!(
                    out,
                    "| {rfc} | {} | {} |",
                    ids.join(", "),
                    modules.join(", ")
                );
            }
            let _ = writeln!(out);
        }
        for (rfc, failure) in data.errata_failed {
            let _ = writeln!(
                out,
                "- RFC {rfc} is built as published: the patch of erratum {} does not apply ({})",
                failure.errata_id, failure.reason
            );
        }
        if !data.errata_failed.is_empty() {
            let _ = writeln!(out);
        }
    }

    let _ = writeln!(out, "## OID label audit\n");
    let _ = writeln!(
        out,
        "Each module checked against the label table without its own document's evidence, and its identifier \
         against the IANA module identifier registries (`W146`). A finding here is a candidate erratum, or an OID for \
         the allowlist (`corpus/overrides/oid-allowlist.toml`).\n"
    );
    if data.label_audit.is_empty() {
        let _ = writeln!(out, "None.\n");
    } else {
        for line in data.label_audit {
            let notes = if line.notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", line.notes.join("; "))
            };
            let _ = writeln!(
                out,
                "- {} `{}` line {}: `{}` {}{notes}",
                line.document,
                line.module,
                line.finding.line,
                line.finding.code,
                line.finding.message
            );
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "Allowlisted OIDs:\n");
    for allowed in data.allowlist {
        let _ = writeln!(out, "- {}: {}", allowed.oid, allowed.reason);
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## OID hint conflicts\n");
    let _ = writeln!(
        out,
        "OIDs whose objects set a type field to different types in different 2002-syntax modules. The decoder's hint \
         table keeps the module of the most recent current RFC.\n"
    );
    if data.hint_conflicts.is_empty() {
        let _ = writeln!(out, "None.\n");
    } else {
        for conflict in data.hint_conflicts {
            let _ = writeln!(
                out,
                "- {} {}: kept {}; also {}",
                conflict.oid,
                conflict.field,
                conflict.kept,
                conflict.others.join(", ")
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Modules needing attention\n");
    let _ = writeln!(
        out,
        "Every module that is not clean, with up to five findings each.\n"
    );
    for (index, (record, outcome)) in data.records.iter().zip(data.outcomes).enumerate() {
        if outcome.status == ParseStatus::Clean {
            continue;
        }
        let oid = record
            .oid
            .as_ref()
            .map_or(String::new(), |o| format!(" {o}"));
        let unmaintained = if data.unmaintained.get(index).copied().unwrap_or(false) {
            let by: Vec<String> = record
                .obsoleted_by
                .iter()
                .map(|n| format!("RFC {n}"))
                .collect();
            format!(" (unmaintained: obsoleted by {})", by.join(", "))
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "### {} `{}`{oid} — {}{unmaintained}\n",
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
    for (draft, notes) in data.draft_notes {
        for note in notes {
            let _ = writeln!(out, "- {draft}: {note}");
        }
    }
    out
}
