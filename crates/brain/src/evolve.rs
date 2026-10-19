//! Between-match evolution and the persisted brain (docs/EVOLUTION.md).

use crate::arena::Arena;
use crate::genome::{GENE_COUNT, GaConfig, Gene, TraitPopulation};
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
}

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
    pub deltas: Vec<TraitDelta>,
    /// Share of sim rounds the squad won against the ghost in the last trained generation.
    pub sim_win_rate: f32,
    /// Ghost calibration: real player round-win rate vs the ghost's against the same squad.
    pub real_player_win: Option<f32>,
    pub ghost_player_win: Option<f32>,
    pub ghost_skill: f32,
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

        // 1. Learn the player.
        self.player_model.ingest(&report.telemetry);

        // 1b. Calibrate the ghost: replay the squad the player just fought against the ghost and
        // nudge ghost skill until the ghost wins as often as the real player did.
        let rounds = report.player_rounds + report.enemy_rounds;
        let (mut real_player_win, mut ghost_player_win) = (None, None);
        if rounds > 0 && self.evaluated {
            let real = report.player_rounds as f32 / rounds as f32;
            let limits = self.limits();
            let n = 16u64;
            let ghost_wins = (0..n)
                .filter(|&k| sim::run_round(arena, &self.player_model, limits, self.squad_minds(9000 + k), 7000 + k).player_won)
                .count();
            let sim_rate = ghost_wins as f32 / n as f32;
            let m = &mut self.player_model;
            m.ghost_skill = (m.ghost_skill * (1.5 * (real - sim_rate)).exp()).clamp(0.5, 3.0);
            real_player_win = Some(real);
            ghost_player_win = Some(sim_rate);
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
        });
        let prev = self.history.len().checked_sub(2).map(|i| self.history[i].stats.clone());
        progress(1.0);
        EvolveSummary {
            generation: self.generation,
            insights: self.player_model.insights(arena, Some(&report.telemetry), prev.as_ref(), self.squad_reaction_ms()),
            deltas,
            sim_win_rate,
            real_player_win,
            ghost_player_win,
            ghost_skill: self.player_model.ghost_skill,
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
fn run_parallel<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = threads.max(1).min(items.len().max(1));
    let chunk = items.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = items.chunks(chunk.max(1)).map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("sim thread panicked")).collect()
    })
}
