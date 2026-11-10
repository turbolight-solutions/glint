# Glint — RFC ASN.1 Validator Design Spec

Nov 2, 2026 · @C B

## Purpose and scope

Build Glint (binary `glint`), a Rust ASN.1 checker for Internet-Draft and RFC authors: it parses modules per ITU-T X.680–X.683 (2021 editions), resolves IMPORTS against a corpus of every ASN.1 module published in an RFC or an active Internet-Draft, and verifies that ASN.1 fragments quoted in a draft's prose match the draft's own module. It ships as a CLI and as a static web app running the same Rust core in the browser.

This spec is written for an implementing model (Claude Sonnet). Sections 3–12 describe what to build; section 13 orders the work into milestones with acceptance criteria; section 14 lists rules for the implementer and open questions for the project owner.

### Goals

1. **Parse faithfully.** Accept the full X.680 notation plus X.681 (information object classes), X.682 (constraints) and X.683 (parameterization), with precise source-span errors.
2. **Resolve imports against all RFCs.** Every `IMPORTS ... FROM` in a draft resolves to a real module from the RFC corpus (or to another module in the same draft) without the author supplying files.
3. **Check semantics that matter to reviewers.** Undefined or unexported symbols, tag clashes, bad OIDs, OID collisions with already-published assignments, constraint and parameter errors.
4. **Keep prose and module in sync.** Every ASN.1 snippet in the body must equal the corresponding definition in the module appendix.
5. **Fit author workflows.** Web paste/upload, a CLI, and CI output (JSON, SARIF) for GitHub-hosted drafts.

### Non-goals (v1)

- Encoding or decoding (BER/DER/PER), code generation, or runtime libraries. Decoding example encodings in drafts is planned after v1 (section 5, Resolved type model).
- SMIv1/SMIv2 MIB modules. They use ASN.1 macros but have their own rules; point users to `smilint`. Detect and skip them with an info diagnostic.
- Any X.208 macro engine. `MACRO` definitions and their uses are kept opaque (section 4, Legacy MACRO handling).
- Storing user-submitted drafts. Unpublished drafts are confidential; the web UI checks them in the browser and sends them nowhere.

## Author workflows

Three entry points, one engine; every path produces the same diagnostic list.

| Workflow | Input | Output |
| --- | --- | --- |
| Web: paste or upload | RFCXML v3, kramdown-rfc/mmark Markdown, plain text, or a bare `.asn` module | Annotated editor view, diagnostics panel, resolved import graph |
| Web: open by name | A draft name such as `draft-ietf-lamps-foo-03`, or an RFC number | Same as above; the text comes from the published data (`documents.bin`), which has every RFC and active draft with an ASN.1 module |
| CLI, local | File path(s) or stdin | Human-readable diagnostics, exit code |
| CI | Repo files via GitHub Action | SARIF for code scanning annotations, or JSON |
| Corpus browsing | Module name, module OID, or any OID | Module source, origin RFC/section, exports, who assigns a given OID |

Typical author loop: write the module in the draft's appendix, quote pieces of it in the body, run `glint check draft.xml` before each revision. Many working groups use the `i-d-template` GitHub setup, so a ready-made Action is a high-leverage deliverable.

## Architecture

A single Cargo workspace with a pure core (no filesystem, network or threads) wrapped by thin shells: the CLI, and `glint-wasm` for the web UI. The core takes text plus a `CorpusProvider` trait object and returns diagnostics.

&#91;embedded content: check pipeline · 6 core stages, 2 shells, offline corpus build\]

The dashed box runs offline every day (the data); everything inside the shaded core is pure and shared by the CLI and the WASM build (the code).

| Crate | Responsibility | Key dependencies |
| --- | --- | --- |
| `glint-syntax` | Lexer, parser, lossless CST, typed AST views, spans | `logos` (lexer), `rowan` (CST) |
| `glint-sema` | Symbol tables, import resolution, type/value/class checking, tagging | — |
| `glint-extract` | Pull ASN.1 blocks out of RFCXML, Markdown and text, with source maps | `roxmltree` or `quick-xml`, `pulldown-cmark` |
| `glint-corpus` | Corpus data model, `CorpusProvider` trait, the data files (`corpus.json`, `documents.bin`, `latest.json`), the OID label table | `serde`, `serde_json` |
| `glint-check` | Snippet consistency, lint rules, diagnostic codes, config | — |
| `glint-diag` | Diagnostic types and renderers (human, JSON, SARIF) | `ariadne` or `miette`, `serde_json` |
| `glint-cli` | The `glint` binary | `clap` |
| `glint-wasm` | The web UI's engine in the browser: checks, documents by name, corpus queries, completion | `wasm-bindgen` |
| `corpus-build` | Offline pipeline that builds the data from RFC and Internet-Draft sources | `reqwest`, `rayon` |
| `web/` | TypeScript frontend (not a crate) | Vite, Bootstrap 5, CodeMirror 6 |

**Naming.** The project is Glint: the glint that catches a reviewer's eye. The binary is `glint`, the config file `glint.toml`, comment directives `-- glint: ...`, environment variables `GLINT_*`, and every workspace crate is `glint-<role>` (the offline builder is crate `glint-corpus-build`, binary `corpus-build`). On crates.io, `glint` (a commit-message tool) and `glint-core`/`glint-common` are already taken, so never use those package names. All workspace crates set `publish = false` except the CLI, which is published as package `glint-asn1` with `[[bin]] name = "glint"`.

Data flow for one check: extract blocks from the document → parse each block → build symbol tables for the draft's modules → resolve imports (draft first, then corpus) → run semantic checks → run snippet consistency → map all spans back to document coordinates → render.

## Parser

Use a hand-written lexer and recursive-descent parser over a lossless concrete syntax tree, with ambiguity deferred to semantic analysis. Do not feed the X.680 BNF to a parser generator: the grammar is ambiguous and context-dependent, and generated parsers give poor errors.

### Lexer (X.680 clauses 11–12)

