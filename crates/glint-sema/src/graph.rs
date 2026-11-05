//! The set of loaded modules (the draft's own and those pulled from the corpus) and import resolution.

use std::collections::HashMap;

use glint_corpus::{CorpusProvider, ModuleRecord, Oid, preferred};
use glint_syntax::ast::{self};
use glint_syntax::{Parse, ParseOptions, SyntaxError, SyntaxKind, SyntaxNode, parse_source_with};

use crate::Options;
use crate::eval::Evaluator;
use crate::finding::SourceId;
use crate::model::{AssignedId, ModuleModel, Selection, Symbol};
use crate::view::{defined_type_ref, type_child};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

/// Where a module came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Draft,
    Corpus {
        record: usize,
    },
    /// The built-in classes `TYPE-IDENTIFIER` and `ABSTRACT-SYNTAX` (X.681 Annexes A and B).
    Builtin,
}

/// A parsed source text.
#[derive(Debug)]
pub struct SourceUnit {
    pub id: SourceId,
    pub origin: Origin,
    pub text: String,
    pub root: SyntaxNode,
    pub syntax_errors: Vec<SyntaxError>,
}

/// How an import was matched to a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matched {
    /// No module found; the imported symbols are opaque.
    Opaque,
    /// By the object identifier in the `AssignedIdentifier` (X.680 13.11).
    Oid,
    /// By `WITH SUCCESSORS` or `WITH DESCENDANTS` (X.680 13.16 f).
    Selection,
    Pin,
    /// By module name, with the OID absent from the corpus.
    NameOnly,
    /// By name, taken from the draft's own modules.
    DraftName,
    /// The module's own name.
    SelfImport,
}

#[derive(Debug, Clone)]
pub struct ResolvedImport {
    pub target: Option<ModuleId>,
    pub matched: Matched,
    /// The evaluated `AssignedIdentifier`, if it could be evaluated.
    pub assigned_oid: Option<Oid>,
    /// Corpus candidates of the same name when the choice was not forced by an OID.
    pub candidates: Vec<usize>,
    /// The `AssignedIdentifier` is present but the module of that OID is not known; resolved by name instead.
    pub oid_mismatch: bool,
}

impl ResolvedImport {
    fn unresolved() -> Self {
        Self {
            target: None,
            matched: Matched::Opaque,
            assigned_oid: None,
            candidates: Vec::new(),
            oid_mismatch: false,
        }
    }
}

#[derive(Debug)]
pub struct LoadedModule {
    pub id: ModuleId,
    pub source: SourceId,
    pub origin: Origin,
    pub node: ast::Module,
    pub model: ModuleModel,
    pub oid: Option<Oid>,
    pub imports: Vec<ResolvedImport>,
}

impl LoadedModule {
    pub fn is_draft(&self) -> bool {
        self.origin == Origin::Draft
    }
}

/// What a name refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Symbol {
        module: ModuleId,
        index: usize,
    },
    /// Imported from a module that is not available.
    Opaque,
    /// Imported from a module that neither defines nor exports it.
    MissingInTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lookup {
    pub found: Found,
    /// The import clause the name came through, if any.
    pub via_clause: Option<usize>,
    /// The name is defined locally and imported, or imported more than once (X.680 13.20).
    pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedError {
    /// The module is not named in `IMPORTS` (X.680 13.16 c).
    ModuleNotImported,
    /// The module is imported, but not this symbol from it.
    SymbolNotImported,
}

const MAX_FOLLOW: usize = 16;

pub struct ModuleGraph<'c> {
    corpus: &'c dyn CorpusProvider,
    options: &'c Options,
    pub units: Vec<SourceUnit>,
    pub modules: Vec<LoadedModule>,
    corpus_loaded: HashMap<usize, ModuleId>,
    resolved_up_to: usize,
    builtin: Option<ModuleId>,
}

