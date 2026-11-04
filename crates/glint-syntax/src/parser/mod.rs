mod constraints;
mod module;
mod types;
mod values;

use rowan::{Checkpoint, GreenNode, GreenNodeBuilder, Language, TextRange, TextSize};

use crate::error::{Severity, SyntaxError};
use crate::lexer::{Token, lex};
use crate::reserved::is_reserved;
use crate::syntax_kind::{Asn1Language, SyntaxKind, SyntaxNode};

const MAX_DEPTH: u32 = 128;

/// The result of parsing: a lossless tree plus every lexical and syntax error found.
#[derive(Debug, Clone)]
pub struct Parse {
    green: GreenNode,
    errors: Vec<SyntaxError>,
}

impl Parse {
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    pub fn errors(&self) -> &[SyntaxError] {
        &self.errors
    }
}

/// Switches that adapt the parser to the kind of source being read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseOptions {
    /// Old modules may define names that later editions reserve (such as `UTF8String`); report `E011` as a
    /// warning for them and let the module's own definition shadow the built-in type.
    pub legacy_names: bool,
}

impl ParseOptions {
    /// Options for modules that were published before the current edition (the RFC corpus).
    pub fn corpus() -> Self {
        Self { legacy_names: true }
    }
}

/// Parses a document that holds one or more modules, or a bare module body when no module header is present.
pub fn parse_source(text: &str) -> Parse {
    parse_source_with(text, ParseOptions::default())
}

/// Like [`parse_source`] with explicit [`ParseOptions`].
pub fn parse_source_with(text: &str, options: ParseOptions) -> Parse {
    run(text, options, |parser| parser.parse_source_file())
}

/// Parses an `AssignmentList` fragment (X.680 13.1 Note 2).
pub fn parse_assignment_list(text: &str) -> Parse {
    run(text, ParseOptions::default(), |parser| {
        parser.parse_fragment()
    })
}

/// What [`parse_prefix`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixKind {
    Type,
    Value,
}

/// The outcome of [`parse_prefix`].
#[derive(Debug, Clone)]
pub struct PrefixParse {
    pub parse: Parse,
    /// Offset just past the last token the construct consumed, in the coordinates of the original text.
    pub end: TextSize,
    pub ok: bool,
}

/// Parses one type or value that starts at byte `start` of `text` and may not extend past byte `limit`, and says
/// where it ended (X.681 11.7, X.683 9.5: a setting or an actual parameter is parsed once its kind is known).
///
/// Node and token ranges are in the coordinates of `text`.
pub fn parse_prefix(
    text: &str,
    start: usize,
    limit: usize,
    kind: PrefixKind,
    options: ParseOptions,
) -> PrefixParse {
    let slice = text.get(start..limit).unwrap_or_default();
    let mut padded = String::with_capacity(limit);
    padded.extend(std::iter::repeat_n(' ', start));
    padded.push_str(slice);
    let mut parser = Parser::new(&padded, options);
    parser
        .builder
        .start_node(Asn1Language::kind_to_raw(SyntaxKind::SourceFile));
    let ok = match kind {
        PrefixKind::Type => parser.parse_type(),
        PrefixKind::Value => parser.parse_value(),
    };
    let end = parser
        .pos
        .checked_sub(1)
        .and_then(|last| parser.token_at(last))
        .map_or_else(
            || TextSize::try_from(start).unwrap_or_default(),
            |token| token.range.end(),
        );
    parser.builder.finish_node();
    let parse = parser.finish();
    let ok = ok && parse.errors().iter().all(|e| e.severity != Severity::Error);
    PrefixParse { parse, end, ok }
}

fn run(text: &str, options: ParseOptions, entry: impl FnOnce(&mut Parser<'_>)) -> Parse {
    let mut parser = Parser::new(text, options);
    entry(&mut parser);
    parser.finish()
}

pub(crate) struct Parser<'t> {
    text: &'t str,
    tokens: Vec<Token>,
    sig: Vec<usize>,
    line_starts: Vec<bool>,
    pos: usize,
    raw: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<SyntaxError>,
    depth: u32,
    any_builtin: bool,
    options: ParseOptions,
    shadowed: Vec<&'t str>,
    last_error_at: Option<usize>,
}

