# Shroud

A first-person shooter written in Rust where the enemies **evolve from your past matches**.
Every match, the squad you face is bred from the squads that did best against *you*,
and it has studied your habits: where you camp, the ranges you fight at, and which way you strafe.

> Status: design phase. Docs are complete; code is partly scaffolded (see [docs/TASKS.md](docs/TASKS.md)).

## How it gets harder
1. **Traits evolve** (genetic algorithm): aggression, flanking, cover use, reaction time, and more.
2. **Tactics adapt** (utility AI): enemies choose to attack, flank, take cover, retreat, or ambush where they predict you'll be.
3. **Combat moves evolve** (NEAT neural nets): strafing, peeking, and when to shoot.
4. **They learn you** (player model): your heatmap, ranges, and reactions feed both the tactics and the training.
5. **Between matches** a fast offline sim trains the squad against a "ghost" of you.
6. **Fair by design**: hard limits on reaction time and aim error loosen slowly, so enemies get smarter, not aimbots.

## Docs
| Doc | What's in it |
|---|---|
| [ARCHITECTURE](docs/ARCHITECTURE.md) | Crates, data flow, tech stack and versions |
| [AI](docs/AI.md) | The three enemy brain layers and fairness limits |
| [PLAYER_MODEL](docs/PLAYER_MODEL.md) | What gets recorded about you and how it's used |
| [EVOLUTION](docs/EVOLUTION.md) | The between-match pipeline, fitness, trainer, save format |
| [GAMEPLAY](docs/GAMEPLAY.md) | Match format, arena, weapons, controls, HUD, debrief |
| [ASSETS](docs/ASSETS.md) | CC0 asset sources, paths, fallbacks |
| [BUILD](docs/BUILD.md) | Building safely: memgate, shared target dir, testing |
| [TASKS](docs/TASKS.md) | Work breakdown, parallel streams, milestones, verification |
| [CONTROL_AND_TELEMETRY](docs/CONTROL_AND_TELEMETRY.md) | Test control port (dev builds only) + crash/perf reporting (ashlar crates on stock Bevy) |
| [ENGINE](docs/ENGINE.md) | Engine research record; decision: stock Bevy ([ADR 0001](docs/adr/0001-engine-strategy.md)) |

## Quick start (once code lands)
```sh
memgate -- cargo test -p brain     # AI tests
memgate -- cargo run -p game       # play
```
