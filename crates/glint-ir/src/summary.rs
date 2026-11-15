//! Effective constraint summaries: the set arithmetic of X.680 50 on integer values, sizes and permitted alphabets,
//! so backends can pick native types without evaluating constraints themselves.

use crate::{
    Alphabet, Bound, Constraint, ElementSet, IrSet, Kind, Range, StringKind, Summary, Type, TypeId,
    Value,
};

/// An interval of integers; `None` is unbounded on that side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub lo: Option<i128>,
    pub hi: Option<i128>,
}

/// A set of integers as sorted, disjoint, non-adjacent intervals.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Intervals(Vec<Interval>);

fn lo_le(a: Option<i128>, b: Option<i128>) -> bool {
    match (a, b) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => a <= b,
    }
}

fn hi_le(a: Option<i128>, b: Option<i128>) -> bool {
    match (a, b) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(a), Some(b)) => a <= b,
    }
}

impl Intervals {
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn full() -> Self {
        Self(vec![Interval { lo: None, hi: None }])
    }

    pub fn point(v: i128) -> Self {
        Self(vec![Interval {
            lo: Some(v),
            hi: Some(v),
        }])
    }

    pub fn range(lo: Option<i128>, hi: Option<i128>) -> Self {
        if let (Some(l), Some(h)) = (lo, hi)
            && l > h
        {
            return Self::empty();
        }
        Self(vec![Interval { lo, hi }])
    }

    pub fn intervals(&self) -> &[Interval] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn normalize(mut items: Vec<Interval>) -> Self {
        items.sort_by(|a, b| match (a.lo, b.lo) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => x.cmp(&y),
        });
        let mut out: Vec<Interval> = Vec::new();
        for item in items {
            if let Some(last) = out.last_mut() {
                // merge when overlapping or adjacent
                let touches = match (last.hi, item.lo) {
                    (None, _) | (_, None) => true,
                    (Some(h), Some(l)) => l <= h.saturating_add(1),
                };
                if touches {
                    if !hi_le(item.hi, last.hi) {
                        last.hi = item.hi;
                    }
                    continue;
                }
            }
            out.push(item);
        }
        Self(out)
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut all = self.0.clone();
        all.extend(other.0.iter().copied());
        Self::normalize(all)
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let mut out = Vec::new();
        for a in &self.0 {
            for b in &other.0 {
                let lo = if lo_le(a.lo, b.lo) { b.lo } else { a.lo };
                let hi = if hi_le(a.hi, b.hi) { a.hi } else { b.hi };
                let nonempty = match (lo, hi) {
                    (Some(l), Some(h)) => l <= h,
                    _ => true,
                };
                if nonempty {
                    out.push(Interval { lo, hi });
                }
            }
        }
        Self::normalize(out)
    }

    /// The integers of `self` that are not in `other`.
    pub fn minus(&self, other: &Self) -> Self {
        // complement of other, then intersect
        let mut complement = Vec::new();
        let mut start: Option<Option<i128>> = Some(None);
        for b in &other.0 {
            if let Some(from) = start {
                let to = b.lo.and_then(|l| l.checked_sub(1));
                let skip = b.lo.is_none() || matches!((from, to), (Some(f), Some(t)) if f > t);
                if !skip {
                    complement.push(Interval { lo: from, hi: to });
                }
            }
            start = b.hi.map(|h| h.checked_add(1));
        }
        if let Some(from) = start {
            complement.push(Interval { lo: from, hi: None });
        }
        self.intersect(&Self(complement))
    }

    /// The smallest interval holding every member, `None` when the set is empty.
    pub fn hull(&self) -> Option<Interval> {
        let first = self.0.first()?;
        let last = self.0.last()?;
        Some(Interval {
            lo: first.lo,
            hi: last.hi,
        })
    }
}

// ---- evaluation of element sets -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Domain {
    Value,
    Size,
    Alphabet,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Effective,
    PerVisible,
}

fn integer_of(value: &Value) -> Option<i128> {
    match value {
        Value::Integer(n) => Some(*n),
        _ => None,
    }
}

