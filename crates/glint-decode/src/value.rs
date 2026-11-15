//! Contents octets of primitive values: validity (X.690 8), what DER adds (X.690 10, 11) and how a value is shown.

use glint_ir::{NamedNumber, StringKind, TimeKind};
use serde::Serialize;

use crate::finding::{self, DecodeFinding};

/// A name the corpus or the schema gives an object identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OidName {
    pub name: String,
    pub module: String,
    /// The corpus document (`rfc5912`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
}

/// A value, shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Rendered {
    Boolean {
        value: bool,
    },
    Integer {
        decimal: String,
        /// The contents octets in hexadecimal, for values of more than eight octets.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hex: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Enumerated {
        decimal: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Null,
    BitString {
        unused: u8,
        /// The bits in hexadecimal, unused bits included.
        hex: String,
        /// The names of the set bits that have one, and the numbers of those that do not.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        named: Vec<String>,
    },
    OctetString {
        hex: String,
    },
    Oid {
        dotted: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        names: Vec<OidName>,
    },
    RelativeOid {
        dotted: String,
    },
    Time {
        text: String,
        /// ISO 8601, in UTC when the value gives an offset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        normalized: Option<String>,
    },
    String {
        text: String,
    },
    Real {
        text: String,
    },
    /// Contents with no interpretation.
    Hex {
        hex: String,
    },
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &b in bytes {
        out.push(char::from(
            *DIGITS.get(usize::from(b >> 4)).unwrap_or(&b'?'),
        ));
        out.push(char::from(
            *DIGITS.get(usize::from(b & 0xf)).unwrap_or(&b'?'),
        ));
    }
    out
}

pub(crate) fn boolean(content: &[u8]) -> (Option<Rendered>, Vec<DecodeFinding>) {
    match content {
        [b] => {
            let mut findings = Vec::new();
            if *b != 0 && *b != 0xff {
                findings.push(finding::der(format!(
                    "BOOLEAN TRUE written as {b:02X}, not FF (X.690 11.1)"
                )));
            }
            (Some(Rendered::Boolean { value: *b != 0 }), findings)
        }
        _ => (
            None,
            vec![finding::invalid(format!(
                "a BOOLEAN has one contents octet, not {} (X.690 8.2.1)",
                content.len()
            ))],
        ),
    }
}

pub(crate) fn null(content: &[u8]) -> (Option<Rendered>, Vec<DecodeFinding>) {
    if content.is_empty() {
        (Some(Rendered::Null), Vec::new())
    } else {
        (
            None,
            vec![finding::invalid(format!(
                "NULL has no contents octets, not {} (X.690 8.8.2)",
                content.len()
            ))],
        )
    }
}

/// The value of a two's complement integer that fits.
pub(crate) fn small_integer(content: &[u8]) -> Option<i128> {
    if content.is_empty() || content.len() > 16 {
        return None;
    }
    let negative = content.first().is_some_and(|b| b & 0x80 != 0);
    let mut value: i128 = if negative { -1 } else { 0 };
    for &b in content {
        value = value.wrapping_shl(8) | i128::from(b);
    }
    Some(value)
}

/// The decimal digits of an unsigned big-endian magnitude.
fn decimal_of_magnitude(bytes: &[u8]) -> String {
    // little-endian limbs of base 10^9
    let mut limbs: Vec<u32> = Vec::new();
    for &b in bytes {
        let mut carry = u64::from(b);
        for limb in &mut limbs {
            let v = u64::from(*limb).wrapping_mul(256).wrapping_add(carry);
            *limb = u32::try_from(v % 1_000_000_000).unwrap_or(0);
            carry = v / 1_000_000_000;
        }
        while carry > 0 {
            limbs.push(u32::try_from(carry % 1_000_000_000).unwrap_or(0));
            carry /= 1_000_000_000;
        }
    }
    let mut text = String::new();
    for (i, limb) in limbs.iter().rev().enumerate() {
        if i == 0 {
            text.push_str(&limb.to_string());
        } else {
            text.push_str(&format!("{limb:09}"));
        }
    }
    if text.is_empty() {
        "0".to_owned()
    } else {
        text
    }
}

/// The decimal form of a two's complement integer of any length.
fn integer_decimal(content: &[u8]) -> String {
    if let Some(v) = small_integer(content) {
        return v.to_string();
    }
    let negative = content.first().is_some_and(|b| b & 0x80 != 0);
    if !negative {
        return decimal_of_magnitude(content);
    }
    // magnitude = two's complement negation
    let mut magnitude: Vec<u8> = content.iter().map(|b| !b).collect();
    for b in magnitude.iter_mut().rev() {
        let (v, overflow) = b.overflowing_add(1);
        *b = v;
        if !overflow {
            break;
        }
    }
    format!("-{}", decimal_of_magnitude(&magnitude))
}

