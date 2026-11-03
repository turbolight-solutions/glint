//! Component list checks: E113 and E126, plus `COMPONENTS OF` expansion shared with the tag checks.

use glint_syntax::{SyntaxKind, SyntaxNode};

use super::Ctx;
use crate::finding::Finding;
use crate::graph::{ModuleGraph, ModuleId};
use crate::model::SymbolKind;
use crate::view::{BaseKind, Component, ComponentList, ListItem, Region, base_kind_of_node};

const MAX_EXPANSION_DEPTH: usize = 8;

/// A component together with the module in which its type is written.
#[derive(Debug, Clone)]
pub(crate) struct Flat {
    pub module: ModuleId,
    pub component: Component,
    /// The component came from a `COMPONENTS OF`.
    pub expanded: bool,
}

/// The kind of the type named by `COMPONENTS OF` (X.680 25.5, 27.2): `None` when it cannot be known.
pub(super) fn components_of_target(
    graph: &ModuleGraph<'_>,
    module: ModuleId,
    ty: &SyntaxNode,
) -> Option<(ModuleId, SyntaxNode, BaseKind)> {
    let peeled = graph.peel_type_through_tags(module, ty)?;
    let kind = base_kind_of_node(&peeled.node)?;
    Some((peeled.module, peeled.node, kind))
}

/// Expands `COMPONENTS OF` items in text order (X.680 25.5): only the root components of the named type are taken.
pub(crate) fn expand(
    graph: &ModuleGraph<'_>,
    module: ModuleId,
    list: &ComponentList,
    depth: usize,
) -> Vec<Flat> {
    let mut out = Vec::new();
    for item in &list.items {
        match item {
            ListItem::Component(component) => out.push(Flat {
                module,
                component: component.clone(),
                expanded: false,
            }),
            ListItem::ComponentsOf(of) => {
                if depth >= MAX_EXPANSION_DEPTH {
                    continue;
                }
                let Some((target_module, node, kind)) = components_of_target(graph, module, &of.ty)
                else {
                    continue;
                };
                if !matches!(kind, BaseKind::Sequence | BaseKind::Set) {
                    continue;
                }
                let inner = ComponentList::read(&node);
                for mut flat in expand(graph, target_module, &inner, depth.saturating_add(1)) {
                    if flat.component.region == Region::Addition {
                        continue;
                    }
                    flat.component.region = of.region;
                    flat.expanded = true;
                    out.push(flat);
                }
            }
        }
    }
    out
}

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    let symbols = ctx.loaded.model.symbols.clone();
    for symbol in &symbols {
        if matches!(symbol.kind, SymbolKind::Macro | SymbolKind::Unknown) {
            continue;
        }
        for node in symbol.node.descendants() {
            if matches!(
                node.kind(),
                SyntaxKind::SequenceType | SyntaxKind::SetType | SyntaxKind::ChoiceType
            ) {
                check_list(ctx, &node);
            }
        }
    }
}

fn check_list(ctx: &mut Ctx<'_, '_, '_>, node: &SyntaxNode) {
    let list = ComponentList::read(node);
    let container = match node.kind() {
        SyntaxKind::SetType => BaseKind::Set,
        SyntaxKind::ChoiceType => BaseKind::Choice,
        _ => BaseKind::Sequence,
    };
    // X.680 25.5, 27.2
    for item in &list.items {
        if let ListItem::ComponentsOf(of) = item
            && let Some((_, _, kind)) = components_of_target(ctx.graph, ctx.module, &of.ty)
            && kind != container
        {
            let wanted = if container == BaseKind::Set {
                "SET"
            } else {
                "SEQUENCE"
            };
            let finding = Finding::new(
                "E126",
                format!("`COMPONENTS OF` must name a {wanted} type"),
                ctx.loc(crate::view::trimmed_range(&of.ty)),
            );
            ctx.report(finding);
        }
    }
    // X.680 25.14, 29.10
    let flat = expand(ctx.graph, ctx.module, &list, 0);
    let mut seen: Vec<&Flat> = Vec::new();
    let mut findings = Vec::new();
    for entry in &flat {
        if entry.expanded {
            if let Some(first) = seen
                .iter()
                .find(|f| f.component.name == entry.component.name)
            {
                findings.push(
                    Finding::new(
                        "E113",
                        format!(
                            "identifier `{}` is used more than once",
                            entry.component.name
                        ),
                        ctx.loc(crate::view::trimmed_range(node)),
                    )
                    .with_note("it comes from COMPONENTS OF")
                    .with_secondary(ctx.loc(first.component.name_range), "first used here"),
                );
            }
        } else if let Some(first) = seen
            .iter()
            .find(|f| f.component.name == entry.component.name)
        {
            let mut finding = Finding::new(
                "E113",
                format!(
                    "identifier `{}` is used more than once",
                    entry.component.name
                ),
                ctx.loc(entry.component.name_range),
            );
            if first.module == ctx.module {
                finding =
                    finding.with_secondary(ctx.loc(first.component.name_range), "first used here");
            }
            findings.push(finding);
        }
        seen.push(entry);
    }
    for finding in findings {
        ctx.report(finding);
    }
}
