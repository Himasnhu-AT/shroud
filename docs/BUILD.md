# Building safely

The machine has 24 GB RAM, 15 cores, and only ~40 GB of free disk. Bevy builds are heavy, so:

## memguard
Run **every** cargo command through it:
```sh
scripts/memguard.sh cargo check -p game
scripts/memguard.sh cargo test  -p brain
```
- Won't start if free RAM < 20% (waits up to 180 s) or free disk < 6 GB.
- Kills the whole process group if free RAM drops below 8% or disk below 2 GB (exit 137).
- Tunable via `MEMGUARD_START_PCT`, `MEMGUARD_KILL_PCT`, `MEMGUARD_MIN_DISK_GB`, `MEMGUARD_WAIT_S`.

## Cargo settings
- `.cargo/config.toml`: `jobs = 6`, and **one shared target dir** for all git worktrees
  (`/Users/himanshu/codes/game/shroud/target`). Bevy is compiled once; parallel builds queue on cargo's lock.
- Profiles: deps at `opt-level = 3` without debug info; our crates at `opt-level = 1` with line tables only.
- No `--release` builds unless needed (it doubles disk use).

## Testing in batches
Write a coherent chunk, then test it. Don't rebuild after every small edit.
1. `cargo test -p brain` for AI logic (fast, no Bevy).
2. `cargo check -p game`, then `clippy`.
3. `cargo run -p game` smoke test (background it, ~15 s, check for panics).

## Parallel work
Teammates work in separate git worktrees (see [TASKS.md](TASKS.md)), all using the shared target dir and memguard.
Keep it to ≤ 3 concurrent agents, and only one doing Bevy builds at a time if RAM is tight.
