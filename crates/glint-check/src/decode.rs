//! The decoder's view of the corpus: the IR of a root type from a corpus module or from user ASN.1, the names of OIDs,
//! and the OID hint table resolved to types (Codegen and Decoder Design, Decoder).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use glint_corpus::{CorpusProvider, Oid};
use glint_decode::{Hint, OidName, Oracle};
use glint_ir::{IrSet, ObjectSetId, Value};
use glint_sema::{Analysis, DraftSource, Options, analyze};
use glint_syntax::{ParseOptions, parse_source_with};

fn options(home_document: Option<String>) -> Options {
    Options {
        oid_collisions: false,
        oid_labels: false,
        policy_checks: false,
        resolution_notes: false,
        home_document,
        ..Options::default()
    }
}

/// Analyzes the corpus module `index` as `check` resolves it (its imports against the corpus) and hands the analysis
/// and the module's name to `f`.
pub(crate) fn with_corpus_module<T>(
    corpus: &dyn CorpusProvider,
    index: usize,
    f: impl FnOnce(&Analysis<'_>, &str) -> Option<T>,
) -> Option<T> {
    let record = corpus.modules().get(index)?;
    let source = DraftSource {
        parse: parse_source_with(&record.text, ParseOptions::corpus()),
        text: record.text.clone(),
    };
    let options = options(Some(record.source.document.clone()));
    let analysis = analyze(vec![source], corpus, &options);
    f(&analysis, &record.name)
}

/// The analyzed module named `name` among the drafts of `analysis`.
fn draft_module(analysis: &Analysis<'_>, name: &str) -> Option<glint_sema::ModuleId> {
    analysis.drafts.iter().copied().find(|&id| {
        analysis
            .graph
            .module(id)
            .is_some_and(|m| m.model.name == name)
    })
}

/// The IR of the type `ty` of the corpus module `index`.
pub fn lower_corpus_type(
    corpus: &dyn CorpusProvider,
    index: usize,
    ty: &str,
) -> Result<IrSet, String> {
    let name = corpus
        .modules()
        .get(index)
        .map(|m| m.name.clone())
        .ok_or("no such corpus module")?;
    with_corpus_module(corpus, index, |analysis, name| {
        let module = draft_module(analysis, name)?;
        glint_sema::ir::lower(&analysis.graph, module, ty)
    })
    .ok_or_else(|| format!("`{name}` defines no type `{ty}`"))
}

/// The IR of the type `ty` of the module `module` of user ASN.1 `texts`, imports resolved against the corpus.
pub fn lower_user_type(
    texts: &[String],
    corpus: &dyn CorpusProvider,
    module: &str,
    ty: &str,
) -> Result<IrSet, String> {
    let sources = texts
        .iter()
        .map(|text| DraftSource {
            parse: parse_source_with(text, ParseOptions::default()),
            text: text.clone(),
        })
        .collect();
    let options = options(None);
    let analysis = analyze(sources, corpus, &options);
    let id = draft_module(&analysis, module)
        .ok_or_else(|| format!("no module `{module}` in the given ASN.1"))?;
    glint_sema::ir::lower(&analysis.graph, id, ty)
        .ok_or_else(|| format!("`{module}` defines no type `{ty}`"))
}

/// The corpus module `name` of `document`, the preferred one when the document defines several.
pub fn find_corpus_module(
    corpus: &dyn CorpusProvider,
    document: &str,
    name: &str,
) -> Option<usize> {
    let candidates: Vec<usize> = corpus
        .by_name(name)
        .iter()
        .copied()
        .filter(|&i| {
            corpus
                .modules()
                .get(i)
                .is_some_and(|m| m.source.document.eq_ignore_ascii_case(document))
        })
        .collect();
    glint_corpus::preferred(corpus, &candidates)
}

/// The IR of a corpus root written `doc#Module.Type` (the PEM label map's form).
pub fn lower_corpus_root(corpus: &dyn CorpusProvider, root: &str) -> Result<IrSet, String> {
    let (document, rest) = root.split_once('#').ok_or("expected DOC#Module.Type")?;
    let (module, ty) = rest.split_once('.').ok_or("expected DOC#Module.Type")?;
    let index = find_corpus_module(corpus, document, module)
        .ok_or_else(|| format!("{document}#{module} is not in the corpus"))?;
    lower_corpus_type(corpus, index, ty)
}

type SetKey = (String, String, String);
/// A lowered object set and the id of its table.
type LoweredSet = (Rc<IrSet>, ObjectSetId);

/// Answers the decoder's questions from the corpus: OID names from the assignments, hints from the hint table, each
/// hinted object set lowered once.
pub struct CorpusOracle<'c> {
    corpus: &'c dyn CorpusProvider,
    sets: RefCell<HashMap<SetKey, Option<LoweredSet>>>,
}

impl<'c> CorpusOracle<'c> {
    pub fn new(corpus: &'c dyn CorpusProvider) -> Self {
        Self {
            corpus,
            sets: RefCell::new(HashMap::new()),
        }
    }

    fn object_set(&self, document: &str, module: &str, set: &str) -> Option<LoweredSet> {
        let key = (document.to_owned(), module.to_owned(), set.to_owned());
        if let Some(found) = self.sets.borrow().get(&key) {
            return found.clone();
        }
        let lowered = find_corpus_module(self.corpus, document, module)
            .and_then(|index| {
                with_corpus_module(self.corpus, index, |analysis, name| {
                    let id = draft_module(analysis, name)?;
                    glint_sema::ir::lower_object_set(&analysis.graph, id, set)
                })
            })
            .map(|(ir, table)| (Rc::new(ir), table));
        self.sets.borrow_mut().insert(key, lowered.clone());
        lowered
    }
}

impl Oracle for CorpusOracle<'_> {
    fn oid_names(&self, dotted: &str) -> Vec<OidName> {
        let Ok(oid) = dotted.parse::<Oid>() else {
            return Vec::new();
        };
        let mut names: Vec<OidName> = Vec::new();
        for assignment in self.corpus.assignments_of(&oid) {
            if names.iter().any(|n| n.name == assignment.name) {
                continue;
            }
            names.push(OidName {
                name: assignment.name.clone(),
                module: assignment.module.clone(),
                document: Some(assignment.document.clone()),
            });
        }
        names
    }

    fn hints(&self, dotted: &str) -> Vec<Hint> {
        let Ok(oid) = dotted.parse::<Oid>() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for hint in self.corpus.oid_hints(&oid) {
            let Some((ir, table)) = self.object_set(&hint.document, &hint.module, &hint.set) else {
                continue;
            };
            let ty = ir.object_set(table).and_then(|t| {
                t.rows
                    .iter()
                    .find(|row| matches!(&row.key, Some(Value::Oid(o)) if o == dotted))
                    .and_then(|row| row.types.get(&hint.field))
                    .cloned()
            });
            if let Some(ty) = ty {
                out.push(Hint {
                    from: format!("{}#{}", hint.document, hint.module),
                    field: format!("{}.{}", hint.class, hint.field),
                    set: Rc::clone(&ir),
                    ty,
                });
            }
        }
        out
    }
}
