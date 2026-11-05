//! The OID label table: which `NameAndNumberForm` labels published documents write for each arc, and the checks that
//! compare a label against it.
//!
//! The evidence comes from published modules only, after verified errata. A document is never evidence about itself:
//! every query takes the document being checked and leaves its evidence out.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::{Oid, same_document};

/// One label written for an arc, and the documents that write it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelEvidence {
    pub label: String,
    pub documents: Vec<String>,
}

/// The labels of one OID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidNode {
    pub oid: Oid,
    pub labels: Vec<LabelEvidence>,
}

/// An OID that is wrong by the rules but deployed, so its subtree is neither evidence nor checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowedOid {
    pub oid: Oid,
    pub reason: String,
}

/// What is wrong with a label, compared with the rest of the corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelProblem {
    /// Under the same parent, the corpus gives the label another number.
    WrongNumber {
        expected: u128,
        documents: Vec<String>,
    },
    /// The corpus has `label(number)` only under another parent.
    WrongParent {
        expected_parent: Oid,
        documents: Vec<String>,
    },
    /// The corpus calls this arc by other labels (information only).
    Synonym { others: Vec<String> },
}

/// The fewest documents that make a reading of an arc established.
pub const MIN_SUPPORT: usize = 2;
/// How many times the support of the reading in question the other reading needs.
pub const DOMINANCE: usize = 3;

/// Whether `a` and `b` differ by one arc: one is the parent of the other, or they are siblings.
fn one_arc_apart(a: &Oid, b: &Oid) -> bool {
    b.parent().as_ref() == Some(a)
        || a.parent().as_ref() == Some(b)
        || (a.len() == b.len() && a.parent() == b.parent())
}

/// The label table with its indexes.
#[derive(Debug, Default)]
pub struct OidLabels {
    nodes: BTreeMap<Oid, Vec<LabelEvidence>>,
    /// `(label, last arc)` to every OID that has it.
    by_label: HashMap<(String, u128), Vec<Oid>>,
    allowlist: Vec<AllowedOid>,
}

fn count_without(documents: &[String], own: Option<&str>) -> usize {
    documents
        .iter()
        .filter(|d| own.is_none_or(|own| !same_document(d, own)))
        .count()
}

impl OidLabels {
    pub fn new(nodes: Vec<OidNode>, allowlist: Vec<AllowedOid>) -> Self {
        let mut by_label: HashMap<(String, u128), Vec<Oid>> = HashMap::new();
        let mut map = BTreeMap::new();
        for node in nodes {
            if let Some(last) = node.oid.last() {
                for evidence in &node.labels {
                    by_label
                        .entry((evidence.label.clone(), last))
                        .or_default()
                        .push(node.oid.clone());
                }
            }
            map.insert(node.oid, node.labels);
        }
        Self {
            nodes: map,
            by_label,
            allowlist,
        }
    }

    /// Builds the table from `(oid, label, document)` triples.
    pub fn collect(
        triples: impl IntoIterator<Item = (Oid, String, String)>,
        allowlist: Vec<AllowedOid>,
    ) -> Self {
        let mut grouped: BTreeMap<Oid, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
        for (oid, label, document) in triples {
            grouped
                .entry(oid)
                .or_default()
                .entry(label)
                .or_default()
                .insert(document);
        }
        let nodes = grouped
            .into_iter()
            .map(|(oid, labels)| OidNode {
                oid,
                labels: labels
                    .into_iter()
                    .map(|(label, documents)| LabelEvidence {
                        label,
                        documents: documents.into_iter().collect(),
                    })
                    .collect(),
            })
            .collect();
        Self::new(nodes, allowlist)
    }

    pub fn nodes(&self) -> Vec<OidNode> {
        self.nodes
            .iter()
            .map(|(oid, labels)| OidNode {
                oid: oid.clone(),
                labels: labels.clone(),
            })
            .collect()
    }

    pub fn allowlist(&self) -> &[AllowedOid] {
        &self.allowlist
    }

    /// The labels of `oid`.
    pub fn labels_of(&self, oid: &Oid) -> &[LabelEvidence] {
        self.nodes.get(oid).map_or(&[], Vec::as_slice)
    }

    /// The allowlist entry that `oid` is at or below.
    pub fn allowed(&self, oid: &Oid) -> Option<&AllowedOid> {
        self.allowlist.iter().find(|a| a.oid.is_prefix_of(oid))
    }

