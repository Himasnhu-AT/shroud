# Control port & telemetry

Both now come from the **`engine` crate** (`~/codes/game/engine`, `engine::devtools` and
`engine::trace`, see its `docs/PORTING.md`), which reuses ashlar's crates:

| Piece | ashlar crates | Why |
|---|---|---|
| **Control port** — **for testing only**: tests and agents drive the game | `ashlar-control`, `ashlar-command`, `ashlar-mcp` | automated playtests, AI regression tests, perf soaks; Claude tests the game over MCP |
| **Crash & performance reporting** | `ashlar-trace` (+ `ashlar-trace-server` self-hosted) | panics, hangs, frame-time histograms, match events |

Pattern of Life's part lives in `crates/game/src/dev/`: its own commands (`commands.rs`, `ai_commands.rs`),
perf spans, metrics keys and session-log events (`mod.rs`). Note: `game/src/telemetry.rs` is
the *player-habit* recorder for the AI — a different thing.

---

## 1. Control port (testing only)

**Purpose:** let automated tests and agents (including Claude via MCP) drive and inspect the game. It is **not** a player feature.

- Behind Pattern of Life's cargo feature `dev` (= `engine/devtools` + `engine/trace`), on by default for
  `cargo run` / `cargo test`. Ship builds use `--no-default-features`: no port, no MCP, no trace,
  the engine ignores its debug flags, and `cargo tree -p game --no-default-features -e normal | grep ashlar`
  prints nothing (`scripts/test-game.sh` and `scripts/package-macos.sh` check it).

### Transport (inherited from ashlar, see its `docs/control-protocol.md`)
- JSON lines over TCP `127.0.0.1:<port>`, one reply per request, in order.
- Request `{"id":1,"key":"<64 hex>","method":"engine.execute","params":{"command":"player.move","params":{...}}}`
  → reply `{"id":1,"ok":true,"result":...}` or `{"ok":false,"error":"...","detail":...}`.
- Loopback only, 0600 key file `control-key.local-<port>` in the game's data dir, constant-time key check,
  bounded lines, connection cap. Standard methods for free: `auth`, `app.ping`, `app.version`, `engine.commands`, `engine.execute`, `engine.batch`.
- Same vocabulary over **MCP** (`shroud --mcp`, stdio) via `ashlar-mcp`, so Claude Code can drive the game as tools.

### Enabling
| Flag | Effect |
|---|---|
| `--control <port>` | start the TCP control server (0 = any free port; prints `control: port=<n> key=<file>`) |
| `--mcp` | serve MCP on stdio so Claude can run test sessions |
| `--headless` | no window/renderer: physics and AI still run (CI, fast AI tests) |
| `--offscreen` | hidden window, every camera renders into one 1920x1080 image (screenshots work behind fullscreen apps) |
| `--play` | skip the main menu |
| `--seed <n>` | deterministic RNG for repeatable tests |
| `--no-trainer` | no background trainer in the menu/debrief |

All are engine dev flags (`engine::cli::dev_flag`): ignored in ship builds.

### Commands
Engine (generic): `engine.state`, `engine.start`, `engine.menu`, `engine.pause`, `engine.resume`,
`engine.quit`, `time.scale {factor}`, `time.step {frames}` (steps through the pause),
`screenshot {path}`, `perf.stats`, `perf.overlay`, `perf.report`, `metrics.*`, `ui.screen
{screen, tab}` (`main`, `setup`, `settings`, `pause`), `ui.nav {input}`, `ui.setting {key, value}`,
`settings.get`, `input.key`, `input.mouse`, `render.stats`, `render.budget`.

Old names kept as aliases: `game.quit` → `engine.quit`, `game.pause` → `engine.pause`,
`game.resume` → `engine.resume`, `capture.screenshot` → `screenshot`.