- **White-space and newlines (12.1.6):** TAB, LF, VT, FF, CR, SPACE and NO-BREAK SPACE are white-space; LF, VT, FF and CR are newlines. Form feeds therefore need no special case in the lexer, only in text-RFC extraction.
- **Names (12.2–12.5):** `typereference` and `modulereference` start upper-case; `identifier` and `valuereference` start lower-case; letters, digits and hyphens only; a hyphen may not be last or doubled. Which one a name is depends on context, so the lexer emits one upper-start and one lower-start name kind.
- **X.681 names (X.681 clause 7):** an `objectclassreference` is a `typereference` with no lower-case letters (7.1); an `objectreference` has `valuereference` form (7.2) and an `objectsetreference` has `typereference` form (7.3). Field references are `&` immediately followed by a name: `&Upper` is a type, value-set or object-set field, `&lower` a value or object field (7.4–7.8); the class definition decides which. A `word` (7.9) is an upper-case name with no lower-case letters or digits, used only in defined syntax. The lexer emits two field-reference kinds (`&Upper`, `&lower`) and nothing finer.
- **Numbers (12.8–12.9):** a `number` may not have a leading zero unless it is `0`; `realnumber` per 12.9. Both violations are `E010`, not silent re-lexing.
- **Strings:** `bstring` (12.10) and `hstring` (12.12) may contain white-space, which is insignificant; `hstring` digits are upper-case `A`–`F` only. `cstring` (12.14) escapes `"` as `""` and may span lines, with spacing next to each line end ignored. `simplestring` (12.16) and `tstring` (12.17) exist only for the time types.
- **Comments (12.6):** `--` ends at the next `--` or the end of the line, whichever comes first; `/* */` nests, and `--` inside it means nothing (and vice versa). Comments and white-space are trivia tokens in the CST.
- **Punctuation:** `::=`, `..`, `...`, `[[`, `]]` (12.20–12.24) and the single characters of 12.37, which include `@`, `.`, `!`, `^`, `|` and `;`. There is no `@.` token. In X.682 `AtNotation` (10.7) the dots after `@` arrive as `.`, `..` or `...` tokens (X.682 10.10 writes `@...errorId`), so the parser counts dots across whatever punctuation follows `@`, not tokens. The non-spacing indicator `&` (5.4) appears only in XML value notation.
- **Separation (12.1.5):** items that could run together must be separated by white-space or a comment; maximal-munch lexing enforces this.
- **Reserved words:** exactly the 12.38 list of the 2021 edition, used as reference names → `E011` (12.2.2, 16.1, 16.6). `ANY`, `DEFINED` and `MACRO` are **not** reserved, so they lex as ordinary names (see Legacy MACRO handling). Words reserved only in later editions, such as the time-type keywords, may appear as names in old corpus modules: report `E011` as a warning for corpus modules. X.208 (1988) had no `UniversalString`, `BMPString` or `UTF8String`, so 1988-syntax modules define them by hand with the universal tag a later edition gave the type (`UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING` in RFC 5280's `PKIX1Explicit88`); in the published modules that do this (`LEGACY_STRING_MODULES`, a closed list by module name and identifier), exactly that definition, with the type's own tag, is correct for the syntax they keep to, so neither `E011` on its name nor `E123` (31.2.5) on its tag is reported. Anywhere else the universal tag is an error, since a new module should not be written in 1988 syntax and a compiler of current ASN.1 rejects the definition.
- **Encoding references (12.25):** upper-case-only names such as `XER` and `PER`, used in encoding prefixes `[XER: ...]`.
- **Out of scope for v1:** the XML lexical items (12.11, 12.13, 12.15, 12.18, 12.28–12.36), which exist only for XML value notation.

### Parser structure

- Entry points: `ModuleDefinition`, `ModuleDefinitionList` (several modules in one block), and `AssignmentList` for bare fragments; 13.1 Note 2 allows a module body outside a module definition.
- IMPORTS ambiguity (13.9): after `FROM modulereference`, an `ObjectIdentifierValue` is always the `AssignedIdentifier`. A following `valuereference` is the `AssignedIdentifier` unless the next token is `,` or `FROM` (then it begins the next `SymbolList`) or `{` `}` (then it is a parameterized symbol). Record which rule fired; `E107` reports violations of 13.9's resolution rules.
- `ValueAssignment`, `ValueSetTypeAssignment` (16.2, 16.6) and the X.681 assignments share leading tokens; they go through the ambiguity strategy below.
- Encoding prefixes (`[XER: ...]`, 31.3) and `ENCODING-CONTROL` sections (clause 54) are kept as opaque token runs. `[TAG: n]` is a normal tag. Unknown encoding references get `W136`, as Annex E's note recommends.
- XML value notation (`XMLValueAssignment`, 16.2) is not supported in v1: keep it as an opaque node and report `W137`.
- Error recovery: on a bad assignment, record an error node, skip to the next line that starts with a reference followed by `::=`, and continue. One typo must not hide every later error.
- Every production function carries its clause, e.g. `// X.680 25.1 SequenceType`.
- Coverage checklist: X.680 Annex L summarizes about 260 productions. It is informative; where it and the clause text differ, the clause text wins.
- Store the CST with `rowan`; expose typed AST wrappers over it. The lossless tree gives exact spans, enables normalized comparison for snippets, and supports a future formatter and LSP.

### Ambiguity strategy

X.681 adds assignment forms that share their first tokens with X.680's (X.680 16.1, 16.2, 16.6; X.681 9.1, 11.1, 12.1):

| Shape | Can be |
| --- | --- |
| `Upper ::= ...` | Type assignment, or class assignment if `Upper` has no lower-case letters |
| `lower Ref ::= ...` | Value assignment if `Ref` is a type, object assignment if `Ref` is a class |
| `Upper Ref ::= { ... }` | Value set assignment if `Ref` is a type, object set assignment if `Ref` is a class |

An all-upper-case name is a valid `typereference` and a valid `objectclassreference` (X.681 7.1), so spelling never decides. Handle it in two phases:

1. **Parse** each right-hand side into a neutral node (`AmbiguousRhs`) when the leading tokens do not decide it, keeping the raw token range.
2. **Resolve** after the symbol table (including imports) is built: decide the kind from what the referenced names denote, then re-parse the token range with the specific production.

### Parameterization (X.683)

- **Definitions (8.1–8.3):** every assignment form has a parameterized variant in which a `ParameterList` `{ ... }` directly follows the first name, before `::=` or before the type or class that governs the right-hand side. Each parameter is `Dummy` or `Governor : Dummy`, where the governor is a type, a class, or another dummy.
- **Uses (9.2, 9.5):** `Name { actual, ... }`. Whether each actual is a type, value, value set, class, object or object set depends on the definition (9.6), so store each actual as a token range, split at top-level commas, and parse it once the definition is resolved. Value set and object set actuals are always written in braces (A.5).
- **IMPORTS and EXPORTS (9.1):** `Name{}` and `Name` mean the same thing.
- **Dummy references (8.4–8.5):** a dummy's scope is the parameter list plus the rest of the assignment, and it hides any other reference with the same name. It does not hide identifiers in named number lists, enumerations or named bit lists. An undecided dummy's kind is fixed by its first use. When a dummy is only passed on as an actual parameter, its kind comes from the callee's definition, so infer kinds to a fixpoint across all parameterized definitions.
- **Tagging (9.8):** references and tags inside an actual parameter keep the tag environment of the module where the actual is written, not that of the definition. A component whose type is an untagged dummy reference is always explicitly tagged (X.680 31.2.7 c), even under `AUTOMATIC TAGS`. The two module examples in 9.8 (`T3` and `T5`) are fixtures with exact expected tags.
- **Instantiation:** expand per use site and memoize by definition plus normalized actuals. 8.7 guarantees that a legal recursive definition such as A.3's `List1` reaches an already-seen instantiation, so expansion terminates; a path that keeps growing (A.3's `List2`) is `E164`.

### Defined syntax (X.681 clauses 10–11)

Objects of a class with `WITH SYNTAX` use that class's own grammar, so store each `{ ... }` object body as a token range and parse it after the class is resolved. The end of the body is found by brace matching, ignoring comments and treating strings as single tokens (10.4 Note). When parsing:

- Inside a `SyntaxList`, `[[` and `]]` are two brackets each (10.5 Note 2); re-split those tokens.
- A `word` takes precedence over a type or object set reference with the same spelling (10.6 Note).
- An optional group is present exactly when the next token can start it (10.10); no backtracking.
- Default syntax `{ &field setting, ... }` is required when the class has no `WITH SYNTAX`, and forbidden when it has one (11.4–11.5).
- Each setting is parsed as a type, value, value set, object or object set according to the field's kind (11.7). A variable-type value setting uses the plain value notation of the type taken from the linked type field (11.8–11.9).
- After `Class.&field`, a parenthesized object set is a table constraint, not a single-value constraint (14.3).

### Built-in classes (X.681 Annexes A–C)

`TYPE-IDENTIFIER` and `ABSTRACT-SYNTAX` are available in every module without importing (A.3, B.3). Ship them as an internal prelude module containing the exact definitions from A.2 and B.2. `INSTANCE OF C` has universal tag 8 (C.4) and its values and subtypes are those of `SEQUENCE { type-id C.&id, value [0] C.&Type }` under `EXPLICIT TAGS` (C.6–C.7); the tag checks and the resolved type model use that sequence.

### Features that must be covered

