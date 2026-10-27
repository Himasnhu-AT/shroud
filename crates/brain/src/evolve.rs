//! Between-match evolution and the persisted brain (docs/EVOLUTION.md).

use crate::arena::Arena;
use crate::insights::{self, MatchPosterior, SquadCard};
use crate::utility::Goal;
use crate::genome::{GENE_COUNT, GaConfig, Gene, TraitGenome, TraitPopulation};
use crate::ghost::{self, CalibConfig, GhostParams, Outcome, RealStats};
use crate::limits::Limits;
use crate::loadout::{self, TRAINER_DIFFICULTY};
use crate::math::{BrainRng, rng_from_seed};
use crate::weapons::WeaponKind;
use crate::mind::{EnemyMind, NEAT_INPUTS, NEAT_OUTPUTS};
use crate::neat::{self, NeatConfig};
use crate::player_model::{MatchStats, MatchTelemetry, PlayerModel};
use crate::sim::{self, RoundResult};
use rand::Rng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrd};
use std::time::{Duration, Instant};

/// v2: trait genomes gained position-evaluation weights (squad layer); older saves reset.
pub const SAVE_VERSION: u32 = 2;
pub const SQUAD: usize = 5;
pub const NEAT_POP: usize = 40;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SquadSlot {
    pub trait_id: usize,
    pub neat_id: usize,
}

/// Per-enemy results from a real match.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnemyReport {
    pub trait_id: usize,
    pub neat_id: usize,
    pub damage_dealt: f32,
    pub kills: u32,
    pub time_alive: f32,
    pub time_engaged: f32,
    /// Seconds spent in each goal, indexed like `Goal::ALL`.
    #[serde(default)]
    pub goal_time: [f32; Goal::COUNT],
    /// Kills on the player by the goal the enemy was in, indexed like `Goal::ALL`.
    #[serde(default)]
    pub kills_by_goal: [u32; Goal::COUNT],
}

/// Index of a goal in `Goal::ALL` (for `EnemyReport::goal_time` / `kills_by_goal`).
pub fn goal_index(g: Goal) -> usize {
    Goal::ALL.iter().position(|&x| x == g).unwrap_or(0)
}

