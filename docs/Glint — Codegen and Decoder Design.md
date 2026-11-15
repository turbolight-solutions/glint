# Glint — Codegen and Decoder Design

Nov 15, 2026 · @C B

## Purpose and scope

Glint has a complete ASN.1 frontend: parser, import resolution against the corpus, semantic checks and a resolved type model (`glint_sema::typemodel`). This spec adds two backends on top of it:

1. **Codegen.** Emit the types reachable from a chosen root type as source for four libraries: **rasn**, **RustCrypto `der`**, **pyasn1** and **Go cryptobyte**. Each output carries a vendored runtime file with the helpers the generated code needs (constraint checks, tag reading, time parsing).
2. **Schema-driven decoder.** Given a module, a type and an encoding (PEM, base64 or DER), show every TLV of the input the way `openssl asn1parse` does, enriched with field names, type names, rendered values, OID labels from the corpus, constraint violations and DER violations.

3. **Example artifacts.** Extract the PEM examples published in RFCs and drafts, identify their types, and serve them for download, each ready to open in the decoder. Decoding every published example also makes the corpus a test suite for the decoder.

All three are available from the CLI and from the static web app.

### Goals

1. **Full fidelity.** Constraints, tags, extensibility, `DEFAULT` values, named numbers and bits, and open types bound by information object sets survive into generated code. Whatever a target cannot express is reported, never silently dropped.
2. **One model, many consumers.** Every backend and the decoder read the same IR, so they cannot disagree about tags or constraints.
3. **The decoder is the oracle.** Generated code is tested by decoding the same vectors as `glint-decode` and comparing the trees.
4. **Same entry points as `check`.** Modules come from the corpus (`source#Name`) or from user ASN.1 (a local file or the checker editor), with imports resolved the way `glint check` resolves them.
5. **Open to new targets.** Adding a backend means writing one emitter module and registering it. The shared layers (IR, naming, constraint planning, runtime contract, tests) don't change, and the CLI and web pick it up from the registry.

### Non-goals (this spec)

- PER, OER, XER and JER. The IR keeps PER-visible constraints separate so a PER/OER decoder can be added later.
- Macro-notation modules (X.208 `MACRO`). Their types stay opaque, as in the type model.
- Published runtime packages (crates.io, PyPI, Go modules). The runtime is vendored per output; it is written as a self-contained library so it can be published later.

## Decisions

| Topic | Decision |
| --- | --- |
| Module source | Corpus modules and user ASN.1 (local file, stdin or the checker editor); imports resolved as `check` resolves them |
| Emit scope | One root type, tree-shaken: the type plus everything it reaches, across modules |
| Open types | Typed dispatch enums generated from the object set's table, with an `Unknown` variant when the set is extensible |
| Decoder encodings | BER accepted; each TLV that breaks DER is flagged |
| Constraint enforcement | On decode and encode: a value held by generated code is always valid |
| Direction | Generated code decodes and encodes |
| Runtime delivery | A vendored, version-stamped `glint_rt.{rs,py,go}` per output |
| Web layout | One `#/workbench` route: a shared picker, then Decode, Artifacts and Codegen tabs |
| Decoder on mismatch | Best effort: mark the TLV, decode its subtree without a schema, carry on with its siblings |
| Parameterized types | Monomorphized per instantiation (the type model already expands them, X.683 9.8) |
| Target library versions | The latest release of each library, recorded in backend metadata and bumped deliberately |
| New targets | A backend registry plus a shared middle layer; adding a target needs no change outside its own module |
| Runtime license | MIT-0, so users can vendor the runtime without carrying notices |
| Conventions | Glint's own; no compatibility with rasn-compiler's output |
| 1988 modules without tables | The decoder (not codegen) falls back to OID-keyed hints from the corpus, marked as heuristic |
| Examples in drafts | `check` decodes the PEM examples in a draft against the draft's types and reports findings |
| Artifacts | PEM blocks from every RFC and active draft, extracted at corpus-build time, identified by try-decode, published as a data file, served from an Artifacts tab |
| Build order | IR → decoder → artifacts → rasn → pyasn1 → der → Go |

## Architecture

```
glint-sema ── typemodel ──lower──▶ glint-ir (data types only, serde, no deps on sema)
                                        │
                     ┌──────────────────┴──────────────────┐
               glint-codegen                          glint-decode ◀── glint-extract::artifacts
               registry + middle layer                BER/DER walker      (PEM scanner)
               rasn | der | pyasn1 | go | ...         → TLV tree + findings       │
                     │                                       │             corpus-build:
          glint codegen (CLI)                  glint decode / artifacts    artifacts.json
                     └──── glint-wasm: codegen(), decode(), artifacts() ──────┘
```