| Area | Constructs |
| --- | --- |
| Module header (13) | Definitive identifier (OID and IRI forms), encoding reference default, `EXPLICIT`/`IMPLICIT`/`AUTOMATIC TAGS`, `EXTENSIBILITY IMPLIED`, `EXPORTS ALL`/list/empty, `IMPORTS` with `WITH SUCCESSORS`/`WITH DESCENDANTS`, external references `Module.name` (14.6) |
| Types (18–48) | All built-ins including the time types (`TIME`, `DATE`, `TIME-OF-DAY`, `DATE-TIME`, `DURATION`), `OID-IRI` and `RELATIVE-OID-IRI`, `EMBEDDED PDV`, `EXTERNAL`, `CHARACTER STRING`; tagged types; `SEQUENCE`/`SET`/`CHOICE` with extension markers, exception specs and version brackets; `COMPONENTS OF`; `SEQUENCE OF`/`SET OF` with named elements and the `SEQUENCE SIZE(...) OF` forms (49.5); selection types (`<`, 30); open types (`Class.&Type`) |
| Values (16–48) | All value notations including OID and relative OID, `{ }`-forms for SEQUENCE/SET/CHOICE, bit strings with named bits, real `{mantissa, base, exponent}`, time values |
| Constraints (49–53) | Single value, contained subtype (`INCLUDES`), value range with `MIN`/`MAX`/`<`, `FROM`, `SIZE`, type constraint, `WITH COMPONENT(S)`, `PATTERN`, `SETTINGS` and time ranges; set arithmetic with `UNION`, `INTERSECTION`, `EXCEPT`, `ALL EXCEPT` and their symbol forms; extensible constraints; exception specs (`!`) |
| X.681 | `CLASS` with all seven field kinds (9.5–9.12), `UNIQUE`, `OPTIONAL`/`DEFAULT` fields, `WITH SYNTAX` (10), objects in default and defined syntax (11), object sets with extension markers (12.3) and set arithmetic (12.10), object class field types and open-type values `Type : Value` (14), information from objects (15), `TYPE-IDENTIFIER`, `ABSTRACT-SYNTAX`, `INSTANCE OF` (Annexes A–C) |
| X.682 | `GeneralConstraint` (8.1): simple table constraints `({Set})` and component relation constraints `({Set}{@a, @.b})` (10.3, 10.7); constraints on `INSTANCE OF` (Annex A); user-defined `CONSTRAINED BY { ... }` with typed parameters (9); contents constraints `CONTAINING Type`, `ENCODED BY oid` and both together (11) |
| X.683 | Parameterized types, values, value sets, classes, objects and object sets (8.1–8.2); parameters with type, class or dummy governors (8.3); `Name{}` in IMPORTS and EXPORTS (9.1); actual parameter lists (9.5); parameterized abstract syntaxes (10) |
| Legacy (X.208) | `ANY`, `ANY DEFINED BY`; `MACRO` definitions and macro-notation assignments kept as opaque token ranges (see Legacy MACRO handling) |
| Opaque in v1 | Encoding prefixes, `ENCODING-CONTROL` sections, XML value notation |

### Legacy MACRO handling

**Decision: opaque.** X.680 (02/2021) defines neither `MACRO` nor `ANY`, and neither is a reserved word (12.38), so the lexer emits ordinary names and the parser recognizes the legacy forms by context: `Name MACRO ::= BEGIN ... END`, and `ANY` or `ANY DEFINED BY identifier` where a type is expected and no `ANY` is defined or imported. A macro's uses follow a grammar the macro itself defines, so they cannot be parsed without a macro engine.

- **Definition:** `Name MACRO ::= BEGIN ... END` is consumed as one opaque node. The name is entered in the symbol table as kind `Macro`, so it can be imported and exported.
- **Use:** an assignment whose right-hand side starts with a known macro name (local or imported) becomes an opaque node. It ends at the next line that starts with a reference followed by `::=`, or at `END`. The defined name is entered with kind `Unknown`, so references to it resolve but are not type-checked.
- **Rest of the module:** parses and checks normally, so ordinary types in legacy modules stay importable.
- **Corpus:** modules containing macros get `syntax_era = "1988-macro"`, with no diagnostic.
- **Drafts:** `E172` if a draft module defines a new MACRO (help text: use an X.681 information object class). `W173` once per draft module that uses imported macro notation, noting that those assignments are not checked.

## Semantic checks

Checks run per module after import resolution. Each code has a default severity, can be silenced in config, and cites the clause it enforces: bare numbers are X.680 (02/2021), and X.681 and X.682 references are marked. "Policy" marks Glint's own rules. A "shall" in the Recommendations is an error by default. Corpus modules never fail a user's check: their findings are shown as notes.

