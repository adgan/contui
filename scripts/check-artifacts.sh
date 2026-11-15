#!/usr/bin/env bash
# Assert the shaped things a user actually depends on, for artifacts already in <dist-dir>.
# usage: check-artifacts.sh <version> <dist-dir>
set -euo pipefail
version="$1" dist="$2"
src="$dist/contui-$version.tar.gz"
bottle="$dist/contui-$version.arm64_tahoe.bottle.tar.gz"

fail() { echo "check-artifacts.sh: $*" >&2; exit 1; }

[ -f "$src" ] || fail "missing $src"
[ -f "$bottle" ] || fail "missing $bottle"

# Source tarball: enough to build, and the bundle inputs the formula copies.
for path in "contui-$version/Cargo.toml" "contui-$version/Cargo.lock" \
            "contui-$version/assets/Info.plist" "contui-$version/assets/fonts/Inter.ttf"; do
  tar -tzf "$src" | grep -qx "$path" || fail "source tarball is missing $path"
done
tar -tzf "$src" | grep -q "contui-$version/src/" || fail "source tarball is missing src/"

# Bottle: the launchable bundle plus the CLI the formula links.
for path in "contui/$version/contUI.app/Contents/MacOS/contUI" \
            "contui/$version/contUI.app/Contents/Info.plist" \
            "contui/$version/bin/contui"; do
  tar -tzf "$bottle" | grep -qx "$path" || fail "bottle is missing $path"
done
tar -tzf "$bottle" | grep -q "contui/$version/contUI.app/Contents/_CodeSignature/" ||
  fail "bottle's app is not signed"

# Formula: parses, pins this version, names a bottle for this platform.
ruby -c "$dist/contui.rb" > /dev/null
grep -qE "^  url \"[^\"]*contui-$version\.tar\.gz\"" "$dist/contui.rb" ||
  fail "formula url is not v$version"
grep -q 'bottle do' "$dist/contui.rb" || fail "formula has no bottle block"
grep -q 'arm64_tahoe:' "$dist/contui.rb" || fail "formula has no arm64_tahoe bottle"

echo "check-artifacts.sh: ok ($version)"
