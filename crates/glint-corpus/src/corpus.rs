use std::collections::{BTreeMap, HashMap};

use crate::{CorpusData, CorpusInfo, DocumentRecord, ModuleRecord, Oid, OidAssignment, OidLabels};

/// Read access to the corpus. The check pipeline sees only this trait, so it stays free of I/O.
pub trait CorpusProvider {
    fn info(&self) -> &CorpusInfo;
    fn modules(&self) -> &[ModuleRecord];
    /// Indexes into [`CorpusProvider::modules`] of every module with this name.
    fn by_name(&self, name: &str) -> &[usize];
    /// The module whose definitive identifier is exactly `oid`.
    fn by_oid(&self, oid: &Oid) -> Option<usize>;
    /// Every corpus assignment of exactly this OID.
    fn assignments_of(&self, oid: &Oid) -> &[OidAssignment];
    /// Every corpus assignment at or below `oid`, in OID order.
    fn assignments_under(&self, oid: &Oid) -> Vec<&OidAssignment>;
    /// The OID label table.
    fn oid_labels(&self) -> &OidLabels;
}

/// A corpus with nothing in it.
#[derive(Debug, Default)]
pub struct EmptyCorpus {
    info: CorpusInfo,
    labels: OidLabels,
}

impl CorpusProvider for EmptyCorpus {
    fn info(&self) -> &CorpusInfo {
        &self.info
    }

    fn modules(&self) -> &[ModuleRecord] {
        &[]
    }

    fn by_name(&self, _name: &str) -> &[usize] {
        &[]
    }

    fn by_oid(&self, _oid: &Oid) -> Option<usize> {
        None
    }

    fn assignments_of(&self, _oid: &Oid) -> &[OidAssignment] {
        &[]
    }

    fn assignments_under(&self, _oid: &Oid) -> Vec<&OidAssignment> {
        Vec::new()
    }

    fn oid_labels(&self) -> &OidLabels {
        &self.labels
    }
}

/// An in-memory corpus with lookup indexes.
#[derive(Debug, Default)]
pub struct Corpus {
    info: CorpusInfo,
    modules: Vec<ModuleRecord>,
    by_name: HashMap<String, Vec<usize>>,
    by_oid: HashMap<Oid, usize>,
    assignments: BTreeMap<Oid, Vec<OidAssignment>>,
    documents: Vec<DocumentRecord>,
    labels: OidLabels,
}

impl Corpus {
    pub fn from_data(data: CorpusData) -> Self {
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_oid: HashMap<Oid, usize> = HashMap::new();
        for (index, module) in data.modules.iter().enumerate() {
            by_name.entry(module.name.clone()).or_default().push(index);
            if let Some(oid) = &module.oid {
                let current = by_oid.get(oid).and_then(|&i| data.modules.get(i));
                let keep_current = current.is_some_and(|c| !is_newer(module, c));
                if !keep_current {
                    by_oid.insert(oid.clone(), index);
                }
            }
        }
        let mut assignments: BTreeMap<Oid, Vec<OidAssignment>> = BTreeMap::new();
        for assignment in data.oid_assignments {
            assignments
                .entry(assignment.oid.clone())
                .or_default()
                .push(assignment);
        }
        Self {
            info: data.info,
            modules: data.modules,
            by_name,
            by_oid,
            assignments,
            documents: data.documents,
            labels: OidLabels::new(data.oid_nodes, data.oid_allowlist),
        }
    }

    /// The document index.
    pub fn documents(&self) -> &[DocumentRecord] {
        &self.documents
    }

    pub fn into_data(self) -> CorpusData {
        CorpusData {
            schema: crate::SCHEMA,
            info: self.info,
            documents: self.documents,
            modules: self.modules,
            oid_assignments: self.assignments.into_values().flatten().collect(),
            oid_nodes: self.labels.nodes(),
            oid_allowlist: self.labels.allowlist().to_vec(),
        }
    }
}

fn is_newer(candidate: &ModuleRecord, current: &ModuleRecord) -> bool {
    let rank = |m: &ModuleRecord| (m.obsoleted_by.is_empty(), m.source.recency());
    rank(candidate) > rank(current)
}

