//! Tagging checks: E120-E125 (X.680 25.2-25.10, 27.3, 29.2-29.9, 31.2, 52.7; X.681 14.2).

use std::fmt;

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use super::Ctx;
use super::components::{Flat, expand};
use crate::finding::Finding;
use crate::graph::{Found, ModuleGraph, ModuleId};
use crate::model::SymbolKind;
use crate::view::{
    ComponentList, Region, TagClass, builtin_by_text, builtin_text, child, defined_type_ref,
    is_textually_tagged, read_tag, tokens, trimmed_range, type_child,
};

const MAX_DEPTH: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tag {
    pub class: TagClass,
    pub number: u128,
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.class {
            TagClass::Context => write!(f, "[{}]", self.number),
            TagClass::Universal => write!(f, "[UNIVERSAL {}]", self.number),
            TagClass::Application => write!(f, "[APPLICATION {}]", self.number),
            TagClass::Private => write!(f, "[PRIVATE {}]", self.number),
        }
    }
}

/// The tags a value of a type can start with (X.680 29.9), plus the indeterminate tag of 52.7.3.
#[derive(Debug, Clone)]
pub(crate) enum TagSet {
    Unknown,
    Known {
        tags: Vec<Tag>,
        /// Matches the tag of the conceptually added element (X.680 52.7.3).
        extension: bool,
        /// An open type: its tag is indeterminate and distinct from no other tag (X.681 14.2 b).
        indeterminate: bool,
    },
}

enum Overlap {
    Tag(Tag),
    Extension,
    Indeterminate,
}

impl TagSet {
    fn one(tag: Tag) -> Self {
        Self::Known {
            tags: vec![tag],
            extension: false,
            indeterminate: false,
        }
    }

    fn universal(number: u32) -> Self {
        Self::one(Tag {
            class: TagClass::Universal,
            number: u128::from(number),
        })
    }

    fn open() -> Self {
        Self::Known {
            tags: Vec::new(),
            extension: true,
            indeterminate: true,
        }
    }

    fn conceptual() -> Self {
        Self::Known {
            tags: Vec::new(),
            extension: true,
            indeterminate: false,
        }
    }

    fn overlap(&self, other: &Self) -> Option<Overlap> {
        let (
            Self::Known {
                tags: a,
                extension: ea,
                indeterminate: ia,
            },
            Self::Known {
                tags: b,
                extension: eb,
                indeterminate: ib,
            },
        ) = (self, other)
        else {
            return None;
        };
        if *ia || *ib {
            return Some(Overlap::Indeterminate);
        }
        if let Some(tag) = a.iter().find(|t| b.contains(t)) {
            return Some(Overlap::Tag(tag.clone()));
        }
        (*ea && *eb).then_some(Overlap::Extension)
    }
}

struct Names {
    name: String,
    range: TextRange,
}

struct Elem {
    names: Names,
    set: TagSet,
    optional: bool,
    region: Region,
}

pub(crate) struct Tagger<'a, 'g, 'c> {
    pub graph: &'g ModuleGraph<'c>,
    pub eval: &'a crate::eval::Evaluator<'g, 'c>,
}

