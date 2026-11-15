//! The schema walk: raw TLVs interpreted as the types of an IR set (X.690 8 for the encodings, X.682 10 for the
//! component relation constraints that select open types).

use std::borrow::Cow;
use std::cmp::Ordering;

use glint_diag::Severity;
use glint_ir::{
    Collection, Constraint, ElementSet, IrSet, Kind, OpenType, Presence, Record, Region,
    StringKind, Summary, TagClass, TagLayer, TagMode, TimeKind, Type, TypeId, Value,
};

use crate::ber::{self, Raw, Tag};
use crate::finding::{self, DecodeFinding};
use crate::value::{self, OidName, Rendered};
use crate::{Decoded, Oracle, SchemaState, Tlv};

/// How many types the walk may enter without consuming a TLV, against reference cycles.
const MAX_DEPTH: usize = 256;

/// The codes that say a hinted type does not fit the encoding.
const MISFIT: &[&str] = &["E400", "E401", "E403", "E404", "E405"];

pub(crate) fn decode(
    bytes: &[u8],
    schema: Option<(&IrSet, TypeId)>,
    oracle: &dyn Oracle,
) -> Decoded {
    let raws = ber::parse_all(bytes, 0..bytes.len(), 0);
    let mut walker = Walker {
        bytes,
        oracle,
        frames: Vec::new(),
        level: 0,
        hinted: None,
        depth: 0,
        last_record: None,
    };
    let mut tlvs = Vec::new();
    for (index, raw) in raws.iter().enumerate() {
        let tlv = match schema {
            Some((set, root)) if index == 0 => match set.get(root) {
                Some(def) => {
                    let label = Label {
                        field: None,
                        type_name: Some(type_name(set, root)),
                    };
                    walker.ty(set, &def.ty, None, raw, label, Ctx::default())
                }
                None => walker.schemaless(raw),
            },
            _ => walker.schemaless(raw),
        };
        tlvs.push(tlv);
    }
    let mut findings = Vec::new();
    if let (Some(first), true) = (raws.first(), raws.len() > 1) {
        findings.push(DecodeFinding::new(
            "E409",
            format!(
                "{} octets follow the value, from offset {}",
                bytes.len().saturating_sub(first.end),
                first.end
            ),
        ));
    }
    Decoded { tlvs, findings }
}

/// `Module.Type`, with the actual parameters of an instantiation.
fn type_name(set: &IrSet, id: TypeId) -> String {
    let Some(def) = set.get(id) else {
        return "?".to_owned();
    };
    let mut name = format!("{}.{}", def.origin.module, def.origin.asn1_name);
    if !def.origin.actuals.is_empty() {
        name.push('{');
        name.push_str(&def.origin.actuals.join(", "));
        name.push('}');
    }
    name
}

/// A value a component relation constraint can key on.
#[derive(Debug, Clone, PartialEq)]
enum Key {
    Oid(String),
    Integer(i128),
}

impl Key {
    fn of(rendered: Option<&Rendered>) -> Option<Self> {
        match rendered? {
            Rendered::Oid { dotted, .. } => Some(Self::Oid(dotted.clone())),
            Rendered::Integer { decimal, .. } | Rendered::Enumerated { decimal, .. } => {
                decimal.parse().ok().map(Self::Integer)
            }
            _ => None,
        }
    }

    fn matches(&self, value: Option<&Value>) -> bool {
        match (self, value) {
            (Self::Oid(a), Some(Value::Oid(b))) => a == b,
            (Self::Integer(a), Some(Value::Integer(b))) => a == b,
            _ => false,
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Oid(dotted) => dotted.clone(),
            Self::Integer(n) => n.to_string(),
        }
    }
}

/// What an open type's table says about its value.
enum Lookup<'s> {
    /// The type of the value.
    Found(&'s Type),
    /// The object with the key leaves the field out: an `OPTIONAL` field with no type for this key.
    Absent {
        finding: DecodeFinding,
        oid: Option<String>,
    },
    /// The key was not decoded, or no object has it.
    Missing {
        finding: DecodeFinding,
        oid: Option<String>,
    },
    /// No table: `ANY`, `ANY DEFINED BY`, or a type field read without one; the OID to try hints for.
    Untabled { oid: Option<String> },
}

/// The components decoded so far of one `SEQUENCE` or `SET`, by path (`algorithm`, `algorithmIdentifier.algorithm`).
struct Frame {
    /// The number of type references followed when the record was entered: `@a` is resolved in the outermost record
    /// of the type definition that holds the constraint (X.682 10.7).
    level: usize,
    /// The component of the enclosing record that this record is, for a path into a record still being decoded
    /// (`@algorithmIdentifier.algorithm` from inside `algorithmIdentifier`, RFC 5912 `SIGNED`).
    name: Option<String>,
    values: Vec<(String, Key)>,
}

#[derive(Debug, Clone, Default)]
struct Label {
    field: Option<String>,
    type_name: Option<String>,
}

/// What the types met on the way to a kind constrain it: the summary of the outermost (it includes those of the types
/// it refers to) and every constraint.
#[derive(Default, Clone)]
struct Ctx<'s> {
    summary: Option<&'s Summary>,
    constraints: Vec<&'s Constraint>,
}

