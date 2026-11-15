# Remaining ASN.1 failures in the corpus

The errors corpus-build and `glint check` still report once every Verified RFC erratum is triaged, why they remain,
and what would fix them. `corpus/errata/to-file.md` lists the defects to report to the RFC Editor; this file is the
wider picture, Glint's own gaps included. Update it when a fix lands (see "How to update").

As of 2026-11-15: corpus build `3a7080be` (482 modules) from a mirror of 2026-11-07.

## Summary

With every Verified erratum triaged (369 decisions, 111 applied), Glint still reports **834 errors in 65 RFC modules
from 54 RFCs**, plus 239 inline mismatches (E300). Many of the module errors are not separate defects: two single
defects repeated line after line account for 240 of them.

| Where | Errors | Modules or RFCs | Notes |
| --- | --- | --- | --- |
| RFC modules, maintained | 546 | 48 modules | A current RFC, or still imported by one |
| RFC modules, unmaintained | 288 | 17 modules | Obsoleted RFC that no maintained module imports |
| Internet-Draft modules | 66 | 19 modules | Drafts can be fixed in their next revision |
| ITU-T and other modules | 12 | 5 modules | From `corpus/sources.toml` |
| Inline ASN.1 in RFCs (E300) | 239 | 298 RFCs checked | A snippet differs from its module; syntax errors in snippets are not reported |

Warnings are left out throughout.

## Why the errors remain

The largest cause is two defects repeated line after line, which are two errata away from 240 fewer errors.

| Cause | Modules | Errors | Of which unmaintained | What it is | Examples | Fix |
| --- | --- | --- | --- | --- | --- | --- |
| One defect repeated | 7 | 240 | 0 | The same mistake on every line: OIDs written with dots inside braces, or every assignment ending in `;` | RFC 4534 (five modules), RFC 2524 (two) | One erratum per RFC |
| Pre-X.680 or non-standard notation | 22 | 208 | 97 | Written before or outside X.680: `OCTETSTRING`, `SEQUENCE` as a name, macros, X.208 forms | RFC 1507, 1142, 1487, the GSS-API modules | Mostly none: historic or obsoleted |
| Cascade from one syntax error | 10 | 196 | 179 | One broken clause (often `IMPORTS`) makes the rest of the module fail | RFC 3126 (two modules), 4683, 2797 | One fix per module |
| Imported module missing from corpus | 4 | 144 | 7 | Imports from X.500 and other modules Glint does not bundle, so every use is "not defined" | RFC 1801, 2510, 3125 | Add the modules to `corpus/sources.toml` |
| Individual semantic defects | 22 | 46 | 5 | Real, isolated errors: tag clashes, wrong classes, duplicate `UNIQUE` values, names a module does not export | RFC 3029, 9690, 4357, 5911, 2459 | Errata, most already in `to-file.md` |

## Every RFC module with errors

The 65 RFC modules that still have errors, worst first. Cause is assigned from each module's first findings and
needs confirming for the smaller entries; "To file" is the section of `corpus/errata/to-file.md` the defect
sits in, blank when no entry exists yet.

