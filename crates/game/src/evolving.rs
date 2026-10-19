//! `GameState::Evolving`: run `brain::evolve` on a background thread (the window
//! stays responsive), show progress, save `brain.json`, then go to `Debrief`.

use bevy::prelude::*;
use brain::arena::Arena;
use brain::evolve::{BrainState, EvolveConfig, EvolveSummary};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;

use crate::match_flow::PendingReport;
use crate::save::{Brain, LastEvolve};
use crate::state::{GameState, has_visuals};

pub struct EvolvingPlugin;

impl Plugin for EvolvingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Evolving), (start, spawn_label.run_if(has_visuals)))
            .add_systems(Update, (poll, update_label.run_if(has_visuals)).run_if(in_state(GameState::Evolving)));
    }
}

#[derive(Resource)]
struct Job {
    handle: Option<JoinHandle<(BrainState, EvolveSummary)>>,
    progress: Arc<AtomicU32>,
}

#[derive(Component)]
struct ProgressText;

fn start(mut commands: Commands, brain: Res<Brain>, mut pending: ResMut<PendingReport>) {
    let report = pending.0.take().unwrap_or_default();
    let mut state = brain.state.clone();
    let progress = Arc::new(AtomicU32::new(0));
    let p = progress.clone();
    let handle = std::thread::Builder::new()
        .name("evolve".into())
        .spawn(move || {
            let arena = Arena::foundry();
            let summary = state.evolve(&report, &arena, &EvolveConfig::default(), |f| p.store((f * 100.0) as u32, Ordering::Relaxed));
            (state, summary)
        })
        .ok();
    commands.insert_resource(Job { handle, progress });
}

fn poll(mut job: ResMut<Job>, mut brain: ResMut<Brain>, mut last: ResMut<LastEvolve>, mut next: ResMut<NextState<GameState>>) {
    let done = job.handle.as_ref().is_none_or(JoinHandle::is_finished);
    if !done {
        return;
    }
    match job.handle.take().map(JoinHandle::join) {
        Some(Ok((state, summary))) => {
            info!("evolved to generation {} in {} ms", summary.generation, summary.elapsed_ms);
            brain.state = state;
            brain.save();
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

fn spawn_label(mut commands: Commands) {
    commands.spawn((
        DespawnOnExit(GameState::Evolving),
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(12),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
        children![
            (Text::new("The squad is evolving"), TextFont::from_font_size(40.0)),
            (
                ProgressText,
                Text::new("Training against your ghost… 0%"),
                TextFont::from_font_size(20.0),
                TextColor(Color::srgb(0.6, 0.65, 0.75)),
            ),
        ],
    ));
}

fn update_label(job: Res<Job>, mut q: Query<&mut Text, With<ProgressText>>) {
    let pct = job.progress.load(Ordering::Relaxed);
    for mut t in &mut q {
        let s = format!("Training against your ghost… {pct}%");
        if t.0 != s {
            t.0 = s;
        }
    }
}
