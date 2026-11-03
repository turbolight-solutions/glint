use crate::Severity;

/// Registry entry for a diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeInfo {
    pub code: &'static str,
    pub severity: Severity,
    pub title: &'static str,
    pub basis: &'static str,
}

const CODES: &[CodeInfo] = &[
    CodeInfo {
        code: "E101",
        severity: Severity::Error,
        title: "Imported symbol not defined in, or not exported by, the target module",
        basis: "13.13, 13.14, 13.16 a–b",
    },
    CodeInfo {
        code: "E104",
        severity: Severity::Error,
        title: "Same module named twice in IMPORTS (by name or OID), or a module importing itself",
        basis: "13.16 e",
    },
    CodeInfo {
        code: "E107",
        severity: Severity::Error,
        title: "Value reference in an AssignedIdentifier does not meet 13.9's rules",
        basis: "13.9",
    },
    CodeInfo {
        code: "E110",
        severity: Severity::Error,
        title: "Undefined reference, or external reference to a module absent from IMPORTS",
        basis: "14.4–14.7; X.681 8.3",
    },
    CodeInfo {
        code: "E111",
        severity: Severity::Error,
        title: "Two assignments with the same name in a module, parameterized or not",
        basis: "13.21; X.683 8.2",
    },
    CodeInfo {
        code: "E113",
        severity: Severity::Error,
        title: "Duplicate identifier among components (after COMPONENTS OF) or alternatives",
        basis: "25.14, 29.10",
    },
    CodeInfo {
        code: "E114",
        severity: Severity::Error,
        title: "Symbol imported from two modules, or also defined locally, used without Module.symbol",
        basis: "13.20",
    },
    CodeInfo {
        code: "E120",
        severity: Severity::Error,
        title: "Tag clash among CHOICE alternatives or SET components, counting every tag of an untagged CHOICE, the conceptually added extension element, and the indeterminate tag of an untagged open type",
        basis: "27.3, 29.3, 29.9, 52.7; X.681 14.2 b",
    },
    CodeInfo {
        code: "E121",
        severity: Severity::Error,
        title: "Tag clash between a run of OPTIONAL/DEFAULT SEQUENCE components and the next component, across extension additions",
        basis: "25.6, 25.7, 52.7",
    },
    CodeInfo {
        code: "E122",
        severity: Severity::Error,
        title: "IMPLICIT on an untagged CHOICE, open type or DummyReference",
        basis: "31.2.9; X.681 14.2 c",
    },
    CodeInfo {
        code: "E123",
        severity: Severity::Error,
        title: "UNIVERSAL class tag in a user module",
        basis: "31.2.5",
    },
    CodeInfo {
        code: "E124",
        severity: Severity::Error,
        title: "Textually tagged extension additions under AUTOMATIC TAGS when the root is untagged",
        basis: "25.9, 29.4",
    },
    CodeInfo {
        code: "E125",
        severity: Severity::Error,
        title: "VersionNumber misuse: mixed with ungrouped additions, below 2, or not increasing",
        basis: "25.16",
    },
    CodeInfo {
        code: "E126",
        severity: Severity::Error,
        title: "COMPONENTS OF names a type that is not a SEQUENCE (or SET inside a set)",
        basis: "25.5, 27.2",
    },
    CodeInfo {
        code: "E130",
        severity: Severity::Error,
        title: "OID with fewer than two arcs, first arc above 2, or second arc 40 or more under arcs 0 and 1",
        basis: "X.660 (via 32.11)",
    },
    CodeInfo {
        code: "E131",
        severity: Severity::Error,
        title: "Value not of a type compatible with its governing type (DEFAULT, named number, defined value)",
        basis: "14.9, 25.12, Annex C",
    },
    CodeInfo {
        code: "E132",
        severity: Severity::Error,
        title: "Empty or inverted range, or constraint value outside the parent type or its extension root",
        basis: "49.7, 50.7–50.11",
    },
    CodeInfo {
        code: "E133",
        severity: Severity::Error,
        title: "Subtype element not applicable to the parent type, such as SIZE on INTEGER",
        basis: "51.1 Tables 11–12",
    },
    CodeInfo {
        code: "E134",
        severity: Severity::Error,
        title: "Bare NameForm OID arc that is not a registered X.660 name; write name(number)",
        basis: "32.7, 32.12",
    },
    CodeInfo {
        code: "E135",
        severity: Severity::Error,
        title: "Exception spec ! without an extension marker, DummyReference or user-defined constraint",
        basis: "49.6",
    },
    CodeInfo {
        code: "I103",
        severity: Severity::Info,
        title: "Import resolved; names the RFC chosen when several candidates exist",
        basis: "policy",
    },
    CodeInfo {
        code: "I141",
        severity: Severity::Info,
        title: "OID uses an author placeholder (TBD, TBD1, id-TBD)",
        basis: "policy",
    },
    CodeInfo {
        code: "W100",
        severity: Severity::Warning,
        title: "Imported module not found in draft, RFC corpus or ITU set; its symbols are treated as opaque",
        basis: "policy",
    },
    CodeInfo {
        code: "W102",
        severity: Severity::Warning,
        title: "Module matched by name, but the IMPORTS OID differs from the corpus module's; the OID is definitive",
        basis: "13.10, 13.11",
    },
    CodeInfo {
        code: "W106",
        severity: Severity::Warning,
        title: "IMPORTS gives no OID for a module that has a definitive OID",
        basis: "13.12",
    },
    CodeInfo {
        code: "W112",
        severity: Severity::Warning,
        title: "Unused import",
        basis: "policy",
    },
    CodeInfo {
        code: "W140",
        severity: Severity::Warning,
        title: "OID assigned in the draft is already assigned in the corpus to a different name",
        basis: "policy",
    },
    CodeInfo {
        code: "I105",
        severity: Severity::Info,
        title: "Using a curated version of a corpus module",
        basis: "policy",
    },
    CodeInfo {
        code: "E001",
        severity: Severity::Error,
        title: "Syntax error",
        basis: "policy",
    },
    CodeInfo {
        code: "E010",
        severity: Severity::Error,
        title: "Lexical error",
        basis: "X.680 12.2-12.3, 12.6, 12.8-12.14",
    },
    CodeInfo {
        code: "E011",
        severity: Severity::Error,
        title: "Reserved word used as a reference name",
        basis: "X.680 12.2.2, 12.38, 16.1, 16.6",
    },
    CodeInfo {
        code: "E108",
        severity: Severity::Error,
        title: "WITH SUCCESSORS or WITH DESCENDANTS without an assigned identifier",
        basis: "X.680 13.16 f",
    },
    CodeInfo {
        code: "I200",
        severity: Severity::Info,
        title: "ASN.1 block skipped",
        basis: "policy",
    },
    CodeInfo {
        code: "W136",
        severity: Severity::Warning,
        title: "Unknown encoding reference",
        basis: "X.680 31.3, 54, Annex E",
    },
    CodeInfo {
        code: "W137",
        severity: Severity::Warning,
        title: "XML value notation, kept as opaque text and not checked",
        basis: "X.680 16.2",
    },
    CodeInfo {
        code: "E150",
        severity: Severity::Error,
        title: "Bad field reference: unknown field in a FieldName chain, Class.&field on an object or object set field, or an information-from-objects combination Table 1 forbids",
        basis: "X.681 9.14, 14.5, 15.4–15.5, 15.11",
    },
    CodeInfo {
        code: "E151",
        severity: Severity::Error,
        title: "Component relation path does not resolve: unknown identifier, a path through a component that is not a SET, SEQUENCE or CHOICE, more dots than enclosing levels, or @ outside any SET or SEQUENCE",
        basis: "X.682 10.8–10.10",
    },
    CodeInfo {
        code: "E152",
        severity: Severity::Error,
        title: "Object does not match its class: wrong syntax form, mandatory field missing, field set twice, setting of the wrong kind, or an optional-group literal with no setting after it",
        basis: "X.681 10.11, 10.12 d, 11.4–11.7",
    },
    CodeInfo {
        code: "E153",
        severity: Severity::Error,
        title: "Class definition error: duplicate field names, UNIQUE with DEFAULT, variable-type field whose optionality conflicts with its type field, chain through a non-object link, or a self-referential chain with no optional link",
        basis: "X.681 9.6, 9.8, 9.10, 9.13, 9.15",
    },
    CodeInfo {
        code: "E154",
        severity: Severity::Error,
        title: "WITH SYNTAX error: field missing or repeated, reserved word from the 10.6 list used as a literal, optional group with no field, or optional groups that make a setting ambiguous",
        basis: "X.681 10.6, 10.9, 10.12 a–c",
    },
    CodeInfo {
        code: "E155",
        severity: Severity::Error,
        title: "Object or object set not of the governing class; a textually identical but separate CLASS definition does not count",
        basis: "X.681 8.2, 11.3, 12.10",
    },
    CodeInfo {
        code: "E156",
        severity: Severity::Error,
        title: "Two objects in one object set with the same UNIQUE field value (when both values are known)",
        basis: "X.681 9.7",
    },
    CodeInfo {
        code: "E157",
        severity: Severity::Error,
        title: "Recursive definition or instantiation of an object or object set",
        basis: "X.681 11.2, 12.2",
    },
    CodeInfo {
        code: "E158",
        severity: Severity::Error,
        title: "Empty object set where one is required: { } without ..., an empty set used directly in a table constraint, or information extracted from an all-empty column",
        basis: "X.681 12.3, 15.12, 15.13",
    },
    CodeInfo {
        code: "E159",
        severity: Severity::Error,
        title: "Open type (type field or variable-type field) used as the type of a value or value set field of a class",
        basis: "X.681 14.2 a, 14.4",
    },
    CodeInfo {
        code: "E160",
        severity: Severity::Error,
        title: "Actual parameter list mismatch: wrong number, order or form of actual parameters, a parameterized name used without actual parameters, or actual parameters given to a name that is not parameterized",
        basis: "X.683 3.4.2, 9.3, 9.6",
    },
    CodeInfo {
        code: "E161",
        severity: Severity::Error,
        title: "Parameter list error: value, value set, object or object set dummy without a governor; type or class dummy with one; governor of the wrong kind; dummy governor that itself has a governor",
        basis: "X.683 8.3",
    },
    CodeInfo {
        code: "E162",
        severity: Severity::Error,
        title: "Dummy reference used inconsistently with its form, its governor, or its other uses",
        basis: "X.683 8.5",
    },
    CodeInfo {
        code: "E163",
        severity: Severity::Error,
        title: "Dummy reference never used in its scope",
        basis: "X.683 8.6",
    },
    CodeInfo {
        code: "E164",
        severity: Severity::Error,
        title: "Illegal recursion: parameterized value, value set, object or object set referring to itself; recursive path whose actual parameters mix a dummy with other text; circular parameterized type, value set or class not broken by OPTIONAL or a non-circular CHOICE alternative",
        basis: "X.683 8.6–8.8",
    },
    CodeInfo {
        code: "E165",
        severity: Severity::Error,
        title: "Governor refers to another governed dummy, or depends on the dummy or the name being defined",
        basis: "X.683 8.9, 8.11",
    },
    CodeInfo {
        code: "E166",
        severity: Severity::Error,
        title: "Right-hand side of a parameterized assignment is only a dummy reference",
        basis: "X.683 8.10",
    },
    CodeInfo {
        code: "E167",
        severity: Severity::Error,
        title: "Actual value or value set not compatible with its governor, or governor admits values that are invalid where the dummy is used",
        basis: "X.683 8.12–8.13; X.680 C.6",
    },
    CodeInfo {
        code: "E168",
        severity: Severity::Error,
        title: "Parameter of a parameterized abstract syntax used other than (directly or via actual parameters) in a constraint",
        basis: "X.683 10.2",
    },
    CodeInfo {
        code: "W169",
        severity: Severity::Warning,
        title: "Variable constraint (depends on an abstract syntax parameter) without an exception spec",
        basis: "X.683 10.3–10.4 Note",
    },
    CodeInfo {
        code: "W170",
        severity: Severity::Warning,
        title: "1988 syntax (ANY) mixed with X.681 constructs in one module",
        basis: "policy",
    },
    CodeInfo {
        code: "W171",
        severity: Severity::Warning,
        title: "Reference style, such as an all-upper-case type name that reads as a class reference",
        basis: "policy (X.681 7.1)",
    },
    CodeInfo {
        code: "E172",
        severity: Severity::Error,
        title: "Draft module defines a new MACRO",
        basis: "policy (not in X.680)",
    },
    CodeInfo {
        code: "W173",
        severity: Severity::Warning,
        title: "Draft module uses imported macro notation; those assignments are not checked",
        basis: "policy",
    },
    CodeInfo {
        code: "E180",
        severity: Severity::Error,
        title: "Table constraint on a type that is not an object class field type or INSTANCE OF, or any constraint other than a simple table constraint on INSTANCE OF",
        basis: "X.682 10.1, A.2",
    },
    CodeInfo {
        code: "E181",
        severity: Severity::Error,
        title: "Constraining object set not of the class named in the constrained field type",
        basis: "X.682 10.4, 10.15",
    },
    CodeInfo {
        code: "E182",
        severity: Severity::Error,
        title: "Referenced component is not a value or value set field of the same class, constrained by the same object set as the referencing component",
        basis: "X.682 10.15",
    },
    CodeInfo {
        code: "E183",
        severity: Severity::Error,
        title: "Referenced component lies in a different alternative of a CHOICE that contains the referencing component",
        basis: "X.682 10.14",
    },
    CodeInfo {
        code: "E184",
        severity: Severity::Error,
        title: "Contents constraint (CONTAINING, ENCODED BY) on a type other than OCTET STRING or a BIT STRING without named bits, combined with any other constraint (directly or via a type reference), or with an ENCODED BY value that is not an OID",
        basis: "X.682 11.2–11.3",
    },
    CodeInfo {
        code: "E185",
        severity: Severity::Error,
        title: "CONSTRAINED BY parameter not of a permitted form: a value or object without its governor, or a governor of the wrong kind",
        basis: "X.682 9.3",
    },
    CodeInfo {
        code: "W201",
        severity: Severity::Warning,
        title: "Block typed as ASN.1 does not look like ASN.1",
        basis: "policy",
    },
    CodeInfo {
        code: "W202",
        severity: Severity::Warning,
        title: "ASN.1 not marked as sourcecode of type asn.1",
        basis: "policy",
    },
    CodeInfo {
        code: "W203",
        severity: Severity::Warning,
        title: "Non-standard type name for an ASN.1 block",
        basis: "policy",
    },
    CodeInfo {
        code: "E203",
        severity: Severity::Error,
        title: "Document is not well-formed XML",
        basis: "policy",
    },
    CodeInfo {
        code: "E300",
        severity: Severity::Error,
        title: "Snippet in the text differs from the module definition of the same name; the normal forms are not identical",
        basis: "X.680 C.3",
    },
    CodeInfo {
        code: "W301",
        severity: Severity::Warning,
        title: "Snippet defines a name that is not in any module of the draft or the corpus",
        basis: "policy",
    },
    CodeInfo {
        code: "W302",
        severity: Severity::Warning,
        title: "Snippet and module definition have the same normal form but are written differently",
        basis: "X.680 C.3",
    },
    CodeInfo {
        code: "I303",
        severity: Severity::Info,
        title: "Snippet name is defined by several modules; says which one the snippet matches",
        basis: "policy",
    },
    CodeInfo {
        code: "W304",
        severity: Severity::Warning,
        title: "Unknown glint directive, or a module= directive naming a module that cannot be found",
        basis: "policy",
    },
    CodeInfo {
        code: "W310",
        severity: Severity::Warning,
        title: "Module defines a name that the text neither shows nor mentions (reverse check, off by default)",
        basis: "policy",
    },
];

/// Every registered code.
pub fn all_codes() -> &'static [CodeInfo] {
    CODES
}

/// Looks up the registered default severity and description of `code`.
pub fn code_info(code: &str) -> Option<&'static CodeInfo> {
    CODES.iter().find(|info| info.code == code)
}
