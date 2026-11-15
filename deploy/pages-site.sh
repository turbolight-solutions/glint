#!/usr/bin/env bash
# Puts together the GitHub Pages site in _site: the site of a code release and the latest data build for the schema
# that release reads. Pages keeps nothing between deploys, so each deploy carries both halves.
#
#   pages-site.sh <tag> [<site zip>] [<data zip>]
#
# Without a site zip, the release's glint-site-<tag>.zip is downloaded (and its digest checked). The data zip, when
# given, is the glint-data-<build>.zip of data.yml. When it has no data for the release's schema, that data is copied
# from the live site, so a code release keeps the data already published.
#
# Needs gh (with GH_TOKEN), curl, jq, unzip and GITHUB_REPOSITORY.
set -euo pipefail
tag="${1:?release tag}"
site_zip="${2:-}"
data_zip="${3:-}"
repo="${GITHUB_REPOSITORY:?set GITHUB_REPOSITORY to owner/name}"
out=_site
work="$(mktemp -d)"
rm -rf "$out"
mkdir -p "$out"

if [ -z "$site_zip" ]; then
  gh release download "$tag" --repo "$repo" --pattern "glint-site-$tag.zip" --pattern "glint-site-$tag.zip.sha256" \
    --dir "$work"
  (cd "$work" && sha256sum --check --quiet "glint-site-$tag.zip.sha256")
  site_zip="$work/glint-site-$tag.zip"
fi
unzip -q "$site_zip" -d "$out"
[ -z "$data_zip" ] || unzip -q -o "$data_zip" -d "$out"

schema="$(gh api "repos/$repo/contents/crates/glint-corpus/src/data.rs?ref=$tag" -H 'Accept: application/vnd.github.raw' |
  sed -n 's/^pub const SCHEMA: u32 = \([0-9][0-9]*\);$/\1/p')"
if [ -z "$schema" ]; then
  echo "::error::Cannot read SCHEMA from crates/glint-corpus/src/data.rs at $tag" >&2
  exit 1
fi
dir="$out/data/v$schema"

if [ ! -f "$dir/latest.json" ]; then
  live="$(gh api "repos/$repo/pages" --jq .html_url 2>/dev/null || true)"
  live="${live%/}"
  if [ -z "$live" ] || ! curl --fail --silent --show-error --location --create-dirs --output "$dir/latest.json" \
    "$live/data/v$schema/latest.json"; then
    echo "::error::No data of schema $schema to publish with $tag: the live site has none. Run the data workflow." >&2
    exit 1
  fi
  build="$(jq -r .build "$dir/latest.json")"
  if ! [[ "$build" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}\.[0-9a-f]{8}$ ]]; then
    echo "::error::$live/data/v$schema/latest.json names no build" >&2
    exit 1
  fi
  for file in corpus.json corpus.json.sha256 documents.bin.gz documents.bin.gz.sha256; do
    curl --fail --silent --show-error --location --create-dirs --output "$dir/$build/$file" \
      "$live/data/v$schema/$build/$file"
  done
  (cd "$dir/$build" && sha256sum --check --quiet corpus.json.sha256 documents.bin.gz.sha256)
fi

build="$(jq -r .build "$dir/latest.json")"
echo "Site: $tag; data: schema $schema, build $build"
rm -rf "$work"
