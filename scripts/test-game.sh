#!/usr/bin/env bash
# Game tests: the control-port scenarios (default features), then a build without the
# `control` feature to prove release builds carry no control code.
set -euo pipefail
cd "$(dirname "$0")/.."

memgate -- cargo test -p game
memgate -- cargo check -p game --no-default-features
echo "test-game: ok"
