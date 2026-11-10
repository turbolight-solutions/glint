#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use glint_check::{Closure, ClosureInput, Problem, Role, closure};
use glint_corpus::{
    Corpus, CorpusData, CorpusProvider, EmptyCorpus, Exports, ModuleRecord, ModuleSource,
    ParseStatus, SourceKind, SyntaxEra,
};

fn record(rfc: u32, text: &str) -> ModuleRecord {
    let name = text.split_whitespace().next().unwrap().to_owned();
    let header = text.split("DEFINITIONS").next().unwrap();
    let oid = header
        .split_once('{')
        .and_then(|(_, rest)| rest.split_once('}'))
        .map(|(arcs, _)| arcs.split_whitespace().collect::<Vec<_>>().join("."))
        .and_then(|dotted| dotted.parse().ok());
    ModuleRecord {
        name,
        oid,
        source: ModuleSource::rfc(rfc, None, SourceKind::Extracted),
        rfc_status: None,
        obsoleted_by: Vec::new(),
        syntax_era: SyntaxEra::Neutral,
        text: text.to_owned(),
        exports: Exports::All,
        parse_status: ParseStatus::Clean,
        sha256: String::new(),
    }
}

fn corpus(records: Vec<ModuleRecord>) -> Corpus {
    Corpus::from_data(CorpusData {
        modules: records,
        ..CorpusData::default()
    })
}

fn input(text: &str) -> ClosureInput {
    ClosureInput::Text {
        label: "input.asn".to_owned(),
        text: text.to_owned(),
    }
}

fn run(inputs: &[ClosureInput], corpus: &dyn CorpusProvider) -> Closure {
    closure(inputs, corpus, &[]).unwrap_or_else(|problems| panic!("{problems:?}"))
}

fn names(result: &Closure) -> Vec<&str> {
    result.modules.iter().map(|m| m.name.as_str()).collect()
}

fn text_of<'a>(result: &'a Closure, name: &str) -> &'a str {
    &result.modules.iter().find(|m| m.name == name).unwrap().text
}

const BASE: &str = "Base { 1 2 3 } DEFINITIONS ::= BEGIN T ::= INTEGER END";
const MIDDLE: &str = "Middle { 1 2 4 } DEFINITIONS ::= BEGIN IMPORTS T FROM Base { 1 2 3 }; U ::= SEQUENCE { t T } END";

#[test]
fn dependencies_come_before_the_modules_that_import_them() {
    let corpus = corpus(vec![record(1, BASE), record(2, MIDDLE)]);
    let result = run(
        &[input(
            "Top DEFINITIONS ::= BEGIN IMPORTS U FROM Middle { 1 2 4 }; V ::= U END",
        )],
        &corpus,
    );
    assert_eq!(names(&result), ["Base", "Middle", "Top"]);
    let roles: Vec<Role> = result.modules.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::Dependency, Role::Dependency, Role::Input]);
    assert_eq!(result.modules[0].source, "rfc1#Base");
    assert_eq!(text_of(&result, "Middle"), MIDDLE);
}

const OTHER_BASE: &str = "Base { 1 2 9 } DEFINITIONS ::= BEGIN T ::= BOOLEAN END";

#[test]
fn two_modules_of_one_name_are_a_problem() {
    let corpus = corpus(vec![
        record(1, BASE),
        record(2, MIDDLE),
        record(3, OTHER_BASE),
    ]);
    let problems = closure(
        &[input(
            "Top DEFINITIONS ::= BEGIN IMPORTS U FROM Middle { 1 2 4 } T FROM Base { 1 2 9 }; \
             V ::= SEQUENCE { u U, t Base.T } END",
        )],
        &corpus,
        &[],
    )
    .unwrap_err();
    assert_eq!(
        problems,
        [Problem::SameName {
            name: "Base".to_owned(),
            sources: vec!["rfc1#Base".to_owned(), "rfc3#Base".to_owned()],
        }]
    );
    assert_eq!(
        problems[0].to_string(),
        "the set needs 2 modules named Base (rfc1#Base, rfc3#Base), and a compiler that finds modules by name \
         takes one"
    );
}

