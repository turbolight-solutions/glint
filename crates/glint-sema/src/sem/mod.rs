//! Phase two of the X.681-X.683 ambiguity strategy: once the symbol tables exist, decide what each assignment and
//! reference denotes, and read classes, objects and object sets with the grammar their class dictates.

mod class;
mod object;
pub(crate) mod toks;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

pub use class::{
    ClassInfo, FORBIDDEN_LITERALS, Field, FieldKind, SynItem, SyntaxSpec, contains_field,
    first_literal,
};
pub use object::{ObjectInfo, ObjectSetSyntax, Operand, Setting, parse_object_set_syntax};

use crate::graph::{Found, ModuleGraph, ModuleId, QualifiedError};
use crate::model::SymbolKind;
use crate::view::{child, defined_type_ref, tokens, trimmed_range, type_child};

/// An assignment of a loaded module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymRef {
    pub module: ModuleId,
    pub index: usize,
}

/// What an assignment defines once the ambiguity of X.680 16 and X.681 9-12 is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Type,
    Class,
    Value,
    ValueSet,
    Object,
    ObjectSet,
    Macro,
    /// Cannot be decided: the governor comes from an unavailable module or is a dummy reference.
    Unknown,
}

/// Whether a type-like node names a class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassRes {
    /// The assignment whose `CLASS { ... }` defines the class.
    Class(SymRef),
    NotClass,
    Unknown,
}

/// A dummy reference of a parameterized assignment (X.683 8.3).
#[derive(Debug, Clone)]
pub struct Dummy {
    pub name: String,
    pub range: TextRange,
    pub governor: Option<SyntaxNode>,
    pub governor_name: Option<String>,
    pub node: SyntaxNode,
}

/// What a name denotes at a place in a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    Sym(SymRef),
    Dummy(usize),
    /// Imported from a module that is not available.
    Opaque,
    Missing,
    /// `Module.name` where the module or symbol is not imported.
    NotImported(bool),
}

const MAX_FOLLOW: usize = 16;

pub struct Sem<'g, 'c> {
    pub graph: &'g ModuleGraph<'c>,
    kinds: RefCell<HashMap<SymRef, Kind>>,
    classes: RefCell<HashMap<SymRef, Option<Rc<ClassInfo>>>>,
}

impl<'g, 'c> Sem<'g, 'c> {
    pub fn new(graph: &'g ModuleGraph<'c>) -> Self {
        Self {
            graph,
            kinds: RefCell::new(HashMap::new()),
            classes: RefCell::new(HashMap::new()),
        }
    }