impl EnemyReport {
    /// Add another report's counters (same genomes across rounds).
    pub fn absorb(&mut self, r: &EnemyReport) {
        self.damage_dealt += r.damage_dealt;
        self.kills += r.kills;
        self.time_alive += r.time_alive;
        self.time_engaged += r.time_engaged;
        for k in 0..Goal::COUNT {
            self.goal_time[k] += r.goal_time[k];
            self.kills_by_goal[k] += r.kills_by_goal[k];
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MatchReport {
    pub telemetry: MatchTelemetry,
    pub enemies: Vec<EnemyReport>,
    pub player_won: bool,
    pub player_rounds: u32,
    pub enemy_rounds: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchSummary {
    pub generation: u32,
    pub player_won: bool,
    pub player_rounds: u32,
    pub enemy_rounds: u32,
    pub sim_win_rate: f32,
    pub avg_fitness: f32,
    #[serde(default)]
    pub stats: MatchStats,
    /// Sim-vs-real for the squad that played this match: its sim win rate when it was
    /// deployed, and the share of real rounds it won.
    #[serde(default)]
    pub deployed_sim_win: Option<f32>,
    #[serde(default)]
    pub real_win: Option<f32>,
    /// Total simulated rounds behind this generation.
    #[serde(default)]
    pub sim_rounds: u64,
    /// Per-match sufficient statistics for gated trends (insights v2).
    #[serde(default)]
    pub posterior: MatchPosterior,
    /// The squad deployed in this match.
    #[serde(default)]
    pub squad: Vec<SquadSlot>,
    /// Outcome gap Δ_g: ghost player-win rate vs the deployed squad minus the real one (before calibrating).
    #[serde(default)]
    pub outcome_gap: Option<f32>,
    /// Ghost calibration error after this match's calibration (see `ghost::calibration_error`).
    #[serde(default)]
    pub calibration_error: Option<f32>,
}

/// A deployed squad's genomes and its real result, kept for ghost calibration (paper §4.1.1).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplaySquad {
    pub generation: u32,
    pub traits: Vec<TraitGenome>,
    pub neat: Vec<neat::Genome>,
    pub player_rounds: u32,
    pub rounds: u32,
}

/// How many past deployed squads calibration replays.
pub const REPLAY_SQUADS: usize = 4;
/// Wall-clock cap for ghost calibration (checked between ES iterations, so ~1.5 s worst case).
pub const CALIB_BUDGET: Duration = Duration::from_millis(1300);

#[derive(Clone, Debug)]
pub struct TraitDelta {
    pub label: &'static str,
    pub before: f32,
    pub after: f32,
}

#[derive(Clone, Debug)]
pub struct EvolveSummary {
    pub generation: u32,
    pub insights: Vec<String>,
    /// "What they learned -> what they did -> did it work" (None before any real match).
    pub card: Option<SquadCard>,
    pub deltas: Vec<TraitDelta>,
    /// Share of sim rounds the squad won against the ghost in the last trained generation.
    pub sim_win_rate: f32,
    /// Ghost calibration: real player round-win rate vs the ghost's against the same squad.
    pub real_player_win: Option<f32>,
    pub ghost_player_win: Option<f32>,
    /// Legacy scalar (aim-skill relative to the default ghost), kept for compatibility.
    pub ghost_skill: f32,
    /// Ghost calibration error after calibrating (RMS of normalised residuals; 0 = perfect).
    pub calibration_error: Option<f32>,
    /// Calibration error before this match's calibration.
    pub calibration_error_before: Option<f32>,
    /// Δ_g for this match, and mean |Δ| over the last 3 matches (milestone: < 0.15).
    pub outcome_gap: Option<f32>,
    pub outcome_gap_mean3: Option<f32>,
    pub neat_generations: u32,
    pub elapsed_ms: u128,
    /// Simulated rounds in this evolve call / in the brain's lifetime.
    pub sim_rounds: u64,
    pub total_sim_rounds: u64,
    /// Sim-vs-real for the squad that just played (see `MatchSummary`).
    pub deployed_sim_win: Option<f32>,
    pub real_win: Option<f32>,
    /// Why the next squad carries what it carries ("They brought more marksmen because you
    /// fight at 35 m"), when the loadout changed.
    pub loadout_note: Option<String>,
    /// The next squad's primaries (one per evolved slot, as the trainer equips them).
    pub next_loadout: Vec<WeaponKind>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrainState {
    pub version: u32,
    pub generation: u32,
    pub seed: u64,
    pub traits: TraitPopulation,
    pub neat: neat::Population,
    pub player_model: PlayerModel,
    pub squad: Vec<SquadSlot>,
    pub evaluated: bool,
    pub history: Vec<MatchSummary>,
    /// Best whole squad so far; competes in every generation so selection can't regress.
    #[serde(default)]
    pub champion: Option<Champion>,
    /// Simulated rounds spent training this brain.
    #[serde(default)]
    pub sim_rounds: u64,
    /// Recent deployed squads with real results (ghost calibration data).
    #[serde(default)]
    pub replay: Vec<ReplaySquad>,
    /// Learned play selection (contextual bandit), updated during real rounds.
    #[serde(default)]
    pub tactics: crate::bandit::TacticsLearner,
}

/// A whole squad (one trait + one NEAT genome per slot), kept outside the populations so
/// it survives breeding.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Champion {
    pub traits: Vec<TraitGenome>,
    pub neat: Vec<neat::Genome>,
    pub score: SquadScore,
    /// Where it sits in the current populations, if it does (valid until they breed).
    #[serde(default)]
    pub slots: Option<Vec<SquadSlot>>,
}

/// A candidate squad's result in one generation. Selection order: survived more halving
/// rungs, then win probability, then mean shaping reward (tiebreak).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SquadScore {
    pub wins: u32,
    pub rounds: u32,
    /// Sum over rounds of the squad-mean shaping reward (rho-bar).
    pub shaping: f32,
    pub rung: u32,
}

impl SquadScore {
    pub fn win_rate(&self) -> f32 {
        self.wins as f32 / self.rounds.max(1) as f32
    }
    pub fn mean_shaping(&self) -> f32 {
        self.shaping / self.rounds.max(1) as f32
    }
    /// `Greater` = better.
    pub fn compare(&self, o: &Self) -> Ordering {
        self.rung
            .cmp(&o.rung)
            .then(self.win_rate().total_cmp(&o.win_rate()))
            .then(self.mean_shaping().total_cmp(&o.mean_shaping()))
    }
    /// Scalar genome fitness: win probability dominates; shaping is worth a few rounds at most.
    pub fn fitness(&self, mean_reward: f32) -> f32 {
        100.0 * self.win_rate() + 0.1 * mean_reward
    }
}

/// Successive halving: every candidate plays `base_rounds`, the better half plays on until it
/// has twice as many, and so on for `rungs` rungs.
#[derive(Clone, Copy, Debug)]
pub struct Halving {
    pub base_rounds: usize,
    pub rungs: usize,
}

/// One simulated round from a candidate's point of view.
pub struct RoundOutcome<T> {
    pub won: bool,
    pub shaping: f32,
    pub detail: T,
}

/// Per-candidate scores and round details, plus the ranking (best first).
pub struct HalvingResult<T> {
    pub scores: Vec<SquadScore>,
    pub details: Vec<Vec<T>>,
    pub ranking: Vec<usize>,
}

/// Seed of round `r` in a generation. Every candidate plays the same seeds (common random numbers).
pub fn round_seed(gen_seed: u64, r: usize) -> u64 {
    let mut z = gen_seed.wrapping_add((r as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Successive halving over `n` candidates. `play(candidate, round_index)` must derive all its
/// randomness from `round_index` so that candidates are compared on identical rounds.
/// Returns `None` if cancelled.
pub fn successive_halving<T: Send>(
    n: usize,
    plan: Halving,
    par: &Parallel,
    play: impl Fn(usize, usize) -> RoundOutcome<T> + Sync,
) -> Option<HalvingResult<T>> {
    let mut scores = vec![SquadScore::default(); n];
    let mut details: Vec<Vec<T>> = (0..n).map(|_| Vec::new()).collect();
    let mut alive: Vec<usize> = (0..n).collect();
    let (mut done, mut target) = (0, plan.base_rounds.max(1));
    let rungs = plan.rungs.max(1);
    for rung in 0..rungs {
        let jobs: Vec<(usize, usize)> = alive.iter().flat_map(|&c| (done..target).map(move |r| (c, r))).collect();
        let out = par.run(&jobs, |&(c, r)| play(c, r))?;
        for ((c, _), o) in jobs.into_iter().zip(out) {
            let s = &mut scores[c];
            s.rounds += 1;
            s.wins += u32::from(o.won);
            s.shaping += o.shaping;
            s.rung = rung as u32;
            details[c].push(o.detail);
        }
        if alive.len() <= 1 || rung + 1 == rungs {
            break;
        }
        alive.sort_by(|&a, &b| scores[b].compare(&scores[a]).then(a.cmp(&b)));
        alive.truncate(alive.len().div_ceil(2));
        done = target;
        target *= 2;
    }
    let mut ranking: Vec<usize> = (0..n).collect();
    ranking.sort_by(|&a, &b| scores[b].compare(&scores[a]).then(a.cmp(&b)));
    Some(HalvingResult { scores, details, ranking })
}

/// How sim work is spread over threads, and how it is stopped.
#[derive(Clone, Copy, Default)]
pub struct Parallel<'a> {
    pub threads: usize,
    pub cancel: Option<&'a AtomicBool>,
    /// Called at the start of each worker thread (e.g. to lower its priority).
    pub worker_init: Option<fn()>,
}

impl Parallel<'_> {
    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(AtomicOrd::Relaxed))
    }

    /// Run `f` over `items` on up to `threads` scoped threads, keeping order. `None` if cancelled.
    pub fn run<T: Sync, R: Send>(&self, items: &[T], f: impl Fn(&T) -> R + Sync) -> Option<Vec<R>> {
        let threads = self.threads.max(1).min(items.len().max(1));
        let chunk = items.len().div_ceil(threads).max(1);
        let work = |c: &[T]| -> Option<Vec<R>> {
            if let Some(init) = self.worker_init {
                init();
            }
            c.iter().map(|x| (!self.cancelled()).then(|| f(x))).collect()
        };
        let parts: Vec<Option<Vec<R>>> = std::thread::scope(|s| {
            let handles: Vec<_> = items.chunks(chunk).map(|c| s.spawn(|| work(c))).collect();
            handles.into_iter().map(|h| h.join().expect("sim thread panicked")).collect()
        });
        let mut out = Vec::with_capacity(items.len());
        for p in parts {
            out.extend(p?);
        }
        Some(out)
    }
}

/// What one training generation produced.
#[derive(Clone, Copy, Debug)]
pub struct GenReport {
    pub rounds: u64,
    /// The champion's score this generation (on the shared seeds).
    pub champion: SquadScore,
    /// Win rate over every round played this generation.
    pub mean_win: f32,
}

/// Team-spirit weight tau, annealed from 0.3 to 1 over a training session (`frac` in 0..=1).
pub fn team_spirit(frac: f32) -> f32 {
    0.3 + 0.7 * frac.clamp(0.0, 1.0)
}

#[derive(Clone, Debug)]
pub struct EvolveConfig {
    /// Wall-clock budget for training.
    pub budget: Duration,
    pub max_neat_generations: u32,
    /// Rounds every candidate squad plays in the first successive-halving rung.
    pub rounds_per_genome: usize,
    /// Candidate squads built from the populations each generation (plus the champion).
    pub squads: usize,
    /// Successive-halving rungs (survivors double their rounds each rung).
    pub rungs: usize,
    pub threads: usize,
}

impl Default for EvolveConfig {
    fn default() -> Self {
        EvolveConfig {
            budget: Duration::from_secs(8),
            max_neat_generations: 40,
            rounds_per_genome: 3,
            squads: 8,
            rungs: 3,
            threads: std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 6),
        }
    }
}

