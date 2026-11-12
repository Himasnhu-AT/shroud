#!/usr/bin/env bash
# Bring a gun from an imported pack into the game (see docs/ASSETS.md, "Gun packs").
#
#   scripts/import-gun.sh <model in assets/library/...> <kind> [gun_prepare options]
#     kind: pistol | heavypistol | smg | rifle | battlerifle | shotgun | marksman | sniper
#     options: --length M  --muzzle +x|-x|+y|-y  --keep a,b  --drop a,b  --max-texture PX
#              --roll 180 (upside-down models)  --texture-set assets/library/.../prefix_   (maps for an FBX that lost its materials)
#
# Writes assets/models/weapons/hd/<kind>.glb (game frame, textures capped) and its
# <kind>.lod1.glb, then prints the guns.json entry with estimated anchors. Packs under a
# no-redistribution licence (Fab Standard, Sketchfab Standard) stay local: models/weapons/hd/ is
# gitignored; commit only the guns.json entry and the ledger/CREDITS lines.
set -euo pipefail
cd "$(dirname "$0")/.."
SRC="$1"; KIND="$2"; shift 2
case "$SRC" in assets/library/*) ;; *) echo "import the pack with the engine's assets.py first (into assets/library/)" >&2; exit 1;; esac
BLENDER="${BLENDER:-/Applications/Blender.app/Contents/MacOS/Blender}"
OUT=assets/models/weapons/hd
mkdir -p "$OUT"
"$BLENDER" -b --factory-startup --python scripts/blender/gun_prepare.py -- "$SRC" "$OUT/$KIND.glb" --kind "$KIND" "$@" 2>&1 | grep -E '^gun_prepare|Error|Traceback'
"$BLENDER" -b --factory-startup --python scripts/blender/gun_lod.py -- "$OUT/$KIND.glb" "$OUT/$KIND.lod1.glb" 0.35 2>&1 | grep -E '^gun lod|Error|Traceback'
echo "Add the entry above to assets/models/weapons/guns.json, then check it:"
echo "  python3 -I scripts/play.py scripts/arms-look.json --out .tmp/arms"
