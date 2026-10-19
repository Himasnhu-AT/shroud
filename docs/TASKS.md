# Tasks & milestones

## Done (2026-10-19)
- M1–M4: brain (GA, NEAT, player model, utility AI, mind, 2D trainer, evolve, save/bootstrap), game core, evolved enemies,
  match flow (best of 3), Evolving/Debrief screens, F3 overlay, test control port + MCP, ashlar-trace reporting
- M5 polish pass: sky/shadows/SSAO/bloom/TAA, ambientCG textures, Kenney rifle viewmodel (sway, bob, recoil, reload, flash,
  impacts, decals); enemies with rifles in hand, muzzle tracers, hit reactions, flinch, fire telegraph, death + dropped gun;
  23 sounds (spatial enemy fire, hit/kill/headshot, footsteps, reload, heartbeat, stingers, ambience), damage vignette,
  direction indicators, camera shake
- Tooling: `scripts/play.py` (offscreen screenshots, private binary copy), `scripts/test-game.sh`, `sim_bench`

## Next
- Hit-reaction / glint visibility tuning; enemy flesh-hit sound; volume setting in a menu; longer ambient loop
- Balance pass: ghost-vs-squad win rates per generation; human playtests
- Props (crates) for set dressing; second weapon

## Earlier log
- Workspace, cargo profiles, `.cargo/config.toml`, memgate, `CLAUDE.md`
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
- [ ] Builds never trip memgate on a normal run

## Agent rules
- Read `CLAUDE.md`; use memgate; test in batches; commit on the worktree branch.
- Keep reports short (< 200 words): branch, files, public API added, what's unfinished.
