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
use crate::evolve::{BrainState, EvolveConfig, GenReport, Parallel, team_spirit};
use crate::limits::Limits;
use crate::math::rng_from_seed;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread::JoinHandle;

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

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
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
            while !p.cancel.load(Ordering::Relaxed) && trainer.session_rounds < cfg.max_session_rounds {
                let arena = &arenas[(trainer.steps as usize) % arenas.len()];
                let Some(s) = trainer.step(arena, &cfg, Some(&p.cancel)) else { break };
                p.rounds.store(trainer.session_rounds, Ordering::Relaxed);
                p.steps.store(trainer.steps, Ordering::Relaxed);
                p.champion_win_milli.store((s.champion_win * 1000.0) as u32, Ordering::Relaxed);
                if let Some(path) = &checkpoint
                    && trainer.steps.is_multiple_of(cfg.checkpoint_every.max(1))
                {
                    let _ = trainer.save(path);
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
}
