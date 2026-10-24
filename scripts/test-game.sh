#!/usr/bin/env bash
# Game tests: the control-port scenarios (default features = `dev`), then a ship build check:
# without `dev` the game compiles and no `ashlar-*` crate (control port, MCP, trace) is linked.
set -euo pipefail
cd "$(dirname "$0")/.."

memgate -- cargo test -p game
memgate -- cargo check -p game --no-default-features
if cargo tree -p game --no-default-features -e normal | grep ashlar; then
  echo "test-game: ship build still depends on ashlar crates" >&2
  exit 1
fi
echo "test-game: ok"
