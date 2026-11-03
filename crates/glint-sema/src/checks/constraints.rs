//! Subtype constraint checks: E132, E133, E135 (X.680 49.6, 50.7-50.11, 51.1 Tables 11-12).

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use super::Ctx;
use super::values::{check_value, elems_of};
use crate::finding::Finding;
use crate::graph::{Found, ModuleId};
use crate::model::{Symbol, SymbolKind};
use crate::view::{BaseKind, base_kind_of_node, child, tokens, trimmed_range, type_child};

type Interval = (i128, i128);

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    let symbols = ctx.loaded.model.symbols.clone();
    for symbol in &symbols {
        if matches!(symbol.kind, SymbolKind::Macro | SymbolKind::Unknown) {
            continue;
        }
        for node in symbol.node.descendants() {
            match node.kind() {
                SyntaxKind::ConstrainedType => {
                    if let (Some(inner), Some(constraint)) =
                        (type_child(&node), child(&node, SyntaxKind::Constraint))
                    {
                        check_constraint(ctx, symbol, &inner, &constraint);
                    }
                }
                SyntaxKind::SequenceOfType | SyntaxKind::SetOfType => {
                    if let Some(constraint) = child(&node, SyntaxKind::Constraint) {
                        check_size_constraint_only(ctx, &constraint);
                    }
                    if let Some(size) = child(&node, SyntaxKind::SizeElement) {
                        check_size(ctx, &size);
                    }
                }
                _ => {}
            }
        }
    }
}

fn check_size_constraint_only(ctx: &mut Ctx<'_, '_, '_>, constraint: &SyntaxNode) {
    for element in leaves(constraint) {
        if element.kind() == SyntaxKind::SizeElement {
            check_size(ctx, &element);
        }
    }
}

fn check_constraint(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    parent: &SyntaxNode,
    constraint: &SyntaxNode,
) {
    check_exception(ctx, symbol, constraint);
    let general = constraint.children().any(|c| {
        matches!(
            c.kind(),
            SyntaxKind::UserDefinedConstraint
                | SyntaxKind::ContentsConstraint
                | SyntaxKind::TableConstraint
        )
    });
    if general {
        return;
    }
    let Some(peeled) = ctx.graph.peel_type_through_tags(ctx.module, parent) else {
        return;
    };
    let kind = base_kind_of_node(&peeled.node);
    let parent_root = root_intervals(ctx, ctx.module, parent, 0);
    for element in leaves(constraint) {
        match element.kind() {
            SyntaxKind::SizeElement => {
                if let Some(kind) = kind
                    && !matches!(
                        kind,
                        BaseKind::BitString
                            | BaseKind::OctetString
                            | BaseKind::CharString
                            | BaseKind::SequenceOf
                            | BaseKind::SetOf
                    )
                {
                    not_applicable(ctx, &element, "SIZE", kind);
                }
                check_size(ctx, &element);
            }
            SyntaxKind::RangeElement => {
                if let Some(kind) = kind
                    && !matches!(kind, BaseKind::Integer | BaseKind::Real)
                {
                    not_applicable(ctx, &element, "a value range", kind);
                    continue;
                }
                check_range(ctx, symbol, parent, &element, kind, parent_root.as_deref());
            }
            SyntaxKind::ValueElement => {
                if kind == Some(BaseKind::Integer)
                    && let Some(value) = child(&element, SyntaxKind::Value)
                {
                    check_value(ctx, symbol, parent, &elems_of(&value), false);
                    if let (Some(root), Some(n)) = (
                        parent_root.as_deref(),
                        ctx.eval.eval_integer(ctx.module, &value),
                    ) {
                        outside_parent(ctx, root, n, trimmed_range(&value));
                    }
                }
            }
            SyntaxKind::FromElement | SyntaxKind::PatternElement => {
                if let Some(kind) = kind
                    && !matches!(kind, BaseKind::CharString)
                {
                    let what = if element.kind() == SyntaxKind::FromElement {
                        "a permitted alphabet (FROM)"
                    } else {
                        "a PATTERN"
                    };
                    not_applicable(ctx, &element, what, kind);
                }
            }
            SyntaxKind::InnerTypeElement => {
                if let Some(kind) = kind
                    && !matches!(
                        kind,
                        BaseKind::Choice
                            | BaseKind::Sequence
                            | BaseKind::Set
                            | BaseKind::SequenceOf
                            | BaseKind::SetOf
                            | BaseKind::Real
                            | BaseKind::Other
                    )
                {
                    not_applicable(
                        ctx,
                        &element,
                        "an inner type constraint (WITH COMPONENT)",
                        kind,
                    );
                }
            }
            _ => {}
        }
    }
}