impl Tagger<'_, '_, '_> {
    fn module_is_automatic(&self, module: ModuleId) -> bool {
        self.graph
            .module(module)
            .is_some_and(|m| m.model.is_automatic())
    }

    fn module_ext_implied(&self, module: ModuleId) -> bool {
        self.graph
            .module(module)
            .is_some_and(|m| m.model.extensibility_implied)
    }

    // X.680 25.3, 29.2: automatic tagging applies only when no root component is textually tagged
    fn automatic_applies(&self, module: ModuleId, flat: &[Flat]) -> bool {
        self.module_is_automatic(module)
            && !flat.iter().any(|f| {
                !f.expanded
                    && f.component.region != Region::Addition
                    && is_textually_tagged(&f.component.ty)
            })
    }

    // X.680 25.10, 29.5
    fn element_sets(&self, module: ModuleId, flat: &[Flat], depth: usize) -> Vec<TagSet> {
        if self.automatic_applies(module, flat) {
            return (0..flat.len())
                .map(|i| {
                    TagSet::one(Tag {
                        class: TagClass::Context,
                        number: u128::try_from(i).unwrap_or_default(),
                    })
                })
                .collect();
        }
        flat.iter()
            .map(|f| self.tag_set(f.module, &f.component.ty, depth.saturating_add(1)))
            .collect()
    }

    // X.680 8.4, 31.2.4, 29.9; X.681 14.2
    pub(crate) fn tag_set(&self, module: ModuleId, ty: &SyntaxNode, depth: usize) -> TagSet {
        if depth > MAX_DEPTH {
            return TagSet::Unknown;
        }
        match ty.kind() {
            SyntaxKind::TaggedType => self.explicit_tag(module, ty),
            SyntaxKind::EncodingPrefixedType | SyntaxKind::ConstrainedType => type_child(ty)
                .map_or(TagSet::Unknown, |inner| {
                    self.tag_set(module, &inner, depth.saturating_add(1))
                }),
            SyntaxKind::DefinedType => self.defined(module, ty, depth),
            SyntaxKind::BuiltinType => builtin_by_text(&builtin_text(ty))
                .map_or(TagSet::Unknown, |(_, number)| TagSet::universal(number)),
            SyntaxKind::IntegerType => TagSet::universal(2),
            SyntaxKind::BitStringType => TagSet::universal(3),
            SyntaxKind::EnumeratedType => TagSet::universal(10),
            SyntaxKind::SequenceType | SyntaxKind::SequenceOfType => TagSet::universal(16),
            SyntaxKind::SetType | SyntaxKind::SetOfType => TagSet::universal(17),
            SyntaxKind::InstanceOfType => TagSet::universal(8),
            SyntaxKind::ChoiceType => self.choice(module, ty, depth),
            SyntaxKind::SelectionType => self.selection(module, ty, depth),
            SyntaxKind::ObjectClassFieldType => {
                let last_is_type_field = child(ty, SyntaxKind::FieldName)
                    .and_then(|f| tokens(&f).into_iter().last())
                    .is_some_and(|t| t.kind() == SyntaxKind::FieldUpper);
                if last_is_type_field {
                    TagSet::open()
                } else {
                    TagSet::Unknown
                }
            }
            _ => TagSet::Unknown,
        }
    }

    fn explicit_tag(&self, module: ModuleId, ty: &SyntaxNode) -> TagSet {
        let Some(tag_node) = child(ty, SyntaxKind::Tag) else {
            return TagSet::Unknown;
        };
        let decl = read_tag(&tag_node);
        let number = decl.number.or_else(|| {
            let (qualifier, name) = decl.reference.as_ref()?;
            let value = self
                .eval
                .integer_of_name(module, qualifier.as_deref(), name)?;
            u128::try_from(value).ok()
        });
        number.map_or(TagSet::Unknown, |number| {
            TagSet::one(Tag {
                class: decl.class,
                number,
            })
        })
    }

    fn defined(&self, module: ModuleId, ty: &SyntaxNode, depth: usize) -> TagSet {
        let Some(reference) = defined_type_ref(ty) else {
            return TagSet::Unknown;
        };
        if reference.module.is_none() && enclosing_parameters(ty).contains(&reference.name) {
            return TagSet::Unknown;
        }
        let lookup = match &reference.module {
            Some(q) => self.graph.lookup_qualified(module, q, &reference.name).ok(),
            None => self.graph.lookup(module, &reference.name),
        };
        let Some(Found::Symbol {
            module: target,
            index,
        }) = lookup.map(|l| l.found)
        else {
            return TagSet::Unknown;
        };
        let Some(symbol) = self.graph.symbol(target, index) else {
            return TagSet::Unknown;
        };
        if symbol.kind != SymbolKind::Type || symbol.is_parameterized() {
            return TagSet::Unknown;
        }
        match &symbol.type_node {
            Some(node) => self.tag_set(target, node, depth.saturating_add(1)),
            None => TagSet::Unknown,
        }
    }

    // X.680 29.9: every possible tag of an untagged CHOICE counts
    fn choice(&self, module: ModuleId, node: &SyntaxNode, depth: usize) -> TagSet {
        let list = ComponentList::read(node);
        let flat = expand(self.graph, module, &list, 0);
        let sets = self.element_sets(module, &flat, depth);
        let mut tags: Vec<Tag> = Vec::new();
        let mut extension = list.is_extensible() || self.module_ext_implied(module);
        let mut indeterminate = false;
        for set in sets {
            match set {
                TagSet::Unknown => return TagSet::Unknown,
                TagSet::Known {
                    tags: more,
                    extension: ext,
                    indeterminate: open,
                } => {
                    for tag in more {
                        if !tags.contains(&tag) {
                            tags.push(tag);
                        }
                    }
                    extension |= ext;
                    indeterminate |= open;
                }
            }
        }
        TagSet::Known {
            tags,
            extension,
            indeterminate,
        }
    }

    // X.680 30.1
    fn selection(&self, module: ModuleId, node: &SyntaxNode, depth: usize) -> TagSet {
        let Some(name) = tokens(node)
            .into_iter()
            .find(|t| t.kind() == SyntaxKind::LowerName)
        else {
            return TagSet::Unknown;
        };
        let Some(inner) = type_child(node) else {
            return TagSet::Unknown;
        };
        let Some(peeled) = self.graph.peel_type_through_tags(module, &inner) else {
            return TagSet::Unknown;
        };
        if peeled.node.kind() != SyntaxKind::ChoiceType {
            return TagSet::Unknown;
        }
        let list = ComponentList::read(&peeled.node);
        let flat = expand(self.graph, peeled.module, &list, 0);
        let sets = self.element_sets(peeled.module, &flat, depth);
        flat.iter()
            .zip(sets)
            .find(|(f, _)| f.component.name == name.text())
            .map_or(TagSet::Unknown, |(_, set)| set)
    }
}

