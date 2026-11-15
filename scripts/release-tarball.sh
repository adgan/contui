#!/usr/bin/env bash
# usage: release-tarball.sh <version> <dist-dir> ; prints sha256
set -euo pipefail
version="$1" dist="$2"
mkdir -p "$dist"
tarball="$dist/contui-$version.tar.gz"
git archive --format=tar.gz --prefix="contui-$version/" -o "$tarball" "v$version"
shasum -a 256 "$tarball" | awk '{print $1}'
