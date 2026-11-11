//! Per-erratum patches to an RFC's source: a single-file unified diff, applied without line numbers.
//!
//! GNU `patch` places a hunk by its line number, searching outward and dropping context until something matches. RFCs
//! repeat whole definitions (a copy in the body, the module in an appendix), so when line numbers drift (a lower
//! erratum backed out, the RFC Editor changing the file) it can edit the wrong copy and report success. Here each
//! hunk's old side (context and removed lines) must occur exactly once in the text, without fuzz, and the hunks must be
//! in order and not overlap; anything else is an error.

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use similar::{Algorithm, DiffTag, capture_diff_slices, group_diff_ops};

use crate::chars::line_forbidden;
use crate::feed::ascii_number;

/// The largest patch accepted.
pub const MAX_PATCH_BYTES: usize = 1024 * 1024;
/// The most hunks one patch may have.
pub const MAX_HUNKS: usize = 200;
const NO_NEWLINE: &str = "\\ No newline at end of file";
const INITIAL_CONTEXT: usize = 3;

static HUNK_HEADER: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^@@ -([0-9]+)(?:,([0-9]+))? \+([0-9]+)(?:,([0-9]+))? @@$").ok());

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct PatchError(pub String);

fn error<T>(message: impl Into<String>) -> Result<T, PatchError> {
    Err(PatchError(message.into()))
}

/// What Glint extracts an RFC from, and so what its patches apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceFormat {
    /// `rfcN.xml`, for RFCs published in RFCXML v3 (RFC 8650 onward).
    Xml,
    /// `rfcN.txt`, as published (page headers and footers included).
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Source {
    pub rfc: u32,
    pub format: SourceFormat,
}

impl Source {
    /// `rfc5280.txt`, `rfc9881.xml`: the name a patch's headers carry.
    pub fn file_name(&self) -> String {
        let extension = match self.format {
            SourceFormat::Xml => "xml",
            SourceFormat::Text => "txt",
        };
        format!("rfc{}.{extension}", self.rfc)
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.file_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The lines, with their line ends, that the hunk replaces: context and removed lines.
    pub old: Vec<String>,
    /// The lines it puts in their place.
    pub new: Vec<String>,
    /// 1-based, for messages.
    pub number: usize,
}

/// The lines of `text` with their `\n`. Only `\n` ends a line (`str::lines` would also drop a `\r`).
fn split_lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn hunk_lines(out: &mut String, prefix: char, lines: &[&str]) {
    for line in lines {
        out.push(prefix);
        out.push_str(line);
        if !line.ends_with('\n') {
            out.push('\n');
            out.push_str(NO_NEWLINE);
            out.push('\n');
        }
    }
}

fn format_patch(source: Source, a: &[&str], b: &[&str], context: usize) -> String {
    let name = source.file_name();
    let mut out = format!("--- a/{name}\n+++ b/{name}\n");
    let ops = capture_diff_slices(Algorithm::Myers, a, b);
    for group in group_diff_ops(ops, context) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let (old, new) = (
            first.old_range().start..last.old_range().end,
            first.new_range().start..last.new_range().end,
        );
        // A unified diff numbers lines from 1, and an empty range by the line before it.
        let start = |r: &std::ops::Range<usize>| {
            if r.is_empty() {
                r.start
            } else {
                r.start.saturating_add(1)
            }
        };
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            start(&old),
            old.len(),
            start(&new),
            new.len()
        ));
        for op in &group {
            let (tag, old_range, new_range) = op.as_tag_tuple();
            let old_lines = a.get(old_range).unwrap_or_default();
            if tag == DiffTag::Equal {
                hunk_lines(&mut out, ' ', old_lines);
            } else {
                hunk_lines(&mut out, '-', old_lines);
                hunk_lines(&mut out, '+', b.get(new_range).unwrap_or_default());
            }
        }
    }
    out
}

/// A patch from `base` to `edited` that [`apply_patch`] accepts. The context starts at 3 lines and widens until every
/// hunk's old side occurs exactly once in `base`, checked by applying the patch.
pub fn make_patch(source: Source, base: &str, edited: &str) -> Result<String, PatchError> {
    if base == edited {
        return error("nothing changed");
    }
    let (a, b) = (split_lines(base), split_lines(edited));
    let widest = a.len().max(b.len());
    let mut context = INITIAL_CONTEXT;
    loop {
        let patch = format_patch(source, &a, &b, context);
        if apply_patch(base, &patch, source).is_ok_and(|text| text == edited) {
            return Ok(patch);
        }
        if context >= widest {
            return error("no context width gives a patch that applies uniquely");
        }
        context = context.saturating_mul(2).min(widest);
    }
}

