//! Normal forms of definitions (X.680 Annex C.3), as trees that can be compared and diffed.
//!
//! Two forms are built from the same syntax tree. The raw form keeps what the author wrote (comments and white
//! space aside). The normal form applies the transformations of C.3.2.2 and C.3.2.3 that Glint supports. Two
//! definitions that differ in the normal form are different (E300); two that differ only in the raw form say the
//! same thing in other words (W302).

use std::collections::BTreeSet;

use glint_syntax::ast::TagDefault;
use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use crate::checks::components::expand;
use crate::eval::Evaluator;
use crate::graph::{ModuleGraph, ModuleId};
use crate::view::{
    ComponentList, Region, builtin_text, child, is_textually_tagged, is_type_node, read_component,
    read_tag, tokens, trimmed_range, type_child,
};

/// The atom of a tag that does not say IMPLICIT or EXPLICIT.
pub(crate) const DEFAULT_TAGGING: &str = "(default)";

/// Which of the two forms is being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    Raw,
    Normal,
}

/// One node of a normal form. Equality ignores `range`, `key` and `partial`.
#[derive(Debug, Clone)]
pub(crate) struct Norm {
    pub label: &'static str,
    pub atom: String,
    /// Identifies an item among its siblings (a component name, for example).
    pub key: Option<String>,
    pub children: Vec<Norm>,
    /// Where the node is written in its source text.
    pub range: TextRange,
    /// Comparison against a longer list is allowed to skip items (`-- glint: partial`).
    pub partial: bool,
    /// The node is a value that names definitions the module cannot resolve, so a difference proves nothing.
    pub uncertain: bool,
}

impl Norm {
    pub fn leaf(label: &'static str, atom: impl Into<String>, range: TextRange) -> Self {
        Self::node(label, atom, range, Vec::new())
    }

    pub fn node(
        label: &'static str,
        atom: impl Into<String>,
        range: TextRange,
        children: Vec<Norm>,
    ) -> Self {
        Self {
            label,
            atom: atom.into(),
            key: None,
            children,
            range,
            partial: false,
            uncertain: false,
        }
    }

