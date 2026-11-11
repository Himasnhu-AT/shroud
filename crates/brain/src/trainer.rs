//! Background trainer (paper section 4.3, roadmap P1-4): keeps running training generations
//! on a copy of the brain while the player is in menus, in small cancellable steps, with
//! checkpoints. Progress is counted in simulated rounds.
//!
//! ```ignore
//! let handle = TrainerHandle::spawn(Trainer::new(brain.clone()), TrainerConfig::default(), None);
//! // ... later, between matches:
//! let trainer = handle.finish();          // cancel + join (returns within one sim round)
//! brain.adopt(trainer.state);             // or pass it to `evolve_merged`
//! ```

use crate::arena::Arena;
use crate::evolve::{BrainState, EvolveConfig, GenReport, Parallel, checked_json, move_aside, team_spirit, write_atomic};
use crate::limits::Limits;
use crate::math::rng_from_seed;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct TrainerConfig {
    /// Squads, halving rungs and rounds per generation (`budget`/`max_neat_generations` unused).
    pub eval: EvolveConfig,
    /// Sim worker threads.
    pub threads: usize,
    /// Rounds over which team spirit anneals from 0.3 to 1.
    pub anneal_rounds: u64,
    /// Stop on its own after this many rounds in one session.
    pub max_session_rounds: u64,
    /// Save a checkpoint every this many steps (and on exit) when given a path.
    pub checkpoint_every: u64,
    /// Also save a checkpoint when this much time has passed since the last one, so a killed
    /// process loses at most this much training.
    pub checkpoint_interval: Duration,
    /// Run at the start of the trainer thread and of each sim worker (e.g. lower priority).
    pub worker_init: Option<fn()>,
}

impl Default for TrainerConfig {
    fn default() -> Self {
        TrainerConfig {
            eval: EvolveConfig::default(),
            threads: 2,
            anneal_rounds: 50_000,
            max_session_rounds: 500_000,
            checkpoint_every: 25,
            checkpoint_interval: Duration::from_secs(30),
            worker_init: None,
        }
    }
}

/// A training session over a copy of the brain. Serializable, so it can be checkpointed and
/// resumed bit-for-bit (each step's randomness comes from `(seed, step)`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trainer {
    pub state: BrainState,
    /// Generations completed in this session.
    pub steps: u64,
    /// Simulated rounds in this session (drives the team-spirit anneal).
    pub session_rounds: u64,
    pub last: Option<StepStats>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StepStats {
    pub rounds: u64,
    pub champion_win: f32,
    pub champion_rounds: u32,
    pub mean_win: f32,
    pub tau: f32,
}

impl Trainer {
    pub fn new(state: BrainState) -> Self {
        Trainer { state, steps: 0, session_rounds: 0, last: None }
    }

    /// Is this session still a valid continuation of `live`? (Same brain, no match since.)
    pub fn matches(&self, live: &BrainState) -> bool {
        self.state.seed == live.seed && self.state.generation == live.generation
    }

    pub fn tau(&self, cfg: &TrainerConfig) -> f32 {
        team_spirit(self.session_rounds as f32 / cfg.anneal_rounds.max(1) as f32)
    }

    /// One training generation. On cancel returns `None` and leaves the trainer unchanged.
    pub fn step(&mut self, arena: &Arena, cfg: &TrainerConfig, cancel: Option<&AtomicBool>) -> Option<StepStats> {
        let par = Parallel { threads: cfg.threads, cancel, worker_init: cfg.worker_init };
        let tau = self.tau(cfg);
        let mut rng = rng_from_seed(self.state.seed ^ 0x7A11_7EA1 ^ self.state.generation.wrapping_mul(0x9E37) as u64 ^ self.steps.wrapping_mul(0x2545_F491_4F6C_DD1D));
        let limits = Limits::for_generation(self.state.generation + 1);
        let mut next = self.state.clone();
        let GenReport { rounds, champion, mean_win } = next.train_generation(arena, limits, &cfg.eval, tau, &par, &mut rng)?;
        self.state = next;
        self.steps += 1;
        self.session_rounds += rounds;
        let s = StepStats { rounds, champion_win: champion.win_rate(), champion_rounds: champion.rounds, mean_win, tau };
        self.last = Some(s);
        Some(s)
    }