fn single_char(value: &Value) -> Option<i128> {
    match value {
        Value::String(s) => {
            let mut chars = s.chars();
            let c = chars.next()?;
            chars.next().is_none().then_some(i128::from(u32::from(c)))
        }
        _ => None,
    }
}

fn bound(b: &Bound, low: bool, read: fn(&Value) -> Option<i128>) -> Option<Option<i128>> {
    match b {
        Bound::Min | Bound::Max => Some(None),
        Bound::Inclusive(v) => read(v).map(Some),
        Bound::Exclusive(v) => {
            let n = read(v)?;
            Some(if low {
                n.checked_add(1)
            } else {
                n.checked_sub(1)
            })
        }
    }
}

/// The members of `set` in `domain`, or `None` when the set does not restrict that domain (or cannot be evaluated).
fn eval(set: &ElementSet, domain: Domain, mode: Mode) -> Option<Intervals> {
    match set {
        ElementSet::All => Some(Intervals::full()),
        ElementSet::Union(items) => {
            let mut out = Intervals::empty();
            for item in items {
                out = out.union(&eval(item, domain, mode)?);
            }
            Some(out)
        }
        ElementSet::Intersection(items) => {
            let mut out: Option<Intervals> = None;
            for item in items {
                if let Some(members) = eval(item, domain, mode) {
                    out = Some(match out {
                        Some(acc) => acc.intersect(&members),
                        None => members,
                    });
                }
            }
            out
        }
        ElementSet::Except(a, b) => {
            let left = eval(a, domain, mode);
            if mode == Mode::PerVisible {
                return left;
            }
            match (left, eval(b, domain, mode)) {
                (Some(l), Some(r)) => Some(l.minus(&r)),
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(Intervals::full().minus(&r)),
                (None, None) => None,
            }
        }
        ElementSet::Value(v) => match domain {
            Domain::Value => integer_of(v).map(Intervals::point),
            Domain::Alphabet => match v {
                Value::String(s) => Some(s.chars().fold(Intervals::empty(), |acc, c| {
                    acc.union(&Intervals::point(i128::from(u32::from(c))))
                })),
                _ => None,
            },
            Domain::Size => None,
        },
        ElementSet::Range { lo, hi } => {
            let read = match domain {
                Domain::Value => integer_of,
                Domain::Alphabet => single_char,
                Domain::Size => return None,
            };
            Some(Intervals::range(
                bound(lo, true, read)?,
                bound(hi, false, read)?,
            ))
        }
        ElementSet::Size(inner) if domain == Domain::Size => root_of(inner, Domain::Value, mode),
        ElementSet::From(inner) if domain == Domain::Alphabet => {
            if mode == Mode::PerVisible && inner.extensible {
                return None;
            }
            root_of(inner, Domain::Alphabet, mode)
        }
        _ => None,
    }
}

fn root_of(constraint: &Constraint, domain: Domain, mode: Mode) -> Option<Intervals> {
    eval(&constraint.root, domain, mode)
}

// ---- summaries over a set -------------------------------------------------------------------

/// Which domains a kind is constrained in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Integer,
    /// A character string type with a fixed number of bits per character (X.691 3.6.16).
    KnownMultiplier,
    OtherString,
    /// Bit, octet strings and collections: sizes only.
    Sized,
    None,
}

fn class_of_kind(kind: &Kind) -> Option<Class> {
    Some(match kind {
        Kind::Integer { .. } => Class::Integer,
        Kind::String { string } => match string {
            StringKind::NumericString
            | StringKind::PrintableString
            | StringKind::VisibleString
            | StringKind::IA5String
            | StringKind::BMPString
            | StringKind::UniversalString => Class::KnownMultiplier,
            _ => Class::OtherString,
        },
        Kind::BitString { .. } | Kind::OctetString | Kind::SequenceOf(_) | Kind::SetOf(_) => {
            Class::Sized
        }
        Kind::Reference { .. } => return None,
        _ => Class::None,
    })
}

struct Ctx {
    classes: Vec<Class>,
    defs: Vec<Option<Summary>>,
}

