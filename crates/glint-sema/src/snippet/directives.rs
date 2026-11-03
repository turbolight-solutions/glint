//! `-- glint:` directives and elision comments inside a fragment.

use glint_syntax::{SyntaxKind, SyntaxNode, TextRange};

use super::norm::PartialMarks;

/// What the comments of one fragment ask for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Directives {
    /// `-- glint: ignore`
    pub ignore: bool,
    /// `-- glint: partial`
    pub partial: bool,
    /// `-- glint: module=Name`
    pub module: Option<(String, TextRange)>,
    /// Directive words that are not known, with the comment they are in.
    pub unknown: Vec<(String, TextRange)>,
    /// Comments that match a configured elision marker.
    pub elisions: Vec<TextRange>,
}

fn comment_body(text: &str) -> &str {
    let body = text.strip_prefix("--").unwrap_or(text);
    let body = body.strip_suffix("--").unwrap_or(body);
    body.trim()
}

/// Reads the directives and elision comments of `root` (design spec, "Elision and directives").
pub(crate) fn scan(root: &SyntaxNode, elision_comments: &[String]) -> Directives {
    let mut found = Directives::default();
    for token in root
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::LineComment)
    {
        let range = token.text_range();
        let text = token.text();
        let body = comment_body(text);
        if let Some(rest) = body.strip_prefix("glint:") {
            for word in rest.split([' ', '\t', ',']).filter(|w| !w.is_empty()) {
                match word {
                    "ignore" => found.ignore = true,
                    "partial" => found.partial = true,
                    other => match other.strip_prefix("module=") {
                        Some(name) if !name.is_empty() => {
                            found.module = Some((name.to_owned(), range));
                        }
                        _ => found.unknown.push((other.to_owned(), range)),
                    },
                }
            }
        } else if elision_comments
            .iter()
            .any(|marker| !marker.is_empty() && text.trim().starts_with(marker.trim()))
        {
            found.elisions.push(range);
        }
    }
    found
}

const LISTS: [SyntaxKind; 6] = [
    SyntaxKind::SequenceType,
    SyntaxKind::SetType,
    SyntaxKind::ChoiceType,
    SyntaxKind::EnumeratedType,
    SyntaxKind::IntegerType,
    SyntaxKind::BitStringType,
];

/// The constructs that enclose an elision comment, and so are partial.
pub(crate) fn marks(root: &SyntaxNode, directives: &Directives) -> PartialMarks {
    let mut ranges = Vec::new();
    for comment in &directives.elisions {
        let enclosing = root
            .descendants()
            .filter(|n| LISTS.contains(&n.kind()))
            .filter(|n| braces_contain(n, comment))
            .min_by_key(|n| n.text_range().len());
        if let Some(node) = enclosing {
            ranges.push(crate::view::trimmed_range(&node));
        }
    }
    PartialMarks {
        all: directives.partial,
        ranges,
    }
}

fn braces_contain(node: &SyntaxNode, comment: &TextRange) -> bool {
    let open = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::LBrace);
    let close = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::RBrace)
        .last();
    match (open, close) {
        (Some(open), Some(close)) => {
            open.text_range().end() <= comment.start()
                && comment.end() <= close.text_range().start()
        }
        _ => false,
    }
}