fn squad_avg(pop: &TraitPopulation, squad: &[SquadSlot]) -> [f32; GENE_COUNT] {
    let mut avg = [0.0; GENE_COUNT];
    for s in squad {
        for (a, g) in avg.iter_mut().zip(pop.genomes[s.trait_id].genes) {
            *a += g / squad.len().max(1) as f32;
        }
    }
    avg
}

impl BrainState {
    pub fn new(seed: u64) -> Self {
        let mut rng = rng_from_seed(seed);
        let traits = TraitPopulation::new(GaConfig::default(), &mut rng);
        let neat = neat::Population::new(NEAT_INPUTS, NEAT_OUTPUTS, NEAT_POP, NeatConfig::default(), &mut rng);
        let squad = (0..SQUAD).map(|i| SquadSlot { trait_id: i, neat_id: i }).collect();
        BrainState {
            version: SAVE_VERSION,
            generation: 0,
            seed,
            traits,
            neat,
            player_model: PlayerModel::default(),
            squad,
            evaluated: false,
            history: Vec::new(),
            champion: None,
            sim_rounds: 0,
            replay: Vec::new(),
            tactics: crate::bandit::TacticsLearner::default(),
        }
    }

    /// Take over populations trained in the background from a copy of this brain. Only valid
    /// between matches, and only if the copy started from this generation. The player model
    /// and history stay ours.
    pub fn adopt(&mut self, trained: BrainState) -> bool {
        if trained.seed != self.seed || trained.generation != self.generation || !trained.evaluated {
            return false;
        }
        self.traits = trained.traits;
        self.neat = trained.neat;
        self.champion = trained.champion;
        self.sim_rounds = self.sim_rounds.max(trained.sim_rounds);
        self.evaluated = true;
        self.migrate();
        self.deploy_champion();
        true
    }

