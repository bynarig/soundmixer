#!/usr/bin/env bash
# Build a signed SoundMixer update for GitHub Releases.
#
#   scripts/release.sh --setup            # once: create your update signing key (keep it safe!)
#   scripts/release.sh "What's new"       # build dist/release/{SoundMixer-X.Y.Z.zip, appcast.json}
#   scripts/release.sh --universal "…"    # same, arm64 + x86_64
#
# Normally you don't run this yourself: scripts/new_release.sh pushes a tag and
# .github/workflows/release.yml runs it. The repository must be public so the
# app can download updates without logging in.
set -euo pipefail
cd "$(dirname "$0")/.."

KEY="${SOUNDMIXER_SIGNING_KEY:-$HOME/.config/soundmixer/update_signing_key}"
CONFIG=mac/Updates.json
REPO=$(python3 -c "import json;print(json.load(open('$CONFIG'))['repo'])")
MIN_MACOS=$(/usr/libexec/PlistBuddy -c "Print LSMinimumSystemVersion" mac/Info.plist)

if [[ "${1:-}" == "--setup" ]]; then
  PUB=$(swift scripts/update_tool.swift generate-keys "$KEY")
  python3 - "$CONFIG" "$PUB" <<'PY'
import json, sys
p, pub = sys.argv[1], sys.argv[2]
c = json.load(open(p)); c["publicKey"] = pub
open(p, "w").write(json.dumps(c, indent=2) + "\n")
PY
  echo "Signing key created: $KEY"
  echo "Public key written to $CONFIG: $PUB"
  echo "BACK UP $KEY (e.g. in your password manager). Without it you can't ship updates to existing installs."
  exit 0
fi

UNIVERSAL=()
if [[ "${1:-}" == "--universal" ]]; then UNIVERSAL=(--universal); shift; fi
NOTES="${1:-}"
[[ -f "$KEY" ]] || { echo "No signing key at $KEY — run: scripts/release.sh --setup" >&2; exit 1; }
EXPECTED=$(python3 -c "import json;print(json.load(open('$CONFIG'))['publicKey'])")
ACTUAL=$(swift scripts/update_tool.swift public-key "$KEY")
[[ "$EXPECTED" == "$ACTUAL" ]] || { echo "Signing key doesn't match publicKey in $CONFIG" >&2; exit 1; }

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
mac/build-app.sh ${UNIVERSAL[@]+"${UNIVERSAL[@]}"}

OUT=dist/release
rm -rf "$OUT"; mkdir -p "$OUT"
ZIP="$OUT/SoundMixer-$VERSION.zip"
ditto -c -k --keepParent mac/build/SoundMixer.app "$ZIP"
SIG=$(swift scripts/update_tool.swift sign "$KEY" "$ZIP")
swift scripts/update_tool.swift verify "$ACTUAL" "$ZIP" "$SIG" >/dev/null

python3 - "$OUT/appcast.json" "$VERSION" "https://github.com/$REPO/releases/download/v$VERSION/SoundMixer-$VERSION.zip" "$SIG" "$NOTES" "$MIN_MACOS" <<'PY'
import json, sys
path, version, url, sig, notes, minimum = sys.argv[1:]
json.dump({"version": version, "url": url, "signature": sig, "notes": notes, "minimumSystemVersion": minimum},
          open(path, "w"), indent=2, ensure_ascii=False)
PY

echo
echo "Release $VERSION ready in $OUT:"
ls -lh "$OUT"
