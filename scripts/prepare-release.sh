#!/usr/bin/env bash
# Everything that does not need macOS: source tarball + stable formula.
# usage: prepare-release.sh <version> <dist-dir> [url-base] [tap-url]
set -euo pipefail
version="$1" dist="$2"
url_base="${3:-https://github.com/adgan/contui/releases/download/v$version}"
tap_url="${4:-https://github.com/adgan/homebrew-tap.git}"

mkdir -p "$dist"
sha="$(scripts/release-tarball.sh "$version" "$dist")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
git clone --depth 1 --quiet "$tap_url" "$tmp/tap"
scripts/generate-formula.sh "$version" "$url_base/contui-$version.tar.gz" "$sha" \
  "$tmp/tap/Formula/contui.rb" > "$dist/contui.rb"
echo "$sha"
