//! The triage ledger, `corpus/errata/ledger.tsv`: one row per triaged erratum, sorted by (rfc, errata_id). The row is
//! the commit point: a patch is applied only while its row says `patched`.

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use regex::Regex;

use crate::Erratum;
use crate::chars::line_forbidden;
use crate::feed::ascii_number;

pub const LEDGER_HEADER: &str = "rfc\terrata_id\tdecision\tby\tfingerprint\trule\tnote";
pub const MAX_NOTE_CHARS: usize = 500;
const COLUMNS: usize = 7;

struct Patterns {
    email: Regex,
    superseded: Regex,
}

static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(|| {
    Some(Patterns {
        // A local part of address characters, so an ASN.1 relation constraint such as `{@field.subfield}` (RFC 5958
        // erratum 8490) is not taken for an address.
        email: Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}")
            .ok()?,
        superseded: Regex::new(r"\bRFC ?([0-9]+)\b").ok()?,
    })
});

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct LedgerError(pub String);

macro_rules! keyword_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &[$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ();

            fn from_str(s: &str) -> Result<Self, ()> {
                match s { $($text => Ok($name::$variant),)+ _ => Err(()) }
            }
        }
    };
}

keyword_enum!(
    /// What triage decided.
    Decision {
        Patched = "patched",
        NotApplicable = "not-applicable",
        Superseded = "superseded",
        NeedsHuman = "needs-human",
    }
);

keyword_enum!(
    /// Who decided.
    Decider { Claude = "claude", Human = "human" }
);

keyword_enum!(
    /// The rule a row was decided under. `modules`: only ASN.1 in extracted modules counted, so a fix to inline ASN.1
    /// was `not-applicable`. `asn1`: ASN.1 in modules and inline counts. `modules` rows of RFCs with inline ASN.1 are
    /// offered for triage again.
    Rule { Modules = "modules", Asn1 = "asn1" }
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub rfc: u32,
    pub errata_id: u32,
    pub decision: Decision,
    pub by: Decider,
    /// [`crate::fingerprint`] of the erratum when it was decided.
    pub fingerprint: String,
    pub rule: Rule,
    pub note: String,
}

impl Row {
    pub fn key(&self) -> (u32, u32) {
        (self.rfc, self.errata_id)
    }
}

/// Why `note` is not acceptable for `decision`, or `None`. Names are screened apart ([`screen_note`]): they come from
/// the feed.
pub fn note_problem(note: &str, decision: Decision) -> Option<String> {
    if note.chars().count() > MAX_NOTE_CHARS {
        return Some(format!("note longer than {MAX_NOTE_CHARS} characters"));
    }
    if note.chars().any(|c| c == '\t' || line_forbidden(c)) {
        return Some("note holds a tab, newline, control, bidi or zero-width character".to_owned());
    }
    let patterns = PATTERNS.as_ref()?;
    if patterns.email.is_match(note) {
        return Some("note holds an email address".to_owned());
    }
    if decision == Decision::Superseded && !patterns.superseded.is_match(note) {
        return Some("a superseded note must name the later RFC (\"RFC NNNN\")".to_owned());
    }
    if matches!(decision, Decision::Superseded | Decision::NeedsHuman) && note.trim().is_empty() {
        return Some(format!("a {decision} decision needs a note"));
    }
    None
}

/// Why `note` must not be recorded for `e`: it holds the name of the erratum's submitter or verifier.
pub fn screen_note(note: &str, e: &Erratum) -> Option<String> {
    let folded = note.to_lowercase();
    e.names
        .iter()
        .any(|name| name.chars().count() >= 3 && folded.contains(&name.to_lowercase()))
        .then(|| "note holds the name of the erratum's submitter or verifier".to_owned())
}

/// The RFC a `superseded` note names.
pub fn superseding_rfc(note: &str) -> Option<u32> {
    let captures = PATTERNS.as_ref()?.superseded.captures(note)?;
    ascii_number(captures.get(1)?.as_str())
}

fn plain_number(field: &str) -> Option<u32> {
    let n = ascii_number(field)?;
    (n.to_string() == field).then_some(n)
}

fn parse_row(line: &str, lineno: usize) -> Result<Row, LedgerError> {
    let error = |what: String| LedgerError(format!("line {lineno}: {what}"));
    let fields: Vec<&str> = line.split('\t').collect();
    let [rfc, errata_id, decision, by, fingerprint, rule, note] = fields.as_slice() else {
        return Err(error(format!(
            "{} fields, expected {COLUMNS}",
            fields.len()
        )));
    };
    let (Some(rfc), Some(errata_id)) = (plain_number(rfc), plain_number(errata_id)) else {
        return Err(error("rfc and errata_id must be plain numbers".to_owned()));
    };
    let decision: Decision = decision
        .parse()
        .map_err(|()| error(format!("unknown decision {decision:?}")))?;
    let by: Decider = by
        .parse()
        .map_err(|()| error("by must be claude or human".to_owned()))?;
    if fingerprint.len() != 16
        || !fingerprint
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(error("fingerprint must be 16 hex digits".to_owned()));
    }
    let rule: Rule = rule
        .parse()
        .map_err(|()| error("rule must be modules or asn1".to_owned()))?;
    if let Some(problem) = note_problem(note, decision) {
        return Err(error(problem));
    }
    Ok(Row {
        rfc,
        errata_id,
        decision,
        by,
        fingerprint: (*fingerprint).to_owned(),
        rule,
        note: (*note).to_owned(),
    })
}

