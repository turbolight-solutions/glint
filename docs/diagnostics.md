# Diagnostic codes

Every code has a fixture (a test named after the clause it enforces), an `explain` entry (`crates/glint-diag/explain.md`, shown by `glint explain <code>`; its `bad` and `good` examples are tested by `crates/glint-check/tests/explain.rs`) and a row here. Codes are stable; severities can be remapped in `glint.toml`.

Implemented so far (M1, M2, M3, M4, M5):

| Code | Default | Meaning | Basis | Fixtures |
| --- | --- | --- | --- | --- |
| `E001` | error | Syntax error: an expected token or construct is missing. Glint's own code; the spec's table has no generic syntax error | policy | `crates/glint-syntax/tests/x680_syntax.rs` |
| `E010` | error | Lexical error: leading zero in a number, name ending in a hyphen, bad `bstring`/`hstring` character, unterminated comment or string, unexpected character | X.680 12.2–12.3, 12.6, 12.8–12.14 | `tests/lexical.rs` |
| `E011` | error | Reserved word used as a reference name. A warning when the parser runs with `ParseOptions::corpus()` and the word is reserved only by editions after X.208 | X.680 12.2.2, 12.38 | `tests/lexical.rs`, `tests/rfc5280_1988.rs` |
| `E108` | error | `WITH SUCCESSORS` / `WITH DESCENDANTS` without an assigned identifier | X.680 13.16 f | `tests/x680_syntax.rs` |
| `W136` | warning | Unknown encoding reference in an encoding prefix or `ENCODING-CONTROL` section (only `TAG`, `XER` and `PER` are defined by Annex E); ignored | X.680 31.3, 54, Annex E | `tests/x680_syntax.rs` |
| `W137` | warning | XML value notation present; kept as an opaque node and not checked | X.680 16.2 | `tests/x680_syntax.rs` |
| `I200` | info | ASN.1 block skipped: empty, refers to an external file (`src`), or never closed | policy | `crates/glint-extract/tests/xml_format.rs` |
| `W201` | warning | Block typed as ASN.1 (`type="asn.1"`, an `asn.1` fence, or a bare `.asn` file) does not look like ASN.1; skipped | policy | `tests/xml_format.rs`, `tests/bare_and_detection.rs` |
| `W202` | warning | Text looks like ASN.1 but is not marked as `sourcecode` of type `asn.1` (untyped XML elements, unmarked Markdown code blocks); extracted anyway. Not raised for plain text, where nothing is marked | policy | `tests/xml_format.rs`, `tests/markdown_format.rs` |
| `W203` | warning | Non-standard spelling of the ASN.1 type (`asn1`, `ASN.1`) in XML or an info string in Markdown; accepted. A lowercase `asn1` fence is accepted silently | policy | `tests/xml_format.rs`, `tests/markdown_format.rs` |
| `E203` | error | The XML document is not well-formed; blocks read before the error are kept. Glint's own code; the spec's extraction codes stop at `W202` | policy | `tests/xml_format.rs` |
| `W100` | warning | Imported module not found in the draft, the RFC corpus or the ITU-T set; its symbols are opaque (once per module) | policy | `tests/imports.rs` |
| `E101` | error | Imported symbol not defined in, or not exported by, the target module | X.680 13.13, 13.14, 13.16 a–b | `tests/imports.rs` |
| `W102` | warning | Module matched by name, but the IMPORTS OID differs from the corpus module's; the OID is definitive | X.680 13.10, 13.11 | `tests/imports.rs` |
| `I103` | info | Import resolved by name to one of several corpus modules | policy | `tests/imports.rs` |
| `E104` | error | Same module imported twice (by name or OID), or a module importing itself | X.680 13.16 e | `tests/imports.rs` |
| `I105` | info | Using a curated (`errata-verified` or `correction`) version of a corpus module | policy | `glint-corpus-build/tests/end_to_end.rs` |
| `W106` | warning | IMPORTS gives no OID for a module that has a definitive OID | X.680 13.12 | `tests/imports.rs` |
| `E107` | error | Value reference in an `AssignedIdentifier` does not meet 13.9 | X.680 13.9 | `tests/imports.rs` |
| `E110` | error | Undefined reference, or external reference to a module or symbol absent from IMPORTS. Covers type, class and value references (including OID components and tag numbers); names inside class definitions, object bodies and object sets are checked too | X.680 14.4–14.7 | `tests/symbols.rs` |
| `E111` | error | Two assignments with the same name | X.680 13.21; X.683 8.2 | `tests/symbols.rs` |
| `W112` | warning | Unused import (no mention of the name outside IMPORTS/EXPORTS) | policy | `tests/symbols.rs` |
| `E113` | error | Duplicate identifier among components (after `COMPONENTS OF`) or alternatives | X.680 25.14, 29.10 | `tests/tags.rs` |
| `E114` | error | Symbol imported twice or also defined locally, used without `Module.symbol` | X.680 13.20 | `tests/symbols.rs` |
| `E120` | error | Tag clash among `CHOICE` alternatives or `SET` components, with the conceptually added element and open types | X.680 27.3, 29.3, 29.9, 52.7; X.681 14.2 b | `tests/tags.rs` |
| `E121` | error | Tag clash in a run of `OPTIONAL`/`DEFAULT` `SEQUENCE` components, or between extension additions and following root components | X.680 25.6, 25.7, 52.7 | `tests/tags.rs` |
| `E122` | error | `IMPLICIT` on an untagged `CHOICE`, open type or dummy reference | X.680 31.2.9; X.681 14.2 c | `tests/tags.rs` |
| `E123` | error | `UNIVERSAL` class tag in a user module | X.680 31.2.5 | `tests/tags.rs` |
| `E124` | error | Textually tagged extension addition under `AUTOMATIC TAGS` when the root is untagged | X.680 25.9, 29.4 | `tests/tags.rs` |
| `E125` | error | `VersionNumber` misuse | X.680 25.16 | `tests/tags.rs` |
| `E126` | error | `COMPONENTS OF` names a type that is not a `SEQUENCE` (or `SET` inside a set) | X.680 25.5, 27.2 | `tests/tags.rs` |
| `E130` | error | OID with fewer than two arcs, first arc above 2, or second arc 40 or more under arcs 0 and 1 | X.660 via X.680 32.11 | `tests/oids.rs` |
| `E131` | error | Value not of a type compatible with its governing type (defaults, value assignments, `SEQUENCE`/`CHOICE`/`SEQUENCE OF` values, OID components that are not OIDs) | X.680 14.9, 25.12, Annex C | `tests/values.rs`, `tests/oids.rs` |
| `E132` | error | Empty or inverted range, negative `SIZE`, or a constraint value outside the parent type's extension root | X.680 49.7, 50.7–50.11 | `tests/values.rs` |
| `E133` | error | Subtype element not applicable to the parent type | X.680 51.1 Tables 11–12 | `tests/values.rs` |
| `E134` | error | Bare `NameForm` OID arc that X.660 does not register at that position | X.680 32.7, 32.12; X.660 A.2–A.4 | `tests/oids.rs` |
| `E135` | error | Exception specification `!` in a constraint without an extension marker | X.680 49.6 | `tests/values.rs` |
| `W140` | warning | OID (or module identifier) already assigned in the corpus to a different name | policy | `tests/oids.rs`, `glint-corpus-build/tests/end_to_end.rs` |
| `I141` | info | OID uses an author placeholder (`TBD`, `TBDn`, `id-TBD`, or `[oid] placeholders`) | policy | `tests/oids.rs` |
| `W142` | warning | OID label has another number under the same parent in the corpus label table | policy | `tests/oids.rs`, `glint-corpus` `labels.rs` |
| `W143` | warning | OID label with this number is only under another parent in the corpus label table | policy | `tests/oids.rs`, `glint-corpus` `labels.rs` |
| `W144` | warning | Module identifier in `IMPORTS` belongs to another corpus module than the one named | policy | `tests/oids.rs` |
| `I145` | info | OID label differs from the corpus's name for the arc (off unless `[oid] synonym_labels = true`) | policy | `tests/oids.rs` |
| `E150` | error | Bad field reference: a field that the class does not have, a chain through a field that is not an object or object set field, `Class.&objectField`, or an information-from-objects combination that Table 1 forbids (a type or variable-type field read from an object set, a variable-type value set from an object, a non-type read where a type is needed) | X.681 9.14, 14.5, 15.4–15.5, 15.11 | `tests/x681.rs` |
| `E151` | error | Component relation path does not resolve: unknown identifier, a path through a component that is not a `SET`, `SEQUENCE` or `CHOICE`, more dots than enclosing constructions, or `@` outside any `SET` or `SEQUENCE` | X.682 10.8–10.10 | `tests/x682.rs` |
| `E152` | error | Object does not match its class: wrong default-syntax field, mandatory field missing, field set twice, a token that the class's `WITH SYNTAX` does not allow, a setting that is not a type, value, value set, object or object set as the field requires, or an optional-group literal with no setting after it | X.681 10.11, 10.12 d, 11.4–11.7 | `tests/x681.rs` |
| `E153` | error | Class definition error: duplicate field names, `UNIQUE` with `DEFAULT` or on a field that is not a fixed-type value field, a value field with no type, a variable-type field that does not name a type field or whose optionality disagrees with it, a chain through a field that is not an object field, a recursive class with no `OPTIONAL` or `DEFAULT` link | X.681 9.6, 9.8, 9.10, 9.13, 9.15 | `tests/x681.rs` |
| `E154` | error | `WITH SYNTAX` error: a field missing, repeated or not in the class, a reserved word from the 10.6 list as a literal, an optional group with no field, adjacent groups that start with the same literal, a group that starts with a literal and is not followed by a literal, unbalanced brackets | X.681 10.6, 10.9, 10.12 a–c | `tests/x681.rs` |
| `E155` | error | Object or object set not of the governing class, or not an object or object set at all; a textually identical but separate `CLASS` definition does not count | X.681 8.2, 11.3, 12.10 | `tests/x681.rs` |
| `E156` | error | Two objects of one object set with the same `UNIQUE` field value (OBJECT IDENTIFIER, INTEGER and string values that Glint can evaluate; sets that use `INTERSECTION` or `EXCEPT` are not checked) | X.681 9.7 | `tests/x681.rs` |
| `E157` | error | Recursive definition of an object or object set, directly or through other objects and sets | X.681 11.2, 12.2 | `tests/x681.rs` |
| `E158` | error | Empty object set: `{ }` without `...` | X.681 12.3 | `tests/x681.rs` |
| `E159` | error | Open type (type field, variable-type value field, or a type that contains one) used as the type of a value or value set field | X.681 14.2 a, 14.4 | `tests/x681.rs` |
| `E160` | error | Actual parameter list mismatch: wrong number or form of actual parameters, a parameterized name used without actual parameters, or actual parameters given to a name that is not parameterized. Names imported from a module matched only by name although the IMPORTS gives another identifier (`W102`) are not checked | X.683 9.2, 9.3, 9.6 | `tests/x683.rs` |
| `E161` | error | Parameter list error: a value or object dummy without a governor, a type or class dummy used as a value or object set (or the reverse), a governor that is not a type or class, a dummy governor that has a governor | X.683 8.3 | `tests/x683.rs` |
| `E162` | error | Dummy reference used both as a type and as a class, or as a value and as a type | X.683 8.5 | `tests/x683.rs` |
| `E163` | error | Dummy reference never used in its scope | X.683 8.6 | `tests/x683.rs` |
| `E164` | error | Illegal recursion: a parameterized value, value set, object or object set that refers to itself; a recursive path whose actual parameter mixes a dummy with other text (a lone `{ dummy }` is accepted); a parameterized type that refers to itself with nothing to end the recursion | X.683 8.6–8.8 | `tests/x683.rs` |
| `E165` | error | Governor refers to another governed dummy, or to the dummy or the name being defined | X.683 8.9, 8.11 | `tests/x683.rs` |
| `E166` | error | Right-hand side of a parameterized assignment is only a dummy reference | X.683 8.10 | `tests/x683.rs` |
| `E167` | error | Actual value not compatible with the governor of its dummy (checked for values written in the same module as the definition) | X.683 8.12 | `tests/x683.rs` |
| `E168` | error | Parameter of a parameterized abstract syntax not used, directly or through actual parameters, in a constraint | X.683 10.2 | `tests/x683.rs` |
| `W169` | warning | Variable constraint (it depends on an abstract syntax parameter) with no exception specification | X.683 10.3–10.4 | `tests/x683.rs` |
| `W170` | warning | 1988 syntax (`ANY`) mixed with X.681 constructs in one module. Not raised for corpus modules | policy | `tests/x683.rs` |
| `W171` | warning | An all-upper-case name for a type, which reads as an information object class. Not raised for corpus modules | policy (X.681 7.1) | `tests/x683.rs` |
| `E172` | error | Module defines a new `MACRO`. Not raised for corpus modules | policy | `tests/x683.rs` |
| `W173` | warning | Module uses imported macro notation; those assignments are not checked. Not raised for corpus modules | policy | `tests/x683.rs` |
| `E180` | error | Component relation constraint on a type that is not an object class field type or `INSTANCE OF`; any constraint other than a simple table constraint on `INSTANCE OF` (a single braced value after another type is read as a single-value constraint, X.681 14.3) | X.682 10.1, A.2 | `tests/x682.rs` |
| `E181` | error | Constraining object set not of the class named in the constrained field type (the class written before the first `.`, as 10.4 says), or not an object set | X.682 10.4, 10.15 | `tests/x682.rs` |
| `E182` | error | Referenced component is not an object class field type of the same class, not a value or value set field, or not constrained by the same object set | X.682 10.15 | `tests/x682.rs` |
| `E183` | error | Referenced component lies in a different alternative of a `CHOICE` that contains the referencing component | X.682 10.14 | `tests/x682.rs` |
| `E184` | error | Contents constraint on a type other than `OCTET STRING` or a `BIT STRING` without named bits, combined with any other constraint (directly or through a type reference), or with an `ENCODED BY` literal that is not an object identifier | X.682 11.2–11.3 | `tests/x682.rs` |
| `E185` | error | `CONSTRAINED BY` parameter not of a permitted form: a value or object without its governor, or a governor that is a value, object or set | X.682 9.3 | `tests/x682.rs` |
| `E300` | error | A snippet's normal form differs from the module definition of the same name (also: a snippet that is another kind of assignment than the module's, a parameterized module definition quoted without its parameters). The primary label is the first difference in the snippet, the secondary label the matching place in the module, further differences are notes. Names the snippet defines are looked up in the draft's modules, then in the modules the draft imports, then in every corpus module; a match with any candidate is enough, otherwise the closest candidate is reported | X.680 C.3 | `crates/glint-check/tests/snippets.rs` |
| `W301` | warning | A snippet defines a name that no module defines (or, with `module=`, that the named module does not define). Info instead of a warning when the draft imports from a module that is not available (`W100`), since the definition may be there | policy | `tests/snippets.rs` |
| `W302` | warning | Same normal form, different text: alternatives or `SET` components reordered, an OID written with names instead of numbers (or built on a defined value), `IMPLICIT`/`EXPLICIT` spelled out or left to the module default, the two `SEQUENCE SIZE (…) OF` forms, enumeration numbers added or omitted, extension parts in another place, `COMPONENTS OF` expanded. Also raised when an object identifier names a definition that cannot be resolved in the module, so no difference can be proved | X.680 C.3 | `tests/snippets.rs` |
| `I303` | info | The snippet's name is defined by several modules; says which one the snippet matches | policy | `tests/snippets.rs` |
| `W304` | warning | Unknown `-- glint:` directive, or `module=` names a module that is neither in the draft nor in the corpus | policy | `tests/snippets.rs` |
| `W310` | warning | A type or class defined in a draft module that the text neither shows in a snippet nor mentions by name. Off unless `snippets.reverse_check = true` | policy | `tests/snippets.rs` |

