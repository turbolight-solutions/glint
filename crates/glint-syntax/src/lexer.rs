use logos::Logos;
use rowan::{TextRange, TextSize};

use crate::SyntaxError;
use crate::syntax_kind::SyntaxKind;

/// A lexed token with its byte range in the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub range: TextRange,
}

/// Splits `text` into tokens (trivia included) and reports lexical errors (`E010`).
pub fn lex(text: &str) -> (Vec<Token>, Vec<SyntaxError>) {
    let mut tokens = Vec::new();
    let mut errors = Vec::new();
    if u32::try_from(text.len()).is_err() {
        errors.push(SyntaxError::new(
            "E010",
            "input is larger than 4 GiB",
            TextRange::default(),
        ));
        return (tokens, errors);
    }

    let mut lexer = SyntaxKind::lexer(text);
    while let Some(result) = lexer.next() {
        let span = lexer.span();
        let Some(range) = to_range(span.start, span.end) else {
            break;
        };
        let slice = lexer.slice();
        let kind = match result {
            Ok(kind) => kind,
            Err(()) => {
                errors.push(unrecognized(slice, range));
                SyntaxKind::Error
            }
        };
        validate(kind, slice, range, &mut errors);
        tokens.push(Token { kind, range });
    }
    check_name_hyphens(&tokens, &mut errors);
    (tokens, errors)
}

fn to_range(start: usize, end: usize) -> Option<TextRange> {
    let start = TextSize::try_from(start).ok()?;
    let end = TextSize::try_from(end).ok()?;
    Some(TextRange::new(start, end))
}

fn unrecognized(slice: &str, range: TextRange) -> SyntaxError {
    if slice.starts_with("/*") {
        SyntaxError::new("E010", "unterminated comment", range)
    } else {
        SyntaxError::new(
            "E010",
            format!(
                "unexpected character `{}`",
                slice.chars().next().unwrap_or('?')
            ),
            range,
        )
    }
}

fn validate(kind: SyntaxKind, slice: &str, range: TextRange, errors: &mut Vec<SyntaxError>) {
    match kind {
        SyntaxKind::Number => {
            // X.680 12.8.2
            if slice.len() > 1 && slice.starts_with('0') {
                errors.push(SyntaxError::new(
                    "E010",
                    "a number may not have a leading zero",
                    range,
                ));
            }
        }
        SyntaxKind::RealNumber => {
            // X.680 12.9
            let integer_part = slice.split(['.', 'e', 'E']).next().unwrap_or_default();
            if integer_part.len() > 1 && integer_part.starts_with('0') {
                errors.push(SyntaxError::new(
                    "E010",
                    "a number may not have a leading zero",
                    range,
                ));
            }
        }
        SyntaxKind::QuotedString => validate_quoted_string(slice, range, errors),
        SyntaxKind::CString => {}
        SyntaxKind::Error if slice == "\"" => {
            errors.push(SyntaxError::new("E010", "unterminated string", range));
        }
        SyntaxKind::Error if slice == "'" => {
            errors.push(SyntaxError::new(
                "E010",
                "unterminated bstring or hstring",
                range,
            ));
        }
        _ => {}
    }
}

// X.680 12.10, 12.12
fn validate_quoted_string(slice: &str, range: TextRange, errors: &mut Vec<SyntaxError>) {
    let Some(body) = slice.strip_prefix('\'') else {
        return;
    };
    let Some((digits, suffix)) = body.rsplit_once('\'') else {
        return;
    };
    let allowed: fn(char) -> bool = match suffix {
        "B" => |c| matches!(c, '0' | '1'),
        "H" => |c| c.is_ascii_digit() || matches!(c, 'A'..='F'),
        _ => {
            errors.push(SyntaxError::new(
                "E010",
                "a quoted string must end with `B` (bstring) or `H` (hstring)",
                range,
            ));
            return;
        }
    };
    let bad = digits.chars().find(|c| !allowed(*c) && !is_whitespace(*c));
    if let Some(bad) = bad {
        let message = if suffix == "H" && bad.is_ascii_lowercase() {
            format!("hstring digits must be upper-case, found `{bad}`")
        } else {
            let kind = if suffix == "B" { "bstring" } else { "hstring" };
            format!("`{bad}` is not allowed in a {kind}")
        };
        errors.push(SyntaxError::new("E010", message, range));
    }
}

// X.680 12.1.6
fn is_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}')
}

// X.680 12.2.1: a name may not end with a hyphen
fn check_name_hyphens(tokens: &[Token], errors: &mut Vec<SyntaxError>) {
    for pair in tokens.windows(2) {
        let [name, minus] = pair else { continue };
        let is_name = matches!(
            name.kind,
            SyntaxKind::UpperName
                | SyntaxKind::LowerName
                | SyntaxKind::FieldUpper
                | SyntaxKind::FieldLower
        );
        if is_name && minus.kind == SyntaxKind::Minus && name.range.end() == minus.range.start() {
            errors.push(SyntaxError::new(
                "E010",
                "a name may not end with a hyphen",
                minus.range,
            ));
        }
    }
}
