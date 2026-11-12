#!/usr/bin/env bash
# Build Tells for Windows x64 on this Mac (cross-compiled with cargo-xwin):
#
#   dist/windows/Tells/       Tells.exe + assets/ + README.txt + CREDITS.md
#   dist/tells-<version>-windows-x64.zip
#
#   scripts/package-windows.sh                # build and zip
#   scripts/package-windows.sh --skip-build   # re-zip what the target dir holds
#   FULL_TEXTURES=1 scripts/package-windows.sh # ship the 4K textures (default cuts them to 2K)
#   TELEMETRY=1 scripts/package-windows.sh     # crash/perf/match reports on (use scripts/release.sh)
#
# Needs: cargo-xwin (cargo install cargo-xwin), rustup target x86_64-pc-windows-msvc,
# llvm-rc (brew install llvm). The exe is a GUI app (no console window) with a static CRT, so it
# runs on a clean Windows 10/11 without the VC++ redistributable.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
GAME_DIR="$(cd .. && pwd)"

SKIP_BUILD=0
for a in "$@"; do
  case "$a" in
    --skip-build) SKIP_BUILD=1 ;;
    -h | --help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "error: unknown argument $a" >&2; exit 2 ;;
  esac
done

step() { printf '\n==> %s\n' "$*"; }
die() { echo "error: $*" >&2; exit 1; }

VERSION="${VERSION:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"
BUILD="$(git rev-list --count HEAD 2>/dev/null || echo 0)"
GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
. scripts/ship-features.sh   # FEATURES, check_ship_tree, SHROUD_BUILD_SHA
TARGET=x86_64-pc-windows-msvc
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')"
BIN="$TARGET_DIR/$TARGET/release/shroud.exe"
RES_DIR="$TARGET_DIR/winres"
DIST="$ROOT/dist"
OUT="$DIST/windows/Tells"
ZIP="$DIST/tells-$VERSION-windows-x64.zip"

if [ "$SKIP_BUILD" = 0 ]; then
  command -v cargo-xwin >/dev/null || die "cargo-xwin missing (cargo install cargo-xwin)"
  rustup target list --installed | grep -qx "$TARGET" || die "rustup target add $TARGET"
  RC="${RC_PATH:-$(command -v llvm-rc || echo /opt/homebrew/opt/llvm/bin/llvm-rc)}"
  [ -x "$RC" ] || die "llvm-rc not found (brew install llvm, or set RC_PATH)"

  step "Check: no dev tooling in the ship build ($SHIP_KIND)"
  check_ship_tree --target "$TARGET"

  step "Resources: icon + version info"
  mkdir -p "$RES_DIR"
  IFS=. read -r V1 V2 V3 <<<"${VERSION%%-*}"
  cat >"$RES_DIR/shroud.rc" <<EOF
1 ICON "$ROOT/packaging/shroud.ico"
1 VERSIONINFO
FILEVERSION $V1,$V2,$V3,$BUILD
PRODUCTVERSION $V1,$V2,$V3,$BUILD
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Tells"
      VALUE "FileDescription", "Tells"
      VALUE "FileVersion", "$VERSION"
      VALUE "InternalName", "shroud"
      VALUE "OriginalFilename", "Tells.exe"
      VALUE "ProductName", "Tells"
      VALUE "ProductVersion", "$VERSION ($GIT_SHA)"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
EOF
  # Relative names: llvm-rc on macOS reads an argument starting with / as an option.
  ( cd "$RES_DIR" && "$RC" -nologo -fo shroud.res shroud.rc )

  step "Build release for $TARGET (no dev tooling)"
  # Static CRT, and build-machine paths rewritten out of panic messages.
  export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static \
    --remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$HOME/.rustup=/rustup \
    --remap-path-prefix=$GAME_DIR=/src"
  # `cargo rustc` passes the flags after `--` to the shroud binary only: the .res, and the GUI
  # subsystem with the normal entry point (no console window).
  SMTP_USER= SMTP_PASS= cargo xwin rustc --release -p game --bin shroud \
    "${FEATURES[@]}" --target "$TARGET" -- \
    -C "link-arg=$RES_DIR/shroud.res" -C link-arg=/SUBSYSTEM:WINDOWS -C link-arg=/ENTRY:mainCRTStartup
fi

[ -f "$BIN" ] || die "$BIN missing (build first, or drop --skip-build)"
check_baked "$BIN"

step "Check the exe: x64, GUI subsystem, version info, no MSVC runtime DLLs"
python3 -I - "$BIN" <<'EOF'
import struct, sys
path = sys.argv[1]
b = open(path, "rb").read()
pe = struct.unpack_from("<I", b, 0x3C)[0]
errors = []
if struct.unpack_from("<H", b, pe + 4)[0] != 0x8664: errors.append("not x64")
if struct.unpack_from("<H", b, pe + 0x5C)[0] != 2: errors.append("not the GUI subsystem")
if "VS_VERSION_INFO".encode("utf-16-le") not in b: errors.append("no VERSIONINFO resource")
for dll in (b"vcruntime140.dll", b"msvcp140.dll"):
    if dll in b.lower(): errors.append(f"imports {dll.decode()} (static CRT didn't apply)")
if errors: sys.exit(f"{path}: " + "; ".join(errors))
print(f"ok: x64, GUI subsystem, VERSIONINFO, static CRT ({len(b) / 1e6:.1f} MB)")
EOF

step "Stage $OUT"
rm -rf "$OUT"; mkdir -p "$OUT"
cp "$BIN" "$OUT/Tells.exe"
# The engine finds the assets folder next to the exe.
# Shipped assets: no MetaHuman, no source JPGs of cooked sets, KTX2s over 2048 px cut to 2K
# (FULL_TEXTURES=1 keeps 4K). See scripts/stage-assets.sh.
scripts/stage-assets.sh "$OUT/assets"
cp assets/CREDITS.md "$OUT/CREDITS.md"
sed -e 's|__DATA_DIR__|%APPDATA%\\shroud\\data|' packaging/README.txt | perl -pe 's/\r?\n/\r\n/' >"$OUT/README.txt"
if strings "$OUT/Tells.exe" | grep -q "$GAME_DIR"; then
  echo "note: the exe still contains the build folder path (GameInfo::assets_fallback); harmless" >&2
fi

step "Zip"
rm -f "$ZIP"
( cd "$DIST/windows" && zip -qr -X "$ZIP" Tells )
step "Done: $ZIP ($(du -h "$ZIP" | cut -f1))"
