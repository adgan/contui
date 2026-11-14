#!/usr/bin/env bash
# Same as `just bundle`, for machines without just.
set -euo pipefail
cargo build --release
rm -rf target/contUI.app
mkdir -p target/contUI.app/Contents/MacOS target/contUI.app/Contents/Resources
cp target/release/contui target/contUI.app/Contents/MacOS/contUI
cp assets/Info.plist target/contUI.app/Contents/Info.plist
cp -R assets/fonts target/contUI.app/Contents/Resources/fonts
codesign --force --deep --sign - target/contUI.app
echo "built target/contUI.app"
