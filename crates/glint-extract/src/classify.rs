use std::sync::LazyLock;

use glint_syntax::{SyntaxKind, lex};
use regex::Regex;

use crate::block::BlockKind;

fn significant(text: &str) -> Vec<(SyntaxKind, &str)> {
    let (tokens, _) = lex(text);
    tokens
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .filter_map(|token| {
            let range = usize::from(token.range.start())..usize::from(token.range.end());
            text.get(range).map(|slice| (token.kind, slice))
        })
        .collect()
}

fn is_module_header_word(kind: SyntaxKind, text: &str) -> bool {
    kind == SyntaxKind::UpperName && text == "DEFINITIONS"
}

/// Whether `text` is plausibly ASN.1: it has a module header, or starts with a name and contains `::=`.
pub fn looks_like_asn1(text: &str) -> bool {
    let tokens = significant(text);
    let starts_with_name = tokens
        .first()
        .is_some_and(|(kind, _)| matches!(kind, SyntaxKind::UpperName | SyntaxKind::LowerName));
    let has_assign = tokens.iter().any(|(kind, _)| *kind == SyntaxKind::Assign);
    let has_header = tokens
        .iter()
        .any(|(kind, word)| is_module_header_word(*kind, word));
    has_assign && (has_header || starts_with_name)
}

/// An SMI macro, or an import from a module that defines them: the mark of a MIB (SMIv1, SMIv2), and of SPPI.
static SMI: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"\b(OBJECT-TYPE|MODULE-IDENTITY|NOTIFICATION-TYPE|OBJECT-IDENTITY|TEXTUAL-CONVENTION|MODULE-COMPLIANCE|OBJECT-GROUP|NOTIFICATION-GROUP|AGENT-CAPABILITIES|TRAP-TYPE)\b|FROM\s+(SNMPv2-SMI|RFC1155-SMI|RFC1065-SMI|SNMPv2-TC|SNMPv2-CONF|RFC-1212|RFC-1215)\b",
    )
    .ok()
});

/// Whether `text` is written in SMI, the MIB language: ASN.1 with macros of its own, which `smilint` checks and Glint
/// does not.
pub fn is_smi(text: &str) -> bool {
    SMI.as_ref().is_some_and(|re| re.is_match(text))
}

/// Whether a block that is not a module is ASN.1 that Glint checks: not a grammar written with `::=` (BNF, ABNF,
/// Diameter command codes) and not SMI. Broken ASN.1 is still ASN.1: this never asks whether the text parses.
pub fn is_inline_asn1(text: &str) -> bool {
    !is_smi(text) && !is_grammar(text)
}