- **`glint-ir`** holds the IR types only. Backends and the decoder depend on it, not on `glint-sema`, so they never touch the CST.
- **Lowering** lives in `glint-sema` (a new `ir` module next to `typemodel`), because it needs `Evaluator`, `Sem` and the tagger. It extends the type model's builder rather than duplicating it. `ConstraintNode` stays the display form; the IR constraint is derived from the same nodes.
- **`glint-codegen`** has a backend registry, a shared middle layer (below) and one module per backend. Emitters are hand-written pretty printers, with no template engine, to keep the wasm small.
- **`glint-decode`** has no I/O. Input sniffing (PEM, base64, DER) lives here too, so the CLI and wasm share it.
- **`glint-extract::artifacts`** finds PEM blocks in document text and XML. It sits next to the module extraction and reuses its line model (page breaks, RFC 8792 folding).

## The IR

### Type set

An `IrSet` is the closure of one root type: a map from stable type ids to definitions, with cycles allowed (`Name ::= SEQUENCE { next Name OPTIONAL }`). Each definition records:

- its origin: module name, module OID, document id, and the ASN.1 name;
- its **kind**: `Boolean`, `Integer`, `Enumerated`, `Real`, `Null`, `BitString`, `OctetString`, `ObjectIdentifier`, `RelativeOid`, the string types, the time types, `Sequence`, `Set`, `Choice`, `SequenceOf`, `SetOf`, `OpenType`, `Reference`, `Opaque(reason)`;
- its effective **tags** and tagging mode, already resolved (`IMPLICIT`/`EXPLICIT`/`AUTOMATIC`, X.680 31.2);
- named numbers and named bits;
- **components**: name, optional, `DEFAULT` value, root or extension-addition region, version group;
- **constraints** (below);
- an **extensible** flag (the `...` marker or `EXTENSIBILITY IMPLIED`).

Each instantiation of a parameterized type gets its own type id, named after the definition and its actual parameters (`AlgorithmIdentifier{DIGEST-ALGORITHM, {HashAlgs}}` → `AlgorithmIdentifierHashAlgs`). The original name is kept for display.

### Constraints

The full algebra of X.680 51 and X.682, evaluated:

```rust
pub struct Constraint {
    pub root: ElementSet,
    pub extensible: bool,
    pub additions: Option<ElementSet>,
    pub exception: Option<Value>,
}

pub enum ElementSet {
    All,
    Union(Vec<ElementSet>),
    Intersection(Vec<ElementSet>),
    Except(Box<ElementSet>, Box<ElementSet>),
    Value(Value),
    Range { lo: Bound, hi: Bound },          // Bound: Min | Max | Inclusive(v) | Exclusive(v)
    Size(Box<Constraint>),
    From(Box<Constraint>),                   // permitted alphabet
    Pattern(String),
    ContainedSubtype(Box<Type>),
    WithComponent(Box<Constraint>),
    WithComponents { partial: bool, items: Vec<ComponentConstraint> },
    Contents { ty: Option<Box<Type>>, encoded_by: Option<Value> },
    Table(TableRef),                         // { set: ObjectSetId, field, relation: Vec<at-path> }
    Settings(String),                        // SETTINGS of the time types
    UserDefined(String),
    Opaque(String),                          // evaluation failed; the text, for a comment and a finding
}
```

Contained and contents types are inline `Type`s (usually a reference to a named type), so a `CONTAINING` an open type keeps its table in place. A table constraint on an open type is not repeated as a constraint: it is the open type's `table`, the dispatch. `DEFAULT` values written by component (`sha1Identifier`) are evaluated to `Value::Record`.

`glint ir <module> <Type> [FILE...] [--json]` (a development command) prints the IR of a root type: a corpus module as `source#Name` or a bare name only one document defines, or a module of the given files. The text form lists every type definition, every object set table, the OID constants and the notes; `--json` prints the `IrSet` itself.

The IR also computes an **effective summary** per type, so backends never have to do set arithmetic just to pick a native type:

- integer value range (selects `u8`/`i32`/`i64`/bignum and the Go and Python equivalents);
- size range for strings and `SEQUENCE OF`;
- effective permitted alphabet;
- the PER-visible subset (X.691 8.3), kept for a later PER/OER decoder.

The summary is advisory; the full constraint is what is enforced.

### Information object sets

