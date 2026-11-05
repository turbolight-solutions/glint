//! Evaluation of object identifier and integer values (X.680 16.2, 19.x, 32.x).

use std::cell::RefCell;
use std::collections::HashMap;

use glint_corpus::Oid;
use glint_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, TextRange};

use crate::graph::{Found, ModuleGraph, ModuleId};
use crate::names::{registered_arc, validate_arcs};
use crate::view::{child, tokens};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OidProblemKind {
    /// A value reference that is not defined anywhere (E110).
    Undefined { name: String },
    /// A reference that is defined but is not an object identifier value (E131).
    NotAnOid { name: String },
    /// A bare `NameForm` that X.660 does not register (E134).
    BareName { name: String },
    /// The notation inside the braces is not an object identifier value (E131).
    Malformed,
    /// The arcs are not a valid object identifier by themselves (E130).
    Arcs(&'static str),
    /// A reference that refers back to itself (E131).
    Circular { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidProblem {
    pub kind: OidProblemKind,
    pub range: TextRange,
}

/// The result of evaluating an object identifier value.
#[derive(Debug, Clone, Default)]
pub struct OidEval {
    /// `None` when evaluation hit a problem or something unknown (an opaque import).
    pub oid: Option<Oid>,
    pub problems: Vec<OidProblem>,
    /// Author placeholders such as `TBD` or `id-TBD1` found among the components (I141).
    pub placeholders: Vec<(String, TextRange)>,
    /// Value references the OID depends on, in order: `(module qualifier, name, range)`.
    pub references: Vec<(Option<String>, String, TextRange)>,
    /// The labels written in the value (`name(number)` and X.660 names), each with the OID of its arc, while the
    /// arcs before it are known.
    pub labels: Vec<OidLabel>,
}

/// A label written in an object identifier value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidLabel {
    pub name: String,
    pub range: TextRange,
    /// The OID of the arc the label names.
    pub oid: Oid,
}

#[derive(Debug, Clone)]
enum NumberForm {
    Literal(u128),
    Reference(Option<String>, String, TextRange),
}

#[derive(Debug, Clone)]
enum Component {
    Number(u128),
    Name(String, TextRange),
    NameAndNumber(String, TextRange, NumberForm),
    External(String, String, TextRange),
    Bad(TextRange),
}

#[derive(Clone)]
enum Memo<T> {
    InProgress,
    Done(T),
}

type MemoTable<T> = HashMap<(ModuleId, usize), Memo<T>>;

enum RefValue {
    Oid(Oid),
    NotOid,
    Unknown,
    Undefined,
    Circular,
}

pub struct Evaluator<'g, 'c> {
    graph: &'g ModuleGraph<'c>,
    oids: RefCell<MemoTable<Option<Oid>>>,
    integers: RefCell<MemoTable<Option<i128>>>,
}

fn group_tokens(group: &SyntaxNode) -> Vec<SyntaxToken> {
    let all = tokens(group);
    let inner: Vec<SyntaxToken> = all
        .into_iter()
        .filter(|t| !matches!(t.kind(), SyntaxKind::LBrace | SyntaxKind::RBrace))
        .collect();
    inner
}

// X.680 32.3
fn components(group: &SyntaxNode) -> Vec<Component> {
    let toks = group_tokens(group);
    let has_nested = group.children().any(|c| c.kind() == SyntaxKind::BraceGroup);
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(token) = toks.get(i) {
        let range = token.text_range();
        match token.kind() {
            SyntaxKind::Number => {
                match token.text().parse::<u128>() {
                    Ok(n) => out.push(Component::Number(n)),
                    Err(_) => out.push(Component::Bad(range)),
                }
                i = i.saturating_add(1);
            }
            SyntaxKind::LowerName => {
                let next = toks.get(i.saturating_add(1));
                if next.is_some_and(|t| t.kind() == SyntaxKind::LParen) {
                    let (form, used) = number_form(&toks, i.saturating_add(2));
                    let close = toks.get(i.saturating_add(2).saturating_add(used));
                    match (form, close) {
                        (Some(form), Some(c)) if c.kind() == SyntaxKind::RParen => {
                            out.push(Component::NameAndNumber(
                                token.text().to_owned(),
                                range,
                                form,
                            ));
                            i = i.saturating_add(3).saturating_add(used);
                        }
                        _ => {
                            out.push(Component::Bad(range));
                            i = i.saturating_add(1);
                        }
                    }
                } else {
                    out.push(Component::Name(token.text().to_owned(), range));
                    i = i.saturating_add(1);
                }
            }
            SyntaxKind::UpperName
                if toks
                    .get(i.saturating_add(1))
                    .is_some_and(|t| t.kind() == SyntaxKind::Dot)
                    && toks
                        .get(i.saturating_add(2))
                        .is_some_and(|t| t.kind() == SyntaxKind::LowerName) =>
            {
                let name = toks.get(i.saturating_add(2));
                if let Some(name) = name {
                    let whole = TextRange::new(range.start(), name.text_range().end());
                    out.push(Component::External(
                        token.text().to_owned(),
                        name.text().to_owned(),
                        whole,
                    ));
                }
                i = i.saturating_add(3);
            }
            _ => {
                out.push(Component::Bad(range));
                i = i.saturating_add(1);
            }
        }
    }
    if has_nested {
        out.push(Component::Bad(group.text_range()));
    }
    out
}

/// Reads a `NumberForm` starting at `start`; returns it with the number of tokens used.
fn number_form(toks: &[SyntaxToken], start: usize) -> (Option<NumberForm>, usize) {
    let Some(first) = toks.get(start) else {
        return (None, 0);
    };
    match first.kind() {
        SyntaxKind::Number => (first.text().parse().ok().map(NumberForm::Literal), 1),
        SyntaxKind::LowerName => (
            Some(NumberForm::Reference(
                None,
                first.text().to_owned(),
                first.text_range(),
            )),
            1,
        ),
        SyntaxKind::UpperName => {
            let dot = toks.get(start.saturating_add(1));
            let name = toks.get(start.saturating_add(2));
            match (dot, name) {
                (Some(d), Some(n))
                    if d.kind() == SyntaxKind::Dot && n.kind() == SyntaxKind::LowerName =>
                {
                    let range = TextRange::new(first.text_range().start(), n.text_range().end());
                    (
                        Some(NumberForm::Reference(
                            Some(first.text().to_owned()),
                            n.text().to_owned(),
                            range,
                        )),
                        3,
                    )
                }
                _ => (None, 0),
            }
        }
        _ => (None, 0),
    }
}

// design spec: `TBD`, `TBD1`, `id-TBD`
fn is_placeholder(name: &str, extra: &[String]) -> bool {
    if extra.iter().any(|p| p.eq_ignore_ascii_case(name)) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_prefix("id-").unwrap_or(&lower);
    stem.strip_prefix("tbd")
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

impl<'g, 'c> Evaluator<'g, 'c> {
    pub fn new(graph: &'g ModuleGraph<'c>) -> Self {
        Self {
            graph,
            oids: RefCell::new(HashMap::new()),
            integers: RefCell::new(HashMap::new()),
        }
    }

    /// Evaluates an object identifier that uses no references: numbers, `name(number)` and registered names.
    pub fn literal_oid(group: &SyntaxNode) -> Option<Oid> {
        let mut arcs: Vec<u128> = Vec::new();
        for component in components(group) {
            match component {
                Component::Number(n) => arcs.push(n),
                Component::NameAndNumber(_, _, NumberForm::Literal(n)) => arcs.push(n),
                Component::Name(name, _) => arcs.push(registered_arc(&arcs, &name)?),
                _ => return None,
            }
        }
        Some(Oid::new(arcs))
    }

    // X.680 32.3-32.13
    pub fn eval_oid_group(&self, module: ModuleId, group: &SyntaxNode) -> OidEval {
        let mut result = OidEval::default();
        let mut arcs: Vec<u128> = Vec::new();
        let mut known = true;
        let placeholder_names = &self.graph.options().placeholders;
        for (position, component) in components(group).into_iter().enumerate() {
            match component {
                Component::Number(n) => arcs.push(n),
                Component::Bad(range) => {
                    result.problems.push(OidProblem {
                        kind: OidProblemKind::Malformed,
                        range,
                    });
                    known = false;
                }
                Component::NameAndNumber(name, range, form) => {
                    let placeholder = is_placeholder(&name, placeholder_names);
                    if placeholder {
                        result.placeholders.push((name.clone(), range));
                    }
                    match form {
                        NumberForm::Literal(n) => {
                            arcs.push(n);
                            if known && !placeholder {
                                result.labels.push(OidLabel {
                                    name,
                                    range,
                                    oid: Oid::new(arcs.clone()),
                                });
                            }
                        }
                        NumberForm::Reference(qualifier, reference, ref_range) => {
                            if is_placeholder(&reference, placeholder_names) {
                                result.placeholders.push((reference, ref_range));
                                known = false;
                                continue;
                            }
                            result.references.push((
                                qualifier.clone(),
                                reference.clone(),
                                ref_range,
                            ));
                            match self.integer_of_name(module, qualifier.as_deref(), &reference) {
                                Some(value) if value >= 0 => {
                                    arcs.push(u128::try_from(value).unwrap_or_default());
                                    if known && !placeholder {
                                        result.labels.push(OidLabel {
                                            name,
                                            range,
                                            oid: Oid::new(arcs.clone()),
                                        });
                                    }
                                }
                                Some(_) => {
                                    result.problems.push(OidProblem {
                                        kind: OidProblemKind::NotAnOid { name: reference },
                                        range: ref_range,
                                    });
                                    known = false;
                                }
                                None => {
                                    if self.name_is_undefined(
                                        module,
                                        qualifier.as_deref(),
                                        &reference,
                                    ) {
                                        result.problems.push(OidProblem {
                                            kind: OidProblemKind::Undefined { name: reference },
                                            range: ref_range,
                                        });
                                    }
                                    known = false;
                                }
                            }
                        }
                    }
                }
                Component::Name(name, range) => {
                    if is_placeholder(&name, placeholder_names) {
                        result.placeholders.push((name, range));
                        known = false;
                        continue;
                    }
                    if let Some(arc) = registered_arc(&arcs, &name) {
                        arcs.push(arc);
                        if known {
                            result.labels.push(OidLabel {
                                name,
                                range,
                                oid: Oid::new(arcs.clone()),
                            });
                        }
                        continue;
                    }
                    result.references.push((None, name.clone(), range));
                    match self.ref_oid(module, None, &name) {
                        RefValue::Oid(oid) if position == 0 => arcs.extend_from_slice(oid.arcs()),
                        RefValue::Oid(_) | RefValue::NotOid | RefValue::Undefined
                            if position > 0 =>
                        {
                            result.problems.push(OidProblem {
                                kind: OidProblemKind::BareName { name },
                                range,
                            });
                            known = false;
                        }
                        RefValue::NotOid => {
                            result.problems.push(OidProblem {
                                kind: OidProblemKind::NotAnOid { name },
                                range,
                            });
                            known = false;
                        }
                        RefValue::Undefined => {
                            result.problems.push(OidProblem {
                                kind: OidProblemKind::Undefined { name },
                                range,
                            });
                            known = false;
                        }
                        RefValue::Circular => {
                            result.problems.push(OidProblem {
                                kind: OidProblemKind::Circular { name },
                                range,
                            });
                            known = false;
                        }
                        RefValue::Unknown | RefValue::Oid(_) => known = false,
                    }
                }
                Component::External(qualifier, name, range) => {
                    result
                        .references
                        .push((Some(qualifier.clone()), name.clone(), range));
                    match self.ref_oid(module, Some(&qualifier), &name) {
                        RefValue::Oid(oid) if position == 0 => arcs.extend_from_slice(oid.arcs()),
                        RefValue::NotOid => {
                            result.problems.push(OidProblem {
                                kind: OidProblemKind::NotAnOid { name },
                                range,
                            });
                            known = false;
                        }
                        RefValue::Undefined => {
                            result.problems.push(OidProblem {
                                kind: OidProblemKind::Undefined {
                                    name: format!("{qualifier}.{name}"),
                                },
                                range,
                            });
                            known = false;
                        }
                        _ => known = false,
                    }
                }
            }
        }
        if known {
            let oid = Oid::new(arcs);
            if let Some(reason) = validate_arcs(&oid) {
                result.problems.push(OidProblem {
                    kind: OidProblemKind::Arcs(reason),
                    range: group.text_range(),
                });
            }
            result.oid = Some(oid);
        }
        result
    }

    /// The object identifier value named `name` (optionally as `qualifier.name`) in `module`.
    pub fn oid_of_name(
        &self,
        module: ModuleId,
        qualifier: Option<&str>,
        name: &str,
    ) -> Option<Oid> {
        match self.ref_oid(module, qualifier, name) {
            RefValue::Oid(oid) => Some(oid),
            _ => None,
        }
    }

    /// The object identifier assigned by the value assignment `index` of `module`.
    pub fn oid_of_symbol(&self, module: ModuleId, index: usize) -> Option<Oid> {
        let key = (module, index);
        match self.oids.borrow().get(&key) {
            Some(Memo::Done(value)) => return value.clone(),
            Some(Memo::InProgress) => return None,
            None => {}
        }
        self.oids.borrow_mut().insert(key, Memo::InProgress);
        let value = self.compute_symbol_oid(module, index);
        self.oids
            .borrow_mut()
            .insert(key, Memo::Done(value.clone()));
        value
    }

    fn compute_symbol_oid(&self, module: ModuleId, index: usize) -> Option<Oid> {
        let symbol = self.graph.symbol(module, index)?;
        if !self.symbol_is_oid_valued(module, index) {
            return None;
        }
        let value = symbol.value_node.as_ref()?;
        self.eval_oid_value(module, value)
    }

    fn eval_oid_value(&self, module: ModuleId, value: &SyntaxNode) -> Option<Oid> {
        if let Some(group) = child(value, SyntaxKind::BraceGroup) {
            return self.eval_oid_group(module, &group).oid;
        }
        let toks = tokens(value);
        match toks.as_slice() {
            [name] if name.kind() == SyntaxKind::LowerName => {
                self.oid_of_name(module, None, name.text())
            }
            [m, _, name] if m.kind() == SyntaxKind::UpperName => {
                self.oid_of_name(module, Some(m.text()), name.text())
            }
            _ => None,
        }
    }

    /// Every object identifier value written in `module`: its identifier, the identifiers in its IMPORTS and its
    /// `OBJECT IDENTIFIER` value assignments.
    pub fn oid_values(&self, module: ModuleId) -> Vec<OidEval> {
        let Some(loaded) = self.graph.module(module) else {
            return Vec::new();
        };
        let mut groups: Vec<SyntaxNode> = loaded.model.oid_group.iter().cloned().collect();
        for clause in &loaded.model.imports {
            if let Some((crate::model::AssignedId::Oid { group }, _)) = &clause.assigned {
                groups.push(group.clone());
            }
        }
        for (index, symbol) in loaded.model.symbols.iter().enumerate() {
            if self.symbol_is_oid_valued(module, index)
                && let Some(group) = symbol
                    .value_node
                    .as_ref()
                    .and_then(|value| child(value, SyntaxKind::BraceGroup))
            {
                groups.push(group);
            }
        }
        groups
            .iter()
            .map(|group| self.eval_oid_group(module, group))
            .collect()
    }

    /// Every `OBJECT IDENTIFIER` value assignment of `module` that evaluates, as `(name, oid)`.
    pub fn oid_assignments(&self, module: ModuleId) -> Vec<(String, Oid)> {
        let Some(loaded) = self.graph.module(module) else {
            return Vec::new();
        };
        (0..loaded.model.symbols.len())
            .filter_map(|index| {
                let symbol = loaded.model.symbols.get(index)?;
                let oid = self.oid_of_symbol(module, index)?;
                Some((symbol.name.clone(), oid))
            })
            .collect()
    }

    /// Whether the value assignment `index` of `module` has type `OBJECT IDENTIFIER`.
    pub fn symbol_is_oid_valued(&self, module: ModuleId, index: usize) -> bool {
        let Some(symbol) = self.graph.symbol(module, index) else {
            return false;
        };
        if !self.graph.is_value_symbol(module, index) {
            return false;
        }
        let Some(type_node) = symbol.type_node.as_ref() else {
            return false;
        };
        self.graph.peel_type(module, type_node).is_some_and(|p| {
            p.node.kind() == SyntaxKind::BuiltinType
                && crate::view::builtin_text(&p.node) == "OBJECT IDENTIFIER"
        })
    }

    fn name_is_undefined(&self, module: ModuleId, qualifier: Option<&str>, name: &str) -> bool {
        match qualifier {
            None => self.graph.lookup(module, name).is_none(),
            Some(q) => self.graph.lookup_qualified(module, q, name).is_err(),
        }
    }

    fn ref_oid(&self, module: ModuleId, qualifier: Option<&str>, name: &str) -> RefValue {
        let lookup = match qualifier {
            None => self.graph.lookup(module, name),
            Some(q) => self.graph.lookup_qualified(module, q, name).ok(),
        };
        let Some(lookup) = lookup else {
            return RefValue::Undefined;
        };
        match lookup.found {
            Found::Symbol {
                module: target,
                index,
            } => {
                let in_progress = matches!(
                    self.oids.borrow().get(&(target, index)),
                    Some(Memo::InProgress)
                );
                if in_progress {
                    return RefValue::Circular;
                }
                if !self.symbol_is_oid_valued(target, index) {
                    return if self.graph.is_value_symbol(target, index) {
                        RefValue::NotOid
                    } else {
                        RefValue::Unknown
                    };
                }
                match self.oid_of_symbol(target, index) {
                    Some(oid) => RefValue::Oid(oid),
                    None => RefValue::Unknown,
                }
            }
            Found::Opaque | Found::MissingInTarget => RefValue::Unknown,
        }
    }

    // ---- integers ---------------------------------------------------------------------------

    /// Evaluates the integer value named `name` in `module`.
    pub fn integer_of_name(
        &self,
        module: ModuleId,
        qualifier: Option<&str>,
        name: &str,
    ) -> Option<i128> {
        let lookup = match qualifier {
            None => self.graph.lookup(module, name)?,
            Some(q) => self.graph.lookup_qualified(module, q, name).ok()?,
        };
        let Found::Symbol {
            module: target,
            index,
        } = lookup.found
        else {
            return None;
        };
        let key = (target, index);
        match self.integers.borrow().get(&key) {
            Some(Memo::Done(value)) => return *value,
            Some(Memo::InProgress) => return None,
            None => {}
        }
        self.integers.borrow_mut().insert(key, Memo::InProgress);
        let value = self.compute_symbol_integer(target, index);
        self.integers.borrow_mut().insert(key, Memo::Done(value));
        value
    }

    fn compute_symbol_integer(&self, module: ModuleId, index: usize) -> Option<i128> {
        let symbol = self.graph.symbol(module, index)?;
        if !self.graph.is_value_symbol(module, index) {
            return None;
        }
        let peeled = self.graph.peel_type(module, symbol.type_node.as_ref()?)?;
        if peeled.node.kind() != SyntaxKind::IntegerType {
            return None;
        }
        self.eval_integer(module, symbol.value_node.as_ref()?)
    }

    /// Evaluates an `IntegerValue` written as a number or a reference to an integer value (X.680 19.9).
    pub fn eval_integer(&self, module: ModuleId, value: &SyntaxNode) -> Option<i128> {
        let toks = tokens(value);
        match toks.as_slice() {
            [n] if n.kind() == SyntaxKind::Number => n.text().parse().ok(),
            [minus, n] if minus.kind() == SyntaxKind::Minus && n.kind() == SyntaxKind::Number => {
                let magnitude: i128 = n.text().parse().ok()?;
                magnitude.checked_neg()
            }
            [name] if name.kind() == SyntaxKind::LowerName => {
                self.integer_of_name(module, None, name.text())
            }
            [m, _, name] if m.kind() == SyntaxKind::UpperName => {
                self.integer_of_name(module, Some(m.text()), name.text())
            }
            _ => None,
        }
    }
}
