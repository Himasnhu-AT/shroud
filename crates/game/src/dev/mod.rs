//! Dev-only glue (feature `dev`, on for `cargo run` / `cargo test`): Shroud's control commands
//! on the engine's control port, perf spans, metrics keys and labels, session-log events and
//! crash/perf trace events. In ship builds (`--no-default-features`) the helpers here are no-ops
//! and none of the engine's devtools are compiled in. Trace events alone also work in release
//! builds with the `telemetry` feature (no devtools).

#[cfg(feature = "dev")]
mod ai_commands;
#[cfg(feature = "dev")]
mod cinema;
#[cfg(feature = "dev")]
mod commands;
#[cfg(feature = "dev")]
mod lighting;
#[cfg(feature = "dev")]
pub use commands::ControlDrive;

use std::marker::PhantomData;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde_json::Value;

/// A trace event property (mirrors `engine::trace::Prop`, which only exists with `telemetry`).
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(feature = "telemetry"), allow(dead_code))]
pub enum Prop {
    Bool(bool),
    Int(i64),
    Float(f64),
}

#[cfg(feature = "telemetry")]
fn props(p: &[(&'static str, Prop)]) -> Vec<(&'static str, engine::trace::Prop)> {
    use engine::trace::Prop as P;
    p.iter()
        .map(|(k, v)| {
            let v = match *v {
                Prop::Bool(b) => P::Bool(b),
                Prop::Int(i) => P::Int(i),
                Prop::Float(f) => P::Float(f),
            };
            (*k, v)
        })
        .collect()
}

/// Records a crash/perf trace event (declared in [`DevPlugin`]). No-op without `telemetry` or
/// when tracing is off.
pub fn trace(world: &World, name: &'static str, p: &[(&'static str, Prop)]) {
    #[cfg(feature = "telemetry")]
    engine::trace::trace_event(world, name, &props(p));
    #[cfg(not(feature = "telemetry"))]
    let _ = (world, name, p);
}

/// Session log + trace from systems. Every method is a no-op without `dev`.
#[derive(SystemParam)]
pub struct Dev<'w> {
    #[cfg(feature = "dev")]
    log: Option<Res<'w, engine::devtools::SessionLog>>,
    #[cfg(feature = "telemetry")]
    tracer: Option<Res<'w, engine::trace::GameTracer>>,
    _p: PhantomData<&'w ()>,
}

impl Dev<'_> {
    /// A session-log event line (`fields` is only built when a log is open).
    pub fn log(&self, name: &str, fields: impl FnOnce() -> Value) {
        #[cfg(feature = "dev")]
        if let Some(l) = &self.log {
            l.event(name, fields());
        }
        #[cfg(not(feature = "dev"))]
        let _ = (name, fields);
    }

    pub fn trace(&self, name: &'static str, p: &[(&'static str, Prop)]) {
        #[cfg(feature = "telemetry")]
        if let Some(t) = &self.tracer {
            engine::trace::trace_event(t, name, &props(p));
        }
        #[cfg(not(feature = "telemetry"))]
        let _ = (name, p);
    }
}

pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, _app: &mut App) {
        #[cfg(feature = "telemetry")]
        build_telemetry(_app);
        #[cfg(feature = "dev")]
        build_dev(_app);
    }
}

/// Match events for crash/perf reports, and the Settings toggle that lets players switch
/// reporting off (dev and release builds with `telemetry`).
#[cfg(feature = "telemetry")]
fn build_telemetry(app: &mut App) {
    use engine::settings::{FieldDef, FieldKind, RegisterSettings};
    use engine::trace::{GameTracer, TraceAppExt, reporting, set_reporting};

    app.declare_trace_events(&["match.start", "match.end", "round.end", "player.died", "ai.evolved", "ai.reset", "bomb.planted", "bomb.defused", "bomb.exploded", "career.objectives", "career.objective", "career.credited", "career.promoted"])
        .register_setting(FieldDef {
            key: "send_reports",
            tab: "Gameplay",
            label: "Send crash and performance reports",
            hint: "Crashes, frame times and match results, with no name attached. What the squad learns about you never leaves this computer.",
            kind: FieldKind::Toggle,
            // No tracer in the world = "Reset tab"'s scratch world: the default is on.
            get: |w| w.get_resource::<GameTracer>().map_or(1.0, |t| reporting(t) as u8 as f32),
            set: |w, v| {
                if let Some(t) = w.get_resource::<GameTracer>() {
                    set_reporting(t, v > 0.5);
                }
            },
        });
}

