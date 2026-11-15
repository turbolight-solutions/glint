//! `DEFAULT` values as typed literals: the IR's value, matched against the plan's types so a backend only prints it.

use glint_ir::{Kind, Value};

use super::check::{Const, constant, named_bits};
use super::{Body, Builder, DispatchId, Leaf, PTypeId, Target, Use, key_of};

/// A value of a plan type.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Bool(bool),
    Int(i128),
    Null,
    Oid(Vec<u64>),
    Bits(Vec<bool>),
    Octets(Vec<u8>),
    Str(String),
    /// An enumeration item, by index into the root items then the additions.
    Enum {
        ty: PTypeId,
        index: usize,
    },
    /// A record value, by field: `None` leaves an `OPTIONAL` component out or gives a `DEFAULT` one its default.
    Record {
        ty: PTypeId,
        fields: Vec<Option<Literal>>,
    },
    Choice {
        ty: PTypeId,
        alternative: usize,
        value: Box<Literal>,
    },
    /// The value of an alias type.
    Newtype {
        ty: PTypeId,
        value: Box<Literal>,
    },
    /// The value of an open type, as the variant its key selects.
    Variant {
        dispatch: DispatchId,
        variant: usize,
        value: Box<Literal>,
    },
}

/// The value of a field of an object of one of the set's tables, written `object.&field` (X.681 14.6), which
/// lowering keeps as written.
fn object_field(b: &Builder<'_>, text: &str) -> Option<Value> {
    let (object, field) = text.split_once('.')?;
    let (object, field) = (object.trim(), field.trim());
    b.set.object_sets.iter().find_map(|table| {
        let row = table
            .rows
            .iter()
            .find(|r| r.object.as_deref() == Some(object))?;
        if table.key_field.as_deref() == Some(field) {
            row.key.clone()
        } else {
            row.values.get(field).cloned()
        }
    })
}

/// `value`, with a reference to a field of an object resolved.
fn resolved<'v>(b: &Builder<'_>, value: &'v Value) -> std::borrow::Cow<'v, Value> {
    match value {
        Value::Raw(text) => {
            object_field(b, text).map_or(std::borrow::Cow::Borrowed(value), std::borrow::Cow::Owned)
        }
        _ => std::borrow::Cow::Borrowed(value),
    }
}

/// The literal of `value` as a value of `use_`, when the target can write it.
pub(crate) fn build(b: &Builder<'_>, use_: &Use, value: &Value) -> Option<Literal> {
    let value = &*resolved(b, value);
    match &use_.target {
        Target::Leaf(leaf) => leaf_literal(b, leaf, value),
        Target::Named(id) => named(b, *id, value),
        _ => None,
    }
}

fn leaf_literal(b: &Builder<'_>, leaf: &Leaf, value: &Value) -> Option<Literal> {
    let kind = match leaf {
        Leaf::Boolean => Kind::Boolean,
        Leaf::Integer { named } => Kind::Integer {
            named: named.clone(),
        },
        Leaf::Null => Kind::Null,
        Leaf::BitString { named } => Kind::BitString {
            named: named.clone(),
        },
        Leaf::OctetString => Kind::OctetString,
        Leaf::ObjectIdentifier => Kind::ObjectIdentifier,
        Leaf::String(string) => Kind::String { string: *string },
        Leaf::Real | Leaf::RelativeOid | Leaf::Time(_) => return None,
    };
    let governing = glint_ir::Type::of(kind);
    if let (Value::Bits(names), Leaf::BitString { named }) = (value, leaf) {
        return named_bits(names, named).map(Literal::Bits);
    }
    Some(match constant(b.set, &governing, value)? {
        Const::Bool(v) => Literal::Bool(v),
        Const::Int(v) => Literal::Int(v),
        Const::Null => Literal::Null,
        Const::Oid(arcs) => Literal::Oid(arcs),
        Const::Str(s) => Literal::Str(s),
        Const::Bits(bits) => Literal::Bits(bits),
        Const::Octets(octets) => Literal::Octets(octets),
    })
}

fn named(b: &Builder<'_>, id: PTypeId, value: &Value) -> Option<Literal> {
    let t = b.types.get(id.0)?;
    match &t.body {
        Body::Alias(inner) => Some(Literal::Newtype {
            ty: id,
            value: Box::new(build(b, inner, value)?),
        }),
        Body::Enumerated {
            items, additions, ..
        } => {
            let index = items.iter().chain(additions).position(|item| match value {
                Value::Identifier(name) => &item.asn1 == name,
                Value::Integer(n) => item.value == *n,
                _ => false,
            })?;
            Some(Literal::Enum { ty: id, index })
        }
        Body::Choice { alternatives, .. } => {
            let Value::Record(pairs) = value else {
                return None;
            };
            let [(name, inner)] = pairs.as_slice() else {
                return None;
            };
            let alternative = alternatives.iter().position(|a| &a.asn1 == name)?;
            let use_ = &alternatives.get(alternative)?.use_;
            Some(Literal::Choice {
                ty: id,
                alternative,
                value: Box::new(build(b, use_, inner)?),
            })
        }
        Body::Record { fields, .. } => {
            let Value::Record(pairs) = value else {
                return None;
            };
            let mut out = Vec::new();
            for field in fields {
                let Some((_, v)) = pairs.iter().find(|(n, _)| n == &field.asn1) else {
                    if matches!(field.presence, super::FieldPresence::Required)
                        && !field.declared_default
                    {
                        return None;
                    }
                    out.push(None);
                    continue;
                };
                let literal = match &field.use_.target {
                    Target::Dispatch(dispatch) => variant(b, id, &field.asn1, *dispatch, pairs, v)?,
                    _ => build(b, &field.use_, v)?,
                };
                out.push(Some(literal));
            }
            Some(Literal::Record {
                ty: id,
                fields: out,
            })
        }
    }
}

/// The variant of an open-type component, chosen by the key the record value gives its relation.
fn variant(
    b: &Builder<'_>,
    owner: PTypeId,
    component: &str,
    dispatch: DispatchId,
    pairs: &[(String, Value)],
    value: &Value,
) -> Option<Literal> {
    let t = b.types.get(owner.0)?;
    let resolution = t.resolutions.iter().find(|r| {
        r.dispatch == dispatch
            && r.open.len() == 1
            && r.open.first().is_some_and(|o| o == component)
    })?;
    let [key_name] = resolution.key.as_slice() else {
        return None;
    };
    let (_, key_value) = pairs.iter().find(|(n, _)| n == key_name)?;
    let key = key_of(&resolved(b, key_value))?;
    let d = b.dispatches.get(dispatch.0)?;
    let index = d.variants.iter().position(|v| v.key == key)?;
    let use_ = &d.variants.get(index)?.use_;
    Some(Literal::Variant {
        dispatch,
        variant: index,
        value: Box::new(build(b, use_, value)?),
    })
}
