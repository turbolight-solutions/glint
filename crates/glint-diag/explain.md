# Diagnostic code explanations

Each section is the text of `glint explain <code>`. Fenced examples tagged `bad` must raise the code and examples tagged `good` must not (`crates/glint-check/tests/explain.rs`); `untested` examples need a corpus or a configuration the test does not have.

The decoder's codes (`E400`–`E409`) have `der` examples: an encoding in hexadecimal, decoded by `glint decode`. An example that names a type (`M.T`) is decoded as that type of the section's `asn1 schema` module; one that names none is decoded without a schema.

## E001

The parser expected a token or construct and found something else. Glint reports the first problem in a construct and then resynchronizes, so one mistake does not hide the rest of the module.

```asn1 bad
M DEFINITIONS ::= BEGIN
  A ::= SEQUENCE { a INTEGER b BOOLEAN }
END
```

The comma between components is missing.

```asn1 good
M DEFINITIONS ::= BEGIN
  A ::= SEQUENCE { a INTEGER, b BOOLEAN }
END
```

In a plain-text draft, ASN.1 is found by a heuristic. A fragment that is not ASN.1 (a hex dump, an example in pseudo-ASN.1) is then reported as a warning; mark real ASN.1 as `<sourcecode type="asn.1">` to get an error.

## E010

A lexical error: a number with a leading zero, a name ending in a hyphen or containing two hyphens in a row, a bad character in a `bstring` or `hstring`, an unterminated comment or string, or a character that is not part of ASN.1 (X.680 12.2–12.14).

```asn1 bad
M DEFINITIONS ::= BEGIN
  x INTEGER ::= 007
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  x INTEGER ::= 7
END
```

Hexadecimal strings use upper-case digits only: `'0A1F'H`, not `'0a1f'H`.

## E011

A reserved word is used as a reference name (X.680 12.38). Words reserved only by editions after X.208 are a warning in published modules, because the old module could not know them, and are not reported on the 1988 definitions of the string types that `E123` allows.

```asn1 bad
M DEFINITIONS ::= BEGIN
  SET ::= INTEGER
END
```

Rename the reference.

## E108

`WITH SUCCESSORS` or `WITH DESCENDANTS` follows an import that has no assigned identifier (X.680 13.16 f). The clause refers to the versions of the module that the identifier names, so it needs one.

```asn1 bad
M DEFINITIONS ::= BEGIN
  IMPORTS A FROM X WITH SUCCESSORS;
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  IMPORTS A FROM X { 1 2 3 } WITH SUCCESSORS;
END
```

## W136

An encoding prefix or an `ENCODING-CONTROL` section names an encoding reference that X.680 Annex E does not define (only `TAG`, `XER` and `PER` are). Glint ignores the construct.

```asn1 bad
M DEFINITIONS ::= BEGIN
  ENCODING-CONTROL ZZZ stuff
END
```

## W137

The module uses XML value notation (X.680 16.2). Glint keeps it as opaque text and does not check it. Prefer the ordinary value notation, which Glint does check.

```asn1 bad
M DEFINITIONS ::= BEGIN
  a INTEGER ::= <INTEGER>5</INTEGER>
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  a INTEGER ::= 5
END
```

## I200

An ASN.1 block was skipped because it is empty, refers to an external file (`src`), or is never closed.

```xml bad
<rfc><middle><section><name>Module</name>
<sourcecode type="asn.1"></sourcecode>
</section></middle></rfc>
```

Put the module text inside the `sourcecode` element. A block with `src` is not read; Glint cannot see the file.

## W201

A block marked as ASN.1 (`type="asn.1"`, an `asn.1` code fence, or a bare `.asn` file) does not look like ASN.1, so Glint skipped it.

```xml bad
<rfc><middle><section><name>Notes</name>
<sourcecode type="asn.1">This text is not ASN.1 at all.</sourcecode>
</section></middle></rfc>
```

Give the block its real type (`type="abnf"`, `type="json"`, and so on) or leave `type` out.

## W202

Text that looks like ASN.1 is not marked as `sourcecode` of type `asn.1`. Glint extracted it anyway, but other tools will not find it. This is not raised for plain-text documents, where nothing is marked.

```xml bad
<rfc><middle><section><name>Module</name>
<sourcecode>
M DEFINITIONS ::= BEGIN
  A ::= INTEGER
END
</sourcecode>
</section></middle></rfc>
```

```xml good
<rfc><middle><section><name>Module</name>
<sourcecode type="asn.1">
M DEFINITIONS ::= BEGIN
  A ::= INTEGER
END
</sourcecode>
</section></middle></rfc>
```

## W203

The type of an ASN.1 block is spelled in a non-standard way (`asn1`, `ASN.1`). The standard spelling is `asn.1`. Glint accepts the other spellings.

```xml bad
<rfc><middle><section><name>Module</name>
<sourcecode type="asn1">
M DEFINITIONS ::= BEGIN
  A ::= INTEGER
END
</sourcecode>
</section></middle></rfc>
```

## E203

The XML document is not well-formed. Blocks read before the error are kept and checked.

```xml bad
<rfc><middle><section>
<sourcecode type="asn.1">A ::= INTEGER</sourcecode>
<!-- this comment is never closed
</section></middle></rfc>
```

Glint reads the XML leniently (mismatched end tags and a bare `&` are accepted), but an unterminated comment, tag or attribute stops the reader. Escape `<` as `&lt;` and `&` as `&amp;` in text, or put an ASN.1 module in a `CDATA` section.

## W100

An imported module was found in neither the draft, the RFC corpus nor the ITU-T set. Its symbols are accepted with unknown kinds and checks that need their definitions are skipped. The warning is given once per module.

```asn1 bad
M DEFINITIONS ::= BEGIN
  IMPORTS A, B FROM Nowhere;
  T ::= SEQUENCE { a A, b B }
END
```

Check the module name. If the module is a draft that is not published yet, put its text in the same document.

## E101

An imported symbol is not defined in the target module, or the target module does not export it (X.680 13.13, 13.14, 13.16 a–b).

```asn1 bad
A DEFINITIONS ::= BEGIN
  X ::= INTEGER
END

M DEFINITIONS ::= BEGIN
  IMPORTS Missing FROM A;
  T ::= SEQUENCE { n Missing }
END
```

