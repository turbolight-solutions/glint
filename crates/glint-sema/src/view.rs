//! Small helpers over the concrete syntax tree: type nodes, components, tags and built-in types.

use glint_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, TextRange};

pub fn is_type_node(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::BuiltinType
            | SyntaxKind::IntegerType
            | SyntaxKind::BitStringType
            | SyntaxKind::EnumeratedType
            | SyntaxKind::SequenceType
            | SyntaxKind::SetType
            | SyntaxKind::ChoiceType
            | SyntaxKind::SequenceOfType
            | SyntaxKind::SetOfType
            | SyntaxKind::InstanceOfType
            | SyntaxKind::AnyType
            | SyntaxKind::TaggedType
            | SyntaxKind::EncodingPrefixedType
            | SyntaxKind::SelectionType
            | SyntaxKind::DefinedType
            | SyntaxKind::ObjectClassFieldType
            | SyntaxKind::TypeFromObject
            | SyntaxKind::ConstrainedType
            | SyntaxKind::ObjectClassDef
    )
}

pub fn tokens(node: &SyntaxNode) -> Vec<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .collect()
}

pub fn child(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().find(|child| child.kind() == kind)
}

pub fn type_child(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.children().find(|child| is_type_node(child.kind()))
}

pub fn first_name(node: &SyntaxNode) -> Option<SyntaxToken> {
    tokens(node).into_iter().find(|t| t.kind().is_name())
}

/// The range of a node without its leading and trailing trivia.
pub fn trimmed_range(node: &SyntaxNode) -> TextRange {
    let first = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia());
    let last = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
        .last();
    match (first, last) {
        (Some(a), Some(b)) => TextRange::new(a.text_range().start(), b.text_range().end()),
        _ => node.text_range(),
    }
}

/// The classification of a type that value and constraint checks care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseKind {
    Boolean,
    Integer,
    BitString,
    OctetString,
    Null,
    ObjectIdentifier,
    RelativeOid,
    Real,
    Enumerated,
    Sequence,
    Set,
    SequenceOf,
    SetOf,
    Choice,
    CharString,
    /// `UTCTime` and `GeneralizedTime`.
    Time,
    Other,
}

/// The base kind and universal tag number of a simple built-in type, from its keyword text (X.680 Table 1).
pub fn builtin_by_text(text: &str) -> Option<(BaseKind, u32)> {
    let entry = match text {
        "BOOLEAN" => (BaseKind::Boolean, 1),
        "INTEGER" => (BaseKind::Integer, 2),
        "BIT STRING" => (BaseKind::BitString, 3),
        "OCTET STRING" => (BaseKind::OctetString, 4),
        "NULL" => (BaseKind::Null, 5),
        "OBJECT IDENTIFIER" => (BaseKind::ObjectIdentifier, 6),
        "ObjectDescriptor" => (BaseKind::CharString, 7),
        "EXTERNAL" => (BaseKind::Other, 8),
        "REAL" => (BaseKind::Real, 9),
        "ENUMERATED" => (BaseKind::Enumerated, 10),
        "EMBEDDED PDV" => (BaseKind::Other, 11),
        "UTF8String" => (BaseKind::CharString, 12),
        "RELATIVE-OID" => (BaseKind::RelativeOid, 13),
        "TIME" => (BaseKind::Other, 14),
        "SEQUENCE" => (BaseKind::Sequence, 16),
        "SET" => (BaseKind::Set, 17),
        "NumericString" => (BaseKind::CharString, 18),
        "PrintableString" => (BaseKind::CharString, 19),
        "TeletexString" | "T61String" => (BaseKind::CharString, 20),
        "VideotexString" => (BaseKind::CharString, 21),
        "IA5String" => (BaseKind::CharString, 22),
        "UTCTime" => (BaseKind::Time, 23),
        "GeneralizedTime" => (BaseKind::Time, 24),
        "GraphicString" => (BaseKind::CharString, 25),
        "VisibleString" | "ISO646String" => (BaseKind::CharString, 26),
        "GeneralString" => (BaseKind::CharString, 27),
        "UniversalString" => (BaseKind::CharString, 28),
        "CHARACTER STRING" => (BaseKind::Other, 29),
        "BMPString" => (BaseKind::CharString, 30),
        "DATE" => (BaseKind::Other, 31),
        "TIME-OF-DAY" => (BaseKind::Other, 32),
        "DATE-TIME" => (BaseKind::Other, 33),
        "DURATION" => (BaseKind::Other, 34),
        "OID-IRI" => (BaseKind::Other, 35),
        "RELATIVE-OID-IRI" => (BaseKind::Other, 36),
        _ => return None,
    };
    Some(entry)
}

