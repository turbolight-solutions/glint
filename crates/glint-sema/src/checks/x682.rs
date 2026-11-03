//! X.682 checks: E151, E180-E185 (table and component relation constraints, contents constraints, user-defined
//! constraints).

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use super::Ctx;
use super::components::expand;
use super::x681::{PathResult, field_names, resolve_path_in};
use crate::finding::Finding;
use crate::graph::ModuleId;
use crate::model::{Symbol, SymbolKind};
use crate::sem::toks::{Tk, flatten, inner, split_commas};
use crate::sem::{
    ClassRes, Dummy, FieldKind, Kind, Operand, Resolved, SymRef, parse_object_set_syntax, range_of,
};
use crate::view::{ComponentList, child, tokens, type_child};

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    let symbols = ctx.loaded.model.symbols.clone();
    for symbol in &symbols {
        if matches!(symbol.kind, SymbolKind::Macro | SymbolKind::Unknown) {
            continue;
        }
        for node in symbol.node.descendants() {
            if node.kind() != SyntaxKind::ConstrainedType {
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
            check_constrained(ctx, symbol, &node);
        }
    }
}

fn check_constrained(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, node: &SyntaxNode) {
    let Some(base) = type_child(node) else { return };
    let Some(constraint) = child(node, SyntaxKind::Constraint) else {
        return;
    };
    for spec in constraint.children() {
        match spec.kind() {
            SyntaxKind::TableConstraint => check_table(ctx, symbol, node, &base, &spec),
            SyntaxKind::ContentsConstraint => check_contents(ctx, node, &base, &spec),
            SyntaxKind::UserDefinedConstraint => check_user_defined(ctx, symbol, &spec),
            _ => {}
        }
    }
    check_instance_of(ctx, &base, &constraint);
}

/// What the type under a constraint is, when it can be told.
enum Base {
    FieldType {
        class: SymRef,
        field: crate::sem::Field,
    },
    InstanceOf,
    Other,
    Unknown,
}

fn base_kind(ctx: &Ctx<'_, '_, '_>, node: &SyntaxNode) -> Base {
    let Some(peeled) = ctx.graph.peel_type(ctx.module, node) else {
        return Base::Unknown;
    };
    match peeled.node.kind() {
        SyntaxKind::ObjectClassFieldType => {
            let Some(names_node) = child(&peeled.node, SyntaxKind::FieldName) else {
                return Base::Unknown;
            };
            let mut names = tokens(&peeled.node)
                .into_iter()
                .filter(|t| t.kind() == SyntaxKind::UpperName);
            let (qualifier, name) = match (names.next(), names.next()) {
                (Some(module), Some(name)) => (Some(module.text().to_owned()), name),
                (Some(name), None) => (None, name),
                _ => return Base::Unknown,
            };
            let scope = ctx.sem.scope_of(&peeled.node);
            let Resolved::Sym(target) =
                ctx.sem
                    .resolve(peeled.module, &scope, qualifier.as_deref(), name.text())
            else {
                return Base::Unknown;
            };
            let ClassRes::Class(class) = ctx.sem.class_res_of_symbol(target) else {
                return Base::Unknown;
            };
            match resolve_path_in(ctx, class, &field_names(&names_node)) {
                PathResult::Found { field } => Base::FieldType { class, field },
                _ => Base::Unknown,
            }
        }
        SyntaxKind::InstanceOfType => Base::InstanceOf,
        SyntaxKind::DefinedType | SyntaxKind::AnyType => Base::Unknown,
        _ => Base::Other,
    }
}

