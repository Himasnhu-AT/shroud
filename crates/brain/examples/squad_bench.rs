//! Squad toughness: ghost (player) win rate vs generation-0 squads on every map, Regular and
//! Elite sized, coordinated. `memgate -- cargo run -p brain --example squad_bench -- [rounds]`
use brain::arena::{Arena, MapId};
use brain::difficulty::Difficulty;
use brain::evolve::BrainState;
use brain::ghost::GhostMode;
use brain::player_model::PlayerModel;
use brain::sim;
use brain::squad::UtilityBudget;

fn main() {
    let rounds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let model = PlayerModel::default();
    let t0 = std::time::Instant::now();
    println!("map       size  ghost_win  timeouts  squad_dmg");
    let mut all = (0.0, 0.0);
    for id in MapId::ALL {
        let a = Arena::build(id);
        for d in [Difficulty::Regular, Difficulty::Elite] {
            let n = d.params().squad_size;
            let (mut wins, mut timeouts, mut dmg) = (0u32, 0u32, 0.0f32);
            for k in 0..rounds {
                let b = BrainState::new(100 + k / 6);
                let minds = b.squad_minds_for(k, n, &d.limits(0));
                let r = sim::run_round_opts(&a, &model, d.limits(0), minds, 7000 + k, GhostMode::V2, true, UtilityBudget::NONE);
                wins += r.player_won as u32;
                timeouts += (r.duration >= sim::ROUND_TIME - 0.1) as u32;
                dmg += r.enemies.iter().map(|e| e.damage_dealt).sum::<f32>();
            }
            let wr = wins as f32 / rounds as f32;
            all.0 += wr;
            all.1 += 1.0;
            println!("{:<9} {:<5} {:<10.3} {:<9} {:.1}", id.label(), n, wr, timeouts, dmg / rounds as f32);
        }
    }
    // (Names a constant that only exists with the encirclement, so a stale shared-target build fails.)
    println!("mean ghost win {:.3}  ({:.1} s, encircle {} s)", all.0 / all.1, t0.elapsed().as_secs_f32(), brain::squad::ENCIRCLE_S);
}
