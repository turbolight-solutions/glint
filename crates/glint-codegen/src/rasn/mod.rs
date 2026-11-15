//! The rasn backend: derive-based types for the rasn codec framework, with the constraint programs, open-type
//! dispatch and value tree of [`glint_rt`](glint_rt.rs) implemented for each.
//!
//! Every named type is a Rust type of its own: a struct for a `SEQUENCE` or `SET`, an enum for a `CHOICE`, an
//! `ENUMERATED` or a dispatch, and a `delegate` newtype for everything else, so each carries its own checks. A
//! module of ASN.1 is a Rust module; `mod.rs` declares them next to the runtime.

use std::fmt::Write as _;

use glint_ir::{StringKind, Summary, TagClass, TagLayer, TagMode, TimeKind};

use crate::naming::{self, Namer};
use crate::plan::{
    Body, Check, Const, Dispatch, DispatchId, EmitPlan, Field, FieldPresence, Leaf, Literal,
    ModulePlan, PType, PTypeId, Resolution, Rule, Target, Use,
};
use crate::{Backend, BackendMeta, Capabilities, File, Finding, Language, Options, Output};

const RUNTIME: &str = include_str!("glint_rt.rs");

pub(crate) struct Rasn {
    meta: BackendMeta,
    caps: Capabilities,
}

pub(crate) static RASN: Rasn = Rasn {
    meta: BackendMeta {
        id: "rasn",
        display_name: "rasn (Rust)",
        language: Language::Rust,
        library: "rasn",
        library_version: "0.30.0",
    },
    caps: Capabilities {
        element_tags: false,
        max_use_tags: 1,
        tagged_records: true,
        pattern: false,
        real: false,
    },
};

struct RustNamer;

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "box", "break", "const", "continue", "crate", "do", "dyn", "else",
    "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl", "in", "let", "loop",
    "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return", "self",
    "static", "struct", "super", "trait", "true", "try", "type", "typeof", "union", "unsafe",
    "unsized", "use", "virtual", "where", "while", "yield", "abstract", "become",
];

const RESERVED_TYPES: &[&str] = &[
    "Self",
    "Option",
    "Some",
    "None",
    "Result",
    "Ok",
    "Err",
    "Box",
    "Vec",
    "String",
    "Default",
    "Clone",
    "Copy",
    "Debug",
    "Eq",
    "PartialEq",
    "Hash",
    "Send",
    "Sync",
    "Sized",
    "Drop",
    "Fn",
    "FnMut",
    "FnOnce",
    "Iterator",
    "ToString",
    "Into",
    "From",
    "AsRef",
    "TryFrom",
    "TryInto",
];

fn escape(name: String, prefix: &str) -> String {
    let name = if name.is_empty() {
        prefix.to_owned()
    } else if name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("{prefix}{name}")
    } else {
        name
    };
    if KEYWORDS.contains(&name.as_str()) {
        format!("{name}_")
    } else {
        name
    }
}

impl Namer for RustNamer {
    fn type_name(&self, asn1: &str) -> String {
        escape(naming::upper_camel(asn1), "T")
    }
    fn field_name(&self, asn1: &str) -> String {
        escape(naming::snake(asn1), "f_")
    }
    fn variant_name(&self, asn1: &str) -> String {
        let name = escape(naming::upper_camel(asn1), "V");
        if name == "Self" {
            "Self_".to_owned()
        } else {
            name
        }
    }
    fn const_name(&self, asn1: &str) -> String {
        escape(naming::screaming(asn1), "C_")
    }
    fn module_name(&self, asn1: &str) -> String {
        let name = escape(naming::snake(asn1), "m_");
        if name == "glint_rt" {
            "glint_rt_".to_owned()
        } else {
            name
        }
    }
    fn reserved_types(&self) -> &[&str] {
        RESERVED_TYPES
    }
}

static NAMER: RustNamer = RustNamer;

impl Backend for Rasn {
    fn meta(&self) -> &BackendMeta {
        &self.meta
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn namer(&self) -> &dyn Namer {
        &NAMER
    }

    fn emit(&self, plan: &EmitPlan, opts: &Options) -> Output {
        let mut output = Output::default();
        let mut findings = Vec::new();
        for (index, module) in plan.modules.iter().enumerate() {
            let mut e = Emitter {
                plan,
                module: index,
                out: String::new(),
                private: String::new(),
                rules: 0,
                findings: &mut findings,
            };
            e.module(module, opts, &self.meta);
            let contents = e.finish(module, opts, &self.meta);
            output.files.push(File {
                path: format!("{}.rs", module.name),
                contents,
            });
        }
        output.files.push(File {
            path: "mod.rs".to_owned(),
            contents: mod_rs(plan, opts, &self.meta),
        });
        output.files.push(self.runtime());
        output.findings = findings;
        output
    }

    fn runtime(&self) -> File {
        File {
            path: "glint_rt.rs".to_owned(),
            contents: RUNTIME.to_owned(),
        }
    }
}

fn header(out: &mut String, plan: &EmitPlan, opts: &Options, meta: &BackendMeta) {
    let _ = writeln!(
        out,
        "//! Generated by Glint {} with the {} backend, for {} {}.",
        opts.glint_version, meta.id, meta.library, meta.library_version
    );
    let _ = writeln!(out, "//! Root type: {}.", plan.root_display);
    if let Some(build) = &opts.data_build {
        let _ = writeln!(out, "//! Corpus data build: {build}.");
    }
    out.push_str("//! Do not edit: regenerate it with `glint codegen`.\n");
}

fn module_line(m: &ModulePlan) -> String {
    let mut line = m.asn1.clone();
    if let Some(oid) = &m.oid {
        let _ = write!(line, " {{ {oid} }}");
    }
    if let Some(document) = &m.document {
        let _ = write!(line, " ({document})");
    }
    line
}

fn mod_rs(plan: &EmitPlan, opts: &Options, meta: &BackendMeta) -> String {
    let mut out = String::new();
    header(&mut out, plan, opts, meta);
    out.push_str("//!\n//! Modules:\n");
    for m in &plan.modules {
        let _ = writeln!(out, "//! - `{}`: {}", m.name, module_line(m));
    }
    out.push_str("//!\n//! `glint_rt` is the runtime these modules need, vendored (MIT-0).\n\n");
    out.push_str("pub mod glint_rt;\n");
    for m in &plan.modules {
        let _ = writeln!(out, "pub mod {};", m.name);
    }
    if let Some(root) = plan.ty(plan.root)
        && let Some(m) = plan.modules.get(root.module)
    {
        let _ = writeln!(
            out,
            "\n#[allow(unused_imports)]\npub use self::{}::{};",
            m.name, root.name
        );
    }
    out
}

fn class(class: TagClass) -> &'static str {
    match class {
        TagClass::Universal => "universal",
        TagClass::Application => "application",
        TagClass::Context => "context",
        TagClass::Private => "private",
    }
}

