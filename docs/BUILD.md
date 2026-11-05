# Building safely

The machine has 24 GB RAM, 15 cores, and only ~40 GB of free disk. Bevy builds are heavy, so:

## memgate
Run **every** cargo command through it:
```sh
memgate -- cargo check -p game
memgate -- cargo test  -p brain
memgate status          # running + queued jobs, free memory
```
- One queue for the whole Mac, shared by all agents and terminals (2 slots by default). Jobs start in arrival order once enough memory is free.
- Kills a job whose process tree passes its cap (9 GB default), or the newest job when the Mac runs low (exit 137). `--retry` re-queues it.
- Release / fat-LTO builds: `memgate --profile release -- cargo build --release`.
- Settings: `memgate config show` (`~/Library/Application Support/memgate/config.toml`).

## Cargo settings
- `.cargo/config.toml`: **one shared target dir** for all git worktrees (build jobs and test threads come from memgate)
  (`/Users/himanshu/codes/game/engine/target`, the engine's: every game shares it). Bevy is compiled once; parallel builds queue on cargo's lock.
- Profiles: deps at `opt-level = 3` without debug info; our crates at `opt-level = 1` with line tables only.
- No `--release` builds unless needed (it doubles disk use).

## Features and ship builds
- `game` depends on the engine with `default-features = false`; its `dev` feature (default)
  turns on `engine/devtools` + `engine/trace` (control port, MCP, offscreen, perf overlay,
  metrics window, crash/perf trace).
- Ship builds: `--no-default-features`. Check without building: `cargo tree -p game
  --no-default-features -e normal | grep ashlar` must print nothing.
- `[profile.release]`: thin LTO, `codegen-units = 1`, stripped, no debug info.
- The engine is a path dependency (`~/codes/game/engine`); its build shares this target dir.

## Testing in batches
Write a coherent chunk, then test it. Don't rebuild after every small edit.
1. `cargo test -p brain` for AI logic (fast, no Bevy).
2. `cargo check -p game`, then `clippy`.
3. `cargo run -p game` smoke test (background it, ~15 s, check for panics).

## Parallel work
Teammates work in separate git worktrees (see [TASKS.md](TASKS.md)), all using the shared target dir and memgate.
Keep it to ≤ 3 concurrent agents, and only one doing Bevy builds at a time if RAM is tight.
