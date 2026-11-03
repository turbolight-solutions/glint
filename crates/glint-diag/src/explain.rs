//! Long descriptions of the diagnostic codes, for `glint explain` and the rule help in SARIF.

const ENTRIES: &str = include_str!("../explain.md");

/// One fenced example in an explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example<'a> {
    /// What follows the language on the fence: `bad`, `good`, or other words.
    pub tags: Vec<&'a str>,
    pub language: &'a str,
    pub text: &'a str,
}

fn entries() -> impl Iterator<Item = (&'static str, &'static str)> {
    ENTRIES.split("\n## ").skip(1).filter_map(|section| {
        let (code, body) = section.split_once('\n')?;
        Some((code.trim(), body.trim_matches('\n')))
    })
}

/// The explanation of `code` as Markdown, if it has one.
pub fn explain(code: &str) -> Option<&'static str> {
    entries()
        .find(|(entry, _)| *entry == code)
        .map(|(_, body)| body)
}

/// Every code that has an explanation.
pub fn explained_codes() -> impl Iterator<Item = &'static str> {
    entries().map(|(code, _)| code)
}

/// The fenced examples of an explanation: ```` ```asn1 bad ````, ```` ```asn1 good ````.
pub fn examples(body: &str) -> Vec<Example<'_>> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("```") {
        let after = rest.get(start.saturating_add(3)..).unwrap_or_default();
        let Some((info, tail)) = after.split_once('\n') else {
            break;
        };
        let Some(end) = tail.find("\n```") else {
            break;
        };
        let mut words = info.split_whitespace();
        let language = words.next().unwrap_or_default();
        out.push(Example {
            tags: words.collect(),
            language,
            text: tail.get(..end).unwrap_or_default(),
        });
        rest = tail.get(end.saturating_add(4)..).unwrap_or_default();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes::all_codes;

    #[test]
    fn every_registered_code_has_an_explanation() {
        for info in all_codes() {
            assert!(explain(info.code).is_some(), "{} has no entry", info.code);
        }
    }

    #[test]
    fn every_explanation_belongs_to_a_registered_code() {
        for code in explained_codes() {
            assert!(crate::code_info(code).is_some(), "{code} is not registered");
        }
    }

    #[test]
    fn examples_are_read_with_their_tags() {
        let found =
            examples("text\n```asn1 bad\nA ::= B\n```\nmore\n```asn1 good\nA ::= NULL\n```\n");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].tags, ["bad"]);
        assert_eq!(found[0].text, "A ::= B");
        assert_eq!(found[1].language, "asn1");
    }
}