impl<'s> Ctx<'s> {
    fn absorb(&mut self, ty: &'s Type) {
        if self.summary.is_none() && !ty.summary.is_empty() {
            self.summary = Some(&ty.summary);
        }
        self.constraints.extend(&ty.constraints);
    }

    /// The type of a contents constraint (`CONTAINING T`), in the root of any constraint met.
    fn contained(&self) -> Option<&'s Type> {
        fn find(set: &ElementSet) -> Option<&Type> {
            match set {
                ElementSet::Contents { ty: Some(ty), .. } => Some(ty),
                ElementSet::Intersection(items) | ElementSet::Union(items) => {
                    items.iter().find_map(find)
                }
                _ => None,
            }
        }
        self.constraints.iter().find_map(|c| find(&c.root))
    }

    /// A simple table constraint (`({ContentSet})`) on the value.
    fn table(&self) -> Option<&'s glint_ir::TableRef> {
        fn find(set: &ElementSet) -> Option<&glint_ir::TableRef> {
            match set {
                ElementSet::Table(table) if table.relation.is_empty() => Some(table),
                ElementSet::Intersection(items) => items.iter().find_map(find),
                _ => None,
            }
        }
        self.constraints.iter().find_map(|c| find(&c.root))
    }
}

/// The tags a type can start with.
enum Starts {
    Any,
    Tags(Vec<glint_ir::Tag>),
}

fn universal(number: u64) -> glint_ir::Tag {
    glint_ir::Tag {
        class: TagClass::Universal,
        number,
    }
}

fn starts(set: &IrSet, ty: &Type, depth: usize) -> Starts {
    if let Some(layer) = ty.tags.first() {
        return Starts::Tags(vec![layer.tag]);
    }
    if depth > 64 {
        return Starts::Any;
    }
    match &ty.kind {
        Kind::Reference { target } => set.get(*target).map_or(Starts::Any, |def| {
            starts(set, &def.ty, depth.saturating_add(1))
        }),
        Kind::Choice(record) => {
            let mut tags = Vec::new();
            for component in &record.components {
                match starts(set, &component.ty, depth.saturating_add(1)) {
                    Starts::Any => return Starts::Any,
                    Starts::Tags(more) => tags.extend(more),
                }
            }
            Starts::Tags(tags)
        }
        kind => kind
            .universal_tag()
            .map_or(Starts::Any, |n| Starts::Tags(vec![universal(n)])),
    }
}

fn fits(set: &IrSet, ty: &Type, raw: &Raw) -> bool {
    match starts(set, ty, 0) {
        Starts::Any => raw.tag.is_some(),
        Starts::Tags(tags) => raw.tag.is_some_and(|t| tags.iter().any(|&tag| t.is(tag))),
    }
}

fn tag_text(class: TagClass, number: u64) -> String {
    match class {
        TagClass::Universal => crate::text::universal_name(number)
            .map_or_else(|| format!("[UNIVERSAL {number}]"), str::to_owned),
        TagClass::Application => format!("[APPLICATION {number}]"),
        TagClass::Context => format!("[{number}]"),
        TagClass::Private => format!("[PRIVATE {number}]"),
    }
}

fn found_text(raw: &Raw) -> String {
    raw.tag.map_or_else(
        || "octets that are not a TLV".to_owned(),
        |t| tag_text(t.class, t.number),
    )
}

/// The value of a TLV, or of the TLV inside an explicit tag.
fn leaf_value(tlv: &Tlv) -> Option<&Rendered> {
    match (&tlv.value, tlv.tagging, tlv.children.as_slice()) {
        (Some(value), _, _) => Some(value),
        (None, Some(TagMode::Explicit), [inner]) => leaf_value(inner),
        _ => None,
    }
}

/// Whether `rendered` is the `DEFAULT` value `default`.
fn is_default(default: &Value, rendered: &Rendered) -> bool {
    match (default, rendered) {
        (Value::Boolean(a), Rendered::Boolean { value }) => a == value,
        (
            Value::Integer(a),
            Rendered::Integer { decimal, .. } | Rendered::Enumerated { decimal, .. },
        ) => decimal.parse::<i128>().ok() == Some(*a),
        (
            Value::Identifier(id),
            Rendered::Integer {
                name: Some(name), ..
            }
            | Rendered::Enumerated {
                name: Some(name), ..
            },
        ) => id == name,
        (Value::Bits(names), Rendered::BitString { named, .. }) => names == named,
        _ => false,
    }
}

/// Lowers every error of a hinted subtree to a warning: a hint is a guess.
fn cap(tlv: &mut Tlv) {
    for finding in &mut tlv.findings {
        if finding.severity == Severity::Error {
            finding.severity = Severity::Warning;
        }
    }
    for child in &mut tlv.children {
        cap(child);
    }
}

fn misfits(tlv: &Tlv) -> bool {
    tlv.walk()
        .iter()
        .any(|t| t.findings.iter().any(|f| MISFIT.contains(&f.code)))
}

