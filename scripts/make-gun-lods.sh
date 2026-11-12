#!/usr/bin/env bash
# Low-detail gun models for enemy hands and distant pickups: each high-detail gun
# (models/weapons/<name>.glb) becomes models/weapons/lod/<name>.lod1.glb in the same frame
# (metres, barrel toward -Z), static (rest pose baked, no rig), material names kept so the
# finishes in weapon/finish.rs still apply.
#
#   scripts/make-gun-lods.sh [RATIO] [name ...]     (default ratio 0.35, every built-in gun)
#
# Default: headless Blender's quadric decimation with sharp edges kept (scripts/blender/gun_lod.py).
# The engine's asset-pipe (vertex clustering) is kept as LOD=asset-pipe, but on these
# hard-surface guns it averages normals across hard edges and breaks thin parts (trigger guards).
# ASSET_PIPE=<binary> uses a prebuilt asset-pipe; otherwise it is run from the engine.
set -euo pipefail
cd "$(dirname "$0")/.."
RATIO="${1:-0.35}"; shift || true
NAMES=("$@")
[ ${#NAMES[@]} -eq 0 ] && NAMES=(pistol heavy_pistol smg rifle battle_rifle shotgun marksman sniper)
ENGINE="${ENGINE:-$HOME/codes/game/engine}"
pipe() {
  if [ -n "${ASSET_PIPE:-}" ]; then "$ASSET_PIPE" "$@"
  else cargo run -q --manifest-path "$ENGINE/Cargo.toml" -p asset-pipe -- "$@"; fi
}
OUT=assets/models/weapons/lod
TMP="$(mktemp -d "${TMPDIR:-/tmp}/gunlods.XXXX")"
mkdir -p "$OUT"
for n in "${NAMES[@]}"; do
  if [ "${LOD:-blender}" = asset-pipe ]; then
    pipe "assets/models/weapons/$n.glb" --out "$TMP/$n" --name "$n" --pivot keep --lods "$RATIO" \
      --collider none --no-impostor --force >/dev/null
    cp "$TMP/$n/$n.lod1.glb" "$OUT/$n.lod1.glb"
  else
    "${BLENDER:-/Applications/Blender.app/Contents/MacOS/Blender}" -b --factory-startup \
      --python scripts/blender/gun_lod.py -- "assets/models/weapons/$n.glb" "$OUT/$n.lod1.glb" "$RATIO" 2>&1 | grep -E '^gun lod|Error' || true
  fi
  echo "$n: $(ls -l "$OUT/$n.lod1.glb" | awk '{print $5}') bytes"
done
rm -rf "$TMP"