Object sets that bind open types are lowered to tables. Each row holds the value of the unique field (usually an OID, possibly an integer) and the type of each type field (`&Type`, `&Params`, `&ExtnType`, ...). Rows come from every object reachable from the set, including imported ones and those inside `CONTAINING` (X.682 11.1, as in `extnValue OCTET STRING (CONTAINING EXTENSION.&ExtnType({ExtensionSet}{@extnID}))`). The table records whether the set is extensible, and records `@` relations as paths from the open type to the key component.

An open type gets a **dispatch type** per (class field, object set) pair. Codegen emits it as an enum keyed by the unique field; the decoder looks the key up in the table.

### Values

OID values, integers, named values and `DEFAULT`s are constants in the IR. Codegen emits named OID constants (`id-ce-keyUsage` → `ID_CE_KEY_USAGE`) for every OID reachable from the root, the table keys included.

### Naming

Every backend has its own namer with three jobs: case conversion (`Rust: UpperCamel types, snake fields; Python: UpperCamel classes, lowerCamel field names kept as in ASN.1; Go: UpperCamel exported`), keyword escaping, and a collision pass over the whole set. Names are deterministic (no hash suffixes), so regenerating gives a clean diff. Each ASN.1 module maps to one output module; imports become module paths.

## Decoder

### Input

`sniff(bytes) -> Vec<Input>`:

1. PEM armor: base64-decode each block and keep its label. Several blocks become several inputs; the CLI decodes all of them and the web lets the user pick.
2. Base64 alphabet only (whitespace ignored): decode it.
3. Otherwise the bytes are DER/BER.

`--input` (CLI) or a dropdown (web) overrides sniffing. A PEM label suggests a default type (`CERTIFICATE` → `Certificate`, `PRIVATE KEY` → `OneAsymmetricKey`, ... per RFC 7468), used when no type is given.

### Output

```rust
pub struct Tlv {
    pub offset: usize,
    pub header_len: usize,
    pub len: Length,                 // Definite(n) | Indefinite
    pub end: usize,                  // end-of-contents octets included
    pub tag: Option<Tag>,            // class, number, constructed; None for octets that are not a TLV
    pub tagging: Option<TagMode>,    // a tag the module writes: EXPLICIT or IMPLICIT
    pub field: Option<String>,       // component name, or component.alternative for a CHOICE
    pub type_name: Option<String>,   // Module.Type as declared, with actual parameters
    pub value: Option<Rendered>,
    pub children: Vec<Tlv>,          // constructed contents, or the value a CONTAINING octet string holds
    pub findings: Vec<DecodeFinding>,
    pub schema: SchemaState,         // Matched | Unmatched (decoded without a schema) | Hinted { from }
}
```

`Rendered` values:
- integers: decimal, plus hex for large ones, plus the named number;
- `ENUMERATED`: the name;
- bit strings: the set bits by name, plus unused bits;
- OIDs: dotted form plus the names the corpus assigns them (`oid_assignments`), with module and document, then the OID constants of the IR;
- times: parsed and normalized;
- strings: decoded text, with a finding when it is not valid for the type;
- octet strings: hex, with `CONTAINING` and open-type contents decoded as children.

`DecodeFinding` reports:
- DER violations: non-minimal length, indefinite length, constructed string, unsorted `SET OF`, an encoded `DEFAULT`, non-minimal integer, BOOLEAN not `00`/`FF`;
- constraint violations;
- tag mismatches;
- unknown extension additions (fine when the type is extensible, otherwise an error);
- unresolved open-type keys;
- trailing data.

Findings carry codes in the existing diagnostics scheme, documented in `explain`: `E400` (malformed), `E401` (invalid contents), `W402` (DER), `E403` (tag mismatch), `E404` (missing component), `E405` (unexpected TLV), `I406` (unknown extension addition), `E407` (constraint), `W408` (unresolved open type), `E409` (trailing data). The 3xx range this spec first sketched belongs to the snippet checks. Constraints are checked from the IR's summaries (value, size, alphabet) and simple table constraints; extensible constraints are not checked, and `WITH COMPONENTS` presence constraints are not yet.

### Mismatch behaviour

When a TLV does not match the expected type, it is marked `Unmatched`, its subtree is decoded without a schema (asn1parse-style), and decoding continues with its siblings. A schema-less decode (no type given) is the same walk with every node `Unmatched`.

### OID hints for tables the module doesn't have

