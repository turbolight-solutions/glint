//! Typed views over the syntax tree.
//!
//! Each wrapper is a thin newtype over a [`SyntaxNode`]; construction with `cast` checks the node kind.

use crate::syntax_kind::{SyntaxKind, SyntaxNode, SyntaxToken};

macro_rules! ast_node {
    ($(#[$meta:meta])* $name:ident = $kind:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxNode);

        impl $name {
            pub fn cast(node: SyntaxNode) -> Option<Self> {
                (node.kind() == SyntaxKind::$kind).then_some(Self(node))
            }

            pub fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

ast_node!(
    /// The root of a parsed document.
    SourceFile = SourceFile
);
ast_node!(
    /// `ModuleReference DefinitiveIdentification DEFINITIONS ... BEGIN ... END` (X.680 13.1).
    Module = ModuleDefinition
);
ast_node!(
    /// `IMPORTS ... ;` (X.680 13.1).
    Imports = Imports
);
ast_node!(
    /// `SymbolList FROM GlobalModuleReference` (X.680 13.1).
    SymbolsFromModule = SymbolsFromModule
);

/// The tag default of a module (X.680 13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagDefault {
    Explicit,
    Implicit,
    Automatic,
}

/// One top-level assignment, classified by its syntactic shape (X.680 13.1, X.681 9-12).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Assignment {
    Type(SyntaxNode),
    Value(SyntaxNode),
    ValueSet(SyntaxNode),
    Macro(SyntaxNode),
    MacroUse(SyntaxNode),
}

impl Assignment {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        match node.kind() {
            SyntaxKind::TypeAssignment => Some(Self::Type(node)),
            SyntaxKind::ValueAssignment => Some(Self::Value(node)),
            SyntaxKind::ValueSetAssignment => Some(Self::ValueSet(node)),
            SyntaxKind::MacroDefinition => Some(Self::Macro(node)),
            SyntaxKind::MacroUse => Some(Self::MacroUse(node)),
            _ => None,
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Type(node)
            | Self::Value(node)
            | Self::ValueSet(node)
            | Self::Macro(node)
            | Self::MacroUse(node) => node,
        }
    }

    /// The name being defined.
    pub fn name(&self) -> Option<SyntaxToken> {
        first_name(self.syntax())
    }
}

fn first_name(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind().is_name())
}

fn child_of_kind(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().find(|child| child.kind() == kind)
}

impl SourceFile {
    pub fn modules(&self) -> impl Iterator<Item = Module> + '_ {
        self.0.children().filter_map(Module::cast)
    }

    /// The assignments of a bare module body (X.680 13.1 Note 2), or of every module when headers are present.
    pub fn assignments(&self) -> impl Iterator<Item = Assignment> + '_ {
        self.0
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::ModuleBody)
            .flat_map(|body| {
                body.children()
                    .filter_map(Assignment::cast)
                    .collect::<Vec<_>>()
            })
    }
}

impl Module {
    pub fn name(&self) -> Option<SyntaxToken> {
        child_of_kind(&self.0, SyntaxKind::ModuleIdentifier).and_then(|id| first_name(&id))
    }

    /// The text of the definitive object identifier, e.g. `{ 1 3 6 1 }`, if present.
    pub fn definitive_oid(&self) -> Option<String> {
        let id = child_of_kind(&self.0, SyntaxKind::ModuleIdentifier)?;
        child_of_kind(&id, SyntaxKind::BraceGroup).map(|group| group.text().to_string())
    }

    /// The explicit tag default; `None` means the empty `TagDefault`, which is `EXPLICIT TAGS` (X.680 13.2).
    pub fn tag_default(&self) -> Option<TagDefault> {
        let node = child_of_kind(&self.0, SyntaxKind::TagDefault)?;
        let word = node
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.kind() == SyntaxKind::Keyword)?;
        match word.text() {
            "EXPLICIT" => Some(TagDefault::Explicit),
            "IMPLICIT" => Some(TagDefault::Implicit),
            "AUTOMATIC" => Some(TagDefault::Automatic),
            _ => None,
        }
    }

    pub fn extensibility_implied(&self) -> bool {
        child_of_kind(&self.0, SyntaxKind::ExtensionDefault).is_some()
    }

    pub fn imports(&self) -> Option<Imports> {
        let body = child_of_kind(&self.0, SyntaxKind::ModuleBody)?;
        child_of_kind(&body, SyntaxKind::Imports).and_then(Imports::cast)
    }

    pub fn assignments(&self) -> impl Iterator<Item = Assignment> + '_ {
        child_of_kind(&self.0, SyntaxKind::ModuleBody)
            .into_iter()
            .flat_map(|body| {
                body.children()
                    .filter_map(Assignment::cast)
                    .collect::<Vec<_>>()
            })
    }
}

impl Imports {
    pub fn symbols_from_modules(&self) -> impl Iterator<Item = SymbolsFromModule> + '_ {
        self.0.children().filter_map(SymbolsFromModule::cast)
    }
}

impl SymbolsFromModule {
    /// The imported symbol names, in order.
    pub fn symbols(&self) -> Vec<String> {
        child_of_kind(&self.0, SyntaxKind::SymbolList)
            .into_iter()
            .flat_map(|list| list.children().collect::<Vec<_>>())
            .filter_map(|symbol| first_name(&symbol))
            .map(|token| token.text().to_owned())
            .collect()
    }

    /// The module the symbols come from.
    pub fn module_name(&self) -> Option<SyntaxToken> {
        child_of_kind(&self.0, SyntaxKind::GlobalModuleReference).and_then(|r| first_name(&r))
    }

    /// The `AssignedIdentifier` text (an object identifier value or a defined value), if given.
    pub fn assigned_identifier(&self) -> Option<String> {
        let reference = child_of_kind(&self.0, SyntaxKind::GlobalModuleReference)?;
        child_of_kind(&reference, SyntaxKind::AssignedIdentifier).map(|n| n.text().to_string())
    }
}
