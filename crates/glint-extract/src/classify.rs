use glint_syntax::{SyntaxKind, lex};

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
    fn other_notations_are_rejected() {
        assert!(!looks_like_asn1("<expr> ::= <term>"));
        assert!(!looks_like_asn1("int main() { return 0; }"));
        assert!(!looks_like_asn1("30 82 01 0a"));
        assert!(!looks_like_asn1("-- DEFINITIONS ::= only in a comment"));
    }
}
