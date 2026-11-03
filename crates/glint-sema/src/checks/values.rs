//! Value checks: E131 and the reference errors inside values (X.680 14.9, 16.x, 19.x, 25.12, Annex C).

use glint_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, TextRange};

use super::Ctx;
use super::components::expand;
use super::references::check_reference;
use crate::finding::Finding;
use crate::graph::Found;
use crate::model::{Symbol, SymbolKind};
use crate::view::{
    BaseKind, ComponentList, Region, base_kind_of_node, child, of_element_type, tokens,
    trimmed_range, type_child,
};

/// A token or child node of a value, in text order.
#[derive(Debug, Clone)]
pub(super) enum Elem {
    Token(SyntaxToken),
    Node(SyntaxNode),
}

impl Elem {
    fn range(&self) -> TextRange {
        match self {
            Self::Token(t) => t.text_range(),
            Self::Node(n) => trimmed_range(n),
        }
    }
}

pub(super) fn elems_of(node: &SyntaxNode) -> Vec<Elem> {
    node.children_with_tokens()
        .filter_map(|e| {
            if let Some(n) = e.as_node() {
                return Some(Elem::Node(n.clone()));
            }
            let t = e.into_token()?;
            (!t.kind().is_trivia()).then_some(Elem::Token(t))
        })
        .collect()
}

fn span(elems: &[Elem]) -> Option<TextRange> {
    let first = elems.first()?.range();
    let last = elems.last()?.range();
    Some(TextRange::new(first.start(), last.end()))
}

/// The syntactic shape of a value.
enum Shape {
    Number,
    Real,
    Quoted(char),
    CString,
    Brace(SyntaxNode),
    Reference(Option<String>, String),
    Keyword(String),
    Choice(String, Vec<Elem>),
    /// `obj.&field` (X.681 15.3) or `name{actuals}` (X.683 9.2): typed by another definition.
    Indirect,
    Other,
}

fn is_value_keyword(text: &str) -> bool {
    matches!(
        text,
        "TRUE"
            | "FALSE"
            | "NULL"
            | "MIN"
            | "MAX"
            | "PLUS-INFINITY"
            | "MINUS-INFINITY"
            | "NOT-A-NUMBER"
    )
}

fn shape(elems: &[Elem]) -> Shape {
    // unwrap a nested `Value` node
    if let [Elem::Node(n)] = elems
        && n.kind() == SyntaxKind::Value
    {
        return shape(&elems_of(n));
    }
    if elems.iter().any(|e| {
        matches!(e, Elem::Node(n) if matches!(n.kind(), SyntaxKind::FieldName | SyntaxKind::ActualParameterList))
    }) {
        return Shape::Indirect;
    }
    match elems {
        [Elem::Node(n)] if n.kind() == SyntaxKind::BraceGroup => Shape::Brace(n.clone()),
        [Elem::Token(t)] => match t.kind() {
            SyntaxKind::Number => Shape::Number,
            SyntaxKind::RealNumber => Shape::Real,
            SyntaxKind::CString => Shape::CString,
            SyntaxKind::QuotedString => Shape::Quoted(t.text().chars().last().unwrap_or(' ')),
            SyntaxKind::LowerName => Shape::Reference(None, t.text().to_owned()),
            SyntaxKind::Keyword => Shape::Keyword(t.text().to_owned()),
            SyntaxKind::UpperName if is_value_keyword(t.text()) => {
                Shape::Keyword(t.text().to_owned())
            }
            _ => Shape::Other,
        },
        [Elem::Token(m), Elem::Token(n)]
            if m.kind() == SyntaxKind::Minus
                && matches!(n.kind(), SyntaxKind::Number | SyntaxKind::RealNumber) =>
        {
            if n.kind() == SyntaxKind::Number {
                Shape::Number
            } else {
                Shape::Real
            }
        }
        [Elem::Token(m), Elem::Token(dot), Elem::Token(n)]
            if m.kind() == SyntaxKind::UpperName
                && dot.kind() == SyntaxKind::Dot
                && n.kind() == SyntaxKind::LowerName =>
        {
            Shape::Reference(Some(m.text().to_owned()), n.text().to_owned())
        }
        [Elem::Token(alt), Elem::Token(colon), rest @ ..]
            if alt.kind() == SyntaxKind::LowerName && colon.kind() == SyntaxKind::Colon =>
        {
            Shape::Choice(alt.text().to_owned(), rest.to_vec())
        }
        _ => Shape::Other,
    }
}