/// Parses a single-file unified diff of `source`, whose headers must be exactly `--- a/<file>` and `+++ b/<file>`.
pub fn parse_patch(patch: &str, source: Source) -> Result<Vec<Hunk>, PatchError> {
    if patch.len() > MAX_PATCH_BYTES {
        return error(format!("patch larger than {MAX_PATCH_BYTES} bytes"));
    }
    let Some(body) = patch.strip_suffix('\n') else {
        return error("patch does not end with a newline");
    };
    let lines: Vec<&str> = body.split('\n').collect();
    let name = source.file_name();
    let (old_header, new_header) = (format!("--- a/{name}"), format!("+++ b/{name}"));
    match lines.as_slice() {
        [first, second, rest @ ..] if *first == old_header && *second == new_header => {
            parse_hunks(rest)
        }
        _ => error(format!(
            "patch must start with \"{old_header}\" and \"{new_header}\" (no other headers)"
        )),
    }
}

fn count(captures: &regex::Captures<'_>, group: usize) -> Option<usize> {
    match captures.get(group) {
        None => Some(1),
        Some(m) => ascii_number(m.as_str()).and_then(|n| usize::try_from(n).ok()),
    }
}

/// The hunks of a diff's lines after its two headers. Line numbers in messages count the headers.
fn parse_hunks(lines: &[&str]) -> Result<Vec<Hunk>, PatchError> {
    let Some(header_re) = HUNK_HEADER.as_ref() else {
        return error("internal: hunk header pattern");
    };
    let lineno = |i: usize| i.saturating_add(3);
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut i = 0usize;
    while let Some(line) = lines.get(i) {
        let header = header_re.captures(line);
        let Some((old_count, new_count)) = header
            .as_ref()
            .and_then(|c| Some((count(c, 2)?, count(c, 4)?)))
        else {
            let shown: String = line.chars().take(60).collect();
            return error(format!(
                "line {}: expected a hunk header, got {shown:?}",
                lineno(i)
            ));
        };
        if hunks.len() >= MAX_HUNKS {
            return error(format!("more than {MAX_HUNKS} hunks"));
        }
        let number = hunks.len().saturating_add(1);
        let mut hunk = Hunk {
            old: Vec::new(),
            new: Vec::new(),
            number,
        };
        i = i.saturating_add(1);
        while hunk.old.len() < old_count || hunk.new.len() < new_count {
            let Some(line) = lines.get(i) else { break };
            if *line == NO_NEWLINE {
                return error(format!("line {}: misplaced \"{NO_NEWLINE}\"", lineno(i)));
            }
            let mut chars = line.chars();
            let marker = chars.next();
            let body = chars.as_str();
            if !matches!(marker, Some(' ' | '-' | '+')) {
                return error(format!(
                    "line {}: hunk {number} line does not start with \" \", \"-\" or \"+\"",
                    lineno(i)
                ));
            }
            let ends_line = lines.get(i.saturating_add(1)) != Some(&NO_NEWLINE);
            if !ends_line {
                i = i.saturating_add(1);
            }
            let text = if ends_line {
                format!("{body}\n")
            } else {
                body.to_owned()
            };
            if matches!(marker, Some(' ' | '-')) {
                hunk.old.push(text.clone());
            }
            if matches!(marker, Some(' ' | '+')) {
                hunk.new.push(text);
            }
            i = i.saturating_add(1);
        }
        if hunk.old.len() != old_count || hunk.new.len() != new_count {
            return error(format!(
                "hunk {number}: line counts do not match its header"
            ));
        }
        hunks.push(hunk);
    }
    if hunks.is_empty() {
        return error("patch has no hunks");
    }
    Ok(hunks)
}

/// Where `needle` occurs in `lines`, as start indices.
fn occurrences(lines: &[&str], needle: &[String]) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    lines
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| window.iter().zip(needle).all(|(a, b)| *a == b))
        .map(|(i, _)| i)
        .collect()
}