```asn1 good
A DEFINITIONS ::= BEGIN
  X ::= INTEGER
END

M DEFINITIONS ::= BEGIN
  IMPORTS X FROM A;
  T ::= SEQUENCE { n X }
END
```

## W102

The module was matched by name, but the object identifier in `IMPORTS` differs from the one in the corpus module. The identifier is definitive (X.680 13.10, 13.11), so Glint treats the imported names as opaque instead of guessing which version you meant. Use `--pin Name=rfcNNNN` to choose a module.

```asn1 untested
M DEFINITIONS ::= BEGIN
  IMPORTS Name FROM PKIX1Explicit88 { 1 3 6 1 5 5 7 0 99 };
  T ::= SEQUENCE { n Name }
END
```

## I103

An import resolved by name to one of several corpus modules with the same name. The note says which one was chosen; pin another with `--pin Name=rfcNNNN` or add the module's object identifier to `IMPORTS`.

## E104

The same module is imported twice (by name or by identifier), or a module imports itself (X.680 13.16 e).

```asn1 bad
M DEFINITIONS ::= BEGIN
  IMPORTS X FROM M;
  T ::= SEQUENCE { x X }
END
```

## I105

A corrected version of a corpus module is used. Published modules sometimes contain errors that the RFC Errata process has confirmed, and Glint applies those Verified errata to the RFC before extracting it (`corpus/errata`); others a maintainer has corrected by hand (a `correction` override). The note names the module, the document and the errata. Overrides that only repair text-extraction damage are silent.

## W106

`IMPORTS` gives no object identifier for a module that has a definitive one (X.680 13.12). Names alone are ambiguous across revisions of a module.

```asn1 untested
M DEFINITIONS ::= BEGIN
  IMPORTS Name FROM Base;
  T ::= SEQUENCE { n Name }
END
```

Add the identifier: `IMPORTS Name FROM Base { 1 3 6 1 4 1 99 1 }`.

## E107

The value reference in an assigned identifier does not meet X.680 13.9: it must be defined, an object identifier, and not defined in terms of itself.

```asn1 bad
M DEFINITIONS ::= BEGIN
  IMPORTS Name FROM Base missing-oid;
  T ::= SEQUENCE { n Name }
END
```

## E110

A reference is not defined, or it names another module's symbol without importing it (X.680 14.4–14.7). This covers type, class and value references, object identifier components, tag numbers, and the names inside class definitions, objects and object sets.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { n Missing }
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { n INTEGER }
END
```

## E111

Two assignments in a module have the same name (X.680 13.21, X.683 8.2). A parameterized and an ordinary assignment count as the same name.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= INTEGER
  T ::= BOOLEAN
END
```

## W112

A name is imported but never used outside `IMPORTS`.

```asn1 bad
A DEFINITIONS ::= BEGIN
  X ::= INTEGER
  Y ::= BOOLEAN
END

M DEFINITIONS ::= BEGIN
  IMPORTS X, Y FROM A;
  T ::= SEQUENCE { a X }
END
```

Remove the import, or silence the rule with `W112 = "off"` in the `[severity]` table of `glint.toml`.

## E113

A `SEQUENCE`, `SET` or `CHOICE` has two components or alternatives with the same identifier (X.680 25.14, 29.10), counting those that `COMPONENTS OF` brings in.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= CHOICE { a INTEGER, a BOOLEAN }
END
```

## E114

A symbol is imported from two modules and is used without a `Module.symbol` qualification (X.680 13.20). A bare name that the module also defines is its own definition, which can only be written bare (14.6); the import then needs `Module.symbol` (see W115).

```asn1 bad
A DEFINITIONS ::= BEGIN
  X ::= INTEGER
END

B DEFINITIONS ::= BEGIN
  X ::= BOOLEAN
END

M DEFINITIONS ::= BEGIN
  IMPORTS X FROM A X FROM B;
  T ::= SEQUENCE { a X }
END
```

Write `A.X` or `B.X`.

## W115

A symbol is imported and also defined in the module. X.680 13.20 allows it, as long as every use of the imported one is written `Module.symbol`, but deprecates it: two definitions under one name are easy to confuse.

```asn1 bad
A DEFINITIONS ::= BEGIN
  X ::= INTEGER
END

M DEFINITIONS ::= BEGIN
  IMPORTS X FROM A;
  X ::= BOOLEAN
  T ::= SEQUENCE { a A.X }
END
```

```asn1 good
A DEFINITIONS ::= BEGIN
  X ::= INTEGER
END

M DEFINITIONS ::= BEGIN
  IMPORTS X FROM A;
  LocalX ::= BOOLEAN
  T ::= SEQUENCE { a X, b LocalX }
END
```

Give the local definition another name.

## E116

Two modules that an import resolves to before the corpus, the document's own and those of the other documents checked in the same run, cannot be told apart: they have the same identifier, or the same name without both having identifiers that differ. X.680 uses a module name only once (13.7) and gives each module an identifier that names it alone (13.8); editions of a module share its name and differ in their identifiers (the NOTE to 13.10), which an import tells apart by identifier. Within one document, a module shown twice with the same name and identifier (a header with a stub body before the full module) is one module, as the corpus takes it.

```asn1 bad
A-88 { 1 2 3 } DEFINITIONS ::= BEGIN
  X ::= INTEGER
END

A-08 { 1 2 3 } DEFINITIONS ::= BEGIN
  X ::= INTEGER
END
```

```asn1 good
A-88 { 1 2 3 } DEFINITIONS ::= BEGIN
  X ::= INTEGER
END

A-08 { 1 2 4 } DEFINITIONS ::= BEGIN
  X ::= INTEGER
END
```

Give each module its own identifier. Across documents, the same module defined in two documents of one run is reported too: check only one of them, or give the new edition its own identifier.

## E120

Two `CHOICE` alternatives or two `SET` components have the same tag (X.680 27.3, 29.3, 29.9, 52.7). Every tag of an untagged `CHOICE` counts, so does the conceptually added extension element, and so does the indeterminate tag of an untagged open type.

```asn1 bad
M DEFINITIONS EXPLICIT TAGS ::= BEGIN
  A ::= CHOICE { b B, c C }
  B ::= CHOICE { d [0] NULL, e [1] NULL }
  C ::= CHOICE { f [0] NULL, g [1] NULL }