| RFC | Module | Status | Maintained | Errors | Main codes | Cause | To file |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1801 | `MHS-DS-Definitions` | Exp | yes | 104 | E110 63, E001 38, E111 2, E011 1 | Missing import |  |
| 3126 | `ETS-ElectronicSignatureFormats-88syntax` | Info (obs. by 5126) | no | 90 | E001 67, E010 14, E101 4, E131 3 | Cascade | low value |
| 4534 | `GSAKMPv1RekeySA` | PS | yes | 88 | E131 88 | Repeated defect | worth filing |
| 3126 | `ETS-ElectronicSignatureFormats-97Syntax` | Info (obs. by 5126) | no | 73 | E001 72, E010 1 | Cascade | low value |
| 2524 | `EMSD-SubmissionAndDeliveryProtocol` | Info | yes | 58 | E001 58 | Repeated defect |  |
| 1507 | `CCITT` | Exp | yes | 51 | E001 19, E110 9, E011 7, E111 5 | Old notation |  |
| 1142 | `ISO10589-ISIS` | Hist (obs. by 7142) | no | 49 | E110 21, E001 12, E131 12, E010 3 | Old notation |  |
| 4534 | `GSAKMPv1RegistrationSA` | PS | yes | 31 | E131 31 | Repeated defect | worth filing |
| 4534 | `GenericDataSA` | PS | yes | 24 | E131 24 | Repeated defect | worth filing |
| 2510 | `PKIXCMP` | PS (obs. by 4210) | yes | 22 | E110 19, E001 3 | Missing import | worth filing |
| 1487 | `Lightweight-Directory-Access-Protocol` | Hist (obs. by 1777, 3494) | no | 20 | E001 12, E011 4, E131 3, E110 1 | Old notation |  |
| 2156 | `Definitions` | PS | yes | 18 | E001 15, E110 2, E101 1 | Old notation |  |
| 4534 | `GSAKMPv1DeRegistrationSA` | PS | yes | 16 | E131 16 | Repeated defect | worth filing |
| 2524 | `EMSD-InterpersonalMessaging1995` | Info | yes | 15 | E001 15 | Repeated defect |  |
| 2797 | `EnrollmentMessageSyntax` | PS (obs. by 5272) | no | 15 | E001 9, E110 4, E101 1, E131 1 | Cascade |  |
| 3029 | `PKIXDVCS` | Exp | yes | 14 | E110 9, E101 2, E121 1, E122 1 | Semantic defect | worth filing |
| 3125 | `ETS-ElectronicSignaturePolicies-88syntax` | Exp | yes | 11 | E110 9, E001 2 | Missing import | worth filing |
| 1507 | `GSSAPI` | Exp | yes | 10 | E010 5, E001 4, E110 1 | Old notation |  |
| 4683 | `PKIXSIM` | PS | yes | 9 | E001 9 | Cascade | worth filing |
| 4534 | `PolicyToken` | PS | yes | 8 | E131 8 | Repeated defect | worth filing |
| 3211 | `PasswordRecipientInfo-97` | PS (obs. by 3369, 3370) | no | 7 | E110 6, E001 1 | Missing import | low value |
| 1507 | `Dass` | Exp | yes | 6 | E001 4, E130 2 | Old notation |  |
| 1508 | `GSS-API` | PS (obs. by 2078) | no | 5 | E001 5 | Old notation |  |
| 1777 | `Lightweight-Directory-Access-Protocol` | Hist (obs. by 3494) | no | 5 | E001 3, E110 2 | Old notation |  |
| 2078 | `GSS-API` | PS (obs. by 2743) | no | 5 | E001 5 | Old notation |  |
| 2157 | `MIXER-MAPPINGS` | PS | yes | 5 | E001 5 | Old notation |  |
| 2743 | `GSS-API` | PS | yes | 5 | E001 5 | Old notation |  |
| 2885 | `MEDIA-GATEWAY-CONTROL` | Hist (obs. by 3015) | no | 5 | E001 4, E110 1 | Old notation |  |
| 3015 | `MEDIA-GATEWAY-CONTROL` | PS (obs. by 3525) | no | 5 | E001 3, E110 2 | Old notation |  |
| 2160 | `POSTSCRIPT-MAPPINGS` | PS | yes | 4 | E001 2, E107 1, E101 1 | Old notation |  |
| 4190 | `CALL-PRIORITY` | Info | yes | 4 | E010 2, E001 2 | Cascade |  |
| 9939 | `PrivateKeyInfoContentTypes` | PS | yes | 4 | E152 4 | Semantic defect | (Reported erratum 9287) |
| 1024 | `OBJECT` | ? | yes | 3 | E001 2, E011 1 | Old notation |  |
| 2875 | `DH-Sign` | PS (obs. by 6955) | no | 3 | E110 2, E001 1 | Old notation |  |
| 3125 | `ETS-ElectronicSignaturePolicies-97Syntax` | Exp | yes | 3 | E101 3 | Semantic defect | worth filing |
| 9690 | `CMS-RSA-KEM-2023` | PS | yes | 3 | E155 3 | Semantic defect | worth filing |
| 1085 | `RFC1085-PS` | ? | yes | 2 | E001 2 | Old notation |  |
| 1214 | `Top-Syntax` | Hist | yes | 2 | E001 2 | Old notation | low value |
| 1494 | `MIME-MHS-MAPPINGS` | Hist | yes | 2 | E001 2 | Old notation |  |
| 2459 | `PKIX1Explicit93` | PS (obs. by 3280) | yes | 2 | E152 2 | Semantic defect | worth filing |
| 2560 | `OCSP` | PS (obs. by 6960) | yes | 2 | E101 2 | Semantic defect |  |
| 5698 | `DSSC` | PS | yes | 2 | E131 2 | Semantic defect |  |
| 5911 | `CMS-AES-CCM-and-AES-GCM-2009` | Info | yes | 2 | E156 2 | Semantic defect | worth filing |
| 1165 | `NTP` | Exp | yes | 1 | E001 1 | Old notation |  |
| 1218 | `NADF-SCHEMA` | Info (obs. by 1255, 1417) | no | 1 | E101 1 | Semantic defect |  |
| 1224 | `MIB` | Exp | yes | 1 | E110 1 | Semantic defect |  |
| 1495 | `MIME-MHS` | PS (obs. by 2156) | yes | 1 | E110 1 | Semantic defect |  |
| 1901 | `COMMUNITY-BASED-SNMPv2` | Hist | yes | 1 | E001 1 | Old notation |  |
| 2459 | `PKIX1Implicit93` | PS (obs. by 3280) | yes | 1 | E114 1 | Semantic defect | worth filing |
| 2630 | `CryptographicMessageSyntax` | PS (obs. by 3369, 3370) | yes | 1 | E101 1 | Semantic defect |  |
| 2649 | `SIGNEDOPERATIONS` | Exp | yes | 1 | E110 1 | Semantic defect |  |
| 2985 | `PKCS-9` | Info | yes | 1 | E101 1 | Semantic defect |  |
| 3161 | `PKIXTSP` | PS | yes | 1 | E001 1 | Cascade |  |
| 3183 | `DOMSECSyntax` | Exp | yes | 1 | E001 1 | Cascade | worth filing |
| 3416 | `SNMPv2-PDU` | Std | yes | 1 | E001 1 | Old notation |  |
| 3739 | `PKIXqualified97` | PS | yes | 1 | E101 1 | Semantic defect |  |
| 4301 | `SPDModule` | PS | yes | 1 | E001 1 | Cascade | worth filing |
| 4357 | `GostR3411-94-DigestSyntax` | Info | yes | 1 | E156 1 | Semantic defect | worth filing |
| 4910 | `MyModule` | Exp | yes | 1 | E110 1 | Semantic defect |  |
| 5272 | `EnrollmentMessageSyntax` | PS (obs. by 10002) | no | 1 | E120 1 | Semantic defect |  |
| 6277 | `OCSP-AGILITY-88` | PS (obs. by 6960) | no | 1 | E001 1 | Cascade | low value |
| 6402 | `EnrollmentMessageSyntax-2011-v08` | PS (obs. by 10002, 10003, 10004) | no | 1 | E121 1 | Semantic defect |  |
| 6402 | `EnrollmentMessageSyntax-2011-v88` | PS (obs. by 10002, 10003, 10004) | no | 1 | E121 1 | Semantic defect |  |
| 7958 | `ResourceRecord` | Info (obs. by 9718) | no | 1 | E110 1 | Semantic defect |  |
| 8017 | `PKCS-1` | Info | yes | 1 | E001 1 | Cascade | worth filing |

