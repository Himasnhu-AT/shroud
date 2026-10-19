//! Perf monitor: process RAM/CPU/GPU, per-thread CPU, frame-time percentiles and per-system-set
//! cost, sampled at 1 Hz on a background thread, logged to the session log and the tracer.
//!
//! Frame side (cheap, every frame): push the real frame time, time `EnemySet` (AI), avian's
//! `StepSimulation` (physics) and `WeaponSet` with marker systems, detect hitches (> 50 ms).
//! Once a second it sends a [`FrameAgg`] to the `perf-sampler` thread, which probes the OS
//! (`perf_sys`), merges both into a [`PerfSnapshot`] (read by the overlay and `perf.stats`),
//! writes a `perf` line to the session log and sets the tracer gauges.
//!
//! GPU frame time is this process's IOKit GPU time per frame (macOS). Per-pass GPU times come
//! from Bevy's `RenderDiagnosticsPlugin`; Apple GPUs can't take timestamps inside passes (Bevy
//! reads 0), so on macOS it's only added with `--gpu-passes`. Game events (state, match/round, evolution, AI reset) are logged from
//! existing resources.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use avian3d::prelude::PhysicsSystems;
use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use serde::Serialize;
use serde_json::{Value, json};

use super::perf_sys::{self, GpuProbe, ThreadCpu};
use super::session_log::SessionLog;
use super::trace::GameTracer;
use super::{data_dir, has_flag};
use crate::enemy::EnemySet;
use crate::match_flow::MatchFlow;
use crate::save::{Brain, LastEvolve};
use crate::state::{GameState, Headless};
use crate::weapon::WeaponSet;

/// A frame longer than this (once playing) is logged as a hitch.
const HITCH: Duration = Duration::from_millis(50);
/// Frame times kept for the percentiles (~10 s at 60 fps).
const RING: usize = 600;
/// Threads listed in a snapshot.
const TOP_THREADS: usize = 8;

/// Perf overlay visible (F2 toggles; `--perf` starts on; `perf.overlay` over the control port).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerfOverlay(pub bool);

/// Latest snapshot from the sampler thread.
#[derive(Resource, Clone, Default)]
pub struct PerfShared(pub Arc<Mutex<Option<Arc<PerfSnapshot>>>>);

impl PerfShared {
    pub fn latest(&self) -> Option<Arc<PerfSnapshot>> {
        self.0.lock().ok().and_then(|s| s.clone())
    }
}

/// Main-thread time the monitor itself costs (frame systems + overlay), for the overhead gate.
#[derive(Resource, Default)]
pub struct PerfCost {
    pub frame_ns: u64,
    sum_ns: u64,
    frames: u32,
    /// Average per frame over the last second, µs.
    pub avg_us: f32,
    pub max_us: f32,
    max_ns: u64,
}

