#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use glint_check::{
    ArcOrigin, Closure, ClosureInput, ClosureOptions, FIRST_AUTO_ARC, OidKey, Problem, Rename,
    Role, Warning, closure,
};
use glint_corpus::{
    Corpus, CorpusData, CorpusProvider, EmptyCorpus, Exports, ModuleRecord, ModuleSource,
    OidAssignment, ParseStatus, SourceKind, SyntaxEra,
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

const AS_WRITTEN: ClosureOptions = ClosureOptions {
    rename: false,
    renumber: false,
    builtin_strings: false,
};
const RENAME: ClosureOptions = ClosureOptions {
    rename: true,
    ..AS_WRITTEN
};
const RENUMBER: ClosureOptions = ClosureOptions {
    renumber: true,
    ..AS_WRITTEN
};
const BUILTIN_STRINGS: ClosureOptions = ClosureOptions {
    builtin_strings: true,
    ..AS_WRITTEN
};

fn run(inputs: &[ClosureInput], corpus: &dyn CorpusProvider, rewrites: ClosureOptions) -> Closure {
    closure(inputs, corpus, &[], rewrites).unwrap_or_else(|problems| panic!("{problems:?}"))
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
        AS_WRITTEN,
    );
    assert_eq!(names(&result), ["Base", "Middle", "Top"]);
    let roles: Vec<Role> = result.modules.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::Dependency, Role::Dependency, Role::Input]);
    assert_eq!(result.modules[0].source, "rfc1#Base");
    assert_eq!(text_of(&result, "Middle"), MIDDLE);
    assert!(result.warnings.is_empty());
}

#[test]
fn an_import_resolved_by_identifier_is_renamed_to_the_module_found() {
    // RFC 9882 imports X509-ML-DSA-2024 with the identifier RFC 9881 published as X509-ML-DSA-2025.
    let corpus = corpus(vec![record(1, BASE)]);
    let result = run(
        &[input(
            "Top DEFINITIONS ::= BEGIN IMPORTS T FROM Old-Base { 1 2 3 }; V ::= Old-Base.T W ::= T END",
        )],
        &corpus,
        RENAME,
    );
    let top = text_of(&result, "Top");
    assert!(top.contains("FROM Base { 1 2 3 }"), "{top}");
    assert!(top.contains("V ::= Base.T"), "{top}");
    assert!(!top.contains("Old-Base"), "{top}");
    assert_eq!(result.renames.len(), 1);
    assert_eq!(
        (
            result.renames[0].from.as_str(),
            result.renames[0].to.as_str()
        ),
        ("Old-Base", "Base")
    );
}

#[test]
fn without_rename_an_import_resolved_to_a_module_of_another_name_is_a_warning() {
    let corpus = corpus(vec![record(1, BASE)]);
    let top = "Top DEFINITIONS ::= BEGIN IMPORTS T FROM Old-Base { 1 2 3 }; V ::= Old-Base.T END";
    let result = run(&[input(top)], &corpus, AS_WRITTEN);
    assert_eq!(text_of(&result, "Top"), top);
    assert!(result.renames.is_empty());
    let warnings: Vec<String> = result.warnings.iter().map(ToString::to_string).collect();
    assert_eq!(
        warnings,
        [
            "Top: imports Old-Base, which resolves by its identifier to Base; a compiler that finds imports by name \
          will not find it"
        ]
    );
}

const OTHER_BASE: &str = "Base { 1 2 9 } DEFINITIONS ::= BEGIN T ::= BOOLEAN END";

