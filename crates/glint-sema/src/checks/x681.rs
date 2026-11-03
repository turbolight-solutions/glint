//! X.681 checks: E150, E152-E159 (classes, objects, object sets, information from objects).

use std::collections::{HashMap, HashSet};

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use super::Ctx;
use super::references::check_reference;
use crate::finding::Finding;
use crate::model::{Symbol, SymbolKind};
use crate::sem::toks::{Tk, flatten};
use crate::sem::{
    ClassInfo, ClassRes, Dummy, FieldKind, Kind, Operand, Resolved, SymRef, SynItem,
    parse_object_set_syntax, range_of,
};
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
        match ctx.sem.kind(sym) {
            Kind::Class => check_class(ctx, sym, symbol),
            Kind::Object => check_object_assignment(ctx, sym, symbol),
            Kind::ObjectSet => check_object_set_assignment(ctx, sym, symbol),
            _ => {}
        }
        check_field_references(ctx, symbol);
    }
    check_recursion(ctx, &symbols);
}

// ---- classes ----------------------------------------------------------------------------------

// X.681 9.3-9.15, 10
fn check_class(ctx: &mut Ctx<'_, '_, '_>, sym: SymRef, symbol: &Symbol) {
    let Some(info) = ctx.sem.class_info(sym) else {
        return;
    };
    let mut seen: HashMap<&str, TextRange> = HashMap::new();
    for field in &info.fields {
        // X.681 9.13
        if let Some(first) = seen.get(field.name.as_str()) {
            let finding = Finding::new(
                "E153",
                format!(
                    "field `{}` is defined more than once in the class",
                    field.name
                ),
                ctx.loc(field.name_range),
            )
            .with_secondary(ctx.loc(*first), "first defined here");
            ctx.report(finding);
        } else {
            seen.insert(field.name.as_str(), field.name_range);
        }
        check_field(ctx, symbol, &info, field);
    }
    check_chain_recursion(ctx, &info);
    if let Some(spec) = &info.syntax {
        check_syntax(ctx, &info, spec);
    }
}

fn check_field(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    info: &ClassInfo,
    field: &crate::sem::Field,
) {
    let sem = ctx.sem;
    if field.kind == FieldKind::Invalid {
        let finding = Finding::new(
            "E153",
            format!("value field `{}` has no type", field.name),
            ctx.loc(field.name_range),
        );
        ctx.report(finding);
    }
    // X.681 9.6
    if field.unique
        && (field.has_default
            || !matches!(field.kind, FieldKind::FixedValue | FieldKind::UnknownLower))
    {
        let why = if field.has_default {
            "`UNIQUE` cannot be combined with `DEFAULT`"
        } else {
            "`UNIQUE` applies only to a fixed-type value field"
        };
        let finding = Finding::new("E153", why, ctx.loc(field.name_range));
        ctx.report(finding);
    }
    // X.681 9.8, 9.10
    if let Some(link) = &field.link {
        check_variable_link(ctx, info, field, link);
    }
    // X.681 14.2 a, 14.4
    if matches!(field.kind, FieldKind::FixedValue | FieldKind::FixedValueSet)
        && let Some(governor) = &field.governor
        && let Some(open) = find_open_type(ctx, symbol, governor, 0)
    {
        let finding = Finding::new(
            "E159",
            format!(
                "the type of {} `{}` is or contains an open type",
                field.kind.describe(),
                field.name
            ),
            ctx.loc(open),
        )
        .with_note("an open type cannot be used, directly or indirectly, as the type of a value or value set field");
        ctx.report(finding);
    }
    let _ = sem;
}

// X.681 9.8 a-b, 9.10 a-b
fn check_variable_link(
    ctx: &mut Ctx<'_, '_, '_>,
    info: &ClassInfo,
    field: &crate::sem::Field,
    link: &SyntaxNode,
) {
    let names = field_names(link);
    let result = resolve_path(ctx, info.sym, &names);
    match result {
        PathResult::Found { field: target } => {
            if target.kind != FieldKind::Type {
                let finding = Finding::new(
                    "E153",
                    format!(
                        "`{}` must name a type field, but `{}` is a {}",
                        render_path(&names),
                        target.name,
                        target.kind.describe()
                    ),
                    ctx.loc(range_of(link)),
                );
                ctx.report(finding);
                return;
            }
            if target.optional && !field.optional {
                let finding = Finding::new(
                    "E153",
                    format!(
                        "`{}` is OPTIONAL, so `{}` must be OPTIONAL too",
                        target.name, field.name
                    ),
                    ctx.loc(field.name_range),
                );
                ctx.report(finding);
            }
            if field.has_default && !target.has_default {
                let finding = Finding::new(
                    "E153",
                    format!(
                        "`{}` has a DEFAULT, so `{}` needs one too",
                        field.name, target.name
                    ),
                    ctx.loc(field.name_range),
                );
                ctx.report(finding);
            }
        }
        PathResult::NotLink { at, name } => {
            let finding = Finding::new(
                "E153",
                format!(
                    "`{name}` is not an object or object set field, so the chain cannot continue through it"
                ),
                ctx.loc(at),
            );
            ctx.report(finding);
        }
        PathResult::NoField { at, name, class } => {
            let finding = Finding::new(
                "E150",
                format!("class `{class}` has no field `{name}`"),
                ctx.loc(at),
            );
            ctx.report(finding);
        }
        PathResult::Unknown => {}
    }
}

