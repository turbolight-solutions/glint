//! The intermediate representation shared by Glint's decoder and code generators.
//!
//! An [`IrSet`] is the closure of one root type: every type it reaches, across modules, with tags resolved,
//! parameterized types monomorphized, constraints evaluated and the information object sets that bind its open types
//! lowered to tables. The crate holds data types only; `glint-sema` lowers modules into them, so consumers never see
//! the syntax tree.

mod dump;
mod summary;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use dump::dump;
pub use summary::{Interval, Intervals, summarize};

/// Bumped whenever the shape of the IR changes.
pub const IR_VERSION: u32 = 1;

/// The index of a [`TypeDef`] in [`IrSet::types`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TypeId(pub u32);

/// The index of an [`ObjectSetTable`] in [`IrSet::object_sets`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectSetId(pub u32);

/// The closure of one root type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrSet {
    pub version: u32,
    pub root: TypeId,
    /// Indexed by [`TypeId`]; ids are given in the order types are first reached from the root, so they are stable
    /// while the modules don't change. References may form cycles.
    pub types: Vec<TypeDef>,
    /// Indexed by [`ObjectSetId`].
    pub object_sets: Vec<ObjectSetTable>,
    /// Every named object identifier value reachable from the root, the table keys included, by name.
    pub oids: Vec<OidConstant>,
    /// What lowering could not represent, with where it happened.
    pub notes: Vec<Note>,
}

impl IrSet {
    pub fn get(&self, id: TypeId) -> Option<&TypeDef> {
        self.types.get(usize::try_from(id.0).ok()?)
    }

    pub fn object_set(&self, id: ObjectSetId) -> Option<&ObjectSetTable> {
        self.object_sets.get(usize::try_from(id.0).ok()?)
    }

    /// The type a reference chain starting at `ty` ends in, with the tags and constraints met on the way left behind.
    pub fn resolve<'a>(&'a self, ty: &'a Type) -> &'a Type {
        let mut current = ty;
        for _ in 0..self.types.len().saturating_add(1) {
            match &current.kind {
                Kind::Reference { target } => match self.get(*target) {
                    Some(def) => current = &def.ty,
                    None => return current,
                },
                _ => return current,
            }
        }
        current
    }
}

/// A named type: a type assignment, or one instantiation of a parameterized type assignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeDef {
    /// The name code generators start from: the ASN.1 name, followed for an instantiation by words taken from its
    /// actual parameters (`AlgorithmIdentifier{SIGNATURE-ALGORITHM, {SignatureAlgorithms}}` →
    /// `AlgorithmIdentifierSignatureAlgorithms`). Unique within a module.
    pub name: String,
    pub origin: Origin,
    pub ty: Type,
}

/// Where a type is defined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub module: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_oid: Option<String>,
    /// The corpus document (`rfc5912`), absent for user ASN.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
    /// The type reference as defined.
    pub asn1_name: String,
    /// The actual parameters of an instantiation, as written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actuals: Vec<String>,
}

/// A type with its tags, kind and constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Type {
    /// The tags written on (or automatically given to) this type, outermost first. The innermost tag of the kind
    /// itself (the universal tag, or the tags of a referenced type) is not repeated here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagLayer>,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<Constraint>,
    /// Advisory: what the constraints, including those of referenced types, amount to.
    #[serde(default, skip_serializing_if = "Summary::is_empty")]
    pub summary: Summary,
}

impl Type {
    pub fn of(kind: Kind) -> Self {
        Self {
            tags: Vec::new(),
            kind,
            constraints: Vec::new(),
            summary: Summary::default(),
        }
    }

