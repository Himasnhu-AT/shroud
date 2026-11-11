# Architecture

## Stack
| Piece | Choice | Why |
|---|---|---|
| Engine | **Bevy =0.20.1** | Ported from 0.19.1 in Nov 2026 with the engine |
| Game shell | **`engine` crate** (`~/codes/game/engine`) | window/headless bootstrap, settings, menus, pause, UI kit, input, audio, FPS controller, render budget, devtools |
| Physics | **avian3d =0.8.0** | ECS-native; v0.8 targets Bevy 0.20 |
| Character controller | the engine's `FpsController` (kinematic, Avian shape casts) | FPS controller crates lag behind Bevy versions |
| NEAT | in-house (~600 lines) | Rust NEAT crates are alpha or stale |
| RNG | rand 0.9 + rand_chacha (seeded) | deterministic sims and tests |
| Save | serde + serde_json | human-readable `brain.json` |
| Test control port | the engine's devtools (`ashlar-control`/`-command`/`-mcp`), cargo feature `dev` only | tests and agents drive the game |
| Crash/perf reporting | the engine's trace (`ashlar-trace`, self-hosted server), feature `dev` only | panics, hangs, frame times, match events |

Upgrade path: once Avian releases support for Bevy 0.20, bump both crates together.

## Workspace
```
shroud/
  Cargo.toml              workspace, profiles tuned for low RAM/disk
  .cargo/config.toml      shared target dir, jobs = 6
  scripts/memgate.sh     RAM/disk guard for every cargo command
  crates/brain/           pure Rust AI, no Bevy dependency
  crates/game/            Bevy app, binary `shroud`
  assets/                 CC0 models (optional; primitives fallback)
  docs/
```

### `brain` (engine-free)
| Module | Role | State |
|---|---|---|
| `math` | `V2` ground-plane vector, seeded RNG, `gauss` | done |
| `arena` (+ `maps/`) | `MapId` and the six layouts (blocks, zones, sites, spawns, cover points), 2D line of sight, collision | done |
| `nav` | nav grid bake, A* (8-neighbour, no corner cutting), string-pull smoothing | done |
| `neat` | NEAT genome, speciation, population, feed-forward network | done |
| `genome` | trait genome + genetic algorithm (technique genes only; aim and reaction are not evolved) | done |
| `limits` | fixed fairness limits: reaction time, aim error, damage per hit. Same at every generation; only difficulty scales them | done |
| `player_model` | telemetry → decayed player profile → insights | done |
| `utility` | tactical goal scoring | done |
| `mind` | `EnemyMind::think(&Observation) -> Action` joins all layers | done |
| `sim` | 2D top-down trainer world + ghost player | done |
| `evolve` | between-match pipeline, `BrainState`, save/load | done |
### `game` (Bevy, on the `engine` crate)
The engine owns the shell: `AppState` (Boot → MainMenu → InGame), pause (Esc, focus loss),
`CoreSettings` (controls/video/audio) + the generated settings screen, the main/pause menus and
UI kit, mouse look + cursor capture, the FPS controller, audio volumes and the sound library,
quality presets, data/assets folders, the render budget and (feature `dev`) the control port,
offscreen capture, perf overlay, metrics window, session log and crash/perf trace.

Pattern of Life keeps only the game: `main` (`engine::app(..)` + plugins), `state` (`GameState` sub-state of
`InGame`: Playing → Evolving → Debrief), `settings` (`ShroudSettings`: difficulty, map, crosshair,
callouts, and their settings rows), `menu` (menu items, match setup screen, Enemy-brain card,
pause score line, crosshair preview), `map`, `player` (controller tuning, health, trigger),
`weapon`, `telemetry` (player habits for the AI), `match_flow`, `evolving`, `trainer_task`,
`enemy/{mod, animation, gun, grenade, debug}`, `ui/{hud, feedback, callouts, debrief}`, `gfx`
(Pattern of Life's look: exposure, fog, sky, FX textures), `audio` (which sounds play when), and `dev`
(feature `dev`: Pattern of Life's control commands, perf spans, metrics keys, session-log events; see
[CONTROL_AND_TELEMETRY.md](CONTROL_AND_TELEMETRY.md)).

## Key design rule: one mind, two worlds
Enemy decisions live only in `brain::mind::EnemyMind`. Both worlds feed it the same
`Observation` and apply the same `Action`:

```
            ┌─────────────── brain ───────────────┐
Observation │ traits ─▶ utility (goal + path)      │ Action
 ─────────▶ │            │                         │ ─────────▶
            │ NEAT net ─▶ micro (strafe/fire/peek) │
            └──────────────────────────────────────┘
   ▲                                                      │
   │ game: ECS sensors (raycasts)      sim: 2D LOS        │
   └──────────── game: Avian movement / sim: 2D step ◀────┘
```
Because the code is shared, what the trainer evolves transfers directly to the real game.
The ground plane maps `V2 { x, y }` ↔ world `(x, height, y)`.

## Data flow per match
```
Playing ──telemetry──▶ MatchReport ──▶ Evolving (background thread)
                                        ├ PlayerModel.ingest (EMA)
                                        ├ trait GA step (real + sim fitness)
                                        ├ NEAT generations vs ghost player
                                        └ save brain.json
                                     ──▶ Debrief (insights, trait deltas) ──▶ next match
```

## Core types (contracts)
```rust
pub struct Observation {
    pub dt: f32,
    pub pos: V2, pub facing: f32, pub hp: f32, pub ammo_frac: f32, pub crouched: bool,
    pub player_visible: bool,
    pub player_pos: V2,          // last known if not visible
    pub player_vel: V2, pub player_hp: f32, pub player_facing: f32,
    pub time_since_seen: f32,
    pub heard_shot: Option<V2>,
    pub allies: Vec<AllyInfo>,   // pos, goal, engaged
    pub taking_fire: bool,
}
pub struct Action {
    pub move_dir: V2,            // world-space, length ≤ 1
    pub sprint: bool, pub crouch: bool,
    pub aim_at: V2, pub aim_height: f32,
    pub fire: bool, pub reload: bool,
}
// NEAT contract: see crates/brain/src/neat/mod.rs header.
```