// X.680 51.1 Tables 11-12
fn not_applicable(ctx: &mut Ctx<'_, '_, '_>, element: &SyntaxNode, what: &str, kind: BaseKind) {
    let finding = Finding::new(
        "E133",
        format!("{what} cannot constrain {}", kind_name(kind)),
        ctx.loc(trimmed_range(element)),
    );
    ctx.report(finding);
}

fn kind_name(kind: BaseKind) -> &'static str {
    match kind {
        BaseKind::Boolean => "a BOOLEAN",
        BaseKind::Integer => "an INTEGER",
        BaseKind::BitString => "a BIT STRING",
        BaseKind::OctetString => "an OCTET STRING",
        BaseKind::Null => "NULL",
        BaseKind::ObjectIdentifier => "an OBJECT IDENTIFIER",
        BaseKind::RelativeOid => "a RELATIVE-OID",
        BaseKind::Real => "a REAL",
        BaseKind::Enumerated => "an ENUMERATED type",
        BaseKind::Sequence => "a SEQUENCE",
        BaseKind::Set => "a SET",
        BaseKind::SequenceOf => "a SEQUENCE OF",
        BaseKind::SetOf => "a SET OF",
        BaseKind::Choice => "a CHOICE",
        BaseKind::CharString => "a character string",
        BaseKind::Time => "UTCTime or GeneralizedTime",
        BaseKind::Other => "this type",
    }
}

// X.680 49.6: an exception specification needs an extensible constraint
fn check_exception(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, constraint: &SyntaxNode) {
    let Some(exception) = child(constraint, SyntaxKind::ExceptionSpec) else {
        return;
    };
    let extensible = tokens(constraint)
        .iter()
        .any(|t| t.kind() == SyntaxKind::Ellipsis);
    let user_defined = child(constraint, SyntaxKind::UserDefinedConstraint).is_some();
    let uses_dummy = tokens_deep(constraint)
        .iter()
        .any(|name| symbol.parameters.contains(name));
    if extensible || user_defined || uses_dummy {
        return;
    }
    let finding = Finding::new(
        "E135",
        "an exception specification needs an extension marker in the constraint",
        ctx.loc(trimmed_range(&exception)),
    );
    ctx.report(finding);
}

fn tokens_deep(node: &SyntaxNode) -> Vec<String> {
    node.descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind().is_name())
        .map(|t| t.text().to_owned())
        .collect()
}

/// The leaf elements of an element set, not entering SIZE, FROM or inner-type constraints.
fn leaves(constraint: &SyntaxNode) -> Vec<SyntaxNode> {
    let mut out = Vec::new();
    collect_leaves(constraint, &mut out);
    out
}

fn collect_leaves(node: &SyntaxNode, out: &mut Vec<SyntaxNode>) {
    for c in node.children() {
        match c.kind() {
            SyntaxKind::UnionSet
            | SyntaxKind::IntersectionSet
            | SyntaxKind::ExclusionSet
            | SyntaxKind::ParenElementSet => collect_leaves(&c, out),
            SyntaxKind::ValueElement
            | SyntaxKind::RangeElement
            | SyntaxKind::SizeElement
            | SyntaxKind::FromElement
            | SyntaxKind::PatternElement
            | SyntaxKind::InnerTypeElement
            | SyntaxKind::ContainedSubtype
            | SyntaxKind::TypeElement
            | SyntaxKind::SettingsElement => out.push(c),
            _ => {}
        }
    }
}

/// One end of a range: a number, or `MIN`/`MAX`, with whether the end is excluded.
struct Bound {
    value: Option<i128>,
    unbounded: bool,
    exclusive: bool,
    range: TextRange,
}

fn read_bounds(ctx: &Ctx<'_, '_, '_>, element: &SyntaxNode) -> Option<(Bound, Bound)> {
    let values: Vec<SyntaxNode> = element
        .children()
        .filter(|c| c.kind() == SyntaxKind::Value)
        .collect();
    let (lo, hi) = (values.first()?, values.last()?);
    let toks = tokens(element);
    let dotdot = toks.iter().position(|t| t.kind() == SyntaxKind::DotDot)?;
    let lo_exclusive = toks.iter().take(dotdot).any(|t| t.kind() == SyntaxKind::Lt);
    let hi_exclusive = toks.iter().skip(dotdot).any(|t| t.kind() == SyntaxKind::Lt);
    let read = |value: &SyntaxNode, exclusive: bool| {
        let is_keyword = tokens(value)
            .iter()
            .any(|t| t.kind() == SyntaxKind::Keyword && matches!(t.text(), "MIN" | "MAX"));
        Bound {
            value: if is_keyword {
                None
            } else {
                ctx.eval.eval_integer(ctx.module, value)
            },
            unbounded: is_keyword,
            exclusive,
            range: trimmed_range(value),
        }
    };
    Some((read(lo, lo_exclusive), read(hi, hi_exclusive)))
}