1988-syntax modules carry no information object sets: RFC 5280's `extnValue` is a plain `OCTET STRING`, and `AlgorithmIdentifier.parameters` is `ANY DEFINED BY algorithm`. In that situation the decoder looks up the sibling OID in a **hint table** and decodes the contents as the type the table names. The table is built by corpus-build from the object sets of published 2002-syntax modules: one row per (OID, class, field) → (document, module, object set, type), the type also in brief for the report. It is published in `corpus.json` as `oid_hints`, an optional field, so the data schema does not change. At decode time the hinted object set is lowered from its module (`glint_sema::ir::lower_object_set`) and the row's type read from it.

The sibling OID is the value of `DEFINED BY`, else the OID of the nearest component decoded before in the innermost record (a nested one, like `algorithm.algorithm`, when no direct one is). Hints apply to open types without a table and to plain `OCTET STRING`s (not `BIT STRING`s) that have no contents constraint. The decoder tries the hinted types in order and keeps the first that fits: one TLV whose decode has no `E400`, `E401`, `E403`, `E404` or `E405`; otherwise the contents stay as they are.

A hinted decode is marked `SchemaState::Hinted { from: "rfc5912#PKIX1Implicit-2009" }`. The web and CLI show it as heuristic (a distinct badge, `~` in the text output), and its findings are capped at warning severity. `ANY DEFINED BY` also uses hints when the module's own text does not resolve the key, and so does an open type whose (extensible) table has no object with the key. Codegen never uses hints: generated code reflects only the selected modules.

### Selecting types

A decode is driven by a **root type**. Besides the CLI and web picker, roots come from the PEM label map and from artifact identification (below). The label map (RFC 7468 labels plus the common de-facto labels) is code. Each entry maps to a list of corpus roots in preference order (`CERTIFICATE` → `rfc5912#PKIX1Explicit-2009.Certificate`, then `rfc5280#PKIX1Explicit88.Certificate`).

### CLI

```
glint decode [<module> <Type>] [FILE|-] [--file x.asn]... [--input auto|pem|base64|der] [--json] [--strict-der] [--no-schema]
```

`<module>` is `source#Name` (`rfc5912#PKIX1Explicit-2009`), a bare module name only one document defines, or with `--file x.asn` the name of a module of user ASN.1. Without a module and type, each PEM block's label chooses the first root of the label map that the corpus has, or the input is decoded without a schema. Text output extends asn1parse's columns:

```
    0:d=0  hl=4 l=1234 cons: SEQUENCE                    Certificate
    4:d=1  hl=4 l= 954 cons:  SEQUENCE                   tbsCertificate: TBSCertificate
    8:d=2  hl=2 l=   3 cons:   [0] EXPLICIT              version: Version
   10:d=3  hl=2 l=   1 prim:    INTEGER                  2 (v3)
   ...
  374:d=5  hl=2 l=   3 prim:      OBJECT IDENTIFIER      extnID: 2.5.29.15 id-ce-keyUsage
  382:d=5  hl=2 l=   4 prim:      OCTET STRING           extnValue: CONTAINING
  384:d=6  hl=2 l=   2 prim:       BIT STRING            KeyUsage digitalSignature, keyCertSign
         ! W402 warning: ...
```

A hinted TLV has `~` before its field. `--json` prints, per input, its encoding and label, the root type, and the `Tlv` trees. The exit code is non-zero on errors (not on DER notes alone, unless `--strict-der`).

## Codegen

### Backends and the registry

```rust
pub trait Backend: Sync {
    fn meta(&self) -> &BackendMeta;
    fn capabilities(&self) -> &Capabilities;
    fn namer(&self) -> &dyn Namer;
    fn emit(&self, plan: &EmitPlan, opts: &Options) -> Output;
    fn runtime(&self) -> File;                    // the vendored helper file
}

pub struct BackendMeta {
    pub id: &'static str,                         // "rasn", "der", "pyasn1", "go-cryptobyte"
    pub display_name: &'static str,
    pub language: Language,                       // picks the CodeMirror mode and file extensions
    pub library: &'static str,                    // "rasn", "golang.org/x/crypto/cryptobyte"
    pub library_version: &'static str,            // the release the output is tested against
}

pub struct Output {
    pub files: Vec<File>,                         // one per ASN.1 module, plus the runtime file
    pub findings: Vec<Finding>,                   // everything not expressible on this target
}

pub fn backends() -> &'static [&'static dyn Backend];
```

The CLI's `--backend` values, the web dropdown, `explain`'s list and the CI compile matrix all come from `backends()`. Nothing outside `glint-codegen` names a backend.

### Middle layer

Each backend states what its library handles natively in `Capabilities`: constraint kinds, open types, extensibility, `SET OF` ordering, high tag numbers, integer widths, `DEFAULT` handling. The shared middle layer turns the `IrSet` into an `EmitPlan` that covers whatever the target lacks:

