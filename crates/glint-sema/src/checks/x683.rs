//! X.683 checks: E160-E168, W169 (parameterized definitions and their uses).

use std::collections::{HashMap, HashSet};

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use super::Ctx;
use crate::finding::Finding;
use crate::model::{Symbol, SymbolKind};
use crate::sem::toks::{Tk, flatten};
use crate::sem::{ClassRes, Dummy, Kind, Resolved, SymRef, dummies_of, range_of};
use crate::view::{child, defined_type_ref, tokens, type_child};

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    let symbols = ctx.loaded.model.symbols.clone();
    for (index, symbol) in symbols.iter().enumerate() {
        if matches!(symbol.kind, SymbolKind::Macro | SymbolKind::Unknown) {
            continue;
        }
        let sym = SymRef {
            module: ctx.module,
            index,
        };
        if symbol.is_parameterized() {
            check_definition(ctx, sym, symbol);
            check_abstract_syntax(ctx, sym, symbol);
        }
        check_uses(ctx, symbol);
    }
    check_recursion(ctx, &symbols);
}

// ---- definitions (X.683 8.1-8.13) -----------------------------------------------------------------

/// How a dummy reference is declared (X.683 8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Declared {
    /// No governor: stands for a type or a class.
    TypeOrClass,
    /// A type governs it: a value or a value set.
    ValueLike,
    /// A class governs it: an object or an object set.
    ObjectLike,
    /// Governed by another dummy, or by something unavailable.
    Unknown,
    /// The governor names something that is neither a type nor a class.
    BadGovernor,
}

fn declared(ctx: &Ctx<'_, '_, '_>, dummy: &Dummy, all: &[Dummy]) -> Declared {
    if let Some(governor) = &dummy.governor {
        let scope = all;
        if governor.kind() == SyntaxKind::DefinedType
            && let Some(reference) = defined_type_ref(governor)
            && reference.module.is_none()
            && scope.iter().any(|d| d.name == reference.name)
        {
            return Declared::Unknown;
        }
        return match ctx.sem.class_of(ctx.module, governor) {
            ClassRes::Class(_) => Declared::ObjectLike,
            ClassRes::Unknown => Declared::Unknown,
            ClassRes::NotClass => {
                if governor.kind() == SyntaxKind::DefinedType
                    && let Some(Resolved::Sym(target)) =
                        ctx.sem.resolve_defined(ctx.module, governor)
                    && matches!(
                        ctx.sem.kind(target),
                        Kind::Value | Kind::Object | Kind::ObjectSet | Kind::ValueSet
                    )
                {
                    Declared::BadGovernor
                } else {
                    Declared::ValueLike
                }
            }
        };
    }
    if let Some(name) = &dummy.governor_name {
        if all.iter().any(|d| &d.name == name) {
            return Declared::Unknown;
        }
        if let Resolved::Sym(target) = ctx.sem.resolve(ctx.module, &[], None, name)
            && matches!(
                ctx.sem.kind(target),
                Kind::Value | Kind::Object | Kind::ObjectSet | Kind::ValueSet
            )
        {
            return Declared::BadGovernor;
        }
        return Declared::Unknown;
    }
    Declared::TypeOrClass
}

