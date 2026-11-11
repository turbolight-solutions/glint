#!/usr/bin/env bash
# Mirrors the inputs of corpus-build into the git-ignored corpus/cache directory: the RFC texts, RFCXML sources and
# the RFC index; the active Internet-Drafts and their index; and the RFC Editor's errata.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p cache/rfc cache/drafts
rsync -az --delete --include='rfc[0-9]*.txt' --include='rfc[0-9]*.xml' --include='rfc-index.xml' --exclude='*' \
  rsync.rfc-editor.org::rfcs/ cache/rfc/
# The archive keeps the latest revision of each active draft; --delete drops drafts that expired or became RFCs.
rsync -az --delete --include='draft-*.txt' --include='draft-*.xml' --include='all_id2.txt' --exclude='*' \
  rsync.ietf.org::internet-drafts/ cache/drafts/
curl --fail --silent --show-error --location --output cache/errata.json https://www.rfc-editor.org/errata.json
# IANA's SMI Numbers registry: its module identifier registries are the authority for W146.
curl --fail --silent --show-error --location --output cache/smi-numbers.xml \
  https://www.iana.org/assignments/smi-numbers/smi-numbers.xml
echo "now run: cargo run --release -p glint-corpus-build -- --rfc-dir corpus/cache/rfc --drafts-dir corpus/cache/drafts \\"
echo "           --overrides corpus/overrides --errata-dir corpus/errata --sources corpus/sources.toml --errata corpus/cache/errata.json \\"
echo "           --iana corpus/cache/smi-numbers.xml --out corpus/out"