| Code | Check | Basis | Default |
| --- | --- | --- | --- |
| `E010` | Lexical error: leading zero in a number, name ending in or doubling a hyphen, bad `bstring`/`hstring` character, unterminated comment or string | 12.2–12.3, 12.6, 12.8–12.14 | error |
| `E011` | Reserved word used as a reference name | 12.2.2, 12.38, 16.1, 16.6 | error |
| `W100` | Imported module not found in the draft or the corpus; its symbols are treated as opaque | policy | warning |
| `E101` | Imported symbol not defined in, or not exported by, the target module | 13.13, 13.14, 13.16 a–b | error |
| `W102` | Module matched by name, but the IMPORTS OID differs from the corpus module's; the OID is definitive | 13.10, 13.11 | warning |
| `I103` | Import resolved; names the RFC chosen when several candidates exist | policy | info |
| `E104` | Same module named twice in IMPORTS (by name or OID), or a module importing itself | 13.16 e | error |
| `W106` | IMPORTS gives no OID for a module that has a definitive OID | 13.12 | warning |
| `E107` | Value reference in an `AssignedIdentifier` does not meet 13.9's rules | 13.9 | error |
| `E108` | `WITH SUCCESSORS`/`WITH DESCENDANTS` without an `AssignedIdentifier` | 13.16 f | error |
| `E110` | Undefined reference, or external reference to a module absent from IMPORTS | 14.4–14.7; X.681 8.3 | error |
| `E111` | Two assignments with the same name in a module, parameterized or not | 13.21; X.683 8.2 | error |
| `W112` | Unused import | policy | warning |
| `E113` | Duplicate identifier among components (after `COMPONENTS OF`) or alternatives | 25.14, 29.10 | error |
| `E114` | Symbol imported from two modules, or also defined locally, used without `Module.symbol` | 13.20 | error |
| `E120` | Tag clash among `CHOICE` alternatives or `SET` components, counting every tag of an untagged `CHOICE`, the conceptually added extension element, and the indeterminate tag of an untagged open type | 27.3, 29.3, 29.9, 52.7; X.681 14.2 b | error |
| `E121` | Tag clash between a run of `OPTIONAL`/`DEFAULT` `SEQUENCE` components and the next component, across extension additions | 25.6, 25.7, 52.7 | error |
| `E122` | `IMPLICIT` on an untagged `CHOICE`, open type or `DummyReference` | 31.2.9; X.681 14.2 c | error |
| `E123` | `UNIVERSAL` class tag in a user module; not reported for a 1988 definition of `UniversalString`, `BMPString` or `UTF8String` with the type's own tag in a listed published module (see Reserved words) | 31.2.5 | error |
| `E124` | Textually tagged extension additions under `AUTOMATIC TAGS` when the root is untagged | 25.9, 29.4 | error |
| `E125` | `VersionNumber` misuse: mixed with ungrouped additions, below 2, or not increasing | 25.16 | error |
| `E126` | `COMPONENTS OF` names a type that is not a `SEQUENCE` (or `SET` inside a set) | 25.5, 27.2 | error |
| `E130` | OID with fewer than two arcs, first arc above 2, or second arc 40 or more under arcs 0 and 1 | X.660 (via 32.11) | error |
| `E131` | Value not of a type compatible with its governing type (`DEFAULT`, named number, defined value) | 14.9, 25.12, Annex C | error |
| `E132` | Empty or inverted range, or constraint value outside the parent type or its extension root | 49.7, 50.7–50.11 | error |
| `E133` | Subtype element not applicable to the parent type, such as `SIZE` on `INTEGER` | 51.1 Tables 11–12 | error |
| `E134` | Bare `NameForm` OID arc that is not a registered X.660 name; write `name(number)` | 32.7, 32.12 | error |
| `E135` | Exception spec `!` without an extension marker, `DummyReference` or user-defined constraint | 49.6 | error |
| `W136` | Unknown encoding reference in an encoding prefix or control section; ignored | 31.3, 54, Annex E | warning |
| `W137` | XML value notation present; not checked in v1 | 16.2 | warning |
| `W140` | OID assigned in the draft is already assigned in the corpus to a different name | policy | warning |
| `I141` | OID uses an author placeholder (`TBD`, `TBD1`, `id-TBD`) | policy | info |
| `E150` | Bad field reference: unknown field in a `FieldName` chain, `Class.&field` on an object or object set field, or an information-from-objects combination Table 1 forbids | X.681 9.14, 14.5, 15.4–15.5, 15.11 | error |
| `E151` | Component relation path does not resolve: unknown identifier, a path through a component that is not a `SET`, `SEQUENCE` or `CHOICE`, more dots than enclosing levels, or `@` outside any `SET` or `SEQUENCE` | X.682 10.8–10.10 | error |
| `E152` | Object does not match its class: wrong syntax form, mandatory field missing, field set twice, setting of the wrong kind, or an optional-group literal with no setting after it | X.681 10.11, 10.12 d, 11.4–11.7 | error |
| `E153` | Class definition error: duplicate field names, `UNIQUE` with `DEFAULT`, variable-type field whose optionality conflicts with its type field, chain through a non-object link, or a self-referential chain with no optional link | X.681 9.6, 9.8, 9.10, 9.13, 9.15 | error |
| `E154` | `WITH SYNTAX` error: field missing or repeated, reserved word from the 10.6 list used as a literal, optional group with no field, or optional groups that make a setting ambiguous | X.681 10.6, 10.9, 10.12 a–c | error |
| `E155` | Object or object set not of the governing class; a textually identical but separate `CLASS` definition does not count | X.681 8.2, 11.3, 12.10 | error |
| `E156` | Two objects in one object set with the same `UNIQUE` field value (when both values are known) | X.681 9.7 | error |
| `E157` | Recursive definition or instantiation of an object or object set | X.681 11.2, 12.2 | error |
| `E158` | Empty object set where one is required: `{ }` without `...`, an empty set used directly in a table constraint, or information extracted from an all-empty column | X.681 12.3, 15.12, 15.13 | error |
| `E159` | Open type (type field or variable-type field) used as the type of a value or value set field of a class | X.681 14.2 a, 14.4 | error |
| `E160` | Actual parameter list mismatch: wrong number, order or form of actual parameters, a parameterized name used without actual parameters, or actual parameters given to a name that is not parameterized | X.683 3.4.2, 9.3, 9.6 | error |
| `E161` | Parameter list error: value, value set, object or object set dummy without a governor; type or class dummy with one; governor of the wrong kind; dummy governor that itself has a governor | X.683 8.3 | error |
| `E162` | Dummy reference used inconsistently with its form, its governor, or its other uses | X.683 8.5 | error |
| `E163` | Dummy reference never used in its scope | X.683 8.6 | error |
| `E164` | Illegal recursion: parameterized value, value set, object or object set referring to itself; recursive path whose actual parameters mix a dummy with other text; circular parameterized type, value set or class not broken by `OPTIONAL` or a non-circular `CHOICE` alternative | X.683 8.6–8.8 | error |
| `E165` | Governor refers to another governed dummy, or depends on the dummy or the name being defined | X.683 8.9, 8.11 | error |
| `E166` | Right-hand side of a parameterized assignment is only a dummy reference | X.683 8.10 | error |
| `E167` | Actual value or value set not compatible with its governor, or governor admits values that are invalid where the dummy is used | X.683 8.12–8.13; X.680 C.6 | error |
| `E168` | Parameter of a parameterized abstract syntax used other than (directly or via actual parameters) in a constraint | X.683 10.2 | error |
| `W169` | Variable constraint (depends on an abstract syntax parameter) without an exception spec | X.683 10.3–10.4 Note | warning |
| `W170` | 1988 syntax (`ANY`) mixed with X.681 constructs in one module | policy | warning |
| `W171` | Reference style, such as an all-upper-case type name that reads as a class reference | policy (X.681 7.1) | warning |
| `E172` | Draft module defines a new `MACRO` | policy (not in X.680) | error |
| `W173` | Draft module uses imported macro notation; those assignments are not checked | policy | warning |
| `E180` | Table constraint on a type that is not an object class field type or `INSTANCE OF`, or any constraint other than a simple table constraint on `INSTANCE OF` | X.682 10.1, A.2 | error |
| `E181` | Constraining object set not of the class named in the constrained field type | X.682 10.4, 10.15 | error |
| `E182` | Referenced component is not a value or value set field of the same class, constrained by the same object set as the referencing component | X.682 10.15 | error |
| `E183` | Referenced component lies in a different alternative of a `CHOICE` that contains the referencing component | X.682 10.14 | error |
| `E184` | Contents constraint (`CONTAINING`, `ENCODED BY`) on a type other than `OCTET STRING` or a `BIT STRING` without named bits, combined with any other constraint (directly or via a type reference), or with an `ENCODED BY` value that is not an OID | X.682 11.2–11.3 | error |
| `E185` | `CONSTRAINED BY` parameter not of a permitted form: a value or object without its governor, or a governor of the wrong kind | X.682 9.3 | error |

Effective tags follow X.680 exactly:

- An empty `TagDefault` means `EXPLICIT TAGS` (13.2), and a module's default applies only to types defined in that module, never to imported ones (13.1 Note 4).
- `[n] T` is explicit or implicit per 31.2.7. It is always explicit for an untagged `CHOICE`, open type or `DummyReference`, even under `IMPLICIT` or `AUTOMATIC TAGS`.
- Automatic tagging is decided per `ComponentTypeLists` or `AlternativeTypeLists`: it applies only if no component is textually tagged (25.2–25.3, 29.2). It runs after `COMPONENTS OF` expansion (25.4, 25.8), numbers root components from 0, and numbers extension additions from one past the largest root number (25.10).
- `EXTENSIBILITY IMPLIED` inserts an implied extension marker in every type that allows one (13.4).
- Distinct-tag checks first add the conceptual extension element of 52.7.

The worked examples in 29.9 and 52.8 become fixtures.

**OID collision (`W140`) is the most valuable author-facing check.** Build a global OID table from the corpus during the corpus build (value assignments of type `OBJECT IDENTIFIER`, evaluated to dotted form) and compare every draft OID against it. Report the RFC and name that already holds the arc.

### Resolved type model (public API)

**Decision:** decoding example encodings is out of v1, but `glint-sema` exposes the resolved type model it already builds for tag and relation checks as a documented public API, so a later decoder is an add-on, not a rewrite.

For every type reachable from a module, the model provides:

- Effective outer tag and tagging mode per component, after the module tag default, `AUTOMATIC TAGS` and untagged-`CHOICE` rules.
- Extensibility: root components, extension additions and version groups, in order.
- Constraints in evaluated form (value ranges, size ranges, permitted alphabets), plus the original syntax for display.
- Open types linked to their table constraint and the referenced component, so a decoder can pick the concrete type from an object set.
- `DEFAULT` and named values evaluated to concrete values (an OID as dotted arcs, a BIT STRING as bits).
- Parameterized definitions instantiated per use site, with each actual parameter's own tag environment applied (X.683 9.8), and memoized so recursive definitions stay finite.
- Table and component relation constraints: the constraining object set, the resolved path to each referenced component, and the constrained type (X.682 10.12–10.13). A decoder then applies the value rules of X.682 10.17–10.21, and treats a value missing from an extensible object set as not an error (X.681 E.2).
- Constrained `INSTANCE OF` as its equivalent sequence, with both components table-constrained and `value` related to `@.type-id` (X.682 A.2).

The model is serializable (`serde`) and covered by snapshot tests. Opaque legacy nodes and opaque external modules appear as explicit `Opaque` entries, never silently missing.

Planned follow-up (post-v1, M8): a directive such as `-- glint: decode Module.Type` above a hex, base64 or PEM block triggers DER decoding, type checking and DER canonical-form checks; plus `glint decode --type Module.Type <file>`.

## Document extraction

The extractor turns any supported input into a list of `Block { text, kind, source_map, origin }`, where `kind` is `Module`, `ModuleList` or `Fragment`, and `source_map` translates every byte offset in `text` back to a line and column in the original file.

