# Architecture

## Stack
| Piece | Choice | Why |
|---|---|---|
| Engine | **Bevy =0.19.1** | Bevy 0.20 (Oct 2026) is out, but Avian doesn't support it yet |
| Physics | **avian3d =0.7.0** | ECS-native; v0.7 targets Bevy 0.19 |
| Character controller | in-house kinematic (Avian shape casts) | FPS controller crates lag behind Bevy versions |
| NEAT | in-house (~600 lines) | Rust NEAT crates are alpha or stale |
| RNG | rand 0.9 + rand_chacha (seeded) | deterministic sims and tests |
| Save | serde + serde_json | human-readable `brain.json` |
| Test control port | `ashlar-control` + `ashlar-command` + `ashlar-mcp` (cargo feature `control`, dev/test only) | tests and agents drive the game |
| Crash/perf reporting | `ashlar-trace` (self-hosted server) | panics, hangs, frame times, match events |

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
| `arena` | "Foundry" layout (blocks, zones, spawns, cover points), 2D line of sight, collision | done |
| `nav` | nav grid bake, A* (8-neighbour, no corner cutting), string-pull smoothing | done |
| `neat/` | NEAT genome, speciation, population, feed-forward network | API stub |
| `genome` | trait genome + genetic algorithm | todo |
| `limits` | fairness clamps by generation | todo |
| `player_model` | telemetry → decayed player profile → insights | todo |
| `utility` | tactical goal scoring | todo |
| `mind` | `EnemyMind::think(&Observation) -> Action` joins all layers | todo |
| `sim` | 2D top-down trainer world + ghost player | todo |
| `evolve` | between-match pipeline, `BrainState`, save/load | todo |

### `game` (Bevy)
`main`, `state` (Menu → Playing → Evolving → Debrief), `map`, `player`, `weapon`,
`telemetry` (player habits for the AI), `match_flow`, `enemy/{spawn, perception, brain_bridge, animation}`, `ui/{hud, debrief, debug}`,
`ops/{control, commands, trace}` (control port + crash/perf reporting, see [CONTROL_AND_TELEMETRY.md](CONTROL_AND_TELEMETRY.md)).

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
