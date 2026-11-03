//! Snippet consistency: every assignment quoted in the text of a draft must say what the module says (design spec,
//! "Snippet consistency"; X.680 Annex C.3).

mod diff;
mod directives;
mod norm;

use std::collections::HashSet;

use glint_corpus::ParseStatus;
use glint_syntax::ast::{Assignment, SourceFile};
use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use crate::eval::Evaluator;
use crate::finding::{Finding, Loc, SourceId};
use crate::graph::{Found, ModuleGraph, ModuleId, Origin};
use crate::model::SymbolKind;
use crate::view::{tokens, trimmed_range};
use diff::{Difference, differences};
use norm::{Form, Normalizer, PartialMarks};

/// Settings of the snippet comparison.
#[derive(Debug, Clone, Default)]
pub struct SnippetOptions {
    /// Comments that mark the enclosing construct of a snippet as partial.
    pub elision_comments: Vec<String>,
}

/// Whether a block carries `-- glint: ignore`.
pub fn is_ignored(root: &SyntaxNode) -> bool {
    directives::scan(root, &[]).ignore
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Candidate {
    module: ModuleId,
    symbol: usize,
}

fn definitions_in(graph: &ModuleGraph<'_>, module: ModuleId, name: &str) -> Vec<Candidate> {
    graph
        .module(module)
        .map(|m| {
            m.model
                .local(name)
                .iter()
                .take(1)
                .map(|&symbol| Candidate { module, symbol })
                .collect()
        })
        .unwrap_or_default()
}

/// The modules of the draft that define `name`.
fn in_draft(graph: &ModuleGraph<'_>, name: &str) -> Vec<Candidate> {
    graph
        .modules
        .iter()
        .filter(|m| m.is_draft())
        .flat_map(|m| definitions_in(graph, m.id, name))
        .collect()
}

/// The definitions of `name` in the modules the draft imports from.
fn in_imports(graph: &ModuleGraph<'_>, name: &str) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = Vec::new();
    for module in graph.modules.iter().filter(|m| m.is_draft()) {
        for import in &module.imports {
            let Some(target) = import.target else {
                continue;
            };
            if let Found::Symbol { module, index } = graph.find_in_module(target, name, 16) {
                let candidate = Candidate {
                    module,
                    symbol: index,
                };
                if !found.contains(&candidate) {
                    found.push(candidate);
                }
            }
        }
    }
    found
}

/// The definitions of `name` in every corpus module that has been loaded and that parses without errors: a module
/// with syntax errors is too unreliable to say what a snippet should look like.
fn in_corpus(graph: &ModuleGraph<'_>, name: &str) -> Vec<Candidate> {
    graph
        .modules
        .iter()
        .filter(|m| match m.origin {
            Origin::Corpus { record } => graph.corpus().modules().get(record).is_some_and(|r| {
                matches!(r.parse_status, ParseStatus::Clean | ParseStatus::Warnings)
            }),
            _ => false,
        })
        .flat_map(|m| definitions_in(graph, m.id, name))
        .collect()
}

fn in_named_module(graph: &ModuleGraph<'_>, module_name: &str, name: &str) -> Vec<Candidate> {
    graph
        .modules
        .iter()
        .filter(|m| m.origin != Origin::Builtin && m.model.name == module_name)
        .flat_map(|m| definitions_in(graph, m.id, name))
        .collect()
}

fn module_exists(graph: &ModuleGraph<'_>, module_name: &str) -> bool {
    graph
        .modules
        .iter()
        .any(|m| m.origin != Origin::Builtin && m.model.name == module_name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Draft,
    Imports,
    /// Any corpus module: the snippet may only share a name with something unrelated.
    Corpus,
}

/// Where a snippet name is looked up: the draft, then what it imports, then the whole corpus.
fn candidates(graph: &ModuleGraph<'_>, name: &str) -> (Vec<Candidate>, Tier) {
    let draft = in_draft(graph, name);
    if !draft.is_empty() {
        return (draft, Tier::Draft);
    }
    let imported = in_imports(graph, name);
    if !imported.is_empty() {
        return (imported, Tier::Imports);
    }
    (in_corpus(graph, name), Tier::Corpus)
}

/// The names of the fragments' assignments that neither the draft nor its imports define: the corpus is searched
/// for them before the comparison starts.
pub(crate) fn names_needing_corpus(graph: &ModuleGraph<'_>, fragments: &[SourceId]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for &id in fragments {
        let Some(unit) = graph.unit(id) else {
            continue;
        };
        let Some(file) = SourceFile::cast(unit.root.clone()) else {
            continue;
        };
        for assignment in file.assignments() {
            let Some(name) = assignment.name() else {
                continue;
            };
            let name = name.text();
            if in_draft(graph, name).is_empty()
                && in_imports(graph, name).is_empty()
                && !names.iter().any(|n| n == name)
            {
                names.push(name.to_owned());
            }
        }
    }
    names
}

fn parameter_names(assignment: &SyntaxNode) -> Vec<String> {
    assignment
        .children()
        .find(|c| c.kind() == SyntaxKind::ParameterList)
        .into_iter()
        .flat_map(|list| list.children().collect::<Vec<_>>())
        .filter(|p| p.kind() == SyntaxKind::Parameter)
        .filter_map(|p| tokens(&p).into_iter().rev().find(|t| t.kind().is_name()))
        .map(|t| t.text().to_owned())
        .collect()
}

enum Outcome {
    Same,
    Equivalent(Vec<Difference>),
    Different(Vec<Difference>),
}

fn kind_word(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::TypeAssignment => "a type assignment",
        SyntaxKind::ValueAssignment => "a value assignment",
        SyntaxKind::ValueSetAssignment => "a value set assignment",
        _ => "another kind of assignment",
    }
}