// X.683 8.3-8.13
fn check_definition(ctx: &mut Ctx<'_, '_, '_>, _sym: SymRef, symbol: &Symbol) {
    let dummies = dummies_of(&symbol.node);
    for dummy in &dummies {
        let kind = declared(ctx, dummy, &dummies);
        // 8.3
        match kind {
            Declared::BadGovernor => {
                let finding = Finding::new(
                    "E161",
                    format!("the governor of `{}` must be a type or a class", dummy.name),
                    ctx.loc(dummy.governor.as_ref().map_or(dummy.range, range_of)),
                );
                ctx.report(finding);
            }
            Declared::TypeOrClass if dummy.name.chars().next().is_some_and(char::is_lowercase) => {
                let finding = Finding::new(
                    "E161",
                    format!(
                        "`{}` stands for a value or an object, so it needs a governor, written `Governor : {}`",
                        dummy.name, dummy.name
                    ),
                    ctx.loc(dummy.range),
                );
                ctx.report(finding);
            }
            _ => {}
        }
        // 8.3: a DummyGovernor has no governor of its own
        if let Some(name) = &dummy.governor_name
            && let Some(other) = dummies.iter().find(|d| &d.name == name)
            && (other.governor.is_some() || other.governor_name.is_some())
        {
            let finding = Finding::new(
                "E161",
                format!(
                    "`{name}` governs `{}` but has a governor itself",
                    dummy.name
                ),
                ctx.loc(dummy.range),
            );
            ctx.report(finding);
        }
        // 8.9, 8.11
        if let Some(governor) = &dummy.governor {
            for token in flatten(governor) {
                if !token.kind.is_name() {
                    continue;
                }
                if token.text == dummy.name || token.text == symbol.name {
                    let finding = Finding::new(
                        "E165",
                        format!(
                            "the governor of `{}` depends on `{}`",
                            dummy.name, token.text
                        ),
                        ctx.loc(token.range),
                    );
                    ctx.report(finding);
                } else if let Some(other) = dummies.iter().find(|d| d.name == token.text)
                    && (other.governor.is_some() || other.governor_name.is_some())
                {
                    let finding = Finding::new(
                        "E165",
                        format!(
                            "the governor of `{}` refers to `{}`, which has a governor",
                            dummy.name, other.name
                        ),
                        ctx.loc(token.range),
                    );
                    ctx.report(finding);
                }
            }
        }
    }
    check_unused(ctx, symbol, &dummies);
    check_rhs_only_dummy(ctx, symbol, &dummies);
    check_usage(ctx, symbol, &dummies);
}

/// Name tokens of the assignment after its parameter list, and inside the governors of the parameters.
fn body_tokens(symbol: &Symbol) -> Vec<Tk> {
    let mut out = Vec::new();
    let params_range = child(&symbol.node, SyntaxKind::ParameterList).map(|l| l.text_range());
    for token in flatten(&symbol.node) {
        if token.range == symbol.name_range {
            continue;
        }
        if let Some(range) = params_range
            && range.contains_range(token.range)
        {
            continue;
        }
        out.push(token);
    }
    if let Some(list) = child(&symbol.node, SyntaxKind::ParameterList) {
        for parameter in list.children() {
            if let Some(governor) = type_child(&parameter) {
                out.extend(flatten(&governor));
            }
            let toks = tokens(&parameter);
            if let Some(colon) = toks.iter().position(|t| t.kind() == SyntaxKind::Colon) {
                for t in toks.iter().take(colon) {
                    out.push(Tk {
                        kind: t.kind(),
                        text: t.text().to_owned(),
                        range: t.text_range(),
                    });
                }
            }
        }
    }
    out
}

// X.683 8.6
fn check_unused(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, dummies: &[Dummy]) {
    let used: HashSet<String> = body_tokens(symbol).into_iter().map(|t| t.text).collect();
    for dummy in dummies {
        if !used.contains(&dummy.name) {
            let finding = Finding::new(
                "E163",
                format!("the dummy reference `{}` is never used", dummy.name),
                ctx.loc(dummy.range),
            );
            ctx.report(finding);
        }
    }
}