/// The dummy references of the assignment that contains `node` (X.683 8.1).
fn enclosing_parameters(node: &SyntaxNode) -> Vec<String> {
    node.ancestors()
        .find(|n| {
            matches!(
                n.kind(),
                SyntaxKind::TypeAssignment
                    | SyntaxKind::ValueAssignment
                    | SyntaxKind::ValueSetAssignment
            )
        })
        .and_then(|assignment| child(&assignment, SyntaxKind::ParameterList))
        .into_iter()
        .flat_map(|list| list.children().collect::<Vec<_>>())
        .filter(|p| p.kind() == SyntaxKind::Parameter)
        .filter_map(|p| tokens(&p).into_iter().rev().find(|t| t.kind().is_name()))
        .map(|t| t.text().to_owned())
        .collect()
}

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    let tagger = Tagger {
        graph: ctx.graph,
        eval: ctx.eval,
    };
    let mut findings: Vec<Finding> = Vec::new();
    let loaded = ctx.loaded;
    let source = loaded.source;
    let loc = move |range: TextRange| crate::finding::Loc { source, range };
    let version_mixing = module_uses_versions(&loaded.node.syntax().clone());
    for symbol in &loaded.model.symbols {
        if matches!(symbol.kind, SymbolKind::Macro | SymbolKind::Unknown) {
            continue;
        }
        for node in symbol.node.descendants() {
            match node.kind() {
                SyntaxKind::TaggedType => check_tagged(ctx, &tagger, &node, &mut findings, &loc),
                SyntaxKind::SequenceType | SyntaxKind::SetType | SyntaxKind::ChoiceType => {
                    check_container(ctx, &tagger, &node, version_mixing, &mut findings, &loc);
                }
                _ => {}
            }
        }
    }
    ctx.findings.extend(findings);
}

