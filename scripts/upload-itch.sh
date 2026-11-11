#!/usr/bin/env bash
# Push Pattern of Life to itch.io (https://himanshuat.itch.io/patternoflife) with butler.
# ITCH_TARGET overrides the user/game (default himanshuat/patternoflife).
#
#   scripts/upload-itch.sh            # push both platforms
#   scripts/upload-itch.sh --dry-run
#
# Build first: scripts/package-windows.sh && scripts/package-macos.sh
# butler must be logged in once (`butler login`). Channels: windows, osx.
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${ITCH_TARGET:-himanshuat/patternoflife}"
VERSION="${VERSION:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"
BUTLER="${BUTLER:-$(command -v butler || echo "$HOME/.local/bin/butler")}"
DRY=""; [ "${1:-}" = --dry-run ] && DRY="--dry-run"

[ -f dist/windows/PatternOfLife/PatternOfLife.exe ] || { echo "error: dist/windows/PatternOfLife missing (scripts/package-windows.sh)" >&2; exit 1; }
[ -d "dist/Pattern of Life.app" ] || { echo "error: dist/Pattern of Life.app missing (scripts/package-macos.sh)" >&2; exit 1; }

cat >dist/windows/PatternOfLife/.itch.toml <<'T'
[[actions]]
name = "play"
path = "PatternOfLife.exe"
T
"$BUTLER" push $DRY dist/windows/PatternOfLife "$TARGET:windows" --userversion "$VERSION"
# Only the stapled .app (plus the readme), not the DMG or other dist files.
STAGE="$(mktemp -d)"
ditto "dist/Pattern of Life.app" "$STAGE/Pattern of Life.app"
sed 's|__DATA_DIR__|~/Library/Application Support/shroud|' packaging/README.txt >"$STAGE/README.txt"
"$BUTLER" push $DRY "$STAGE" "$TARGET:osx" --userversion "$VERSION"
rm -rf "$STAGE"
"$BUTLER" status "$TARGET"