/// ASN.1 comments: from `--` to the next `--` or the end of the line.
static COMMENT: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?m)--.*?(?:--|$)").ok());
static STRING: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#""[^"\n]*""#).ok());
/// A BNF nonterminal (`<Host-number>`, `< AVP Header: 376 >`); ASN.1 writes `<` only in ranges and selection types.
static ANGLE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"<\s*[A-Za-z][^<>\n]*>|<[A-Za-z]").ok());
/// ABNF repetition and character values (`1*DIGIT`, `%x41`, `#x20`); ASN.1 has none of these characters.
static OPERATOR: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\*|[#%]x").ok());
static DEFAULT_STRING: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"DEFAULT\s+"""#).ok());

fn replace_all(re: &LazyLock<Option<Regex>>, text: &str, with: &str) -> String {
    re.as_ref().map_or_else(
        || text.to_owned(),
        |re| re.replace_all(text, with).into_owned(),
    )
}

fn matches(re: &LazyLock<Option<Regex>>, text: &str) -> bool {
    re.as_ref().is_some_and(|re| re.is_match(text))
}

/// Whether most assignments in `text` are grammar rules rather than ASN.1. A rule is marked by what ASN.1 does not
/// write outside braces and parentheses: alternatives with `|` or `/`, a quoted literal as an element, `?` or `+`;
/// anywhere, ABNF's `*` and `%x`/`#x`; a `<nonterminal>`; or a left-hand side that is not a name.
fn is_grammar(text: &str) -> bool {
    let text = replace_all(&COMMENT, text, " ");
    let rules = assignments(&text);
    let marked = rules
        .iter()
        .filter(|(lhs, rhs)| is_grammar_rule(lhs, rhs))
        .count();
    !rules.is_empty() && marked.saturating_mul(2) >= rules.len()
}

/// Each `::=` of `text` with what comes before it on its line (or, when nothing does, the line before) and the text
/// up to the next one.
fn assignments(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let at: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("::="))
        .map(|(i, _)| i)
        .collect();
    let lhs_on_line = |i: usize| {
        lines
            .get(i)
            .and_then(|line| line.split_once("::="))
            .map_or("", |(lhs, _)| lhs.trim())
    };
    let mut rules = Vec::new();
    for (k, &i) in at.iter().enumerate() {
        let Some((lhs, first)) = lines.get(i).and_then(|line| line.split_once("::=")) else {
            continue;
        };
        let lhs = if lhs.trim().is_empty() {
            lines
                .get(..i)
                .unwrap_or_default()
                .iter()
                .rev()
                .find(|line| !line.trim().is_empty())
                .map_or("", |line| line.trim())
        } else {
            lhs.trim()
        };
        let next = at.get(k.saturating_add(1)).copied();
        let mut end = next.unwrap_or(lines.len());
        // The next rule's name on a line of its own belongs to the next rule.
        if next.is_some_and(|n| lhs_on_line(n).is_empty()) {
            end = end.saturating_sub(1).max(i.saturating_add(1));
        }
        let rest = lines.get(i.saturating_add(1)..end).unwrap_or_default();
        let mut rhs = first.to_owned();
        for line in rest {
            rhs.push('\n');
            rhs.push_str(line);
        }
        rules.push((lhs.to_owned(), rhs));
    }
    rules
}

/// The text of `rhs` outside parentheses, braces and brackets, up to the first blank line there: a definition ends
/// at a blank line, and prose may follow it.
fn top_level(rhs: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut seen = false;
    for (at, ch) in rhs.char_indices() {
        if ch == '\n' && depth == 0 && seen {
            let after = rhs.get(at.saturating_add(1)..).unwrap_or_default();
            if after.trim_start_matches([' ', '\t']).starts_with('\n') {
                break;
            }
        }
        if !ch.is_whitespace() {
            seen = true;
        }
        match ch {
            '(' | '{' | '[' => {
                depth = depth.saturating_add(1);
                out.push(' ');
            }
            ')' | '}' | ']' => {
                depth = depth.saturating_sub(1);
                out.push(' ');
            }
            _ if depth == 0 => out.push(ch),
            _ => out.push(' '),
        }
    }
    out
}

fn is_grammar_rule(lhs: &str, rhs: &str) -> bool {
    let single_name = lhs.split_whitespace().count() == 1;
    let lhs_is_not_a_name = lhs.contains(['<', '>', '$'])
        || lhs
            .chars()
            .next()
            .is_some_and(|c| !c.is_ascii_alphabetic() && c != '&');
    let rhs = replace_all(&STRING, rhs, "\"\"");
    let outside = top_level(&rhs);
    let quoted = single_name && replace_all(&DEFAULT_STRING, &outside, "").contains("\"\"");
    let angle = single_name && !rhs.contains("..") && matches(&ANGLE, &rhs);
    let operator = outside.contains(['|', '/', '?', '+']) || matches(&OPERATOR, &rhs);
    lhs_is_not_a_name || quoted || angle || operator
}