### Snippet comparison (M5)

A *snippet* is an assignment in a block that is not a module (a `Fragment`). Each is compared with the definition of the same name by building two trees from the syntax trees, without comments or white space:

- the **raw form** keeps what was written;
- the **normal form** applies X.680 C.3.2.2, C.3.2.3 and C.3.2.4 as far as Glint supports: `INTEGER`, `BIT STRING` and `ENUMERATED` items in alphabetical order (enumeration numbers assigned as in 20.3); `CHOICE` alternatives and `SET` components in alphabetical order; extension parts after the root, a second `...` dropped, an implied `...` added; `IMPLICIT` and `EXPLICIT` resolved against the module's tag default (a `CHOICE` or open type is always explicit); `COMPONENTS OF` expanded; automatic tagging applied; object identifier values reduced to numbers; white space dropped from `bstring` and `hstring`; the constraint of the two `SEQUENCE SIZE (…) OF` forms written the same way.

Different normal forms are `E300`; equal normal forms with different raw forms are `W302`. Glint departs from C.3 in three ways: type and value references are not expanded (C.3.2.3 f and g), so a snippet must use the module's reference names; automatic tagging is applied *before* `CHOICE` alternatives are sorted, because the order of the text decides the tags; and the normal form of a value set assignment is not turned into a constrained type (C.3.2.2 a). Constructs C.3.3 leaves out (information object classes, objects, object sets, constraints) are compared as token sequences.

Directives are comments in the fragment: `-- glint: partial` (the snippet's components must appear in the module in the same order; the module may have more), `-- glint: ignore` (skip the block; also works on a module block), `-- glint: module=Name` (compare only against that module). A comment that begins with one of `snippets.elision_comments` makes the enclosing construct partial. `--no-snippets` skips fragments altogether, including their syntax errors.

Where a snippet's name is looked up: the draft's modules; if none defines it, the modules the draft imports from; if none of those does, every corpus module that parses without errors, and then only definitions of the same kind (the type or value type starts the same way), because a name such as `algorithm` is shared by unrelated modules. In a plain-text document, a fragment is found by a heuristic (nothing marks it as ASN.1), so syntax errors in it are warnings, not errors.

All other codes in the design spec are assigned to the milestone that implements them (M5 is done; M6 and M7 add no codes).

`W137` (XML value notation) was emitted since M1 but missing from the registry; M7 registered it.
