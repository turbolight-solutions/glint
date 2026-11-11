//! The RFC Editor's errata feed, `errata.json`: one JSON object per erratum, of every status.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value};

use crate::chars::feed_forbidden;

pub const VERIFIED: &str = "Verified";
/// The largest feed accepted (about 12 MB in 2026).
pub const MAX_FEED_BYTES: usize = 64 * 1024 * 1024;
/// A new feed may not have fewer Verified errata than this share of the last good copy's.
const MIN_VERIFIED_PERCENT: usize = 90;
/// A feed with more malformed entries than this share is refused.
const MAX_MALFORMED_PERCENT: usize = 5;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FeedError {
    #[error("the feed is larger than {MAX_FEED_BYTES} bytes")]
    TooLarge,
    #[error("the feed is not UTF-8")]
    NotUtf8,
    #[error("the feed is not JSON: {0}")]
    NotJson(String),
    #[error("the feed is not a JSON list")]
    NotList,
}

/// One erratum of the feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erratum {
    pub errata_id: u32,
    pub rfc: u32,
    /// `Verified`, `Reported`, `Held for Document Update`, `Rejected`.
    pub status: String,
    pub section: String,
    pub orig_text: String,
    pub correct_text: String,
    pub notes: String,
    /// The submitter's and verifier's names. Never printed or stored; only used to keep them out of ledger notes
    /// ([`crate::screen_note`]).
    pub names: Vec<String>,
}

impl Erratum {
    pub fn key(&self) -> (u32, u32) {
        (self.rfc, self.errata_id)
    }

    pub fn is_verified(&self) -> bool {
        self.status == VERIFIED
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Feed {
    /// Every usable erratum, by id.
    pub errata: BTreeMap<u32, Erratum>,
    /// Entries in the JSON list.
    pub total: usize,
    /// Why each skipped entry was skipped, naming `erratum ID (rfcN)` when those were read.
    pub malformed: Vec<String>,
}

impl Feed {
    pub fn verified(&self) -> impl Iterator<Item = &Erratum> {
        self.errata.values().filter(|e| e.is_verified())
    }

    /// The errata of one RFC, every status, by id.
    pub fn for_rfc(&self, rfc: u32) -> impl Iterator<Item = &Erratum> {
        self.errata.values().filter(move |e| e.rfc == rfc)
    }
}

/// A non-empty run of ASCII digits as a number. `char::is_ascii_digit`, so no other script's digits are taken.
pub(crate) fn ascii_number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn errata_id_of(entry: &Value) -> Option<u32> {
    entry
        .get("errata_id")
        .and_then(Value::as_str)
        .and_then(ascii_number)
}

fn text_field(entry: &Map<String, Value>, name: &str) -> Result<String, String> {
    match entry.get(name) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) if s.chars().any(feed_forbidden) => Err(format!(
            "{name} holds a control, bidi or zero-width character"
        )),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(format!("{name} is not a string")),
    }
}

fn parse_entry(entry: &Value) -> Result<Erratum, String> {
    let object = entry.as_object().ok_or("not an object")?;
    let errata_id = errata_id_of(entry).ok_or("errata_id is not a number")?;
    let rfc = object
        .get("doc-id")
        .and_then(Value::as_str)
        .and_then(|doc| doc.strip_prefix("RFC"))
        .and_then(ascii_number)
        .ok_or_else(|| format!("erratum {errata_id}: doc-id is not RFC<number>"))?;
    let label = format!("erratum {errata_id} (rfc{rfc})");
    let status = match object.get("errata_status_code") {
        Some(Value::String(s)) if !s.chars().any(feed_forbidden) => s.clone(),
        _ => return Err(format!("{label}: bad errata_status_code")),
    };
    let field = |name| text_field(object, name).map_err(|e| format!("{label}: {e}"));
    let names = ["submitter_name", "verifier_name"]
        .iter()
        .filter_map(|n| object.get(*n).and_then(Value::as_str))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(Erratum {
        errata_id,
        rfc,
        status,
        section: field("section")?,
        orig_text: field("orig_text")?,
        correct_text: field("correct_text")?,
        notes: field("notes")?,
        names,
    })
}

/// Parses `errata.json`. Entries that fail validation are skipped and listed in [`Feed::malformed`]; an id that occurs
/// more than once skips every copy, a malformed copy included, so the result never depends on the order.
pub fn parse_feed(raw: &[u8]) -> Result<Feed, FeedError> {
    if raw.len() > MAX_FEED_BYTES {
        return Err(FeedError::TooLarge);
    }
    let text = std::str::from_utf8(raw).map_err(|_| FeedError::NotUtf8)?;
    // serde_json refuses nesting deeper than 128, so a hostile feed cannot exhaust the stack.
    let data: Value = serde_json::from_str(text).map_err(|e| FeedError::NotJson(e.to_string()))?;
    let entries = data.as_array().ok_or(FeedError::NotList)?;
    let mut id_counts: HashMap<u32, usize> = HashMap::new();
    for id in entries.iter().filter_map(errata_id_of) {
        let count = id_counts.entry(id).or_default();
        *count = count.saturating_add(1);
    }
    let mut feed = Feed {
        total: entries.len(),
        ..Feed::default()
    };
    for (index, entry) in entries.iter().enumerate() {
        match parse_entry(entry) {
            Err(reason) => feed.malformed.push(format!("entry {index}: {reason}")),
            Ok(e) if id_counts.get(&e.errata_id).copied().unwrap_or_default() > 1 => {
                feed.malformed.push(format!(
                    "erratum {} (rfc{}): duplicate id",
                    e.errata_id, e.rfc
                ))
            }
            Ok(e) => {
                feed.errata.insert(e.errata_id, e);
            }
        }
    }
    Ok(feed)
}

