#!/usr/bin/env bash
# Same as `just bundle`, for machines without just.
set -euo pipefail
cargo build --release
rm -rf target/Dockyard.app
mkdir -p target/Dockyard.app/Contents/MacOS target/Dockyard.app/Contents/Resources
cp target/release/dockyard target/Dockyard.app/Contents/MacOS/Dockyard
cp assets/Info.plist target/Dockyard.app/Contents/Info.plist
cp -R assets/fonts target/Dockyard.app/Contents/Resources/fonts
codesign --force --deep --sign - target/Dockyard.app
echo "built target/Dockyard.app"
