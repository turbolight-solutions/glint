#!/usr/bin/env bash
# Writes a .gz (and, when the brotli tool is installed, a .br) beside every file of the site under the given
# directories that compresses well, for nginx's gzip_static and brotli_static. The originals stay.
set -euo pipefail
for dir in "$@"; do
  find "$dir" -type f \( -name '*.html' -o -name '*.js' -o -name '*.css' -o -name '*.wasm' -o -name '*.json' \
    -o -name '*.bin' -o -name '*.svg' \) -print0 |
    while IFS= read -r -d '' file; do
      gzip -9 -k -f -n "$file"
      if command -v brotli >/dev/null; then brotli -q 11 -k -f "$file"; fi
    done
done
