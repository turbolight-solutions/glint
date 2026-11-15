//! The middle layer (Codegen and Decoder Design, Middle layer): the [`IrSet`] of a root type turned into an
//! [`EmitPlan`] a backend only has to print.
//!
//! - Every inline `SEQUENCE`, `SET`, `CHOICE` and `ENUMERATED` is hoisted into a named type, and so is whatever the
//!   target cannot tag in place ([`Capabilities`]): a tagged list element, a chain of explicit tags, a tagged
//!   constructed type.
//! - Tags are collapsed (X.680 31.2.7): an implicit tag replaces the tag inside it, so a use carries at most the
//!   explicit layers plus one implicit one.
//! - Each open type bound by a component relation constraint becomes a [`Dispatch`]: one variant per object of the
//!   set that sets the type field, keyed by the object's unique field. The relation is anchored to the record its
//!   `@` path starts from (X.682 10.7), as a [`Resolution`] of paths by component name.
//! - Constraints become [`Check`] programs; `DEFAULT` values become typed [`Literal`]s.
//! - Names go through the backend's [`Namer`] and a collision pass per scope.

mod check;
mod literal;

use std::collections::HashMap;

use glint_ir::{
    IrSet, Kind, NamedNumber, ObjectSetId, OpenType, Presence, Region, StringKind, Summary,
    TableRef, Tag, TagLayer, TagMode, TimeKind, Type, TypeId, Value,
};

use crate::naming::{Namer, Scope};
use crate::{Capabilities, Finding};

pub use check::{Check, Const, Rule};
pub use literal::Literal;

/// The index of a [`PType`] in [`EmitPlan::types`]. The types of the IR keep their [`TypeId`] as index; hoisted
/// types follow them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PTypeId(pub usize);

/// The index of a [`Dispatch`] in [`EmitPlan::dispatches`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DispatchId(pub usize);

/// What a backend prints.
#[derive(Debug, Clone)]
pub struct EmitPlan {
    pub root: PTypeId,
    /// `PKIX1Explicit-2009.Certificate`.
    pub root_display: String,
    pub modules: Vec<ModulePlan>,
    pub types: Vec<PType>,
    pub dispatches: Vec<Dispatch>,
    pub findings: Vec<Finding>,
}

/// One ASN.1 module: one output file.
#[derive(Debug, Clone)]
pub struct ModulePlan {
    pub asn1: String,
    /// The output module's name, after the backend's rules.
    pub name: String,
    pub oid: Option<String>,
    pub document: Option<String>,
    /// In the order the plan reached them.
    pub types: Vec<PTypeId>,
    pub dispatches: Vec<DispatchId>,
    pub oids: Vec<OidConst>,
}

/// A named object identifier value.
#[derive(Debug, Clone)]
pub struct OidConst {
    pub name: String,
    pub asn1: String,
    pub arcs: Vec<u64>,
}

/// A named type of the output.
#[derive(Debug, Clone)]
pub struct PType {
    pub name: String,
    pub module: usize,
    /// For comments and messages: `PKIX1Explicit-2009.TBSCertificate`, or `GeneralName.otherName` for a hoisted type.
    pub display: String,
    /// The tag of the type itself, at most one layer.
    pub tags: Vec<TagLayer>,
    pub body: Body,
    /// The constraints of the type itself.
    pub rules: Vec<Rule>,
    /// Open types below this record that its components key (X.682 10.7).
    pub resolutions: Vec<Resolution>,
    /// The type of the IR it comes from, when it is not hoisted.
    pub origin: Option<TypeId>,
    base: String,
}

// built once per plan and read in place: the size of the alias variant does not matter
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Body {
    /// A newtype over one use: a leaf, another named type, a list, a contents constraint, an open type.
    Alias(Use),
    Record {
        set: bool,
        fields: Vec<Field>,
        extensible: bool,
    },
    Choice {
        alternatives: Vec<Field>,
        extensible: bool,
    },
    Enumerated {
        items: Vec<Item>,
        additions: Vec<Item>,
        extensible: bool,
    },
}

/// A record component or a choice alternative.
#[derive(Debug, Clone)]
pub struct Field {
    pub asn1: String,
    pub name: String,
    pub presence: FieldPresence,
    pub region: Region,
    pub use_: Use,
    /// The field holds its value behind a pointer: the value's type contains the owner.
    pub boxed: bool,
    /// The IR gives the component a `DEFAULT`, whether or not the target can write it.
    declared_default: bool,
}

#[derive(Debug, Clone)]
pub enum FieldPresence {
    Required,
    Optional,
    Default(Literal),
}

/// An enumeration item.
#[derive(Debug, Clone)]
pub struct Item {
    pub asn1: String,
    pub name: String,
    pub value: i128,
}

/// One use of a type: a component, an element, an alias, a variant.
#[derive(Debug, Clone)]
pub struct Use {
    /// Collapsed, at most [`Capabilities::max_use_tags`] layers.
    pub tags: Vec<TagLayer>,
    pub target: Target,
    /// The constraints written at this use.
    pub rules: Vec<Rule>,
    /// What every constraint met down to the leaf amounts to.
    pub summary: Summary,
}