    /// The documents other than `own` that write `label` for `oid`.
    fn support(&self, oid: &Oid, label: &str, own: Option<&str>) -> Vec<String> {
        self.labels_of(oid)
            .iter()
            .filter(|e| e.label == label)
            .flat_map(|e| e.documents.iter())
            .filter(|d| own.is_none_or(|own| !same_document(d, own)))
            .cloned()
            .collect()
    }

    /// Whether documents other than `own` label `oid` or an arc right below it.
    fn known(&self, oid: &Oid, own: Option<&str>) -> bool {
        self.nodes
            .range(oid.clone()..)
            .take_while(|(key, _)| {
                oid.is_prefix_of(key) && key.len() <= oid.len().saturating_add(1)
            })
            .any(|(_, labels)| labels.iter().any(|e| count_without(&e.documents, own) > 0))
    }

    /// Compares the label `label` written for `oid` with what documents other than `own` write. `defined` says
    /// whether a module has exactly `oid` as its identifier, which makes the arc established whatever its labels.
    ///
    /// Labels are reused freely (`id-mod(0)`, `us(840)` under both `member-body` and `country`), and modules get new
    /// arcs between editions, so a problem is reported only when the evidence is one-sided: the arc is not
    /// established, and the other reading has [`MIN_SUPPORT`] documents and [`DOMINANCE`] times the support.
    pub fn check(
        &self,
        oid: &Oid,
        label: &str,
        own: Option<&str>,
        defined: bool,
    ) -> Option<LabelProblem> {
        let (Some(parent), Some(number)) = (oid.parent(), oid.last()) else {
            return None;
        };
        if parent.is_empty() || self.allowed(oid).is_some() {
            return None;
        }
        let here = self.support(oid, label, own).len();
        let established = defined || here >= MIN_SUPPORT;
        let outweighs = |documents: &[String]| {
            documents.len() >= MIN_SUPPORT
                && documents.len() >= DOMINANCE.saturating_mul(here.max(1))
        };

        // The same label under the same parent with another number.
        if !established {
            let mut best: Option<(u128, Vec<String>)> = None;
            for (sibling, _) in self
                .nodes
                .range(parent.child(0)..)
                .take_while(|(key, _)| parent.is_prefix_of(key))
                .filter(|(key, _)| key.len() == oid.len() && *key != oid)
            {
                let documents = self.support(sibling, label, own);
                if outweighs(&documents)
                    && best.as_ref().is_none_or(|(_, b)| documents.len() > b.len())
                    && let Some(arc) = sibling.last()
                {
                    best = Some((arc, documents));
                }
            }
            if let Some((expected, documents)) = best {
                return Some(LabelProblem::WrongNumber {
                    expected,
                    documents,
                });
            }
        }

        // `label(number)` under exactly one other parent, one arc away from this one (an arc missing, extra or
        // wrong), while nobody writes it under this parent.
        if !established && here == 0 {
            let elsewhere: Vec<(Oid, Vec<String>)> = self
                .by_label
                .get(&(label.to_owned(), number))
                .into_iter()
                .flatten()
                .filter(|other| *other != oid)
                .filter_map(|other| {
                    let documents = self.support(other, label, own);
                    if documents.is_empty() {
                        None
                    } else {
                        Some((other.parent()?, documents))
                    }
                })
                .collect();
            if let [(expected_parent, documents)] = elsewhere.as_slice()
                && outweighs(documents)
                && one_arc_apart(&parent, expected_parent)
                && self.known(&parent, own)
            {
                return Some(LabelProblem::WrongParent {
                    expected_parent: expected_parent.clone(),
                    documents: documents.clone(),
                });
            }
        }

        if here == 0 {
            let others: Vec<String> = self
                .labels_of(oid)
                .iter()
                .filter(|e| e.label != label && count_without(&e.documents, own) > 0)
                .map(|e| e.label.clone())
                .collect();
            if !others.is_empty() {
                return Some(LabelProblem::Synonym { others });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(text: &str) -> Oid {
        text.parse().unwrap()
    }

    fn table() -> OidLabels {
        let mut triples = Vec::new();
        for doc in ["rfc1", "rfc2", "rfc3"] {
            triples.push((oid("1.3.6"), "dod".to_owned(), doc.to_owned()));
            triples.push((
                oid("1.2.840.113549.1.9.16.2"),
                "id-aa".to_owned(),
                doc.to_owned(),
            ));
            triples.push((
                oid("1.2.840.113549.1.9.16"),
                "smime".to_owned(),
                doc.to_owned(),
            ));
        }
        triples.push((oid("1.3.4"), "dod".to_owned(), "rfc9".to_owned()));
        triples.push((
            oid("1.2.840.113549.1.9.2"),
            "id-aa".to_owned(),
            "rfc9".to_owned(),
        ));
        triples.push((
            oid("1.2.840.113549.1.9.1"),
            "emailAddress".to_owned(),
            "rfc1".to_owned(),
        ));
        for (parent, doc) in [("1.3.6.1.5.5.7", "rfc1"), ("1.3.6.1.5.5.8", "rfc2")] {
            triples.push((oid(parent).child(0), "id-mod".to_owned(), doc.to_owned()));
        }
        OidLabels::collect(
            triples,
            vec![AllowedOid {
                oid: oid("2.2.840.10040.2"),
                reason: "deployed".to_owned(),
            }],
        )
    }

    #[test]
    fn a_label_with_the_number_most_documents_use_is_fine() {
        assert_eq!(table().check(&oid("1.3.6"), "dod", None, false), None);
    }

    #[test]
    fn a_label_with_a_minority_number_is_wrong_and_a_document_is_not_its_own_evidence() {
        let table = table();
        assert!(matches!(
            table.check(&oid("1.3.4"), "dod", Some("rfc9"), false),
            Some(LabelProblem::WrongNumber { expected: 6, .. })
        ));
        assert!(matches!(
            table.check(&oid("1.3.4"), "dod", None, false),
            Some(LabelProblem::WrongNumber { expected: 6, .. })
        ));
    }

    #[test]
    fn a_defined_or_established_arc_is_not_reported() {
        let table = table();
        assert_eq!(table.check(&oid("1.3.4"), "dod", Some("rfc9"), true), None);
        let mut triples = vec![(oid("1.3.4"), "dod".to_owned(), "rfc8".to_owned())];
        triples.extend(table.nodes().into_iter().flat_map(|node| {
            node.labels.into_iter().flat_map(move |e| {
                let oid = node.oid.clone();
                e.documents
                    .into_iter()
                    .map(move |d| (oid.clone(), e.label.clone(), d))
            })
        }));
        let two = OidLabels::collect(triples, Vec::new());
        assert_eq!(two.check(&oid("1.3.4"), "dod", Some("rfc7"), false), None);
    }

    #[test]
    fn a_label_under_an_unrelated_parent_is_not_reported() {
        let mut triples = Vec::new();
        for doc in ["rfc1", "rfc2", "rfc3"] {
            triples.push((oid("1.3.6.1.5.5.7.0"), "mod".to_owned(), doc.to_owned()));
        }
        triples.push((
            oid("1.2.840.113549.1.9.16.1"),
            "ct".to_owned(),
            "rfc1".to_owned(),
        ));
        let table = OidLabels::collect(triples, Vec::new());
        assert_eq!(
            table.check(&oid("1.2.840.113549.1.9.16.0"), "mod", None, false),
            None
        );
    }

    #[test]
    fn a_label_under_the_wrong_parent_is_found() {
        let problem = table().check(&oid("1.2.840.113549.1.9.2"), "id-aa", Some("rfc9"), false);
        assert_eq!(
            problem,
            Some(LabelProblem::WrongParent {
                expected_parent: oid("1.2.840.113549.1.9.16"),
                documents: vec!["rfc1".into(), "rfc2".into(), "rfc3".into()],
            })
        );
    }

    #[test]
    fn a_label_used_under_several_parents_or_an_unknown_parent_is_not_reported() {
        let table = table();
        assert_eq!(
            table.check(&oid("1.3.6.1.5.5.9.0"), "id-mod", None, false),
            None
        );
        assert_eq!(
            table.check(&oid("1.3.6.1.4.1.99.2"), "id-aa", None, false),
            None
        );
    }

    #[test]
    fn another_name_for_a_known_arc_is_a_synonym_and_the_allowlist_is_quiet() {
        let table = table();
        assert_eq!(
            table.check(&oid("1.3.6"), "department-of-defense", None, false),
            Some(LabelProblem::Synonym {
                others: vec!["dod".into()]
            })
        );
        assert_eq!(
            table.check(&oid("2.2.840.10040.2.1"), "x", None, false),
            None
        );
    }
}
