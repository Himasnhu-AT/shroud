//! NEAT (NeuroEvolution of Augmenting Topologies), written in-house.
//!
//! CONTRACT (other modules depend on exactly this API):
//! - `Population::new(inputs, outputs, size, cfg, rng)`
//! - set `pop.genomes[i].fitness` (higher is better, keep >= 0), then `pop.evolve(rng)`
//! - `genome.build()` -> `Network`; `net.activate(&inputs)` -> outputs in [-1, 1]
//! - everything except `Network` is `Serialize + Deserialize`; all types are `Clone + Send`.
//!
//! STUB: to be implemented by the NEAT teammate.

use crate::math::BrainRng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NeatConfig {
    pub weight_mutate_rate: f32,
    pub weight_perturb_power: f32,
    pub add_connection_rate: f32,
    pub add_node_rate: f32,
    pub compat_threshold: f32,
    pub elitism: usize,
}

impl Default for NeatConfig {
    fn default() -> Self {
        NeatConfig {
            weight_mutate_rate: 0.8,
            weight_perturb_power: 0.5,
            add_connection_rate: 0.08,
            add_node_rate: 0.03,
            compat_threshold: 3.0,
            elitism: 2,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Genome {
    pub fitness: f32,
}

impl Genome {
    pub fn build(&self) -> Network {
        todo!("NEAT teammate")
    }
    pub fn num_hidden(&self) -> usize {
        todo!()
    }
    pub fn num_enabled_connections(&self) -> usize {
        todo!()
    }
}

#[derive(Clone, Debug)]
pub struct Network {}

impl Network {
    /// Feed-forward evaluation. `inputs.len()` must equal the population's input count.
    pub fn activate(&mut self, _inputs: &[f32]) -> &[f32] {
        todo!()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Population {
    pub genomes: Vec<Genome>,
    pub generation: u32,
}

impl Population {
    pub fn new(_inputs: usize, _outputs: usize, _size: usize, _cfg: NeatConfig, _rng: &mut BrainRng) -> Self {
        todo!()
    }
    /// Replace `genomes` with the next generation, using each genome's `fitness`.
    pub fn evolve(&mut self, _rng: &mut BrainRng) {
        todo!()
    }
    /// Fittest genome of the last evaluated generation (champion).
    pub fn best(&self) -> &Genome {
        todo!()
    }
    pub fn species_count(&self) -> usize {
        todo!()
    }
}