// X.680 51.7, 50.10, 50.11
fn check_range(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    parent: &SyntaxNode,
    element: &SyntaxNode,
    kind: Option<BaseKind>,
    parent_root: Option<&[Interval]>,
) {
    if kind == Some(BaseKind::Integer) {
        for value in element.children().filter(|c| c.kind() == SyntaxKind::Value) {
            check_value(ctx, symbol, parent, &elems_of(&value), true);
        }
    }
    let Some((lo, hi)) = read_bounds(ctx, element) else {
        return;
    };
    if let (Some(a), Some(b)) = (lo.value, hi.value) {
        let first = if lo.exclusive { a.saturating_add(1) } else { a };
        let last = if hi.exclusive { b.saturating_sub(1) } else { b };
        if first > last {
            let finding = Finding::new(
                "E132",
                if a > b {
                    format!("the range {a}..{b} is inverted")
                } else {
                    format!("the range {a}..{b} is empty")
                },
                ctx.loc(trimmed_range(element)),
            );
            ctx.report(finding);
            return;
        }
    }
    if let Some(root) = parent_root {
        for bound in [&lo, &hi] {
            if let (Some(n), false) = (bound.value, bound.unbounded) {
                outside_parent(ctx, root, n, bound.range);
            }
        }
    }
}

// X.680 50.10, 50.11
fn outside_parent(ctx: &mut Ctx<'_, '_, '_>, root: &[Interval], value: i128, range: TextRange) {
    if root.iter().any(|&(lo, hi)| lo <= value && value <= hi) {
        return;
    }
    let finding = Finding::new(
        "E132",
        format!("{value} is not a value of the constrained type's extension root"),
        ctx.loc(range),
    );
    ctx.report(finding);
}

// X.680 51.5
fn check_size(ctx: &mut Ctx<'_, '_, '_>, size: &SyntaxNode) {
    let Some(inner) = child(size, SyntaxKind::Constraint) else {
        return;
    };
    for element in leaves(&inner) {
        match element.kind() {
            SyntaxKind::RangeElement => {
                if let Some((lo, hi)) = read_bounds(ctx, &element) {
                    if let Some(a) = lo.value
                        && a < 0
                    {
                        let finding = Finding::new(
                            "E132",
                            "a SIZE bound cannot be negative",
                            ctx.loc(lo.range),
                        );
                        ctx.report(finding);
                    }
                    if let (Some(a), Some(b)) = (lo.value, hi.value)
                        && a > b
                    {
                        let finding = Finding::new(
                            "E132",
                            format!("the SIZE range {a}..{b} is inverted"),
                            ctx.loc(trimmed_range(&element)),
                        );
                        ctx.report(finding);
                    }
                }
            }
            SyntaxKind::ValueElement => {
                if let Some(value) = child(&element, SyntaxKind::Value)
                    && let Some(n) = ctx.eval.eval_integer(ctx.module, &value)
                    && n < 0
                {
                    let finding = Finding::new(
                        "E132",
                        "a SIZE bound cannot be negative",
                        ctx.loc(trimmed_range(&value)),
                    );
                    ctx.report(finding);
                }
            }
            _ => {}
        }
    }
}

// ---- value sets of INTEGER types ---------------------------------------------------------------