/// X.681 A.2 and B.2: available in every module without importing (A.3, B.3).
const BUILTIN_MODULE: &str = "Builtin-X681 DEFINITIONS ::= BEGIN
TYPE-IDENTIFIER ::= CLASS {
    &id OBJECT IDENTIFIER UNIQUE,
    &Type
} WITH SYNTAX { &Type IDENTIFIED BY &id }
ABSTRACT-SYNTAX ::= CLASS {
    &id OBJECT IDENTIFIER UNIQUE,
    &Type,
    &property BIT STRING { handles-invalid-encodings(0) } DEFAULT {}
} WITH SYNTAX { &Type IDENTIFIED BY &id [HAS PROPERTY &property] }
END";

impl<'c> ModuleGraph<'c> {
    pub fn new(corpus: &'c dyn CorpusProvider, options: &'c Options) -> Self {
        Self {
            corpus,
            options,
            units: Vec::new(),
            modules: Vec::new(),
            corpus_loaded: HashMap::new(),
            resolved_up_to: 0,
            builtin: None,
        }
    }

    /// Loads the built-in classes as a hidden module. Call after every other module is loaded, so that source ids
    /// of the draft and the corpus keep their positions.
    pub fn load_builtin(&mut self) {
        if self.builtin.is_some() {
            return;
        }
        let parse = parse_source_with(BUILTIN_MODULE, ParseOptions::corpus());
        self.builtin = self
            .add_unit(Origin::Builtin, BUILTIN_MODULE.to_owned(), &parse)
            .first()
            .copied();
    }

    pub fn builtin(&self) -> Option<ModuleId> {
        self.builtin
    }