/// Whether `tokens`, which start at a module header's `DEFINITIONS` and run to the next header, hold a module body:
/// an assignment, `IMPORTS`, `EXPORTS` or `END` after `BEGIN`. A header with nothing after its `BEGIN` is a quote of
/// the header, as in RFC 4998 Section 2.1.1, which shows `ERS {...} DEFINITIONS IMPLICIT TAGS ::= BEGIN` before the
/// module in Appendix B; a module missing only its `END` still has a body.
fn has_body(tokens: &[(SyntaxKind, &str)]) -> bool {
    let Some(begin) = tokens
        .iter()
        .position(|(kind, word)| *kind == SyntaxKind::UpperName && *word == "BEGIN")
    else {
        return false;
    };
    tokens
        .get(begin.saturating_add(1)..)
        .unwrap_or_default()
        .iter()
        .any(|(kind, word)| {
            *kind == SyntaxKind::Assign
                || (*kind == SyntaxKind::UpperName
                    && matches!(*word, "END" | "IMPORTS" | "EXPORTS"))
        })
}

/// Classifies `text` by the number of module definitions it contains: module headers followed by a body. A block
/// whose only headers have no body is a fragment.
pub fn block_kind(text: &str) -> BlockKind {
    let tokens = significant(text);
    let starts: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, (kind, word))| is_module_header_word(*kind, word))
        .map(|(at, _)| at)
        .collect();
    let headers = starts
        .iter()
        .enumerate()
        .filter(|&(number, &start)| {
            let end = starts
                .get(number.saturating_add(1))
                .copied()
                .unwrap_or(tokens.len());
            has_body(tokens.get(start..end).unwrap_or_default())
        })
        .count();
    match headers {
        0 => BlockKind::Fragment,
        1 => BlockKind::Module,
        _ => BlockKind::ModuleList,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_text_looks_like_asn1() {
        assert!(looks_like_asn1("M DEFINITIONS ::= BEGIN END"));
        assert_eq!(block_kind("M DEFINITIONS ::= BEGIN END"), BlockKind::Module);
    }

    #[test]
    fn assignments_are_fragments() {
        assert!(looks_like_asn1("-- note\nFoo ::= INTEGER"));
        assert_eq!(block_kind("Foo ::= INTEGER"), BlockKind::Fragment);
        assert!(looks_like_asn1("id-foo OBJECT IDENTIFIER ::= { 1 2 }"));
    }

    #[test]
    fn two_headers_make_a_module_list() {
        let text = "A DEFINITIONS ::= BEGIN END\nB DEFINITIONS ::= BEGIN END";
        assert_eq!(block_kind(text), BlockKind::ModuleList);
    }

    #[test]
    fn a_header_with_nothing_after_begin_is_a_fragment() {
        let header = "ERS {iso(1) identified-organization(3) dod(6)\n  internet(1) security(5) mechanisms(5)\n  \
                      ltans(11) id-mod(0) id-mod-ers88(2) id-mod-ers88-v1(1) }\nDEFINITIONS IMPLICIT TAGS ::=\nBEGIN\n";
        assert!(looks_like_asn1(header));
        assert_eq!(block_kind(header), BlockKind::Fragment);
        assert_eq!(block_kind("M DEFINITIONS ::="), BlockKind::Fragment);
    }

    #[test]
    fn two_quoted_headers_are_a_fragment() {
        let text = "A { 1 2 } DEFINITIONS ::= BEGIN\nB { 1 3 } DEFINITIONS ::= BEGIN\n";
        assert_eq!(block_kind(text), BlockKind::Fragment);
    }

    #[test]
    fn a_module_body_makes_a_module_even_without_end() {
        assert_eq!(block_kind("M DEFINITIONS ::= BEGIN END"), BlockKind::Module);
        assert_eq!(
            block_kind("M DEFINITIONS ::= BEGIN Foo ::= INTEGER"),
            BlockKind::Module
        );
        assert_eq!(
            block_kind("M DEFINITIONS ::= BEGIN IMPORTS Foo FROM N;"),
            BlockKind::Module
        );
        let text = "A { 1 2 } DEFINITIONS ::= BEGIN\nB DEFINITIONS ::= BEGIN Foo ::= INTEGER END";
        assert_eq!(block_kind(text), BlockKind::Module);
    }

    #[test]
    fn grammars_written_with_assignments_are_not_inline_asn1() {
        // RFC 1730 (IMAP4 ABNF-like BNF), RFC 4740 (Diameter command codes), RFC 166 (BNF with `|` and `<null>`),
        // RFC 1009 (`IP-address ::= { <Network-number>, <Host-number> }`), RFC 1869 (ABNF repetition), RFC 4194.
        for grammar in [
            "address         ::= \"(\" addr_name SPACE addr_adl SPACE addr_mailbox\n                    SPACE addr_host \")\"\n\naddr_adl        ::= nstring\n",
            "SIP-Auth-Data-Item ::= < AVP Header: 376 >\n                       { SIP-Authentication-Scheme }\n",
            "label          ::=  INTEGER | <null>\ninputstream    ::=  terms | <null>\nterms          ::=  term | terms , term\n",
            "IP-address ::=  { <Network-number>, <Host-number> }\n",
            "esmtp-keyword ::= (ALPHA / DIGIT) *(ALPHA / DIGIT / \"-\")\nsize-param ::= [1*DIGIT]\n",
            "whitespace ::= (#x20 | #x9 | #xC | #xD | #xA)\nhexdigit   ::= [0-9A-Fa-f]\nhexbytes   ::= whitespace* hexdigit (hexdigit|whitespace)*\n",
            "msg_recent      ::= \"RECENT\"\nnil             ::= \"NIL\"\n",
        ] {
            assert!(!is_inline_asn1(grammar), "{grammar}");
        }
    }

    #[test]
    fn broken_asn1_is_still_inline_asn1() {
        // RFC 3280's `Name` lacks its identifier; RFC 9215 writes `NUMERIC STRING`; RFC 1028 uses underscores;
        // RFC 2524 ends definitions with `;`.
        for asn1 in [
            "Name ::= CHOICE {\n  RDNSequence }\n\nRDNSequence ::= SEQUENCE OF RelativeDistinguishedName\n",
            "OGRN ::= NUMERIC STRING (SIZE(13))\n",
            "var_value_type          ::=     CHOICE {\n    number INTEGER,\n    string OCTET STRING }\n",
            "DateTime ::= INTEGER;\nContentIntegrityCheck ::= INTEGER (0..65535);\n",
        ] {
            assert!(is_inline_asn1(asn1), "{asn1}");
        }
    }

    #[test]
    fn asn1_that_writes_grammar_characters_inside_brackets_is_inline_asn1() {
        for asn1 in [
            "Version ::= UTF8String (\"1.0\" | \"2.0\")\n",
            "MyMessageType ::= SEQUENCE {\n   version UTF8String DEFAULT \"1.0\" }\n",
            "Traffic-Light ::= [VALUES ALL CAPITALIZED, red AS \"RED\"]\n    ENUMERATED { red, amber, green }\n",
            "Algorithms ALGORITHM ::= { alg-a | alg-b, ... }\n",
            "Small ::= INTEGER (0<..<10)\n",
            "mount OBJECT-CLASS ::= {\n    SUBCLASS OF { top }\n    ID { <oid3> } }\n",
            "Choice ::= SEQUENCE { a  alternative < Message }\n",
            "-- see section 4 / 5 for the * values\nT ::= INTEGER\n",
        ] {
            assert!(is_inline_asn1(asn1), "{asn1}");
        }
    }

    #[test]
    fn smi_is_not_inline_asn1() {
        let text = "DisplayString ::= TEXTUAL-CONVENTION\n    STATUS       current\n    SYNTAX       OCTET STRING (SIZE (0..255))\n";
        assert!(is_smi(text));
        assert!(!is_inline_asn1(text));
        assert!(!is_smi("Cert ::= SEQUENCE { serial INTEGER }"));
    }

    #[test]
    fn other_notations_are_rejected() {
        assert!(!looks_like_asn1("<expr> ::= <term>"));
        assert!(!looks_like_asn1("int main() { return 0; }"));
        assert!(!looks_like_asn1("30 82 01 0a"));
        assert!(!looks_like_asn1("-- DEFINITIONS ::= only in a comment"));
    }
}