    /// Save atomically, with non-finite floats replaced (see `BrainState::save`).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut t = self.clone();
        t.state.sanitize();
        if let Some(s) = &mut t.last {
            for v in [&mut s.champion_win, &mut s.mean_win, &mut s.tau] {
                if !v.is_finite() {
                    *v = 0.0;
                }
            }
        }
        write_atomic(path, &checked_json(&t)?)
    }

    /// Load a checkpoint. Like `BrainState::load_or_new`, it migrates old saves: weapon genes
    /// get seeded and player-model grids saved at an older map size are reset to the current size.
    /// `None` if it is missing or unusable (see [`load_checked`](Self::load_checked)).
    pub fn load(path: &Path) -> Option<Self> {
        Self::load_checked(path).ok().flatten()
    }

    /// [`load`](Self::load), telling a missing checkpoint (`Ok(None)`) from one that doesn't
    /// parse or fails `BrainState::validate` (`Err`, after moving it aside like a corrupt
    /// `brain.json`).
    pub fn load_checked(path: &Path) -> Result<Option<Self>, String> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("could not read {}: {e}", path.display())),
        };
        let checked = serde_json::from_slice::<Trainer>(&bytes).map_err(|e| e.to_string()).and_then(|mut t| {
            t.state.migrate();
            t.state.player_model.fit_grids();
            t.state.validate().map(|()| t)
        });
        checked.map(Some).map_err(|why| move_aside(path, &bytes, &why))
    }
}

/// Live counters shared with the trainer thread.
#[derive(Default)]
pub struct TrainerProgress {
    pub cancel: AtomicBool,
    /// Simulated rounds in this session so far.
    pub rounds: AtomicU64,
    pub steps: AtomicU64,
    /// Champion win rate x 1000 after the last step.
    pub champion_win_milli: AtomicU32,
}

/// A trainer running on its own thread.
pub struct TrainerHandle {
    pub progress: Arc<TrainerProgress>,
    join: JoinHandle<Trainer>,
}

impl TrainerHandle {
    /// Start training. Runs until cancelled or `max_session_rounds`; saves to `checkpoint`
    /// every `checkpoint_every` steps and on exit.
    pub fn spawn(mut trainer: Trainer, cfg: TrainerConfig, checkpoint: Option<PathBuf>) -> std::io::Result<Self> {
        let progress = Arc::new(TrainerProgress::default());
        progress.rounds.store(trainer.session_rounds, Ordering::Relaxed);
        let p = progress.clone();
        let join = std::thread::Builder::new().name("brain-trainer".into()).spawn(move || {
            if let Some(init) = cfg.worker_init {
                init();
            }
            // Train on every map in turn, so squads don't overfit Foundry's layout.
            let arenas: Vec<Arena> = crate::arena::MapId::ALL.iter().map(|&m| Arena::build(m)).collect();
            let mut saved_at = Instant::now();
            while !p.cancel.load(Ordering::Relaxed) && trainer.session_rounds < cfg.max_session_rounds {
                let arena = &arenas[(trainer.steps as usize) % arenas.len()];
                let Some(s) = trainer.step(arena, &cfg, Some(&p.cancel)) else { break };
                p.rounds.store(trainer.session_rounds, Ordering::Relaxed);
                p.steps.store(trainer.steps, Ordering::Relaxed);
                p.champion_win_milli.store((s.champion_win * 1000.0) as u32, Ordering::Relaxed);
                if let Some(path) = &checkpoint
                    && (trainer.steps.is_multiple_of(cfg.checkpoint_every.max(1)) || saved_at.elapsed() >= cfg.checkpoint_interval)
                {
                    let _ = trainer.save(path);
                    saved_at = Instant::now();
                }
            }
            if let Some(path) = &checkpoint {
                let _ = trainer.save(path);
            }
            trainer
        })?;
        Ok(TrainerHandle { progress, join })
    }

    /// Ask the thread to stop (non-blocking; it stops within about one sim round).
    pub fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    pub fn rounds(&self) -> u64 {
        self.progress.rounds.load(Ordering::Relaxed)
    }

    /// Cancel and wait for the trainer. `None` if the thread panicked.
    pub fn finish(self) -> Option<Trainer> {
        self.cancel();
        self.join.join().ok()
    }

    /// Cancel and wait at most `timeout` for the trainer (e.g. on app exit, where blocking for
    /// a whole sim round is too long). `Err(self)` if it is still running; it still saves its
    /// checkpoint when it does stop, unless the process exits first.
    ///
    /// ```ignore
    /// match handle.finish_within(Duration::from_secs(2)) {
    ///     Ok(Some(trainer)) => { /* stopped and checkpointed */ }
    ///     Ok(None) => { /* the thread panicked */ }
    ///     Err(_still_running) => { /* gave up waiting */ }
    /// }
    /// ```
    pub fn finish_within(self, timeout: Duration) -> Result<Option<Trainer>, Self> {
        self.cancel();
        let until = Instant::now() + timeout;
        while !self.join.is_finished() {
            if Instant::now() >= until {
                return Err(self);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(self.finish())
    }
}
