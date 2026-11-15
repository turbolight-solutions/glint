# Distribution

How Glint is packaged and run outside a development checkout (milestone M7).

## Output formats

`glint check --format human|json|sarif`.

- `human` (default) prints excerpts to standard error, then `glint <version>, data <build>`.
- `json` prints `{ "schema_version": 1, "glint_version", "data_build", "diagnostics": [...] }` to standard output, one object per document.
- `sarif` prints one SARIF 2.1.0 log to standard output, with one run for all documents. Each code that occurred is a rule (`id`, title, default level, `helpUri`, and the `glint explain` text as help). Primary locations are in the checked document, with a region in characters (`columnKind: unicodeCodePoints`). Secondary labels become `relatedLocations`; a label in a corpus module links to the RFC at the RFC Editor. A finding whose primary label is inside a corpus module is anchored at the top of the checked document, with the real place in the message.

The run's `properties.glintDataBuild` names the data build, beside the tool's version.

Severity levels map to SARIF as `error` → `error`, `warning` → `warning`, `info` → `note`. Paths in `uri` are the paths given on the command line, with `\` turned into `/` and a leading `./` removed, so run Glint from the repository root for GitHub to place annotations.

## `glint explain`

`glint explain` lists every code; `glint explain E300` prints the title, the clause that is its basis, and a description with `bad` and `good` examples. The text is in `crates/glint-diag/explain.md`, one `## CODE` section per code. `crates/glint-check/tests/explain.rs` runs every `bad` example and requires its code, and runs every `good` example and requires the code to be absent. Examples marked `untested` need a corpus or a setting the test does not have. A new diagnostic code needs a section (the unit tests in `glint-diag` fail without one).

## `glint check --draft`

`glint check --draft draft-ietf-lamps-foo-03` (or `--draft draft-ietf-lamps-foo` for the latest revision, or `--draft rfc5280`) downloads the RFCXML source, or the text if there is no XML, from `www.ietf.org`, `www.rfc-editor.org` or `datatracker.ietf.org`, only over HTTPS and only following redirects inside those hosts. The limit is 5 MiB. `glint.toml` is looked for in the current directory and above.

## Code and data

Glint is code (the `glint` binary, and the site: `index.html`, the scripts and `glint.wasm`) and data (`corpus.json`, `documents.bin.gz`). Code changes only with a release; data changes every day, with RFCs, drafts, errata and overrides. Nothing data-like is compiled into the code.

The data carries a schema number, and code reads exactly one schema; the schema is also in the path, so code never sees data of another schema. The site publishes, per schema:

- `data/v<schema>/latest.json`: `{ "schema", "build" }`, the current build; not cached.
- `data/v<schema>/<build>/corpus.json` and `documents.bin.gz` (the texts of the RFCs and active drafts with an ASN.1 module, for the web UI: an RFC from 8650 on, or a draft with RFCXML, as its published text with the ASN.1 blocks its XML marks, and its (corrected) RFCXML beside it; gzipped by `corpus-build`, because Pages would not compress it, and unpacked by the page), each with a `.sha256` of the file as published; never changed once published. Only the latest build is on the site.

A build id is `<UTC date>.<first 8 hex digits of a SHA-256 of the content>`. `corpus-build` reads the clock once, at its start (`--date` overrides), and everything after reads the id from the data. Every result names the code version and the data build. When the schema changes, the site publishes only the new one; older Glint versions keep the last data of their schema, and their age warning (data over 30 days old) tells their users to upgrade.

## `glint corpus update`

`glint corpus update` reads `data/v<schema>/latest.json` from the Glint site (`--url` names another site), downloads that build's `corpus.json`, checks it against its `.sha256` and that it is data of this schema and of that build, and replaces `corpus.json` in the user cache (`$XDG_CACHE_HOME/glint`, `~/Library/Caches/glint`, `%LOCALAPPDATA%\glint`; `GLINT_CACHE_DIR` overrides), removing the `corpus.bundle` of older versions. `--from <file>` installs a file instead of downloading, for machines without access.

The data used by `glint check` and `glint corpus ...` is, in order: `--corpus`, `$GLINT_CORPUS`, the downloaded `corpus.json`. With none of them those commands fail and say to run `glint corpus update`; there is no download without asking. Data older than 30 days gets a warning.