/// Applies `patch` to `text`; see the module documentation for the rules. The error names the hunk that fails.
pub fn apply_patch(text: &str, patch: &str, source: Source) -> Result<String, PatchError> {
    apply_hunks(text, &parse_patch(patch, source)?)
}

fn apply_hunks(text: &str, hunks: &[Hunk]) -> Result<String, PatchError> {
    let lines = split_lines(text);
    let mut spans: Vec<(usize, usize, &Hunk)> = Vec::new();
    for hunk in hunks {
        if hunk.old.is_empty() {
            if !lines.is_empty() {
                return error(format!("hunk {}: no context to place it by", hunk.number));
            }
            spans.push((0, 0, hunk));
            continue;
        }
        let found = occurrences(&lines, &hunk.old);
        match found.as_slice() {
            [] => {
                return error(format!(
                    "hunk {}: its context and removed lines are not in the text",
                    hunk.number
                ));
            }
            [start] => spans.push((*start, start.saturating_add(hunk.old.len()), hunk)),
            many => {
                let shown: Vec<String> = many
                    .iter()
                    .take(5)
                    .map(|f| f.saturating_add(1).to_string())
                    .collect();
                return error(format!(
                    "hunk {}: its context occurs {} times (lines {})",
                    hunk.number,
                    many.len(),
                    shown.join(", ")
                ));
            }
        }
    }
    for pair in spans.windows(2) {
        if let [(_, previous_end, previous), (start, _, hunk)] = pair
            && start < previous_end
        {
            return error(format!(
                "hunk {} overlaps or precedes hunk {}",
                hunk.number, previous.number
            ));
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut position = 0usize;
    for (start, end, hunk) in &spans {
        out.extend(
            lines
                .get(position..*start)
                .unwrap_or_default()
                .iter()
                .copied(),
        );
        out.extend(hunk.new.iter().map(String::as_str));
        position = *end;
    }
    out.extend(lines.get(position..).unwrap_or_default().iter().copied());
    Ok(out)
}

/// The 1-based line of `text` where each hunk of `patch` first changes it: its first removed line, or for a pure
/// insertion the line it is inserted before. `patch` must apply to `text`.
pub fn changed_lines(patch: &str, source: Source, text: &str) -> Result<Vec<usize>, PatchError> {
    let lines = split_lines(text);
    parse_patch(patch, source)?
        .iter()
        .map(|hunk| {
            let start = if hunk.old.is_empty() {
                0
            } else {
                *occurrences(&lines, &hunk.old)
                    .first()
                    .ok_or_else(|| PatchError(format!("hunk {} does not apply", hunk.number)))?
            };
            let same = hunk
                .old
                .iter()
                .zip(&hunk.new)
                .take_while(|(a, b)| a == b)
                .count();
            Ok(start.saturating_add(same).saturating_add(1))
        })
        .collect()
}

/// The erratum whose patch stopped a series, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesFailure {
    pub errata_id: u32,
    pub reason: String,
}

fn sorted(patches: &[(u32, String)]) -> Vec<&(u32, String)> {
    let mut sorted: Vec<&(u32, String)> = patches.iter().collect();
    sorted.sort_by_key(|(id, _)| *id);
    sorted
}

/// Applies `(errata_id, patch)` pairs in errata-id order, all or nothing.
pub fn apply_series(
    base: &str,
    patches: &[(u32, String)],
    source: Source,
) -> Result<String, SeriesFailure> {
    let mut text = base.to_owned();
    for (errata_id, patch) in sorted(patches) {
        text = apply_patch(&text, patch, source).map_err(|e| SeriesFailure {
            errata_id: *errata_id,
            reason: e.0,
        })?;
    }
    Ok(text)
}

/// The series after one erratum's patch is added, replaced or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseOutcome {
    /// The new series, by errata id. Patches above the change are re-made against their new base; from a failure on,
    /// they are kept as they were.
    pub patches: Vec<(u32, String)>,
    /// The first higher patch that no longer applies; it and every later one need a person.
    pub failure: Option<SeriesFailure>,
}

