mod common;
use common::*;
use glint_syntax::ast::{Assignment, SourceFile, TagDefault};

const SRC: &str = "Foo { 1 3 6 1 } DEFINITIONS AUTOMATIC TAGS EXTENSIBILITY IMPLIED ::= BEGIN
IMPORTS id-x, Bar FROM Other { 1 2 } Baz FROM Third;
A ::= INTEGER
b A ::= 1
V A ::= { 1 | 2 }
END";

#[test]
fn module_header_views() {
    let parse = assert_clean(SRC);
    let file = SourceFile::cast(parse.syntax()).unwrap();
    let module = file.modules().next().unwrap();
    assert_eq!(module.name().unwrap().text(), "Foo");
    assert_eq!(module.definitive_oid().as_deref(), Some("{ 1 3 6 1 }"));
    assert_eq!(module.tag_default(), Some(TagDefault::Automatic));
    assert!(module.extensibility_implied());
}

#[test]
fn import_views() {
    let parse = assert_clean(SRC);
    let file = SourceFile::cast(parse.syntax()).unwrap();
    let imports = file.modules().next().unwrap().imports().unwrap();
    let groups: Vec<_> = imports.symbols_from_modules().collect();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].symbols(), ["id-x", "Bar"]);
    assert_eq!(groups[0].module_name().unwrap().text(), "Other");
    assert_eq!(groups[0].assigned_identifier().as_deref(), Some("{ 1 2 }"));
    assert_eq!(groups[1].assigned_identifier(), None);
}

#[test]
fn assignment_views() {
    let parse = assert_clean(SRC);
    let file = SourceFile::cast(parse.syntax()).unwrap();
    let module = file.modules().next().unwrap();
    let names: Vec<_> = module
        .assignments()
        .map(|a| a.name().unwrap().text().to_owned())
        .collect();
    assert_eq!(names, ["A", "b", "V"]);
    assert!(matches!(
        module.assignments().nth(1),
        Some(Assignment::Value(_))
    ));
    assert!(matches!(
        module.assignments().nth(2),
        Some(Assignment::ValueSet(_))
    ));
}

#[test]
fn an_absent_tag_default_is_none() {
    let parse = assert_clean("M DEFINITIONS ::= BEGIN END");
    let file = SourceFile::cast(parse.syntax()).unwrap();
    assert_eq!(file.modules().next().unwrap().tag_default(), None);
}

#[test]
fn bare_bodies_expose_assignments_through_the_source_file() {
    let parse = assert_clean("A ::= INTEGER\nb A ::= 1");
    let file = SourceFile::cast(parse.syntax()).unwrap();
    assert_eq!(file.assignments().count(), 2);
}
