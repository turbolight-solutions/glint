mod common;
use common::*;
use glint_syntax::{SyntaxKind, lex};

fn kinds(src: &str) -> Vec<SyntaxKind> {
    lex(src)
        .0
        .into_iter()
        .map(|t| t.kind)
        .filter(|k| !k.is_trivia())
        .collect()
}

#[test]
fn x680_12_1_6_white_space_includes_no_break_space_and_form_feed() {
    assert_clean(&module("A ::=\u{A0}INTEGER\u{0C}B ::= BOOLEAN"));
}

#[test]
fn x680_12_2_names_split_by_initial_letter_case() {
    assert_eq!(
        kinds("Abc abc A-b a-b-c1"),
        vec![
            SyntaxKind::UpperName,
            SyntaxKind::LowerName,
            SyntaxKind::UpperName,
            SyntaxKind::LowerName
        ]
    );
}

#[test]
fn x680_12_2_name_with_trailing_hyphen_is_e010() {
    assert_eq!(codes(&module("A- ::= INTEGER")), vec!["E010", "E001"]);
}

#[test]
fn x680_12_2_double_hyphen_starts_a_comment() {
    assert_clean(&module("A ::= INTEGER -- trailing comment\nB ::= BOOLEAN"));
    assert_eq!(kinds("a--b"), vec![SyntaxKind::LowerName]);
}

#[test]
fn x681_7_4_field_references_have_two_kinds() {
    assert_eq!(
        kinds("&Type &value &"),
        vec![
            SyntaxKind::FieldUpper,
            SyntaxKind::FieldLower,
            SyntaxKind::Ampersand
        ]
    );
}

#[test]
fn x680_12_8_number_with_leading_zero_is_e010() {
    assert_eq!(codes(&module("a INTEGER ::= 007")), vec!["E010"]);
    assert_clean(&module("a INTEGER ::= 0"));
}

#[test]
fn x680_12_9_real_numbers() {
    assert_eq!(kinds("1.5 2E3 4.5e-2"), vec![SyntaxKind::RealNumber; 3]);
    assert_eq!(
        kinds("1..5"),
        vec![SyntaxKind::Number, SyntaxKind::DotDot, SyntaxKind::Number]
    );
}

#[test]
fn x680_12_10_bstring() {
    assert_clean(&module("a BIT STRING ::= '0101 1100'B"));
    assert_eq!(codes(&module("a BIT STRING ::= '0102'B")), vec!["E010"]);
}

#[test]
fn x680_12_12_hstring_is_upper_case_only() {
    assert_clean(&module("a OCTET STRING ::= '0A1F'H"));
    assert_eq!(codes(&module("a OCTET STRING ::= '0a1f'H")), vec!["E010"]);
    assert_eq!(codes(&module("a OCTET STRING ::= '0A1F'")), vec!["E010"]);
}

#[test]
fn x680_12_14_cstring_escapes_quote_and_spans_lines() {
    assert_clean(&module("a UTF8String ::= \"say \"\"hi\"\"\nand bye\""));
    assert_eq!(
        codes(&module("a UTF8String ::= \"unterminated")).first(),
        Some(&"E010")
    );
}

#[test]
fn x680_12_6_line_comment_ends_at_second_double_hyphen() {
    assert_clean(&module("A ::= -- comment -- INTEGER"));
    assert_clean(&module("-- a lone - hyphen --\nA ::= INTEGER"));
}

#[test]
fn x680_12_6_block_comments_nest() {
    assert_clean(&module(
        "A ::= /* outer /* inner -- */ still comment */ INTEGER",
    ));
    assert_clean(&module("/* -- not a line comment */ A ::= INTEGER"));
}

#[test]
fn x680_12_6_unterminated_block_comment_is_e010() {
    assert!(codes(&module("A ::= INTEGER /* never closed")).contains(&"E010"));
}

#[test]
fn x680_12_20_punctuation_tokens() {
    assert_eq!(
        kinds("::= ... .. [[ ]] [ ] { } ( ) < > , . - : = & @ | ! ^ ;"),
        vec![
            SyntaxKind::Assign,
            SyntaxKind::Ellipsis,
            SyntaxKind::DotDot,
            SyntaxKind::LDoubleBracket,
            SyntaxKind::RDoubleBracket,
            SyntaxKind::LBracket,
            SyntaxKind::RBracket,
            SyntaxKind::LBrace,
            SyntaxKind::RBrace,
            SyntaxKind::LParen,
            SyntaxKind::RParen,
            SyntaxKind::Lt,
            SyntaxKind::Gt,
            SyntaxKind::Comma,
            SyntaxKind::Dot,
            SyntaxKind::Minus,
            SyntaxKind::Colon,
            SyntaxKind::Equals,
            SyntaxKind::Ampersand,
            SyntaxKind::At,
            SyntaxKind::Pipe,
            SyntaxKind::Bang,
            SyntaxKind::Caret,
            SyntaxKind::Semicolon,
        ]
    );
}

#[test]
fn x682_10_7_at_notation_dots_are_ordinary_tokens() {
    assert_eq!(
        kinds("@...errorId"),
        vec![SyntaxKind::At, SyntaxKind::Ellipsis, SyntaxKind::LowerName]
    );
    assert_eq!(
        kinds("@.a"),
        vec![SyntaxKind::At, SyntaxKind::Dot, SyntaxKind::LowerName]
    );
    assert_eq!(
        kinds("@..a"),
        vec![SyntaxKind::At, SyntaxKind::DotDot, SyntaxKind::LowerName]
    );
}

#[test]
fn x680_12_38_reserved_word_as_a_name_is_e011() {
    assert_eq!(module_codes("SEQUENCE ::= INTEGER"), vec!["E011"]);
    assert_eq!(module_codes("OPTIONAL ::= INTEGER"), vec!["E011"]);
}

#[test]
fn x680_12_38_any_defined_and_macro_are_not_reserved() {
    assert!(!glint_syntax::is_reserved("ANY"));
    assert!(!glint_syntax::is_reserved("DEFINED"));
    assert!(!glint_syntax::is_reserved("MACRO"));
}

#[test]
fn unexpected_character_is_e010_and_does_not_stop_parsing() {
    let found = codes(&module("A ::= INTEGER # B ::= BOOLEAN"));
    assert_eq!(found, vec!["E010"]);
}
