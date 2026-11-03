//! The resolved type model: what a decoder or a documentation tool needs to know about the types of a module,
//! after tagging, extensibility, constraints, open types and parameterization are resolved.
//!
//! The model is serializable and meant to be stable enough for tools; it is built from the same tree the checks
//! read, so it never disagrees with them about tags.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use glint_syntax::ast::TagDefault;
use glint_syntax::{SyntaxKind, SyntaxNode};
use serde::Serialize;

use crate::checks::components::expand;
use crate::checks::tags::{Tag, TagSet, Tagger};
use crate::eval::Evaluator;
use crate::graph::{ModuleGraph, ModuleId};
use crate::model::SymbolKind;
use crate::sem::toks::flatten;
use crate::sem::{ClassRes, Kind, Resolved, Sem, SymRef, dummies_of};
use crate::view::{
    ComponentList, Region, TagClass, builtin_text, child, defined_type_ref, is_textually_tagged,
    read_tag, tokens, type_child,
};

const MAX_DEPTH: usize = 24;

/// The types of one module.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleTypes {
    pub module: String,
    pub tag_default: String,
    pub extensibility_implied: bool,
    pub types: Vec<TypeDefinition>,
}

/// A type assignment.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TypeDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<String>,
    pub ty: TypeNode,
}

/// How a tag was assigned (X.680 25.3, 31.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaggingMode {
    Explicit,
    Implicit,
}

/// One outer tag of a type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagInfo {
    pub class: String,
    pub number: u128,
}

/// A value after evaluation (`DEFAULT` values and named numbers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DefaultValue {
    Integer(i128),
    Boolean(bool),
    Null,
    /// An object identifier as dotted arcs.
    Oid(String),
    /// A bit string by the names of its set bits.
    Bits(Vec<String>),
    Enumerated(String),
    /// Any other value, as written.
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NamedValue {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<i128>,
}

/// A constraint in evaluated form, with the original syntax for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConstraintNode {
    pub kind: String,
    pub text: String,
    /// The permitted values of a numeric constraint as `(low, high)`; `MIN` and `MAX` stay symbolic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<(String, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<(String, String)>,
}

/// An open type tied to the table constraint that selects its concrete type (X.682 10.12-10.13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenTypeLink {
    pub class: String,
    pub field: String,
    pub object_set: String,
    /// The `@` paths of the component relation constraint.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub referenced: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComponentNode {
    pub name: String,
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<DefaultValue>,
    pub region: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u128>,
    pub ty: TypeNode,
}

/// A type with everything resolved about it.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct TypeNode {
    pub kind: String,
    /// The name of the referenced type for a reference: `Name` or `Module.Name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The effective outer tags; several for an untagged CHOICE, none for an open type.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagInfo>,
    /// Set when the node carries a tag of its own (textual or automatic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<TaggingMode>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub automatic_tag: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub extensible: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ComponentNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Box<TypeNode>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub named_values: Vec<NamedValue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<ConstraintNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_type: Option<OpenTypeLink>,
    /// The definition a reference stands for, when the model expands it (inside an instantiation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<Box<TypeNode>>,
    /// The actual parameters of a parameterized reference, as written.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actuals: Vec<String>,
    /// The parameterized definition expanded for these actuals (X.683 9.8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiation: Option<Box<TypeNode>>,
    /// Why the model stops here: macro notation, an unavailable module, an unevaluated feature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opaque: Option<String>,
}

impl TypeNode {
    fn of(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            ..Self::default()
        }
    }

    fn opaque(kind: &str, why: &str) -> Self {
        Self {
            kind: "Opaque".to_owned(),
            opaque: Some(format!("{kind}: {why}")),
            ..Self::default()
        }
    }
}