struct Comparison<'a, 'c> {
    graph: &'a ModuleGraph<'c>,
    eval: &'a Evaluator<'a, 'c>,
}

impl Comparison<'_, '_> {
    // X.680 C.3.1
    fn compare(
        &self,
        snippet: &SyntaxNode,
        marks: &PartialMarks,
        candidate: Candidate,
    ) -> Option<(Outcome, TextRange, bool)> {
        let module = self.graph.module(candidate.module)?;
        let symbol = module.model.symbols.get(candidate.symbol)?;
        let module_range = trimmed_range(&symbol.node);
        if snippet.kind() != symbol.node.kind() {
            let reason = format!(
                "the snippet is {} but the module defines {}",
                kind_word(snippet.kind()),
                kind_word(symbol.node.kind())
            );
            return Some((
                Outcome::Different(vec![Difference {
                    snippet: trimmed_range(snippet),
                    module: module_range,
                    reason,
                    uncertain: false,
                }]),
                module_range,
                false,
            ));
        }
        let snippet_dummies = parameter_names(snippet);
        let no_marks = PartialMarks::default();
        let build = |form: Form, node: &SyntaxNode, marks: &PartialMarks, dummies: &[String]| {
            Normalizer {
                graph: self.graph,
                eval: self.eval,
                module: candidate.module,
                form,
                marks,
                dummies,
            }
            .assignment(node)
        };
        let snippet_normal = build(Form::Normal, snippet, marks, &snippet_dummies);
        let module_normal = build(Form::Normal, &symbol.node, &no_marks, &symbol.parameters);
        let related = same_shape(&snippet_normal, &module_normal);
        let normal = differences(&snippet_normal, &module_normal);
        if normal.iter().any(|d| !d.uncertain) {
            let certain = normal.into_iter().filter(|d| !d.uncertain).collect();
            return Some((Outcome::Different(certain), module_range, related));
        }
        if !normal.is_empty() {
            let unresolved = normal
                .into_iter()
                .map(|d| Difference {
                    reason: format!(
                        "{}; the names in it cannot be resolved in the module, so the two may be equal",
                        d.reason
                    ),
                    ..d
                })
                .collect();
            return Some((Outcome::Equivalent(unresolved), module_range, related));
        }
        let raw = differences(
            &build(Form::Raw, snippet, marks, &snippet_dummies),
            &build(Form::Raw, &symbol.node, &no_marks, &symbol.parameters),
        );
        let outcome = if raw.is_empty() {
            Outcome::Same
        } else {
            Outcome::Equivalent(raw)
        };
        Some((outcome, module_range, related))
    }
}

/// Whether two assignments define the same kind of thing: the type they stand for starts the same way.
fn same_shape(snippet: &norm::Norm, module: &norm::Norm) -> bool {
    let root = |n: &norm::Norm| {
        n.children
            .iter()
            .find(|c| c.label != "parameters")
            .map(|c| (c.label, c.atom.clone()))
    };
    root(snippet) == root(module)
}