/// The keyword text of a `BuiltinType` node, such as `OCTET STRING`.
pub fn builtin_text(node: &SyntaxNode) -> String {
    tokens(node)
        .iter()
        .map(|t| t.text().to_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The syntactic category of a type node, looking through constraints and encoding prefixes but not tags or references.
pub fn base_kind_of_node(node: &SyntaxNode) -> Option<BaseKind> {
    match node.kind() {
        SyntaxKind::BuiltinType => builtin_by_text(&builtin_text(node)).map(|(kind, _)| kind),
        SyntaxKind::IntegerType => Some(BaseKind::Integer),
        SyntaxKind::BitStringType => Some(BaseKind::BitString),
        SyntaxKind::EnumeratedType => Some(BaseKind::Enumerated),
        SyntaxKind::SequenceType => Some(BaseKind::Sequence),
        SyntaxKind::SetType => Some(BaseKind::Set),
        SyntaxKind::ChoiceType => Some(BaseKind::Choice),
        SyntaxKind::SequenceOfType => Some(BaseKind::SequenceOf),
        SyntaxKind::SetOfType => Some(BaseKind::SetOf),
        SyntaxKind::InstanceOfType => Some(BaseKind::Other),
        _ => None,
    }
}

/// The class of a tag (X.680 31.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TagClass {
    Universal,
    Application,
    Context,
    Private,
}

/// A parsed `Tag` node.
#[derive(Debug, Clone)]
pub struct TagDecl {
    pub class: TagClass,
    /// The class number when it is a plain number.
    pub number: Option<u128>,
    /// A `DefinedValue` class number, as `(module, name)`.
    pub reference: Option<(Option<String>, String)>,
    pub range: TextRange,
}

/// Reads a `Tag` node (X.680 31.2.1).
pub fn read_tag(node: &SyntaxNode) -> TagDecl {
    let mut class = TagClass::Context;
    let mut number = None;
    let mut names: Vec<String> = Vec::new();
    let all = tokens(node);
    for (index, token) in all.iter().enumerate() {
        let is_encoding_reference = token.text() == "TAG"
            && all
                .get(index.saturating_add(1))
                .is_some_and(|next| next.kind() == SyntaxKind::Colon);
        match (token.kind(), token.text()) {
            (SyntaxKind::Keyword, "UNIVERSAL") => class = TagClass::Universal,
            (SyntaxKind::Keyword, "APPLICATION") => class = TagClass::Application,
            (SyntaxKind::Keyword, "PRIVATE") => class = TagClass::Private,
            (SyntaxKind::Number, text) => number = text.parse().ok(),
            (SyntaxKind::LowerName | SyntaxKind::UpperName, text) if !is_encoding_reference => {
                names.push(text.to_owned());
            }
            _ => {}
        }
    }
    let reference = match names.as_slice() {
        [name] if number.is_none() => Some((None, name.clone())),
        [module, name] if number.is_none() => Some((Some(module.clone()), name.clone())),
        _ => None,
    };
    TagDecl {
        class,
        number,
        reference,
        range: trimmed_range(node),
    }
}

/// Whether a type is textually tagged (X.680 25.2).
pub fn is_textually_tagged(node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::TaggedType => true,
        SyntaxKind::EncodingPrefixedType => {
            type_child(node).is_some_and(|t| is_textually_tagged(&t))
        }
        _ => false,
    }
}

/// A reference to a type by name: `Name` or `Module.Name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub module: Option<String>,
    pub name: String,
    pub range: TextRange,
}

/// Reads the reference of a `DefinedType` node (X.680 17.1).
pub fn defined_type_ref(node: &SyntaxNode) -> Option<TypeRef> {
    let toks = tokens(node);
    let names: Vec<&SyntaxToken> = toks
        .iter()
        .take_while(|t| t.kind() != SyntaxKind::LBrace)
        .filter(|t| t.kind() == SyntaxKind::UpperName)
        .collect();
    match names.as_slice() {
        [name] => Some(TypeRef {
            module: None,
            name: name.text().to_owned(),
            range: name.text_range(),
        }),
        [module, name] => Some(TypeRef {
            module: Some(module.text().to_owned()),
            name: name.text().to_owned(),
            range: TextRange::new(module.text_range().start(), name.text_range().end()),
        }),
        _ => None,
    }
}