/// Finds an open type used directly or through type references in `node` (X.681 14.2 a).
fn find_open_type(
    ctx: &Ctx<'_, '_, '_>,
    symbol: &Symbol,
    node: &SyntaxNode,
    depth: usize,
) -> Option<TextRange> {
    if depth > 8 {
        return None;
    }
    let sem = ctx.sem;
    for candidate in std::iter::once(node.clone()).chain(node.descendants()) {
        match candidate.kind() {
            SyntaxKind::ObjectClassFieldType => {
                if let Some(kind) = field_type_kind(ctx, &candidate)
                    && matches!(
                        kind,
                        FieldKind::Type | FieldKind::VariableValue | FieldKind::VariableValueSet
                    )
                {
                    return Some(range_of(&candidate));
                }
            }
            SyntaxKind::DefinedType => {
                let Some(Resolved::Sym(target)) = sem.resolve_defined(ctx.module, &candidate)
                else {
                    continue;
                };
                if let Some(target_symbol) = sem.symbol(target)
                    && target_symbol.kind == SymbolKind::Type
                    && !target_symbol.is_parameterized()
                    && let Some(rhs) = &target_symbol.type_node
                    && rhs.kind() != SyntaxKind::ObjectClassDef
                    && sem.kind(target) == Kind::Type
                    && find_open_type_in(ctx, target, rhs, depth.saturating_add(1)).is_some()
                {
                    return Some(range_of(&candidate));
                }
            }
            _ => {}
        }
    }
    let _ = symbol;
    None
}

fn find_open_type_in(
    ctx: &Ctx<'_, '_, '_>,
    target: SymRef,
    node: &SyntaxNode,
    depth: usize,
) -> Option<TextRange> {
    if depth > 8 {
        return None;
    }
    let sem = ctx.sem;
    for candidate in std::iter::once(node.clone()).chain(node.descendants()) {
        match candidate.kind() {
            SyntaxKind::ObjectClassFieldType => {
                if let Some(kind) = field_type_kind_in(ctx, target.module, &candidate)
                    && matches!(
                        kind,
                        FieldKind::Type | FieldKind::VariableValue | FieldKind::VariableValueSet
                    )
                {
                    return Some(range_of(&candidate));
                }
            }
            SyntaxKind::DefinedType => {
                let Some(Resolved::Sym(next)) = sem.resolve_defined(target.module, &candidate)
                else {
                    continue;
                };
                if next != target
                    && let Some(next_symbol) = sem.symbol(next)
                    && next_symbol.kind == SymbolKind::Type
                    && !next_symbol.is_parameterized()
                    && let Some(rhs) = &next_symbol.type_node
                    && rhs.kind() != SyntaxKind::ObjectClassDef
                    && find_open_type_in(ctx, next, rhs, depth.saturating_add(1)).is_some()
                {
                    return Some(range_of(&candidate));
                }
            }
            _ => {}
        }
    }
    None
}

/// The kind of the final field that an `ObjectClassFieldType` names, when it can be determined.
pub(super) fn field_type_kind(ctx: &Ctx<'_, '_, '_>, node: &SyntaxNode) -> Option<FieldKind> {
    field_type_kind_in(ctx, ctx.module, node)
}

fn field_type_kind_in(
    ctx: &Ctx<'_, '_, '_>,
    module: crate::graph::ModuleId,
    node: &SyntaxNode,
) -> Option<FieldKind> {
    let sem = ctx.sem;
    let (qualifier, name) = class_name_of(node)?;
    let scope = sem.scope_of(node);
    let Resolved::Sym(target) = sem.resolve(module, &scope, qualifier.as_deref(), &name) else {
        return None;
    };
    let ClassRes::Class(class) = sem.class_res_of_symbol(target) else {
        return None;
    };
    let names = field_names(&child(node, SyntaxKind::FieldName)?);
    match resolve_path_in(ctx, class, &names) {
        PathResult::Found { field } => Some(field.kind),
        _ => None,
    }
}

fn class_name_of(node: &SyntaxNode) -> Option<(Option<String>, String)> {
    let names: Vec<_> = tokens(node)
        .into_iter()
        .filter(|t| t.kind() == SyntaxKind::UpperName)
        .collect();
    match names.as_slice() {
        [name] => Some((None, name.text().to_owned())),
        [module, name, ..] => Some((Some(module.text().to_owned()), name.text().to_owned())),
        [] => None,
    }
}

// X.681 9.15
fn check_chain_recursion(ctx: &mut Ctx<'_, '_, '_>, info: &ClassInfo) {
    for field in &info.fields {
        let (FieldKind::Object(ClassRes::Class(target))
        | FieldKind::ObjectSet(ClassRes::Class(target))) = field.kind
        else {
            continue;
        };
        if field.optional || field.has_default {
            continue;
        }
        if reaches_without_optional(ctx, target, info.sym, &mut HashSet::new()) {
            let finding = Finding::new(
                "E153",
                format!(
                    "field `{}` makes the class recursive with no OPTIONAL or DEFAULT link in the chain",
                    field.name
                ),
                ctx.loc(field.name_range),
            );
            ctx.report(finding);
        }
    }
}

fn reaches_without_optional(
    ctx: &Ctx<'_, '_, '_>,
    from: SymRef,
    goal: SymRef,
    seen: &mut HashSet<SymRef>,
) -> bool {
    if from == goal {
        return true;
    }
    if !seen.insert(from) {
        return false;
    }
    let Some(info) = ctx.sem.class_info(from) else {
        return false;
    };
    info.fields.iter().any(|f| match f.kind {
        FieldKind::Object(ClassRes::Class(next)) | FieldKind::ObjectSet(ClassRes::Class(next)) => {
            !f.optional && !f.has_default && reaches_without_optional(ctx, next, goal, seen)
        }
        _ => false,
    })
}