/// Adds, replaces (`replacement` is `Some`) or removes (`None`) the patch of `errata_id` in an RFC's series and rebases
/// every higher patch onto its new base with a three-way merge ([`merge`]): its change is carried onto the new base
/// when the lines it changes are untouched there, and the patch is made again. `replacement` is a patch against
/// `base` plus the patches below `errata_id`. An error means the series below `errata_id`, the old series up to
/// `errata_id`, or the replacement, does not apply.
pub fn rebase(
    base: &str,
    series: &[(u32, String)],
    errata_id: u32,
    replacement: Option<&str>,
    source: Source,
) -> Result<RebaseOutcome, SeriesFailure> {
    let ordered = sorted(series);
    let below: Vec<(u32, String)> = ordered
        .iter()
        .filter(|(id, _)| *id < errata_id)
        .map(|item| (*item).clone())
        .collect();
    let above: Vec<&(u32, String)> = ordered
        .iter()
        .filter(|(id, _)| *id > errata_id)
        .copied()
        .collect();
    let below_text = apply_series(base, &below, source)?;
    let failed = |id: u32| {
        move |e: PatchError| SeriesFailure {
            errata_id: id,
            reason: e.0,
        }
    };
    // The base of the first higher patch before the change, and after it.
    let mut old_text = match ordered.iter().find(|(id, _)| *id == errata_id) {
        Some((_, old_patch)) => {
            apply_patch(&below_text, old_patch, source).map_err(failed(errata_id))?
        }
        None => below_text.clone(),
    };
    let mut text = match replacement {
        Some(patch) => apply_patch(&below_text, patch, source).map_err(failed(errata_id))?,
        None => below_text,
    };
    let mut patches = below;
    if let Some(patch) = replacement {
        patches.push((errata_id, patch.to_owned()));
    }
    let mut failure = None;
    for (id, patch) in above {
        if failure.is_some() {
            patches.push((*id, patch.clone()));
            continue;
        }
        let rebased = apply_patch(&old_text, patch, source).and_then(|old_next| {
            let next = merge(&old_text, &text, &old_next)?;
            let new_patch = if next == text {
                patch.clone()
            } else {
                make_patch(source, &text, &next)?
            };
            Ok((new_patch, old_next, next))
        });
        match rebased {
            Ok((new_patch, old_next, next)) => {
                patches.push((*id, new_patch));
                old_text = old_next;
                text = next;
            }
            Err(e) => {
                failure = Some(SeriesFailure {
                    errata_id: *id,
                    reason: e.0,
                });
                patches.push((*id, patch.clone()));
            }
        }
    }
    Ok(RebaseOutcome { patches, failure })
}

/// Where each line of `old` is in `new`, for the lines a diff leaves unchanged: `(old start, old end, new start)` of
/// every run of equal lines.
fn equal_runs(old: &[&str], new: &[&str]) -> Vec<(usize, usize, usize)> {
    capture_diff_slices(Algorithm::Myers, old, new)
        .iter()
        .filter_map(|op| {
            let (tag, o, n) = op.as_tag_tuple();
            (tag == DiffTag::Equal).then_some((o.start, o.end, n.start))
        })
        .collect()
}

/// Three-way merge: `ours` is `base` with one erratum's change, `theirs` is `base` with another's. Returns `theirs`
/// with our change too, when every line our change replaces, and the place of every line it inserts, is unchanged
/// in `theirs`; otherwise the changes conflict and a person must decide.
fn merge(base: &str, theirs: &str, ours: &str) -> Result<String, PatchError> {
    let (b, t, o) = (split_lines(base), split_lines(theirs), split_lines(ours));
    let runs = equal_runs(&b, &t);
    // The line of `theirs` holding line `i` of `base`, when it is unchanged and so is the line before it (`strict`),
    // which keeps a range of base lines contiguous in `theirs`.
    let position = |i: usize| -> Option<usize> {
        runs.iter()
            .find(|(start, end, _)| (*start..*end).contains(&i))
            .map(|(start, _, new_start)| new_start.saturating_add(i.saturating_sub(*start)))
    };
    let same_run = |a: usize, z: usize| runs.iter().any(|(start, end, _)| *start <= a && z < *end);
    let mut edits: Vec<(usize, usize, Vec<&str>)> = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &b, &o) {
        let (tag, old, new) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        let lines: Vec<&str> = o.get(new).unwrap_or_default().to_vec();
        let conflict =
            || PatchError("it changes lines another erratum of the RFC changes too".to_owned());
        let (start, end) = if old.is_empty() {
            // An insertion before base line `old.start`: its neighbours must be unchanged and still neighbours.
            let at = old.start;
            match (at.checked_sub(1), at < b.len()) {
                (None, true) => (position(0).filter(|p| *p == 0).ok_or_else(conflict)?, 0),
                (None, false) => (0, 0),
                (Some(previous), false) => {
                    let p = position(previous)
                        .filter(|p| p.saturating_add(1) == t.len())
                        .ok_or_else(conflict)?;
                    (p.saturating_add(1), p.saturating_add(1))
                }
                (Some(previous), true) if same_run(previous, at) => {
                    let p = position(at).ok_or_else(conflict)?;
                    (p, p)
                }
                (Some(_), true) => return Err(conflict()),
            }
        } else {
            let last = old.end.saturating_sub(1);
            if !same_run(old.start, last) {
                return Err(conflict());
            }
            let (Some(first), Some(end)) = (position(old.start), position(last)) else {
                return Err(conflict());
            };
            (first, end.saturating_add(1))
        };
        edits.push((start, end, lines));
    }
    let mut out = String::with_capacity(theirs.len());
    let mut at = 0usize;
    for (start, end, lines) in &edits {
        if *start < at {
            return error("changes overlap after the merge");
        }
        out.extend(t.get(at..*start).unwrap_or_default().iter().copied());
        out.extend(lines.iter().copied());
        at = *end;
    }
    out.extend(t.get(at..).unwrap_or_default().iter().copied());
    Ok(out)
}

