# Distribution

How Glint is packaged and run outside a development checkout (milestone M7).

## Output formats

`glint check --format human|json|sarif`.

- `human` (default) prints excerpts to standard error.
- `json` prints `{ "schema_version": 1, "diagnostics": [...] }` to standard output, one object per document.
- `sarif` prints one SARIF 2.1.0 log to standard output, with one run for all documents. Each code that occurred is a rule (`id`, title, default level, `helpUri`, and the `glint explain` text as help). Primary locations are in the checked document, with a region in characters (`columnKind: unicodeCodePoints`). Secondary labels become `relatedLocations`; a label in a corpus module links to the RFC at the RFC Editor. A finding whose primary label is inside a corpus module is anchored at the top of the checked document, with the real place in the message.

Severity levels map to SARIF as `error` → `error`, `warning` → `warning`, `info` → `note`. Paths in `uri` are the paths given on the command line, with `\` turned into `/` and a leading `./` removed, so run Glint from the repository root for GitHub to place annotations.

## `glint explain`

`glint explain` lists every code; `glint explain E300` prints the title, the clause that is its basis, and a description with `bad` and `good` examples. The text is in `crates/glint-diag/explain.md`, one `## CODE` section per code. `crates/glint-check/tests/explain.rs` runs every `bad` example and requires its code, and runs every `good` example and requires the code to be absent. Examples marked `untested` need a corpus or a setting the test does not have. A new diagnostic code needs a section (the unit tests in `glint-diag` fail without one).

## `glint check --draft`

`glint check --draft draft-ietf-lamps-foo-03` (or `--draft draft-ietf-lamps-foo` for the latest revision, or `--draft rfc5280`) downloads the RFCXML source, or the text if there is no XML, from `www.ietf.org`, `www.rfc-editor.org` or `datatracker.ietf.org`, only over HTTPS and only following redirects inside those hosts. The limit is 5 MiB. `glint.toml` is looked for in the current directory and above.

## `glint corpus update`

`glint corpus update` downloads `corpus.bundle` from the latest GitHub release (or `--url`), checks it against `corpus.bundle.sha256` when that file is there, checks that it reads as a bundle of this version of Glint, and then replaces `corpus.bundle` in the user cache (`$XDG_CACHE_HOME/glint`, `~/Library/Caches/glint`, `%LOCALAPPDATA%\glint`; `GLINT_CACHE_DIR` overrides). `--from <file>` installs a file instead of downloading, for machines without access.

The corpus used by `glint check` and `glint corpus ...` is, in order: `--corpus`, `$GLINT_CORPUS`, the cached bundle, the bundle embedded at build time. A cached bundle that cannot be read (after an upgrade changes the bundle format, say) is skipped with a note. Releases embed the newest corpus, so `update` matters between releases.

## Release binaries

`.github/workflows/release.yml` runs on a `v*` tag:

1. builds the corpus bundle from the RFC mirror (`corpus/fetch.sh`, then `glint-corpus-build`);
2. builds `glint` for Linux x86-64, macOS arm64 and x86-64, and Windows x86-64 with that bundle embedded, packages each as `glint-<target>.tar.gz` (`.zip` on Windows) with `LICENSE` and `NOTICE`, and runs `--version`, `explain` and `corpus list` on the result;
3. creates the GitHub release with the archives, their `.sha256` files, `corpus.bundle`, `corpus.bundle.sha256` and the corpus `report.md`;
4. builds and pushes the container image.

`.github/workflows/ci.yml` builds release binaries for Linux, macOS and Windows on every push and pull request, without a corpus, and runs a smoke test.

## GitHub Action

`action/action.yml` is a composite action: it downloads the release binary for the runner (checking the digest), runs `glint check --format sarif`, uploads the file with `github/codeql-action/upload-sarif`, prints the findings in the log and fails the job if Glint did. Use it as `glint-asn1/glint/action@v0`. Inputs: `files` (paths or globs, one per line), `version`, `config`, `deny-warnings`, `upload`, `category`, `extra-args`. Pull requests from forks cannot upload SARIF (no `security-events: write`); set `upload: "false"` there and read the log.

`examples/i-d-template/.github/workflows/glint.yml` is a ready workflow for a repository made from `martinthomson/i-d-template`.

## Container image

The `Dockerfile` builds the frontend, the server and the CLI and puts them in a distroless image that runs as a non-root user and holds nothing that changes: no volume, no database.

```sh
cp corpus/out/corpus-*.bundle corpus/embedded.bundle      # or the corpus.bundle from a release
docker build -t glint .
docker run --rm -p 8080:8080 glint
```

Configuration is by environment variables (the defaults are in `crates/glint-server/src/settings.rs`):

| Variable | Default | Meaning |
| --- | --- | --- |
| `GLINT_BIND` | `0.0.0.0:8080` in the image, `127.0.0.1:8080` otherwise | Listen address |
| `GLINT_CORPUS` | embedded bundle | Bundle file to load instead |
| `GLINT_STATIC_DIR` | `/app/web` in the image | Built frontend |
| `GLINT_MAX_BODY` | 5 MiB | Largest request body, in bytes |
| `GLINT_FETCH_ALLOWLIST` | `www.ietf.org,datatracker.ietf.org,www.rfc-editor.org` | Hosts a draft may be fetched from |
| `GLINT_CHECK_TIMEOUT_SECS` | 10 | Time for one check |
| `GLINT_REQUEST_TIMEOUT_SECS` | 30 | Time for one request |
| `GLINT_MAX_CONCURRENT_CHECKS` | CPU count | Checks that run at once |
| `GLINT_RATE_LIMIT_PER_MINUTE` | 120 | Requests per client and minute; 0 turns it off |
| `GLINT_TRUST_PROXY` | false | Take the client address from `X-Forwarded-For` (only behind a proxy you control) |
| `RUST_LOG` | `info` | Log filter; logs are JSON and never contain request bodies |

`GET /health` answers 200. Responses are gzip-compressed when the client accepts it, and carry a `Content-Security-Policy` (scripts and styles from the service itself and the pinned Bootstrap on jsDelivr; `object-src 'none'`, `frame-ancestors 'none'`), `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.

## Not done

- **WASM build** (the M7 stretch goal): not started. The core crates are I/O-free, but `glint-corpus` reads bundles with `zstd` (C code) and nothing has been built for `wasm32` yet.
- **The sample repository.** The acceptance test of M7 needs a real GitHub repository with the action installed and a pull request; that has not been done (see the handoff).
