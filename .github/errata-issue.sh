#!/usr/bin/env bash
# Keeps one GitHub issue, "RFC errata awaiting triage", in step with `glint errata queue`: opened when something waits
# on a person (errata to triage, needs-human errata, feed entries the tools cannot read), updated when that list
# changes, closed when it is empty. The maintainers in MAINTAINERS are mentioned when it opens or changes, so GitHub
# emails them; a day with the same list changes nothing.
#
# Usage: .github/errata-issue.sh QUEUE_OUTPUT BUILD   (gh needs GH_TOKEN with issues: write, and GH_REPO)
set -euo pipefail

queue_file=$1
build=$2
title="RFC errata awaiting triage"

count() { # the number at the start of the line matching $1, else 0
  local n
  n=$(grep -E -m1 "^[0-9]+ $1" "$queue_file" | cut -d' ' -f1 || true)
  echo "${n:-0}"
}
to_triage=$(count "errata awaiting triage")
needs_human=$(count "needs-human")
skipped=$(count "feed entries skipped")
total=$((to_triage + needs_human + skipped))

mentions=$(grep -E '^@[A-Za-z0-9-]+(/[A-Za-z0-9._-]+)?$' MAINTAINERS | tr '\n' ' ' | sed 's/ $//')
# The scope count changes when an RFC with ASN.1 is published; that is no news for the maintainers.
digest=$(sed -E 's/ \([0-9]+ RFCs in scope\)//' "$queue_file" | sha256sum | cut -c1-16)
marker="<!-- errata-queue: $digest -->"

issue=$(gh issue list --state open --search "\"$title\" in:title" --json number,title \
  --jq "map(select(.title == \"$title\")) | first | .number // empty")

if [ "$total" -eq 0 ]; then
  if [ -n "$issue" ]; then
    gh issue close "$issue" --comment "Nothing waits on a person any more (data build \`$build\`)."
  fi
  exit 0
fi

body=$(mktemp)
{
  echo "$marker"
  echo "The daily data build \`$build\` found RFC errata waiting on a person:"
  echo
  echo "- $to_triage to triage (new, changed, withdrawn, or a patch that no longer applies)"
  echo "- $needs_human \`needs-human\`"
  echo "- $skipped feed entries the tools cannot read (decide them at the erratum's page)"
  echo
  echo "Triage them in a checkout with the \`errata-triage\` skill (\`.claude/skills/errata-triage/SKILL.md\`) after"
  echo "\`corpus/fetch.sh\`; the data build applies a decision the day after it is merged. \`glint errata queue\` says:"
  echo
  echo '```'
  cat "$queue_file"
  echo '```'
  echo
  echo "Errata pages: https://www.rfc-editor.org/errata/eidID. This issue is updated by \`.github/workflows/data.yml\`"
  echo "and closed when the list is empty."
} > "$body"

if [ -z "$issue" ]; then
  { echo; echo "cc $mentions"; } >> "$body"
  gh issue create --title "$title" --body-file "$body"
elif current=$(gh issue view "$issue" --json body --jq .body) && [[ "$current" != *"$marker"* ]]; then
  gh issue edit "$issue" --body-file "$body"
  gh issue comment "$issue" --body "The list changed (data build \`$build\`): $to_triage to triage, $needs_human needs-human, $skipped unreadable. cc $mentions"
fi
