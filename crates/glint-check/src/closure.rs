//! The import closure of a set of modules: every module they need, resolved as `glint check` resolves imports
//! (X.680 13.9-13.12, 13.16 f), in dependency order, for a compiler that reads one module per file and looks imports
//! up by name.
//!
//! The modules are written as they are, except for the rewrites [`ClosureOptions`] asks for. What such a compiler
//! cannot take and no option fixes is reported: two modules of one name are a [`Problem`], since one file per name
//! cannot hold them; anything else is a [`Warning`]. The rewrites, each reported:
//! - `rename`: two modules of one name that the set needs (RFC 5280's and RFC 2459's `PKIX1Explicit88`) get names
//!   of their own, and an import that resolves (by its object identifier) to a module of another name is renamed to
//!   that module, in the `FROM` clause and in `Old.symbol` references.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use glint_corpus::{CorpusProvider, Oid, unique_label};
use glint_sema::{AssignedId, Matched, ModuleGraph, ModuleId, Options, Origin};
use glint_syntax::{ParseOptions, SyntaxKind, TextRange, parse_source_with};

/// The rewrites to make; with none, every module is written as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClosureOptions {
    /// Give modules of one name names of their own, and rename imports to the module their identifier resolves to.
    pub rename: bool,
}

/// Something in the closure that a compiler which finds imports by name may reject, left as written because the
/// option that fixes it was not given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// An import resolves by its identifier to a module of another name (fixed by `rename`).
    ImportName(Rename),
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImportName(rename) => write!(
                f,
                "{}: imports {}, which resolves by its identifier to {}; a compiler that finds imports by name \
                 will not find it",
                rename.importer, rename.from, rename.to
            ),
        }
    }
}

/// A module to start the closure from.
#[derive(Debug, Clone)]
pub enum ClosureInput {
    /// Module text, and what to call it in reports. It takes the place of a corpus module with its identifier.
    Text { label: String, text: String },
    /// A corpus module, by its index in the corpus modules.
    Corpus { record: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Input,
    Dependency,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Dependency => "dependency",
        }
    }
}

/// One module of the closure, rewritten.
#[derive(Debug, Clone)]
pub struct ClosureModule {
    pub name: String,
    pub role: Role,
    /// The input label, or the corpus label (`rfc5912#PKIX1Explicit-2009`).
    pub source: String,
    pub text: String,
}

/// An import renamed to the module its identifier resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub importer: String,
    pub from: String,
    pub to: String,
}

/// A module given a new name because the closure needs another module of its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRename {
    /// The corpus label or input label of the module.
    pub source: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone)]
pub struct Closure {
    /// Dependencies before the modules that import them.
    pub modules: Vec<ClosureModule>,
    /// Modules renamed so that every module of the closure has its own name.
    pub module_renames: Vec<ModuleRename>,
    pub renames: Vec<Rename>,
    /// Import cycles, as module names; the order within a cycle is approximate.
    pub cycles: Vec<Vec<String>>,
    /// What was left as written that a compiler may reject.
    pub warnings: Vec<Warning>,
}

/// Why the closure is not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The input holds no module definition.
    NoModule { input: String },
    /// No module answers an import.
    Missing {
        name: String,
        identifier: Option<String>,
        importers: Vec<String>,
    },
    /// The set needs two or more modules of one name (fixed by `rename`): their sources.
    SameName { name: String, sources: Vec<String> },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoModule { input } => write!(f, "{input}: no module definition"),
            Self::Missing {
                name,
                identifier,
                importers,
            } => {
                write!(f, "{name} (not found")?;
                if let Some(identifier) = identifier {
                    write!(f, "; FROM {identifier}")?;
                }
                write!(f, "; imported by {})", importers.join(", "))
            }
            Self::SameName { name, sources } => write!(
                f,
                "the set needs {} modules named {name} ({}), and a compiler that finds modules by name takes one",
                sources.len(),
                sources.join(", ")
            ),
        }
    }
}