/// Builds the type model of `module` (every non-parameterized and parameterized type assignment).
pub fn resolve_module(graph: &ModuleGraph<'_>, module: ModuleId) -> Option<ModuleTypes> {
    let loaded = graph.module(module)?;
    let eval = Evaluator::new(graph);
    let sem = Sem::new(graph);
    let builder = Builder::new(graph, &eval, &sem);
    let mut types = Vec::new();
    for (index, symbol) in loaded.model.symbols.iter().enumerate() {
        let sym = SymRef { module, index };
        match symbol.kind {
            SymbolKind::Type if sem.kind(sym) == Kind::Type => {
                let Some(node) = &symbol.type_node else {
                    continue;
                };
                let env = Env::new(module);
                types.push(TypeDefinition {
                    name: symbol.name.clone(),
                    parameters: symbol.parameters.clone(),
                    ty: builder.build(node, &env, 0, false),
                });
            }
            SymbolKind::Macro | SymbolKind::Unknown => types.push(TypeDefinition {
                name: symbol.name.clone(),
                parameters: Vec::new(),
                ty: TypeNode::opaque("macro notation", "not modelled"),
            }),
            _ => {}
        }
    }
    Some(ModuleTypes {
        module: loaded.model.name.clone(),
        tag_default: match loaded.model.tag_default {
            TagDefault::Explicit => "EXPLICIT",
            TagDefault::Implicit => "IMPLICIT",
            TagDefault::Automatic => "AUTOMATIC",
        }
        .to_owned(),
        extensibility_implied: loaded.model.extensibility_implied,
        types,
    })
}

/// Builds the model of the type assignment `name` of `module`.
pub fn resolve_type(graph: &ModuleGraph<'_>, module: ModuleId, name: &str) -> Option<TypeNode> {
    let eval = Evaluator::new(graph);
    let sem = Sem::new(graph);
    let builder = Builder::new(graph, &eval, &sem);
    let symbol = graph.module(module)?.model.first_local(name)?;
    let node = symbol.type_node.as_ref()?;
    Some(builder.build(node, &Env::new(module), 0, true))
}

/// Where names are looked up and which dummy references stand for what.
#[derive(Clone)]
struct Env {
    module: ModuleId,
    subst: HashMap<String, TypeNode>,
    /// The normalized text of each actual parameter, to recognize an instantiation seen before (X.683 8.7).
    texts: HashMap<String, String>,
    dummies: HashSet<String>,
}

impl Env {
    fn new(module: ModuleId) -> Self {
        Self {
            module,
            subst: HashMap::new(),
            texts: HashMap::new(),
            dummies: HashSet::new(),
        }
    }
}

struct Builder<'a, 'g, 'c> {
    graph: &'g ModuleGraph<'c>,
    tagger: Tagger<'a, 'g, 'c>,
    sem: &'a Sem<'g, 'c>,
    memo: RefCell<HashMap<String, TypeNode>>,
    in_progress: RefCell<HashSet<String>>,
}

impl<'a, 'g, 'c> Builder<'a, 'g, 'c> {
    fn new(graph: &'g ModuleGraph<'c>, eval: &'a Evaluator<'g, 'c>, sem: &'a Sem<'g, 'c>) -> Self {
        Self {
            graph,
            tagger: Tagger { graph, eval },
            sem,
            memo: RefCell::new(HashMap::new()),
            in_progress: RefCell::new(HashSet::new()),
        }
    }

    fn tag_info(tag: &Tag) -> TagInfo {
        TagInfo {
            class: match tag.class {
                TagClass::Universal => "UNIVERSAL",
                TagClass::Application => "APPLICATION",
                TagClass::Context => "CONTEXT",
                TagClass::Private => "PRIVATE",
            }
            .to_owned(),
            number: tag.number,
        }
    }

    fn natural_tags(&self, module: ModuleId, ty: &SyntaxNode) -> Vec<TagInfo> {
        match self.tagger.tag_set(module, ty, 0) {
            TagSet::Known { tags, .. } => tags.iter().map(Self::tag_info).collect(),
            TagSet::Unknown => Vec::new(),
        }
    }