// X.682 10.1, 10.3-10.4, 10.7-10.15
fn check_table(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    constrained: &SyntaxNode,
    base_node: &SyntaxNode,
    table: &SyntaxNode,
) {
    let groups: Vec<SyntaxNode> = table
        .children()
        .filter(|c| c.kind() == SyntaxKind::BraceGroup)
        .collect();
    let Some(set) = groups.first() else { return };
    let has_relation = groups.len() > 1;
    let base = base_kind(ctx, base_node);
    match base {
        Base::Other => {
            // X.681 14.3: `({...})` on another type is read as a single-value constraint
            if has_relation {
                let finding = Finding::new(
                    "E180",
                    "a component relation constraint can only follow an object class field type or INSTANCE OF",
                    ctx.loc(range_of(table)),
                );
                ctx.report(finding);
            }
            return;
        }
        Base::Unknown => return,
        Base::InstanceOf | Base::FieldType { .. } => {}
    }
    let class = match &base {
        Base::FieldType { class, .. } => Some(*class),
        _ => None,
    };
    check_constraining_set(ctx, symbol, class, set);
    if has_relation
        && let (Some(relation), Base::FieldType { class, field }) = (groups.get(1), &base)
    {
        check_relation(ctx, constrained, set, relation, *class, field);
    }
}

// X.682 10.4, 10.12; X.681 12.3, 15.12
fn check_constraining_set(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    class: Option<SymRef>,
    group: &SyntaxNode,
) {
    let Some(class) = class else { return };
    let Some(info) = ctx.sem.class_info(class) else {
        return;
    };
    let syntax = parse_object_set_syntax(group);
    if let [
        Operand::Named {
            qualifier,
            name,
            name_range,
            path,
            ..
        },
    ] = syntax.operands.as_slice()
        && path.is_empty()
        && !syntax.extensible
    {
        let scope: Vec<Dummy> = ctx.sem.scope_of(&symbol.node);
        match ctx
            .sem
            .resolve(ctx.module, &scope, qualifier.as_deref(), name)
        {
            Resolved::Sym(target) => match ctx.sem.kind(target) {
                Kind::ObjectSet => {
                    if let ClassRes::Class(found) = ctx.sem.governor_class(target)
                        && found != class
                    {
                        let finding = Finding::new(
                            "E181",
                            format!(
                                "`{name}` is an object set of class `{}`, but the constrained field type is of class `{}`",
                                symbol_name(ctx, found),
                                symbol_name(ctx, class)
                            ),
                            ctx.loc(*name_range),
                        );
                        ctx.report(finding);
                    }
                }
                Kind::Unknown => {}
                _ => {
                    let finding = Finding::new(
                        "E181",
                        format!("`{name}` is not an information object set"),
                        ctx.loc(*name_range),
                    );
                    ctx.report(finding);
                }
            },
            Resolved::Missing | Resolved::NotImported(_) => {
                super::references::check_reference(
                    ctx,
                    symbol,
                    qualifier.as_deref(),
                    name,
                    *name_range,
                    "object set",
                );
            }
            Resolved::Dummy(_) | Resolved::Opaque => {}
        }
        return;
    }
    super::x681::check_object_set_body(ctx, symbol, &info, group, true);
}

fn symbol_name(ctx: &Ctx<'_, '_, '_>, sym: SymRef) -> String {
    ctx.sem
        .symbol(sym)
        .map_or_else(String::new, |s| s.name.clone())
}

/// One `@...a.b` of a component relation constraint (X.682 10.7).
struct At {
    dots: usize,
    ids: Vec<(String, TextRange)>,
    range: TextRange,
}

fn read_ats(group: &SyntaxNode) -> Vec<At> {
    let tokens = inner(group);
    let mut out = Vec::new();
    for entry in split_commas(&tokens) {
        let mut iter = entry.iter().peekable();
        let Some(at) = iter.next() else { continue };
        if at.kind != SyntaxKind::At {
            continue;
        }
        let mut dots = 0usize;
        while let Some(token) = iter.peek() {
            let count = match token.kind {
                SyntaxKind::Dot => 1usize,
                SyntaxKind::DotDot => 2,
                SyntaxKind::Ellipsis => 3,
                _ => break,
            };
            dots = dots.saturating_add(count);
            iter.next();
        }
        let mut ids = Vec::new();
        let mut last = at.range;
        while let Some(token) = iter.next() {
            if token.kind == SyntaxKind::LowerName {
                ids.push((token.text.clone(), token.range));
                last = token.range;
                if iter.peek().is_some_and(|t| t.kind == SyntaxKind::Dot) {
                    iter.next();
                    continue;
                }
            }
            break;
        }
        out.push(At {
            dots,
            ids,
            range: TextRange::new(at.range.start(), last.end()),
        });
    }
    out
}