/// Parses and validates ledger text.
pub fn parse_ledger(text: &str) -> Result<Vec<Row>, LedgerError> {
    let Some(body) = text.strip_suffix('\n') else {
        return Err(LedgerError(
            "the ledger does not end with a newline".to_owned(),
        ));
    };
    let mut lines = body.split('\n');
    if lines.next() != Some(LEDGER_HEADER) {
        return Err(LedgerError(format!(
            "line 1: the header must be {LEDGER_HEADER:?}"
        )));
    }
    let mut rows: Vec<Row> = Vec::new();
    for (lineno, line) in lines.enumerate().map(|(i, l)| (i.saturating_add(2), l)) {
        let row = parse_row(line, lineno)?;
        if let Some(last) = rows.last()
            && row.key() <= last.key()
        {
            let what = if row.key() == last.key() {
                "duplicate"
            } else {
                "out of order"
            };
            return Err(LedgerError(format!(
                "line {lineno}: ({}, {}) is {what}; rows must be sorted by (rfc, errata_id) and unique",
                row.rfc, row.errata_id
            )));
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Ledger text for `rows`, sorted. Refuses duplicates and notes [`note_problem`] refuses.
pub fn format_ledger(rows: &[Row]) -> Result<String, LedgerError> {
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by_key(|r| r.key());
    if sorted
        .windows(2)
        .any(|w| matches!(w, [a, b] if a.key() == b.key()))
    {
        return Err(LedgerError("duplicate (rfc, errata_id) rows".to_owned()));
    }
    let mut out = format!("{LEDGER_HEADER}\n");
    for r in sorted {
        if let Some(problem) = note_problem(&r.note, r.decision) {
            return Err(LedgerError(format!(
                "({}, {}): {problem}",
                r.rfc, r.errata_id
            )));
        }
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            r.rfc, r.errata_id, r.decision, r.by, r.fingerprint, r.rule, r.note
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(rfc: u32, id: u32, decision: Decision, note: &str) -> Row {
        Row {
            rfc,
            errata_id: id,
            decision,
            by: Decider::Claude,
            fingerprint: "0123456789abcdef".into(),
            rule: Rule::Asn1,
            note: note.into(),
        }
    }

    #[test]
    fn round_trip() {
        let rows = vec![
            row(5, 2, Decision::Superseded, "RFC 9999 replaces it"),
            row(5, 1, Decision::Patched, "x"),
        ];
        let text = format_ledger(&rows).unwrap_or_default();
        let parsed = parse_ledger(&text).unwrap_or_default();
        assert_eq!(
            parsed.iter().map(Row::key).collect::<Vec<_>>(),
            [(5, 1), (5, 2)]
        );
        assert_eq!(format_ledger(&parsed).unwrap_or_default(), text);
    }

    #[test]
    fn malformed_ledgers_name_the_line() {
        let header = format!("{LEDGER_HEADER}\n");
        let cases = [
            ("", "does not end with a newline"),
            ("rfc\terrata_id\n", "line 1: the header"),
            (
                &*format!("{header}5\t1\tpatched\tclaude\t0123456789abcdef\tasn1\n"),
                "line 2: 6 fields",
            ),
            (
                &*format!("{header}05\t1\tpatched\tclaude\t0123456789abcdef\tasn1\tx\n"),
                "plain numbers",
            ),
            (
                &*format!("{header}5\t1\tmaybe\tclaude\t0123456789abcdef\tasn1\tx\n"),
                "unknown decision",
            ),
            (
                &*format!("{header}5\t1\tpatched\tclaude\t0123456789ABCDEF\tasn1\tx\n"),
                "16 hex digits",
            ),
            (
                &*format!("{header}5\t1\tpatched\tclaude\t0123456789abcdef\tall\tx\n"),
                "rule must be",
            ),
            (
                &*format!(
                    "{header}5\t2\tpatched\tclaude\t0123456789abcdef\tasn1\tx\n5\t1\tpatched\tclaude\t0123456789abcdef\tasn1\tx\n"
                ),
                "line 3: (5, 1) is out of order",
            ),
        ];
        for (text, message) in cases {
            let err = parse_ledger(text).err().map(|e| e.0).unwrap_or_default();
            assert!(err.contains(message), "{text:?}: {err}");
        }
    }

    #[test]
    fn notes_are_screened() {
        assert!(note_problem(&"x".repeat(501), Decision::Patched).is_some());
        assert!(note_problem("a\u{202e}b", Decision::Patched).is_some());
        assert!(note_problem("ask jane.doe@example.com", Decision::Patched).is_some());
        assert_eq!(
            note_problem(
                "constraint {@privateKeyAlgorithm.algorithm}",
                Decision::Patched
            ),
            None
        );
        assert!(note_problem("later RFC", Decision::Superseded).is_some());
        assert_eq!(
            note_problem("RFC 5912 replaces it", Decision::Superseded),
            None
        );
        assert!(note_problem(" ", Decision::NeedsHuman).is_some());
        assert_eq!(superseding_rfc("see RFC5912"), Some(5912));
    }

    #[test]
    fn submitter_names_are_kept_out_of_notes() {
        let e = Erratum {
            errata_id: 1,
            rfc: 1,
            status: "Verified".into(),
            section: String::new(),
            orig_text: String::new(),
            correct_text: String::new(),
            notes: String::new(),
            names: vec!["Sam Submitter".into(), "Al".into()],
        };
        assert!(screen_note("as SAM SUBMITTER said", &e).is_some());
        assert_eq!(screen_note("also see Algorithm", &e), None);
    }
}