END
```

```asn1 good
M DEFINITIONS AUTOMATIC TAGS ::= BEGIN
  A ::= CHOICE { b B, c C }
  B ::= CHOICE { d NULL, e NULL }
  C ::= CHOICE { f NULL, g NULL }
END
```

`AUTOMATIC TAGS` removes the clash by tagging every alternative.

## E121

In a `SEQUENCE`, a run of `OPTIONAL` or `DEFAULT` components has the same tag as the component after it, so a decoder cannot tell which one is present (X.680 25.6, 25.7, 52.7). Extension additions count too.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a INTEGER OPTIONAL, b INTEGER }
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a [0] INTEGER OPTIONAL, b INTEGER }
END
```

## E122

`IMPLICIT` is applied to an untagged `CHOICE`, an open type or a dummy reference (X.680 31.2.9, X.681 14.2 c). These have no single tag to replace, so the tag must be explicit.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= [1] IMPLICIT CHOICE { a INTEGER, b BOOLEAN }
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T ::= [1] EXPLICIT CHOICE { a INTEGER, b BOOLEAN }
END
```

## E123

A `UNIVERSAL` class tag appears in a user module (X.680 31.2.5). Universal tags are assigned by ITU-T and ISO.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= [UNIVERSAL 5] IMPLICIT INTEGER
END
```

One case is not reported at all, in a closed list of published modules. X.208 (1988) had no `UniversalString`, `BMPString` or `UTF8String`, so modules that keep to 1988 syntax define them with the universal tag a later edition gave the type, as RFC 5280's `PKIX1Explicit88` does: `Name ::= [UNIVERSAL n] IMPLICIT OCTET STRING` with the type's own tag (28, 30, 12). That is correct 1988 ASN.1, and the built-in type has the same tag and encoding. In the modules of `glint_sema::LEGACY_STRING_MODULES` (`PKIX1Explicit88` of RFC 2459, 3280 and 5280, RFC 2510's `PKIXCMP`, RFC 2634's `ExtendedSecurityServices`, RFC 4683's `PKIXSIM` and RFC 5272's `EnrollmentMessageSyntax`, by name and module identifier), neither this code nor `E011` (the name is a reserved word in later editions) is reported on that definition. Anywhere else it is an error, as the example below: a new module has no reason to be written in 1988 syntax, and a compiler of current ASN.1 rejects the definition. Delete it and use the built-in type.

```asn1 bad
M DEFINITIONS ::= BEGIN
  UTF8String ::= [UNIVERSAL 12] IMPLICIT OCTET STRING
END
```

## E124

A textually tagged extension addition in a module with `AUTOMATIC TAGS`, when the root has no tags (X.680 25.9, 29.4).

```asn1 bad
M DEFINITIONS AUTOMATIC TAGS ::= BEGIN
  T ::= SEQUENCE { a INTEGER, ..., b [0] INTEGER OPTIONAL }
END
```

## E125

The version number of an extension addition group is not valid: version numbers start at 2 and must be increasing (X.680 25.16).

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a INTEGER, ..., [[ 1: b INTEGER OPTIONAL ]] }
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a INTEGER, ..., [[ 2: b INTEGER OPTIONAL ]] }
END
```

## E126

`COMPONENTS OF` names a type that is not a `SEQUENCE` (inside a sequence) or a `SET` (inside a set) (X.680 25.5, 27.2).

```asn1 bad
M DEFINITIONS ::= BEGIN
  U ::= SET { a INTEGER }
  T ::= SEQUENCE { COMPONENTS OF U, b BOOLEAN }
END
```

## E130

An object identifier value has fewer than two arcs, a first arc above 2, or a second arc of 40 or more under arcs 0 and 1 (X.660 via X.680 32.11).

```asn1 bad
Mine { 1 } DEFINITIONS ::= BEGIN
END
```

```asn1 good
Mine { 1 3 6 1 4 1 99 1 } DEFINITIONS ::= BEGIN
END
```

## E131

A value does not fit its type. This covers defaults, value assignments, `SEQUENCE`, `CHOICE` and `SEQUENCE OF` values, and object identifier components that are not object identifiers (X.680 14.9, 25.12, Annex C).

```asn1 bad
M DEFINITIONS ::= BEGIN
  v INTEGER ::= TRUE
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  v INTEGER ::= 5
END
```

## E132

A constraint has an empty or inverted range, a negative `SIZE`, or a value outside the extension root of the parent type (X.680 49.7, 50.7–50.11).

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= INTEGER (5..1)
END
```

## E133

A subtype element is not applicable to the parent type (X.680 51.1, Tables 11 and 12). A value range applies to numbers and some strings, not to `OCTET STRING`, for example.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= OCTET STRING (1..4)
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T ::= OCTET STRING (SIZE (1..4))
END
```

## E134

An object identifier uses a bare name form for an arc that X.660 does not register at that position (X.680 32.7, 32.12).

```asn1 bad
M DEFINITIONS ::= BEGIN
  o OBJECT IDENTIFIER ::= { iso recommendation 1 }
END
```

`recommendation` is an arc of `itu-t`, not of `iso`.

## E135

An exception specification (`!`) appears in a constraint of a type that has no extension marker (X.680 49.6).

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= INTEGER (1..5 ! 7)
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T ::= INTEGER (1..5, ... ! 7)
END
```

## I109

An import resolved to a module of an active Internet-Draft. The corpus has the latest revision of every active draft, so a draft can import a module that is not published yet; the note is there because that module may still change. Import resolution by name prefers a current RFC (or a module of another standards body), then an active draft, the most recent first, then an obsoleted RFC; a draft module with syntax errors is never chosen by name.

```asn1 untested
M DEFINITIONS ::= BEGIN
IMPORTS NewThing FROM ModuleOfADraft;
END
```

## W140

An object identifier (or a module identifier) is already assigned in the corpus to a different name. A new draft must not reuse an identifier that an RFC registered.

```asn1 untested
Mine { 1 3 6 1 5 5 7 0 18 } DEFINITIONS ::= BEGIN
END
```

If you are updating an existing module on purpose, keep its name.

## I141

An object identifier uses an author placeholder (`TBD`, `TBD1`, `TBDMOD`, `id-TBD`, any word that starts with `TBD`, an upper-case word written as the number of an arc such as `id-mod-foo(MODNUM)`, or any word in `[oid] placeholders` of `glint.toml`). It is an information message so that you remember to replace the placeholder before the RFC is approved. Raise it with `I141 = "warning"` in `[severity]`.

