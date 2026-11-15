//! Constraint plans: each constraint of a type as a small language-neutral check program over its value, for the
//! runtime to interpret (Codegen and Decoder Design, Constraint enforcement).
//!
//! Like the decoder, generated code checks the root of a constraint that is not extensible; an extensible constraint
//! admits values outside its root, so it has nothing to reject. A part that cannot be evaluated makes the union it
//! is in permissive and drops out of an intersection, and the constraint is reported as not checked.

use glint_ir::{
    Bound, ComponentConstraint, Constraint, ElementSet, IrSet, Kind, NamedNumber,
    PresenceConstraint, Type, Value,
};

use crate::Capabilities;

/// One constraint of a type: its program and its text, for error messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub check: Check,
    pub text: String,
}

/// A check over one value.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    Pass,
    All(Vec<Check>),
    Any(Vec<Check>),
    Not(Box<Check>),
    /// The integer (a value, a size, a code point) is in one of the ranges; `None` is unbounded.
    Int(Vec<(Option<i128>, Option<i128>)>),
    /// The value equals the constant.
    Equals(Const),
    /// The size (characters, bits, octets, elements) passes.
    Size(Box<Check>),
    /// Every character's code point passes.
    Alphabet(Box<Check>),
    /// The string matches the regular expression (X.680 Annex A).
    Pattern(String),
    /// The component, when present, passes.
    Component {
        name: String,
        check: Box<Check>,
    },
    /// The component (or alternative) is present, or absent.
    Presence {
        name: String,
        present: bool,
    },
    /// Every element passes.
    Each(Box<Check>),
}

/// A value to compare with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Const {
    Bool(bool),
    Int(i128),
    Null,
    Oid(Vec<u64>),
    Str(String),
    Bits(Vec<bool>),
    Octets(Vec<u8>),
}

#[derive(Clone, Copy)]
enum Domain<'s> {
    /// Values of the governing type.
    Value(&'s Type),
    /// Sizes.
    Size,
    /// Characters of a permitted alphabet.
    Char,
}

struct Lowering<'s> {
    set: &'s IrSet,
    caps: &'s Capabilities,
    /// A part could not be evaluated.
    unknown: bool,
}

/// The rules of `ty`'s own constraints, and the text of each that is not (fully) checked.
pub(crate) fn lower(set: &IrSet, ty: &Type, caps: &Capabilities) -> (Vec<Rule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut unsupported = Vec::new();
    for constraint in &ty.constraints {
        let mut l = Lowering {
            set,
            caps,
            unknown: false,
        };
        let check = l.constraint(constraint, Domain::Value(ty));
        if l.unknown {
            unsupported.push(constraint.text.clone());
        }
        match check {
            Some(Check::Pass) | None => {}
            Some(check) => rules.push(Rule {
                check,
                text: constraint.text.clone(),
            }),
        }
    }
    (rules, unsupported)
}

fn bits_of_hex(hex: &str) -> Option<Vec<bool>> {
    let mut out = Vec::new();
    for c in hex.chars() {
        let d = c.to_digit(16)?;
        for shift in (0..4).rev() {
            out.push(d >> shift & 1 == 1);
        }
    }
    Some(out)
}

fn octets_of_hex(hex: &str) -> Option<Vec<u8>> {
    let digits: Vec<u32> = hex.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?;
    digits
        .chunks(2)
        .map(|pair| {
            let hi = *pair.first()?;
            let lo = pair.get(1).copied().unwrap_or(0);
            u8::try_from(hi << 4 | lo).ok()
        })
        .collect()
}

/// The bits a named-bit value sets.
pub(crate) fn named_bits(names: &[String], named: &[NamedNumber]) -> Option<Vec<bool>> {
    let mut positions = Vec::new();
    for name in names {
        let n = named.iter().find(|n| &n.name == name)?;
        positions.push(usize::try_from(n.value).ok()?);
    }
    let len = positions.iter().max().map_or(0, |m| m.saturating_add(1));
    let mut bits = vec![false; len];
    for p in positions {
        if let Some(b) = bits.get_mut(p) {
            *b = true;
        }
    }
    Some(bits)
}