// X.680 31.2.5, 31.2.9
fn check_tagged(
    ctx: &Ctx<'_, '_, '_>,
    tagger: &Tagger<'_, '_, '_>,
    node: &SyntaxNode,
    findings: &mut Vec<Finding>,
    loc: &impl Fn(TextRange) -> crate::finding::Loc,
) {
    let Some(tag_node) = child(node, SyntaxKind::Tag) else {
        return;
    };
    let decl = read_tag(&tag_node);
    if decl.class == TagClass::Universal {
        findings.push(Finding::new(
            "E123",
            "UNIVERSAL class tags are reserved for types defined in X.680",
            loc(decl.range),
        ));
    }
    let implicit = tokens(node)
        .into_iter()
        .find(|t| t.kind() == SyntaxKind::Keyword && t.text() == "IMPLICIT");
    let (Some(keyword), Some(inner)) = (implicit, type_child(node)) else {
        return;
    };
    let reason = untagged_dynamic_kind(ctx, tagger, &inner);
    if let Some(what) = reason {
        findings.push(
            Finding::new(
                "E122",
                format!("`IMPLICIT` cannot be applied to {what}"),
                loc(keyword.text_range()),
            )
            .with_help(
                "remove `IMPLICIT`: tagging an untagged CHOICE or open type is always explicit",
            ),
        );
    }
}

/// Names the kind when `inner` is an untagged choice, open type or dummy reference (X.680 31.2.9).
fn untagged_dynamic_kind(
    ctx: &Ctx<'_, '_, '_>,
    tagger: &Tagger<'_, '_, '_>,
    inner: &SyntaxNode,
) -> Option<&'static str> {
    let mut node = inner.clone();
    loop {
        match node.kind() {
            SyntaxKind::ConstrainedType | SyntaxKind::EncodingPrefixedType => {
                node = type_child(&node)?;
            }
            SyntaxKind::ChoiceType => return Some("an untagged CHOICE"),
            SyntaxKind::AnyType => return Some("an open type"),
            SyntaxKind::ObjectClassFieldType => {
                let type_field = child(&node, SyntaxKind::FieldName)
                    .and_then(|f| tokens(&f).into_iter().last())
                    .is_some_and(|t| t.kind() == SyntaxKind::FieldUpper);
                return type_field.then_some("an open type");
            }
            SyntaxKind::DefinedType => {
                let reference = defined_type_ref(&node)?;
                if reference.module.is_none()
                    && enclosing_parameters(&node).contains(&reference.name)
                {
                    return Some("an untagged dummy reference");
                }
                let peeled = tagger.graph.peel_type(ctx.module, &node)?;
                return match peeled.node.kind() {
                    SyntaxKind::ChoiceType => Some("an untagged CHOICE"),
                    SyntaxKind::AnyType => Some("an open type"),
                    SyntaxKind::ObjectClassFieldType => {
                        let type_field = child(&peeled.node, SyntaxKind::FieldName)
                            .and_then(|f| tokens(&f).into_iter().last())
                            .is_some_and(|t| t.kind() == SyntaxKind::FieldUpper);
                        type_field.then_some("an open type")
                    }
                    _ => None,
                };
            }
            _ => return None,
        }
    }
}

fn module_uses_versions(module: &SyntaxNode) -> bool {
    module
        .descendants()
        .any(|n| n.kind() == SyntaxKind::VersionNumber)
}

