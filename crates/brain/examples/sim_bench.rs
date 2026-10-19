//! Headless evolution bench: `memgate -- cargo run -p brain --example sim_bench -- [matches] [seed]`
//! Each "match" reports a synthetic 2-round result (the player alternates 2-0 / 0-2 / 1-1) so the
//! ghost calibration runs too.
use brain::arena::Arena;
use brain::evolve::{BrainState, EvolveConfig, MatchReport};
use brain::ghost::GhostMode;
use brain::limits::Limits;
use brain::sim;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let matches: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let arena = Arena::foundry();
    let mut brain = BrainState::new(seed);
    let cfg = EvolveConfig::default();

    let t = std::time::Instant::now();
    let rate = |mode| {
        let n = 40;
        let wins = (0..n).filter(|&k| sim::run_round_with(&arena, &brain.player_model, Limits::for_generation(0), brain.squad_minds(k), k, mode).player_won).count();
        wins as f32 / n as f32
    };
    let (v1, v2) = (rate(GhostMode::V1), rate(GhostMode::V2));
    println!("ghost player-win vs gen-0 squad: v1 {v1:.2}  v2 {v2:.2}  ({:.1} ms/round)", t.elapsed().as_secs_f32() * 1000.0 / 80.0);

    println!("gen  sim_win  neat_gens  ms     calib_err      gap    top_delta");
    for g in 0..matches {
        let pr = [2, 0, 1][g as usize % 3];
        let report = MatchReport { player_rounds: pr, enemy_rounds: 2 - pr, player_won: pr == 2, ..Default::default() };
        let s = brain.evolve(&report, &arena, &cfg, |_| {});
        let d = s.deltas.first().map(|d| format!("{} {:+.2}", d.label, d.after - d.before)).unwrap_or_default();
        let ce = match (s.calibration_error_before, s.calibration_error) {
            (Some(b), Some(a)) => format!("{b:.2}->{a:.2}"),
            _ => "-".into(),
        };
        let gap = s.outcome_gap.map_or("-".into(), |x| format!("{x:+.2}"));
        println!("{:<4} {:<8.2} {:<10} {:<6} {ce:<14} {gap:<6} {d}", s.generation, s.sim_win_rate, s.neat_generations, s.elapsed_ms);
    }
    let m = &brain.player_model.ghost;
    println!("final ghost: aim {:.2} deg, reaction x{:.2}, turn {:.1} rad/s, preaim {:.2}, aggression {:.2}, retreat {:.2}", m.aim_sigma_deg, m.reaction_scale, m.turn_rate, m.preaim, m.aggression, m.retreat_hp);
}
