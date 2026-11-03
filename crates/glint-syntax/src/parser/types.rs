use super::Parser;
use crate::error::SyntaxError;
use crate::syntax_kind::SyntaxKind;

const SIMPLE_BUILTIN_TYPES: &[&str] = &[
    "BOOLEAN",
    "NULL",
    "REAL",
    "EXTERNAL",
    "RELATIVE-OID",
    "OID-IRI",
    "RELATIVE-OID-IRI",
    "TIME",
    "DATE",
    "TIME-OF-DAY",
    "DATE-TIME",
    "DURATION",
    "BMPString",
    "GeneralString",
    "GraphicString",
    "IA5String",
    "ISO646String",
    "NumericString",
    "PrintableString",
    "TeletexString",
    "T61String",
    "UniversalString",
    "UTF8String",
    "VideotexString",
    "VisibleString",
    "GeneralizedTime",
    "UTCTime",
    "ObjectDescriptor",
];

const COMPOUND_BUILTIN_TYPES: &[&str] = &[
    "INTEGER",
    "BIT",
    "OCTET",
    "OBJECT",
    "EMBEDDED",
    "CHARACTER",
    "INSTANCE",
    "ENUMERATED",
    "SEQUENCE",
    "SET",
    "CHOICE",
    "CLASS",
];

const ENCODING_ONLY_NAMES: &[&str] = &["TYPE-IDENTIFIER", "ABSTRACT-SYNTAX"];