impl PerfCost {
    pub fn add(&mut self, d: Duration) {
        self.frame_ns += d.as_nanos() as u64;
    }
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Pct {
    pub p50: f32,
    pub p95: f32,
    pub p99: f32,
    pub max: f32,
}

#[derive(Serialize, Clone, Debug)]
pub struct ThreadRow {
    pub name: String,
    pub cpu_pct: f32,
}

#[derive(Serialize, Clone, Debug)]
pub struct PassRow {
    pub name: String,
    pub ms: f32,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct GpuRow {
    /// This process's share of GPU time, % (IOKit AppUsage).
    pub process_util_pct: Option<f32>,
    /// Whole-device utilisation, % (all apps).
    pub device_util_pct: Option<f32>,
    /// This process's Metal allocations, MB.
    pub process_alloc_mb: Option<f32>,
    /// Whole-device GPU memory in use, MB.
    pub device_in_use_mb: Option<f32>,
    /// This process's GPU busy time per frame, ms (IOKit time / frames).
    pub frame_ms: Option<f32>,
    /// Sum of top-level render pass GPU times, ms (timestamp queries).
    pub passes_ms: Option<f32>,
    pub passes: Vec<PassRow>,
}

/// Per-frame averages (ms) of what the game measures itself, over the last second.
#[derive(Serialize, Clone, Debug, Default)]
pub struct GameRow {
    pub state: String,
    pub entities: u32,
    pub ai_ms: f32,
    pub ai_max_ms: f32,
    pub physics_ms: f32,
    pub weapons_ms: f32,
}

/// Everything the monitor knows, once a second.
#[derive(Serialize, Clone, Debug, Default)]
pub struct PerfSnapshot {
    pub fps: f32,
    pub frame_ms: Pct,
    /// Process CPU, % of one core.
    pub cpu_pct: Option<f32>,
    pub cores: usize,
    pub threads: Vec<ThreadRow>,
    /// Thread pools (name without ` (n)`), summed % of one core.
    pub pools: Vec<ThreadRow>,
    pub rss_mb: Option<f32>,
    pub footprint_mb: Option<f32>,
    pub gpu: GpuRow,
    pub game: GameRow,
    pub hitches: u32,
    /// Monitor's own main-thread cost, µs per frame (average, worst) over the last second.
    pub overhead_us: f32,
    pub overhead_max_us: f32,
    /// The main thread sent nothing for > 1.5 s (frame data is stale).
    pub stalled: bool,
}

/// What the main thread hands the sampler once a second.
#[derive(Clone, Debug, Default)]
struct FrameAgg {
    frames: u32,
    fps: f32,
    frame_ms: Pct,
    gpu_frame_ms: Option<f32>,
    passes: Vec<PassRow>,
    game: GameRow,
    hitches: u32,
    overhead_us: f32,
    overhead_max_us: f32,
}

#[derive(Default)]
struct Span {
    start: Option<Instant>,
    frame_ns: u64,
    sum_ns: u64,
    max_ns: u64,
}

impl Span {
    fn begin(&mut self) {
        self.start = Some(Instant::now());
    }
    fn end(&mut self) {
        if let Some(s) = self.start.take() {
            self.frame_ns += s.elapsed().as_nanos() as u64;
        }
    }
    fn close_frame(&mut self) {
        self.sum_ns += self.frame_ns;
        self.max_ns = self.max_ns.max(self.frame_ns);
        self.frame_ns = 0;
    }
    /// `(avg ms per frame, max ms)`, then reset.
    fn take(&mut self, frames: u32) -> (f32, f32) {
        let r = (
            self.sum_ns as f32 / frames.max(1) as f32 / 1e6,
            self.max_ns as f32 / 1e6,
        );
        self.sum_ns = 0;
        self.max_ns = 0;
        r
    }
}

#[derive(Resource, Default)]
struct SetClock {
    ai: Span,
    physics: Span,
    weapons: Span,
}

#[derive(Resource)]
struct PerfFrame {
    tx: Sender<FrameAgg>,
    times: VecDeque<f32>,
    frames: u32,
    max_dt: f32,
    hitches: u32,
    armed: bool,
    next: Duration,
}

pub struct PerfPlugin;

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        // Baseline for overhead measurements: the whole monitor off.
        if std::env::var("SHROUD_PERF").is_ok_and(|v| v == "off") {
            return;
        }
        let headless = has_flag("headless");
        let log = SessionLog::start(&data_dir().join("logs"));
        match &log {
            Some(l) => info!("perf: session log {}", l.path.display()),
            None => warn!("perf: could not open the session log"),
        }
        let shared = PerfShared::default();
        let (tx, rx) = channel();
        let gauges = app.world().get_resource::<GameTracer>().and_then(|t| Gauges::new(&t.0));
        let main_tid = perf_sys::current_thread_id();
        {
            let (shared, log) = (shared.clone(), log.clone());
            let _ = std::thread::Builder::new()
                .name("perf-sampler".into())
                .spawn(move || sampler(rx, shared, log, gauges, !headless, main_tid));
        }
        if let Some(l) = &log {
            l.event(
                "session.start",
                json!({"version": env!("CARGO_PKG_VERSION"), "pid": std::process::id(), "headless": headless,
                       "args": std::env::args().skip(1).collect::<Vec<_>>()}),
            );
            app.insert_resource(l.clone());
        }
        if !headless && (cfg!(not(target_os = "macos")) || has_flag("gpu-passes")) {
            app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
        }
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.add_systems(bevy::render::Render, note_render_thread);
        }
        app.insert_resource(shared)
            .insert_resource(PerfOverlay(has_flag("perf") && !headless))
            .init_resource::<PerfCost>()
            .init_resource::<SetClock>()
            .insert_resource(PerfFrame {
                tx,
                times: VecDeque::with_capacity(RING),
                frames: 0,
                max_dt: 0.0,
                hitches: 0,
                armed: false,
                next: Duration::from_secs(1),
            })
            .add_systems(
                Update,
                (
                    (|mut c: ResMut<SetClock>| c.ai.begin()).before(EnemySet),
                    (|mut c: ResMut<SetClock>| c.ai.end()).after(EnemySet),
                    (|mut c: ResMut<SetClock>| c.weapons.begin()).before(WeaponSet),
                    (|mut c: ResMut<SetClock>| c.weapons.end()).after(WeaponSet),
                ),
            )
            .add_systems(
                FixedPostUpdate,
                (
                    (|mut c: ResMut<SetClock>| c.physics.begin()).before(PhysicsSystems::StepSimulation),
                    (|mut c: ResMut<SetClock>| c.physics.end()).after(PhysicsSystems::StepSimulation),
                ),
            )
            .add_systems(Last, (watch_events, frame_tick).chain());
        #[cfg(feature = "control")]
        {
            use super::control::{ControlAppExt, params};
            app.add_control_command(
                "perf.overlay",
                "Show or hide the perf overlay",
                params(&[("on", "boolean", "default true")]),
                |world: &mut World, p: Value| {
                    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
                    world.resource_mut::<PerfOverlay>().0 = on;
                    Ok(json!({"on": on}))
                },
            );
        }
    }
}