fn tag_attr(layer: &TagLayer) -> String {
    let inner = format!("{}, {}", class(layer.tag.class), layer.tag.number);
    match layer.mode {
        TagMode::Explicit => format!("tag(explicit({inner}))"),
        TagMode::Implicit => format!("tag({inner})"),
    }
}

/// The narrowest native integer that holds every value the summary permits, else rasn's arbitrary-precision one.
fn int_type(summary: &Summary) -> &'static str {
    let Some(range) = summary.value.filter(|r| !r.extensible) else {
        return "rasn::types::Integer";
    };
    let (Some(lo), Some(hi)) = (range.lo, range.hi) else {
        return "rasn::types::Integer";
    };
    let fits = |min: i128, max: i128| lo >= min && hi <= max;
    if lo >= 0 {
        if fits(0, i128::from(u8::MAX)) {
            "u8"
        } else if fits(0, i128::from(u16::MAX)) {
            "u16"
        } else if fits(0, i128::from(u32::MAX)) {
            "u32"
        } else if fits(0, i128::from(u64::MAX)) {
            "u64"
        } else {
            "rasn::types::Integer"
        }
    } else if fits(i128::from(i8::MIN), i128::from(i8::MAX)) {
        "i8"
    } else if fits(i128::from(i16::MIN), i128::from(i16::MAX)) {
        "i16"
    } else if fits(i128::from(i32::MIN), i128::from(i32::MAX)) {
        "i32"
    } else if fits(i128::from(i64::MIN), i128::from(i64::MAX)) {
        "i64"
    } else {
        "rasn::types::Integer"
    }
}

fn string_type(kind: StringKind) -> &'static str {
    match kind {
        StringKind::UTF8String => "rasn::types::Utf8String",
        StringKind::NumericString => "rasn::types::NumericString",
        StringKind::PrintableString => "rasn::types::PrintableString",
        StringKind::TeletexString => "rasn::types::TeletexString",
        StringKind::IA5String => "rasn::types::Ia5String",
        StringKind::GraphicString => "rasn::types::GraphicString",
        StringKind::VisibleString => "rasn::types::VisibleString",
        StringKind::GeneralString => "rasn::types::GeneralString",
        StringKind::BMPString => "rasn::types::BmpString",
        StringKind::UniversalString => "glint_rt::Raw<28>",
        StringKind::VideotexString => "glint_rt::Raw<21>",
        StringKind::ObjectDescriptor => "glint_rt::Raw<7>",
    }
}

fn time_type(kind: TimeKind) -> String {
    match kind {
        TimeKind::UTCTime => "rasn::types::UtcTime".to_owned(),
        TimeKind::GeneralizedTime => "rasn::types::GeneralizedTime".to_owned(),
        other => format!("glint_rt::Raw<{}>", other.universal_tag()),
    }
}

fn check_code(check: &Check) -> String {
    let list = |items: &[Check]| items.iter().map(check_code).collect::<Vec<_>>().join(", ");
    match check {
        Check::Pass | Check::Pattern(_) => "glint_rt::Check::Pass".to_owned(),
        Check::All(items) => format!("glint_rt::Check::All(&[{}])", list(items)),
        Check::Any(items) => format!("glint_rt::Check::Any(&[{}])", list(items)),
        Check::Not(c) => format!("glint_rt::Check::Not(&{})", check_code(c)),
        Check::Int(ranges) => {
            let bound =
                |b: &Option<i128>| b.map_or_else(|| "None".to_owned(), |v| format!("Some({v})"));
            let items: Vec<String> = ranges
                .iter()
                .map(|(lo, hi)| format!("({}, {})", bound(lo), bound(hi)))
                .collect();
            format!("glint_rt::Check::Int(&[{}])", items.join(", "))
        }
        Check::Equals(c) => format!("glint_rt::Check::Equals({})", const_code(c)),
        Check::Size(c) => format!("glint_rt::Check::Size(&{})", check_code(c)),
        Check::Alphabet(c) => format!("glint_rt::Check::Alphabet(&{})", check_code(c)),
        Check::Component { name, check } => {
            format!(
                "glint_rt::Check::Component({name:?}, &{})",
                check_code(check)
            )
        }
        Check::Presence { name, present } => {
            format!("glint_rt::Check::Presence({name:?}, {present})")
        }
        Check::Each(c) => format!("glint_rt::Check::Each(&{})", check_code(c)),
    }
}