fn check_container(
    ctx: &Ctx<'_, '_, '_>,
    tagger: &Tagger<'_, '_, '_>,
    node: &SyntaxNode,
    version_mixing: bool,
    findings: &mut Vec<Finding>,
    loc: &impl Fn(TextRange) -> crate::finding::Loc,
) {
    let list = ComponentList::read(node);
    check_versions(&list, version_mixing, findings, loc);
    let flat = expand(ctx.graph, ctx.module, &list, 0);
    let automatic = tagger.automatic_applies(ctx.module, &flat);

    // X.680 25.9, 29.4
    if automatic {
        for entry in &flat {
            if entry.component.region == Region::Addition
                && is_textually_tagged(&entry.component.ty)
            {
                findings.push(
                    Finding::new(
                        "E124",
                        format!(
                            "extension addition `{}` is tagged, but the root components are untagged under AUTOMATIC TAGS",
                            entry.component.name
                        ),
                        loc(entry.component.name_range),
                    )
                    .with_help("remove the tag, or tag the root components too"),
                );
            }
        }
        return;
    }

    let sets = tagger.element_sets(ctx.module, &flat, 0);
    let extensible = list.is_extensible() || tagger.module_ext_implied(ctx.module);
    let marker_range = list
        .markers
        .last()
        .copied()
        .unwrap_or_else(|| trimmed_range(node));
    let elems: Vec<Elem> = flat
        .iter()
        .zip(sets)
        .map(|(f, set)| Elem {
            names: Names {
                name: f.component.name.clone(),
                range: if f.expanded {
                    trimmed_range(node)
                } else {
                    f.component.name_range
                },
            },
            set,
            optional: f.component.is_optional_or_default(),
            region: f.component.region,
        })
        .collect();

    match node.kind() {
        SyntaxKind::SequenceType => check_sequence(&elems, extensible, marker_range, findings, loc),
        kind => {
            let what = if kind == SyntaxKind::ChoiceType {
                "alternatives"
            } else {
                "components"
            };
            check_distinct(&elems, extensible, marker_range, what, findings, loc);
        }
    }
}

// X.680 27.3, 29.3, 52.7
fn check_distinct(
    elems: &[Elem],
    extensible: bool,
    marker_range: TextRange,
    what: &str,
    findings: &mut Vec<Finding>,
    loc: &impl Fn(TextRange) -> crate::finding::Loc,
) {
    for (j, later) in elems.iter().enumerate() {
        for earlier in elems.iter().take(j) {
            if let Some(overlap) = earlier.set.overlap(&later.set) {
                findings.push(clash("E120", what, earlier, later, &overlap, loc));
            }
        }
        if extensible && let Some(overlap) = later.set.overlap(&TagSet::conceptual()) {
            findings.push(extension_clash("E120", later, marker_range, &overlap, loc));
        }
    }
}

// X.680 25.6, 25.7, 52.7
fn check_sequence(
    elems: &[Elem],
    extensible: bool,
    marker_range: TextRange,
    findings: &mut Vec<Finding>,
    loc: &impl Fn(TextRange) -> crate::finding::Loc,
) {
    let conceptual = Elem {
        names: Names {
            name: "the extension insertion point".to_owned(),
            range: marker_range,
        },
        set: TagSet::conceptual(),
        optional: false,
        region: Region::Addition,
    };
    let mut series: Vec<&Elem> = elems
        .iter()
        .filter(|e| e.region != Region::TrailingRoot)
        .collect();
    if extensible {
        series.push(&conceptual);
    }
    series.extend(elems.iter().filter(|e| e.region == Region::TrailingRoot));

    let mut i = 0;
    while let Some(first) = series.get(i) {
        if !first.optional {
            i = i.saturating_add(1);
            continue;
        }
        let run_end = series
            .iter()
            .enumerate()
            .skip(i)
            .find(|(_, e)| !e.optional)
            .map_or(series.len(), |(index, _)| index);
        let group_end = run_end.saturating_add(1).min(series.len());
        let group: Vec<&&Elem> = series.iter().take(group_end).skip(i).collect();
        for (j, later) in group.iter().enumerate() {
            for earlier in group.iter().take(j) {
                if let Some(overlap) = earlier.set.overlap(&later.set) {
                    findings.push(sequence_clash(earlier, later, &overlap, loc));
                }
            }
        }
        i = run_end;
    }

    // X.680 25.7: extension additions against the root components that follow them
    let trailing: Vec<&Elem> = elems
        .iter()
        .filter(|e| e.region == Region::TrailingRoot)
        .collect();
    let following_end = trailing
        .iter()
        .position(|e| !e.optional)
        .map_or(trailing.len(), |p| p.saturating_add(1));
    for addition in elems.iter().filter(|e| e.region == Region::Addition) {
        for later in trailing.iter().take(following_end) {
            if let Some(overlap) = addition.set.overlap(&later.set) {
                findings.push(sequence_clash(addition, later, &overlap, loc));
            }
        }
    }
}