/// Splits the content of a brace group at top-level commas.
fn brace_items(group: &SyntaxNode) -> Vec<Vec<Elem>> {
    let mut items: Vec<Vec<Elem>> = vec![Vec::new()];
    for element in elems_of(group) {
        match &element {
            Elem::Token(t) if matches!(t.kind(), SyntaxKind::LBrace | SyntaxKind::RBrace) => {}
            Elem::Token(t) if t.kind() == SyntaxKind::Comma => items.push(Vec::new()),
            _ => {
                if let Some(last) = items.last_mut() {
                    last.push(element);
                }
            }
        }
    }
    items.retain(|i| !i.is_empty());
    items
}

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    let symbols = ctx.loaded.model.symbols.clone();
    for symbol in &symbols {
        if matches!(symbol.kind, SymbolKind::Macro | SymbolKind::Unknown) {
            continue;
        }
        let index = ctx
            .loaded
            .model
            .symbols
            .iter()
            .position(|s| s.node == symbol.node);
        if index.is_some_and(|i| ctx.graph.is_value_symbol(ctx.module, i))
            && let (Some(ty), Some(value)) = (&symbol.type_node, &symbol.value_node)
            && matches!(value.kind(), SyntaxKind::Value | SyntaxKind::AmbiguousRhs)
            && !is_oid_valued(ctx, symbol)
        {
            check_value(ctx, symbol, ty, &elems_of(value), false);
        }
        for node in symbol.node.descendants() {
            match node.kind() {
                SyntaxKind::ComponentType => check_default(ctx, symbol, &node),
                SyntaxKind::Tag => check_tag_reference(ctx, symbol, &node),
                SyntaxKind::NamedNumber | SyntaxKind::Enumeration => {
                    check_number_reference(ctx, symbol, &node);
                }
                _ => {}
            }
        }
    }
}

fn is_oid_valued(ctx: &Ctx<'_, '_, '_>, symbol: &Symbol) -> bool {
    ctx.loaded
        .model
        .symbols
        .iter()
        .position(|s| s.node == symbol.node)
        .is_some_and(|index| ctx.eval.symbol_is_oid_valued(ctx.module, index))
}

// X.680 25.12
fn check_default(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, node: &SyntaxNode) {
    let Some(named) = child(node, SyntaxKind::NamedType) else {
        return;
    };
    let (Some(ty), Some(value)) = (type_child(&named), child(node, SyntaxKind::Value)) else {
        return;
    };
    check_value(ctx, symbol, &ty, &elems_of(&value), false);
}

// X.680 31.2.3: the class number reference must be defined
fn check_tag_reference(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, node: &SyntaxNode) {
    let decl = crate::view::read_tag(node);
    if let Some((qualifier, name)) = &decl.reference {
        check_integer_reference(
            ctx,
            symbol,
            qualifier.as_deref(),
            name,
            decl.range,
            "tag number",
        );
    }
}

// X.680 19.x: a named number or enumeration value written as a reference
fn check_number_reference(ctx: &mut Ctx<'_, '_, '_>, symbol: &Symbol, node: &SyntaxNode) {
    let Some(value) = child(node, SyntaxKind::Value) else {
        return;
    };
    if let Shape::Reference(qualifier, name) = shape(&elems_of(&value)) {
        let range = trimmed_range(&value);
        check_integer_reference(ctx, symbol, qualifier.as_deref(), &name, range, "number");
    }
}