/// Resolves the imports of `inputs` against `corpus` and returns every module needed, with the rewrites `rewrites`
/// asks for, or every reason the set cannot be written. `pins` are `Name -> rfcNNNN | dotted OID`, as for `glint
/// check --pin`.
pub fn closure(
    inputs: &[ClosureInput],
    corpus: &dyn CorpusProvider,
    pins: &[(String, String)],
    rewrites: ClosureOptions,
) -> Result<Closure, Vec<Problem>> {
    let options = Options {
        pins: pins.iter().cloned().collect(),
        oid_collisions: false,
        resolution_notes: false,
        policy_checks: false,
        oid_labels: false,
        ..Options::default()
    };
    let mut graph = ModuleGraph::new(corpus, &options);
    let mut problems = Vec::new();
    let mut labels: HashMap<ModuleId, String> = HashMap::new();
    let mut roots: Vec<ModuleId> = Vec::new();
    for input in inputs {
        let ClosureInput::Text { label, text } = input else {
            continue;
        };
        let parse = parse_source_with(text, ParseOptions::corpus());
        let ids = graph.add_draft(text.clone(), &parse);
        if ids.is_empty() {
            problems.push(Problem::NoModule {
                input: label.clone(),
            });
        }
        for id in ids {
            labels.insert(id, label.clone());
            roots.push(id);
        }
    }
    // A corpus input is added like a text input, so that it takes the place of other corpus modules with its
    // identifier, and keeps the identifier the corpus evaluated even when its header uses value references.
    for input in inputs {
        let ClosureInput::Corpus { record } = input else {
            continue;
        };
        let Some(module) = corpus.modules().get(*record) else {
            problems.push(Problem::NoModule {
                input: format!("corpus module {record}"),
            });
            continue;
        };
        let parse = parse_source_with(&module.text, ParseOptions::corpus());
        let ids = graph.add_draft(module.text.clone(), &parse);
        let label = unique_label(corpus, module);
        if ids.is_empty() {
            problems.push(Problem::NoModule {
                input: label.clone(),
            });
        }
        for id in ids {
            if let Some(loaded) = graph
                .modules
                .get_mut(usize::try_from(id.0).unwrap_or(usize::MAX))
                && loaded.model.name == module.name
                && loaded.oid.is_none()
            {
                loaded.oid.clone_from(&module.oid);
            }
            labels.insert(id, label.clone());
            roots.push(id);
        }
    }
    graph.resolve_all();
    redirect_to_inputs(&mut graph, &roots);

    let walk = Walk::new(&graph, &roots);
    problems.extend(walk.missing(&graph));
    let mut warnings = Vec::new();
    if !rewrites.rename {
        problems.extend(
            same_names(&graph, &walk.order)
                .into_iter()
                .map(|(name, ids)| Problem::SameName {
                    name,
                    sources: ids
                        .iter()
                        .map(|&id| source_label(&graph, id, &labels))
                        .collect(),
                }),
        );
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    let mut edits: HashMap<ModuleId, Vec<(TextRange, String)>> = HashMap::new();
    // Rewrites not asked for are worked out on a copy of the edits that is then dropped, to report what they would
    // change.
    let mut dropped: HashMap<ModuleId, Vec<(TextRange, String)>> = HashMap::new();
    let (names, module_renames) = if rewrites.rename {
        distinct_names(&graph, &walk.order, &roots, &labels)
    } else {
        (HashMap::new(), Vec::new())
    };
    for (&id, name) in &names {
        if let Some(module) = graph.module(id) {
            edits
                .entry(id)
                .or_default()
                .push((module.model.name_range, name.clone()));
        }
    }
    let mut renames = renames(
        &graph,
        &walk.order,
        &names,
        if rewrites.rename {
            &mut edits
        } else {
            &mut dropped
        },
    );
    if !rewrites.rename {
        warnings.extend(renames.drain(..).map(Warning::ImportName));
    }

    let root_set: HashSet<ModuleId> = roots.iter().copied().collect();
    let modules = walk
        .order
        .iter()
        .filter_map(|&id| {
            let module = graph.module(id)?;
            let unit = graph.unit(module.source)?;
            let range = module.model.range;
            let start = usize::from(range.start());
            let mut text = unit.text.get(start..usize::from(range.end()))?.to_owned();
            let mut changes = edits.remove(&id).unwrap_or_default();
            changes.sort_by_key(|(range, _)| std::cmp::Reverse(range.start()));
            for (change, replacement) in changes {
                let from = usize::from(change.start()).checked_sub(start)?;
                let to = usize::from(change.end()).checked_sub(start)?;
                text.replace_range(from..to, &replacement);
            }
            Some(ClosureModule {
                name: names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| module.model.name.clone()),
                role: if root_set.contains(&id) {
                    Role::Input
                } else {
                    Role::Dependency
                },
                source: source_label(&graph, id, &labels),
                text: text.trim().to_owned(),
            })
        })
        .collect();
    Ok(Closure {
        modules,
        module_renames,
        renames,
        cycles: walk.cycles,
        warnings,
    })
}

