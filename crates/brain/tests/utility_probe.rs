//! Balance probe (ignored; run with `--ignored --nocapture`): player win rates against the
//! same squads with and without utility and anchors.
use brain::arena::Arena;
use brain::evolve::BrainState;
use brain::ghost::GhostMode;
use brain::limits::Limits;
use brain::player_model::PlayerModel;
use brain::sim;
use brain::squad::UtilityBudget;

#[test]
#[ignore]
fn probe() {
    let a = Arena::yard();
    let model = PlayerModel::default();
    let brains: Vec<BrainState> = (1..=4).map(BrainState::new).collect();
    for mode in [GhostMode::V1, GhostMode::V2] {
        for (name, u) in [("none", UtilityBudget::NONE), ("frag", UtilityBudget { frags: 2, flashes: 0 }), ("flash", UtilityBudget { frags: 0, flashes: 2 }), ("std", UtilityBudget::STANDARD)] {
            let mut wins = 0;
            for (b, s) in brains.iter().enumerate() {
                for k in 0..30u64 {
                    let seed = 100 + b as u64 * 1000 + k;
                    wins += sim::run_round_opts(&a, &model, Limits::for_generation(0), s.squad_minds(seed), seed, mode, true, u).player_won as u32;
                }
            }
            eprintln!("{mode:?} utility {name}: player win {:.2}", wins as f32 / 120.0);
        }
    }
}