pub(super) fn is_builtin_type_keyword(text: &str) -> bool {
    SIMPLE_BUILTIN_TYPES.contains(&text) || COMPOUND_BUILTIN_TYPES.contains(&text)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComponentMode {
    Sequence,
    Set,
    Choice,
}

impl Parser<'_> {
    pub(super) fn at_builtin_type_start(&self) -> bool {
        self.at(SyntaxKind::UpperName)
            && !self.is_shadowed(self.nth_text(0))
            && (is_builtin_type_keyword(self.nth_text(0))
                || (self.any_builtin && self.nth_text(0) == "ANY"))
    }

    // X.680 18.1: Type, ConstrainedType (49.1)
    pub(super) fn parse_type(&mut self) -> bool {
        if !self.enter() {
            return false;
        }
        let checkpoint = self.checkpoint();
        let ok = self.parse_type_core();
        while ok && self.at(SyntaxKind::LParen) {
            self.start_node_at(checkpoint, SyntaxKind::ConstrainedType);
            self.parse_constraint();
            self.finish_node();
        }
        self.leave();
        ok
    }

    pub(super) fn parse_type_core(&mut self) -> bool {
        match self.cur() {
            Some(SyntaxKind::LBracket) => self.parse_prefixed_type(),
            Some(SyntaxKind::LowerName) => self.parse_lower_type(),
            Some(SyntaxKind::UpperName) => self.parse_upper_type(),
            _ => {
                self.error_expected("a type");
                false
            }
        }
    }

    // X.680 30.1 SelectionType; X.681 15.2 TypeFromObject
    fn parse_lower_type(&mut self) -> bool {
        if self.nth_is(1, SyntaxKind::Lt) {
            self.start_node(SyntaxKind::SelectionType);
            self.bump();
            self.bump();
            let ok = self.parse_type();
            self.finish_node();
            ok
        } else if self.nth_is(1, SyntaxKind::Dot)
            && matches!(
                self.nth(2),
                Some(SyntaxKind::FieldUpper | SyntaxKind::FieldLower)
            )
        {
            self.start_node(SyntaxKind::TypeFromObject);
            self.bump();
            self.bump();
            self.parse_field_name();
            self.finish_node();
            true
        } else {
            self.error_expected("a type");
            false
        }
    }

    fn parse_upper_type(&mut self) -> bool {
        let text = self.nth_text(0);
        if self.is_shadowed(text) {
            return self.parse_defined_type();
        }
        match text {
            "INTEGER" => {
                self.start_node(SyntaxKind::IntegerType);
                self.bump_kw();
                if self.at(SyntaxKind::LBrace) {
                    self.parse_named_number_list();
                }
                self.finish_node();
                true
            }
            "BIT" => {
                self.start_node(SyntaxKind::BitStringType);
                self.bump_kw();
                self.expect_kw("STRING");
                if self.at(SyntaxKind::LBrace) {
                    self.parse_named_number_list();
                }
                self.finish_node();
                true
            }
            "OCTET" => self.parse_two_word_type("STRING"),
            "OBJECT" => self.parse_two_word_type("IDENTIFIER"),
            "EMBEDDED" => self.parse_two_word_type("PDV"),
            "CHARACTER" => self.parse_two_word_type("STRING"),
            "INSTANCE" => {
                self.start_node(SyntaxKind::InstanceOfType);
                self.bump_kw();
                self.expect_kw("OF");
                let ok = self.at(SyntaxKind::UpperName) && self.parse_defined_type();
                if !ok {
                    self.error_expected("an object class reference");
                }
                self.finish_node();
                ok
            }
            "ENUMERATED" => self.parse_enumerated_type(),
            "SEQUENCE" => self.parse_sequence_or_set(true),
            "SET" => self.parse_sequence_or_set(false),
            "CHOICE" => {
                self.start_node(SyntaxKind::ChoiceType);
                self.bump_kw();
                let ok = self.parse_component_items(ComponentMode::Choice);
                self.finish_node();
                ok
            }
            "CLASS" => {
                self.parse_object_class_def();
                true
            }
            "ANY" if self.any_builtin => {
                self.start_node(SyntaxKind::AnyType);
                self.bump_kw();
                if self.at_kw("DEFINED") {
                    self.bump_kw();
                    self.expect_kw("BY");
                    self.expect(SyntaxKind::LowerName, "an identifier");
                }
                self.finish_node();
                true
            }
            _ if SIMPLE_BUILTIN_TYPES.contains(&text) => {
                self.start_node(SyntaxKind::BuiltinType);
                self.bump_kw();
                self.finish_node();
                true
            }
            _ if crate::reserved::is_reserved(text) && !ENCODING_ONLY_NAMES.contains(&text) => {
                self.error_expected("a type");
                false
            }
            _ => self.parse_defined_type(),
        }
    }

    fn parse_two_word_type(&mut self, second: &str) -> bool {
        self.start_node(SyntaxKind::BuiltinType);
        self.bump_kw();
        let ok = self.expect_kw(second);
        self.finish_node();
        ok
    }

    // X.680 17.1 DefinedType, X.681 14.1 ObjectClassFieldType, X.683 9.2 ParameterizedType
    fn parse_defined_type(&mut self) -> bool {
        let checkpoint = self.checkpoint();
        self.bump();
        let mut kind = SyntaxKind::DefinedType;
        if self.at(SyntaxKind::Dot) && self.nth_is(1, SyntaxKind::UpperName) {
            self.bump();
            self.bump();
        }
        if self.at(SyntaxKind::Dot)
            && matches!(
                self.nth(1),
                Some(SyntaxKind::FieldUpper | SyntaxKind::FieldLower)
            )
        {
            kind = SyntaxKind::ObjectClassFieldType;
            self.bump();
            self.parse_field_name();
        }
        self.start_node_at(checkpoint, kind);
        if kind == SyntaxKind::DefinedType && self.at(SyntaxKind::LBrace) {
            self.parse_actual_parameter_list();
        }
        self.finish_node();
        true
    }

    // X.681 9.14 FieldName
    pub(super) fn parse_field_name(&mut self) {
        self.start_node(SyntaxKind::FieldName);
        if matches!(
            self.cur(),
            Some(SyntaxKind::FieldUpper | SyntaxKind::FieldLower)
        ) {
            self.bump();
        } else {
            self.error_expected("a field reference");
        }
        while self.at(SyntaxKind::Dot)
            && matches!(
                self.nth(1),
                Some(SyntaxKind::FieldUpper | SyntaxKind::FieldLower)
            )
        {
            self.bump();
            self.bump();
        }
        self.finish_node();
    }

    // X.680 31.1 TaggedType, 31.3 EncodingPrefixedType
    fn parse_prefixed_type(&mut self) -> bool {
        let is_encoding_prefix = self.nth_is(1, SyntaxKind::UpperName)
            && self.nth_is(2, SyntaxKind::Colon)
            && self.nth_text(1) != "TAG"
            && crate::reserved::is_all_upper(self.nth_text(1));
        if is_encoding_prefix {
            self.start_node(SyntaxKind::EncodingPrefixedType);
            self.start_node(SyntaxKind::EncodingPrefix);
            let name = self.nth_text(1);
            if !super::module::is_known_encoding_reference(name) {
                let range = self.current_range();
                self.errors.push(SyntaxError::new(
                    "W136",
                    format!("unknown encoding reference `{name}`; the prefix is ignored"),
                    range,
                ));
            }
            let group_len = self
                .skip_balanced(0)
                .unwrap_or_else(|| self.sig.len().saturating_sub(self.pos));
            for _ in 0..group_len {
                self.bump();
            }
            self.finish_node();
            let ok = self.parse_type();
            self.finish_node();
            return ok;
        }
        self.start_node(SyntaxKind::TaggedType);
        self.parse_tag();
        if self.at_any_kw(&["IMPLICIT", "EXPLICIT"]) {
            self.bump_kw();
        }
        let ok = self.parse_type();
        self.finish_node();
        ok
    }

    // X.680 31.1 Tag
    fn parse_tag(&mut self) {
        self.start_node(SyntaxKind::Tag);
        self.bump();
        if self.nth_text(0) == "TAG" && self.nth_is(1, SyntaxKind::Colon) {
            self.bump();
            self.bump();
        }
        if self.at_any_kw(&["UNIVERSAL", "APPLICATION", "PRIVATE"]) {
            self.bump_kw();
        }
        match self.cur() {
            Some(SyntaxKind::Number | SyntaxKind::LowerName) => self.bump(),
            Some(SyntaxKind::UpperName)
                if self.nth_is(1, SyntaxKind::Dot) && self.nth_is(2, SyntaxKind::LowerName) =>
            {
                self.bump();
                self.bump();
                self.bump();
            }
            _ => self.error_expected("a class number"),
        }
        self.expect(SyntaxKind::RBracket, "`]`");
        self.finish_node();
    }

    // X.680 19.1 NamedNumberList, 22.1 NamedBitList
    fn parse_named_number_list(&mut self) {
        self.start_node(SyntaxKind::NamedNumberList);
        self.bump();
        loop {
            if self.at(SyntaxKind::LowerName) {
                self.start_node(SyntaxKind::NamedNumber);
                self.bump();
                if self.eat(SyntaxKind::LParen) {
                    self.parse_value();
                    self.expect(SyntaxKind::RParen, "`)`");
                } else {
                    self.error_expected("`(`");
                }
                self.finish_node();
            } else {
                self.error_expected("a named number");
                self.skip_until(&[SyntaxKind::Comma]);
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
        self.finish_node();
    }

    // X.680 20.1 EnumeratedType
    fn parse_enumerated_type(&mut self) -> bool {
        self.start_node(SyntaxKind::EnumeratedType);
        self.bump_kw();
        let mut ok = self.expect(SyntaxKind::LBrace, "`{`");
        if ok {
            loop {
                if self.at(SyntaxKind::Ellipsis) {
                    self.parse_extension_marker();
                } else if self.at(SyntaxKind::LowerName) {
                    self.start_node(SyntaxKind::Enumeration);
                    self.bump();
                    if self.eat(SyntaxKind::LParen) {
                        self.parse_value();
                        self.expect(SyntaxKind::RParen, "`)`");
                    }
                    self.finish_node();
                } else {
                    self.error_expected("an enumeration item");
                    self.skip_until(&[SyntaxKind::Comma]);
                }
                if !self.eat(SyntaxKind::Comma) {
                    break;
                }
            }
            ok = self.close_brace();
        }
        self.finish_node();
        ok
    }

    fn close_brace(&mut self) -> bool {
        if self.eat(SyntaxKind::RBrace) {
            return true;
        }
        self.error_expected("`}`");
        self.skip_until(&[SyntaxKind::RBrace]);
        self.eat(SyntaxKind::RBrace);
        false
    }

    // X.680 25.1 SequenceType, 26.1 SequenceOfType, 27.1 SetType, 28.1 SetOfType, 49.5 TypeWithConstraint
    fn parse_sequence_or_set(&mut self, is_sequence: bool) -> bool {
        if self.nth_is(1, SyntaxKind::LBrace) {
            let kind = if is_sequence {
                SyntaxKind::SequenceType
            } else {
                SyntaxKind::SetType
            };
            let mode = if is_sequence {
                ComponentMode::Sequence
            } else {
                ComponentMode::Set
            };
            self.start_node(kind);
            self.bump_kw();
            let ok = self.parse_component_items(mode);
            self.finish_node();
            return ok;
        }
        let kind = if is_sequence {
            SyntaxKind::SequenceOfType
        } else {
            SyntaxKind::SetOfType
        };
        self.start_node(kind);
        self.bump_kw();
        if self.at(SyntaxKind::LParen) {
            self.parse_constraint();
        } else if self.at_kw("SIZE") {
            self.parse_size_element();
        }
        let mut ok = self.expect_kw("OF");
        if ok {
            ok = self.parse_of_element();
        }
        self.finish_node();
        ok
    }

    fn parse_of_element(&mut self) -> bool {
        if self.at(SyntaxKind::LowerName)
            && !self.nth_is(1, SyntaxKind::Lt)
            && !self.nth_is(1, SyntaxKind::Dot)
        {
            self.start_node(SyntaxKind::NamedType);
            self.bump();
            let ok = self.parse_type();
            self.finish_node();
            ok
        } else {
            self.parse_type()
        }
    }

    fn parse_component_items(&mut self, mode: ComponentMode) -> bool {
        if !self.expect(SyntaxKind::LBrace, "`{`") {
            return false;
        }
        if self.at(SyntaxKind::RBrace) {
            self.bump();
            return true;
        }
        loop {
            if !self.parse_component_item(mode) {
                self.skip_until(&[SyntaxKind::Comma, SyntaxKind::RBrace]);
            }
            if self.eat(SyntaxKind::Comma) {
                if self.at(SyntaxKind::RBrace) {
                    self.error_expected("a component after `,`");
                    break;
                }
                continue;
            }
            break;
        }
        self.close_brace()
    }

    fn parse_component_item(&mut self, mode: ComponentMode) -> bool {
        match self.cur() {
            Some(SyntaxKind::Ellipsis) => {
                self.parse_extension_marker();
                true
            }
            Some(SyntaxKind::LDoubleBracket) => self.parse_extension_addition_group(mode),
            Some(SyntaxKind::UpperName)
                if self.at_kw("COMPONENTS") && mode != ComponentMode::Choice =>
            {
                self.start_node(SyntaxKind::ComponentsOf);
                self.bump_kw();
                self.expect_kw("OF");
                let ok = self.parse_type();
                self.finish_node();
                ok
            }
            Some(SyntaxKind::LowerName) => self.parse_component_type(mode),
            _ => {
                self.error_expected("a component");
                false
            }
        }
    }

    // X.680 25.1 ExtensionAndException
    fn parse_extension_marker(&mut self) {
        self.start_node(SyntaxKind::ExtensionMarker);
        self.bump();
        if self.at(SyntaxKind::Bang) {
            self.parse_exception_spec();
        }
        self.finish_node();
    }

    // X.680 53.1 ExceptionSpec
    pub(super) fn parse_exception_spec(&mut self) {
        self.start_node(SyntaxKind::ExceptionSpec);
        self.bump();
        self.parse_value();
        self.finish_node();
    }

    // X.680 25.1 ExtensionAdditionGroup
    fn parse_extension_addition_group(&mut self, mode: ComponentMode) -> bool {
        self.start_node(SyntaxKind::ExtensionAdditionGroup);
        self.bump();
        if self.at(SyntaxKind::Number) && self.nth_is(1, SyntaxKind::Colon) {
            self.start_node(SyntaxKind::VersionNumber);
            self.bump();
            self.bump();
            self.finish_node();
        }
        loop {
            if !self.at(SyntaxKind::LowerName) {
                self.error_expected("a component");
                self.skip_until(&[SyntaxKind::Comma, SyntaxKind::RDoubleBracket]);
            } else {
                self.parse_component_type(mode);
            }
            if !self.eat(SyntaxKind::Comma) {
                break;
            }
        }
        let ok = self.eat(SyntaxKind::RDoubleBracket);
        if !ok {
            self.error_expected("`]]`");
            self.skip_until(&[SyntaxKind::RDoubleBracket]);
            self.eat(SyntaxKind::RDoubleBracket);
        }
        self.finish_node();
        ok
    }

    // X.680 25.1 ComponentType, 29.1 NamedType
    fn parse_component_type(&mut self, mode: ComponentMode) -> bool {
        self.start_node(SyntaxKind::ComponentType);
        self.start_node(SyntaxKind::NamedType);
        self.bump();
        let ok = self.parse_type();
        self.finish_node();
        if ok && (self.at_kw("OPTIONAL") || self.at_kw("DEFAULT")) {
            if mode == ComponentMode::Choice {
                self.error(
                    "E001",
                    "`OPTIONAL` and `DEFAULT` are not allowed in a CHOICE",
                );
            }
            let is_default = self.at_kw("DEFAULT");
            self.bump_kw();
            if is_default {
                self.parse_value();
            }
        }
        self.finish_node();
        ok
    }
}