```asn1 bad
M DEFINITIONS ::= BEGIN
  base OBJECT IDENTIFIER ::= { 1 3 6 1 4 1 99 }
  o OBJECT IDENTIFIER ::= { base id-TBD }
END
```

## W142

A label in an object identifier has a different number than the published documents give it under the same parent. Glint keeps a table of the `name(number)` labels that RFCs and the standalone ITU-T and ETSI modules write for each arc, with verified errata applied; the document being checked is never counted as evidence about itself. The note names the documents that use the other number. RFC 2797 and RFC 5272 wrote `dod(4)` where every other RFC writes `dod(6)`:

```asn1 untested
M DEFINITIONS ::= BEGIN
  id-pkix OBJECT IDENTIFIER ::= { iso(1) identified-organization(3) dod(4) internet(1)
      security(5) mechanisms(5) pkix(7) }
END
```

Wrong but deployed OIDs, such as `holdInstruction` (2.2.840.10040.2), are on an allowlist in the corpus data and are not reported.

## W143

A label with this number appears in the published documents only under a different parent, and the parent written here is one the corpus knows. This usually means an arc is missing or extra before the label. RFC 6268 left out `smime(16)`:

```asn1 untested
M DEFINITIONS ::= BEGIN
  id-aa-multipleSignatures OBJECT IDENTIFIER ::= { iso(1) member-body(2) us(840)
      rsadsi(113549) pkcs(1) pkcs9(9) id-aa(2) 51 }
END
```

A label that the corpus uses under several parents (`id-mod(0)`, say) is not reported.

## W144

`IMPORTS` names a module with an identifier that belongs to a different module in the corpus, while the corpus has the named module under another identifier. The identifier is definitive, so the import resolves to the other module; usually the identifier was copied from the wrong place. (When no module has the identifier and the import resolves by name, `W102` reports the difference instead.)

```asn1 untested
M DEFINITIONS ::= BEGIN
IMPORTS Certificate FROM PKIX1Explicit88
    { iso(1) identified-organization(3) dod(6) internet(1) security(5) mechanisms(5)
      pkix(7) id-mod(0) id-pkix1-implicit(19) };
END
```

## W146

A module's own identifier is in an arc whose identifiers IANA assigns (the "SMI Security for ... Module Identifier" registries: PKIX, S/MIME, LTANS, msec and IPsec ASN.1 modules), and IANA registers that identifier for another module. Usually an arc was copied from the wrong place, or a number was reused. The entry belongs to the module when it cites the module's document, when a document it cites defines a module of the same name, or when its description is the module's label; the check is quiet when IANA does not register the arc or cites only documents outside the corpus. Glint only warns: the module's identifier is still the one imports resolve against.

RFC 5912 gives `AttributeCertificateVersion1-2009` the S/MIME identifier 1.2.840.113549.1.9.16.0.49, which IANA registers as `id-mod-MD5-XOR-EXPERIMENT` (RFC 6210, erratum 3866); IANA registers the module under PKIX as 1.3.6.1.5.5.7.0.49 (erratum 9255). An import that gives the IANA identifier gets `W102` with a note naming both entries.

```asn1 untested
AttributeCertificateVersion1-2009
    {iso(1) member-body(2) us(840) rsadsi(113549) pkcs(1) pkcs-9(9)
    smime(16) modules(0) id-mod-v1AttrCert-02(49)}
DEFINITIONS EXPLICIT TAGS ::= BEGIN
END
```

## I145

A label differs from the name the published documents give the same arc. Different names for one arc are legal and common, so the message is off by default; turn it on with `synonym_labels = true` in `[oid]` of `glint.toml`.

## E150

A field reference is wrong: the class has no such field, the chain goes through a field that is not an object or object set field, `Class.&objectField` is used where an object is required, or an information-from-objects selection is one that X.681 Table 1 forbids (X.681 9.14, 14.5, 15.4–15.5, 15.11).

```asn1 bad
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS { &id OBJECT IDENTIFIER UNIQUE, &Type OPTIONAL }
  T ::= ATTR.&nope
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS { &id OBJECT IDENTIFIER UNIQUE, &Type OPTIONAL }
  T ::= ATTR.&Type
END
```

## E151

A component relation path (`{@a}`, `{@.b}`, `{@..c}`) does not resolve: the identifier is unknown, the path goes through a component that is not a `SET`, `SEQUENCE` or `CHOICE`, it has more dots than there are enclosing constructions, or `@` is used outside any `SET` or `SEQUENCE` (X.682 10.8–10.10).

```asn1 bad
M DEFINITIONS ::= BEGIN
  ERROR-CLASS ::= CLASS { &category PrintableString (SIZE(1)), &code INTEGER, &Type }
    WITH SYNTAX { &category &code &Type }
  ErrorSet ERROR-CLASS ::= { {"A" 1 INTEGER} | {"A" 2 REAL} }
  T ::= SEQUENCE {
    a ERROR-CLASS.&category ({ErrorSet}),
    b ERROR-CLASS.&code ({ErrorSet}{@nope})
  }
END
```

## E152

An object does not match its class: a field is set that the class does not have, a mandatory field is missing, a field is set twice, a token is not allowed by the class's `WITH SYNTAX`, a setting is not of the kind the field needs, or an optional-group literal has no setting after it (X.681 10.11, 10.12 d, 11.4–11.7).

```asn1 bad
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS {
    &id OBJECT IDENTIFIER UNIQUE,
    &Type OPTIONAL
  } WITH SYNTAX { [TYPE &Type] IDENTIFIED BY &id }
  a ATTR ::= { TYPE INTEGER }
END
```

The mandatory field `&id` is missing.

```asn1 good
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS {
    &id OBJECT IDENTIFIER UNIQUE,
    &Type OPTIONAL
  } WITH SYNTAX { [TYPE &Type] IDENTIFIED BY &id }
  a ATTR ::= { TYPE INTEGER IDENTIFIED BY { 1 2 } }
END
```

## E153

