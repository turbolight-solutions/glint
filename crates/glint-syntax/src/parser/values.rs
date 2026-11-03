use super::Parser;
use crate::syntax_kind::SyntaxKind;

const VALUE_KEYWORDS: &[&str] = &[
    "TRUE",
    "FALSE",
    "NULL",
    "MIN",
    "MAX",
    "PLUS-INFINITY",
    "MINUS-INFINITY",
    "NOT-A-NUMBER",
];

impl Parser<'_> {
    // X.680 clauses 16-48: value notation is kept neutral; typed interpretation happens after name resolution
    pub(super) fn parse_value(&mut self) -> bool {
        if !self.enter() {
            return false;
        }
        self.start_node(SyntaxKind::Value);
        let ok = self.parse_value_body();
        self.finish_node();
        self.leave();
        ok
    }

    fn parse_value_body(&mut self) -> bool {
        match self.cur() {
            Some(SyntaxKind::LBrace) => {
                self.parse_brace_group();
                true
            }
            Some(SyntaxKind::Minus) => {
                self.bump();
                if matches!(
                    self.cur(),
                    Some(SyntaxKind::Number | SyntaxKind::RealNumber)
                ) {
                    self.bump();
                    true
                } else {
                    self.error_expected("a number");
                    false
                }
            }
            Some(
                SyntaxKind::Number
                | SyntaxKind::RealNumber
                | SyntaxKind::QuotedString
                | SyntaxKind::CString,
            ) => {
                self.bump();
                true
            }
            Some(SyntaxKind::LowerName) => self.parse_lower_value(),
            Some(SyntaxKind::UpperName) => self.parse_upper_value(),
            _ => {
                self.error_expected("a value");
                false
            }
        }
    }

    // X.680 14.6 ExternalValueReference, 16.5 ChoiceValue, X.681 15.3 ValueFromObject, X.683 9.2
    fn parse_lower_value(&mut self) -> bool {
        self.bump();
        if self.at(SyntaxKind::Colon) {
            self.bump();
            return self.parse_value();
        }
        if self.at(SyntaxKind::Dot)
            && matches!(
                self.nth(1),
                Some(SyntaxKind::FieldUpper | SyntaxKind::FieldLower)
            )
        {
            self.bump();
            self.parse_field_name();
        } else if self.at(SyntaxKind::LBrace) && !self.is_line_start(0) {
            self.parse_actual_parameter_list();
        }
        true
    }

    fn parse_upper_value(&mut self) -> bool {
        let text = self.nth_text(0);
        if VALUE_KEYWORDS.contains(&text) && !self.nth_is(1, SyntaxKind::Colon) {
            self.bump_kw();
            return true;
        }
        if self.nth_is(1, SyntaxKind::Dot) && self.nth_is(2, SyntaxKind::LowerName) {
            self.bump();
            self.bump();
            self.bump();
            if self.at(SyntaxKind::LBrace) && !self.is_line_start(0) {
                self.parse_actual_parameter_list();
            }
            return true;
        }
        if crate::reserved::is_reserved(text) && !self.at_builtin_type_start() {
            self.error_expected("a value");
            return false;
        }
        if !self.parse_type() {
            return false;
        }
        if self.expect(SyntaxKind::Colon, "`:`") {
            return self.parse_value();
        }
        false
    }

    /// A balanced `{ ... }` kept as a neutral token range; interpreted once the governing type or class is known.
    pub(super) fn parse_brace_group(&mut self) {
        if !self.enter() {
            self.bump();
            return;
        }
        self.start_node(SyntaxKind::BraceGroup);
        self.bump();
        loop {
            match self.cur() {
                Some(SyntaxKind::RBrace) => {
                    self.bump();
                    break;
                }
                Some(SyntaxKind::LBrace) => self.parse_brace_group(),
                None => {
                    self.error("E001", "unclosed `{`");
                    break;
                }
                Some(_) if self.at_sync_point(0) => {
                    self.error("E001", "unclosed `{`");
                    break;
                }
                Some(_) => self.bump(),
            }
        }
        self.finish_node();
        self.leave();
    }
}
