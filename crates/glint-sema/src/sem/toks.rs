//! Flat token views over brace groups, for the notation whose grammar depends on a class or definition
//! (X.681 10-12, X.683 9.5): the parser keeps such bodies as token ranges.

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tk {
    pub kind: SyntaxKind,
    pub text: String,
    pub range: TextRange,
}

impl Tk {
    pub fn is(&self, kind: SyntaxKind) -> bool {
        self.kind == kind
    }

    pub fn is_word(&self, word: &str) -> bool {
        matches!(self.kind, SyntaxKind::UpperName | SyntaxKind::Keyword) && self.text == word
    }

    pub fn is_open(&self) -> bool {
        matches!(
            self.kind,
            SyntaxKind::LBrace | SyntaxKind::LParen | SyntaxKind::LBracket
        )
    }

    pub fn is_close(&self) -> bool {
        matches!(
            self.kind,
            SyntaxKind::RBrace | SyntaxKind::RParen | SyntaxKind::RBracket
        )
    }
}

/// Every significant token below `node`, in text order.
pub fn flatten(node: &SyntaxNode) -> Vec<Tk> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| Tk {
            kind: token.kind(),
            text: token.text().to_owned(),
            range: token.text_range(),
        })
        .collect()
}

/// The tokens between the outer braces of a `BraceGroup`.
pub fn inner(group: &SyntaxNode) -> Vec<Tk> {
    let mut all = flatten(group);
    if all.first().is_some_and(|t| t.is(SyntaxKind::LBrace)) {
        all.remove(0);
    }
    if all.last().is_some_and(|t| t.is(SyntaxKind::RBrace)) {
        all.pop();
    }
    all
}

/// Splits `tokens` at commas that are not nested in brackets.
pub fn split_commas(tokens: &[Tk]) -> Vec<&[Tk]> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if token.is_open() {
            depth = depth.saturating_add(1);
        } else if token.is_close() {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && token.is(SyntaxKind::Comma) {
            parts.push(tokens.get(start..index).unwrap_or_default());
            start = index.saturating_add(1);
        }
    }
    parts.push(tokens.get(start..).unwrap_or_default());
    parts
}

/// The range spanned by `tokens`.
pub fn span(tokens: &[Tk]) -> Option<TextRange> {
    let first = tokens.first()?;
    let last = tokens.last()?;
    Some(TextRange::new(first.range.start(), last.range.end()))
}

/// Index just past the bracket group that opens at `tokens[at]`.
pub fn skip_group(tokens: &[Tk], at: usize) -> usize {
    let mut depth = 0usize;
    for (offset, token) in tokens.iter().enumerate().skip(at) {
        if token.is_open() {
            depth = depth.saturating_add(1);
        } else if token.is_close() {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return offset.saturating_add(1);
            }
        }
    }
    tokens.len()
}
