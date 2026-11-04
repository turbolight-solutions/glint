# Glint

> **Disclaimer:** This project is vibe-coded, and the code has undergone very little review. Expect bugs, and verify anything important independently before relying on it.
>
> A web UI is available at <https://glint.turbolightsolutions.com>.

Glint (binary `glint`) is an ASN.1 checker for Internet-Draft and RFC authors. It parses modules per ITU-T X.680–X.683 (2021), resolves `IMPORTS` against a bundled corpus of every ASN.1 module published in an RFC, and verifies that ASN.1 quoted in a draft's prose matches the draft's own module.

The full design is in [`docs/`](docs/). Work follows the milestones in that spec; **M1 (X.680 core syntax)**, **M2 (extraction)**, **M3 (corpus, imports, core semantics)**, **M4 (X.681–X.683)**, **M5 (snippet consistency)**, **M6 (server and web UI)** and **M7 (distribution)** are what exist today.

## Status

| Milestone | State |
| --- | --- |
| M1 — X.680 core syntax | lexer, recursive-descent parser, lossless CST, error recovery, `glint parse` |
| M2 — Extraction | RFCXML v2/v3, Markdown and plain-text extractors with source maps, `glint extract` |
| M3 — Corpus, imports, core semantics | `corpus-build`, bundle format, overrides, import resolution, checks `W100`, `E101`–`E135`, `W140`, `glint check`, `glint corpus` |
| M4 — X.681, X.682, X.683 | information object classes, objects and object sets with `WITH SYNTAX`, table and component relation constraints, parameterization, checks `E150`–`E168`, `W169`–`W173`, `E180`–`E185`, the resolved type model (`glint_sema::typemodel`) |
| M5 — Snippet consistency | X.680 Annex C.3 normal-form comparison of the assignments quoted in the text with the module definitions (`E300`, `W301`, `W302`, `I303`, `W304`, optional `W310`), `-- glint:` directives and elision comments |
| M6 — Server and web UI | `axum` API (`/api/v1`), draft fetching, static hosting, TypeScript frontend (CodeMirror, Bootstrap), corpus browser, OID lookup |
| M7 — Distribution | SARIF output, `glint explain`, `check --draft`, `corpus update`, release workflow, GitHub Action, Docker image, security headers and compression (WASM not started); see [`docs/distribution.md`](docs/distribution.md) |

## Use

```sh
cargo run -p glint-asn1 -- parse module.asn            # report syntax errors
cargo run -p glint-asn1 -- parse module.asn --tree     # also dump the concrete syntax tree
cargo run -p glint-asn1 -- parse old-rfc.asn --legacy-names --format json
```

```sh
cargo run -p glint-asn1 -- extract draft.xml                 # list the ASN.1 blocks found (also .md, .txt)
cargo run -p glint-asn1 -- extract draft.txt -o modules/     # write modules/<ModuleName>.asn for each module
cargo run -p glint-asn1 -- extract draft.md -o modules/ --fragments
```

```sh
corpus/fetch.sh                                  # mirror the RFCs into corpus/cache/rfc
cargo run --release -p glint-corpus-build -- --rfc-dir corpus/cache/rfc --overrides corpus/overrides \
    --sources corpus/sources.toml --out corpus/out --date $(date +%F)   # bundle + report.md
cp corpus/out/corpus-*.bundle corpus/embedded.bundle      # embedded into the next `cargo build` (git-ignored)
# Modules that other standards bodies publish as files (ITU-T, ETSI, ...) are listed in corpus/sources.toml.

cargo run -p glint-asn1 -- check draft.xml                # extract, parse, resolve imports, run the checks
cargo run -p glint-asn1 -- check draft.txt --format json --pin PKIX1Explicit88=rfc5280   # or --pin ERS=1.3.6.1.5.5.11.0.1.1
cargo run -p glint-asn1 -- corpus list --filter pkix      # also: corpus show <name|oid|rfcNNNN>, corpus oid <dotted>
cargo run -p glint-asn1 -- check draft.xml --format sarif > glint.sarif    # for GitHub code scanning
cargo run -p glint-asn1 -- check --draft draft-ietf-lamps-foo-03          # fetch from the IETF archive, then check
cargo run -p glint-asn1 -- explain E300                   # what a code means, with examples
cargo run -p glint-asn1 -- corpus update                  # newest published corpus bundle into the user cache
```

```sh
GLINT_STATIC_DIR=web/dist cargo run -p glint-server       # API and web UI on 127.0.0.1:8080 (cd web && npm run build first)
```

Without an embedded bundle, pass `--corpus <file>` or set `GLINT_CORPUS`. Settings live in `glint.toml`
(see the design spec).

Exit codes: `0` no errors, `1` errors found (or warnings with `--deny-warnings`), `2` usage or I/O failure.

## Development

```sh
spec/fetch.sh                      # download the pinned ITU-T texts into the git-ignored spec/ directory
cargo test --workspace
cargo clippy --workspace --all-targets
cd fuzz && cargo +nightly fuzz run parser
```

Crate layout, code conventions and diagnostic codes: see the design spec and [`docs/diagnostics.md`](docs/diagnostics.md).

## License

Apache-2.0, see [LICENSE](LICENSE). Fixtures taken from RFC text carry the IETF Trust notice in their headers.