impl<'t> Parser<'t> {
    fn new(text: &'t str, options: ParseOptions) -> Self {
        let (tokens, errors) = lex(text);
        let mut sig = Vec::new();
        let mut line_starts = Vec::new();
        let mut newline_seen = true;
        for (index, token) in tokens.iter().enumerate() {
            if token.kind == SyntaxKind::Whitespace {
                let slice = text
                    .get(usize::from(token.range.start())..usize::from(token.range.end()))
                    .unwrap_or_default();
                newline_seen |= slice.contains(['\n', '\r', '\u{0B}', '\u{0C}']);
            } else if !token.kind.is_trivia() {
                sig.push(index);
                line_starts.push(newline_seen);
                newline_seen = false;
            }
        }
        let mut parser = Self {
            text,
            tokens,
            sig,
            line_starts,
            pos: 0,
            raw: 0,
            builder: GreenNodeBuilder::new(),
            errors,
            depth: 0,
            any_builtin: true,
            options,
            shadowed: Vec::new(),
            last_error_at: None,
        };
        parser.any_builtin = parser.compute_any_builtin();
        parser.shadowed = parser.compute_shadowed();
        parser
    }

    fn finish_root(&mut self) {
        self.flush_trivia(self.tokens.len());
        self.finish_node();
    }

    fn finish(self) -> Parse {
        Parse {
            green: self.builder.finish(),
            errors: self.errors,
        }
    }