/// Why `feed` should not replace `previous`, the last good copy, or `None` if it may.
pub fn check_plausible(feed: &Feed, previous: Option<&Feed>) -> Option<String> {
    if feed.total == 0 {
        return Some("the feed is empty".to_owned());
    }
    let malformed = feed.malformed.len();
    if malformed.saturating_mul(100) > MAX_MALFORMED_PERCENT.saturating_mul(feed.total) {
        return Some(format!(
            "{malformed} of {} entries are malformed (more than {MAX_MALFORMED_PERCENT}%)",
            feed.total
        ));
    }
    let previous = previous?;
    let old = previous.verified().count();
    let new = feed.verified().count();
    if new.saturating_mul(100) < MIN_VERIFIED_PERCENT.saturating_mul(old) {
        return Some(format!(
            "{new} Verified errata, down from {old} (less than {MIN_VERIFIED_PERCENT}%)"
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, doc: &str) -> Value {
        json!({"errata_id": id, "doc-id": doc, "errata_status_code": "Verified", "section": "3",
               "orig_text": "a", "correct_text": "b", "notes": null,
               "submitter_name": " Sam Submitter ", "verifier_name": ""})
    }

    fn feed(entries: Vec<Value>) -> Feed {
        parse_feed(
            serde_json::to_string(&entries)
                .unwrap_or_default()
                .as_bytes(),
        )
        .unwrap_or_default()
    }

    #[test]
    fn entries_are_parsed_with_null_as_empty_and_names_kept_apart() {
        let f = feed(vec![entry("12", "RFC5280")]);
        let e = &f.errata[&12];
        assert_eq!(
            (e.rfc, e.notes.as_str(), e.names.clone()),
            (5280, "", vec!["Sam Submitter".to_owned()])
        );
        assert_eq!(f.verified().count(), 1);
    }

    #[test]
    fn malformed_entries_are_skipped_and_named() {
        let mut bad_text = entry("3", "RFC1");
        bad_text["orig_text"] = json!("a\u{202e}b");
        let mut number_id = entry("4", "RFC1");
        number_id["errata_id"] = json!(4);
        let f = feed(vec![
            entry("x1", "RFC1"),
            entry("2", "draft-foo"),
            bad_text,
            number_id,
            json!("str"),
        ]);
        assert!(f.errata.is_empty());
        assert_eq!(
            f.malformed,
            [
                "entry 0: errata_id is not a number",
                "entry 1: erratum 2: doc-id is not RFC<number>",
                "entry 2: erratum 3 (rfc1): orig_text holds a control, bidi or zero-width character",
                "entry 3: errata_id is not a number",
                "entry 4: not an object",
            ]
        );
    }

    #[test]
    fn non_ascii_digits_are_not_numbers() {
        assert_eq!(ascii_number("١٢"), None);
        assert_eq!(ascii_number("012"), Some(12));
        assert_eq!(ascii_number(""), None);
    }

    #[test]
    fn a_duplicate_id_skips_every_copy() {
        let mut broken = entry("7", "RFC2");
        broken["section"] = json!(5);
        for entries in [
            vec![entry("7", "RFC1"), broken.clone()],
            vec![broken, entry("7", "RFC1")],
        ] {
            let f = feed(entries);
            assert!(f.errata.is_empty());
            assert!(f.malformed.iter().any(|m| m.contains("duplicate id")));
        }
    }

    #[test]
    fn unusable_documents_are_errors() {
        assert_eq!(parse_feed(b"{}"), Err(FeedError::NotList));
        assert_eq!(parse_feed(b"\xff"), Err(FeedError::NotUtf8));
        assert!(matches!(parse_feed(b"[1,"), Err(FeedError::NotJson(_))));
        let deep = "[".repeat(10_000);
        assert!(matches!(
            parse_feed(deep.as_bytes()),
            Err(FeedError::NotJson(_))
        ));
    }

    #[test]
    fn plausibility_against_the_last_good_copy() {
        let many = feed((1..=20).map(|i| entry(&i.to_string(), "RFC1")).collect());
        let fewer = feed((1..=10).map(|i| entry(&i.to_string(), "RFC1")).collect());
        assert_eq!(
            check_plausible(&fewer, Some(&many)).as_deref(),
            Some("10 Verified errata, down from 20 (less than 90%)")
        );
        assert_eq!(check_plausible(&many, Some(&fewer)), None);
        assert_eq!(
            check_plausible(&feed(vec![]), None).as_deref(),
            Some("the feed is empty")
        );
        let mut entries: Vec<Value> = (1..=10).map(|i| entry(&i.to_string(), "RFC1")).collect();
        entries.push(json!(1));
        assert!(check_plausible(&feed(entries), None).is_some_and(|m| m.contains("malformed")));
    }
}
