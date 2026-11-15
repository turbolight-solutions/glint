//! A readable text form of an [`IrSet`], for snapshots and debugging.

use std::fmt::Write as _;

use crate::{
    Bound, Component, Constraint, ElementSet, IrSet, Kind, ObjectSetTable, Presence, Range, Region,
    Summary, TagClass, TagMode, Type, Value,
};

/// Renders `set` as indented text: every type definition, then every object set table, then the OID constants.
pub fn dump(set: &IrSet) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "root #{}", set.root.0);
    for (index, def) in set.types.iter().enumerate() {
        let origin = &def.origin;
        let mut source = origin.module.clone();
        if let Some(document) = &origin.document {
            let _ = write!(source, ", {document}");
        }
        let _ = write!(out, "\n#{index} {} ({source})", def.name);
        if !origin.actuals.is_empty() || origin.asn1_name != def.name {
            let _ = write!(out, " = {}", origin.asn1_name);
            if !origin.actuals.is_empty() {
                let _ = write!(out, "{{{}}}", origin.actuals.join(", "));
            }
        }
        out.push('\n');
        let _ = write!(out, "  ");
        ty(set, &def.ty, 1, &mut out);
        out.push('\n');
    }
    for (index, table) in set.object_sets.iter().enumerate() {
        object_set(set, index, table, &mut out);
    }
    if !set.oids.is_empty() {
        out.push_str("\noids\n");
        for oid in &set.oids {
            let _ = writeln!(out, "  {} = {} ({})", oid.name, oid.oid, oid.module);
        }
    }
    if !set.notes.is_empty() {
        out.push_str("\nnotes\n");
        for note in &set.notes {
            let _ = writeln!(out, "  {}: {}", note.at, note.message);
        }
    }
    out
}

fn indent(depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn tag_class(class: TagClass) -> &'static str {
    match class {
        TagClass::Universal => "UNIVERSAL ",
        TagClass::Application => "APPLICATION ",
        TagClass::Context => "",
        TagClass::Private => "PRIVATE ",
    }
}

fn ty(set: &IrSet, t: &Type, depth: usize, out: &mut String) {
    for layer in &t.tags {
        let mode = match layer.mode {
            TagMode::Explicit => "EXPLICIT",
            TagMode::Implicit => "IMPLICIT",
        };
        let auto = if layer.automatic { " auto" } else { "" };
        let _ = write!(
            out,
            "[{}{}] {mode}{auto} ",
            tag_class(layer.tag.class),
            layer.tag.number
        );
    }
    if let Kind::SequenceOf(collection) | Kind::SetOf(collection) = &t.kind {
        // `SEQUENCE (SIZE (1..MAX)) OF T`: the constraints belong to the collection, not the element
        out.push_str(if matches!(t.kind, Kind::SequenceOf(_)) {
            "SEQUENCE"
        } else {
            "SET"
        });
        for c in &t.constraints {
            out.push(' ');
            constraint(set, c, out);
        }
        summary(&t.summary, out);
        out.push_str(" OF ");
        if let Some(name) = &collection.element_name {
            let _ = write!(out, "{name} ");
        }
        ty(set, &collection.element, depth, out);
        return;
    }
    kind(set, &t.kind, depth, out);
    for c in &t.constraints {
        out.push(' ');
        constraint(set, c, out);
    }
    summary(&t.summary, out);
}

fn kind(set: &IrSet, k: &Kind, depth: usize, out: &mut String) {
    match k {
        Kind::Boolean => out.push_str("BOOLEAN"),
        Kind::Integer { named } => {
            out.push_str("INTEGER");
            named_numbers(named, &[], false, out);
        }
        Kind::Enumerated {
            root,
            additions,
            extensible,
        } => {
            out.push_str("ENUMERATED");
            named_numbers(root, additions, *extensible, out);
        }
        Kind::Real => out.push_str("REAL"),
        Kind::Null => out.push_str("NULL"),
        Kind::BitString { named } => {
            out.push_str("BIT STRING");
            named_numbers(named, &[], false, out);
        }
        Kind::OctetString => out.push_str("OCTET STRING"),
        Kind::ObjectIdentifier => out.push_str("OBJECT IDENTIFIER"),
        Kind::RelativeOid => out.push_str("RELATIVE-OID"),
        Kind::String { string } => {
            let _ = write!(out, "{string:?}");
        }
        Kind::Time { time } => {
            let _ = write!(out, "{time:?}");
        }
        Kind::Sequence(record) | Kind::Set(record) | Kind::Choice(record) => {
            out.push_str(match k {
                Kind::Sequence(_) => "SEQUENCE",
                Kind::Set(_) => "SET",
                _ => "CHOICE",
            });
            out.push_str(" {");
            if record.extensible {
                out.push_str(" ...");
            }
            out.push('\n');
            for c in &record.components {
                component(set, c, depth.saturating_add(1), out);
            }
            indent(depth, out);
            out.push('}');
        }
        Kind::SequenceOf(collection) | Kind::SetOf(collection) => {
            out.push_str(if matches!(k, Kind::SequenceOf(_)) {
                "SEQUENCE OF "
            } else {
                "SET OF "
            });
            if let Some(name) = &collection.element_name {
                let _ = write!(out, "{name} ");
            }
            ty(set, &collection.element, depth, out);
        }
        Kind::OpenType(open) => {
            out.push_str("open type");
            if let (Some(class), Some(field)) = (&open.class, &open.field) {
                let _ = write!(out, " {class}.{field}");
            }
            if let Some(table) = &open.table {
                let _ = write!(out, " table(set#{} {}", table.set.0, table.field);
                for path in &table.relation {
                    let _ = write!(out, " @{path}");
                }
                out.push(')');
            }
            if let Some(by) = &open.defined_by {
                let _ = write!(out, " DEFINED BY {by}");
            }
        }
        Kind::Reference { target } => {
            let name = set.get(*target).map_or("?", |d| d.name.as_str());
            let _ = write!(out, "-> #{} {name}", target.0);
        }
        Kind::Other { name } => out.push_str(name),
        Kind::Opaque { reason } => {
            let _ = write!(out, "OPAQUE({reason})");
        }
    }
}

