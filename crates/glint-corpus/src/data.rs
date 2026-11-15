//! The published data files: `corpus.json`, `documents.bin`, `artifacts.json` and the `latest.json` pointer.
//!
//! Data carries a schema number and code accepts exactly one. The schema is also in the URL path
//! (`data/v<schema>/...`), so code of one schema never sees data of another. Each build of the data has an id,
//! `<UTC date>.<hash>`, which every file of the build repeats.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::CorpusData;

/// The schema of the data this code reads. Bumped whenever the layout of any data file changes.
pub const SCHEMA: u32 = 5;

/// The directory of every build of the current schema, relative to the site root.
pub fn schema_dir() -> String {
    format!("data/v{SCHEMA}")
}

/// Where the pointer to the current build is published, relative to the site root.
pub fn latest_path() -> String {
    format!("{}/latest.json", schema_dir())
}

/// The directory of one build, relative to the site root.
pub fn build_dir(build: &str) -> String {
    format!("{}/{build}", schema_dir())
}

pub const CORPUS_FILE: &str = "corpus.json";
/// `documents.bin`, gzipped: the site serves it as it is (GitHub Pages would not compress it), and the page unpacks it.
/// [`Documents::read`] takes the unpacked bytes.
pub const DOCUMENTS_FILE: &str = "documents.bin.gz";

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error(
        "the data has schema {found}, but this version of Glint reads schema {SCHEMA}; upgrade Glint or use data of schema {SCHEMA}"
    )]
    Schema { found: u32 },
    #[error("the data is damaged: {0}")]
    Damaged(String),
    #[error("{file} is from build {found}, but the corpus is from build {expected}")]
    BuildMismatch {
        file: &'static str,
        found: String,
        expected: String,
    },
}

#[derive(Deserialize)]
struct SchemaOnly {
    schema: u32,
}

pub(crate) fn check_schema(json: &[u8]) -> Result<(), DataError> {
    let found = serde_json::from_slice::<SchemaOnly>(json)
        .map_err(|e| DataError::Damaged(e.to_string()))?
        .schema;
    if found == SCHEMA {
        Ok(())
    } else {
        Err(DataError::Schema { found })
    }
}

/// Whether `text` looks like a build id: `YYYY-MM-DD.` and eight lowercase hex digits.
pub fn valid_build_id(text: &str) -> bool {
    let Some((date, hash)) = text.split_once('.') else {
        return false;
    };
    let date_ok = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    date_ok
        && hash.len() == 8
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The date part of a build id.
pub fn build_date(build: &str) -> &str {
    build.split_once('.').map_or(build, |(date, _)| date)
}

impl CorpusData {
    /// Reads `corpus.json`, refusing data of another schema.
    pub fn from_json(json: &[u8]) -> Result<Self, DataError> {
        check_schema(json)?;
        serde_json::from_slice(json).map_err(|e| DataError::Damaged(e.to_string()))
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// `latest.json`: names the current build of a schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointer {
    pub schema: u32,
    pub build: String,
}

impl Pointer {
    pub fn from_json(json: &[u8]) -> Result<Self, DataError> {
        check_schema(json)?;
        let pointer: Self =
            serde_json::from_slice(json).map_err(|e| DataError::Damaged(e.to_string()))?;
        if valid_build_id(&pointer.build) {
            Ok(pointer)
        } else {
            Err(DataError::Damaged(format!(
                "`{}` is not a build id",
                pointer.build
            )))
        }
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// One document in `documents.bin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentEntry {
    /// The document id: `rfc5280`, `draft-ietf-lamps-foo-03`.
    pub id: String,
    /// `xml` or `text`.
    pub format: String,
    /// Byte range of the text in the body.
    pub start: usize,
    pub len: usize,
    /// For the text of an RFC or draft whose source is RFCXML: the blocks of the XML elements that XML extraction takes
    /// as ASN.1, which a check extracts instead of reading the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<DocumentBlock>>,
    /// For the text of an RFC or draft whose source is RFCXML (with `blocks`): where in the body its RFCXML is (an RFC's
    /// corrected: the published XML with the XML patches of its errata), the document's second view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xml: Option<BodyRange>,
}

/// A byte range of the body of `documents.bin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyRange {
    pub start: usize,
    pub len: usize,
}

impl BodyRange {
    fn of<'a>(&self, body: &'a [u8]) -> Option<&'a str> {
        let end = self.start.checked_add(self.len)?;
        std::str::from_utf8(body.get(self.start..end)?).ok()
    }
}