    /// Make the champion the deployed squad. If it isn't in the populations, write it over the
    /// weakest genomes (with the champion's fitness, so it breeds).
    pub fn deploy_champion(&mut self) {
        let Some(ch) = &mut self.champion else { return };
        if ch.slots.is_none() {
            let fit = ch.score.fitness(ch.score.mean_shaping());
            let mut weak_t = self.traits.ranked();
            weak_t.reverse();
            let mut weak_n: Vec<usize> = (0..self.neat.genomes.len()).collect();
            weak_n.sort_by(|&a, &b| self.neat.genomes[a].fitness.total_cmp(&self.neat.genomes[b].fitness));
            let mut slots = Vec::new();
            for (k, (tg, ng)) in ch.traits.iter().zip(&ch.neat).enumerate() {
                let (t, n) = (weak_t[k % weak_t.len()], weak_n[k % weak_n.len()]);
                self.traits.genomes[t] = TraitGenome { fitness: fit, ..tg.clone() };
                let mut ng = ng.clone();
                ng.fitness = fit;
                self.neat.genomes[n] = ng;
                slots.push(SquadSlot { trait_id: t, neat_id: n });
            }
            ch.slots = Some(slots);
        }
        if let Some(s) = &ch.slots {
            self.squad = s.clone();
        }
    }

    /// First launch: train against the default ghost so generation 1 is already competent.
    /// Doesn't count as a generation and leaves no history.
    pub fn bootstrap(&mut self, arena: &Arena, cfg: &EvolveConfig) {
        if self.evaluated || self.generation > 0 {
            return;
        }
        self.evolve(&MatchReport::default(), arena, cfg, |_| {});
        self.generation = 0;
        self.history.clear();
        self.player_model = PlayerModel::default();
    }

    /// Average reaction time of the current squad (ms).
    pub fn squad_reaction_ms(&self) -> f32 {
        let l = self.limits();
        let n = self.squad.len().max(1) as f32;
        self.squad.iter().map(|s| self.traits.genomes[s.trait_id].decode(&l).reaction_time * 1000.0).sum::<f32>() / n
    }

    pub fn limits(&self) -> Limits {
        Limits::for_generation(self.generation)
    }

    /// Build the minds for the current squad (one per enemy spawn).
    pub fn squad_minds(&self, seed: u64) -> Vec<EnemyMind> {
        self.squad_minds_for(seed, self.squad.len(), &self.limits())
    }

    /// `n` minds under `limits` (e.g. `Difficulty::limits`), equipped as the trainer does.
    pub fn squad_minds_for(&self, seed: u64, n: usize, limits: &Limits) -> Vec<EnemyMind> {
        self.squad_minds_on(seed, n, limits, TRAINER_DIFFICULTY)
    }

