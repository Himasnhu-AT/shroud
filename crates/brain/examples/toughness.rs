//! Squad toughness at generation 0 on every map and difficulty, grenades off so it measures
//! the bots' own fighting. Each cell: player (ghost v2) win rate as the sim scores it (a
//! 60 s timeout counts as a player win), then how often he actually killed the squad, and the
//! number of timeouts (the game itself has no round clock).
//! `memgate -- cargo run -p brain --example toughness -- [rounds_per_brain]`
use brain::arena::{Arena, MapId};
use brain::difficulty::Difficulty;
use brain::evolve::BrainState;
use brain::ghost::GhostMode;
use brain::sim;
use brain::squad::UtilityBudget;

fn main() {
    let rounds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(12);
    let brains: Vec<BrainState> = (1..=4).map(BrainState::new).collect();
    let t = std::time::Instant::now();
    let n_cell = brains.len() as u64 * rounds;
    println!("gen-0 squads vs ghost v2, {n_cell} rounds per cell: player_win / player_kills_squad / timeouts");
    println!("{:<8} {:>18} {:>18} {:>18} {:>18}", "map", "Recruit", "Regular", "Veteran", "Elite");
    let (mut pw_sum, mut sw_sum, mut cells) = (0.0, 0.0, 0.0);
    for id in MapId::ALL {
        let a = Arena::build(id);
        let mut line = format!("{:<8}", id.label());
        for d in Difficulty::ALL {
            let p = d.params();
            let limits = d.limits(0);
            let (mut player, mut squad, mut timeouts) = (0u32, 0u32, 0u32);
            for (b, s) in brains.iter().enumerate() {
                for k in 0..rounds {
                    let seed = 7_000 + b as u64 * 1000 + k;
                    let minds = s.squad_minds_for(seed, p.squad_size, &limits);
                    let r = sim::run_round_opts(&a, &s.player_model, limits, minds, seed, GhostMode::V2, p.coordination, UtilityBudget::NONE);
                    player += r.player_won as u32;
                    squad += (r.player_won && r.duration < sim::ROUND_TIME - 0.1) as u32;
                    timeouts += (r.duration >= sim::ROUND_TIME - 0.1) as u32;
                }
            }
            let (pw, sw) = (player as f32 / n_cell as f32, squad as f32 / n_cell as f32);
            pw_sum += pw;
            sw_sum += sw;
            cells += 1.0;
            line += &format!(" {:>18}", format!("{pw:.2} / {sw:.2} / {timeouts:>3}"));
        }
        println!("{line}");
    }
    println!("mean player win {:.3}, mean player-kills-squad {:.3}  ({:.1} s)", pw_sum / cells, sw_sum / cells, t.elapsed().as_secs_f32());
}