/// Where a component sits relative to the extension markers (X.680 25.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Root,
    Addition,
    TrailingRoot,
}

/// A `NamedType` with its optionality, as it appears in a `SEQUENCE`, `SET` or `CHOICE`.
#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,
    pub name_range: TextRange,
    pub ty: SyntaxNode,
    pub optional: bool,
    pub default: Option<SyntaxNode>,
    pub region: Region,
    /// The `ExtensionAdditionGroup` this component is in, as an index into [`ComponentList::groups`].
    pub group: Option<usize>,
}

impl Component {
    pub fn is_optional_or_default(&self) -> bool {
        self.optional || self.default.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct ComponentsOf {
    pub ty: SyntaxNode,
    pub region: Region,
}

#[derive(Debug, Clone)]
pub enum ListItem {
    Component(Component),
    ComponentsOf(ComponentsOf),
}

#[derive(Debug, Clone)]
pub struct Group {
    pub range: TextRange,
    pub version: Option<(u128, TextRange)>,
}

/// The items between the braces of a `SEQUENCE`, `SET` or `CHOICE`, in text order.
#[derive(Debug, Clone, Default)]
pub struct ComponentList {
    pub items: Vec<ListItem>,
    pub markers: Vec<TextRange>,
    pub groups: Vec<Group>,
}

impl ComponentList {
    // X.680 25.1, 29.1
    pub fn read(node: &SyntaxNode) -> Self {
        let mut list = Self::default();
        let mut region = Region::Root;
        for item in node.children() {
            match item.kind() {
                SyntaxKind::ExtensionMarker => {
                    list.markers.push(trimmed_range(&item));
                    region = match list.markers.len() {
                        1 => Region::Addition,
                        _ => Region::TrailingRoot,
                    };
                }
                SyntaxKind::ComponentType => {
                    if let Some(component) = read_component(&item, region, None) {
                        list.items.push(ListItem::Component(component));
                    }
                }
                SyntaxKind::ComponentsOf => {
                    if let Some(ty) = type_child(&item) {
                        list.items
                            .push(ListItem::ComponentsOf(ComponentsOf { ty, region }));
                    }
                }
                SyntaxKind::ExtensionAdditionGroup => {
                    let index = list.groups.len();
                    let version = child(&item, SyntaxKind::VersionNumber).and_then(|v| {
                        let number = tokens(&v)
                            .into_iter()
                            .find(|t| t.kind() == SyntaxKind::Number)?;
                        Some((number.text().parse().ok()?, number.text_range()))
                    });
                    list.groups.push(Group {
                        range: trimmed_range(&item),
                        version,
                    });
                    for inner in item.children() {
                        if inner.kind() == SyntaxKind::ComponentType
                            && let Some(c) = read_component(&inner, Region::Addition, Some(index))
                        {
                            list.items.push(ListItem::Component(c));
                        }
                    }
                }
                _ => {}
            }
        }
        list
    }

    pub fn components(&self) -> impl Iterator<Item = &Component> {
        self.items.iter().filter_map(|item| match item {
            ListItem::Component(c) => Some(c),
            ListItem::ComponentsOf(_) => None,
        })
    }

    pub fn is_extensible(&self) -> bool {
        !self.markers.is_empty()
    }
}

pub(crate) fn read_component(
    node: &SyntaxNode,
    region: Region,
    group: Option<usize>,
) -> Option<Component> {
    let named = child(node, SyntaxKind::NamedType)?;
    let name_token = tokens(&named)
        .into_iter()
        .find(|t| t.kind() == SyntaxKind::LowerName)?;
    let ty = type_child(&named)?;
    let mut optional = false;
    let mut has_default = false;
    for token in tokens(node) {
        if token.kind() == SyntaxKind::Keyword {
            match token.text() {
                "OPTIONAL" => optional = true,
                "DEFAULT" => has_default = true,
                _ => {}
            }
        }
    }
    let default = if has_default {
        child(node, SyntaxKind::Value)
    } else {
        None
    };
    Some(Component {
        name: name_token.text().to_owned(),
        name_range: name_token.text_range(),
        ty,
        optional,
        default,
        region,
        group,
    })
}

/// The element type of a `SEQUENCE OF` or `SET OF` (X.680 26.1, 28.1).
pub fn of_element_type(node: &SyntaxNode) -> Option<SyntaxNode> {
    if let Some(named) = child(node, SyntaxKind::NamedType) {
        return type_child(&named);
    }
    type_child(node)
}
