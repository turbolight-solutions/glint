#!/usr/bin/env bash
# Publishes to the Glint host over SSH. Layout on the host, under /var/www/glint:
#   releases/<version>/   the site of one code release; `current` points at the live one
#   data/v<schema>/<build>/  one data build, never changed once published; old builds stay for pins
#   data/v<schema>/latest.json  the pointer to the current build, replaced last
#
#   deploy.sh code <site dir> <version>
#   deploy.sh data <dir holding data/>
#
# Needs DEPLOY_TARGET (user@host) and an SSH key and known_hosts entry for it.
set -euo pipefail
target="${DEPLOY_TARGET:?set DEPLOY_TARGET to user@host}"
base=/var/www/glint
case "${1:-}" in
  code)
    site="${2:?site directory}"
    version="${3:?version}"
    rsync -a --delete "$site/" "$target:$base/releases/$version/"
    ssh "$target" "ln -sfn '$base/releases/$version' '$base/current'"
    ;;
  data)
    root="${2:?directory holding data/}"
    for pointer in "$root"/data/v*/latest.json; do
      schema_dir="$(dirname "$pointer")"
      schema="$(basename "$schema_dir")"
      # The build directories first, so the pointer never names a build that is not there yet.
      rsync -a --exclude latest.json "$schema_dir/" "$target:$base/data/$schema/"
      rsync -a "$pointer" "$target:$base/data/$schema/.latest.json.new"
      ssh "$target" "mv -f '$base/data/$schema/.latest.json.new' '$base/data/$schema/latest.json'"
    done
    ;;
  *)
    echo "usage: deploy.sh code <site dir> <version> | deploy.sh data <dir holding data/>" >&2
    exit 2
    ;;
esac