An information object class definition is wrong: duplicate field names, `UNIQUE` together with `DEFAULT` or on a field that is not a fixed-type value field, a value field with no type, a variable-type field that does not name a type field or whose optionality disagrees with it, a chain through a field that is not an object field, or a recursive class with no `OPTIONAL` or `DEFAULT` link (X.681 9.6, 9.8, 9.10, 9.13, 9.15).

```asn1 bad
M DEFINITIONS ::= BEGIN
  C ::= CLASS { &id INTEGER, &id BOOLEAN }
END
```

## E154

The `WITH SYNTAX` of a class is wrong: a field is missing, repeated or not in the class, a reserved word is used as a literal, an optional group has no field, adjacent groups start with the same literal, a group that starts with a literal is not followed by a literal, or the brackets are unbalanced (X.681 10.6, 10.9, 10.12 a–c).

```asn1 bad
M DEFINITIONS ::= BEGIN
  C ::= CLASS { &a INTEGER OPTIONAL, &b INTEGER OPTIONAL, &id INTEGER }
    WITH SYNTAX { [X &a] [X &b] ID &id }
END
```

Give the two optional groups different leading literals, such as `[A &a] [B &b]`.

## E155

An object or object set is not of the governing class, or is not an object or object set at all. A separate `CLASS` definition with the same text is a different class (X.681 8.2, 11.3, 12.10).

```asn1 bad
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS { &id OBJECT IDENTIFIER UNIQUE }
  v INTEGER ::= 5
  S ATTR ::= { v }
END
```

## E156

Two objects of one object set have the same value in a `UNIQUE` field (X.681 9.7). Glint compares object identifier, integer and string values that it can evaluate, and skips sets built with `INTERSECTION` or `EXCEPT`.

```asn1 bad
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS { &id OBJECT IDENTIFIER UNIQUE } WITH SYNTAX { IDENTIFIED BY &id }
  a ATTR ::= { IDENTIFIED BY { 1 2 } }
  b ATTR ::= { IDENTIFIED BY { 1 2 } }
  S ATTR ::= { a | b }
END
```

## E157

An object or object set is defined in terms of itself, directly or through others (X.681 11.2, 12.2).

```asn1 bad
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS { &id OBJECT IDENTIFIER UNIQUE }
  S ATTR ::= { S }
END
```

## E158

An empty object set: `{ }` without `...` (X.681 12.3). Write `{ ... }` for a set that is extensible and has no objects yet.

```asn1 bad
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS { &id OBJECT IDENTIFIER UNIQUE }
  S ATTR ::= { }
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  ATTR ::= CLASS { &id OBJECT IDENTIFIER UNIQUE }
  S ATTR ::= { ... }
END
```

## E159

An open type (a type field, a variable-type value field, or a type that contains one) is used as the type of a value or value set field (X.681 14.2 a, 14.4).

```asn1 bad
M DEFINITIONS ::= BEGIN
  C2 ::= CLASS { &Open }
  C ::= CLASS { &Open, &v C2.&Open }
END
```

## E160

The actual parameter list does not match the definition: the wrong number or form of actuals, a parameterized name used without actuals, or actuals given to a name that is not parameterized (X.683 9.2, 9.3, 9.6). Names imported from a module matched only by name when `IMPORTS` gives another identifier (`W102`) are not checked.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T { A, B } ::= SEQUENCE { a A, b B }
  U ::= T { INTEGER }
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T { A, B } ::= SEQUENCE { a A, b B }
  U ::= T { INTEGER, BOOLEAN }
END
```

## E161

A parameter list is wrong: a value or object dummy has no governor, a type or class dummy is used as a value or object set (or the reverse), a governor is not a type or class, or a dummy's governor has a governor itself (X.683 8.3).

```asn1 bad
M DEFINITIONS ::= BEGIN
  T { value } ::= INTEGER (value)
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T { INTEGER : value } ::= INTEGER (value)
END
```

## E162

A dummy reference is used both as a type and as a class, or as a value and as a type (X.683 8.5).

```asn1 bad
M DEFINITIONS ::= BEGIN
  C ::= CLASS { &id INTEGER }
  T { X } ::= SEQUENCE { a X, b X.&id }
END
```

## E163

A dummy reference is never used in its scope (X.683 8.6).

```asn1 bad
M DEFINITIONS ::= BEGIN
  T { A, B } ::= SEQUENCE { a A }
END
```

## E164

Illegal recursion in a parameterized definition: a value, value set, object or object set that refers to itself; a recursive path whose actual parameter mixes a dummy with other text; or a parameterized type that refers to itself with nothing to end the recursion (X.683 8.6–8.8).

```asn1 bad
M DEFINITIONS ::= BEGIN
  v { INTEGER : n } INTEGER ::= v { n }
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  List1 { E } ::= SEQUENCE { elem E, next List1 { E } OPTIONAL }
END
```

## E165

A governor refers to another governed dummy, to the dummy itself, or to the name being defined (X.683 8.9, 8.11).

```asn1 bad
M DEFINITIONS ::= BEGIN
  T { T : x } ::= SEQUENCE { y INTEGER (x) }
END
```

## E166

The right-hand side of a parameterized assignment is only a dummy reference (X.683 8.10).

```asn1 bad
M DEFINITIONS ::= BEGIN
  T { A } ::= A
END
```

## E167

An actual value is not compatible with the governor of its dummy (X.683 8.12). Glint checks values written in the same module as the definition.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T { INTEGER : n } ::= SEQUENCE { a INTEGER (n) }
  U ::= T { "text" }
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T { INTEGER : n } ::= SEQUENCE { a INTEGER (n) }
  U ::= T { 5 }
END
```

## E168

A parameter of a parameterized abstract syntax is not used, directly or through actual parameters, in a constraint (X.683 10.2).

```asn1 bad
M DEFINITIONS ::= BEGIN
  PDU { INTEGER : bound } ::= SEQUENCE { a INTEGER (0..10), b INTEGER DEFAULT bound }
  yyy-Abstract-Syntax { INTEGER : bound } ABSTRACT-SYNTAX ::= { PDU { bound } IDENTIFIED BY { 1 2 3 } }
END
```

## W169

A variable constraint (one that depends on an abstract syntax parameter) has no exception specification (X.683 10.3–10.4). A decoder that finds a value outside the bound has no way to say what happened.

