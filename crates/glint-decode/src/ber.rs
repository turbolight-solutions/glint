//! The schema-less layer: identifier and length octets (X.690 8.1), read into a tree of raw TLVs that the schema walk
//! then interprets. Everything here is BER; what DER forbids is recorded as a finding, never rejected.

use std::ops::Range;

use glint_ir::TagClass;
use serde::Serialize;

use crate::finding::{self, DecodeFinding};

/// How deep constructed encodings may nest before the rest is kept as contents.
pub(crate) const MAX_NESTING: usize = 64;

/// An identifier octet group (X.690 8.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Tag {
    pub class: TagClass,
    pub number: u64,
    pub constructed: bool,
}

impl Tag {
    pub fn universal(number: u64) -> Self {
        Self {
            class: TagClass::Universal,
            number,
            constructed: false,
        }
    }

    /// Whether the class and number are those of `tag`, whatever the form.
    pub fn is(&self, tag: glint_ir::Tag) -> bool {
        self.class == tag.class && self.number == tag.number
    }
}

/// A length (X.690 8.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase", tag = "form", content = "octets")]
pub enum Length {
    Definite(usize),
    Indefinite,
}

/// One TLV as the bytes give it, before any schema.
#[derive(Debug, Clone)]
pub(crate) struct Raw {
    pub offset: usize,
    pub header_len: usize,
    pub len: Length,
    /// `None` for bytes that do not start a TLV; they are all contents.
    pub tag: Option<Tag>,
    /// The contents octets, without the end-of-contents octets of an indefinite length.
    pub content: Range<usize>,
    /// The end of the TLV, end-of-contents octets included.
    pub end: usize,
    /// The TLVs of a constructed encoding.
    pub children: Vec<Raw>,
    pub findings: Vec<DecodeFinding>,
}

impl Raw {
    pub fn constructed(&self) -> bool {
        self.tag.is_some_and(|t| t.constructed)
    }

    /// Bytes that could not be read as a TLV.
    fn junk(range: Range<usize>, finding: DecodeFinding) -> Self {
        Self {
            offset: range.start,
            header_len: 0,
            len: Length::Definite(range.end.saturating_sub(range.start)),
            tag: None,
            content: range.clone(),
            end: range.end,
            children: Vec::new(),
            findings: vec![finding],
        }
    }
}

struct Header {
    tag: Tag,
    header_len: usize,
    len: Length,
    findings: Vec<DecodeFinding>,
}

fn byte(bytes: &[u8], at: usize) -> Option<u8> {
    bytes.get(at).copied()
}

/// Reads the identifier and length octets at `at`, which must lie before `end`.
fn header(bytes: &[u8], at: usize, end: usize) -> Result<Header, String> {
    let mut findings = Vec::new();
    let mut pos = at;
    let first = byte(bytes, pos)
        .filter(|_| pos < end)
        .ok_or("no identifier octets")?;
    pos = pos.saturating_add(1);
    let class = match first >> 6 {
        0 => TagClass::Universal,
        1 => TagClass::Application,
        2 => TagClass::Context,
        _ => TagClass::Private,
    };
    let constructed = first & 0x20 != 0;
    let mut number = u64::from(first & 0x1f);
    if number == 0x1f {
        // high-tag-number form (8.1.2.4)
        number = 0;
        let mut count = 0usize;
        loop {
            let b = byte(bytes, pos)
                .filter(|_| pos < end)
                .ok_or("the tag number is cut off")?;
            if count == 0 && b == 0x80 {
                return Err(
                    "the tag number starts with a zero group (X.690 8.1.2.4.2 c)".to_owned(),
                );
            }
            pos = pos.saturating_add(1);
            count = count.saturating_add(1);
            number = number
                .checked_mul(128)
                .and_then(|n| n.checked_add(u64::from(b & 0x7f)))
                .ok_or("the tag number is too large")?;
            if b & 0x80 == 0 {
                break;
            }
        }
        if number < 31 {
            findings.push(finding::der(format!(
                "tag number {number} written in the high-tag-number form (X.690 8.1.2.4)"
            )));
        }
    }
    let first_len = byte(bytes, pos)
        .filter(|_| pos < end)
        .ok_or("no length octets")?;
    pos = pos.saturating_add(1);
    let len = if first_len < 0x80 {
        Length::Definite(usize::from(first_len))
    } else if first_len == 0x80 {
        if !constructed {
            return Err("indefinite length on a primitive encoding (X.690 8.1.3.2 a)".to_owned());
        }
        findings.push(finding::der("indefinite length (X.690 10.1)"));
        Length::Indefinite
    } else if first_len == 0xff {
        return Err("length octet 0xFF is reserved (X.690 8.1.3.5 c)".to_owned());
    } else {
        let count = usize::from(first_len & 0x7f);
        let mut value: usize = 0;
        let mut leading_zero = false;
        for i in 0..count {
            let b = byte(bytes, pos)
                .filter(|_| pos < end)
                .ok_or("the length is cut off")?;
            if i == 0 && b == 0 {
                leading_zero = true;
            }
            pos = pos.saturating_add(1);
            value = value
                .checked_mul(256)
                .and_then(|v| v.checked_add(usize::from(b)))
                .ok_or("the length is too large")?;
        }
        if value < 0x80 || leading_zero {
            findings.push(finding::der(format!(
                "length {value} not written in the fewest octets (X.690 10.1)"
            )));
        }
        Length::Definite(value)
    };
    Ok(Header {
        tag: Tag {
            class,
            number,
            constructed,
        },
        header_len: pos.saturating_sub(at),
        len,
        findings,
    })
}

