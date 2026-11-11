//! Which RFCs' errata are triaged, and which errata wait for a decision.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::LazyLock;

use glint_extract::is_inline_asn1;
use regex::Regex;

use crate::ledger::{Decision, Row, Rule};
use crate::{Feed, fingerprint};

struct Patterns {
    header: Regex,
    structure: Regex,
}

// A module header's OID block followed by DEFINITIONS, or a structured type, an OID value assignment or an
// IMPORTS ... FROM clause. Never `::=` alone: errata of BNF in other RFCs use it. The structure counts only in text the
// extractor would take for inline ASN.1 (`glint_extract::is_inline_asn1`): not a grammar, not SMI.
static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(|| {
    Some(Patterns {
        header: Regex::new(r"\}\s*DEFINITIONS\b").ok()?,
        structure: Regex::new(
            r"::=\s*(?:SEQUENCE|SET|CHOICE)\s*(?:\{|(?:SIZE\s*\([^)]*\)\s*)?OF\b)|\bOBJECT IDENTIFIER\s*::=\s*\{|\bIMPORTS\b[^;]*?\bFROM\s+[A-Z]",
        )
        .ok()?,
    })
});

fn asn1_structure(text: &str) -> bool {
    PATTERNS.as_ref().is_some_and(|p| {
        p.header.is_match(text) || (p.structure.is_match(text) && is_inline_asn1(text))
    })
}

/// Whether an erratum's original or corrected text quotes ASN.1 structure. It brings into scope RFCs whose ASN.1 the
/// corpus does not see as published, such as RFC 7508 and 7914, whose module names are capitalised wrongly. The notes
/// are prose and not looked at.
pub fn asn1_relevant(orig_text: &str, correct_text: &str) -> bool {
    asn1_structure(orig_text) || asn1_structure(correct_text)
}

/// The RFCs whose Verified errata are triaged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub rfcs: BTreeSet<u32>,
    /// The RFCs with inline ASN.1, whose rows decided under [`Rule::Modules`] are triaged again.
    pub with_inline_asn1: BTreeSet<u32>,
}

/// RFCs with modules or inline ASN.1 (`asn1_rfcs`), RFCs with a ledger row or a patch, and RFCs with a Verified erratum
/// [`asn1_relevant`] flags.
pub fn compute_scope(
    asn1_rfcs: &BTreeSet<u32>,
    inline_rfcs: &BTreeSet<u32>,
    rows: &[Row],
    patch_keys: &BTreeSet<(u32, u32)>,
    feed: Option<&Feed>,
) -> Scope {
    let mut rfcs: BTreeSet<u32> = asn1_rfcs.iter().chain(inline_rfcs).copied().collect();
    rfcs.extend(rows.iter().map(|r| r.rfc));
    rfcs.extend(patch_keys.iter().map(|(rfc, _)| *rfc));
    if let Some(feed) = feed {
        let flagged: Vec<u32> = feed
            .verified()
            .filter(|e| !rfcs.contains(&e.rfc) && asn1_relevant(&e.orig_text, &e.correct_text))
            .map(|e| e.rfc)
            .collect();
        rfcs.extend(flagged);
    }
    Scope {
        rfcs,
        with_inline_asn1: inline_rfcs.clone(),
    }
}

/// Why an erratum waits for triage, from the most to the least pressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QueueReason {
    /// A patched erratum that is no longer Verified, or no longer in the feed: back it out.
    Withdrawn,
    /// Its patch, or a lower one of the same RFC, does not apply.
    PatchFails,
    /// The erratum changed since it was decided.
    Changed,
    /// An in-scope Verified erratum without a ledger row.
    New,
    /// Decided when only modules counted, on an RFC with inline ASN.1.
    Rescope,
}

impl QueueReason {
    pub fn as_str(self) -> &'static str {
        match self {
            QueueReason::Withdrawn => "withdrawn",
            QueueReason::PatchFails => "patch-fails",
            QueueReason::Changed => "changed",
            QueueReason::New => "new",
            QueueReason::Rescope => "rescope",
        }
    }
}

impl fmt::Display for QueueReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub rfc: u32,
    pub errata_id: u32,
    pub reason: QueueReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queue {
    /// By (rfc, errata_id).
    pub items: Vec<QueueItem>,
    /// `needs-human` rows whose erratum did not change: listed apart, not triaged again.
    pub needs_human: Vec<Row>,
}