fn describe(overlap: &Overlap) -> String {
    match overlap {
        Overlap::Tag(tag) => format!("the tag {tag}"),
        Overlap::Extension => {
            "a tag that clashes with the conceptual element of an extensible CHOICE".to_owned()
        }
        Overlap::Indeterminate => "an open type, whose tag is indeterminate".to_owned(),
    }
}

fn clash(
    code: &'static str,
    what: &str,
    earlier: &Elem,
    later: &Elem,
    overlap: &Overlap,
    loc: &impl Fn(TextRange) -> crate::finding::Loc,
) -> Finding {
    Finding::new(
        code,
        format!(
            "{what} `{}` and `{}` have {}",
            earlier.names.name,
            later.names.name,
            describe(overlap)
        ),
        loc(later.names.range),
    )
    .with_secondary(loc(earlier.names.range), "same tag here")
}

fn sequence_clash(
    earlier: &Elem,
    later: &Elem,
    overlap: &Overlap,
    loc: &impl Fn(TextRange) -> crate::finding::Loc,
) -> Finding {
    Finding::new(
        "E121",
        format!(
            "`{}` follows the OPTIONAL or DEFAULT component `{}` with {}",
            later.names.name,
            earlier.names.name,
            describe(overlap)
        ),
        loc(later.names.range),
    )
    .with_secondary(loc(earlier.names.range), "clashes with this component")
}

fn extension_clash(
    code: &'static str,
    element: &Elem,
    marker: TextRange,
    overlap: &Overlap,
    loc: &impl Fn(TextRange) -> crate::finding::Loc,
) -> Finding {
    Finding::new(
        code,
        format!(
            "`{}` has {}, which clashes with the conceptual element added at the extension marker",
            element.names.name,
            describe(overlap)
        ),
        loc(element.names.range),
    )
    .with_secondary(loc(marker), "extension marker (X.680 52.7)")
}

// X.680 25.16
fn check_versions(
    list: &ComponentList,
    module_uses_versions: bool,
    findings: &mut Vec<Finding>,
    loc: &impl Fn(TextRange) -> crate::finding::Loc,
) {
    if module_uses_versions {
        for component in list.components() {
            if component.region == Region::Addition && component.group.is_none() {
                findings.push(Finding::new(
                    "E125",
                    "a VersionNumber is used in this module, so every extension addition must be a version group",
                    loc(component.name_range),
                ));
            }
        }
        for group in &list.groups {
            if group.version.is_none() {
                findings.push(Finding::new(
                    "E125",
                    "a VersionNumber is used in this module, so every extension addition group needs one",
                    loc(group.range),
                ));
            }
        }
    }
    let mut previous: Option<u128> = None;
    for group in &list.groups {
        let Some((number, range)) = group.version else {
            continue;
        };
        if number < 2 {
            findings.push(Finding::new(
                "E125",
                "a VersionNumber must be 2 or more",
                loc(range),
            ));
        }
        if let Some(before) = previous
            && number <= before
        {
            findings.push(Finding::new(
                "E125",
                format!("VersionNumber {number} does not increase over the previous {before}"),
                loc(range),
            ));
        }
        previous = Some(number);
    }
}
