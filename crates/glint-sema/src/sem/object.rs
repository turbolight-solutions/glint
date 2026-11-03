//! Information objects (X.681 11) and object sets (X.681 12): their bodies are read once the governing class is
//! known.

use glint_syntax::{
    ParseOptions, PrefixKind, SyntaxKind, SyntaxNode, TextRange, TextSize, parse_prefix,
};

use super::class::{ClassInfo, FieldKind, SynItem};
use super::toks::{Tk, inner, skip_group, span, split_commas};
use super::{Sem, SymRef, range_of};
use crate::graph::{ModuleId, Origin};

/// A defect found while reading an object, reported by the check that asked.
#[derive(Debug, Clone)]
pub struct Problem {
    pub code: &'static str,
    pub message: String,
    pub range: TextRange,
}

impl Problem {
    fn new(code: &'static str, message: impl Into<String>, range: TextRange) -> Self {
        Self {
            code,
            message: message.into(),
            range,
        }
    }
}

/// The setting of one field of an object.
#[derive(Debug, Clone)]
pub struct Setting {
    /// With the leading `&`.
    pub field: String,
    pub field_range: TextRange,
    /// The text of the setting itself.
    pub range: TextRange,
    pub kind: FieldKind,
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub class: SymRef,
    pub settings: Vec<Setting>,
    pub range: TextRange,
}

#[derive(Debug, Clone)]
pub struct ParsedObject {
    pub info: ObjectInfo,
    pub problems: Vec<Problem>,
}

impl Sem<'_, '_> {
    /// The text of the source `module` was parsed from.
    pub fn source_text(&self, module: ModuleId) -> &str {
        self.graph
            .module(module)
            .and_then(|m| self.graph.unit(m.source))
            .map_or("", |unit| unit.text.as_str())
    }

    pub fn parse_options(&self, module: ModuleId) -> ParseOptions {
        match self.graph.module(module).map(|m| m.origin) {
            Some(Origin::Draft) => ParseOptions::default(),
            _ => ParseOptions::corpus(),
        }
    }

    /// Reads an `ObjectDefn` (X.681 11.4): default syntax when the class has no `WITH SYNTAX`, defined syntax otherwise.
    pub fn parse_object(
        &self,
        module: ModuleId,
        class: &ClassInfo,
        body: &SyntaxNode,
    ) -> ParsedObject {
        self.parse_object_tokens(module, class, &inner(body), range_of(body))
    }

    /// Like [`Sem::parse_object`] for the tokens between the braces of an object.
    pub fn parse_object_tokens(
        &self,
        module: ModuleId,
        class: &ClassInfo,
        tokens: &[Tk],
        range: TextRange,
    ) -> ParsedObject {
        let mut matcher = Matcher {
            class,
            text: self.source_text(module),
            options: self.parse_options(module),
            settings: Vec::new(),
            problems: Vec::new(),
        };
        match &class.syntax {
            Some(spec) => matcher.defined_syntax(tokens, &spec.items, range),
            None => matcher.default_syntax(tokens, range),
        }
        matcher.finish(range)
    }
}

struct Matcher<'a> {
    class: &'a ClassInfo,
    text: &'a str,
    options: ParseOptions,
    settings: Vec<Setting>,
    problems: Vec<Problem>,
}

