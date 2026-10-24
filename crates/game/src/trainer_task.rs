//! Background trainer (roadmap P1-4): keeps evolving a copy of the brain while the player is in
//! the menu or debrief, on at most 2 low-priority threads. It stops when a match starts, and
//! its result is merged into the live brain only between matches (`Evolving`, before the
//! real-match evolve). Off in headless runs (it would make seeded runs timing-dependent) and
//! with `--no-trainer`.

use bevy::prelude::*;
use brain::evolve::BrainState;
use brain::trainer::{Trainer, TrainerConfig, TrainerHandle};
use serde_json::json;
use std::path::PathBuf;

use crate::dev::Dev;
use crate::save::Brain;
use crate::state::{AppState, GameState, Headless};

pub struct TrainerTaskPlugin;

impl Plugin for TrainerTaskPlugin {
    fn build(&self, app: &mut App) {
        let checkpoint = app.world().resource::<engine::paths::DataDir>().file("trainer.json");
        app.insert_resource(BackgroundTrainer { checkpoint, ..default() })
            .add_systems(OnEnter(AppState::MainMenu), start)
            .add_systems(OnEnter(GameState::Debrief), start)
            .add_systems(OnEnter(GameState::Playing), stop);
    }
}

/// The background trainer, idle or running.
#[derive(Resource, Default)]
pub struct BackgroundTrainer {
    running: Option<TrainerHandle>,
    /// Stopped session, kept to resume (or merge) while the brain hasn't moved on.
    parked: Option<Trainer>,
    /// Rounds simulated in background sessions since the last merge (for the UI).
    pub rounds_unmerged: u64,
    /// `<data dir>/trainer.json`: the session checkpoint.
    checkpoint: PathBuf,
}

impl BackgroundTrainer {
    /// Simulated rounds in the current (running or parked) session.
    pub fn session_rounds(&self) -> u64 {
        match (&self.running, &self.parked) {
            (Some(h), _) => h.rounds(),
            (None, Some(t)) => t.session_rounds,
            _ => 0,
        }
    }

    /// Stop training and hand over its brain if it is a continuation of `live`.
    /// Call only between matches.
    pub fn take_for_merge(&mut self, live: &BrainState, log: &Dev) -> Option<BrainState> {
        self.halt(log);
        let t = self.parked.take()?;
        let _ = std::fs::remove_file(&self.checkpoint);
        self.rounds_unmerged = 0;
        let ok = t.matches(live) && t.state.evaluated && t.steps > 0;
        log.log("trainer", || json!({ "action": if ok { "merge" } else { "discard" }, "steps": t.steps, "rounds": t.session_rounds }));
        ok.then_some(t.state)
    }

    fn halt(&mut self, log: &Dev) {
        let Some(h) = self.running.take() else { return };
        if let Some(t) = h.finish() {
            let win = t.last.map_or(0.0, |s| s.champion_win);
            log.log("trainer", || json!({ "action": "stop", "steps": t.steps, "rounds": t.session_rounds, "champion_win": win }));
            self.rounds_unmerged = t.session_rounds;
            self.parked = Some(t);
        }
    }
}

/// Lower the calling thread's scheduling priority so rendering always wins.
fn lower_priority() {
    // SAFETY: plain syscalls on the calling thread; failure is harmless.
    #[cfg(target_os = "macos")]
    unsafe {
        libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, libc::PRIO_DARWIN_BG);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 15);
    }
}

fn start(mut bg: ResMut<BackgroundTrainer>, brain: Res<Brain>, headless: Res<Headless>, log: Dev) {
    if headless.0 || engine::cli::dev_flag("no-trainer") || bg.running.is_some() || !brain.state.evaluated {
        return;
    }
    // Resume the parked session, or a checkpoint from an earlier run, if still valid.
    let trainer = bg
        .parked
        .take()
        .or_else(|| Trainer::load(&bg.checkpoint))
        .filter(|t| t.matches(&brain.state))
        .unwrap_or_else(|| Trainer::new(brain.state.clone()));
    let cfg = TrainerConfig { threads: 2, worker_init: Some(lower_priority), ..Default::default() };
    let (steps, rounds) = (trainer.steps, trainer.session_rounds);
    match TrainerHandle::spawn(trainer, cfg, Some(bg.checkpoint.clone())) {
        Ok(h) => {
            bg.running = Some(h);
            log.log("trainer", || json!({ "action": "start", "generation": brain.state.generation, "resumed_steps": steps, "rounds": rounds }));
        }
        Err(e) => warn!("trainer: could not start: {e}"),
    }
}

fn stop(mut bg: ResMut<BackgroundTrainer>, log: Dev) {
    bg.halt(&log);
}
