#!/usr/bin/env bash
# usage: release-tarball.sh <version> <dist-dir> ; prints sha256
set -euo pipefail
version="$1" dist="$2"
mkdir -p "$dist"
tarball="$dist/contui-$version.tar.gz"

# The tarball must come from the tag, never from whatever is checked out. A missing tag
# used to fall back to HEAD, which published a versioned URL whose bytes belonged to no
# tag and could never be reproduced.
ref="v$version"
git rev-parse --verify --quiet "refs/tags/$ref" >/dev/null || {
  echo "release-tarball.sh: tag $ref is not present; tag the release commit first" >&2
  exit 1
}
git archive --format=tar.gz --prefix="contui-$version/" -o "$tarball" "$ref"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$tarball" | awk '{print $1}'
else
  shasum -a 256 "$tarball" | awk '{print $1}'
fi
