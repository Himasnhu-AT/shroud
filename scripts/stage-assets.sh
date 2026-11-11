#!/usr/bin/env bash
# Copy assets/ into a packaged build, minus what players don't need, with 4K textures cut to 2K.
#
#   scripts/stage-assets.sh DEST                   # DEST becomes the shipped assets folder
#   FULL_TEXTURES=1 scripts/stage-assets.sh DEST   # keep the 4K KTX2s (full-quality build)
#   MAX_TEXTURE=1024 scripts/stage-assets.sh DEST  # another cap
#
# Used by package-macos.sh and package-windows.sh. Only the staged copy is trimmed: the 4K
# originals in assets/ stay as they are. Trimming drops the top mip level of any KTX2 bigger
# than MAX_TEXTURE (default 2048) with scripts/ktx2_trim.py (no re-encode). The engine's quality
# cap still applies on top (Low 1K, Medium 2K, High/Ultra keep what ships).
set -euo pipefail
cd "$(dirname "$0")/.."
DEST="${1:?usage: scripts/stage-assets.sh DEST}"
MAX_TEXTURE="${MAX_TEXTURE:-2048}"

# models/metahuman is Epic content generated locally (MetaHuman license): never shipped; the
# game falls back to the CC0 mannequin enemy and hands.
# Source JPGs are dead weight once a set is cooked: the engine loads color/normal/orm.ktx2
# when color.ktx2 exists. Sets that aren't cooked yet keep their JPGs.
COOKED_JPGS="$(mktemp)"
trap 'rm -f "$COOKED_JPGS"' EXIT
for set in assets/textures/*/; do
  [ -f "${set}color.ktx2" ] && find "$set" -maxdepth 1 -name '*.jpg' | sed 's|^assets/||' >>"$COOKED_JPGS"
done
mkdir -p "$DEST"
rsync -a --delete --exclude '.DS_Store' --exclude '*.md' --exclude '*metahuman*' --exclude-from="$COOKED_JPGS" assets/ "$DEST/"

if [ "${FULL_TEXTURES:-0}" = 1 ]; then
  echo "stage-assets: FULL_TEXTURES=1, KTX2s kept at full size"
else
  python3 -I scripts/ktx2_trim.py --max "$MAX_TEXTURE" -q "$DEST"
fi
echo "stage-assets: $DEST $(du -sh "$DEST" | cut -f1)"
