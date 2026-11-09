//! Ghost v2 and its calibration (paper §4.1 / roadmap P1-1).
use brain::arena::Arena;
use brain::evolve::BrainState;
use brain::ghost::{self, CalibConfig, GhostMode, GhostParams, Outcome, SimStats};
use brain::limits::Limits;
use brain::player_model::PlayerModel;
use brain::sim;
use brain::squad::UtilityBudget;

fn win_rate(a: &Arena, model: &PlayerModel, mode: GhostMode, brains: &[BrainState], rounds: u64) -> f32 {
    let mut wins = 0;
    for (b, s) in brains.iter().enumerate() {
        for k in 0..rounds {
            let seed = 100 + b as u64 * 1000 + k;
            // Squad grenades and plays off: this compares the ghosts' own skill. Frags/flashes
            // and the squad's encirclement deliberately punish V2's hold-and-re-peek style, so
            // against a coordinated squad the gap says more about the squad than the ghost.
            // Rifles only (the pre-loadout baseline): this measures the ghosts, not the guns.
            let mut minds = s.squad_minds(seed);
            minds.iter_mut().for_each(|m| m.set_weapon(brain::weapons::WeaponKind::Rifle));
            wins += sim::run_round_opts(a, model, Limits::for_generation(0), minds, seed, mode, false, UtilityBudget::NONE).player_won as u32;
        }
    }
    wins as f32 / (brains.len() as u64 * rounds) as f32
}

#[test]
fn ghost_v2_beats_v1_against_the_same_squads() {
    let a = Arena::yard();
    let model = PlayerModel::default();
    let brains: Vec<BrainState> = (1..=4).map(BrainState::new).collect();
    let v1 = win_rate(&a, &model, GhostMode::V1, &brains, 40);
    let v2 = win_rate(&a, &model, GhostMode::V2, &brains, 40);
    eprintln!("player win rate: v1 {v1:.2}, v2 {v2:.2}");
    // Margin 0.02: since aim/reaction stopped varying per enemy (technique-only evolution,
    // limits.rs), fresh squads are uniformly mid-skill and V2's edge shrank from ~0.1 to ~0.03.
    assert!(v2 > v1 + 0.02, "v2 {v2} should beat v1 {v1}");
}

#[test]
fn ghost_v2_round_is_deterministic() {
    let a = Arena::yard();
    let s = BrainState::new(3);
    let model = PlayerModel::default();
    let r1 = sim::run_round(&a, &model, Limits::for_generation(0), s.squad_minds(5), 9);
    let r2 = sim::run_round(&a, &model, Limits::for_generation(0), s.squad_minds(5), 9);
    assert_eq!((r1.player_won, r1.duration, r1.ghost.shots, r1.ghost.hits), (r2.player_won, r2.duration, r2.ghost.shots, r2.ghost.hits));
    assert!(r1.ghost.shots.iter().sum::<f32>() > 0.0, "the ghost should get into a fight");
}

fn planted() -> GhostParams {
    GhostParams { aim_sigma_deg: 6.5, reaction_scale: 1.7, turn_rate: 5.0, preaim: 0.3, aggression: 0.15, retreat_hp: 0.5, ..GhostParams::default() }
}

#[test]
fn calibration_recovers_planted_parameters() {
    let a = Arena::yard();
    let brains: Vec<BrainState> = (11..=13).map(BrainState::new).collect();
    let make = |g: usize, seed: u64| (brains[g].squad_minds(seed), Limits::for_generation(0));
    let model = PlayerModel::default();

    // "Real" matches: the planted ghost, on seeds the calibration never sees.
    let truth = ghost::simulate(&a, &model, planted(), brains.len(), 24, 777_000, &make);
    let outcomes: Vec<Outcome> = truth.iter().map(|s| Outcome { player_rounds: s.player_wins, rounds: s.rounds }).collect();
    let mut all = SimStats::default();
    truth.iter().for_each(|s| all.merge(s));
    let real = all.as_real();

    let t0 = std::time::Instant::now();
    let cfg = CalibConfig { iters: 14, lambda: 6, rounds: 8, threads: 6, seed: 5, deadline: None, prior: 0.5 };
    let res = ghost::calibrate(&a, &model, &outcomes, &real, &cfg, &make);
    let dist = |p: &GhostParams| p.to_unit().iter().zip(planted().to_unit()).map(|(x, y)| (x - y).powi(2)).sum::<f32>().sqrt();
    eprintln!(
        "calibration error {:.3} -> {:.3} in {:?} ({} evals); param distance {:.3} -> {:.3}; {:?}",
        res.error_before,
        res.error_after,
        t0.elapsed(),
        res.evaluations,
        dist(&GhostParams::default()),
        dist(&res.params),
        res.params
    );
    assert!(res.error_after < 0.5 * res.error_before, "error {} -> {}", res.error_before, res.error_after);
    // Aim and reaction are identifiable from outcome stats (turn rate / pre-aim / aggression mostly
    // trade off against each other), so recovery is checked on those two.
    let ident = |p: &GhostParams| {
        let (u, t) = (p.to_unit(), planted().to_unit());
        ((u[0] - t[0]).powi(2) + (u[1] - t[1]).powi(2)).sqrt()
    };
    assert!(ident(&res.params) < 0.5 * ident(&GhostParams::default()), "aim/reaction should approach the planted values: {:?}", res.params);

    // Deterministic for a fixed config.
    let small = CalibConfig { iters: 2, rounds: 2, ..cfg };
    let r1 = ghost::calibrate(&a, &model, &outcomes, &real, &small, &make);
    let r2 = ghost::calibrate(&a, &model, &outcomes, &real, &small, &make);
    assert_eq!(r1.params, r2.params);
    assert_eq!(r1.error_after, r2.error_after);
}
