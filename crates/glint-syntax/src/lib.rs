//! Lexer, parser and lossless concrete syntax tree for ASN.1 (ITU-T X.680-X.683).
//!
//! The parser accepts the full X.680 notation. Notation that depends on name resolution (value notation,
//! value sets, objects, object sets, defined syntax) is kept as neutral token ranges for later phases.

pub mod ast;
mod error;
mod lexer;
mod parser;
mod reserved;
mod syntax_kind;

pub use error::{Severity, SyntaxError};
pub use lexer::{Token, lex};
pub use parser::{
    Parse, ParseOptions, PrefixKind, PrefixParse, parse_assignment_list, parse_prefix,
    parse_source, parse_source_with,
};
pub use reserved::{is_all_upper, is_later_edition_word, is_reserved};
pub use rowan::{TextRange, TextSize};
pub use syntax_kind::{Asn1Language, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