    fn compute_shadowed(&self) -> Vec<&'t str> {
        if !self.options.legacy_names {
            return Vec::new();
        }
        (0..self.sig.len())
            .filter(|&offset| {
                self.kind_at(offset) == Some(SyntaxKind::UpperName)
                    && self.kind_at(offset.saturating_add(1)) == Some(SyntaxKind::Assign)
                    && !offset
                        .checked_sub(1)
                        .is_some_and(|before| self.kind_at(before) == Some(SyntaxKind::LowerName))
            })
            .map(|offset| self.text_at(offset))
            .filter(|name| crate::reserved::is_later_edition_word(name))
            .collect()
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.shadowed.contains(&name)
    }

    // X.680 Annex E.1 / Legacy: `ANY` is a type only if the module neither defines nor imports it
    fn compute_any_builtin(&self) -> bool {
        let mut in_imports = false;
        for offset in 0..self.sig.len() {
            let text = self.text_at(offset);
            let kind = self.kind_at(offset);
            if kind == Some(SyntaxKind::UpperName) {
                match text {
                    "IMPORTS" => in_imports = true,
                    "ANY" if in_imports => return false,
                    "ANY" if self.kind_at(offset.saturating_add(1)) == Some(SyntaxKind::Assign) => {
                        return false;
                    }
                    _ => {}
                }
            } else if kind == Some(SyntaxKind::Semicolon) {
                in_imports = false;
            }
        }
        true
    }

    // ---- token access -------------------------------------------------------------------------

    fn token_at(&self, absolute: usize) -> Option<&Token> {
        self.sig.get(absolute).and_then(|&i| self.tokens.get(i))
    }

    fn kind_at(&self, absolute: usize) -> Option<SyntaxKind> {
        self.token_at(absolute).map(|t| t.kind)
    }

    fn text_at(&self, absolute: usize) -> &'t str {
        self.token_at(absolute)
            .and_then(|t| {
                self.text
                    .get(usize::from(t.range.start())..usize::from(t.range.end()))
            })
            .unwrap_or_default()
    }

    fn nth(&self, n: usize) -> Option<SyntaxKind> {
        self.kind_at(self.pos.saturating_add(n))
    }

    fn nth_text(&self, n: usize) -> &'t str {
        self.text_at(self.pos.saturating_add(n))
    }

    fn cur(&self) -> Option<SyntaxKind> {
        self.nth(0)
    }

    fn eof(&self) -> bool {
        self.cur().is_none()
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.cur() == Some(kind)
    }

    fn nth_is(&self, n: usize, kind: SyntaxKind) -> bool {
        self.nth(n) == Some(kind)
    }

    fn nth_kw(&self, n: usize, word: &str) -> bool {
        self.nth(n) == Some(SyntaxKind::UpperName) && self.nth_text(n) == word
    }

    fn at_kw(&self, word: &str) -> bool {
        self.nth_kw(0, word)
    }

    fn at_any_kw(&self, words: &[&str]) -> bool {
        self.at(SyntaxKind::UpperName) && words.contains(&self.nth_text(0))
    }

    fn is_line_start(&self, n: usize) -> bool {
        self.line_starts
            .get(self.pos.saturating_add(n))
            .copied()
            .unwrap_or(false)
    }

    // ---- tree building ------------------------------------------------------------------------

    fn flush_trivia(&mut self, until: usize) {
        while self.raw < until {
            let Some(token) = self.tokens.get(self.raw).copied() else {
                break;
            };
            self.emit(token.kind, token.range);
            self.raw = self.raw.saturating_add(1);
        }
    }

    fn emit(&mut self, kind: SyntaxKind, range: TextRange) {
        let text = self
            .text
            .get(usize::from(range.start())..usize::from(range.end()))
            .unwrap_or_default();
        self.builder.token(Asn1Language::kind_to_raw(kind), text);
    }

    fn next_sig_raw(&self) -> usize {
        self.sig.get(self.pos).copied().unwrap_or(self.tokens.len())
    }

    fn eat_trivia(&mut self) {
        let target = self.next_sig_raw();
        self.flush_trivia(target);
    }

    fn start_node(&mut self, kind: SyntaxKind) {
        self.eat_trivia();
        self.builder.start_node(Asn1Language::kind_to_raw(kind));
    }

    fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    fn checkpoint(&mut self) -> Checkpoint {
        self.eat_trivia();
        self.builder.checkpoint()
    }

    fn start_node_at(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) {
        self.builder
            .start_node_at(checkpoint, Asn1Language::kind_to_raw(kind));
    }

    fn bump(&mut self) {
        if let Some(kind) = self.cur() {
            self.bump_as(kind);
        }
    }

    fn bump_kw(&mut self) {
        self.bump_as(SyntaxKind::Keyword);
    }

    fn bump_as(&mut self, kind: SyntaxKind) {
        let Some(&raw_index) = self.sig.get(self.pos) else {
            return;
        };
        self.flush_trivia(raw_index);
        if let Some(token) = self.tokens.get(raw_index).copied() {
            self.emit(kind, token.range);
            self.raw = raw_index.saturating_add(1);
        }
        self.pos = self.pos.saturating_add(1);
    }

    fn eat(&mut self, kind: SyntaxKind) -> bool {
        let found = self.at(kind);
        if found {
            self.bump();
        }
        found
    }

    fn eat_kw(&mut self, word: &str) -> bool {
        let found = self.at_kw(word);
        if found {
            self.bump_kw();
        }
        found
    }

    // ---- errors -------------------------------------------------------------------------------

    fn current_range(&self) -> TextRange {
        match self.token_at(self.pos) {
            Some(token) => token.range,
            None => {
                let end = TextSize::try_from(self.text.len()).unwrap_or_default();
                TextRange::empty(end)
            }
        }
    }

    fn describe_current(&self) -> String {
        match self.cur() {
            Some(_) => format!("`{}`", self.nth_text(0)),
            None => "end of input".to_owned(),
        }
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>) {
        if self.last_error_at == Some(self.pos) {
            return;
        }
        self.last_error_at = Some(self.pos);
        let range = self.current_range();
        self.errors.push(SyntaxError::new(code, message, range));
    }

    fn error_expected(&mut self, what: &str) {
        let message = format!("expected {what}, found {}", self.describe_current());
        self.error("E001", message);
    }

    fn expect(&mut self, kind: SyntaxKind, what: &str) -> bool {
        if self.eat(kind) {
            return true;
        }
        self.error_expected(what);
        false
    }

    fn expect_kw(&mut self, word: &str) -> bool {
        if self.eat_kw(word) {
            return true;
        }
        self.error_expected(&format!("`{word}`"));
        false
    }

    // X.680 12.2.2
    fn bump_defined_name(&mut self) {
        let name = self.nth_text(0);
        if is_reserved(name) {
            let message = format!("`{name}` is a reserved word and cannot be used as a name");
            let mut error = SyntaxError::new("E011", message, self.current_range());
            if self.options.legacy_names && crate::reserved::is_later_edition_word(name) {
                error = error.with_severity(Severity::Warning);
            }
            self.errors.push(error);
        }
        self.bump();
    }

    fn enter(&mut self) -> bool {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_DEPTH {
            self.error("E001", "nesting is too deep");
            self.depth = self.depth.saturating_sub(1);
            return false;
        }
        true
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // ---- lookahead and recovery ---------------------------------------------------------------

    /// Offset just past the bracket group that opens at offset `n`, or `None` when it never closes.
    fn skip_balanced(&self, n: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut offset = n;
        loop {
            match self.nth(offset)? {
                SyntaxKind::LBrace | SyntaxKind::LParen | SyntaxKind::LBracket => {
                    depth = depth.saturating_add(1);
                }
                SyntaxKind::RBrace | SyntaxKind::RParen | SyntaxKind::RBracket => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(offset.saturating_add(1));
                    }
                }
                _ => {}
            }
            offset = offset.saturating_add(1);
        }
    }

    /// Whether a top-level `:` appears before the next top-level `,` or closing brace, starting at offset `n`.
    fn colon_before_separator(&self, n: usize) -> bool {
        let mut offset = n;
        loop {
            match self.nth(offset) {
                Some(SyntaxKind::Colon) => return true,
                Some(SyntaxKind::Comma | SyntaxKind::RBrace) | None => return false,
                Some(SyntaxKind::LBrace | SyntaxKind::LParen | SyntaxKind::LBracket) => {
                    match self.skip_balanced(offset) {
                        Some(next) => offset = next,
                        None => return false,
                    }
                }
                Some(_) => offset = offset.saturating_add(1),
            }
        }
    }

    // X.680 13.1: a line that starts a new assignment
    fn at_sync_point(&self, n: usize) -> bool {
        if !self.is_line_start(n) {
            return false;
        }
        match self.nth(n) {
            Some(SyntaxKind::UpperName | SyntaxKind::LowerName) => {}
            _ => return false,
        }
        let mut offset = n;
        let mut depth = 0usize;
        loop {
            let next = offset.saturating_add(1);
            match self.nth(next) {
                Some(SyntaxKind::Assign) if depth == 0 => return true,
                None => return false,
                Some(SyntaxKind::LBrace) => depth = depth.saturating_add(1),
                // A `}` that closes a group opened before this line: the line continues that group
                // (`version(1)} DEFINITIONS ::=`), it does not start an assignment.
                Some(SyntaxKind::RBrace) if depth == 0 => return false,
                Some(SyntaxKind::RBrace) => depth = depth.saturating_sub(1),
                Some(_) => {}
            }
            if self.is_line_start(next) {
                return false;
            }
            offset = next;
        }
    }

    /// Whether the tokens at offset `n` can plausibly begin an assignment.
    fn looks_like_assignment_start(&self, n: usize) -> bool {
        if !matches!(
            self.nth(n),
            Some(SyntaxKind::UpperName | SyntaxKind::LowerName)
        ) {
            return false;
        }
        let mut depth = 0usize;
        let mut offset = n.saturating_add(1);
        let limit = n.saturating_add(40);
        while offset < limit {
            match self.nth(offset) {
                Some(SyntaxKind::Assign) if depth == 0 => return true,
                Some(SyntaxKind::LBrace | SyntaxKind::LParen | SyntaxKind::LBracket) => {
                    depth = depth.saturating_add(1);
                }
                Some(SyntaxKind::RBrace | SyntaxKind::RParen | SyntaxKind::RBracket) => {
                    if depth == 0 {
                        return false;
                    }
                    depth = depth.saturating_sub(1);
                }
                Some(SyntaxKind::Comma | SyntaxKind::Semicolon) if depth == 0 => return false,
                Some(_) => {}
                None => return false,
            }
            offset = offset.saturating_add(1);
        }
        false
    }

    /// Wraps tokens in an error node until one of `stops` (or an unmatched closer, or a sync point) at nesting depth zero.
    fn skip_until(&mut self, stops: &[SyntaxKind]) {
        if self.eof() || self.at_recovery_stop(stops, 0) {
            return;
        }
        self.start_node(SyntaxKind::ErrorNode);
        let mut depth = 0usize;
        while let Some(kind) = self.cur() {
            if self.at_recovery_stop(stops, depth) {
                break;
            }
            match kind {
                SyntaxKind::LBrace
                | SyntaxKind::LParen
                | SyntaxKind::LBracket
                | SyntaxKind::LDoubleBracket => depth = depth.saturating_add(1),
                SyntaxKind::RBrace
                | SyntaxKind::RParen
                | SyntaxKind::RBracket
                | SyntaxKind::RDoubleBracket => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.bump();
        }
        self.finish_node();
    }

    fn at_recovery_stop(&self, stops: &[SyntaxKind], depth: usize) -> bool {
        let Some(kind) = self.cur() else { return true };
        if self.at_sync_point(0) || self.at_kw("END") {
            return true;
        }
        if depth > 0 {
            return false;
        }
        stops.contains(&kind)
            || matches!(
                kind,
                SyntaxKind::RBrace
                    | SyntaxKind::RParen
                    | SyntaxKind::RBracket
                    | SyntaxKind::RDoubleBracket
            )
    }
}
