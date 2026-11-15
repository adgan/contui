#!/usr/bin/env bash
# Everything that does not need macOS: source tarball + stable formula.
# usage: prepare-release.sh <version> <dist-dir> [url-base] [tap-url]
set -euo pipefail
version="$1" dist="$2"
url_base="${3:-https://github.com/${GITHUB_REPOSITORY:-adgan/contui}/releases/download/v$version}"
tap_url="${4:-https://github.com/${GITHUB_REPOSITORY_OWNER:-adgan}/homebrew-tap.git}"

mkdir -p "$dist"
sha="$(scripts/release-tarball.sh "$version" "$dist")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
git clone --depth 1 --quiet "$tap_url" "$tmp/tap"
# Write to a temp file first: a failing generator must not leave a truncated dist/contui.rb
# that build-bottle.sh would copy over the tap's formula.
scripts/generate-formula.sh "$version" "$url_base/contui-$version.tar.gz" "$sha" \
  "$tmp/tap/Formula/contui.rb" > "$tmp/contui.rb"
grep -q '^class Contui < Formula' "$tmp/contui.rb" || {
  echo "prepare-release.sh: generated formula looks wrong, refusing to write it" >&2
  exit 1
}
mv "$tmp/contui.rb" "$dist/contui.rb"
echo "$sha"