// X.683 8.10
fn check_rhs_only_dummy(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, dummies: &[Dummy]) {
    let rhs = match symbol.kind {
        SymbolKind::Type | SymbolKind::Class => symbol.type_node.as_ref().map(flatten),
        SymbolKind::Value | SymbolKind::ObjectOrValue => symbol.value_node.as_ref().map(flatten),
        _ => None,
    };
    if let Some([only]) = rhs.as_deref()
        && dummies.iter().any(|d| d.name == only.text)
    {
        let finding = Finding::new(
            "E166",
            "the right-hand side of a parameterized assignment cannot be only a dummy reference",
            ctx.loc(only.range),
        );
        ctx.report(finding);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Use {
    Type,
    Class,
    Value,
    ObjectSet,
}

// X.683 8.5: usage of a dummy must agree with its declaration and with its other uses
fn check_usage(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, dummies: &[Dummy]) {
    let mut uses: HashMap<String, Vec<(Use, TextRange)>> = HashMap::new();
    let params_range = child(&symbol.node, SyntaxKind::ParameterList).map(|l| l.text_range());
    for node in symbol.node.descendants() {
        if params_range.is_some_and(|r| r.contains_range(node.text_range())) {
            continue;
        }
        if node.ancestors().skip(1).any(|a| {
            matches!(
                a.kind(),
                SyntaxKind::ActualParameter | SyntaxKind::BraceGroup
            )
        }) {
            continue;
        }
        match node.kind() {
            SyntaxKind::DefinedType => {
                let in_constraint = node.ancestors().skip(1).any(|a| {
                    matches!(
                        a.kind(),
                        SyntaxKind::Constraint
                            | SyntaxKind::TypeElement
                            | SyntaxKind::ContainedSubtype
                    )
                });
                if let Some(reference) = defined_type_ref(&node)
                    && reference.module.is_none()
                    && !in_constraint
                    && dummies.iter().any(|d| d.name == reference.name)
                    && !node
                        .children()
                        .any(|c| c.kind() == SyntaxKind::ActualParameterList)
                {
                    let in_class_position = node.parent().is_some_and(|p| {
                        matches!(
                            p.kind(),
                            SyntaxKind::ValueAssignment | SyntaxKind::ValueSetAssignment
                        )
                    });
                    uses.entry(reference.name.clone()).or_default().push((
                        if in_class_position {
                            Use::Class
                        } else {
                            Use::Type
                        },
                        reference.range,
                    ));
                }
            }
            SyntaxKind::ObjectClassFieldType => {
                if let Some(first) = tokens(&node)
                    .into_iter()
                    .find(|t| t.kind() == SyntaxKind::UpperName)
                    && dummies.iter().any(|d| d.name == first.text())
                {
                    uses.entry(first.text().to_owned())
                        .or_default()
                        .push((Use::Class, first.text_range()));
                }
            }
            SyntaxKind::TableConstraint => {
                for group in node
                    .children()
                    .filter(|c| c.kind() == SyntaxKind::BraceGroup)
                    .take(1)
                {
                    for token in flatten(&group) {
                        if token.kind == SyntaxKind::UpperName
                            && dummies.iter().any(|d| d.name == token.text)
                        {
                            uses.entry(token.text.clone())
                                .or_default()
                                .push((Use::ObjectSet, token.range));
                        }
                    }
                }
            }
            SyntaxKind::Value => {
                let toks = flatten(&node);
                if let [only] = toks.as_slice()
                    && only.kind == SyntaxKind::LowerName
                    && dummies.iter().any(|d| d.name == only.text)
                {
                    uses.entry(only.text.clone())
                        .or_default()
                        .push((Use::Value, only.range));
                }
            }
            _ => {}
        }
    }
    for dummy in dummies {
        let Some(found) = uses.get(&dummy.name) else {
            continue;
        };
        let declared = declared(ctx, dummy, dummies);
        for (usage, range) in found {
            let bad = match (declared, usage) {
                (Declared::TypeOrClass, Use::Value | Use::ObjectSet) => {
                    Some("a value or object set")
                }
                (Declared::ValueLike | Declared::ObjectLike, Use::Type | Use::Class) => {
                    Some("a type or class")
                }
                (Declared::ObjectLike, Use::Value) => None,
                _ => None,
            };
            if let Some(what) = bad {
                let message = if declared == Declared::TypeOrClass {
                    format!(
                        "`{}` stands for a type or class (it has no governor), but it is used as {what}",
                        dummy.name
                    )
                } else {
                    format!(
                        "`{}` has a governor, so it stands for a value, value set, object or object set, but it is used as {what}",
                        dummy.name
                    )
                };
                let finding = Finding::new("E161", message, ctx.loc(*range));
                ctx.report(finding);
            }
        }
        // E162: a dummy used as both a type and a class, or as a value and an object set
        let kinds: HashSet<Use> = found.iter().map(|(u, _)| *u).collect();
        if (kinds.contains(&Use::Type) && kinds.contains(&Use::Class))
            || (kinds.contains(&Use::Value) && kinds.contains(&Use::Type))
            || (kinds.contains(&Use::Value) && kinds.contains(&Use::Class))
        {
            let finding = Finding::new(
                "E162",
                format!("`{}` is used in ways that cannot all be right", dummy.name),
                ctx.loc(found.first().map_or(dummy.range, |f| f.1)),
            );
            ctx.report(finding);
        }
    }
}

// ---- uses (X.683 9.1-9.8) -------------------------------------------------------------------------

fn check_uses(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol) {
    for node in symbol.node.descendants() {
        match node.kind() {
            SyntaxKind::DefinedType => {
                if node.ancestors().skip(1).any(|a| {
                    matches!(
                        a.kind(),
                        SyntaxKind::ActualParameter
                            | SyntaxKind::BraceGroup
                            | SyntaxKind::ParameterList
                    )
                }) {
                    continue;
                }
                check_type_use(ctx, symbol, &node);
            }
            SyntaxKind::Value => {
                if node.ancestors().skip(1).any(|a| {
                    matches!(
                        a.kind(),
                        SyntaxKind::ActualParameter | SyntaxKind::BraceGroup
                    )
                }) {
                    continue;
                }
                check_value_use(ctx, symbol, &node);
            }
            _ => {}
        }
    }
}

fn check_type_use(ctx: &mut Ctx<'_, '_, '_>, caller: &Symbol, node: &SyntaxNode) {
    let Some(Resolved::Sym(target)) = ctx.sem.resolve_defined(ctx.module, node) else {
        return;
    };
    let Some(callee) = ctx.sem.symbol(target) else {
        return;
    };
    let list = child(node, SyntaxKind::ActualParameterList);
    let range = defined_type_ref(node).map_or_else(|| range_of(node), |r| r.range);
    check_arity_and_forms(ctx, caller, callee, target, list.as_ref(), range);
}

fn check_value_use(ctx: &mut Ctx<'_, '_, '_>, caller: &Symbol, node: &SyntaxNode) {
    let toks = tokens(node);
    let Some(first) = toks.iter().find(|t| t.kind() == SyntaxKind::LowerName) else {
        return;
    };
    let list = child(node, SyntaxKind::ActualParameterList);
    if list.is_none() && child(node, SyntaxKind::FieldName).is_some() {
        return;
    }
    let scope = ctx.sem.scope_of(node);
    let Resolved::Sym(target) = ctx.sem.resolve(ctx.module, &scope, None, first.text()) else {
        return;
    };
    let Some(callee) = ctx.sem.symbol(target) else {
        return;
    };
    if list.is_none() && !callee.is_parameterized() {
        return;
    }
    if callee.kind == SymbolKind::Macro {
        return;
    }
    check_arity_and_forms(
        ctx,
        caller,
        callee,
        target,
        list.as_ref(),
        first.text_range(),
    );
}

// X.683 9.2, 9.3, 9.6
fn check_arity_and_forms(
    ctx: &mut Ctx<'_, '_, '_>,
    caller: &Symbol,
    callee: &Symbol,
    target: SymRef,
    list: Option<&SyntaxNode>,
    range: TextRange,
) {
    if matches!(callee.kind, SymbolKind::Macro | SymbolKind::Unknown) {
        return;
    }
    match (callee.is_parameterized(), list) {
        (false, Some(_)) => {
            let finding = Finding::new(
                "E160",
                format!(
                    "`{}` is not parameterized, but is given actual parameters",
                    callee.name
                ),
                ctx.loc(range),
            );
            ctx.report(finding);
        }
        (true, None) => {
            let finding = Finding::new(
                "E160",
                format!(
                    "`{}` is parameterized and needs {} actual parameter(s)",
                    callee.name,
                    callee.parameters.len()
                ),
                ctx.loc(range),
            );
            ctx.report(finding);
        }
        (true, Some(list)) => {
            let actuals = actuals_of(list);
            if actuals.len() != callee.parameters.len() {
                let finding = Finding::new(
                    "E160",
                    format!(
                        "`{}` takes {} actual parameter(s), but {} given",
                        callee.name,
                        callee.parameters.len(),
                        actuals.len()
                    ),
                    ctx.loc(range),
                );
                ctx.report(finding);
                return;
            }
            let dummies = dummies_of(&callee.node);
            for (actual, dummy) in actuals.iter().zip(&dummies) {
                let kind = declared_in(ctx, target, dummy, &dummies);
                if kind == Declared::ValueLike && form_problem(actual, kind).is_none() {
                    check_value_actual(ctx, caller, target, dummy, actual);
                }
                if let Some(problem) = form_problem(actual, kind) {
                    let finding = Finding::new(
                        "E160",
                        format!(
                            "actual parameter for `{}` of `{}`: {problem}",
                            dummy.name, callee.name
                        ),
                        ctx.loc(actual.first().map_or(range, |t| t.range)),
                    );
                    ctx.report(finding);
                }
            }
        }
        (false, None) => {}
    }
}

fn declared_in(ctx: &Ctx<'_, '_, '_>, target: SymRef, dummy: &Dummy, all: &[Dummy]) -> Declared {
    if let Some(governor) = &dummy.governor {
        if governor.kind() == SyntaxKind::DefinedType
            && let Some(reference) = defined_type_ref(governor)
            && reference.module.is_none()
            && all.iter().any(|d| d.name == reference.name)
        {
            return Declared::Unknown;
        }
        return match ctx.sem.class_of(target.module, governor) {
            ClassRes::Class(_) => Declared::ObjectLike,
            ClassRes::NotClass => Declared::ValueLike,
            ClassRes::Unknown => Declared::Unknown,
        };
    }
    if dummy.governor_name.is_some() {
        Declared::Unknown
    } else {
        Declared::TypeOrClass
    }
}

/// The token ranges of an `ActualParameterList`, split at top-level commas (X.683 9.5).
fn actuals_of(list: &SyntaxNode) -> Vec<Vec<Tk>> {
    list.children()
        .filter(|c| c.kind() == SyntaxKind::ActualParameter)
        .map(|c| flatten(&c))
        .collect()
}

const VALUE_WORDS: &[&str] = &[
    "TRUE",
    "FALSE",
    "NULL",
    "MIN",
    "MAX",
    "PLUS-INFINITY",
    "MINUS-INFINITY",
    "NOT-A-NUMBER",
];

// X.683 9.6, A.5
fn form_problem(actual: &[Tk], declared: Declared) -> Option<&'static str> {
    let first = actual.first()?;
    let braced = first.kind == SyntaxKind::LBrace;
    let is_type_from_object = first.kind == SyntaxKind::LowerName
        && actual.get(1).is_some_and(|t| t.kind == SyntaxKind::Dot)
        && actual
            .get(2)
            .is_some_and(|t| t.kind == SyntaxKind::FieldUpper);
    match declared {
        Declared::TypeOrClass => {
            if braced {
                Some("a type or class is expected, not a braced value set or object set")
            } else if (first.kind == SyntaxKind::LowerName && !is_type_from_object)
                || matches!(
                    first.kind,
                    SyntaxKind::Number | SyntaxKind::CString | SyntaxKind::QuotedString
                )
            {
                Some("a type or class is expected, not a value")
            } else {
                None
            }
        }
        Declared::ValueLike => {
            if first.kind == SyntaxKind::UpperName
                && !VALUE_WORDS.contains(&first.text.as_str())
                && !actual.iter().any(|t| t.kind == SyntaxKind::Colon)
            {
                Some("a value or a braced value set is expected, not a type")
            } else {
                None
            }
        }
        Declared::ObjectLike => {
            if first.kind == SyntaxKind::UpperName {
                Some(
                    "an object or a braced object set is expected (an object set is written in braces)",
                )
            } else if matches!(
                first.kind,
                SyntaxKind::Number | SyntaxKind::CString | SyntaxKind::QuotedString
            ) {
                Some("an object or a braced object set is expected, not a literal")
            } else {
                None
            }
        }
        Declared::Unknown | Declared::BadGovernor => None,
    }
}

// ---- recursion (X.683 8.6-8.8) ------------------------------------------------------------------

fn check_recursion(ctx: &mut Ctx<'_, '_, '_>, symbols: &[Symbol]) {
    // 8.6: parameterized values, value sets, objects and object sets cannot refer to themselves
    for (index, symbol) in symbols.iter().enumerate() {
        if !symbol.is_parameterized() {
            continue;
        }
        let sym = SymRef {
            module: ctx.module,
            index,
        };
        let value_like = matches!(
            ctx.sem.kind(sym),
            Kind::Value | Kind::ValueSet | Kind::Object | Kind::ObjectSet
        );
        if value_like {
            let shadowed: HashSet<&str> = symbol.parameters.iter().map(String::as_str).collect();
            let self_use = flatten(&symbol.node)
                .into_iter()
                .filter(|t| t.range != symbol.name_range)
                .find(|t| t.text == symbol.name && !shadowed.contains(t.text.as_str()));
            if let Some(token) = self_use {
                let finding = Finding::new(
                    "E164",
                    format!("`{}` refers to itself", symbol.name),
                    ctx.loc(token.range),
                );
                ctx.report(finding);
            }
            continue;
        }
        if ctx.sem.kind(sym) == Kind::Type || ctx.sem.kind(sym) == Kind::Class {
            check_type_recursion(ctx, sym, symbol);
        }
    }
}

// X.683 8.7, 8.8
fn check_type_recursion(ctx: &mut Ctx<'_, '_, '_>, sym: SymRef, symbol: &Symbol) {
    let Some(rhs) = &symbol.type_node else { return };
    let dummies = dummies_of(&symbol.node);
    // 8.8: a circular reference must be broken
    let mut found = Vec::new();
    unbroken_self_references(symbol, rhs, false, &mut found);
    for range in found {
        let finding = Finding::new(
            "E164",
            format!(
                "`{}` refers to itself with nothing to end the recursion: make the reference OPTIONAL or put it in a CHOICE",
                symbol.name
            ),
            ctx.loc(range),
        );
        ctx.report(finding);
    }
    // 8.7: along a recursive path, an actual parameter is either a lone dummy or has no dummy
    for node in rhs.descendants().chain(std::iter::once(rhs.clone())) {
        if node.kind() != SyntaxKind::DefinedType {
            continue;
        }
        let Some(list) = child(&node, SyntaxKind::ActualParameterList) else {
            continue;
        };
        let Some(Resolved::Sym(target)) = ctx.sem.resolve_defined(ctx.module, &node) else {
            continue;
        };
        if !reaches(ctx, target, sym, &mut HashSet::new()) {
            continue;
        }
        for actual in actuals_of(&list) {
            let mentions = actual
                .iter()
                .filter(|t| dummies.iter().any(|d| d.name == t.text))
                .count();
            let braced_lone = actual.len() == 3
                && actual.first().is_some_and(|t| t.kind == SyntaxKind::LBrace)
                && actual.last().is_some_and(|t| t.kind == SyntaxKind::RBrace);
            if mentions > 0 && actual.len() > 1 && !braced_lone {
                let finding = Finding::new(
                    "E164",
                    "an actual parameter on a recursive path must be a lone dummy reference or contain no dummy reference",
                    ctx.loc(actual.first().map_or(range_of(&node), |t| t.range)),
                );
                ctx.report(finding);
            }
        }
    }
}

fn reaches(ctx: &Ctx<'_, '_, '_>, from: SymRef, goal: SymRef, seen: &mut HashSet<SymRef>) -> bool {
    if from == goal {
        return true;
    }
    if !seen.insert(from) {
        return false;
    }
    let Some(symbol) = ctx.sem.symbol(from) else {
        return false;
    };
    let Some(rhs) = &symbol.type_node else {
        return false;
    };
    let shadow: Vec<&str> = symbol.parameters.iter().map(String::as_str).collect();
    rhs.descendants()
        .chain(std::iter::once(rhs.clone()))
        .any(|n| {
            if n.kind() != SyntaxKind::DefinedType {
                return false;
            }
            let Some(reference) = defined_type_ref(&n) else {
                return false;
            };
            if reference.module.is_none() && shadow.contains(&reference.name.as_str()) {
                return false;
            }
            match ctx.sem.resolve(
                from.module,
                &[],
                reference.module.as_deref(),
                &reference.name,
            ) {
                Resolved::Sym(next) => reaches(ctx, next, goal, seen),
                _ => false,
            }
        })
}

fn unbroken_self_references(
    symbol: &Symbol,
    node: &SyntaxNode,
    broken: bool,
    out: &mut Vec<TextRange>,
) {
    match node.kind() {
        SyntaxKind::DefinedType => {
            if let Some(reference) = defined_type_ref(node)
                && reference.module.is_none()
                && reference.name == symbol.name
                && !broken
                && !symbol.parameters.iter().any(|p| p == &reference.name)
            {
                out.push(reference.range);
            }
        }
        SyntaxKind::ComponentType => {
            let optional = tokens(node).iter().any(|t| {
                t.kind() == SyntaxKind::Keyword && matches!(t.text(), "OPTIONAL" | "DEFAULT")
            });
            for c in node.children() {
                unbroken_self_references(symbol, &c, broken || optional, out);
            }
        }
        SyntaxKind::ChoiceType | SyntaxKind::SequenceOfType | SyntaxKind::SetOfType => {
            for c in node.children() {
                unbroken_self_references(symbol, &c, true, out);
            }
        }
        SyntaxKind::ActualParameterList | SyntaxKind::BraceGroup | SyntaxKind::Constraint => {}
        _ => {
            for c in node.children() {
                unbroken_self_references(symbol, &c, broken, out);
            }
        }
    }
}

// X.683 8.12: an actual value must suit the governor of its dummy
fn check_value_actual(
    ctx: &mut Ctx<'_, '_, '_>,
    caller: &Symbol,
    target: SymRef,
    dummy: &Dummy,
    actual: &[Tk],
) {
    let (Some(governor), Some(first), Some(last)) =
        (&dummy.governor, actual.first(), actual.last())
    else {
        return;
    };
    if target.module != ctx.module
        || first.kind == SyntaxKind::LBrace
        || caller.parameters.iter().any(|p| p == &first.text)
    {
        return;
    }
    let text = ctx.sem.source_text(ctx.module).to_owned();
    let parsed = glint_syntax::parse_prefix(
        &text,
        usize::from(first.range.start()),
        usize::from(last.range.end()),
        glint_syntax::PrefixKind::Value,
        ctx.sem.parse_options(ctx.module),
    );
    let Some(value) = parsed
        .parse
        .syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::Value)
    else {
        return;
    };
    let before = ctx.findings.len();
    super::values::check_value(
        ctx,
        caller,
        governor,
        &super::values::elems_of(&value),
        false,
    );
    for finding in ctx.findings.iter_mut().skip(before) {
        if finding.code == "E131" {
            finding.code = "E167";
        }
    }
}

