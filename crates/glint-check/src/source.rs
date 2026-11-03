//! Names and URLs of the documents Glint can fetch by name: Internet-Drafts and RFCs. No I/O happens here.

/// A draft or RFC to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentRef {
    /// A draft, with its revision when the caller gave one.
    Draft {
        name: String,
        rev: Option<String>,
    },
    Rfc(u32),
}

/// Why a name or number was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefError {
    #[error("a draft name looks like `draft-ietf-lamps-foo` or `draft-ietf-lamps-foo-03`")]
    BadDraft,
    #[error("a revision is two digits, like `03`")]
    BadRevision,
    #[error("an RFC number is between 1 and 99999")]
    BadRfc,
}

pub fn valid_draft_name(name: &str) -> bool {
    name.len() <= 120
        && name.starts_with("draft-")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.ends_with('-')
        && !name.contains("--")
}

pub fn valid_rev(rev: &str) -> bool {
    rev.len() == 2 && rev.bytes().all(|b| b.is_ascii_digit())
}

/// `draft-foo-bar-03` without a separate revision is the draft `draft-foo-bar` at revision `03`.
pub fn split_revision(draft: String, rev: Option<String>) -> (String, Option<String>) {
    if rev.is_some() {
        return (draft, rev);
    }
    match draft.rsplit_once('-') {
        Some((base, tail)) if valid_rev(tail) && valid_draft_name(base) => {
            (base.to_owned(), Some(tail.to_owned()))
        }
        _ => (draft, None),
    }
}

impl DocumentRef {
    /// Reads `draft-ietf-lamps-foo-03`, `draft-ietf-lamps-foo`, `rfc5280` or `RFC 5280`.
    pub fn parse(text: &str) -> Result<Self, RefError> {
        let lower = text.trim().to_ascii_lowercase();
        if let Some(number) = lower.strip_prefix("rfc") {
            let number = number.trim_start_matches([' ', '-']);
            return number
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=99_999).contains(n))
                .map(Self::Rfc)
                .ok_or(RefError::BadRfc);
        }
        Self::draft(&lower, None)
    }

    /// A draft by name and optional revision, checking both.
    pub fn draft(name: &str, rev: Option<&str>) -> Result<Self, RefError> {
        let (name, rev) = split_revision(name.trim().to_ascii_lowercase(), rev.map(str::to_owned));
        if !valid_draft_name(&name) {
            return Err(RefError::BadDraft);
        }
        if rev.as_deref().is_some_and(|r| !valid_rev(r)) {
            return Err(RefError::BadRevision);
        }
        Ok(Self::Draft { name, rev })
    }

    /// An RFC by number, checking the range.
    pub fn rfc(number: u32) -> Result<Self, RefError> {
        if (1..=99_999).contains(&number) {
            Ok(Self::Rfc(number))
        } else {
            Err(RefError::BadRfc)
        }
    }
}

/// Where the latest revision number of a draft is published.
pub fn latest_revision_url(draft: &str) -> String {
    format!("https://datatracker.ietf.org/api/v1/doc/document/{draft}/?format=json")
}

/// The latest revision in the datatracker's answer, if it is a valid one.
pub fn revision_from_datatracker(body: &str) -> Option<String> {
    let rev = serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("rev")?
        .as_str()?
        .to_owned();
    valid_rev(&rev).then_some(rev)
}

/// The URLs to try for a draft revision, best source first.
pub fn draft_urls(draft: &str, rev: &str) -> [String; 2] {
    [
        format!("https://www.ietf.org/archive/id/{draft}-{rev}.xml"),
        format!("https://www.ietf.org/archive/id/{draft}-{rev}.txt"),
    ]
}

/// The URLs to try for an RFC, best source first.
pub fn rfc_urls(number: u32) -> [String; 2] {
    [
        format!("https://www.rfc-editor.org/rfc/rfc{number}.xml"),
        format!("https://www.rfc-editor.org/rfc/rfc{number}.txt"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_revision_in_the_name_is_split_off() {
        assert_eq!(
            DocumentRef::parse("draft-ietf-lamps-foo-03"),
            Ok(DocumentRef::Draft {
                name: "draft-ietf-lamps-foo".into(),
                rev: Some("03".into())
            })
        );
        assert_eq!(
            DocumentRef::parse("draft-ietf-lamps-foo"),
            Ok(DocumentRef::Draft {
                name: "draft-ietf-lamps-foo".into(),
                rev: None
            })
        );
    }

    #[test]
    fn rfcs_are_read_in_several_spellings() {
        for text in ["rfc5280", "RFC 5280", "rfc-5280"] {
            assert_eq!(DocumentRef::parse(text), Ok(DocumentRef::Rfc(5280)));
        }
        assert_eq!(DocumentRef::parse("rfc0"), Err(RefError::BadRfc));
    }

    #[test]
    fn bad_names_are_refused() {
        assert_eq!(DocumentRef::parse("foo"), Err(RefError::BadDraft));
        assert_eq!(
            DocumentRef::draft("draft-a-b", Some("3")),
            Err(RefError::BadRevision)
        );
    }

    #[test]
    fn the_datatracker_answer_gives_the_revision() {
        assert_eq!(
            revision_from_datatracker(r#"{"rev":"07"}"#),
            Some("07".into())
        );
        assert_eq!(revision_from_datatracker(r#"{"rev":"x"}"#), None);
        assert_eq!(revision_from_datatracker("nope"), None);
    }
}
