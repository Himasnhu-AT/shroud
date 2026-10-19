# Tasks & milestones

## Done
- Workspace, cargo profiles, `.cargo/config.toml`, `scripts/memguard.sh`, `CLAUDE.md`
- `brain::math`, `brain::arena` (Foundry layout, zones, cover points, LOS, collision), `brain::nav` (grid + A* + smoothing)
- `brain::neat` API stub (contract in the file header)
- Base commit `f18e9ad`

## Parallel streams (separate worktrees)
| Stream | Scope | Depends on | Model |
|---|---|---|---|
| **A. NEAT** | `brain/src/neat/` full implementation + `tests/neat.rs` (XOR, serde, acyclic) | NEAT contract only | Sonnet |
| **B. Game core** | `game`: states, map from `Arena`, FPS controller, rifle, Health/Damage/ShotFired messages, HUD, dummies, asset fallback | `brain::arena` only | Opus |
| **C. Assets** | download CC0 packs, trim to `.glb`, `assets/CREDITS.md` | nothing | Haiku/Sonnet |
| **D. Brain core** (lead) | `genome`, `limits`, `player_model`, `utility`, `mind` (Observation/Action) | NEAT contract | lead |

After A + D merge:
| Stream | Scope |
|---|---|
| **E. Trainer** | `sim` + ghost player + `evolve` + `BrainState` save/load + `--sim-bench`; test "gen-10 beats gen-0" |
| **F. Enemy integration** | `game/enemy/*`: spawn, perception (raycast LOS, vision cone, hearing), brain bridge, animation |
| **H. Ops** | test control port (dev-only feature) + MCP + headless mode, ashlar-trace plugin (see CONTROL_AND_TELEMETRY.md) |
| **G. Loop & UI** | telemetry, match flow (best of 3), Evolving screen (background thread + progress), Debrief, F3 overlay |

## Milestones
1. **M1 Brain core:** A + D merged, `cargo test -p brain` green.
2. **M2 Shooting gallery:** B. You can walk, crouch behind cover, and shoot dummies.
3. **M3 Enemies fight back:** F. 5 enemies using `EnemyMind` with gen-0 genomes.
4. **M4 Evolution:** E + G. Full loop: match → evolve → debrief → harder match; `brain.json` persists.
5. **M5 Polish:** assets, tuning so gen 0 is beatable and gen 10 is noticeably smarter.

## Verification checklist
- [ ] NEAT solves XOR (< 20 s, dev profile)
- [ ] GA converges on a toy fitness function
- [ ] Player model EMA and confidence behave as specified
- [ ] `brain.json` round-trips; corrupt file → backup + fresh start
- [ ] Sim: gen-10 squads beat gen-0 squads against a fixed ghost (seed-averaged)
- [ ] Game: 3 matches played; generation increments; debrief shows insights + trait deltas
- [ ] Restart the game: generation and model persist
- [ ] F3 overlay shows goals/paths
- [ ] Control port: headless game driven end-to-end by `tests/control.rs`
- [ ] Trace: local ashlar-trace-server shows session, frame histogram, test crash
- [ ] Builds never trip memguard on a normal run

## Agent rules
- Read `CLAUDE.md`; use memguard; test in batches; commit on the worktree branch.
- Keep reports short (< 200 words): branch, files, public API added, what's unfinished.
