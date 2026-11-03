//! Object identifier checks: E130, E134, W140, I141 (X.680 32.x; X.660) and the diagnostics of OID evaluation.

use glint_diag::Severity;
use glint_syntax::SyntaxNode;

use super::Ctx;
use crate::eval::{OidEval, OidProblemKind};
use crate::finding::Finding;
use crate::model::AssignedId;

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    check_header(ctx);
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
    }
    check_assignments(ctx);
}

// X.680 13.8
fn check_header(ctx: &mut Ctx<'_, '_, '_>) {
    let Some(group) = ctx.loaded.model.oid_group.clone() else {
        return;
    };
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
}

// X.680 32.3-32.13; policy W140
fn check_assignments(ctx: &mut Ctx<'_, '_, '_>) {
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
        if !ctx.graph.options().oid_collisions {
            continue;
        }
        let Some(oid) = result.oid else {
            continue;
        };
        let holders = ctx.graph.corpus().assignments_of(&oid);
        if holders.is_empty() || holders.iter().any(|h| h.name == symbol.name) {
            continue;
        }
        if let Some(first) = holders.first() {
            let rfc = first
                .rfc
                .map_or_else(|| first.module.clone(), |n| format!("RFC {n}"));
            let finding = Finding::new(
                "W140",
                format!(
                    "{oid} is already assigned in {rfc} to `{}` (module {})",
                    first.name, first.module
                ),
                ctx.loc(symbol.name_range),
            )
            .with_note(format!("this draft assigns it to `{}`", symbol.name));
            ctx.report(finding);
        }
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
