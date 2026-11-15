#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

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
out="$(scripts/generate-formula.sh 0.1.0 "https://example.test/contui-0.1.0.tar.gz" "$sha" "$base")"

printf '%s\n' "$out" | ruby -c > /dev/null
printf '%s\n' "$out" | grep -qx '  url "https://example.test/contui-0.1.0.tar.gz"'
printf '%s\n' "$out" | grep -qx "  sha256 \"$sha\""
printf '%s\n' "$out" | grep -qx '  version "0.1.0"'
printf '%s\n' "$out" | grep -qx '  head "https://github.com/adgan/contui.git", branch: "main"'
printf '%s\n' "$out" | grep -qx '  depends_on macos: :tahoe'
printf '%s\n' "$out" | grep -qx '  depends_on arch: :arm64'
! printf '%s\n' "$out" | grep -q 'bottle do'
! printf '%s\n' "$out" | grep -q 'deadbeef'

echo "generate_formula_test: ok"