#[test]
fn every_missing_import_is_named_with_its_identifier_and_importers() {
    let problems = closure(
        &[
            input("A DEFINITIONS ::= BEGIN IMPORTS X FROM Gone { 1 9 9 }; Y ::= X END"),
            input("B DEFINITIONS ::= BEGIN IMPORTS X FROM Gone { 1 9 9 }; Z ::= X END"),
        ],
        &EmptyCorpus::default(),
        &[],
    )
    .unwrap_err();
    assert_eq!(
        problems,
        [Problem::Missing {
            name: "Gone".to_owned(),
            identifier: Some("{ 1 9 9 }".to_owned()),
            importers: vec!["A".to_owned(), "B".to_owned()],
        }]
    );
    assert_eq!(
        problems[0].to_string(),
        "Gone (not found; FROM { 1 9 9 }; imported by A, B)"
    );
}

#[test]
fn an_input_takes_the_place_of_the_corpus_module_with_its_identifier() {
    // A mutated copy of a corpus module must not be replaced by the corpus copy through an import cycle.
    let corpus = corpus(vec![
        record(
            1,
            "Base { 1 2 3 } DEFINITIONS ::= BEGIN IMPORTS U FROM Middle { 1 2 4 }; T ::= INTEGER END",
        ),
        record(2, MIDDLE),
    ]);
    let result = run(
        &[input(
            "Base { 1 2 3 } DEFINITIONS ::= BEGIN IMPORTS U FROM Middle { 1 2 4 }; T ::= BOOLEAN END",
        )],
        &corpus,
    );
    assert_eq!(names(&result), ["Middle", "Base"]);
    assert!(text_of(&result, "Base").contains("T ::= BOOLEAN"));
    assert_eq!(result.cycles.len(), 1);
}

#[test]
fn an_input_without_a_module_is_a_problem() {
    let problems = closure(&[input("T ::= INTEGER")], &EmptyCorpus::default(), &[]).unwrap_err();
    assert_eq!(
        problems,
        [Problem::NoModule {
            input: "input.asn".to_owned()
        }]
    );
}

#[test]
fn a_corpus_input_is_the_corpus_module_itself_not_a_copy() {
    // An identifier the parser cannot evaluate alone (a reference) must not make an import cycle load the module
    // a second time.
    let corpus = corpus(vec![
        record(
            1,
            "Cyc { id-cyc } DEFINITIONS ::= BEGIN IMPORTS U FROM Other { 1 2 8 }; id-cyc OBJECT IDENTIFIER ::= { 1 2 7 } T ::= INTEGER END",
        ),
        record(
            2,
            "Other { 1 2 8 } DEFINITIONS ::= BEGIN IMPORTS T FROM Cyc; U ::= T END",
        ),
    ]);
    let result = run(&[ClosureInput::Corpus { record: 0 }], &corpus);
    assert_eq!(names(&result), ["Other", "Cyc"]);
    assert_eq!(result.modules[1].role, Role::Input);
    assert_eq!(result.modules[1].source, "rfc1#Cyc");
}

#[test]
fn a_corpus_input_keeps_the_identifier_the_corpus_evaluated() {
    // X.501 modules write their identifier with value references; an import by that identifier from a module of the
    // cycle must find the input, not load the corpus copy.
    let mut cyc = record(
        1,
        "Cyc { id-cyc } DEFINITIONS ::= BEGIN IMPORTS U FROM Other { 1 2 8 }; id-cyc OBJECT IDENTIFIER ::= { 1 2 7 } T ::= INTEGER END",
    );
    cyc.oid = Some("1.2.7".parse().unwrap());
    let corpus = corpus(vec![
        cyc,
        record(
            2,
            "Other { 1 2 8 } DEFINITIONS ::= BEGIN IMPORTS T FROM Cyc-Renamed { 1 2 7 }; U ::= T END",
        ),
    ]);
    let result = run(&[ClosureInput::Corpus { record: 0 }], &corpus);
    assert_eq!(names(&result), ["Other", "Cyc"]);
}
