#!/usr/bin/env bash
# Export the MetaHuman lookdev textures this game uses (head region chart, pore normal, iris,
# sclera, ...) from a local UE 5.8 install into assets/models/metahuman/textures (gitignored:
# Epic content). Runs the editor headless once (a few minutes); close other UE sessions first.
set -euo pipefail
cd "$(dirname "$0")/.."
UE="${UE_ROOT:-/Users/Shared/Epic Games/UE_5.8}"
export MH_TEX_OUT="$PWD/assets/models/metahuman/textures"
mkdir -p "$MH_TEX_OUT"
memgate -- "$UE/Engine/Binaries/Mac/UnrealEditor.app/Contents/MacOS/UnrealEditor" "$PWD/scripts/metahuman/TexExport.uproject" \
  -run=pythonscript -script="$PWD/scripts/metahuman/export_textures.py" -unattended -nullrhi -nosplash -nosound -stdout 2>&1 | grep -E "TEXEXPORT|Error" || true
ls "$MH_TEX_OUT"