/// Id of the render thread (Bevy's pipelined rendering thread is unnamed).
static RENDER_TID: AtomicU64 = AtomicU64::new(0);

/// Exclusive, so it runs on the thread driving the render schedule.
fn note_render_thread(_: &mut World) {
    if RENDER_TID.load(Ordering::Relaxed) == 0 {
        RENDER_TID.store(perf_sys::current_thread_id(), Ordering::Relaxed);
    }
}

/// Snapshot as JSON for `perf.stats`.
pub fn stats_json(world: &World) -> Value {
    let snap = world.get_resource::<PerfShared>().and_then(|s| s.latest());
    let cost = world.get_resource::<PerfCost>();
    json!({
        "monitor": snap.map(|s| serde_json::to_value(&*s).unwrap_or(Value::Null)),
        "overhead_us": cost.map(|c| c.avg_us),
        "overlay": world.get_resource::<PerfOverlay>().map(|o| o.0),
        "session_log": world.get_resource::<SessionLog>().map(|l| l.path.display().to_string()),
    })
}

fn frame_tick(
    real: Res<Time<Real>>,
    mut pf: ResMut<PerfFrame>,
    mut clock: ResMut<SetClock>,
    mut cost: ResMut<PerfCost>,
    state: Option<Res<State<GameState>>>,
    store: Option<Res<DiagnosticsStore>>,
    log: Option<Res<SessionLog>>,
    entities: &bevy::ecs::entity::Entities,
) {
    let t0 = Instant::now();
    let dt = real.delta_secs();
    let playing = state.as_ref().is_some_and(|s| *s.get() == GameState::Playing);
    if dt > 0.0 {
        if pf.times.len() == RING {
            pf.times.pop_front();
        }
        pf.times.push_back(dt);
        pf.frames += 1;
        pf.max_dt = pf.max_dt.max(dt);
        if pf.armed && real.delta() > HITCH {
            pf.hitches += 1;
            if let Some(l) = &log {
                l.event(
                    "perf.hitch",
                    json!({"ms": dt * 1000.0, "state": state.as_ref().map(|s| format!("{:?}", s.get()))}),
                );
            }
        }
    }
    pf.armed |= playing;
    clock.ai.close_frame();
    clock.physics.close_frame();
    clock.weapons.close_frame();

    if real.elapsed() >= pf.next {
        pf.next = real.elapsed() + Duration::from_secs(1);
        let mut v: Vec<f32> = pf.times.iter().copied().collect();
        v.sort_unstable_by(f32::total_cmp);
        let pct = |p: f32| v.get(((v.len().max(1) - 1) as f32 * p).round() as usize).map_or(0.0, |x| x * 1000.0);
        let mean = if v.is_empty() { 0.0 } else { v.iter().sum::<f32>() / v.len() as f32 };
        let frames = pf.frames;
        let (ai_ms, ai_max_ms) = clock.ai.take(frames);
        let (physics_ms, _) = clock.physics.take(frames);
        let (weapons_ms, _) = clock.weapons.take(frames);
        let (gpu_frame_ms, passes) = store.map(|s| gpu_passes(&s)).unwrap_or_default();
        let agg = FrameAgg {
            frames,
            fps: if mean > 0.0 { 1.0 / mean } else { 0.0 },
            frame_ms: Pct { p50: pct(0.5), p95: pct(0.95), p99: pct(0.99), max: pf.max_dt * 1000.0 },
            gpu_frame_ms,
            passes,
            game: GameRow {
                state: state.map(|s| format!("{:?}", s.get())).unwrap_or_default(),
                entities: entities.count_spawned(),
                ai_ms,
                ai_max_ms,
                physics_ms,
                weapons_ms,
            },
            hitches: pf.hitches,
            overhead_us: cost.avg_us,
            overhead_max_us: cost.max_us,
        };
        pf.frames = 0;
        pf.max_dt = 0.0;
        pf.hitches = 0;
        let _ = pf.tx.send(agg);
        cost.avg_us = cost.sum_ns as f32 / cost.frames.max(1) as f32 / 1000.0;
        cost.max_us = cost.max_ns as f32 / 1000.0;
        cost.sum_ns = 0;
        cost.max_ns = 0;
        cost.frames = 0;
    }
    // Close this frame's cost (the overlay adds its share earlier in the frame).
    let ns = cost.frame_ns + t0.elapsed().as_nanos() as u64;
    cost.sum_ns += ns;
    cost.max_ns = cost.max_ns.max(ns);
    cost.frames += 1;
    cost.frame_ns = 0;
}

