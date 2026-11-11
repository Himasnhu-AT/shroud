#!/usr/bin/env bash
# Build a universal, Developer ID-signed, notarized Pattern of Life dmg for sharing.
#
#   scripts/package-macos.sh            # build, sign, notarize, staple
#   NOTARIZE=0 scripts/package-macos.sh # skip notarization (local test)
#   FULL_TEXTURES=1 scripts/package-macos.sh # ship the 4K textures (default cuts them to 2K)
#   TELEMETRY=1 scripts/package-macos.sh     # crash/perf/match reports on (use scripts/release.sh,
#                                            # which bakes in the server and write key)
#
# Env: SIGN_ID (Developer ID Application identity), NOTARY_PROFILE (notarytool keychain profile),
#      BUNDLE_ID, VERSION. Output: dist/PatternOfLife-<version>.dmg, and
#      dist/pattern-of-life-<version>-macos-universal.zip (the stapled app, for itch.io / butler).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
SIGN_ID="${SIGN_ID:-Developer ID Application: Himanshu . (38QXSRV6AP)}"
NOTARY_PROFILE="${NOTARY_PROFILE:-xcnotarize}"
# The bundle id keeps the codename: changing it would reset keychain/notarization identity.
BUNDLE_ID="${BUNDLE_ID:-com.himanshuat.shroud}"
VERSION="${VERSION:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"
BUILD="$(git rev-list --count HEAD)"
NOTARIZE="${NOTARIZE:-1}"
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')"
DIST="$ROOT/dist"; APP="$DIST/Pattern of Life.app"; DMG="$DIST/PatternOfLife-$VERSION.dmg"
ZIP="$DIST/pattern-of-life-$VERSION-macos-universal.zip"
export MACOSX_DEPLOYMENT_TARGET=12.0

step() { printf '\n==> %s\n' "$*"; }

. scripts/ship-features.sh   # FEATURES, check_ship_tree, SHROUD_BUILD_SHA

step "Check: no dev tooling in the ship build ($SHIP_KIND)"
check_ship_tree

step "Build release (no dev tooling) for arm64 and x86_64"
# The executable inside the bundle stays `shroud` (the cargo binary name).
# Build-machine paths rewritten out of panic messages and debug info (as package-windows.sh).
GAME_DIR="$(cd .. && pwd)"
export RUSTFLAGS="${RUSTFLAGS:-} \
  --remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$HOME/.rustup=/rustup \
  --remap-path-prefix=$GAME_DIR=/src"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  SMTP_USER= SMTP_PASS= cargo build --release -p game "${FEATURES[@]}" --target "$t"
done

for t in aarch64-apple-darwin x86_64-apple-darwin; do check_baked "$TARGET_DIR/$t/release/shroud"; done

step "Assemble Pattern of Life.app"
rm -rf "$APP" "$DMG"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/shroud" \
  "$TARGET_DIR/aarch64-apple-darwin/release/shroud" "$TARGET_DIR/x86_64-apple-darwin/release/shroud"
# Shipped assets: no MetaHuman, no source JPGs of cooked sets, KTX2s over 2048 px cut to 2K
# (FULL_TEXTURES=1 keeps 4K). See scripts/stage-assets.sh.
scripts/stage-assets.sh "$APP/Contents/Resources/assets"
cp assets/CREDITS.md "$APP/Contents/Resources/CREDITS.md"
sed -e "s/__BUNDLE_ID__/$BUNDLE_ID/" -e "s/__VERSION__/$VERSION/" -e "s/__BUILD__/$BUILD/" packaging/Info.plist > "$APP/Contents/Info.plist"
[ -f packaging/AppIcon.icns ] && cp packaging/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# Something on this Mac drops Finder `Icon\r` files and FinderInfo into new folders; they break
# the code seal. Strip them before signing, and zip from a clean copy below.
find "$APP" -name $'Icon\r' -delete; xattr -cr "$APP"

step "Sign (hardened runtime, secure timestamp)"
codesign --force --timestamp --options runtime --entitlements packaging/entitlements.plist --sign "$SIGN_ID" "$APP/Contents/MacOS/shroud"
codesign --force --timestamp --options runtime --entitlements packaging/entitlements.plist --sign "$SIGN_ID" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

step "Make DMG"
STAGE="$(mktemp -d)"; cp -R "$APP" "$STAGE/"; ln -s /Applications "$STAGE/Applications"
sed 's|__DATA_DIR__|~/Library/Application Support/shroud|' packaging/README.txt >"$STAGE/Read me first.txt"
hdiutil create -volname "Pattern of Life" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
codesign --force --timestamp --sign "$SIGN_ID" "$DMG"

if [ "$NOTARIZE" = "1" ]; then
  step "Notarize (a few minutes)"
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  step "Staple"
  xcrun stapler staple "$DMG"
  xcrun stapler staple "$APP"
  spctl -a -t open --context context:primary-signature -vv "$DMG"
  find "$APP" -name $'Icon\r' -delete; xattr -cr "$APP"
  spctl -a -t exec -vv "$APP"
fi
step "Zip the app for itch.io"
rm -f "$ZIP"
ZSTAGE="$(mktemp -d)"
ditto --norsrc --noextattr --noqtn "$APP" "$ZSTAGE/Pattern of Life.app"
find "$ZSTAGE" -name $'Icon\r' -delete; xattr -cr "$ZSTAGE"
codesign --verify --strict --deep "$ZSTAGE/Pattern of Life.app"
( cd "$ZSTAGE" && ditto -c -k --norsrc --noextattr --noqtn --keepParent "Pattern of Life.app" "$ZIP" )
rm -rf "$ZSTAGE"
step "Done: $DMG ($(du -h "$DMG" | cut -f1)), $ZIP ($(du -h "$ZIP" | cut -f1))"
