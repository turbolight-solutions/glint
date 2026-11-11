//! Where an erratum's quoted text is in an RFC. A hint for triage only; patches never use it.

use std::sync::LazyLock;

use regex::Regex;

/// A quote shorter than this, whitespace removed, is not looked for.
const LOCATE_MIN: usize = 8;
/// A quoted piece shorter than this does not count for a partial match.
const PARTIAL_PIECE_MIN: usize = 12;
/// Matched lines further apart than this are separate regions.
const PARTIAL_GAP: usize = 40;

struct Patterns {
    elision: Regex,
    change_bar: Regex,
}

static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(|| {
    Some(Patterns {
        elision: Regex::new(r"\[\.\.\.\]|\.\.\.|\x{2026}").ok()?,
        change_bar: Regex::new(r"^\s*[|^]\s").ok()?,
    })
});

/// Where a quote is: 1-based first and last line, whether the whole quote matched, and for a partial match how many
/// of its distinctive pieces did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub first: usize,
    pub last: usize,
    pub exact: bool,
    pub matched: usize,
    pub total: usize,
}

fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Every place `needle` (an erratum's original or corrected text) occurs in `text`, in line order.
///
/// Exact: the whole quote, ignoring all whitespace, at every occurrence (RFCs repeat definitions). Failing that,
/// partial: errata quote with `...` elisions and `|` change bars, wrap the quote in prose, or change capitalisation,
/// so each piece of a quoted line between elisions, of at least 12 characters with whitespace removed and case
/// ignored, is looked for in the lines of `text`. Lines found within 40 of each other form a region, kept if it holds
/// at least a third of the pieces (and two, when there are two or more).
pub fn locate_all(text: &str, needle: &str) -> Vec<Match> {
    let target = squash(needle);
    if target.chars().count() < LOCATE_MIN {
        return Vec::new();
    }
    let squashed: Vec<String> = text.split('\n').map(squash).collect();
    let exact = exact_matches(&squashed, &target);
    if !exact.is_empty() {
        return exact;
    }
    partial_matches(&squashed, needle)
}

fn exact_matches(squashed: &[String], target: &str) -> Vec<Match> {
    let mut starts = Vec::with_capacity(squashed.len());
    let mut position = 0usize;
    for line in squashed {
        starts.push(position);
        position = position.saturating_add(line.len());
    }
    let joined = squashed.concat();
    // The 1-based line holding byte `offset` of `joined`; empty lines are skipped.
    let line_at = |offset: usize| starts.partition_point(|&s| s <= offset).max(1);
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(at) = joined.get(from..).and_then(|rest| rest.find(target)) {
        let start = from.saturating_add(at);
        let end = start.saturating_add(target.len());
        found.push(Match {
            first: line_at(start),
            last: line_at(end.saturating_sub(1)),
            exact: true,
            matched: 0,
            total: 0,
        });
        from = end;
    }
    found
}

fn partial_matches(squashed: &[String], needle: &str) -> Vec<Match> {
    let Some(patterns) = PATTERNS.as_ref() else {
        return Vec::new();
    };
    let mut keys: Vec<String> = Vec::new();
    for line in needle.split('\n') {
        let unbarred = patterns.change_bar.replace(line, " ");
        for piece in patterns.elision.split(&unbarred) {
            let key = squash(piece).to_lowercase();
            if key.chars().count() >= PARTIAL_PIECE_MIN && !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    if keys.is_empty() {
        return Vec::new();
    }
    let folded: Vec<String> = squashed.iter().map(|s| s.to_lowercase()).collect();
    let mut hits: Vec<(usize, usize)> = keys
        .iter()
        .enumerate()
        .flat_map(|(k, key)| {
            folded
                .iter()
                .enumerate()
                .filter(move |(_, line)| line.contains(key.as_str()))
                .map(move |(j, _)| (j, k))
        })
        .collect();
    hits.sort_unstable();
    let need = keys.len().min(2).max(keys.len().div_ceil(3));
    let mut regions: Vec<Vec<(usize, usize)>> = Vec::new();
    for hit in hits {
        match regions.last_mut() {
            Some(region)
                if region
                    .last()
                    .is_some_and(|(j, _)| hit.0.saturating_sub(*j) <= PARTIAL_GAP) =>
            {
                region.push(hit);
            }
            _ => regions.push(vec![hit]),
        }
    }
    regions
        .iter()
        .filter_map(|region| {
            let mut distinct: Vec<usize> = region.iter().map(|(_, k)| *k).collect();
            distinct.sort_unstable();
            distinct.dedup();
            let (first, last) = (region.first()?.0, region.last()?.0);
            (distinct.len() >= need).then_some(Match {
                first: first.saturating_add(1),
                last: last.saturating_add(1),
                exact: false,
                matched: distinct.len(),
                total: keys.len(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exact_occurrence_ignoring_whitespace() {
        let text =
            "intro\nFoo ::= SEQUENCE {\n  a INTEGER }\nprose\n\nFoo ::= SEQUENCE { a INTEGER }\n";
        let found = locate_all(text, "Foo ::= SEQUENCE {a INTEGER}");
        assert_eq!(
            found
                .iter()
                .map(|m| (m.first, m.last, m.exact))
                .collect::<Vec<_>>(),
            [(2, 3, true), (6, 6, true)]
        );
    }

    #[test]
    fn short_quotes_are_not_looked_for() {
        assert!(locate_all("a b c d\n", "a b").is_empty());
    }

    #[test]
    fn elided_quotes_match_partly() {
        let text = "x\nCertificateSerialNumber ::= INTEGER\nsome other line\nsignatureAlgorithm AlgorithmIdentifier,\n";
        let quote = "| CertificateSerialNumber ::= INTEGER\n...\nSIGNATUREALGORITHM AlgorithmIdentifier,\nnot in the text at all";
        let found = locate_all(text, quote);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found
                .first()
                .map(|m| (m.first, m.last, m.exact, m.matched, m.total)),
            Some((2, 4, false, 2, 3))
        );
    }

    #[test]
    fn distant_hits_are_separate_regions() {
        let filler = "prose\n".repeat(50);
        let text = format!(
            "CertificateSerialNumber ::= INTEGER\n{filler}signatureAlgorithm AlgorithmIdentifier\n"
        );
        let quote =
            "CertificateSerialNumber ::= INTEGER ... signatureAlgorithm AlgorithmIdentifier";
        assert!(locate_all(&text, quote).is_empty());
    }
}
