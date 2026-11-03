#!/usr/bin/env bash
# Downloads the pinned ITU-T editions (02/2021 for X.680-X.683) into the git-ignored spec/ directory.
set -euo pipefail
cd "$(dirname "$0")"
BASE="${GLINT_ITU_BASE:-https://www.itu.int/rec/dologin_pub.asp?lang=e&id}"
fetch() { # <file> <recommendation-id>
  [ -f "$1" ] || curl -fsSL -o "$1" "$BASE=$2!!PDF-E&type=items"
}
fetch X.680-202102.pdf T-REC-X.680-202102-I
fetch X.681-202102.pdf T-REC-X.681-202102-I
fetch X.682-202102.pdf T-REC-X.682-202102-I
fetch X.683-202102.pdf T-REC-X.683-202102-I
fetch X.660-201107.pdf T-REC-X.660-201107-I