fn named_numbers(
    root: &[crate::NamedNumber],
    additions: &[crate::NamedNumber],
    extensible: bool,
    out: &mut String,
) {
    if root.is_empty() && additions.is_empty() && !extensible {
        return;
    }
    let mut items: Vec<String> = root
        .iter()
        .map(|n| format!("{}({})", n.name, n.value))
        .collect();
    if extensible {
        items.push("...".to_owned());
    }
    items.extend(additions.iter().map(|n| format!("{}({})", n.name, n.value)));
    let _ = write!(out, " {{ {} }}", items.join(", "));
}

fn component(set: &IrSet, c: &Component, depth: usize, out: &mut String) {
    indent(depth, out);
    out.push_str(&c.name);
    match &c.presence {
        Presence::Required => {}
        Presence::Optional => out.push_str(" OPTIONAL"),
        Presence::Default(v) => {
            out.push_str(" DEFAULT ");
            value(v, out);
        }
    }
    match c.region {
        Region::Root => {}
        Region::Addition => out.push_str(" +ext"),
        Region::TrailingRoot => out.push_str(" +trailing"),
    }
    if let Some(group) = c.group {
        let _ = write!(out, " [[{group}");
        if let Some(version) = c.version {
            let _ = write!(out, " v{version}");
        }
        out.push_str("]]");
    }
    out.push_str(": ");
    ty(set, &c.ty, depth, out);
    out.push('\n');
}

fn value(v: &Value, out: &mut String) {
    match v {
        Value::Boolean(b) => out.push_str(if *b { "TRUE" } else { "FALSE" }),
        Value::Integer(n) => {
            let _ = write!(out, "{n}");
        }
        Value::Real(text) | Value::Identifier(text) | Value::Raw(text) => out.push_str(text),
        Value::Null => out.push_str("NULL"),
        Value::Oid(dotted) => {
            let _ = write!(out, "oid {dotted}");
        }
        Value::Bits(names) => {
            let _ = write!(out, "{{{}}}", names.join(", "));
        }
        Value::Bstring(bits) => {
            let _ = write!(out, "'{bits}'B");
        }
        Value::Hstring(hex) => {
            let _ = write!(out, "'{hex}'H");
        }
        Value::String(s) => {
            let _ = write!(out, "{s:?}");
        }
        Value::Record(fields) => {
            out.push('{');
            for (index, (name, v)) in fields.iter().enumerate() {
                out.push_str(if index > 0 { ", " } else { " " });
                out.push_str(name);
                out.push(' ');
                value(v, out);
            }
            out.push_str(" }");
        }
    }
}

fn constraint(set: &IrSet, c: &Constraint, out: &mut String) {
    out.push('(');
    element_set(set, &c.root, out);
    if c.extensible {
        out.push_str(", ...");
        if let Some(additions) = &c.additions {
            out.push_str(", ");
            element_set(set, additions, out);
        }
    }
    if let Some(exception) = &c.exception {
        out.push_str(" ! ");
        value(exception, out);
    }
    out.push(')');
}

fn bound(b: &Bound, out: &mut String) -> bool {
    match b {
        Bound::Min => out.push_str("MIN"),
        Bound::Max => out.push_str("MAX"),
        Bound::Inclusive(v) => value(v, out),
        Bound::Exclusive(v) => {
            value(v, out);
            return true;
        }
    }
    false
}

fn join(set: &IrSet, items: &[ElementSet], sep: &str, out: &mut String) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(sep);
        }
        element_set(set, item, out);
    }
}