    pub fn opaque(reason: impl Into<String>) -> Self {
        Self::of(Kind::Opaque {
            reason: reason.into(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagClass {
    Universal,
    Application,
    Context,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Tag {
    pub class: TagClass,
    pub number: u64,
}

/// How a tag applies (X.680 31.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagMode {
    Explicit,
    Implicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagLayer {
    pub tag: Tag,
    pub mode: TagMode,
    /// Given by `AUTOMATIC TAGS` rather than written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic: bool,
}

/// What a type is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Boolean,
    Integer {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        named: Vec<NamedNumber>,
    },
    Enumerated {
        root: Vec<NamedNumber>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        additions: Vec<NamedNumber>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        extensible: bool,
    },
    Real,
    Null,
    BitString {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        named: Vec<NamedNumber>,
    },
    OctetString,
    ObjectIdentifier,
    RelativeOid,
    String {
        string: StringKind,
    },
    Time {
        time: TimeKind,
    },
    Sequence(Record),
    Set(Record),
    Choice(Record),
    SequenceOf(Collection),
    SetOf(Collection),
    /// An open type (X.681 14.2, X.680 Annex for `ANY`).
    OpenType(OpenType),
    /// A named type of the set.
    Reference {
        #[serde(rename = "to")]
        target: TypeId,
    },
    /// `EXTERNAL`, `EMBEDDED PDV`, `CHARACTER STRING`, `OID-IRI`, `RELATIVE-OID-IRI`: types with an associated type
    /// that few targets support, kept by name.
    Other {
        name: String,
    },
    /// Lowering stopped here: macro notation, an unavailable module, or a construct the IR does not model.
    Opaque {
        reason: String,
    },
}

impl Kind {
    /// The universal tag of a kind that has one.
    pub fn universal_tag(&self) -> Option<u64> {
        Some(match self {
            Self::Boolean => 1,
            Self::Integer { .. } => 2,
            Self::BitString { .. } => 3,
            Self::OctetString => 4,
            Self::Null => 5,
            Self::ObjectIdentifier => 6,
            Self::Real => 9,
            Self::Enumerated { .. } => 10,
            Self::RelativeOid => 13,
            Self::Sequence(_) | Self::SequenceOf(_) => 16,
            Self::Set(_) | Self::SetOf(_) => 17,
            Self::String { string } => string.universal_tag(),
            Self::Time { time } => time.universal_tag(),
            Self::Other { name } => match name.as_str() {
                "EXTERNAL" => 8,
                "EMBEDDED PDV" => 11,
                "CHARACTER STRING" => 29,
                "OID-IRI" => 35,
                "RELATIVE-OID-IRI" => 36,
                _ => return None,
            },
            Self::Choice(_) | Self::OpenType(_) | Self::Reference { .. } | Self::Opaque { .. } => {
                return None;
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StringKind {
    UTF8String,
    NumericString,
    PrintableString,
    TeletexString,
    VideotexString,
    IA5String,
    GraphicString,
    VisibleString,
    GeneralString,
    UniversalString,
    BMPString,
    ObjectDescriptor,
}

impl StringKind {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "UTF8String" => Self::UTF8String,
            "NumericString" => Self::NumericString,
            "PrintableString" => Self::PrintableString,
            "TeletexString" | "T61String" => Self::TeletexString,
            "VideotexString" => Self::VideotexString,
            "IA5String" => Self::IA5String,
            "GraphicString" => Self::GraphicString,
            "VisibleString" | "ISO646String" => Self::VisibleString,
            "GeneralString" => Self::GeneralString,
            "UniversalString" => Self::UniversalString,
            "BMPString" => Self::BMPString,
            "ObjectDescriptor" => Self::ObjectDescriptor,
            _ => return None,
        })
    }

    pub fn universal_tag(self) -> u64 {
        match self {
            Self::ObjectDescriptor => 7,
            Self::UTF8String => 12,
            Self::NumericString => 18,
            Self::PrintableString => 19,
            Self::TeletexString => 20,
            Self::VideotexString => 21,
            Self::IA5String => 22,
            Self::GraphicString => 25,
            Self::VisibleString => 26,
            Self::GeneralString => 27,
            Self::UniversalString => 28,
            Self::BMPString => 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeKind {
    UTCTime,
    GeneralizedTime,
    #[serde(rename = "TIME")]
    Time,
    #[serde(rename = "DATE")]
    Date,
    #[serde(rename = "TIME-OF-DAY")]
    TimeOfDay,
    #[serde(rename = "DATE-TIME")]
    DateTime,
    #[serde(rename = "DURATION")]
    Duration,
}

impl TimeKind {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "UTCTime" => Self::UTCTime,
            "GeneralizedTime" => Self::GeneralizedTime,
            "TIME" => Self::Time,
            "DATE" => Self::Date,
            "TIME-OF-DAY" => Self::TimeOfDay,
            "DATE-TIME" => Self::DateTime,
            "DURATION" => Self::Duration,
            _ => return None,
        })
    }

    pub fn universal_tag(self) -> u64 {
        match self {
            Self::UTCTime => 23,
            Self::GeneralizedTime => 24,
            Self::Time => 14,
            Self::Date => 31,
            Self::TimeOfDay => 32,
            Self::DateTime => 33,
            Self::Duration => 34,
        }
    }
}

/// A named number, named bit or enumeration item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedNumber {
    pub name: String,
    pub value: i128,
}

/// The components of a `SEQUENCE` or `SET`, or the alternatives of a `CHOICE`, in textual order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub components: Vec<Component>,
    /// An extension marker is present, or the module has `EXTENSIBILITY IMPLIED`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extensible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    #[serde(default, skip_serializing_if = "Presence::is_required")]
    pub presence: Presence,
    #[serde(default, skip_serializing_if = "Region::is_root")]
    pub region: Region,
    /// The extension addition group (`[[ ... ]]`) the component belongs to, numbered in textual order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>,
    /// The version number of that group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    #[default]
    Required,
    Optional,
    Default(Value),
}

impl Presence {
    pub fn is_required(&self) -> bool {
        matches!(self, Self::Required)
    }
}

/// Where a component sits relative to the extension markers (X.680 25.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Region {
    #[default]
    Root,
    Addition,
    /// A root component after the second extension marker.
    TrailingRoot,
}

impl Region {
    pub fn is_root(&self) -> bool {
        matches!(self, Self::Root)
    }
}

/// The element of a `SEQUENCE OF` or `SET OF`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    /// `SEQUENCE OF name Type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_name: Option<String>,
    pub element: Box<Type>,
}

/// An open type: its concrete type is chosen by a table, by `DEFINED BY`, or not at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenType {
    /// The class and type field it comes from (`EXTENSION`, `&ExtnType`), absent for `ANY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The component relation constraint that selects the type (X.682 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<TableRef>,
    /// `ANY DEFINED BY component`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defined_by: Option<String>,
}

/// A table constraint: the values (or types) of `field` in the objects of `set`, keyed through `relation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRef {
    pub set: ObjectSetId,
    pub field: String,
    /// The `@` paths of a component relation constraint, without the `@` (`algorithm`, `.id`,
    /// `algorithmIdentifier.algorithm`); empty for a simple table constraint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation: Vec<String>,
}

// ---- values ---------------------------------------------------------------------------------

/// A value after evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Value {
    Boolean(bool),
    Integer(i128),
    /// A real number as written.
    Real(String),
    Null,
    /// Dotted arcs.
    Oid(String),
    /// A bit string by the names of its set bits.
    Bits(Vec<String>),
    /// `'0101'B`, as written between the quotes.
    Bstring(String),
    /// `'0A0B'H`, as written between the quotes.
    Hstring(String),
    /// A character string value.
    String(String),
    /// An enumeration item, or an identifier that names a value of the governing type.
    Identifier(String),
    /// A `SEQUENCE` or `SET` value, by component.
    Record(Vec<(String, Value)>),
    /// Anything else, as written.
    Raw(String),
}

