//! `GameState::Evolving`: run `brain::evolve` on a background thread (the window
//! stays responsive), show progress, save `brain.json`, then go to `Debrief`.

use bevy::prelude::*;
use brain::arena::Arena;
use brain::evolve::{BrainState, EvolveConfig, EvolveSummary};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;

use crate::match_flow::PendingReport;
use crate::ops::session_log::SessionLog;
use crate::save::{Brain, LastEvolve};
use crate::state::{GameState, has_visuals};
use crate::trainer_task::{BackgroundTrainer, TrainerTaskPlugin};

pub struct EvolvingPlugin;

impl Plugin for EvolvingPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TrainerTaskPlugin).add_systems(OnEnter(GameState::Evolving), (start, spawn_label.run_if(has_visuals)))
            .add_systems(Update, (poll, update_label.run_if(has_visuals)).run_if(in_state(GameState::Evolving)));
    }
}

#[derive(Resource)]
struct Job {
    handle: Option<JoinHandle<(BrainState, EvolveSummary)>>,
    progress: Arc<AtomicU32>,
    /// Simulated rounds merged from the background trainer / behind the brain so far.
    merged_rounds: u64,
    total_rounds: u64,
    sim_vs_real: String,
}

#[derive(Component)]
struct ProgressText;

#[derive(Component)]
struct DetailText;

fn start(
    mut commands: Commands,
    brain: Res<Brain>,
    mut pending: ResMut<PendingReport>,
    mut bg: ResMut<BackgroundTrainer>,
    log: Option<Res<SessionLog>>,
    arena: Res<crate::map::ArenaRes>,
) {
    let map = arena.0.id;
    let report = pending.0.take().unwrap_or_default();
    let mut state = brain.state.clone();
    // Between matches: merge what the background trainer learned since the last match.
    let background_rounds = bg.session_rounds();
    let trained = bg.take_for_merge(&state, log.as_deref());
    let merged_rounds = if trained.is_some() { background_rounds } else { 0 };
    // Sim-vs-real for the squad that just played.
    let total = report.player_rounds + report.enemy_rounds;
    let sim_vs_real = match (state.champion.as_ref(), total) {
        (Some(c), t) if t > 0 => format!(
            "This squad: won {:.0}% of real rounds vs {:.0}% in the sim",
            100.0 * report.enemy_rounds as f32 / t as f32,
            100.0 * c.score.win_rate()
        ),
        _ => String::new(),
    };
    let progress = Arc::new(AtomicU32::new(0));
    let p = progress.clone();
    let handle = std::thread::Builder::new()
        .name("evolve".into())
        .spawn(move || {
            let arena = Arena::build(map);
            let summary = state.evolve_merged(&report, trained, &arena, &EvolveConfig::default(), |f| p.store((f * 100.0) as u32, Ordering::Relaxed));
            (state, summary)
        })
        .ok();
    commands.insert_resource(Job { handle, progress, merged_rounds, total_rounds: brain.state.sim_rounds + merged_rounds, sim_vs_real });
}

fn poll(
    mut job: ResMut<Job>,
    mut brain: ResMut<Brain>,
    mut last: ResMut<LastEvolve>,
    mut next: ResMut<NextState<GameState>>,
    tracer: Option<Res<crate::ops::trace::GameTracer>>,
) {
    let done = job.handle.as_ref().is_none_or(JoinHandle::is_finished);
    if !done {
        return;
    }
    match job.handle.take().map(JoinHandle::join) {
        Some(Ok((state, summary))) => {
            info!("evolved to generation {} in {} ms", summary.generation, summary.elapsed_ms);
            brain.state = state;
            brain.save();
            if let Some(t) = &tracer {
                use ashlar_trace::Prop;
                crate::ops::trace::trace_event(
                    t,
                    crate::ops::trace::GameEvent::Evolved,
                    &[("generation", Prop::Int(summary.generation as i64)), ("ms", Prop::Int(summary.elapsed_ms as i64)), ("sim_win_rate", Prop::Float(summary.sim_win_rate as f64))],
                );
            }
            last.0 = Some(summary);
        }
        // Training failed: keep the old squad and carry on.
        _ => {
            error!("evolution failed; keeping the previous squad");
            last.0 = None;
        }
    }
    next.set(GameState::Debrief);
}

#[derive(Component)]
struct ProgressFill;

fn spawn_label(mut commands: Commands) {
    use crate::ui::theme::*;
    commands
        .spawn((
            DespawnOnExit(GameState::Evolving),
            fullscreen(),
            BackgroundColor(BG.with_alpha(0.94)),
            GlobalZIndex(20),
        ))
        .with_children(|s| {
            s.spawn((
                Node {
                    width: percent(100),
                    height: percent(100),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: px(GAP_M),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|c| {
                c.spawn(eyebrow("Between matches", INTEL));
                c.spawn(text("THE SQUAD IS EVOLVING", display(SIZE_H1), TEXT));
                c.spawn((
                    Node { width: px(640), height: px(6), margin: UiRect::top(px(GAP_S)), ..default() },
                    BackgroundColor(RAISED),
                    Pickable::IGNORE,
                ))
                .with_child((ProgressFill, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(INTEL), Pickable::IGNORE));
                c.spawn((ProgressText, text("Training against your ghost... 0%", body_bold(SIZE_BODY), TEXT)));
                c.spawn((DetailText, text("", body(SIZE_SMALL), TEXT_MUTED), TextLayout::justify(Justify::Center)));
            });
        });
}

fn update_label(
    job: Res<Job>,
    mut q: Query<&mut Text, With<ProgressText>>,
    mut d: Query<&mut Text, (With<DetailText>, Without<ProgressText>)>,
    mut fill: Query<&mut Node, With<ProgressFill>>,
) {
    let pct = job.progress.load(Ordering::Relaxed);
    for mut n in &mut fill {
        n.width = percent(pct.min(100) as f32);
    }
    for mut t in &mut q {
        let s = format!("Training against your ghost... {pct}%");
        if t.0 != s {
            t.0 = s;
        }
    }
    let mut s = format!("{} simulated rounds so far", job.total_rounds);
    if job.merged_rounds > 0 {
        s += &format!(" ({} trained in the background since the last match)", job.merged_rounds);
    }
    if !job.sim_vs_real.is_empty() {
        s += &format!("\n{}", job.sim_vs_real);
    }
    for mut t in &mut d {
        if t.0 != s {
            t.0 = s.clone();
        }
    }
}
