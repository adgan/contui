#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# `! cmd` does not trip `set -e` (bash ignores -e for a negated command), so every
# negative assertion goes through refute, which fails loudly.
assert_contains() { # needle, haystack-file, message
  grep -qxF "$1" "$2" || { echo "FAIL: $3 (missing: $1)" >&2; exit 1; }
}
refute_contains() {
  if grep -qF "$1" "$2"; then echo "FAIL: $2 contains forbidden: $1" >&2; exit 1; fi
}

base="$tmp/base.rb"
cat > "$base" <<'RB'
class Contui < Formula
  desc "Fast native Docker client for macOS"
  homepage "https://github.com/adgan/contui"
  version "0.1.0"
  license "MIT"

  bottle do
    root_url "https://example.test/old"
    sha256 arm64_tahoe: "deadbeef"
  end

  head "https://github.com/adgan/contui.git", branch: "main"

  depends_on "rust" => :build
  depends_on macos: :tahoe
  depends_on arch: :arm64

  def install
    system "cargo", "install", *std_cargo_args
  end
end
RB

sha="1111111111111111111111111111111111111111111111111111111111111111"
scripts/generate-formula.sh 0.1.0 "https://example.test/contui-0.1.0.tar.gz" "$sha" "$base" > "$tmp/out.rb"

ruby -c "$tmp/out.rb" > /dev/null
assert_contains '  url "https://example.test/contui-0.1.0.tar.gz"' "$tmp/out.rb" "url inserted"
assert_contains "  sha256 \"$sha\"" "$tmp/out.rb" "sha inserted"
assert_contains '  head "https://github.com/adgan/contui.git", branch: "main"' "$tmp/out.rb" "head kept"
assert_contains '  depends_on macos: :tahoe' "$tmp/out.rb" "macos gate kept"
assert_contains '  depends_on arch: :arm64' "$tmp/out.rb" "arch gate kept"
refute_contains 'bottle do' "$tmp/out.rb"
refute_contains 'deadbeef' "$tmp/out.rb"
# `brew audit --strict` rejects a version that only repeats the URL, and wants the url
# before where `version` used to be.
refute_contains 'version "' "$tmp/out.rb"
[ "$(grep -n 'url "' "$tmp/out.rb" | cut -d: -f1 | head -1)" = 4 ]

# Idempotent: regenerating from an already-generated formula gives the same bytes.
scripts/generate-formula.sh 0.1.0 "https://example.test/contui-0.1.0.tar.gz" "$sha" "$tmp/out.rb" > "$tmp/again.rb"
cmp -s "$tmp/out.rb" "$tmp/again.rb" || { echo "FAIL: not idempotent" >&2; exit 1; }

# A released formula keeps its bottle block between `head` and `depends_on`, with blank
# lines on both sides. Stripping it must not leave a double blank line: `brew style`
# fails on Layout/EmptyLines and the maintainer would hand-edit the formula again.
released="$tmp/released.rb"
cat > "$released" <<'RB'
class Contui < Formula
  desc "Fast native Docker client for macOS"
  homepage "https://github.com/adgan/contui"
  url "https://example.test/contui-0.0.9.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"

  head "https://github.com/adgan/contui.git", branch: "main"

  bottle do
    root_url "https://example.test/0.0.9"
    sha256 cellar: :any_skip_relocation, arm64_tahoe: "cafebabe"
  end

  depends_on "rust" => :build
  depends_on arch: :arm64
  depends_on macos: :tahoe
end
RB
scripts/generate-formula.sh 0.1.0 "https://example.test/contui-0.1.0.tar.gz" "$sha" "$released" > "$tmp/next.rb"
if awk 'prev == "" && $0 == "" { found = 1 } { prev = $0 } END { exit !found }' "$tmp/next.rb"; then
  echo "FAIL: stripping the bottle block left a double blank line" >&2
  exit 1
fi
assert_contains '  head "https://github.com/adgan/contui.git", branch: "main"' "$tmp/next.rb" "head kept across regeneration"
refute_contains '0.0.9' "$tmp/next.rb"

echo "generate_formula_test: ok"
