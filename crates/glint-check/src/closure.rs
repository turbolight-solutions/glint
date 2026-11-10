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
//! - `renumber`: an object identifier whose last arc has no number (`TBD`, `TBDMOD`, `name(TBD)`) gets one. Arcs are
//!   keyed by what they identify: a module (its header and every `FROM` clause that imports it) or a value (`id-x
//!   OBJECT IDENTIFIER ::= { ... }`). When the set or the corpus gives the key a number elsewhere, that number is
//!   copied; when it gives two different numbers, the closure fails; otherwise numbers are assigned from
//!   [`FIRST_AUTO_ARC`] up, in key order. Only modules of Internet-Drafts and input texts are renumbered: an arc
//!   with no number in a published document (an RFC) is left as written, with a warning, since only a correction of
//!   the document (an RFC erratum) can number it.
//! - `builtin_strings`: a 1988-syntax module's own definition of `UniversalString`, `BMPString` or `UTF8String` with
//!   the type's universal tag is commented out, with the imports of it, since these are built-in types in current
//!   ASN.1 (see [`glint_sema::legacy_string_definition`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use glint_corpus::{CorpusProvider, Oid, unique_label};
use glint_sema::{
    AssignedId, Evaluator, Matched, ModuleGraph, ModuleId, Options, Origin, PlaceholderArc,
    placeholder_arc,
};
use glint_syntax::{ParseOptions, SyntaxKind, SyntaxNode, TextRange, TextSize, parse_source_with};

/// A definition of a built-in string type that the closure commented out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinDefinition {
    /// The module, by its name in the closure.
    pub module: String,
    pub name: String,
}

/// An import of a commented-out definition that the closure commented out too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinImport {
    /// The importing module, by its name in the closure.
    pub importer: String,
    pub name: String,
    /// The module it was imported from, by its name in the closure.
    pub from: String,
}

/// The rewrites to make; with none, every module is written as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClosureOptions {
    /// Give modules of one name names of their own, and rename imports to the module their identifier resolves to.
    pub rename: bool,
    /// Give object identifier arcs that have no number one.
    pub renumber: bool,
    /// Comment out 1988 definitions of built-in string types, and the imports of them.
    pub builtin_strings: bool,
}

/// Something in the closure that a compiler which finds imports by name may reject, left as written because the
/// option that fixes it was not given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// An import resolves by its identifier to a module of another name (fixed by `rename`).
    ImportName(Rename),
    /// An object identifier arc has no number (fixed by `renumber`): the modules, each with what is written.
    Placeholder {
        key: OidKey,
        sites: Vec<(String, String)>,
    },
    /// An object identifier arc has no number in a published document, which no option numbers: only a correction
    /// of the document (an RFC erratum) can. The modules, each with what is written.
    PublishedPlaceholder {
        key: OidKey,
        document: String,
        sites: Vec<(String, String)>,
    },
    /// A 1988 definition of a built-in string type (fixed by `builtin_strings`).
    BuiltinString(BuiltinDefinition),
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
            Self::Placeholder { key, sites } => write!(
                f,
                "OID: {key} has no number for its last arc in {}",
                written_sites(sites)
            ),
            Self::PublishedPlaceholder {
                key,
                document,
                sites,
            } => write!(
                f,
                "OID: {key} has no number for its last arc in {}, as published in {document}",
                written_sites(sites)
            ),
            Self::BuiltinString(definition) => write!(
                f,
                "{}: defines {}, a built-in type in current ASN.1, which a compiler of current ASN.1 rejects",
                definition.module, definition.name
            ),
        }
    }
}

/// `Module (TBD), Other (TBD1)`.
fn written_sites(sites: &[(String, String)]) -> String {
    let sites: Vec<String> = sites
        .iter()
        .map(|(module, written)| format!("{module} ({written})"))
        .collect();
    sites.join(", ")
}

/// The first number given to an arc that has none anywhere.
pub const FIRST_AUTO_ARC: u128 = 10_000;

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

/// What an object identifier arc identifies.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OidKey {
    Module(String),
    Value(String),
}

