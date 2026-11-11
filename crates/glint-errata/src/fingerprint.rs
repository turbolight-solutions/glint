//! The fingerprint of an erratum's reviewed content, stored in its ledger row so a changed erratum is triaged again.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::Erratum;

fn normalise(text: &str) -> String {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = unified.split('\n').map(str::trim_end).collect();
    lines.join("\n").trim_end().to_owned()
}

/// A JSON string as Python's `json.dumps` writes it (`ensure_ascii`): `"`, `\` and the short control escapes, every
/// other character outside space..tilde as `\uXXXX` in lower-case hex, UTF-16 surrogate pairs above U+FFFF. Ledgers
/// were first written by a Python tool; this keeps their fingerprints.
fn python_json_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// The first 16 hex digits of SHA-256 over the reviewed content of `e`: status, section, original and corrected text
/// and notes, with CRLF and CR turned into LF and trailing whitespace stripped (a missing field counts as "").
pub fn fingerprint(e: &Erratum) -> String {
    let mut json = String::from("[");
    for (i, part) in [
        &e.status,
        &e.section,
        &e.orig_text,
        &e.correct_text,
        &e.notes,
    ]
    .iter()
    .enumerate()
    {
        if i > 0 {
            json.push_str(", ");
        }
        python_json_string(&normalise(part), &mut json);
    }
    json.push(']');
    let digest = Sha256::digest(json.as_bytes());
    let mut hex = String::new();
    for byte in digest.iter().take(8) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    fn erratum(orig: &str) -> Erratum {
        Erratum {
            errata_id: 1,
            rfc: 1,
            status: "Verified".into(),
            section: "A".into(),
            orig_text: orig.into(),
            correct_text: String::new(),
            notes: String::new(),
            names: vec![],
        }
    }

    #[test]
    fn json_matches_python() {
        let mut out = String::new();
        python_json_string("a\"\\\n\r\t\u{8}\u{c}\u{1}\u{7f}é€😀", &mut out);
        assert_eq!(
            out,
            r#""a\"\\\n\r\t\b\f\u0001\u007f\u00e9\u20ac\ud83d\ude00""#
        );
    }

    #[test]
    fn known_value_from_the_python_tool() {
        // python3 -c 'import hashlib,json; print(hashlib.sha256(json.dumps(
        //   ["Verified","A","x ::= INTEGER","",""]).encode()).hexdigest()[:16])'
        assert_eq!(fingerprint(&erratum("x ::= INTEGER")), "8c19a10d0f444976");
    }

    #[test]
    fn line_endings_and_trailing_whitespace_do_not_count() {
        assert_eq!(
            fingerprint(&erratum("a  \r\nb\t\n\n")),
            fingerprint(&erratum("a\nb"))
        );
        assert_ne!(fingerprint(&erratum("a\nb")), fingerprint(&erratum("a b")));
    }
}