#[test]
fn two_modules_of_one_name_get_names_of_their_own() {
    // CMS imports RFC 5280's PKIX1Explicit88, and RFC 3281's attribute certificate module, which CMS also
    // imports, takes RFC 2459's: a compiler that finds modules by name cannot take both as PKIX1Explicit88.
    let corpus = corpus(vec![
        record(1, BASE),
        record(2, MIDDLE),
        record(3, OTHER_BASE),
    ]);
    let result = run(
        &[input(
            "Top DEFINITIONS ::= BEGIN IMPORTS U FROM Middle { 1 2 4 } T FROM Base { 1 2 9 }; \
             V ::= SEQUENCE { u U, t Base.T } END",
        )],
        &corpus,
        RENAME,
    );
    let mut got = names(&result);
    got.sort_unstable();
    assert_eq!(got, ["Base-rfc1", "Base-rfc3", "Middle", "Top"]);
    assert!(text_of(&result, "Base-rfc1").starts_with("Base-rfc1 { 1 2 3 }"));
    assert!(text_of(&result, "Base-rfc3").starts_with("Base-rfc3 { 1 2 9 }"));
    let middle = text_of(&result, "Middle");
    assert!(middle.contains("FROM Base-rfc1 { 1 2 3 }"), "{middle}");
    let top = text_of(&result, "Top");
    assert!(top.contains("FROM Base-rfc3 { 1 2 9 }"), "{top}");
    assert!(top.contains("t Base-rfc3.T"), "{top}");
    let renamed: Vec<(&str, &str, &str)> = result
        .module_renames
        .iter()
        .map(|r| (r.source.as_str(), r.from.as_str(), r.to.as_str()))
        .collect();
    assert_eq!(
        renamed,
        [
            ("rfc1#Base", "Base", "Base-rfc1"),
            ("rfc3#Base", "Base", "Base-rfc3")
        ]
    );
}

