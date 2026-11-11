#!/usr/bin/env bash
# Build the MetaHuman enemy (assets/models/metahuman/enemy_mh.glb) from MetaHuman DNA files with
# bevy_metahuman's dna2gltf, baking both Quaternius UAL animation libraries onto it.
#
# The output is Epic content (MetaHuman license): it is gitignored and never shipped in this repo.
# Without it the game uses the CC0 mannequin (models/enemy.glb).
#
#   scripts/make-metahuman.sh [DNA_DIR]
# DNA_DIR holds SKM_Face.dna + SKM_Body.dna (or head.dna + body.dna from a MetaHuman Creator DCC
# export). Default: UE 5.8's archetype DNAs.
set -euo pipefail
cd "$(dirname "$0")/.."
DNA_DIR="${1:-/Users/Shared/Epic Games/UE_5.8/Engine/Plugins/MetaHuman/MetaHumanCoreTechLib/Content/ArchetypeDNA}"
PKG="${BEVY_METAHUMAN:-$(cd .. && pwd)/bevy_metahuman}"
pick() { for f in "$@"; do [ -f "$DNA_DIR/$f" ] && { echo "$DNA_DIR/$f"; return; }; done; echo "missing $* in $DNA_DIR" >&2; exit 1; }
HEAD=$(pick head.dna SKM_Face.dna)
BODY=$(pick body.dna SKM_Body.dna)
mkdir -p assets/models/metahuman
# Only the clips the enemy plays (crates/game/src/enemy/animation.rs) plus a few kept for upcoming
# use: every baked clip costs memory on each of the 818 joints.
CLIPS="Pistol_Idle_Loop,Idle_Loop,Walk_Loop,Jog_Fwd_Loop,Sprint_Loop,Crouch_Idle_Loop,Crouch_Fwd_Loop,Pistol_Aim_Neutral,Pistol_Aim_Up,Pistol_Aim_Down,Pistol_Shoot,Pistol_Reload,Hit_Chest,Hit_Head,Death01,Hit_Knockback,OverhandThrow"
# Textures from scripts/export-metahuman-textures.sh, when exported.
T=assets/models/metahuman/textures
TEX=()
[ -f "$T/head_color.png" ] && TEX+=(--skin-guide "$T/head_color.png")
[ -f "$T/skin_micro_normal.png" ] && TEX+=(--skin-micro-normal "$T/skin_micro_normal.png")
[ -f "$T/iris.png" ] && [ -f "$T/sclera.png" ] && TEX+=(--iris "$T/iris.png" --sclera "$T/sclera.png")
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/codes/game/engine/target}" cargo run -q --manifest-path "$PKG/Cargo.toml" -p dna2gltf -- \
  --clips "$CLIPS" --body "$BODY" --head "$HEAD" --body-lod 1 --head-lod 1 \
  --anim assets/models/enemy.glb --anim assets/models/ual2/UAL2_Standard.glb \
  ${TEX[@]+"${TEX[@]}"} --hands-out assets/models/metahuman/hands_mh.glb --name MetaHumanEnemy -o assets/models/metahuman/enemy_mh.glb