/// The constant a value of `governing` is, when it is a simple one.
pub(crate) fn constant(set: &IrSet, governing: &Type, value: &Value) -> Option<Const> {
    let kind = &set.resolve(governing).kind;
    Some(match (value, kind) {
        (Value::Boolean(b), _) => Const::Bool(*b),
        (Value::Integer(n), _) => Const::Int(*n),
        (Value::Identifier(name), Kind::Integer { named }) => {
            Const::Int(named.iter().find(|n| &n.name == name)?.value)
        }
        (
            Value::Identifier(name),
            Kind::Enumerated {
                root, additions, ..
            },
        ) => Const::Int(
            root.iter()
                .chain(additions)
                .find(|n| &n.name == name)?
                .value,
        ),
        (Value::Null, _) => Const::Null,
        (Value::Oid(dotted), _) => Const::Oid(
            dotted
                .split('.')
                .map(|a| a.parse().ok())
                .collect::<Option<_>>()?,
        ),
        (Value::String(s), _) => Const::Str(s.clone()),
        (Value::Bits(names), Kind::BitString { named }) => Const::Bits(named_bits(names, named)?),
        (Value::Bstring(b), Kind::BitString { .. }) => {
            Const::Bits(b.chars().map(|c| c == '1').collect())
        }
        (Value::Hstring(h), Kind::BitString { .. }) => Const::Bits(bits_of_hex(h)?),
        (Value::Hstring(h), Kind::OctetString) => Const::Octets(octets_of_hex(h)?),
        (Value::Bstring(b), Kind::OctetString) => {
            let bits: Vec<bool> = b.chars().map(|c| c == '1').collect();
            Const::Octets(
                bits.chunks(8)
                    .map(|chunk| {
                        chunk.iter().enumerate().fold(
                            0u8,
                            |acc, (i, &bit)| if bit { acc | 0x80 >> i } else { acc },
                        )
                    })
                    .collect(),
            )
        }
        _ => return None,
    })
}

fn integer_kind(set: &IrSet, ty: &Type) -> bool {
    matches!(
        set.resolve(ty).kind,
        Kind::Integer { .. } | Kind::Enumerated { .. }
    )
}

impl<'s> Lowering<'s> {
    fn constraint(&mut self, constraint: &'s Constraint, domain: Domain<'s>) -> Option<Check> {
        if constraint.extensible {
            return Some(Check::Pass);
        }
        self.set_of(&constraint.root, domain)
    }

    fn number(&mut self, value: &Value, domain: Domain<'s>) -> Option<i128> {
        match (value, domain) {
            (Value::Integer(n), _) => Some(*n),
            (Value::Identifier(_), Domain::Value(ty)) => match constant(self.set, ty, value) {
                Some(Const::Int(n)) => Some(n),
                _ => None,
            },
            (Value::String(s), Domain::Char) => {
                let mut chars = s.chars();
                let c = chars.next()?;
                chars.next().is_none().then(|| i128::from(u32::from(c)))
            }
            _ => None,
        }
    }

    fn bound(&mut self, bound: &Bound, low: bool, domain: Domain<'s>) -> Option<Option<i128>> {
        match bound {
            Bound::Min | Bound::Max => Some(None),
            Bound::Inclusive(v) => self.number(v, domain).map(Some),
            Bound::Exclusive(v) => {
                let n = self.number(v, domain)?;
                Some(Some(if low {
                    n.checked_add(1)?
                } else {
                    n.checked_sub(1)?
                }))
            }
        }
    }

    /// The check of an element set; `None`, and the lowering marked, when it cannot be evaluated.
    fn set_of(&mut self, set: &'s ElementSet, domain: Domain<'s>) -> Option<Check> {
        let out = self.eval(set, domain);
        if out.is_none() {
            self.unknown = true;
        }
        out
    }

