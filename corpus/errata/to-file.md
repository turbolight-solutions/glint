# Errata to file

Defects in RFC ASN.1 that no erratum covers, worth reporting to the RFC Editor. Once a defect is reported, its entry
is removed from this list, whatever the erratum's status (Reported, Held for Document Update, Verified); a Verified
one then reaches `glint errata queue` and is triaged like any other. The `errata-triage` skill does this: see
"Errata to file" in `.claude/skills/errata-triage/SKILL.md`.

Each entry names the RFC, the module (or section), the defect, the fix, how it shows (a Glint code, or erlc when Glint
accepts the text), and **Known errata**: the RFC's errata in the feed when the entry was last checked. An erratum of
the RFC that is not in that list is new: if it reports the defect, remove the entry; if not, add its id to the list.
`glint errata list N` prints the RFC's errata, whatever their status.

Most important first: modules that current RFCs import, then the rest.

## RFC 9690, CMS-RSA-KEM-2023

`SMimeCapsSet` lists the key-wrap objects `kwa-aes128-wrap`, `kwa-aes192-wrap` and `kwa-aes256-wrap` (class
`KEY-WRAP`) where `SMIME-CAPS` objects are needed. The Camellia lines next to them have it right. Fix: append
`.&smimeCaps` to the three AES lines. Glint `E155`.
Known errata: none.

## RFC 8017, PKCS-1 (Appendix C)

The comment in `Version ::= INTEGER { two-prime(0), multi(1) } (CONSTRAINED BY {-- version MUST` wraps onto the next
line, so `be multi if otherPrimeInfos present --})` is outside the comment and a syntax error. The copy in Section
A.1.2 is on one line. Fix: rejoin the comment. Glint `E001`.
Known errata: 5111, 5154, 5235, 5576, 5577, 7405.

## RFC 4357, GostR3411-94-DigestSyntax

`GostR3411-94-DigestAlgorithms` lists two objects with the same `UNIQUE` identifier `id-GostR3411-94` (without and
with parameters), which X.681 forbids. Fix: one object whose parameters are optional. Breaks the 8 GOST modules of RFCs
4357 and 4490 that import it. Glint `E156`.
Known errata: 1467, 1473, 4221, 5927, 5928.

## RFC 4534, all five modules

Object identifiers written dotted inside braces (`{1.3.6.1.5.5.12.0.3}`) in headers, imports and values, which is not
ASN.1. Fix: `{1 3 6 1 5 5 12 0 3}`. Glint `E131`.
Known errata: 64, 940, 942, 2351, 2352, 2367, 2368, 2369, 2370, 2371, 2372.

## RFC 6664, PUBLIC-KEY-SMIME-CAPABILITIES and PUBLIC-KEY-SMIME-CAPABILITIES-88

The 2002-syntax module's `SMimeCaps` holds `scap-pk-rsaSSA-PSS` (via `PubKeys-SMimeCaps`) and `scap-sa-rsaSSA-PSS`,
both identified by `id-RSASSA-PSS`, which violates `UNIQUE`; fix: drop one. The 1988-syntax module uses
`[1] pk-dsa.&Params`, 2002 object syntax with `pk-dsa` undefined; fix: `Dss-Parms` from `PKIX1Algorithms88`. erlc.
Known errata: 8657 (not about these).

## RFC 4683, PKIXSIM

The `IMPORTS` clause has no closing `;` (erratum 1047, patched, added an import without it). Glint `E001`.
Known errata: 1047, 2353, 2354, 2355, 2356, 2357, 2358, 2359, 2360, 2361, 2362, 2363.

## RFC 4301, SPDModule

`ANY -- DEFINED BY algorithm} -- ...`: the closing `}` is inside the comment. Erratum 4709 reported the module's
compile errors and was Rejected; a narrower report of this one defect may fare better. Glint `E001`.
Known errata: 135, 717, 1684, 1713, 2178, 2179, 2180, 2181, 2182, 2183, 2184, 2661, 4709, 6635.

## RFC 6277, OCSP-AGILITY-88

A `;` after the `PKIX1Explicit88` import ends `IMPORTS` early, so `SMIMECapability FROM ...` is a syntax error. Fix:
drop that `;`. Glint `E001`.
Known errata: 5892.

## RFC 5275, SMIMESymmetricKeyDistribution

