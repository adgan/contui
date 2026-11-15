#!/usr/bin/env bash
# Rewrites a tap formula for a tagged release: stable url + sha, no bottle block.
# usage: generate-formula.sh <version> <tarball-url> <sha256> <base-formula>
set -euo pipefail
version="$1" url="$2" sha="$3" base="$4"

grep -qx "  version \"$version\"" "$base" || {
  echo "generate-formula.sh: $base has no 'version \"$version\"' line" >&2
  exit 1
}

awk -v url="$url" -v sha="$sha" -v version="$version" '
  $0 ~ /^[[:space:]]*bottle do[[:space:]]*$/ { inbottle = 1; next }
  inbottle && $0 ~ /^[[:space:]]*end[[:space:]]*$/ { inbottle = 0; next }
  inbottle { next }
  { print }
  $0 == "  version \"" version "\"" {
    print "  url \"" url "\""
    print "  sha256 \"" sha "\""
  }
' "$base"
