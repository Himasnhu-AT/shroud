# Control port & telemetry (stock Bevy + two ashlar crates)

We use **Bevy as-is** and add only two things, both reused from `~/codes/synthai/ashlar`:

| Addition | ashlar crates | Why |
|---|---|---|
| **Control port** — agents and tests drive the game | `ashlar-control` (TCP JSON-lines server, key file, hardening), `ashlar-command` (`Dispatch` trait, command list), `ashlar-mcp` (MCP over stdio for free) | automated playtests, AI regression tests, Claude can play/inspect the game |
| **Crash & performance reporting** | `ashlar-trace` (+ `ashlar-trace-server` self-hosted) | panics, hangs, frame-time histograms, custom gauges, match events |

Neither crate depends on egui or ashlar's app layer (control: `serde`, `serde_json`, `thiserror`, `log`, `ashlar-command`; trace: `ashlar-trace-proto`, `flate2`, `ureq`, `sysinfo`), so they sit next to Bevy cleanly.
We do **not** use `ashlar-telemetry` (it wires ashlar's command chokepoint and native dialogs, which a Bevy game doesn't have).

Code lives in `crates/game/src/ops/` (`control.rs`, `commands.rs`, `trace.rs`). Note: `game/src/telemetry.rs` is the *player-habit* recorder for the AI — a different thing.

Dependencies (pin a git rev while ashlar is pre-0.1):
```toml
ashlar-control = { git = "<ashlar repo>", rev = "<sha>" }   # or path = "../../../synthai/ashlar/crates/control" locally
ashlar-command = { git = "<ashlar repo>", rev = "<sha>" }
ashlar-mcp     = { git = "<ashlar repo>", rev = "<sha>" }
ashlar-trace   = { git = "<ashlar repo>", rev = "<sha>", features = ["sysinfo"] }
```

---

## 1. Control port

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
| `--control <port>` | start the TCP control server (off by default in release builds) |
| `--mcp` | serve MCP on stdio |
| `--headless` | no window/renderer: `MinimalPlugins` + `ScheduleRunnerPlugin`, physics and AI still run (CI, fast AI tests) |
| `--seed <n>` | deterministic RNG for repeatable tests |

### Bevy integration
```
ashlar_control::Server::start(config, wake) ─▶ Receiver<Request>   (server thread)
                                                     │
PreUpdate, exclusive system `drain_control(world)`: try_recv() all ─▶ GameDispatch::execute(&mut World, id, params)
   ├─ instant commands  → request.ok(result) immediately
   └─ multi-frame ones  → stored in `PendingReplies`; answered by a system when done (e.g. after N frames / duration)
```
- `wake` is a no-op in normal play (Bevy redraws continuously); in a paused/low-power state it sends a `RequestRedraw`.
- `GameDispatch` implements `ashlar_command::Dispatch`, so `engine.commands` lists every game command with its params schema.
- Commands go through the **same input path** as a human where possible (they write a `VirtualInput` resource the player controller reads), so tests exercise real code.

### Game commands
| Command | Params | Result |
|---|---|---|
| `game.state` | — | `{state, match, round, score, generation, player:{pos, hp, ammo}, enemies:[{id, pos, hp, goal}]}` |
| `game.start` / `game.quit` | — | `{}` |
| `game.pause` / `game.resume` | — | `{}` |
| `time.scale` | `{factor}` (0–8) | `{}` |
| `time.step` | `{frames}` (while paused) | `{frame}` after stepping — deterministic tests |
| `player.teleport` | `{x, z, yaw?}` | `{}` |
| `player.look` | `{yaw, pitch}` or `{at:[x,y,z]}` | `{}` |
| `player.move` | `{forward, right, ms, sprint?, crouch?}` | final `{pos}` (multi-frame) |
| `player.fire` | `{shots}` | `{hits, damage}` (multi-frame) |
| `player.reload` / `player.crouch` | `{on?}` | `{}` |
| `player.god` | `{on}` | `{}` (soak tests) |
| `enemy.list` | — | `[{id, pos, hp, goal, traits, neat_id}]` |
| `enemy.freeze` | `{on, id?}` | `{}` |
| `enemy.kill` | `{id?}` (all if omitted) | `{killed}` |
| `ai.brain` | — | `{generation, squad traits, player_model summary, insights}` |
| `ai.evolve` | `{matches?: 1}` | run the evolve pipeline now with the current report → `{generation, deltas}` |
| `ai.reset` | — | backup + fresh `brain.json` |
| `sim.run` | `{generations, seed}` | headless trainer fitness curve (no rendering needed) |
| `capture.screenshot` | `{path}` | `{path}` (Bevy `Screenshot` → file; windowed only) |
| `debug.overlay` | `{on}` | `{}` |
| `perf.stats` | — | `{fps, frame_ms:{p50,p95,p99}, entities, enemies, draw_calls?}` |

