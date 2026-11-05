//! Object identifier checks: E130, E134, W140, I141 (X.680 32.x; X.660), the label table checks W142, W143, W144,
//! I145, and the diagnostics of OID evaluation.

use std::collections::HashSet;

use glint_corpus::{LabelProblem, Oid, document_title, same_document};
use glint_diag::Severity;
use glint_syntax::SyntaxNode;

use super::Ctx;
use crate::eval::{OidEval, OidProblemKind};
use crate::finding::Finding;
use crate::graph::Matched;
use crate::model::AssignedId;

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    let mut evaluated: Vec<OidEval> = Vec::new();
    evaluated.extend(check_header(ctx));
    let groups: Vec<SyntaxNode> = ctx
        .loaded
        .model
        .imports
        .iter()
        .filter_map(|clause| match &clause.assigned {
            Some((AssignedId::Oid { group }, _)) => Some(group.clone()),
            _ => None,
        })
        .collect();
    for group in groups {
        let result = ctx.eval.eval_oid_group(ctx.module, &group);
        report_problems(ctx, &result);
        evaluated.push(result);
    }
    evaluated.extend(check_assignments(ctx));
    if ctx.graph.options().oid_labels {
        check_labels(ctx, &evaluated);
        check_import_identifiers(ctx);
    }
}

// X.680 13.8
fn check_header(ctx: &mut Ctx<'_, '_, '_>) -> Option<OidEval> {
    let group = ctx.loaded.model.oid_group.clone()?;
    let result = ctx.eval.eval_oid_group(ctx.module, &group);
    report_problems(ctx, &result);
    if ctx.graph.options().oid_collisions
        && let Some(oid) = &result.oid
    {
        let corpus = ctx.graph.corpus();
        let holder = corpus
            .by_oid(oid)
            .and_then(|index| corpus.modules().get(index))
            .filter(|record| record.name != ctx.loaded.model.name);
        if let Some(record) = holder {
            let finding = Finding::new(
                "W140",
                format!(
                    "module identifier {oid} is already assigned to module `{}` in {}",
                    record.name,
                    record.source.label()
                ),
                ctx.loc(crate::view::trimmed_range(&group)),
            );
            ctx.report(finding);
        }
    }
    Some(result)
}

// X.680 32.3-32.13; policy W140
fn check_assignments(ctx: &mut Ctx<'_, '_, '_>) -> Vec<OidEval> {
    let mut evaluated = Vec::new();
    let count = ctx.loaded.model.symbols.len();
    for index in 0..count {
        if !ctx.eval.symbol_is_oid_valued(ctx.module, index) {
            continue;
        }
        let Some(symbol) = ctx.loaded.model.symbols.get(index).cloned() else {
            continue;
        };
        let Some(value) = symbol.value_node.as_ref() else {
            continue;
        };
        let Some(group) = crate::view::child(value, glint_syntax::SyntaxKind::BraceGroup) else {
            continue;
        };
        let result = ctx.eval.eval_oid_group(ctx.module, &group);
        report_problems(ctx, &result);
        let oid = result.oid.clone();
        evaluated.push(result);
        if !ctx.graph.options().oid_collisions {
            continue;
        }
        let Some(oid) = oid else {
            continue;
        };
        let own = ctx.graph.options().own_document.as_deref();
        let holders: Vec<_> = ctx
            .graph
            .corpus()
            .assignments_of(&oid)
            .iter()
            .filter(|h| own.is_none_or(|own| !same_document(&h.document, own)))
            .collect();
        if holders.is_empty() || holders.iter().any(|h| h.name == symbol.name) {
            continue;
        }
        if let Some(first) = holders.first() {
            let document = glint_corpus::document_title(&first.document);
            let finding = Finding::new(
                "W140",
                format!(
                    "{oid} is already assigned in {document} to `{}` (module {})",
                    first.name, first.module
                ),
                ctx.loc(symbol.name_range),
            )
            .with_note(format!("this draft assigns it to `{}`", symbol.name));
            ctx.report(finding);
        }
    }
    evaluated
}

/// `RFC 5280, RFC 5912 and 3 more`.
fn document_list(documents: &[String]) -> String {
    const SHOWN: usize = 3;
    let titles: Vec<String> = documents
        .iter()
        .take(SHOWN)
        .map(|d| document_title(d))
        .collect();
    let rest = documents.len().saturating_sub(SHOWN);
    if rest > 0 {
        format!("{} and {rest} more", titles.join(", "))
    } else {
        titles.join(", ")
    }
}

fn parent_text(oid: &Oid) -> String {
    oid.parent().map_or_else(String::new, |p| p.to_string())
}