- **Constraint plan.** Each constraint the target cannot express natively becomes a small language-neutral check program: range, size, alphabet, pattern, presence, and/or/not, recursion into components and elements. Backends only print it, so a new target gets full constraint fidelity by implementing a printer of about a dozen ops.
- **Dispatch plan.** The open-type tables become enums with their variants, key constants and `Unknown` handling.
- **Naming.** A collision pass runs over the backend's `Namer` (case rules, keyword list, reserved runtime names).
- **Gaps.** A finding is attached for every construct no capability or plan covers.

### Adding a target

A new backend is one module in `glint-codegen` that:
- implements `Backend` and is listed in `backends()`;
- ships a runtime file that implements the runtime contract: tag and length reading, check-program primitives, DER `SET OF` sorting where the library lacks it, and time parsing;
- provides fixture snapshots and a CI job that compiles the output and runs the differential test (see Testing).

No change to the IR, decoder, CLI or web is needed. Likely next targets are TypeScript (`@peculiar/asn1-schema`), Java (Bouncy Castle) and Go `encoding/asn1` structs; none is in scope here.

### Library versions

Each backend targets the latest release of its library when its milestone starts, recorded in `BackendMeta::library_version` and in every generated file's header. A version bump is a deliberate change: update the constant, regenerate snapshots, and the CI compile job proves the output still builds and round-trips. A scheduled job reports when a newer release exists.

### File headers

Every generated file has a header with the Glint code version, the data build ID, the backend and library version, the root type and the source modules. The runtime file carries its own version so a stale vendored copy can be spotted. Its license is MIT-0, stated in its header.

### Per-library mapping

| ASN.1 | rasn | RustCrypto `der` | pyasn1 | Go cryptobyte |
| --- | --- | --- | --- | --- |
| `SEQUENCE` | `#[derive(AsnType, Decode, Encode)]` struct | `#[derive(Sequence)]` struct | `univ.Sequence` + `namedtype.NamedTypes` | struct + generated `Unmarshal`/`Marshal` |
| `CHOICE` | `#[rasn(choice)]` enum | `#[derive(Choice)]` enum | `univ.Choice` | interface or tagged struct + peek dispatch |
| `OPTIONAL` / `DEFAULT` | `Option<T>` / `#[rasn(default = ...)]` | `optional = "true"` / `default = ...` | `OptionalNamedType` / `DefaultedNamedType` | pointer / zero-value check in runtime |
| Tags | `#[rasn(tag(explicit(context, 0)))]` | `context_specific`, `tag_mode` | `.subtype(implicitTag=..., explicitTag=...)` | runtime tag helpers (high tag numbers included) |
| `INTEGER` | width from summary, else `Integer` | width from summary, else `Int`/`Uint` | `univ.Integer` + `namedval` | `int64` or `*big.Int` |
| Constraints | `size`, `value`, `from`, `extensible` attributes; runtime for the rest | generated `validate()`, called from the decode and encode hooks | `subtypeSpec` with `ConstraintsUnion`/`Intersection`/`Exclusion`, `WithComponentsConstraint` | generated `Validate()` |
| `SET OF` | `SetOf<T>` | `SetOfVec<T>` (DER order) | `univ.SetOf` | slice, sorted on marshal by the runtime |
| Open types | dispatch enum + `Unknown(Any)` | dispatch enum + `Unknown(Any)` | `opentype.OpenType` mapping | dispatch interface + `Unknown` raw |
| Extensibility | `#[rasn(extensible)]` | runtime skips unknown trailing components | native | runtime skips unknown trailing components |

Known target gaps, each reported as a finding plus a comment in the output:
- rasn decodes an untagged `CHOICE` or open type by trying it, so when such a component is `OPTIONAL`, absent and followed by another component (RFC 5912 `TBSCertList.nextUpdate`), the value does not decode (`W420`).
- rasn's BER decoder rejects unknown extension additions. The runtime works around it: an extensible `SEQUENCE` gets a trailing `_extensions: glint_rt::Extensions` field that keeps them, so they re-encode unchanged. An extensible `SET` or `CHOICE` still rejects unknown additions.
- `der` has no constraints, extensibility or open types of its own: everything goes through generated hooks and the runtime.
- cryptobyte's `asn1.Tag` is a `uint8`, so it cannot express tag numbers above 30; the runtime reads those itself.
- pyasn1 has no `PATTERN` constraint: the runtime supplies one built on `re`, approximating X.680 Annex A regular expressions.
- `REAL` and `EMBEDDED PDV` have uneven support; types the target lacks become runtime types.

