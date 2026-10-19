//! Between-match evolution and the persisted brain (docs/EVOLUTION.md).

use crate::arena::Arena;
use crate::insights::{self, MatchPosterior, SquadCard};
use crate::utility::Goal;
use crate::genome::{GENE_COUNT, GaConfig, Gene, TraitGenome, TraitPopulation};
use crate::ghost::{self, CalibConfig, GhostParams, Outcome, RealStats};
use crate::limits::Limits;
use crate::math::{BrainRng, rng_from_seed};
use crate::mind::{EnemyMind, NEAT_INPUTS, NEAT_OUTPUTS};
use crate::neat::{self, NeatConfig};
use crate::player_model::{MatchStats, MatchTelemetry, PlayerModel};
use crate::sim::{self, RoundResult};
use rand::Rng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::path::Path;
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
    pub goal_time: [f32; 7],
    /// Kills on the player by the goal the enemy was in, indexed like `Goal::ALL`.
    #[serde(default)]
    pub kills_by_goal: [u32; 7],
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
        for k in 0..7 {
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
    /// Recent deployed squads with real results (ghost calibration data).
    #[serde(default)]
    pub replay: Vec<ReplaySquad>,
}

#[derive(Clone, Debug)]
pub struct EvolveConfig {
    /// Wall-clock budget for training.
    pub budget: Duration,
    pub max_neat_generations: u32,
    /// Rounds each NEAT genome plays per generation.
    pub rounds_per_genome: usize,
    pub threads: usize,
}