// X.681 10.5-10.12
fn check_syntax(ctx: &mut Ctx<'_, '_, '_>, info: &ClassInfo, spec: &crate::sem::SyntaxSpec) {
    for (message, range) in &spec.problems {
        let finding = Finding::new("E154", message.clone(), ctx.loc(*range));
        ctx.report(finding);
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut fields = Vec::new();
    collect_syntax_fields(&spec.items, &mut fields);
    for (name, range) in &fields {
        let count = counts.entry(name.as_str()).or_insert(0);
        *count = count.saturating_add(1);
        if *count == 2 {
            let finding = Finding::new(
                "E154",
                format!("`{name}` appears more than once in the syntax list"),
                ctx.loc(*range),
            );
            ctx.report(finding);
        }
        if info.field(name).is_none() {
            let finding = Finding::new(
                "E154",
                format!("the syntax list names `{name}`, which is not a field of the class"),
                ctx.loc(*range),
            );
            ctx.report(finding);
        }
    }
    for field in &info.fields {
        if !counts.contains_key(field.name.as_str()) {
            let finding = Finding::new(
                "E154",
                format!("field `{}` does not appear in the syntax list", field.name),
                ctx.loc(spec.range),
            )
            .with_secondary(ctx.loc(field.name_range), "the field");
            ctx.report(finding);
        }
    }
    check_items(ctx, &spec.items);
}

fn collect_syntax_fields(items: &[SynItem], out: &mut Vec<(String, TextRange)>) {
    for item in items {
        match item {
            SynItem::Field(name, range) => out.push((name.clone(), *range)),
            SynItem::Group(nested, _) => collect_syntax_fields(nested, out),
            SynItem::Literal(..) => {}
        }
    }
}

fn check_items(ctx: &mut Ctx<'_, '_, '_>, items: &[SynItem]) {
    let mut preceding: Vec<&str> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            SynItem::Literal(text, range) => {
                preceding.clear();
                if crate::sem::FORBIDDEN_LITERALS.contains(&text.as_str()) {
                    let finding = Finding::new(
                        "E154",
                        format!(
                            "`{text}` is a reserved word that cannot be used as a literal in a syntax list"
                        ),
                        ctx.loc(*range),
                    );
                    ctx.report(finding);
                }
            }
            SynItem::Field(..) => preceding.clear(),
            SynItem::Group(nested, range) => {
                // X.681 10.12 a
                if !crate::sem::contains_field(nested) {
                    let finding = Finding::new(
                        "E154",
                        "an optional group needs at least one field",
                        ctx.loc(*range),
                    );
                    ctx.report(finding);
                }
                // X.681 10.12 c
                if let Some(first) = nested.first().and_then(crate::sem::first_literal) {
                    if let Some(next) = items.get(index.saturating_add(1))
                        && !matches!(next, SynItem::Group(..))
                        && !matches!(next, SynItem::Literal(..))
                    {
                        let finding = Finding::new(
                            "E154",
                            "an optional group that starts with a literal must be followed by a literal",
                            ctx.loc(*range),
                        );
                        ctx.report(finding);
                    }
                    if preceding.contains(&first) {
                        let finding = Finding::new(
                            "E154",
                            format!(
                                "`{first}` also starts an adjacent optional group, so the syntax is ambiguous"
                            ),
                            ctx.loc(*range),
                        );
                        ctx.report(finding);
                    }
                    preceding.push(first);
                } else {
                    preceding.clear();
                }
                check_items(ctx, nested);
            }
        }
    }
}

// ---- paths through fields -----------------------------------------------------------------------

pub(super) enum PathResult {
    Found {
        field: crate::sem::Field,
    },
    NotLink {
        at: TextRange,
        name: String,
    },
    NoField {
        at: TextRange,
        name: String,
        class: String,
    },
    Unknown,
}

pub(super) fn field_names(node: &SyntaxNode) -> Vec<(String, TextRange)> {
    tokens(node)
        .into_iter()
        .filter(|t| matches!(t.kind(), SyntaxKind::FieldUpper | SyntaxKind::FieldLower))
        .map(|t| (t.text().to_owned(), t.text_range()))
        .collect()
}

