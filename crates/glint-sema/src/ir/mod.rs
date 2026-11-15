//! Lowering into `glint-ir`: the closure of one root type with tags resolved, parameterized types monomorphized,
//! constraints evaluated and the object sets that bind open types lowered to tables (Codegen and Decoder Design,
//! "The IR").
//!
//! The lowering follows the type model's builder ([`crate::typemodel`]) for tags and components, so the two never
//! disagree about tags; it differs in keeping references as type ids and in evaluating what the type model only
//! displays.

mod constraint;
mod objects;
mod value;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use glint_ir::{
    Collection, Component, IrSet, Kind, NamedNumber, Note, ObjectSetId, ObjectSetTable,
    OidConstant, OpenType, Origin, Presence, Record, Region, StringKind, Tag, TagClass, TagLayer,
    TagMode, TimeKind, Type, TypeDef, TypeId, Value,
};
use glint_syntax::ast::TagDefault;
use glint_syntax::{PrefixKind, SyntaxKind, SyntaxNode, TextRange};

use crate::checks::components::expand;
use crate::eval::Evaluator;
use crate::graph::{ModuleGraph, ModuleId, Origin as ModuleOrigin};
use crate::legacy::{is_legacy_string_module, legacy_string_definition};
use crate::sem::toks::flatten;
use crate::sem::{ClassRes, FieldKind, Kind as SymKind, Resolved, Sem, SymRef, dummies_of};
use crate::view::{
    ComponentList, Region as ViewRegion, TagClass as ViewTagClass, builtin_text, child,
    defined_type_ref, is_textually_tagged, is_type_node, read_tag, tokens, type_child,
};

/// How deep inline types may nest, and how many instantiations may be open at once.
const MAX_DEPTH: usize = 48;

/// Lowers the type assignment `name` of `module` and everything it reaches.
pub fn lower(graph: &ModuleGraph<'_>, module: ModuleId, name: &str) -> Option<IrSet> {
    let loaded = graph.module(module)?;
    let index = *loaded.model.local(name).first()?;
    let eval = Evaluator::new(graph);
    let sem = Sem::new(graph);
    let mut lowerer = Lowerer::new(graph, &eval, &sem);
    let root = lowerer.named(SymRef { module, index }, Vec::new());
    let mut set = IrSet {
        version: glint_ir::IR_VERSION,
        root,
        types: lowerer.types,
        object_sets: lowerer.sets,
        oids: lowerer.oids.into_values().collect(),
        notes: lowerer.notes,
    };
    glint_ir::summarize(&mut set);
    Some(set)
}

/// Lowers the object set assignment `name` of `module` to a table, with the types its rows reach. The root of the set
/// is a placeholder definition named after the object set; the table is the returned id.
pub fn lower_object_set(
    graph: &ModuleGraph<'_>,
    module: ModuleId,
    name: &str,
) -> Option<(IrSet, ObjectSetId)> {
    let loaded = graph.module(module)?;
    let index = *loaded.model.local(name).first()?;
    let sym = SymRef { module, index };
    let eval = Evaluator::new(graph);
    let sem = Sem::new(graph);
    if sem.kind(sym) != SymKind::ObjectSet {
        return None;
    }
    let ClassRes::Class(class) = sem.governor_class(sym) else {
        return None;
    };
    let group = sem
        .symbol(sym)
        .and_then(|s| s.value_node.as_ref())
        .and_then(|v| crate::view::child(v, SyntaxKind::BraceGroup))?;
    let mut lowerer = Lowerer::new(graph, &eval, &sem);
    lowerer.at = format!("{}.{name}", lowerer.module_name(module));
    let root = lowerer.placeholder(module, name, Type::opaque("an object set"));
    let table = lowerer.object_set(&group, &Env::new(module), Some(class))?;
    let mut set = IrSet {
        version: glint_ir::IR_VERSION,
        root,
        types: lowerer.types,
        object_sets: lowerer.sets,
        oids: lowerer.oids.into_values().collect(),
        notes: lowerer.notes,
    };
    glint_ir::summarize(&mut set);
    Some((set, table))
}

/// A row of the decoder's OID hint table: the object set `set` of the module binds `field` of `class` to `ty` for the
/// objects keyed by `oid` (Codegen and Decoder Design, "OID hints").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintRow {
    pub oid: String,
    pub class: String,
    pub field: String,
    pub set: String,
    /// The type, briefly: a definition name or a built-in type.
    pub ty: String,
}

/// The hint rows of every object set assignment of `module` whose class is keyed by an object identifier.
pub fn oid_hints(graph: &ModuleGraph<'_>, module: ModuleId) -> Vec<HintRow> {
    let Some(loaded) = graph.module(module) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for symbol in &loaded.model.symbols {
        let Some((set, table)) = lower_object_set(graph, module, &symbol.name) else {
            continue;
        };
        let Some(table) = set.object_set(table) else {
            continue;
        };
        for row in &table.rows {
            let Some(Value::Oid(oid)) = &row.key else {
                continue;
            };
            for (field, ty) in &row.types {
                if matches!(ty.kind, Kind::Opaque { .. }) {
                    continue;
                }
                rows.push(HintRow {
                    oid: oid.clone(),
                    class: table.class.clone(),
                    field: field.clone(),
                    set: symbol.name.clone(),
                    ty: brief(&set, ty),
                });
            }
        }
    }
    rows.sort_by(|a, b| {
        (&a.oid, &a.class, &a.field, &a.set).cmp(&(&b.oid, &b.class, &b.field, &b.set))
    });
    rows.dedup_by(|a, b| {
        (&a.oid, &a.class, &a.field, &a.ty) == (&b.oid, &b.class, &b.field, &b.ty)
    });
    rows
}

