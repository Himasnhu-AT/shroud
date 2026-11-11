//! Background trainer (roadmap P1-4): keeps evolving a copy of the brain while the player is in
//! the menu or debrief, on at most 2 low-priority threads. It stops when a match starts, and
//! its result is merged into the live brain only between matches (`Evolving`, before the
//! real-match evolve). Off in headless runs (it would make seeded runs timing-dependent) and
//! with `--no-trainer`.

use bevy::prelude::*;
use brain::evolve::BrainState;
use brain::trainer::{Trainer, TrainerConfig, TrainerHandle};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
            .add_systems(OnEnter(GameState::Playing), stop)
            .add_systems(Update, reap)
            .add_systems(Last, stop_on_exit);
    }
}

/// The background trainer, idle or running.
#[derive(Resource, Default)]
pub struct BackgroundTrainer {
    running: Option<TrainerHandle>,
    /// `running` was asked to stop and is winding down (finishing its sim round, saving).
    stopping: bool,
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

    /// `<data dir>/trainer.json`.
    pub fn checkpoint(&self) -> &Path {
        &self.checkpoint
    }

    /// Stop training and hand over its brain if it is a continuation of `live`.
    /// Call only between matches. A merged session's checkpoint stays on disk: the caller
    /// deletes it (`checkpoint()`) only once the merged brain is saved, so quitting or a failed
    /// evolve can't lose the background training.
    pub fn take_for_merge(&mut self, live: &BrainState, log: &Dev) -> Option<BrainState> {
        self.halt(log);
        let t = self.parked.take()?;
        self.rounds_unmerged = 0;
        let ok = t.matches(live) && t.state.evaluated && t.steps > 0;
        if !ok {
            let _ = std::fs::remove_file(&self.checkpoint);
        }
        log.log("trainer", || json!({ "action": if ok { "merge" } else { "discard" }, "steps": t.steps, "rounds": t.session_rounds }));
        ok.then_some(t.state)
    }

    fn halt(&mut self, log: &Dev) {
        self.stopping = false;
        let Some(h) = self.running.take() else { return };
        match h.finish() {
            Some(t) => {
                let win = t.last.map_or(0.0, |s| s.champion_win);
                log.log("trainer", || json!({ "action": "stop", "steps": t.steps, "rounds": t.session_rounds, "champion_win": win }));
                self.rounds_unmerged = t.session_rounds;
                self.parked = Some(t);
            }
            None => warn!("trainer: the background trainer thread panicked; its session is lost (the last checkpoint, if any, is kept)"),
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
    // A trainer still winding down from the last match: collect it first (between matches,
    // so waiting is fine).
    if bg.stopping {
        bg.halt(&log);
    }
    if headless.0 || engine::cli::dev_flag("no-trainer") || bg.running.is_some() || !brain.state.evaluated {
        return;
    }
    // Resume the parked session, or a checkpoint from an earlier run, if still valid.
    let trainer = bg
        .parked
        .take()
        .or_else(|| match Trainer::load_checked(&bg.checkpoint) {
            Ok(t) => t,
            Err(note) => {
                warn!("trainer: {note}");
                None
            }
        })
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

/// A match is starting: ask the trainer to stop without waiting for it. Joining here blocked the
/// main thread for the rest of its sim round plus a checkpoint save, right as the round began.
fn stop(mut bg: ResMut<BackgroundTrainer>) {
    if let Some(h) = &bg.running {
        h.cancel();
        bg.stopping = true;
    }
}

/// Collect a stopped trainer once its thread has actually finished (the join is then instant).
fn reap(mut bg: ResMut<BackgroundTrainer>, log: Dev) {
    if bg.stopping && bg.running.as_ref().is_some_and(TrainerHandle::is_finished) {
        bg.halt(&log);
    }
}

/// How long quitting waits for the trainer to finish its sim round and save its checkpoint.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// The app is quitting: stop the trainer and give it a moment to write its checkpoint, so the
/// generations since the last periodic checkpoint aren't lost.
fn stop_on_exit(mut exits: MessageReader<AppExit>, mut bg: ResMut<BackgroundTrainer>) {
    if exits.read().next().is_none() {
        return;
    }
    let Some(h) = bg.running.take() else { return };
    bg.stopping = false;
    match h.finish_within(EXIT_WAIT) {
        Ok(Some(t)) => info!("trainer: stopped on exit after {} steps (checkpoint saved)", t.steps),
        Ok(None) => warn!("trainer: the background trainer thread panicked"),
        Err(_) => warn!("trainer: still in a sim round after {} s at exit; keeping the last periodic checkpoint", EXIT_WAIT.as_secs()),
    }
}
