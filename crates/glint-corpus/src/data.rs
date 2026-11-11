//! The published data files: `corpus.json`, `documents.bin` and the `latest.json` pointer.
//!
//! Data carries a schema number and code accepts exactly one. The schema is also in the URL path
//! (`data/v<schema>/...`), so code of one schema never sees data of another. Each build of the data has an id,
//! `<UTC date>.<hash>`, which every file of the build repeats.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::CorpusData;

/// The schema of the data this code reads. Bumped whenever the layout of any data file changes.
pub const SCHEMA: u32 = 2;

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
pub const DOCUMENTS_FILE: &str = "documents.bin";

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

fn check_schema(json: &[u8]) -> Result<(), DataError> {
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
    /// Builds the file from `(id, format, text)` triples.
    pub fn write(build: &str, documents: &[(String, String, String)]) -> Vec<u8> {
        let mut entries = Vec::with_capacity(documents.len());
        let mut body: Vec<u8> = Vec::new();
        for (id, format, text) in documents {
            entries.push(DocumentEntry {
                id: id.clone(),
                format: format.clone(),
                start: body.len(),
                len: text.len(),
            });
            body.extend_from_slice(text.as_bytes());
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
            let fits = entry
                .start
                .checked_add(entry.len)
                .is_some_and(|end| end <= body.len());
            if !fits {
                return Err(DataError::Damaged(format!(
                    "documents.bin: {} lies outside the file",
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
        let entry = self.entries.get(*self.by_id.get(id)?)?;
        let end = entry.start.checked_add(entry.len)?;
        let bytes = self.body.get(entry.start..end)?;
        Some((std::str::from_utf8(bytes).ok()?, entry.format.as_str()))
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

    #[test]
    fn documents_round_trip() {
        let docs = vec![
            ("rfc1".to_owned(), "text".to_owned(), "one\ntwo".to_owned()),
            ("rfc2".to_owned(), "xml".to_owned(), "<rfc/>".to_owned()),
        ];
        let bytes = Documents::write("2026-11-05.0123abcd", &docs);
        let read = Documents::read(bytes).unwrap();
        assert_eq!(read.get("rfc1"), Some(("one\ntwo", "text")));
        assert_eq!(read.get("rfc2"), Some(("<rfc/>", "xml")));
        assert_eq!(read.get("rfc3"), None);
        assert!(read.require_build("2026-11-05.0123abcd").is_ok());
        assert!(read.require_build("2026-11-06.0123abcd").is_err());
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
