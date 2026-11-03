//! Class definitions (X.681 9) and their `WITH SYNTAX` lists (X.681 10).

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use super::toks::{Tk, inner};
use super::{ClassRes, Sem, SymRef, range_of};
use crate::view::{child, tokens, type_child};

/// What a field is, once its governor is resolved (X.681 9.5-9.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Type,
    FixedValue,
    VariableValue,
    FixedValueSet,
    VariableValueSet,
    Object(ClassRes),
    ObjectSet(ClassRes),
    /// The governor comes from an unavailable module, so value and object fields cannot be told apart.
    UnknownLower,
    UnknownUpper,
    /// `&lower` with no type.
    Invalid,
}

impl FieldKind {
    pub fn is_value_like(self) -> bool {
        matches!(
            self,
            Self::FixedValue
                | Self::VariableValue
                | Self::FixedValueSet
                | Self::VariableValueSet
                | Self::UnknownLower
                | Self::UnknownUpper
        )
    }

    pub fn is_link(self) -> bool {
        matches!(self, Self::Object(_) | Self::ObjectSet(_))
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Type => "type field",
            Self::FixedValue => "fixed-type value field",
            Self::VariableValue => "variable-type value field",
            Self::FixedValueSet => "fixed-type value set field",
            Self::VariableValueSet => "variable-type value set field",
            Self::Object(_) => "object field",
            Self::ObjectSet(_) => "object set field",
            Self::UnknownLower | Self::UnknownUpper | Self::Invalid => "field",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Field {
    /// With the leading `&`.
    pub name: String,
    pub name_range: TextRange,
    pub kind: FieldKind,
    pub optional: bool,
    pub has_default: bool,
    pub unique: bool,
    /// The type or class after the field name.
    pub governor: Option<SyntaxNode>,
    /// The `FieldName` of a variable-type field.
    pub link: Option<SyntaxNode>,
    pub default: Option<SyntaxNode>,
    pub node: SyntaxNode,
}

impl Field {
    pub fn is_mandatory(&self) -> bool {
        !self.optional && !self.has_default
    }
}

/// One element of a `SyntaxList` (X.681 10.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SynItem {
    Literal(String, TextRange),
    Field(String, TextRange),
    Group(Vec<SynItem>, TextRange),
}

impl SynItem {
    pub fn range(&self) -> TextRange {
        match self {
            Self::Literal(_, r) | Self::Field(_, r) | Self::Group(_, r) => *r,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyntaxSpec {
    pub items: Vec<SynItem>,
    pub range: TextRange,
    /// Malformed spots found while reading, as `(message, range)`.
    pub problems: Vec<(String, TextRange)>,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub sym: SymRef,
    pub node: SyntaxNode,
    pub fields: Vec<Field>,
    pub syntax: Option<SyntaxSpec>,
}

impl ClassInfo {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

pub(super) fn build(sem: &Sem<'_, '_>, sym: SymRef) -> Option<ClassInfo> {
    let symbol = sem.symbol(sym)?;
    let node = symbol
        .type_node
        .clone()
        .filter(|n| n.kind() == SyntaxKind::ObjectClassDef)?;
    let fields = child(&node, SyntaxKind::ClassFieldList)
        .into_iter()
        .flat_map(|list| list.children().collect::<Vec<_>>())
        .filter(|n| n.kind() == SyntaxKind::FieldSpec)
        .filter_map(|spec| read_field(sem, sym, &spec))
        .collect();
    let syntax = node
        .children()
        .find(|n| n.kind() == SyntaxKind::BraceGroup)
        .map(|group| read_syntax(&group));
    Some(ClassInfo {
        sym,
        node,
        fields,
        syntax,
    })
}

// X.681 9.5-9.12
fn read_field(sem: &Sem<'_, '_>, class: SymRef, spec: &SyntaxNode) -> Option<Field> {
    let toks = tokens(spec);
    let name = toks
        .iter()
        .find(|t| matches!(t.kind(), SyntaxKind::FieldUpper | SyntaxKind::FieldLower))?;
    let upper = name.kind() == SyntaxKind::FieldUpper;
    let mut unique = false;
    let mut optional = false;
    let mut has_default = false;
    for token in &toks {
        if token.kind() == SyntaxKind::Keyword {
            match token.text() {
                "UNIQUE" => unique = true,
                "OPTIONAL" => optional = true,
                "DEFAULT" => has_default = true,
                _ => {}
            }
        }
    }
    let link = child(spec, SyntaxKind::FieldName);
    let default_kw_end = toks
        .iter()
        .find(|t| t.kind() == SyntaxKind::Keyword && t.text() == "DEFAULT")
        .map(|t| t.text_range().end());
    let before_default =
        |n: &SyntaxNode| default_kw_end.is_none_or(|end| n.text_range().end() <= end);
    let governor = spec
        .children()
        .find(|n| before_default(n) && crate::view::is_type_node(n.kind()));
    let default = spec
        .children()
        .find(|n| {
            !before_default(n) && matches!(n.kind(), SyntaxKind::Value | SyntaxKind::BraceGroup)
        })
        .or_else(|| {
            spec.children()
                .find(|n| !before_default(n) && crate::view::is_type_node(n.kind()))
        });
    let class_res = governor.as_ref().map(|g| sem.class_of(class.module, g));
    let kind = match (upper, &link, &governor) {
        (true, None, None) => FieldKind::Type,
        (false, None, None) => FieldKind::Invalid,
        (true, Some(_), _) => FieldKind::VariableValueSet,
        (false, Some(_), _) => FieldKind::VariableValue,
        (true, None, Some(_)) => match class_res {
            Some(ClassRes::Class(c)) => FieldKind::ObjectSet(ClassRes::Class(c)),
            Some(ClassRes::NotClass) => FieldKind::FixedValueSet,
            _ => FieldKind::UnknownUpper,
        },
        (false, None, Some(_)) => match class_res {
            Some(ClassRes::Class(c)) => FieldKind::Object(ClassRes::Class(c)),
            Some(ClassRes::NotClass) => FieldKind::FixedValue,
            _ => FieldKind::UnknownLower,
        },
    };
    let _ = type_child;
    Some(Field {
        name: name.text().to_owned(),
        name_range: name.text_range(),
        kind,
        optional,
        has_default,
        unique,
        governor,
        link,
        default,
        node: spec.clone(),
    })
}

/// Words that may not be used as literals (X.681 10.6).
pub const FORBIDDEN_LITERALS: &[&str] = &[
    "ABSTRACT-SYNTAX",
    "BIT",
    "BOOLEAN",
    "CHARACTER",
    "CHOICE",
    "CONTAINING",
    "DATE",
    "DATE-TIME",
    "DURATION",
    "EMBEDDED",
    "END",
    "ENUMERATED",
    "EXTERNAL",
    "FALSE",
    "INSTANCE",
    "INTEGER",
    "MINUS-INFINITY",
    "NOT-A-NUMBER",
    "NULL",
    "OBJECT",
    "OCTET",
    "OID-IRI",
    "PLUS-INFINITY",
    "REAL",
    "RELATIVE-OID",
    "RELATIVE-OID-IRI",
    "SEQUENCE",
    "SET",
    "TIME",
    "TIME-OF-DAY",
    "TRUE",
    "TYPE-IDENTIFIER",
];

/// Reads a `WITH SYNTAX` list (X.681 10.5), splitting `[[` and `]]` into two brackets (10.5 Note 2).
fn read_syntax(group: &SyntaxNode) -> SyntaxSpec {
    let mut problems = Vec::new();
    let mut stream: Vec<Tk> = Vec::new();
    for token in inner(group) {
        match token.kind {
            SyntaxKind::LDoubleBracket | SyntaxKind::RDoubleBracket => {
                let single = if token.kind == SyntaxKind::LDoubleBracket {
                    SyntaxKind::LBracket
                } else {
                    SyntaxKind::RBracket
                };
                let text = if single == SyntaxKind::LBracket {
                    "["
                } else {
                    "]"
                };
                let mid = token
                    .range
                    .start()
                    .checked_add(glint_syntax::TextSize::from(1))
                    .unwrap_or(token.range.end());
                stream.push(Tk {
                    kind: single,
                    text: text.to_owned(),
                    range: TextRange::new(token.range.start(), mid),
                });
                stream.push(Tk {
                    kind: single,
                    text: text.to_owned(),
                    range: TextRange::new(mid, token.range.end()),
                });
            }
            _ => stream.push(token),
        }
    }
    let mut at = 0usize;
    let items = read_items(&stream, &mut at, false, &mut problems);
    let range = range_of(group);
    SyntaxSpec {
        items,
        range,
        problems,
    }
}

fn read_items(
    stream: &[Tk],
    at: &mut usize,
    in_group: bool,
    problems: &mut Vec<(String, TextRange)>,
) -> Vec<SynItem> {
    let mut items = Vec::new();
    while let Some(token) = stream.get(*at) {
        match token.kind {
            SyntaxKind::RBracket if in_group => return items,
            SyntaxKind::RBracket => {
                problems.push(("unmatched `]`".to_owned(), token.range));
                *at = at.saturating_add(1);
            }
            SyntaxKind::LBracket => {
                let start = token.range.start();
                *at = at.saturating_add(1);
                let nested = read_items(stream, at, true, problems);
                let end = match stream.get(*at) {
                    Some(close) if close.kind == SyntaxKind::RBracket => {
                        *at = at.saturating_add(1);
                        close.range.end()
                    }
                    _ => {
                        problems.push(("`[` is never closed".to_owned(), token.range));
                        nested.last().map_or(token.range.end(), |i| i.range().end())
                    }
                };
                items.push(SynItem::Group(nested, TextRange::new(start, end)));
            }
            SyntaxKind::FieldUpper | SyntaxKind::FieldLower => {
                items.push(SynItem::Field(token.text.clone(), token.range));
                *at = at.saturating_add(1);
            }
            SyntaxKind::Comma => {
                items.push(SynItem::Literal(",".to_owned(), token.range));
                *at = at.saturating_add(1);
            }
            SyntaxKind::UpperName | SyntaxKind::Keyword => {
                items.push(SynItem::Literal(token.text.clone(), token.range));
                *at = at.saturating_add(1);
            }
            _ => {
                problems.push((
                    format!("`{}` cannot appear in a syntax list", token.text),
                    token.range,
                ));
                *at = at.saturating_add(1);
            }
        }
    }
    items
}

/// The first literal text a group or item would require, if it starts with one.
pub fn first_literal(item: &SynItem) -> Option<&str> {
    match item {
        SynItem::Literal(text, _) => Some(text),
        SynItem::Group(nested, _) => nested.first().and_then(first_literal),
        SynItem::Field(..) => None,
    }
}

pub fn contains_field(items: &[SynItem]) -> bool {
    items.iter().any(|item| match item {
        SynItem::Field(..) => true,
        SynItem::Group(nested, _) => contains_field(nested),
        SynItem::Literal(..) => false,
    })
}