/// Points imports of a corpus module that an input stands for at the input: the same name and identifier, or the
/// same name when the input has no identifier. Import resolution prefers inputs only for imports made by inputs or
/// by identifier, so a corpus module that imports an input by name alone would otherwise bring in the corpus copy.
fn redirect_to_inputs(graph: &mut ModuleGraph<'_>, roots: &[ModuleId]) {
    let inputs: Vec<(String, Option<Oid>, ModuleId)> = roots
        .iter()
        .filter_map(|&id| {
            let module = graph.module(id)?;
            Some((module.model.name.clone(), module.oid.clone(), id))
        })
        .collect();
    let mut stands_for: HashMap<ModuleId, ModuleId> = HashMap::new();
    for module in &graph.modules {
        if !matches!(module.origin, Origin::Corpus { .. }) {
            continue;
        }
        let input = inputs.iter().find(|(name, oid, _)| {
            name == &module.model.name && (oid.is_none() || oid == &module.oid)
        });
        if let Some((_, _, input)) = input {
            stands_for.insert(module.id, *input);
        }
    }
    if stands_for.is_empty() {
        return;
    }
    for module in &mut graph.modules {
        for import in &mut module.imports {
            if let Some(input) = import.target.and_then(|t| stands_for.get(&t)) {
                import.target = Some(*input);
            }
        }
    }
}

fn module_name(graph: &ModuleGraph<'_>, id: ModuleId) -> String {
    graph
        .module(id)
        .map(|m| m.model.name.clone())
        .unwrap_or_default()
}