#[derive(Debug, Clone)]
pub enum Target {
    Leaf(Leaf),
    Named(PTypeId),
    List {
        set: bool,
        element_name: Option<String>,
        element: Box<Use>,
    },
    /// An open type bound by a component relation: the dispatch its anchor resolves.
    Dispatch(DispatchId),
    /// An open type nothing resolves (`ANY`, an unconstrained type field, an opaque type): any one TLV.
    Any,
    /// An octet or bit string whose contents are the encoding of `inner` (X.682 11).
    Containing {
        bits: bool,
        inner: Box<Use>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Leaf {
    Boolean,
    Integer { named: Vec<NamedNumber> },
    Real,
    Null,
    BitString { named: Vec<NamedNumber> },
    OctetString,
    ObjectIdentifier,
    RelativeOid,
    String(StringKind),
    Time(TimeKind),
}

/// The value of a key field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyValue {
    Oid(Vec<u64>),
    Integer(i128),
}

impl KeyValue {
    pub fn text(&self) -> String {
        match self {
            Self::Oid(arcs) => arcs
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join("."),
            Self::Integer(n) => n.to_string(),
        }
    }
}

/// An open type's dispatch: the type field of the objects of one set, by key.
#[derive(Debug, Clone)]
pub struct Dispatch {
    pub name: String,
    pub module: usize,
    /// `SignatureAlgorithms &Params`.
    pub display: String,
    pub set: ObjectSetId,
    pub field: String,
    pub variants: Vec<Variant>,
    /// Keys of objects that leave the field out: their open type stays raw.
    pub absent: Vec<KeyValue>,
    /// Keys outside the set are allowed; otherwise resolving one is an error.
    pub extensible: bool,
    base: String,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    /// The object reference, or the key's name for an object written in place.
    pub asn1: String,
    pub key: KeyValue,
    pub use_: Use,
    pub boxed: bool,
}

/// An open type a record's components key: resolve the value at `open` with the value at `key`, both paths of
/// component names from the record (X.682 10.7). A path crosses lists element by element.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub key: Vec<String>,
    pub open: Vec<String>,
    pub dispatch: DispatchId,
    /// The constraint as written: `@algorithm`.
    pub text: String,
}

impl EmitPlan {
    pub fn ty(&self, id: PTypeId) -> Option<&PType> {
        self.types.get(id.0)
    }

    pub fn dispatch(&self, id: DispatchId) -> Option<&Dispatch> {
        self.dispatches.get(id.0)
    }

    /// The use an alias chain starting at `id` ends in, and the type that holds it.
    pub fn alias_target(&self, id: PTypeId) -> Option<(&PType, &Use)> {
        let mut current = self.ty(id)?;
        for _ in 0..=self.types.len() {
            match &current.body {
                Body::Alias(Use {
                    target: Target::Named(next),
                    ..
                }) => current = self.ty(*next)?,
                Body::Alias(use_) => return Some((current, use_)),
                _ => return None,
            }
        }
        None
    }