/// A type in a few words: the name it refers to, or its built-in kind.
fn brief(set: &IrSet, ty: &Type) -> String {
    match &ty.kind {
        Kind::Reference { target } => set
            .get(*target)
            .map_or_else(|| "?".to_owned(), |def| def.origin.asn1_name.clone()),
        Kind::Boolean => "BOOLEAN".to_owned(),
        Kind::Integer { .. } => "INTEGER".to_owned(),
        Kind::Enumerated { .. } => "ENUMERATED".to_owned(),
        Kind::Real => "REAL".to_owned(),
        Kind::Null => "NULL".to_owned(),
        Kind::BitString { .. } => "BIT STRING".to_owned(),
        Kind::OctetString => "OCTET STRING".to_owned(),
        Kind::ObjectIdentifier => "OBJECT IDENTIFIER".to_owned(),
        Kind::RelativeOid => "RELATIVE-OID".to_owned(),
        Kind::String { string } => format!("{string:?}"),
        Kind::Time { time } => format!("{time:?}"),
        Kind::Sequence(_) => "SEQUENCE".to_owned(),
        Kind::Set(_) => "SET".to_owned(),
        Kind::Choice(_) => "CHOICE".to_owned(),
        Kind::SequenceOf(c) => format!("SEQUENCE OF {}", brief(set, &c.element)),
        Kind::SetOf(c) => format!("SET OF {}", brief(set, &c.element)),
        Kind::OpenType(_) => "open type".to_owned(),
        Kind::Other { name } => name.clone(),
        Kind::Opaque { .. } => "opaque".to_owned(),
    }
}

/// What a dummy reference stands for inside one instantiation (X.683 9).
#[derive(Debug, Clone)]
enum Actual {
    Type(Box<Type>),
    Value(Value),
    ObjectSet(ObjectSetId),
    Class(SymRef),
    /// Could not be read; uses of the dummy become opaque.
    Unknown(String),
}

/// Where names are looked up, and what the dummies in scope stand for.
#[derive(Clone)]
struct Env {
    module: ModuleId,
    actuals: Rc<HashMap<String, Actual>>,
    /// The actual parameters as written in the caller's terms, for display and naming.
    texts: Rc<HashMap<String, String>>,
}

impl Env {
    fn new(module: ModuleId) -> Self {
        Self {
            module,
            actuals: Rc::new(HashMap::new()),
            texts: Rc::new(HashMap::new()),
        }
    }

    fn actual(&self, name: &str) -> Option<&Actual> {
        self.actuals.get(name)
    }
}

/// What the governing type of a value is, so the value can be read in its terms.
#[derive(Debug, Clone, Default)]
enum Hint {
    #[default]
    None,
    Integer(Vec<NamedNumber>),
    Enumerated,
    Bits,
    Oid,
    String,
    /// A `SEQUENCE` or `SET`, resolved, for values written by component.
    Record(Box<Type>),
}

struct Lowerer<'a, 'g, 'c> {
    graph: &'g ModuleGraph<'c>,
    eval: &'a Evaluator<'g, 'c>,
    sem: &'a Sem<'g, 'c>,
    types: Vec<TypeDef>,
    memo: HashMap<String, TypeId>,
    names: HashSet<(String, String)>,
    sets: Vec<ObjectSetTable>,
    set_memo: HashMap<String, ObjectSetId>,
    oids: BTreeMap<(String, String), OidConstant>,
    notes: Vec<Note>,
    /// The type definition being lowered, for notes.
    at: String,
    open: usize,
    /// How many value references are being followed, against cycles.
    values_open: usize,
}

impl<'a, 'g, 'c> Lowerer<'a, 'g, 'c> {
    fn new(graph: &'g ModuleGraph<'c>, eval: &'a Evaluator<'g, 'c>, sem: &'a Sem<'g, 'c>) -> Self {
        Self {
            graph,
            eval,
            sem,
            types: Vec::new(),
            memo: HashMap::new(),
            names: HashSet::new(),
            sets: Vec::new(),
            set_memo: HashMap::new(),
            oids: BTreeMap::new(),
            notes: Vec::new(),
            at: String::new(),
            open: 0,
            values_open: 0,
        }
    }

