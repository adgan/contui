#!/usr/bin/env bash
# The only macOS step: install the prepared formula with --build-bottle, bottle it,
# write the bottle block back into the formula. Needs dist/contui.rb from prepare-release.sh.
# usage: build-bottle.sh <version> <dist-dir> [url-base]
set -euo pipefail
version="$1" dist="$2"
url_base="${3:-https://github.com/adgan/contui/releases/download/v$version}"
tap="$(brew --repository adgan/tap)"
formula="$dist/contui.rb"

[ -f "$formula" ] || { echo "build-bottle.sh: no $formula" >&2; exit 1; }
brew tap adgan/tap >/dev/null 2>&1 || true
# A leftover HEAD keg makes `brew bottle` read the wrong keg's receipt ("not installed
# with --build-bottle"), and an installed keg blocks the stable install. Clear both.
brew unlink contui >/dev/null 2>&1 || true
brew uninstall --force contui >/dev/null 2>&1 || true
cp "$formula" "$tap/Formula/contui.rb"

HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 \
  brew install --build-bottle adgan/tap/contui
(cd "$dist" && HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 \
  brew bottle --json --no-rebuild --root-url "$url_base" adgan/tap/contui)
json="$(ls "$dist"/contui--"$version".*.bottle.json)"
brew bottle --merge --write --no-commit "$json"
cp "$tap/Formula/contui.rb" "$formula"

# `brew bottle` names its tarball with the GitHub Packages double dash
# (contui--0.1.0.arm64_tahoe.bottle.tar.gz), but a plain root_url download resolves
# contui-0.1.0.arm64_tahoe.bottle.tar.gz (Utils::Bottles::Filename#url_encode).
# Ship the single-dash name or every pour 404s.
for f in "$dist"/contui--"$version".*.bottle.tar.gz; do
  mv "$f" "$dist/$(basename "$f" | sed "s/^contui--$version/contui-$version/")"
done

ls -1 "$dist"/contui-"$version".*.bottle.tar.gz "$formula"