    /// `n` minds under `limits`, guns handed out under `difficulty`'s caps. With more enemies
    /// than squad slots, slots are reused (same genomes, their own seeds and minds).
    pub fn squad_minds_on(&self, seed: u64, n: usize, limits: &Limits, difficulty: crate::difficulty::Difficulty) -> Vec<EnemyMind> {
        let mut minds = self.squad_minds_raw(seed, n, limits);
        loadout::equip(&mut minds, difficulty);
        minds
    }

    /// The current squad's primaries as the trainer equips them.
    pub fn squad_loadout(&self) -> Vec<WeaponKind> {
        let l = self.limits();
        self.squad_minds_for(0, self.squad.len(), &l).iter().map(EnemyMind::weapon).collect()
    }

    /// Fill weapon genes missing from an old save (they get their seeded preferences).
    pub fn migrate(&mut self) {
        self.traits.genomes.iter_mut().for_each(TraitGenome::fill_loadout);
        if let Some(c) = &mut self.champion {
            c.traits.iter_mut().for_each(TraitGenome::fill_loadout);
        }
        for r in &mut self.replay {
            r.traits.iter_mut().for_each(TraitGenome::fill_loadout);
        }
    }

    fn squad_minds_raw(&self, seed: u64, n: usize, limits: &Limits) -> Vec<EnemyMind> {
        if self.squad.is_empty() {
            return Vec::new();
        }
        (0..n)
            .map(|k| {
                let s = self.squad[k % self.squad.len()];
                EnemyMind::new(
                    &self.traits.genomes[s.trait_id],
                    s.trait_id,
                    &self.neat.genomes[s.neat_id],
                    s.neat_id,
                    limits,
                    seed.wrapping_add(k as u64 * 7919),
                )
            })
            .collect()
    }

    /// The whole between-match pipeline. `progress` gets 0..=1.
    pub fn evolve(&mut self, report: &MatchReport, arena: &Arena, cfg: &EvolveConfig, progress: impl FnMut(f32)) -> EvolveSummary {
        self.evolve_merged(report, None, arena, cfg, progress)
    }