    pub fn corpus(&self) -> &'c dyn CorpusProvider {
        self.corpus
    }

    pub fn options(&self) -> &'c Options {
        self.options
    }

    pub fn module(&self, id: ModuleId) -> Option<&LoadedModule> {
        self.modules.get(usize::try_from(id.0).ok()?)
    }

    pub fn unit(&self, id: SourceId) -> Option<&SourceUnit> {
        self.units.get(usize::try_from(id.0).ok()?)
    }

    pub fn symbol(&self, module: ModuleId, index: usize) -> Option<&Symbol> {
        self.module(module)?.model.symbols.get(index)
    }

    /// Adds the modules of an already parsed draft block and returns their ids.
    pub fn add_draft(&mut self, text: String, parse: &Parse) -> Vec<ModuleId> {
        self.add_unit(Origin::Draft, text, parse)
    }

    fn add_unit(&mut self, origin: Origin, text: String, parse: &Parse) -> Vec<ModuleId> {
        let source = SourceId(u32::try_from(self.units.len()).unwrap_or(u32::MAX));
        let root = parse.syntax();
        let Some(file) = ast::SourceFile::cast(root.clone()) else {
            return Vec::new();
        };
        self.units.push(SourceUnit {
            id: source,
            origin,
            text,
            root,
            syntax_errors: parse.errors().to_vec(),
        });
        let mut ids = Vec::new();
        for node in file.modules() {
            let Some(model) = ModuleModel::build(&node) else {
                continue;
            };
            let id = ModuleId(u32::try_from(self.modules.len()).unwrap_or(u32::MAX));
            let oid = match origin {
                Origin::Corpus { record } => self
                    .corpus
                    .modules()
                    .get(record)
                    .and_then(|r| r.oid.clone()),
                Origin::Draft => model.oid_group.as_ref().and_then(Evaluator::literal_oid),
                Origin::Builtin => None,
            };
            let import_count = model.imports.len();
            self.modules.push(LoadedModule {
                id,
                source,
                origin,
                node,
                model,
                oid,
                imports: vec![ResolvedImport::unresolved(); import_count],
            });
            ids.push(id);
        }
        ids
    }

    fn load_corpus_module(&mut self, record: usize) -> Option<ModuleId> {
        if let Some(&id) = self.corpus_loaded.get(&record) {
            return Some(id);
        }
        let text = self.corpus.modules().get(record)?.text.clone();
        let parse = parse_source_with(&text, ParseOptions::corpus());
        let ids = self.add_unit(Origin::Corpus { record }, text, &parse);
        let name = self.corpus.modules().get(record)?.name.clone();
        let id = ids
            .iter()
            .copied()
            .find(|&id| self.module(id).is_some_and(|m| m.model.name == name))
            .or_else(|| ids.first().copied())?;
        self.corpus_loaded.insert(record, id);
        Some(id)
    }

    /// Loads the corpus modules whose text looks like it defines `name`, the most recent first, and returns how many
    /// were loaded. Used to find the module a snippet quotes when neither the draft nor its imports define the name.
    pub fn load_corpus_defining(&mut self, name: &str) -> usize {
        const LIMIT: usize = 24;
        let mut records: Vec<usize> = self
            .corpus
            .modules()
            .iter()
            .enumerate()
            .filter(|(_, module)| text_defines(&module.text, name))
            .map(|(index, _)| index)
            .collect();
        records.sort_by_key(|&index| {
            self.corpus
                .modules()
                .get(index)
                .map(|m| std::cmp::Reverse((m.obsoleted_by.is_empty(), m.source.recency())))
        });
        records.truncate(LIMIT);
        for &record in &records {
            self.load_corpus_module(record);
        }
        records.len()
    }

    /// Resolves the imports of every module loaded so far, loading corpus modules on demand until nothing new appears.
    pub fn resolve_all(&mut self) {
        while self.resolved_up_to < self.modules.len() {
            let index = self.resolved_up_to;
            self.resolved_up_to = index.saturating_add(1);
            self.resolve_module(ModuleId(u32::try_from(index).unwrap_or(u32::MAX)));
        }
    }

    fn resolve_module(&mut self, id: ModuleId) {
        let clause_count = self.module(id).map_or(0, |m| m.model.imports.len());
        let needs_evaluation = |graph: &Self, clause: usize| {
            graph
                .module(id)
                .and_then(|m| m.model.imports.get(clause))
                .is_some_and(|c| matches!(c.assigned, Some((AssignedId::Defined { .. }, _))))
        };
        let (simple, deferred): (Vec<usize>, Vec<usize>) =
            (0..clause_count).partition(|&clause| !needs_evaluation(self, clause));
        for clause in simple.into_iter().chain(deferred) {
            let resolved = self.resolve_clause(id, clause);
            if let Some(slot) = self
                .modules
                .get_mut(usize::try_from(id.0).unwrap_or(usize::MAX))
                .and_then(|m| m.imports.get_mut(clause))
            {
                *slot = resolved;
            }
        }
    }

    // X.680 13.9-13.12, 13.16 f; design spec "Import resolution order"
    fn resolve_clause(&mut self, from: ModuleId, clause_index: usize) -> ResolvedImport {
        let Some(importer) = self.module(from) else {
            return ResolvedImport::unresolved();
        };
        let Some(clause) = importer.model.imports.get(clause_index).cloned() else {
            return ResolvedImport::unresolved();
        };
        let importer_origin = importer.origin;
        let importer_oid = importer.oid.clone();
        let importer_name = importer.model.name.clone();
        let importer_document = match importer_origin {
            Origin::Corpus { record } => self
                .corpus
                .modules()
                .get(record)
                .map(|r| r.source.document.clone()),
            Origin::Draft => self.options.home_document.clone(),
            Origin::Builtin => None,
        };

        let assigned_oid = match &clause.assigned {
            Some((AssignedId::Oid { group }, _)) => {
                Evaluator::new(self).eval_oid_group(from, group).oid
            }
            Some((AssignedId::Defined { module, name, .. }, _)) => {
                Evaluator::new(self).oid_of_name(from, module.as_deref(), name)
            }
            None => None,
        };

        if clause.module_name == importer_name
            || (assigned_oid.is_some() && assigned_oid == importer_oid)
        {
            return ResolvedImport {
                target: Some(from),
                matched: Matched::SelfImport,
                assigned_oid,
                ..ResolvedImport::unresolved()
            };
        }

        if let Some(pin) = self.options.pins.get(&clause.module_name)
            && matches!(importer_origin, Origin::Draft)
            && let Some(record) = self.pinned_record(&clause.module_name, pin)
        {
            let target = self.load_corpus_module(record);
            return ResolvedImport {
                target,
                matched: Matched::Pin,
                assigned_oid,
                ..ResolvedImport::unresolved()
            };
        }

        if let Some(oid) = &assigned_oid {
            let found = match clause.selection {
                None => self.find_exact_oid(oid),
                Some((Selection::Successors, _)) => self.find_successor(oid),
                Some((Selection::Descendants, _)) => self.find_descendant(oid),
            };
            match found {
                Some(Candidate::Draft(target)) => {
                    return ResolvedImport {
                        target: Some(target),
                        matched: if clause.selection.is_some() {
                            Matched::Selection
                        } else {
                            Matched::Oid
                        },
                        assigned_oid,
                        ..ResolvedImport::unresolved()
                    };
                }
                Some(Candidate::Corpus(record)) => {
                    let target = self.load_corpus_module(record);
                    return ResolvedImport {
                        target,
                        matched: if clause.selection.is_some() {
                            Matched::Selection
                        } else {
                            Matched::Oid
                        },
                        assigned_oid,
                        ..ResolvedImport::unresolved()
                    };
                }
                None => {}
            }
        }

        let oid_mismatch = assigned_oid.is_some();
        if matches!(importer_origin, Origin::Draft) {
            let draft = self
                .modules
                .iter()
                .find(|m| m.is_draft() && m.model.name == clause.module_name && m.id != from);
            if let Some(module) = draft {
                return ResolvedImport {
                    target: Some(module.id),
                    matched: Matched::DraftName,
                    assigned_oid,
                    oid_mismatch,
                    ..ResolvedImport::unresolved()
                };
            }
        }
        let candidates: Vec<usize> = self
            .corpus
            .by_name(&clause.module_name)
            .iter()
            .copied()
            .filter(|&index| {
                self.corpus
                    .modules()
                    .get(index)
                    .is_some_and(ModuleRecord::resolvable_by_name)
            })
            .collect();
        let same_document = importer_document.and_then(|document| {
            candidates.iter().copied().find(|&index| {
                self.corpus
                    .modules()
                    .get(index)
                    .is_some_and(|m| m.source.document == document)
            })
        });
        let chosen = same_document.or_else(|| preferred(self.corpus, &candidates));
        match chosen {
            Some(record) => {
                let target = self.load_corpus_module(record);
                ResolvedImport {
                    target,
                    matched: Matched::NameOnly,
                    assigned_oid,
                    candidates,
                    oid_mismatch,
                }
            }
            None => ResolvedImport {
                assigned_oid,
                oid_mismatch,
                ..ResolvedImport::unresolved()
            },
        }
    }

    /// `pin` is a document id (`rfcNNNN`, `X.509`) or the dotted OID of the module. A document that defines
    /// several modules of the name needs the OID form to say which.
    fn pinned_record(&self, name: &str, pin: &str) -> Option<usize> {
        let by_name = self.corpus.by_name(name);
        let matching: Vec<usize> = if let Ok(oid) = pin.parse::<Oid>() {
            by_name
                .iter()
                .copied()
                .filter(|&index| {
                    self.corpus
                        .modules()
                        .get(index)
                        .is_some_and(|m| m.oid.as_ref() == Some(&oid))
                })
                .collect()
        } else {
            by_name
                .iter()
                .copied()
                .filter(|&index| {
                    self.corpus
                        .modules()
                        .get(index)
                        .is_some_and(|m| m.source.document.eq_ignore_ascii_case(pin))
                })
                .collect()
        };
        preferred(self.corpus, &matching)
    }

    fn find_exact_oid(&self, oid: &Oid) -> Option<Candidate> {
        self.modules
            .iter()
            .find(|m| m.is_draft() && m.oid.as_ref() == Some(oid))
            .map(|m| Candidate::Draft(m.id))
            .or_else(|| self.corpus.by_oid(oid).map(Candidate::Corpus))
    }

    // X.680 13.16 f i
    fn find_successor(&self, oid: &Oid) -> Option<Candidate> {
        let last = oid.last()?;
        let key = |candidate: &Oid| -> Option<u128> {
            let same_parent = candidate.len() == oid.len() && candidate.parent() == oid.parent();
            let arc = candidate.last()?;
            (same_parent && arc >= last).then_some(arc)
        };
        let drafts = self
            .modules
            .iter()
            .filter(|m| m.is_draft())
            .filter_map(|m| Some((key(m.oid.as_ref()?)?, Candidate::Draft(m.id))));
        let corpus = self
            .corpus
            .modules()
            .iter()
            .enumerate()
            .filter_map(|(index, m)| Some((key(m.oid.as_ref()?)?, Candidate::Corpus(index))));
        drafts
            .chain(corpus)
            .max_by_key(|(arc, _)| *arc)
            .map(|(_, c)| c)
    }

    // X.680 13.16 f ii
    fn find_descendant(&self, oid: &Oid) -> Option<Candidate> {
        let under = |candidate: &Oid| oid.is_prefix_of(candidate).then(|| candidate.clone());
        let drafts = self
            .modules
            .iter()
            .filter(|m| m.is_draft())
            .filter_map(|m| Some((under(m.oid.as_ref()?)?, Candidate::Draft(m.id))));
        let corpus = self
            .corpus
            .modules()
            .iter()
            .enumerate()
            .filter_map(|(index, m)| Some((under(m.oid.as_ref()?)?, Candidate::Corpus(index))));
        drafts
            .chain(corpus)
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, c)| c)
    }

    // ---- name lookup ------------------------------------------------------------------------

    /// Finds what the unqualified `name` refers to in `from` (X.680 13.19, 13.20).
    pub fn lookup(&self, from: ModuleId, name: &str) -> Option<Lookup> {
        let module = self.module(from)?;
        let local = module.model.local(name).first().copied();
        let imported = module.model.imported(name);
        let count = usize::from(local.is_some()).saturating_add(imported.len());
        let ambiguous = count > 1;
        if let Some(index) = local {
            return Some(Lookup {
                found: Found::Symbol {
                    module: from,
                    index,
                },
                via_clause: None,
                ambiguous,
            });
        }
        let Some(&(clause, _)) = imported.first() else {
            let builtin = self.builtin?;
            let &index = self.module(builtin)?.model.local(name).first()?;
            return Some(Lookup {
                found: Found::Symbol {
                    module: builtin,
                    index,
                },
                via_clause: None,
                ambiguous: false,
            });
        };
        Some(Lookup {
            found: self.follow_import(from, clause, name, MAX_FOLLOW),
            via_clause: Some(clause),
            ambiguous,
        })
    }

    /// Finds `module.name` in `from` (X.680 13.16 c).
    pub fn lookup_qualified(
        &self,
        from: ModuleId,
        module_name: &str,
        name: &str,
    ) -> Result<Lookup, QualifiedError> {
        let module = self.module(from).ok_or(QualifiedError::ModuleNotImported)?;
        let clause = module
            .model
            .clause_by_module_name(module_name)
            .ok_or(QualifiedError::ModuleNotImported)?;
        let listed = module
            .model
            .imports
            .get(clause)
            .is_some_and(|c| c.symbols.iter().any(|s| s.name == name));
        if !listed {
            return Err(QualifiedError::SymbolNotImported);
        }
        Ok(Lookup {
            found: self.follow_import(from, clause, name, MAX_FOLLOW),
            via_clause: Some(clause),
            ambiguous: false,
        })
    }

    fn follow_import(&self, from: ModuleId, clause: usize, name: &str, budget: usize) -> Found {
        let Some(target) = self
            .module(from)
            .and_then(|m| m.imports.get(clause))
            .and_then(|r| r.target)
        else {
            return Found::Opaque;
        };
        self.find_in_module(target, name, budget)
    }

    /// Finds `name` as defined by `module`, or imported by it exactly once and not defined (X.680 13.16 a).
    pub fn find_in_module(&self, module: ModuleId, name: &str, budget: usize) -> Found {
        let Some(loaded) = self.module(module) else {
            return Found::MissingInTarget;
        };
        if let Some(&index) = loaded.model.local(name).first() {
            return Found::Symbol { module, index };
        }
        match (loaded.model.imported(name), budget.checked_sub(1)) {
            ([(clause, _)], Some(rest)) => self.follow_import(module, *clause, name, rest),
            _ => Found::MissingInTarget,
        }
    }

    /// Whether the assignment is a value assignment. `lower Governor ::= { ... }` is a value when the governor is a
    /// type and an information object when it is a class (X.681 11.1).
    pub fn is_value_symbol(&self, module: ModuleId, index: usize) -> bool {
        let Some(symbol) = self.symbol(module, index) else {
            return false;
        };
        match symbol.kind {
            crate::model::SymbolKind::Value => true,
            crate::model::SymbolKind::ObjectOrValue => symbol
                .type_node
                .as_ref()
                .is_some_and(|governor| self.peel_type(module, governor).is_some()),
            _ => false,
        }
    }

    /// Follows type references until something that is not a plain `DefinedType`, looking through constraints and
    /// encoding prefixes but not through tags. Returns the module and node in which the final type is written.
    pub fn peel_type(&self, module: ModuleId, node: &SyntaxNode) -> Option<Peeled> {
        self.peel(module, node, false, MAX_FOLLOW)
    }

    /// Like [`ModuleGraph::peel_type`] but also looks through tags.
    pub fn peel_type_through_tags(&self, module: ModuleId, node: &SyntaxNode) -> Option<Peeled> {
        self.peel(module, node, true, MAX_FOLLOW)
    }

    fn peel(
        &self,
        module: ModuleId,
        node: &SyntaxNode,
        through_tags: bool,
        budget: usize,
    ) -> Option<Peeled> {
        let rest = budget.checked_sub(1)?;
        match node.kind() {
            SyntaxKind::ConstrainedType | SyntaxKind::EncodingPrefixedType => {
                let inner = type_child(node)?;
                self.peel(module, &inner, through_tags, rest)
            }
            SyntaxKind::TaggedType if through_tags => {
                let inner = type_child(node)?;
                self.peel(module, &inner, through_tags, rest)
            }
            SyntaxKind::DefinedType => {
                let reference = defined_type_ref(node)?;
                let lookup = match &reference.module {
                    Some(qualifier) => self
                        .lookup_qualified(module, qualifier, &reference.name)
                        .ok()?,
                    None => self.lookup(module, &reference.name)?,
                };
                match lookup.found {
                    Found::Symbol {
                        module: target,
                        index,
                    } => {
                        let symbol = self.symbol(target, index)?;
                        if symbol.is_parameterized()
                            || symbol.kind != crate::model::SymbolKind::Type
                        {
                            return None;
                        }
                        self.peel(target, symbol.type_node.as_ref()?, through_tags, rest)
                    }
                    _ => None,
                }
            }
            _ => Some(Peeled {
                module,
                node: node.clone(),
            }),
        }
    }
}

/// The result of following a type through references.
#[derive(Debug, Clone)]
pub struct Peeled {
    pub module: ModuleId,
    pub node: SyntaxNode,
}

enum Candidate {
    Draft(ModuleId),
    Corpus(usize),
}

/// Whether `text` seems to define `name`: the name stands alone and `::=` follows soon, with nothing in between that
/// ends an assignment. A cheap test made before parsing.
fn text_defines(text: &str, name: &str) -> bool {
    const REACH: usize = 200;
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '-';
    text.match_indices(name).any(|(start, _)| {
        let before = text.get(..start).and_then(|t| t.chars().next_back());
        let after = text
            .get(start.saturating_add(name.len())..)
            .unwrap_or_default();
        let standalone = before.is_none_or(|c| !is_name_char(c))
            && after.chars().next().is_none_or(|c| !is_name_char(c));
        let window: String = after.chars().take(REACH).collect();
        standalone
            && window.find("::=").is_some_and(|at| {
                window
                    .get(..at)
                    .is_some_and(|gap| !gap.contains(';') && !gap.contains("\n\n"))
            })
    })
}