impl Matcher<'_> {
    fn problem(&mut self, message: impl Into<String>, range: TextRange) {
        self.problems.push(Problem::new("E152", message, range));
    }

    fn finish(mut self, range: TextRange) -> ParsedObject {
        // X.681 11.5, 10.11
        let missing: Vec<String> = self
            .class
            .fields
            .iter()
            .filter(|f| f.is_mandatory() && !self.settings.iter().any(|s| s.field == f.name))
            .map(|f| f.name.clone())
            .collect();
        for name in missing {
            self.problem(format!("mandatory field `{name}` has no setting"), range);
        }
        ParsedObject {
            info: ObjectInfo {
                class: self.class.sym,
                settings: self.settings,
                range,
            },
            problems: self.problems,
        }
    }

    // X.681 11.5
    fn default_syntax(&mut self, tokens: &[Tk], body: TextRange) {
        if tokens.is_empty() {
            return;
        }
        for entry in split_commas(tokens) {
            let Some(first) = entry.first() else {
                self.problem("empty field setting", body);
                continue;
            };
            if !matches!(first.kind, SyntaxKind::FieldUpper | SyntaxKind::FieldLower) {
                self.problem(
                    format!(
                        "expected a field reference such as `&id`, found `{}`: this class has no WITH SYNTAX",
                        first.text
                    ),
                    first.range,
                );
                continue;
            }
            let Some(field) = self.class.field(&first.text) else {
                self.problem(
                    format!("the class has no field `{}`", first.text),
                    first.range,
                );
                continue;
            };
            if self.settings.iter().any(|s| s.field == first.text) {
                self.problem(format!("`{}` is set twice", first.text), first.range);
                continue;
            }
            let rest = entry.get(1..).unwrap_or_default();
            let kind = field.kind;
            let limit = entry.last().map_or(first.range.end(), |t| t.range.end());
            let mut at = 0usize;
            match self.setting(rest, &mut at, kind, limit) {
                Some(range) if at == rest.len() => self.settings.push(Setting {
                    field: first.text.clone(),
                    field_range: first.range,
                    range,
                    kind,
                }),
                Some(range) => {
                    let extra = rest.get(at).map_or(range, |t| t.range);
                    self.problem(
                        format!(
                            "unexpected `{}` after the setting of `{}`",
                            token_text(rest, at),
                            first.text
                        ),
                        extra,
                    );
                }
                None => {}
            }
        }
    }

    // X.681 11.6, 10.10
    fn defined_syntax(&mut self, tokens: &[Tk], items: &[SynItem], body: TextRange) {
        let limit = tokens.last().map_or(body.end(), |t| t.range.end());
        let mut at = 0usize;
        if self.list(tokens, &mut at, items, &[], limit).is_none() {
            return;
        }
        if let Some(token) = tokens.get(at) {
            self.problem(
                format!(
                    "unexpected `{}`: the object does not follow the class's WITH SYNTAX",
                    token.text
                ),
                token.range,
            );
        }
    }

    fn list(
        &mut self,
        tokens: &[Tk],
        at: &mut usize,
        items: &[SynItem],
        outer_follow: &[String],
        limit: TextSize,
    ) -> Option<()> {
        for (index, item) in items.iter().enumerate() {
            match item {
                SynItem::Literal(text, _) => match tokens.get(*at) {
                    Some(token) if literal_matches(token, text) => *at = at.saturating_add(1),
                    other => {
                        let range = other.map_or_else(|| TextRange::empty(limit), |t| t.range);
                        self.problem(
                            format!("expected `{text}` according to the class's WITH SYNTAX"),
                            range,
                        );
                        return None;
                    }
                },
                SynItem::Field(name, _) => {
                    let field = self.class.field(name)?;
                    let Some(first) = tokens.get(*at) else {
                        self.problem(
                            format!("missing setting for `{name}`"),
                            TextRange::empty(limit),
                        );
                        return None;
                    };
                    let start = first.range;
                    let kind = field.kind;
                    let range = self.setting(tokens, at, kind, limit)?;
                    if self.settings.iter().any(|s| s.field == *name) {
                        self.problem(format!("`{name}` is set twice"), start);
                    }
                    self.settings.push(Setting {
                        field: name.clone(),
                        field_range: start,
                        range,
                        kind,
                    });
                }
                SynItem::Group(nested, _) => {
                    let follow = following(items, index, outer_follow);
                    if group_present(tokens.get(*at), nested, &follow) {
                        self.list(tokens, at, nested, &follow, limit)?;
                    }
                }
            }
        }
        Some(())
    }

    /// Reads one setting that starts at `tokens[*at]` and advances `at` past it (X.681 11.7).
    fn setting(
        &mut self,
        tokens: &[Tk],
        at: &mut usize,
        kind: FieldKind,
        limit: TextSize,
    ) -> Option<TextRange> {
        let Some(first) = tokens.get(*at) else {
            self.problem("missing setting", TextRange::empty(limit));
            return None;
        };
        let start = first.range.start();
        let braces = matches!(
            kind,
            FieldKind::FixedValueSet
                | FieldKind::VariableValueSet
                | FieldKind::ObjectSet(_)
                | FieldKind::UnknownUpper
        );
        if braces {
            if !first.is(SyntaxKind::LBrace) {
                self.problem(
                    format!(
                        "a {} takes a setting in braces, found `{}`",
                        kind.describe(),
                        first.text
                    ),
                    first.range,
                );
                return None;
            }
            let next = skip_group(tokens, *at);
            let end = tokens
                .get(next.saturating_sub(1))
                .map_or(first.range.end(), |t| t.range.end());
            *at = next;
            return Some(TextRange::new(start, end));
        }
        let what = if kind == FieldKind::Type {
            PrefixKind::Type
        } else {
            PrefixKind::Value
        };
        let result = parse_prefix(
            self.text,
            usize::from(start),
            usize::from(limit),
            what,
            self.options,
        );
        if !result.ok {
            let message = if kind == FieldKind::Type {
                "a type field needs a type".to_owned()
            } else {
                format!(
                    "`{}` does not start a valid value for a {}",
                    first.text,
                    kind.describe()
                )
            };
            self.problem(message, first.range);
            return None;
        }
        let end = result.end;
        while tokens.get(*at).is_some_and(|t| t.range.start() < end) {
            *at = at.saturating_add(1);
        }
        Some(TextRange::new(start, end))
    }
}