/// Why an added line of `patch` is unacceptable, or `None`. Every `+` line after the two headers is an added line, one
/// whose text starts with `++ ` too.
pub fn forbidden_in_patch(patch: &str) -> Option<String> {
    patch.split('\n').enumerate().skip(2).find_map(|(i, line)| {
        let added = line.strip_prefix('+')?;
        added.chars().any(line_forbidden).then(|| {
            format!(
                "line {} adds a control, bidi or zero-width character",
                i.saturating_add(1)
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: Source = Source {
        rfc: 100,
        format: SourceFormat::Text,
    };

    fn make(base: &str, edited: &str) -> String {
        make_patch(SOURCE, base, edited).unwrap_or_default()
    }

    fn apply(text: &str, patch: &str) -> Result<String, String> {
        apply_patch(text, patch, SOURCE).map_err(|e| e.0)
    }

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn a_patch_round_trips() {
        let base = numbered(20);
        let edited = base
            .replace("line 7\n", "line seven\n")
            .replace("line 15\n", "");
        let patch = make(&base, &edited);
        assert!(patch.starts_with("--- a/rfc100.txt\n+++ b/rfc100.txt\n@@ -4,"));
        assert_eq!(apply(&base, &patch), Ok(edited));
    }

    #[test]
    fn context_widens_until_each_hunk_is_unique() {
        let block = "A ::= SEQUENCE {\n  a INTEGER\n}\n";
        let base = format!("Body:\n{block}\nprose\nmore prose\nAppendix:\n{block}END\n");
        let edited = base
            .replacen("  a INTEGER", "  a INTEGER (0..9)", 2)
            .replacen("  a INTEGER (0..9)", "  a INTEGER", 1);
        let patch = make(&base, &edited);
        assert_eq!(apply(&base, &patch), Ok(edited.clone()));
        let narrow = format_patch(SOURCE, &split_lines(&base), &split_lines(&edited), 1);
        assert!(apply(&base, &narrow).is_err_and(|e| e.contains("occurs 2 times")));
    }

    #[test]
    fn a_patch_ignores_line_numbers() {
        let base = numbered(10);
        let patch = make(&base, &base.replace("line 5\n", "five\n"));
        let shifted = format!("new first line\n{base}");
        assert_eq!(
            apply(&shifted, &patch),
            Ok(shifted.replace("line 5\n", "five\n"))
        );
    }

    #[test]
    fn missing_end_of_line_is_kept() {
        let base = "a\nb\nc";
        let edited = "a\nb\nC";
        let patch = make(base, edited);
        assert!(patch.contains("\\ No newline at end of file\n"));
        assert_eq!(apply(base, &patch), Ok(edited.to_owned()));
    }

    #[test]
    fn carriage_returns_belong_to_the_line() {
        let base = "a\r\nb\r\nc\r\n";
        let edited = "a\r\nB\r\nc\r\n";
        assert_eq!(apply(base, &make(base, edited)), Ok(edited.to_owned()));
    }

    #[test]
    fn malformed_patches_are_refused() {
        let base = numbered(5);
        let patch = make(&base, &base.replace("line 3", "three"));
        let cases = [
            (patch.replace("rfc100.txt", "rfc101.txt"), "must start with"),
            (
                patch.replace("--- a/rfc100.txt\n", "diff --git a b\n--- a/rfc100.txt\n"),
                "must start with",
            ),
            (patch.trim_end().to_owned(), "does not end with a newline"),
            (
                patch.replace("@@ -1,5 +1,5 @@", "@@ -1,6 +1,5 @@"),
                "line counts",
            ),
            (patch.replace(" line 2", "?line 2"), "does not start with"),
            (
                "--- a/rfc100.txt\n+++ b/rfc100.txt\n".to_owned(),
                "no hunks",
            ),
            (
                patch.replace("@@ -1,5 +1,5 @@", "@@ garbage @@"),
                "expected a hunk header",
            ),
        ];
        for (bad, message) in cases {
            assert!(
                apply(&base, &bad).is_err_and(|e| e.contains(message)),
                "{message}: {bad}"
            );
        }
        assert!(
            apply(&base, &"x".repeat(MAX_PATCH_BYTES + 1))
                .is_err_and(|e| e.contains("larger than"))
        );
        assert!(apply("other\n", &patch).is_err_and(|e| e.contains("not in the text")));
    }

    #[test]
    fn hunks_must_be_in_order() {
        let base = numbered(30);
        let patch = make(
            &base,
            &base.replace("line 3\n", "3\n").replace("line 25\n", "25\n"),
        );
        let mut parts: Vec<&str> = patch.split("@@ -").collect();
        parts.swap(1, 2);
        let swapped = parts.join("@@ -");
        assert!(apply(&base, &swapped).is_err_and(|e| e.contains("overlaps or precedes")));
    }

    #[test]
    fn insertions_at_the_start_and_the_end() {
        let base = numbered(8);
        let edited = format!("first\n{base}last\n");
        assert_eq!(apply(&base, &make(&base, &edited)), Ok(edited.clone()));
        assert_eq!(apply("", &make("", "only\n")), Ok("only\n".to_owned()));
    }

    #[test]
    fn too_many_hunks_are_refused() {
        let base = numbered(2000);
        let mut edited = base.clone();
        for i in (1..=2000).step_by(9) {
            edited = edited.replace(&format!("line {i}\n"), &format!("LINE {i}\n"));
        }
        let patch = format_patch(SOURCE, &split_lines(&base), &split_lines(&edited), 1);
        assert!(apply(&base, &patch).is_err_and(|e| e.contains("more than 200 hunks")));
    }

    #[test]
    fn identical_texts_give_no_patch() {
        assert!(make_patch(SOURCE, "a\n", "a\n").is_err());
    }

    #[test]
    fn changed_lines_skip_leading_context() {
        let base = numbered(10);
        let patch = make(&base, &base.replace("line 6\n", "six\n"));
        assert_eq!(changed_lines(&patch, SOURCE, &base), Ok(vec![6]));
    }

    #[test]
    fn series_apply_in_id_order_or_not_at_all() {
        let base = numbered(10);
        let first = base.replace("line 2\n", "two\n");
        let p1 = make(&base, &first);
        let p2 = make(&first, &first.replace("line 8\n", "eight\n"));
        let both = vec![(9, p2.clone()), (4, p1.clone())];
        assert_eq!(
            apply_series(&base, &both, SOURCE),
            Ok(first.replace("line 8\n", "eight\n"))
        );
        let broken = vec![(4, p1), (9, p2.replace("line 7", "line 77"))];
        assert!(apply_series(&base, &broken, SOURCE).is_err_and(|f| f.errata_id == 9));
    }

    #[test]
    fn backing_out_a_patch_rebases_the_later_ones() {
        let base = numbered(10);
        let t1 = base.replace("line 1\n", "one\n");
        let t2 = t1.replace("line 5\n", "five\n");
        let t3 = t2.replace("line 9\n", "nine\n");
        let series = vec![
            (1, make(&base, &t1)),
            (2, make(&t1, &t2)),
            (3, make(&t2, &t3)),
        ];
        let outcome = rebase(&base, &series, 2, None, SOURCE).unwrap_or_else(|_| RebaseOutcome {
            patches: vec![],
            failure: None,
        });
        assert_eq!(outcome.failure, None);
        assert_eq!(
            outcome
                .patches
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(
            apply_series(&base, &outcome.patches, SOURCE),
            Ok(t3.replace("five\n", "line 5\n"))
        );
    }

    #[test]
    fn a_later_patch_whose_context_a_lower_one_changes_is_merged() {
        let base = numbered(10);
        let t2 = base.replace("line 6\n", "six\n");
        let series = vec![(5, make(&base, &t2))];
        let lower = make(&base, &base.replace("line 5\n", "five\n"));
        let outcome = rebase(&base, &series, 3, Some(&lower), SOURCE).ok();
        assert_eq!(outcome.as_ref().and_then(|o| o.failure.clone()), None);
        let patches = outcome.map(|o| o.patches).unwrap_or_default();
        assert_eq!(
            apply_series(&base, &patches, SOURCE),
            Ok(base
                .replace("line 5\n", "five\n")
                .replace("line 6\n", "six\n"))
        );
        let backed_out = rebase(&base, &patches, 3, None, SOURCE).ok();
        let patches = backed_out.map(|o| o.patches).unwrap_or_default();
        assert_eq!(apply_series(&base, &patches, SOURCE), Ok(t2));
    }

    #[test]
    fn inserted_lines_follow_their_neighbours() {
        let base = numbered(10);
        let t2 = base.replace("line 6\n", "line 6\nnew line\n");
        let series = vec![(5, make(&base, &t2))];
        let lower = make(&base, &base.replace("line 1\n", "one\nzero\n"));
        let patches = rebase(&base, &series, 3, Some(&lower), SOURCE)
            .map(|o| o.patches)
            .unwrap_or_default();
        assert_eq!(
            apply_series(&base, &patches, SOURCE),
            Ok(base
                .replace("line 1\n", "one\nzero\n")
                .replace("line 6\n", "line 6\nnew line\n"))
        );
    }

    #[test]
    fn two_errata_changing_one_line_conflict() {
        let base = numbered(10);
        let t2 = base.replace("line 5\n", "FIVE\n");
        let series = vec![(5, make(&base, &t2))];
        let lower = make(&base, &base.replace("line 5\n", "five\n"));
        let failure = rebase(&base, &series, 3, Some(&lower), SOURCE)
            .ok()
            .and_then(|o| o.failure);
        assert!(failure.is_some_and(
            |f| f.errata_id == 5 && f.reason.contains("changes lines another erratum")
        ));
    }

    #[test]
    fn a_later_patch_that_depended_on_the_backed_out_one_fails() {
        let base = numbered(10);
        let t1 = base.replace("line 5\n", "five\n");
        let t2 = t1.replace("five\n", "FIVE\n");
        let series = vec![(1, make(&base, &t1)), (2, make(&t1, &t2))];
        let outcome = rebase(&base, &series, 1, None, SOURCE).ok();
        let failure = outcome.and_then(|o| o.failure);
        assert_eq!(failure.map(|f| f.errata_id), Some(2));
    }

    #[test]
    fn a_new_lower_erratum_is_inserted_and_the_rest_rebased() {
        let base = numbered(10);
        let t2 = base.replace("line 8\n", "eight\n");
        let series = vec![(5, make(&base, &t2))];
        let new = make(&base, &base.replace("line 2\n", "two\n"));
        let outcome = rebase(&base, &series, 3, Some(&new), SOURCE).ok();
        let patches = outcome.map(|o| o.patches).unwrap_or_default();
        assert_eq!(
            patches.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [3, 5]
        );
        assert_eq!(
            apply_series(&base, &patches, SOURCE),
            Ok(base
                .replace("line 2\n", "two\n")
                .replace("line 8\n", "eight\n"))
        );
    }

    #[test]
    fn added_lines_are_screened() {
        let patch = "--- a/rfc1.txt\n+++ b/rfc1.txt\n@@ -1 +1 @@\n-a\n+b\u{202e}c\n";
        assert_eq!(
            forbidden_in_patch(patch).as_deref(),
            Some("line 5 adds a control, bidi or zero-width character")
        );
        assert_eq!(
            forbidden_in_patch("--- a\n+++ b\u{202e}\n@@ -1 +1 @@\n-a\n+b\n"),
            None
        );
    }
}
