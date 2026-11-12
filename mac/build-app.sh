#!/bin/sh
# Build SoundMixer.app: the Rust core, then the SwiftUI app, then the bundle (ad-hoc signed).
# Usage: mac/build-app.sh   → mac/build/SoundMixer.app
set -eu
cd "$(dirname "$0")"
(cd .. && cargo build --release)
swift build -c release
app=build/SoundMixer.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$(swift build -c release --show-bin-path)/SoundMixer" "$app/Contents/MacOS/"
cp Info.plist "$app/Contents/"
codesign --force --sign - "$app"
echo "Built $(pwd)/$app"