impl fmt::Display for OidKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module(name) => write!(f, "module {name}"),
            Self::Value(name) => write!(f, "value {name}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArcOrigin {
    /// Taken from where the key already has this number.
    Copied {
        from: String,
    },
    AutoAssigned,
}

/// A number given to the placeholder arcs of one key.
#[derive(Debug, Clone)]
pub struct ArcFill {
    pub key: OidKey,
    pub arc: u128,
    pub origin: ArcOrigin,
    /// The modules whose text was changed, each with what the author had written.
    pub sites: Vec<(String, String)>,
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
    /// Definitions of built-in string types commented out ([`glint_sema::legacy_string_definition`]).
    pub builtin_definitions: Vec<BuiltinDefinition>,
    /// Imports of those definitions commented out.
    pub builtin_imports: Vec<BuiltinImport>,
    pub renames: Vec<Rename>,
    pub arcs: Vec<ArcFill>,
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
    /// The key has two different numbers in the set or the corpus.
    ArcConflict {
        key: OidKey,
        numbers: Vec<(u128, Vec<String>)>,
    },
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
            Self::ArcConflict { key, numbers } => {
                let listed: Vec<String> = numbers
                    .iter()
                    .map(|(arc, sources)| format!("{arc} in {}", sources.join(", ")))
                    .collect();
                write!(
                    f,
                    "{key} has a placeholder arc and conflicting numbers: {}",
                    listed.join("; ")
                )
            }
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
    // The document of each module of a published document (not an Internet-Draft); input texts are not.
    let mut published: HashMap<ModuleId, String> = HashMap::new();
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
            if !module.source.is_draft() {
                published.insert(id, module.source.document.clone());
            }
            labels.insert(id, label.clone());
            roots.push(id);
        }
    }
    graph.resolve_all();
    redirect_to_inputs(&mut graph, &roots);
    for module in &graph.modules {
        if let Origin::Corpus { record } = module.origin
            && let Some(record) = corpus.modules().get(record)
            && !record.source.is_draft()
        {
            published.insert(module.id, record.source.document.clone());
        }
    }

    let walk = Walk::new(&graph, &roots);
    problems.extend(walk.missing(&graph));
    let mut warnings = Vec::new();
    let (sites, kept) = split_published(placeholder_sites(&graph, &walk.order), &published);
    warnings.extend(kept.into_iter().map(|((key, document), sites)| {
        Warning::PublishedPlaceholder {
            key,
            document,
            sites: sites
                .iter()
                .map(|(module, placeholder)| {
                    (module_name(&graph, *module), placeholder.written.clone())
                })
                .collect(),
        }
    }));
    let fills = if rewrites.renumber {
        match fill_arcs(&graph, &walk.order, &labels, sites) {
            Ok(fills) => fills,
            Err(conflicts) => {
                problems.extend(conflicts);
                Vec::new()
            }
        }
    } else {
        warnings.extend(sites.into_iter().map(|(key, sites)| {
            Warning::Placeholder {
                key,
                sites: sites
                    .iter()
                    .map(|(module, placeholder)| {
                        (module_name(&graph, *module), placeholder.written.clone())
                    })
                    .collect(),
            }
        }));
        Vec::new()
    };
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
    let mut arcs: Vec<ArcFill> = Vec::new();
    for fill in fills {
        let mut sites = Vec::new();
        for (module, placeholder) in &fill.sites {
            edits
                .entry(*module)
                .or_default()
                .push((placeholder.range, placeholder.replacement(fill.arc)));
            sites.push((module_name(&graph, *module), placeholder.written.clone()));
        }
        arcs.push(ArcFill {
            key: fill.key,
            arc: fill.arc,
            origin: fill.origin,
            sites,
        });
    }
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
    let (mut builtin_definitions, mut builtin_imports) = builtin_strings(
        &graph,
        &walk.order,
        &names,
        if rewrites.builtin_strings {
            &mut edits
        } else {
            &mut dropped
        },
    );
    if !rewrites.builtin_strings {
        warnings.extend(builtin_definitions.drain(..).map(Warning::BuiltinString));
        builtin_imports.clear();
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
        builtin_definitions,
        builtin_imports,
        renames,
        arcs,
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

/// Comments out each 1988 definition of a built-in string type, and each import of such a definition.
///
/// X.208 (1988) had no `UniversalString`, `BMPString` or `UTF8String`, so modules that keep to 1988 syntax define
/// them by hand with the universal tag a later edition gave the type, as RFC 5280's `PKIX1Explicit88` does
/// (`UniversalString ::= [UNIVERSAL 28] IMPLICIT OCTET STRING`). Under current ASN.1 the names are reserved words
/// (X.680 12.38, 12.2.2) and a user module may not use universal tags (31.2.5), so a compiler of current ASN.1
/// rejects the definition; the built-in type is encoded alike. Exactly that form
/// ([`glint_sema::legacy_string_definition`]) is commented out, in any module of the closure, as RFC 5280's
/// `PKIX1Implicit88` asks a reader to do "if 'new' types are supported"; any other definition of these names is
/// left for the compiler. An import of it goes too: the imported symbol, or the whole `SymbolsFromModule` when nothing else is
/// imported from that module (`IMPORTS ;` is valid, 13.1). Uses `/* */` comments (12.6.4), which may span lines
/// and contain `--`, so nothing of the author's text is lost. Edits are insertions, so they compose with the
/// renames made inside a commented-out import.
fn builtin_strings(
    graph: &ModuleGraph<'_>,
    order: &[ModuleId],
    names: &HashMap<ModuleId, String>,
    edits: &mut HashMap<ModuleId, Vec<(TextRange, String)>>,
) -> (Vec<BuiltinDefinition>, Vec<BuiltinImport>) {
    let final_name = |id: ModuleId| {
        names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| module_name(graph, id))
    };
    let mut removed: HashMap<ModuleId, Vec<String>> = HashMap::new();
    let mut definitions = Vec::new();
    for &id in order {
        let Some(module) = graph.module(id) else {
            continue;
        };
        for symbol in &module.model.symbols {
            let Some(name) = glint_sema::legacy_string_definition(&symbol.node) else {
                continue;
            };
            let text = symbol.node.text().to_string();
            if text.contains("/*") || text.contains("*/") {
                continue;
            }
            let range = symbol.node.text_range();
            comment_out(
                edits.entry(id).or_default(),
                range,
                "built in since ASN.1:1994; see the header of combined.asn",
            );
            removed.entry(id).or_default().push(name.to_owned());
            definitions.push(BuiltinDefinition {
                module: final_name(id),
                name: name.to_owned(),
            });
        }
    }
    let mut imports = Vec::new();
    for &id in order {
        let Some(module) = graph.module(id) else {
            continue;
        };
        for (clause, resolved) in module.model.imports.iter().zip(&module.imports) {
            let Some(gone) = resolved.target.and_then(|t| removed.get(&t)) else {
                continue;
            };
            let drop: Vec<bool> = clause
                .symbols
                .iter()
                .map(|s| gone.contains(&s.name))
                .collect();
            if !drop.contains(&true) {
                continue;
            }
            let from = resolved.target.map(final_name).unwrap_or_default();
            for (symbol, _) in clause.symbols.iter().zip(&drop).filter(|(_, d)| **d) {
                imports.push(BuiltinImport {
                    importer: final_name(id),
                    name: symbol.name.clone(),
                    from: from.clone(),
                });
            }
            let changes = edits.entry(id).or_default();
            let Some(last_kept) = drop.iter().rposition(|d| !d) else {
                comment_out(changes, clause.range, "a built-in type in current ASN.1");
                continue;
            };
            // Each run of dropped symbols before the last one kept, `X, Y,` through the comma after it, is one
            // comment, so no two comments meet.
            let mut index = 0;
            while index < last_kept {
                if !drop.get(index).copied().unwrap_or(false) {
                    index = index.saturating_add(1);
                    continue;
                }
                let first = index;
                while drop.get(index).copied().unwrap_or(false) {
                    index = index.saturating_add(1);
                }
                let last = index.checked_sub(1).and_then(|i| clause.symbols.get(i));
                if let (Some(start), Some(last)) = (clause.symbols.get(first), last) {
                    let end = comma_after(module.node.syntax(), last.range.end())
                        .or_else(|| clause.symbols.get(index).map(|next| next.range.start()))
                        .unwrap_or(last.range.end());
                    comment_out(changes, TextRange::new(start.range.start(), end), "");
                }
            }
            // `, X, Y` after the last symbol kept.
            if let (Some(kept), Some(last)) = (clause.symbols.get(last_kept), clause.symbols.last())
                && last_kept.saturating_add(1) < clause.symbols.len()
            {
                comment_out(
                    changes,
                    TextRange::new(kept.range.end(), last.range.end()),
                    "",
                );
            }
        }
    }
    (definitions, imports)
}