    fn note(&mut self, message: impl Into<String>) {
        let note = Note {
            at: self.at.clone(),
            message: message.into(),
        };
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    fn opaque(&mut self, reason: impl Into<String>) -> Type {
        let reason = reason.into();
        self.note(reason.clone());
        Type::opaque(reason)
    }

    fn module_name(&self, module: ModuleId) -> String {
        self.graph
            .module(module)
            .map_or_else(String::new, |m| m.model.name.clone())
    }

    fn origin(&self, module: ModuleId, asn1_name: &str, actuals: Vec<String>) -> Origin {
        let loaded = self.graph.module(module);
        let document = loaded.and_then(|m| match m.origin {
            ModuleOrigin::Corpus { record } => self
                .graph
                .corpus()
                .modules()
                .get(record)
                .map(|r| r.source.document.clone()),
            // a corpus module checked as a draft keeps its document
            ModuleOrigin::Draft => self.graph.options().home_document.clone(),
            ModuleOrigin::Builtin => None,
        });
        Origin {
            module: self.module_name(module),
            module_oid: loaded.and_then(|m| m.oid.as_ref()).map(ToString::to_string),
            document,
            asn1_name: asn1_name.to_owned(),
            actuals,
        }
    }

    /// Parses the type, value or element sets written at `range` of `module`'s source (a setting, an actual
    /// parameter or the body of a value set).
    fn parse_at(&self, module: ModuleId, range: TextRange, kind: PrefixKind) -> Option<SyntaxNode> {
        let source = self.sem.source_text(module);
        let parsed = glint_syntax::parse_prefix(
            source,
            usize::from(range.start()),
            usize::from(range.end()),
            kind,
            self.sem.parse_options(module),
        );
        if !parsed.ok {
            return None;
        }
        let root = parsed.parse.syntax();
        root.children().find(|n| match kind {
            PrefixKind::Type => is_type_node(n.kind()),
            PrefixKind::Value => n.kind() == SyntaxKind::Value,
            PrefixKind::ElementSet => n.kind() == SyntaxKind::Constraint,
        })
    }

    // ---- named types ------------------------------------------------------------------------

    /// The type id of the assignment `sym` instantiated with `actuals` (none for a plain type assignment), lowered on
    /// first use (X.683 9.8: one definition per distinct instantiation).
    fn named(&mut self, sym: SymRef, actuals: Vec<(String, Actual)>) -> TypeId {
        let key = format!(
            "{}:{}|{}",
            sym.module.0,
            sym.index,
            actuals
                .iter()
                .map(|(_, a)| actual_key(a))
                .collect::<Vec<_>>()
                .join("|")
        );
        if let Some(&id) = self.memo.get(&key) {
            return id;
        }
        let Some(symbol) = self.sem.symbol(sym) else {
            return self.placeholder(sym.module, "?", Type::opaque("unknown symbol"));
        };
        let texts: Vec<String> = actuals.iter().map(|(text, _)| text.clone()).collect();
        let name = self.unique_name(sym.module, &symbol.name, &actuals);
        let id = self.push(TypeDef {
            name: name.clone(),
            origin: self.origin(sym.module, &symbol.name, texts),
            ty: Type::opaque("being lowered"),
        });
        self.memo.insert(key, id);
        let at = format!("{}.{name}", self.module_name(sym.module));
        let outer = std::mem::replace(&mut self.at, at);
        let ty = if self.open >= MAX_DEPTH {
            self.opaque("instantiations nest too deeply")
        } else if let Some(node) = symbol.type_node.clone() {
            self.open = self.open.saturating_add(1);
            let dummies = dummies_of(&symbol.node);
            let mut map = HashMap::new();
            let mut text_map = HashMap::new();
            for (dummy, (text, actual)) in dummies.iter().zip(actuals) {
                text_map.insert(dummy.name.clone(), text);
                map.insert(dummy.name.clone(), actual);
            }
            let env = Env {
                module: sym.module,
                actuals: Rc::new(map),
                texts: Rc::new(text_map),
            };
            let ty = match self.sem.kind(sym) {
                SymKind::Type => self.ty(&node, &env, 0),
                SymKind::ValueSet => self.value_set_type(sym, &env),
                SymKind::Macro => self.opaque("macro notation"),
                other => self.opaque(format!("`{}` is not a type ({other:?})", symbol.name)),
            };
            self.open = self.open.saturating_sub(1);
            ty
        } else {
            self.opaque(format!("`{}` has no type", symbol.name))
        };
        self.at = outer;
        if let Some(def) = usize::try_from(id.0)
            .ok()
            .and_then(|i| self.types.get_mut(i))
        {
            def.ty = ty;
        }
        id
    }

    fn push(&mut self, def: TypeDef) -> TypeId {
        let id = TypeId(u32::try_from(self.types.len()).unwrap_or(u32::MAX));
        self.types.push(def);
        id
    }

    fn placeholder(&mut self, module: ModuleId, name: &str, ty: Type) -> TypeId {
        let origin = self.origin(module, name, Vec::new());
        self.push(TypeDef {
            name: name.to_owned(),
            origin,
            ty,
        })
    }

    /// The name of a definition: the ASN.1 name, plus words from the actual parameters for an instantiation, made
    /// unique within its module by a number.
    fn unique_name(
        &mut self,
        module: ModuleId,
        base: &str,
        actuals: &[(String, Actual)],
    ) -> String {
        let mut name = base.to_owned();
        for (text, actual) in actuals {
            if matches!(actual, Actual::Class(_)) {
                continue;
            }
            for word in text
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .filter(|w| !w.is_empty())
            {
                name.push_str(&upper_camel(word));
            }
        }
        let module_name = self.module_name(module);
        let mut candidate = name.clone();
        let mut n = 1u32;
        while !self.names.insert((module_name.clone(), candidate.clone())) {
            n = n.saturating_add(1);
            candidate = format!("{name}{n}");
        }
        candidate
    }

    // ---- types ------------------------------------------------------------------------------

    // X.680 8.4, 25, 27, 29, 31; X.681 14; X.683 9.8
    fn ty(&mut self, node: &SyntaxNode, env: &Env, depth: usize) -> Type {
        if depth > MAX_DEPTH {
            return self.opaque("types nest too deeply");
        }
        let next = depth.saturating_add(1);
        match node.kind() {
            SyntaxKind::TaggedType => self.tagged(node, env, next),
            SyntaxKind::EncodingPrefixedType => match type_child(node) {
                Some(inner) => self.ty(&inner, env, next),
                None => self.opaque("encoding prefix without a type"),
            },
            SyntaxKind::ConstrainedType => self.constrained(node, env, next),
            SyntaxKind::BuiltinType => {
                let text = builtin_text(node);
                self.builtin(&text)
            }
            SyntaxKind::IntegerType => Type::of(Kind::Integer {
                named: self.named_numbers(node, env),
            }),
            SyntaxKind::BitStringType => Type::of(Kind::BitString {
                named: self.named_numbers(node, env),
            }),
            SyntaxKind::EnumeratedType => self.enumerated(node, env),
            SyntaxKind::SequenceType | SyntaxKind::SetType | SyntaxKind::ChoiceType => {
                self.record(node, env, next)
            }
            SyntaxKind::SequenceOfType | SyntaxKind::SetOfType => self.collection(node, env, next),
            SyntaxKind::InstanceOfType => self.instance_of(node, env),
            SyntaxKind::DefinedType => self.defined(node, env, next),
            SyntaxKind::ObjectClassFieldType => self.class_field(node, env, next),
            SyntaxKind::SelectionType => self.selection(node, env, next),
            SyntaxKind::AnyType => {
                let defined_by = tokens(node)
                    .into_iter()
                    .find(|t| t.kind() == SyntaxKind::LowerName)
                    .map(|t| t.text().to_owned());
                Type::of(Kind::OpenType(OpenType {
                    class: None,
                    field: None,
                    table: None,
                    defined_by,
                }))
            }
            SyntaxKind::TypeFromObject => {
                self.opaque("a type taken from an object is not modelled")
            }
            SyntaxKind::ObjectClassDef => self.opaque("a class is not a type"),
            _ => self.opaque("unrecognized type"),
        }
    }

    fn builtin(&mut self, text: &str) -> Type {
        let kind = match text {
            "BOOLEAN" => Kind::Boolean,
            "NULL" => Kind::Null,
            "REAL" => Kind::Real,
            "INTEGER" => Kind::Integer { named: Vec::new() },
            "BIT STRING" => Kind::BitString { named: Vec::new() },
            "OCTET STRING" => Kind::OctetString,
            "OBJECT IDENTIFIER" => Kind::ObjectIdentifier,
            "RELATIVE-OID" => Kind::RelativeOid,
            "EXTERNAL" | "EMBEDDED PDV" | "CHARACTER STRING" | "OID-IRI" | "RELATIVE-OID-IRI" => {
                Kind::Other {
                    name: text.to_owned(),
                }
            }
            _ => {
                if let Some(string) = StringKind::from_name(text) {
                    Kind::String { string }
                } else if let Some(time) = TimeKind::from_name(text) {
                    Kind::Time { time }
                } else {
                    return self.opaque(format!("unknown built-in type `{text}`"));
                }
            }
        };
        Type::of(kind)
    }

    // X.680 31.2.7
    fn tagged(&mut self, node: &SyntaxNode, env: &Env, depth: usize) -> Type {
        let Some(inner) = type_child(node) else {
            return self.opaque("tag without a type");
        };
        let mut ty = self.ty(&inner, env, depth);
        let Some(tag_node) = child(node, SyntaxKind::Tag) else {
            return ty;
        };
        let decl = read_tag(&tag_node);
        let number = decl.number.or_else(|| {
            let (qualifier, name) = decl.reference.as_ref()?;
            if qualifier.is_none()
                && let Some(Actual::Value(Value::Integer(n))) = env.actual(name)
            {
                return u128::try_from(*n).ok();
            }
            let value = self
                .eval
                .integer_of_name(env.module, qualifier.as_deref(), name)?;
            u128::try_from(value).ok()
        });
        let Some(number) = number.and_then(|n| u64::try_from(n).ok()) else {
            return self.opaque("a tag number that cannot be evaluated");
        };
        let keyword = tokens(node)
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
        let implicit = match keyword.as_deref() {
            Some("IMPLICIT") => true,
            Some(_) => false,
            None => default_implicit,
        } && !self.is_dynamic(&inner, env);
        ty.tags.insert(
            0,
            TagLayer {
                tag: Tag {
                    class: tag_class(decl.class),
                    number,
                },
                mode: if implicit {
                    TagMode::Implicit
                } else {
                    TagMode::Explicit
                },
                automatic: false,
            },
        );
        ty
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
                    if reference.module.is_none() && env.actual(&reference.name).is_some() {
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

    fn constrained(&mut self, node: &SyntaxNode, env: &Env, depth: usize) -> Type {
        let Some(inner) = type_child(node) else {
            return self.opaque("constraint without a type");
        };
        let field = (inner.kind() == SyntaxKind::ObjectClassFieldType)
            .then(|| self.field_ref(&inner, env))
            .flatten();
        let mut ty = self.ty(&inner, env, depth);
        for constraint in node
            .children()
            .filter(|c| c.kind() == SyntaxKind::Constraint)
        {
            // X.682 A.2: `INSTANCE OF C ({Set})` constrains type-id to the set's ids and selects the value's type
            if inner.kind() == SyntaxKind::InstanceOfType
                && let Some(table) = child(&constraint, SyntaxKind::TableConstraint)
                && let Kind::Sequence(record) = &mut ty.kind
            {
                let class = type_child(&inner).and_then(|c| self.class_named(&c, env));
                let text = constraint::text_of(&constraint);
                let id = self.table_ref(&table, env, class, "&id");
                let value = id.clone().map(|mut t| {
                    t.field = "&Type".to_owned();
                    t.relation = vec!["type-id".to_owned()];
                    t
                });
                for component in &mut record.components {
                    match (component.name.as_str(), &mut component.ty.kind) {
                        ("type-id", _) => {
                            if let Some(id) = &id {
                                component.ty.constraints.push(glint_ir::Constraint::of(
                                    glint_ir::ElementSet::Table(id.clone()),
                                    text.clone(),
                                ));
                            }
                        }
                        ("value", Kind::OpenType(open)) => open.table.clone_from(&value),
                        _ => {}
                    }
                }
                continue;
            }
            // X.682 10.3: a table constraint on an open type selects its type
            if let Kind::OpenType(open) = &mut ty.kind
                && let Some(table) = child(&constraint, SyntaxKind::TableConstraint)
            {
                let class = field.as_ref().and_then(|f| f.class);
                let field_name = field.as_ref().map_or_else(String::new, |f| f.field.clone());
                let mut open = open.clone();
                open.table = self.table_ref(&table, env, class, &field_name);
                ty.kind = Kind::OpenType(open);
                continue;
            }
            let hint = self.hint(&ty);
            let lowered = self.constraint(&constraint, env, field.as_ref(), &hint);
            ty.constraints.push(lowered);
        }
        ty
    }

    fn named_numbers(&mut self, node: &SyntaxNode, env: &Env) -> Vec<NamedNumber> {
        let mut out = Vec::new();
        for item in node
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::NamedNumber)
        {
            let Some(name) = tokens(&item)
                .into_iter()
                .find(|t| t.kind() == SyntaxKind::LowerName)
            else {
                continue;
            };
            let value = child(&item, SyntaxKind::Value).map(|v| self.value(&v, env, &Hint::None));
            match value {
                Some(Value::Integer(value)) => out.push(NamedNumber {
                    name: name.text().to_owned(),
                    value,
                }),
                _ => self.note(format!(
                    "the value of `{}` cannot be evaluated",
                    name.text()
                )),
            }
        }
        out
    }

    // X.680 20.2-20.5: unnumbered root items take the smallest unused non-negative numbers, unnumbered additions one
    // more than every item before them
    fn enumerated(&mut self, node: &SyntaxNode, env: &Env) -> Type {
        let marker = node
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::Ellipsis)
            .map(|t| t.text_range().start());
        let mut root: Vec<(String, Option<i128>)> = Vec::new();
        let mut additions: Vec<(String, Option<i128>)> = Vec::new();
        for item in node
            .descendants()
            .filter(|n| matches!(n.kind(), SyntaxKind::Enumeration | SyntaxKind::NamedNumber))
        {
            let Some(name) = tokens(&item)
                .into_iter()
                .find(|t| t.kind() == SyntaxKind::LowerName)
            else {
                continue;
            };
            let value = child(&item, SyntaxKind::Value).and_then(|v| {
                match self.value(&v, env, &Hint::None) {
                    Value::Integer(n) => Some(n),
                    _ => None,
                }
            });
            let entry = (name.text().to_owned(), value);
            if marker.is_some_and(|m| item.text_range().start() > m) {
                additions.push(entry);
            } else {
                root.push(entry);
            }
        }
        let mut used: HashSet<i128> = root.iter().filter_map(|(_, v)| *v).collect();
        let mut next = 0i128;
        let root: Vec<NamedNumber> = root
            .into_iter()
            .map(|(name, value)| {
                let value = value.unwrap_or_else(|| {
                    while used.contains(&next) {
                        next = next.saturating_add(1);
                    }
                    used.insert(next);
                    next
                });
                NamedNumber { name, value }
            })
            .collect();
        let mut highest = root.iter().map(|n| n.value).max().unwrap_or(-1);
        let additions = additions
            .into_iter()
            .map(|(name, value)| {
                let value = value.unwrap_or_else(|| highest.saturating_add(1));
                highest = highest.max(value);
                NamedNumber { name, value }
            })
            .collect();
        Type::of(Kind::Enumerated {
            root,
            additions,
            extensible: marker.is_some()
                || self
                    .graph
                    .module(env.module)
                    .is_some_and(|m| m.model.extensibility_implied),
        })
    }

    // X.680 25, 27, 29
    fn record(&mut self, node: &SyntaxNode, env: &Env, depth: usize) -> Type {
        let module = env.module;
        let list = ComponentList::read(node);
        let extensible = list.is_extensible()
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
                    && f.component.region != ViewRegion::Addition
                    && is_textually_tagged(&f.component.ty)
            });
        let mut components = Vec::new();
        for (index, entry) in flat.iter().enumerate() {
            let component = &entry.component;
            let sub_env = if entry.module == module {
                env.clone()
            } else {
                Env::new(entry.module)
            };
            let mut ty = self.ty(&component.ty, &sub_env, depth);
            if automatic {
                let dynamic = self.is_dynamic(&component.ty, &sub_env);
                ty.tags.insert(
                    0,
                    TagLayer {
                        tag: Tag {
                            class: TagClass::Context,
                            number: u64::try_from(index).unwrap_or(u64::MAX),
                        },
                        mode: if dynamic {
                            TagMode::Explicit
                        } else {
                            TagMode::Implicit
                        },
                        automatic: true,
                    },
                );
            }
            let presence = match &component.default {
                Some(value) => {
                    let hint = self.hint(&ty);
                    Presence::Default(self.value(value, &sub_env, &hint))
                }
                None if component.optional => Presence::Optional,
                None => Presence::Required,
            };
            let version = component
                .group
                .and_then(|g| list.groups.get(g))
                .and_then(|g| g.version.map(|(v, _)| v))
                .and_then(|v| u64::try_from(v).ok());
            components.push(Component {
                name: component.name.clone(),
                presence,
                region: match component.region {
                    ViewRegion::Root => Region::Root,
                    ViewRegion::Addition => Region::Addition,
                    ViewRegion::TrailingRoot => Region::TrailingRoot,
                },
                group: component.group.and_then(|g| u32::try_from(g).ok()),
                version,
                ty,
            });
        }
        let record = Record {
            components,
            extensible,
        };
        Type::of(match node.kind() {
            SyntaxKind::SequenceType => Kind::Sequence(record),
            SyntaxKind::SetType => Kind::Set(record),
            _ => Kind::Choice(record),
        })
    }

    // X.680 26, 28; 49.5 `SEQUENCE SIZE (...) OF`
    fn collection(&mut self, node: &SyntaxNode, env: &Env, depth: usize) -> Type {
        let element_name = child(node, SyntaxKind::NamedType)
            .and_then(|named| {
                tokens(&named)
                    .into_iter()
                    .find(|t| t.kind() == SyntaxKind::LowerName)
            })
            .map(|t| t.text().to_owned());
        let element = match crate::view::of_element_type(node) {
            Some(element) => self.ty(&element, env, depth),
            None => self.opaque("collection without an element type"),
        };
        let collection = Collection {
            element_name,
            element: Box::new(element),
        };
        let mut ty = Type::of(if node.kind() == SyntaxKind::SequenceOfType {
            Kind::SequenceOf(collection)
        } else {
            Kind::SetOf(collection)
        });
        if let Some(constraint) = child(node, SyntaxKind::Constraint) {
            let lowered = self.constraint(&constraint, env, None, &Hint::None);
            ty.constraints.push(lowered);
        } else if let Some(size) = child(node, SyntaxKind::SizeElement) {
            let text = constraint::text_of(&size);
            let root = self.elements(&size, env, &Hint::None);
            ty.constraints.push(glint_ir::Constraint::of(root, text));
        }
        ty
    }

    // X.681 Annex C.6, X.682 A.2: INSTANCE OF is a SEQUENCE { type-id, value [0] }, tagged [UNIVERSAL 8]
    fn instance_of(&mut self, node: &SyntaxNode, env: &Env) -> Type {
        let class_name = type_child(node)
            .filter(|c| self.class_named(c, env).is_some())
            .and_then(|c| defined_type_ref(&c))
            .map(|r| r.name);
        let mut value = Type::of(Kind::OpenType(OpenType {
            class: class_name,
            field: Some("&Type".to_owned()),
            table: None,
            defined_by: None,
        }));
        value.tags.push(TagLayer {
            tag: Tag {
                class: TagClass::Context,
                number: 0,
            },
            mode: TagMode::Explicit,
            automatic: false,
        });
        let mut ty = Type::of(Kind::Sequence(Record {
            components: vec![
                Component {
                    name: "type-id".to_owned(),
                    presence: Presence::Required,
                    region: Region::Root,
                    group: None,
                    version: None,
                    ty: Type::of(Kind::ObjectIdentifier),
                },
                Component {
                    name: "value".to_owned(),
                    presence: Presence::Required,
                    region: Region::Root,
                    group: None,
                    version: None,
                    ty: value,
                },
            ],
            extensible: false,
        }));
        ty.tags.push(TagLayer {
            tag: Tag {
                class: TagClass::Universal,
                number: 8,
            },
            mode: TagMode::Implicit,
            automatic: false,
        });
        ty
    }

    // X.680 17, X.683 9
    fn defined(&mut self, node: &SyntaxNode, env: &Env, depth: usize) -> Type {
        let Some(reference) = defined_type_ref(node) else {
            return self.opaque("unreadable type reference");
        };
        if reference.module.is_none()
            && let Some(actual) = env.actual(&reference.name)
        {
            return match actual.clone() {
                Actual::Type(ty) => *ty,
                Actual::Unknown(why) => self.opaque(why),
                _ => self.opaque(format!("the dummy `{}` is not a type", reference.name)),
            };
        }
        let display = match &reference.module {
            Some(module) => format!("{module}.{}", reference.name),
            None => reference.name.clone(),
        };
        let resolved = self.sem.resolve(
            env.module,
            &[],
            reference.module.as_deref(),
            &reference.name,
        );
        let target = match resolved {
            Resolved::Sym(target) => target,
            Resolved::Opaque => {
                return self.opaque(format!("`{display}`: the defining module is not available"));
            }
            _ => return self.opaque(format!("`{display}` is neither defined nor imported")),
        };
        let Some(symbol) = self.sem.symbol(target) else {
            return self.opaque(format!("`{display}` is not defined"));
        };
        // X.680 41: the 1988 definitions of the newer string types are the built-in types
        if let Some(name) = legacy_string_definition(&symbol.node)
            && self
                .graph
                .module(target.module)
                .is_some_and(|m| is_legacy_string_module(&m.model.name, m.oid.as_ref()))
            && let Some(string) = StringKind::from_name(name)
        {
            return Type::of(Kind::String { string });
        }
        if let Some(list) = child(node, SyntaxKind::ActualParameterList)
            && symbol.is_parameterized()
        {
            let actuals = self.actuals(env, target, &list, depth);
            return Type::of(Kind::Reference {
                target: self.named(target, actuals),
            });
        }
        if symbol.is_parameterized() {
            return self.opaque(format!("`{display}` is used without its parameters"));
        }
        match self.sem.kind(target) {
            SymKind::Type | SymKind::ValueSet => Type::of(Kind::Reference {
                target: self.named(target, Vec::new()),
            }),
            SymKind::Macro => self.opaque(format!("`{display}` is macro notation")),
            other => self.opaque(format!("`{display}` is not a type ({other:?})")),
        }
    }

    /// Reads the actual parameters of an instantiation of `target` written in `caller` (X.683 9.5): each is parsed
    /// once the kind of its dummy is known, and lowered in the caller's environment (X.683 9.7).
    fn actuals(
        &mut self,
        caller: &Env,
        target: SymRef,
        list: &SyntaxNode,
        depth: usize,
    ) -> Vec<(String, Actual)> {
        let Some(symbol) = self.sem.symbol(target) else {
            return Vec::new();
        };
        let dummies = dummies_of(&symbol.node);
        let nodes: Vec<SyntaxNode> = list
            .children()
            .filter(|c| c.kind() == SyntaxKind::ActualParameter)
            .collect();
        let mut out: Vec<(String, Actual)> = Vec::new();
        for (dummy, node) in dummies.iter().zip(&nodes) {
            let toks = flatten(node);
            let text = toks
                .iter()
                .map(|t| {
                    caller
                        .texts
                        .get(&t.text)
                        .map_or(t.text.as_str(), String::as_str)
                })
                .collect::<Vec<_>>()
                .join(" ");
            let range = crate::sem::toks::span(&toks).unwrap_or_else(|| node.text_range());
            let lower_dummy = dummy.name.starts_with(|c: char| c.is_ascii_lowercase());
            // the governor is a class, written directly or as a class dummy bound earlier
            let governor_class = dummy.governor.as_ref().and_then(|g| {
                if let ClassRes::Class(class) = self.sem.class_of(target.module, g) {
                    return Some(class);
                }
                let reference = defined_type_ref(g)?;
                out.iter().zip(&dummies).find_map(|((_, a), d)| match a {
                    Actual::Class(class) if d.name == reference.name => Some(*class),
                    _ => None,
                })
            });
            let class_governor = governor_class.is_some();
            let actual = if dummy.governor.is_some() && class_governor {
                if lower_dummy {
                    Actual::Unknown(format!(
                        "the object parameter `{}` is not modelled",
                        dummy.name
                    ))
                } else {
                    self.object_set_actual(caller, node, range, governor_class)
                }
            } else if dummy.governor.is_some() {
                if lower_dummy {
                    match self.parse_at(caller.module, range, PrefixKind::Value) {
                        Some(value) => Actual::Value(self.value(&value, caller, &Hint::None)),
                        None => Actual::Unknown(format!("unreadable value parameter `{text}`")),
                    }
                } else {
                    Actual::Unknown(format!(
                        "the value set parameter `{}` is not modelled",
                        dummy.name
                    ))
                }
            } else {
                match self.parse_at(caller.module, range, PrefixKind::Type) {
                    Some(actual) => match self.class_named(&actual, caller) {
                        Some(class) => Actual::Class(class),
                        None => Actual::Type(Box::new(self.ty(&actual, caller, depth))),
                    },
                    None => Actual::Unknown(format!("unreadable type parameter `{text}`")),
                }
            };
            out.push((text, actual));
        }
        out
    }

    fn object_set_actual(
        &mut self,
        caller: &Env,
        node: &SyntaxNode,
        range: TextRange,
        class: Option<SymRef>,
    ) -> Actual {
        let group = node
            .descendants()
            .find(|n| n.kind() == SyntaxKind::BraceGroup)
            .or_else(|| {
                self.parse_at(caller.module, range, PrefixKind::Value)
                    .and_then(|v| child(&v, SyntaxKind::BraceGroup))
            });
        match group.and_then(|g| self.object_set(&g, caller, class)) {
            Some(id) => Actual::ObjectSet(id),
            None => Actual::Unknown("an object set parameter that cannot be read".to_owned()),
        }
    }

    /// The class `node` names, through class dummies.
    fn class_named(&self, node: &SyntaxNode, env: &Env) -> Option<SymRef> {
        if node.kind() != SyntaxKind::DefinedType {
            return None;
        }
        let reference = defined_type_ref(node)?;
        if reference.module.is_none()
            && let Some(actual) = env.actual(&reference.name)
        {
            return match actual {
                Actual::Class(class) => Some(*class),
                _ => None,
            };
        }
        match self.sem.resolve(
            env.module,
            &[],
            reference.module.as_deref(),
            &reference.name,
        ) {
            Resolved::Sym(sym) => match self.sem.class_res_of_symbol(sym) {
                ClassRes::Class(class) => Some(class),
                _ => None,
            },
            _ => None,
        }
    }

    // X.681 14
    fn class_field(&mut self, node: &SyntaxNode, env: &Env, depth: usize) -> Type {
        let Some(field) = self.field_ref(node, env) else {
            return self.opaque("unreadable class field reference");
        };
        if field.path_len > 1 {
            return self.opaque(format!(
                "the field path `{}` is not modelled",
                field.written
            ));
        }
        let open = || {
            Type::of(Kind::OpenType(OpenType {
                class: Some(field.class_name.clone()),
                field: Some(field.field.clone()),
                table: None,
                defined_by: None,
            }))
        };
        let Some(class) = field.class else {
            return if field.type_field {
                open()
            } else {
                self.opaque(format!("the class of `{}` is not available", field.written))
            };
        };
        let Some(info) = self.sem.class_info(class) else {
            return self.opaque(format!("the class of `{}` is not available", field.written));
        };
        let Some(spec) = info.field(&field.field) else {
            return self.opaque(format!("`{}` names no field of its class", field.written));
        };
        match spec.kind {
            FieldKind::Type | FieldKind::VariableValue => open(),
            FieldKind::FixedValue | FieldKind::FixedValueSet => match spec.governor.clone() {
                Some(governor) => self.ty(&governor, &Env::new(class.module), depth),
                None => self.opaque(format!("`{}` has no type", field.written)),
            },
            _ => self.opaque(format!("`{}` is not a type or value field", field.written)),
        }
    }

    /// The class and field of an `ObjectClassFieldType`.
    fn field_ref(&self, node: &SyntaxNode, env: &Env) -> Option<FieldRef> {
        let names: Vec<String> = tokens(node)
            .into_iter()
            .filter(|t| t.kind() == SyntaxKind::UpperName)
            .map(|t| t.text().to_owned())
            .collect();
        let field_node = child(node, SyntaxKind::FieldName)?;
        let field_tokens: Vec<_> = tokens(&field_node)
            .into_iter()
            .filter(|t| matches!(t.kind(), SyntaxKind::FieldUpper | SyntaxKind::FieldLower))
            .collect();
        let first = field_tokens.first()?;
        let type_field = field_tokens
            .last()
            .is_some_and(|t| t.kind() == SyntaxKind::FieldUpper);
        let (qualifier, name) = match names.as_slice() {
            [module, name, ..] => (Some(module.clone()), name.clone()),
            [name] => (None, name.clone()),
            [] => return None,
        };
        let class = if qualifier.is_none()
            && let Some(actual) = env.actual(&name)
        {
            match actual {
                Actual::Class(class) => Some(*class),
                _ => None,
            }
        } else {
            match self
                .sem
                .resolve(env.module, &[], qualifier.as_deref(), &name)
            {
                Resolved::Sym(sym) => match self.sem.class_res_of_symbol(sym) {
                    ClassRes::Class(class) => Some(class),
                    _ => None,
                },
                _ => None,
            }
        };
        let class_name = class
            .and_then(|c| self.sem.symbol(c))
            .map_or(name.clone(), |s| s.name.clone());
        let written = format!(
            "{name}.{}",
            field_tokens
                .iter()
                .map(|t| t.text().to_owned())
                .collect::<Vec<_>>()
                .join(".")
        );
        Some(FieldRef {
            class,
            class_name,
            field: first.text().to_owned(),
            path_len: field_tokens.len(),
            type_field,
            written,
        })
    }

    // X.680 30
    fn selection(&mut self, node: &SyntaxNode, env: &Env, depth: usize) -> Type {
        let Some(name) = tokens(node)
            .into_iter()
            .find(|t| t.kind() == SyntaxKind::LowerName)
            .map(|t| t.text().to_owned())
        else {
            return self.opaque("unreadable selection type");
        };
        let Some(inner) = type_child(node) else {
            return self.opaque("selection type without a type");
        };
        let selected = self.ty(&inner, env, depth);
        let found = match &self.resolved(&selected).kind {
            Kind::Choice(record) => record
                .components
                .iter()
                .find(|c| c.name == name)
                .map(|c| c.ty.clone()),
            _ => None,
        };
        match found {
            Some(ty) => ty,
            None => self.opaque(format!(
                "`{name}` is not an alternative of the selected CHOICE"
            )),
        }
    }

    /// The type a chain of references ends in, as lowered so far.
    fn resolved(&self, ty: &Type) -> Type {
        let mut current = ty.clone();
        for _ in 0..=self.types.len() {
            match &current.kind {
                Kind::Reference { target } => {
                    match usize::try_from(target.0)
                        .ok()
                        .and_then(|i| self.types.get(i))
                    {
                        Some(def) => current = def.ty.clone(),
                        None => break,
                    }
                }
                _ => break,
            }
        }
        current
    }

    /// How values of `ty` are read.
    fn hint(&self, ty: &Type) -> Hint {
        match self.resolved(ty).kind {
            Kind::Integer { named } => Hint::Integer(named),
            Kind::Enumerated { .. } => Hint::Enumerated,
            Kind::BitString { .. } => Hint::Bits,
            Kind::ObjectIdentifier => Hint::Oid,
            Kind::String { .. } => Hint::String,
            kind @ (Kind::Sequence(_) | Kind::Set(_)) => Hint::Record(Box::new(Type::of(kind))),
            _ => Hint::None,
        }
    }

    /// A value set used as a type: its governor constrained by its element sets (X.680 16.7).
    fn value_set_type(&mut self, sym: SymRef, env: &Env) -> Type {
        let Some(symbol) = self.sem.symbol(sym) else {
            return self.opaque("unknown value set");
        };
        let Some(governor) = symbol.type_node.clone() else {
            return self.opaque(format!("the value set `{}` has no governor", symbol.name));
        };
        let mut ty = self.ty(&governor, env, 0);
        let hint = self.hint(&ty);
        match self.value_set_constraint(sym, env, &hint) {
            Some(constraint) => ty.constraints.push(constraint),
            None => {
                let text = format!("the elements of `{}` cannot be read", symbol.name);
                ty.constraints.push(glint_ir::Constraint::of(
                    glint_ir::ElementSet::Opaque(text.clone()),
                    text.clone(),
                ));
                self.note(text);
            }
        }
        ty
    }
}