/// The modules the draft imports from that are in neither the draft nor the corpus.
fn unavailable_imports(graph: &ModuleGraph<'_>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for module in graph.modules.iter().filter(|m| m.is_draft()) {
        for (clause, resolved) in module.model.imports.iter().zip(&module.imports) {
            if resolved.target.is_none() && !names.contains(&clause.module_name) {
                names.push(clause.module_name.clone());
            }
        }
    }
    names
}

fn module_label(graph: &ModuleGraph<'_>, module: ModuleId) -> String {
    let Some(loaded) = graph.module(module) else {
        return String::new();
    };
    match loaded.origin {
        Origin::Corpus { record } => graph
            .corpus()
            .modules()
            .get(record)
            .map_or_else(|| loaded.model.name.clone(), |r| r.label()),
        Origin::Draft | Origin::Builtin => loaded.model.name.clone(),
    }
}

fn describe(differences: &[Difference]) -> (String, Vec<String>) {
    let mut reasons = differences.iter().map(|d| d.reason.clone());
    let first = reasons.next().unwrap_or_default();
    let more: Vec<String> = reasons.map(|r| format!("also: {r}")).collect();
    (first, more)
}

/// Compares every assignment of every fragment with the module definition of the same name.
pub(crate) fn check(
    graph: &ModuleGraph<'_>,
    fragments: &[SourceId],
    options: &SnippetOptions,
    findings: &mut Vec<Finding>,
) {
    let eval = Evaluator::new(graph);
    let comparison = Comparison { graph, eval: &eval };
    for &id in fragments {
        let Some(unit) = graph.unit(id) else {
            continue;
        };
        let Some(file) = SourceFile::cast(unit.root.clone()) else {
            continue;
        };
        let found = directives::scan(&unit.root, &options.elision_comments);
        if found.ignore {
            continue;
        }
        for (word, range) in &found.unknown {
            findings.push(Finding::new(
                "W304",
                format!("unknown glint directive `{word}`"),
                Loc {
                    source: id,
                    range: *range,
                },
            ));
        }
        let marks = directives::marks(&unit.root, &found);
        let filter = found.module.as_ref().map(|(name, _)| name.as_str());
        if let Some((name, range)) = &found.module
            && !module_exists(graph, name)
        {
            findings.push(Finding::new(
                "W304",
                format!(
                    "`module={name}` names a module that is neither in the draft nor in the corpus"
                ),
                Loc {
                    source: id,
                    range: *range,
                },
            ));
            continue;
        }
        let assignments: Vec<Assignment> = file.assignments().collect();
        let starts: Vec<_> = assignments
            .iter()
            .map(|a| a.syntax().text_range().start())
            .collect();
        for (index, assignment) in assignments.iter().enumerate() {
            let start = assignment.syntax().text_range().start();
            let end = starts
                .get(index.saturating_add(1))
                .copied()
                .unwrap_or_else(|| unit.root.text_range().end());
            let region = TextRange::new(start, end);
            let broken = unit
                .syntax_errors
                .iter()
                .any(|e| region.contains_inclusive(e.range.start()));
            check_assignment(
                graph,
                &comparison,
                id,
                assignment,
                &marks,
                filter,
                findings,
                broken,
            );
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn check_assignment(
    graph: &ModuleGraph<'_>,
    comparison: &Comparison<'_, '_>,
    source: SourceId,
    assignment: &Assignment,
    marks: &PartialMarks,
    filter: Option<&str>,
    findings: &mut Vec<Finding>,
    broken: bool,
) {
    let node = assignment.syntax();
    let range = trimmed_range(node);
    if broken
        || node
            .descendants()
            .any(|n| n.kind() == SyntaxKind::ErrorNode)
    {
        return;
    }
    let Some(name_token) = assignment.name() else {
        return;
    };
    if matches!(assignment, Assignment::Macro(_) | Assignment::MacroUse(_)) {
        return;
    }
    let name = name_token.text().to_owned();
    let here = |range: TextRange| Loc { source, range };
    let (candidates, tier) = match filter {
        Some(module_name) => (in_named_module(graph, module_name, &name), Tier::Draft),
        None => candidates(graph, &name),
    };
    let results: Vec<(Candidate, Outcome, TextRange)> = candidates
        .iter()
        .filter_map(|&c| {
            comparison
                .compare(node, marks, c)
                .map(|(outcome, module_range, related)| (c, outcome, module_range, related))
        })
        .filter(|(_, _, _, related)| tier != Tier::Corpus || *related)
        .map(|(c, outcome, module_range, _)| (c, outcome, module_range))
        .collect();
    if results.is_empty() {
        let unavailable = unavailable_imports(graph);
        let finding = match (filter, unavailable.is_empty()) {
            (Some(module_name), _) => Finding::new(
                "W301",
                format!("snippet defines `{name}`, which module {module_name} does not define"),
                here(name_token.text_range()),
            ),
            (None, true) => Finding::new(
                "W301",
                format!("snippet defines `{name}`, which is not in any module"),
                here(name_token.text_range()),
            ),
            (None, false) => Finding::new(
                "W301",
                format!(
                    "snippet defines `{name}`, which is not in any available module; it may be defined in {}, which the draft imports from and which is not available",
                    unavailable.join(", ")
                ),
                here(name_token.text_range()),
            )
            .with_severity(glint_diag::Severity::Info),
        };
        findings.push(finding);
        return;
    }
    let module_loc = |c: Candidate, range: TextRange| {
        graph.module(c.module).map(|m| Loc {
            source: m.source,
            range,
        })
    };
    if let Some((c, _, _)) = results.iter().find(|(_, o, _)| matches!(o, Outcome::Same)) {
        if candidates.len() > 1 {
            let label = module_label(graph, c.module);
            findings.push(Finding::new(
                "I303",
                format!(
                    "`{name}` is defined by {} modules; the snippet matches the one in {label}",
                    candidates.len()
                ),
                here(name_token.text_range()),
            ));
        }
        return;
    }
    let best_equivalent = results
        .iter()
        .filter_map(|(c, o, r)| match o {
            Outcome::Equivalent(d) => Some((c, d, r)),
            _ => None,
        })
        .min_by_key(|(_, d, _)| d.len());
    if let Some((c, diffs, module_range)) = best_equivalent {
        let label = module_label(graph, c.module);
        let (first, more) = describe(diffs);
        let primary = diffs.first().map_or(range, |d| d.snippet);
        let secondary = diffs.first().map_or(*module_range, |d| d.module);
        let mut finding = Finding::new(
            "W302",
            format!("`{name}` is written differently from its definition in {label}: {first}"),
            here(primary),
        );
        if let Some(loc) = module_loc(*c, secondary) {
            finding = finding.with_secondary(loc, "module definition");
        }
        for note in more {
            finding = finding.with_note(note);
        }
        findings.push(finding.with_note("both mean the same after normalization (X.680 C.3)"));
        return;
    }
    let best_different = results
        .iter()
        .filter_map(|(c, o, r)| match o {
            Outcome::Different(d) => Some((c, d, r)),
            _ => None,
        })
        .min_by_key(|(_, d, _)| d.len());
    if let Some((c, diffs, module_range)) = best_different {
        let label = module_label(graph, c.module);
        let (first, more) = describe(diffs);
        let primary = diffs.first().map_or(range, |d| d.snippet);
        let secondary = diffs.first().map_or(*module_range, |d| d.module);
        let mut finding = Finding::new(
            "E300",
            format!("`{name}` differs from its definition in {label}: {first}"),
            here(primary),
        );
        if let Some(loc) = module_loc(*c, secondary) {
            finding = finding.with_secondary(loc, "module definition");
        }
        for note in more {
            finding = finding.with_note(note);
        }
        if candidates.len() > 1 {
            finding = finding.with_note(format!(
                "closest of {} modules that define `{name}`; none matches",
                candidates.len()
            ));
        }
        findings.push(finding);
    }
}

/// Reports the types and classes of the draft's modules that the text neither shows nor mentions (W310).
pub fn reverse_check(
    graph: &ModuleGraph<'_>,
    drafts: &[ModuleId],
    mentioned: &HashSet<String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for &id in drafts {
        let Some(module) = graph.module(id) else {
            continue;
        };
        for symbol in &module.model.symbols {
            if !matches!(symbol.kind, SymbolKind::Type | SymbolKind::Class)
                || mentioned.contains(&symbol.name)
            {
                continue;
            }
            findings.push(Finding::new(
                "W310",
                format!(
                    "`{}` is defined in module {} but the text never shows or mentions it",
                    symbol.name, module.model.name
                ),
                Loc {
                    source: module.source,
                    range: symbol.name_range,
                },
            ));
        }
    }
    findings
}