fn join<T: ToString>(items: &[T]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn const_code(c: &Const) -> String {
    match c {
        Const::Bool(b) => format!("glint_rt::Const::Bool({b})"),
        Const::Int(n) => format!("glint_rt::Const::Int({n})"),
        Const::Null => "glint_rt::Const::Null".to_owned(),
        Const::Oid(arcs) => format!("glint_rt::Const::Oid(&[{}])", join(arcs)),
        Const::Str(s) => format!("glint_rt::Const::Str({s:?})"),
        Const::Bits(bits) => format!("glint_rt::Const::Bits(&[{}])", join(bits)),
        Const::Octets(octets) => format!("glint_rt::Const::Octets(&[{}])", join(octets)),
    }
}

struct Emitter<'p, 'f> {
    plan: &'p EmitPlan,
    module: usize,
    /// The public items.
    out: String,
    /// Rule constants and default functions, after the public items.
    private: String,
    rules: usize,
    findings: &'f mut Vec<Finding>,
}

impl Emitter<'_, '_> {
    fn finish(self, m: &ModulePlan, opts: &Options, meta: &BackendMeta) -> String {
        let mut file = String::new();
        header(&mut file, self.plan, opts, meta);
        let _ = writeln!(file, "//!\n//! ASN.1 module {}.", module_line(m));
        file.push_str("\n#![allow(clippy::all, clippy::pedantic, dead_code, unused_imports, unused_variables, non_camel_case_types, non_snake_case, irrefutable_let_patterns, unreachable_patterns)]\n\n");
        file.push_str("use rasn::types::{AsnType as _, Choice as _, Constructed as _, DecodeChoice as _, Enumerated as _};\nuse rasn::{Decode as _, Decoder as _, Encode as _, Encoder as _};\n\nuse super::glint_rt;\n\n");
        file.push_str(&self.out);
        if !self.private.is_empty() {
            file.push_str("// ---- constraints and defaults ----\n\n");
            file.push_str(&self.private);
        }
        file
    }

    fn module(&mut self, m: &ModulePlan, _opts: &Options, _meta: &BackendMeta) {
        if !m.oids.is_empty() {
            for o in &m.oids {
                let _ = writeln!(
                    self.out,
                    "/// `{}`: {}\npub const {}: &rasn::types::Oid = rasn::types::Oid::const_new(&[{}]);",
                    o.asn1,
                    o.arcs
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join("."),
                    o.name,
                    join(&o.arcs)
                );
            }
            self.out.push('\n');
        }
        for &id in &m.types {
            if let Some(t) = self.plan.ty(id) {
                self.ty(id, t);
            }
        }
        for &id in &m.dispatches {
            if let Some(d) = self.plan.dispatch(id) {
                self.dispatch(d);
            }
        }
    }

    fn finding(&mut self, at: &str, message: impl Into<String>) {
        let f = Finding::new("W420", at, message);
        if !self.findings.contains(&f) {
            self.findings.push(f);
        }
    }

    // ---- names and types ------------------------------------------------------------------------

    fn type_path(&self, id: PTypeId) -> String {
        let Some(t) = self.plan.ty(id) else {
            return "rasn::types::Any".to_owned();
        };
        self.qualified(t.module, &t.name)
    }

    fn dispatch_path(&self, id: DispatchId) -> String {
        let Some(d) = self.plan.dispatch(id) else {
            return "rasn::types::Any".to_owned();
        };
        self.qualified(d.module, &d.name)
    }

    fn qualified(&self, module: usize, name: &str) -> String {
        if module == self.module {
            name.to_owned()
        } else {
            let m = self
                .plan
                .modules
                .get(module)
                .map_or("", |m| m.name.as_str());
            format!("super::{m}::{name}")
        }
    }

    fn leaf_type(&mut self, leaf: &Leaf, summary: &Summary) -> String {
        match leaf {
            Leaf::Boolean => "bool".to_owned(),
            Leaf::Integer { .. } => int_type(summary).to_owned(),
            Leaf::Real => "rasn::types::Any".to_owned(),
            Leaf::Null => "()".to_owned(),
            Leaf::BitString { .. } => "rasn::types::BitString".to_owned(),
            Leaf::OctetString => "rasn::types::OctetString".to_owned(),
            Leaf::ObjectIdentifier => "rasn::types::ObjectIdentifier".to_owned(),
            Leaf::RelativeOid => "glint_rt::Raw<13>".to_owned(),
            Leaf::String(kind) => string_type(*kind).to_owned(),
            Leaf::Time(kind) => time_type(*kind),
        }
    }

    /// The Rust type of a use, without `Option` or `Box`.
    fn use_type(&mut self, use_: &Use) -> String {
        match &use_.target {
            Target::Leaf(leaf) => self.leaf_type(leaf, &use_.summary),
            Target::Named(id) => self.type_path(*id),
            Target::List { set, element, .. } => {
                let inner = self.use_type(element);
                if *set {
                    format!("rasn::types::SetOf<{inner}>")
                } else {
                    format!("Vec<{inner}>")
                }
            }
            Target::Dispatch(id) => self.dispatch_path(*id),
            Target::Any => "rasn::types::Any".to_owned(),
            Target::Containing { bits, inner } => {
                let open = matches!(inner.target, Target::Dispatch(_));
                let wrapper = match (bits, open) {
                    (false, false) => "Containing",
                    (true, false) => "BitsContaining",
                    (false, true) => "ContainingOpen",
                    (true, true) => "BitsContainingOpen",
                };
                format!("glint_rt::{wrapper}<{}>", self.use_type(inner))
            }
        }
    }

    fn field_type(&mut self, field: &Field) -> String {
        let mut ty = self.use_type(&field.use_);
        if field.boxed {
            ty = format!("Box<{ty}>");
        }
        if matches!(field.presence, FieldPresence::Optional) {
            ty = format!("Option<{ty}>");
        }
        ty
    }

    // ---- checks, trees, resolution --------------------------------------------------------------

    /// A constant of `rules`, by name.
    fn rules_const(&mut self, rules: &[Rule]) -> String {
        let name = format!("RULES_{}", self.rules);
        self.rules = self.rules.saturating_add(1);
        let items: Vec<String> = rules
            .iter()
            .map(|r| {
                format!(
                    "    glint_rt::Rule {{ check: {}, text: {:?} }},",
                    check_code(&r.check),
                    r.text
                )
            })
            .collect();
        let _ = writeln!(
            self.private,
            "const {name}: &[glint_rt::Rule] = &[\n{}\n];\n",
            items.join("\n")
        );
        name
    }

    /// Statements that validate the value `expr` (a reference) as `use_`, at the path `path` (a `String`).
    fn validate_code(&mut self, expr: &str, use_: &Use, path: &str, indent: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{indent}{{");
        let _ = writeln!(out, "{indent}    let p: String = {path};");
        if !use_.rules.is_empty() {
            let name = self.rules_const(&use_.rules);
            let _ = writeln!(out, "{indent}    glint_rt::check({name}, {expr}, &p)?;");
        }
        match &use_.target {
            Target::List { element, .. } if needs_element_code(element) => {
                let inner = self.validate_code(
                    "e",
                    element,
                    "glint_rt::index(&p, i)",
                    &format!("{indent}        "),
                );
                let _ = writeln!(
                    out,
                    "{indent}    for (i, e) in ({expr}).iter().enumerate() {{\n{inner}{indent}    }}"
                );
            }
            _ => {
                let _ = writeln!(out, "{indent}    glint_rt::Glint::validate({expr}, &p)?;");
            }
        }
        let _ = writeln!(out, "{indent}}}");
        out
    }

    /// An expression of the tree of `expr` (a reference) as `use_`.
    fn tree_code(&mut self, expr: &str, use_: &Use) -> String {
        match &use_.target {
            Target::List {
                element_name,
                element,
                ..
            } => {
                let inner = self.tree_code("e", element);
                format!("glint_rt::list_tree(({expr}).iter().map(|e| {inner}), {element_name:?})")
            }
            _ => format!("glint_rt::Glint::tree({expr})"),
        }
    }

    // ---- types ------------------------------------------------------------------------------------

    fn doc(&mut self, t: &PType) {
        let _ = writeln!(self.out, "/// `{}`", t.display);
    }

    fn ty(&mut self, id: PTypeId, t: &PType) {
        match &t.body {
            Body::Record {
                set,
                fields,
                extensible,
            } => self.record(id, t, *set, fields, *extensible),
            Body::Choice { alternatives, .. } => self.choice(t, alternatives),
            Body::Enumerated {
                items,
                additions,
                extensible,
            } => self.enumerated(t, items, additions, *extensible),
            Body::Alias(use_) => self.alias(t, use_),
        }
        self.out.push('\n');
    }

    fn identifier(t: &PType) -> String {
        t.display
            .rsplit(['.', ' '])
            .next()
            .unwrap_or(&t.name)
            .to_owned()
    }

    fn record(&mut self, id: PTypeId, t: &PType, set: bool, fields: &[Field], extensible: bool) {
        self.doc(t);
        self.out.push_str("#[derive(rasn::AsnType, rasn::Decode, rasn::Encode, Debug, Clone, PartialEq, Eq, Hash)]\n");
        let mut attrs = Vec::new();
        if set {
            attrs.push("set".to_owned());
        }
        if let Some(layer) = t.tags.first() {
            attrs.push(tag_attr(layer));
        }
        attrs.push(format!("identifier = {:?}", Self::identifier(t)));
        let _ = writeln!(self.out, "#[rasn({})]", attrs.join(", "));
        let _ = writeln!(self.out, "pub struct {} {{", t.name);
        for field in fields {
            let mut attrs = Vec::new();
            if let Some(layer) = field.use_.tags.first() {
                attrs.push(tag_attr(layer));
            }
            if let FieldPresence::Default(_) = field.presence {
                attrs.push(format!("default = {:?}", default_fn(t, field)));
            }
            attrs.push(format!("identifier = {:?}", field.asn1));
            let ty = self.field_type(field);
            let _ = writeln!(
                self.out,
                "    #[rasn({})]\n    pub {}: {ty},",
                attrs.join(", "),
                field.name
            );
        }
        for (i, field) in fields.iter().enumerate() {
            if !matches!(field.presence, FieldPresence::Required)
                && field.use_.tags.is_empty()
                && i.saturating_add(1) < fields.len()
                && self.untagged_dynamic(&field.use_)
            {
                self.finding(
                    &format!("{}.{}", t.display, field.asn1),
                    "rasn cannot tell this absent untagged CHOICE or open type from the component after it: a value without it does not decode",
                );
            }
        }
        if keeps_extensions(set, fields, extensible) {
            self.out.push_str("    /// Extension additions this type does not know, kept to re-encode.\n    pub _extensions: glint_rt::Extensions,\n");
        }
        self.out.push_str("}\n\n");
        for field in fields {
            if let FieldPresence::Default(literal) = &field.presence {
                let ty = self.field_type(field);
                let value = self.literal_code(literal, &field.use_).map(|v| {
                    if field.boxed {
                        format!("Box::new({v})")
                    } else {
                        v
                    }
                });
                match value {
                    Some(value) => {
                        let _ = writeln!(
                            self.private,
                            "pub(super) fn {}() -> {ty} {{\n    {value}\n}}\n",
                            default_fn(t, field)
                        );
                    }
                    None => {
                        self.finding(
                            &format!("{}.{}", t.display, field.asn1),
                            "the DEFAULT value cannot be written; the type's default stands in",
                        );
                        let _ = writeln!(
                            self.private,
                            "pub(super) fn {}() -> {ty} {{\n    Default::default()\n}}\n",
                            default_fn(t, field)
                        );
                    }
                }
            }
        }
        // the Glint impl
        let _ = writeln!(self.out, "impl glint_rt::Glint for {} {{", t.name);
        // validate
        self.out
            .push_str("    fn validate(&self, path: &str) -> Result<(), glint_rt::Error> {\n");
        if !t.rules.is_empty() {
            let name = self.rules_const(&t.rules);
            let _ = writeln!(self.out, "        glint_rt::check({name}, self, path)?;");
        }
        for field in fields {
            let path = format!("glint_rt::join(path, {:?})", field.asn1);
            match field.presence {
                FieldPresence::Optional => {
                    let code = self.validate_code("v", &field.use_, &path, "            ");
                    let _ = writeln!(
                        self.out,
                        "        if let Some(v) = &self.{} {{\n{code}        }}",
                        field.name
                    );
                }
                _ => {
                    let code = self.validate_code(
                        &format!("&self.{}", field.name),
                        &field.use_,
                        &path,
                        "        ",
                    );
                    self.out.push_str(&code);
                }
            }
        }
        self.out.push_str("        Ok(())\n    }\n");
        // resolve
        let resolving =
            !t.resolutions.is_empty() || fields.iter().any(|f| self.may_hold_open(&f.use_));
        if resolving {
            self.out.push_str(
                "    fn resolve(&mut self, path: &str) -> Result<(), glint_rt::Error> {\n",
            );
        }
        for field in fields.iter().filter(|_| resolving) {
            if !self.may_hold_open(&field.use_) {
                continue;
            }
            let path = format!("&glint_rt::join(path, {:?})", field.asn1);
            match field.presence {
                FieldPresence::Optional => {
                    let _ = writeln!(
                        self.out,
                        "        if let Some(v) = &mut self.{} {{\n            glint_rt::Glint::resolve(v, {path})?;\n        }}",
                        field.name
                    );
                }
                _ => {
                    let _ = writeln!(
                        self.out,
                        "        glint_rt::Glint::resolve(&mut self.{}, {path})?;",
                        field.name
                    );
                }
            }
        }
        for r in &t.resolutions {
            let _ = writeln!(
                self.out,
                "        glint_rt::resolve_at(self, &{}, path)?;",
                resolution_code(r)
            );
        }
        if resolving {
            self.out.push_str("        Ok(())\n    }\n");
        }
        // tree
        self.out.push_str(
            "    fn tree(&self) -> glint_rt::Tree {\n        let mut items = Vec::new();\n",
        );
        for field in fields {
            match &field.presence {
                FieldPresence::Optional => {
                    let tree = self.tree_code("v", &field.use_);
                    let _ = writeln!(
                        self.out,
                        "        if let Some(v) = &self.{} {{\n            items.push({tree}.labeled({:?}));\n        }}",
                        field.name, field.asn1
                    );
                }
                FieldPresence::Default(_) => {
                    let tree = self.tree_code(&format!("&self.{}", field.name), &field.use_);
                    let _ = writeln!(
                        self.out,
                        "        if self.{} != {}() {{\n            items.push({tree}.labeled({:?}));\n        }}",
                        field.name,
                        default_fn(t, field),
                        field.asn1
                    );
                }
                FieldPresence::Required => {
                    let tree = self.tree_code(&format!("&self.{}", field.name), &field.use_);
                    let _ = writeln!(
                        self.out,
                        "        items.push({tree}.labeled({:?}));",
                        field.asn1
                    );
                }
            }
        }
        self.out
            .push_str("        glint_rt::Tree::list(items)\n    }\n");
        // components
        for mutable in [false, true] {
            let (m, as_ref, deref) = if mutable {
                ("mut ", "as_mut", "&mut ")
            } else {
                ("", "as_ref", "&")
            };
            let _ = writeln!(
                self.out,
                "    fn component{}(&{m}self, name: &str) -> Option<Option<&{m}dyn glint_rt::Glint>> {{\n        Some(match name {{",
                if mutable { "_mut" } else { "" }
            );
            for field in fields {
                let value = match (&field.presence, field.boxed) {
                    (FieldPresence::Optional, false) => format!(
                        "self.{}.{as_ref}().map(|v| v as &{m}dyn glint_rt::Glint)",
                        field.name
                    ),
                    (FieldPresence::Optional, true) => format!(
                        "self.{}.{as_ref}().map(|v| &{m}**v as &{m}dyn glint_rt::Glint)",
                        field.name
                    ),
                    (_, false) => format!(
                        "Some({deref}self.{} as &{m}dyn glint_rt::Glint)",
                        field.name
                    ),
                    (_, true) => format!(
                        "Some({deref}*self.{} as &{m}dyn glint_rt::Glint)",
                        field.name
                    ),
                };
                let _ = writeln!(self.out, "            {:?} => {value},", field.asn1);
            }
            self.out
                .push_str("            _ => return None,\n        })\n    }\n");
        }
        self.out.push_str("}\n");
        let _ = id;
    }

    /// Whether `use_` is a `CHOICE` or an open type, which rasn decodes by trying it rather than by its tag.
    fn untagged_dynamic(&self, use_: &Use) -> bool {
        match &use_.target {
            Target::Dispatch(_) | Target::Any => true,
            Target::Named(id) => self.plan.ty(*id).is_some_and(|t| {
                t.tags.is_empty()
                    && match &t.body {
                        Body::Choice { .. } => true,
                        Body::Alias(inner) => inner.tags.is_empty() && self.untagged_dynamic(inner),
                        _ => false,
                    }
            }),
            _ => false,
        }
    }

    /// Whether a value of `use_` can hold an open type, so its `resolve` has work to do.
    fn may_hold_open(&self, use_: &Use) -> bool {
        let mut seen = Vec::new();
        self.holds_open(use_, &mut seen)
    }

    fn holds_open(&self, use_: &Use, seen: &mut Vec<PTypeId>) -> bool {
        match &use_.target {
            Target::Dispatch(_) => true,
            Target::Containing { inner, .. } => self.holds_open(inner, seen),
            Target::List { element, .. } => self.holds_open(element, seen),
            Target::Named(id) => {
                if seen.contains(id) {
                    return false;
                }
                seen.push(*id);
                let Some(t) = self.plan.ty(*id) else {
                    return false;
                };
                if !t.resolutions.is_empty() {
                    return true;
                }
                match &t.body {
                    Body::Alias(u) => self.holds_open(u, seen),
                    Body::Record { fields, .. } => {
                        fields.iter().any(|f| self.holds_open(&f.use_, seen))
                    }
                    Body::Choice { alternatives, .. } => {
                        alternatives.iter().any(|f| self.holds_open(&f.use_, seen))
                    }
                    Body::Enumerated { .. } => false,
                }
            }
            Target::Leaf(_) | Target::Any => false,
        }
    }

    fn choice(&mut self, t: &PType, alternatives: &[Field]) {
        self.doc(t);
        self.out.push_str("#[derive(rasn::AsnType, rasn::Decode, rasn::Encode, Debug, Clone, PartialEq, Eq, Hash)]\n");
        let _ = writeln!(
            self.out,
            "#[rasn(choice, identifier = {:?})]",
            Self::identifier(t)
        );
        let _ = writeln!(self.out, "pub enum {} {{", t.name);
        for alt in alternatives {
            let mut attrs = Vec::new();
            if let Some(layer) = alt.use_.tags.first() {
                attrs.push(tag_attr(layer));
            }
            attrs.push(format!("identifier = {:?}", alt.asn1));
            let mut ty = self.use_type(&alt.use_);
            if alt.boxed {
                ty = format!("Box<{ty}>");
            }
            let _ = writeln!(
                self.out,
                "    #[rasn({})]\n    {}({ty}),",
                attrs.join(", "),
                alt.name
            );
        }
        self.out.push_str("}\n\n");
        let _ = writeln!(self.out, "impl glint_rt::Glint for {} {{", t.name);
        self.out
            .push_str("    fn validate(&self, path: &str) -> Result<(), glint_rt::Error> {\n");
        if !t.rules.is_empty() {
            let name = self.rules_const(&t.rules);
            let _ = writeln!(self.out, "        glint_rt::check({name}, self, path)?;");
        }
        self.out.push_str("        match self {\n");
        for alt in alternatives {
            let code = self.validate_code(
                "v",
                &alt.use_,
                &format!("glint_rt::join(path, {:?})", alt.asn1),
                "                ",
            );
            let _ = writeln!(
                self.out,
                "            Self::{}(v) => {{\n{code}            }}",
                alt.name
            );
        }
        self.out.push_str("        }\n        Ok(())\n    }\n");
        if alternatives.iter().any(|a| self.may_hold_open(&a.use_)) {
            self.out.push_str("    fn resolve(&mut self, path: &str) -> Result<(), glint_rt::Error> {\n        match self {\n");
            for alt in alternatives {
                let _ = writeln!(
                    self.out,
                    "            Self::{}(v) => glint_rt::Glint::resolve(v, &glint_rt::join(path, {:?})),",
                    alt.name, alt.asn1
                );
            }
            self.out.push_str("        }\n    }\n");
        }
        self.out
            .push_str("    fn tree(&self) -> glint_rt::Tree {\n        match self {\n");
        for alt in alternatives {
            let tree = self.tree_code("v", &alt.use_);
            let _ = writeln!(
                self.out,
                "            Self::{}(v) => {tree}.labeled({:?}),",
                alt.name, alt.asn1
            );
        }
        self.out.push_str("        }\n    }\n");
        for mutable in [false, true] {
            let m = if mutable { "mut " } else { "" };
            let _ = writeln!(
                self.out,
                "    fn component{}(&{m}self, name: &str) -> Option<Option<&{m}dyn glint_rt::Glint>> {{\n        match name {{",
                if mutable { "_mut" } else { "" }
            );
            for alt in alternatives {
                let value = if alt.boxed {
                    format!("&{m}**v")
                } else {
                    "v".to_owned()
                };
                let _ = writeln!(
                    self.out,
                    "            {:?} => Some(if let Self::{}(v) = self {{ Some({value} as &{m}dyn glint_rt::Glint) }} else {{ None }}),",
                    alt.asn1, alt.name
                );
            }
            self.out
                .push_str("            _ => None,\n        }\n    }\n");
        }
        self.out.push_str("}\n");
    }

    fn enumerated(
        &mut self,
        t: &PType,
        items: &[crate::plan::Item],
        additions: &[crate::plan::Item],
        extensible: bool,
    ) {
        self.doc(t);
        self.out.push_str("#[derive(rasn::AsnType, rasn::Decode, rasn::Encode, Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
        let _ = writeln!(
            self.out,
            "#[rasn(enumerated, identifier = {:?})]",
            Self::identifier(t)
        );
        if extensible {
            self.out.push_str("#[non_exhaustive]\n");
        }
        let _ = writeln!(self.out, "pub enum {} {{", t.name);
        for item in items {
            let _ = writeln!(
                self.out,
                "    #[rasn(identifier = {:?})]\n    {} = {},",
                item.asn1, item.name, item.value
            );
        }
        for item in additions {
            let _ = writeln!(
                self.out,
                "    #[rasn(extension_addition, identifier = {:?})]\n    {} = {},",
                item.asn1, item.name, item.value
            );
        }
        self.out.push_str("}\n\n");
        let _ = writeln!(self.out, "impl glint_rt::Glint for {} {{", t.name);
        if !t.rules.is_empty() {
            let name = self.rules_const(&t.rules);
            let _ = writeln!(
                self.out,
                "    fn validate(&self, path: &str) -> Result<(), glint_rt::Error> {{\n        glint_rt::check({name}, self, path)\n    }}"
            );
        }
        self.out.push_str("    fn tree(&self) -> glint_rt::Tree {\n        glint_rt::Tree::leaf((*self as isize).to_string())\n    }\n");
        self.out.push_str(
            "    fn int(&self) -> Option<i128> {\n        Some(*self as isize as i128)\n    }\n",
        );
        self.out.push_str("    fn equals(&self, c: &glint_rt::Const) -> Option<bool> {\n        match c {\n            glint_rt::Const::Int(v) => Some(*self as isize as i128 == *v),\n            _ => None,\n        }\n    }\n");
        self.out.push_str("}\n");
    }

    fn alias(&mut self, t: &PType, use_: &Use) {
        self.doc(t);
        let inner = self.use_type(use_);
        self.out.push_str("#[derive(rasn::AsnType, rasn::Decode, rasn::Encode, Debug, Clone, PartialEq, Eq, Hash)]\n");
        let mut attrs = vec!["delegate".to_owned()];
        if let Some(layer) = t.tags.first() {
            attrs.push(tag_attr(layer));
        }
        attrs.push(format!("identifier = {:?}", Self::identifier(t)));
        let _ = writeln!(self.out, "#[rasn({})]", attrs.join(", "));
        let _ = writeln!(self.out, "pub struct {}(pub {inner});\n", t.name);
        // named numbers and bits
        match self.plan.leaf_of(use_) {
            Some(Leaf::Integer { named })
                if !named.is_empty()
                    && matches!(use_.target, Target::Leaf(_))
                    && inner != "rasn::types::Integer" =>
            {
                let _ = writeln!(self.out, "impl {} {{", t.name);
                for n in named {
                    let _ = writeln!(
                        self.out,
                        "    pub const {}: Self = Self({});",
                        NAMER.const_name(&n.name),
                        n.value
                    );
                }
                self.out.push_str("}\n\n");
            }
            Some(Leaf::BitString { named })
                if !named.is_empty() && matches!(use_.target, Target::Leaf(_)) =>
            {
                let _ = writeln!(self.out, "impl {} {{", t.name);
                for n in named {
                    let _ = writeln!(
                        self.out,
                        "    /// The bit `{}`.\n    pub const {}: usize = {};",
                        n.name,
                        NAMER.const_name(&n.name),
                        n.value
                    );
                }
                self.out.push_str("}\n\n");
            }
            _ => {}
        }
        let _ = writeln!(self.out, "impl glint_rt::Glint for {} {{", t.name);
        self.out
            .push_str("    fn validate(&self, path: &str) -> Result<(), glint_rt::Error> {\n");
        if !t.rules.is_empty() {
            let name = self.rules_const(&t.rules);
            let _ = writeln!(self.out, "        glint_rt::check({name}, self, path)?;");
        }
        let code = self.validate_code("&self.0", use_, "path.to_owned()", "        ");
        self.out.push_str(&code);
        self.out.push_str("        Ok(())\n    }\n");
        let tree = self.tree_code("&self.0", use_);
        let _ = writeln!(
            self.out,
            "    fn tree(&self) -> glint_rt::Tree {{\n        {tree}\n    }}"
        );
        self.out.push_str(DELEGATE);
        self.out.push_str("}\n");
    }

    fn dispatch(&mut self, d: &Dispatch) {
        let _ = writeln!(
            self.out,
            "/// The open type `{}`: one variant per object that sets the field, keyed by its unique field.",
            d.display
        );
        self.out
            .push_str("#[derive(Debug, Clone, PartialEq, Eq, Hash)]\n");
        let _ = writeln!(self.out, "pub enum {} {{", d.name);
        let mut types = Vec::new();
        for v in &d.variants {
            let mut ty = self.use_type(&v.use_);
            types.push(ty.clone());
            if v.boxed {
                ty = format!("Box<{ty}>");
            }
            let _ = writeln!(
                self.out,
                "    /// `{}` ({})\n    {}({ty}),",
                v.asn1,
                v.key.text(),
                v.name
            );
        }
        self.out.push_str("    /// A value not resolved yet, or whose key the set gives no type.\n    Unknown(rasn::types::Any),\n}\n\n");
        let name = &d.name;
        let _ = writeln!(
            self.out,
            "impl rasn::AsnType for {name} {{\n    const TAG: rasn::types::Tag = rasn::types::Tag::EOC;\n    const TAG_TREE: rasn::types::TagTree = rasn::types::TagTree::Choice(&[]);\n}}\n"
        );
        let _ = writeln!(
            self.out,
            "impl rasn::Decode for {name} {{\n    fn decode_with_tag_and_constraints<D: rasn::Decoder>(decoder: &mut D, tag: rasn::types::Tag, _: &rasn::types::Constraints) -> Result<Self, D::Error> {{\n        decoder.decode_any(tag).map(Self::Unknown)\n    }}\n}}\n"
        );
        let _ = writeln!(
            self.out,
            "impl rasn::Encode for {name} {{\n    fn encode<'b, E: rasn::Encoder<'b>>(&self, encoder: &mut E) -> Result<(), E::Error> {{\n        match self {{"
        );
        for v in &d.variants {
            let _ = writeln!(
                self.out,
                "            Self::{}(v) => rasn::Encode::encode(v, encoder),",
                v.name
            );
        }
        self.out.push_str(
            "            Self::Unknown(v) => rasn::Encode::encode(v, encoder),\n        }\n    }\n",
        );
        self.out.push_str("    fn encode_with_tag_and_constraints<'b, E: rasn::Encoder<'b>>(&self, encoder: &mut E, _: rasn::types::Tag, _: &rasn::types::Constraints, _: rasn::types::Identifier) -> Result<(), E::Error> {\n        rasn::Encode::encode(self, encoder)\n    }\n}\n\n");
        // Open
        let _ = writeln!(
            self.out,
            "impl glint_rt::Open for {name} {{\n    fn open(key: Option<&glint_rt::Key>, bytes: &[u8], path: &str) -> Result<Option<Self>, glint_rt::Error> {{\n        let Some(key) = key else {{\n            return Ok(None);\n        }};"
        );
        for (v, ty) in d.variants.iter().zip(&types) {
            let test = key_test(&v.key);
            let wrap = if v.boxed {
                format!("Self::{}(Box::new(v))", v.name)
            } else {
                format!("Self::{}(v)", v.name)
            };
            let _ = writeln!(
                self.out,
                "        if {test} {{\n            return glint_rt::decode_inner::<{ty}>(bytes, path).map(|v| Some({wrap}));\n        }}"
            );
        }
        for key in &d.absent {
            let _ = writeln!(
                self.out,
                "        if {} {{\n            return Ok(None);\n        }}",
                key_test(key)
            );
        }
        if d.extensible {
            self.out.push_str("        Ok(None)\n");
        } else {
            let _ = writeln!(
                self.out,
                "        Err(glint_rt::Error::new(path, format!(\"{{key}} is not the key of an object of {}\", )))",
                d.display
                    .replace('"', "\\\"")
                    .replace('{', "{{")
                    .replace('}', "}}")
            );
        }
        self.out.push_str(
            "    }\n    fn key(&self) -> Option<glint_rt::Key> {\n        match self {\n",
        );
        for v in &d.variants {
            let key = match &v.key {
                crate::plan::KeyValue::Oid(arcs) => {
                    format!("glint_rt::Key::Oid(vec![{}])", join(arcs))
                }
                crate::plan::KeyValue::Integer(n) => format!("glint_rt::Key::Int({n})"),
            };
            let _ = writeln!(self.out, "            Self::{}(_) => Some({key}),", v.name);
        }
        self.out
            .push_str("            Self::Unknown(_) => None,\n        }\n    }\n}\n\n");
        // Glint
        let _ = writeln!(
            self.out,
            "impl glint_rt::Glint for {name} {{\n    fn validate(&self, path: &str) -> Result<(), glint_rt::Error> {{\n        match self {{"
        );
        for v in &d.variants {
            let code = self.validate_code("v", &v.use_, "path.to_owned()", "                ");
            let _ = writeln!(
                self.out,
                "            Self::{}(v) => {{\n{code}            }}",
                v.name
            );
        }
        self.out
            .push_str("            Self::Unknown(_) => {}\n        }\n        Ok(())\n    }\n");
        self.out.push_str("    fn resolve(&mut self, path: &str) -> Result<(), glint_rt::Error> {\n        match self {\n");
        for v in &d.variants {
            let _ = writeln!(
                self.out,
                "            Self::{}(v) => glint_rt::Glint::resolve(v, path),",
                v.name
            );
        }
        self.out
            .push_str("            Self::Unknown(_) => Ok(()),\n        }\n    }\n");
        self.out
            .push_str("    fn tree(&self) -> glint_rt::Tree {\n        match self {\n");
        for v in &d.variants {
            let tree = self.tree_code("v", &v.use_);
            let _ = writeln!(self.out, "            Self::{}(v) => {tree},", v.name);
        }
        self.out.push_str(
            "            Self::Unknown(v) => glint_rt::Glint::tree(v),\n        }\n    }\n",
        );
        self.out.push_str("    fn dispatch(&mut self, key: Option<&glint_rt::Key>, path: &str) -> Result<(), glint_rt::Error> {\n        let raw = match self {\n            Self::Unknown(v) => Some(v.as_bytes().to_vec()),\n            _ => None,\n        };\n        glint_rt::dispatch_open(self, raw.as_deref(), key, path)\n    }\n}\n\n");
    }

    // ---- literals ---------------------------------------------------------------------------------

    fn literal_code(&mut self, literal: &Literal, use_: &Use) -> Option<String> {
        Some(match literal {
            Literal::Bool(b) => b.to_string(),
            Literal::Int(n) => {
                let ty = self.use_type(use_);
                if ty == "rasn::types::Integer" {
                    format!("rasn::types::Integer::from({n}i128)")
                } else {
                    format!("{n}")
                }
            }
            Literal::Null => "()".to_owned(),
            Literal::Oid(arcs) => format!("glint_rt::oid(&[{}])", join(arcs)),
            Literal::Bits(bits) => format!("glint_rt::bits(&[{}])", join(bits)),
            Literal::Octets(octets) if octets.is_empty() => {
                "rasn::types::OctetString::from(Vec::<u8>::new())".to_owned()
            }
            Literal::Octets(octets) => format!(
                "rasn::types::OctetString::from(vec![{}u8])",
                join(octets).replacen(", ", "u8, ", 1)
            ),
            Literal::Str(s) => format!("TryFrom::try_from({s:?}).unwrap_or_default()"),
            Literal::Enum { ty, index } => {
                let t = self.plan.ty(*ty)?;
                let Body::Enumerated {
                    items, additions, ..
                } = &t.body
                else {
                    return None;
                };
                let item = items.iter().chain(additions).nth(*index)?;
                format!("{}::{}", self.type_path(*ty), item.name)
            }
            Literal::Newtype { ty, value } => {
                let t = self.plan.ty(*ty)?;
                let Body::Alias(inner) = &t.body else {
                    return None;
                };
                let inner = inner.clone();
                format!(
                    "{}({})",
                    self.type_path(*ty),
                    self.literal_code(value, &inner)?
                )
            }
            Literal::Choice {
                ty,
                alternative,
                value,
            } => {
                let t = self.plan.ty(*ty)?;
                let Body::Choice { alternatives, .. } = &t.body else {
                    return None;
                };
                let alt = alternatives.get(*alternative)?.clone();
                let mut v = self.literal_code(value, &alt.use_)?;
                if alt.boxed {
                    v = format!("Box::new({v})");
                }
                format!("{}::{}({v})", self.type_path(*ty), alt.name)
            }
            Literal::Record { ty, fields } => {
                let t = self.plan.ty(*ty)?;
                let Body::Record { fields: defs, .. } = &t.body else {
                    return None;
                };
                let defs = defs.clone();
                let mut parts = Vec::new();
                for (def, value) in defs.iter().zip(fields) {
                    let v = match (value, &def.presence) {
                        (None, FieldPresence::Optional) => "None".to_owned(),
                        (None, FieldPresence::Default(_)) => {
                            format!("{}()", self.qualified(t.module, &default_fn(t, def)))
                        }
                        (None, FieldPresence::Required) => return None,
                        (Some(v), presence) => {
                            let mut code = self.literal_code(v, &def.use_)?;
                            if def.boxed {
                                code = format!("Box::new({code})");
                            }
                            if matches!(presence, FieldPresence::Optional) {
                                code = format!("Some({code})");
                            }
                            code
                        }
                    };
                    parts.push(format!("{}: {v}", def.name));
                }
                if let Body::Record {
                    set,
                    fields,
                    extensible,
                } = &t.body
                    && keeps_extensions(*set, fields, *extensible)
                {
                    parts.push("_extensions: Default::default()".to_owned());
                }
                format!("{} {{ {} }}", self.type_path(*ty), parts.join(", "))
            }
            Literal::Variant {
                dispatch,
                variant,
                value,
            } => {
                let d = self.plan.dispatch(*dispatch)?;
                let v = d.variants.get(*variant)?.clone();
                let mut code = self.literal_code(value, &v.use_)?;
                if v.boxed {
                    code = format!("Box::new({code})");
                }
                format!("{}::{}({code})", self.dispatch_path(*dispatch), v.name)
            }
        })
    }
}

/// Whether a record carries the unknown extension additions it meets: an extensible `SEQUENCE` whose additions end
/// it (rasn decodes a `SET` by tag, and a root component after the additions follows them).
fn keeps_extensions(set: bool, fields: &[Field], extensible: bool) -> bool {
    extensible
        && !set
        && !fields
            .iter()
            .any(|f| f.region == glint_ir::Region::TrailingRoot)
}

/// Every element of a list needs code of its own when it carries constraints or is itself such a list.
fn needs_element_code(element: &Use) -> bool {
    !element.rules.is_empty()
        || matches!(&element.target, Target::List { element, .. } if needs_element_code(element))
}

fn default_fn(t: &PType, field: &Field) -> String {
    format!(
        "{}_{}_default",
        naming::snake(&t.name),
        field.name.trim_end_matches('_')
    )
}

fn key_test(key: &crate::plan::KeyValue) -> String {
    match key {
        crate::plan::KeyValue::Oid(arcs) => format!("key.is_oid(&[{}])", join(arcs)),
        crate::plan::KeyValue::Integer(n) => format!("key.is_int({n})"),
    }
}

fn resolution_code(r: &Resolution) -> String {
    let path = |p: &[String]| {
        p.iter()
            .map(|s| format!("{s:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "glint_rt::Resolution {{ key: &[{}], open: &[{}], text: {:?} }}",
        path(&r.key),
        path(&r.open),
        r.text
    )
}

/// The inspection methods of an alias, answered by the value it wraps.
const DELEGATE: &str = "    fn resolve(&mut self, path: &str) -> Result<(), glint_rt::Error> {
        glint_rt::Glint::resolve(&mut self.0, path)
    }
    fn int(&self) -> Option<i128> {
        glint_rt::Glint::int(&self.0)
    }
    fn size(&self) -> Option<usize> {
        glint_rt::Glint::size(&self.0)
    }
    fn chars(&self) -> Option<Vec<u32>> {
        glint_rt::Glint::chars(&self.0)
    }
    fn oid(&self) -> Option<&[u32]> {
        glint_rt::Glint::oid(&self.0)
    }
    fn equals(&self, c: &glint_rt::Const) -> Option<bool> {
        glint_rt::Glint::equals(&self.0, c)
    }
    fn component(&self, name: &str) -> Option<Option<&dyn glint_rt::Glint>> {
        glint_rt::Glint::component(&self.0, name)
    }
    fn component_mut(&mut self, name: &str) -> Option<Option<&mut dyn glint_rt::Glint>> {
        glint_rt::Glint::component_mut(&mut self.0, name)
    }
    fn each(&self, f: &mut dyn FnMut(&dyn glint_rt::Glint, usize)) -> bool {
        glint_rt::Glint::each(&self.0, f)
    }
    fn each_mut(&mut self, f: &mut dyn FnMut(&mut dyn glint_rt::Glint, usize)) -> bool {
        glint_rt::Glint::each_mut(&mut self.0, f)
    }
    fn dispatch(&mut self, key: Option<&glint_rt::Key>, path: &str) -> Result<(), glint_rt::Error> {
        glint_rt::Glint::dispatch(&mut self.0, key, path)
    }
";
