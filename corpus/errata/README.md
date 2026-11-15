# RFC errata

Verified RFC errata that correct ASN.1, in modules or inline, applied to the RFCs before `corpus-build` extracts
them (`--errata-dir corpus/errata`). The corrected modules name the errata that changed them (`source.errata` in
`corpus.json`, `I105` when a check uses one), and `report.md` lists the errata applied per RFC.

- `ledger.tsv`: one row per triaged erratum, sorted by (`rfc`, `errata_id`): `rfc`, `errata_id`, `decision`
  (`patched`, `not-applicable`, `superseded`, `needs-human`), `by` (`claude`, `human`), `fingerprint` (of the
  erratum's reviewed content in `errata.json`, so a changed erratum is triaged again), `rule` (`modules`: decided
  when only modules counted; `asn1`: modules and inline ASN.1), `note` (one line, at most 500 characters, never a
  name or an email address; a `superseded` note names the RFC that replaces the text).
- `patches/rfcN/<errata_id>.patch`: a unified diff of the document `corpus-build` reads, `rfcN.xml` for RFCs with
  RFCXML (8650 on) and `rfcN.txt` otherwise, with headers exactly `--- a/rfcN.xml` and `+++ b/rfcN.xml`. Each patch
  applies to the published document plus the RFC's lower-numbered patched errata.
- `patches/rfcN/<errata_id>.txt.patch`, for RFCs whose patches are against `rfcN.xml`: the same correction to the
  published `rfcN.txt` (headers `--- a/rfcN.txt` and `+++ b/rfcN.txt`), which a reader sees. It applies to the
  published text plus the RFC's lower-numbered text patches, under the same rules. `glint errata record` derives it
  from the XML patch when that changes only `<sourcecode>` and `<artwork>` content, block by block; otherwise it is
  the change made to the text work file. `glint errata check` requires one per patched erratum, checks that the series
  applies and that each agrees with its XML patch. corpus-build applies them to the text the web UI shows.
- `to-file.md`: defects in RFC ASN.1 that no erratum reports yet, each with the RFC's known errata. An entry is
  removed once an erratum reports it, whatever its status (the `errata-triage` skill checks with `glint errata list`).

A patch is applied only while its ledger row says `patched`. An RFC's patches apply in erratum-id order, all or
nothing, and without line numbers: each hunk's old side (context and removed lines) must occur exactly once in the
text, without fuzz, so a patch never lands on the wrong copy of a definition the RFC repeats. A series that does not
apply leaves its RFC as published and is reported; malformed data (ledger, stray files, a patch without its row, a
control or bidi character in an added line, with `--errata` a patched erratum that is not a Verified erratum of its
RFC) fails the build.

Errata that only fix prose are not applied. Fixes that are not Verified errata are overrides (`corpus/overrides`).

The daily data build keeps an issue, "RFC errata awaiting triage", open while `glint errata queue` lists anything
waiting on a person, and mentions the maintainers in `MAINTAINERS` when that list changes. A feed entry the tools
cannot read (a control or bidi character) is decided by a person at its erratum page and recorded with `glint errata
record N ID --by human`; its ledger row has a fingerprint of zeros, so the entry is triaged again if the feed
corrects it.