/// The class and field named by an `ObjectClassFieldType`.
struct FieldRef {
    class: Option<SymRef>,
    class_name: String,
    /// The first field name, with its `&`.
    field: String,
    path_len: usize,
    type_field: bool,
    written: String,
}

fn tag_class(class: ViewTagClass) -> TagClass {
    match class {
        ViewTagClass::Universal => TagClass::Universal,
        ViewTagClass::Application => TagClass::Application,
        ViewTagClass::Context => TagClass::Context,
        ViewTagClass::Private => TagClass::Private,
    }
}

/// `ub-name` → `UbName`, `SIGNATURE-ALGORITHM` → `SignatureAlgorithm`, `HashAlgs` → `HashAlgs`.
fn upper_camel(word: &str) -> String {
    let all_caps = word.chars().all(|c| !c.is_ascii_lowercase());
    word.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            let first = chars.next().map(|c| c.to_ascii_uppercase());
            let rest: String = if all_caps {
                chars.map(|c| c.to_ascii_lowercase()).collect()
            } else {
                chars.collect()
            };
            first.into_iter().chain(rest.chars()).collect::<String>()
        })
        .collect()
}

/// A comparable form of an actual parameter, so that equal instantiations share one definition.
fn actual_key(actual: &Actual) -> String {
    match actual {
        Actual::Type(ty) => format!("T{ty:?}"),
        Actual::Value(value) => format!("V{value:?}"),
        Actual::ObjectSet(id) => format!("S{}", id.0),
        Actual::Class(sym) => format!("C{}:{}", sym.module.0, sym.index),
        Actual::Unknown(why) => format!("U{why}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::upper_camel;

    #[test]
    fn words_become_upper_camel() {
        assert_eq!(upper_camel("ub-name"), "UbName");
        assert_eq!(upper_camel("SIGNATURE-ALGORITHM"), "SignatureAlgorithm");
        assert_eq!(upper_camel("HashAlgs"), "HashAlgs");
        assert_eq!(upper_camel("id-ce-keyUsage"), "IdCeKeyUsage");
    }
}