/// A block of a document's text that its RFCXML source marks as ASN.1 (or that XML extraction takes as ASN.1 by its
/// content: no `type`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentBlock {
    /// Byte range in the document's text of the block's lines, line breaks included.
    pub start: usize,
    pub end: usize,
    /// The margin the text puts before each line of the element's content: `indent` bytes of spaces, with a `|` at
    /// byte `bar` when the element is quoted (`<blockquote>`).
    pub indent: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<usize>,
    /// In a paginated text (an Internet-Draft's): the byte ranges `[start, end)` in the text of the page breaks between
    /// the block's lines (footer, form feed, header and the blank lines around them), which are not the element's
    /// content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<[usize; 2]>,
    /// The element's `type` attribute.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_attr: Option<String>,
    /// The section number (`4.1`, `A`), and whether it is an appendix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub appendix: bool,
    /// The section title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// A document to write into `documents.bin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentText {
    pub id: String,
    /// `xml` or `text`.
    pub format: String,
    pub text: String,
    /// See [`DocumentEntry::blocks`].
    pub blocks: Option<Vec<DocumentBlock>>,
    /// See [`DocumentEntry::xml`].
    pub xml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DocumentsHeader {
    schema: u32,
    build: String,
    documents: Vec<DocumentEntry>,
}

/// `documents.bin`: the full text of every document with checkable ASN.1, for checking a document by name without
/// fetching it. The file is a JSON header on the first line, then the texts one after another.
#[derive(Debug, Default)]
pub struct Documents {
    build: String,
    entries: Vec<DocumentEntry>,
    by_id: HashMap<String, usize>,
    body: Vec<u8>,
}

impl Documents {
    /// Builds the file.
    pub fn write(build: &str, documents: &[DocumentText]) -> Vec<u8> {
        let mut entries = Vec::with_capacity(documents.len());
        let mut body: Vec<u8> = Vec::new();
        for document in documents {
            let start = body.len();
            body.extend_from_slice(document.text.as_bytes());
            let xml = document.xml.as_ref().map(|xml| {
                let range = BodyRange {
                    start: body.len(),
                    len: xml.len(),
                };
                body.extend_from_slice(xml.as_bytes());
                range
            });
            entries.push(DocumentEntry {
                id: document.id.clone(),
                format: document.format.clone(),
                start,
                len: document.text.len(),
                blocks: document.blocks.clone(),
                xml,
            });
        }
        let header = DocumentsHeader {
            schema: SCHEMA,
            build: build.to_owned(),
            documents: entries,
        };
        let mut out = serde_json::to_vec(&header).unwrap_or_default();
        out.push(b'\n');
        out.extend_from_slice(&body);
        out
    }