    /// The leaf a use ends in, through aliases.
    pub fn leaf_of<'a>(&'a self, use_: &'a Use) -> Option<&'a Leaf> {
        match &use_.target {
            Target::Leaf(leaf) => Some(leaf),
            Target::Named(id) => match self.alias_target(*id)? {
                (
                    _,
                    Use {
                        target: Target::Leaf(leaf),
                        ..
                    },
                ) => Some(leaf),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The tags of a type after X.680 31.2.7: each implicit tag replaces the tag of the layer inside it, keeping that
/// layer's mode; an implicit tag on the innermost layer replaces the type's own.
pub fn collapse(tags: &[TagLayer]) -> Vec<TagLayer> {
    let mut out = Vec::new();
    let mut pending: Option<Tag> = None;
    for layer in tags {
        let tag = pending.take().unwrap_or(layer.tag);
        match layer.mode {
            TagMode::Implicit => pending = Some(tag),
            TagMode::Explicit => out.push(TagLayer {
                tag,
                mode: TagMode::Explicit,
                automatic: layer.automatic,
            }),
        }
    }
    if let Some(tag) = pending {
        out.push(TagLayer {
            tag,
            mode: TagMode::Implicit,
            automatic: false,
        });
    }
    out
}

// ---- building ---------------------------------------------------------------------------------

/// A record or choice being lowered, for anchoring relations.
#[derive(Debug, Clone)]
struct Frame {
    owner: PTypeId,
    /// The component being lowered.
    component: String,
    /// A `SEQUENCE` or `SET`: relation paths count these (a `CHOICE` is crossed, not counted).
    record: bool,
}

struct Builder<'s> {
    set: &'s IrSet,
    namer: &'s dyn Namer,
    caps: &'s Capabilities,
    types: Vec<PType>,
    dispatches: Vec<Dispatch>,
    dispatch_ids: HashMap<(u32, String), DispatchId>,
    modules: Vec<ModulePlan>,
    module_ids: HashMap<String, usize>,
    findings: Vec<Finding>,
    frames: Vec<Frame>,
    /// `DEFAULT`s met while lowering, built once every type exists: (record, component, value).
    defaults: Vec<(PTypeId, usize, &'s Value)>,
}

/// Builds the plan of `set` for a backend with `namer` and `caps`.
pub fn build(set: &IrSet, namer: &dyn Namer, caps: &Capabilities) -> EmitPlan {
    let mut b = Builder {
        set,
        namer,
        caps,
        types: Vec::new(),
        dispatches: Vec::new(),
        dispatch_ids: HashMap::new(),
        modules: Vec::new(),
        module_ids: HashMap::new(),
        findings: Vec::new(),
        frames: Vec::new(),
        defaults: Vec::new(),
    };
    // the types of the IR first, so they keep their ids and claim their names before hoisted ones
    for def in &set.types {
        let module = b.module(
            &def.origin.module,
            def.origin.module_oid.as_deref(),
            def.origin.document.as_deref(),
        );
        b.types.push(PType {
            name: String::new(),
            module,
            display: format!("{}.{}", def.origin.module, def.origin.asn1_name),
            tags: Vec::new(),
            body: Body::Alias(Use::of(Target::Any)),
            rules: Vec::new(),
            resolutions: Vec::new(),
            origin: None,
            base: def.name.clone(),
        });
    }
    for (index, def) in set.types.iter().enumerate() {
        let id = PTypeId(index);
        b.define(id, &def.ty);
        if let Some(t) = b.types.get_mut(index) {
            t.origin = u32::try_from(index).ok().map(TypeId);
        }
    }
    b.defaults();
    b.oids();
    b.boxing();
    b.names();
    for (index, t) in b.types.iter().enumerate() {
        if let Some(m) = b.modules.get_mut(t.module) {
            m.types.push(PTypeId(index));
        }
    }
    for (index, d) in b.dispatches.iter().enumerate() {
        if let Some(m) = b.modules.get_mut(d.module) {
            m.dispatches.push(DispatchId(index));
        }
    }
    let root = PTypeId(usize::try_from(set.root.0).unwrap_or(0));
    let root_display = b
        .types
        .get(root.0)
        .map(|t| t.display.clone())
        .unwrap_or_default();
    EmitPlan {
        root,
        root_display,
        modules: b.modules,
        types: b.types,
        dispatches: b.dispatches,
        findings: b.findings,
    }
}

impl Use {
    fn of(target: Target) -> Self {
        Self {
            tags: Vec::new(),
            target,
            rules: Vec::new(),
            summary: Summary::default(),
        }
    }
}

/// The type a contents constraint names, from the roots of `constraints` (as the decoder reads them).
fn contained(constraints: &[glint_ir::Constraint]) -> Option<&Type> {
    fn find(set: &glint_ir::ElementSet) -> Option<&Type> {
        match set {
            glint_ir::ElementSet::Contents { ty: Some(ty), .. } => Some(ty),
            glint_ir::ElementSet::Intersection(items) | glint_ir::ElementSet::Union(items) => {
                items.iter().find_map(find)
            }
            _ => None,
        }
    }
    constraints.iter().find_map(|c| find(&c.root))
}

fn arcs(dotted: &str) -> Option<Vec<u64>> {
    dotted.split('.').map(|a| a.parse().ok()).collect()
}

fn key_of(value: &Value) -> Option<KeyValue> {
    match value {
        Value::Oid(dotted) => arcs(dotted).map(KeyValue::Oid),
        Value::Integer(n) => Some(KeyValue::Integer(*n)),
        _ => None,
    }
}

/// The words of a set expression: `{ CertExtensions }` → `CertExtensions`.
fn set_words(name: &str) -> String {
    name.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

impl<'s> Builder<'s> {
    fn module(&mut self, name: &str, oid: Option<&str>, document: Option<&str>) -> usize {
        if let Some(&id) = self.module_ids.get(name) {
            if let Some(m) = self.modules.get_mut(id) {
                if m.oid.is_none() {
                    m.oid = oid.map(str::to_owned);
                }
                if m.document.is_none() {
                    m.document = document.map(str::to_owned);
                }
            }
            return id;
        }
        let id = self.modules.len();
        self.modules.push(ModulePlan {
            asn1: name.to_owned(),
            name: String::new(),
            oid: oid.map(str::to_owned),
            document: document.map(str::to_owned),
            types: Vec::new(),
            dispatches: Vec::new(),
            oids: Vec::new(),
        });
        self.module_ids.insert(name.to_owned(), id);
        id
    }

    fn finding(&mut self, code: &'static str, at: &str, message: impl Into<String>) {
        let finding = Finding::new(code, at, message);
        if !self.findings.contains(&finding) {
            self.findings.push(finding);
        }
    }

    fn display(&self, id: PTypeId) -> String {
        self.types
            .get(id.0)
            .map(|t| t.display.clone())
            .unwrap_or_default()
    }

    fn base(&self, id: PTypeId) -> String {
        self.types
            .get(id.0)
            .map(|t| t.base.clone())
            .unwrap_or_default()
    }

    fn new_type(&mut self, module: usize, base: String, display: String) -> PTypeId {
        let id = PTypeId(self.types.len());
        self.types.push(PType {
            name: String::new(),
            module,
            display,
            tags: Vec::new(),
            body: Body::Alias(Use::of(Target::Any)),
            rules: Vec::new(),
            resolutions: Vec::new(),
            origin: None,
            base,
        });
        id
    }

    /// Lowers `ty` as the definition of `id`: its own tags, its body, its own constraints.
    fn define(&mut self, id: PTypeId, ty: &'s Type) {
        let at = self.display(id);
        let module = self.types.get(id.0).map_or(0, |t| t.module);
        let tags = collapse(&ty.tags);
        let rules = self.rules(ty, &at);
        let constructed = matches!(
            ty.kind,
            Kind::Sequence(_) | Kind::Set(_) | Kind::Choice(_) | Kind::Enumerated { .. }
        );
        let keep_tags = tags.is_empty()
            || (!constructed && tags.len() <= 1)
            || (self.caps.tagged_records
                && matches!(ty.kind, Kind::Sequence(_) | Kind::Set(_))
                && matches!(tags.as_slice(), [l] if l.mode == TagMode::Implicit));
        if keep_tags {
            let body = self.body(id, ty, &at);
            if let Some(t) = self.types.get_mut(id.0) {
                t.tags = tags;
                t.body = body;
                t.rules = rules;
            }
            return;
        }
        if constructed {
            // a tagged newtype over the untagged constructed type
            let word = match ty.kind {
                Kind::Sequence(_) => "Sequence",
                Kind::Set(_) => "Set",
                Kind::Choice(_) => "Choice",
                _ => "Enumerated",
            };
            let inner = self.new_type(module, format!("{}-{word}", self.base(id)), at.clone());
            let body = self.body(inner, ty, &at);
            if let Some(t) = self.types.get_mut(inner.0) {
                t.body = body;
            }
            let (own, rest) = split_first(tags);
            let use_ = self.untagged(
                Use {
                    tags: rest,
                    target: Target::Named(inner),
                    rules: Vec::new(),
                    summary: Summary::default(),
                },
                module,
                &format!("{}-Tagged", self.base(id)),
                &at,
            );
            if let Some(t) = self.types.get_mut(id.0) {
                t.tags = own;
                t.body = Body::Alias(use_);
                t.rules = rules;
            }
            return;
        }
        let (own, rest) = split_first(tags);
        let mut use_ = self.use_kind(ty, &self.base(id), &at, module);
        use_.tags = rest;
        let use_ = self.untagged(use_, module, &format!("{}-Tagged", self.base(id)), &at);
        if let Some(t) = self.types.get_mut(id.0) {
            t.tags = own;
            t.body = Body::Alias(use_);
            t.rules = rules;
        }
    }

    /// The body of a type, without its tags.
    fn body(&mut self, owner: PTypeId, ty: &'s Type, at: &str) -> Body {
        let module = self.types.get(owner.0).map_or(0, |t| t.module);
        let base = self.base(owner);
        match &ty.kind {
            Kind::Sequence(record) | Kind::Set(record) | Kind::Choice(record) => {
                let is_choice = matches!(ty.kind, Kind::Choice(_));
                let mut fields = Vec::new();
                for (index, component) in record.components.iter().enumerate() {
                    if let Presence::Default(value) = &component.presence {
                        self.defaults.push((owner, index, value));
                    }
                    self.frames.push(Frame {
                        owner,
                        component: component.name.clone(),
                        record: !is_choice,
                    });
                    let field_at = format!("{at}.{}", component.name);
                    let use_ = self.use_of(
                        &component.ty,
                        &format!("{base}-{}", component.name),
                        &field_at,
                        module,
                    );
                    self.frames.pop();
                    let presence = match &component.presence {
                        Presence::Required => FieldPresence::Required,
                        Presence::Optional => FieldPresence::Optional,
                        // built once every type exists (`defaults`)
                        Presence::Default(_) => FieldPresence::Required,
                    };
                    fields.push(Field {
                        asn1: component.name.clone(),
                        name: String::new(),
                        presence,
                        region: component.region,
                        use_,
                        boxed: false,
                        declared_default: matches!(component.presence, Presence::Default(_)),
                    });
                }
                if is_choice {
                    Body::Choice {
                        alternatives: fields,
                        extensible: record.extensible,
                    }
                } else {
                    Body::Record {
                        set: matches!(ty.kind, Kind::Set(_)),
                        fields,
                        extensible: record.extensible,
                    }
                }
            }
            Kind::Enumerated {
                root,
                additions,
                extensible,
            } => {
                let item = |n: &NamedNumber| Item {
                    asn1: n.name.clone(),
                    name: String::new(),
                    value: n.value,
                };
                Body::Enumerated {
                    items: root.iter().map(item).collect(),
                    additions: additions.iter().map(item).collect(),
                    extensible: *extensible,
                }
            }
            _ => Body::Alias(self.use_kind(ty, &base, at, module)),
        }
    }

    /// A use of `ty`: its tags, constraints and target, with inline constructed types hoisted.
    fn use_of(&mut self, ty: &'s Type, base: &str, at: &str, module: usize) -> Use {
        let mut use_ = match &ty.kind {
            Kind::Sequence(_) | Kind::Set(_) | Kind::Choice(_) | Kind::Enumerated { .. } => {
                let id = self.new_type(module, base.to_owned(), at.to_owned());
                let body = self.body(id, ty, at);
                if let Some(t) = self.types.get_mut(id.0) {
                    t.body = body;
                }
                Use {
                    tags: Vec::new(),
                    target: Target::Named(id),
                    rules: self.rules(ty, at),
                    summary: ty.summary.clone(),
                }
            }
            _ => self.use_kind(ty, base, at, module),
        };
        use_.tags = collapse(&ty.tags);
        self.fit(use_, module, base, at)
    }

    /// A use of a type that is not hoisted, without its tags.
    fn use_kind(&mut self, ty: &'s Type, base: &str, at: &str, module: usize) -> Use {
        let rules = self.rules(ty, at);
        let target = match &ty.kind {
            Kind::Reference { target } => {
                Target::Named(PTypeId(usize::try_from(target.0).unwrap_or(usize::MAX)))
            }
            Kind::Boolean => Target::Leaf(Leaf::Boolean),
            Kind::Integer { named } => Target::Leaf(Leaf::Integer {
                named: named.clone(),
            }),
            Kind::Real if self.caps.real => Target::Leaf(Leaf::Real),
            Kind::Real => {
                self.finding(
                    "W420",
                    at,
                    "REAL is not supported by the target; the value is kept as a raw TLV",
                );
                Target::Any
            }
            Kind::Null => Target::Leaf(Leaf::Null),
            Kind::BitString { .. } | Kind::OctetString if contained(&ty.constraints).is_some() => {
                let bits = !matches!(ty.kind, Kind::OctetString);
                let inner = match contained(&ty.constraints) {
                    Some(inner) => {
                        let (base, at) = (format!("{base}-Contents"), format!("{at} CONTAINING"));
                        let use_ = self.use_of(inner, &base, &at, module);
                        self.untagged(use_, module, &base, &at)
                    }
                    None => Use::of(Target::Any),
                };
                Target::Containing {
                    bits,
                    inner: Box::new(inner),
                }
            }
            Kind::BitString { named } => Target::Leaf(Leaf::BitString {
                named: named.clone(),
            }),
            Kind::OctetString => Target::Leaf(Leaf::OctetString),
            Kind::ObjectIdentifier => Target::Leaf(Leaf::ObjectIdentifier),
            Kind::RelativeOid => Target::Leaf(Leaf::RelativeOid),
            Kind::String { string } => Target::Leaf(Leaf::String(*string)),
            Kind::Time { time } => Target::Leaf(Leaf::Time(*time)),
            Kind::SequenceOf(c) | Kind::SetOf(c) => {
                let word = c.element_name.clone().unwrap_or_else(|| "Item".to_owned());
                let element = self.use_of(
                    &c.element,
                    &format!("{base}-{word}"),
                    &format!("{at}[]"),
                    module,
                );
                let element = if self.caps.element_tags {
                    element
                } else {
                    self.untagged(
                        element,
                        module,
                        &format!("{base}-{word}"),
                        &format!("{at}[]"),
                    )
                };
                Target::List {
                    set: matches!(ty.kind, Kind::SetOf(_)),
                    element_name: c.element_name.clone(),
                    element: Box::new(element),
                }
            }
            Kind::OpenType(open) => self.open_type(open, at),
            Kind::Other { name } => {
                self.finding(
                    "W420",
                    at,
                    format!(
                        "{name} is not supported by the target; the value is kept as a raw TLV"
                    ),
                );
                Target::Any
            }
            Kind::Opaque { reason } => {
                self.finding(
                    "I422",
                    at,
                    format!("the type was not lowered ({reason}); the value is kept as a raw TLV"),
                );
                Target::Any
            }
            Kind::Sequence(_) | Kind::Set(_) | Kind::Choice(_) | Kind::Enumerated { .. } => {
                let id = self.new_type(module, base.to_owned(), at.to_owned());
                let body = self.body(id, ty, at);
                if let Some(t) = self.types.get_mut(id.0) {
                    t.body = body;
                }
                Target::Named(id)
            }
        };
        Use {
            tags: Vec::new(),
            target,
            rules,
            summary: ty.summary.clone(),
        }
    }

    /// `use_` without tags: a tagged use becomes a named type that carries them, since alias bodies, list elements,
    /// variants and contained types have nowhere to write a tag.
    fn untagged(&mut self, use_: Use, module: usize, base: &str, at: &str) -> Use {
        if use_.tags.is_empty() {
            return use_;
        }
        let summary = use_.summary.clone();
        let id = self.hoist(use_, module, base, at);
        Use {
            summary,
            ..Use::of(Target::Named(id))
        }
    }

    /// A named type for `use_`, carrying its first tag as its own and the rest in named types below it.
    fn hoist(&mut self, use_: Use, module: usize, base: &str, at: &str) -> PTypeId {
        let id = self.new_type(module, base.to_owned(), at.to_owned());
        let (own, rest) = split_first(use_.tags.clone());
        let inner = Use { tags: rest, ..use_ };
        let inner = self.untagged(inner, module, &format!("{base}-Inner"), at);
        if let Some(t) = self.types.get_mut(id.0) {
            t.tags = own;
            t.body = Body::Alias(inner);
        }
        id
    }

    /// `use_` with at most as many tags as the target can write on one use; deeper layers go into named types.
    fn fit(&mut self, use_: Use, module: usize, base: &str, at: &str) -> Use {
        if use_.tags.len() <= self.caps.max_use_tags.max(1) {
            return use_;
        }
        let mut tags = use_.tags.clone();
        let outer = tags.remove(0);
        let inner = Use {
            tags,
            ..use_.clone()
        };
        let id = self.hoist(inner, module, &format!("{base}-Tagged"), at);
        Use {
            tags: vec![outer],
            target: Target::Named(id),
            rules: Vec::new(),
            summary: use_.summary,
        }
    }

    /// The target of an open type: its dispatch when a relation keys it and the relation has an anchor.
    fn open_type(&mut self, open: &OpenType, at: &str) -> Target {
        let Some(table) = open.table.as_ref().filter(|t| !t.relation.is_empty()) else {
            return Target::Any;
        };
        let Some(path) = table.relation.first() else {
            return Target::Any;
        };
        if table.relation.len() > 1 {
            self.finding(
                "W420",
                at,
                format!(
                    "only the first of the relations {} selects the type",
                    table.relation.join(", ")
                ),
            );
        }
        let dots = path.chars().take_while(|&c| c == '.').count();
        let records: Vec<usize> = self
            .frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.record)
            .map(|(i, _)| i)
            .collect();
        let anchor = if dots == 0 {
            records.first().copied()
        } else {
            records
                .len()
                .checked_sub(dots)
                .and_then(|i| records.get(i).copied())
        };
        let Some(anchor) = anchor else {
            self.finding(
                "W420",
                at,
                format!("no record holds the relation @{path}; the value is kept as a raw TLV"),
            );
            return Target::Any;
        };
        let Some(owner) = self.frames.get(anchor).map(|f| f.owner) else {
            return Target::Any;
        };
        let dispatch = self.dispatch(table);
        let open_path: Vec<String> = self
            .frames
            .iter()
            .skip(anchor)
            .map(|f| f.component.clone())
            .collect();
        let key: Vec<String> = path
            .get(dots..)
            .unwrap_or_default()
            .split('.')
            .map(str::to_owned)
            .collect();
        if let Some(t) = self.types.get_mut(owner.0) {
            let resolution = Resolution {
                key,
                open: open_path,
                dispatch,
                text: format!("@{path}"),
            };
            t.resolutions.push(resolution);
        }
        Target::Dispatch(dispatch)
    }

    fn dispatch(&mut self, table: &TableRef) -> DispatchId {
        if let Some(&id) = self.dispatch_ids.get(&(table.set.0, table.field.clone())) {
            return id;
        }
        let id = DispatchId(self.dispatches.len());
        self.dispatch_ids
            .insert((table.set.0, table.field.clone()), id);
        let set = self.set;
        let Some(objects) = set.object_set(table.set) else {
            self.dispatches.push(Dispatch {
                name: String::new(),
                module: 0,
                display: table.field.clone(),
                set: table.set,
                field: table.field.clone(),
                variants: Vec::new(),
                absent: Vec::new(),
                extensible: true,
                base: format!("Set-{}", table.field),
            });
            return id;
        };
        let module = self.module(&objects.module, None, None);
        let base = format!(
            "{}-{}",
            set_words(&objects.name),
            table.field.trim_start_matches('&')
        );
        let display = format!("{}.{} {}", objects.module, objects.name, table.field);
        self.dispatches.push(Dispatch {
            name: String::new(),
            module,
            display: display.clone(),
            set: table.set,
            field: table.field.clone(),
            variants: Vec::new(),
            absent: Vec::new(),
            extensible: objects.extensible,
            base: base.clone(),
        });
        // the frames of the record holding the open type do not reach into the objects' types
        let frames = std::mem::take(&mut self.frames);
        let mut variants: Vec<Variant> = Vec::new();
        let mut absent = Vec::new();
        for row in &objects.rows {
            let Some(key) = row.key.as_ref().and_then(key_of) else {
                continue;
            };
            if variants.iter().any(|v| v.key == key) || absent.contains(&key) {
                continue;
            }
            let asn1 = row
                .object
                .clone()
                .or_else(|| row.key_name.clone())
                .unwrap_or_else(|| format!("key-{}", key.text().replace('.', "-")));
            match row.types.get(&table.field) {
                Some(ty) => {
                    let (vbase, vat) = (format!("{base}-{asn1}"), format!("{display} {asn1}"));
                    let use_ = self.use_of(ty, &vbase, &vat, module);
                    let use_ = self.untagged(use_, module, &vbase, &vat);
                    variants.push(Variant {
                        name: String::new(),
                        asn1,
                        key,
                        use_,
                        boxed: false,
                    });
                }
                None => absent.push(key),
            }
        }
        self.frames = frames;
        if let Some(d) = self.dispatches.get_mut(id.0) {
            d.variants = variants;
            d.absent = absent;
        }
        id
    }

    fn rules(&mut self, ty: &Type, at: &str) -> Vec<Rule> {
        let (rules, unsupported) = check::lower(self.set, ty, self.caps);
        for text in unsupported {
            self.finding("W421", at, format!("the constraint {text} is not checked"));
        }
        rules
    }

    /// Builds the `DEFAULT` literals, now that every type exists.
    fn defaults(&mut self) {
        for (owner, field, value) in std::mem::take(&mut self.defaults) {
            let Some(use_) = self.field_use(owner, field).cloned() else {
                continue;
            };
            let at = format!("{}.{}", self.display(owner), self.field_asn1(owner, field));
            let presence = match literal::build(self, &use_, value) {
                Some(literal) => FieldPresence::Default(literal),
                None => {
                    self.finding(
                        "W420",
                        &at,
                        "the DEFAULT value cannot be written for the target; the component is OPTIONAL in the output",
                    );
                    FieldPresence::Optional
                }
            };
            if let Some(Body::Record { fields, .. }) =
                self.types.get_mut(owner.0).map(|t| &mut t.body)
                && let Some(f) = fields.get_mut(field)
            {
                f.presence = presence;
            }
        }
    }

    fn field_use(&self, owner: PTypeId, field: usize) -> Option<&Use> {
        match &self.types.get(owner.0)?.body {
            Body::Record { fields, .. } => fields.get(field).map(|f| &f.use_),
            _ => None,
        }
    }

    fn field_asn1(&self, owner: PTypeId, field: usize) -> String {
        match self.types.get(owner.0).map(|t| &t.body) {
            Some(Body::Record { fields, .. }) => fields
                .get(field)
                .map(|f| f.asn1.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn oids(&mut self) {
        let set = self.set;
        for constant in &set.oids {
            let Some(arcs) = arcs(&constant.oid) else {
                continue;
            };
            let module = self.module(&constant.module, None, None);
            if let Some(m) = self.modules.get_mut(module)
                && !m.oids.iter().any(|o| o.asn1 == constant.name)
            {
                m.oids.push(OidConst {
                    name: String::new(),
                    asn1: constant.name.clone(),
                    arcs,
                });
            }
        }
    }

    /// Marks the fields and variants whose value contains their owner, which a target with value types must box.
    fn boxing(&mut self) {
        let n = self.types.len();
        let node_count = n.saturating_add(self.dispatches.len());
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); node_count];
        let target_node = |use_: &Use| -> Option<usize> { by_value(use_, n) };
        for (i, t) in self.types.iter().enumerate() {
            let uses: Vec<&Use> = match &t.body {
                Body::Alias(u) => vec![u],
                Body::Record { fields, .. } => fields.iter().map(|f| &f.use_).collect(),
                Body::Choice { alternatives, .. } => alternatives.iter().map(|f| &f.use_).collect(),
                Body::Enumerated { .. } => Vec::new(),
            };
            if let Some(slot) = edges.get_mut(i) {
                slot.extend(uses.into_iter().filter_map(target_node));
            }
        }
        for (i, d) in self.dispatches.iter().enumerate() {
            if let Some(slot) = edges.get_mut(n.saturating_add(i)) {
                slot.extend(d.variants.iter().map(|v| &v.use_).filter_map(target_node));
            }
        }
        let scc = tarjan(&edges);
        let same = |a: usize, b: usize| scc.get(a).is_some_and(|x| scc.get(b) == Some(x));
        for (i, t) in self.types.iter_mut().enumerate() {
            let fields = match &mut t.body {
                Body::Record { fields, .. } => fields,
                Body::Choice { alternatives, .. } => alternatives,
                _ => continue,
            };
            for f in fields {
                if let Some(node) = by_value(&f.use_, n)
                    && same(i, node)
                {
                    f.boxed = true;
                }
            }
        }
        for (i, d) in self.dispatches.iter_mut().enumerate() {
            for v in &mut d.variants {
                if let Some(node) = by_value(&v.use_, n)
                    && same(n.saturating_add(i), node)
                {
                    v.boxed = true;
                }
            }
        }
    }

    /// The collision pass: module names, then per module the types of the IR, the dispatches and the hoisted types,
    /// then fields, variants and constants within their scopes.
    fn names(&mut self) {
        let namer = self.namer;
        let mut modules = Scope::new(&[]);
        for m in &mut self.modules {
            m.name = modules.claim(namer.module_name(&m.asn1));
        }
        let mut scopes: Vec<Scope> = self
            .modules
            .iter()
            .map(|_| Scope::new(namer.reserved_types()))
            .collect();
        let ir = self.set.types.len();
        let order: Vec<usize> = (0..ir).collect();
        for i in order {
            if let Some(t) = self.types.get_mut(i)
                && let Some(scope) = scopes.get_mut(t.module)
            {
                t.name = scope.claim(namer.type_name(&t.base));
            }
        }
        for d in &mut self.dispatches {
            if let Some(scope) = scopes.get_mut(d.module) {
                d.name = scope.claim(namer.type_name(&d.base));
            }
        }
        for t in self.types.iter_mut().skip(ir) {
            if let Some(scope) = scopes.get_mut(t.module) {
                t.name = scope.claim(namer.type_name(&t.base));
            }
        }
        for t in &mut self.types {
            match &mut t.body {
                Body::Record { fields, .. } => {
                    let mut scope = Scope::new(&[]);
                    for f in fields {
                        f.name = scope.claim(namer.field_name(&f.asn1));
                    }
                }
                Body::Choice { alternatives, .. } => {
                    let mut scope = Scope::new(&[]);
                    for f in alternatives {
                        f.name = scope.claim(namer.variant_name(&f.asn1));
                    }
                }
                Body::Enumerated {
                    items, additions, ..
                } => {
                    let mut scope = Scope::new(&[]);
                    for item in items.iter_mut().chain(additions.iter_mut()) {
                        item.name = scope.claim(namer.variant_name(&item.asn1));
                    }
                }
                Body::Alias(_) => {}
            }
        }
        for d in &mut self.dispatches {
            let mut scope = Scope::new(&["Unknown"]);
            for v in &mut d.variants {
                v.name = scope.claim(namer.variant_name(&v.asn1));
            }
        }
        for m in &mut self.modules {
            let mut scope = Scope::new(&[]);
            for o in &mut m.oids {
                o.name = scope.claim(namer.const_name(&o.asn1));
            }
        }
    }
}

fn split_first(mut tags: Vec<TagLayer>) -> (Vec<TagLayer>, Vec<TagLayer>) {
    if tags.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let first = tags.remove(0);
    (vec![first], tags)
}

/// The node a use holds by value: a named type or a dispatch (through contents constraints, not lists).
fn by_value(use_: &Use, types: usize) -> Option<usize> {
    match &use_.target {
        Target::Named(id) => Some(id.0),
        Target::Dispatch(id) => Some(types.saturating_add(id.0)),
        Target::Containing { inner, .. } => by_value(inner, types),
        _ => None,
    }
}

/// Strongly connected components; the component index of each node.
fn tarjan(edges: &[Vec<usize>]) -> Vec<usize> {
    struct State<'e> {
        edges: &'e [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        comp: Vec<usize>,
        comps: usize,
    }
    fn visit(s: &mut State<'_>, v: usize) {
        let edges = s.edges;
        if let Some(slot) = s.index.get_mut(v) {
            *slot = Some(s.next);
        }
        if let Some(slot) = s.low.get_mut(v) {
            *slot = s.next;
        }
        s.next = s.next.saturating_add(1);
        s.stack.push(v);
        if let Some(slot) = s.on_stack.get_mut(v) {
            *slot = true;
        }
        for &w in edges.get(v).into_iter().flatten() {
            if s.index.get(w).copied().flatten().is_none() {
                visit(s, w);
                let lw = s.low.get(w).copied().unwrap_or(usize::MAX);
                if let Some(lv) = s.low.get_mut(v) {
                    *lv = (*lv).min(lw);
                }
            } else if s.on_stack.get(w).copied().unwrap_or(false) {
                let iw = s.index.get(w).copied().flatten().unwrap_or(usize::MAX);
                if let Some(lv) = s.low.get_mut(v) {
                    *lv = (*lv).min(iw);
                }
            }
        }
        if s.low.get(v) == s.index.get(v).copied().flatten().as_ref() {
            while let Some(w) = s.stack.pop() {
                if let Some(slot) = s.on_stack.get_mut(w) {
                    *slot = false;
                }
                if let Some(slot) = s.comp.get_mut(w) {
                    *slot = s.comps;
                }
                if w == v {
                    break;
                }
            }
            s.comps = s.comps.saturating_add(1);
        }
    }
    let n = edges.len();
    let mut s = State {
        edges,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        comp: vec![0; n],
        comps: 0,
    };
    for v in 0..n {
        if s.index.get(v).copied().flatten().is_none() {
            visit(&mut s, v);
        }
    }
    s.comp
}

#[cfg(test)]
mod tests {
    use glint_ir::TagClass;

    use super::*;

    fn layer(number: u64, mode: TagMode) -> TagLayer {
        TagLayer {
            tag: Tag {
                class: TagClass::Context,
                number,
            },
            mode,
            automatic: false,
        }
    }

    #[test]
    fn implicit_tags_replace_the_layer_inside() {
        let universal8 = TagLayer {
            tag: Tag {
                class: TagClass::Universal,
                number: 8,
            },
            mode: TagMode::Implicit,
            automatic: false,
        };
        assert_eq!(
            collapse(&[layer(0, TagMode::Implicit), universal8]),
            vec![layer(0, TagMode::Implicit)]
        );
        assert_eq!(
            collapse(&[layer(1, TagMode::Implicit), layer(2, TagMode::Explicit)]),
            vec![layer(1, TagMode::Explicit)]
        );
        assert_eq!(
            collapse(&[layer(1, TagMode::Explicit), layer(2, TagMode::Explicit)]),
            vec![layer(1, TagMode::Explicit), layer(2, TagMode::Explicit)]
        );
    }

    #[test]
    fn components_in_one_scc_are_found() {
        let edges = vec![vec![1], vec![0], vec![0]];
        let scc = tarjan(&edges);
        assert_eq!(scc[0], scc[1]);
        assert_ne!(scc[0], scc[2]);
    }
}