// ---- abstract syntax parameters (X.683 10) ---------------------------------------------------------

// X.683 10.2-10.4
pub(super) fn check_abstract_syntax(ctx: &mut Ctx<'_, '_, '_>, sym: SymRef, symbol: &Symbol) {
    if !symbol.is_parameterized() {
        return;
    }
    let ClassRes::Class(class) = ctx.sem.governor_class(sym) else {
        return;
    };
    if Some(class.module) != ctx.graph.builtin() {
        return;
    }
    let Some(info) = ctx.sem.class_info(class) else {
        return;
    };
    if ctx
        .sem
        .symbol(class)
        .is_none_or(|c| c.name != "ABSTRACT-SYNTAX")
    {
        return;
    }
    let Some(value) = &symbol.value_node else {
        return;
    };
    let Some(group) = child(value, SyntaxKind::BraceGroup) else {
        return;
    };
    let parsed = ctx.sem.parse_object(ctx.module, &info, &group);
    let Some(setting) = parsed.info.settings.iter().find(|s| s.field == "&Type") else {
        return;
    };
    let text = ctx.sem.source_text(ctx.module).to_owned();
    let result = glint_syntax::parse_prefix(
        &text,
        usize::from(setting.range.start()),
        usize::from(setting.range.end()),
        glint_syntax::PrefixKind::Type,
        ctx.sem.parse_options(ctx.module),
    );
    let root = result.parse.syntax();
    let Some(ty) = root
        .children()
        .find(|n| crate::view::is_type_node(n.kind()))
    else {
        return;
    };
    for dummy in dummies_of(&symbol.node) {
        match used_in_constraint(ctx, &ty, ctx.module, &dummy.name, 0) {
            Constraint::Unused => {
                let finding = Finding::new(
                    "E168",
                    format!(
                        "the abstract syntax parameter `{}` is not used in a constraint",
                        dummy.name
                    ),
                    ctx.loc(dummy.range),
                );
                ctx.report(finding);
            }
            Constraint::Variable { exception: false } => {
                let finding = Finding::new(
                    "W169",
                    format!(
                        "a constraint depends on the abstract syntax parameter `{}` and has no exception specification",
                        dummy.name
                    ),
                    ctx.loc(setting.range),
                )
                .with_help("add an exception specification such as `! Error : value` (X.680 53.4)");
                ctx.report(finding);
            }
            Constraint::Variable { exception: true } => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Constraint {
    Unused,
    Variable { exception: bool },
}

/// Whether `name` is used, directly or through actual parameters, inside a constraint of `node` (X.683 10.2).
fn used_in_constraint(
    ctx: &Ctx<'_, '_, '_>,
    node: &SyntaxNode,
    module: crate::graph::ModuleId,
    name: &str,
    depth: usize,
) -> Constraint {
    if depth > 6 {
        return Constraint::Unused;
    }
    let mut result = Constraint::Unused;
    for candidate in std::iter::once(node.clone()).chain(node.descendants()) {
        match candidate.kind() {
            SyntaxKind::Constraint => {
                if flatten(&candidate).iter().any(|t| t.text == name) {
                    let exception = candidate
                        .children()
                        .any(|c| c.kind() == SyntaxKind::ExceptionSpec);
                    return Constraint::Variable { exception };
                }
            }
            SyntaxKind::DefinedType => {
                let Some(list) = child(&candidate, SyntaxKind::ActualParameterList) else {
                    continue;
                };
                let Some(Resolved::Sym(callee)) = ctx.sem.resolve_defined(module, &candidate)
                else {
                    continue;
                };
                let Some(callee_symbol) = ctx.sem.symbol(callee) else {
                    continue;
                };
                let callee_dummies = dummies_of(&callee_symbol.node);
                for (actual, dummy) in actuals_of(&list).iter().zip(&callee_dummies) {
                    if !actual.iter().any(|t| t.text == name) {
                        continue;
                    }
                    let Some(rhs) = &callee_symbol.type_node else {
                        continue;
                    };
                    match used_in_constraint(
                        ctx,
                        rhs,
                        callee.module,
                        &dummy.name,
                        depth.saturating_add(1),
                    ) {
                        Constraint::Unused => {}
                        found @ Constraint::Variable { .. } => result = found,
                    }
                }
            }
            _ => {}
        }
    }
    result
}