const CONSTRUCTIONS: &[SyntaxKind] = &[
    SyntaxKind::SequenceType,
    SyntaxKind::SetType,
    SyntaxKind::SequenceOfType,
    SyntaxKind::SetOfType,
    SyntaxKind::ChoiceType,
];

fn is_record(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::SequenceType | SyntaxKind::SetType | SyntaxKind::ChoiceType
    )
}

// X.682 10.7-10.15
fn check_relation(
    ctx: &mut Ctx<'_, '_, '_>,
    constrained: &SyntaxNode,
    set: &SyntaxNode,
    relation: &SyntaxNode,
    class: SymRef,
    field: &crate::sem::Field,
) {
    let _ = field;
    let enclosing: Vec<SyntaxNode> = constrained
        .ancestors()
        .skip(1)
        .take_while(|a| {
            !matches!(
                a.kind(),
                SyntaxKind::TypeAssignment
                    | SyntaxKind::ValueAssignment
                    | SyntaxKind::ValueSetAssignment
            )
        })
        .filter(|a| CONSTRUCTIONS.contains(&a.kind()))
        .collect();
    let set_text = norm(set);
    for at in read_ats(relation) {
        let Some(start) = parent_structure(&enclosing, at.dots) else {
            let in_record = enclosing
                .iter()
                .any(|a| matches!(a.kind(), SyntaxKind::SequenceType | SyntaxKind::SetType));
            let message = if in_record {
                "there are not that many enclosing constructions for this many dots"
            } else {
                "`@` is only allowed inside a SET or SEQUENCE"
            };
            let finding = Finding::new("E151", message, ctx.loc(at.range));
            ctx.report(finding);
            continue;
        };
        let Some(target) = follow_ids(ctx, &start, &at) else {
            continue;
        };
        let Some(target) = target else { continue };
        check_referenced(ctx, constrained, &at, &target, class, &set_text);
    }
}

