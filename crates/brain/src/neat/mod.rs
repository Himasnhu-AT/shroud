//! NEAT (NeuroEvolution of Augmenting Topologies), written in-house.
//!
//! CONTRACT (other modules depend on exactly this API):
//! - `Population::new(inputs, outputs, size, cfg, rng)`
//! - set `pop.genomes[i].fitness` (higher is better, keep >= 0), then `pop.evolve(rng)`
//! - `genome.build()` -> `Network`; `net.activate(&inputs)` -> outputs in [-1, 1]
//! - everything except `Network` is `Serialize + Deserialize`; all types are `Clone + Send`.
//!
//! Invariant: the set of all connection genes of a genome (enabled or not) is acyclic,
//! so every genome builds a feed-forward network.

use crate::math::{gauss, BrainRng};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const C_EXCESS: f32 = 1.0;
const C_DISJOINT: f32 = 1.0;
const C_WEIGHT: f32 = 0.4;
const WEIGHT_LIMIT: f32 = 8.0;
const TOGGLE_RATE: f32 = 0.01;
const REPLACE_RATE: f32 = 0.1;
const SURVIVAL: f32 = 0.3;
const MUTATE_ONLY_RATE: f32 = 0.25;
const DISABLE_INHERIT: f32 = 0.75;
const STALE_LIMIT: u32 = 15;
const TARGET_SPECIES_MIN: usize = 5;
const TARGET_SPECIES_MAX: usize = 10;
const THRESHOLD_STEP: f32 = 0.3;
const THRESHOLD_MIN: f32 = 0.3;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Input,
    Bias,
    Hidden,
    Output,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeGene {
    pub id: u32,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnGene {
    pub in_node: u32,
    pub out_node: u32,
    pub weight: f32,
    pub enabled: bool,
    pub innovation: u32,
}

/// Global innovation numbers: identical structural mutations share numbers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InnovationTracker {
    pub next_innovation: u32,
    pub next_node: u32,
    conns: HashMap<u64, u32>,
    splits: HashMap<u32, u32>,
}