    // X.680 8.4, 25, 27, 29, 31; X.681 14; X.683 9.8
    fn build(&self, ty: &SyntaxNode, env: &Env, depth: usize, inline: bool) -> TypeNode {
        if depth > MAX_DEPTH {
            return TypeNode::opaque("depth", "nesting too deep");
        }
        let next = depth.saturating_add(1);
        let module = env.module;
        match ty.kind() {
            SyntaxKind::TaggedType => self.tagged(ty, env, next, inline),
            SyntaxKind::EncodingPrefixedType => type_child(ty).map_or_else(
                || TypeNode::opaque("encoding prefix", "empty"),
                |inner| self.build(&inner, env, next, inline),
            ),
            SyntaxKind::ConstrainedType => {
                let Some(inner) = type_child(ty) else {
                    return TypeNode::opaque("constraint", "no type");
                };
                let mut node = self.build(&inner, env, next, inline);
                for constraint in ty.children().filter(|c| c.kind() == SyntaxKind::Constraint) {
                    node.constraints.push(self.constraint(module, &constraint));
                    if let Some(link) = self.open_type_link(ty, &constraint, env) {
                        node.open_type = Some(link);
                    }
                    // X.682 11.1: `OCTET STRING (CONTAINING Class.&Type({Set}{@id}))` carries the open type
                    for spec in constraint
                        .children()
                        .filter(|c| c.kind() == SyntaxKind::ContentsConstraint)
                    {
                        let Some(contained) = type_child(&spec) else {
                            continue;
                        };
                        for inner_constraint in contained
                            .children()
                            .filter(|c| c.kind() == SyntaxKind::Constraint)
                        {
                            if let Some(link) =
                                self.open_type_link(&contained, &inner_constraint, env)
                            {
                                node.open_type = Some(link);
                            }
                        }
                    }
                }
                node
            }
            SyntaxKind::BuiltinType => {
                let text = builtin_text(ty);
                let mut node = TypeNode::of(&text);
                node.tags = self.natural_tags(module, ty);
                node
            }
            SyntaxKind::IntegerType | SyntaxKind::EnumeratedType | SyntaxKind::BitStringType => {
                let mut node = TypeNode::of(match ty.kind() {
                    SyntaxKind::IntegerType => "INTEGER",
                    SyntaxKind::EnumeratedType => "ENUMERATED",
                    _ => "BIT STRING",
                });
                node.tags = self.natural_tags(module, ty);
                node.named_values = self.named_values(module, ty);
                node.extensible = ty
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                    .any(|t| t.kind() == SyntaxKind::Ellipsis);
                node
            }
            SyntaxKind::SequenceType | SyntaxKind::SetType | SyntaxKind::ChoiceType => {
                self.record(ty, env, next, inline)
            }
            SyntaxKind::SequenceOfType | SyntaxKind::SetOfType => {
                let mut node = TypeNode::of(if ty.kind() == SyntaxKind::SequenceOfType {
                    "SEQUENCE OF"
                } else {
                    "SET OF"
                });
                node.tags = self.natural_tags(module, ty);
                if let Some(element) = crate::view::of_element_type(ty) {
                    node.element = Some(Box::new(self.build(&element, env, next, inline)));
                }
                node
            }
            SyntaxKind::InstanceOfType => self.instance_of(ty, env, next),
            SyntaxKind::DefinedType => self.defined(ty, env, next, inline),
            SyntaxKind::ObjectClassFieldType => self.class_field(ty, env, next),
            SyntaxKind::SelectionType => {
                let mut node = TypeNode::of("Selection");
                node.reference = Some(
                    flatten(ty)
                        .iter()
                        .map(|t| t.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                node.tags = self.natural_tags(module, ty);
                node
            }
            SyntaxKind::AnyType => TypeNode::opaque("ANY", "1988 open type"),
            SyntaxKind::TypeFromObject => TypeNode::opaque("TypeFromObject", "not modelled"),
            _ => TypeNode::opaque("type", "not modelled"),
        }
    }

    // X.680 31.2.7
    fn tagged(&self, ty: &SyntaxNode, env: &Env, depth: usize, inline: bool) -> TypeNode {
        let Some(inner) = type_child(ty) else {
            return TypeNode::opaque("tag", "no type");
        };
        let mut node = self.build(&inner, env, depth, inline);
        let Some(tag_node) = child(ty, SyntaxKind::Tag) else {
            return node;
        };
        let decl = read_tag(&tag_node);
        let number = decl.number.or_else(|| {
            let (qualifier, name) = decl.reference.as_ref()?;
            let value = self
                .tagger
                .eval
                .integer_of_name(env.module, qualifier.as_deref(), name)?;
            u128::try_from(value).ok()
        });
        let keyword = tokens(ty)
            .into_iter()
            .find(|t| {
                t.kind() == SyntaxKind::Keyword && matches!(t.text(), "IMPLICIT" | "EXPLICIT")
            })
            .map(|t| t.text().to_owned());
        let default_implicit = self.graph.module(env.module).is_some_and(|m| {
            matches!(
                m.model.tag_default,
                TagDefault::Implicit | TagDefault::Automatic
            )
        });
        let dynamic = self.is_dynamic(&inner, env);
        let implicit = match keyword.as_deref() {
            Some("IMPLICIT") => true,
            Some(_) => false,
            None => default_implicit,
        } && !dynamic;
        node.mode = Some(if implicit {
            TaggingMode::Implicit
        } else {
            TaggingMode::Explicit
        });
        if let Some(number) = number {
            node.tags = vec![TagInfo {
                class: Self::tag_info(&Tag {
                    class: decl.class,
                    number,
                })
                .class,
                number,
            }];
        }
        node
    }

    /// An untagged CHOICE, open type or dummy reference is always explicitly tagged (X.680 31.2.7 c).
    fn is_dynamic(&self, inner: &SyntaxNode, env: &Env) -> bool {
        let mut node = inner.clone();
        loop {
            match node.kind() {
                SyntaxKind::ConstrainedType | SyntaxKind::EncodingPrefixedType => {
                    match type_child(&node) {
                        Some(next) => node = next,
                        None => return false,
                    }
                }
                SyntaxKind::ChoiceType | SyntaxKind::AnyType => return true,
                SyntaxKind::ObjectClassFieldType => {
                    return child(&node, SyntaxKind::FieldName)
                        .and_then(|f| tokens(&f).into_iter().last())
                        .is_some_and(|t| t.kind() == SyntaxKind::FieldUpper);
                }
                SyntaxKind::DefinedType => {
                    let Some(reference) = defined_type_ref(&node) else {
                        return false;
                    };
                    if reference.module.is_none() && env.dummies.contains(&reference.name) {
                        return true;
                    }
                    return self.graph.peel_type(env.module, &node).is_some_and(|p| {
                        matches!(p.node.kind(), SyntaxKind::ChoiceType | SyntaxKind::AnyType)
                    });
                }
                _ => return false,
            }
        }
    }

    // X.680 25, 27, 29
    fn record(&self, ty: &SyntaxNode, env: &Env, depth: usize, inline: bool) -> TypeNode {
        let module = env.module;
        let is_choice = ty.kind() == SyntaxKind::ChoiceType;
        let mut node = TypeNode::of(match ty.kind() {
            SyntaxKind::SequenceType => "SEQUENCE",
            SyntaxKind::SetType => "SET",
            _ => "CHOICE",
        });
        node.tags = self.natural_tags(module, ty);
        let list = ComponentList::read(ty);
        node.extensible = list.is_extensible()
            || self
                .graph
                .module(module)
                .is_some_and(|m| m.model.extensibility_implied);
        let flat = expand(self.graph, module, &list, 0);
        let automatic = self
            .graph
            .module(module)
            .is_some_and(|m| m.model.is_automatic())
            && !flat.iter().any(|f| {
                !f.expanded
                    && f.component.region != Region::Addition
                    && is_textually_tagged(&f.component.ty)
            });
        for (index, entry) in flat.iter().enumerate() {
            let component = &entry.component;
            let sub_env = if entry.module == module {
                env.clone()
            } else {
                Env::new(entry.module)
            };
            let mut ty_node = self.build(&component.ty, &sub_env, depth, inline);
            if automatic && ty_node.mode.is_none() {
                let dynamic = self.is_dynamic(&component.ty, &sub_env);
                ty_node.tags = vec![TagInfo {
                    class: "CONTEXT".to_owned(),
                    number: u128::try_from(index).unwrap_or_default(),
                }];
                ty_node.mode = Some(if dynamic {
                    TaggingMode::Explicit
                } else {
                    TaggingMode::Implicit
                });
                ty_node.automatic_tag = true;
            }
            let default = component
                .default
                .as_ref()
                .map(|value| self.default_value(entry.module, &component.ty, value));
            let group = component.group;
            let version = group
                .and_then(|g| list.groups.get(g))
                .and_then(|g| g.version.map(|(v, _)| v));
            node.components.push(ComponentNode {
                name: component.name.clone(),
                optional: component.optional,
                default,
                region: match component.region {
                    Region::Root => "root",
                    Region::Addition => "addition",
                    Region::TrailingRoot => "trailing-root",
                }
                .to_owned(),
                group,
                version,
                ty: ty_node,
            });
        }
        let _ = is_choice;
        node
    }

    fn named_values(&self, module: ModuleId, ty: &SyntaxNode) -> Vec<NamedValue> {
        let mut out = Vec::new();
        for item in ty.descendants() {
            if !matches!(
                item.kind(),
                SyntaxKind::NamedNumber | SyntaxKind::Enumeration
            ) {
                continue;
            }
            let Some(name) = tokens(&item)
                .into_iter()
                .find(|t| t.kind() == SyntaxKind::LowerName)
            else {
                continue;
            };
            let value = child(&item, SyntaxKind::Value)
                .and_then(|v| self.tagger.eval.eval_integer(module, &v));
            out.push(NamedValue {
                name: name.text().to_owned(),
                value,
            });
        }
        out
    }

    // X.680 14.9, 17.6; evaluated forms of DEFAULT values
    fn default_value(&self, module: ModuleId, ty: &SyntaxNode, value: &SyntaxNode) -> DefaultValue {
        let toks = flatten(value);
        let raw = toks
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let peeled = self.graph.peel_type_through_tags(module, ty);
        let kind = peeled.as_ref().map(|p| p.node.kind());
        match (kind, toks.as_slice()) {
            (_, [only]) if only.text == "TRUE" => DefaultValue::Boolean(true),
            (_, [only]) if only.text == "FALSE" => DefaultValue::Boolean(false),
            (_, [only]) if only.text == "NULL" => DefaultValue::Null,
            (Some(SyntaxKind::EnumeratedType), [only]) if only.kind == SyntaxKind::LowerName => {
                DefaultValue::Enumerated(only.text.clone())
            }
            (Some(SyntaxKind::BitStringType), _) => match child(value, SyntaxKind::BraceGroup) {
                Some(group) => DefaultValue::Bits(
                    crate::sem::toks::inner(&group)
                        .into_iter()
                        .filter(|t| t.kind == SyntaxKind::LowerName)
                        .map(|t| t.text)
                        .collect(),
                ),
                None => DefaultValue::Raw(raw),
            },
            (Some(SyntaxKind::IntegerType), _) | (Some(SyntaxKind::BuiltinType), [_]) => self
                .tagger
                .eval
                .eval_integer(module, value)
                .map_or(DefaultValue::Raw(raw), DefaultValue::Integer),
            _ => {
                if let Some(group) = child(value, SyntaxKind::BraceGroup)
                    && kind.is_some_and(|k| k == SyntaxKind::BuiltinType)
                    && peeled
                        .as_ref()
                        .is_some_and(|p| builtin_text(&p.node) == "OBJECT IDENTIFIER")
                    && let Some(oid) = self.tagger.eval.eval_oid_group(module, &group).oid
                {
                    return DefaultValue::Oid(oid.to_string());
                }
                DefaultValue::Raw(raw)
            }
        }
    }

    // X.680 49-53 in evaluated form
    fn constraint(&self, module: ModuleId, constraint: &SyntaxNode) -> ConstraintNode {
        let text = flatten(constraint)
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let spec = constraint.children().next();
        let kind_of = |n: &SyntaxNode| match n.kind() {
            SyntaxKind::TableConstraint => "table",
            SyntaxKind::ContentsConstraint => "contents",
            SyntaxKind::UserDefinedConstraint => "user-defined",
            SyntaxKind::ValueElement => "single value",
            SyntaxKind::RangeElement => "range",
            SyntaxKind::SizeElement => "size",
            SyntaxKind::FromElement => "permitted alphabet",
            SyntaxKind::PatternElement => "pattern",
            SyntaxKind::InnerTypeElement => "inner type",
            SyntaxKind::ContainedSubtype => "contained subtype",
            SyntaxKind::UnionSet | SyntaxKind::IntersectionSet | SyntaxKind::ExclusionSet => {
                "set arithmetic"
            }
            _ => "constraint",
        };
        let kind = spec.as_ref().map_or("constraint", kind_of).to_owned();
        let mut out = ConstraintNode {
            kind,
            text,
            range: None,
            size: None,
        };
        if let Some(spec) = spec {
            match spec.kind() {
                SyntaxKind::RangeElement => out.range = self.bounds(module, &spec),
                SyntaxKind::ValueElement => {
                    if let Some(value) = child(&spec, SyntaxKind::Value)
                        && let Some(n) = self.tagger.eval.eval_integer(module, &value)
                    {
                        out.range = Some((n.to_string(), n.to_string()));
                    }
                }
                SyntaxKind::SizeElement => {
                    if let Some(inner) = child(&spec, SyntaxKind::Constraint)
                        && let Some(first) = inner.children().next()
                    {
                        out.size = match first.kind() {
                            SyntaxKind::RangeElement => self.bounds(module, &first),
                            SyntaxKind::ValueElement => child(&first, SyntaxKind::Value)
                                .and_then(|v| self.tagger.eval.eval_integer(module, &v))
                                .map(|n| (n.to_string(), n.to_string())),
                            _ => None,
                        };
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn bounds(&self, module: ModuleId, range: &SyntaxNode) -> Option<(String, String)> {
        let values: Vec<SyntaxNode> = range
            .children()
            .filter(|c| c.kind() == SyntaxKind::Value)
            .collect();
        let render = |v: &SyntaxNode| -> String {
            self.tagger.eval.eval_integer(module, v).map_or_else(
                || {
                    flatten(v)
                        .iter()
                        .map(|t| t.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                },
                |n| n.to_string(),
            )
        };
        match values.as_slice() {
            [low, high] => Some((render(low), render(high))),
            _ => None,
        }
    }

    // X.682 10.12-10.13
    fn open_type_link(
        &self,
        constrained: &SyntaxNode,
        constraint: &SyntaxNode,
        env: &Env,
    ) -> Option<OpenTypeLink> {
        let table = child(constraint, SyntaxKind::TableConstraint)?;
        let inner = type_child(constrained)?;
        if inner.kind() != SyntaxKind::ObjectClassFieldType {
            return None;
        }
        let names: Vec<String> = tokens(&inner)
            .into_iter()
            .filter(|t| t.kind() == SyntaxKind::UpperName)
            .map(|t| t.text().to_owned())
            .collect();
        let class = names.last()?.clone();
        let field = child(&inner, SyntaxKind::FieldName)?;
        let field_text = flatten(&field)
            .iter()
            .map(|t| t.text.as_str())
            .collect::<String>();
        let mut groups = table
            .children()
            .filter(|c| c.kind() == SyntaxKind::BraceGroup);
        let set = groups.next()?;
        let object_set = crate::sem::toks::inner(&set)
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let referenced = groups
            .next()
            .map(|g| {
                crate::sem::toks::split_commas(&crate::sem::toks::inner(&g))
                    .iter()
                    .map(|entry| entry.iter().map(|t| t.text.as_str()).collect::<String>())
                    .collect()
            })
            .unwrap_or_default();
        let _ = env;
        Some(OpenTypeLink {
            class,
            field: field_text,
            object_set,
            referenced,
        })
    }

    // X.681 14
    fn class_field(&self, ty: &SyntaxNode, env: &Env, depth: usize) -> TypeNode {
        let names: Vec<String> = tokens(ty)
            .into_iter()
            .filter(|t| t.kind() == SyntaxKind::UpperName)
            .map(|t| t.text().to_owned())
            .collect();
        let field = child(ty, SyntaxKind::FieldName)
            .map(|f| {
                flatten(&f)
                    .iter()
                    .map(|t| t.text.as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();
        let reference = format!("{}.{}", names.last().map_or("", String::as_str), field);
        let mut node = TypeNode::of("ObjectClassFieldType");
        node.reference = Some(reference);
        let last_is_type_field = child(ty, SyntaxKind::FieldName)
            .and_then(|f| tokens(&f).into_iter().last())
            .is_some_and(|t| t.kind() == SyntaxKind::FieldUpper);
        if last_is_type_field {
            node.kind = "OpenType".to_owned();
            return node;
        }
        // a fixed-type value field denotes the type written in the class (X.681 14.3)
        let (qualifier, name) = match names.as_slice() {
            [module, name, ..] => (Some(module.as_str()), name.as_str()),
            [name] => (None, name.as_str()),
            [] => return node,
        };
        let scope = self.sem.scope_of(ty);
        if let Resolved::Sym(target) = self.sem.resolve(env.module, &scope, qualifier, name)
            && let ClassRes::Class(class) = self.sem.class_res_of_symbol(target)
            && let Some(info) = self.sem.class_info(class)
            && let Some(f) = child(ty, SyntaxKind::FieldName)
                .and_then(|f| tokens(&f).into_iter().next().map(|t| t.text().to_owned()))
            && let Some(field_info) = info.field(&f)
            && field_info.kind == crate::sem::FieldKind::FixedValue
            && let Some(governor) = &field_info.governor
        {
            let class_env = Env::new(class.module);
            let mut resolved = self.build(governor, &class_env, depth, false);
            resolved.reference = node.reference;
            return resolved;
        }
        node
    }

    // X.681 Annex C.6, X.682 A.2
    fn instance_of(&self, ty: &SyntaxNode, env: &Env, depth: usize) -> TypeNode {
        let class = type_child(ty)
            .map(|c| self.build(&c, env, depth, false))
            .and_then(|n| n.reference);
        let mut node = TypeNode::of("SEQUENCE");
        node.reference = Some(format!("INSTANCE OF {}", class.as_deref().unwrap_or("?")));
        node.tags = vec![TagInfo {
            class: "UNIVERSAL".to_owned(),
            number: 8,
        }];
        let mut type_id = TypeNode::of("OBJECT IDENTIFIER");
        type_id.tags = vec![TagInfo {
            class: "UNIVERSAL".to_owned(),
            number: 6,
        }];
        let mut value = TypeNode::of("OpenType");
        value.mode = Some(TaggingMode::Explicit);
        value.tags = vec![TagInfo {
            class: "CONTEXT".to_owned(),
            number: 0,
        }];
        node.components = vec![
            ComponentNode {
                name: "type-id".to_owned(),
                optional: false,
                default: None,
                region: "root".to_owned(),
                group: None,
                version: None,
                ty: type_id,
            },
            ComponentNode {
                name: "value".to_owned(),
                optional: false,
                default: None,
                region: "root".to_owned(),
                group: None,
                version: None,
                ty: value,
            },
        ];
        node
    }

    // X.680 17, X.683 9
    fn defined(&self, ty: &SyntaxNode, env: &Env, depth: usize, inline: bool) -> TypeNode {
        let Some(reference) = defined_type_ref(ty) else {
            return TypeNode::opaque("reference", "unreadable");
        };
        if reference.module.is_none() {
            if let Some(actual) = env.subst.get(&reference.name) {
                return actual.clone();
            }
            if env.dummies.contains(&reference.name) {
                return TypeNode {
                    kind: "Dummy".to_owned(),
                    reference: Some(reference.name),
                    ..TypeNode::default()
                };
            }
        }
        let display = match &reference.module {
            Some(module) => format!("{module}.{}", reference.name),
            None => reference.name.clone(),
        };
        let mut node = TypeNode::of("Reference");
        node.reference = Some(display.clone());
        let scope = self.sem.scope_of(ty);
        let resolved = self.sem.resolve(
            env.module,
            &scope,
            reference.module.as_deref(),
            &reference.name,
        );
        let Resolved::Sym(target) = resolved else {
            node.opaque = Some("the defining module is not available".to_owned());
            return node;
        };
        let Some(symbol) = self.sem.symbol(target) else {
            return node;
        };
        node.tags = self.natural_tags(env.module, ty);
        let actual_list = child(ty, SyntaxKind::ActualParameterList);
        if let Some(list) = actual_list
            && symbol.is_parameterized()
        {
            let actuals: Vec<Vec<crate::sem::toks::Tk>> = list
                .children()
                .filter(|c| c.kind() == SyntaxKind::ActualParameter)
                .map(|c| flatten(&c))
                .collect();
            node.actuals = actuals
                .iter()
                .map(|a| {
                    a.iter()
                        .map(|t| t.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            node.instantiation = self.instantiate(env, target, &actuals, depth).map(Box::new);
            return node;
        }
        if inline
            && !symbol.is_parameterized()
            && symbol.kind == SymbolKind::Type
            && self.sem.kind(target) == Kind::Type
            && let Some(rhs) = &symbol.type_node
        {
            let key = format!("expand:{}:{}", target.module.0, target.index);
            if self.in_progress.borrow_mut().insert(key.clone()) {
                node.expanded = Some(Box::new(self.build(
                    rhs,
                    &Env::new(target.module),
                    depth,
                    false,
                )));
                self.in_progress.borrow_mut().remove(&key);
            }
        }
        node
    }

    // X.683 9.7, 9.8: the actuals keep the tagging environment of the module where they are written
    fn instantiate(
        &self,
        caller: &Env,
        target: SymRef,
        actuals: &[Vec<crate::sem::toks::Tk>],
        depth: usize,
    ) -> Option<TypeNode> {
        let symbol = self.sem.symbol(target)?;
        let dummies = dummies_of(&symbol.node);
        let normalized: Vec<String> = actuals
            .iter()
            .map(|a| {
                a.iter()
                    .map(|t| {
                        caller
                            .texts
                            .get(&t.text)
                            .map_or(t.text.as_str(), String::as_str)
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let key = format!(
            "{}:{}:{}:{}",
            target.module.0,
            target.index,
            caller.module.0,
            normalized.join(",")
        );
        if let Some(found) = self.memo.borrow().get(&key) {
            return Some(found.clone());
        }
        if !self.in_progress.borrow_mut().insert(key.clone()) {
            return Some(TypeNode {
                kind: "Recursive".to_owned(),
                reference: Some(symbol.name.clone()),
                ..TypeNode::default()
            });
        }
        let mut env = Env::new(target.module);
        for (dummy, text) in dummies.iter().zip(&normalized) {
            env.dummies.insert(dummy.name.clone());
            env.texts.insert(dummy.name.clone(), text.clone());
        }
        let source = self.sem.source_text(caller.module).to_owned();
        let options = self.sem.parse_options(caller.module);
        for (dummy, actual) in dummies.iter().zip(actuals) {
            if dummy.governor.is_some() || dummy.governor_name.is_some() {
                continue;
            }
            let (Some(first), Some(last)) = (actual.first(), actual.last()) else {
                continue;
            };
            let parsed = glint_syntax::parse_prefix(
                &source,
                usize::from(first.range.start()),
                usize::from(last.range.end()),
                glint_syntax::PrefixKind::Type,
                options,
            );
            if !parsed.ok {
                continue;
            }
            let root = parsed.parse.syntax();
            if let Some(actual_type) = root
                .children()
                .find(|n| crate::view::is_type_node(n.kind()))
            {
                let built = self.build(&actual_type, caller, depth, true);
                env.subst.insert(dummy.name.clone(), built);
            }
        }
        let result = symbol
            .type_node
            .as_ref()
            .map(|rhs| self.build(rhs, &env, depth, false));
        self.in_progress.borrow_mut().remove(&key);
        if let Some(done) = &result {
            self.memo.borrow_mut().insert(key, done.clone());
        }
        result
    }
}