```asn1 bad
M DEFINITIONS ::= BEGIN
  PDU { INTEGER : bound } ::= SEQUENCE { a INTEGER (0..bound) }
  yyy-Abstract-Syntax { INTEGER : bound } ABSTRACT-SYNTAX ::= { PDU { bound } IDENTIFIED BY { 1 2 3 } }
END
```

## W170

A module mixes 1988 syntax (`ANY`) with X.681 constructs. `ANY` was removed in 1994; use an open type from an information object class. Not raised for published modules.

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a ANY }
  C ::= CLASS { &id INTEGER }
END
```

## W171

An all-upper-case name is used for a type, which reads as an information object class (X.681 7.1). Not raised for published modules.

```asn1 bad
M DEFINITIONS ::= BEGIN
  FOO ::= INTEGER
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  Foo ::= INTEGER
END
```

## E172

The module defines a new `MACRO`. Macros were removed from ASN.1 in 2002 and Glint does not process them. Use information object classes. Not raised for published modules.

```asn1 bad
M DEFINITIONS ::= BEGIN
  OP MACRO ::= BEGIN TYPE NOTATION ::= "x" END
END
```

## W173

The module uses imported macro notation (`OPERATION`, `ERROR`, `BIND` and so on from the 1988 remote operations modules). Assignments that use them are not checked. Not raised for published modules.

```asn1 untested
M DEFINITIONS ::= BEGIN
  IMPORTS OPERATION FROM Remote-Operations-Notation;
  op OPERATION ::= { }
END
```

## E180

A component relation constraint is on a type that is not an object class field type or `INSTANCE OF`, or `INSTANCE OF` has a constraint that is not a simple table constraint (X.682 10.1, A.2). A single braced value after another type is read as a single-value constraint (X.681 14.3).

```asn1 bad
M DEFINITIONS ::= BEGIN
  ERROR-CLASS ::= CLASS { &code INTEGER UNIQUE, &Type } WITH SYNTAX { &code &Type }
  ErrorSet ERROR-CLASS ::= { {1 INTEGER} }
  T ::= SEQUENCE { a INTEGER, b INTEGER ({ErrorSet}{@a}) }
END
```

## E181

The constraining object set is not of the class named in the constrained field type, or is not an object set (X.682 10.4, 10.15). The class is the one written before the first `.`.

```asn1 bad
M DEFINITIONS ::= BEGIN
  ERROR-CLASS ::= CLASS { &code INTEGER UNIQUE, &Type } WITH SYNTAX { &code &Type }
  OTHER ::= CLASS { &x INTEGER UNIQUE }
  OtherSet OTHER ::= { {&x 1} }
  T ::= SEQUENCE { a ERROR-CLASS.&code ({OtherSet}) }
END
```

## E182

A component named in a component relation constraint is not an object class field type of the same class, is not a value or value set field, or is not constrained by the same object set (X.682 10.15).

```asn1 bad
M DEFINITIONS ::= BEGIN
  ERROR-CLASS ::= CLASS { &code INTEGER UNIQUE, &Type } WITH SYNTAX { &code &Type }
  ErrorSet ERROR-CLASS ::= { {1 INTEGER} }
  T ::= SEQUENCE { a INTEGER, b ERROR-CLASS.&code ({ErrorSet}{@a}) }
END
```

## E183

The referenced component lies in a different alternative of a `CHOICE` that contains the referencing component (X.682 10.14).

```asn1 bad
M DEFINITIONS ::= BEGIN
  ERROR-CLASS ::= CLASS { &category PrintableString (SIZE(1)), &code INTEGER, &Type }
    WITH SYNTAX { &category &code &Type }
  ErrorSet ERROR-CLASS ::= { {"A" 1 INTEGER} }
  T ::= CHOICE {
    one SEQUENCE { a ERROR-CLASS.&category ({ErrorSet}) },
    two SEQUENCE { b ERROR-CLASS.&code ({ErrorSet}{@one.a}) }
  }
END
```

## E184

A contents constraint (`CONTAINING`, `ENCODED BY`) is on a type other than `OCTET STRING` or a `BIT STRING` without named bits, or is combined with another constraint (directly or through a type reference), or its `ENCODED BY` value is not an object identifier (X.682 11.2–11.3).

```asn1 bad
M DEFINITIONS ::= BEGIN
  T ::= INTEGER (CONTAINING BOOLEAN)
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  T ::= OCTET STRING (CONTAINING BOOLEAN)
END
```

## E185

A `CONSTRAINED BY` parameter is not in a permitted form: a value or object without its governor, or a governor that is a value, object or object set (X.682 9.3).

```asn1 bad
M DEFINITIONS ::= BEGIN
  v INTEGER ::= 5
  T ::= OCTET STRING (CONSTRAINED BY { v })
END
```

```asn1 good
M DEFINITIONS ::= BEGIN
  v INTEGER ::= 5
  T ::= OCTET STRING (CONSTRAINED BY { INTEGER : v })
END
```

## E300

The ASN.1 quoted in the text of a draft is not the same as the definition of the same name in a module. Glint compares the two in the normal form of X.680 Annex C.3 (without white space and comments, with component order, tags and extension parts made explicit), so reordering `CHOICE` alternatives alone is not an error, but a changed tag, a renamed component, a dropped `OPTIONAL` or a changed constraint is. The primary label is the first difference in the snippet; the secondary label is the matching place in the module.

```xml bad
<rfc><middle>
<section><name>Text</name>
<sourcecode type="asn.1">
A ::= SEQUENCE { a INTEGER, b BOOLEAN }
</sourcecode>
</section>
<section><name>Module</name>
<sourcecode type="asn.1">
M DEFINITIONS EXPLICIT TAGS ::= BEGIN
  A ::= SEQUENCE { a INTEGER, b BOOLEAN OPTIONAL }
END
</sourcecode>
</section>
</middle></rfc>
```

Fix whichever side is wrong. To show only part of a type, write `-- glint: partial` in the snippet, or end the omitted part with a comment such as `-- ...` (see `snippets.elision_comments` in `glint.toml`).

## W301

A snippet defines a name that no module defines: not in the draft, not in the modules it imports, and not in the corpus. It is information when the draft imports from a module that is not available (`W100`), since the definition may be there.

```xml bad
<rfc><middle>
<section><name>Text</name>
<sourcecode type="asn.1">
Orphan ::= INTEGER
</sourcecode>
</section>
<section><name>Module</name>
<sourcecode type="asn.1">
M DEFINITIONS EXPLICIT TAGS ::= BEGIN
  A ::= INTEGER
