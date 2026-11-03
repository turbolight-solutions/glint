//! The per-module symbol model: header, exports, imports and assignments.

use std::collections::HashMap;

use glint_syntax::ast::{self, TagDefault};
use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use crate::view::{child, first_name, tokens, trimmed_range, type_child};

/// What an assignment defines, as far as its syntax shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Type,
    Class,
    Value,
    /// `lower Governor ::= { ... }`: a value or an information object, decided by the governor.
    ObjectOrValue,
    /// `Upper Governor ::= { ... }`: a value set or an information object set.
    SetOfValuesOrObjects,
    Macro,
    /// An assignment written in imported macro notation.
    Unknown,
}

impl SymbolKind {
    pub fn is_value_like(self) -> bool {
        matches!(self, Self::Value | Self::ObjectOrValue | Self::Unknown)
    }
}

/// One assignment of a module.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub name_range: TextRange,
    pub kind: SymbolKind,
    pub node: SyntaxNode,
    /// The dummy references of a parameterized assignment (X.683 8.1).
    pub parameters: Vec<String>,
    /// The type (or class) on the left-hand side or right-hand side of the assignment, if any.
    pub type_node: Option<SyntaxNode>,
    /// The value node of a value assignment.
    pub value_node: Option<SyntaxNode>,
}

impl Symbol {
    pub fn is_parameterized(&self) -> bool {
        !self.parameters.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportsModel {
    /// No `EXPORTS` clause.
    Absent,
    All,
    List(Vec<(String, TextRange)>),
}

/// The `WITH SUCCESSORS` / `WITH DESCENDANTS` option of an import (X.680 13.16 f).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Successors,
    Descendants,
}

/// An `AssignedIdentifier` (X.680 13.9).
#[derive(Debug, Clone)]
pub enum AssignedId {
    /// `{ ... }`.
    Oid { group: SyntaxNode },
    /// A defined value, as `(module, name)`.
    Defined {
        module: Option<String>,
        name: String,
        range: TextRange,
    },
}

#[derive(Debug, Clone)]
pub struct ImportedSymbol {
    pub name: String,
    pub range: TextRange,
}

/// One `SymbolsFromModule` of an `IMPORTS` clause.
#[derive(Debug, Clone)]
pub struct ImportClause {
    pub range: TextRange,
    pub module_name: String,
    pub module_range: TextRange,
    pub assigned: Option<(AssignedId, TextRange)>,
    pub selection: Option<(Selection, TextRange)>,
    pub symbols: Vec<ImportedSymbol>,
}

/// The header and assignments of one module.
#[derive(Debug, Clone)]
pub struct ModuleModel {
    pub name: String,
    pub name_range: TextRange,
    pub range: TextRange,
    pub oid_group: Option<SyntaxNode>,
    pub tag_default: TagDefault,
    pub tag_default_range: Option<TextRange>,
    pub extensibility_implied: bool,
    pub exports: ExportsModel,
    pub imports: Vec<ImportClause>,
    pub symbols: Vec<Symbol>,
    by_name: HashMap<String, Vec<usize>>,
    imported_by_name: HashMap<String, Vec<(usize, usize)>>,
}

impl ModuleModel {
    // X.680 13.1
    pub fn build(module: &ast::Module) -> Option<Self> {
        let name_token = module.name()?;
        let id_node = child(module.syntax(), SyntaxKind::ModuleIdentifier);
        let oid_group = id_node
            .as_ref()
            .and_then(|id| child(id, SyntaxKind::BraceGroup));
        let tag_node = child(module.syntax(), SyntaxKind::TagDefault);
        let body = child(module.syntax(), SyntaxKind::ModuleBody);
        let exports = body
            .as_ref()
            .and_then(|b| child(b, SyntaxKind::Exports))
            .map_or(ExportsModel::Absent, |node| read_exports(&node));
        let imports = module
            .imports()
            .map_or_else(Vec::new, |node| read_imports(node.syntax()));
        let symbols: Vec<Symbol> = module
            .assignments()
            .filter_map(|a| read_symbol(&a))
            .collect();
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, symbol) in symbols.iter().enumerate() {
            by_name.entry(symbol.name.clone()).or_default().push(index);
        }
        let mut imported_by_name: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (clause_index, clause) in imports.iter().enumerate() {
            for (symbol_index, symbol) in clause.symbols.iter().enumerate() {
                imported_by_name
                    .entry(symbol.name.clone())
                    .or_default()
                    .push((clause_index, symbol_index));
            }
        }
        Some(Self {
            name: name_token.text().to_owned(),
            name_range: name_token.text_range(),
            range: trimmed_range(module.syntax()),
            oid_group,
            tag_default: module.tag_default().unwrap_or(TagDefault::Explicit),
            tag_default_range: tag_node.as_ref().map(trimmed_range),
            extensibility_implied: module.extensibility_implied(),
            exports,
            imports,
            symbols,
            by_name,
            imported_by_name,
        })
    }

    /// Indexes of every local assignment of `name`, in text order.
    pub fn local(&self, name: &str) -> &[usize] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn first_local(&self, name: &str) -> Option<&Symbol> {
        self.local(name).first().and_then(|&i| self.symbols.get(i))
    }