fn token_text(tokens: &[Tk], at: usize) -> &str {
    tokens.get(at).map_or("", |t| t.text.as_str())
}

fn literal_matches(token: &Tk, text: &str) -> bool {
    if text == "," {
        token.is(SyntaxKind::Comma)
    } else {
        token.is_word(text)
    }
}

/// The literals that may come right after `items[index]` (X.681 10.10).
fn following(items: &[SynItem], index: usize, outer: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items.iter().skip(index.saturating_add(1)) {
        if let Some(text) = super::class::first_literal(item) {
            out.push(text.to_owned());
        }
        if !matches!(item, SynItem::Group(..)) {
            return out;
        }
    }
    out.extend(outer.iter().cloned());
    out
}

// X.681 10.10: a group is present exactly when the next token can start it
fn group_present(next: Option<&Tk>, nested: &[SynItem], follow: &[String]) -> bool {
    let Some(token) = next else { return false };
    match nested.first().and_then(super::class::first_literal) {
        Some(text) => literal_matches(token, text),
        None => {
            let word = token.is(SyntaxKind::UpperName) && follow.iter().any(|f| f == &token.text);
            !word
        }
    }
}

// ---- object sets ----------------------------------------------------------------------------

/// One operand of an object set expression (X.681 12.10).
#[derive(Debug, Clone)]
pub enum Operand {
    /// `{ ... }`: an object written in place.
    Inline {
        body: Vec<Tk>,
        range: TextRange,
    },
    /// `name`, `Module.Name`, `name{...}`, `obj.&field`, `Set.&field`.
    Named {
        qualifier: Option<String>,
        name: String,
        name_range: TextRange,
        lower: bool,
        actuals: Option<TextRange>,
        /// The `.&field` chain after the name.
        path: Vec<(String, TextRange)>,
        range: TextRange,
    },
    Other {
        range: TextRange,
    },
}

/// The syntax of an `ObjectSet` (X.681 12.3).
#[derive(Debug, Clone, Default)]
pub struct ObjectSetSyntax {
    pub operands: Vec<Operand>,
    /// `...` appears.
    pub extensible: bool,
    /// `INTERSECTION`, `EXCEPT` or `ALL EXCEPT` appear, so the members cannot be listed by looking at operands.
    pub arithmetic: bool,
}

