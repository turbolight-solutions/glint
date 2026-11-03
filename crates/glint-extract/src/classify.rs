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

/// Classifies `text` by the number of module headers it contains.
pub fn block_kind(text: &str) -> BlockKind {
    let headers = significant(text)
        .iter()
        .filter(|(kind, word)| is_module_header_word(*kind, word))
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
    fn other_notations_are_rejected() {
        assert!(!looks_like_asn1("<expr> ::= <term>"));
        assert!(!looks_like_asn1("int main() { return 0; }"));
        assert!(!looks_like_asn1("30 82 01 0a"));
        assert!(!looks_like_asn1("-- DEFINITIONS ::= only in a comment"));
    }
}