fn integer_checks(content: &[u8], what: &str) -> Vec<DecodeFinding> {
    let mut findings = Vec::new();
    if let [first, second, ..] = content
        && ((*first == 0 && second & 0x80 == 0) || (*first == 0xff && second & 0x80 != 0))
    {
        findings.push(finding::invalid(format!(
            "{what} not written in the fewest octets (X.690 8.3.2)"
        )));
    }
    findings
}

fn name_of(named: &[NamedNumber], value: Option<i128>) -> Option<String> {
    let value = value?;
    named
        .iter()
        .find(|n| n.value == value)
        .map(|n| n.name.clone())
}

pub(crate) fn integer(
    content: &[u8],
    named: &[NamedNumber],
) -> (Option<Rendered>, Vec<DecodeFinding>) {
    if content.is_empty() {
        return (
            None,
            vec![finding::invalid(
                "an INTEGER has at least one contents octet (X.690 8.3.1)",
            )],
        );
    }
    let findings = integer_checks(content, "INTEGER");
    let value = small_integer(content);
    (
        Some(Rendered::Integer {
            decimal: integer_decimal(content),
            hex: (content.len() > 8).then(|| hex(content)),
            name: name_of(named, value),
        }),
        findings,
    )
}

pub(crate) fn enumerated(
    content: &[u8],
    items: &[NamedNumber],
    extensible: bool,
) -> (Option<Rendered>, Vec<DecodeFinding>) {
    if content.is_empty() {
        return (
            None,
            vec![finding::invalid(
                "an ENUMERATED has at least one contents octet (X.690 8.4)",
            )],
        );
    }
    let mut findings = integer_checks(content, "ENUMERATED");
    let value = small_integer(content);
    let name = name_of(items, value);
    if name.is_none() && !extensible {
        findings.push(finding::invalid(format!(
            "{} is not an item of the enumeration",
            integer_decimal(content)
        )));
    }
    (
        Some(Rendered::Enumerated {
            decimal: integer_decimal(content),
            name,
        }),
        findings,
    )
}

/// The bits of a BIT STRING: `content` is the primitive contents octets, initial octet included.
pub(crate) fn bit_string(
    content: &[u8],
    named: &[NamedNumber],
) -> (Option<Rendered>, Vec<DecodeFinding>) {
    let Some((&unused, bits)) = content.split_first() else {
        return (
            None,
            vec![finding::invalid(
                "a BIT STRING has at least one contents octet (X.690 8.6.2)",
            )],
        );
    };
    let mut findings = Vec::new();
    if unused > 7 {
        return (
            None,
            vec![finding::invalid(format!(
                "{unused} unused bits; at most 7 (X.690 8.6.2.2)"
            ))],
        );
    }
    if bits.is_empty() && unused != 0 {
        findings.push(finding::invalid(
            "an empty BIT STRING has 0 unused bits (X.690 8.6.2.3)",
        ));
    }
    if let Some(last) = bits.last() {
        let mask = (1u16 << unused).saturating_sub(1);
        if u16::from(*last) & mask != 0 {
            findings.push(finding::der("unused bits are not zero (X.690 11.2.1)"));
        }
    }
    let total = bits
        .len()
        .saturating_mul(8)
        .saturating_sub(usize::from(unused));
    let is_set = |i: usize| {
        bits.get(i / 8)
            .is_some_and(|b| b & (0x80u8 >> (i % 8)) != 0)
    };
    let mut set_names = Vec::new();
    if !named.is_empty() {
        for i in 0..total {
            if is_set(i) {
                let name = named
                    .iter()
                    .find(|n| usize::try_from(n.value).ok() == Some(i))
                    .map_or_else(|| i.to_string(), |n| n.name.clone());
                set_names.push(name);
            }
        }
        if total > 0 && !is_set(total.saturating_sub(1)) {
            findings.push(finding::der(
                "a BIT STRING with named bits ends in a 0 bit (X.690 11.2.2)",
            ));
        }
    }
    (
        Some(Rendered::BitString {
            unused,
            hex: hex(bits),
            named: set_names,
        }),
        findings,
    )
}