Status: PS Proposed Standard, Std Internet Standard, Info Informational, Exp Experimental, Hist Historic, ? not in
the RFC index.

## Inline ASN.1

Inline ASN.1 (definitions quoted in an RFC's body) is checked against the modules: 239 E300 (a definition differs
from its module), 336 W301 (it defines a name no module has) and 156 W302 (same value, written differently) remain,
in 143 RFCs. Most E300 are not defects: the body is often deliberately simpler than the module.

| RFC | E300 | What the differences are | Defect? |
| --- | --- | --- | --- |
| 9688 | 32 | Body writes `OBJECT IDENTIFIER`, the module uses its `OID` alias | No: Glint does not see through the alias |
| 1801 | 19 | Page numbers (`10`, `20`, `30`) extracted into the snippet | No: extraction noise in Glint |
| 4055 | 16 | Body uses 1988 value notation (`{ id-sha1, NULL }`), the module names the components | No: older notation for the same value |
| 2025 | 13 | `US(840)` and an `ANY DEFINED BY` the module types | Partly: `US(840)` is not valid ASN.1 |
| 9810 | 13 | Body uses plain `AlgorithmIdentifier`, the module the parameterized one | No: simplified on purpose |
| 9481 | 12 | `11(11)` as an arc name, so the value cannot be read | Yes, minor: `11(11)` is not a valid name form |
| 2459, 3280, 5280 | 7, 5, 3 | Body component names or types differ from `PKIX1Explicit88` | Check one by one |
| 10002, 5272, 2797 | 7, 4, 3 | Body uses unparameterized types, or a named bound for a number | Mostly no |
| 4210 | 6 | Body says `Certificate`, the module `CMPCertificate` | Yes: the same kind of fix as errata 3949 and 4078, which were patched |
| 59 others | 1 to 4 each | Mixed | Not yet reviewed |