/// `(sum of top-level pass GPU ms, top passes)` from `render/<pass>/elapsed_gpu`.
fn gpu_passes(store: &DiagnosticsStore) -> (Option<f32>, Vec<PassRow>) {
    let mut passes: Vec<PassRow> = store
        .iter()
        .filter_map(|d| {
            let p = d.path().as_str();
            let name = p.strip_prefix("render/")?.strip_suffix("/elapsed_gpu")?;
            if name.contains('/') {
                return None; // nested span; its parent already counts it
            }
            Some(PassRow { name: name.to_string(), ms: d.smoothed()? as f32 })
        })
        .filter(|p| p.ms > 0.0)
        .collect();
    if passes.is_empty() {
        return (None, passes);
    }
    let total = passes.iter().map(|p| p.ms).sum();
    passes.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    passes.truncate(6);
    (Some(total), passes)
}

#[derive(Default)]
struct EventWatch {
    state: Option<GameState>,
    match_no: u32,
    round: u32,
    round_over: bool,
    brain_seed: Option<u64>,
}

fn watch_events(
    log: Option<Res<SessionLog>>,
    mut cost: ResMut<PerfCost>,
    state: Option<Res<State<GameState>>>,
    flow: Option<Res<MatchFlow>>,
    evolve: Option<Res<LastEvolve>>,
    brain: Option<Res<Brain>>,
    mut w: Local<EventWatch>,
) {
    let Some(log) = log else { return };
    let t0 = Instant::now();
    let mut match_ended = false;
    if let Some(s) = state.filter(|s| w.state != Some(*s.get())) {
        log.event("state", json!({"from": w.state.map(|s| format!("{s:?}")), "to": format!("{:?}", s.get())}));
        // A finished match always goes Playing → Evolving.
        match_ended = w.state == Some(GameState::Playing) && *s.get() == GameState::Evolving;
        w.state = Some(*s.get());
    }
    if match_ended
        && let Some(f) = &flow
        && let Some((won, pr, er)) = f.last_result
    {
        log.event("match.end", json!({"match": f.match_no, "player_won": won, "player_rounds": pr, "enemy_rounds": er}));
    }
    if let Some(f) = flow.filter(|f| f.is_changed()) {
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
    if let Some(e) = evolve.filter(|e| e.is_changed())
        && let Some(s) = &e.0
    {
        log.event(
            "ai.evolved",
            json!({"generation": s.generation, "sim_win_rate": s.sim_win_rate, "ms": s.elapsed_ms as u64,
                   "neat_generations": s.neat_generations, "insights": s.insights}),
        );
    }
    if let Some(b) = brain {
        let seed = b.state.seed;
        if w.brain_seed.is_some_and(|old| old != seed) {
            log.event("ai.reset", json!({"generation": b.state.generation}));
        }
        w.brain_seed = Some(seed);
    }
    cost.add(t0.elapsed());
}

/// Tracer gauges (only when tracing is on).
struct Gauges {
    cpu: ashlar_trace::Gauge,
    rss: ashlar_trace::Gauge,
    footprint: ashlar_trace::Gauge,
    gpu_util: ashlar_trace::Gauge,
    gpu_mem: ashlar_trace::Gauge,
    gpu_ms: ashlar_trace::Gauge,
    p99: ashlar_trace::Gauge,
    ai_ms: ashlar_trace::Gauge,
}

impl Gauges {
    fn new(t: &ashlar_trace::Tracer) -> Option<Self> {
        t.is_enabled().then(|| Gauges {
            cpu: t.gauge("cpu_pct"),
            rss: t.gauge("rss_mb"),
            footprint: t.gauge("footprint_mb"),
            gpu_util: t.gauge("gpu_util_pct"),
            gpu_mem: t.gauge("gpu_mem_mb"),
            gpu_ms: t.gauge("gpu_frame_ms"),
            p99: t.gauge("frame_p99_ms"),
            ai_ms: t.gauge("ai_ms"),
        })
    }
}

fn sampler(
    rx: Receiver<FrameAgg>,
    shared: PerfShared,
    log: Option<SessionLog>,
    gauges: Option<Gauges>,
    gpu: bool,
    main_tid: u64,
) {
    let mut probe = gpu.then(GpuProbe::new);
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut prev_t = Instant::now();
    let mut prev_cpu = perf_sys::process_cpu_ns();
    let mut prev_threads: HashMap<u64, u64> = HashMap::new();
    let mut prev_gpu_ns: Option<u64> = None;
    let mut agg = FrameAgg::default();
    loop {
        let stalled = match rx.recv_timeout(Duration::from_millis(1500)) {
            Ok(a) => {
                agg = rx.try_iter().last().unwrap_or(a);
                false
            }
            Err(RecvTimeoutError::Timeout) => true,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let now = Instant::now();
        let wall_ns = now.duration_since(prev_t).as_nanos().max(1) as f64;
        prev_t = now;

        let cpu = perf_sys::process_cpu_ns();
        let cpu_pct = match (cpu, prev_cpu) {
            (Some(c), Some(p)) => Some((c.saturating_sub(p) as f64 / wall_ns * 100.0) as f32),
            _ => None,
        };
        prev_cpu = cpu;

        let threads: Vec<ThreadCpu> = perf_sys::threads();
        let mut rows: Vec<ThreadRow> = Vec::with_capacity(threads.len());
        let mut next_prev = HashMap::with_capacity(threads.len());
        for th in &threads {
            let d = prev_threads.get(&th.id).map_or(0, |p| th.cpu_ns.saturating_sub(*p));
            next_prev.insert(th.id, th.cpu_ns);
            let name = if th.id == main_tid {
                "main".to_string()
            } else if th.id == RENDER_TID.load(Ordering::Relaxed) {
                "render".to_string()
            } else if th.name.is_empty() {
                format!("thread-{}", th.id)
            } else {
                th.name.clone()
            };
            rows.push(ThreadRow { name, cpu_pct: (d as f64 / wall_ns * 100.0) as f32 });
        }
        prev_threads = next_prev;
        let mut pools: HashMap<String, f32> = HashMap::new();
        for r in &rows {
            let base = match r.name.rfind(" (") {
                Some(i) if r.name.ends_with(')') => &r.name[..i],
                _ => r.name.as_str(),
            };
            *pools.entry(base.to_string()).or_default() += r.cpu_pct;
        }
        let mut pools: Vec<ThreadRow> = pools
            .into_iter()
            .filter(|(_, pct)| *pct >= 0.1)
            .map(|(name, cpu_pct)| ThreadRow { name, cpu_pct })
            .collect();
        pools.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct));
        rows.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct));
        rows.truncate(TOP_THREADS);

        let mem = perf_sys::process_mem();
        let g = probe.as_mut().map(|p| p.sample()).unwrap_or_default();
        let process_util_pct = match (g.process_gpu_ns, prev_gpu_ns) {
            (Some(c), Some(p)) => Some((c.saturating_sub(p) as f64 / wall_ns * 100.0).min(100.0) as f32),
            _ => None,
        };
        let gpu_frame_ms = match (g.process_gpu_ns, prev_gpu_ns) {
            (Some(c), Some(p)) if !stalled && agg.frames > 0 => {
                Some(c.saturating_sub(p) as f32 / agg.frames as f32 / 1e6)
            }
            _ => None,
        };
        prev_gpu_ns = g.process_gpu_ns;
        let mb = |b: u64| b as f32 / 1_048_576.0;

        let snap = PerfSnapshot {
            fps: agg.fps,
            frame_ms: agg.frame_ms.clone(),
            cpu_pct,
            cores,
            threads: rows,
            pools,
            rss_mb: mem.map(|m| mb(m.0)),
            footprint_mb: mem.map(|m| mb(m.1)),
            gpu: GpuRow {
                process_util_pct,
                device_util_pct: g.device_util.map(|v| v as f32),
                process_alloc_mb: g.process_alloc.map(mb),
                device_in_use_mb: g.in_use_system_mem.map(mb),
                frame_ms: gpu_frame_ms,
                passes_ms: agg.gpu_frame_ms,
                passes: agg.passes.clone(),
            },
            game: agg.game.clone(),
            hitches: if stalled { 0 } else { agg.hitches },
            overhead_us: agg.overhead_us,
            overhead_max_us: agg.overhead_max_us,
            stalled,
        };
        if let Some(gs) = &gauges {
            let set = |g: &ashlar_trace::Gauge, v: Option<f32>| {
                if let Some(v) = v {
                    g.set(v as f64)
                }
            };
            set(&gs.cpu, snap.cpu_pct);
            set(&gs.rss, snap.rss_mb);
            set(&gs.footprint, snap.footprint_mb);
            set(&gs.gpu_util, snap.gpu.process_util_pct.or(snap.gpu.device_util_pct));
            set(&gs.gpu_mem, snap.gpu.process_alloc_mb);
            set(&gs.gpu_ms, snap.gpu.frame_ms);
            set(&gs.p99, Some(snap.frame_ms.p99));
            set(&gs.ai_ms, Some(snap.game.ai_ms));
        }
        if let Some(l) = &log {
            l.write("perf", serde_json::to_value(&snap).unwrap_or(Value::Null));
        }
        if let Ok(mut s) = shared.0.lock() {
            *s = Some(Arc::new(snap));
        }
    }
}

/// Headless-safe check used by the overlay plugin.
pub fn overlay_allowed(h: Res<Headless>) -> bool {
    !h.0
}