`DEFAULT { id-aes128-wrap }` is a `SEQUENCE` value without its component name. Fix:
`DEFAULT { algorithm id-aes128-wrap }`. erlc (internal crash).
Known errata: 2069, 5943.

## RFC 5911, SMIMESymmetricKeyDistribution-2009

Two imports name a module by its 1988-syntax name with the 2009 module's identifier: `CryptographicMessageSyntaxAlgorithms`
with `...16.0.37` (`CryptographicMessageSyntaxAlgorithms-2009`) and `EnrollmentMessageSyntax` with `...7.0.53`
(`EnrollmentMessageSyntax-2009`). Glint resolves them by identifier; tools that look modules up by name do not. Fix:
the `-2009` names. Glint `W144`.
Known errata: 2612, 2648, 2665, 3128, 9285.

## RFC 4073, ContentCollectionModule

Imports `CryptographicMessageSyntax2004` with the identifier `...16.0.14`, which is RFC 3369's `CryptographicMessageSyntax`;
RFC 3852's `CryptographicMessageSyntax2004` is `...16.0.24`. Fix: one or the other, consistently. Glint `W144`.
Known errata: 762.

## RFC 3029, PKIXDVCS

Tag errors: `PathProcInput` has two `BOOLEAN DEFAULT FALSE` components in a row (`inhibitPolicyMapping`,
`explicitPolicyReqd`), so the second cannot be told from the first (`E121`); `ocspcertstatus [5] IMPLICIT CertStatus`
puts `IMPLICIT` on a `CHOICE` (`E122`); `Data` has `messageImprint DigestInfo` and `certs SEQUENCE ... OF` with the same
tag (`E120`). Fix: tag them. (`Integer` for `INTEGER` is erratum 6445.)
Known errata: 6443, 6444, 6445.

## RFC 3183, DOMSECSyntax

A comment wraps: `identifier` on the next line falls outside the `--` comment. Glint `E001`.
Known errata: 3757.

## RFC 3125, ETS-ElectronicSignaturePolicies-88syntax and -97Syntax

`GeneralName`, `GeneralNames` and `PolicyInformation` are imported from `PKIX1Explicit88`; they are in
`PKIX1Implicit88`. The 88 module also has a comment wrapped onto a ` - Certificate...` line (single dash), and the 97
module imports `PKIX1Explicit93` with `PKIX1Explicit88`'s identifier (`...7.0.1`; `PKIX1Explicit93` is `...7.0.3`).
Glint `E001`, `E110` (88), `E101`, `W144` (97).
Known errata: 5901, 5902.

## RFC 3126, ETS-ElectronicSignatureFormats-88syntax and -97Syntax

`"SignaturePolicy CHOICE {` (a stray quote, and no `SignaturePolicyIdentifier ::=`), upper-case component identifiers,
an object identifier name split across lines (`id-pkix1-explicit-` / `88(1)`), and in the 97 module a comment written
`- - Certificate...`. Glint `E001`, `E010`.
Known errata: 1067.

## RFC 2459, PKIX1Explicit93

`pkcs9email ATTRIBUTE ::= { WITH SYNTAX PHGString, ID ... }`: the comma breaks the class's defined syntax. Glint `E152`.
Known errata: 432.

## RFC 2510, PKIXCMP

Two `FROM` identifiers end in `}}`, and `IMPORTS` is not closed. Glint `E001`.
Known errata: 9183.

## RFC 3211, PasswordRecipientInfo-97

A trailing comma after `PBKDF2-params` before `FROM`, and `FROM PKCS5 {... pkcs-5(5)}` names the pkcs-5 arc, not a
module: the module meant is RFC 2898's `PKCS5v2-0` (`{... pkcs-5(5) modules(16) pkcs-5v2-0(1)}`). Glint `E001`.
Known errata: none.

## RFC 3560, CMS-RSAES-OAEP

X.208 value notation throughout: `SEQUENCE` values without component names (`{ id-sha1, NULL }`,
`{ id-mgf1, sha1Identifier }`, nested `{ id-RSAES-OAEP, {...} }` for an `ANY` parameter). RFC 4055's module is the
maintained form, so a note pointing there may be the better erratum. erlc (internal crash).
Known errata: none.

## RFC 1214, Top-Syntax

X.208 unnamed components (`SEQUENCE {AttributeType, AttributeValue}`); naming them is enough. Historic. Glint `E001`.
Known errata: none.
