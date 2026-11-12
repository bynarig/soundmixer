#!/usr/bin/env bash
# Build SoundMixer.app: the Rust core, the SwiftUI app, the icon and the bundle.
#   mac/build-app.sh               # this Mac's architecture → mac/build/SoundMixer.app
#   mac/build-app.sh --universal   # arm64 + x86_64 (rustup target add x86_64-apple-darwin)
# Signed with the permanent identity from scripts/setup_codesign.sh when present (macOS then keeps
# the audio permission across updates), otherwise ad-hoc.
set -euo pipefail
cd "$(dirname "$0")/.."

UNIVERSAL=0
[[ "${1:-}" == "--universal" ]] && UNIVERSAL=1
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
UPDATE_FEED=$(python3 -c "import json;print(json.load(open('mac/Updates.json'))['feedURL'])")
UPDATE_KEY=$(python3 -c "import json;print(json.load(open('mac/Updates.json'))['publicKey'])")

SWIFT_ARGS=(-c release)
if [[ $UNIVERSAL == 1 ]]; then
  cargo build --release --locked --target aarch64-apple-darwin --target x86_64-apple-darwin
  # the Swift package links target/release/libsoundmixer.a: make it the universal one
  mkdir -p target/release
  lipo -create target/aarch64-apple-darwin/release/libsoundmixer.a target/x86_64-apple-darwin/release/libsoundmixer.a \
    -output target/release/libsoundmixer.a
  SWIFT_ARGS+=(--arch arm64 --arch x86_64)
else
  cargo build --release --locked
fi
(cd mac && swift build "${SWIFT_ARGS[@]}")
BIN="$(cd mac && swift build "${SWIFT_ARGS[@]}" --show-bin-path)/SoundMixer"

APP=mac/build/SoundMixer.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/SoundMixer"

# icon
ICON_WORK=$(mktemp -d)
trap 'rm -rf "$ICON_WORK"' EXIT
swift scripts/make_icon.swift "$ICON_WORK/icon.png"
mkdir "$ICON_WORK/AppIcon.iconset"
for s in 16 32 128 256 512; do
  sips -z $s $s "$ICON_WORK/icon.png" --out "$ICON_WORK/AppIcon.iconset/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d "$ICON_WORK/icon.png" --out "$ICON_WORK/AppIcon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICON_WORK/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"

# Info.plist: the template plus version, icon and update settings
PLIST="$APP/Contents/Info.plist"
cp mac/Info.plist "$PLIST"
plutil -replace CFBundleShortVersionString -string "$VERSION" "$PLIST"
plutil -replace CFBundleVersion -string "$VERSION" "$PLIST"
plutil -replace CFBundleIconFile -string AppIcon "$PLIST"
plutil -replace SoundMixerUpdateFeedURL -string "$UPDATE_FEED" "$PLIST"
plutil -replace SoundMixerUpdatePublicKey -string "$UPDATE_KEY" "$PLIST"

SIGN_KC="${SOUNDMIXER_CONFIG_DIR:-$HOME/.config/soundmixer}/codesign.keychain-db"
SIGN_ID="SoundMixer Code Signing"
if [[ -f "$SIGN_KC" ]] && security find-identity -p codesigning "$SIGN_KC" 2>/dev/null | grep -q "$SIGN_ID"; then
  PASSFILE="$(dirname "$SIGN_KC")/codesign.keychain-password"
  [[ -f "$PASSFILE" ]] && security unlock-keychain -p "$(cat "$PASSFILE")" "$SIGN_KC"
  codesign --force --keychain "$SIGN_KC" --sign "$SIGN_ID" "$APP"
else
  codesign --force --sign - "$APP"
fi
codesign --verify --strict "$APP"
echo "Built $(pwd)/$APP ($VERSION)"