### What it enables
- **Agent playtests:** an agent (or Claude via MCP) runs `game.start`, steers with `player.*`, reads `game.state`, takes screenshots.
- **AI regression tests:** `--headless --seed 1 --control 0` + script: play scripted matches, `ai.evolve`, assert squads get harder.
- **Perf soak:** god mode + time scale 4 for 30 minutes, then read `perf.stats` and the trace dashboard.

---

## 2. Crash & performance reporting (`ashlar-trace`)

### Setup
```rust
ashlar_trace::events! {
    pub enum Category { Match = "match", Ai = "ai", Perf = "perf" }
    pub enum GameEvent {
        MatchStart = "match.start" in Match,
        MatchEnd   = "match.end"   in Match,
        RoundEnd   = "round.end"   in Match,
        PlayerDied = "player.died" in Match,
        Evolved    = "ai.evolved"  in Ai,
        AiReset    = "ai.reset"    in Ai,
        Hitch      = "perf.hitch"  in Perf,
    }
}
let tracer = ashlar_trace::Tracer::builder(WRITE_KEY, "shroud", env!("CARGO_PKG_VERSION"))
    .data_dir(data_dir)                       // ~/Library/Application Support/shroud/
    .endpoint(TRACE_ENDPOINT)                 // self-hosted ashlar-trace-server
    .listen(Category::Match).listen(Category::Ai).listen(Category::Perf)
    .init();
```
Inserted as a Bevy `Resource` by `TracePlugin`.

### Bevy hooks
| Hook | Where | Call |
|---|---|---|
| Hang detection | `First` starts `tracer.turn()`, `Last` drops it | a frame that never ends → hang report |
| Frame time | `Last` | `tracer.frame(time.delta())` (real time, not virtual) |
| Gauges | every 1 s | entity count, enemy count, generation, `RenderDiagnostics` GPU time when available, process memory |
| Events | match flow / evolve systems | `tracer.track(GameEvent::…)` with props (generation, rounds, duration, evolve ms) |
| Panics | startup | ashlar-trace panic hook, chained before Bevy's; backtrace + recent log lines |
| Unclean exit | next launch | reported automatically by ashlar-trace |
| Evolve thread | trainer | panic is caught, reported, and the game continues with the old squad |

Long but expected stalls (shader compile on first load) are excluded by starting hang detection only after the first `Playing` frame.

### Privacy
- Nothing is sent until the first-run notice in the Menu is shown (`tracer.notice_shown()`).
- Settings toggles per category; `DO_NOT_TRACK=1` honoured. No player names; player-model data is **not** uploaded.

### Dev setup
Run `ashlar-trace-server` locally; point `TRACE_ENDPOINT` at it via env (see `/Users/himanshu/.claude/bin/env-keys` for checking `.env`). The dashboard shows crash groups, frame-time percentiles per version, hitches, match/evolve events.

---

## Tasks
| Stream | Scope | Done when |
|---|---|---|
| **H1 Control** | `ops/control.rs`, `ops/commands.rs`, `--control/--mcp/--headless/--seed`, the command table above | `tests/control.rs`: starts headless game on port 0, reads key file, `app.ping`, `game.start`, `player.move`, `game.state` round-trip |
| **H2 Trace** | `ops/trace.rs` `TracePlugin`, events, gauges, panic hook, first-run notice | local server shows a session, frame histogram, and a deliberate test panic (`--crash-test`) |
