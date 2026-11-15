//! Constraints in evaluated form (X.680 49-51, X.682 8-11).

use glint_ir::{
    Bound, ComponentConstraint, Constraint, ElementSet, PresenceConstraint, TableRef, Value,
};
use glint_syntax::{PrefixKind, SyntaxKind, SyntaxNode, TextRange};

use super::{Actual, Env, FieldRef, Hint, Lowerer};
use crate::sem::toks::{flatten, inner, split_commas};
use crate::sem::{Kind as SymKind, Resolved, SymRef};
use crate::view::{child, defined_type_ref, tokens, type_child};

/// The constraint as written.
pub(super) fn text_of(node: &SyntaxNode) -> String {
    flatten(node)
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_element(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::UnionSet
            | SyntaxKind::IntersectionSet
            | SyntaxKind::ExclusionSet
            | SyntaxKind::ParenElementSet
            | SyntaxKind::ValueElement
            | SyntaxKind::RangeElement
            | SyntaxKind::TypeElement
            | SyntaxKind::SizeElement
            | SyntaxKind::FromElement
            | SyntaxKind::ContainedSubtype
            | SyntaxKind::InnerTypeElement
            | SyntaxKind::PatternElement
            | SyntaxKind::SettingsElement
    )
}

fn elements_of(node: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> {
    node.children().filter(|c| is_element(c.kind()))
}

impl Lowerer<'_, '_, '_> {
    /// Lowers a `Constraint` node. `field` is the class field the constrained type comes from, for a table
    /// constraint; `hint` reads the values of the constrained type.
    pub(super) fn constraint(
        &mut self,
        node: &SyntaxNode,
        env: &Env,
        field: Option<&FieldRef>,
        hint: &Hint,
    ) -> Constraint {
        let text = text_of(node);
        let first = node.children().next();
        match first.as_ref().map(SyntaxNode::kind) {
            Some(SyntaxKind::UserDefinedConstraint) => {
                return Constraint::of(ElementSet::UserDefined(text.clone()), text);
            }
            // X.682 11
            Some(SyntaxKind::ContentsConstraint) => {
                let Some(spec) = first else {
                    return Constraint::of(ElementSet::Opaque(text.clone()), text);
                };
                let ty = type_child(&spec).map(|t| Box::new(self.ty(&t, env, 0)));
                let encoded_by =
                    child(&spec, SyntaxKind::Value).map(|v| self.value(&v, env, &Hint::Oid));
                return Constraint::of(ElementSet::Contents { ty, encoded_by }, text);
            }
            // X.682 10.3: a simple table constraint, or a component relation constraint on a value field
            Some(SyntaxKind::TableConstraint) => {
                let Some(table) = first else {
                    return Constraint::of(ElementSet::Opaque(text.clone()), text);
                };
                let root = match field {
                    Some(field) => match self.table_ref(&table, env, field.class, &field.field) {
                        Some(table) => ElementSet::Table(table),
                        None => self.opaque_set(&text, "the object set cannot be read"),
                    },
                    None => self.opaque_set(&text, "a table constraint outside a class field"),
                };
                return Constraint::of(root, text);
            }
            _ => {}
        }
        let marker = tokens(node)
            .into_iter()
            .find(|t| t.kind() == SyntaxKind::Ellipsis)
            .map(|t| t.text_range().start());
        let mut root = None;
        let mut additions = None;
        for element in elements_of(node) {
            let lowered = self.elements(&element, env, hint);
            if marker.is_some_and(|m| element.text_range().start() > m) {
                additions = Some(lowered);
            } else if root.is_none() {
                root = Some(lowered);
            }
        }
        let exception = child(node, SyntaxKind::ExceptionSpec)
            .and_then(|spec| child(&spec, SyntaxKind::Value))
            .map(|v| self.value(&v, env, &Hint::None));
        Constraint {
            root: root.unwrap_or(ElementSet::All),
            extensible: marker.is_some(),
            additions,
            exception,
            text,
        }
    }

    fn opaque_set(&mut self, text: &str, why: &str) -> ElementSet {
        self.note(format!("{why}: {text}"));
        ElementSet::Opaque(text.to_owned())
    }

    /// Lowers one element set (X.680 50, 51).
    pub(super) fn elements(&mut self, node: &SyntaxNode, env: &Env, hint: &Hint) -> ElementSet {
        match node.kind() {
            SyntaxKind::UnionSet | SyntaxKind::IntersectionSet => {
                let items = elements_of(node)
                    .map(|e| self.elements(&e, env, hint))
                    .collect();
                if node.kind() == SyntaxKind::UnionSet {
                    ElementSet::Union(items)
                } else {
                    ElementSet::Intersection(items)
                }
            }
            SyntaxKind::ExclusionSet => {
                let all = tokens(node).first().is_some_and(|t| t.text() == "ALL");
                let parts: Vec<ElementSet> = elements_of(node)
                    .map(|e| self.elements(&e, env, hint))
                    .collect();
                match (all, parts.as_slice()) {
                    (true, [excluded]) => {
                        ElementSet::Except(Box::new(ElementSet::All), Box::new(excluded.clone()))
                    }
                    (false, [kept, excluded]) => {
                        ElementSet::Except(Box::new(kept.clone()), Box::new(excluded.clone()))
                    }
                    _ => self.opaque_set(&text_of(node), "unreadable EXCEPT"),
                }
            }
            SyntaxKind::ParenElementSet => match elements_of(node).next() {
                Some(inner) => self.elements(&inner, env, hint),
                None => self.opaque_set(&text_of(node), "empty parentheses"),
            },
            SyntaxKind::ValueElement => {
                let value = child(node, SyntaxKind::Value)
                    .map(|v| self.value(&v, env, hint))
                    .or_else(|| {
                        child(node, SyntaxKind::BraceGroup).map(|g| self.value(&g, env, hint))
                    });
                match value {
                    Some(value) => ElementSet::Value(value),
                    None => self.opaque_set(&text_of(node), "unreadable value"),
                }
            }
            SyntaxKind::RangeElement => self.range(node, env, hint),
            SyntaxKind::SizeElement => match child(node, SyntaxKind::Constraint) {
                Some(inner) => ElementSet::Size(Box::new(self.constraint(
                    &inner,
                    env,
                    None,
                    &Hint::Integer(Vec::new()),
                ))),
                None => self.opaque_set(&text_of(node), "SIZE without a constraint"),
            },
            SyntaxKind::FromElement => match child(node, SyntaxKind::Constraint) {
                Some(inner) => {
                    ElementSet::From(Box::new(self.constraint(&inner, env, None, &Hint::String)))
                }
                None => self.opaque_set(&text_of(node), "FROM without a constraint"),
            },
            SyntaxKind::InnerTypeElement => self.inner_type(node, env),
            SyntaxKind::PatternElement => {
                match child(node, SyntaxKind::Value).map(|v| self.value(&v, env, &Hint::String)) {
                    Some(Value::String(pattern)) => ElementSet::Pattern(pattern),
                    _ => self.opaque_set(&text_of(node), "a pattern that is not a string"),
                }
            }
            SyntaxKind::SettingsElement => {
                let setting = tokens(node)
                    .into_iter()
                    .find(|t| t.kind() == SyntaxKind::CString)
                    .map(|t| unquote(t.text()))
                    .unwrap_or_default();
                ElementSet::Settings(setting)
            }
            SyntaxKind::ContainedSubtype => match type_child(node) {
                Some(ty) => ElementSet::ContainedSubtype(Box::new(self.ty(&ty, env, 0))),
                None => self.opaque_set(&text_of(node), "INCLUDES without a type"),
            },
            SyntaxKind::TypeElement => self.type_element(node, env, hint),
            _ => self.opaque_set(&text_of(node), "unrecognized constraint element"),
        }
    }

    // X.680 51.4: `lo..hi` with `<` excluding an end
    fn range(&mut self, node: &SyntaxNode, env: &Env, hint: &Hint) -> ElementSet {
        let toks = tokens(node);
        let dots = toks
            .iter()
            .position(|t| t.kind() == SyntaxKind::DotDot)
            .unwrap_or(0);
        let lo_open = toks.iter().take(dots).any(|t| t.kind() == SyntaxKind::Lt);
        let hi_open = toks.iter().skip(dots).any(|t| t.kind() == SyntaxKind::Lt);
        let values: Vec<SyntaxNode> = node
            .children()
            .filter(|c| c.kind() == SyntaxKind::Value)
            .collect();
        let [lo, hi] = values.as_slice() else {
            return self.opaque_set(&text_of(node), "unreadable range");
        };
        let lo = self.bound(lo, env, hint, lo_open);
        let hi = self.bound(hi, env, hint, hi_open);
        ElementSet::Range { lo, hi }
    }

    fn bound(&mut self, node: &SyntaxNode, env: &Env, hint: &Hint, open: bool) -> Bound {
        let text = text_of(node);
        match text.as_str() {
            "MIN" => Bound::Min,
            "MAX" => Bound::Max,
            _ => {
                let value = self.value(node, env, hint);
                if open {
                    Bound::Exclusive(value)
                } else {
                    Bound::Inclusive(value)
                }
            }
        }
    }

    // X.680 51.8
    fn inner_type(&mut self, node: &SyntaxNode, env: &Env) -> ElementSet {
        let single = tokens(node).iter().any(|t| t.text() == "COMPONENT");
        if single {
            return match child(node, SyntaxKind::Constraint) {
                Some(inner) => ElementSet::WithComponent(Box::new(self.constraint(
                    &inner,
                    env,
                    None,
                    &Hint::None,
                ))),
                None => self.opaque_set(&text_of(node), "WITH COMPONENT without a constraint"),
            };
        }
        let first_named = node
            .children()
            .find(|c| c.kind() == SyntaxKind::NamedConstraint)
            .map(|c| c.text_range().start());
        let partial = tokens(node).iter().any(|t| {
            t.kind() == SyntaxKind::Ellipsis
                && first_named.is_none_or(|start| t.text_range().start() < start)
        });
        let mut items = Vec::new();
        for named in node
            .children()
            .filter(|c| c.kind() == SyntaxKind::NamedConstraint)
        {
            let toks = tokens(&named);
            let Some(name) = toks.iter().find(|t| t.kind() == SyntaxKind::LowerName) else {
                continue;
            };
            let presence = toks.iter().find_map(|t| match t.text() {
                "PRESENT" => Some(PresenceConstraint::Present),
                "ABSENT" => Some(PresenceConstraint::Absent),
                "OPTIONAL" => Some(PresenceConstraint::Optional),
                _ => None,
            });
            let constraint = child(&named, SyntaxKind::Constraint)
                .map(|c| self.constraint(&c, env, None, &Hint::None));
            items.push(ComponentConstraint {
                name: name.text().to_owned(),
                constraint,
                presence,
            });
        }
        ElementSet::WithComponents { partial, items }
    }

    // X.680 51.3: a type in a constraint is a contained subtype, unless it names a value set (16.7)
    fn type_element(&mut self, node: &SyntaxNode, env: &Env, hint: &Hint) -> ElementSet {
        let Some(ty) = type_child(node) else {
            return self.opaque_set(&text_of(node), "unreadable type element");
        };
        if ty.kind() == SyntaxKind::DefinedType
            && let Some(reference) = defined_type_ref(&ty)
        {
            if reference.module.is_none()
                && let Some(Actual::Unknown(why)) = env.actual(&reference.name)
            {
                let why = why.clone();
                return self.opaque_set(&text_of(node), &why);
            }
            if let Resolved::Sym(target) = self.sem.resolve(
                env.module,
                &[],
                reference.module.as_deref(),
                &reference.name,
            ) && self.sem.kind(target) == SymKind::ValueSet
                && child(&ty, SyntaxKind::ActualParameterList).is_none()
            {
                return match self.value_set_constraint(target, &Env::new(target.module), hint) {
                    Some(constraint) => constraint.root,
                    None => self.opaque_set(&text_of(node), "the value set cannot be read"),
                };
            }
        }
        ElementSet::ContainedSubtype(Box::new(self.ty(&ty, env, 0)))
    }

    /// The element sets of the value set assignment `sym`, read as a constraint (X.680 16.7).
    pub(super) fn value_set_constraint(
        &mut self,
        sym: SymRef,
        env: &Env,
        hint: &Hint,
    ) -> Option<Constraint> {
        let symbol = self.sem.symbol(sym)?;
        let rhs = child(&symbol.node, SyntaxKind::AmbiguousRhs)?;
        let group = child(&rhs, SyntaxKind::BraceGroup)?;
        let toks = flatten(&group);
        let (first, last) = (toks.first()?, toks.last()?);
        if first.range.end() >= last.range.start() {
            return None;
        }
        let range = TextRange::new(first.range.end(), last.range.start());
        let node = self.parse_at(sym.module, range, PrefixKind::ElementSet)?;
        Some(self.constraint(&node, env, None, hint))
    }

    /// The table of a `TableConstraint` (X.682 10.3): the object set, the field and the `@` paths.
    pub(super) fn table_ref(
        &mut self,
        table: &SyntaxNode,
        env: &Env,
        class: Option<SymRef>,
        field: &str,
    ) -> Option<TableRef> {
        let mut groups = table
            .children()
            .filter(|c| c.kind() == SyntaxKind::BraceGroup);
        let set_group = groups.next()?;
        let set = self.object_set(&set_group, env, class)?;
        let relation = groups
            .next()
            .map(|g| {
                split_commas(&inner(&g))
                    .iter()
                    .map(|entry| {
                        entry
                            .iter()
                            .map(|t| t.text.as_str())
                            .collect::<String>()
                            .trim_start_matches('@')
                            .to_owned()
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(TableRef {
            set,
            field: field.to_owned(),
            relation,
        })
    }
}

/// A `cstring` without its quotes, with doubled quotes undone (X.680 12.14).
pub(super) fn unquote(text: &str) -> String {
    let inner = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text);
    inner.replace("\"\"", "\"")
}
