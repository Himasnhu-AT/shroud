//! Trait genome: an enemy's personality, evolved with a plain genetic algorithm.
//! Genes live in [0, 1]; accessors map them to real ranges (see docs/AI.md §1).

use crate::limits::Limits;
use crate::math::{BrainRng, gauss};
use rand::Rng;
use serde::{Deserialize, Serialize};

pub const GENE_COUNT: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gene {
    Aggression = 0,
    PreferredRange,
    FlankBias,
    CoverAffinity,
    ReactionTime,
    AimError,
    BurstLength,
    RetreatHealth,
    TeamCohesion,
    PredictionTrust,
}

impl Gene {
    pub const ALL: [Gene; GENE_COUNT] = [
        Gene::Aggression,
        Gene::PreferredRange,
        Gene::FlankBias,
        Gene::CoverAffinity,
        Gene::ReactionTime,
        Gene::AimError,
        Gene::BurstLength,
        Gene::RetreatHealth,
        Gene::TeamCohesion,
        Gene::PredictionTrust,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Gene::Aggression => "Aggression",
            Gene::PreferredRange => "Preferred range",
            Gene::FlankBias => "Flanking",
            Gene::CoverAffinity => "Cover use",
            Gene::ReactionTime => "Reaction speed",
            Gene::AimError => "Aim",
            Gene::BurstLength => "Burst length",
            Gene::RetreatHealth => "Caution",
            Gene::TeamCohesion => "Teamwork",
            Gene::PredictionTrust => "Prediction",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraitGenome {
    pub genes: [f32; GENE_COUNT],
    pub fitness: f32,
    /// How many evaluations contributed to `fitness` (for averaging).
    pub evals: u32,
}

/// Decoded traits in real units, with fairness limits applied.
#[derive(Clone, Copy, Debug)]
pub struct Traits {
    pub aggression: f32,
    pub preferred_range: f32,
    pub flank_bias: f32,
    pub cover_affinity: f32,
    pub reaction_time: f32,
    pub aim_error_deg: f32,
    pub burst_length: u32,
    pub retreat_health: f32,
    pub team_cohesion: f32,
    pub prediction_trust: f32,
}

impl TraitGenome {
    pub fn random(rng: &mut BrainRng) -> Self {
        let mut genes = [0.0; GENE_COUNT];
        for g in &mut genes {
            *g = rng.random();
        }
        TraitGenome { genes, fitness: 0.0, evals: 0 }
    }

    pub fn get(&self, g: Gene) -> f32 {
        self.genes[g as usize]
    }

    /// Decode genes to real values. Higher ReactionTime/AimError genes mean *better* (faster, tighter).
    pub fn decode(&self, limits: &Limits) -> Traits {
        let g = |x: Gene| self.get(x);
        Traits {
            aggression: g(Gene::Aggression),
            preferred_range: 6.0 + 24.0 * g(Gene::PreferredRange),
            flank_bias: g(Gene::FlankBias),
            cover_affinity: g(Gene::CoverAffinity),
            reaction_time: 0.7 - (0.7 - limits.min_reaction_s) * g(Gene::ReactionTime),
            aim_error_deg: 8.0 - (8.0 - limits.min_aim_error_deg) * g(Gene::AimError),
            burst_length: 2 + (8.0 * g(Gene::BurstLength)).round() as u32,
            retreat_health: 0.6 * g(Gene::RetreatHealth),
            team_cohesion: g(Gene::TeamCohesion),
            prediction_trust: g(Gene::PredictionTrust),
        }
    }

    pub fn crossover(a: &Self, b: &Self, rng: &mut BrainRng) -> Self {
        let mut genes = [0.0; GENE_COUNT];
        for (i, g) in genes.iter_mut().enumerate() {
            *g = if rng.random::<bool>() { a.genes[i] } else { b.genes[i] };
        }
        TraitGenome { genes, fitness: 0.0, evals: 0 }
    }

    pub fn mutate(&mut self, rate: f32, sigma: f32, rng: &mut BrainRng) {
        for g in &mut self.genes {
            if rng.random::<f32>() < rate {
                *g = (*g + gauss(rng) * sigma).clamp(0.0, 1.0);
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GaConfig {
    pub population: usize,
    pub elites: usize,
    pub tournament: usize,
    pub mutation_rate: f32,
    pub mutation_sigma: f32,
}

impl Default for GaConfig {
    fn default() -> Self {
        GaConfig { population: 24, elites: 2, tournament: 3, mutation_rate: 0.2, mutation_sigma: 0.08 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraitPopulation {
    pub genomes: Vec<TraitGenome>,
    pub cfg: GaConfig,
    pub generation: u32,
}

impl TraitPopulation {
    pub fn new(cfg: GaConfig, rng: &mut BrainRng) -> Self {
        let genomes = (0..cfg.population).map(|_| TraitGenome::random(rng)).collect();
        TraitPopulation { genomes, cfg, generation: 0 }
    }

    /// Indices sorted by fitness, best first.
    pub fn ranked(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.genomes.len()).collect();
        idx.sort_by(|&a, &b| self.genomes[b].fitness.total_cmp(&self.genomes[a].fitness));
        idx
    }

    fn tournament(&self, rng: &mut BrainRng) -> &TraitGenome {
        let n = self.genomes.len();
        let mut best = &self.genomes[rng.random_range(0..n)];
        for _ in 1..self.cfg.tournament {
            let c = &self.genomes[rng.random_range(0..n)];
            if c.fitness > best.fitness {
                best = c;
            }
        }
        best
    }

    /// Breed the next generation from current fitness values.
    pub fn evolve(&mut self, rng: &mut BrainRng) {
        let ranked = self.ranked();
        let mut next: Vec<TraitGenome> = ranked
            .iter()
            .take(self.cfg.elites)
            .map(|&i| TraitGenome { fitness: 0.0, evals: 0, ..self.genomes[i].clone() })
            .collect();
        while next.len() < self.cfg.population {
            let (a, b) = (self.tournament(rng), self.tournament(rng));
            let mut child = TraitGenome::crossover(a, b, rng);
            child.mutate(self.cfg.mutation_rate, self.cfg.mutation_sigma, rng);
            next.push(child);
        }
        self.genomes = next;
        self.generation += 1;
    }

    /// Squad for the next match: top `elite` plus `explore` random others.
    pub fn pick_squad(&self, elite: usize, explore: usize, rng: &mut BrainRng) -> Vec<usize> {
        let ranked = self.ranked();
        let mut out: Vec<usize> = ranked.iter().copied().take(elite).collect();
        let mut rest: Vec<usize> = ranked.iter().copied().skip(elite).collect();
        for _ in 0..explore.min(rest.len()) {
            let k = rng.random_range(0..rest.len());
            out.push(rest.swap_remove(k));
        }
        out
    }
}