| Format | How blocks are found | Pitfalls |
| --- | --- | --- |
| RFCXML v3 | `<sourcecode type="asn.1">`; also accept `asn1` and `ASN.1` case-insensitively with a warning | XML entities (`&lt;`) and CDATA change offsets: the source map must account for them. `markers="true"` adds `<CODE BEGINS>`/`<CODE ENDS>` lines to strip |
| RFCXML v2 (older drafts) | `<artwork type="asn.1">`, else `<artwork>` whose content contains `DEFINITIONS` and `::=` | Untyped artwork needs the heuristic classifier below |
| Markdown (kramdown-rfc, mmark) | Fenced code blocks with info string `asn.1` or `asn1` | Indented code blocks: apply the heuristic classifier. YAML front matter is ignored |
| Plain text (`.txt` drafts and RFCs) | Lines between `<CODE BEGINS>`/`<CODE ENDS>`; otherwise from a `... DEFINITIONS ... ::=` header to the matching `END`; otherwise indented runs whose first line matches `^\s*[A-Za-z][\w-]*\s*::=` | Page breaks: strip form feeds, running headers (`RFC 5280  PKIX Certificate and CRL Profile  May 2008`) and footers (`[Page 17]`). The source map must skip the removed lines |
| Bare ASN.1 | Whole file is one block | None |

Classify a block as `Module` when it contains a module header, `ModuleList` when it contains several, else `Fragment`. Record `origin` as section number and title where the format provides them, so diagnostics read "Section 4.1" rather than "line 812".

Extraction failures are diagnostics too (`I200` block skipped, `W201` block typed `asn.1` but does not look like ASN.1, `W202` text resembles ASN.1 but is not marked as `sourcecode type="asn.1"`).

## Snippet consistency

Every assignment in a `Fragment` block must match an assignment of the same name in one of the draft's modules, or, failing that, in a corpus module the draft imports or cites. A match compares normalized syntax trees, not text.

### Algorithm

1. Parse each fragment with the `AssignmentList` entry point. Parse failures are reported as normal syntax errors at the snippet's location.
2. For each assignment, look up its name in the draft's modules. If several modules define it (common when a draft carries both 1988 and 2002 syntax modules), compare against each and pass if any matches; report which one matched at info level.
3. If not found in the draft, look in corpus modules imported by the draft, then in the whole corpus. A snippet that quotes RFC 5280's `Certificate` is checked against RFC 5280. Not found anywhere: `W301 snippet defines X, which is not in any module`.
4. Normalize both sides using X.680 Annex C.3 ("identical type definitions"), in the context of the module that defines the assignment, whose `TagDefault` and `ExtensionDefault` also apply to the snippet:
   - remove comments (C.3.2.1);
   - apply the C.3.2.2 transformations: value-set assignments become constrained types; named numbers, enumerations, named bits and `CHOICE` roots are put in alphabetical order; OID components become numbers; extension parts move to the end; `IMPLICIT` is made explicit; the implied extension marker is added;
   - expand `COMPONENTS OF` and apply automatic tagging (C.3.2.3 c–d);
   - normalize values (C.3.2.5–C.3.2.6). Glint departs from C.3 in one way: type and value references are not expanded (C.3.2.3 f–g). The snippet must use the same reference names as the module.
5. Compare:
   - Normal forms differ → `E300`, with the first differing node as the primary label in the snippet, the module definition as the secondary label, and a short reason ("component `version` is `OPTIONAL` here but `DEFAULT v1` in the module").
   - Normal forms equal but the text differs (alternatives reordered, an OID written with names instead of numbers, `IMPLICIT` spelled out, the two `SEQUENCE SIZE(...) OF` forms of 49.5) → `W302`, because readers compare the text.
6. C.3 does not cover X.681–683 constructs (C.3.3). For those, compare the CSTs with trivia removed; any difference is `E300`.

### Elision and directives

Authors sometimes abbreviate snippets. A literal `...` is always an extension marker, never an elision. Editorial elision must be a comment, and the tool recognizes:

- `-- glint: partial` anywhere in a fragment: components in the snippet must appear in the module in the same order, but the module may have more.
- `-- glint: ignore` on a block: skip the block entirely.
- `-- glint: module=ModuleName`: compare only against that module.

Also make elision markers configurable (`elision_comments = ["-- (snip)", "-- ..."]` in config). When a line matching one is seen, treat the enclosing construct as partial.

### Reverse check (optional, off by default)

`W310`: a type defined in the module is never shown or mentioned in the body. Useful for reviewers, noisy for authors, so it is opt-in.

## RFC module corpus

The corpus is data, not code: `corpus-build` builds it every day, and it is published on the Glint site as `data/v<schema>/<build>/corpus.json` (with `documents.bin`, the texts of the documents, for the web UI) and a `data/v<schema>/latest.json` pointer. The build id is `<UTC date>.<hash of the content>`; code accepts exactly one schema, and every result names both the code version and the data build. The CLI and the web UI carry no data; `glint corpus update` downloads it. Automatic extraction plus a hand-curated override directory is the only workable design: many published modules have errata, and extraction from old text RFCs is lossy.

### Build pipeline

1. **Fetch** the RFC collection (RFC Editor rsync mirror or bulk download), `rfc-index.xml` for metadata (status, obsoletes/obsoleted-by, updates), the active Internet-Drafts with `all_id2.txt` (IETF rsync mirror), and errata data from the RFC Editor errata service.
2. **Extract** with `glint-extract`: XML for RFCs published in v3 format (RFC 8650 onward), text otherwise.
3. **Apply overrides** from `corpus/overrides/<document>/<ModuleName>.asn` plus a `meta.toml` (reason, errata IDs applied, curator notes); each entry names its `document` (`rfc5280`, `X.509`). An override replaces the module wholesale, whichever body published it, and keeps the module's document, edition and URL. A module is identified by document, name and OID, not by name: an RFC may define two modules with one name and different OIDs (RFC 4998 defines `ERS` for 1988 and for 1997 syntax). An override then carries `oid = "..."` (the identifier of the module it replaces, as extracted) and its file is `<document>/<ModuleName>-<oid>.asn`; without `oid` the override is refused if the document defines the name with more than one OID. An override that replaces no module of the corpus is refused. The `errata-verified` tier applies to RFCs only.
   When one RFC shows the same module more than once (a header in the body and the full text in an appendix), the build keeps the copy that parses without syntax errors, then among those the longest (a quoted header may have a placeholder body, as in RFC 4120), then the first.
4. **Parse and check** each module with no corpus (pass 1), then again with the partial corpus to resolve imports (pass 2). Record status: `clean`, `warnings`, `errors`, `unparseable`.
5. **Index** by module name (a name may map to several modules), by module OID, by every OID value assigned (for `W140`), and by the `name(number)` labels published modules write for each arc (the label table, for `W142`–`W144`; drafts are not evidence), and the entries of IANA's module identifier registries from `smi-numbers.xml` (for `W146`, and the registry note of `W102`; errata the registry cites are resolved with `errata.json`). Import resolution by name prefers a current RFC or a module of another body, then an active draft (newest first), then an obsoleted RFC.
6. **Emit** `data/v<schema>/<build>/corpus.json` and `documents.bin` with `.sha256` files, the `latest.json` pointer, and a human-readable `report.md` listing every module, its status and its diagnostics, and an audit of every module against the label table. The report is the curator's to-do list.

### Override policy

**Decision:** overrides live in a public GitHub repo and arrive as pull requests. Each `meta.toml` declares one of three tiers, and the tier sets the review bar.

| Tier | Fixes | Review to merge |
| --- | --- | --- |
| `extraction` | Page breaks, whitespace, other extractor mistakes | CI proves the override's tokens equal the RFC text with page artifacts removed; any contributor may merge once green |
| `errata-verified` | Corrections from RFC Editor errata with status Verified | Must cite the errata ID; CI checks the status via the errata data; one maintainer approves |
| `correction` | Reported or held errata, or obvious typos with no erratum | Written justification in `meta.toml`; two maintainers approve, ideally one from the owning working group |

- `CODEOWNERS` maps override paths to maintainers; initially the project owner owns everything, with area experts added later (e.g. LAMPS reviewers for PKIX modules).
- `meta.toml` fields: `tier`, `rfc`, `module`, `errata_ids` (list), `justification` (required for `correction`), `author`, `date`.
- Transparency: whenever a check uses an `errata-verified` or `correction` module, emit `I105 using curated version of <Module> from <document> (<tier>, errata <ids>)` once per module. `extraction` overrides are silent.
- `report.md` lists every override with its tier, so readers can audit how far the corpus departs from the published text.

