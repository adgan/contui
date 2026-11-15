#!/usr/bin/env bash
# The only macOS step: install the prepared formula with --build-bottle, bottle it,
# write the bottle block back into the formula. Needs dist/contui.rb from prepare-release.sh.
# usage: build-bottle.sh <version> <dist-dir> [url-base]
set -euo pipefail
version="$1" dist="$2"
url_base="${3:-https://github.com/${GITHUB_REPOSITORY:-adgan/contui}/releases/download/v$version}"
tap="$(brew --repository adgan/tap)"
formula="$dist/contui.rb"

grep -q '^class Contui < Formula' "$formula" 2>/dev/null || {
  echo "build-bottle.sh: $formula is missing or not a formula; run prepare-release.sh first" >&2
  exit 1
}
grep -q "contui-$version.tar.gz" "$formula" || {
  echo "build-bottle.sh: $formula does not point at contui-$version.tar.gz" >&2
  exit 1
}

brew tap adgan/tap >/dev/null 2>&1 || true
# This tap checkout is shared with the machine's real installs. Put the formula back on
# the way out, success or failure, so a local run never leaves a file:// formula behind.
backup="$(mktemp)"
cp "$tap/Formula/contui.rb" "$backup"
trap 'cp "$backup" "$tap/Formula/contui.rb"; rm -f "$backup"' EXIT

# A leftover HEAD keg makes `brew bottle` read the wrong keg's receipt ("not installed
# with --build-bottle"), and an installed keg blocks the stable install. Clear both.
brew unlink contui >/dev/null 2>&1 || true
brew uninstall --force contui >/dev/null 2>&1 || true
cp "$formula" "$tap/Formula/contui.rb"

brew install --build-bottle adgan/tap/contui
(cd "$dist" && brew bottle --json --no-rebuild --root-url "$url_base" adgan/tap/contui)
json="$(ls -1 "$dist"/contui--"$version".*.bottle.json | head -1)"
brew bottle --merge --write --no-commit "$json"
cp "$tap/Formula/contui.rb" "$formula"

# `brew bottle` names its tarball with the GitHub Packages double dash
# (contui--0.1.0.arm64_tahoe.bottle.tar.gz), but a plain root_url download resolves
# contui-0.1.0.arm64_tahoe.bottle.tar.gz (Utils::Bottles::Filename#url_encode).
# Ship the single-dash name or every pour 404s.
for f in "$dist"/contui--"$version".*.bottle.tar.gz; do
  mv "$f" "$dist/$(basename "$f" | sed "s/^contui--$version/contui-$version/")"
done

# Test the keg here, not in the caller. `brew test` refuses unless the installed keg is
# the tap's latest version, and the EXIT trap above puts the tap formula back to whatever
# `brew tap` cloned — the previous release. Run after that and only the release already
# in the tap could ever pass.
brew test adgan/tap/contui

ls -1 "$dist"/contui-"$version".*.bottle.tar.gz "$formula"
