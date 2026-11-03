use super::Parser;
use crate::error::SyntaxError;
use crate::syntax_kind::{Asn1Language, SyntaxKind};
use rowan::Language;

// X.680 Annex E, Table E.1
const KNOWN_ENCODING_REFERENCES: &[&str] = &["TAG", "XER", "PER"];

pub(super) fn is_known_encoding_reference(name: &str) -> bool {
    KNOWN_ENCODING_REFERENCES.contains(&name)
}

impl Parser<'_> {
    pub(super) fn parse_source_file(&mut self) {
        self.builder
            .start_node(Asn1Language::kind_to_raw(SyntaxKind::SourceFile));
        if self.has_module_header() {
            self.parse_module_list();
        } else {
            self.parse_module_body(false);
        }
        self.finish_root();
    }

    pub(super) fn parse_fragment(&mut self) {
        self.builder
            .start_node(Asn1Language::kind_to_raw(SyntaxKind::SourceFile));
        self.start_node(SyntaxKind::ModuleBody);
        self.parse_assignments(false);
        self.finish_node();
        self.finish_root();
    }

    fn has_module_header(&self) -> bool {
        (0..self.sig.len()).any(|offset| self.nth_kw(offset, "DEFINITIONS"))
    }

    fn at_module_header(&self) -> bool {
        if !self.at(SyntaxKind::UpperName) {
            return false;
        }
        let mut offset = 1;
        if self.nth_is(offset, SyntaxKind::LBrace) {
            match self.skip_balanced(offset) {
                Some(next) => offset = next,
                None => return false,
            }
        }
        if self.nth_is(offset, SyntaxKind::CString) {
            offset = offset.saturating_add(1);
        }
        self.nth_kw(offset, "DEFINITIONS")
    }

    fn parse_module_list(&mut self) {
        while !self.eof() {
            if self.at_module_header() {
                self.parse_module();
            } else {
                self.error_expected("a module definition");
                self.start_node(SyntaxKind::ErrorNode);
                self.bump();
                while !self.eof() && !self.at_module_header() {
                    self.bump();
                }
                self.finish_node();
            }
        }
    }

    // X.680 13.1
    fn parse_module(&mut self) {
        self.start_node(SyntaxKind::ModuleDefinition);
        self.parse_module_identifier();
        self.expect_kw("DEFINITIONS");
        self.parse_module_defaults();
        self.expect(SyntaxKind::Assign, "`::=`");
        self.expect_kw("BEGIN");
        self.parse_module_body(true);
        while self.at_kw("ENCODING-CONTROL") {
            self.parse_encoding_control_section();
        }
        self.expect_kw("END");
        self.finish_node();
    }

    // X.680 13.1: ModuleIdentifier, DefinitiveIdentification
    fn parse_module_identifier(&mut self) {
        self.start_node(SyntaxKind::ModuleIdentifier);
        self.bump_defined_name();
        if self.at(SyntaxKind::LBrace) {
            self.parse_brace_group();
        }
        if self.at(SyntaxKind::CString) {
            self.bump();
        }
        self.finish_node();
    }

    // X.680 13.1: EncodingReferenceDefault, TagDefault, ExtensionDefault
    fn parse_module_defaults(&mut self) {
        if self.at(SyntaxKind::UpperName) && self.nth_kw(1, "INSTRUCTIONS") {
            self.start_node(SyntaxKind::EncodingReferenceDefault);
            self.bump();
            self.bump_kw();
            self.finish_node();
        }
        if self.at_any_kw(&["EXPLICIT", "IMPLICIT", "AUTOMATIC"]) {
            self.start_node(SyntaxKind::TagDefault);
            self.bump_kw();
            self.expect_kw("TAGS");
            self.finish_node();
        }
        if self.at_kw("EXTENSIBILITY") {
            self.start_node(SyntaxKind::ExtensionDefault);
            self.bump_kw();
            self.expect_kw("IMPLIED");
            self.finish_node();
        }
    }

    // X.680 clause 54
    fn parse_encoding_control_section(&mut self) {
        self.start_node(SyntaxKind::EncodingControlSection);
        self.bump_kw();
        if self.at(SyntaxKind::UpperName) && !self.at_kw("END") {
            let name = self.nth_text(0);
            if !is_known_encoding_reference(name) {
                let range = self.current_range();
                self.errors.push(SyntaxError::new(
                    "W136",
                    format!("unknown encoding reference `{name}`; the section is ignored"),
                    range,
                ));
            }
            self.bump();
        }
        while !self.eof() && !self.at_kw("END") && !self.at_kw("ENCODING-CONTROL") {
            self.bump();
        }
        self.finish_node();
    }

    // X.680 13.1: ModuleBody
    fn parse_module_body(&mut self, in_module: bool) {
        self.start_node(SyntaxKind::ModuleBody);
        if self.at_kw("EXPORTS") {
            self.parse_exports();
        }
        if self.at_kw("IMPORTS") {
            self.parse_imports();
        }
        self.parse_assignments(in_module);
        self.finish_node();
    }

    // X.680 13.1: Exports
    fn parse_exports(&mut self) {
        self.start_node(SyntaxKind::Exports);
        self.bump_kw();
        if self.at_kw("ALL") {
            self.bump_kw();
        } else if !self.at(SyntaxKind::Semicolon) {
            self.parse_symbol_list();
        }
        if !self.expect(SyntaxKind::Semicolon, "`;`") {
            self.skip_until(&[SyntaxKind::Semicolon]);
            self.eat(SyntaxKind::Semicolon);
        }
        self.finish_node();
    }

    // X.680 13.1: Imports
    fn parse_imports(&mut self) {
        self.start_node(SyntaxKind::Imports);
        self.bump_kw();
        while !self.at(SyntaxKind::Semicolon) && !self.eof() {
            let before = self.pos;
            self.parse_symbols_from_module();
            if self.pos == before {
                break;
            }
        }
        if !self.expect(SyntaxKind::Semicolon, "`;`") {
            self.skip_until(&[SyntaxKind::Semicolon]);
            self.eat(SyntaxKind::Semicolon);
        }
        self.finish_node();
    }

    // X.680 13.1: SymbolsFromModule
    fn parse_symbols_from_module(&mut self) {
        self.start_node(SyntaxKind::SymbolsFromModule);
        self.parse_symbol_list();
        if self.expect_kw("FROM") {
            self.parse_global_module_reference();
        } else {
            self.skip_until(&[SyntaxKind::Semicolon]);
        }
        self.finish_node();
    }

    // X.680 13.1: SymbolList
    fn parse_symbol_list(&mut self) {
        self.start_node(SyntaxKind::SymbolList);
        loop {
            self.parse_symbol();
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        self.finish_node();
    }

    // X.680 13.1: Symbol, ParameterizedReference (X.683 9.1)
    fn parse_symbol(&mut self) {
        self.start_node(SyntaxKind::Symbol);
        if self.at(SyntaxKind::UpperName) || self.at(SyntaxKind::LowerName) {
            self.bump();
            if self.at(SyntaxKind::LBrace) && self.nth_is(1, SyntaxKind::RBrace) {
                self.bump();
                self.bump();
            }
        } else {
            self.error_expected("a symbol");
        }
        self.finish_node();
    }

    // X.680 13.1, 13.9, 13.16 f: GlobalModuleReference
    fn parse_global_module_reference(&mut self) {
        self.start_node(SyntaxKind::GlobalModuleReference);
        if self.at(SyntaxKind::UpperName) {
            self.bump();
        } else {
            self.error_expected("a module reference");
        }
        let has_identifier = self.at_assigned_identifier();
        if has_identifier {
            self.start_node(SyntaxKind::AssignedIdentifier);
            self.parse_assigned_identifier_body();
            self.finish_node();
        }
        if self.at_kw("WITH") && matches!(self.nth_text(1), "SUCCESSORS" | "DESCENDANTS") {
            if !has_identifier {
                let range = self.current_range();
                self.errors.push(SyntaxError::new(
                    "E108",
                    "`WITH SUCCESSORS` and `WITH DESCENDANTS` require an assigned identifier",
                    range,
                ));
            }
            self.bump_kw();
            self.bump_kw();
        }
        self.finish_node();
    }

    // X.680 13.9
    fn at_assigned_identifier(&self) -> bool {
        match self.cur() {
            Some(SyntaxKind::LBrace) => true,
            Some(SyntaxKind::LowerName) => {
                let begins_next_symbol_list = self.nth_is(1, SyntaxKind::Comma)
                    || self.nth_kw(1, "FROM")
                    || (self.nth_is(1, SyntaxKind::LBrace) && self.nth_is(2, SyntaxKind::RBrace));
                !begins_next_symbol_list
            }
            Some(SyntaxKind::UpperName) => {
                self.nth_is(1, SyntaxKind::Dot) && self.nth_is(2, SyntaxKind::LowerName)
            }
            _ => false,
        }
    }

    fn parse_assigned_identifier_body(&mut self) {
        match self.cur() {
            Some(SyntaxKind::LBrace) => self.parse_brace_group(),
            Some(SyntaxKind::UpperName) => {
                self.bump();
                self.bump();
                self.bump();
            }
            _ => self.bump(),
        }
    }

    // X.680 13.1: AssignmentList
    pub(super) fn parse_assignments(&mut self, in_module: bool) {
        loop {
            if self.eof() {
                break;
            }
            if in_module && (self.at_kw("END") || self.at_kw("ENCODING-CONTROL")) {
                break;
            }
            let before = self.pos;
            self.parse_assignment();
            if self.pos == before {
                self.skip_unexpected();
                continue;
            }
            if self.eof() || (in_module && (self.at_kw("END") || self.at_kw("ENCODING-CONTROL"))) {
                continue;
            }
            if !self.looks_like_assignment_start(0) {
                self.error_expected("an assignment");
                self.skip_until(&[]);
                if self.pos == before {
                    self.skip_unexpected();
                }
            }
        }
    }

    fn skip_unexpected(&mut self) {
        if self.eof() {
            return;
        }
        self.error_expected("an assignment");
        self.start_node(SyntaxKind::ErrorNode);
        self.bump();
        self.finish_node();
        self.skip_until(&[]);
    }

    // X.680 16.1, 16.2, 16.6; X.681 9.1, 11.1, 12.1; X.683 8.1
    fn parse_assignment(&mut self) {
        match self.cur() {
            Some(SyntaxKind::UpperName) => self.parse_upper_assignment(),
            Some(SyntaxKind::LowerName) => self.parse_lower_assignment(),
            _ => self.error_expected("an assignment"),
        }
    }

    fn parse_upper_assignment(&mut self) {
        if self.nth_text(1) == "MACRO" && self.nth_is(2, SyntaxKind::Assign) {
            self.parse_macro_definition();
        } else if self.nth_is(1, SyntaxKind::Assign) {
            self.start_node(SyntaxKind::TypeAssignment);
            self.bump_defined_name();
            self.bump();
            self.parse_type_or_class();
            self.finish_node();
        } else if self.nth_is(1, SyntaxKind::LBrace) {
            let after_parameters = self.skip_balanced(1);
            let plain_type = after_parameters.is_some_and(|n| self.nth_is(n, SyntaxKind::Assign));
            if plain_type {
                self.start_node(SyntaxKind::TypeAssignment);
                self.bump_defined_name();
                self.parse_parameter_list();
                self.bump();
                self.parse_type_or_class();
            } else {
                self.start_node(SyntaxKind::ValueSetAssignment);
                self.bump_defined_name();
                self.parse_parameter_list();
                self.parse_value_set_rest();
            }
            self.finish_node();
        } else {
            self.start_node(SyntaxKind::ValueSetAssignment);
            self.bump_defined_name();
            self.parse_value_set_rest();
            self.finish_node();
        }
    }

    // X.680 16.6: governor Type "::=" ValueSet
    fn parse_value_set_rest(&mut self) {
        self.parse_type_or_class();
        if self.expect(SyntaxKind::Assign, "`::=`") {
            self.start_node(SyntaxKind::AmbiguousRhs);
            if self.at(SyntaxKind::LBrace) {
                self.parse_brace_group();
            } else {
                self.error_expected("`{`");
            }
            self.finish_node();
        }
    }

    fn parse_lower_assignment(&mut self) {
        if self.at_macro_use() {
            self.parse_macro_use();
            return;
        }
        self.start_node(SyntaxKind::ValueAssignment);
        self.bump_defined_name();
        if self.at(SyntaxKind::LBrace) {
            self.parse_parameter_list();
        }
        let governor_may_be_class = self.at(SyntaxKind::UpperName) && !self.at_builtin_type_start();
        self.parse_type_or_class();
        if self.expect(SyntaxKind::Assign, "`::=`") {
            if governor_may_be_class && self.at(SyntaxKind::LBrace) {
                self.start_node(SyntaxKind::AmbiguousRhs);
                self.parse_brace_group();
                self.finish_node();
            } else if self.at(SyntaxKind::Lt) {
                self.parse_xml_value();
            } else {
                self.parse_value();
            }
        }
        self.finish_node();
    }

    // X.680 16.2 XMLValueAssignment: kept opaque, not checked in v1
    fn parse_xml_value(&mut self) {
        let range = self.current_range();
        self.errors.push(SyntaxError::new(
            "W137",
            "XML value notation is not checked",
            range,
        ));
        self.start_node(SyntaxKind::XmlValue);
        let mut open_elements = 0usize;
        while self.at(SyntaxKind::Lt) {
            let closing = self.nth_is(1, SyntaxKind::Slash);
            let mut self_closing = false;
            while let Some(kind) = self.cur() {
                if self.at_sync_point(0) {
                    break;
                }
                self_closing = kind == SyntaxKind::Slash && self.nth_is(1, SyntaxKind::Gt);
                self.bump();
                if kind == SyntaxKind::Gt {
                    break;
                }
            }
            if closing || self_closing {
                open_elements = open_elements.saturating_sub(1);
            } else {
                open_elements = open_elements.saturating_add(1);
            }
            if open_elements == 0 {
                break;
            }
            while !self.eof() && !self.at(SyntaxKind::Lt) && !self.at_sync_point(0) {
                self.bump();
            }
        }
        self.finish_node();
    }

    // X.680 Legacy: `name MACRONAME ... ::= value`
    fn at_macro_use(&self) -> bool {
        if !self.nth_is(1, SyntaxKind::UpperName) || self.nth_is(1, SyntaxKind::Assign) {
            return false;
        }
        let text = self.nth_text(1);
        if crate::parser::types::is_builtin_type_keyword(text)
            || (text == "ANY" && self.any_builtin)
        {
            return false;
        }
        !matches!(
            self.nth(2),
            None | Some(
                SyntaxKind::Assign
                    | SyntaxKind::Dot
                    | SyntaxKind::LBrace
                    | SyntaxKind::LParen
                    | SyntaxKind::Lt
            )
        )
    }

    fn parse_macro_definition(&mut self) {
        self.start_node(SyntaxKind::MacroDefinition);
        self.bump_defined_name();
        self.bump();
        self.bump();
        if !self.eat_kw("BEGIN") {
            self.error_expected("`BEGIN`");
        }
        while !self.eof() && !self.at_kw("END") {
            self.bump();
        }
        if !self.eat_kw("END") {
            self.error_expected("`END`");
        }
        self.finish_node();
    }

    fn parse_macro_use(&mut self) {
        self.start_node(SyntaxKind::MacroUse);
        self.bump_defined_name();
        self.bump();
        while let Some(kind) = self.cur() {
            if self.at_kw("END") || self.at_macro_boundary() {
                break;
            }
            match kind {
                SyntaxKind::LBrace => self.parse_brace_group(),
                SyntaxKind::Assign => {
                    self.bump();
                    self.parse_value();
                    break;
                }
                _ => self.bump(),
            }
        }
        self.finish_node();
    }

    fn at_macro_boundary(&self) -> bool {
        self.is_line_start(0)
            && matches!(
                self.cur(),
                Some(SyntaxKind::UpperName | SyntaxKind::LowerName)
            )
            && (self.nth_is(1, SyntaxKind::Assign)
                || (self.nth_text(1) == "MACRO" && self.nth_is(2, SyntaxKind::Assign)))
    }

    // X.683 8.1-8.3
    fn parse_parameter_list(&mut self) {
        self.start_node(SyntaxKind::ParameterList);
        self.bump();
        if !self.at(SyntaxKind::RBrace) {
            loop {
                self.parse_parameter();
                if !self.eat(SyntaxKind::Comma) {
                    break;
                }
            }
        }
        if !self.expect(SyntaxKind::RBrace, "`}`") {
            self.skip_until(&[SyntaxKind::RBrace, SyntaxKind::Assign]);
            self.eat(SyntaxKind::RBrace);
        }
        self.finish_node();
    }

    fn parse_parameter(&mut self) {
        self.start_node(SyntaxKind::Parameter);
        if self.colon_before_separator(0) {
            if self.at(SyntaxKind::LowerName) {
                self.bump();
            } else {
                self.parse_type_or_class();
            }
            self.expect(SyntaxKind::Colon, "`:`");
        }
        if self.at(SyntaxKind::UpperName) || self.at(SyntaxKind::LowerName) {
            self.bump();
        } else {
            self.error_expected("a parameter name");
        }
        self.finish_node();
    }

    // X.683 9.5
    pub(super) fn parse_actual_parameter_list(&mut self) {
        self.start_node(SyntaxKind::ActualParameterList);
        self.bump();
        if !self.at(SyntaxKind::RBrace) {
            loop {
                self.start_node(SyntaxKind::ActualParameter);
                while let Some(kind) = self.cur() {
                    if matches!(kind, SyntaxKind::Comma | SyntaxKind::RBrace)
                        || self.at_sync_point(0)
                    {
                        break;
                    }
                    if kind == SyntaxKind::LBrace {
                        self.parse_brace_group();
                    } else {
                        self.bump();
                    }
                }
                self.finish_node();
                if !self.eat(SyntaxKind::Comma) {
                    break;
                }
            }
        }
        self.expect(SyntaxKind::RBrace, "`}`");
        self.finish_node();
    }

    // X.681 9.2
    fn parse_type_or_class(&mut self) {
        if self.at_kw("CLASS") {
            self.parse_object_class_def();
        } else {
            self.parse_type();
        }
    }

    // X.681 9.3 ObjectClassDefn
    pub(super) fn parse_object_class_def(&mut self) {
        self.start_node(SyntaxKind::ObjectClassDef);
        self.bump_kw();
        if self.at(SyntaxKind::LBrace) {
            self.parse_class_field_list();
        } else {
            self.error_expected("`{`");
        }
        if self.at_kw("WITH") && self.nth_kw(1, "SYNTAX") {
            self.bump_kw();
            self.bump_kw();
            if self.at(SyntaxKind::LBrace) {
                self.parse_brace_group();
            } else {
                self.error_expected("`{`");
            }
        }
        self.finish_node();
    }

    // X.681 9.3: "{" FieldSpec "," + "}"
    fn parse_class_field_list(&mut self) {
        if !self.enter() {
            self.bump();
            return;
        }
        self.start_node(SyntaxKind::ClassFieldList);
        self.bump();
        loop {
            self.parse_field_spec();
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        if !self.eat(SyntaxKind::RBrace) {
            self.error_expected("`}`");
            self.skip_until(&[SyntaxKind::RBrace]);
            self.eat(SyntaxKind::RBrace);
        }
        self.finish_node();
        self.leave();
    }

    // X.681 9.4-9.12: the seven field kinds share one shape; the kind is decided once the governor is resolved
    fn parse_field_spec(&mut self) {
        self.start_node(SyntaxKind::FieldSpec);
        let mut has_governor = false;
        if matches!(
            self.cur(),
            Some(SyntaxKind::FieldUpper | SyntaxKind::FieldLower)
        ) {
            self.bump();
            if matches!(
                self.cur(),
                Some(SyntaxKind::FieldUpper | SyntaxKind::FieldLower)
            ) {
                self.parse_field_name();
                has_governor = true;
            } else if !self.at_any_kw(&["OPTIONAL", "DEFAULT", "UNIQUE"])
                && !matches!(
                    self.cur(),
                    Some(SyntaxKind::Comma | SyntaxKind::RBrace) | None
                )
            {
                has_governor = true;
                if self.at_kw("CLASS") {
                    self.parse_object_class_def();
                } else {
                    self.parse_type();
                }
            }
        } else {
            self.error_expected("a field reference");
            self.skip_until(&[SyntaxKind::Comma, SyntaxKind::RBrace]);
        }
        if self.at_kw("UNIQUE") {
            self.bump_kw();
        }
        if self.at_kw("OPTIONAL") {
            self.bump_kw();
        } else if self.at_kw("DEFAULT") {
            self.bump_kw();
            if self.at(SyntaxKind::LBrace) {
                self.parse_brace_group();
            } else if has_governor {
                self.parse_value();
            } else {
                self.parse_type();
            }
        }
        self.finish_node();
    }
}