fn norm(node: &SyntaxNode) -> String {
    flatten(node)
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

// X.682 10.10
fn parent_structure(enclosing: &[SyntaxNode], dots: usize) -> Option<SyntaxNode> {
    if dots == 0 {
        return enclosing
            .iter()
            .rev()
            .find(|a| is_record(a.kind()))
            .cloned();
    }
    let innermost = enclosing
        .iter()
        .position(|a| matches!(a.kind(), SyntaxKind::SequenceType | SyntaxKind::SetType))?;
    enclosing
        .get(innermost.saturating_add(dots.saturating_sub(1)))
        .cloned()
}

/// A resolved referenced component.
struct Target {
    module: ModuleId,
    component: crate::view::Component,
    /// The alternative or component of the nearest enclosing structure on the path, for E183.
    node: SyntaxNode,
}

/// Walks the identifiers of an `AtNotation` (X.682 10.8-10.9). `Some(None)` means "unknown, say nothing".
fn follow_ids(ctx: &mut Ctx<'_, '_, '_>, start: &SyntaxNode, at: &At) -> Option<Option<Target>> {
    let mut module = ctx.module;
    let mut parent = start.clone();
    let mut found: Option<Target> = None;
    for (position, (id, range)) in at.ids.iter().enumerate() {
        if !is_record(parent.kind()) {
            let finding = Finding::new(
                "E151",
                format!(
                    "`{}` is not a SET, SEQUENCE or CHOICE, so `{id}` cannot be selected from it",
                    at.ids
                        .get(position.saturating_sub(1))
                        .map_or("", |p| p.0.as_str())
                ),
                ctx.loc(*range),
            );
            ctx.report(finding);
            return None;
        }
        let list = ComponentList::read(&parent);
        let flat = expand(ctx.graph, module, &list, 0);
        let Some(entry) = flat.iter().find(|f| f.component.name == *id) else {
            let finding = Finding::new(
                "E151",
                format!("no component or alternative `{id}` in the structure this `@` points into"),
                ctx.loc(*range),
            );
            ctx.report(finding);
            return None;
        };
        let component = entry.component.clone();
        let node = component
            .ty
            .parent()
            .unwrap_or_else(|| component.ty.clone());
        found = Some(Target {
            module: entry.module,
            component: component.clone(),
            node,
        });
        if position.saturating_add(1) < at.ids.len() {
            let Some(peeled) = ctx
                .graph
                .peel_type_through_tags(entry.module, &component.ty)
            else {
                return Some(None);
            };
            module = peeled.module;
            parent = peeled.node;
        }
    }
    Some(found)
}

// X.682 10.14, 10.15
fn check_referenced(
    ctx: &mut Ctx<'_, '_, '_>,
    constrained: &SyntaxNode,
    at: &At,
    target: &Target,
    class: SymRef,
    set_text: &str,
) {
    let referencing_component = constrained
        .ancestors()
        .find(|a| a.kind() == SyntaxKind::NamedType);
    // E183
    if let Some(referencing) = &referencing_component {
        for choice in referencing
            .ancestors()
            .filter(|a| a.kind() == SyntaxKind::ChoiceType)
        {
            let within = |node: &SyntaxNode| {
                node.ancestors()
                    .find(|a| a.parent().as_ref() == Some(&choice))
            };
            if choice.text_range().contains_range(target.node.text_range())
                && target.module == ctx.module
                && let (Some(a), Some(b)) = (within(referencing), within(&target.node))
                && a != b
            {
                let finding = Finding::new(
                    "E183",
                    format!(
                        "`{}` is in a different alternative of the CHOICE than the component that refers to it",
                        at.ids.last().map_or("", |i| i.0.as_str())
                    ),
                    ctx.loc(at.range),
                );
                ctx.report(finding);
                return;
            }
        }
    }
    // E182
    let ty = &target.component.ty;
    let mut constraints: Vec<SyntaxNode> = Vec::new();
    let mut inner_node = ty.clone();
    while matches!(
        inner_node.kind(),
        SyntaxKind::ConstrainedType | SyntaxKind::TaggedType | SyntaxKind::EncodingPrefixedType
    ) {
        if let Some(c) = child(&inner_node, SyntaxKind::Constraint) {
            constraints.push(c);
        }
        match type_child(&inner_node) {
            Some(next) => inner_node = next,
            None => break,
        }
    }
    if inner_node.kind() == SyntaxKind::DefinedType || inner_node.kind() == SyntaxKind::AnyType {
        return;
    }
    if inner_node.kind() != SyntaxKind::ObjectClassFieldType {
        let finding = Finding::new(
            "E182",
            format!(
                "`{}` is not an object class field type, so it cannot be referenced by a component relation constraint",
                at.ids.last().map_or("", |i| i.0.as_str())
            ),
            ctx.loc(at.range),
        );
        ctx.report(finding);
        return;
    }
    let other = ObjectClassFieldTypeView::of(ctx, target.module, &inner_node);
    if let Some(other_class) = other.class
        && other_class != class
    {
        let finding = Finding::new(
            "E182",
            format!(
                "`{}` is a field of class `{}`, not of class `{}`",
                at.ids.last().map_or("", |i| i.0.as_str()),
                symbol_name(ctx, other_class),
                symbol_name(ctx, class)
            ),
            ctx.loc(at.range),
        );
        ctx.report(finding);
        return;
    }
    if let Some(kind) = other.kind
        && !kind.is_value_like()
    {
        let finding = Finding::new(
            "E182",
            format!(
                "`{}` is a {}, not a value or value set field",
                at.ids.last().map_or("", |i| i.0.as_str()),
                kind.describe()
            ),
            ctx.loc(at.range),
        );
        ctx.report(finding);
        return;
    }
    let same_set = constraints.iter().any(|c| {
        c.children().any(|spec| {
            spec.kind() == SyntaxKind::TableConstraint
                && spec
                    .children()
                    .find(|g| g.kind() == SyntaxKind::BraceGroup)
                    .is_some_and(|g| norm(&g) == set_text)
        })
    });
    if target.module == ctx.module && !same_set {
        let finding = Finding::new(
            "E182",
            format!(
                "`{}` is not constrained by the same object set",
                at.ids.last().map_or("", |i| i.0.as_str())
            ),
            ctx.loc(at.range),
        );
        ctx.report(finding);
    }
}

struct ObjectClassFieldTypeView {
    class: Option<SymRef>,
    kind: Option<FieldKind>,
}

impl ObjectClassFieldTypeView {
    fn of(ctx: &Ctx<'_, '_, '_>, module: ModuleId, node: &SyntaxNode) -> Self {
        let none = Self {
            class: None,
            kind: None,
        };
        let Some(names_node) = child(node, SyntaxKind::FieldName) else {
            return none;
        };
        let mut names = tokens(node)
            .into_iter()
            .filter(|t| t.kind() == SyntaxKind::UpperName);
        let (qualifier, name) = match (names.next(), names.next()) {
            (Some(q), Some(n)) => (Some(q.text().to_owned()), n),
            (Some(n), None) => (None, n),
            _ => return none,
        };
        let scope = ctx.sem.scope_of(node);
        let Resolved::Sym(target) =
            ctx.sem
                .resolve(module, &scope, qualifier.as_deref(), name.text())
        else {
            return none;
        };
        let ClassRes::Class(class) = ctx.sem.class_res_of_symbol(target) else {
            return none;
        };
        match resolve_path_in(ctx, class, &field_names(&names_node)) {
            PathResult::Found { field } => Self {
                class: Some(class),
                kind: Some(field.kind),
            },
            _ => Self {
                class: Some(class),
                kind: None,
            },
        }
    }
}

// X.682 A.2
fn check_instance_of(ctx: &mut Ctx<'_, '_, '_>, base: &SyntaxNode, constraint: &SyntaxNode) {
    if !matches!(base_kind(ctx, base), Base::InstanceOf) {
        return;
    }
    let simple_table = constraint.children().any(|c| {
        c.kind() == SyntaxKind::TableConstraint
            && c.children()
                .filter(|g| g.kind() == SyntaxKind::BraceGroup)
                .count()
                == 1
    });
    if !simple_table {
        let finding = Finding::new(
            "E180",
            "the only constraint that can follow INSTANCE OF is a simple table constraint",
            ctx.loc(range_of(constraint)),
        );
        ctx.report(finding);
    }
}

// X.682 11.1-11.3
fn check_contents(
    ctx: &mut Ctx<'_, '_, '_>,
    node: &SyntaxNode,
    base: &SyntaxNode,
    contents: &SyntaxNode,
) {
    let peeled = ctx.graph.peel_type(ctx.module, base);
    let known = peeled.as_ref().map(|p| &p.node);
    let applicable = known.is_none_or(|n| match n.kind() {
        SyntaxKind::BuiltinType => {
            let text = crate::view::builtin_text(n);
            text == "OCTET STRING"
        }
        SyntaxKind::BitStringType => child(n, SyntaxKind::NamedNumberList).is_none(),
        SyntaxKind::DefinedType | SyntaxKind::AnyType | SyntaxKind::ObjectClassFieldType => true,
        _ => false,
    });
    if !applicable {
        let finding = Finding::new(
            "E184",
            "a contents constraint applies only to OCTET STRING and to BIT STRING without named bits",
            ctx.loc(range_of(contents)),
        );
        ctx.report(finding);
    }
    let others = other_constraints(ctx, node);
    if others {
        let finding = Finding::new(
            "E184",
            "a type with a contents constraint cannot have other constraints, directly or through a type reference",
            ctx.loc(range_of(contents)),
        );
        ctx.report(finding);
    }
    // X.682 11.2
    if let Some(value) = child(contents, SyntaxKind::Value) {
        let toks = flatten(&value);
        let wrong = match toks.as_slice() {
            [only] => matches!(
                only.kind,
                SyntaxKind::Number | SyntaxKind::CString | SyntaxKind::QuotedString
            ),
            _ => false,
        };
        if wrong {
            let finding = Finding::new(
                "E184",
                "the value after ENCODED BY must be an OBJECT IDENTIFIER",
                ctx.loc(range_of(&value)),
            );
            ctx.report(finding);
        }
    }
}

/// Whether the constrained type carries constraints besides the contents constraint (X.682 11.3).
fn other_constraints(ctx: &Ctx<'_, '_, '_>, node: &SyntaxNode) -> bool {
    let mut total = 0usize;
    let mut contents = 0usize;
    let mut top = node.clone();
    while let Some(parent) = top.parent() {
        if parent.kind() == SyntaxKind::ConstrainedType {
            top = parent;
        } else {
            break;
        }
    }
    let mut at = Some(top);
    let mut module = ctx.module;
    let mut budget = 12usize;
    while let Some(current) = at.take() {
        budget = budget.saturating_sub(1);
        if budget == 0 {
            break;
        }
        match current.kind() {
            SyntaxKind::ConstrainedType => {
                for constraint in current
                    .children()
                    .filter(|c| c.kind() == SyntaxKind::Constraint)
                {
                    for spec in constraint.children() {
                        total = total.saturating_add(1);
                        if spec.kind() == SyntaxKind::ContentsConstraint {
                            contents = contents.saturating_add(1);
                        }
                    }
                    if constraint.children().next().is_none() {
                        total = total.saturating_add(1);
                    }
                }
                at = type_child(&current);
            }
            SyntaxKind::EncodingPrefixedType => at = type_child(&current),
            SyntaxKind::DefinedType => {
                let Some(Resolved::Sym(target)) = ctx.sem.resolve_defined(module, &current) else {
                    break;
                };
                let Some(symbol) = ctx.sem.symbol(target) else {
                    break;
                };
                if symbol.kind != SymbolKind::Type || symbol.is_parameterized() {
                    break;
                }
                module = target.module;
                at = symbol.type_node.clone();
            }
            _ => break,
        }
    }
    total > contents
}

// X.682 9.1-9.3
fn check_user_defined(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, spec: &SyntaxNode) {
    let Some(group) = child(spec, SyntaxKind::BraceGroup) else {
        return;
    };
    let toks = inner(&group);
    if toks.is_empty() {
        return;
    }
    let scope = ctx.sem.scope_of(&symbol.node);
    for entry in split_commas(&toks) {
        let Some(first) = entry.first() else { continue };
        let colon = entry.iter().position(|t| t.kind == SyntaxKind::Colon);
        match colon {
            Some(at) => {
                let governor = entry.get(..at).unwrap_or_default();
                check_governor(ctx, symbol, &scope, governor, first);
            }
            None => {
                if first.kind == SyntaxKind::LowerName && entry.len() == 1 {
                    let finding = Finding::new(
                        "E185",
                        format!(
                            "`{}` is a value or object and needs its governor, written `Governor : {}`",
                            first.text, first.text
                        ),
                        ctx.loc(first.range),
                    );
                    ctx.report(finding);
                } else if matches!(
                    first.kind,
                    SyntaxKind::Number | SyntaxKind::CString | SyntaxKind::QuotedString
                ) {
                    let finding = Finding::new(
                        "E185",
                        "a value parameter needs its governor, written `Governor : value`",
                        ctx.loc(first.range),
                    );
                    ctx.report(finding);
                }
            }
        }
    }
}

fn check_governor(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    scope: &[Dummy],
    governor: &[Tk],
    at: &Tk,
) {
    let [name] = governor else { return };
    if name.kind != SyntaxKind::UpperName {
        let finding = Finding::new(
            "E185",
            format!("`{}` cannot be a governor", name.text),
            ctx.loc(at.range),
        );
        ctx.report(finding);
        return;
    }
    if let Resolved::Sym(target) = ctx.sem.resolve(ctx.module, scope, None, &name.text)
        && matches!(
            ctx.sem.kind(target),
            Kind::Value | Kind::Object | Kind::ObjectSet | Kind::ValueSet
        )
    {
        let finding = Finding::new(
            "E185",
            format!(
                "`{}` is not a type or class, so it cannot be a governor",
                name.text
            ),
            ctx.loc(name.range),
        );
        ctx.report(finding);
    }
    let _ = symbol;
}
