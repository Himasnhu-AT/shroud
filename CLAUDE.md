# Pattern of Life (codename Shroud)

FPS in Rust (Bevy 0.20.1 + avian3d 0.8) where enemies evolve from your past matches.
Plan: ~/.claude/plans/make-game-in-rust-peppy-snail.md

## Engine playbook (read first)
Shroud is built on `~/codes/game/engine`. Before rendering, asset, performance or gameplay-module work, read and follow:
- `/Users/himanshu/codes/game/engine/CLAUDE.md`: the engine's must-follow rules and doc index.
- `/Users/himanshu/codes/game/engine/docs/GAME_GUIDE.md`: the playbook. It covers architecture, perf rules (merging, Scenery/Actor, shadows, warm-up, governor), the asset pipeline (texture cook, LODs), measuring correctly, and symptom → fix tables.

Shroud specifics on top of the guide:
- Textures are cooked to KTX2 with the engine's cook: `cargo run --manifest-path /Users/himanshu/codes/game/engine/tools/cook/Cargo.toml -- assets/textures`. Re-run it after adding or changing a texture set; unchanged sets are skipped. No `--release` needed.
- Pooled/moving FX are tagged `Actor`.
- Lamp shadows follow the nearest-K rule and the governor `Game(n)` (`map/look.rs`).
- Plain level blocks merge per material (`map.rs`, `dressing::Batch`).
- Pipelines warm at round start (`gfx.rs`).

## Layout
- `crates/brain` — pure Rust AI (no Bevy): arena/nav, trait genome GA, NEAT, player model, utility AI, 2D trainer sim.
- `crates/game` — Bevy app (binary `shroud`) on the `engine` crate (`~/codes/game/engine`: window, settings, menus, pause, UI kit, input, audio, FPS controller, render budget, devtools). Shroud keeps only game code; `dev` feature (default) = engine devtools + trace, ship builds use `--no-default-features`.
- `assets/` — CC0 models; game must fall back to primitives if any are missing.

## Rules
- Run cargo directly (owner, 11 Nov 2026: memgate is no longer used here). Builds still serialise on cargo's lock in the shared target.
- Every game and the engine share one target dir, owned by the engine: `/Users/himanshu/codes/game/engine/target` (set in each repo's `.cargo/config.toml`). Bevy compiles there once for all of them. Prune it with `~/codes/game/engine/scripts/sweep-target.sh` (cargo-sweep: drops artifacts unused for 7+ days). Builds queue on cargo's lock; that's intended. Don't set a different CARGO_TARGET_DIR. Disk is tight (~40 GB free): don't `cargo build --release` unless asked.
- Test in batches: write a coherent chunk, then `cargo test -p brain` (or `check -p game`), not after every edit.
- Pin versions: bevy `=0.20.1`, avian3d `=0.8.0`, rand 0.9. Look up Bevy/Avian APIs in `~/.cargo/registry/src/*/bevy_*-0.20.*` and `avian3d-0.8.0` rather than guessing; 0.20 differs from older tutorials.
- Shared-target caveat: cargo gives the same workspace crate in different worktrees the same artifact identity, so a sibling worktree's build can make yours look up to date with *its* code (symptom: "unresolved import" for items that exist). Fix: `touch` the affected crate's sources and rebuild. Don't run divergent worktree builds of the same crate back to back without that.
- Visual/behaviour checks: `python3 -I scripts/play.py scripts/look-at-enemies.json --out <dir>` launches the game (muted, offscreen by default: hidden window, camera renders to an image, so it works even if another app is fullscreen; mouse never grabbed) via the control port, runs the steps, saves screenshots. Look at the PNGs before claiming a visual change works. `--headless` for non-visual checks.
- Test runs never play sound: control-port/MCP/offscreen runs are muted unless `--sound`; play.py also passes `--mute`.