Inline syntax errors are not reported at all, so a broken inline block shows nothing, and fixing it can surface new
W301 or E300. That is why some applied errata (RFC 2631 5897, 3062 4899) added findings.

## Drafts and ITU-T modules

19 Internet-Draft modules have 66 errors, and 5 ITU-T modules have 12. Drafts get no errata: their authors fix them
in the next revision.

| Document | Module | Errors | What is wrong |
| --- | --- | --- | --- |
| draft-wei-aic-identity-cert-02 | `VARWOF-AIC` | 12 | Broken module header, then types it never defines (E001, E110), tag clashes (E121) |
| draft-ietf-plants-merkle-tree-certs-08 | `MerkleTreeCertificates` | 12 | 11 types used but not imported (E110) |
| draft-geng-acme-sm2dualcert-rotation-00 | `SM2-KEY-ROTATION-EXTENSIONS` | 10 | Invalid OID components (E131); imports `SubjectPublicKeyInfo` from a module that lacks it |
| draft-sharif-apki-agent-pki-01 | `APKI-2026` | 6 | OPTIONAL components with clashing tags (E121) |
| draft-braet-sidrops-spa-profile-00 | `RPKI-SPA-2026` | 5 | Object definitions that do not match their class (E152, E153) |
| draft-ietf-lamps-cms-fn-dsa-00 | `FN-DSA-Module-2026` | 3 | Imports objects the certificates draft does not define (E101, E110) |
| draft-vangeest-lamps-update-asn1-signed-type-01 | `AsymmetricKeyPackageModuleV1-2026` | 2 | Uses the `CONTENT-TYPE` syntax that RFC 5911 erratum 2612 replaced (E152) |
| draft-ietf-sidrops-rpki-ccr-11, draft-ietf-sidrops-rpki-erik-protocol-07 | 2 modules | 1 each | A type defined in terms of itself (E157) |
| 11 other drafts | 11 modules | 1 or 2 each | Single import, OID or parameter errors (E101, E131, E160, E152) |
| ITU-T X.501 | `ServiceAdministration` | 6 | Open types in fixed-type fields (E159) |
| ITU-T X.509 | `AuthenticationFramework`, `CertificateExtensions` | 2, 1 | Open types after OPTIONAL components (E121), one undefined value |
| ITU-T X.882 | `Remote-Operations-Realizations` | 2 | Self-referencing definitions (E157) |
| ITU-T X.420 | `PKCS7` | 1 | Imports a name its source module lacks (E101) |

