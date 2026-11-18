# Tells

_Codename Shroud: the crates, the `shroud` binary and the save folder keep that name._

A first-person shooter written in Rust where the enemies **evolve from your past matches**.
Every match, the squad you face is bred from the squads that did best against *you*,
and it has studied your habits: where you camp, the ranges you fight at, and which way you strafe.

**They get smarter, never faster.** Aim, reaction time and damage are fixed by the difficulty you
pick; evolution only improves how the squad plays.

> Status: in development and playable. Five-round bomb defusal (first to 3) with a round economy
> and buy menu, grenades, six maps, a career with ranks and match objectives, and a bodycam
> camera (the default). Work tracking: [docs/TASKS.md](docs/TASKS.md), [TODO.md](TODO.md).

## How it gets harder
1. **Traits evolve** (genetic algorithm): aggression, preferred range, flanking, cover use,
   teamwork, caution, weapon choice and more. Never aim or reaction speed.
2. **Tactics adapt** (utility AI): enemies choose to attack, flank, take cover, retreat, or ambush where they predict you'll be.
3. **Combat moves evolve** (NEAT neural nets): strafing, peeking, and when to shoot.
4. **They learn you** (player model): your heatmap, ranges, and reactions feed both the tactics and the training.
5. **Between matches** a fast offline sim trains the squad against a "ghost" of you.
6. **Fair by design**: reaction time, aim error and damage per hit are fixed limits set by the
   difficulty. They never change with generation, so a generation-20 squad aims exactly like a
   generation-1 squad. It just plays you better.

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

**Play it:** [himanshuat.itch.io/tells](https://himanshuat.itch.io/tells)

## Quick start
Needs a Rust toolchain. The engine (Hull) is a git dependency
([himasnhu-AT/hull-dep](https://github.com/himasnhu-AT/hull-dep)), so Cargo fetches it on the first build.
```sh
git clone https://github.com/himasnhu-AT/shroud && cd shroud
cargo test -p brain     # AI tests
cargo run -p game       # play
```