    pub fn symbol(&self, sym: SymRef) -> Option<&'g crate::model::Symbol> {
        self.graph.symbol(sym.module, sym.index)
    }

    /// The dummy references in scope at `node` (X.683 8.4).
    pub fn scope_of(&self, node: &SyntaxNode) -> Vec<Dummy> {
        node.ancestors()
            .find(|n| {
                matches!(
                    n.kind(),
                    SyntaxKind::TypeAssignment
                        | SyntaxKind::ValueAssignment
                        | SyntaxKind::ValueSetAssignment
                )
            })
            .map(|assignment| dummies_of(&assignment))
            .unwrap_or_default()
    }

    /// Resolves `name` (optionally qualified) as written in `module` with `scope` in force (X.683 8.4 first).
    pub fn resolve(
        &self,
        module: ModuleId,
        scope: &[Dummy],
        qualifier: Option<&str>,
        name: &str,
    ) -> Resolved {
        match qualifier {
            Some(module_name) => match self.graph.lookup_qualified(module, module_name, name) {
                Ok(lookup) => self.lookup_to_resolved(module, lookup),
                Err(QualifiedError::ModuleNotImported) => Resolved::NotImported(false),
                Err(QualifiedError::SymbolNotImported) => Resolved::NotImported(true),
            },
            None => {
                if let Some(position) = scope.iter().position(|d| d.name == name) {
                    return Resolved::Dummy(position);
                }
                match self.graph.lookup(module, name) {
                    Some(lookup) => self.lookup_to_resolved(module, lookup),
                    None => Resolved::Missing,
                }
            }
        }
    }

    /// A name imported from a module that was matched by name although the IMPORTS gives a different identifier
    /// (`W102`) may denote a different edition than the author meant, so its definition is not trusted.
    fn lookup_to_resolved(&self, module: ModuleId, lookup: crate::graph::Lookup) -> Resolved {
        let skewed = lookup
            .via_clause
            .is_some_and(|clause| self.clause_skewed(module, clause, 4));
        if skewed && matches!(lookup.found, Found::Symbol { .. }) {
            return Resolved::Opaque;
        }
        self.found_to_resolved(lookup.found)
    }

    fn clause_skewed(&self, module: ModuleId, clause: usize, budget: usize) -> bool {
        let Some(rest) = budget.checked_sub(1) else {
            return false;
        };
        let Some(loaded) = self.graph.module(module) else {
            return false;
        };
        let Some(model) = loaded.model.imports.get(clause) else {
            return false;
        };
        let named_only = loaded
            .imports
            .get(clause)
            .is_some_and(|r| r.matched == crate::graph::Matched::NameOnly);
        if named_only && model.assigned.is_some() {
            return true;
        }
        match &model.assigned {
            Some((
                crate::model::AssignedId::Defined {
                    module: None, name, ..
                },
                _,
            )) => loaded
                .model
                .imported(name)
                .first()
                .is_some_and(|&(c, _)| self.clause_skewed(module, c, rest)),
            _ => false,
        }
    }

    fn found_to_resolved(&self, found: Found) -> Resolved {
        match found {
            Found::Symbol { module, index } => Resolved::Sym(SymRef { module, index }),
            Found::Opaque => Resolved::Opaque,
            Found::MissingInTarget => Resolved::Missing,
        }
    }

    /// Resolves the reference of a `DefinedType` node.
    pub fn resolve_defined(&self, module: ModuleId, node: &SyntaxNode) -> Option<Resolved> {
        let reference = defined_type_ref(node)?;
        let scope = self.scope_of(node);
        Some(self.resolve(module, &scope, reference.module.as_deref(), &reference.name))
    }

    // ---- kinds ----------------------------------------------------------------------------------

    /// X.681 9.1, 11.1, 12.1 and X.680 16.2, 16.6: decides which production an assignment is.
    pub fn kind(&self, sym: SymRef) -> Kind {
        if let Some(&cached) = self.kinds.borrow().get(&sym) {
            return cached;
        }
        let kind = self.compute_kind(sym, MAX_FOLLOW);
        self.kinds.borrow_mut().insert(sym, kind);
        kind
    }

    fn compute_kind(&self, sym: SymRef, budget: usize) -> Kind {
        let Some(symbol) = self.symbol(sym) else {
            return Kind::Unknown;
        };
        match symbol.kind {
            SymbolKind::Macro => Kind::Macro,
            SymbolKind::Unknown => Kind::Unknown,
            SymbolKind::Class => Kind::Class,
            SymbolKind::Value => match self.governor_class(sym) {
                ClassRes::Class(_) => Kind::Object,
                _ => Kind::Value,
            },
            SymbolKind::Type => match self.class_res_budget(sym, budget) {
                ClassRes::Class(_) => Kind::Class,
                _ => Kind::Type,
            },
            SymbolKind::ObjectOrValue => match self.governor_class(sym) {
                ClassRes::Class(_) => Kind::Object,
                ClassRes::NotClass => Kind::Value,
                ClassRes::Unknown => Kind::Unknown,
            },
            SymbolKind::SetOfValuesOrObjects => match self.governor_class(sym) {
                ClassRes::Class(_) => Kind::ObjectSet,
                ClassRes::NotClass => Kind::ValueSet,
                ClassRes::Unknown => Kind::Unknown,
            },
        }
    }

    /// The class that governs an object or object set assignment (X.681 11.1, 12.1).
    pub fn governor_class(&self, sym: SymRef) -> ClassRes {
        let Some(symbol) = self.symbol(sym) else {
            return ClassRes::Unknown;
        };
        match &symbol.type_node {
            Some(node) => self.class_of(sym.module, node),
            None => ClassRes::Unknown,
        }
    }

    /// Whether the type-like `node`, written in `module`, names a class (X.681 9.2).
    pub fn class_of(&self, module: ModuleId, node: &SyntaxNode) -> ClassRes {
        self.class_of_budget(module, node, MAX_FOLLOW)
    }

    fn class_of_budget(&self, module: ModuleId, node: &SyntaxNode, budget: usize) -> ClassRes {
        let Some(rest) = budget.checked_sub(1) else {
            return ClassRes::Unknown;
        };
        match node.kind() {
            SyntaxKind::ObjectClassDef => ClassRes::Unknown,
            SyntaxKind::DefinedType => match self.resolve_defined(module, node) {
                Some(Resolved::Sym(target)) => self.class_res_budget(target, rest),
                Some(Resolved::Dummy(_) | Resolved::Opaque) | None => ClassRes::Unknown,
                Some(Resolved::Missing | Resolved::NotImported(_)) => ClassRes::Unknown,
            },
            SyntaxKind::ConstrainedType | SyntaxKind::EncodingPrefixedType => ClassRes::NotClass,
            _ => ClassRes::NotClass,
        }
    }

    fn class_res_budget(&self, sym: SymRef, budget: usize) -> ClassRes {
        let Some(rest) = budget.checked_sub(1) else {
            return ClassRes::Unknown;
        };
        let Some(symbol) = self.symbol(sym) else {
            return ClassRes::Unknown;
        };
        match symbol.kind {
            SymbolKind::Class => ClassRes::Class(sym),
            SymbolKind::Type => match &symbol.type_node {
                Some(node) if node.kind() == SyntaxKind::DefinedType => {
                    self.class_of_budget(sym.module, node, rest)
                }
                Some(node) if node.kind() == SyntaxKind::ObjectClassDef => ClassRes::Class(sym),
                _ => ClassRes::NotClass,
            },
            SymbolKind::Macro | SymbolKind::Unknown => ClassRes::Unknown,
            _ => ClassRes::NotClass,
        }
    }

    /// The class named by `node`, as the assignment that defines it.
    pub fn class_res_of_symbol(&self, sym: SymRef) -> ClassRes {
        self.class_res_budget(sym, MAX_FOLLOW)
    }

    pub fn class_ref(&self, module: ModuleId, node: &SyntaxNode) -> Option<SymRef> {
        match self.class_of(module, node) {
            ClassRes::Class(sym) => Some(sym),
            _ => None,
        }
    }

    pub fn class_info(&self, sym: SymRef) -> Option<Rc<ClassInfo>> {
        if let Some(cached) = self.classes.borrow().get(&sym) {
            return cached.clone();
        }
        let info = class::build(self, sym).map(Rc::new);
        self.classes.borrow_mut().insert(sym, info.clone());
        info
    }

    /// The class of the object `sym`.
    pub fn object_class(&self, sym: SymRef) -> Option<SymRef> {
        self.class_ref(sym.module, self.symbol(sym)?.type_node.as_ref()?)
    }
}

/// Reads the `ParameterList` of an assignment node.
pub fn dummies_of(assignment: &SyntaxNode) -> Vec<Dummy> {
    let Some(list) = child(assignment, SyntaxKind::ParameterList) else {
        return Vec::new();
    };
    list.children()
        .filter(|p| p.kind() == SyntaxKind::Parameter)
        .filter_map(|parameter| {
            let toks = tokens(&parameter);
            let name = toks.iter().rev().find(|t| t.kind().is_name())?.clone();
            let colon = toks.iter().position(|t| t.kind() == SyntaxKind::Colon);
            let governor = colon.and_then(|_| type_child(&parameter));
            let governor_name = colon.and_then(|at| {
                toks.get(..at)?
                    .iter()
                    .find(|t| t.kind().is_name())
                    .map(|t| t.text().to_owned())
            });
            Some(Dummy {
                name: name.text().to_owned(),
                range: name.text_range(),
                governor,
                governor_name,
                node: parameter,
            })
        })
        .collect()
}

/// The range of a node without leading and trailing trivia.
pub fn range_of(node: &SyntaxNode) -> TextRange {
    trimmed_range(node)
}