/// The triage queue. `failing` holds the (rfc, errata_id) of patches that do not apply ([`failing_keys`]).
pub fn compute_queue(
    feed: &Feed,
    rows: &[Row],
    scope: &Scope,
    failing: &BTreeSet<(u32, u32)>,
) -> Queue {
    let by_key: BTreeMap<(u32, u32), &Row> = rows.iter().map(|r| (r.key(), r)).collect();
    let mut reasons: BTreeMap<(u32, u32), QueueReason> = BTreeMap::new();
    let mut needs_human = Vec::new();
    let mut queue = |key, reason: QueueReason| {
        let current = reasons.entry(key).or_insert(reason);
        *current = (*current).min(reason);
    };
    for e in feed.verified().filter(|e| scope.rfcs.contains(&e.rfc)) {
        match by_key.get(&e.key()) {
            None => queue(e.key(), QueueReason::New),
            Some(row) if row.fingerprint != fingerprint(e) => queue(e.key(), QueueReason::Changed),
            Some(row) if row.decision == Decision::NeedsHuman => needs_human.push((*row).clone()),
            Some(row)
                if row.rule == Rule::Modules
                    && matches!(row.decision, Decision::NotApplicable | Decision::Patched)
                    && scope.with_inline_asn1.contains(&row.rfc) =>
            {
                queue(e.key(), QueueReason::Rescope);
            }
            Some(_) => {}
        }
    }
    for row in rows.iter().filter(|r| r.decision == Decision::Patched) {
        let current = feed.errata.get(&row.errata_id);
        if !current.is_some_and(|e| e.is_verified() && e.rfc == row.rfc) {
            queue(row.key(), QueueReason::Withdrawn);
        }
    }
    for key in failing {
        queue(*key, QueueReason::PatchFails);
    }
    needs_human.sort_by_key(Row::key);
    Queue {
        items: reasons
            .into_iter()
            .map(|((rfc, errata_id), reason)| QueueItem {
                rfc,
                errata_id,
                reason,
            })
            .collect(),
        needs_human,
    }
}

