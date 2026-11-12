#!/usr/bin/env bash
# The Tactical Soldier Outfit (Fab, RMDavid; Fab Standard: local only) fitted to the enemy's
# MetaHuman: assets/models/gear/outfit_mh.glb. With it the enemy wears the outfit instead of the
# generated gear (gear_mh.glb), which stays the fallback (enemy/skin.rs).
#
#   scripts/make-outfit-mh.sh [~/Downloads/fab_assets_tells/soldier_outfit_textures.zip]
#
# Needs assets/models/metahuman/enemy_mh.glb (scripts/make-metahuman.sh) and the outfit imported
# with the engine's importer (assets/library/characters/tactical-soldier-outfit/). The texture
# zip is unpacked into a fresh temp folder (paths checked, nothing executed) and deleted after.
set -euo pipefail
cd "$(dirname "$0")/.."
BLENDER="${BLENDER:-/Applications/Blender.app/Contents/MacOS/Blender}"
ZIP="${1:-$HOME/Downloads/fab_assets_tells/soldier_outfit_textures.zip}"
SRC=assets/models/metahuman/enemy_mh.glb
LIB=assets/library/characters/tactical-soldier-outfit
OUT=assets/models/gear/outfit_mh.glb
[ -f "$SRC" ] || { echo "no $SRC: run scripts/make-metahuman.sh first" >&2; exit 1; }
[ -f "$LIB/Soldier_Outfit_v1.glb" ] || { echo "import source.zip with the engine's assets.py first ($LIB)" >&2; exit 1; }
TMP="$(mktemp -d "${TMPDIR:-/tmp}/outfit_tex.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
python3 -I - "$ZIP" "$TMP" <<'PY'
import os, sys, zipfile
src, dst = sys.argv[1], os.path.realpath(sys.argv[2])
total = 0
with zipfile.ZipFile(src) as z:
    for i in z.infolist():
        total += i.file_size
        t = os.path.realpath(os.path.join(dst, i.filename))
        if total > 2 << 30 or not t.startswith(dst + os.sep) or (i.external_attr >> 16) & 0o170000 == 0o120000:
            sys.exit(f"refusing {i.filename}")
        if i.filename.lower().endswith((".png", ".jpg", ".jpeg")) and "Gun_Texture" not in i.filename:
            z.extract(i, dst)
PY
mkdir -p "$(dirname "$OUT")"
"$BLENDER" -b --factory-startup --python scripts/blender/make_outfit_mh.py -- "$PWD/$SRC" "$PWD/$LIB/Soldier_Outfit_v1.glb" \
  "$PWD/$LIB/Soldier_Helmet_Outfit_v1.glb" "$TMP" "$PWD/$OUT" 2>&1 | grep -E '^outfit|Error|Traceback' || true
python3 -I scripts/gear_rebind.py "$SRC" "$OUT"
echo "wrote $OUT"