/// The end of the comma that is the first token after `offset`, if it is one.
fn comma_after(node: &SyntaxNode, offset: TextSize) -> Option<TextSize> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|t| !t.kind().is_trivia())
        .find(|t| t.text_range().start() >= offset)
        .filter(|t| t.kind() == SyntaxKind::Comma)
        .map(|t| t.text_range().end())
}

/// Wraps `range` in a `/* */` comment by two insertions, with `note` after it as a `--` comment.
fn comment_out(changes: &mut Vec<(TextRange, String)>, range: TextRange, note: &str) {
    changes.push((TextRange::empty(range.start()), "/* ".to_owned()));
    let end = if note.is_empty() {
        " */".to_owned()
    } else {
        format!(" */ -- {note} --")
    };
    changes.push((TextRange::empty(range.end()), end));
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

struct Fill {
    key: OidKey,
    arc: u128,
    origin: ArcOrigin,
    sites: Vec<(ModuleId, PlaceholderArc)>,
}

/// The `{ ... }` of an `OBJECT IDENTIFIER` value assignment.
fn brace_group(value: &SyntaxNode) -> Option<SyntaxNode> {
    value
        .children()
        .find(|c| c.kind() == SyntaxKind::BraceGroup)
}

type Sites = BTreeMap<OidKey, Vec<(ModuleId, PlaceholderArc)>>;

/// Sites in modules of published documents, by key and document.
type PublishedSites = BTreeMap<(OidKey, String), Vec<(ModuleId, PlaceholderArc)>>;

/// Every placeholder arc of the closure, keyed.
fn placeholder_sites(graph: &ModuleGraph<'_>, order: &[ModuleId]) -> Sites {
    let evaluator = Evaluator::new(graph);
    let mut sites = Sites::new();
    for &id in order {
        let Some(module) = graph.module(id) else {
            continue;
        };
        if let Some(arc) = module.model.oid_group.as_ref().and_then(placeholder_arc) {
            sites
                .entry(OidKey::Module(module.model.name.clone()))
                .or_default()
                .push((id, arc));
        }
        for (clause, resolved) in module.model.imports.iter().zip(&module.imports) {
            let Some((AssignedId::Oid { group }, _)) = &clause.assigned else {
                continue;
            };
            if let Some(arc) = placeholder_arc(group) {
                let name = resolved
                    .target
                    .map_or_else(|| clause.module_name.clone(), |t| module_name(graph, t));
                sites
                    .entry(OidKey::Module(name))
                    .or_default()
                    .push((id, arc));
            }
        }
        for (index, symbol) in module.model.symbols.iter().enumerate() {
            if !evaluator.symbol_is_oid_valued(id, index) {
                continue;
            }
            if let Some(arc) = symbol
                .value_node
                .as_ref()
                .and_then(brace_group)
                .as_ref()
                .and_then(placeholder_arc)
            {
                sites
                    .entry(OidKey::Value(symbol.name.clone()))
                    .or_default()
                    .push((id, arc));
            }
        }
    }
    sites
}

/// `sites` without the sites in modules of published documents, and those sites by key and document.
fn split_published(sites: Sites, published: &HashMap<ModuleId, String>) -> (Sites, PublishedSites) {
    let mut open = Sites::new();
    let mut kept = PublishedSites::new();
    for (key, key_sites) in sites {
        for (module, arc) in key_sites {
            match published.get(&module) {
                Some(document) => kept
                    .entry((key.clone(), document.clone()))
                    .or_default()
                    .push((module, arc)),
                None => open.entry(key.clone()).or_default().push((module, arc)),
            }
        }
    }
    (open, kept)
}

/// The number each key of `sites` gets.
fn fill_arcs(
    graph: &ModuleGraph<'_>,
    order: &[ModuleId],
    labels: &HashMap<ModuleId, String>,
    sites: Sites,
) -> Result<Vec<Fill>, Vec<Problem>> {
    if sites.is_empty() {
        return Ok(Vec::new());
    }
    let evaluator = Evaluator::new(graph);
    let corpus_values = corpus_value_arcs(graph.corpus());
    let mut fills = Vec::new();
    let mut conflicts = Vec::new();
    let mut next = FIRST_AUTO_ARC;
    for (key, key_sites) in sites {
        let numbers = known_arcs(graph, &evaluator, order, labels, &corpus_values, &key);
        let (arc, origin) = match numbers.len() {
            0 => {
                let arc = next;
                next = next.saturating_add(1);
                (arc, ArcOrigin::AutoAssigned)
            }
            1 => {
                let Some((&arc, sources)) = numbers.iter().next() else {
                    continue;
                };
                let from = sources.first().cloned().unwrap_or_default();
                (arc, ArcOrigin::Copied { from })
            }
            _ => {
                conflicts.push(Problem::ArcConflict {
                    key,
                    numbers: numbers.into_iter().collect(),
                });
                continue;
            }
        };
        fills.push(Fill {
            key,
            arc,
            origin,
            sites: key_sites,
        });
    }
    if conflicts.is_empty() {
        Ok(fills)
    } else {
        Err(conflicts)
    }
}

/// The last arc of every value assignment in the corpus, by value name, with where it is made.
fn corpus_value_arcs(corpus: &dyn CorpusProvider) -> HashMap<String, Vec<(u128, String)>> {
    let mut arcs: HashMap<String, Vec<(u128, String)>> = HashMap::new();
    for root in 0..=2 {
        for assignment in corpus.assignments_under(&Oid::new(vec![root])) {
            if let Some(last) = assignment.oid.last() {
                arcs.entry(assignment.name.clone()).or_default().push((
                    last,
                    format!("{}#{}", assignment.document, assignment.module),
                ));
            }
        }
    }
    arcs
}

/// Every number the set or the corpus gives `key` as its last arc, with where.
fn known_arcs(
    graph: &ModuleGraph<'_>,
    evaluator: &Evaluator<'_, '_>,
    order: &[ModuleId],
    labels: &HashMap<ModuleId, String>,
    corpus_values: &HashMap<String, Vec<(u128, String)>>,
    key: &OidKey,
) -> BTreeMap<u128, Vec<String>> {
    let mut numbers: BTreeMap<u128, Vec<String>> = BTreeMap::new();
    let mut add = |arc: Option<u128>, source: String| {
        if let Some(arc) = arc {
            let sources = numbers.entry(arc).or_default();
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
    };
    match key {
        OidKey::Module(name) => {
            for &id in order {
                let Some(module) = graph.module(id) else {
                    continue;
                };
                if &module.model.name == name {
                    add(
                        module.oid.as_ref().and_then(Oid::last),
                        source_label(graph, id, labels),
                    );
                }
                for (clause, resolved) in module.model.imports.iter().zip(&module.imports) {
                    let imported = resolved
                        .target
                        .map_or_else(|| clause.module_name.clone(), |t| module_name(graph, t));
                    if &imported == name {
                        add(
                            resolved.assigned_oid.as_ref().and_then(Oid::last),
                            format!("IMPORTS of {}", source_label(graph, id, labels)),
                        );
                    }
                }
            }
            let corpus = graph.corpus();
            for &record in corpus.by_name(name) {
                if let Some(module) = corpus.modules().get(record) {
                    add(
                        module.oid.as_ref().and_then(Oid::last),
                        unique_label(corpus, module),
                    );
                }
            }
        }
        OidKey::Value(name) => {
            for &id in order {
                for (value, oid) in evaluator.oid_assignments(id) {
                    if &value == name {
                        add(oid.last(), source_label(graph, id, labels));
                    }
                }
            }
            for (arc, source) in corpus_values.get(name).into_iter().flatten() {
                add(Some(*arc), source.clone());
            }
        }
    }
    numbers
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