// policy: the label table (W142, W143, I145)
fn check_labels(ctx: &mut Ctx<'_, '_, '_>, evaluated: &[OidEval]) {
    let options = ctx.graph.options();
    let own = options.own_document.as_deref();
    let table = ctx.graph.corpus().oid_labels();
    let mut seen: HashSet<(Oid, String)> = HashSet::new();
    for result in evaluated {
        let value = result
            .oid
            .as_ref()
            .or_else(|| result.labels.last().map(|l| &l.oid));
        if value.is_some_and(|v| table.allowed(v).is_some()) {
            continue;
        }
        for label in &result.labels {
            if !seen.insert((label.oid.clone(), label.name.clone())) {
                continue;
            }
            // A module identifier marks its arc as established, whichever document defines it: modules get new
            // arcs between editions, and the labels of the old arc then differ from the new ones.
            let defined = ctx.graph.corpus().by_oid(&label.oid).is_some();
            let Some(problem) = table.check(&label.oid, &label.name, own, defined) else {
                continue;
            };
            // The arcs after a wrong one are under the wrong parent too; the first problem is the one to fix.
            let stop = !matches!(problem, LabelProblem::Synonym { .. });
            let number = label.oid.last().unwrap_or_default();
            let written = format!("{}({number})", label.name);
            let finding = match problem {
                LabelProblem::WrongNumber {
                    expected,
                    documents,
                } => Finding::new(
                    "W142",
                    format!(
                        "`{written}`: under {} the corpus numbers `{}` {expected}",
                        parent_text(&label.oid),
                        label.name
                    ),
                    ctx.loc(label.range),
                )
                .with_note(format!("as written in {}", document_list(&documents)))
                .with_help(format!(
                    "check the arc; the corpus writes `{}({expected})`",
                    label.name
                )),
                LabelProblem::WrongParent {
                    expected_parent,
                    documents,
                } => Finding::new(
                    "W143",
                    format!(
                        "`{written}` is under {expected_parent} in the corpus, not under {}",
                        parent_text(&label.oid)
                    ),
                    ctx.loc(label.range),
                )
                .with_note(format!("as written in {}", document_list(&documents)))
                .with_help("check for a missing or extra arc before it"),
                LabelProblem::Synonym { others } if options.oid_synonyms => Finding::new(
                    "I145",
                    format!(
                        "the corpus calls arc {} `{}`, not `{}`",
                        label.oid,
                        others.join("`, `"),
                        label.name
                    ),
                    ctx.loc(label.range),
                )
                .with_severity(Severity::Info),
                LabelProblem::Synonym { .. } => continue,
            };
            ctx.report(finding);
            if stop {
                break;
            }
        }
    }
}

// policy: module identifiers in IMPORTS against the corpus (W144)
fn check_import_identifiers(ctx: &mut Ctx<'_, '_, '_>) {
    let own = ctx.graph.options().own_document.clone();
    let corpus = ctx.graph.corpus();
    let clauses: Vec<_> = ctx
        .loaded
        .model
        .imports
        .iter()
        .zip(&ctx.loaded.imports)
        .filter_map(|(clause, resolved)| match &clause.assigned {
            Some((AssignedId::Oid { group }, _)) => {
                Some((clause.clone(), resolved.clone(), group.clone()))
            }
            _ => None,
        })
        .collect();
    for (clause, resolved, group) in clauses {
        // Only an import that the OID matched to a module of another name: one matched by name with another OID
        // gets W102, and a selection or a pin is the author's choice.
        if resolved.matched != Matched::Oid {
            continue;
        }
        let target_name = resolved
            .target
            .and_then(|t| ctx.graph.module(t))
            .map(|t| t.model.name.clone());
        if target_name.as_deref() == Some(clause.module_name.as_str()) {
            continue;
        }
        let Some(oid) = ctx.eval.eval_oid_group(ctx.module, &group).oid else {
            continue;
        };
        if corpus.oid_labels().allowed(&oid).is_some() {
            continue;
        }
        let named: Vec<_> = corpus
            .by_name(&clause.module_name)
            .iter()
            .filter_map(|&index| corpus.modules().get(index))
            .filter(|m| {
                !m.source.is_draft()
                    && own
                        .as_deref()
                        .is_none_or(|own| !same_document(&m.source.document, own))
            })
            .collect();
        if named.iter().any(|m| m.oid.as_ref() == Some(&oid)) {
            continue;
        }
        let Some(other) = named.iter().find(|m| m.oid.is_some()) else {
            continue;
        };
        let their = other
            .oid
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let finding = Finding::new(
            "W144",
            format!(
                "module `{}` has the identifier {their} in {}, not {oid}",
                clause.module_name,
                other.source.title()
            ),
            ctx.loc(clause.module_range),
        )
        .with_help(format!(
            "write `{} {{ {} }}`",
            clause.module_name,
            their.replace('.', " ")
        ));
        ctx.report(finding);
    }
}

/// Turns the problems found while evaluating an object identifier value into findings.
pub(super) fn report_problems(ctx: &mut Ctx<'_, '_, '_>, result: &OidEval) {
    for problem in &result.problems {
        let range = ctx.loc(problem.range);
        let finding = match &problem.kind {
            OidProblemKind::Undefined { name } => Finding::new(
                "E110",
                format!("value `{name}` is not defined in this module or imported"),
                range,
            ),
            OidProblemKind::NotAnOid { name } => Finding::new(
                "E131",
                format!("`{name}` is not an object identifier value"),
                range,
            ),
            OidProblemKind::BareName { name } => Finding::new(
                "E134",
                format!("`{name}` is not an identifier that X.660 registers at this position"),
                range,
            )
            .with_help(format!("write `{name}(number)`")),
            OidProblemKind::Malformed => Finding::new(
                "E131",
                "this is not a valid object identifier component",
                range,
            ),
            OidProblemKind::Arcs(reason) => Finding::new("E130", *reason, range),
            OidProblemKind::Circular { name } => Finding::new(
                "E131",
                format!("`{name}` is defined in terms of itself"),
                range,
            ),
        };
        ctx.report(finding);
    }
    for (name, range) in &result.placeholders {
        let finding = Finding::new(
            "I141",
            format!("`{name}` is a placeholder; the object identifier is not final"),
            ctx.loc(*range),
        )
        .with_severity(Severity::Info);
        ctx.report(finding);
    }
}