## Releases and the data workflow

Releases are made on GitHub (Releases → Draft a new release, with a new `v*` tag). Publishing one runs `.github/workflows/release.yml`, which publishes code only:

1. builds `glint` for Linux x86-64, macOS arm64, and Windows x86-64, packages each as `glint-<target>.tar.gz` (`.zip` on Windows) with `LICENSE` and `NOTICE`, and runs `--version`, `explain`, and `corpus list` (which must say how to get data);
2. builds the site (`web/build-wasm.sh`, then Vite) and packages it as `glint-site-<tag>.zip`, the code zip;
3. attaches the archives, the code zip and their `.sha256` files;
4. deploys the code zip to GitHub Pages with the live site's data (`.github/workflows/pages.yml`).

`.github/workflows/data.yml` runs every day: it mirrors the RFCs, the active drafts, the errata and IANA's SMI Numbers registry (`corpus/fetch.sh`), runs `corpus-build`, uploads `glint-data-<build>.zip` (`data/v<schema>/<build>/` and `latest.json`) and `report.md` as artifacts, and deploys the build to GitHub Pages with the site of the latest release (`.github/workflows/pages.yml`). It also runs `glint errata queue` on the build and keeps one issue, "RFC errata awaiting triage", in step with it (`.github/errata-issue.sh`): opened when errata wait on a person, updated with a comment that mentions the maintainers in `MAINTAINERS` when the list changes, closed when it is empty. That job alone has `issues: write`; the issue never holds up the data.

`.github/workflows/ci.yml` builds release binaries for Linux, macOS and Windows on every push and pull request and runs a smoke test with an empty corpus file.

## The site

The site is static and served by GitHub Pages at the custom domain <https://glint.turbolightsolutions.com> (a DNS `CNAME` to `turbolight-solutions.github.io`; the repository's Pages source is "GitHub Actions", its custom domain is set in the Pages settings rather than by a `CNAME` file, and the `github-pages` environment allows the `v*` tags to deploy). Pages keeps nothing between deploys, so every deploy carries a whole site: `deploy/pages-site.sh` unpacks the code zip of a release and adds the latest data build for the schema that release reads, from the data workflow's zip or else from the live site. A release takes the live data and the data workflow takes the latest release's site; a deploy that would publish no data for the release's schema fails instead. So the first deploy is a release followed by a run of the data workflow. `pages.yml` can also be run by hand to redeploy the latest release with the live data.

Vite builds with a relative base, so the site works at any path (also at `turbolight-solutions.github.io/glint/`), and the worker finds the data relative to the page. The page routes with the URL hash, so there is no fallback to `index.html`. Pages sets no headers of ours: the built `index.html` carries the `Content-Security-Policy` in a meta tag (`'wasm-unsafe-eval'` for the wasm, scripts and styles from the site and the pinned Bootstrap on jsDelivr, `object-src 'none'`; `frame-ancestors` cannot be set there) and a `no-referrer` policy. Pages caches everything for ten minutes and compresses by content type itself. `pages.yml` creates the Pages deployment itself (`actions/github-script`) rather than with `actions/deploy-pages`, which always uses the commit as the deployment's build version: Pages treats a build version it already has as deployed, reporting success but serving the earlier site, and the daily data deploy usually runs on a commit already deployed. The build version is `<commit>.<run id>.<attempt>`.

## GitHub Action

`action/action.yml` is a composite action: it downloads the release binary for the runner (checking the digest), downloads the latest corpus data (`glint corpus update`) and prints its build, runs `glint check --format sarif`, uploads the file with `github/codeql-action/upload-sarif`, prints the findings in the log and fails the job if Glint did. Use it as `turbolight-solutions/glint/action@v0`. Inputs: `files` (paths or globs, one per line), `version`, `config`, `deny-warnings`, `upload`, `category`, `extra-args`. Pull requests from forks cannot upload SARIF (no `security-events: write`); set `upload: "false"` there and read the log.

`examples/i-d-template/.github/workflows/glint.yml` is a ready workflow for a repository made from `martinthomson/i-d-template`.

## Not done

- **The sample repository.** The acceptance test of M7 needs a real GitHub repository with the action installed and a pull request; that has not been done (see the handoff).
