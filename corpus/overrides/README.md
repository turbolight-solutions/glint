# Corpus overrides

An override replaces a module wholesale, whichever standards body published it. Layout: `<document>/<ModuleName>.asn`
and `<document>/meta.toml` with one `[[override]]` table per module. The `document` of an entry names the specification
the module is published in, as in the corpus: `rfc4998`, `X.509`, `TS 102 941`. (A directory name is only for
organising; use the document id, with characters a file system accepts.) An override whose module is not in the
corpus is refused, so remove it when the module goes.

A module name does not identify a module: a document can define two modules with the same name and different OIDs
(RFC 4998 defines `ERS` once for 1988 syntax and once for 1997 syntax). When it does, give the entry the
definitive identifier of the module it replaces, as extracted, and name the file `<ModuleName>-<oid>.asn`:

```toml
[[override]]
tier = "correction"
document = "rfc4998"
module = "ERS"
oid = "1.3.6.1.5.5.11.0.1.1"   # rfc4998/ERS-1.3.6.1.5.5.11.0.1.1.asn
```

An entry without `oid` is refused if the document defines that name with more than one OID.

| Tier | Fixes | Review |
| --- | --- | --- |
| `extraction` | Page breaks, whitespace; `corpus-build` checks the tokens equal the extracted ones | any contributor, once CI is green |
| `errata-verified` | Errata with status Verified; cite `errata_ids` (checked with `--errata errata.json`); RFCs only | one maintainer |
| `correction` | Reported or held errata, obvious typos; `justification` required | two maintainers |

Fields: `tier`, `document`, `module`, `oid` (optional), `errata_ids`, `justification`, `author`, `date`.