/// The arcs of an object identifier or relative object identifier (X.690 8.19, 8.20).
pub(crate) fn arcs(content: &[u8], relative: bool) -> Result<Vec<u128>, String> {
    if content.is_empty() {
        return Err(
            "an object identifier has at least one contents octet (X.690 8.19.1)".to_owned(),
        );
    }
    let mut out = Vec::new();
    let mut value: u128 = 0;
    let mut start = true;
    for &b in content {
        if start && b == 0x80 {
            return Err("a subidentifier starts with 0x80 (X.690 8.19.2)".to_owned());
        }
        start = false;
        value = value
            .checked_mul(128)
            .and_then(|v| v.checked_add(u128::from(b & 0x7f)))
            .ok_or("a subidentifier is too large")?;
        if b & 0x80 == 0 {
            if out.is_empty() && !relative {
                let (first, second) = match value {
                    0..40 => (0, value),
                    40..80 => (1, value.saturating_sub(40)),
                    _ => (2, value.saturating_sub(80)),
                };
                out.push(first);
                out.push(second);
            } else {
                out.push(value);
            }
            value = 0;
            start = true;
        }
    }
    if !start {
        return Err("the last subidentifier is cut off (X.690 8.19.2)".to_owned());
    }
    Ok(out)
}

pub(crate) fn dotted(arcs: &[u128]) -> String {
    arcs.iter()
        .map(u128::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn real(content: &[u8]) -> (Option<Rendered>, Vec<DecodeFinding>) {
    let Some((&first, rest)) = content.split_first() else {
        return (
            Some(Rendered::Real {
                text: "0".to_owned(),
            }),
            Vec::new(),
        );
    };
    let text = match first {
        0x40 => "PLUS-INFINITY".to_owned(),
        0x41 => "MINUS-INFINITY".to_owned(),
        0x42 => "NOT-A-NUMBER".to_owned(),
        0x43 => "-0".to_owned(),
        b if b & 0xc0 == 0 => String::from_utf8_lossy(rest).trim().to_owned(),
        _ => match binary_real(first, rest) {
            Some(v) => v.to_string(),
            None => format!("binary {}", hex(content)),
        },
    };
    (Some(Rendered::Real { text }), Vec::new())
}

/// A binary REAL (X.690 8.5.7), approximately, when it fits an `f64`.
fn binary_real(first: u8, rest: &[u8]) -> Option<f64> {
    let negative = first & 0x40 != 0;
    let base: f64 = match (first >> 4) & 0x3 {
        0 => 2.0,
        1 => 8.0,
        2 => 16.0,
        _ => return None,
    };
    let scale = i32::from((first >> 2) & 0x3);
    let (exp_len, rest) = match first & 0x3 {
        3 => {
            let (&n, rest) = rest.split_first()?;
            (usize::from(n), rest)
        }
        n => (usize::from(n).saturating_add(1), rest),
    };
    let exp_bytes = rest.get(..exp_len)?;
    let mantissa_bytes = rest.get(exp_len..)?;
    let exponent = i32::try_from(small_integer(exp_bytes)?).ok()?;
    let mut mantissa = 0f64;
    for &b in mantissa_bytes {
        mantissa = mantissa * 256.0 + f64::from(b);
    }
    let value = mantissa * 2f64.powi(scale) * base.powi(exponent);
    Some(if negative { -value } else { value })
}

fn printable(c: char) -> bool {
    c.is_ascii_alphanumeric() || " '()+,-./:=?".contains(c)
}

/// The text of a character string, and what is wrong with it for its type.
pub(crate) fn string(content: &[u8], kind: StringKind) -> (Option<Rendered>, Vec<DecodeFinding>) {
    let mut findings = Vec::new();
    let latin1 = || content.iter().map(|&b| char::from(b)).collect::<String>();
    let name = format!("{kind:?}");
    let text = match kind {
        StringKind::UTF8String => match std::str::from_utf8(content) {
            Ok(text) => text.to_owned(),
            Err(_) => {
                findings.push(finding::invalid("UTF8String contents are not UTF-8"));
                String::from_utf8_lossy(content).into_owned()
            }
        },
        StringKind::BMPString => {
            if !content.len().is_multiple_of(2) {
                findings.push(finding::invalid(
                    "BMPString contents are not a whole number of 2-octet characters",
                ));
            }
            let units: Vec<u16> = content
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            char::decode_utf16(units.iter().copied())
                .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }
        StringKind::UniversalString => {
            if !content.len().is_multiple_of(4) {
                findings.push(finding::invalid(
                    "UniversalString contents are not a whole number of 4-octet characters",
                ));
            }
            content
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| {
                    char::from_u32(u32::from_be_bytes(*c)).unwrap_or(char::REPLACEMENT_CHARACTER)
                })
                .collect()
        }
        _ => latin1(),
    };
    let bad = match kind {
        StringKind::PrintableString => text.chars().find(|&c| !printable(c)),
        StringKind::NumericString => text.chars().find(|&c| !(c.is_ascii_digit() || c == ' ')),
        StringKind::IA5String => text.chars().find(|&c| !c.is_ascii()),
        StringKind::VisibleString => text.chars().find(|&c| !(' '..='~').contains(&c)),
        _ => None,
    };
    if let Some(c) = bad {
        findings.push(finding::invalid(format!(
            "{name} cannot hold the character {:?} (U+{:04X})",
            c,
            u32::from(c)
        )));
    }
    (Some(Rendered::String { text }), findings)
}

/// The number of characters of a string value, for `SIZE`.
pub(crate) fn char_count(rendered: &Rendered) -> Option<usize> {
    match rendered {
        Rendered::String { text } => Some(text.chars().count()),
        _ => None,
    }
}

fn digits(text: &str, range: std::ops::Range<usize>) -> Option<u32> {
    let part = text.get(range)?;
    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// A UTCTime or GeneralizedTime, checked and normalized (X.680 46, 47; X.690 11.7, 11.8).
pub(crate) fn time(content: &[u8], kind: TimeKind) -> (Option<Rendered>, Vec<DecodeFinding>) {
    let text = content.iter().map(|&b| char::from(b)).collect::<String>();
    let mut findings = Vec::new();
    let normalized = match kind {
        TimeKind::UTCTime => utc_time(&text, &mut findings),
        TimeKind::GeneralizedTime => generalized_time(&text, &mut findings),
        _ => None,
    };
    if normalized.is_none() && matches!(kind, TimeKind::UTCTime | TimeKind::GeneralizedTime) {
        findings.push(finding::invalid(format!(
            "`{text}` is not a valid {kind:?}"
        )));
    }
    (Some(Rendered::Time { text, normalized }), findings)
}

struct Clock {
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: Option<u32>,
    fraction: Option<String>,
    zone: Zone,
}

enum Zone {
    Utc,
    Local,
    Offset(char, u32, u32),
}

impl Clock {
    fn valid(&self) -> bool {
        (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.hour <= 23
            && self.minute <= 59
            && self.second.is_none_or(|s| s <= 60)
    }

    fn iso(&self) -> String {
        let mut out = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        );
        if let Some(s) = self.second {
            out.push_str(&format!(":{s:02}"));
        }
        if let Some(f) = &self.fraction {
            out.push('.');
            out.push_str(f);
        }
        match self.zone {
            Zone::Utc => out.push('Z'),
            Zone::Local => {}
            Zone::Offset(sign, h, m) => out.push_str(&format!("{sign}{h:02}:{m:02}")),
        }
        out
    }
}

/// Reads the zone at `rest`: `Z`, `+hhmm`, `-hhmm`, or nothing.
fn zone(rest: &str) -> Option<Zone> {
    match rest.as_bytes().first() {
        None => Some(Zone::Local),
        Some(b'Z') if rest.len() == 1 => Some(Zone::Utc),
        Some(&sign @ (b'+' | b'-')) if rest.len() == 5 => {
            let h = digits(rest, 1..3)?;
            let m = digits(rest, 3..5)?;
            (h <= 23 && m <= 59).then_some(Zone::Offset(char::from(sign), h, m))
        }
        _ => None,
    }
}

fn utc_time(text: &str, findings: &mut Vec<DecodeFinding>) -> Option<String> {
    let yy = digits(text, 0..2)?;
    let mut clock = Clock {
        // RFC 5280 4.1.2.5.1
        year: if yy >= 50 {
            1900u32.saturating_add(yy)
        } else {
            2000u32.saturating_add(yy)
        },
        month: digits(text, 2..4)?,
        day: digits(text, 4..6)?,
        hour: digits(text, 6..8)?,
        minute: digits(text, 8..10)?,
        second: None,
        fraction: None,
        zone: Zone::Local,
    };
    let mut rest = text.get(10..)?;
    if let Some(s) = digits(rest, 0..2) {
        clock.second = Some(s);
        rest = rest.get(2..)?;
    }
    clock.zone = zone(rest)?;
    if matches!(clock.zone, Zone::Local) || !clock.valid() {
        return None;
    }
    if clock.second.is_none() || !matches!(clock.zone, Zone::Utc) {
        findings.push(finding::der(
            "a UTCTime is written YYMMDDHHMMSSZ (X.690 11.8)",
        ));
    }
    Some(clock.iso())
}

fn generalized_time(text: &str, findings: &mut Vec<DecodeFinding>) -> Option<String> {
    let mut clock = Clock {
        year: digits(text, 0..4)?,
        month: digits(text, 4..6)?,
        day: digits(text, 6..8)?,
        hour: digits(text, 8..10)?,
        minute: 0,
        second: None,
        fraction: None,
        zone: Zone::Local,
    };
    let mut rest = text.get(10..)?;
    let mut has_minute = false;
    if let Some(m) = digits(rest, 0..2) {
        clock.minute = m;
        has_minute = true;
        rest = rest.get(2..)?;
        if let Some(s) = digits(rest, 0..2) {
            clock.second = Some(s);
            rest = rest.get(2..)?;
        }
    }
    let mut comma = false;
    if let Some(sep @ (b'.' | b',')) = rest.as_bytes().first().copied() {
        comma = sep == b',';
        let tail = rest.get(1..)?;
        let len = tail.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 {
            return None;
        }
        clock.fraction = Some(tail.get(..len)?.to_owned());
        rest = tail.get(len..)?;
    }
    clock.zone = zone(rest)?;
    if !clock.valid() {
        return None;
    }
    let trailing_zero = clock.fraction.as_deref().is_some_and(|f| f.ends_with('0'));
    if !has_minute
        || clock.second.is_none()
        || !matches!(clock.zone, Zone::Utc)
        || comma
        || trailing_zero
    {
        findings.push(finding::der(
            "a GeneralizedTime is written YYYYMMDDHHMMSS[.f]Z, with no trailing zeros in the fraction (X.690 11.7)",
        ));
    }
    Some(clock.iso())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn integers_of_every_size() {
        assert_eq!(integer_decimal(&[0x00]), "0");
        assert_eq!(integer_decimal(&[0xff]), "-1");
        assert_eq!(integer_decimal(&[0x00, 0x80]), "128");
        let big = [0x01; 20];
        assert_eq!(
            integer_decimal(&big),
            "5731378969925109483151705226338364782964441345"
        );
        let mut negative = vec![0xff; 20];
        negative[19] = 0x00;
        assert_eq!(integer_decimal(&negative), "-256");
        assert_eq!(integer(&[0x00, 0x05], &[]).1[0].code, "E401");
    }

    #[test]
    fn object_identifiers() {
        assert_eq!(
            dotted(&arcs(&[0x55, 0x1d, 0x0f], false).unwrap()),
            "2.5.29.15"
        );
        assert_eq!(
            dotted(&arcs(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d], false).unwrap()),
            "1.2.840.113549"
        );
        assert!(arcs(&[0x2a, 0x80, 0x01], false).is_err());
        assert!(arcs(&[0x2a, 0x86], false).is_err());
    }

    #[test]
    fn bit_strings() {
        let named = [
            NamedNumber {
                name: "digitalSignature".into(),
                value: 0,
            },
            NamedNumber {
                name: "keyCertSign".into(),
                value: 5,
            },
        ];
        let (value, findings) = bit_string(&[0x02, 0x84], &named);
        assert!(findings.is_empty(), "{findings:?}");
        let Some(Rendered::BitString { named, .. }) = value else {
            panic!()
        };
        assert_eq!(named, ["digitalSignature", "keyCertSign"]);
        assert_eq!(bit_string(&[0x01, 0x85], &[]).1[0].code, "W402");
    }

    #[test]
    fn times() {
        let (value, findings) = time(b"240102030405Z", TimeKind::UTCTime);
        assert!(findings.is_empty());
        let Some(Rendered::Time { normalized, .. }) = value else {
            panic!()
        };
        assert_eq!(normalized.as_deref(), Some("2024-01-02T03:04:05Z"));
        assert_eq!(time(b"2401020304Z", TimeKind::UTCTime).1[0].code, "W402");
        assert_eq!(
            time(b"20240102030405.50Z", TimeKind::GeneralizedTime).1[0].code,
            "W402"
        );
        assert_eq!(
            time(b"20241302030405Z", TimeKind::GeneralizedTime).1[0].code,
            "E401"
        );
    }

    #[test]
    fn strings() {
        assert!(string(b"Hello", StringKind::PrintableString).1.is_empty());
        assert_eq!(
            string(b"a@b", StringKind::PrintableString).1[0].code,
            "E401"
        );
        let (value, _) = string(&[0x00, 0x41, 0x00, 0x42], StringKind::BMPString);
        assert_eq!(value, Some(Rendered::String { text: "AB".into() }));
    }
}
