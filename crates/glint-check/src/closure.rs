//! The import closure of a set of modules: every module they need, resolved as `glint check` resolves imports
//! (X.680 13.9-13.12, 13.16 f), in dependency order, for a compiler that reads one module per file and looks imports
//! up by name.
//!
//! The modules are written as they are. Two modules of one name are a [`Problem`], since such a compiler keeps one
//! module per name and one file per name cannot hold them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use glint_corpus::{CorpusProvider, Oid, unique_label};
use glint_sema::{AssignedId, Matched, ModuleGraph, ModuleId, Options, Origin};
use glint_syntax::{ParseOptions, parse_source_with};

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

/// One module of the closure.
#[derive(Debug, Clone)]
pub struct ClosureModule {
    pub name: String,
    pub role: Role,
    /// The input label, or the corpus label (`rfc5912#PKIX1Explicit-2009`).
    pub source: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Closure {
    /// Dependencies before the modules that import them.
    pub modules: Vec<ClosureModule>,
    /// Import cycles, as module names; the order within a cycle is approximate.
    pub cycles: Vec<Vec<String>>,
}

/// Why the closure cannot be written.
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
    /// The set needs two or more modules of one name: their sources.
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

/// Resolves the imports of `inputs` against `corpus` and returns every module needed, or every reason the set cannot
/// be written. `pins` are `Name -> rfcNNNN | dotted OID`, as for `glint check --pin`.
pub fn closure(
    inputs: &[ClosureInput],
    corpus: &dyn CorpusProvider,
    pins: &[(String, String)],
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
    if !problems.is_empty() {
        return Err(problems);
    }

    let root_set: HashSet<ModuleId> = roots.iter().copied().collect();
    let modules = walk
        .order
        .iter()
        .filter_map(|&id| {
            let module = graph.module(id)?;
            let unit = graph.unit(module.source)?;
            let range = module.model.range;
            let text = unit
                .text
                .get(usize::from(range.start())..usize::from(range.end()))?;
            Some(ClosureModule {
                name: module.model.name.clone(),
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
        cycles: walk.cycles,
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