    /// `evolve`, first adopting populations a background `Trainer` built from this brain
    /// (after the real-match steps, which refer to the squad that actually played).
    pub fn evolve_merged(
        &mut self,
        report: &MatchReport,
        trained: Option<BrainState>,
        arena: &Arena,
        cfg: &EvolveConfig,
        mut progress: impl FnMut(f32),
    ) -> EvolveSummary {
        let start = Instant::now();
        let rounds_before = self.sim_rounds;
        let deployed_sim_win = self.champion.as_ref().map(|c| c.score.win_rate());
        let loadout_before = self.squad_loadout();
        let mut rng = rng_from_seed(self.seed ^ (self.generation as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
        let before = squad_avg(&self.traits, &self.squad);

        // 1. Learn the player (keeping the model the squad was trained against, for the card).
        let model_before = self.player_model.clone();
        self.player_model.ingest(&report.telemetry);
        // The play-selection bandit forgets a little each match, so it keeps adapting.
        if report.player_rounds + report.enemy_rounds > 0 {
            self.tactics.forget();
        }

        // 1b. Calibrate the ghost (paper §4.1.1): replay recently deployed squads against it and fit
        // the Ghost v2 parameters to their real outcomes plus this match's TTK, round length and
        // accuracy by range.
        let deployed = self.squad.clone();
        let rounds = report.player_rounds + report.enemy_rounds;
        let (mut real_player_win, mut ghost_player_win, mut outcome_gap) = (None, None, None);
        let (mut calibration_error, mut calibration_error_before) = (None, None);
        if rounds > 0 && self.evaluated {
            self.replay.push(ReplaySquad {
                generation: self.generation,
                traits: deployed.iter().map(|s| self.traits.genomes[s.trait_id].clone()).collect(),
                neat: deployed.iter().map(|s| self.neat.genomes[s.neat_id].clone()).collect(),
                player_rounds: report.player_rounds,
                rounds,
            });
            let drop = self.replay.len().saturating_sub(REPLAY_SQUADS);
            self.replay.drain(..drop);
            let outcomes: Vec<Outcome> = self.replay.iter().map(|r| Outcome { player_rounds: r.player_rounds as f32, rounds: r.rounds as f32 }).collect();
            let real_stats = RealStats::from_telemetry(&report.telemetry, rounds);
            let replay = &self.replay;
            let make = |g: usize, seed: u64| {
                let r = &replay[g];
                let limits = Limits::for_generation(r.generation);
                let mut minds: Vec<EnemyMind> = r
                    .traits
                    .iter()
                    .zip(&r.neat)
                    .enumerate()
                    .map(|(k, (tg, ng))| EnemyMind::new(tg, k, ng, k, &limits, seed.wrapping_add(k as u64 * 7919)))
                    .collect();
                loadout::equip(&mut minds, TRAINER_DIFFICULTY);
                (minds, limits)
            };
            let ccfg = CalibConfig {
                threads: cfg.threads,
                seed: self.seed ^ self.generation as u64,
                deadline: Some(Instant::now() + CALIB_BUDGET),
                ..CalibConfig::default()
            };
            let res = ghost::calibrate(arena, &self.player_model, &outcomes, &real_stats, &ccfg, &make);
            let real = report.player_rounds as f32 / rounds as f32;
            let sim_rate = *res.win_before.last().unwrap_or(&0.0);
            real_player_win = Some(real);
            ghost_player_win = Some(sim_rate);
            outcome_gap = Some(sim_rate - real);
            calibration_error = Some(res.error_after);
            calibration_error_before = Some(res.error_before);
            let m = &mut self.player_model;
            m.ghost = res.params;
            m.ghost_skill = (GhostParams::default().aim_sigma_deg / res.params.aim_sigma_deg.max(0.1)).clamp(0.5, 3.0);
        }
        let real_win = (rounds > 0).then(|| report.enemy_rounds as f32 / rounds as f32);

        // 2. Real-match fitness bonus for the deployed genomes (blended with their sim fitness).
        if self.evaluated {
            for e in &report.enemies {
                let real = sim::EnemyStats {
                    damage_dealt: e.damage_dealt,
                    kills: e.kills,
                    time_alive: e.time_alive,
                    time_engaged: e.time_engaged,
                    ..Default::default()
                }
                .fitness();
                if let Some(g) = self.traits.genomes.get_mut(e.trait_id) {
                    g.fitness = 0.6 * g.fitness + 0.4 * real;
                }
                if let Some(g) = self.neat.genomes.get_mut(e.neat_id) {
                    g.fitness = 0.6 * g.fitness + 0.4 * real;
                }
            }
        }
        // 3. Take over background training, if any (its populations replace ours).
        if let Some(t) = trained {
            self.adopt(t);
        }

        // 4. Train against the ghost: whole squads, common random numbers, successive halving.
        //    (Both populations breed every generation inside `train_generation`.)
        let mut sim_win_rate = self.champion.as_ref().map_or(0.0, |c| c.score.win_rate());
        let mut gens = 0;
        let limits = Limits::for_generation(self.generation + 1);
        let par = Parallel { threads: cfg.threads, ..Default::default() };
        while gens < cfg.max_neat_generations && (gens == 0 || start.elapsed() < cfg.budget) {
            let frac = (gens as f32 / cfg.max_neat_generations as f32).max(start.elapsed().as_secs_f32() / cfg.budget.as_secs_f32());
            if let Some(r) = self.train_generation(arena, limits, cfg, team_spirit(frac), &par, &mut rng) {
                sim_win_rate = r.champion.win_rate();
            }
            gens += 1;
            progress((start.elapsed().as_secs_f32() / cfg.budget.as_secs_f32()).min(0.95).max(gens as f32 / cfg.max_neat_generations as f32 * 0.95));
        }

        // 5. Deploy the champion squad.
        self.generation += 1;
        self.deploy_champion();

        let after = squad_avg(&self.traits, &self.squad);
        let mut deltas: Vec<TraitDelta> = Gene::ALL
            .iter()
            .map(|&g| TraitDelta { label: g.label(), before: before[g as usize], after: after[g as usize] })
            .collect();
        deltas.sort_by(|a, b| (b.after - b.before).abs().total_cmp(&(a.after - a.before).abs()));
        deltas.truncate(4);

        let avg_fitness = self.squad.iter().map(|s| self.traits.genomes[s.trait_id].fitness).sum::<f32>() / SQUAD as f32;
        self.history.push(MatchSummary {
            generation: self.generation,
            player_won: report.player_won,
            player_rounds: report.player_rounds,
            enemy_rounds: report.enemy_rounds,
            sim_win_rate,
            avg_fitness,
            stats: MatchStats::from_telemetry(&report.telemetry),
            deployed_sim_win,
            real_win,
            sim_rounds: self.sim_rounds,
            posterior: MatchPosterior::from_telemetry(&report.telemetry),
            squad: deployed,
            outcome_gap,
            calibration_error,
        });
        let gaps: Vec<f32> = self.history.iter().rev().take(3).filter_map(|h| h.outcome_gap).collect();
        let outcome_gap_mean3 = (!gaps.is_empty()).then(|| gaps.iter().map(|g| g.abs()).sum::<f32>() / gaps.len() as f32);
        let prev = self.history.len().checked_sub(2).map(|i| self.history[i].stats.clone());
        let posts: Vec<MatchPosterior> = self.history.iter().map(|h| h.posterior.clone()).collect();
        let top = insights::insights_v2(&self.player_model, arena, &posts).into_iter().next();
        let card = (rounds > 0).then(|| {
            insights::squad_card(&model_before, arena, &report.telemetry, &report.enemies, report.player_rounds, report.enemy_rounds, top.as_ref())
        });
        let next_loadout = self.squad_loadout();
        let loadout_note = loadout_note(&loadout_before, &next_loadout, &self.player_model);
        progress(1.0);
        EvolveSummary {
            generation: self.generation,
            insights: self.player_model.insights_with_history(arena, Some(&report.telemetry), prev.as_ref(), &posts, self.squad_reaction_ms()),
            card,
            deltas,
            sim_win_rate,
            real_player_win,
            ghost_player_win,
            ghost_skill: self.player_model.ghost_skill,
            calibration_error,
            calibration_error_before,
            outcome_gap,
            outcome_gap_mean3,
            neat_generations: gens,
            elapsed_ms: start.elapsed().as_millis(),
            sim_rounds: self.sim_rounds - rounds_before,
            total_sim_rounds: self.sim_rounds,
            deployed_sim_win,
            real_win,
            loadout_note,
            next_loadout,
        }
    }

    /// One training generation (paper section 4.3): breed both populations (once evaluated),
    /// build `cfg.squads` whole squads plus the champion, score them on shared seeds with
    /// successive halving, and set genome fitness from squad win rate plus the team-spirit
    /// reward r_i = (1 - tau) rho_i + tau rho_bar. Returns `None` if cancelled, in which case the
    /// populations may be bred but unscored: callers that cancel should work on a copy.
    pub fn train_generation(&mut self, arena: &Arena, limits: Limits, cfg: &EvolveConfig, tau: f32, par: &Parallel, rng: &mut BrainRng) -> Option<GenReport> {
        if self.evaluated {
            self.neat.evolve(rng);
            self.traits.evolve(rng);
            if let Some(c) = &mut self.champion {
                c.slots = None; // the populations changed under it
            }
        }
        // Fixed pairing for the whole generation: position k of a shuffled NEAT order goes with
        // position k of a shuffled trait order, and consecutive positions form a squad.
        let mut on: Vec<usize> = (0..self.neat.genomes.len()).collect();
        let mut ot: Vec<usize> = (0..self.traits.genomes.len()).collect();
        on.shuffle(rng);
        ot.shuffle(rng);
        let mut squads: Vec<Vec<SquadSlot>> = (0..cfg.squads.max(1))
            .map(|k| (0..SQUAD).map(|j| SquadSlot { trait_id: ot[(k * SQUAD + j) % ot.len()], neat_id: on[(k * SQUAD + j) % on.len()] }).collect())
            .collect();
        let pop_n = squads.len();
        let champ = self.champion.clone().filter(|c| c.traits.len() == SQUAD && c.neat.len() == SQUAD);
        if champ.is_some() {
            squads.push(Vec::new()); // the champion plays with its own genomes
        }
        let gen_seed: u64 = rng.random();
        let plan = Halving { base_rounds: cfg.rounds_per_genome, rungs: cfg.rungs };
        let res = successive_halving(squads.len(), plan, par, |c, r| {
            let seed = round_seed(gen_seed, r);
            let mut minds: Vec<EnemyMind> = (0..SQUAD)
                .map(|j| match (&champ, c >= pop_n) {
                    (Some(ch), true) => EnemyMind::new(&ch.traits[j], 0, &ch.neat[j], 0, &limits, seed ^ j as u64),
                    _ => {
                        let s = squads[c][j];
                        EnemyMind::new(&self.traits.genomes[s.trait_id], s.trait_id, &self.neat.genomes[s.neat_id], s.neat_id, &limits, seed ^ j as u64)
                    }
                })
                .collect();
            // Guns from the weapon genes, as the game hands them out.
            loadout::equip(&mut minds, TRAINER_DIFFICULTY);
            let rr: RoundResult = sim::run_round(arena, &self.player_model, limits, minds, seed);
            let rho: Vec<f32> = rr.enemies.iter().map(sim::EnemyStats::fitness).collect();
            let mean = rho.iter().sum::<f32>() / rho.len().max(1) as f32;
            let reward: Vec<f32> = rho.iter().map(|&p| (1.0 - tau) * p + tau * mean).collect();
            RoundOutcome { won: !rr.player_won, shaping: mean, detail: reward }
        })?;

        let mut nf = vec![(0.0f32, 0u32); self.neat.genomes.len()];
        let mut tf = vec![(0.0f32, 0u32); self.traits.genomes.len()];
        for (c, slots) in squads.iter().enumerate().take(pop_n) {
            let rounds = &res.details[c];
            for (j, s) in slots.iter().enumerate() {
                let mean_r = rounds.iter().map(|d| d[j]).sum::<f32>() / rounds.len().max(1) as f32;
                let f = res.scores[c].fitness(mean_r);
                nf[s.neat_id].0 += f;
                nf[s.neat_id].1 += 1;
                tf[s.trait_id].0 += f;
                tf[s.trait_id].1 += 1;
            }
        }
        for (g, (s, n)) in self.neat.genomes.iter_mut().zip(nf) {
            g.fitness = if n > 0 { s / n as f32 } else { 0.0 };
        }
        for (g, (s, n)) in self.traits.genomes.iter_mut().zip(tf) {
            g.fitness = if n > 0 { s / n as f32 } else { 0.0 };
            g.evals = n;
        }

        let best = res.ranking[0];
        self.champion = Some(match champ {
            Some(mut ch) if best >= pop_n => {
                ch.score = res.scores[best];
                ch
            }
            _ => {
                let slots = squads[best].clone();
                Champion {
                    traits: slots.iter().map(|s| self.traits.genomes[s.trait_id].clone()).collect(),
                    neat: slots.iter().map(|s| self.neat.genomes[s.neat_id].clone()).collect(),
                    score: res.scores[best],
                    slots: Some(slots),
                }
            }
        });
        self.evaluated = true;
        let rounds: u64 = res.scores.iter().map(|s| s.rounds as u64).sum();
        let wins: u64 = res.scores.iter().map(|s| s.wins as u64).sum();
        self.sim_rounds += rounds;
        Some(GenReport { rounds, champion: res.scores[best], mean_win: wins as f32 / rounds.max(1) as f32 })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }

    /// Load, or start fresh. A corrupt or old file is kept as `brain.json.bak`.
    pub fn load_or_new(path: &Path, seed: u64) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<BrainState>(&bytes) {
                Ok(mut s) if s.version == SAVE_VERSION => {
                    // Saves from before loadouts load as-is; weapon genes get seeded.
                    s.migrate();
                    s
                }
                _ => {
                    let _ = std::fs::rename(path, path.with_extension("json.bak"));
                    BrainState::new(seed)
                }
            },
            Err(_) => BrainState::new(seed),
        }
    }
}