impl Default for EvolveConfig {
    fn default() -> Self {
        EvolveConfig {
            budget: Duration::from_secs(8),
            max_neat_generations: 40,
            rounds_per_genome: 3,
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
            replay: Vec::new(),
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
        let limits = self.limits();
        self.squad
            .iter()
            .enumerate()
            .map(|(k, s)| {
                EnemyMind::new(
                    &self.traits.genomes[s.trait_id],
                    s.trait_id,
                    &self.neat.genomes[s.neat_id],
                    s.neat_id,
                    &limits,
                    seed.wrapping_add(k as u64 * 7919),
                )
            })
            .collect()
    }

    /// The whole between-match pipeline. `progress` gets 0..=1.
    pub fn evolve(&mut self, report: &MatchReport, arena: &Arena, cfg: &EvolveConfig, mut progress: impl FnMut(f32)) -> EvolveSummary {
        let start = Instant::now();
        let mut rng = rng_from_seed(self.seed ^ (self.generation as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
        let before = squad_avg(&self.traits, &self.squad);

        // 1. Learn the player (keeping the model the squad was trained against, for the card).
        let model_before = self.player_model.clone();
        self.player_model.ingest(&report.telemetry);

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
                let minds = r
                    .traits
                    .iter()
                    .zip(&r.neat)
                    .enumerate()
                    .map(|(k, (tg, ng))| EnemyMind::new(tg, k, ng, k, &limits, seed.wrapping_add(k as u64 * 7919)))
                    .collect();
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

        // 2. Real-match fitness bonus for the deployed genomes (blended with their sim fitness).
        if self.evaluated {
            for e in &report.enemies {
                let real = sim::EnemyStats {
                    damage_dealt: e.damage_dealt,
                    kills: e.kills,
                    time_alive: e.time_alive,
                    time_engaged: e.time_engaged,
                    damage_taken: 0.0,
                }
                .fitness();
                if let Some(g) = self.traits.genomes.get_mut(e.trait_id) {
                    g.fitness = 0.6 * g.fitness + 0.4 * real;
                }
                if let Some(g) = self.neat.genomes.get_mut(e.neat_id) {
                    g.fitness = 0.6 * g.fitness + 0.4 * real;
                }
            }
            // 3. Breed new trait genomes.
            self.traits.evolve(&mut rng);
        }

        // 4. Train against the ghost.
        let mut sim_win_rate = 0.0;
        let mut gens = 0;
        let limits = Limits::for_generation(self.generation + 1);
        while gens < cfg.max_neat_generations && (gens == 0 || start.elapsed() < cfg.budget) {
            if self.evaluated {
                self.neat.evolve(&mut rng);
            }
            sim_win_rate = self.evaluate(arena, limits, cfg, &mut rng);
            self.evaluated = true;
            gens += 1;
            progress((start.elapsed().as_secs_f32() / cfg.budget.as_secs_f32()).min(0.95).max(gens as f32 / cfg.max_neat_generations as f32 * 0.95));
        }

        // 5. Pick the next squad: top traits + explorers, each with a strong NEAT genome.
        self.generation += 1;
        let trait_ids = self.traits.pick_squad(3, SQUAD - 3, &mut rng);
        let mut neat_rank: Vec<usize> = (0..self.neat.genomes.len()).collect();
        neat_rank.sort_by(|&a, &b| self.neat.genomes[b].fitness.total_cmp(&self.neat.genomes[a].fitness));
        self.squad = trait_ids.iter().enumerate().map(|(k, &t)| SquadSlot { trait_id: t, neat_id: neat_rank[k % neat_rank.len().min(3)] }).collect();

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
        }
    }

    /// Evaluate every NEAT genome (and the trait genomes they're paired with) in sim rounds.
    /// Overwrites fitness with sim averages. Returns the squad win rate.
    fn evaluate(&mut self, arena: &Arena, limits: Limits, cfg: &EvolveConfig, rng: &mut BrainRng) -> f32 {
        // Build round line-ups.
        let mut lineups: Vec<(Vec<SquadSlot>, u64)> = Vec::new();
        for _ in 0..cfg.rounds_per_genome {
            let mut ids: Vec<usize> = (0..self.neat.genomes.len()).collect();
            ids.shuffle(rng);
            for chunk in ids.chunks(SQUAD) {
                let slots = chunk
                    .iter()
                    .map(|&n| SquadSlot { trait_id: rng.random_range(0..self.traits.genomes.len()), neat_id: n })
                    .collect();
                lineups.push((slots, rng.random()));
            }
        }
        let results: Vec<RoundResult> = run_parallel(&lineups, cfg.threads, |(slots, seed)| {
            let minds = slots
                .iter()
                .enumerate()
                .map(|(k, s)| {
                    EnemyMind::new(&self.traits.genomes[s.trait_id], s.trait_id, &self.neat.genomes[s.neat_id], s.neat_id, &limits, seed ^ k as u64)
                })
                .collect();
            sim::run_round(arena, &self.player_model, limits, minds, *seed)
        });

        let mut nf = vec![(0.0f32, 0u32); self.neat.genomes.len()];
        let mut tf = vec![(0.0f32, 0u32); self.traits.genomes.len()];
        let mut wins = 0;
        for ((slots, _), r) in lineups.iter().zip(&results) {
            if !r.player_won {
                wins += 1;
            }
            // Team bonus: everyone in a winning squad shares credit.
            let team = if r.player_won { 0.0 } else { 30.0 };
            for (s, st) in slots.iter().zip(&r.enemies) {
                let f = st.fitness() + team;
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
            if n > 0 {
                g.fitness = s / n as f32;
                g.evals = n;
            }
        }
        wins as f32 / lineups.len().max(1) as f32
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
                Ok(s) if s.version == SAVE_VERSION => s,
                _ => {
                    let _ = std::fs::rename(path, path.with_extension("json.bak"));
                    BrainState::new(seed)
                }
            },
            Err(_) => BrainState::new(seed),
        }
    }
}

/// Run `f` over `items` on up to `threads` scoped threads, keeping order.
pub(crate) fn run_parallel<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = threads.max(1).min(items.len().max(1));
    let chunk = items.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = items.chunks(chunk.max(1)).map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("sim thread panicked")).collect()
    })
}