/// Reads the TLVs of `range` one after another. Bytes that do not form a TLV end the run as one junk node.
pub(crate) fn parse_all(bytes: &[u8], range: Range<usize>, depth: usize) -> Vec<Raw> {
    let mut out = Vec::new();
    let mut pos = range.start;
    while pos < range.end {
        let raw = parse_one(bytes, pos, range.end, depth);
        pos = raw.end.max(pos.saturating_add(1));
        out.push(raw);
    }
    out
}

/// Reads one TLV at `at`; it must end by `end`.
pub(crate) fn parse_one(bytes: &[u8], at: usize, end: usize, depth: usize) -> Raw {
    let header = match header(bytes, at, end) {
        Ok(header) => header,
        Err(message) => return Raw::junk(at..end, finding::malformed(message)),
    };
    let start = at.saturating_add(header.header_len);
    let mut raw = Raw {
        offset: at,
        header_len: header.header_len,
        len: header.len,
        tag: Some(header.tag),
        content: start..start,
        end: start,
        children: Vec::new(),
        findings: header.findings,
    };
    match header.len {
        Length::Definite(n) => {
            let content_end = match start.checked_add(n) {
                Some(e) if e <= end => e,
                _ => {
                    raw.findings.push(finding::malformed(format!(
                        "the length is {n}, but only {} octets follow",
                        end.saturating_sub(start)
                    )));
                    end
                }
            };
            raw.content = start..content_end;
            raw.end = content_end;
            if header.tag.constructed {
                if depth >= MAX_NESTING {
                    raw.findings
                        .push(finding::malformed("constructed encodings nest too deeply"));
                } else {
                    raw.children = parse_all(bytes, raw.content.clone(), depth.saturating_add(1));
                }
            }
        }
        Length::Indefinite => {
            // contents run to the end-of-contents octets at this level (8.1.5)
            let mut pos = start;
            let mut closed = false;
            while pos < end {
                if byte(bytes, pos) == Some(0)
                    && byte(bytes, pos.saturating_add(1)) == Some(0)
                    && pos.saturating_add(2) <= end
                {
                    closed = true;
                    break;
                }
                if depth >= MAX_NESTING {
                    raw.findings
                        .push(finding::malformed("constructed encodings nest too deeply"));
                    pos = end;
                    break;
                }
                let child = parse_one(bytes, pos, end, depth.saturating_add(1));
                pos = child.end.max(pos.saturating_add(1));
                raw.children.push(child);
            }
            raw.content = start..pos;
            raw.end = if closed { pos.saturating_add(2) } else { pos };
            if !closed {
                raw.findings.push(finding::malformed(
                    "no end-of-contents octets for the indefinite length",
                ));
            }
        }
    }
    raw
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn short_long_and_indefinite_lengths() {
        let bytes = [0x30, 0x03, 0x02, 0x01, 0x05];
        let raws = parse_all(&bytes, 0..bytes.len(), 0);
        assert_eq!(raws.len(), 1);
        assert_eq!(raws[0].children.len(), 1);
        assert!(raws[0].findings.is_empty());

        let long = [0x04, 0x81, 0x01, 0xaa];
        let raw = parse_one(&long, 0, long.len(), 0);
        assert_eq!(raw.len, Length::Definite(1));
        assert_eq!(raw.findings[0].code, "W402");

        let indefinite = [0x30, 0x80, 0x05, 0x00, 0x00, 0x00];
        let raw = parse_one(&indefinite, 0, indefinite.len(), 0);
        assert_eq!(raw.len, Length::Indefinite);
        assert_eq!(raw.children.len(), 1);
        assert_eq!(raw.end, 6);
    }

    #[test]
    fn truncation_and_junk() {
        let cut = [0x04, 0x05, 0x01];
        let raw = parse_one(&cut, 0, cut.len(), 0);
        assert_eq!(raw.content, 2..3);
        assert_eq!(raw.findings[0].code, "E400");

        let junk = [0x1f];
        let raw = parse_one(&junk, 0, junk.len(), 0);
        assert!(raw.tag.is_none());
        assert_eq!(raw.end, 1);
    }

    #[test]
    fn high_tag_numbers() {
        let bytes = [0x9f, 0x81, 0x00, 0x00];
        let raw = parse_one(&bytes, 0, bytes.len(), 0);
        let tag = raw.tag.unwrap();
        assert_eq!((tag.class, tag.number), (TagClass::Context, 128));
    }
}
