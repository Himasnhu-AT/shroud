//! Headless evolution bench: `memgate -- cargo run -p brain --example sim_bench -- [matches] [seed]`
//!
//! `held_out` is the deployed squad's win rate on 32 fixed seeds the trainer never sees,
//! so it measures squad strength rather than the trainer's own (selected, optimistic) estimate.
use brain::arena::Arena;
use brain::evolve::{BrainState, EvolveConfig, MatchReport};
use brain::sim;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let matches: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let arena = Arena::foundry();
    let mut brain = BrainState::new(seed);
    let cfg = EvolveConfig::default();
    let held_out = |b: &BrainState| {
        let wins = (0..32u64).filter(|&k| !sim::run_round(&arena, &b.player_model, b.limits(), b.squad_minds(50_000 + k), 90_000 + k).player_won).count();
        wins as f32 / 32.0
    };
    println!("gen  sim_win  held_out  neat_gens  ms     top_delta");
    for _ in 0..matches {
        let s = brain.evolve(&MatchReport::default(), &arena, &cfg, |_| {});
        let d = s.deltas.first().map(|d| format!("{} {:+.2}", d.label, d.after - d.before)).unwrap_or_default();
        println!("{:<4} {:<8.2} {:<9.2} {:<10} {:<6} {d}", s.generation, s.sim_win_rate, held_out(&brain), s.neat_generations, s.elapsed_ms);
    }
}
