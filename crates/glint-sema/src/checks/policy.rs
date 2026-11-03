//! Glint's own rules for new text: W170, W171, E172, W173. Off for published (corpus) modules.

use glint_syntax::SyntaxKind;

use super::Ctx;
use crate::finding::Finding;
use crate::model::SymbolKind;
use crate::sem::{Kind, SymRef};

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    if !ctx.graph.options().policy_checks {
        return;
    }
    let symbols = ctx.loaded.model.symbols.clone();
    // E172, W173
    let mut warned_macro_use = false;
    for symbol in &symbols {
        match symbol.kind {
            SymbolKind::Macro => {
                let finding = Finding::new(
                    "E172",
                    format!(
                        "`{}` defines a new MACRO, which X.680 (2021) no longer has",
                        symbol.name
                    ),
                    ctx.loc(symbol.name_range),
                )
                .with_help("define an information object class instead (X.681 clause 9)");
                ctx.report(finding);
            }
            SymbolKind::Unknown if !warned_macro_use => {
                warned_macro_use = true;
                let finding = Finding::new(
                    "W173",
                    format!(
                        "`{}` is written in imported macro notation, which Glint does not check",
                        symbol.name
                    ),
                    ctx.loc(symbol.name_range),
                );
                ctx.report(finding);
            }
            _ => {}
        }
    }
    // W170
    let node = ctx.loaded.node.syntax().clone();
    let any = node.descendants().find(|n| n.kind() == SyntaxKind::AnyType);
    let x681 = node.descendants().any(|n| {
        matches!(
            n.kind(),
            SyntaxKind::ObjectClassDef
                | SyntaxKind::ObjectClassFieldType
                | SyntaxKind::TableConstraint
                | SyntaxKind::InstanceOfType
        )
    });
    if let Some(any) = any
        && x681
    {
        let finding = Finding::new(
            "W170",
            "this module mixes 1988 syntax (`ANY`) with X.681 constructs",
            ctx.loc(crate::view::trimmed_range(&any)),
        );
        ctx.report(finding);
    }
    // W171
    for (index, symbol) in symbols.iter().enumerate() {
        if symbol.kind != SymbolKind::Type || symbol.is_parameterized() {
            continue;
        }
        let sym = SymRef {
            module: ctx.module,
            index,
        };
        if ctx.sem.kind(sym) == Kind::Type
            && symbol
                .name
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .count()
                > 1
            && glint_syntax::is_all_upper(&symbol.name)
        {
            let finding = Finding::new(
                "W171",
                format!(
                    "`{}` is a type, but an all-upper-case name reads as an information object class",
                    symbol.name
                ),
                ctx.loc(symbol.name_range),
            );
            ctx.report(finding);
        }
    }
}
