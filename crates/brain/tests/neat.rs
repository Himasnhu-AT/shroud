use brain::math::rng_from_seed;
use brain::neat::{NeatConfig, Network, Population};
use std::time::Instant;

const CASES: [([f32; 2], f32); 4] = [([0., 0.], 0.), ([0., 1.], 1.), ([1., 0.], 1.), ([1., 1.], 0.)];

fn xor_error(net: &mut Network) -> f32 {
    let mut e = 0.0;
    for (i, t) in CASES {
        let o = (net.activate(&i)[0] + 1.0) / 2.0;
        e += (o - t).abs();
    }
    e / 4.0
}

fn evolve_xor(gens: usize) -> Population {
    let mut rng = rng_from_seed(42);
    let mut pop = Population::new(2, 1, 150, NeatConfig::default(), &mut rng);
    for _ in 0..gens {
        let mut solved = false;
        for g in pop.genomes.iter_mut() {
            let e = xor_error(&mut g.build());
            g.fitness = (1.0 - e).powi(2) * 4.0;
            solved |= e < 0.1;
        }
        if solved {
            break;
        }
        pop.evolve(&mut rng);
    }
    pop
}

#[test]
fn xor_is_solved() {
    let t = Instant::now();
    let mut pop = evolve_xor(300);
    let best = pop.genomes.iter_mut().map(|g| xor_error(&mut g.build())).fold(f32::MAX, f32::min);
    assert!(best < 0.1, "best xor error {best}, species {}", pop.species_count());
    assert!(t.elapsed().as_secs_f32() < 20.0);
}

#[test]
fn population_serde_roundtrip() {
    let pop = evolve_xor(15);
    let json = serde_json::to_string(&pop).unwrap();
    let back: Population = serde_json::from_str(&json).unwrap();
    assert_eq!(back.genomes.len(), pop.genomes.len());
    assert_eq!(back.generation, pop.generation);
    assert_eq!(back.tracker.next_innovation, pop.tracker.next_innovation);
    assert_eq!(back.species_count(), pop.species_count());
    assert_eq!(back.best().conns.len(), pop.best().conns.len());
}

#[test]
fn evolved_network_is_acyclic() {
    let mut rng = rng_from_seed(7);
    let cfg = NeatConfig { add_node_rate: 0.2, add_connection_rate: 0.4, ..Default::default() };
    let mut pop = Population::new(14, 4, 60, cfg, &mut rng);
    for _ in 0..40 {
        for (i, g) in pop.genomes.iter_mut().enumerate() {
            g.fitness = (i % 7) as f32 + 1.0;
        }
        pop.evolve(&mut rng);
    }
    for g in &pop.genomes {
        let mut net = g.build();
        assert!(net.is_acyclic() && g.is_acyclic());
        assert_eq!(net.activate(&[0.5; 14]).len(), 4);
    }
    assert!(pop.genomes.iter().any(|g| g.num_hidden() > 0));
}
