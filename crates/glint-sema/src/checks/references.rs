//! Reference checks: E110, E111, W112, E114, W115 (X.680 13.19-13.21, 14.4-14.7).

use std::collections::HashSet;

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use super::Ctx;
use crate::finding::Finding;
use crate::graph::{Lookup, QualifiedError};
use crate::model::Symbol;
use crate::view::{defined_type_ref, tokens};

const BUILTIN_CLASSES: &[&str] = &["TYPE-IDENTIFIER", "ABSTRACT-SYNTAX"];

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    check_duplicates(ctx);
    check_defined_and_imported(ctx);
    let symbols = ctx.loaded.model.symbols.clone();
    for symbol in &symbols {
        if matches!(
            symbol.kind,
            crate::model::SymbolKind::Macro | crate::model::SymbolKind::Unknown
        ) {
            continue;
        }
        walk(ctx, symbol, &symbol.node);
    }
    check_unused_imports(ctx);
}

// X.680 13.21; X.683 8.2
fn check_duplicates(ctx: &mut Ctx<'_, '_, '_>) {
    let mut seen: Vec<(&str, TextRange)> = Vec::new();
    let mut findings = Vec::new();
    for symbol in &ctx.loaded.model.symbols {
        if let Some((_, first)) = seen.iter().find(|(name, _)| *name == symbol.name) {
            findings.push(
                Finding::new(
                    "E111",
                    format!("`{}` is defined more than once", symbol.name),
                    ctx.loc(symbol.name_range),
                )
                .with_secondary(ctx.loc(*first), "first defined here"),
            );
        } else {
            seen.push((symbol.name.as_str(), symbol.name_range));
        }
    }
    for finding in findings {
        ctx.report(finding);
    }
}

// X.680 13.20: a symbol both imported and assigned is valid (the import must be written `Module.symbol`) but
// deprecated. Reported once per name, on its first local definition; a second one is E111.
fn check_defined_and_imported(ctx: &mut Ctx<'_, '_, '_>) {
    let model = &ctx.loaded.model;
    let mut findings = Vec::new();
    let mut reported: HashSet<&str> = HashSet::new();
    for symbol in &model.symbols {
        let imports = model.imported(&symbol.name);
        if imports.is_empty() || !reported.insert(symbol.name.as_str()) {
            continue;
        }
        let clauses: Vec<_> = imports
            .iter()
            .filter_map(|&(clause, index)| {
                let clause = model.imports.get(clause)?;
                Some((
                    clause.module_name.as_str(),
                    clause.symbols.get(index)?.range,
                ))
            })
            .collect();
        let Some(&(first_module, _)) = clauses.first() else {
            continue;
        };
        let from = clauses
            .iter()
            .map(|(module, _)| format!("`{module}`"))
            .collect::<Vec<_>>()
            .join(" and ");
        let mut finding = Finding::new(
            "W115",
            format!(
                "`{name}` is imported from {from} and also defined here, which X.680 13.20 deprecates",
                name = symbol.name
            ),
            ctx.loc(symbol.name_range),
        )
        .with_help(format!(
            "give this definition another name; until then the imported one can only be written `{first_module}.{name}`",
            name = symbol.name
        ));
        for (_, range) in &clauses {
            finding = finding.with_secondary(ctx.loc(*range), "imported here");
        }
        findings.push(finding);
    }
    for finding in findings {
        ctx.report(finding);
    }
}

fn walk(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, node: &SyntaxNode) {
    for child in node.children() {
        match child.kind() {
            SyntaxKind::ParameterList
            | SyntaxKind::TableConstraint
            | SyntaxKind::ActualParameterList
            | SyntaxKind::AmbiguousRhs
            | SyntaxKind::UserDefinedConstraint
            | SyntaxKind::XmlValue
            | SyntaxKind::Value
            | SyntaxKind::MacroUse
            | SyntaxKind::MacroDefinition => {}
            SyntaxKind::DefinedType => {
                check_type_reference(ctx, symbol, &child);
            }
            SyntaxKind::ObjectClassFieldType => {
                check_class_reference(ctx, symbol, &child);
            }
            _ => walk(ctx, symbol, &child),
        }
    }
}