The ITU-T findings are probably checks Glint applies more strictly than X.681 requires (E121 and E159 on open
types), not defects in the Recommendations; worth confirming before reporting anything to ITU-T.

## What to do next

The biggest lever is one erratum for RFC 4534 (167 errors).

Glint

- [ ] Find which X.500 modules RFC 1801 and RFC 2510 import that the corpus lacks, and add them to
  `corpus/sources.toml` (about 80 E110).
- [ ] Stop page numbers from entering extracted inline ASN.1 (RFC 1801, 16 E300).
- [ ] Treat an `OID` type alias as `OBJECT IDENTIFIER` when comparing snippets (RFC 9688, 32 E300).

Errata to file

- [ ] RFC 4534: OIDs written with dots inside braces, in all five modules (167 errors).
- [ ] The other 17 "Worth filing" entries in `corpus/errata/to-file.md`, most important first.
- [ ] Consider new entries not in `to-file.md` yet: RFC 2524's assignments ending in `;` (73 errors,
  Informational), RFC 4210's inline `Certificate` where the module has `CMPCertificate`, RFC 9481's `11(11)` arc
  names, RFC 2025's `US(840)`.

Decisions for a person

- [ ] RFC 3183 erratum 3757 (`needs-human`): its OID lacks `smime(16)`; file a corrected erratum.
- [ ] Feed entry for RFC 2459 erratum 8911, which the tools skip (a control or bidi character): decide it at the
  [errata page](https://www.rfc-editor.org/errata/eid8911).
- [ ] Update `draft-vangeest-lamps-update-asn1-signed-type` to the new `CONTENT-TYPE` syntax in its next revision.

## Method and data

Counts come from the corpus build named under the title. "Maintained" is as `report.md` marks it: a module is
unmaintained when its RFC is obsoleted and no maintained module of another document imports it.

- Module errors are corpus-build's findings of severity error, one row per finding. One defect can produce many
  findings, and a syntax error can hide later ones, so counts are not defect counts.
- Inline counts come from `glint check` on each RFC's text with its errata applied, keeping E300, W301 and W302: the
  checks Glint makes on inline ASN.1.
- Cause groups were assigned by reading each module's first findings; the small entries (5 errors or fewer) need
  confirming.

## How to update

After a fix lands (an erratum patched, a parser or extraction fix, a module added to `sources.toml`), rebuild and
recount:

1. Build the corpus with the findings dumped (`corpus/fetch.sh` first if the mirror is old):

   ```sh
   cargo run --release -p glint-corpus-build -- --rfc-dir corpus/cache/rfc --drafts-dir corpus/cache/drafts \
       --overrides corpus/overrides --errata-dir corpus/errata --sources corpus/sources.toml --errata corpus/cache/errata.json \
       --iana corpus/cache/smi-numbers.xml --out corpus/out --dump-findings corpus/cache/findings.tsv
   ```

2. Module errors: the rows of `corpus/cache/findings.tsv` (document, module, code, line, message) whose code starts with `E`,
   counted per module. `report.md` gives the totals without unmaintained modules and marks them under "Modules
   needing attention".
3. Inline findings: apply each RFC's source patches from `corpus/errata/patches/rfcN/` (`<id>.patch`, not the
   `.txt.patch` ones) to a copy of its source, in id order, run `cargo run --release -p glint-asn1 -- check --format json --corpus corpus/out/data/v4/<build>/corpus.json
   <copy>`, and count E300, W301 and W302.
4. Update the tables and the as-of line, and tick the items in "What to do next". Drop a row when its module is
   clean; move a module to another cause when a fix shows the first diagnosis was wrong.
