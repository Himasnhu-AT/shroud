#!/usr/bin/env bash
# Push Shroud to itch.io (https://himanshuat.itch.io/shroud) with butler.
#
#   scripts/upload-itch.sh            # push both platforms
#   scripts/upload-itch.sh --dry-run
#
# Build first: scripts/package-windows.sh && scripts/package-macos.sh
# butler must be logged in once (`butler login`). Channels: windows, osx.
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${ITCH_TARGET:-himanshuat/shroud}"
VERSION="${VERSION:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"
BUTLER="${BUTLER:-$(command -v butler || echo "$HOME/.local/bin/butler")}"
DRY=""; [ "${1:-}" = --dry-run ] && DRY="--dry-run"

[ -f dist/windows/Shroud/Shroud.exe ] || { echo "error: dist/windows/Shroud missing (scripts/package-windows.sh)" >&2; exit 1; }
[ -d dist/Shroud.app ] || { echo "error: dist/Shroud.app missing (scripts/package-macos.sh)" >&2; exit 1; }

cat >dist/windows/Shroud/.itch.toml <<'T'
[[actions]]
name = "play"
path = "Shroud.exe"
T
"$BUTLER" push $DRY dist/windows/Shroud "$TARGET:windows" --userversion "$VERSION"
# Only the stapled .app (plus the readme), not the DMG or other dist files.
STAGE="$(mktemp -d)"
ditto dist/Shroud.app "$STAGE/Shroud.app"
sed 's|__DATA_DIR__|~/Library/Application Support/shroud|' packaging/README.txt >"$STAGE/README.txt"
"$BUTLER" push $DRY "$STAGE" "$TARGET:osx" --userversion "$VERSION"
rm -rf "$STAGE"
"$BUTLER" status "$TARGET"