### Module record

| Field | Notes |
| --- | --- |
| `name` | e.g. `PKIX1Explicit88` |
| `oid` | Dotted form of the definitive identifier, if any |
| `source` | The standards `body` (`rfc`, `itu-t`, `etsi`, ...), the `document` within it (`rfc5280`, `X.509`), its `edition` and source `url` where it has them, the section, and whether the text was extracted, taken from a standalone file, or overridden. Every body's modules are records of this one shape |
| `rfc_status` | Standards track, informational, etc.; obsoleted-by list |
| `syntax_era` | `1988` (uses `ANY`/`MACRO`), `2002+` (uses X.681), or `neutral` |
| `text` | The exact module text used |
| `exports` | Explicit list or `ALL` |
| `parse_status` | From step 4 |
| `sha256` | Of `text`, for reproducibility |

### Import resolution order

1. **OID first.** When IMPORTS gives an `AssignedIdentifier`, it is definitive (13.11) and the module name may differ (13.10). Evaluate a value-reference identifier per 13.9 first. Look for the OID in the draft's own modules, then in the corpus.
2. **Version selection (13.16 f).** `WITH SUCCESSORS` picks the module whose OID equals the import's with the last arc incremented zero or more times, greatest wins. `WITH DESCENDANTS` picks the module at or below the import's node with the largest OID, comparing arc by arc and preferring the longer OID on a tie.
3. **Name only.** With no `AssignedIdentifier`, match by name in the draft, then in the corpus. If several corpus modules match, prefer one from the importer's own document, then not obsoleted, then the most recent document (RFCs are ordered by number; other bodies have no order and fall through), then the largest OID (editions of a module differ in the last arc, so the larger is the later one), and emit `I103` naming the choice. Emit `W106` when the chosen module has a definitive OID (13.12).
4. **Pins.** Author pins in config (`[pins] PKIX1Explicit88 = "rfc5280"`) override 1–3. A pin is a document id (`rfc5280`, `X.509`) or the module's dotted OID (`ERS = "1.3.6.1.5.5.11.0.1.1"`), the latter picking one of several modules of the same name, including two in one document.
5. **Exports.**
   - An explicit `EXPORTS` list limits what can be imported.
   - An empty `Exports` or `EXPORTS ALL` exposes everything defined or imported, except symbols imported more than once (13.13–13.14).
   - A symbol the target itself imports can be re-imported only if it appears once there and is not defined there (13.16 a).
   - Named numbers, enumerations and named bits travel with their type (13.15, 13.18).

The same module name is defined in several RFCs (e.g. PKIX modules in RFC 3280 and RFC 5280, and 2002-syntax rewrites in RFC 5912). That is why X.680 makes the OID definitive and Glint offers pins. The module name is only a reference within one module's scope, so Glint never uses it alone as a key: the corpus holds every module it finds, whatever its name, and `by_oid` is the exact lookup.

### Modules published as files of their own

RFC modules import modules that other standards bodies publish as standalone files: ITU-T X.500-series modules (`InformationFramework`, `AuthenticationFramework`, etc.), and in time ETSI and others. **Decision:** these are corpus modules like any other. The only thing that differs by body is how the text arrives: RFC modules are extracted from prose; other bodies' modules are downloaded as `.asn` files.

- `corpus/sources.toml` lists them. A `[[body]]` names a standards body (`id`) and carries the `notice` that credits it. A `[[module]]` has `body`, `document`, `edition`, `name`, `oid`, `url` and `include`. Adding a body or a document means adding entries there; no code changes.
- The OID identifies an entry: editions of a document reuse the module name and change the OID, so each edition is its own entry.
- A download is used, and cached, only when it is the module the entry lists: an ASN.1 module whose own identifier is the entry's `oid`. An HTML page (a rendering of the module, or an error page served with status 200), a file of another edition, a failed download or a module that cannot be read fails the build, naming the entry, its URL and the reason, before anything is written: the published data never silently lacks a listed module. A cached file that is not the module is downloaded again.
- `include = "when-imported"` (the default) bundles a module only when some corpus module imports it and nothing in the corpus satisfies the import, found by walking unresolved imports after pass 1. An import with a literal `AssignedIdentifier` takes the entry with that OID; an import without one takes the entries of that name while the corpus has no module of that name. `include = "always"` bundles it in every build.
- `corpus-build` downloads the files at build time into `--cache`, as the bytes received; they are never committed to the repo. A file that is not UTF-8 is read as Windows-1252 (older ITU-T files are), so copyright notices keep their characters.
- Each record keeps its body, document, edition and source URL, with `kind = "standalone"`.
- The data ships the IETF Trust BSD notice for RFC code components and the notice of each body that has a module in the corpus (the ITU-T credit, for ITU-T).
- Redistribution: the owner has decided to redistribute these modules in the published data without seeking separate permission. Keep each body's credit notice and each module's source URL.

Any import that resolves to neither the draft nor the corpus falls back to an **opaque external module**: its imported symbols are accepted with unknown kind, checks that need their definitions are skipped, and `W100` is reported once per module.

## Diagnostics

One `Diagnostic` type feeds every renderer, so the CLI, API, SARIF and editor all show identical findings.

```rust
pub struct Diagnostic {
    pub code: &'static str,        // "E101"
    pub severity: Severity,        // Error | Warning | Info
    pub message: String,           // one line, no trailing period
    pub primary: Label,            // where the problem is
    pub secondary: Vec<Label>,     // e.g. the module definition a snippet disagrees with
    pub notes: Vec<String>,        // context: "resolved to RFC 5912"
    pub help: Option<String>,      // a fix the author can apply
}

pub struct Label {
    pub file: FileId,              // the draft, or a corpus module
    pub span: Range<usize>,        // byte offsets in the ORIGINAL file
    pub line_col: (u32, u32),      // 1-based, precomputed via the source map
    pub section: Option<String>,   // "Section 4.1" / "Appendix A"
    pub text: Option<String>,
}
```

Renderers: `human` (colored, with source excerpts, via `ariadne` or `miette`), `json` (schema versioned with a `schema_version` field), and `sarif` (SARIF 2.1.0, with rule metadata from the code registry). Labels pointing into corpus modules render as `rfc5280#PKIX1Explicit88:120`. Severity can be remapped per code in config.

## CLI

The binary is `glint`, built with `clap`. It carries no corpus: `glint corpus update` downloads the data once, and checks then work offline.

| Command | Purpose |
| --- | --- |
| `glint check <files...>` | Extract, parse, check; format auto-detected from extension and content (`-` reads stdin) |
| `glint check --draft draft-ietf-lamps-foo-03` | Fetch a draft from the IETF archive, then check |
| `glint extract <file> -o <dir>` | Write each extracted module to `<dir>/<ModuleName>.asn`; when the document defines several modules of one name, each goes to `<ModuleName>-<oid>.asn`, as overrides name them |
| `glint extract <file> --format json [-o <dir>]` | List the blocks on stdout as JSON: kind, section, title, line range, and each module's name, dotted OID (`null` unless every arc is a number) and line range; with `-o`, also the file each module (and, with `--fragments`, each fragment) was written to, holding its text as the corpus build records it |
| `glint corpus list [--filter <text>]` | List modules with RFC, OID and status |
| `glint corpus show <name\|oid\|document>` | Print a module's text and metadata; a document is `rfc5280`, `X.509`, ... |
| `glint corpus oid <dotted-oid>` | Who assigns this OID, and its parent arcs |
| `glint corpus update` | Download the latest published data (or `--build <id>`) to the user cache; without data, commands that need it say to run this |
| `glint explain <code>` | Long description of a diagnostic code with examples |

Common flags: `--format human|json|sarif`, `--config <path>` (default: nearest `glint.toml` upward from the input), `--deny-warnings`, `--corpus <corpus.json>`, `--pin Name=rfcNNNN|<oid>`, `--no-snippets`, `--color auto|always|never`.

Exit codes: `0` no errors, `1` errors found (or warnings with `--deny-warnings`), `2` usage or I/O failure.

### Config file (`glint.toml`)