fn check_integer_reference(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    qualifier: Option<&str>,
    name: &str,
    range: TextRange,
    what: &str,
) {
    let Some(lookup) = check_reference(ctx, symbol, qualifier, name, range, "value") else {
        return;
    };
    if let Found::Symbol { module, index } = lookup.found
        && ctx
            .eval
            .integer_of_name(ctx.module, qualifier, name)
            .is_none()
        && let Some(target) = ctx.graph.symbol(module, index)
        && target.kind == SymbolKind::Value
        && let Some(ty) = &target.type_node
        && let Some(peeled) = ctx.graph.peel_type(module, ty)
        && base_kind_of_node(&peeled.node).is_some_and(|k| k != BaseKind::Integer)
    {
        let finding = Finding::new(
            "E131",
            format!("`{name}` is not an integer value, so it cannot be a {what}"),
            ctx.loc(range),
        );
        ctx.report(finding);
    }
}

fn expectation(kind: BaseKind) -> &'static str {
    match kind {
        BaseKind::Boolean => "a BOOLEAN value (TRUE or FALSE)",
        BaseKind::Integer => "an INTEGER value",
        BaseKind::BitString => "a BIT STRING value",
        BaseKind::OctetString => "an OCTET STRING value",
        BaseKind::Null => "NULL",
        BaseKind::ObjectIdentifier => "an OBJECT IDENTIFIER value",
        BaseKind::RelativeOid => "a RELATIVE-OID value",
        BaseKind::Real => "a REAL value",
        BaseKind::Enumerated => "an ENUMERATED value",
        BaseKind::Sequence => "a SEQUENCE value",
        BaseKind::Set => "a SET value",
        BaseKind::SequenceOf => "a SEQUENCE OF value",
        BaseKind::SetOf => "a SET OF value",
        BaseKind::Choice => "a CHOICE value (`alternative : value`)",
        BaseKind::CharString => "a character string value",
        BaseKind::Time => "a time value (a quoted string)",
        BaseKind::Other => "a value",
    }
}

fn mismatch(ctx: &mut Ctx<'_, '_, '_>, kind: BaseKind, elems: &[Elem]) {
    let Some(range) = span(elems) else {
        return;
    };
    let finding = Finding::new(
        "E131",
        format!("expected {}", expectation(kind)),
        ctx.loc(range),
    );
    ctx.report(finding);
}