fn root_intervals(
    ctx: &Ctx<'_, '_, '_>,
    module: ModuleId,
    ty: &SyntaxNode,
    depth: usize,
) -> Option<Vec<Interval>> {
    if depth > 16 {
        return None;
    }
    match ty.kind() {
        SyntaxKind::IntegerType => Some(vec![(i128::MIN, i128::MAX)]),
        SyntaxKind::TaggedType | SyntaxKind::EncodingPrefixedType => {
            root_intervals(ctx, module, &type_child(ty)?, depth.saturating_add(1))
        }
        SyntaxKind::ConstrainedType => {
            let inner = root_intervals(ctx, module, &type_child(ty)?, depth.saturating_add(1))?;
            let own = constraint_intervals(ctx, module, &child(ty, SyntaxKind::Constraint)?)?;
            Some(intersect(&inner, &own))
        }
        SyntaxKind::DefinedType => {
            let reference = crate::view::defined_type_ref(ty)?;
            let lookup = match &reference.module {
                Some(q) => ctx
                    .graph
                    .lookup_qualified(module, q, &reference.name)
                    .ok()?,
                None => ctx.graph.lookup(module, &reference.name)?,
            };
            let Found::Symbol {
                module: target,
                index,
            } = lookup.found
            else {
                return None;
            };
            let symbol = ctx.graph.symbol(target, index)?;
            if symbol.kind != SymbolKind::Type || symbol.is_parameterized() {
                return None;
            }
            root_intervals(
                ctx,
                target,
                symbol.type_node.as_ref()?,
                depth.saturating_add(1),
            )
        }
        _ => None,
    }
}

fn constraint_intervals(
    ctx: &Ctx<'_, '_, '_>,
    module: ModuleId,
    constraint: &SyntaxNode,
) -> Option<Vec<Interval>> {
    let first = constraint
        .children()
        .find(|c| !matches!(c.kind(), SyntaxKind::ExceptionSpec))?;
    if matches!(
        first.kind(),
        SyntaxKind::UserDefinedConstraint
            | SyntaxKind::ContentsConstraint
            | SyntaxKind::TableConstraint
    ) {
        return None;
    }
    set_intervals(ctx, module, &first)
}

fn set_intervals(
    ctx: &Ctx<'_, '_, '_>,
    module: ModuleId,
    node: &SyntaxNode,
) -> Option<Vec<Interval>> {
    match node.kind() {
        SyntaxKind::ValueElement => {
            let n = ctx
                .eval
                .eval_integer(module, &child(node, SyntaxKind::Value)?)?;
            Some(vec![(n, n)])
        }
        SyntaxKind::RangeElement => {
            let (lo, hi) = read_bounds_in(ctx, module, node)?;
            Some(vec![(lo, hi)])
        }
        SyntaxKind::UnionSet => {
            let mut all = Vec::new();
            for part in node.children() {
                all.extend(set_intervals(ctx, module, &part)?);
            }
            Some(all)
        }
        SyntaxKind::IntersectionSet => {
            let mut parts = node.children();
            let mut acc = set_intervals(ctx, module, &parts.next()?)?;
            for part in parts {
                acc = intersect(&acc, &set_intervals(ctx, module, &part)?);
            }
            Some(acc)
        }
        SyntaxKind::ParenElementSet => set_intervals(ctx, module, &node.children().next()?),
        _ => None,
    }
}

fn read_bounds_in(
    ctx: &Ctx<'_, '_, '_>,
    module: ModuleId,
    node: &SyntaxNode,
) -> Option<(i128, i128)> {
    let values: Vec<SyntaxNode> = node
        .children()
        .filter(|c| c.kind() == SyntaxKind::Value)
        .collect();
    let (lo, hi) = (values.first()?, values.last()?);
    let toks = tokens(node);
    let dotdot = toks.iter().position(|t| t.kind() == SyntaxKind::DotDot)?;
    let lo_excl = toks.iter().take(dotdot).any(|t| t.kind() == SyntaxKind::Lt);
    let hi_excl = toks.iter().skip(dotdot).any(|t| t.kind() == SyntaxKind::Lt);
    let end = |value: &SyntaxNode, low: bool| -> Option<i128> {
        let keyword = tokens(value)
            .into_iter()
            .find(|t| t.kind() == SyntaxKind::Keyword);
        match keyword.as_ref().map(|t| t.text()) {
            Some("MIN") => Some(i128::MIN),
            Some("MAX") => Some(i128::MAX),
            Some(_) => None,
            None => {
                let n = ctx.eval.eval_integer(module, value)?;
                let _ = low;
                Some(n)
            }
        }
    };
    let a = end(lo, true)?;
    let b = end(hi, false)?;
    let a = if lo_excl { a.saturating_add(1) } else { a };
    let b = if hi_excl { b.saturating_sub(1) } else { b };
    Some((a, b))
}

fn intersect(a: &[Interval], b: &[Interval]) -> Vec<Interval> {
    let mut out = Vec::new();
    for &(a_lo, a_hi) in a {
        for &(b_lo, b_hi) in b {
            let lo = a_lo.max(b_lo);
            let hi = a_hi.min(b_hi);
            if lo <= hi {
                out.push((lo, hi));
            }
        }
    }
    out
}
