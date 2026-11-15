#!/usr/bin/env bash
# Rewrites a tap formula for a tagged release: stable url + sha, no bottle block, no
# `version` line (Homebrew reads the version from the URL, and `brew audit --strict`
# rejects one that repeats it). Idempotent: feed it its own output and nothing changes.
# usage: generate-formula.sh <version> <tarball-url> <sha256> <base-formula>
set -euo pipefail
version="$1" url="$2" sha="$3" base="$4"

grep -q '^  homepage "' "$base" || {
  echo "generate-formula.sh: $base has no 'homepage \"…\"' line to anchor the url" >&2
  exit 1
}

# The URL carries the version; make sure the caller's build agrees with it.
case "$url" in
  *"$version".tar.gz) ;;
  *) echo "generate-formula.sh: $url does not carry version $version" >&2; exit 1 ;;
esac

awk -v url="$url" -v sha="$sha" '
  /^[[:space:]]*bottle do[[:space:]]*$/ { inbottle = 1; next }
  inbottle && /^[[:space:]]*end[[:space:]]*$/ { inbottle = 0; next }
  inbottle { next }
  /^[[:space:]]*url "/ { next }
  /^[[:space:]]*sha256 "/ { next }
  /^[[:space:]]*version "/ { cover = 1; next }
  cover && /^[[:space:]]*$/ { cover = 0; next }
  { cover = 0; print }
  /^[[:space:]]*homepage "/ {
    print "  url \"" url "\""
    print "  sha256 \"" sha "\""
  }
' "$base"