fn element_set(set: &IrSet, e: &ElementSet, out: &mut String) {
    match e {
        ElementSet::All => out.push_str("ALL"),
        ElementSet::Union(items) => {
            out.push('(');
            join(set, items, " | ", out);
            out.push(')');
        }
        ElementSet::Intersection(items) => {
            out.push('(');
            join(set, items, " ^ ", out);
            out.push(')');
        }
        ElementSet::Except(a, b) => {
            out.push('(');
            element_set(set, a, out);
            out.push_str(" EXCEPT ");
            element_set(set, b, out);
            out.push(')');
        }
        ElementSet::Value(v) => value(v, out),
        ElementSet::Range { lo, hi } => {
            let lo_open = bound(lo, out);
            out.push_str(if lo_open { "<.." } else { ".." });
            let mut hi_text = String::new();
            let hi_open = bound(hi, &mut hi_text);
            if hi_open {
                out.push('<');
            }
            out.push_str(&hi_text);
        }
        ElementSet::Size(c) => {
            out.push_str("SIZE ");
            constraint(set, c, out);
        }
        ElementSet::From(c) => {
            out.push_str("FROM ");
            constraint(set, c, out);
        }
        ElementSet::Pattern(p) => {
            let _ = write!(out, "PATTERN {p:?}");
        }
        ElementSet::ContainedSubtype(t) => {
            out.push_str("INCLUDES ");
            ty(set, t, 0, out);
        }
        ElementSet::WithComponent(c) => {
            out.push_str("WITH COMPONENT ");
            constraint(set, c, out);
        }
        ElementSet::WithComponents { partial, items } => {
            out.push_str("WITH COMPONENTS {");
            if *partial {
                out.push_str(" ...,");
            }
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let _ = write!(out, " {}", item.name);
                if let Some(c) = &item.constraint {
                    out.push(' ');
                    constraint(set, c, out);
                }
                if let Some(p) = item.presence {
                    let _ = write!(out, " {p:?}");
                }
            }
            out.push_str(" }");
        }
        ElementSet::Contents { ty: t, encoded_by } => {
            if let Some(t) = t {
                out.push_str("CONTAINING ");
                ty(set, t, 0, out);
            }
            if let Some(v) = encoded_by {
                if t.is_some() {
                    out.push(' ');
                }
                out.push_str("ENCODED BY ");
                value(v, out);
            }
        }
        ElementSet::Table(table) => {
            let _ = write!(out, "table(set#{} {}", table.set.0, table.field);
            for path in &table.relation {
                let _ = write!(out, " @{path}");
            }
            out.push(')');
        }
        ElementSet::Settings(s) => {
            let _ = write!(out, "SETTINGS {s:?}");
        }
        ElementSet::UserDefined(text) => {
            let _ = write!(out, "user-defined {text:?}");
        }
        ElementSet::Opaque(text) => {
            let _ = write!(out, "OPAQUE {text:?}");
        }
    }
}

fn range(name: &str, r: &Range, out: &mut String) {
    let lo = r.lo.map_or_else(|| "MIN".to_owned(), |v| v.to_string());
    let hi = r.hi.map_or_else(|| "MAX".to_owned(), |v| v.to_string());
    let ext = if r.extensible { ",..." } else { "" };
    let _ = write!(out, " {name}={lo}..{hi}{ext}");
}

fn summary_fields(s: &Summary, out: &mut String) {
    if let Some(r) = &s.value {
        range("value", r, out);
    }
    if let Some(r) = &s.size {
        range("size", r, out);
    }
    if let Some(a) = &s.alphabet {
        let ranges: Vec<String> = a
            .ranges
            .iter()
            .map(|(lo, hi)| {
                if lo == hi {
                    format!("{lo:X}")
                } else {
                    format!("{lo:X}-{hi:X}")
                }
            })
            .collect();
        let ext = if a.extensible { ",..." } else { "" };
        let _ = write!(out, " alphabet={}{ext}", ranges.join(","));
    }
}

fn summary(s: &Summary, out: &mut String) {
    if s.is_empty() {
        return;
    }
    out.push_str("  {");
    summary_fields(s, out);
    if let Some(per) = &s.per_visible {
        out.push_str(" per:");
        if per.is_empty() {
            out.push_str(" none");
        }
        summary_fields(per, out);
    }
    out.push_str(" }");
}

fn object_set(set: &IrSet, index: usize, table: &ObjectSetTable, out: &mut String) {
    let _ = write!(
        out,
        "\nset#{index} {} ({}) {}",
        table.name, table.module, table.class
    );
    if let Some(key) = &table.key_field {
        let _ = write!(out, " key {key}");
    }
    if table.extensible {
        out.push_str(" ...");
    }
    out.push('\n');
    for row in &table.rows {
        out.push_str("  ");
        out.push_str(row.object.as_deref().unwrap_or("{inline}"));
        if let Some(key) = &row.key {
            out.push(' ');
            value(key, out);
            if let Some(name) = &row.key_name {
                let _ = write!(out, " ({name})");
            }
        }
        out.push('\n');
        for (field, t) in &row.types {
            let _ = write!(out, "    {field}: ");
            ty(set, t, 2, out);
            out.push('\n');
        }
        for (field, v) in &row.values {
            let _ = write!(out, "    {field} = ");
            value(v, out);
            out.push('\n');
        }
    }
}