END
</sourcecode>
</section>
</middle></rfc>
```

## W302

The snippet and the module definition have the same normal form but are written differently: alternatives or `SET` components reordered, an object identifier written with names instead of numbers, `IMPLICIT` or `EXPLICIT` spelled out or left to the module default, the two `SEQUENCE SIZE (…) OF` forms, enumeration numbers added or left out, or `COMPONENTS OF` expanded. Readers of the text may not see that the two agree, so consider quoting the module exactly.

```xml bad
<rfc><middle>
<section><name>Text</name>
<sourcecode type="asn.1">
A ::= CHOICE { b BOOLEAN, a INTEGER }
</sourcecode>
</section>
<section><name>Module</name>
<sourcecode type="asn.1">
M DEFINITIONS EXPLICIT TAGS ::= BEGIN
  A ::= CHOICE { a INTEGER, b BOOLEAN }
END
</sourcecode>
</section>
</middle></rfc>
```

## I303

The snippet's name is defined by several modules; the note says which one the snippet matches. Add `-- glint: module=Name` to the snippet to choose the module yourself.

## W304

A `-- glint:` directive is unknown, or `module=` names a module that is neither in the draft nor in the corpus.

```xml bad
<rfc><middle>
<section><name>Text</name>
<sourcecode type="asn.1">
-- glint: nonsense
A ::= INTEGER
</sourcecode>
</section>
<section><name>Module</name>
<sourcecode type="asn.1">
M DEFINITIONS EXPLICIT TAGS ::= BEGIN
  A ::= INTEGER
END
</sourcecode>
</section>
</middle></rfc>
```

The directives are `partial`, `ignore` and `module=Name`.

## W310

A type or class defined in a draft module that the text neither shows in a snippet nor mentions by name. This reverse check is off unless `snippets.reverse_check = true` in `glint.toml`.

```asn1 untested
M DEFINITIONS ::= BEGIN
  Hidden ::= INTEGER
END
```

## E400

The identifier or length octets cannot be read (a tag number or length cut off, the reserved length octet `FF`, an indefinite length on a primitive encoding), a length runs past the end of the input, an indefinite length has no end-of-contents octets, or constructed encodings nest more than 64 deep (X.690 8.1). The octets from there on are shown as they are.

```der bad
04 05 01 02
```

The length says 5 octets, but only 2 follow.

```der good
04 02 01 02
```

## E401

The contents octets are not a valid encoding of a value of the type: a BOOLEAN with other than one octet, a NULL with any, an empty INTEGER or one not written in the fewest octets, an OBJECT IDENTIFIER subidentifier that starts with `80` or is cut off, a BIT STRING with more than 7 unused bits, a UTCTime or GeneralizedTime that is not a valid time, a character the string type cannot hold, or an ENUMERATED value that is not an item of a type that is not extensible (X.690 8.2–8.23).

```der bad
02 02 00 05
```

The leading `00` is not needed: 5 is `02 01 05`.

```der good
02 01 05
```

## W402

The encoding is valid BER but not DER (X.690 10, 11): an indefinite length, a length not written in the fewest octets, a string in the constructed form, BOOLEAN TRUE written as other than `FF`, unused bits of a BIT STRING that are not zero, a BIT STRING with named bits that ends in a zero bit, a component encoded with its `DEFAULT` value, `SET` components out of tag order, `SET OF` elements out of order, or a UTCTime or GeneralizedTime not in the form DER requires. Content decoded without a schema, such as an open type no object set resolves, is checked too; there a `SET` cannot be told from a `SET OF`, so its elements may be in either order. Certificates and other signed structures must be DER, because the signature covers the exact octets. `glint decode --strict-der` fails on these.

```der bad
01 01 01
```

TRUE in DER is `01 01 FF`.

```der good
01 01 FF
```

## E403

The TLV does not match the type the schema expects at its place: another tag, or the primitive form where the type is constructed. The TLV and everything in it are decoded without a schema, and decoding carries on with the next TLV.

```asn1 schema
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE OF INTEGER
END
```

```der bad M.T
30 03 01 01 FF
```

The element is a BOOLEAN, not an INTEGER.

```der good M.T
30 03 02 01 05
```

## E404

A component that is neither `OPTIONAL` nor `DEFAULT` is missing from a `SEQUENCE` or `SET`. Components of extension addition groups may be absent: a sender may use an earlier version of the type.

```asn1 schema
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a INTEGER, b BOOLEAN }
END
```

```der bad M.T
30 03 02 01 05
```

```der good M.T
30 06 02 01 05 01 01 FF
```

## E405

A TLV that is not a component of its `SEQUENCE` or `SET`, when the type is not extensible; a second TLV inside an explicit tag; or octets after the value inside a `CONTAINING` octet string. The TLV is decoded without a schema.

```asn1 schema
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a INTEGER }
END
```

```der bad M.T
30 06 02 01 05 01 01 FF
```

```der good M.T
30 03 02 01 05
```

## I406

A TLV that matches no component of an extensible `SEQUENCE`, `SET` or `CHOICE`: an extension addition from a later version of the type than the schema. X.680 52 asks a decoder to accept it; it is decoded without a schema.

```asn1 schema
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a INTEGER, ... }
END
```

```der bad M.T
30 06 02 01 05 01 01 FF
```

```der good M.T
30 03 02 01 05
```

## E407

The value is outside the constraints of its type: an integer outside its value range, a string, octet string, bit string or collection outside its size range, a character outside the permitted alphabet, or a value that is not the key of any object of the set in a simple table constraint of a set that is not extensible. Extensible constraints (`(1..10, ...)`) are not checked, since a later version may permit more.

```asn1 schema
M DEFINITIONS ::= BEGIN
  T ::= INTEGER (1..10)