### Constraint enforcement

Values are checked on decode and on encode. The middle layer turns each constraint into a check program (ranges over values, sizes and code points, constant equality, presence, per-component and per-element checks, and/or/not), which generated code carries as static data and the runtime interprets through one trait every generated type implements; `glint_rt::decode`/`decode_der` resolve open types and then check, and `glint_rt::encode` resolves a copy (so an open type whose key names another type is refused) and checks before writing. The rasn backend emits no `size`/`value`/`from` attributes of its own: they only matter to PER, and rasn's BER size check counts octets, not characters. In pyasn1 the native checks run, and the runtime adds what they lack. In `der` and Go, the generated `validate()`/`Validate()` runs inside the generated decode and encode paths, so hand-built values are rejected at encode time. Errors name the ASN.1 path (`tbsCertificate.extensions[3].extnValue`) and the constraint as written.

### CLI

```
glint codegen <module> <Type> --backend rasn|der|pyasn1|go-cryptobyte [--out DIR] [--file x.asn]
```

With no `--out`, the module files and runtime go to stdout, separated by file-name headers.

## Example artifacts

### Extraction

`glint-extract::artifacts` scans a document for PEM blocks (`-----BEGIN <label>-----` … `-----END <label>-----`):

- **Text:** it works on the same line model as module extraction, so page headers and footers, form feeds and indentation inside a block are dropped. RFC 8792 folding (`\` and `\\`) is undone before base64 decoding.
- **XML:** it reads the content of `<sourcecode>` and `<artwork>` elements; a block that spans elements is not joined.
- **Validation:** a block is kept only if its base64 decodes and its bytes form complete BER. Anything else gets an extraction finding with the location (a broken example is worth reporting too): no `END` line before the next `BEGIN`, a section heading or the end; an `END` line with another label; a body that is not base64 (an example elided with `...`); octets that are not one BER encoding.
- **OpenPGP armor** (RFC 4880, `-----BEGIN PGP ...-----`) looks like PEM but is not DER: it is counted and left out, with no finding.

Each artifact records:
- the document;
- an id, stable while the document doesn't change (`rfc9500-4`: the n-th artifact in text order);
- the section, line (in the document's source: its RFCXML when it has one), and figure caption when there is one (the `<figure>`'s name, or a `Figure N: ...` line right after the block);
- the label, the DER bytes and their SHA-256.

Other textual forms (bare base64, hex dumps, dumpasn1 listings) are out of scope. The scanner is a list of recognizers, so one can be added later with its own validation rule.

### Identification

Each artifact gets a root type, chosen in this order:

1. **Label.** A label in the PEM label map gives the candidate roots; the first that decodes cleanly wins. Confidence: `label`.
2. **Try-decode.** Otherwise, or if no label candidate decodes cleanly, candidates are tried in order: the document's own module types (only top-level types that are not referenced by another type of the module), then the common corpus roots (`Certificate`, `CertificateList`, `CertificationRequest`, `ContentInfo`, `OneAsymmetricKey`, `SubjectPublicKeyInfo`, `OCSPResponse`, `TimeStampResp`, ...). A candidate is accepted when every TLV is `Matched` (hints allowed) with no error findings. Ties go to the earlier candidate, then to the one with fewer warnings. Confidence: `decoded`.
3. **None.** No candidate fits. Confidence: `unknown`; the artifact is still served, with the closest candidate (the most TLVs decoded with a schema) and its first mismatch.

A candidate *fits* when the structure matches: no `E400`, `E403`, `E404`, `E405` or `E409` outside the decoded contents of an octet or bit string. Errors that remain (invalid contents, constraints, a broken `extnValue`) are the example's errors, reported with the type. A candidate that decodes with no error at all wins at once; otherwise the first that fits. A label candidate that fits with errors is kept only when no later candidate decodes cleanly.

Identification runs at corpus-build time with that day's code, so the results are data and are refreshed daily, like the OID label table.

### Serving format

An artifact whose label is in the PEM label map is served as PEM, re-armored with its canonical label and 64-column lines. An artifact whose label is not in the map is served as base64 of the DER, with the original label shown beside it. The raw DER is always available too.

### Data file

corpus-build writes `artifacts.json` into `data/v<schema>/<build-id>/`, next to `corpus.json`, with a `.sha256`, under the same schema and build-ID rules (the build ID hashes it too). Broken blocks are listed in it as well, without octets. It holds the artifact index (the fields above plus identification and decode-finding codes) and the DER bytes as base64. The web fetches it lazily when the Artifacts tab opens. `glint corpus update` downloads it with the rest of the data. Adding it bumps the data schema.

### Corpus self-audit

Decoding every published example is the decoder's broadest test suite. corpus-build's `report.md` gains an *Example artifacts* section listing:
- counts by confidence and by label;
- every artifact that fails to decode under its label's type;
- every artifact with constraint or DER findings;
- **erratum candidates**: every artifact identified as a type whose decode has errors, and every artifact that decodes cleanly only as a type its label does not name;
- every broken block.

A golden file in the repo, `corpus/artifacts.expected` (artifact id → root type, confidence, finding codes; RFCs only, since drafts change daily), is diffed by the data workflow, as a warning that never holds up the data; corpus-build writes the day's copy as `artifacts.expected` beside `report.md`. A change is either a decoder fix (update the file) or a decoder regression. When the decoder is correct and the RFC's example is wrong, the report lists it as an **erratum candidate**, as input to errata triage.

### Examples in a checked draft

`glint check` (CLI, web and Action) extracts the PEM blocks in the draft and identifies them the same way, trying the draft's own modules first. It then reports:
- an error when a block's base64 or BER is malformed;
- a warning when no candidate decodes cleanly, naming the closest candidate and its first mismatch;
- a warning when a block decodes cleanly only as a type its label does not name;
- a warning for constraint violations in a decoded example;
- a note for DER violations;
- an info note naming the type each example was identified as (off by default, like other resolution notes).

These are `E410`, `W411`, `W412`, `I413`, `W415` and `I414`, documented in `explain`; `glint.toml` turns them off with `[examples] check = false` and the last on with `[examples] identified = true`. Drafts stay in the browser as before.

### CLI

```
glint artifacts list [<document>] [--label L] [--type T] [--json]
glint artifacts get <artifact-id> [--format pem|base64|der] [-o FILE]
glint artifacts extract FILE           # scan a local document, no corpus data needed
```

`glint decode --artifact rfc9500-4` decodes an artifact with its identified type (for `unknown`, the closest candidate). The artifacts are read from the `artifacts.json` beside the corpus data, or `--artifacts FILE`.

## Web: `#/workbench`

- **Picker:** module (corpus search like the corpus browser, or "modules from the checker document"), then type (types of that module, searchable). Route state is in the hash (`#workbench/rfc5912/PKIX1Explicit-2009/Certificate`, with `?tab=artifacts`, `oid=`, `artifact=` and `document=`; `#/workbench/...` reads the same), so links can be shared. The source `checker` stands for the modules of the checker's document. Pasted or dropped input is never in the hash.
- **Decode tab:** a textarea for PEM or base64 that is also a drop target for files in any of the three encodings, plus an input-format override. The TLV tree has collapsible nodes showing field, type, value and finding badges. A hex pane is linked both ways: hovering or selecting a node highlights its header and contents bytes, and clicking a byte selects its node. A PEM with several blocks gets a block selector.
- **Artifacts tab:** browse by document, or filter by label, identified type and confidence. Each row shows the document and section (linking to the checker's view of it), the label, type, confidence, size and finding badges. Actions: download (PEM when the label is known, otherwise base64; DER always available), copy, and **Open in Decode**, which sets the picker to the identified type and loads the bytes into the Decode tab. A document opened in the checker links to its artifacts here.
- **Codegen tab:** a backend dropdown filled from the registry, plus a read-only CodeMirror view with one tab per output file (the runtime file included), each with copy and download buttons; the whole output can also be downloaded as a zip. Codegen findings are listed under the editor.
- **wasm API:** `decode(request, bytes) -> json`, `types(request) -> json` (the types of a module, for the picker), `loadArtifacts(bytes)`, `artifacts(query) -> json`, `artifact(id) -> json` (with its download text), and later `codegen(request) -> json`, behind `web/src/api.ts` like `check_document`. The worker fetches `artifacts.json` on the first artifacts call. A decode with the checker's document extracts its modules as `check` does. Nothing leaves the browser.
- **Size:** measure the wasm growth. If the backends add too much, split them into a second wasm module loaded lazily when the Codegen tab opens. Measured at milestone 4: the decoder and the workbench API add 161 KB (36 KB gzipped) to a 3.49 MB module; no split.

## Testing

- **IR:** insta snapshots of the lowered IR for a fixed set of corpus roots: `Certificate` (RFC 5280 and RFC 5912 modules), `ContentInfo` (RFC 5652/6268), `OneAsymmetricKey` (RFC 5958), `TimeStampResp` (RFC 3161), `OCSPResponse` (RFC 6960). Unit tests for constraint evaluation: set arithmetic, extensibility, `WITH COMPONENTS`.
- **Decoder:** golden text and JSON trees for a hand-picked set of RFC vectors (RFC 4134 CMS examples, RFC 8410 keys, RFC 9500 test keys) and synthetic BER edge cases. Every published artifact is checked through `corpus/artifacts.expected` (Example artifacts, Corpus self-audit).
- **Extraction:** fixtures for page breaks inside a block, RFC 8792 folding, indented blocks, XML `<sourcecode>`, truncated and malformed blocks. Proptest: decoding random bytes never panics, and every byte is attributed to exactly one TLV.
- **Codegen:** snapshots of each backend's output for the fixture roots. In CI, compile the output: `cargo check` for rasn and `der`, `python -c import` plus a decode run for pyasn1, `go vet` and `go build` for cryptobyte.
- **Differential:** each backend's generated code decodes every vector, including every artifact identified as `label` or `decoded`, and prints a normalized JSON tree, which must equal `glint-decode`'s. Every vector is re-encoded and must round-trip byte-for-byte for DER inputs. Constraint-violating vectors must be rejected by every backend.

## Milestones

1. **IR and lowering** (done). Constraint algebra, effective summaries, object-set tables, monomorphization, naming inputs. *Accept:* IR snapshots for the fixture roots with no `Opaque` constraints in the RFC 5912 `Certificate` closure.
2. **Decoder, CLI** (done). Sniffing, BER walk, rendering, DER and constraint findings, mismatch fallback, OID hint table, `glint decode`. *Accept:* a real certificate decoded with the RFC 5912 modules shows every extension's contents decoded through the `ExtensionSet` table, with OID labels. The same certificate decoded with the RFC 5280 modules shows its extensions decoded through hints, marked heuristic.
3. **Artifacts** (done). PEM extraction, identification, `artifacts.json`, the report section, `corpus/artifacts.expected`, `glint artifacts`, example checks in `check`. *Accept:* every PEM example in the corpus is extracted or has an extraction finding; every decode failure is triaged as a decoder bug (fixed) or an erratum candidate (listed).
4. **Workbench web route: Decode and Artifacts tabs** (done). *Accept:* drag-and-drop of PEM, base64 and DER files; tree and hex linked both ways; a shareable hash route; artifact download in the right format; Open in Decode.
5. **Codegen middle layer, registry, rasn backend + runtime** (done). *Accept:* fixture roots compile, the differential test passes over the vectors and artifacts, round-trip passes. Measured: 24 roots (the fixture roots, a synthetic module of constraints, and every root the published examples are identified as) compile against rasn 0.30.0; 357 vectors decode to the decoder's tree and re-encode byte for byte, 20 are rejected as the decoder rejects them, 8 BER vectors decode (not compared), and hand-built invalid values are refused on encode.
6. **pyasn1 backend.**
7. **RustCrypto `der` backend.**
8. **Go cryptobyte backend.**
9. **Codegen tab in the workbench.**

Milestones 6–8 have the same acceptance criteria as milestone 5. They also confirm that a backend needs no change outside its own module; any change the shared layers need goes back into them.

### Codegen codes

`W420` (a construct the target cannot express, and what the code does instead), `W421` (a constraint the code does not check: `CONSTRAINED BY`, `PATTERN` without regular expressions, an unevaluable value) and `I422` (a type lowering left opaque, kept as a raw TLV), documented in `explain` with examples that `glint-codegen`'s tests run through every backend.

## Decided questions

- **Hint conflicts:** when two 2002-syntax modules map one OID to different types, the module of the most recent current RFC wins, and the conflict is listed in the corpus-build report.
- **Erratum candidates:** an artifact the decoder rejects because the RFC's example is wrong stays in the Artifacts tab with its findings and an "example has errors" badge.
- **Artifacts in obsoleted RFCs:** kept and marked obsoleted.
- **Artifact size:** measured at milestone 3: 551 KB for 368 examples in 70 documents (55 broken blocks, 121 OpenPGP blocks left out). Not split; Pages compresses JSON.
- **Library versions:** the latest release of each, recorded in backend metadata (Codegen, Library versions).
- **Runtime license:** MIT-0.
- **1988 modules without tables:** decoder-only OID hints (Decoder, OID hints).
- **rasn-compiler:** no compatibility; Glint's conventions.
- **Examples in drafts:** `check` decodes them, and every published example becomes a decoder test (Example artifacts).
