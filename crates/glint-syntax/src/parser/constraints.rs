use super::Parser;
use crate::syntax_kind::SyntaxKind;

impl Parser<'_> {
    // X.680 49.6 Constraint
    pub(super) fn parse_constraint(&mut self) {
        if !self.enter() {
            self.bump();
            return;
        }
        self.start_node(SyntaxKind::Constraint);
        self.bump();
        self.parse_constraint_spec();
        if self.at(SyntaxKind::Bang) {
            self.parse_exception_spec();
        }
        if !self.eat(SyntaxKind::RParen) {
            self.error_expected("`)`");
            self.skip_until(&[SyntaxKind::RParen]);
            self.eat(SyntaxKind::RParen);
        }
        self.finish_node();
        self.leave();
    }

    // X.680 49.6 ConstraintSpec, GeneralConstraint (X.682 8.1)
    fn parse_constraint_spec(&mut self) {
        match self.cur() {
            Some(SyntaxKind::UpperName) if self.at_kw("CONSTRAINED") => {
                self.start_node(SyntaxKind::UserDefinedConstraint);
                self.bump_kw();
                self.expect_kw("BY");
                if self.at(SyntaxKind::LBrace) {
                    self.parse_brace_group();
                } else {
                    self.error_expected("`{`");
                }
                self.finish_node();
            }
            Some(SyntaxKind::UpperName) if self.at_any_kw(&["CONTAINING", "ENCODED"]) => {
                self.parse_contents_constraint();
            }
            Some(SyntaxKind::LBrace) => {
                self.start_node(SyntaxKind::TableConstraint);
                self.parse_brace_group();
                if self.at(SyntaxKind::LBrace) {
                    self.parse_brace_group();
                }
                self.finish_node();
            }
            _ => self.parse_element_set_specs(),
        }
    }

    // X.682 11.1
    fn parse_contents_constraint(&mut self) {
        self.start_node(SyntaxKind::ContentsConstraint);
        if self.at_kw("CONTAINING") {
            self.bump_kw();
            self.parse_type();
        }
        if self.at_kw("ENCODED") {
            self.bump_kw();
            self.expect_kw("BY");
            self.parse_value();
        }
        self.finish_node();
    }

    // X.680 50.1 ElementSetSpecs
    fn parse_element_set_specs(&mut self) {
        self.parse_element_set_spec();
        if self.at(SyntaxKind::Comma) && self.nth_is(1, SyntaxKind::Ellipsis) {
            self.bump();
            self.bump();
            if self.eat(SyntaxKind::Comma) {
                self.parse_element_set_spec();
            }
        }
    }

    // X.680 50.1 ElementSetSpec
    fn parse_element_set_spec(&mut self) {
        if self.at_kw("ALL") {
            self.start_node(SyntaxKind::ExclusionSet);
            self.bump_kw();
            if self.expect_kw("EXCEPT") {
                self.parse_elements();
            }
            self.finish_node();
        } else {
            self.parse_unions();
        }
    }

    fn parse_unions(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_intersections();
        if self.at(SyntaxKind::Pipe) || self.at_kw("UNION") {
            self.start_node_at(checkpoint, SyntaxKind::UnionSet);
            while self.at(SyntaxKind::Pipe) || self.at_kw("UNION") {
                self.bump();
                self.parse_intersections();
            }
            self.finish_node();
        }
    }

    fn parse_intersections(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_exclusion();
        if self.at(SyntaxKind::Caret) || self.at_kw("INTERSECTION") {
            self.start_node_at(checkpoint, SyntaxKind::IntersectionSet);
            while self.at(SyntaxKind::Caret) || self.at_kw("INTERSECTION") {
                self.bump();
                self.parse_exclusion();
            }
            self.finish_node();
        }
    }

    fn parse_exclusion(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_elements();
        if self.at_kw("EXCEPT") {
            self.start_node_at(checkpoint, SyntaxKind::ExclusionSet);
            self.bump_kw();
            self.parse_elements();
            self.finish_node();
        }
    }

    // X.680 50.1 Elements, 51.1 SubtypeElements
    fn parse_elements(&mut self) {
        if !self.enter() {
            return;
        }
        match self.cur() {
            Some(SyntaxKind::LParen) => {
                self.start_node(SyntaxKind::ParenElementSet);
                self.bump();
                self.parse_element_set_spec();
                self.expect(SyntaxKind::RParen, "`)`");
                self.finish_node();
            }
            Some(SyntaxKind::UpperName) => self.parse_upper_element(),
            Some(SyntaxKind::LBrace) => {
                self.start_node(SyntaxKind::ValueElement);
                self.parse_brace_group();
                self.finish_node();
            }
            _ => self.parse_value_or_range(),
        }
        self.leave();
    }

    fn parse_upper_element(&mut self) {
        match self.nth_text(0) {
            "SIZE" => self.parse_size_element(),
            "FROM" => {
                self.start_node(SyntaxKind::FromElement);
                self.bump_kw();
                self.parse_constraint_if_paren();
                self.finish_node();
            }
            "WITH" => self.parse_inner_type_element(),
            "INCLUDES" => {
                self.start_node(SyntaxKind::ContainedSubtype);
                self.bump_kw();
                self.parse_type();
                self.finish_node();
            }
            "PATTERN" => {
                self.start_node(SyntaxKind::PatternElement);
                self.bump_kw();
                self.parse_value();
                self.finish_node();
            }
            "SETTINGS" => {
                self.start_node(SyntaxKind::SettingsElement);
                self.bump_kw();
                self.expect(SyntaxKind::CString, "a quoted string");
                self.finish_node();
            }
            "TRUE" | "FALSE" | "NULL" | "MIN" | "MAX" | "PLUS-INFINITY" | "MINUS-INFINITY"
            | "NOT-A-NUMBER" => self.parse_value_or_range(),
            _ if self.nth_is(1, SyntaxKind::Dot) && self.nth_is(2, SyntaxKind::LowerName) => {
                self.parse_value_or_range();
            }
            _ => {
                self.start_node(SyntaxKind::TypeElement);
                self.parse_type_core();
                self.finish_node();
            }
        }
    }

    fn parse_constraint_if_paren(&mut self) {
        if self.at(SyntaxKind::LParen) {
            self.parse_constraint();
        } else {
            self.error_expected("`(`");
        }
    }

    // X.680 51.5 SizeConstraint
    pub(super) fn parse_size_element(&mut self) {
        self.start_node(SyntaxKind::SizeElement);
        self.bump_kw();
        self.parse_constraint_if_paren();
        self.finish_node();
    }

    // X.680 51.8 InnerTypeConstraints
    fn parse_inner_type_element(&mut self) {
        self.start_node(SyntaxKind::InnerTypeElement);
        self.bump_kw();
        if self.at_kw("COMPONENT") {
            self.bump_kw();
            self.parse_constraint_if_paren();
        } else if self.at_kw("COMPONENTS") {
            self.bump_kw();
            self.parse_multiple_type_constraints();
        } else {
            self.error_expected("`COMPONENT` or `COMPONENTS`");
        }
        self.finish_node();
    }

    fn parse_multiple_type_constraints(&mut self) {
        if !self.expect(SyntaxKind::LBrace, "`{`") {
            return;
        }
        loop {
            if self.at(SyntaxKind::Ellipsis) {
                self.bump();
            } else if self.at(SyntaxKind::LowerName) {
                self.start_node(SyntaxKind::NamedConstraint);
                self.bump();
                if self.at(SyntaxKind::LParen) {
                    self.parse_constraint();
                }
                if self.at_any_kw(&["PRESENT", "ABSENT", "OPTIONAL"]) {
                    self.bump_kw();
                }
                self.finish_node();
            } else {
                self.error_expected("a component constraint");
                self.skip_until(&[SyntaxKind::Comma, SyntaxKind::RBrace]);
            }
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        if !self.eat(SyntaxKind::RBrace) {
            self.error_expected("`}`");
            self.skip_until(&[SyntaxKind::RBrace]);
            self.eat(SyntaxKind::RBrace);
        }
    }

    // X.680 51.1 SingleValue, 51.7 ValueRange
    fn parse_value_or_range(&mut self) {
        let checkpoint = self.checkpoint();
        if !self.parse_value() {
            return;
        }
        let is_range = self.at(SyntaxKind::DotDot)
            || (self.at(SyntaxKind::Lt) && self.nth_is(1, SyntaxKind::DotDot));
        if is_range {
            self.start_node_at(checkpoint, SyntaxKind::RangeElement);
            self.eat(SyntaxKind::Lt);
            self.bump();
            self.eat(SyntaxKind::Lt);
            self.parse_value();
        } else {
            self.start_node_at(checkpoint, SyntaxKind::ValueElement);
        }
        self.finish_node();
    }
}