Pattern of Life:
| Command | Params | Result |
|---|---|---|
| `game.state` | — | `{state (Menu/Playing/Evolving/Debrief), paused, time_scale, frame, seed, player:{pos, hp, ammo, yaw, pitch, crouched, grounded}}` |
| `game.start` | — | enter the game (or restart the round when already in it), unpaused |
| `game.menu` | — | abandon the match, main menu |
| `player.teleport` | `{x, z, y?, yaw?}` | `{}` |
| `player.look` | `{yaw, pitch}` or `{at:[x,y,z]}` | `{yaw, pitch}` |
| `player.move` | `{forward, right, ms, sprint?, crouch?}` | final `{pos}` (multi-frame) |
| `player.fire` | `{shots}` | `{shots, hits, damage}` (multi-frame) |
| `player.reload` / `player.crouch` | `{on?}` | `{}` (reload presses R even on a full mag: the bodycam press check) |
| `player.god` | `{on}` | `{}` (soak tests) |
| `enemy.list` / `enemy.freeze` / `enemy.kill` / `enemy.lean` | | |
| `econ.state` | | money, loss streak, freeze/buy window, carried kit, next-round loadout, squad wallets + buy mode |
| `econ.set_money` | `{money?, squad?}` | `econ.state` |
| `buy` | `{item, anywhere?}` (gun name, `kevlar`, `helmet`) | `{bought, money}`; error if outside the buy window, owned or too poor |
| `buy.menu` | `{open?}` | `{open}` (opens only when buying is possible) |
| `video.camera_style` | `{style?}` (`classic` / `bodycam`) | `{style}` |
| `video.bodycam` | `{turn?, strafe?, fire?, ms?}` (non-blocking drive) | free-aim state: weapon/camera/delta angles, roll, muzzle + barrel (camera space), press check |
| `round.freeze` | `{seconds, now?}` | freeze time for later rounds (and now); 0 by default in test runs |
| `career.state` | — | rank, XP, next rank, this match's objectives (`text`, `xp`, `progress`, `target`, `done`), the match log per round, last `award` |
| `career.set_xp` / `career.reroll` | `{xp}` / `{seed}` | set (and save) the XP / roll this match's objectives again |
| `match.state`, `ai.brain`, `ai.evolve`, `ai.reset`, `ai.squad`, `ai.throw`, `ai.difficulty` | | |
| `map.load {name}` / `map.info` | | |
| `sim.run` | `{generations, seed}` | headless trainer fitness curve |
| `debug.overlay` | `{on}` | enemy debug view (F3) |

Settings keys for `ui.setting`: Pattern of Life's `difficulty`, `map`, `show_callouts`, `crosshair_color`,
`crosshair_size`, and the engine's `sensitivity`, `invert_y`, `quality`, `fov_deg` (was `fov`),
`window_mode`, `vsync`, `show_fps`, `ui_scale`, `master_volume`, `sfx_volume`, `ambience_volume`.

### Test scenarios (`crates/game/tests/`, run headless with `--seed`)
| Test | Steps | Asserts |
|---|---|---|
| `smoke` | `app.ping`, `game.start`, step 300 frames | state is `Playing`, no panic, 5 enemies alive |
| `movement` | teleport behind low cover, `player.crouch`, `enemy.list` | crouched player not visible to enemies; standing is |
| `shooting` | look at frozen enemy, `player.fire {shots:5}` | hits > 0, enemy hp dropped by hits × 25 |
| `enemies_fight_back` | god mode, unfreeze, step 20 s | player took damage; enemy goals include Attack/Flank/TakeCover |
| `match_flow` | `enemy.kill` ×3 rounds (first to 3) | state goes Playing → Evolving → Debrief; generation +1 |
| `evolution_gets_harder` | play N scripted matches with a fixed scripted player, `ai.evolve` each | squad damage dealt / time-to-kill improves across generations (seed-averaged) |
| `persistence` | evolve, quit, relaunch | `ai.brain.generation` survives restart |
| `perf_soak` (manual/nightly) | god mode, `time.scale 4`, 30 min | no hangs; p99 frame time under budget (`perf.stats` + trace dashboard) |

**Claude as tester:** `shroud --mcp` exposes the same commands as MCP tools, so Claude can run exploratory test sessions: play a round, take screenshots, check enemy goals, and report bugs.

---

## 2. Crash & performance reporting (`ashlar-trace`, via the engine)

The engine's `TracePlugin` (feature `dev`) builds the tracer from `SHROUD_TRACE_ENDPOINT` /
`SHROUD_TRACE_KEY` (a no-op tracer without both), arms hang detection on the first in-game
frame, reports frame times and `perf.hitch`, installs the panic hook and handles `--crash-test`.
Pattern of Life declares its events in `dev::DevPlugin` (`match.start`, `match.end`, `round.end`,
`player.died`, `ai.evolved`, `ai.reset`, `bomb.*`, `career.objectives`, `career.objective`,
`career.credited`, `career.promoted`) and records them with `dev::Dev::trace` /
`dev::trace` (no-ops in ship builds).

Perf monitor: `ai` (`EnemySet`) and `weapons` (`WeaponSet`) are perf spans; physics is timed by
the engine. The overlay is on F2 (`PerfOverlayKey`; F3 is the enemy debug view). Metrics window
(F4, dev runs): Pattern of Life records `ai.think_ms`, `ai.squad_ms`, `enemies.alive`, `enemies.engaged`,
`match.round` and labels each run with `map` and `difficulty`.

### Privacy
- Nothing is sent until the first-run notice in the Menu is shown (`tracer.notice_shown()`).
- Settings toggles per category; `DO_NOT_TRACK=1` honoured. No player names; player-model data is **not** uploaded.

### Dev setup
Run `ashlar-trace-server` locally; point `TRACE_ENDPOINT` at it via env (see `/Users/himanshu/.claude/bin/env-keys` for checking `.env`). The dashboard shows crash groups, frame-time percentiles per version, hitches, match/evolve events.

---