/// X.690 11.6: `SET OF` encodings compare as octet strings, the shorter padded with zeros.
fn compare_padded(a: &[u8], b: &[u8]) -> Ordering {
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

fn class_rank(class: TagClass) -> u8 {
    match class {
        TagClass::Universal => 0,
        TagClass::Application => 1,
        TagClass::Context => 2,
        TagClass::Private => 3,
    }
}

fn outside(value: i128, range: &glint_ir::Range) -> bool {
    range.lo.is_some_and(|lo| value < lo) || range.hi.is_some_and(|hi| value > hi)
}

fn range_text(range: &glint_ir::Range) -> String {
    let bound = |b: Option<i128>, word: &str| b.map_or_else(|| word.to_owned(), |v| v.to_string());
    format!("{}..{}", bound(range.lo, "MIN"), bound(range.hi, "MAX"))
}

struct Walker<'b> {
    bytes: &'b [u8],
    oracle: &'b dyn Oracle,
    frames: Vec<Frame>,
    level: usize,
    /// The module a hinted subtree is decoded from.
    hinted: Option<String>,
    depth: usize,
    /// The values of the record decoded last, for the record that holds it.
    last_record: Option<Vec<(String, Key)>>,
}

impl<'b> Walker<'b> {
    fn content(&self, raw: &Raw) -> &'b [u8] {
        self.bytes.get(raw.content.clone()).unwrap_or_default()
    }

    fn state(&self) -> SchemaState {
        match &self.hinted {
            Some(from) => SchemaState::Hinted { from: from.clone() },
            None => SchemaState::Matched,
        }
    }

    fn node(&self, raw: &Raw, label: Label) -> Tlv {
        Tlv {
            offset: raw.offset,
            header_len: raw.header_len,
            len: raw.len,
            end: raw.end,
            tag: raw.tag,
            tagging: None,
            field: label.field,
            type_name: label.type_name,
            value: None,
            children: Vec::new(),
            findings: raw.findings.clone(),
            schema: self.state(),
        }
    }

    fn oid_names(&self, dotted: &str, set: Option<&IrSet>) -> Vec<OidName> {
        let mut names = self.oracle.oid_names(dotted);
        if let Some(set) = set {
            for constant in set.oids.iter().filter(|c| c.oid == dotted) {
                if !names.iter().any(|n| n.name == constant.name) {
                    names.push(OidName {
                        name: constant.name.clone(),
                        module: constant.module.clone(),
                        document: None,
                    });
                }
            }
        }
        names
    }

    // ---- without a schema -------------------------------------------------------------------

    /// The TLV as `openssl asn1parse` would show it.
    fn schemaless(&mut self, raw: &Raw) -> Tlv {
        let mut node = self.node(raw, Label::default());
        node.schema = SchemaState::Unmatched;
        let Some(tag) = raw.tag else {
            return node;
        };
        if tag.constructed {
            node.children = raw.children.iter().map(|c| self.schemaless(c)).collect();
            return node;
        }
        let content = self.content(raw);
        let (rendered, findings) = if tag.class == TagClass::Universal {
            self.universal_value(tag.number, content, None)
        } else {
            (
                Some(Rendered::Hex {
                    hex: value::hex(content),
                }),
                Vec::new(),
            )
        };
        node.value = rendered;
        node.findings.extend(findings);
        node
    }

    /// The value of a primitive of universal tag `number`, with what is wrong with it.
    fn universal_value(
        &self,
        number: u64,
        content: &[u8],
        set: Option<&IrSet>,
    ) -> (Option<Rendered>, Vec<DecodeFinding>) {
        match number {
            1 => value::boolean(content),
            2 => value::integer(content, &[]),
            3 => value::bit_string(content, &[]),
            4 => (
                Some(Rendered::OctetString {
                    hex: value::hex(content),
                }),
                Vec::new(),
            ),
            5 => value::null(content),
            6 | 13 => match value::arcs(content, number == 13) {
                Ok(arcs) => {
                    let dotted = value::dotted(&arcs);
                    let rendered = if number == 13 {
                        Rendered::RelativeOid { dotted }
                    } else {
                        Rendered::Oid {
                            names: self.oid_names(&dotted, set),
                            dotted,
                        }
                    };
                    (Some(rendered), Vec::new())
                }
                Err(message) => (None, vec![finding::invalid(message)]),
            },
            9 => value::real(content),
            10 => value::enumerated(content, &[], true),
            23 => value::time(content, TimeKind::UTCTime),
            24 => value::time(content, TimeKind::GeneralizedTime),
            _ => match string_kind(number) {
                Some(kind) => value::string(content, kind),
                None => (
                    Some(Rendered::Hex {
                        hex: value::hex(content),
                    }),
                    Vec::new(),
                ),
            },
        }
    }

    /// `raw` does not match its type: shown without a schema, with what was expected.
    fn mismatch(&mut self, raw: &Raw, label: Label, expected: &str) -> Tlv {
        let mut node = self.schemaless(raw);
        node.field = label.field;
        node.type_name = label.type_name;
        node.findings.push(DecodeFinding::new(
            "E403",
            format!("expected {expected}, found {}", found_text(raw)),
        ));
        node
    }

    // ---- with a schema ----------------------------------------------------------------------

    /// Decodes `raw` as `ty`, `ovr` replacing its outermost tag (an implicit tag written outside it).
    fn ty<'s>(
        &mut self,
        set: &'s IrSet,
        ty: &'s Type,
        ovr: Option<glint_ir::Tag>,
        raw: &Raw,
        mut label: Label,
        mut ctx: Ctx<'s>,
    ) -> Tlv {
        if self.depth > MAX_DEPTH {
            let mut node = self.schemaless(raw);
            node.findings
                .push(finding::malformed("the type nests too deeply to follow"));
            return node;
        }
        self.depth = self.depth.saturating_add(1);
        ctx.absorb(ty);
        if let Kind::Reference { target } = ty.kind
            && label.type_name.is_none()
        {
            label.type_name = Some(type_name(set, target));
        }
        let node = self.tagged(set, &ty.tags, ty, ovr, raw, label, ctx);
        self.depth = self.depth.saturating_sub(1);
        node
    }

    #[allow(clippy::too_many_arguments)]
    fn tagged<'s>(
        &mut self,
        set: &'s IrSet,
        tags: &'s [TagLayer],
        ty: &'s Type,
        ovr: Option<glint_ir::Tag>,
        raw: &Raw,
        label: Label,
        ctx: Ctx<'s>,
    ) -> Tlv {
        let Some((layer, rest)) = tags.split_first() else {
            return self.kind(set, ty, ovr, raw, label, ctx);
        };
        let expected = ovr.unwrap_or(layer.tag);
        match layer.mode {
            TagMode::Implicit => {
                let mut node = self.tagged(set, rest, ty, Some(expected), raw, label, ctx);
                if node.tagging.is_none() && node.schema != SchemaState::Unmatched {
                    node.tagging = Some(TagMode::Implicit);
                }
                node
            }
            TagMode::Explicit => {
                let tag = tag_text(expected.class, expected.number);
                if !raw.tag.is_some_and(|t| t.is(expected) && t.constructed) {
                    return self.mismatch(raw, label, &format!("{tag} EXPLICIT (constructed)"));
                }
                let mut node = self.node(raw, label);
                node.tagging = Some(TagMode::Explicit);
                let mut children = raw.children.iter();
                match children.next() {
                    Some(inner) => {
                        let child = self.tagged(set, rest, ty, None, inner, Label::default(), ctx);
                        node.children.push(child);
                    }
                    None => node.findings.push(finding::invalid(format!(
                        "the explicit tag {tag} holds no TLV (X.690 8.14.2)"
                    ))),
                }
                for extra in children {
                    let mut child = self.schemaless(extra);
                    child.findings.push(DecodeFinding::new(
                        "E405",
                        format!("the explicit tag {tag} holds more than one TLV (X.690 8.14.2)"),
                    ));
                    node.children.push(child);
                }
                node
            }
        }
    }

    fn kind<'s>(
        &mut self,
        set: &'s IrSet,
        ty: &'s Type,
        ovr: Option<glint_ir::Tag>,
        raw: &Raw,
        label: Label,
        ctx: Ctx<'s>,
    ) -> Tlv {
        match &ty.kind {
            Kind::Reference { target } => match set.get(*target) {
                Some(def) => {
                    self.level = self.level.saturating_add(1);
                    let node = self.ty(set, &def.ty, ovr, raw, label, ctx);
                    self.level = self.level.saturating_sub(1);
                    node
                }
                None => self.schemaless(raw),
            },
            Kind::Sequence(record) => self.record(set, record, false, ovr, raw, label),
            Kind::Set(record) => self.record(set, record, true, ovr, raw, label),
            Kind::Choice(record) => self.choice(set, record, raw, label),
            Kind::SequenceOf(c) => self.collection(set, c, false, ovr, raw, label, &ctx),
            Kind::SetOf(c) => self.collection(set, c, true, ovr, raw, label, &ctx),
            Kind::OpenType(open) => self.open_type(set, open, raw, label),
            Kind::Opaque { .. } => {
                let mut node = self.schemaless(raw);
                node.field = label.field;
                node.type_name = label.type_name;
                node
            }
            kind => match kind.universal_tag() {
                Some(number) => self.primitive(set, ty, number, ovr, raw, label, &ctx),
                None => {
                    let mut node = self.schemaless(raw);
                    node.field = label.field;
                    node.type_name = label.type_name;
                    node
                }
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn primitive<'s>(
        &mut self,
        set: &'s IrSet,
        ty: &'s Type,
        number: u64,
        ovr: Option<glint_ir::Tag>,
        raw: &Raw,
        label: Label,
        ctx: &Ctx<'s>,
    ) -> Tlv {
        let expected = ovr.unwrap_or(universal(number));
        let kind = &ty.kind;
        if !raw.tag.is_some_and(|t| t.is(expected)) {
            let mut text = tag_text(expected.class, expected.number);
            if ovr.is_some() {
                text = format!("{text} IMPLICIT");
            }
            return self.mismatch(raw, label, &text);
        }
        let mut node = self.node(raw, label);
        let stringish = matches!(
            kind,
            Kind::BitString { .. } | Kind::OctetString | Kind::String { .. } | Kind::Time { .. }
        );
        let content: Cow<'_, [u8]> = if raw.constructed() {
            if !stringish {
                node.findings.push(finding::invalid(format!(
                    "a {} is encoded in the primitive form (X.690 8.1.2.5)",
                    tag_text(TagClass::Universal, number)
                )));
                node.children = raw.children.iter().map(|c| self.schemaless(c)).collect();
                return node;
            }
            node.findings.push(finding::der(
                "a string in the constructed form (X.690 10.2)",
            ));
            node.children = raw.children.iter().map(|c| self.schemaless(c)).collect();
            let (joined, all_segments) =
                self.segments(raw, number, matches!(kind, Kind::BitString { .. }));
            if !all_segments {
                node.findings.push(finding::invalid(format!(
                    "a segment of the constructed {} is not a {0} (X.690 8.7.3.2)",
                    tag_text(TagClass::Universal, number)
                )));
            }
            Cow::Owned(joined)
        } else {
            Cow::Borrowed(self.content(raw))
        };
        let (rendered, findings) = match kind {
            Kind::Boolean => value::boolean(&content),
            Kind::Integer { named } => value::integer(&content, named),
            Kind::Enumerated {
                root,
                additions,
                extensible,
            } => {
                let items: Vec<_> = root.iter().chain(additions).cloned().collect();
                value::enumerated(&content, &items, *extensible)
            }
            Kind::BitString { named } => value::bit_string(&content, named),
            Kind::String { string } => value::string(&content, *string),
            Kind::Time { time } => value::time(&content, *time),
            _ => self.universal_value(number, &content, Some(set)),
        };
        node.value = rendered;
        node.findings.extend(findings);
        self.check_constraints(set, &mut node, kind, &content, ctx);
        if !raw.constructed() && matches!(kind, Kind::OctetString | Kind::BitString { .. }) {
            self.contents(set, &mut node, raw, kind, ctx);
        }
        node
    }

    /// The contents of a string in the constructed form: its primitive segments, joined (X.690 8.6.4, 8.7.3, 8.23.6),
    /// and whether every segment has the universal tag `number`.
    fn segments(&self, raw: &Raw, number: u64, bits: bool) -> (Vec<u8>, bool) {
        let mut out = Vec::new();
        let mut unused = 0u8;
        let mut all = true;
        let mut stack: Vec<&Raw> = raw.children.iter().rev().collect();
        while let Some(segment) = stack.pop() {
            all &= segment.tag.is_some_and(|t| t.is(universal(number)));
            if segment.constructed() {
                stack.extend(segment.children.iter().rev());
                continue;
            }
            let content = self.content(segment);
            if bits {
                if let Some((&u, rest)) = content.split_first() {
                    unused = u;
                    out.extend_from_slice(rest);
                }
            } else {
                out.extend_from_slice(content);
            }
        }
        if bits {
            out.insert(0, unused);
        }
        (out, all)
    }

    fn check_constraints<'s>(
        &self,
        set: &'s IrSet,
        node: &mut Tlv,
        kind: &Kind,
        content: &[u8],
        ctx: &Ctx<'s>,
    ) {
        if let Some(table) = ctx.table()
            && let Some(key) = Key::of(node.value.as_ref())
            && let Some(objects) = set.object_set(table.set)
            && !objects.extensible
            && !objects.rows.iter().any(|row| key.matches(row.key.as_ref()))
        {
            node.findings.push(DecodeFinding::new(
                "E407",
                format!(
                    "{} is not the {} of an object of {}",
                    key.text(),
                    table.field,
                    objects.name
                ),
            ));
        }
        let Some(summary) = ctx.summary else {
            return;
        };
        if let (Some(range), Kind::Integer { .. }) = (&summary.value, kind)
            && !range.extensible
            && let Some(v) = value::small_integer(content)
            && outside(v, range)
        {
            node.findings.push(DecodeFinding::new(
                "E407",
                format!("{v} is outside the permitted values {}", range_text(range)),
            ));
        }
        let size = match kind {
            Kind::OctetString => Some(content.len()),
            Kind::BitString { .. } => content
                .split_first()
                .map(|(&u, bits)| bits.len().saturating_mul(8).saturating_sub(usize::from(u))),
            Kind::String { .. } => node.value.as_ref().and_then(value::char_count),
            _ => None,
        };
        if let (Some(range), Some(size)) = (&summary.size, size)
            && !range.extensible
            && outside(i128::try_from(size).unwrap_or(i128::MAX), range)
        {
            node.findings.push(DecodeFinding::new(
                "E407",
                format!(
                    "size {size} is outside the permitted sizes {}",
                    range_text(range)
                ),
            ));
        }
        if let (Some(alphabet), Some(Rendered::String { text })) = (&summary.alphabet, &node.value)
            && !alphabet.extensible
            && let Some(c) = text.chars().find(|&c| {
                !alphabet
                    .ranges
                    .iter()
                    .any(|&(lo, hi)| (lo..=hi).contains(&u32::from(c)))
            })
        {
            node.findings.push(DecodeFinding::new(
                "E407",
                format!(
                    "the character {c:?} (U+{:04X}) is not in the permitted alphabet",
                    u32::from(c)
                ),
            ));
        }
    }

    /// Decodes the contents of an octet or bit string that a contents constraint, or else the OID hint table, gives a
    /// type.
    fn contents<'s>(
        &mut self,
        set: &'s IrSet,
        node: &mut Tlv,
        raw: &Raw,
        kind: &Kind,
        ctx: &Ctx<'s>,
    ) {
        let bits = matches!(kind, Kind::BitString { .. });
        let mut range = raw.content.clone();
        if bits {
            if self.bytes.get(range.start) != Some(&0) {
                return;
            }
            range.start = range.start.saturating_add(1);
        }
        if let Some(inner) = ctx.contained() {
            if let Kind::OpenType(open) = &set.resolve(inner).kind {
                // The contents of an open type the table does not resolve are octets, not a TLV: a signature whose
                // algorithm object sets no `&Value` holds bits, not an encoding.
                match self.lookup(set, open) {
                    Lookup::Found(_) => {}
                    Lookup::Absent { oid, .. } | Lookup::Untabled { oid } => {
                        if !bits && let Some(child) = self.try_hints(oid, range) {
                            node.children.push(child);
                        }
                        return;
                    }
                    Lookup::Missing { finding, oid } => {
                        match self.try_hints(oid, range) {
                            Some(child) => node.children.push(child),
                            None => node.findings.push(finding),
                        }
                        return;
                    }
                }
            }
            let raws = ber::parse_all(self.bytes, range, 0);
            let mut raws = raws.iter();
            match raws.next() {
                Some(first) => {
                    let child = self.ty(set, inner, None, first, Label::default(), Ctx::default());
                    node.children.push(child);
                }
                None => node.findings.push(finding::invalid(
                    "the contents are empty, but the type says what they contain",
                )),
            }
            for extra in raws {
                let mut child = self.schemaless(extra);
                child.findings.push(DecodeFinding::new(
                    "E405",
                    "the contents hold more than the contained value",
                ));
                node.children.push(child);
            }
            return;
        }
        if bits {
            return;
        }
        if let Some(child) = self.try_hints(self.sibling_oid(), range) {
            node.children.push(child);
        }
    }

    /// The value of the component that `path` (a component relation path without its `@`) names.
    fn relation(&self, path: &str) -> Option<Key> {
        let dots = path.chars().take_while(|&c| c == '.').count();
        let name = path.get(dots..)?;
        let last = self.frames.len().checked_sub(1)?;
        let start = if dots == 0 {
            let mut found = last;
            for (index, frame) in self.frames.iter().enumerate().rev() {
                if frame.level != self.level {
                    break;
                }
                found = index;
            }
            found
        } else {
            last.checked_sub(dots.saturating_sub(1))?
        };
        let find = |frame: &Frame, name: &str| {
            frame
                .values
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, k)| k.clone())
        };
        if let Some(key) = self.frames.get(start).and_then(|f| find(f, name)) {
            return Some(key);
        }
        // A path into a record that is still open: its values reach the enclosing record only when it closes.
        let mut prefix = String::new();
        for frame in self.frames.iter().skip(start.saturating_add(1)) {
            let component = frame.name.as_deref()?;
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(component);
            if let Some(rest) = name
                .strip_prefix(prefix.as_str())
                .and_then(|r| r.strip_prefix('.'))
                && let Some(key) = find(frame, rest)
            {
                return Some(key);
            }
        }
        None
    }

    /// The OID of the nearest component decoded before, in the innermost record: what an `ANY` or a plain octet string
    /// in a 1988 module is usually defined by.
    fn sibling_oid(&self) -> Option<String> {
        let frame = self.frames.last()?;
        let oid = |direct: bool| {
            frame.values.iter().rev().find_map(|(name, key)| match key {
                Key::Oid(dotted) if !direct || !name.contains('.') => Some(dotted.clone()),
                _ => None,
            })
        };
        oid(true).or_else(|| oid(false))
    }

    /// Decodes the TLV in `range` as the first type the hint table names for `oid` that fits it.
    fn try_hints(&mut self, oid: Option<String>, range: std::ops::Range<usize>) -> Option<Tlv> {
        let oid = oid?;
        let hints = self.oracle.hints(&oid);
        if hints.is_empty() {
            return None;
        }
        let raws = ber::parse_all(self.bytes, range, 0);
        let [raw] = raws.as_slice() else {
            return None;
        };
        self.hinted_walk(&hints, raw, Label::default())
    }

    /// Decodes `raw` as the first hinted type that fits it, named after that type rather than the open type.
    fn hinted_walk(&mut self, hints: &[crate::Hint], raw: &Raw, label: Label) -> Option<Tlv> {
        raw.tag?;
        let label = Label {
            field: label.field,
            type_name: None,
        };
        for hint in hints {
            let outer = self.hinted.replace(hint.from.clone());
            let frames = std::mem::take(&mut self.frames);
            let level = std::mem::replace(&mut self.level, 0);
            let mut tlv = self.ty(
                &hint.set,
                &hint.ty,
                None,
                raw,
                label.clone(),
                Ctx::default(),
            );
            self.frames = frames;
            self.level = level;
            self.hinted = outer;
            if !misfits(&tlv) {
                cap(&mut tlv);
                return Some(tlv);
            }
        }
        None
    }

    /// What the table of an open type says about its value, from the key decoded so far.
    fn lookup<'s>(&self, set: &'s IrSet, open: &'s OpenType) -> Lookup<'s> {
        let key = match (&open.table, &open.defined_by) {
            (Some(table), _) if !table.relation.is_empty() => {
                table.relation.first().and_then(|path| self.relation(path))
            }
            (_, Some(name)) => self.relation(name),
            _ => None,
        };
        if let Some(table) = &open.table
            && let Some(path) = table.relation.first()
            && let Some(objects) = set.object_set(table.set)
        {
            let Some(key) = key else {
                return Lookup::Missing {
                    finding: DecodeFinding::new(
                        "W408",
                        format!("the open type is keyed by `@{path}`, which was not decoded"),
                    ),
                    oid: None,
                };
            };
            let row = objects
                .rows
                .iter()
                .find(|row| key.matches(row.key.as_ref()));
            if let Some(ty) = row.and_then(|row| row.types.get(&table.field)) {
                return Lookup::Found(ty);
            }
            let oid = match &key {
                Key::Oid(dotted) => Some(dotted.clone()),
                Key::Integer(_) => None,
            };
            if row.is_some() {
                return Lookup::Absent {
                    finding: DecodeFinding::new(
                        "W408",
                        format!(
                            "the object of {} keyed by {} does not set {}",
                            objects.name,
                            key.text(),
                            table.field
                        ),
                    ),
                    oid,
                };
            }
            let mut finding = DecodeFinding::new(
                "W408",
                format!(
                    "{} is not the key of an object of {}",
                    key.text(),
                    objects.name
                ),
            );
            if !objects.extensible {
                finding = finding.with_severity(Severity::Error);
            }
            return Lookup::Missing { finding, oid };
        }
        // ANY, ANY DEFINED BY, or a type field with no table: the hint table, if anything
        let oid = match key {
            Some(Key::Oid(dotted)) => Some(dotted),
            Some(Key::Integer(_)) => None,
            None => self.sibling_oid(),
        };
        Lookup::Untabled { oid }
    }

    fn open_type(&mut self, set: &IrSet, open: &OpenType, raw: &Raw, label: Label) -> Tlv {
        let (finding, oid) = match self.lookup(set, open) {
            Lookup::Found(ty) => return self.ty(set, ty, None, raw, label, Ctx::default()),
            Lookup::Absent { finding, oid } | Lookup::Missing { finding, oid } => {
                (Some(finding), oid)
            }
            Lookup::Untabled { oid } => (None, oid),
        };
        if let Some(oid) = oid {
            let hints = self.oracle.hints(&oid);
            if let Some(node) = self.hinted_walk(&hints, raw, label.clone()) {
                return node;
            }
        }
        let mut node = self.schemaless(raw);
        node.field = label.field;
        node.findings.extend(finding);
        node
    }

    fn record(
        &mut self,
        set: &IrSet,
        record: &Record,
        is_set: bool,
        ovr: Option<glint_ir::Tag>,
        raw: &Raw,
        label: Label,
    ) -> Tlv {
        let expected = ovr.unwrap_or(universal(if is_set { 17 } else { 16 }));
        if !raw.tag.is_some_and(|t| t.is(expected) && t.constructed) {
            let mut text = tag_text(expected.class, expected.number);
            if ovr.is_some() {
                text = format!("{text} IMPLICIT");
            }
            return self.mismatch(raw, label, &text);
        }
        let name = label.field.clone();
        let mut node = self.node(raw, label);
        self.frames.push(Frame {
            level: self.level,
            name,
            values: Vec::new(),
        });
        let components = &record.components;
        let mut seen = vec![false; components.len()];
        let mut next = 0usize;
        for child in &raw.children {
            let pick = components
                .iter()
                .enumerate()
                .skip(if is_set { 0 } else { next })
                .find(|(i, c)| !seen.get(*i).copied().unwrap_or(true) && fits(set, &c.ty, child))
                .map(|(i, _)| i);
            let Some(index) = pick else {
                let mut tlv = self.schemaless(child);
                tlv.findings.push(if record.extensible {
                    DecodeFinding::new(
                        "I406",
                        format!(
                            "{} is not a component of the type; an unknown extension addition",
                            found_text(child)
                        ),
                    )
                } else {
                    DecodeFinding::new(
                        "E405",
                        format!("{} is not a component of the type", found_text(child)),
                    )
                });
                node.children.push(tlv);
                continue;
            };
            let Some(component) = components.get(index) else {
                continue;
            };
            if let Some(slot) = seen.get_mut(index) {
                *slot = true;
            }
            next = index.saturating_add(1);
            self.last_record = None;
            let label = Label {
                field: Some(component.name.clone()),
                type_name: None,
            };
            let tlv = self.ty(set, &component.ty, None, child, label, Ctx::default());
            let mut tlv = tlv;
            if let (Presence::Default(default), Some(value)) =
                (&component.presence, leaf_value(&tlv))
                && is_default(default, value)
            {
                tlv.findings.push(finding::der(format!(
                    "`{}` is encoded with its DEFAULT value (X.690 11.5)",
                    component.name
                )));
            }
            let nested = self.last_record.take();
            if let Some(frame) = self.frames.last_mut() {
                if let Some(key) = Key::of(leaf_value(&tlv)) {
                    frame.values.push((component.name.clone(), key));
                }
                for (path, key) in nested.into_iter().flatten() {
                    frame
                        .values
                        .push((format!("{}.{path}", component.name), key));
                }
            }
            node.children.push(tlv);
        }
        for (component, seen) in components.iter().zip(&seen) {
            if !seen && component.presence.is_required() && component.region != Region::Addition {
                node.findings.push(DecodeFinding::new(
                    "E404",
                    format!("the component `{}` is missing", component.name),
                ));
            }
        }
        if is_set {
            let tags: Vec<Tag> = raw.children.iter().filter_map(|c| c.tag).collect();
            if tags.windows(2).any(|w| match w {
                [a, b] => (class_rank(a.class), a.number) > (class_rank(b.class), b.number),
                _ => false,
            }) {
                node.findings.push(finding::der(
                    "the components of a SET are not in tag order (X.690 10.3)",
                ));
            }
        }
        self.last_record = self.frames.pop().map(|f| f.values);
        node
    }

    fn choice(&mut self, set: &IrSet, record: &Record, raw: &Raw, label: Label) -> Tlv {
        let alternative = record.components.iter().find(|c| fits(set, &c.ty, raw));
        let Some(alternative) = alternative else {
            if record.extensible {
                let mut node = self.schemaless(raw);
                node.field = label.field;
                node.type_name = label.type_name;
                node.findings.push(DecodeFinding::new(
                    "I406",
                    format!(
                        "{} is not an alternative of the CHOICE; an unknown extension addition",
                        found_text(raw)
                    ),
                ));
                return node;
            }
            return self.mismatch(raw, label, "an alternative of the CHOICE");
        };
        let field = match &label.field {
            Some(outer) => format!("{outer}.{}", alternative.name),
            None => alternative.name.clone(),
        };
        let inner = Label {
            field: Some(field),
            type_name: label.type_name,
        };
        self.ty(set, &alternative.ty, None, raw, inner, Ctx::default())
    }

    #[allow(clippy::too_many_arguments)]
    fn collection(
        &mut self,
        set: &IrSet,
        collection: &Collection,
        is_set: bool,
        ovr: Option<glint_ir::Tag>,
        raw: &Raw,
        label: Label,
        ctx: &Ctx<'_>,
    ) -> Tlv {
        let expected = ovr.unwrap_or(universal(if is_set { 17 } else { 16 }));
        if !raw.tag.is_some_and(|t| t.is(expected) && t.constructed) {
            let mut text = tag_text(expected.class, expected.number);
            if ovr.is_some() {
                text = format!("{text} IMPLICIT");
            }
            return self.mismatch(raw, label, &text);
        }
        let mut node = self.node(raw, label);
        for child in &raw.children {
            let label = Label {
                field: collection.element_name.clone(),
                type_name: None,
            };
            let tlv = self.ty(set, &collection.element, None, child, label, Ctx::default());
            node.children.push(tlv);
        }
        self.last_record = None;
        if let Some(range) = ctx.summary.and_then(|s| s.size.as_ref())
            && !range.extensible
            && outside(
                i128::try_from(raw.children.len()).unwrap_or(i128::MAX),
                range,
            )
        {
            node.findings.push(DecodeFinding::new(
                "E407",
                format!(
                    "{} elements, outside the permitted sizes {}",
                    raw.children.len(),
                    range_text(range)
                ),
            ));
        }
        if is_set {
            let encodings: Vec<&[u8]> = raw
                .children
                .iter()
                .map(|c| self.bytes.get(c.offset..c.end).unwrap_or_default())
                .collect();
            if encodings
                .windows(2)
                .any(|w| matches!(w, [a, b] if compare_padded(a, b) == Ordering::Greater))
            {
                node.findings.push(finding::der(
                    "the elements of a SET OF are not in ascending order of their encodings (X.690 11.6)",
                ));
            }
        }
        node
    }
}

fn string_kind(number: u64) -> Option<StringKind> {
    Some(match number {
        7 => StringKind::ObjectDescriptor,
        12 => StringKind::UTF8String,
        18 => StringKind::NumericString,
        19 => StringKind::PrintableString,
        20 => StringKind::TeletexString,
        21 => StringKind::VideotexString,
        22 => StringKind::IA5String,
        25 => StringKind::GraphicString,
        26 => StringKind::VisibleString,
        27 => StringKind::GeneralString,
        28 => StringKind::UniversalString,
        30 => StringKind::BMPString,
        _ => return None,
    })
}