fn check_class_reference(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, node: &SyntaxNode) {
    let names: Vec<_> = tokens(node)
        .into_iter()
        .filter(|t| t.kind() == SyntaxKind::UpperName)
        .collect();
    let (qualifier, name) = match names.as_slice() {
        [name] => (None, name.clone()),
        [module, name, ..] => (Some(module.text().to_owned()), name.clone()),
        [] => return,
    };
    let range = name.text_range();
    check_reference(
        ctx,
        symbol,
        qualifier.as_deref(),
        name.text(),
        range,
        "class",
    );
}

fn check_type_reference(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, node: &SyntaxNode) {
    let Some(reference) = defined_type_ref(node) else {
        return;
    };
    check_reference(
        ctx,
        symbol,
        reference.module.as_deref(),
        &reference.name,
        reference.range,
        "type",
    );
}

/// Reports E110 or E114 for a reference written at `range`; returns the lookup when the name resolved.
pub(super) fn check_reference(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    qualifier: Option<&str>,
    name: &str,
    range: TextRange,
    what: &str,
) -> Option<Lookup> {
    match qualifier {
        Some(module_name) => match ctx.graph.lookup_qualified(ctx.module, module_name, name) {
            Ok(lookup) => Some(lookup),
            Err(QualifiedError::ModuleNotImported) => {
                let finding = Finding::new(
                    "E110",
                    format!("module `{module_name}` is not named in IMPORTS"),
                    ctx.loc(range),
                );
                ctx.report(finding);
                None
            }
            Err(QualifiedError::SymbolNotImported) => {
                let finding = Finding::new(
                    "E110",
                    format!("`{name}` is not imported from `{module_name}`"),
                    ctx.loc(range),
                );
                ctx.report(finding);
                None
            }
        },
        None => {
            if symbol.parameters.iter().any(|p| p == name) || BUILTIN_CLASSES.contains(&name) {
                return None;
            }
            let Some(lookup) = ctx.graph.lookup(ctx.module, name) else {
                let finding = Finding::new(
                    "E110",
                    format!("{what} `{name}` is not defined in this module or imported"),
                    ctx.loc(range),
                );
                ctx.report(finding);
                return None;
            };
            if lookup.ambiguous {
                let finding = Finding::new(
                    "E114",
                    format!(
                        "`{name}` is defined or imported more than once and must be written `Module.{name}`"
                    ),
                    ctx.loc(range),
                );
                ctx.report(finding);
            }
            Some(lookup)
        }
    }
}

// W112: a name that never appears outside IMPORTS and EXPORTS
fn check_unused_imports(ctx: &mut Ctx<'_, '_, '_>) {
    let model = &ctx.loaded.model;
    let defining: HashSet<TextRange> = model.symbols.iter().map(|s| s.name_range).collect();
    let mut used: HashSet<String> = HashSet::new();
    for element in ctx.loaded.node.syntax().descendants_with_tokens() {
        let Some(token) = element.into_token() else {
            continue;
        };
        let nameish = token.kind().is_name() || token.kind() == SyntaxKind::Keyword;
        if !nameish || defining.contains(&token.text_range()) {
            continue;
        }
        let inside_clause = token.parent_ancestors().any(|n| {
            matches!(
                n.kind(),
                SyntaxKind::Imports | SyntaxKind::Exports | SyntaxKind::ModuleIdentifier
            )
        });
        if !inside_clause {
            used.insert(token.text().to_owned());
        }
    }
    let mut findings = Vec::new();
    for clause in &model.imports {
        for symbol in &clause.symbols {
            if !used.contains(&symbol.name) {
                findings.push(Finding::new(
                    "W112",
                    format!(
                        "`{}` is imported from `{}` but never used",
                        symbol.name, clause.module_name
                    ),
                    ctx.loc(symbol.range),
                ));
            }
        }
    }
    for finding in findings {
        ctx.report(finding);
    }
}