END
```

```der bad M.T
02 01 0B
```

```der good M.T
02 01 0A
```

## W408

An open type (a type field read through a component relation constraint, X.682 10) whose type the object set does not give: the component that holds the key was not decoded, the set has no object with the key, or the object does not set the field. The contents are decoded through the OID hint table when it names a type for the key, otherwise without a schema. When the set is not extensible, a key outside it is an error. A value for a field the object does not set is an error too, extensible set or not: the object's cell for the field is empty, so no value satisfies the constraint (X.682 10.20) and the component must be absent, as for an algorithm whose parameters are absent.

```asn1 schema
M DEFINITIONS ::= BEGIN
  ALG ::= CLASS { &id OBJECT IDENTIFIER UNIQUE, &Params OPTIONAL } WITH SYNTAX { ID &id [PARAMS &Params] }
  alg-a ALG ::= { ID { 1 2 3 } PARAMS INTEGER }
  Algs ALG ::= { alg-a, ... }
  T ::= SEQUENCE { id ALG.&id({Algs}), params ALG.&Params({Algs}{@id}) }
END
```

```der bad M.T
30 07 06 02 2A 04 02 01 05
```

`1.2.4` is not the identifier of an object of `Algs`.

```der good M.T
30 07 06 02 2A 03 02 01 05
```

## E409

Octets follow the top-level value: the input holds more than one value, or junk after it. Each further TLV is shown, decoded without a schema.

```der bad
05 00 05 00
```

```der good
05 00
```

## E410

A PEM example (RFC 7468) that cannot be read: it has no `-----END` line, its `-----END` line names another label, its body is not base64 (often an example shortened with `...`), or its octets are not one complete BER encoding. RFC 8792 folding and RFC 1421 headers (`Proc-Type: ...`) are undone first. OpenPGP armor (`-----BEGIN PGP ...-----`) is not PEM and is not checked.

```xml bad
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
MAYCAQUC...
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

```xml good
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
MAYCAQUCAQc=
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

## W411

No type decodes the PEM example. Glint tries, in order, the types its label names (`CERTIFICATE` is a `Certificate` of RFC 5912 or RFC 5280, and so on), the top-level types of the document's own modules (constructed types that no other type of the module uses), then common corpus types (`Certificate`, `ContentInfo`, `OneAsymmetricKey`, ...). A type fits when the structure matches: tags, components and lengths. The closest candidate and its first mismatch are given. `glint decode` with the intended type shows where the example goes wrong.

```xml bad
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
BAIBAg==
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

The octets are an `OCTET STRING`, not a `Pair`.

```xml good
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
MAYCAQUCAQc=
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

## W412

The PEM example has the shape of the type it was identified as, but values that type does not allow: invalid contents (a bad time, a character the string type cannot hold), a constraint violation, or a contained value (an extension's `extnValue`) that is not an encoding of its type. Either the example or the module is wrong.

```xml bad
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
MAYCAQUCAQo=
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

`b` is 10, outside `0..9`.

```xml good
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
MAYCAQUCAQc=
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

## I413

The PEM example is valid BER but not DER (an indefinite length, a `DEFAULT` value encoded, a `SET OF` out of order, ...; see `W402`). Certificates and other signed structures must be DER.

```xml bad
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
MIACAQUCAQcAAA==
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

The `SEQUENCE` has an indefinite length.

```xml good
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
MAYCAQUCAQc=
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

## W415

The PEM example decodes cleanly, but only as a type its label does not name: a PKCS #1 `RSAPrivateKey` labelled `PRIVATE KEY` (which RFC 7468 Section 10 gives to PKCS #8 `OneAsymmetricKey`), a `SubjectPublicKeyInfo` labelled `CERTIFICATE`. Either the label or the octets are wrong; readers' tools go by the label. Only raised when the corpus has the label's types.

```xml bad untested
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PRIVATE KEY-----
MAYCAQUCAQc=
-----END PRIVATE KEY-----
</sourcecode>
</section>
</middle></rfc>
```

With the corpus, the octets are a `Pair`, not the `OneAsymmetricKey` that `PRIVATE KEY` names.

## I414

Names the type each PEM example was identified as, and how: `label` (a type its label names), `decoded` (a type that decodes it, tried without the label's help). Off unless `glint.toml` has `[examples] identified = true`.

```xml good untested
<rfc><middle>
<section><name>Module</name>
<sourcecode type="asn.1">
Example DEFINITIONS EXPLICIT TAGS ::= BEGIN
  Pair ::= SEQUENCE { a INTEGER, b INTEGER (0..9) }
END
</sourcecode>
</section>
<section><name>Example</name>
<sourcecode type="pem">
-----BEGIN PAIR-----
MAYCAQUCAQc=
-----END PAIR-----
</sourcecode>
</section>
</middle></rfc>
```

## E416

`glint decode` reached a type the IR could not lower: a module that is not available (an import that neither a `--file` nor the corpus answers), a symbol that is neither defined nor imported, or macro notation. The TLV is decoded without a schema. Since nothing checked its value against the type, the decode fails: supply the missing module with `--file`, or decode with `--no-schema` to look at the octets.

```asn1 schema
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a Missing }
END
```

```der bad M.T
30 03 02 01 05
```

Without a schema, nothing is claimed about the type:

```der good
30 03 02 01 05
```

## W420

`glint codegen` met a construct the target library cannot express, and the generated code does something else, which the message says: a `DEFAULT` value the target cannot write makes the component `OPTIONAL`; a `REAL`, `EXTERNAL`, `EMBEDDED PDV` or `CHARACTER STRING` value is kept as a raw TLV; an open type whose `@` path no enclosing record holds is kept raw; of several relations, only the first selects the type.

```asn1 bad codegen M.T
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { reading REAL }
END
```

```asn1 good codegen M.T
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { reading INTEGER }
END
```

## W421

A constraint the generated code does not check: `CONSTRAINED BY`, `PATTERN` on a target with no regular expressions, or a constraint whose values cannot be evaluated. Every other constraint is checked when a value is decoded and when it is encoded. An extensible constraint is not checked, since its extensions admit values outside its root.

```asn1 bad codegen M.T
M DEFINITIONS ::= BEGIN
  T ::= OCTET STRING (CONSTRAINED BY { -- signed by the issuer -- })
END
```

```asn1 good codegen M.T
M DEFINITIONS ::= BEGIN
  T ::= OCTET STRING (SIZE (1..64))
END
```

## I422

A type the IR could not lower, such as a reference to a module that is not available, or macro notation. The generated code keeps its value as a raw TLV, which decodes and re-encodes as it is.

```asn1 bad codegen M.T
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a Missing }
END
```

```asn1 good codegen M.T
M DEFINITIONS ::= BEGIN
  T ::= SEQUENCE { a INTEGER }
END
```