fn domains(class: Class, mode: Mode) -> &'static [Domain] {
    match (class, mode) {
        (Class::Integer, _) => &[Domain::Value],
        (Class::KnownMultiplier, _) | (Class::OtherString, Mode::Effective) => {
            &[Domain::Size, Domain::Alphabet]
        }
        (Class::Sized, _) => &[Domain::Size],
        (Class::OtherString, Mode::PerVisible) | (Class::None, _) => &[],
    }
}

fn to_range(members: &Intervals, extensible: bool) -> Range {
    let hull = members.hull().unwrap_or(Interval {
        lo: Some(0),
        hi: Some(-1),
    });
    Range {
        lo: hull.lo,
        hi: hull.hi,
        extensible,
    }
}

fn to_alphabet(members: &Intervals, extensible: bool) -> Alphabet {
    Alphabet {
        ranges: members
            .intervals()
            .iter()
            .map(|i| {
                let lo = i.lo.and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
                let hi =
                    i.hi.and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(0x0010_FFFF);
                (lo, hi)
            })
            .collect(),
        extensible,
    }
}

fn from_range(range: &Range) -> Intervals {
    Intervals::range(range.lo, range.hi)
}

fn from_alphabet(alphabet: &Alphabet) -> Intervals {
    alphabet
        .ranges
        .iter()
        .fold(Intervals::empty(), |acc, (lo, hi)| {
            acc.union(&Intervals::range(
                Some(i128::from(*lo)),
                Some(i128::from(*hi)),
            ))
        })
}

/// Serial application (X.680 49.7): each constraint narrows what the previous ones permit; the extensibility of the
/// result is that of the last constraint that restricts the domain.
fn fold(base: Option<&Summary>, constraints: &[Constraint], class: Class, mode: Mode) -> Summary {
    let mut out = Summary::default();
    for &domain in domains(class, mode) {
        let mut acc: Option<(Intervals, bool)> = base.and_then(|b| match domain {
            Domain::Value => b.value.as_ref().map(|r| (from_range(r), r.extensible)),
            Domain::Size => b.size.as_ref().map(|r| (from_range(r), r.extensible)),
            Domain::Alphabet => b
                .alphabet
                .as_ref()
                .map(|a| (from_alphabet(a), a.extensible)),
        });
        for constraint in constraints {
            let Some(members) = root_of(constraint, domain, mode) else {
                continue;
            };
            acc = Some(match acc {
                Some((prev, _)) => (prev.intersect(&members), constraint.extensible),
                None => (members, constraint.extensible),
            });
        }
        if let Some((members, extensible)) = acc {
            match domain {
                Domain::Value => out.value = Some(to_range(&members, extensible)),
                Domain::Size => out.size = Some(to_range(&members, extensible)),
                Domain::Alphabet => out.alphabet = Some(to_alphabet(&members, extensible)),
            }
        }
    }
    out
}

impl Ctx {
    fn class(&self, ty: &Type) -> Class {
        match &ty.kind {
            Kind::Reference { target } => usize::try_from(target.0)
                .ok()
                .and_then(|i| self.classes.get(i).copied())
                .unwrap_or(Class::None),
            kind => class_of_kind(kind).unwrap_or(Class::None),
        }
    }

    fn base(&self, ty: &Type) -> Option<Summary> {
        match &ty.kind {
            Kind::Reference { target } => usize::try_from(target.0)
                .ok()
                .and_then(|i| self.defs.get(i))
                .and_then(Clone::clone),
            _ => None,
        }
    }

    fn summary(&self, ty: &Type) -> Summary {
        let class = self.class(ty);
        let base = self.base(ty);
        let mut effective = fold(base.as_ref(), &ty.constraints, class, Mode::Effective);
        let per_base = base.as_ref().map(|b| {
            b.per_visible
                .as_deref()
                .cloned()
                .unwrap_or_else(|| b.clone())
        });
        let mut per = fold(per_base.as_ref(), &ty.constraints, class, Mode::PerVisible);
        per.per_visible = None;
        let mut plain = effective.clone();
        plain.per_visible = None;
        if per != plain {
            effective.per_visible = Some(Box::new(per));
        }
        effective
    }