/// Reads the elements between the braces of an object set.
pub fn parse_object_set_syntax(group: &SyntaxNode) -> ObjectSetSyntax {
    let tokens = inner(group);
    let mut out = ObjectSetSyntax::default();
    read_set(&tokens, &mut out);
    out
}

fn read_set(tokens: &[Tk], out: &mut ObjectSetSyntax) {
    let mut at = 0usize;
    while let Some(token) = tokens.get(at) {
        match token.kind {
            SyntaxKind::Ellipsis => {
                out.extensible = true;
                at = at.saturating_add(1);
            }
            SyntaxKind::Comma | SyntaxKind::Pipe | SyntaxKind::Caret => {
                if token.kind == SyntaxKind::Caret {
                    out.arithmetic = true;
                }
                at = at.saturating_add(1);
            }
            SyntaxKind::UpperName | SyntaxKind::Keyword
                if matches!(token.text.as_str(), "UNION") =>
            {
                at = at.saturating_add(1);
            }
            SyntaxKind::UpperName | SyntaxKind::Keyword
                if matches!(token.text.as_str(), "INTERSECTION" | "EXCEPT" | "ALL") =>
            {
                out.arithmetic = true;
                at = at.saturating_add(1);
            }
            SyntaxKind::LBrace => {
                let end = skip_group(tokens, at);
                let body: Vec<Tk> = tokens
                    .get(at..end)
                    .unwrap_or_default()
                    .iter()
                    .skip(1)
                    .take(end.saturating_sub(at).saturating_sub(2))
                    .cloned()
                    .collect();
                let range = span(tokens.get(at..end).unwrap_or_default()).unwrap_or(token.range);
                out.operands.push(Operand::Inline { body, range });
                at = end;
            }
            SyntaxKind::LParen => {
                let end = skip_group(tokens, at);
                let nested = tokens
                    .get(at.saturating_add(1)..end.saturating_sub(1))
                    .unwrap_or_default();
                read_set(nested, out);
                at = end;
            }
            SyntaxKind::LowerName | SyntaxKind::UpperName => {
                let (operand, next) = read_named(tokens, at);
                out.operands.push(operand);
                at = next;
            }
            _ => {
                out.operands.push(Operand::Other { range: token.range });
                at = at.saturating_add(1);
            }
        }
    }
}

fn read_named(tokens: &[Tk], start: usize) -> (Operand, usize) {
    let Some(first) = tokens.get(start) else {
        return (
            Operand::Other {
                range: TextRange::default(),
            },
            start.saturating_add(1),
        );
    };
    let lower = first.is(SyntaxKind::LowerName);
    let mut at = start.saturating_add(1);
    let mut qualifier = None;
    let mut name = first.clone();
    if !lower
        && tokens.get(at).is_some_and(|t| t.is(SyntaxKind::Dot))
        && tokens
            .get(at.saturating_add(1))
            .is_some_and(|t| t.is(SyntaxKind::LowerName) || t.is(SyntaxKind::UpperName))
        && let Some(second) = tokens.get(at.saturating_add(1))
    {
        qualifier = Some(first.text.clone());
        name = second.clone();
        at = at.saturating_add(2);
    }
    let lower = lower || name.is(SyntaxKind::LowerName);
    let mut actuals = None;
    if tokens.get(at).is_some_and(|t| t.is(SyntaxKind::LBrace)) {
        let end = skip_group(tokens, at);
        actuals = span(tokens.get(at..end).unwrap_or_default());
        at = end;
    }
    let mut path = Vec::new();
    while tokens.get(at).is_some_and(|t| t.is(SyntaxKind::Dot))
        && let Some(field) = tokens.get(at.saturating_add(1))
        && matches!(field.kind, SyntaxKind::FieldUpper | SyntaxKind::FieldLower)
    {
        path.push((field.text.clone(), field.range));
        at = at.saturating_add(2);
    }
    let range = span(tokens.get(start..at).unwrap_or_default()).unwrap_or(first.range);
    (
        Operand::Named {
            qualifier,
            name: name.text.clone(),
            name_range: name.range,
            lower,
            actuals,
            path,
            range,
        },
        at,
    )
}