impl InnovationTracker {
    pub fn conn(&mut self, a: u32, b: u32) -> u32 {
        let key = ((a as u64) << 32) | b as u64;
        if let Some(&i) = self.conns.get(&key) {
            return i;
        }
        let i = self.next_innovation;
        self.next_innovation += 1;
        self.conns.insert(key, i);
        i
    }
    pub fn new_node(&mut self) -> u32 {
        let n = self.next_node;
        self.next_node += 1;
        n
    }
    /// Node id created by splitting connection `innovation` (shared across genomes).
    pub fn split(&mut self, innovation: u32) -> u32 {
        if let Some(&n) = self.splits.get(&innovation) {
            return n;
        }
        let n = self.new_node();
        self.splits.insert(innovation, n);
        n
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Genome {
    pub fitness: f32,
    pub num_inputs: usize,
    pub num_outputs: usize,
    /// Inputs, then bias, then outputs, then hidden nodes.
    pub nodes: Vec<NodeGene>,
    /// Sorted by innovation number.
    pub conns: Vec<ConnGene>,
}

impl Genome {
    fn minimal(inputs: usize, outputs: usize, tracker: &mut InnovationTracker, rng: &mut BrainRng) -> Genome {
        let mut nodes = Vec::new();
        for i in 0..inputs {
            nodes.push(NodeGene { id: i as u32, kind: NodeKind::Input });
        }
        nodes.push(NodeGene { id: inputs as u32, kind: NodeKind::Bias });
        for o in 0..outputs {
            nodes.push(NodeGene { id: (inputs + 1 + o) as u32, kind: NodeKind::Output });
        }
        let mut conns = Vec::new();
        for i in 0..=inputs {
            for o in 0..outputs {
                let (a, b) = (i as u32, (inputs + 1 + o) as u32);
                conns.push(ConnGene {
                    in_node: a,
                    out_node: b,
                    weight: gauss(rng).clamp(-WEIGHT_LIMIT, WEIGHT_LIMIT),
                    enabled: true,
                    innovation: tracker.conn(a, b),
                });
            }
        }
        conns.sort_by_key(|c| c.innovation);
        Genome { fitness: 0.0, num_inputs: inputs, num_outputs: outputs, nodes, conns }
    }

    pub fn build(&self) -> Network {
        Network::from_genome(self)
    }
    pub fn num_hidden(&self) -> usize {
        self.nodes.iter().filter(|n| n.kind == NodeKind::Hidden).count()
    }
    pub fn num_enabled_connections(&self) -> usize {
        self.conns.iter().filter(|c| c.enabled).count()
    }
    /// True if the connection graph (enabled genes) has no cycle.
    pub fn is_acyclic(&self) -> bool {
        self.build().is_acyclic()
    }

    fn has_node(&self, id: u32) -> bool {
        self.nodes.iter().any(|n| n.id == id)
    }

    fn insert_conn(&mut self, c: ConnGene) {
        let pos = self.conns.partition_point(|x| x.innovation < c.innovation);
        self.conns.insert(pos, c);
    }

    /// Is `to` reachable from `from` following any connection genes?
    fn reachable(&self, from: u32, to: u32) -> bool {
        let mut stack = vec![from];
        let mut seen = vec![from];
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            for c in self.conns.iter().filter(|c| c.in_node == n) {
                if !seen.contains(&c.out_node) {
                    seen.push(c.out_node);
                    stack.push(c.out_node);
                }
            }
        }
        false
    }

    fn mutate_weights(&mut self, power: f32, rng: &mut BrainRng) {
        for c in &mut self.conns {
            if rng.random::<f32>() < REPLACE_RATE {
                c.weight = gauss(rng);
            } else {
                c.weight += gauss(rng) * power;
            }
            c.weight = c.weight.clamp(-WEIGHT_LIMIT, WEIGHT_LIMIT);
        }
    }

    fn mutate_add_connection(&mut self, tracker: &mut InnovationTracker, rng: &mut BrainRng) {
        let n = self.nodes.len();
        for _ in 0..30 {
            let a = self.nodes[rng.random_range(0..n)].clone();
            let b = self.nodes[rng.random_range(0..n)].clone();
            if a.kind == NodeKind::Output || matches!(b.kind, NodeKind::Input | NodeKind::Bias) || a.id == b.id {
                continue;
            }
            if self.conns.iter().any(|c| c.in_node == a.id && c.out_node == b.id) {
                continue;
            }
            if self.reachable(b.id, a.id) {
                continue;
            }
            let innovation = tracker.conn(a.id, b.id);
            self.insert_conn(ConnGene {
                in_node: a.id,
                out_node: b.id,
                weight: gauss(rng).clamp(-WEIGHT_LIMIT, WEIGHT_LIMIT),
                enabled: true,
                innovation,
            });
            return;
        }
    }

    fn mutate_add_node(&mut self, tracker: &mut InnovationTracker, rng: &mut BrainRng) {
        let enabled: Vec<usize> = (0..self.conns.len()).filter(|&i| self.conns[i].enabled).collect();
        if enabled.is_empty() {
            return;
        }
        let idx = enabled[rng.random_range(0..enabled.len())];
        let old = self.conns[idx].clone();
        let mut id = tracker.split(old.innovation);
        if self.has_node(id) {
            id = tracker.new_node();
        }
        self.conns[idx].enabled = false;
        self.nodes.push(NodeGene { id, kind: NodeKind::Hidden });
        let i1 = tracker.conn(old.in_node, id);
        let i2 = tracker.conn(id, old.out_node);
        self.insert_conn(ConnGene { in_node: old.in_node, out_node: id, weight: 1.0, enabled: true, innovation: i1 });
        self.insert_conn(ConnGene { in_node: id, out_node: old.out_node, weight: old.weight, enabled: true, innovation: i2 });
    }

    fn mutate(&mut self, cfg: &NeatConfig, tracker: &mut InnovationTracker, rng: &mut BrainRng) {
        if rng.random::<f32>() < cfg.weight_mutate_rate {
            self.mutate_weights(cfg.weight_perturb_power, rng);
        }
        if rng.random::<f32>() < cfg.add_connection_rate {
            self.mutate_add_connection(tracker, rng);
        }
        if rng.random::<f32>() < cfg.add_node_rate {
            self.mutate_add_node(tracker, rng);
        }
        if rng.random::<f32>() < TOGGLE_RATE && !self.conns.is_empty() {
            let i = rng.random_range(0..self.conns.len());
            self.conns[i].enabled = !self.conns[i].enabled;
        }
    }

    /// Compatibility distance (excess, disjoint, mean matching weight difference).
    pub fn distance(&self, other: &Genome) -> f32 {
        let (a, b) = (&self.conns, &other.conns);
        let (mut i, mut j) = (0, 0);
        let (mut disjoint, mut matching, mut wdiff, mut excess) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let max_a = a.last().map_or(0, |c| c.innovation);
        let max_b = b.last().map_or(0, |c| c.innovation);
        while i < a.len() && j < b.len() {
            let (x, y) = (&a[i], &b[j]);
            if x.innovation == y.innovation {
                matching += 1.0;
                wdiff += (x.weight - y.weight).abs();
                i += 1;
                j += 1;
            } else if x.innovation < y.innovation {
                if x.innovation > max_b {
                    excess += 1.0
                } else {
                    disjoint += 1.0
                }
                i += 1;
            } else {
                if y.innovation > max_a {
                    excess += 1.0
                } else {
                    disjoint += 1.0
                }
                j += 1;
            }
        }
        excess += (a.len() - i + b.len() - j) as f32;
        let n = a.len().max(b.len()).max(1) as f32;
        let n = if n < 20.0 { 1.0 } else { n };
        let w = if matching > 0.0 { wdiff / matching } else { 0.0 };
        C_EXCESS * excess / n + C_DISJOINT * disjoint / n + C_WEIGHT * w
    }

    /// `fitter` supplies disjoint/excess genes and the node set.
    fn crossover(fitter: &Genome, other: &Genome, rng: &mut BrainRng) -> Genome {
        let mut conns = Vec::with_capacity(fitter.conns.len());
        for g in &fitter.conns {
            let mut c = g.clone();
            if let Ok(k) = other.conns.binary_search_by_key(&g.innovation, |x| x.innovation) {
                let o = &other.conns[k];
                if rng.random::<bool>() {
                    c.weight = o.weight;
                }
                c.enabled = if !g.enabled || !o.enabled { rng.random::<f32>() >= DISABLE_INHERIT } else { true };
            }
            conns.push(c);
        }
        Genome {
            fitness: 0.0,
            num_inputs: fitter.num_inputs,
            num_outputs: fitter.num_outputs,
            nodes: fitter.nodes.clone(),
            conns,
        }
    }
}

/// Feed-forward evaluator. Buffers are allocated once in `build`.
#[derive(Clone, Debug)]
pub struct Network {
    n_in: usize,
    values: Vec<f32>,
    /// Computed nodes in topological order: (value index, edge range start, end).
    order: Vec<(u32, u32, u32)>,
    edges: Vec<(u32, f32)>,
    out_idx: Vec<u32>,
    out: Vec<f32>,
    acyclic: bool,
}

impl Network {
    fn from_genome(g: &Genome) -> Network {
        let n = g.nodes.len();
        let index: HashMap<u32, usize> = g.nodes.iter().enumerate().map(|(i, nd)| (nd.id, i)).collect();
        let mut incoming: Vec<Vec<(u32, f32)>> = vec![Vec::new(); n];
        let mut indeg = vec![0usize; n];
        let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); n];
        for c in g.conns.iter().filter(|c| c.enabled) {
            if let (Some(&a), Some(&b)) = (index.get(&c.in_node), index.get(&c.out_node)) {
                incoming[b].push((a as u32, c.weight));
                outgoing[a].push(b);
                indeg[b] += 1;
            }
        }
        let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut head = 0;
        while head < queue.len() {
            let i = queue[head];
            head += 1;
            for &o in &outgoing[i] {
                indeg[o] -= 1;
                if indeg[o] == 0 {
                    queue.push(o);
                }
            }
        }
        let acyclic = queue.len() == n;
        let mut order = Vec::new();
        let mut edges = Vec::new();
        for &i in &queue {
            if matches!(g.nodes[i].kind, NodeKind::Input | NodeKind::Bias) {
                continue;
            }
            let start = edges.len() as u32;
            edges.extend_from_slice(&incoming[i]);
            order.push((i as u32, start, edges.len() as u32));
        }
        let out_idx: Vec<u32> = g
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, nd)| nd.kind == NodeKind::Output)
            .map(|(i, _)| i as u32)
            .collect();
        Network {
            n_in: g.num_inputs,
            values: vec![0.0; n],
            order,
            edges,
            out: vec![0.0; out_idx.len()],
            out_idx,
            acyclic,
        }
    }

    /// Feed-forward evaluation. `inputs.len()` must equal the population's input count.
    pub fn activate(&mut self, inputs: &[f32]) -> &[f32] {
        debug_assert_eq!(inputs.len(), self.n_in);
        let n = self.n_in.min(inputs.len());
        self.values[..n].copy_from_slice(&inputs[..n]);
        self.values[self.n_in] = 1.0;
        for &(node, s, e) in &self.order {
            let mut sum = 0.0;
            for &(src, w) in &self.edges[s as usize..e as usize] {
                sum += self.values[src as usize] * w;
            }
            self.values[node as usize] = sum.tanh();
        }
        for (o, &i) in self.out.iter_mut().zip(&self.out_idx) {
            *o = self.values[i as usize];
        }
        &self.out
    }

    pub fn is_acyclic(&self) -> bool {
        self.acyclic
    }
    pub fn num_outputs(&self) -> usize {
        self.out.len()
    }
    pub fn num_inputs(&self) -> usize {
        self.n_in
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Species {
    pub id: u32,
    pub representative: Genome,
    pub best_fitness: f32,
    pub stale: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Population {
    pub genomes: Vec<Genome>,
    pub generation: u32,
    pub cfg: NeatConfig,
    pub tracker: InnovationTracker,
    pub species: Vec<Species>,
    /// Current (dynamic) compatibility threshold.
    pub threshold: f32,
    pub num_inputs: usize,
    pub num_outputs: usize,
    best: Genome,
    next_species_id: u32,
}

impl Population {
    pub fn new(inputs: usize, outputs: usize, size: usize, cfg: NeatConfig, rng: &mut BrainRng) -> Self {
        let mut tracker = InnovationTracker { next_node: (inputs + 1 + outputs) as u32, ..Default::default() };
        let size = size.max(1);
        let genomes: Vec<Genome> = (0..size).map(|_| Genome::minimal(inputs, outputs, &mut tracker, rng)).collect();
        let mut pop = Population {
            best: genomes[0].clone(),
            genomes,
            generation: 0,
            threshold: cfg.compat_threshold,
            cfg,
            tracker,
            species: Vec::new(),
            num_inputs: inputs,
            num_outputs: outputs,
            next_species_id: 0,
        };
        pop.speciate();
        pop
    }

    /// Assign genomes to species (dropping empty ones); returns members per species.
    fn speciate(&mut self) -> Vec<Vec<usize>> {
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); self.species.len()];
        for (i, g) in self.genomes.iter().enumerate() {
            let found = self.species.iter().position(|s| s.representative.distance(g) < self.threshold);
            match found {
                Some(k) => members[k].push(i),
                None => {
                    self.species.push(Species {
                        id: self.next_species_id,
                        representative: g.clone(),
                        best_fitness: f32::NEG_INFINITY,
                        stale: 0,
                    });
                    self.next_species_id += 1;
                    members.push(vec![i]);
                }
            }
        }
        let mut k = 0;
        self.species.retain(|_| {
            let keep = !members[k].is_empty();
            k += 1;
            keep
        });
        members.retain(|m| !m.is_empty());
        members
    }

    /// Replace `genomes` with the next generation, using each genome's `fitness`.
    pub fn evolve(&mut self, rng: &mut BrainRng) {
        let n = self.genomes.len();
        if n == 0 {
            return;
        }
        let fit = |g: &Genome| if g.fitness.is_finite() { g.fitness.max(0.0) } else { 0.0 };
        let mut by_fit: Vec<usize> = (0..n).collect();
        by_fit.sort_by(|&a, &b| fit(&self.genomes[b]).total_cmp(&fit(&self.genomes[a])));
        let best_i = by_fit[0];
        self.best = self.genomes[best_i].clone();

        let members = self.speciate();
        let species_found = self.species.len();
        let mut alive: Vec<usize> = Vec::new();
        for (k, m) in members.iter().enumerate() {
            let top = m.iter().map(|&i| fit(&self.genomes[i])).fold(f32::NEG_INFINITY, f32::max);
            let s = &mut self.species[k];
            if top > s.best_fitness + 1e-6 {
                s.best_fitness = top;
                s.stale = 0;
            } else {
                s.stale += 1;
            }
            if s.stale < STALE_LIMIT || m.contains(&best_i) {
                alive.push(k);
            }
        }

        // Dynamic threshold for the next speciation.
        if species_found > TARGET_SPECIES_MAX {
            self.threshold += THRESHOLD_STEP;
        } else if species_found < TARGET_SPECIES_MIN {
            self.threshold = (self.threshold - THRESHOLD_STEP).max(THRESHOLD_MIN);
        }

        let elites = self.cfg.elitism.min(n);
        let mut next: Vec<Genome> = by_fit[..elites]
            .iter()
            .map(|&i| {
                let mut g = self.genomes[i].clone();
                g.fitness = 0.0;
                g
            })
            .collect();
        let slots = n - elites;

        // Offspring allocation by mean (fitness-shared) species fitness.
        let shares: Vec<f32> = alive
            .iter()
            .map(|&k| members[k].iter().map(|&i| fit(&self.genomes[i])).sum::<f32>() / members[k].len() as f32)
            .collect();
        let total: f32 = shares.iter().sum();
        let ideal: Vec<f32> = shares
            .iter()
            .map(|s| if total > 0.0 { s / total * slots as f32 } else { slots as f32 / alive.len() as f32 })
            .collect();
        let mut counts: Vec<usize> = ideal.iter().map(|x| x.floor() as usize).collect();
        let left = slots.saturating_sub(counts.iter().sum::<usize>());
        let mut rem: Vec<usize> = (0..alive.len()).collect();
        rem.sort_by(|&a, &b| (ideal[b] - ideal[b].floor()).total_cmp(&(ideal[a] - ideal[a].floor())));
        for &r in rem.iter().cycle().take(left) {
            counts[r] += 1;
        }

        for (ai, &k) in alive.iter().enumerate() {
            let mut m = members[k].clone();
            m.sort_by(|&a, &b| fit(&self.genomes[b]).total_cmp(&fit(&self.genomes[a])));
            let survivors = ((m.len() as f32 * SURVIVAL).ceil() as usize).clamp(1, m.len());
            for _ in 0..counts[ai] {
                let pa = m[rng.random_range(0..survivors)];
                let mut child = if survivors == 1 || rng.random::<f32>() < MUTATE_ONLY_RATE {
                    self.genomes[pa].clone()
                } else {
                    let pb = m[rng.random_range(0..survivors)];
                    let (a, b) = (&self.genomes[pa], &self.genomes[pb]);
                    if fit(a) >= fit(b) {
                        Genome::crossover(a, b, rng)
                    } else {
                        Genome::crossover(b, a, rng)
                    }
                };
                child.fitness = 0.0;
                child.mutate(&self.cfg, &mut self.tracker, rng);
                next.push(child);
            }
        }
        // Safety net (e.g. rounding): fill from the champion.
        while next.len() < n {
            let mut g = self.best.clone();
            g.fitness = 0.0;
            g.mutate(&self.cfg, &mut self.tracker, rng);
            next.push(g);
        }
        next.truncate(n);

        // Keep only surviving species, with a fresh random representative.
        let mut kept = Vec::new();
        for &k in &alive {
            let mut s = self.species[k].clone();
            s.representative = self.genomes[members[k][rng.random_range(0..members[k].len())]].clone();
            kept.push(s);
        }
        self.species = kept;
        self.genomes = next;
        self.generation += 1;
    }

    /// Fittest genome of the last evaluated generation (champion).
    pub fn best(&self) -> &Genome {
        &self.best
    }
    pub fn species_count(&self) -> usize {
        self.species.len()
    }
}