/// The failing erratum and every higher patched erratum of `rfc`.
pub fn failing_keys(rows: &[Row], rfc: u32, failing_id: u32) -> BTreeSet<(u32, u32)> {
    rows.iter()
        .filter(|r| r.rfc == rfc && r.decision == Decision::Patched && r.errata_id >= failing_id)
        .map(Row::key)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Erratum;
    use crate::ledger::Decider;

    fn erratum(id: u32, rfc: u32, status: &str, orig: &str) -> Erratum {
        Erratum {
            errata_id: id,
            rfc,
            status: status.into(),
            section: "1".into(),
            orig_text: orig.into(),
            correct_text: String::new(),
            notes: String::new(),
            names: vec![],
        }
    }

    fn feed(errata: Vec<Erratum>) -> Feed {
        Feed {
            total: errata.len(),
            errata: errata.into_iter().map(|e| (e.errata_id, e)).collect(),
            malformed: vec![],
        }
    }

    fn row(e: &Erratum, decision: Decision, rule: Rule) -> Row {
        Row {
            rfc: e.rfc,
            errata_id: e.errata_id,
            decision,
            by: Decider::Claude,
            fingerprint: fingerprint(e),
            rule,
            note: "RFC 9999 x".into(),
        }
    }

    #[test]
    fn relevance_looks_for_asn1_structure() {
        assert!(asn1_relevant("Foo { 1 2 } DEFINITIONS ::= BEGIN", ""));
        assert!(asn1_relevant("", "A ::= SEQUENCE SIZE (1..MAX) OF B"));
        assert!(asn1_relevant(
            "id-x OBJECT IDENTIFIER ::= { id-pkix 1 }",
            ""
        ));
        assert!(asn1_relevant("IMPORTS a FROM Mod", ""));
        assert!(!asn1_relevant("rule ::= \"a\" / \"b\"", ""));
        assert!(!asn1_relevant(
            "x OBJECT-TYPE\n SYNTAX SEQUENCE OF Entry\nE ::= SEQUENCE {",
            ""
        ));
        assert!(asn1_relevant(
            "C ::= CLASS { &id } WITH SYNTAX { ID &id }\nX ::= SEQUENCE {",
            ""
        ));
        // RFC 8360 erratum 5870: an X.681 object whose field is spelled `SYNTAX`, which is not SMI.
        assert!(asn1_relevant(
            "ext-pe-autonomousSysIds-v2 EXTENSION ::= {\n  SYNTAX ASIdentifiers\n  IDENTIFIED BY id-pe-autonomousSysIds-v2\n}\n\nid-pe-autonomousSysIds-v2 OBJECT IDENTIFIER ::= { id-pe 29 }",
            ""
        ));
        assert!(!asn1_relevant(
            "rule ::= SEQUENCE { a } | other\nother ::= \"x\" / \"y\"",
            ""
        ));
    }

    #[test]
    fn scope_takes_asn1_rows_patches_and_flagged_errata() {
        let f = feed(vec![
            erratum(1, 10, "Verified", "A ::= SEQUENCE {"),
            erratum(2, 11, "Reported", "A ::= SEQUENCE {"),
            erratum(3, 12, "Verified", "prose"),
        ]);
        let scope = compute_scope(
            &[20].into(),
            &[21].into(),
            &[row(
                &erratum(4, 30, "Verified", ""),
                Decision::NotApplicable,
                Rule::Asn1,
            )],
            &[(40, 5)].into(),
            Some(&f),
        );
        assert_eq!(scope.rfcs, [10, 20, 21, 30, 40].into());
    }

    #[test]
    fn queue_reasons() {
        let new = erratum(1, 10, "Verified", "a");
        let changed = erratum(2, 10, "Verified", "b");
        let decided = erratum(3, 10, "Verified", "c");
        let human = erratum(4, 10, "Verified", "d");
        let old_rule = erratum(5, 10, "Verified", "e");
        let withdrawn = erratum(6, 10, "Rejected", "f");
        let out_of_scope = erratum(7, 99, "Verified", "g");
        let f = feed(vec![
            new.clone(),
            changed.clone(),
            decided.clone(),
            human.clone(),
            old_rule.clone(),
            withdrawn.clone(),
            out_of_scope,
        ]);
        let mut changed_row = row(&changed, Decision::NotApplicable, Rule::Asn1);
        changed_row.fingerprint = "0000000000000000".into();
        let rows = vec![
            changed_row,
            row(&decided, Decision::Patched, Rule::Asn1),
            row(&human, Decision::NeedsHuman, Rule::Asn1),
            row(&old_rule, Decision::NotApplicable, Rule::Modules),
            row(&withdrawn, Decision::Patched, Rule::Asn1),
        ];
        let scope = Scope {
            rfcs: [10].into(),
            with_inline_asn1: [10].into(),
        };
        let failing = failing_keys(&rows, 10, 3);
        let q = compute_queue(&f, &rows, &scope, &failing);
        let got: Vec<(u32, &str)> = q
            .items
            .iter()
            .map(|i| (i.errata_id, i.reason.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (1, "new"),
                (2, "changed"),
                (3, "patch-fails"),
                (5, "rescope"),
                (6, "withdrawn")
            ]
        );
        assert_eq!(
            q.needs_human
                .iter()
                .map(|r| r.errata_id)
                .collect::<Vec<_>>(),
            [4]
        );
    }

    #[test]
    fn a_changed_needs_human_erratum_is_queued_and_a_withdrawn_decided_one_stays_quiet() {
        let human = erratum(1, 10, "Verified", "a");
        let rejected = erratum(2, 10, "Rejected", "b");
        let mut human_row = row(&human, Decision::NeedsHuman, Rule::Asn1);
        human_row.fingerprint = "0000000000000000".into();
        let rows = vec![
            human_row,
            row(&rejected, Decision::NotApplicable, Rule::Asn1),
        ];
        let scope = Scope {
            rfcs: [10].into(),
            with_inline_asn1: BTreeSet::new(),
        };
        let q = compute_queue(
            &feed(vec![human, rejected]),
            &rows,
            &scope,
            &BTreeSet::new(),
        );
        assert_eq!(
            q.items,
            [QueueItem {
                rfc: 10,
                errata_id: 1,
                reason: QueueReason::Changed
            }]
        );
        assert!(q.needs_human.is_empty());
    }

    #[test]
    fn old_rule_rows_stay_put_without_inline_asn1() {
        let e = erratum(5, 10, "Verified", "e");
        let rows = vec![row(&e, Decision::NotApplicable, Rule::Modules)];
        let scope = Scope {
            rfcs: [10].into(),
            with_inline_asn1: BTreeSet::new(),
        };
        assert!(
            compute_queue(&feed(vec![e]), &rows, &scope, &BTreeSet::new())
                .items
                .is_empty()
        );
    }
}
