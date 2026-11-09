//! Constructs of 1988 ASN.1 (X.208) that current ASN.1 rejects but that are correct in the modules written in it.

use glint_corpus::Oid;
use glint_syntax::{SyntaxError, SyntaxKind, SyntaxNode, TextRange, ast};

use crate::eval::Evaluator;
use crate::model::ModuleModel;

use crate::view::{TagClass, builtin_text, child, read_tag, tokens, type_child};

/// The character string types that X.680 has built in since ASN.1:1994 (`UTF8String` since an amendment to that
/// edition), with their universal tag numbers (X.680 8.4, Table 1).
///
/// X.208 (1988) had no such types, so modules that keep to 1988 syntax and need them define them by hand with the
/// tag the later edition assigned, as RFC 5280's `PKIX1Explicit88` does:
///
/// ```text
/// UniversalString ::= [UNIVERSAL 28] IMPLICIT OCTET STRING
/// ```
///
/// Under current ASN.1 the names are reserved words (12.38) that no type reference may be (12.2.2), and a user
/// module may not use universal class tags (31.2.5), so a compiler of current ASN.1 rejects the definition; the
/// built-in type is encoded alike.
pub const LEGACY_STRING_TYPES: [(&str, u128); 3] = [
    ("UniversalString", 28),
    ("BMPString", 30),
    ("UTF8String", 12),
];

/// The published 1988-syntax modules that define one of [`LEGACY_STRING_TYPES`] by hand, by module name and module
/// identifier, with the documents that publish them. In these the definition is correct for the syntax they keep
/// to, so Glint reports nothing on it; a new module has no reason to keep to 1988 syntax, so anywhere else the
/// definition is an error. The list is closed; it grows only if an older document that does this is found.
pub const LEGACY_STRING_MODULES: [(&str, &[u128], &str); 6] = [
    ("PKIX1Explicit88", &[1, 3, 6, 1, 5, 5, 7, 0, 1], "RFC 2459"),
    ("PKIXCMP", &[1, 3, 6, 1, 5, 5, 7, 0, 9], "RFC 2510"),
    (
        "ExtendedSecurityServices",
        &[1, 2, 840, 113549, 1, 9, 16, 0, 2],
        "RFC 2634",
    ),
    (
        "PKIX1Explicit88",
        &[1, 3, 6, 1, 5, 5, 7, 0, 18],
        "RFC 3280, RFC 5280",
    ),
    // As published, `dod(4)`, and as RFC 5272 erratum 2063 corrects it, `dod(6)`.
    (
        "EnrollmentMessageSyntax",
        &[1, 3, 4, 1, 5, 5, 7, 0, 23],
        "RFC 5272",
    ),
    (
        "EnrollmentMessageSyntax",
        &[1, 3, 6, 1, 5, 5, 7, 0, 23],
        "RFC 5272",
    ),
];

/// Whether the module of this name and identifier is one of [`LEGACY_STRING_MODULES`].
pub fn is_legacy_string_module(name: &str, oid: Option<&Oid>) -> bool {
    oid.is_some_and(|oid| {
        LEGACY_STRING_MODULES
            .iter()
            .any(|(module, arcs, _)| *module == name && oid.arcs() == *arcs)
    })
}

/// `errors` without the reserved-word findings (`E011`) on the names of [`LEGACY_STRING_TYPES`] defined by a module of
/// [`LEGACY_STRING_MODULES`] in `root`: the name is a reserved word only in a later edition than the module's.
pub fn without_legacy_name_errors(errors: &[SyntaxError], root: &SyntaxNode) -> Vec<SyntaxError> {
    let exempt = legacy_definition_names(root);
    errors
        .iter()
        .filter(|e| !(e.code == "E011" && exempt.contains(&e.range)))
        .cloned()
        .collect()
}

/// The ranges of the names that the listed modules in `root` define as one of [`LEGACY_STRING_TYPES`].
fn legacy_definition_names(root: &SyntaxNode) -> Vec<TextRange> {
    let Some(file) = ast::SourceFile::cast(root.clone()) else {
        return Vec::new();
    };
    let mut ranges = Vec::new();
    for module in file.modules() {
        let Some(model) = ModuleModel::build(&module) else {
            continue;
        };
        let oid = model.oid_group.as_ref().and_then(Evaluator::literal_oid);
        if !is_legacy_string_module(&model.name, oid.as_ref()) {
            continue;
        }
        for assignment in module.syntax().descendants() {
            if legacy_string_definition(&assignment).is_some()
                && let Some(name) = defined_name(&assignment)
            {
                ranges.push(name.text_range());
            }
        }
    }
    ranges
}

fn defined_name(assignment: &SyntaxNode) -> Option<glint_syntax::SyntaxToken> {
    tokens(assignment).into_iter().find(|t| t.kind().is_name())
}

/// The type `assignment` defines when it is a 1988 definition of one of [`LEGACY_STRING_TYPES`] with exactly that
/// type's universal tag: `Name ::= [UNIVERSAL n] IMPLICIT OCTET STRING`. Any other form (another tag, another type,
/// a constraint) is not one.
pub fn legacy_string_definition(assignment: &SyntaxNode) -> Option<&'static str> {
    if assignment.kind() != SyntaxKind::TypeAssignment {
        return None;
    }
    let name = defined_name(assignment)?;
    let &(defined, number) = LEGACY_STRING_TYPES
        .iter()
        .find(|(n, _)| *n == name.text())?;
    let tagged = type_child(assignment).filter(|t| t.kind() == SyntaxKind::TaggedType)?;
    let tag = read_tag(&child(&tagged, SyntaxKind::Tag)?);
    let implicit = tokens(&tagged)
        .iter()
        .any(|t| t.kind() == SyntaxKind::Keyword && t.text() == "IMPLICIT");
    let inner = type_child(&tagged)?;
    let octet_string =
        inner.kind() == SyntaxKind::BuiltinType && builtin_text(&inner) == "OCTET STRING";
    (tag.class == TagClass::Universal && tag.number == Some(number) && implicit && octet_string)
        .then_some(defined)
}