fn source_label(
    graph: &ModuleGraph<'_>,
    id: ModuleId,
    labels: &HashMap<ModuleId, String>,
) -> String {
    if let Some(label) = labels.get(&id) {
        return label.clone();
    }
    match graph.module(id).map(|m| m.origin) {
        Some(Origin::Corpus { record }) => graph
            .corpus()
            .modules()
            .get(record)
            .map(|r| unique_label(graph.corpus(), r))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// The modules reachable from the inputs, dependencies first.
struct Walk {
    order: Vec<ModuleId>,
    cycles: Vec<Vec<String>>,
}

impl Walk {
    fn new(graph: &ModuleGraph<'_>, roots: &[ModuleId]) -> Self {
        let mut walk = Self {
            order: Vec::new(),
            cycles: Vec::new(),
        };
        let mut done: HashSet<ModuleId> = HashSet::new();
        let mut stack: Vec<ModuleId> = Vec::new();
        for &root in roots {
            walk.visit(graph, root, &mut done, &mut stack);
        }
        walk
    }

    fn visit(
        &mut self,
        graph: &ModuleGraph<'_>,
        id: ModuleId,
        done: &mut HashSet<ModuleId>,
        stack: &mut Vec<ModuleId>,
    ) {
        if done.contains(&id) {
            return;
        }
        if let Some(at) = stack.iter().position(|&s| s == id) {
            let cycle = stack
                .iter()
                .skip(at)
                .map(|&m| module_name(graph, m))
                .collect();
            self.cycles.push(cycle);
            return;
        }
        stack.push(id);
        for target in targets(graph, id) {
            self.visit(graph, target, done, stack);
        }
        stack.pop();
        if done.insert(id) {
            self.order.push(id);
        }
    }

    fn missing(&self, graph: &ModuleGraph<'_>) -> Vec<Problem> {
        let mut missing: BTreeMap<String, (Option<String>, Vec<String>)> = BTreeMap::new();
        for &id in &self.order {
            let Some(module) = graph.module(id) else {
                continue;
            };
            for (clause, resolved) in module.model.imports.iter().zip(&module.imports) {
                if resolved.target.is_some() {
                    continue;
                }
                let identifier = clause
                    .assigned
                    .as_ref()
                    .map(|(assigned, _)| match assigned {
                        AssignedId::Oid { group } => squash(&group.text().to_string()),
                        AssignedId::Defined { name, .. } => name.clone(),
                    });
                let entry = missing
                    .entry(clause.module_name.clone())
                    .or_insert_with(|| (identifier, Vec::new()));
                if !entry.1.contains(&module.model.name) {
                    entry.1.push(module.model.name.clone());
                }
            }
        }
        missing
            .into_iter()
            .map(|(name, (identifier, importers))| Problem::Missing {
                name,
                identifier,
                importers,
            })
            .collect()
    }
}

/// The modules `id` imports from, other than itself and the built-in classes.
fn targets(graph: &ModuleGraph<'_>, id: ModuleId) -> Vec<ModuleId> {
    let Some(module) = graph.module(id) else {
        return Vec::new();
    };
    module
        .imports
        .iter()
        .filter(|r| r.matched != Matched::SelfImport)
        .filter_map(|r| r.target)
        .filter(|&t| t != id && Some(t) != graph.builtin())
        .collect()
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The modules of the closure that share a name with another, by name.
fn same_names(graph: &ModuleGraph<'_>, order: &[ModuleId]) -> BTreeMap<String, Vec<ModuleId>> {
    let mut by_name: BTreeMap<String, Vec<ModuleId>> = BTreeMap::new();
    for &id in order {
        by_name.entry(module_name(graph, id)).or_default().push(id);
    }
    by_name.retain(|_, ids| ids.len() > 1);
    by_name
}

/// New names for modules of the closure that share a name with another, so that a compiler that finds modules by
/// name can take them all. X.680 allows importing from two modules of one name (13.10, NOTE), but such a compiler
/// keeps one module per name. An input keeps its name when it is the only input of that name; every other module
/// of the name becomes `<Name>-<document>` (`PKIX1Explicit88-rfc2459`), or `<Name>-<document>-<OID arcs>` when the
/// document defines more than one module of the name (RFC 4998's two `ERS`), so the name never depends on order.
fn distinct_names(
    graph: &ModuleGraph<'_>,
    order: &[ModuleId],
    roots: &[ModuleId],
    labels: &HashMap<ModuleId, String>,
) -> (HashMap<ModuleId, String>, Vec<ModuleRename>) {
    let document_of = |id: ModuleId| match graph.module(id).map(|m| m.origin) {
        Some(Origin::Corpus { record }) => graph
            .corpus()
            .modules()
            .get(record)
            .map_or_else(|| "input".to_owned(), |r| r.source.document.clone()),
        _ => "input".to_owned(),
    };
    let mut taken: HashSet<String> = order.iter().map(|&id| module_name(graph, id)).collect();
    let mut names = HashMap::new();
    let mut renamed = Vec::new();
    for (name, ids) in &same_names(graph, order) {
        let inputs = ids.iter().filter(|id| roots.contains(id)).count();
        let documents: Vec<String> = ids.iter().map(|&id| document_of(id)).collect();
        for (&id, document) in ids.iter().zip(&documents) {
            if inputs == 1 && roots.contains(&id) {
                continue;
            }
            let twins = documents.iter().filter(|d| *d == document).count() > 1;
            let oid = graph
                .module(id)
                .and_then(|m| m.oid.as_ref())
                .map(ToString::to_string);
            let base = match &oid {
                Some(oid) if twins => reference(&format!("{name}-{document}-{oid}")),
                _ => reference(&format!("{name}-{document}")),
            };
            let new_name = std::iter::once(base.clone())
                .chain((2_u32..).map(|copy| format!("{base}-{copy}")))
                .find(|candidate| !taken.contains(candidate))
                .unwrap_or(base);
            taken.insert(new_name.clone());
            renamed.push(ModuleRename {
                source: source_label(graph, id, labels),
                from: name.clone(),
                to: new_name.clone(),
            });
            names.insert(id, new_name);
        }
    }
    (names, renamed)
}

/// `text` as a module reference (X.680 12.2, 12.5): letters, digits and single hyphens, not ending in a hyphen.
fn reference(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '-' };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Renames imports that resolved to a module of another name, or to a module given a new name (`names`), in the
/// `FROM` clause and in `Old.symbol` references, and returns what was renamed.
fn renames(
    graph: &ModuleGraph<'_>,
    order: &[ModuleId],
    names: &HashMap<ModuleId, String>,
    edits: &mut HashMap<ModuleId, Vec<(TextRange, String)>>,
) -> Vec<Rename> {
    let final_name = |id: ModuleId| {
        names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| module_name(graph, id))
    };
    let mut renamed = Vec::new();
    for &id in order {
        let Some(module) = graph.module(id) else {
            continue;
        };
        for (clause, resolved) in module.model.imports.iter().zip(&module.imports) {
            let Some(target) = resolved.target else {
                continue;
            };
            if resolved.matched == Matched::SelfImport {
                continue;
            }
            let real = final_name(target);
            if real == clause.module_name {
                continue;
            }
            let changes = edits.entry(id).or_default();
            changes.push((clause.module_range, real.clone()));
            let tokens: Vec<_> = module
                .node
                .syntax()
                .descendants_with_tokens()
                .filter_map(|element| element.into_token())
                .filter(|t| !t.kind().is_trivia())
                .collect();
            for (index, token) in tokens.iter().enumerate() {
                let dotted = tokens
                    .get(index.saturating_add(1))
                    .is_some_and(|next| next.kind() == SyntaxKind::Dot);
                if token.kind() == SyntaxKind::UpperName
                    && token.text() == clause.module_name
                    && dotted
                    && token.text_range() != clause.module_range
                {
                    changes.push((token.text_range(), real.clone()));
                }
            }
            renamed.push(Rename {
                importer: final_name(id),
                from: clause.module_name.clone(),
                to: real,
            });
        }
    }
    renamed
}
