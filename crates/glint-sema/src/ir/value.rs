//! Values in evaluated form: `DEFAULT`s, constraint values, table keys and value parameters.

use glint_ir::{Kind, OidConstant, Type, Value};
use glint_syntax::{PrefixKind, SyntaxKind, SyntaxNode};

use super::constraint::unquote;
use super::{Actual, Env, Hint, Lowerer};
use crate::graph::{Found, ModuleId};
use crate::sem::toks::{flatten, inner, span, split_commas};
use crate::view::child;

/// How many value references may be followed in a row.
const MAX_VALUE_DEPTH: usize = 8;

impl Lowerer<'_, '_, '_> {
    /// Evaluates a value node (a `Value`, or a `BraceGroup` written as a value) in `env`, reading it as `hint`
    /// directs.
    pub(super) fn value(&mut self, node: &SyntaxNode, env: &Env, hint: &Hint) -> Value {
        let group = if node.kind() == SyntaxKind::BraceGroup {
            Some(node.clone())
        } else {
            child(node, SyntaxKind::BraceGroup)
        };
        if let Some(group) = group {
            return self.brace_value(&group, env, hint);
        }
        let toks = flatten(node);
        let raw = || {
            toks.iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };
        match toks.as_slice() {
            [n] if n.kind == SyntaxKind::Number => n
                .text
                .parse()
                .map_or_else(|_| Value::Raw(raw()), Value::Integer),
            [minus, n] if minus.kind == SyntaxKind::Minus && n.kind == SyntaxKind::Number => n
                .text
                .parse::<i128>()
                .ok()
                .and_then(i128::checked_neg)
                .map_or_else(|| Value::Raw(raw()), Value::Integer),
            [n] if n.kind == SyntaxKind::RealNumber => Value::Real(n.text.clone()),
            [only] if only.text == "TRUE" => Value::Boolean(true),
            [only] if only.text == "FALSE" => Value::Boolean(false),
            [only] if only.text == "NULL" => Value::Null,
            [s] if s.kind == SyntaxKind::CString => Value::String(unquote(&s.text)),
            [s] if s.kind == SyntaxKind::QuotedString => quoted(&s.text),
            [name] if name.kind == SyntaxKind::LowerName => {
                self.named_value(env, None, &name.text, hint)
            }
            [module, dot, name]
                if module.kind == SyntaxKind::UpperName
                    && dot.kind == SyntaxKind::Dot
                    && name.kind == SyntaxKind::LowerName =>
            {
                self.named_value(env, Some(&module.text), &name.text, hint)
            }
            _ => Value::Raw(raw()),
        }
    }

    fn named_value(
        &mut self,
        env: &Env,
        qualifier: Option<&str>,
        name: &str,
        hint: &Hint,
    ) -> Value {
        if qualifier.is_none()
            && let Some(actual) = env.actual(name)
        {
            return match actual {
                Actual::Value(value) => value.clone(),
                _ => Value::Identifier(name.to_owned()),
            };
        }
        match hint {
            Hint::Integer(named) => {
                if let Some(n) = named.iter().find(|n| n.name == name) {
                    return Value::Integer(n.value);
                }
            }
            Hint::Enumerated | Hint::Bits => return Value::Identifier(name.to_owned()),
            _ => {}
        }
        if let Some(oid) = self.eval.oid_of_name(env.module, qualifier, name) {
            let dotted = oid.to_string();
            self.record_oid(env.module, qualifier, name, &dotted);
            return Value::Oid(dotted);
        }
        if let Some(n) = self.eval.integer_of_name(env.module, qualifier, name) {
            return Value::Integer(n);
        }
        if let Some(value) = self.defined_value(env, qualifier, name, hint)
            && !matches!(value, Value::Raw(_))
        {
            return value;
        }
        Value::Identifier(name.to_owned())
    }

    /// The value of the value assignment `name`, read in its own module.
    fn defined_value(
        &mut self,
        env: &Env,
        qualifier: Option<&str>,
        name: &str,
        hint: &Hint,
    ) -> Option<Value> {
        let lookup = match qualifier {
            None => self.graph.lookup(env.module, name)?,
            Some(q) => self.graph.lookup_qualified(env.module, q, name).ok()?,
        };
        let Found::Symbol { module, index } = lookup.found else {
            return None;
        };
        let is_value =
            self.sem.kind(crate::sem::SymRef { module, index }) == crate::sem::Kind::Value;
        if !is_value || self.values_open > MAX_VALUE_DEPTH {
            return None;
        }
        let node = self.graph.symbol(module, index)?.value_node.clone()?;
        self.values_open = self.values_open.saturating_add(1);
        let value = self.value(&node, &Env::new(module), hint);
        self.values_open = self.values_open.saturating_sub(1);
        Some(value)
    }

    // X.680 25.18: `{ name value, ... }`
    fn record_value(&mut self, group: &SyntaxNode, env: &Env, record: &Type) -> Option<Value> {
        let components = match &record.kind {
            Kind::Sequence(r) | Kind::Set(r) => &r.components,
            _ => return None,
        };
        let toks = inner(group);
        let mut fields = Vec::new();
        for item in split_commas(&toks) {
            let (first, rest) = item.split_first()?;
            if first.kind != SyntaxKind::LowerName {
                return None;
            }
            let mut hint = components
                .iter()
                .find(|c| c.name == first.text)
                .map_or(Hint::None, |c| self.hint(&c.ty));
            // an open type value `Type : value` is read as a value of that type
            let rest = match rest.iter().position(|t| t.kind == SyntaxKind::Colon) {
                Some(colon) => {
                    let written = rest.get(..colon).unwrap_or_default();
                    if let Some(node) =
                        span(written).and_then(|r| self.parse_at(env.module, r, PrefixKind::Type))
                    {
                        let ty = self.ty(&node, env, 0);
                        hint = self.hint(&ty);
                    }
                    rest.get(colon.saturating_add(1)..).unwrap_or_default()
                }
                None => rest,
            };
            let range = span(rest)?;
            let value = match self.parse_at(env.module, range, PrefixKind::Value) {
                Some(node) => self.value(&node, env, &hint),
                None => Value::Raw(
                    rest.iter()
                        .map(|t| t.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            };
            fields.push((first.text.clone(), value));
        }
        Some(Value::Record(fields))
    }

    /// Notes the OID constant `name`, under the module that defines it.
    fn record_oid(&mut self, module: ModuleId, qualifier: Option<&str>, name: &str, dotted: &str) {
        let lookup = match qualifier {
            None => self.graph.lookup(module, name),
            Some(q) => self.graph.lookup_qualified(module, q, name).ok(),
        };
        let defining = match lookup.map(|l| l.found) {
            Some(Found::Symbol { module, .. }) => module,
            _ => module,
        };
        let module_name = self.module_name(defining);
        self.oids
            .entry((module_name.clone(), name.to_owned()))
            .or_insert_with(|| OidConstant {
                name: name.to_owned(),
                module: module_name,
                oid: dotted.to_owned(),
            });
    }

    // X.680 22.16 bit string values by name; 32.3 object identifier values
    fn brace_value(&mut self, group: &SyntaxNode, env: &Env, hint: &Hint) -> Value {
        if let Hint::Record(record) = hint
            && let Some(value) = self.record_value(group, env, record)
        {
            return value;
        }
        let toks = inner(group);
        let names_only = toks
            .iter()
            .all(|t| matches!(t.kind, SyntaxKind::LowerName | SyntaxKind::Comma));
        if names_only
            && (matches!(hint, Hint::Bits) || toks.iter().any(|t| t.kind == SyntaxKind::Comma))
        {
            return Value::Bits(
                toks.iter()
                    .filter(|t| t.kind == SyntaxKind::LowerName)
                    .map(|t| t.text.clone())
                    .collect(),
            );
        }
        if !matches!(hint, Hint::Integer(_) | Hint::String | Hint::Enumerated)
            && let Some(oid) = self.eval.eval_oid_group(env.module, group).oid
        {
            return Value::Oid(oid.to_string());
        }
        Value::Raw(
            flatten(group)
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

/// `'0101'B` or `'0A'H`.
fn quoted(text: &str) -> Value {
    let body = text.strip_prefix('\'').unwrap_or(text);
    if let Some(bits) = body.strip_suffix("'B") {
        return Value::Bstring(bits.chars().filter(|c| !c.is_whitespace()).collect());
    }
    if let Some(hex) = body.strip_suffix("'H") {
        return Value::Hstring(hex.chars().filter(|c| !c.is_whitespace()).collect());
    }
    Value::Raw(text.to_owned())
}