/// Checks `elems` as a value of the type written at `ty` (X.680 Annex C.6 and the value clauses).
pub(super) fn check_value(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    ty: &SyntaxNode,
    elems: &[Elem],
    allow_min_max: bool,
) {
    if elems.is_empty() {
        return;
    }
    if elems
        .iter()
        .any(|e| matches!(e, Elem::Node(n) if n.kind() == SyntaxKind::XmlValue))
    {
        return;
    }
    let Some(peeled) = ctx.graph.peel_type_through_tags(ctx.module, ty) else {
        return;
    };
    let Some(kind) = base_kind_of_node(&peeled.node) else {
        return;
    };
    let the_shape = shape(elems);
    if matches!(the_shape, Shape::Indirect) {
        return;
    }
    let named = named_identifiers(&peeled.node);

    if let Shape::Reference(qualifier, name) = &the_shape {
        let identifier_of_type = qualifier.is_none() && named.iter().any(|n| n == name);
        if identifier_of_type {
            return;
        }
        check_value_reference(ctx, symbol, qualifier.as_deref(), name, kind, elems);
        return;
    }
    if let Shape::Keyword(word) = &the_shape
        && allow_min_max
        && matches!(word.as_str(), "MIN" | "MAX")
    {
        return;
    }
    let ok = match (kind, &the_shape) {
        (BaseKind::Boolean, Shape::Keyword(w)) => matches!(w.as_str(), "TRUE" | "FALSE"),
        (BaseKind::Null, Shape::Keyword(w)) => w == "NULL",
        (BaseKind::Integer, Shape::Number) => true,
        (BaseKind::Real, Shape::Number | Shape::Real | Shape::Brace(_)) => true,
        (BaseKind::Real, Shape::Keyword(w)) => {
            matches!(
                w.as_str(),
                "PLUS-INFINITY" | "MINUS-INFINITY" | "NOT-A-NUMBER"
            )
        }
        (BaseKind::Enumerated, _) => false,
        (BaseKind::BitString | BaseKind::OctetString, Shape::Quoted(c)) => {
            matches!(c, 'B' | 'H' | 'b' | 'h')
        }
        (BaseKind::BitString | BaseKind::OctetString, Shape::Brace(_)) => true,
        (BaseKind::CharString, Shape::CString | Shape::Brace(_)) => true,
        (BaseKind::Time, Shape::CString) => true,
        (BaseKind::RelativeOid, Shape::Brace(_)) => return,
        (BaseKind::ObjectIdentifier, Shape::Brace(group)) => {
            let result = ctx.eval.eval_oid_group(ctx.module, group);
            super::oids::report_problems(ctx, &result);
            return;
        }
        (BaseKind::Choice, Shape::Choice(alt, rest)) => {
            check_choice_value(ctx, symbol, &peeled.node, peeled.module, alt, rest, elems);
            return;
        }
        (BaseKind::Sequence | BaseKind::Set, Shape::Brace(group)) => {
            check_sequence_value(ctx, symbol, &peeled.node, peeled.module, group);
            return;
        }
        (BaseKind::SequenceOf | BaseKind::SetOf, Shape::Brace(group)) => {
            check_of_value(ctx, symbol, &peeled.node, group);
            return;
        }
        _ => false,
    };
    if !ok {
        mismatch(ctx, kind, elems);
    }
}

/// The identifiers a value of this type can be written as (named numbers, enumerations, named bits).
fn named_identifiers(node: &SyntaxNode) -> Vec<String> {
    let mut names = Vec::new();
    match node.kind() {
        SyntaxKind::IntegerType | SyntaxKind::BitStringType => {
            for list in node
                .children()
                .filter(|c| c.kind() == SyntaxKind::NamedNumberList)
            {
                for item in list
                    .children()
                    .filter(|c| c.kind() == SyntaxKind::NamedNumber)
                {
                    if let Some(t) = tokens(&item)
                        .into_iter()
                        .find(|t| t.kind() == SyntaxKind::LowerName)
                    {
                        names.push(t.text().to_owned());
                    }
                }
            }
        }
        SyntaxKind::EnumeratedType => {
            for item in node
                .children()
                .filter(|c| c.kind() == SyntaxKind::Enumeration)
            {
                if let Some(t) = tokens(&item)
                    .into_iter()
                    .find(|t| t.kind() == SyntaxKind::LowerName)
                {
                    names.push(t.text().to_owned());
                }
            }
        }
        _ => {}
    }
    names
}

// X.680 14.6, C.6
fn check_value_reference(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    qualifier: Option<&str>,
    name: &str,
    expected: BaseKind,
    elems: &[Elem],
) {
    let Some(range) = span(elems) else {
        return;
    };
    let Some(lookup) = check_reference(ctx, symbol, qualifier, name, range, "value") else {
        return;
    };
    let Found::Symbol { module, index } = lookup.found else {
        return;
    };
    let Some(target) = ctx.graph.symbol(module, index) else {
        return;
    };
    if !ctx.graph.is_value_symbol(module, index) {
        return;
    }
    let Some(actual) = target
        .type_node
        .as_ref()
        .and_then(|t| ctx.graph.peel_type_through_tags(module, t))
        .and_then(|p| base_kind_of_node(&p.node))
    else {
        return;
    };
    if !compatible(expected, actual) {
        let finding = Finding::new(
            "E131",
            format!(
                "`{name}` is {}, but {} is expected",
                expectation(actual),
                expectation(expected)
            ),
            ctx.loc(range),
        );
        ctx.report(finding);
    }
}

