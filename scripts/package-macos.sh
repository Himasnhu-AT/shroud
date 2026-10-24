#!/usr/bin/env bash
# Build a universal, Developer ID-signed, notarized Shroud.dmg for sharing.
#
#   scripts/package-macos.sh            # build, sign, notarize, staple
#   NOTARIZE=0 scripts/package-macos.sh # skip notarization (local test)
#
# Env: SIGN_ID (Developer ID Application identity), NOTARY_PROFILE (notarytool keychain profile),
#      BUNDLE_ID, VERSION. Output: dist/Shroud-<version>.dmg
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
SIGN_ID="${SIGN_ID:-Developer ID Application: Himanshu . (38QXSRV6AP)}"
NOTARY_PROFILE="${NOTARY_PROFILE:-xcnotarize}"
BUNDLE_ID="${BUNDLE_ID:-com.himanshuat.shroud}"
VERSION="${VERSION:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"
BUILD="$(git rev-list --count HEAD)"
NOTARIZE="${NOTARIZE:-1}"
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')"
DIST="$ROOT/dist"; APP="$DIST/Shroud.app"; DMG="$DIST/Shroud-$VERSION.dmg"
export MACOSX_DEPLOYMENT_TARGET=12.0

step() { printf '\n==> %s\n' "$*"; }

step "Check: no dev tooling (ashlar-*) in the ship build"
if cargo tree -p game --no-default-features -e normal | grep ashlar; then echo "ashlar crates in the ship build" >&2; exit 1; fi

step "Build release (no dev tooling) for arm64 and x86_64"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  memgate --profile release -- cargo build --release -p game --no-default-features --target "$t"
done

step "Assemble Shroud.app"
rm -rf "$APP" "$DMG"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/shroud" \
  "$TARGET_DIR/aarch64-apple-darwin/release/shroud" "$TARGET_DIR/x86_64-apple-darwin/release/shroud"
rsync -a --exclude '.DS_Store' --exclude '*.md' assets/ "$APP/Contents/Resources/assets/"
cp assets/CREDITS.md "$APP/Contents/Resources/CREDITS.md"
sed -e "s/__BUNDLE_ID__/$BUNDLE_ID/" -e "s/__VERSION__/$VERSION/" -e "s/__BUILD__/$BUILD/" packaging/Info.plist > "$APP/Contents/Info.plist"
[ -f packaging/AppIcon.icns ] && cp packaging/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

step "Sign (hardened runtime, secure timestamp)"
codesign --force --timestamp --options runtime --entitlements packaging/entitlements.plist --sign "$SIGN_ID" "$APP/Contents/MacOS/shroud"
codesign --force --timestamp --options runtime --entitlements packaging/entitlements.plist --sign "$SIGN_ID" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

step "Make DMG"
STAGE="$(mktemp -d)"; cp -R "$APP" "$STAGE/"; ln -s /Applications "$STAGE/Applications"
cp packaging/README-friends.txt "$STAGE/Read me first.txt"
hdiutil create -volname "Shroud" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
codesign --force --timestamp --sign "$SIGN_ID" "$DMG"

if [ "$NOTARIZE" = "1" ]; then
  step "Notarize (a few minutes)"
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  step "Staple"
  xcrun stapler staple "$DMG"
  xcrun stapler staple "$APP"
  spctl -a -t open --context context:primary-signature -vv "$DMG"
  spctl -a -t exec -vv "$APP"
fi
step "Done: $DMG ($(du -h "$DMG" | cut -f1))"
