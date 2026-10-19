//! Crash, hang and performance reporting through `ashlar-trace`.
//!
//! - The tracer is built from `SHROUD_TRACE_ENDPOINT` and `SHROUD_TRACE_KEY`; without both it is a
//!   no-op tracer (never fails, never blocks).
//! - Hang detection: a turn runs from `First` to `Last`, armed only after the first `Playing`
//!   frame (so shader compiles and asset loads at startup are not hangs).
//! - `frame()` every frame (real time), gauges every second, `perf.hitch` for slow frames.
//! - The panic hook is installed when the tracer starts, in `TracePlugin::build`. That runs
//!   after `DefaultPlugins`, so it wraps Bevy's hook: ours reports first, then chains to Bevy's.
//! - Nothing is sent until the menu has been shown (`notice_shown`).
//! - `--crash-test` panics on purpose 2 s after start.
//!
//! Other modules report events with [`trace_event`].

use std::time::Duration;

use ashlar_trace::{Gauge, Prop, Tracer};
use bevy::prelude::*;

use super::{data_dir, has_flag};
use crate::state::{GameState, has_visuals};

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

/// A frame longer than this (after the first `Playing` frame) is reported as `perf.hitch`.
const HITCH: Duration = Duration::from_millis(250);

/// The game's tracer. Cheap to clone; every call is a no-op when tracing is off.
#[derive(Resource, Clone)]
pub struct GameTracer(pub Tracer);

/// Anything [`trace_event`] can find the tracer through.
pub trait TraceSource {
    fn tracer(&self) -> Option<&Tracer>;
}

impl TraceSource for World {
    fn tracer(&self) -> Option<&Tracer> {
        self.get_resource::<GameTracer>().map(|t| &t.0)
    }
}

impl TraceSource for GameTracer {
    fn tracer(&self) -> Option<&Tracer> {
        Some(&self.0)
    }
}

impl TraceSource for Res<'_, GameTracer> {
    fn tracer(&self) -> Option<&Tracer> {
        Some(&self.0)
    }
}

impl TraceSource for Tracer {
    fn tracer(&self) -> Option<&Tracer> {
        Some(self)
    }
}

/// Records a game event: `trace_event(world, GameEvent::Evolved, &[("generation", Prop::Int(3))])`
/// with a `&World`, `&Res<GameTracer>` or `&GameTracer`.
pub fn trace_event(src: &(impl TraceSource + ?Sized), event: GameEvent, props: &[(&'static str, Prop)]) {
    if let Some(t) = src.tracer() {
        t.track_with(event, props);
    }
}

#[derive(Resource, Default)]
struct TraceState {
    /// Hang detection runs (set after the first `Playing` frame).
    armed: bool,
    in_turn: bool,
    started: bool,
    next_gauges: Duration,
    gauges: Option<Gauges>,
}

struct Gauges {
    entities: Gauge,
    fps: Gauge,
}

#[derive(Resource)]
struct CrashTest;

pub struct TracePlugin;

impl Plugin for TracePlugin {
    fn build(&self, app: &mut App) {
        let tracer = make_tracer();
        let gauges = tracer.is_enabled().then(|| Gauges {
            entities: tracer.gauge("entities"),
            fps: tracer.gauge("fps"),
        });
        app.insert_resource(GameTracer(tracer))
            .insert_resource(TraceState { gauges, ..default() })
            .add_systems(First, begin_turn)
            .add_systems(Last, end_frame)
            .add_systems(OnEnter(GameState::Menu), notice_shown.run_if(has_visuals));
        if has_flag("crash-test") {
            app.insert_resource(CrashTest)
                .add_systems(Update, crash_test.run_if(resource_exists::<CrashTest>));
        }
    }
}

fn make_tracer() -> Tracer {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    match (var("SHROUD_TRACE_ENDPOINT"), var("SHROUD_TRACE_KEY")) {
        (Some(endpoint), Some(key)) => Tracer::builder(&key, "shroud", env!("CARGO_PKG_VERSION"))
            .data_dir(data_dir().join("trace"))
            .endpoint(endpoint)
            .listen(Category::Match)
            .listen(Category::Ai)
            .listen(Category::Perf)
            .init(),
        _ => Tracer::disabled(),
    }
}

fn begin_turn(tracer: Res<GameTracer>, mut st: ResMut<TraceState>) {
    if st.armed && !st.in_turn {
        tracer.0.begin_turn();
        st.in_turn = true;
    }
}

fn end_frame(world: &mut World) {
    let tracer = world.resource::<GameTracer>().0.clone();
    if !tracer.is_enabled() {
        return;
    }
    let real = *world.resource::<Time<Real>>();
    let playing = world
        .get_resource::<State<GameState>>()
        .is_some_and(|s| *s.get() == GameState::Playing);
    let entities = world.entities().count_spawned();
    let mut st = world.resource_mut::<TraceState>();

    let dt = real.delta();
    tracer.frame(dt);
    if !st.started {
        tracer.first_frame();
        st.started = true;
    }
    if st.in_turn {
        tracer.end_turn();
        st.in_turn = false;
        if dt > HITCH {
            tracer.track_with(GameEvent::Hitch, &[("ms", Prop::Int(dt.as_millis() as i64))]);
        }
    }
    if !st.armed && playing {
        st.armed = true;
    }
    if real.elapsed() >= st.next_gauges {
        st.next_gauges = real.elapsed() + Duration::from_secs(1);
        if let Some(g) = &st.gauges {
            g.entities.set(entities as f64);
            if dt > Duration::ZERO {
                g.fps.set(1.0 / dt.as_secs_f64());
            }
        }
    }
}

fn notice_shown(tracer: Res<GameTracer>) {
    tracer.0.notice_shown();
}

fn crash_test(time: Res<Time<Real>>) {
    if time.elapsed() > Duration::from_secs(2) {
        panic!("--crash-test: deliberate panic to test crash reporting");
    }
}