    fn eval(&mut self, set: &'s ElementSet, domain: Domain<'s>) -> Option<Check> {
        match set {
            ElementSet::All => Some(Check::Pass),
            ElementSet::Union(items) => {
                let mut checks = Vec::new();
                for item in items {
                    match self.set_of(item, domain) {
                        Some(Check::Pass) => return Some(Check::Pass),
                        Some(check) => checks.push(check),
                        // values matching what cannot be evaluated are permitted too
                        None => return None,
                    }
                }
                Some(match checks.len() {
                    1 => checks.remove(0),
                    _ => Check::Any(checks),
                })
            }
            ElementSet::Intersection(items) => {
                let checks: Vec<Check> = items
                    .iter()
                    .filter_map(|item| self.set_of(item, domain))
                    .filter(|c| *c != Check::Pass)
                    .collect();
                Some(match checks.len() {
                    0 => Check::Pass,
                    1 => checks.into_iter().next().unwrap_or(Check::Pass),
                    _ => Check::All(checks),
                })
            }
            ElementSet::Except(a, b) => {
                let a = self.set_of(a, domain)?;
                match self.set_of(b, domain) {
                    Some(b) => Some(Check::All(vec![a, Check::Not(Box::new(b))])),
                    None => Some(a),
                }
            }
            ElementSet::Value(v) => match domain {
                Domain::Value(ty) if integer_kind(self.set, ty) => self
                    .number(v, domain)
                    .map(|n| Check::Int(vec![(Some(n), Some(n))])),
                Domain::Value(ty) => constant(self.set, ty, v).map(Check::Equals),
                Domain::Size => self
                    .number(v, domain)
                    .map(|n| Check::Int(vec![(Some(n), Some(n))])),
                Domain::Char => match v {
                    Value::String(s) => Some(Check::Int(
                        s.chars()
                            .map(|c| {
                                let n = i128::from(u32::from(c));
                                (Some(n), Some(n))
                            })
                            .collect(),
                    )),
                    _ => None,
                },
            },
            ElementSet::Range { lo, hi } => {
                if let Domain::Value(ty) = domain
                    && !integer_kind(self.set, ty)
                {
                    return None;
                }
                let lo = self.bound(lo, true, domain)?;
                let hi = self.bound(hi, false, domain)?;
                Some(Check::Int(vec![(lo, hi)]))
            }
            ElementSet::Size(inner) => match self.constraint(inner, Domain::Size)? {
                Check::Pass => Some(Check::Pass),
                check => Some(Check::Size(Box::new(check))),
            },
            ElementSet::From(inner) => match self.constraint(inner, Domain::Char)? {
                Check::Pass => Some(Check::Pass),
                check => Some(Check::Alphabet(Box::new(check))),
            },
            ElementSet::Pattern(p) if self.caps.pattern => Some(Check::Pattern(p.clone())),
            ElementSet::Pattern(_) => None,
            ElementSet::ContainedSubtype(inner) => {
                let mut checks = Vec::new();
                let mut current: &Type = inner;
                for _ in 0..=self.set.types.len() {
                    for c in &current.constraints {
                        let governing = current;
                        match self.constraint(c, Domain::Value(governing)) {
                            Some(Check::Pass) => {}
                            Some(check) => checks.push(check),
                            None => return None,
                        }
                    }
                    match &current.kind {
                        Kind::Reference { target } => match self.set.get(*target) {
                            Some(def) => current = &def.ty,
                            None => break,
                        },
                        _ => break,
                    }
                }
                Some(match checks.len() {
                    0 => Check::Pass,
                    _ => Check::All(checks),
                })
            }
            ElementSet::WithComponent(inner) => {
                let Domain::Value(ty) = domain else {
                    return None;
                };
                match &self.set.resolve(ty).kind {
                    Kind::SequenceOf(c) | Kind::SetOf(c) => {
                        match self.constraint(inner, Domain::Value(&c.element))? {
                            Check::Pass => Some(Check::Pass),
                            check => Some(Check::Each(Box::new(check))),
                        }
                    }
                    _ => None,
                }
            }
            ElementSet::WithComponents { partial, items } => {
                let Domain::Value(ty) = domain else {
                    return None;
                };
                self.with_components(ty, *partial, items)
            }
            ElementSet::Table(table) => {
                let Domain::Value(ty) = domain else {
                    return None;
                };
                let objects = self.set.object_set(table.set)?;
                if objects.extensible || !table.relation.is_empty() {
                    return Some(Check::Pass);
                }
                if matches!(self.set.resolve(ty).kind, Kind::OpenType(_)) {
                    return Some(Check::Pass);
                }
                let keys: Vec<Check> = objects
                    .rows
                    .iter()
                    .filter_map(|row| row.key.as_ref())
                    .filter_map(|key| constant(self.set, ty, key))
                    .map(Check::Equals)
                    .collect();
                Some(Check::Any(keys))
            }
            ElementSet::Contents { .. } | ElementSet::Settings(_) => Some(Check::Pass),
            ElementSet::UserDefined(_) | ElementSet::Opaque(_) => None,
        }
    }

