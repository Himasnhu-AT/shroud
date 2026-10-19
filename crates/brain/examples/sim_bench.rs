//! Headless evolution bench: `memgate -- cargo run -p brain --example sim_bench -- [matches] [seed]`
use brain::arena::Arena;
use brain::evolve::{BrainState, EvolveConfig, MatchReport};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let matches: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let arena = Arena::foundry();
    let mut brain = BrainState::new(seed);
    let cfg = EvolveConfig::default();
    println!("gen  sim_win  neat_gens  ms     top_delta");
    for _ in 0..matches {
        let s = brain.evolve(&MatchReport::default(), &arena, &cfg, |_| {});
        let d = s.deltas.first().map(|d| format!("{} {:+.2}", d.label, d.after - d.before)).unwrap_or_default();
        println!("{:<4} {:<8.2} {:<10} {:<6} {d}", s.generation, s.sim_win_rate, s.neat_generations, s.elapsed_ms);
    }
}
