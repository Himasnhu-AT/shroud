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

use crate::ops::session_log::SessionLog;
use crate::save::{Brain, data_dir};
use crate::state::{GameState, Headless};

pub struct TrainerTaskPlugin;

impl Plugin for TrainerTaskPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BackgroundTrainer>()
            .add_systems(OnEnter(GameState::Menu), start)
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
    pub fn take_for_merge(&mut self, live: &BrainState, log: Option<&SessionLog>) -> Option<BrainState> {
        self.halt(log);
        let t = self.parked.take()?;
        let _ = std::fs::remove_file(checkpoint_path());
        self.rounds_unmerged = 0;
        let ok = t.matches(live) && t.state.evaluated && t.steps > 0;
        if let Some(l) = log {
            l.event("trainer", json!({ "action": if ok { "merge" } else { "discard" }, "steps": t.steps, "rounds": t.session_rounds }));
        }
        ok.then_some(t.state)
    }

    fn halt(&mut self, log: Option<&SessionLog>) {
        let Some(h) = self.running.take() else { return };
        if let Some(t) = h.finish() {
            if let Some(l) = log {
                let win = t.last.map_or(0.0, |s| s.champion_win);
                l.event("trainer", json!({ "action": "stop", "steps": t.steps, "rounds": t.session_rounds, "champion_win": win }));
            }
            self.rounds_unmerged = t.session_rounds;
            self.parked = Some(t);
        }
    }
}

fn checkpoint_path() -> PathBuf {
    data_dir().join("trainer.json")
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

fn start(mut bg: ResMut<BackgroundTrainer>, brain: Res<Brain>, headless: Res<Headless>, log: Option<Res<SessionLog>>) {
    if headless.0 || crate::ops::has_flag("no-trainer") || bg.running.is_some() || !brain.state.evaluated {
        return;
    }
    // Resume the parked session, or a checkpoint from an earlier run, if still valid.
    let trainer = bg
        .parked
        .take()
        .or_else(|| Trainer::load(&checkpoint_path()))
        .filter(|t| t.matches(&brain.state))
        .unwrap_or_else(|| Trainer::new(brain.state.clone()));
    let cfg = TrainerConfig { threads: 2, worker_init: Some(lower_priority), ..Default::default() };
    let (steps, rounds) = (trainer.steps, trainer.session_rounds);
    match TrainerHandle::spawn(trainer, cfg, Some(checkpoint_path())) {
        Ok(h) => {
            bg.running = Some(h);
            if let Some(l) = &log {
                l.event("trainer", json!({ "action": "start", "generation": brain.state.generation, "resumed_steps": steps, "rounds": rounds }));
            }
        }
        Err(e) => warn!("trainer: could not start: {e}"),
    }
}

fn stop(mut bg: ResMut<BackgroundTrainer>, log: Option<Res<SessionLog>>) {
    bg.halt(log.as_deref());
}