fn render_path(names: &[(String, TextRange)]) -> String {
    names
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn resolve_path(ctx: &Ctx<'_, '_, '_>, class: SymRef, names: &[(String, TextRange)]) -> PathResult {
    resolve_path_in(ctx, class, names)
}

/// Follows a `FieldName` from `class` (X.681 9.14).
pub(super) fn resolve_path_in(
    ctx: &Ctx<'_, '_, '_>,
    class: SymRef,
    names: &[(String, TextRange)],
) -> PathResult {
    let mut current = class;
    for (position, (name, range)) in names.iter().enumerate() {
        let Some(info) = ctx.sem.class_info(current) else {
            return PathResult::Unknown;
        };
        let Some(field) = info.field(name) else {
            let class_name = ctx
                .sem
                .symbol(current)
                .map_or_else(String::new, |s| s.name.clone());
            return PathResult::NoField {
                at: *range,
                name: name.clone(),
                class: class_name,
            };
        };
        if position.saturating_add(1) == names.len() {
            return PathResult::Found {
                field: field.clone(),
            };
        }
        match field.kind {
            FieldKind::Object(ClassRes::Class(next))
            | FieldKind::ObjectSet(ClassRes::Class(next)) => {
                current = next;
            }
            FieldKind::Object(_) | FieldKind::ObjectSet(_) => return PathResult::Unknown,
            _ => {
                return PathResult::NotLink {
                    at: *range,
                    name: name.clone(),
                };
            }
        }
    }
    PathResult::Unknown
}

// ---- information from objects (X.681 14.1, 14.5, 15) ----------------------------------------------

/// What the part before `.&field` denotes.
enum Base {
    Class(SymRef),
    Object(SymRef),
    ObjectSet(SymRef),
    Unknown,
    Other,
}

fn base_of(ctx: &Ctx<'_, '_, '_>, scope: &[Dummy], qualifier: Option<&str>, name: &str) -> Base {
    let sem = ctx.sem;
    match sem.resolve(ctx.module, scope, qualifier, name) {
        Resolved::Sym(target) => match sem.kind(target) {
            Kind::Class => match sem.class_res_of_symbol(target) {
                ClassRes::Class(c) => Base::Class(c),
                _ => Base::Unknown,
            },
            Kind::Object => sem.object_class(target).map_or(Base::Unknown, Base::Object),
            Kind::ObjectSet => match sem.governor_class(target) {
                ClassRes::Class(c) => Base::ObjectSet(c),
                _ => Base::Unknown,
            },
            Kind::Unknown | Kind::Macro => Base::Unknown,
            _ => Base::Other,
        },
        _ => Base::Unknown,
    }
}

/// Walks every `Class.&f`, `obj.&f` and `Set.&f` in the assignment (X.681 14.1, 15.4-15.5).
fn check_field_references(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol) {
    for node in symbol.node.descendants() {
        match node.kind() {
            SyntaxKind::ObjectClassFieldType => {
                if inside_raw(&node) {
                    continue;
                }
                let Some((qualifier, name)) = class_name_of(&node) else {
                    continue;
                };
                let Some(names_node) = child(&node, SyntaxKind::FieldName) else {
                    continue;
                };
                let scope = ctx.sem.scope_of(&node);
                let base = base_of(ctx, &scope, qualifier.as_deref(), &name);
                check_info_path(
                    ctx,
                    &base,
                    &field_names(&names_node),
                    What::TypeOrClassField,
                    &name,
                );
            }
            SyntaxKind::TypeFromObject => {
                if inside_raw(&node) {
                    continue;
                }
                let Some(name_token) = tokens(&node)
                    .into_iter()
                    .find(|t| t.kind() == SyntaxKind::LowerName)
                else {
                    continue;
                };
                let Some(names_node) = child(&node, SyntaxKind::FieldName) else {
                    continue;
                };
                let scope = ctx.sem.scope_of(&node);
                let base = base_of(ctx, &scope, None, name_token.text());
                check_info_path(
                    ctx,
                    &base,
                    &field_names(&names_node),
                    What::Type,
                    name_token.text(),
                );
            }
            SyntaxKind::Value => {
                if inside_raw(&node) {
                    continue;
                }
                let toks = tokens(&node);
                let (Some(name_token), Some(names_node)) = (
                    toks.iter().find(|t| t.kind() == SyntaxKind::LowerName),
                    child(&node, SyntaxKind::FieldName),
                ) else {
                    continue;
                };
                let scope = ctx.sem.scope_of(&node);
                let base = base_of(ctx, &scope, None, name_token.text());
                check_info_path(
                    ctx,
                    &base,
                    &field_names(&names_node),
                    What::Value,
                    name_token.text(),
                );
            }
            _ => {}
        }
    }
}

fn inside_raw(node: &SyntaxNode) -> bool {
    node.ancestors().skip(1).any(|n| {
        matches!(
            n.kind(),
            SyntaxKind::BraceGroup | SyntaxKind::ActualParameter | SyntaxKind::MacroUse
        )
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum What {
    TypeOrClassField,
    Type,
    Value,
}

// X.681 14.5, 15.4, 15.5 Table 1, 15.11
fn check_info_path(
    ctx: &mut Ctx<'_, '_, '_>,
    base: &Base,
    names: &[(String, TextRange)],
    what: What,
    base_name: &str,
) {
    let (class, base_is) = match base {
        Base::Class(c) => (*c, 0u8),
        Base::Object(c) => (*c, 1),
        Base::ObjectSet(c) => (*c, 2),
        Base::Unknown => return,
        Base::Other => {
            if let Some((_, range)) = names.first() {
                let finding = Finding::new(
                    "E150",
                    format!(
                        "`{base_name}` is neither a class, an object nor an object set, so `.{}` cannot follow it",
                        names.first().map_or("", |n| n.0.as_str())
                    ),
                    ctx.loc(*range),
                );
                ctx.report(finding);
            }
            return;
        }
    };
    match resolve_path_in(ctx, class, names) {
        PathResult::Found { field } => {
            let kind = field.kind;
            let problem = match (base_is, what) {
                (0, _) if kind.is_link() => Some(format!(
                    "`{}` is an {}, which cannot be used as a type",
                    field.name,
                    kind.describe()
                )),
                (1, What::Type) if kind != FieldKind::Type => Some(format!(
                    "`{}.{}` is a {}, but a type is needed here",
                    base_name,
                    field.name,
                    kind.describe()
                )),
                (1, _) if kind == FieldKind::VariableValueSet => Some(format!(
                    "a {} cannot be read from an object (X.681 Table 1)",
                    kind.describe()
                )),
                (2, _)
                    if matches!(
                        kind,
                        FieldKind::Type | FieldKind::VariableValue | FieldKind::VariableValueSet
                    ) =>
                {
                    Some(format!(
                        "a {} cannot be read from an object set (X.681 15.11)",
                        kind.describe()
                    ))
                }
                _ => None,
            };
            if let Some(message) = problem
                && let Some((_, last)) = names.last()
            {
                let finding = Finding::new("E150", message, ctx.loc(*last));
                ctx.report(finding);
            }
        }
        PathResult::NotLink { at, name } => {
            let finding = Finding::new(
                "E150",
                format!(
                    "`{name}` is not an object or object set field, so the chain cannot continue through it"
                ),
                ctx.loc(at),
            );
            ctx.report(finding);
        }
        PathResult::NoField { at, name, class } => {
            let finding = Finding::new(
                "E150",
                format!("class `{class}` has no field `{name}`"),
                ctx.loc(at),
            );
            ctx.report(finding);
        }
        PathResult::Unknown => {}
    }
}

// ---- objects ------------------------------------------------------------------------------------

// X.681 11.1-11.7
fn check_object_assignment(ctx: &mut Ctx<'_, '_, '_>, sym: SymRef, symbol: &Symbol) {
    let ClassRes::Class(class_sym) = ctx.sem.governor_class(sym) else {
        return;
    };
    let Some(class) = ctx.sem.class_info(class_sym) else {
        return;
    };
    let Some(value) = &symbol.value_node else {
        return;
    };
    match value.kind() {
        SyntaxKind::AmbiguousRhs => {
            if let Some(group) = child(value, SyntaxKind::BraceGroup) {
                check_object_body(
                    ctx,
                    symbol,
                    &class,
                    &crate::sem::toks::inner(&group),
                    range_of(&group),
                );
            }
        }
        SyntaxKind::Value => check_object_reference_rhs(ctx, symbol, &class, value),
        _ => {}
    }
}

fn check_object_body(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    class: &ClassInfo,
    tokens_inner: &[Tk],
    range: TextRange,
) {
    let parsed = ctx
        .sem
        .parse_object_tokens(ctx.module, class, tokens_inner, range);
    for problem in &parsed.problems {
        let finding = Finding::new(
            problem.code,
            problem.message.clone(),
            ctx.loc(problem.range),
        );
        ctx.report(finding);
    }
    if !parsed.problems.is_empty() {
        return;
    }
    let scope = ctx.sem.scope_of(&symbol.node);
    for setting in &parsed.info.settings {
        check_setting_references(ctx, symbol, &scope, setting);
    }
}

/// Checks the names a setting uses (X.680 14.4-14.7).
fn check_setting_references(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    scope: &[Dummy],
    setting: &crate::sem::Setting,
) {
    let text = ctx.sem.source_text(ctx.module).to_owned();
    let start = usize::from(setting.range.start());
    let limit = usize::from(setting.range.end());
    let options = ctx.sem.parse_options(ctx.module);
    match setting.kind {
        FieldKind::Type => {
            let parsed = glint_syntax::parse_prefix(
                &text,
                start,
                limit,
                glint_syntax::PrefixKind::Type,
                options,
            );
            check_fragment_references(ctx, symbol, scope, &parsed.parse.syntax());
        }
        FieldKind::FixedValue
        | FieldKind::VariableValue
        | FieldKind::Object(_)
        | FieldKind::UnknownLower => {
            let parsed = glint_syntax::parse_prefix(
                &text,
                start,
                limit,
                glint_syntax::PrefixKind::Value,
                options,
            );
            let root = parsed.parse.syntax();
            let toks = tokens_of(&root);
            if let [only] = toks.as_slice()
                && only.kind == SyntaxKind::LowerName
                && !scope.iter().any(|d| d.name == only.text)
                && !is_identifier_of_governor(ctx, symbol, setting, &only.text)
            {
                check_reference(ctx, symbol, None, &only.text, only.range, "value");
            }
            check_fragment_references(ctx, symbol, scope, &root);
        }
        _ => {}
    }
}

fn tokens_of(root: &SyntaxNode) -> Vec<Tk> {
    flatten(root)
}

fn check_fragment_references(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    scope: &[Dummy],
    root: &SyntaxNode,
) {
    for node in root.descendants() {
        if node.kind() != SyntaxKind::DefinedType {
            continue;
        }
        if node.ancestors().skip(1).any(|a| {
            matches!(
                a.kind(),
                SyntaxKind::BraceGroup | SyntaxKind::ActualParameter
            )
        }) {
            continue;
        }
        let Some(reference) = defined_type_ref(&node) else {
            continue;
        };
        if reference.module.is_none() && scope.iter().any(|d| d.name == reference.name) {
            continue;
        }
        check_reference(
            ctx,
            symbol,
            reference.module.as_deref(),
            &reference.name,
            reference.range,
            "type",
        );
    }
}

// X.681 11.3 a, c
fn check_object_reference_rhs(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    class: &ClassInfo,
    value: &SyntaxNode,
) {
    let toks = flatten(value);
    let Some(first) = toks.first() else { return };
    if first.kind != SyntaxKind::LowerName {
        return;
    }
    let scope = ctx.sem.scope_of(&symbol.node);
    let mut operand_tokens = toks.clone();
    operand_tokens.retain(|_| true);
    check_named_object(ctx, symbol, &scope, class, &toks, true);
}

/// Checks a reference to an object or object set where a member of `class` is needed (X.681 12.10 a-b).
fn check_named_object(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    scope: &[Dummy],
    class: &ClassInfo,
    toks: &[Tk],
    expect_object: bool,
) {
    let Some(first) = toks.first() else { return };
    let (qualifier, name_token, rest_at) = match (toks.first(), toks.get(1), toks.get(2)) {
        (Some(module), Some(dot), Some(name))
            if dot.kind == SyntaxKind::Dot
                && module.kind == SyntaxKind::UpperName
                && name.kind.is_name() =>
        {
            (Some(module.text.clone()), name.clone(), 3usize)
        }
        _ => (None, first.clone(), 1usize),
    };
    let rest = toks.get(rest_at..).unwrap_or_default();
    let path: Vec<(String, TextRange)> = rest
        .iter()
        .filter(|t| matches!(t.kind, SyntaxKind::FieldUpper | SyntaxKind::FieldLower))
        .map(|t| (t.text.clone(), t.range))
        .collect();
    let resolved = ctx
        .sem
        .resolve(ctx.module, scope, qualifier.as_deref(), &name_token.text);
    match resolved {
        Resolved::Missing | Resolved::NotImported(_) => {
            check_reference(
                ctx,
                symbol,
                qualifier.as_deref(),
                &name_token.text,
                name_token.range,
                if expect_object {
                    "object"
                } else {
                    "object set"
                },
            );
        }
        Resolved::Dummy(_) | Resolved::Opaque => {}
        Resolved::Sym(target) => {
            let kind = ctx.sem.kind(target);
            let target_class = match kind {
                Kind::Object => ctx.sem.object_class(target),
                Kind::ObjectSet => match ctx.sem.governor_class(target) {
                    ClassRes::Class(c) => Some(c),
                    _ => None,
                },
                Kind::Unknown => return,
                other => {
                    let what = match other {
                        Kind::Type => "a type",
                        Kind::Class => "a class",
                        Kind::Value => "a value",
                        Kind::ValueSet => "a value set",
                        _ => "something else",
                    };
                    let finding = Finding::new(
                        "E155",
                        format!(
                            "`{}` is {what}, not an {} of class `{}`",
                            name_token.text,
                            if expect_object {
                                "object"
                            } else {
                                "object set"
                            },
                            class_name(ctx, class.sym)
                        ),
                        ctx.loc(name_token.range),
                    );
                    ctx.report(finding);
                    return;
                }
            };
            let result_class = if path.is_empty() {
                target_class
            } else if let Some(c) = target_class {
                match resolve_path_in(ctx, c, &path) {
                    PathResult::Found { field } => match field.kind {
                        FieldKind::Object(ClassRes::Class(c))
                        | FieldKind::ObjectSet(ClassRes::Class(c)) => Some(c),
                        FieldKind::Object(_) | FieldKind::ObjectSet(_) => None,
                        other => {
                            let finding = Finding::new(
                                "E150",
                                format!(
                                    "`{}` is a {}, not an object or object set",
                                    field.name,
                                    other.describe()
                                ),
                                ctx.loc(path.last().map_or(name_token.range, |p| p.1)),
                            );
                            ctx.report(finding);
                            None
                        }
                    },
                    PathResult::NotLink { at, name } => {
                        let finding = Finding::new(
                            "E150",
                            format!(
                                "`{name}` is not an object or object set field, so the chain cannot continue through it"
                            ),
                            ctx.loc(at),
                        );
                        ctx.report(finding);
                        None
                    }
                    PathResult::NoField { at, name, class } => {
                        let finding = Finding::new(
                            "E150",
                            format!("class `{class}` has no field `{name}`"),
                            ctx.loc(at),
                        );
                        ctx.report(finding);
                        None
                    }
                    PathResult::Unknown => None,
                }
            } else {
                None
            };
            if let Some(found) = result_class
                && found != class.sym
            {
                let finding = Finding::new(
                    "E155",
                    format!(
                        "`{}` is of class `{}`, but class `{}` is needed",
                        name_token.text,
                        class_name(ctx, found),
                        class_name(ctx, class.sym)
                    ),
                    ctx.loc(name_token.range),
                );
                ctx.report(finding);
            }
        }
    }
}

fn class_name(ctx: &Ctx<'_, '_, '_>, class: SymRef) -> String {
    ctx.sem
        .symbol(class)
        .map_or_else(String::new, |s| s.name.clone())
}

// ---- object sets ----------------------------------------------------------------------------------

// X.681 12.1-12.3, 12.10
fn check_object_set_assignment(ctx: &mut Ctx<'_, '_, '_>, sym: SymRef, symbol: &Symbol) {
    let ClassRes::Class(class_sym) = ctx.sem.governor_class(sym) else {
        return;
    };
    let Some(class) = ctx.sem.class_info(class_sym) else {
        return;
    };
    let Some(value) = &symbol.value_node else {
        return;
    };
    let Some(group) = child(value, SyntaxKind::BraceGroup) else {
        return;
    };
    check_object_set_body(ctx, symbol, &class, &group, true);
    check_unique(ctx, sym, &class, &group);
}

pub(super) fn check_object_set_body(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    class: &ClassInfo,
    group: &SyntaxNode,
    allow_empty_check: bool,
) {
    let syntax = parse_object_set_syntax(group);
    // X.681 12.3
    if allow_empty_check && syntax.operands.is_empty() && !syntax.extensible {
        let finding = Finding::new(
            "E158",
            "an object set needs at least one object unless it is written `{ ... }`",
            ctx.loc(range_of(group)),
        );
        ctx.report(finding);
    }
    let scope = ctx.sem.scope_of(&symbol.node);
    for operand in &syntax.operands {
        match operand {
            Operand::Inline { body, range } => {
                check_object_body(ctx, symbol, class, body, *range);
            }
            Operand::Named {
                qualifier,
                name,
                name_range,
                lower,
                path,
                range,
                ..
            } => {
                let mut toks: Vec<Tk> = Vec::new();
                if let Some(q) = qualifier {
                    toks.push(Tk {
                        kind: SyntaxKind::UpperName,
                        text: q.clone(),
                        range: TextRange::empty(range.start()),
                    });
                    toks.push(Tk {
                        kind: SyntaxKind::Dot,
                        text: ".".into(),
                        range: TextRange::empty(range.start()),
                    });
                }
                toks.push(Tk {
                    kind: if *lower {
                        SyntaxKind::LowerName
                    } else {
                        SyntaxKind::UpperName
                    },
                    text: name.clone(),
                    range: *name_range,
                });
                for (field, field_range) in path {
                    toks.push(Tk {
                        kind: if field.chars().nth(1).is_some_and(char::is_uppercase) {
                            SyntaxKind::FieldUpper
                        } else {
                            SyntaxKind::FieldLower
                        },
                        text: field.clone(),
                        range: *field_range,
                    });
                }
                check_named_object(ctx, symbol, &scope, class, &toks, *lower);
            }
            Operand::Other { range } => {
                let finding =
                    Finding::new("E152", "unexpected token in an object set", ctx.loc(*range));
                ctx.report(finding);
            }
        }
    }
}

// X.681 9.7
fn check_unique(ctx: &mut Ctx<'_, '_, '_>, set: SymRef, class: &ClassInfo, group: &SyntaxNode) {
    let unique: Vec<&crate::sem::Field> = class.fields.iter().filter(|f| f.unique).collect();
    if unique.is_empty() {
        return;
    }
    let syntax = parse_object_set_syntax(group);
    if syntax.arithmetic {
        return;
    }
    let mut members: Vec<(Member, crate::sem::ObjectInfo, crate::graph::ModuleId)> = Vec::new();
    let mut visited = HashSet::new();
    visited.insert(set);
    collect_members(
        ctx,
        ctx.module,
        class,
        &syntax,
        &mut members,
        &mut visited,
        0,
    );
    for field in unique {
        let mut seen: HashMap<String, (Member, TextRange)> = HashMap::new();
        for (member, object, module) in &members {
            let Some(setting) = object.settings.iter().find(|s| s.field == field.name) else {
                continue;
            };
            let Some(key) = unique_key(ctx, *module, setting) else {
                continue;
            };
            match seen.get(&key) {
                Some((first, first_range)) if first != member => {
                    let finding = Finding::new(
                        "E156",
                        format!(
                            "two objects in this set have the same value for the UNIQUE field `{}`",
                            field.name
                        ),
                        ctx.loc(member.range_in(ctx.module, setting.range)),
                    )
                    .with_secondary(ctx.loc(*first_range), "the other object");
                    if member.module == ctx.module {
                        ctx.report(finding);
                    }
                }
                Some(_) => {}
                None => {
                    seen.insert(key, (*member, member.range_in(ctx.module, setting.range)));
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Member {
    module: crate::graph::ModuleId,
    id: usize,
}

impl Member {
    fn range_in(&self, _module: crate::graph::ModuleId, range: TextRange) -> TextRange {
        range
    }
}

fn collect_members(
    ctx: &Ctx<'_, '_, '_>,
    module: crate::graph::ModuleId,
    class: &ClassInfo,
    syntax: &crate::sem::ObjectSetSyntax,
    out: &mut Vec<(Member, crate::sem::ObjectInfo, crate::graph::ModuleId)>,
    visited: &mut HashSet<SymRef>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let sem = ctx.sem;
    for operand in &syntax.operands {
        match operand {
            Operand::Inline { body, range } => {
                let parsed = sem.parse_object_tokens(module, class, body, *range);
                let id = usize::from(range.start());
                out.push((Member { module, id }, parsed.info, module));
            }
            Operand::Named {
                qualifier,
                name,
                lower,
                path,
                actuals,
                ..
            } => {
                if !path.is_empty() || actuals.is_some() {
                    continue;
                }
                let scope = Vec::new();
                let Resolved::Sym(target) = sem.resolve(module, &scope, qualifier.as_deref(), name)
                else {
                    continue;
                };
                if !visited.insert(target) && !*lower {
                    continue;
                }
                match sem.kind(target) {
                    Kind::Object => {
                        let Some(symbol) = sem.symbol(target) else {
                            continue;
                        };
                        let Some(value) = &symbol.value_node else {
                            continue;
                        };
                        let Some(group) = child(value, SyntaxKind::BraceGroup) else {
                            continue;
                        };
                        let parsed = sem.parse_object(target.module, class, &group);
                        out.push((
                            Member {
                                module: target.module,
                                id: target.index.saturating_add(1_000_000),
                            },
                            parsed.info,
                            target.module,
                        ));
                    }
                    Kind::ObjectSet => {
                        let Some(symbol) = sem.symbol(target) else {
                            continue;
                        };
                        let Some(value) = &symbol.value_node else {
                            continue;
                        };
                        let Some(group) = child(value, SyntaxKind::BraceGroup) else {
                            continue;
                        };
                        let nested = parse_object_set_syntax(&group);
                        if nested.arithmetic {
                            continue;
                        }
                        collect_members(
                            ctx,
                            target.module,
                            class,
                            &nested,
                            out,
                            visited,
                            depth.saturating_add(1),
                        );
                    }
                    _ => {}
                }
            }
            Operand::Other { .. } => {}
        }
    }
    let _ = type_child;
}

/// A comparable form of the value a UNIQUE field is set to.
fn unique_key(
    ctx: &Ctx<'_, '_, '_>,
    module: crate::graph::ModuleId,
    setting: &crate::sem::Setting,
) -> Option<String> {
    let text = ctx.sem.source_text(module);
    let options = ctx.sem.parse_options(module);
    let parsed = glint_syntax::parse_prefix(
        text,
        usize::from(setting.range.start()),
        usize::from(setting.range.end()),
        glint_syntax::PrefixKind::Value,
        options,
    );
    if !parsed.ok {
        return None;
    }
    let root = parsed.parse.syntax();
    let value = root.children().find(|n| n.kind() == SyntaxKind::Value)?;
    let toks = flatten(&value);
    match toks.as_slice() {
        [only] if only.kind == SyntaxKind::LowerName => ctx
            .eval
            .oid_of_name(module, None, &only.text)
            .map(|oid| format!("oid:{oid}"))
            .or_else(|| {
                ctx.eval
                    .integer_of_name(module, None, &only.text)
                    .map(|n| format!("int:{n}"))
            }),
        [only] if only.kind == SyntaxKind::Number => Some(format!("int:{}", only.text)),
        [only] if matches!(only.kind, SyntaxKind::CString | SyntaxKind::QuotedString) => {
            Some(format!("str:{}", only.text))
        }
        _ => {
            let group = child(&value, SyntaxKind::BraceGroup)?;
            ctx.eval
                .eval_oid_group(module, &group)
                .oid
                .map(|oid| format!("oid:{oid}"))
        }
    }
}

// ---- recursion (X.681 11.2, 12.2) --------------------------------------------------------------------

fn check_recursion(ctx: &mut Ctx<'_, '_, '_>, symbols: &[Symbol]) {
    for (index, symbol) in symbols.iter().enumerate() {
        let sym = SymRef {
            module: ctx.module,
            index,
        };
        if !matches!(ctx.sem.kind(sym), Kind::Object | Kind::ObjectSet) || symbol.is_parameterized()
        {
            continue;
        }
        let mut stack = vec![sym];
        if reaches_itself(ctx, sym, sym, &mut stack, &mut HashSet::new()) {
            let finding = Finding::new(
                "E157",
                format!("`{}` is defined in terms of itself", symbol.name),
                ctx.loc(symbol.name_range),
            );
            ctx.report(finding);
        }
    }
}

fn reaches_itself(
    ctx: &Ctx<'_, '_, '_>,
    goal: SymRef,
    at: SymRef,
    stack: &mut Vec<SymRef>,
    seen: &mut HashSet<SymRef>,
) -> bool {
    let Some(symbol) = ctx.sem.symbol(at) else {
        return false;
    };
    let Some(value) = &symbol.value_node else {
        return false;
    };
    let scope = dummies_names(symbol);
    for token in flatten(value) {
        if !token.kind.is_name() || scope.contains(&token.text) {
            continue;
        }
        if token.kind == SyntaxKind::UpperName && token.text.chars().any(|c| c.is_lowercase()) {
            // fallthrough: object set names look like type names
        }
        let Some(lookup) = ctx.graph_lookup(at.module, &token.text) else {
            continue;
        };
        if !matches!(ctx.sem.kind(lookup), Kind::Object | Kind::ObjectSet) {
            continue;
        }
        if lookup == goal {
            return true;
        }
        if ctx.sem.symbol(lookup).is_some_and(Symbol::is_parameterized) {
            continue;
        }
        if seen.insert(lookup) {
            stack.push(lookup);
            if reaches_itself(ctx, goal, lookup, stack, seen) {
                return true;
            }
            stack.pop();
        }
    }
    false
}

fn dummies_names(symbol: &Symbol) -> Vec<String> {
    symbol.parameters.clone()
}

impl Ctx<'_, '_, '_> {
    fn graph_lookup(&self, module: crate::graph::ModuleId, name: &str) -> Option<SymRef> {
        match self.graph.lookup(module, name)?.found {
            crate::graph::Found::Symbol { module, index } => Some(SymRef { module, index }),
            _ => None,
        }
    }
}

/// Whether a bare name that sets a value field is an enumeration item, named number or named bit of the field's type
/// (X.680 19.12, 20.11, 20.13).
fn is_identifier_of_governor(
    ctx: &Ctx<'_, '_, '_>,
    symbol: &Symbol,
    setting: &crate::sem::Setting,
    name: &str,
) -> bool {
    if setting.kind != FieldKind::FixedValue {
        return !matches!(setting.kind, FieldKind::Object(_));
    }
    let Some(governor) = governor_of_setting(ctx, symbol, setting) else {
        return true;
    };
    let Some(peeled) = ctx.graph.peel_type_through_tags(governor.0, &governor.1) else {
        return true;
    };
    match peeled.node.kind() {
        SyntaxKind::EnumeratedType | SyntaxKind::IntegerType | SyntaxKind::BitStringType => peeled
            .node
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == SyntaxKind::LowerName && t.text() == name),
        SyntaxKind::DefinedType | SyntaxKind::AnyType => true,
        _ => false,
    }
}

/// The governing type of the field a setting sets, and the module it is written in.
fn governor_of_setting(
    ctx: &Ctx<'_, '_, '_>,
    symbol: &Symbol,
    setting: &crate::sem::Setting,
) -> Option<(crate::graph::ModuleId, SyntaxNode)> {
    let sym = SymRef {
        module: ctx.module,
        index: ctx
            .loaded
            .model
            .symbols
            .iter()
            .position(|s| s.name_range == symbol.name_range)?,
    };
    let ClassRes::Class(class) = ctx.sem.governor_class(sym) else {
        return None;
    };
    let info = ctx.sem.class_info(class)?;
    let field = info.field(&setting.field)?;
    Some((class.module, field.governor.clone()?))
}
