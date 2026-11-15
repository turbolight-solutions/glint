//! The text form of a decode: `openssl asn1parse`'s columns, with field names, types, values and findings.

use std::fmt::Write as _;

use glint_ir::{TagClass, TagMode};

use crate::{Decoded, Length, Rendered, SchemaState, Tlv};

/// Octets of hexadecimal shown before the rest is elided.
const HEX_SHOWN: usize = 32;

/// The name of a universal tag.
pub(crate) fn universal_name(number: u64) -> Option<&'static str> {
    Some(match number {
        0 => "EOC",
        1 => "BOOLEAN",
        2 => "INTEGER",
        3 => "BIT STRING",
        4 => "OCTET STRING",
        5 => "NULL",
        6 => "OBJECT IDENTIFIER",
        7 => "ObjectDescriptor",
        8 => "EXTERNAL",
        9 => "REAL",
        10 => "ENUMERATED",
        11 => "EMBEDDED PDV",
        12 => "UTF8String",
        13 => "RELATIVE-OID",
        14 => "TIME",
        16 => "SEQUENCE",
        17 => "SET",
        18 => "NumericString",
        19 => "PrintableString",
        20 => "TeletexString",
        21 => "VideotexString",
        22 => "IA5String",
        23 => "UTCTime",
        24 => "GeneralizedTime",
        25 => "GraphicString",
        26 => "VisibleString",
        27 => "GeneralString",
        28 => "UniversalString",
        29 => "CHARACTER STRING",
        30 => "BMPString",
        31 => "DATE",
        32 => "TIME-OF-DAY",
        33 => "DATE-TIME",
        34 => "DURATION",
        35 => "OID-IRI",
        36 => "RELATIVE-OID-IRI",
        _ => return None,
    })
}

fn tag_column(tlv: &Tlv) -> String {
    let Some(tag) = tlv.tag else {
        return "(not a TLV)".to_owned();
    };
    let mut text = match tag.class {
        TagClass::Universal => universal_name(tag.number)
            .map_or_else(|| format!("[UNIVERSAL {}]", tag.number), str::to_owned),
        TagClass::Application => format!("[APPLICATION {}]", tag.number),
        TagClass::Context => format!("[{}]", tag.number),
        TagClass::Private => format!("[PRIVATE {}]", tag.number),
    };
    match tlv.tagging {
        Some(TagMode::Explicit) => text.push_str(" EXPLICIT"),
        Some(TagMode::Implicit) => text.push_str(" IMPLICIT"),
        None => {}
    }
    text
}

fn elided_hex(hex: &str) -> String {
    let shown = HEX_SHOWN.saturating_mul(2);
    match hex.get(..shown) {
        Some(head) if hex.len() > shown => format!("{head}… ({} octets)", hex.len() / 2),
        _ => hex.to_owned(),
    }
}

fn value_text(tlv: &Tlv) -> Option<String> {
    let value = tlv.value.as_ref()?;
    // the children of a primitive octet or bit string are its contained value; of a constructed one, its segments
    let contains = !tlv.children.is_empty() && tlv.tag.is_some_and(|t| !t.constructed);
    Some(match value {
        Rendered::Boolean { value } => if *value { "TRUE" } else { "FALSE" }.to_owned(),
        Rendered::Integer { decimal, hex, name } => {
            let mut text = match hex {
                Some(hex) => format!("0x{}", elided_hex(hex)),
                None => decimal.clone(),
            };
            if let Some(name) = name {
                let _ = write!(text, " ({name})");
            }
            text
        }
        Rendered::Enumerated { decimal, name } => match name {
            Some(name) => format!("{decimal} ({name})"),
            None => decimal.clone(),
        },
        Rendered::Null => return None,
        Rendered::BitString { unused, hex, named } => {
            if named.is_empty() {
                if contains {
                    "CONTAINING".to_owned()
                } else {
                    format!("{} unused bits: {}", unused, elided_hex(hex))
                }
            } else {
                named.join(", ")
            }
        }
        Rendered::OctetString { hex } => {
            if contains {
                "CONTAINING".to_owned()
            } else {
                elided_hex(hex)
            }
        }
        Rendered::Oid { dotted, names } => match names.first() {
            Some(name) => format!("{dotted} {}", name.name),
            None => dotted.clone(),
        },
        Rendered::RelativeOid { dotted } => dotted.clone(),
        Rendered::Time { text, normalized } => match normalized {
            Some(normalized) => format!("{text} ({normalized})"),
            None => text.clone(),
        },
        Rendered::String { text } => text.escape_debug().to_string(),
        Rendered::Real { text } => text.clone(),
        Rendered::Hex { hex } => elided_hex(hex),
    })
}

fn short_type(name: &str) -> &str {
    // `Module.Type{...}`: the module name has no `.` but the actual parameters may
    let base_end = name.find('{').unwrap_or(name.len());
    match name.get(..base_end).and_then(|base| base.find('.')) {
        Some(dot) => name.get(dot.saturating_add(1)..).unwrap_or(name),
        None => name,
    }
}

fn line(tlv: &Tlv, depth: usize, out: &mut String) {
    let len = match tlv.len {
        Length::Definite(n) => format!("{n:>4}"),
        Length::Indefinite => " inf".to_owned(),
    };
    let form = match tlv.tag {
        Some(tag) if tag.constructed => "cons",
        Some(_) => "prim",
        None => "    ",
    };
    let indent = " ".repeat(depth);
    let tag = format!("{indent}{}", tag_column(tlv));
    let mut annotation = Vec::new();
    if let SchemaState::Hinted { .. } = tlv.schema {
        annotation.push("~".to_owned());
    }
    match (&tlv.field, &tlv.type_name) {
        (Some(field), Some(ty)) => annotation.push(format!("{field}: {}", short_type(ty))),
        (Some(field), None) => annotation.push(format!("{field}:")),
        (None, Some(ty)) => annotation.push(short_type(ty).to_owned()),
        (None, None) => {}
    }
    if let Some(value) = value_text(tlv) {
        annotation.push(value);
    }
    let _ = writeln!(
        out,
        "{:>5}:d={:<2} hl={} l={len} {form}: {tag:<30} {}",
        tlv.offset,
        depth,
        tlv.header_len,
        annotation.join(" ")
    );
    for finding in &tlv.findings {
        let _ = writeln!(
            out,
            "{:>7}{indent}! {} {}: {}",
            "",
            finding.code,
            match finding.severity {
                glint_diag::Severity::Error => "error",
                glint_diag::Severity::Warning => "warning",
                glint_diag::Severity::Info => "info",
            },
            finding.message
        );
    }
    for child in &tlv.children {
        line(child, depth.saturating_add(1), out);
    }
}

/// The decode as text, one line per TLV.
pub fn render_text(decoded: &Decoded) -> String {
    let mut out = String::new();
    for tlv in &decoded.tlvs {
        line(tlv, 0, &mut out);
    }
    for finding in &decoded.findings {
        let _ = writeln!(out, "! {} {}", finding.code, finding.message);
    }
    out
}
