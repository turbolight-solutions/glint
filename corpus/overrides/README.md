# Corpus overrides

An override replaces an extracted module wholesale. Layout: `rfcNNNN/<ModuleName>.asn` and `rfcNNNN/meta.toml`
with one `[[override]]` table per module.

| Tier | Fixes | Review |
| --- | --- | --- |
| `extraction` | Page breaks, whitespace; `corpus-build` checks the tokens equal the extracted ones | any contributor, once CI is green |
| `errata-verified` | Errata with status Verified; cite `errata_ids` (checked with `--errata errata.json`) | one maintainer |
| `correction` | Reported or held errata, obvious typos; `justification` required | two maintainers |

Fields: `tier`, `rfc`, `module`, `errata_ids`, `justification`, `author`, `date`.