    /// Reads the file, refusing another schema.
    pub fn read(bytes: Vec<u8>) -> Result<Self, DataError> {
        let newline = bytes
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| DataError::Damaged("documents.bin has no header".to_owned()))?;
        let (head, rest) = bytes.split_at(newline);
        check_schema(head)?;
        let header: DocumentsHeader =
            serde_json::from_slice(head).map_err(|e| DataError::Damaged(e.to_string()))?;
        let body = rest.get(1..).unwrap_or_default().to_vec();
        for entry in &header.documents {
            let text = BodyRange {
                start: entry.start,
                len: entry.len,
            };
            let fits = |range: &BodyRange| {
                range
                    .start
                    .checked_add(range.len)
                    .is_some_and(|end| end <= body.len())
            };
            if !fits(&text) || entry.xml.as_ref().is_some_and(|xml| !fits(xml)) {
                return Err(DataError::Damaged(format!(
                    "documents.bin: {} lies outside the file",
                    entry.id
                )));
            }
            let blocks_fit = entry.blocks.iter().flatten().all(|b| {
                let mut at = b.start;
                b.end <= entry.len
                    && b.breaks.iter().all(|&[start, end]| {
                        let inside = at <= start && start <= end;
                        at = end;
                        inside
                    })
                    && at <= b.end
            });
            if !blocks_fit {
                return Err(DataError::Damaged(format!(
                    "documents.bin: a block of {} lies outside its text",
                    entry.id
                )));
            }
        }
        let by_id = header
            .documents
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.id.clone(), index))
            .collect();
        Ok(Self {
            build: header.build,
            entries: header.documents,
            by_id,
            body,
        })
    }

    /// Refuses the file unless it is from the same build as the corpus.
    pub fn require_build(&self, expected: &str) -> Result<(), DataError> {
        if self.build == expected {
            Ok(())
        } else {
            Err(DataError::BuildMismatch {
                file: DOCUMENTS_FILE,
                found: self.build.clone(),
                expected: expected.to_owned(),
            })
        }
    }

    pub fn build(&self) -> &str {
        &self.build
    }

    pub fn entries(&self) -> &[DocumentEntry] {
        &self.entries
    }

    /// The text and format of a document.
    pub fn get(&self, id: &str) -> Option<(&str, &str)> {
        let entry = self.entry(id)?;
        let text = BodyRange {
            start: entry.start,
            len: entry.len,
        }
        .of(&self.body)?;
        Some((text, entry.format.as_str()))
    }

    /// The RFCXML of an RFC (corrected) or draft stored as its text, when the file has it ([`DocumentEntry::xml`]).
    pub fn xml(&self, id: &str) -> Option<&str> {
        self.entry(id)?.xml.as_ref()?.of(&self.body)
    }

    /// The header entry of a document.
    pub fn entry(&self, id: &str) -> Option<&DocumentEntry> {
        self.entries.get(*self.by_id.get(id)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_round_trips_and_another_schema_is_refused() {
        let mut data = CorpusData::default();
        data.info.build = "2026-11-05.0123abcd".to_owned();
        let json = data.to_json();
        assert_eq!(CorpusData::from_json(&json).unwrap(), data);
        let other = String::from_utf8(json).unwrap().replacen(
            &format!("\"schema\":{SCHEMA}"),
            "\"schema\":99",
            1,
        );
        assert!(matches!(
            CorpusData::from_json(other.as_bytes()),
            Err(DataError::Schema { found: 99 })
        ));
    }

    fn document(id: &str, format: &str, text: &str) -> DocumentText {
        DocumentText {
            id: id.to_owned(),
            format: format.to_owned(),
            text: text.to_owned(),
            blocks: None,
            xml: None,
        }
    }

    #[test]
    fn documents_round_trip() {
        let block = DocumentBlock {
            start: 4,
            end: 8,
            indent: 3,
            bar: None,
            breaks: vec![[5, 6]],
            type_attr: Some("asn.1".to_owned()),
            section: Some("A".to_owned()),
            appendix: true,
            title: None,
        };
        let docs = vec![
            document("rfc1", "text", "one\ntwo"),
            document("rfc2", "xml", "<rfc/>"),
            DocumentText {
                blocks: Some(vec![block.clone()]),
                xml: Some("<rfc>one A</rfc>".to_owned()),
                ..document("rfc8692", "text", "one\n   A\n")
            },
            document("rfc3", "text", "three"),
        ];
        let bytes = Documents::write("2026-11-05.0123abcd", &docs);
        let read = Documents::read(bytes).unwrap();
        assert_eq!(read.get("rfc1"), Some(("one\ntwo", "text")));
        assert_eq!(read.get("rfc2"), Some(("<rfc/>", "xml")));
        assert_eq!(read.get("rfc4"), None);
        assert_eq!(read.entry("rfc1").unwrap().blocks, None);
        assert_eq!(read.entry("rfc8692").unwrap().blocks, Some(vec![block]));
        // The second view of an RFC stored as text, and the text after it, each where they belong.
        assert_eq!(read.get("rfc8692"), Some(("one\n   A\n", "text")));
        assert_eq!(read.xml("rfc8692"), Some("<rfc>one A</rfc>"));
        assert_eq!(read.get("rfc3"), Some(("three", "text")));
        assert_eq!(read.xml("rfc1"), None);
        assert_eq!(read.xml("rfc4"), None);
        assert!(read.require_build("2026-11-05.0123abcd").is_ok());
        assert!(read.require_build("2026-11-06.0123abcd").is_err());
    }

    #[test]
    fn a_block_outside_its_text_is_damage() {
        let block = |start, end, breaks: Vec<[usize; 2]>| DocumentBlock {
            start,
            end,
            indent: 0,
            bar: None,
            breaks,
            type_attr: None,
            section: None,
            appendix: false,
            title: None,
        };
        let read = |block: DocumentBlock| {
            let docs = vec![DocumentText {
                blocks: Some(vec![block]),
                ..document("rfc1", "text", "one\ntwo")
            }];
            Documents::read(Documents::write("2026-11-05.0123abcd", &docs))
        };
        assert!(read(block(0, 7, vec![[2, 3], [4, 5]])).is_ok());
        // Past the text, a page break outside the block, out of order, or backwards.
        for damaged in [
            block(2, 9, Vec::new()),
            block(0, 4, vec![[3, 5]]),
            block(2, 7, vec![[1, 3]]),
            block(0, 7, vec![[4, 5], [2, 3]]),
            block(0, 7, vec![[3, 2]]),
        ] {
            assert!(
                matches!(read(damaged.clone()), Err(DataError::Damaged(m)) if m.contains("block of rfc1")),
                "{damaged:?}"
            );
        }
    }

    #[test]
    fn an_xml_view_outside_the_file_is_damage() {
        let docs = vec![DocumentText {
            xml: Some("<rfc/>".to_owned()),
            ..document("rfc1", "text", "one")
        }];
        let mut bytes = Documents::write("2026-11-05.0123abcd", &docs);
        bytes.truncate(bytes.len() - 1);
        assert!(
            matches!(Documents::read(bytes), Err(DataError::Damaged(m)) if m.contains("rfc1 lies outside"))
        );
    }

    #[test]
    fn build_ids() {
        assert!(valid_build_id("2026-11-05.0123abcd"));
        assert!(!valid_build_id("2026-11-05"));
        assert!(!valid_build_id("2026-11-05.0123ABCD"));
        assert!(!valid_build_id("../../etc.0123abcd"));
        assert_eq!(build_date("2026-11-05.0123abcd"), "2026-11-05");
    }
}