/// Explain a loadout change for the debrief, from what the player model says about him.
pub fn loadout_note(before: &[WeaponKind], after: &[WeaponKind], model: &PlayerModel) -> Option<String> {
    let (b, a) = (loadout::counts(before), loadout::counts(after));
    let range = model.engagement_range();
    let fav = model.favourite_weapon();
    // The kind that gained the most slots (ties: the most specialised).
    let order = [3usize, 2, 0, 1];
    let gained = order.iter().copied().filter(|&k| a[k] > b[k]).max_by_key(|&k| a[k] - b[k])?;
    let n = a[gained];
    let text = match WeaponKind::PRIMARIES[gained] {
        WeaponKind::Marksman if range >= 20.0 => format!("They brought {} because you fight at {:.0} m+", plural(n, "a marksman", "marksmen"), range),
        WeaponKind::Marksman => format!("They brought {}: your long sightlines get watched now", plural(n, "a marksman", "marksmen")),
        WeaponKind::Shotgun if range < 12.0 => format!("They brought {} because you keep fights inside {:.0} m", plural(n, "a shotgun", "shotguns"), range.max(5.0)),
        WeaponKind::Shotgun => format!("They brought {} to breach the rooms you hold", plural(n, "a shotgun", "shotguns")),
        WeaponKind::Smg if fav == WeaponKind::Marksman => format!("{} to rush you while you're scoped in", plural(n, "An SMG rusher", "SMG rushers")),
        WeaponKind::Smg => format!("{} to flank you up close", plural(n, "An SMG rusher", "SMG rushers")),
        _ => format!("Back to rifles ({}): steady mid-range fire works against you at {:.0} m", n, range),
    };
    Some(text)
}

fn plural(n: u32, one: &str, many: &str) -> String {
    if n == 1 { one.to_string() } else { format!("{n} {many}") }
}

/// Run `f` over `items` on up to `threads` scoped threads, keeping order.
pub(crate) fn run_parallel
<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = threads.max(1).min(items.len().max(1));
    let chunk = items.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = items.chunks(chunk.max(1)).map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("sim thread panicked")).collect()
    })
}
