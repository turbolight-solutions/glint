use std::collections::{BTreeMap, HashMap};

use crate::{Bundle, CorpusInfo, ModuleRecord, Oid, OidAssignment};

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
}

/// A corpus with nothing in it.
#[derive(Debug, Default)]
pub struct EmptyCorpus {
    info: CorpusInfo,
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
}

/// An in-memory corpus with lookup indexes.
#[derive(Debug, Default)]
pub struct Corpus {
    info: CorpusInfo,
    modules: Vec<ModuleRecord>,
    by_name: HashMap<String, Vec<usize>>,
    by_oid: HashMap<Oid, usize>,
    assignments: BTreeMap<Oid, Vec<OidAssignment>>,
}

impl Corpus {
    pub fn from_bundle(bundle: Bundle) -> Self {
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_oid: HashMap<Oid, usize> = HashMap::new();
        for (index, module) in bundle.modules.iter().enumerate() {
            by_name.entry(module.name.clone()).or_default().push(index);
            if let Some(oid) = &module.oid {
                let current = by_oid.get(oid).and_then(|&i| bundle.modules.get(i));
                let keep_current = current.is_some_and(|c| !is_newer(module, c));
                if !keep_current {
                    by_oid.insert(oid.clone(), index);
                }
            }
        }
        let mut assignments: BTreeMap<Oid, Vec<OidAssignment>> = BTreeMap::new();
        for assignment in bundle.oid_assignments {
            assignments
                .entry(assignment.oid.clone())
                .or_default()
                .push(assignment);
        }
        Self {
            info: bundle.info,
            modules: bundle.modules,
            by_name,
            by_oid,
            assignments,
        }
    }

    pub fn into_bundle(self) -> Bundle {
        Bundle {
            info: self.info,
            modules: self.modules,
            oid_assignments: self.assignments.into_values().flatten().collect(),
        }
    }
}

fn is_newer(candidate: &ModuleRecord, current: &ModuleRecord) -> bool {
    let rank = |m: &ModuleRecord| (m.obsoleted_by.is_empty(), m.source.rfc);
    rank(candidate) > rank(current)
}

/// Picks one of `candidates` by the import resolution policy: not obsoleted first, then the newest RFC.
pub fn preferred(corpus: &dyn CorpusProvider, candidates: &[usize]) -> Option<usize> {
    candidates
        .iter()
        .copied()
        .filter_map(|index| corpus.modules().get(index).map(|module| (index, module)))
        .max_by_key(|(_, module)| (module.obsoleted_by.is_empty(), module.source.rfc))
        .map(|(index, _)| index)
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
}