#[test]
fn without_rename_two_modules_of_one_name_are_a_problem() {
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
        AS_WRITTEN,
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
fn an_input_keeps_its_name_beside_a_dependency_of_that_name() {
    let corpus = corpus(vec![record(1, BASE), record(2, MIDDLE)]);
    let result = run(
        &[input(
            "Base { 1 2 9 } DEFINITIONS ::= BEGIN IMPORTS U FROM Middle { 1 2 4 }; V ::= U END",
        )],
        &corpus,
        RENAME,
    );
    assert_eq!(names(&result), ["Base-rfc1", "Middle", "Base"]);
    assert_eq!(result.module_renames.len(), 1);
}

#[test]
fn same_named_modules_of_one_document_are_told_apart_by_identifier() {
    // RFC 4998 defines ERS twice, for 1988 and for 1997 syntax.
    let corpus = corpus(vec![record(4, BASE), record(4, OTHER_BASE)]);
    let result = run(
        &[input(
            "Top DEFINITIONS ::= BEGIN IMPORTS T FROM Base { 1 2 3 } \
             T2 FROM Other { 1 2 9 }; A ::= T B ::= T2 END",
        )],
        &corpus,
        RENAME,
    );
    let mut got = names(&result);
    got.sort_unstable();
    assert_eq!(got, ["Base-rfc4-1-2-3", "Base-rfc4-1-2-9", "Top"]);
}

const EXPLICIT88: &str = "Explicit88 { 1 2 5 } DEFINITIONS ::= BEGIN
UniversalString ::= [UNIVERSAL 28] IMPLICIT OCTET STRING -- defined in ASN.1:1993
BMPString ::= [UNIVERSAL 30] IMPLICIT OCTET STRING
UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING
Name ::= CHOICE { utf8 UTF8String, bmp BMPString }
END";

#[test]
fn definitions_of_built_in_string_types_and_their_imports_are_commented_out() {
    // RFC 5280's PKIX1Explicit88 is 1988 ASN.1 and defines these types with their universal tags.
    let corpus = corpus(vec![record(5280, EXPLICIT88)]);
    let result = run(
        &[input(
            "Implicit88 DEFINITIONS ::= BEGIN IMPORTS BMPString, UTF8String, -- new types --\n\
             Name, UniversalString FROM Explicit88 { 1 2 5 }; A ::= SEQUENCE { n Name, s UTF8String } END",
        )],
        &corpus,
        BUILTIN_STRINGS,
    );
    let explicit = text_of(&result, "Explicit88");
    assert!(
        explicit.contains(
            "/* UniversalString ::= [UNIVERSAL 28] IMPLICIT OCTET STRING */ -- built in since ASN.1:1994; \
             see the header of combined.asn -- -- defined in ASN.1:1993"
        ),
        "{explicit}"
    );
    assert!(explicit.contains("/* UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING */"));
    assert!(explicit.contains("Name ::= CHOICE { utf8 UTF8String, bmp BMPString }"));
    let implicit = text_of(&result, "Implicit88");
    assert!(
        implicit.contains(
            "IMPORTS /* BMPString, UTF8String, */ -- new types --\nName/* , UniversalString */ FROM"
        ),
        "{implicit}"
    );
    let definitions: Vec<&str> = result
        .builtin_definitions
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(definitions, ["UniversalString", "BMPString", "UTF8String"]);
    let imports: Vec<(&str, &str)> = result
        .builtin_imports
        .iter()
        .map(|i| (i.importer.as_str(), i.name.as_str()))
        .collect();
    assert_eq!(
        imports,
        [
            ("Implicit88", "BMPString"),
            ("Implicit88", "UTF8String"),
            ("Implicit88", "UniversalString")
        ]
    );
}

#[test]
fn without_builtin_strings_a_definition_of_a_built_in_string_type_is_a_warning() {
    let corpus = corpus(vec![record(5280, EXPLICIT88)]);
    let result = run(
        &[input(
            "Top DEFINITIONS ::= BEGIN IMPORTS UTF8String FROM Explicit88 { 1 2 5 }; A ::= UTF8String END",
        )],
        &corpus,
        AS_WRITTEN,
    );
    assert_eq!(text_of(&result, "Explicit88"), EXPLICIT88);
    assert!(result.builtin_definitions.is_empty() && result.builtin_imports.is_empty());
    let warnings: Vec<String> = result.warnings.iter().map(ToString::to_string).collect();
    assert_eq!(
        warnings,
        ["UniversalString", "BMPString", "UTF8String"].map(|name| format!(
            "Explicit88: defines {name}, a built-in type in current ASN.1, which a compiler of current ASN.1 \
             rejects"
        ))
    );
}

#[test]
fn an_import_of_nothing_but_a_built_in_string_type_is_commented_out_whole() {
    // RFC 5698's DSSC imports UTF8String alone from PKIX1Explicit88.
    let corpus = corpus(vec![record(5280, EXPLICIT88)]);
    let result = run(
        &[input(
            "Dssc DEFINITIONS ::= BEGIN IMPORTS UTF8String FROM Explicit88 { 1 2 5 }; A ::= UTF8String END",
        )],
        &corpus,
        BUILTIN_STRINGS,
    );
    let dssc = text_of(&result, "Dssc");
    assert!(
        dssc.contains(
            "IMPORTS /* UTF8String FROM Explicit88 { 1 2 5 } */ -- a built-in type in current ASN.1 --;"
        ),
        "{dssc}"
    );
}

#[test]
fn a_definition_with_another_tag_is_left_for_the_compiler() {
    let corpus = corpus(vec![record(
        1,
        "Odd { 1 2 6 } DEFINITIONS ::= BEGIN UTF8String ::= [UNIVERSAL 4] IMPLICIT OCTET STRING END",
    )]);
    let result = run(
        &[input(
            "Top DEFINITIONS ::= BEGIN IMPORTS UTF8String FROM Odd { 1 2 6 }; A ::= UTF8String END",
        )],
        &corpus,
        BUILTIN_STRINGS,
    );
    assert!(result.builtin_definitions.is_empty() && result.builtin_imports.is_empty());
    assert!(text_of(&result, "Odd").contains("UTF8String ::= [UNIVERSAL 4] IMPLICIT OCTET STRING"));
}

#[test]
fn a_placeholder_takes_the_number_the_corpus_gives_the_same_module() {
    // A draft still saying TBDMOD while its RFC is published with the number.
    let corpus = corpus(vec![
        record(1, BASE),
        record(10073, "Comp { 1 2 5 127 } DEFINITIONS ::= BEGIN END"),
    ]);
    let result = run(
        &[input(
            "Comp { 1 2 5 id-mod-comp(TBDMOD) } DEFINITIONS ::= BEGIN IMPORTS T FROM Base { 1 2 3 }; END",
        )],
        &corpus,
        RENUMBER,
    );
    assert!(text_of(&result, "Comp").starts_with("Comp { 1 2 5 id-mod-comp(127) }"));
    assert_eq!(result.arcs.len(), 1);
    let fill = &result.arcs[0];
    assert_eq!(fill.key, OidKey::Module("Comp".to_owned()));
    assert_eq!(fill.arc, 127);
    assert_eq!(
        fill.origin,
        ArcOrigin::Copied {
            from: "rfc10073#Comp".to_owned()
        }
    );
    assert_eq!(fill.sites, [("Comp".to_owned(), "TBDMOD".to_owned())]);
}

#[test]
fn a_key_with_no_number_anywhere_gets_one_number_at_every_site() {
    let inputs = [
        input("A { 1 2 9 TBD } DEFINITIONS ::= BEGIN X ::= INTEGER END"),
        input(
            "B DEFINITIONS ::= BEGIN IMPORTS X FROM A { 1 2 9 TBD }; id-b OBJECT IDENTIFIER ::= { 1 2 10 TBD1 } Y ::= X END",
        ),
    ];
    let result = run(&inputs, &EmptyCorpus::default(), RENUMBER);
    assert!(text_of(&result, "A").starts_with(&format!("A {{ 1 2 9 {FIRST_AUTO_ARC} }}")));
    let b = text_of(&result, "B");
    assert!(
        b.contains(&format!("FROM A {{ 1 2 9 {FIRST_AUTO_ARC} }}")),
        "{b}"
    );
    let value_arc = FIRST_AUTO_ARC + 1;
    assert!(b.contains(&format!("{{ 1 2 10 {value_arc} }}")), "{b}");
    let keys: Vec<(&OidKey, u128, &ArcOrigin)> = result
        .arcs
        .iter()
        .map(|f| (&f.key, f.arc, &f.origin))
        .collect();
    assert_eq!(
        keys,
        [
            (
                &OidKey::Module("A".to_owned()),
                FIRST_AUTO_ARC,
                &ArcOrigin::AutoAssigned
            ),
            (
                &OidKey::Value("id-b".to_owned()),
                value_arc,
                &ArcOrigin::AutoAssigned
            ),
        ]
    );
    assert_eq!(result.arcs[0].sites.len(), 2, "the header and the import");
}

#[test]
fn a_value_placeholder_takes_the_number_the_corpus_assigns_that_name() {
    let corpus = Corpus::from_data(CorpusData {
        oid_assignments: vec![OidAssignment {
            oid: "1.2.10.42".parse().unwrap(),
            name: "id-b".to_owned(),
            module: "Published".to_owned(),
            document: "rfc7".to_owned(),
        }],
        ..CorpusData::default()
    });
    let result = run(
        &[input(
            "B DEFINITIONS ::= BEGIN id-b OBJECT IDENTIFIER ::= { 1 2 10 TBD } END",
        )],
        &corpus,
        RENUMBER,
    );
    assert!(text_of(&result, "B").contains("{ 1 2 10 42 }"));
    assert_eq!(
        result.arcs[0].origin,
        ArcOrigin::Copied {
            from: "rfc7#Published".to_owned()
        }
    );
}

#[test]
fn two_numbers_for_one_key_are_a_conflict() {
    let corpus = corpus(vec![
        record(
            1,
            "Comp { 1 2 5 127 } DEFINITIONS ::= BEGIN X ::= INTEGER END",
        ),
        record(
            2,
            "User DEFINITIONS ::= BEGIN IMPORTS X FROM Comp { 1 2 5 128 }; Z ::= X END",
        ),
    ]);
    let problems = closure(
        &[
            input("Comp { 1 2 5 TBDMOD } DEFINITIONS ::= BEGIN X ::= INTEGER END"),
            input("Top DEFINITIONS ::= BEGIN IMPORTS Z FROM User; W ::= Z END"),
        ],
        &corpus,
        &[],
        RENUMBER,
    )
    .unwrap_err();
    let conflict = problems
        .iter()
        .find_map(|p| match p {
            Problem::ArcConflict { key, numbers } => Some((key, numbers)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{problems:?}"));
    assert_eq!(conflict.0, &OidKey::Module("Comp".to_owned()));
    let arcs: Vec<u128> = conflict.1.iter().map(|(arc, _)| *arc).collect();
    assert_eq!(arcs, [127, 128]);
    let line = problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        line.contains("127 in rfc1#Comp") && line.contains("128 in"),
        "{line}"
    );
}

#[test]
fn without_renumber_two_numbers_for_one_key_are_no_problem() {
    let corpus = corpus(vec![
        record(
            1,
            "Comp { 1 2 5 127 } DEFINITIONS ::= BEGIN X ::= INTEGER END",
        ),
        record(
            2,
            "User DEFINITIONS ::= BEGIN IMPORTS X FROM Comp { 1 2 5 128 }; Z ::= X END",
        ),
    ]);
    let result = run(
        &[
            input("Comp { 1 2 5 TBDMOD } DEFINITIONS ::= BEGIN X ::= INTEGER END"),
            input("Top DEFINITIONS ::= BEGIN IMPORTS Z FROM User; W ::= Z END"),
        ],
        &corpus,
        AS_WRITTEN,
    );
    assert!(text_of(&result, "Comp").contains("TBDMOD"));
    let warnings: Vec<String> = result.warnings.iter().map(ToString::to_string).collect();
    assert_eq!(
        warnings,
        ["OID: module Comp has no number for its last arc in Comp (TBDMOD)"]
    );
}

fn draft_record(document: &str, text: &str) -> ModuleRecord {
    ModuleRecord {
        source: ModuleSource::draft(document, "2026-11-10", None, SourceKind::Extracted),
        ..record(1, text)
    }
}

const ISIS: &str = "Isis { 1 2 11 } DEFINITIONS ::= BEGIN id-isis OBJECT IDENTIFIER ::= { 1 2 12 TBD } T ::= INTEGER END";

#[test]
fn renumber_leaves_a_placeholder_in_an_rfc_as_written_with_a_warning() {
    // Only an erratum can number an arc an RFC published without one.
    let corpus = corpus(vec![record(1142, ISIS)]);
    let expected = [Warning::PublishedPlaceholder {
        key: OidKey::Value("id-isis".to_owned()),
        document: "rfc1142".to_owned(),
        sites: vec![("Isis".to_owned(), "TBD".to_owned())],
    }];
    for inputs in [
        vec![input(
            "User { 1 2 13 } DEFINITIONS ::= BEGIN IMPORTS T FROM Isis { 1 2 11 }; END",
        )],
        vec![ClosureInput::Corpus { record: 0 }],
    ] {
        for rewrites in [RENUMBER, AS_WRITTEN] {
            let result = run(&inputs, &corpus, rewrites);
            assert_eq!(text_of(&result, "Isis"), ISIS);
            assert!(result.arcs.is_empty());
            assert_eq!(result.warnings, expected);
        }
    }
}

#[test]
fn renumber_numbers_the_draft_and_input_sites_of_a_key_and_leaves_the_rfc_site() {
    let corpus = corpus(vec![
        record(
            9999,
            "Dep { 1 2 14 TBD } DEFINITIONS ::= BEGIN T ::= INTEGER END",
        ),
        draft_record(
            "draft-ietf-foo-bar-01",
            "Draft { 1 2 15 TBD } DEFINITIONS ::= BEGIN IMPORTS T FROM Dep { 1 2 14 TBD }; END",
        ),
    ]);
    let inputs = [
        ClosureInput::Corpus { record: 1 },
        input("User { 1 2 16 } DEFINITIONS ::= BEGIN IMPORTS T FROM Dep { 1 2 14 TBD }; END"),
    ];
    let result = run(&inputs, &corpus, RENUMBER);
    assert!(text_of(&result, "Dep").starts_with("Dep { 1 2 14 TBD }"));
    let module_arc = FIRST_AUTO_ARC;
    let draft_arc = FIRST_AUTO_ARC + 1;
    let draft = text_of(&result, "Draft");
    assert!(
        draft.starts_with(&format!("Draft {{ 1 2 15 {draft_arc} }}")),
        "{draft}"
    );
    assert!(
        draft.contains(&format!("FROM Dep {{ 1 2 14 {module_arc} }}")),
        "{draft}"
    );
    assert!(text_of(&result, "User").contains(&format!("FROM Dep {{ 1 2 14 {module_arc} }}")));
    let dep = &result.arcs[0];
    assert_eq!(dep.key, OidKey::Module("Dep".to_owned()));
    assert_eq!(
        dep.sites,
        [
            ("User".to_owned(), "TBD".to_owned()),
            ("Draft".to_owned(), "TBD".to_owned())
        ]
    );
    assert_eq!(
        result.warnings,
        [Warning::PublishedPlaceholder {
            key: OidKey::Module("Dep".to_owned()),
            document: "rfc9999".to_owned(),
            sites: vec![("Dep".to_owned(), "TBD".to_owned())],
        }]
    );
}

#[test]
fn a_malformed_number_is_left_for_the_compiler() {
    let result = run(
        &[input("A { 1 2 3x } DEFINITIONS ::= BEGIN END")],
        &EmptyCorpus::default(),
        RENUMBER,
    );
    assert!(result.arcs.is_empty());
    assert!(text_of(&result, "A").contains("3x"));
}

#[test]
fn an_empty_number_form_gets_its_number_inside_the_parentheses() {
    let result = run(
        &[input("A { 1 2 id-mod-a() } DEFINITIONS ::= BEGIN END")],
        &EmptyCorpus::default(),
        RENUMBER,
    );
    let a = text_of(&result, "A");
    assert!(
        a.starts_with(&format!("A {{ 1 2 id-mod-a({FIRST_AUTO_ARC}) }}")),
        "{a}"
    );
    assert_eq!(result.arcs[0].sites[0].1, "()");
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
        AS_WRITTEN,
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
        AS_WRITTEN,
    );
    assert_eq!(names(&result), ["Middle", "Base"]);
    assert!(text_of(&result, "Base").contains("T ::= BOOLEAN"));
    assert_eq!(result.cycles.len(), 1);
}

#[test]
fn an_input_without_a_module_is_a_problem() {
    let problems = closure(
        &[input("T ::= INTEGER")],
        &EmptyCorpus::default(),
        &[],
        AS_WRITTEN,
    )
    .unwrap_err();
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
    let result = run(&[ClosureInput::Corpus { record: 0 }], &corpus, AS_WRITTEN);
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
    let result = run(&[ClosureInput::Corpus { record: 0 }], &corpus, AS_WRITTEN);
    assert_eq!(names(&result), ["Other", "Cyc"]);
    assert_eq!(
        result.warnings,
        [Warning::ImportName(Rename {
            importer: "Other".to_owned(),
            from: "Cyc-Renamed".to_owned(),
            to: "Cyc".to_owned(),
        })]
    );
}
