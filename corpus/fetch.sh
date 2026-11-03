#!/usr/bin/env bash
# Mirrors the RFC texts, RFCXML sources and the RFC index into the git-ignored corpus/cache/rfc directory.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p cache/rfc
rsync -az --include='rfc[0-9]*.txt' --include='rfc[0-9]*.xml' --include='rfc-index.xml' --exclude='*' \
  rsync.rfc-editor.org::rfcs/ cache/rfc/
echo "now run: cargo run --release -p glint-corpus-build -- --rfc-dir corpus/cache/rfc --overrides corpus/overrides \\"
echo "           --itu-manifest corpus/itu-modules.toml --out corpus/out --date \$(date +%F)"