    /// `(clause index, symbol index within the clause)` of every import of `name`.
    pub fn imported(&self, name: &str) -> &[(usize, usize)] {
        self.imported_by_name.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn clause_by_module_name(&self, name: &str) -> Option<usize> {
        self.imports.iter().position(|c| c.module_name == name)
    }

    pub fn is_automatic(&self) -> bool {
        self.tag_default == TagDefault::Automatic
    }
}

// X.680 13.1 Exports
fn read_exports(node: &SyntaxNode) -> ExportsModel {
    if tokens(node)
        .iter()
        .any(|t| t.kind() == SyntaxKind::Keyword && t.text() == "ALL")
    {
        return ExportsModel::All;
    }
    let names = child(node, SyntaxKind::SymbolList)
        .into_iter()
        .flat_map(|list| list.children().collect::<Vec<_>>())
        .filter_map(|symbol| first_name(&symbol))
        .map(|t| (t.text().to_owned(), t.text_range()))
        .collect();
    ExportsModel::List(names)
}

// X.680 13.1 Imports
fn read_imports(node: &SyntaxNode) -> Vec<ImportClause> {
    node.children()
        .filter(|c| c.kind() == SyntaxKind::SymbolsFromModule)
        .filter_map(|clause| read_clause(&clause))
        .collect()
}

fn read_clause(clause: &SyntaxNode) -> Option<ImportClause> {
    let reference = child(clause, SyntaxKind::GlobalModuleReference)?;
    let module_token = tokens(&reference)
        .into_iter()
        .find(|t| t.kind() == SyntaxKind::UpperName)?;
    let assigned = child(&reference, SyntaxKind::AssignedIdentifier).and_then(|node| {
        let range = trimmed_range(&node);
        if let Some(group) = child(&node, SyntaxKind::BraceGroup) {
            return Some((AssignedId::Oid { group }, range));
        }
        let toks = tokens(&node);
        let id = match toks.as_slice() {
            [name] => AssignedId::Defined {
                module: None,
                name: name.text().to_owned(),
                range,
            },
            [module, _, name] => AssignedId::Defined {
                module: Some(module.text().to_owned()),
                name: name.text().to_owned(),
                range,
            },
            _ => return None,
        };
        Some((id, range))
    });
    let selection = {
        let toks = tokens(&reference);
        toks.iter().find_map(|t| {
            let which = match t.text() {
                "SUCCESSORS" if t.kind() == SyntaxKind::Keyword => Selection::Successors,
                "DESCENDANTS" if t.kind() == SyntaxKind::Keyword => Selection::Descendants,
                _ => return None,
            };
            Some((which, t.text_range()))
        })
    };
    let symbols = child(clause, SyntaxKind::SymbolList)
        .into_iter()
        .flat_map(|list| list.children().collect::<Vec<_>>())
        .filter_map(|symbol| first_name(&symbol))
        .map(|t| ImportedSymbol {
            name: t.text().to_owned(),
            range: t.text_range(),
        })
        .collect();
    Some(ImportClause {
        range: trimmed_range(clause),
        module_name: module_token.text().to_owned(),
        module_range: module_token.text_range(),
        assigned,
        selection,
        symbols,
    })
}

// X.680 13.21
fn read_symbol(assignment: &ast::Assignment) -> Option<Symbol> {
    let node = assignment.syntax().clone();
    let name = assignment.name()?;
    let parameters = child(&node, SyntaxKind::ParameterList)
        .into_iter()
        .flat_map(|list| list.children().collect::<Vec<_>>())
        .filter(|p| p.kind() == SyntaxKind::Parameter)
        .filter_map(|p| tokens(&p).into_iter().rev().find(|t| t.kind().is_name()))
        .map(|t| t.text().to_owned())
        .collect();
    let has_ambiguous_rhs = child(&node, SyntaxKind::AmbiguousRhs).is_some();
    let (kind, type_node, value_node) = match assignment {
        ast::Assignment::Type(_) => {
            let rhs = type_child(&node);
            let kind = match &rhs {
                Some(t) if t.kind() == SyntaxKind::ObjectClassDef => SymbolKind::Class,
                _ => SymbolKind::Type,
            };
            (kind, rhs, None)
        }
        ast::Assignment::Value(_) => {
            let kind = if has_ambiguous_rhs {
                SymbolKind::ObjectOrValue
            } else {
                SymbolKind::Value
            };
            let value = node.children().find(|c| {
                matches!(
                    c.kind(),
                    SyntaxKind::Value | SyntaxKind::XmlValue | SyntaxKind::AmbiguousRhs
                )
            });
            (kind, type_child(&node), value)
        }
        ast::Assignment::ValueSet(_) => (
            SymbolKind::SetOfValuesOrObjects,
            type_child(&node),
            child(&node, SyntaxKind::AmbiguousRhs),
        ),
        ast::Assignment::Macro(_) => (SymbolKind::Macro, None, None),
        ast::Assignment::MacroUse(_) => (SymbolKind::Unknown, None, None),
    };
    Some(Symbol {
        name: name.text().to_owned(),
        name_range: name.text_range(),
        kind,
        node,
        parameters,
        type_node,
        value_node,
    })
}

/// The syntax era of a module, from the constructs it uses (design spec, "Module record").
pub fn syntax_era(module: &ast::Module) -> glint_corpus::SyntaxEra {
    use glint_corpus::SyntaxEra;
    let mut has_macro = false;
    let mut has_any = false;
    let mut has_x681 = false;
    for node in module.syntax().descendants() {
        match node.kind() {
            SyntaxKind::MacroDefinition | SyntaxKind::MacroUse => has_macro = true,
            SyntaxKind::AnyType => has_any = true,
            SyntaxKind::ObjectClassDef
            | SyntaxKind::ObjectClassFieldType
            | SyntaxKind::TableConstraint
            | SyntaxKind::ParameterList
            | SyntaxKind::InstanceOfType => has_x681 = true,
            _ => {}
        }
    }
    if has_macro {
        SyntaxEra::Y1988Macro
    } else if has_x681 {
        SyntaxEra::Y2002
    } else if has_any {
        SyntaxEra::Y1988
    } else {
        SyntaxEra::Neutral
    }
}
