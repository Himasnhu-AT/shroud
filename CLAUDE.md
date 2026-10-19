# Shroud

FPS in Rust (Bevy 0.19.1 + avian3d 0.7) where enemies evolve from your past matches.
Plan: ~/.claude/plans/make-game-in-rust-peppy-snail.md

## Layout
- `crates/brain` — pure Rust AI (no Bevy): arena/nav, trait genome GA, NEAT, player model, utility AI, 2D trainer sim.
- `crates/game` — Bevy app (binary `shroud`).
- `assets/` — CC0 models; game must fall back to primitives if any are missing.

## Rules
- **Run every cargo command through memgate**: `memgate -- cargo <args>` (installed in `~/.cargo/bin`). It queues jobs machine-wide (2 slots across all agents/terminals), waits for free memory, and kills runaway jobs (exit 137). Waiting in the queue is expected; never bypass it. `memgate status` shows the queue.
- All worktrees share `/Users/himanshu/codes/game/shroud/target` (set in `.cargo/config.toml`). Builds queue on cargo's lock; that's intended. Don't set a different CARGO_TARGET_DIR. Disk is tight (~40 GB free): don't `cargo build --release` unless asked.
- Test in batches: write a coherent chunk, then `memgate -- cargo test -p brain` (or `check -p game`), not after every edit.
- Pin versions: bevy `=0.19.1`, avian3d `=0.7.0`, rand 0.9. Look up Bevy/Avian APIs in `~/.cargo/registry/src/*/bevy_*-0.19.*` and `avian3d-0.7.0` rather than guessing; 0.19 differs from older tutorials.
- Shared-target caveat: cargo gives the same workspace crate in different worktrees the same artifact identity, so a sibling worktree's build can make yours look up to date with *its* code (symptom: "unresolved import" for items that exist). Fix: `touch` the affected crate's sources and rebuild. Don't run divergent worktree builds of the same crate back to back without that.
