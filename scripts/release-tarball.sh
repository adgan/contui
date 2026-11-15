#!/usr/bin/env bash
# usage: release-tarball.sh <version> <dist-dir> ; prints sha256
set -euo pipefail
version="$1" dist="$2"
mkdir -p "$dist"
tarball="$dist/contui-$version.tar.gz"

# The tag, when it is there. In CI the checkout is already pinned to the tag, so HEAD is
# the same commit; the fallback keeps the Linux prepare job working if the ref is absent.
ref="v$version"
git rev-parse --verify --quiet "refs/tags/$ref" >/dev/null || ref=HEAD
git archive --format=tar.gz --prefix="contui-$version/" -o "$tarball" "$ref"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$tarball" | awk '{print $1}'
else
  shasum -a 256 "$tarball" | awk '{print $1}'
fi