    fn with_components(
        &mut self,
        ty: &'s Type,
        partial: bool,
        items: &'s [ComponentConstraint],
    ) -> Option<Check> {
        let record = match &self.set.resolve(ty).kind {
            Kind::Sequence(r) | Kind::Set(r) | Kind::Choice(r) => r,
            _ => return None,
        };
        let choice = matches!(self.set.resolve(ty).kind, Kind::Choice(_));
        let mut checks = Vec::new();
        for item in items {
            let component = record.components.iter().find(|c| c.name == item.name)?;
            match item.presence {
                Some(PresenceConstraint::Present) => checks.push(Check::Presence {
                    name: item.name.clone(),
                    present: true,
                }),
                Some(PresenceConstraint::Absent) => checks.push(Check::Presence {
                    name: item.name.clone(),
                    present: false,
                }),
                Some(PresenceConstraint::Optional) | None => {}
            }
            if let Some(constraint) = &item.constraint {
                match self.constraint(constraint, Domain::Value(&component.ty)) {
                    Some(Check::Pass) => {}
                    Some(check) => checks.push(Check::Component {
                        name: item.name.clone(),
                        check: Box::new(check),
                    }),
                    None => {}
                }
            }
        }
        if !partial {
            // a full specification: what it does not list is absent (X.680 51.8.6)
            for c in &record.components {
                let listed = items.iter().any(|i| i.name == c.name);
                if !listed && (choice || !c.presence.is_required()) {
                    checks.push(Check::Presence {
                        name: c.name.clone(),
                        present: false,
                    });
                }
            }
        }
        Some(match checks.len() {
            0 => Check::Pass,
            1 => checks.remove(0),
            _ => Check::All(checks),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use glint_ir::{IR_VERSION, Summary};

    use super::*;

    fn caps() -> Capabilities {
        Capabilities {
            element_tags: false,
            max_use_tags: 1,
            tagged_records: true,
            pattern: false,
            real: false,
        }
    }

    fn empty() -> IrSet {
        IrSet {
            version: IR_VERSION,
            root: glint_ir::TypeId(0),
            types: Vec::new(),
            object_sets: Vec::new(),
            oids: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn int(v: i128) -> Bound {
        Bound::Inclusive(Value::Integer(v))
    }

    #[test]
    fn sizes_and_ranges() {
        let mut ty = Type::of(Kind::Integer { named: Vec::new() });
        ty.constraints.push(Constraint::of(
            ElementSet::Range {
                lo: int(0),
                hi: Bound::Max,
            },
            "(0..MAX)",
        ));
        let (rules, unsupported) = lower(&empty(), &ty, &caps());
        assert!(unsupported.is_empty());
        assert_eq!(rules[0].check, Check::Int(vec![(Some(0), None)]));

        let mut ty = Type::of(Kind::OctetString);
        ty.constraints.push(Constraint::of(
            ElementSet::Size(Box::new(Constraint::of(
                ElementSet::Range {
                    lo: int(1),
                    hi: int(4),
                },
                "",
            ))),
            "(SIZE (1..4))",
        ));
        let (rules, _) = lower(&empty(), &ty, &caps());
        assert_eq!(
            rules[0].check,
            Check::Size(Box::new(Check::Int(vec![(Some(1), Some(4))])))
        );
    }

    #[test]
    fn extensible_and_unknown_constraints() {
        let mut ty = Type::of(Kind::Integer { named: Vec::new() });
        let mut c = Constraint::of(
            ElementSet::Range {
                lo: int(0),
                hi: int(9),
            },
            "(0..9, ...)",
        );
        c.extensible = true;
        ty.constraints.push(c);
        ty.constraints.push(Constraint::of(
            ElementSet::Union(vec![
                ElementSet::Range {
                    lo: int(0),
                    hi: int(9),
                },
                ElementSet::UserDefined("CONSTRAINED BY {}".into()),
            ]),
            "(0..9 | CONSTRAINED BY {})",
        ));
        let (rules, unsupported) = lower(&empty(), &ty, &caps());
        assert!(rules.is_empty());
        assert_eq!(unsupported, ["(0..9 | CONSTRAINED BY {})"]);
        let _ = Summary::default();
    }
}
