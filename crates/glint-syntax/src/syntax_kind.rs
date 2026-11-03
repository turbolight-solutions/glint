use logos::Logos;

/// Every token and node kind of the concrete syntax tree.
///
/// Token kinds up to and including [`SyntaxKind::Error`] are produced by the lexer;
/// the remaining kinds are produced by the parser.
#[derive(Logos, strum::FromRepr, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum SyntaxKind {
    // X.680 12.1.6
    #[regex(r"[ \t\r\n\x0B\x0C\u{A0}]+")]
    Whitespace,
    // X.680 12.6.3
    #[regex(r"--([^\-\r\n\x0B\x0C]|-[^\-\r\n\x0B\x0C])*(--|-)?")]
    LineComment,
    // X.680 12.6.5
    #[token("/*", block_comment)]
    BlockComment,

    // X.680 12.2, 12.4, 12.5, 12.25
    #[regex(r"[A-Z][A-Za-z0-9]*(-[A-Za-z0-9]+)*")]
    UpperName,
    // X.680 12.3, 12.7
    #[regex(r"[a-z][A-Za-z0-9]*(-[A-Za-z0-9]+)*")]
    LowerName,
    // X.681 7.4-7.8
    #[regex(r"&[A-Z][A-Za-z0-9]*(-[A-Za-z0-9]+)*")]
    FieldUpper,
    #[regex(r"&[a-z][A-Za-z0-9]*(-[A-Za-z0-9]+)*")]
    FieldLower,

    // X.680 12.8
    #[regex(r"[0-9]+")]
    Number,
    // X.680 12.9
    #[regex(r"[0-9]+\.[0-9]+([eE]-?[0-9]+)?|[0-9]+[eE]-?[0-9]+")]
    RealNumber,
    // X.680 12.10, 12.12
    #[regex(r"'[^']*'[A-Za-z]?")]
    QuotedString,
    // X.680 12.14
    #[regex(r#""([^"]|"")*""#)]
    CString,

    // X.680 12.20-12.24, 12.37
    #[token("::=")]
    Assign,
    #[token("...")]
    Ellipsis,
    #[token("..")]
    DotDot,
    #[token("[[")]
    LDoubleBracket,
    #[token("]]")]
    RDoubleBracket,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("-")]
    Minus,
    #[token(":")]
    Colon,
    #[token("=")]
    Equals,
    #[token("&")]
    Ampersand,
    #[token("@")]
    At,
    #[token("|")]
    Pipe,
    #[token("!")]
    Bang,
    #[token("^")]
    Caret,
    #[token(";")]
    Semicolon,
    #[token("/")]
    Slash,
    #[token("*")]
    Star,
    #[token("'")]
    #[token("\"")]
    Error,

    /// A reserved word consumed as a keyword by the parser (never produced by the lexer).
    Keyword,

    SourceFile,
    ModuleDefinition,
    ModuleIdentifier,
    EncodingReferenceDefault,
    TagDefault,
    ExtensionDefault,
    ModuleBody,
    Exports,
    Imports,
    SymbolsFromModule,
    SymbolList,
    Symbol,
    GlobalModuleReference,
    AssignedIdentifier,
    EncodingControlSection,

    TypeAssignment,
    ValueAssignment,
    ValueSetAssignment,
    AmbiguousRhs,
    MacroDefinition,
    MacroUse,
    ParameterList,
    Parameter,
    ActualParameterList,
    ActualParameter,

    BuiltinType,
    IntegerType,
    BitStringType,
    EnumeratedType,
    SequenceType,
    SetType,
    ChoiceType,
    SequenceOfType,
    SetOfType,
    InstanceOfType,
    AnyType,
    TaggedType,
    Tag,
    EncodingPrefixedType,
    EncodingPrefix,
    SelectionType,
    DefinedType,
    ObjectClassFieldType,
    TypeFromObject,
    ConstrainedType,
    ObjectClassDef,
    ClassFieldList,
    FieldSpec,
    NamedType,
    NamedNumberList,
    NamedNumber,
    Enumeration,
    ComponentType,
    ComponentsOf,
    ExtensionMarker,
    ExtensionAdditionGroup,
    VersionNumber,
    ExceptionSpec,
    FieldName,

    Value,
    BraceGroup,
    XmlValue,

    Constraint,
    UserDefinedConstraint,
    ContentsConstraint,
    TableConstraint,
    UnionSet,
    IntersectionSet,
    ExclusionSet,
    ParenElementSet,
    ValueElement,
    RangeElement,
    TypeElement,
    SizeElement,
    FromElement,
    ContainedSubtype,
    InnerTypeElement,
    NamedConstraint,
    PatternElement,
    SettingsElement,

    ErrorNode,
}

fn block_comment(lex: &mut logos::Lexer<'_, SyntaxKind>) -> bool {
    let bytes = lex.remainder().as_bytes();
    let mut depth = 1usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let pair = (bytes.get(i), bytes.get(i.saturating_add(1)));
        match pair {
            (Some(b'/'), Some(b'*')) => {
                depth = depth.saturating_add(1);
                i = i.saturating_add(2);
            }
            (Some(b'*'), Some(b'/')) => {
                depth = depth.saturating_sub(1);
                i = i.saturating_add(2);
                if depth == 0 {
                    lex.bump(i);
                    return true;
                }
            }
            _ => i = i.saturating_add(1),
        }
    }
    lex.bump(bytes.len());
    false
}

impl SyntaxKind {
    /// Trivia is attached to the tree but invisible to the parser.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::LineComment | Self::BlockComment | Self::Error
        )
    }

    pub fn is_name(self) -> bool {
        matches!(self, Self::UpperName | Self::LowerName)
    }
}

/// The rowan language marker for ASN.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Asn1Language {}

impl rowan::Language for Asn1Language {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from_repr(raw.0).unwrap_or(SyntaxKind::ErrorNode)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

pub type SyntaxNode = rowan::SyntaxNode<Asn1Language>;
pub type SyntaxToken = rowan::SyntaxToken<Asn1Language>;
pub type SyntaxElement = rowan::SyntaxElement<Asn1Language>;
