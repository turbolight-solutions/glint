use std::collections::{BTreeMap, HashMap};

use crate::{
    CorpusData, CorpusInfo, DocumentRecord, IanaModuleId, ModuleRecord, Oid, OidAssignment,
    OidLabels, draft_name,
};

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
    /// The IANA module identifier entry for `oid`, or for the nearest arc above it that IANA registers.
    fn iana_module_id(&self, _oid: &Oid) -> Option<&IanaModuleId> {
        None
    }
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
    iana: BTreeMap<Oid, IanaModuleId>,
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
            iana: data
                .iana_module_ids
                .into_iter()
                .map(|entry| (entry.oid.clone(), entry))
                .collect(),
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
            iana_module_ids: self.iana.into_values().collect(),
        }
    }
}

fn is_newer(candidate: &ModuleRecord, current: &ModuleRecord) -> bool {
    candidate.rank() > current.rank()
}

/// Picks one of `candidates` by the import resolution policy ([`ModuleRecord::rank`]), then the highest OID (editions of
/// a module share a name and differ in the last arc, so the later one has the larger OID).
pub fn preferred(corpus: &dyn CorpusProvider, candidates: &[usize]) -> Option<usize> {
    candidates
        .iter()
        .copied()
        .filter_map(|index| corpus.modules().get(index).map(|module| (index, module)))
        .max_by_key(|(_, module)| (module.rank(), module.oid.clone()))
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

    fn iana_module_id(&self, oid: &Oid) -> Option<&IanaModuleId> {
        let mut current = Some(oid.clone());
        while let Some(arc) = current {
            if let Some(entry) = self.iana.get(&arc) {
                return Some(entry);
            }
            current = arc.parent();
        }
        None
    }
}

/// Whether the cited `document` (an IANA reference, maybe a draft without revision) is `ours`.
fn cites(document: &str, ours: &str) -> bool {
    let strip = |d: &str| draft_name(d).unwrap_or(d).to_ascii_lowercase();
    strip(document) == strip(ours)
}

/// `id-mod-v1AttrCert-02` and `v1AttrCert-02` alike.
fn normalized_label(label: &str) -> String {
    let lower = label.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("id-mod-")
        .or_else(|| lower.strip_prefix("id-"))
        .unwrap_or(&lower);
    rest.to_owned()
}

/// What the IANA module identifier registries say about a module's identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IanaVerdict<'c> {
    /// The entry is the module's.
    Registered(&'c IanaModuleId),
    /// The entry is another module's.
    Conflict(&'c IanaModuleId),
}

/// What IANA registers the module identifier `oid` (or the arc above it that IANA registers) for, given a module
/// `name` defined in `document` with the label `label` on its last arc; `None` when IANA does not register the arc
/// or the entry cannot be judged.
///
/// The entry is the module's when it cites the module's own document or an RFC that obsoletes it, when a document it
/// cites defines a module of the same name (IANA keeps the reference current as a module is republished), or when its
/// description is the module's label or name. Otherwise it is another module's, but only when a cited document is in the corpus, so that
/// what it defines is known.
pub fn iana_verdict<'c>(
    corpus: &'c dyn CorpusProvider,
    name: &str,
    document: Option<&str>,
    oid: &Oid,
    label: Option<&str>,
) -> Option<IanaVerdict<'c>> {
    let entry = corpus.iana_module_id(oid)?;
    if entry.documents.is_empty() {
        return None;
    }
    let registered = Some(IanaVerdict::Registered(entry));
    if document.is_some_and(|ours| entry.documents.iter().any(|d| cites(d, ours))) {
        return registered;
    }
    let description = normalized_label(&entry.description);
    if label.is_some_and(|label| normalized_label(label) == description)
        || entry.description.eq_ignore_ascii_case(name)
    {
        return registered;
    }
    let mut known = false;
    for module in corpus.modules() {
        // An RFC that obsoletes the module's own may rename the module (RFC 9582's `RPKI-ROA-2023` for RFC 6482's
        // `RPKI-ROA`), and IANA then cites the new RFC.
        if module.name == name
            && document.is_some_and(|ours| cites(&module.source.document, ours))
            && module.obsoleted_by.iter().any(|number| {
                entry
                    .documents
                    .iter()
                    .any(|d| cites(d, &format!("rfc{number}")))
            })
        {
            return registered;
        }
        if !entry
            .documents
            .iter()
            .any(|d| cites(d, &module.source.document))
        {
            continue;
        }
        known = true;
        if module.name == name {
            return registered;
        }
    }
    known.then_some(IanaVerdict::Conflict(entry))
}

/// The entry of [`iana_verdict`] when it is another module's: warn only, the entry is evidence, not a correction.
pub fn iana_conflict<'c>(
    corpus: &'c dyn CorpusProvider,
    name: &str,
    document: Option<&str>,
    oid: &Oid,
    label: Option<&str>,
) -> Option<&'c IanaModuleId> {
    match iana_verdict(corpus, name, document, oid, label)? {
        IanaVerdict::Conflict(entry) => Some(entry),
        IanaVerdict::Registered(_) => None,
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

    #[test]
    fn an_iana_entry_citing_the_rfc_that_obsoletes_the_module_is_the_modules() {
        let mut old = record("RPKI-ROA", 6482, "1.2.840.113549.1.9.16.0.61");
        old.obsoleted_by = vec![9582];
        let new = record("RPKI-ROA-2023", 9582, "1.2.840.113549.1.9.16.0.73");
        let mut data = corpus(vec![old, new]).into_data();
        data.iana_module_ids = vec![IanaModuleId {
            oid: "1.2.840.113549.1.9.16.0.61".parse().unwrap(),
            description: "id-mod-rpkiROA".to_owned(),
            registry: String::new(),
            documents: vec!["rfc9582".to_owned()],
            errata: Vec::new(),
        }];
        let corpus = Corpus::from_data(data);
        let oid: Oid = "1.2.840.113549.1.9.16.0.61".parse().unwrap();
        let verdict = |name, document| iana_verdict(&corpus, name, Some(document), &oid, None);
        assert!(matches!(
            verdict("RPKI-ROA", "rfc6482"),
            Some(IanaVerdict::Registered(_))
        ));
        assert!(matches!(
            verdict("Unrelated", "rfc9999"),
            Some(IanaVerdict::Conflict(_))
        ));
        assert_eq!(
            corpus.iana_module_id(&oid.child(1)).map(|e| e.oid.clone()),
            Some(oid.clone()),
            "an arc below a registered one has its entry"
        );
    }
}