```toml
[pins]
PKIX1Explicit88 = "rfc5280"

[severity]
W112 = "off"        # unused imports
I141 = "warning"    # make TBD OIDs louder before IESG

[snippets]
elision_comments = ["-- (snip)", "-- ..."]
reverse_check = false

[oid]
placeholders = ["TBD", "TBD1", "TBD2"]
```

Also ship a composite GitHub Action (`glint-action`) that downloads a release binary, runs `glint check --format sarif`, and uploads the result for code-scanning annotations.

## Web: static site and frontend

**Decision: a static site; Glint runs in the browser.** The page loads `glint.wasm` (the `glint-wasm` crate) in a Web Worker and the published data beside it: the `latest.json` pointer and the `corpus.json` it names while the wasm compiles, then `documents.bin` in the background. Nothing is sent anywhere, so unpublished drafts stay on the author's machine, and there is no server to run: nginx serves files. A draft or RFC is opened by name from `documents.bin` instead of being fetched from the IETF (which a browser could not do across origins).

### In-browser API

`web/src/api.ts` is the page's interface to the worker; each call answers with the JSON of the matching method of `glint-wasm`.

| Call | Request | Response |
| --- | --- | --- |
| `check` | `{ content, name?, format?, options: { pins?, severity?, snippets? } }` | `{ glint_version, data_build, document, files[], blocks[], modules[], diagnostics[] }` |
| `checkDocument` | `{ draft: "draft-ietf-lamps-foo", rev?: "03" }` or `{ rfc: 5280 }` | Same as `check`, plus the document's text |
| `modules(q, offset, limit)` | Search by name or OID substring | `{ total, offset, limit, modules: [...] }` |
| `module(source, name, oid?)` | `oid` picks one of several modules of that name in the source | Module text and metadata |
| `oid(dotted)` | — | Labels, assignments of this OID and its children across the corpus |
| `corpus()` | — | Code version, data build, counts |
| `complete(text, name, format, position)` | — | Completions at the position: module names after `FROM`, exported symbols in `IMPORTS`, known arcs in an OID, the module's names |

Data of another schema, or a file of another build than the pointer names, is refused and logged; pasted text can still be checked.

### Frontend

- **Stack:** Vite + TypeScript, no heavy framework needed (Svelte or plain TS components are fine). CodeMirror 6 with a small ASN.1 syntax mode (Lezer grammar or stream parser) for highlighting.
- **Styling:** Bootstrap 5 for layout, components and theming. Load it from a CDN (pin the version and add Subresource Integrity hashes to the `<link>` and `<script>` tags). Use its grid and utilities for the two-pane layout, `nav-tabs` for the right-pane tabs, `list-group`/`table` for diagnostics and module lists, and its color modes (`data-bs-theme`) for light/dark. Use Bootstrap's JavaScript only where needed (tabs, collapse), not jQuery; custom CSS is limited to CodeMirror theming and diagnostic severity colors.
- **Layout:** left pane is the document editor; right pane has tabs for Diagnostics, Modules (extracted, with parse status), and Imports (each import and which RFC it resolved to, linked to the corpus viewer).
- **Inline feedback:** diagnostics become CodeMirror lint markers; clicking a snippet mismatch jumps to the module definition (secondary label).
- **Inputs:** paste, file upload, or draft name/RFC number box. Re-check on demand plus debounced auto-check (800 ms) for documents under 1 MB; larger ones show a notice and wait for the Check button.
- **Corpus browser:** search box over `modules`, modules open in the checker at their start line, and an OID lookup page.
- **Shareable state:** keep the draft name in the URL hash (`#draft-ietf-lamps-foo-03`); never put pasted content in the URL.

### Hosting