fn compatible(expected: BaseKind, actual: BaseKind) -> bool {
    expected == actual
        || matches!(
            (expected, actual),
            (BaseKind::Set, BaseKind::Sequence)
                | (BaseKind::Sequence, BaseKind::Set)
                | (BaseKind::Other, _)
                | (_, BaseKind::Other)
        )
}

// X.680 29.11
fn check_choice_value(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    choice: &SyntaxNode,
    choice_module: crate::graph::ModuleId,
    alt: &str,
    rest: &[Elem],
    whole: &[Elem],
) {
    let list = ComponentList::read(choice);
    let alternative = list.components().find(|c| c.name == alt);
    let Some(alternative) = alternative else {
        if let Some(range) = span(whole) {
            let finding = Finding::new(
                "E131",
                format!("`{alt}` is not an alternative of this CHOICE"),
                ctx.loc(range),
            );
            ctx.report(finding);
        }
        return;
    };
    if choice_module == ctx.module {
        check_value(ctx, symbol, &alternative.ty, rest, false);
    }
}

// X.680 25.18-25.20
fn check_sequence_value(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    node: &SyntaxNode,
    node_module: crate::graph::ModuleId,
    group: &SyntaxNode,
) {
    let list = ComponentList::read(node);
    let flat = expand(ctx.graph, node_module, &list, 0);
    let items = brace_items(group);
    // X.208 allowed omitting the component identifiers; such positional values are left unchecked
    let positional = items.iter().any(|item| {
        !matches!(item.first(), Some(Elem::Token(t)) if t.kind() == SyntaxKind::LowerName)
            || item.len() < 2
    });
    if positional && !items.is_empty() {
        return;
    }
    let mut given: Vec<String> = Vec::new();
    for item in items {
        let Some(Elem::Token(first)) = item.first() else {
            continue;
        };
        if first.kind() != SyntaxKind::LowerName {
            mismatch(ctx, BaseKind::Sequence, &item);
            continue;
        }
        let Some(entry) = flat.iter().find(|f| f.component.name == first.text()) else {
            let finding = Finding::new(
                "E131",
                format!("`{}` is not a component of this type", first.text()),
                ctx.loc(first.text_range()),
            );
            ctx.report(finding);
            continue;
        };
        given.push(first.text().to_owned());
        if entry.module == ctx.module {
            let rest: Vec<Elem> = item.iter().skip(1).cloned().collect();
            check_value(ctx, symbol, &entry.component.ty, &rest, false);
        }
    }
    let missing: Vec<&str> = flat
        .iter()
        .filter(|f| f.component.region != Region::Addition && !f.component.is_optional_or_default())
        .map(|f| f.component.name.as_str())
        .filter(|name| !given.iter().any(|g| g == name))
        .collect();
    if !missing.is_empty() {
        let finding = Finding::new(
            "E131",
            format!("missing value for component `{}`", missing.join("`, `")),
            ctx.loc(trimmed_range(group)),
        );
        ctx.report(finding);
    }
}

// X.680 26.x, 28.x
fn check_of_value(
    ctx: &mut Ctx<'_, '_, '_>,
    symbol: &Symbol,
    node: &SyntaxNode,
    group: &SyntaxNode,
) {
    let Some(element_type) = of_element_type(node) else {
        return;
    };
    for item in brace_items(group) {
        let elems: Vec<Elem> = match item.first() {
            Some(Elem::Token(t))
                if t.kind() == SyntaxKind::LowerName
                    && item.get(1).is_some_and(|e| !matches!(e, Elem::Token(n) if n.kind() == SyntaxKind::Colon || n.kind() == SyntaxKind::Dot)) =>
            {
                item.iter().skip(1).cloned().collect()
            }
            _ => item,
        };
        check_value(ctx, symbol, &element_type, &elems, false);
    }
}