/// A named object identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidConstant {
    pub name: String,
    pub module: String,
    pub oid: String,
}

// ---- constraints ----------------------------------------------------------------------------

/// One constraint of a type, evaluated (X.680 49-51, X.682).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub root: ElementSet,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extensible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<ElementSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exception: Option<Value>,
    /// The constraint as written, for comments and messages.
    pub text: String,
}

impl Constraint {
    pub fn of(root: ElementSet, text: impl Into<String>) -> Self {
        Self {
            root,
            extensible: false,
            additions: None,
            exception: None,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ElementSet {
    All,
    Union(Vec<ElementSet>),
    Intersection(Vec<ElementSet>),
    Except(Box<ElementSet>, Box<ElementSet>),
    Value(Value),
    Range {
        lo: Bound,
        hi: Bound,
    },
    Size(Box<Constraint>),
    /// A permitted alphabet.
    From(Box<Constraint>),
    Pattern(String),
    ContainedSubtype(Box<Type>),
    WithComponent(Box<Constraint>),
    WithComponents {
        partial: bool,
        items: Vec<ComponentConstraint>,
    },
    Contents {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ty: Option<Box<Type>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encoded_by: Option<Value>,
    },
    Table(TableRef),
    /// `SETTINGS "..."` of the time types (X.680 51.11).
    Settings(String),
    /// `CONSTRAINED BY { ... }`, as written.
    UserDefined(String),
    /// Evaluation failed; the text, for a comment and a finding.
    Opaque(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Bound {
    Min,
    Max,
    Inclusive(Value),
    Exclusive(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentConstraint {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint: Option<Constraint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<PresenceConstraint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PresenceConstraint {
    Present,
    Absent,
    Optional,
}

/// What the constraints of a type amount to, so a backend can choose a native type without set arithmetic.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Summary {
    /// The permitted values of an integer type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Range>,
    /// The permitted sizes of a string type or a collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Range>,
    /// The permitted characters of a character string type, as inclusive code point ranges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alphabet: Option<Alphabet>,
    /// The subset visible to PER (X.691 8.3), for a later PER or OER decoder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_visible: Option<Box<Summary>>,
}

impl Summary {
    pub fn is_empty(&self) -> bool {
        self.value.is_none()
            && self.size.is_none()
            && self.alphabet.is_none()
            && self.per_visible.is_none()
    }
}

/// The smallest interval holding the root of an integer constraint; `None` is unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lo: Option<i128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hi: Option<i128>,
    /// Values outside the root are permitted as extensions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extensible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alphabet {
    pub ranges: Vec<(u32, u32)>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extensible: bool,
}

// ---- information object sets ----------------------------------------------------------------

/// An information object set lowered to a table (X.681 12, X.682 10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectSetTable {
    /// The set as written: its reference name, or the text of the set expression.
    pub name: String,
    pub module: String,
    pub class: String,
    /// The `UNIQUE` field rows are keyed by (`&id`), when the class has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_field: Option<String>,
    pub rows: Vec<Row>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extensible: bool,
}

/// One object of a set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// The object reference, absent for an object written in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// The value of the key field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Value>,
    /// The name the key was written as (`id-ce-keyUsage`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    /// The settings of the type fields.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, Type>,
    /// The settings of the fixed-type value fields.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
}

/// Something lowering could not represent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The type or object set it happened in.
    pub at: String,
    pub message: String,
}
