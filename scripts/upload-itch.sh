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
MAC_ZIP="dist/pattern-of-life-$VERSION-macos-universal.zip"
[ -f "$MAC_ZIP" ] || { echo "error: $MAC_ZIP missing (scripts/package-macos.sh)" >&2; exit 1; }
# Something on this Mac drops Finder `Icon\r` files into new folders; they break the app's
# code seal and have no place in a Windows build either. Strip them right before each push.
clean() { find "$1" -name $'Icon\r' -delete; xattr -cr "$1"; }

cat >dist/windows/PatternOfLife/.itch.toml <<'T'
[[actions]]
name = "play"
path = "PatternOfLife.exe"
T
clean dist/windows/PatternOfLife
"$BUTLER" push $DRY dist/windows/PatternOfLife "$TARGET:windows" --userversion "$VERSION" --ignore "Icon?" --ignore "**/Icon?"
# Only the stapled .app (plus the readme), from the verified zip, not the DMG or other dist files.
STAGE="$(mktemp -d)"
ditto -x -k "$MAC_ZIP" "$STAGE"
sed 's|__DATA_DIR__|~/Library/Application Support/shroud|' packaging/README.txt >"$STAGE/README.txt"
clean "$STAGE"
codesign --verify --strict --deep "$STAGE/Pattern of Life.app"
"$BUTLER" push $DRY "$STAGE" "$TARGET:osx" --userversion "$VERSION" --ignore "Icon?" --ignore "**/Icon?"
rm -rf "$STAGE"
"$BUTLER" status "$TARGET"