    fn apply(&self, ty: &mut Type) {
        ty.summary = self.summary(ty);
        match &mut ty.kind {
            Kind::Sequence(record) | Kind::Set(record) | Kind::Choice(record) => {
                for component in &mut record.components {
                    self.apply(&mut component.ty);
                }
            }
            Kind::SequenceOf(collection) | Kind::SetOf(collection) => {
                self.apply(&mut collection.element);
            }
            _ => {}
        }
        for constraint in &mut ty.constraints {
            self.apply_constraint(constraint);
        }
    }

    fn apply_constraint(&self, constraint: &mut Constraint) {
        self.apply_set(&mut constraint.root);
        if let Some(additions) = &mut constraint.additions {
            self.apply_set(additions);
        }
    }

    fn apply_set(&self, set: &mut ElementSet) {
        match set {
            ElementSet::Union(items) | ElementSet::Intersection(items) => {
                for item in items {
                    self.apply_set(item);
                }
            }
            ElementSet::Except(a, b) => {
                self.apply_set(a);
                self.apply_set(b);
            }
            ElementSet::Size(c) | ElementSet::From(c) | ElementSet::WithComponent(c) => {
                self.apply_constraint(c);
            }
            ElementSet::WithComponents { items, .. } => {
                for item in items {
                    if let Some(c) = &mut item.constraint {
                        self.apply_constraint(c);
                    }
                }
            }
            ElementSet::ContainedSubtype(ty) => self.apply(ty),
            ElementSet::Contents { ty: Some(ty), .. } => self.apply(ty),
            _ => {}
        }
    }
}

fn resolve_classes(set: &IrSet) -> Vec<Class> {
    let mut out = vec![Class::None; set.types.len()];
    for (index, slot) in out.iter_mut().enumerate() {
        let mut current = set.types.get(index).map(|d| &d.ty);
        for _ in 0..set.types.len().saturating_add(1) {
            let Some(ty) = current else { break };
            match class_of_kind(&ty.kind) {
                Some(class) => {
                    *slot = class;
                    break;
                }
                None => {
                    current = match &ty.kind {
                        Kind::Reference { target } => set.get(*target).map(|d| &d.ty),
                        _ => None,
                    };
                }
            }
        }
    }
    out
}

fn def_summary(
    set: &IrSet,
    ctx: &mut Ctx,
    id: TypeId,
    visiting: &mut Vec<bool>,
) -> Option<Summary> {
    let index = usize::try_from(id.0).ok()?;
    if let Some(Some(done)) = ctx.defs.get(index) {
        return Some(done.clone());
    }
    if visiting.get(index).copied().unwrap_or(true) {
        return None;
    }
    if let Some(flag) = visiting.get_mut(index) {
        *flag = true;
    }
    let ty = &set.get(id)?.ty;
    if let Kind::Reference { target } = &ty.kind {
        def_summary(set, ctx, *target, visiting);
    }
    let summary = ctx.summary(ty);
    if let Some(slot) = ctx.defs.get_mut(index) {
        *slot = Some(summary.clone());
    }
    Some(summary)
}