/// Picks one of `candidates` by the import resolution policy: not obsoleted first, then the most recent document, then the
/// highest OID (editions of a module share a name and differ in the last arc, so the later one has the larger OID).
pub fn preferred(corpus: &dyn CorpusProvider, candidates: &[usize]) -> Option<usize> {
    candidates
        .iter()
        .copied()
        .filter_map(|index| corpus.modules().get(index).map(|module| (index, module)))
        .max_by_key(|(_, module)| {
            (
                module.obsoleted_by.is_empty(),
                module.source.recency(),
                module.oid.clone(),
            )
        })
        .map(|(index, _)| index)
}

/// [`ModuleRecord::label`], plus `@<oid>` when the source has another module of the same name (RFC 4998 has
/// two `ERS`), so that the label names exactly one module.
pub fn unique_label(corpus: &dyn CorpusProvider, record: &ModuleRecord) -> String {
    let label = record.label();
    let source = record.source.label();
    let twins = corpus
        .by_name(&record.name)
        .iter()
        .filter_map(|&index| corpus.modules().get(index))
        .filter(|other| other.source.label() == source)
        .count();
    match &record.oid {
        Some(oid) if twins > 1 => format!("{label}@{oid}"),
        _ => label,
    }
}

impl CorpusProvider for Corpus {
    fn info(&self) -> &CorpusInfo {
        &self.info
    }

    fn modules(&self) -> &[ModuleRecord] {
        &self.modules
    }

    fn by_name(&self, name: &str) -> &[usize] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    fn by_oid(&self, oid: &Oid) -> Option<usize> {
        self.by_oid.get(oid).copied()
    }

    fn assignments_of(&self, oid: &Oid) -> &[OidAssignment] {
        self.assignments.get(oid).map_or(&[], Vec::as_slice)
    }

    fn assignments_under(&self, oid: &Oid) -> Vec<&OidAssignment> {
        self.assignments
            .range(oid.clone()..)
            .take_while(|(key, _)| oid.is_prefix_of(key))
            .flat_map(|(_, list)| list.iter())
            .collect()
    }

    fn oid_labels(&self) -> &OidLabels {
        &self.labels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Exports, ModuleSource, ParseStatus, SourceKind, SyntaxEra};

    fn record(name: &str, rfc: u32, oid: &str) -> ModuleRecord {
        ModuleRecord {
            name: name.to_owned(),
            oid: oid.parse().ok(),
            source: ModuleSource::rfc(rfc, None, SourceKind::Extracted),
            rfc_status: None,
            obsoleted_by: Vec::new(),
            syntax_era: SyntaxEra::Neutral,
            text: String::new(),
            exports: Exports::All,
            parse_status: ParseStatus::Clean,
            sha256: String::new(),
        }
    }

    fn corpus(modules: Vec<ModuleRecord>) -> Corpus {
        Corpus::from_data(CorpusData {
            modules,
            ..CorpusData::default()
        })
    }

    #[test]
    fn a_label_carries_the_oid_only_when_the_source_has_two_modules_of_the_name() {
        let corpus = corpus(vec![
            record("Twin", 1, "1.2.3.1"),
            record("Twin", 1, "1.2.3.2"),
            record("Twin", 2, "1.2.3.3"),
            record("Solo", 1, "1.2.4.1"),
        ]);
        let label = |index: usize| unique_label(&corpus, &corpus.modules()[index]);
        assert_eq!(label(0), "rfc1#Twin@1.2.3.1");
        assert_eq!(label(1), "rfc1#Twin@1.2.3.2");
        assert_eq!(
            label(2),
            "rfc2#Twin",
            "another RFC's module of the name does not count"
        );
        assert_eq!(label(3), "rfc1#Solo");
    }

    #[test]
    fn the_later_edition_is_preferred_when_nothing_else_differs() {
        let corpus = corpus(vec![
            record("Twin", 1, "1.2.3.2"),
            record("Twin", 1, "1.2.3.1"),
        ]);
        let all: Vec<usize> = corpus.by_name("Twin").to_vec();
        let chosen = preferred(&corpus, &all).unwrap();
        assert_eq!(
            corpus.modules()[chosen].oid.as_ref().unwrap().to_string(),
            "1.2.3.2"
        );
    }
}