#[cfg(feature = "dev")]
fn build_dev(app: &mut App) {
    use engine::devtools::{AddControlCommand, PerfAppExt, PerfOverlayKey};

    use crate::enemy::EnemySet;
    use crate::weapon::WeaponSet;

    // F3 is the enemy debug view; the perf overlay stays on F2.
    app.insert_resource(PerfOverlayKey(KeyCode::F2))
        .perf_span(FixedUpdate, EnemySet, "ai")
        .perf_span(Update, WeaponSet, "weapons")
        .add_systems(Update, watch_events);
    commands::register(app);
    ai_commands::register(app);
    cinema::register(app);
    lighting::register(app);
    // Pre-engine command names used by scripts and tests.
    app.alias_control_command("game.quit", "engine.quit")
        .alias_control_command("game.pause", "engine.pause")
        .alias_control_command("game.resume", "engine.resume")
        .alias_control_command("capture.screenshot", "screenshot");
    // Metrics only exist when the engine's devtools are on for this run (not in headless tests).
    if app.world().contains_resource::<engine::devtools::Metrics>() {
        app.add_systems(Update, record_metrics);
        let mut m = app.world_mut().resource_mut::<engine::devtools::Metrics>();
        m.set_threshold("ai.think_ms", 2.0, 4.0);
        m.set_threshold("ai.squad_ms", 0.5, 1.0);
        if let Some(mut c) = app.world_mut().get_resource_mut::<engine::devtools::MetricsCompare>() {
            c.add("ai.think_ms", "avg").add("ai.think_ms", "p95").add("ai.squad_ms", "avg");
        }
    }
}

/// One sample per frame of Shroud's keys, plus the map/difficulty labels that make runs
/// comparable in the metrics window.
#[cfg(feature = "dev")]
fn record_metrics(
    mut m: ResMut<engine::devtools::Metrics>,
    mut timings: ResMut<crate::enemy::AiTimings>,
    enemies: Query<&crate::enemy::EnemyCombat, With<crate::enemy::Enemy>>,
    flow: Res<crate::match_flow::MatchFlow>,
    state: Option<Res<State<crate::GameState>>>,
    settings: Res<crate::settings::ShroudSettings>,
    arena: Res<crate::map::ArenaRes>,
) {
    if state.is_some_and(|s| *s.get() == crate::GameState::Playing) {
        let (alive, engaged) = enemies.iter().filter(|c| !c.dead).fold((0u32, 0u32), |(n, e), c| (n + 1, e + c.engaged() as u32));
        m.record("enemies.alive", alive as f64);
        m.record("enemies.engaged", engaged as f64);
        m.record("match.round", flow.round as f64);
        if timings.ran {
            m.record("ai.think_ms", timings.think_ms as f64);
            m.record("ai.squad_ms", timings.squad_ms as f64);
            timings.ran = false;
        }
    }
    for (key, value) in [("map", arena.0.id.label()), ("difficulty", settings.difficulty.label())] {
        if m.labels().get(key).map(String::as_str) != Some(value) {
            m.set_label(key, value);
        }
    }
}

#[cfg(feature = "dev")]
#[derive(Default)]
struct EventWatch {
    state: Option<crate::GameState>,
    match_no: u32,
    round: u32,
    round_over: bool,
    brain_seed: Option<u64>,
}

/// Match / round / evolution lines in the session log (the engine logs `AppState` changes,
/// pause and resume itself).
#[cfg(feature = "dev")]
fn watch_events(
    log: Option<Res<engine::devtools::SessionLog>>,
    state: Option<Res<State<crate::GameState>>>,
    flow: Res<crate::match_flow::MatchFlow>,
    evolve: Res<crate::save::LastEvolve>,
    brain: Res<crate::save::Brain>,
    mut w: Local<EventWatch>,
) {
    use crate::GameState;
    use serde_json::json;

    let Some(log) = log else { return };
    let now = state.map(|s| *s.get());
    let mut match_ended = false;
    if now != w.state {
        log.event("state", json!({"from": w.state.map(|s| format!("{s:?}")), "to": now.map(|s| format!("{s:?}"))}));
        // A finished match always goes Playing → Evolving.
        match_ended = w.state == Some(GameState::Playing) && now == Some(GameState::Evolving);
        w.state = now;
    }
    if match_ended && let Some((won, pr, er)) = flow.last_result {
        log.event("match.end", json!({"match": flow.match_no, "player_won": won, "player_rounds": pr, "enemy_rounds": er}));
    }
    if flow.is_changed() {
        let f = &*flow;
        if f.match_no != w.match_no {
            log.event("match.start", json!({"match": f.match_no}));
        }
        if f.match_no != w.match_no || f.round != w.round {
            log.event("round.start", json!({"match": f.match_no, "round": f.round}));
        }
        if let (Some((_, won)), false) = (f.round_over, w.round_over) {
            log.event(
                "round.end",
                json!({"match": f.match_no, "round": f.round, "player_won": won,
                       "player_rounds": f.player_rounds, "enemy_rounds": f.enemy_rounds}),
            );
        }
        (w.match_no, w.round, w.round_over) = (f.match_no, f.round, f.round_over.is_some());
    }
    if evolve.is_changed()
        && let Some(s) = &evolve.0
    {
        log.event(
            "ai.evolved",
            json!({"generation": s.generation, "sim_win_rate": s.sim_win_rate, "ms": s.elapsed_ms as u64,
                   "neat_generations": s.neat_generations, "insights": s.insights}),
        );
    }
    let seed = brain.state.seed;
    if w.brain_seed.is_some_and(|old| old != seed) {
        log.event("ai.reset", json!({"generation": brain.state.generation}));
    }
    w.brain_seed = Some(seed);
}