/// Computes the [`Summary`] of every type of `set`, nested ones included.
pub fn summarize(set: &mut IrSet) {
    let mut ctx = Ctx {
        classes: resolve_classes(set),
        defs: vec![None; set.types.len()],
    };
    let mut visiting = vec![false; set.types.len()];
    for index in 0..set.types.len() {
        if let Ok(id) = u32::try_from(index) {
            def_summary(set, &mut ctx, TypeId(id), &mut visiting);
        }
    }
    for def in &mut set.types {
        ctx.apply(&mut def.ty);
    }
    for table in &mut set.object_sets {
        for row in &mut table.rows {
            for ty in row.types.values_mut() {
                ctx.apply(ty);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn r(lo: i128, hi: i128) -> Intervals {
        Intervals::range(Some(lo), Some(hi))
    }

    #[test]
    fn union_merges_adjacent_and_overlapping() {
        let u = r(1, 3).union(&r(4, 6)).union(&r(10, 12)).union(&r(11, 20));
        assert_eq!(
            u,
            Intervals(vec![
                Interval {
                    lo: Some(1),
                    hi: Some(6)
                },
                Interval {
                    lo: Some(10),
                    hi: Some(20)
                },
            ])
        );
    }

    #[test]
    fn intersection_and_difference() {
        assert_eq!(r(1, 10).intersect(&r(5, 20)), r(5, 10));
        assert!(r(1, 2).intersect(&r(3, 4)).is_empty());
        assert_eq!(r(1, 10).minus(&r(4, 6)), r(1, 3).union(&r(7, 10)));
        assert_eq!(
            Intervals::full().minus(&Intervals::point(0)).hull(),
            Some(Interval { lo: None, hi: None })
        );
        assert_eq!(r(1, 10).minus(&Intervals::range(None, Some(5))), r(6, 10));
    }

    fn v(n: i128) -> Bound {
        Bound::Inclusive(Value::Integer(n))
    }

    #[test]
    fn except_and_exclusive_bounds() {
        let set = ElementSet::Except(
            Box::new(ElementSet::Range {
                lo: v(0),
                hi: Bound::Exclusive(Value::Integer(100)),
            }),
            Box::new(ElementSet::Range {
                lo: v(50),
                hi: Bound::Max,
            }),
        );
        let members = eval(&set, Domain::Value, Mode::Effective).unwrap();
        assert_eq!(members, r(0, 49));
        // PER ignores the EXCEPT part
        let per = eval(&set, Domain::Value, Mode::PerVisible).unwrap();
        assert_eq!(per, r(0, 99));
    }

    #[test]
    fn serial_constraints_narrow_and_keep_the_last_extensibility() {
        let mut first = Constraint::of(
            ElementSet::Range {
                lo: v(0),
                hi: v(255),
            },
            "(0..255)",
        );
        first.extensible = true;
        let second = Constraint::of(
            ElementSet::Range {
                lo: v(1),
                hi: Bound::Max,
            },
            "(1..MAX)",
        );
        let summary = fold(None, &[first, second], Class::Integer, Mode::Effective);
        assert_eq!(
            summary.value,
            Some(Range {
                lo: Some(1),
                hi: Some(255),
                extensible: false
            })
        );
    }

    #[test]
    fn size_and_alphabet_of_a_string() {
        let size = Constraint::of(
            ElementSet::Size(Box::new(Constraint::of(
                ElementSet::Range {
                    lo: v(1),
                    hi: Bound::Max,
                },
                "",
            ))),
            "(SIZE (1..MAX))",
        );
        let from = Constraint::of(
            ElementSet::From(Box::new(Constraint::of(
                ElementSet::Union(vec![
                    ElementSet::Range {
                        lo: Bound::Inclusive(Value::String("0".into())),
                        hi: Bound::Inclusive(Value::String("9".into())),
                    },
                    ElementSet::Value(Value::String(" ".into())),
                ]),
                "",
            ))),
            "(FROM (\"0\"..\"9\" | \" \"))",
        );
        let summary = fold(None, &[size, from], Class::KnownMultiplier, Mode::Effective);
        assert_eq!(
            summary.size,
            Some(Range {
                lo: Some(1),
                hi: None,
                extensible: false
            })
        );
        assert_eq!(summary.alphabet.unwrap().ranges, vec![(32, 32), (48, 57)]);
    }

    #[test]
    fn a_union_with_an_unknown_part_does_not_restrict() {
        let set = ElementSet::Union(vec![
            ElementSet::Range { lo: v(1), hi: v(2) },
            ElementSet::UserDefined("CONSTRAINED BY {}".into()),
        ]);
        assert!(eval(&set, Domain::Value, Mode::Effective).is_none());
        let set = ElementSet::Intersection(vec![
            ElementSet::Range { lo: v(1), hi: v(2) },
            ElementSet::UserDefined("CONSTRAINED BY {}".into()),
        ]);
        assert_eq!(eval(&set, Domain::Value, Mode::Effective), Some(r(1, 2)));
    }
}