**Decision: self-host first, built for adoption by the IETF Tools Team.** The IETF [Author Tools web service](https://authors.ietf.org/author-tools-web-service) already chains xml2rfc, kramdown-rfc and idnits and invites new tasks, and the [tools catalog](https://authors.ietf.org/tools-catalog) lists no ASN.1 validator.

1. **Now:** a static site on any small host the owner controls: each code release is a directory switched in with a symlink, and the data is published daily beside it (`deploy/`).
2. **Adoption-ready from day one:** license the project Apache-2.0; static files only, so any web server can host it; no host-specific code.
3. **After M5:** propose it to the IETF Tools Team, either as an Author Tools task ("Validate ASN.1" beside idnits) or as a standalone service on their infrastructure.

## Testing

The RFC corpus is the main test suite: the parse rate over all extracted modules is the headline metric, tracked per commit.

| Layer | Method |
| --- | --- |
| Lexer and parser | Unit tests per production, named after the X.680 clause; snapshot tests of CST dumps with `insta` |
| Grammar coverage | Fixtures keyed to the productions summarized in X.680 Annex L (informative; clause text wins), every row of the feature table in section 4, the Recommendation's worked examples (29.9, 52.8, C.3), X.681 Annex D, the X.682 clause 10 and Annex A examples (including `@...errorId`), and the X.683 examples (8.5, the 9.8 tagging modules with their exact expected tags, and Annex A) |
| Corpus regression | Parse and check every corpus module in CI; fail if the clean count drops or any previously clean module regresses |
| Differential | Compare parse acceptance on the corpus against existing open-source ASN.1 tools (e.g. Python `asn1tools` or `pycrate`, `asn1c`); investigate every disagreement |
| Semantics | One fixture pair (bad, good) per diagnostic code; the `explain` text uses the same examples |
| Extraction | Real drafts in each format (RFCXML v2/v3, kramdown-rfc, text) with expected block lists and source-map round-trip tests (every mapped offset points at the same character) |
| Snippets | Synthetic drafts with deliberate mismatches: renamed component, changed tag, dropped `OPTIONAL`, changed constraint, elided components with and without `partial` |
| Robustness | `cargo-fuzz` targets for lexer, parser and extractor; no panics, bounded time on 1 MB input |
| Web engine | Native tests of `glint-wasm` (checks by name, corpus queries, completion); the page's tests use a stand-in for the worker |

Also evaluate the `rasn-compiler` crate early: as a reference for tricky grammar cases, and to decide whether any part is worth reusing. The design assumes an owned parser because spans, recovery and the lossless tree are central.

### Blind test: RFC 5280

RFC 5280 is known to contain at least one divergence between ASN.1 quoted in the prose of Section 4 and its Appendix A modules. This spec deliberately does not say what it is: the test measures whether Glint finds it unaided.

- **Fetch:** an integration test downloads the plain-text RFC 5280 from the RFC Editor, verifies a pinned SHA-256 (record the hash on first download), and caches it in a git-ignored directory so later runs work offline. Gate it behind a `net-tests` feature so ordinary `cargo test` needs no network.
- **Run:** check the document as an author would, through the text extractor, using the document's own Appendix A modules. Neither the corpus copy of RFC 5280 nor any override may be involved.
- **Pass:** at least one `E300` whose primary label lies in Section 4 and whose secondary label lies in Appendix A. The test also writes every snippet finding (`E300`, `W301`, `W302`), with sections and lines, to a report for review.
- **Review:** the owner checks the report. Once the finding is confirmed and any false positives are fixed, the report is frozen as an `insta` snapshot and becomes a regression test.
- **Implementer rules:**
  - While building M1–M5, do not read RFC 5280's errata or search for known divergences in it.
  - Do not add code, directives, config or fixtures specific to RFC 5280.
  - If the test fails, fix the general extractor or comparison logic, not the test.

## Milestones

Seven milestones; M1–M5 are sequential, the web track (M6) can start once M3 lands, and M7 packages everything.

&#91;embedded content: milestone order · 7 milestones, one parallel track\]

1. **M1 — X.680 core syntax.** Lexer, recursive-descent parser for X.680 without X.681–683, lossless CST, error recovery, human diagnostics, a dev command `glint parse <file.asn>`.
   - Done when: every X.680 grammar fixture passes; the RFC 5280 1988-syntax modules (hand-copied) parse with zero errors; a 1-hour fuzz run finds no panics.
2. **M2 — Extraction.** RFCXML v2/v3, Markdown and text extractors with source maps; `glint extract`.
   - Done when: source-map round-trip tests pass for all formats; RFC 5280 (text) and at least one v3-XML RFC with a module extract exactly the expected modules; page headers and footers inside a module are removed.
3. **M3 — Corpus, imports, core semantics.** `corpus-build`, bundle format, overrides, `report.md`, import resolution, checks `W100, E101`–`E132` and `W140`, `glint check`, `glint corpus ...`.
   - Done when: the bundle builds from the full RFC set and `report.md` records a parse-rate baseline; a sample draft importing PKIX modules resolves with no user-supplied files; the tagging test matrix passes; an OID already assigned in RFC 5280 triggers `W140`.
4. **M4 — X.681, X.682, X.683.** Two-phase ambiguity resolution, `WITH SYNTAX` objects, parameterization, checks `E150`–`E169` and `E180`–`E185`.
   - Done when: the RFC 5912 modules parse and check clean (or clean with documented errata overrides); the corpus clean count rises over the M3 baseline; the resolved type model API is published, with snapshot tests for the RFC 5280 and RFC 5912 types.
5. **M5 — Snippet consistency.** Annex C.3 normal-form comparison, `W302`, `E300`/`W301`, directives, elision config, optional `W310`.
   - Done when: the synthetic mismatch suite passes; the RFC 5280 blind test (section 12) finds a Section 4 divergence and the owner confirms it; five recent LAMPS working group drafts run with no unexplained false positives.
6. **M6 — Server and web UI** (after M3, in parallel with M4–M5). `axum` API, draft fetching, TypeScript frontend with Bootstrap and CodeMirror, corpus browser, OID lookup.
   - Done when: API integration tests pass, including body-limit and timeout tests; a draft checked by name shows inline markers and import links end to end.
7. **M7 — Distribution.** JSON and SARIF output, `glint explain`, release binaries for Linux/macOS/Windows, the GitHub Action, `corpus update`, Apache-2.0 license files, and the web UI as a static site running a WASM build.
   - Done when: a sample i-d-template repo shows SARIF annotations on a pull request; release artifacts build in CI.

## Notes for the implementer, risks, open questions

### Rules for the implementing model

- Work one milestone at a time. Finish its acceptance criteria and tests before starting the next.
- Do not invent ASN.1 rules. Every X.680–X.683 clause number in this spec was checked against the 02/2021 editions. When unsure, write a failing test with a `TODO(spec)` comment quoting the unclear clause, and ask.
- Put a one-line clause reference (`// X.680 25.1`) on every parser function and semantic check. This is the one standing exception to minimal comments.
- Keep `glint-syntax`, `glint-sema`, `glint-check` and `glint-extract` free of I/O, `tokio` and global state.
- Follow the code conventions below in every crate, including tools and tests where stated.
- Prefer widely used crates; justify each new dependency in the PR description and add it only to `[workspace.dependencies]` (convention 4).
- Every diagnostic code gets a fixture, an `explain` entry, and a row in `docs/diagnostics.md`.
- Never hand-edit the generated corpus data; fix the extractor or add an override.

**Normative sources.** `spec/fetch.sh` downloads the pinned 02/2021 editions of X.680–X.683, plus X.660, into a git-ignored `spec/` directory; never commit them, and do not use the older 2002 texts. Clauses to consult per milestone: M1 — X.680 clauses 5, 11–13, 16–54 and Annex L; M3 — X.680 clauses 8, 13–14, 25–33, 49–53 and Annex C; M4 — X.681 clauses 7–15 and Annexes A–C and F, X.682 clauses 8–11 and Annex A, then X.683 clauses 8–10 and Annex A; M5 — X.680 Annex C.3.

### Code conventions

1. **Minimal comments.** If code needs a comment to be understood, improve the code instead: better names, smaller functions, types that encode invariants. Allowed comments: clause references, `INFALLIBLE:` justifications, `TODO(spec)` markers, and doc comments on public items of library crates.
2. **No panics, except justified ones.** Production code must not panic. That covers `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`, slice indexing, and unchecked arithmetic that can overflow.
   - Enforce with workspace lints in the root `Cargo.toml`: deny `clippy::unwrap_used`, `expect_used`, `panic`, `unreachable`, `todo`, `unimplemented`, `indexing_slicing`, and `arithmetic_side_effects`.
   - Where an invariant truly guarantees success, keep the call but mark it: an `// INFALLIBLE: <why the invariant holds>` comment directly above, plus `#[expect(clippy::<lint>)]` on that statement. A reviewer must be able to verify the reason locally.
   - Prefer restructuring over `INFALLIBLE`: return `Option`/`Result`, use iterators instead of indices, use `checked_*` arithmetic.
   - Test code (`#[cfg(test)]`, `tests/`, fuzz targets) may panic freely.
3. **No backward compatibility between iterations.** Until a 1.0 release, change APIs, CLI flags, config keys, JSON output and the data format freely. No deprecation shims, migration code, or compatibility flags. Update all callers, fixtures and docs in the same change. Still bump `schema_version` and the data schema when they change, so mismatched files fail loudly.
4. **All dependencies and versions live in the workspace root.** Every external crate, its exact version and its default features are declared once under `[workspace.dependencies]` in the root `Cargo.toml`. Member crates write only `name = { workspace = true }`, optionally adding `features = [...]` or `optional = true`; a version or path in a member `Cargo.toml` is a review failure. Internal crates are listed there too, by path. Shared `[workspace.package]` fields (edition, license, repository, rust-version) and `[workspace.lints]` are inherited the same way. Commit `Cargo.lock`.
5. **Use well-known crates instead of hand-rolled code.** Before writing infrastructure, check for an established crate and use it. Examples: `logos` for tokenizing, `rowan` for the syntax tree, `roxmltree`/`quick-xml` for XML, `pulldown-cmark` for Markdown, `clap` for arguments, `serde`/`serde_json` for serialization, `ariadne` or `miette` for diagnostic rendering, `serde-sarif` for SARIF types, `base64`, `hex` and `pem` for encodings, `zstd` for compression, `similar` for text diffs, `toml` for config, `thiserror` for error types, `insta` for snapshots, `proptest` for property tests, `tower-http` for limits, timeouts and static files.
   - "Well known" means widely downloaded, actively maintained, and permissively licensed (compatible with Apache-2.0). Check with `cargo deny` in CI for licenses and advisories.
   - Hand-rolling is allowed only where the crate cannot meet a requirement in this spec. Record why in the PR description; a short justification beats a comment.
   - Standing exception: the ASN.1 parser itself and the semantic analysis are hand-written by design (section 4), because spans, error recovery and the lossless tree are the product. Their building blocks (`logos`, `rowan`) still come from crates. Evaluate `rasn-compiler` before writing any part of the X.681–683 handling, and reuse anything that fits.

### Risks

| Risk | Mitigation |
| --- | --- |
| X.681 ambiguity and `WITH SYNTAX` objects make the parser balloon | Two-phase parse (section 4); land X.680 core first, then X.681–683 |
| Text-RFC extraction is lossy (page breaks inside modules) | Source maps plus overrides; report shows every unparseable module |
| Tagging rules are subtle and easy to get almost right | Dedicated test matrix; differential tests against other tools |
| Published modules contain errors | Curated overrides with errata IDs; corpus errors never block a user's check, they appear as notes |
| Snippet comparison is too strict for real drafts | Normalization rules, `partial` directive, configurable elision markers; tune against real drafts |

### Open questions for the project owner

- [x] Non-RFC modules — decided: standards bodies' standalone modules (ITU-T, ETSI, ...) are corpus modules like RFC ones, listed in `corpus/sources.toml`, fetched at build time and redistributed in the bundle with a credit notice per body; by default only those RFCs import are bundled; unknown modules become opaque (`W100`).
- [x] MACRO support — decided: opaque definitions and uses; `E172` for new macros in drafts, `W173` for uses.
- [x] Override curation — decided: public repo, PRs, three review tiers, `I105` transparency note (section 8, Override policy).
- [x] Hosting — decided: a self-hosted static site (code releases plus daily data), Apache-2.0, proposal to the IETF Tools Team after M5 (section 11, Hosting).
- [x] Example encodings — decided: not in v1; the resolved type model is a public API now, and a decoder is planned as M8 (section 5, Resolved type model).