    pub fn keyed(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The node written out for a message.
    pub fn text(&self) -> String {
        let mut parts = vec![self.atom.clone()];
        parts.extend(self.children.iter().map(Norm::text));
        parts.retain(|p| !p.is_empty());
        parts.join(" ")
    }
}

/// Constructs of a snippet marked as partial.
#[derive(Debug, Clone, Default)]
pub(crate) struct PartialMarks {
    /// The whole fragment is partial.
    pub all: bool,
    /// Ranges of the constructs that hold an elision comment.
    pub ranges: Vec<TextRange>,
}

pub(crate) struct Normalizer<'a, 'c> {
    pub graph: &'a ModuleGraph<'c>,
    pub eval: &'a Evaluator<'a, 'c>,
    /// The module in whose context the definition is read (its tag and extension defaults apply).
    pub module: ModuleId,
    pub form: Form,
    pub marks: &'a PartialMarks,
    /// Dummy references of the enclosing parameterized assignment.
    pub dummies: &'a [String],
}

/// The tokens of `node` written compactly, trivia removed.
pub(crate) fn compact_text(node: &SyntaxNode) -> String {
    compact_with(node, |token| token.text().to_owned())
}

fn compact_with(node: &SyntaxNode, map: impl Fn(&glint_syntax::SyntaxToken) -> String) -> String {
    let mut out = String::new();
    let mut previous: Option<SyntaxKind> = None;
    for token in node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
    {
        if let Some(prev) = previous
            && needs_space(prev, token.kind())
        {
            out.push(' ');
        }
        out.push_str(&map(&token));
        previous = Some(token.kind());
    }
    out
}

fn needs_space(previous: SyntaxKind, next: SyntaxKind) -> bool {
    use SyntaxKind::{
        At, Caret, Comma, Dot, DotDot, LBrace, LBracket, LDoubleBracket, LParen, Minus, RBrace,
        RBracket, RDoubleBracket, RParen, Semicolon,
    };
    if matches!(next, RBrace) || matches!(previous, LBrace) {
        return true;
    }
    if matches!(
        previous,
        LParen | LBracket | LDoubleBracket | Dot | DotDot | Minus | At | Caret
    ) {
        return false;
    }
    !matches!(
        next,
        RParen | RBracket | RDoubleBracket | Comma | Dot | DotDot | Semicolon | LParen
    )
}

fn strip_blanks(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

impl<'a, 'c> Normalizer<'a, 'c> {
    fn with_module(&self, module: ModuleId) -> Normalizer<'a, 'c> {
        Normalizer {
            module,
            graph: self.graph,
            eval: self.eval,
            form: self.form,
            marks: self.marks,
            dummies: self.dummies,
        }
    }

    fn normal(&self) -> bool {
        self.form == Form::Normal
    }

    fn is_partial(&self, node: &SyntaxNode) -> bool {
        self.marks.all || self.marks.ranges.contains(&trimmed_range(node))
    }

    fn other(&self, node: &SyntaxNode) -> Norm {
        Norm::leaf("other", compact_text(node), trimmed_range(node))
    }

    /// The normal form of an assignment (X.680 C.3.2).
    pub fn assignment(&self, node: &SyntaxNode) -> Norm {
        let label = match node.kind() {
            SyntaxKind::TypeAssignment => "type assignment",
            SyntaxKind::ValueAssignment => "value assignment",
            SyntaxKind::ValueSetAssignment => "value set assignment",
            _ => return self.other(node),
        };
        let name = tokens(node)
            .into_iter()
            .find(|t| t.kind().is_name())
            .map(|t| t.text().to_owned())
            .unwrap_or_default();
        let children = node
            .children()
            .map(|c| match c.kind() {
                SyntaxKind::ParameterList => {
                    Norm::leaf("parameters", compact_text(&c), trimmed_range(&c))
                }
                SyntaxKind::Value | SyntaxKind::XmlValue => self.value(&c),
                kind if is_type_node(kind) => self.ty(&c),
                _ => self.other(&c),
            })
            .collect();
        Norm::node(label, name, trimmed_range(node), children)
    }

    // X.680 C.3.2
    fn ty(&self, node: &SyntaxNode) -> Norm {
        match node.kind() {
            SyntaxKind::BuiltinType => Norm::leaf("type", builtin_text(node), trimmed_range(node)),
            SyntaxKind::IntegerType | SyntaxKind::BitStringType => self.named_list_type(node),
            SyntaxKind::EnumeratedType => self.enumerated(node),
            SyntaxKind::SequenceType | SyntaxKind::SetType | SyntaxKind::ChoiceType => {
                self.composite(node)
            }
            SyntaxKind::SequenceOfType | SyntaxKind::SetOfType => self.of_type(node),
            SyntaxKind::TaggedType => self.tagged(node),
            SyntaxKind::ConstrainedType => self.constrained(node),
            SyntaxKind::DefinedType => Norm::leaf("type", compact_text(node), trimmed_range(node)),
            _ => self.other(node),
        }
    }

    fn header(node: &SyntaxNode) -> String {
        tokens(node)
            .iter()
            .filter(|t| t.kind() == SyntaxKind::Keyword)
            .map(|t| t.text().to_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn integer_text(&self, value: &SyntaxNode) -> String {
        if self.normal()
            && let Some(number) = self.eval.eval_integer(self.module, value)
        {
            return number.to_string();
        }
        compact_text(value)
    }

    // X.680 19.1, 22.1; C.3.2.2 b, d
    fn named_list_type(&self, node: &SyntaxNode) -> Norm {
        let (label, noun) = if node.kind() == SyntaxKind::IntegerType {
            ("named number", "INTEGER")
        } else {
            ("named bit", "BIT STRING")
        };
        let mut items: Vec<Norm> = child(node, SyntaxKind::NamedNumberList)
            .into_iter()
            .flat_map(|list| list.children().collect::<Vec<_>>())
            .filter(|c| c.kind() == SyntaxKind::NamedNumber)
            .filter_map(|item| {
                let name = tokens(&item).into_iter().find(|t| t.kind().is_name())?;
                let value = child(&item, SyntaxKind::Value)
                    .map(|v| self.integer_text(&v))
                    .unwrap_or_default();
                Some(
                    Norm::leaf(
                        label,
                        format!("{}({value})", name.text()),
                        trimmed_range(&item),
                    )
                    .keyed(name.text()),
                )
            })
            .collect();
        if self.normal() {
            items.sort_by(|a, b| a.key.cmp(&b.key));
        }
        let mut out = Norm::node(
            "type",
            Self::header(node).if_empty(noun),
            trimmed_range(node),
            items,
        );
        out.partial = self.is_partial(node);
        out
    }

    fn marker(&self, node: &SyntaxNode) -> Norm {
        let exception = child(node, SyntaxKind::ExceptionSpec)
            .map(|e| format!(" {}", compact_text(&e)))
            .unwrap_or_default();
        Norm::leaf(
            "extension marker",
            format!("...{exception}"),
            trimmed_range(node),
        )
        .keyed("...")
    }

    // X.680 20.1-20.6; C.3.2.2 c
    fn enumerated(&self, node: &SyntaxNode) -> Norm {
        struct Item {
            name: String,
            value: Option<String>,
            number: Option<i128>,
            range: TextRange,
        }
        let mut root: Vec<Item> = Vec::new();
        let mut additions: Vec<Item> = Vec::new();
        let mut marker: Option<Norm> = None;
        let mut raw_children: Vec<Norm> = Vec::new();
        for item in node.children() {
            match item.kind() {
                SyntaxKind::ExtensionMarker => {
                    let norm = self.marker(&item);
                    if marker.is_none() {
                        marker = Some(norm.clone());
                    }
                    raw_children.push(norm);
                }
                SyntaxKind::Enumeration => {
                    let Some(name) = tokens(&item).into_iter().find(|t| t.kind().is_name()) else {
                        continue;
                    };
                    let value_node = child(&item, SyntaxKind::Value);
                    let number = value_node
                        .as_ref()
                        .and_then(|v| self.eval.eval_integer(self.module, v));
                    let entry = Item {
                        name: name.text().to_owned(),
                        value: value_node.as_ref().map(|v| self.integer_text(v)),
                        number,
                        range: trimmed_range(&item),
                    };
                    if !self.normal() {
                        let text = match &entry.value {
                            Some(v) => format!("{}({v})", entry.name),
                            None => entry.name.clone(),
                        };
                        raw_children.push(
                            Norm::leaf("enumeration item", text, entry.range).keyed(&entry.name),
                        );
                    }
                    if marker.is_some() {
                        additions.push(entry);
                    } else {
                        root.push(entry);
                    }
                }
                _ => {}
            }
        }
        let mut out = if self.normal() {
            let mut used: BTreeSet<i128> = root.iter().filter_map(|i| i.number).collect();
            let mut next: i128 = 0;
            let mut highest: Option<i128> = root.iter().filter_map(|i| i.number).max();
            let mut numbered: Vec<Norm> = Vec::new();
            for item in &mut root {
                let number = match item.number {
                    Some(n) => n,
                    None => {
                        while used.contains(&next) {
                            next = next.saturating_add(1);
                        }
                        let n = next;
                        used.insert(n);
                        highest = Some(highest.map_or(n, |h| h.max(n)));
                        n
                    }
                };
                numbered.push(
                    Norm::leaf(
                        "enumeration item",
                        format!("{}({number})", item.name),
                        item.range,
                    )
                    .keyed(&item.name),
                );
            }
            numbered.sort_by(|a, b| a.key.cmp(&b.key));
            if let Some(marker) = marker {
                numbered.push(marker);
            }
            let addition_used: BTreeSet<i128> = additions.iter().filter_map(|i| i.number).collect();
            for item in &additions {
                let number = match item.number {
                    Some(n) => n,
                    None => {
                        let mut n = highest.map_or(0, |h| h.saturating_add(1));
                        while addition_used.contains(&n) || used.contains(&n) {
                            n = n.saturating_add(1);
                        }
                        n
                    }
                };
                used.insert(number);
                highest = Some(highest.map_or(number, |h| h.max(number)));
                numbered.push(
                    Norm::leaf(
                        "enumeration item",
                        format!("{}({number})", item.name),
                        item.range,
                    )
                    .keyed(&item.name),
                );
            }
            Norm::node("type", "ENUMERATED", trimmed_range(node), numbered)
        } else {
            Norm::node("type", "ENUMERATED", trimmed_range(node), raw_children)
        };
        out.partial = self.is_partial(node);
        out
    }

    fn module_context(&self) -> Option<(TagDefault, bool)> {
        let module = self.graph.module(self.module)?;
        Some((module.model.tag_default, module.model.extensibility_implied))
    }

    // X.680 31.2.7
    fn forces_explicit(&self, inner: &SyntaxNode) -> bool {
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
                    let Some(reference) = crate::view::defined_type_ref(&node) else {
                        return false;
                    };
                    if reference.module.is_none() && self.dummies.contains(&reference.name) {
                        return true;
                    }
                    return self.graph.peel_type(self.module, &node).is_some_and(|p| {
                        matches!(p.node.kind(), SyntaxKind::ChoiceType | SyntaxKind::AnyType)
                    });
                }
                _ => return false,
            }
        }
    }

    // X.680 31.2; C.3.2.2 g, h
    fn tagged(&self, node: &SyntaxNode) -> Norm {
        let range = trimmed_range(node);
        let Some(tag_node) = child(node, SyntaxKind::Tag) else {
            return self.other(node);
        };
        let Some(inner) = type_child(node) else {
            return self.other(node);
        };
        let keyword = tokens(node)
            .into_iter()
            .find(|t| {
                t.kind() == SyntaxKind::Keyword && matches!(t.text(), "IMPLICIT" | "EXPLICIT")
            })
            .map(|t| t.text().to_owned());
        let inner_norm = self.ty(&inner);
        if !self.normal() {
            let tagging = Norm::leaf(
                "tagging",
                keyword.unwrap_or_else(|| DEFAULT_TAGGING.to_owned()),
                range,
            );
            return Norm::node(
                "tagged",
                compact_text(&tag_node),
                range,
                vec![tagging, inner_norm],
            );
        }
        let decl = read_tag(&tag_node);
        let number = decl.number.or_else(|| {
            let (qualifier, name) = decl.reference.as_ref()?;
            let value = self
                .eval
                .integer_of_name(self.module, qualifier.as_deref(), name)?;
            u128::try_from(value).ok()
        });
        let atom = match number {
            Some(n) => tag_atom(decl.class, n),
            None => compact_text(&tag_node),
        };
        let default_implicit = self.module_context().is_some_and(|(default, _)| {
            matches!(default, TagDefault::Implicit | TagDefault::Automatic)
        });
        let implicit = match keyword.as_deref() {
            Some("IMPLICIT") => true,
            Some(_) => false,
            None => default_implicit,
        } && !self.forces_explicit(&inner);
        let tagging = Norm::leaf(
            "tagging",
            if implicit { "IMPLICIT" } else { "EXPLICIT" },
            range,
        );
        Norm::node("tagged", atom, range, vec![tagging, inner_norm])
    }

    fn constraint(&self, node: &SyntaxNode) -> Norm {
        let text = if self.normal() && node.kind() == SyntaxKind::Constraint {
            node.children()
                .map(|c| compact_text(&c))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            compact_text(node)
        };
        Norm::leaf("constraint", text, trimmed_range(node))
    }

    fn constrained(&self, node: &SyntaxNode) -> Norm {
        let children = node
            .children()
            .map(|c| {
                if is_type_node(c.kind()) {
                    self.ty(&c)
                } else {
                    self.constraint(&c)
                }
            })
            .collect();
        Norm::node("constrained", "", trimmed_range(node), children)
    }

    // X.680 26.1, 28.1; X.680 49.5
    fn of_type(&self, node: &SyntaxNode) -> Norm {
        let header = if node.kind() == SyntaxKind::SequenceOfType {
            "SEQUENCE OF"
        } else {
            "SET OF"
        };
        let mut children = Vec::new();
        for item in node.children() {
            match item.kind() {
                SyntaxKind::Constraint => children.push(self.constraint(&item)),
                SyntaxKind::SizeElement => {
                    let text = compact_text(&item);
                    let atom = if self.normal() {
                        text
                    } else {
                        format!("{text} (bare)")
                    };
                    children.push(Norm::leaf("constraint", atom, trimmed_range(&item)));
                }
                SyntaxKind::NamedType => {
                    if let Some(name) = tokens(&item).into_iter().find(|t| t.kind().is_name()) {
                        children.push(Norm::leaf("element name", name.text(), name.text_range()));
                    }
                    if let Some(element) = type_child(&item) {
                        children.push(self.ty(&element));
                    }
                }
                kind if is_type_node(kind) => children.push(self.ty(&item)),
                _ => {}
            }
        }
        Norm::node("type", header, trimmed_range(node), children)
    }

    // X.680 C.3.2.5
    pub fn value(&self, node: &SyntaxNode) -> Norm {
        let range = trimmed_range(node);
        if self.normal() {
            let braces = child(node, SyntaxKind::BraceGroup);
            if let Some(group) = braces
                && node.children().count() == 1
            {
                let evaluated = self.eval.eval_oid_group(self.module, &group);
                if let Some(oid) = evaluated.oid
                    && !oid.is_empty()
                {
                    return Norm::leaf("value", format!("OID {oid}"), range);
                }
                let names_a_definition = tokens(&group)
                    .iter()
                    .any(|t| t.kind() == SyntaxKind::LowerName);
                if names_a_definition && !evaluated.references.is_empty() {
                    let mut leaf = Norm::leaf("value", compact_text(node), range);
                    leaf.uncertain = true;
                    return leaf;
                }
            }
            let text = compact_with(node, |t| {
                if t.kind() == SyntaxKind::QuotedString {
                    strip_blanks(t.text())
                } else {
                    t.text().to_owned()
                }
            });
            return Norm::leaf("value", text, range);
        }
        Norm::leaf("value", compact_text(node), range)
    }

    fn presence(&self, optional: bool, default: Option<&SyntaxNode>, range: TextRange) -> Norm {
        let atom = match (optional, default) {
            (_, Some(value)) => format!("DEFAULT {}", self.value(value).atom),
            (true, None) => "OPTIONAL".to_owned(),
            (false, None) => "mandatory".to_owned(),
        };
        Norm::leaf("presence", atom, range)
    }

    fn component_norm(
        &self,
        label: &'static str,
        component: &crate::view::Component,
        automatic_index: Option<usize>,
    ) -> Norm {
        let mut ty = self.ty(&component.ty);
        if let Some(index) = automatic_index {
            let range = trimmed_range(&component.ty);
            let tagging = Norm::leaf(
                "tagging",
                if self.forces_explicit(&component.ty) {
                    "EXPLICIT"
                } else {
                    "IMPLICIT"
                },
                range,
            );
            ty = Norm::node("tagged", format!("[{index}]"), range, vec![tagging, ty]);
        }
        let mut children = vec![ty];
        if label == "component" {
            children.push(self.presence(
                component.optional,
                component.default.as_ref(),
                component.name_range,
            ));
        }
        Norm::node(
            label,
            component.name.clone(),
            component.name_range,
            children,
        )
        .keyed(component.name.clone())
    }

    // X.680 25, 27, 29; C.3.2.2 g, h; C.3.2.3 c, d; C.3.2.4
    fn composite(&self, node: &SyntaxNode) -> Norm {
        let header = match node.kind() {
            SyntaxKind::SequenceType => "SEQUENCE",
            SyntaxKind::SetType => "SET",
            _ => "CHOICE",
        };
        let label = if node.kind() == SyntaxKind::ChoiceType {
            "alternative"
        } else {
            "component"
        };
        let children = if self.normal() {
            self.composite_normal(node, header, label)
        } else {
            self.composite_raw(node, label)
        };
        let mut out = Norm::node("type", header, trimmed_range(node), children);
        out.partial = self.is_partial(node);
        out
    }

    fn composite_raw(&self, node: &SyntaxNode, label: &'static str) -> Vec<Norm> {
        let mut out = Vec::new();
        let mut group_index = 0usize;
        for item in node.children() {
            match item.kind() {
                SyntaxKind::ComponentType => {
                    if let Some(component) = read_component(&item, Region::Root, None) {
                        out.push(self.component_norm(label, &component, None));
                    }
                }
                SyntaxKind::ComponentsOf => {
                    out.push(
                        Norm::leaf("components of", compact_text(&item), trimmed_range(&item))
                            .keyed(format!("COMPONENTS OF {}", compact_text(&item))),
                    );
                }
                SyntaxKind::ExtensionMarker => out.push(self.marker(&item)),
                SyntaxKind::ExtensionAdditionGroup => {
                    let version = child(&item, SyntaxKind::VersionNumber)
                        .map(|v| compact_text(&v))
                        .unwrap_or_default();
                    let members = item
                        .children()
                        .filter(|c| c.kind() == SyntaxKind::ComponentType)
                        .filter_map(|c| read_component(&c, Region::Addition, Some(group_index)))
                        .map(|c| self.component_norm(label, &c, None))
                        .collect();
                    out.push(
                        Norm::node("addition group", version, trimmed_range(&item), members)
                            .keyed(format!("[[{group_index}")),
                    );
                    group_index = group_index.saturating_add(1);
                }
                _ => {}
            }
        }
        out
    }

    fn composite_normal(&self, node: &SyntaxNode, header: &str, label: &'static str) -> Vec<Norm> {
        let list = ComponentList::read(node);
        let flat = expand(self.graph, self.module, &list, 0);
        let (module_tags, implied) = self.module_context().unzip();
        let extensible = list.is_extensible() || implied.unwrap_or(false);
        let automatic = module_tags == Some(TagDefault::Automatic)
            && !flat.iter().any(|f| {
                !f.expanded
                    && f.component.region != Region::Addition
                    && is_textually_tagged(&f.component.ty)
            });
        let (roots, additions): (Vec<_>, Vec<_>) = flat
            .iter()
            .partition(|f| f.component.region != Region::Addition);
        let mut numbered: Vec<(usize, &crate::checks::components::Flat)> = roots
            .iter()
            .chain(additions.iter())
            .copied()
            .enumerate()
            .collect();
        let split = roots.len();
        let additions_numbered = numbered.split_off(split.min(numbered.len()));
        let build = |index: usize, entry: &crate::checks::components::Flat| {
            let automatic_index =
                (automatic && !is_textually_tagged(&entry.component.ty)).then_some(index);
            self.with_module(entry.module)
                .component_norm(label, &entry.component, automatic_index)
        };
        let mut root_norms: Vec<Norm> = numbered.iter().map(|(i, e)| build(*i, e)).collect();
        if header != "SEQUENCE" {
            root_norms.sort_by(|a, b| a.key.cmp(&b.key));
        }
        let mut out = root_norms;
        if extensible {
            let marker = node
                .children()
                .find(|c| c.kind() == SyntaxKind::ExtensionMarker)
                .map(|m| self.marker(&m))
                .unwrap_or_else(|| {
                    Norm::leaf("extension marker", "...", trimmed_range(node)).keyed("...")
                });
            out.push(marker);
        }
        let mut current: Option<(usize, Norm)> = None;
        let mut group_count = 0usize;
        for (index, entry) in additions_numbered {
            let norm = build(index, entry);
            match (entry.component.group, current.as_mut()) {
                (Some(g), Some((open, group))) if *open == g => group.children.push(norm),
                (Some(g), _) => {
                    if let Some((_, group)) = current.take() {
                        out.push(group);
                    }
                    let version = list
                        .groups
                        .get(g)
                        .and_then(|group| group.version)
                        .map(|(v, _)| format!("{v}:"))
                        .unwrap_or_default();
                    let group = Norm::node("addition group", version, norm.range, vec![norm])
                        .keyed(format!("[[{group_count}"));
                    group_count = group_count.saturating_add(1);
                    current = Some((g, group));
                }
                (None, _) => {
                    if let Some((_, group)) = current.take() {
                        out.push(group);
                    }
                    out.push(norm);
                }
            }
        }
        if let Some((_, group)) = current.take() {
            out.push(group);
        }
        out
    }
}

fn tag_atom(class: crate::view::TagClass, number: u128) -> String {
    use crate::view::TagClass;
    match class {
        TagClass::Context => format!("[{number}]"),
        TagClass::Universal => format!("[UNIVERSAL {number}]"),
        TagClass::Application => format!("[APPLICATION {number}]"),
        TagClass::Private => format!("[PRIVATE {number}]"),
    }
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_owned()
        } else {
            self
        }
    }
}
