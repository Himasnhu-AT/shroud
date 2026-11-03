use brain::arena::Arena;
use brain::evolve::{BrainState, EvolveConfig, MatchReport};
use brain::genome::{GaConfig, TraitPopulation};
use brain::limits::Limits;
use brain::math::{V2, rng_from_seed};
use brain::player_model::{MatchTelemetry, PlayerModel};
use brain::sim;
use std::time::Duration;

#[test]
fn nav_finds_path_around_center_tower() {
    let a = Arena::foundry();
    let p = a.nav.find_path(V2::new(0.0, 10.0), V2::new(0.0, -10.0)).expect("path");
    assert!(p.len() >= 2, "must detour around the tower");
    assert!(p.iter().all(|w| a.nav.walkable_at(*w)));
}

#[test]
fn low_cover_blocks_only_crouched() {
    let a = Arena::foundry();
    // Long low wall at z = 26 between x = 0..8.
    let (from, to) = (V2::new(4.0, 29.0), V2::new(4.0, 22.0));
    assert!(a.line_of_sight(from, false, to, false));
    assert!(!a.line_of_sight(from, true, to, false));
}

#[test]
fn ga_converges_on_toy_fitness() {
    let mut rng = rng_from_seed(1);
    let mut pop = TraitPopulation::new(GaConfig::default(), &mut rng);
    let target = 0.8;
    let score = |p: &TraitPopulation| p.genomes.iter().map(|g| -g.genes.iter().map(|x| (x - target).powi(2)).sum::<f32>()).fold(f32::MIN, f32::max);
    let start = score(&pop);
    for _ in 0..60 {
        for g in &mut pop.genomes {
            g.fitness = 10.0 - g.genes.iter().map(|x| (x - target).powi(2)).sum::<f32>();
        }
        pop.evolve(&mut rng);
    }
    assert!(score(&pop) > start * 0.2, "best error should shrink a lot: {start} -> {}", score(&pop));
}

#[test]
fn player_model_ema_and_confidence() {
    let mut m = PlayerModel::default();
    assert_eq!(m.confidence(), 0.0);
    let mut t = MatchTelemetry::default();
    for i in 0..200 {
        t.sample(V2::new(0.0, 20.0), if i % 2 == 0 { 0.0 } else { 3.0 }, false, true, -1.0);
    }
    for _ in 0..20 {
        t.shot(12.0, true, false);
    }
    t.reaction(0.25);
    m.ingest(&t);
    assert!(m.strafe_bias < -0.5, "strafes left: {}", m.strafe_bias);
    assert!(m.accuracy_at(12.0) > 0.7);
    assert!(m.confidence() > 0.3);
    let a = Arena::foundry();
    let ins = m.insights(&a, Some(&t), None, 500.0);
    assert!(ins.iter().any(|s| s.contains("south yard")), "{ins:?}");
    assert!(ins.iter().all(|s| s.is_ascii()), "UI font is ASCII-only: {ins:?}");
}

#[test]
fn sim_round_finishes_and_is_deterministic() {
    let a = Arena::foundry();
    let s = BrainState::new(7);
    let model = PlayerModel::default();
    let r1 = sim::run_round(&a, &model, Limits::for_generation(0), s.squad_minds(1), 42);
    let r2 = sim::run_round(&a, &model, Limits::for_generation(0), s.squad_minds(1), 42);
    assert!(r1.duration > 0.0);
    assert_eq!(r1.player_won, r2.player_won);
    assert_eq!(r1.duration, r2.duration);
}

#[test]
fn save_load_round_trip() {
    let dir = std::env::temp_dir().join(format!("shroud-test-{}", std::process::id()));
    let path = dir.join("brain.json");
    let s = BrainState::new(3);
    s.save(&path).unwrap();
    let l = BrainState::load_or_new(&path, 99);
    assert_eq!(l.seed, 3);
    std::fs::write(&path, b"not json").unwrap();
    let fresh = BrainState::load_or_new(&path, 99);
    assert_eq!(fresh.seed, 99);
    assert!(path.with_extension("json.bak").exists());
    let _ = std::fs::remove_dir_all(dir);
}

/// The key claim: evolved squads beat fresh ones against the same ghost.
#[test]
fn evolution_makes_squads_stronger() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    // Evaluate under Recruit mechanics: at the default level the fresh squad already wins most
    // rounds against the ghost, leaving no headroom to see technique gains. Evolution never
    // changes aim/reaction/damage (`limits.rs`), so the gain measured here is technique alone.
    let limits = brain::difficulty::Difficulty::Recruit.limits(0);
    let mut s = BrainState::new(11);
    let gen0 = s.clone();
    let cfg = EvolveConfig { budget: Duration::from_secs(60), max_neat_generations: 4, ..Default::default() };
    // Six generations, 48 rounds: since the perception/aggression pass (addendum) the gen-0
    // squad already wins ~half its rounds, so three generations on 24 rounds left too little
    // headroom to measure the gain reliably.
    for _ in 0..6 {
        s.evolve(&MatchReport::default(), &a, &cfg, |_| {});
    }
    let score = |b: &BrainState| -> (f32, f32) {
        let (mut wins, mut dmg) = (0.0, 0.0);
        for seed in 0..48 {
            let r = sim::run_round(&a, &model, limits, b.squad_minds(seed), 1000 + seed);
            if !r.player_won {
                wins += 1.0;
            }
            dmg += r.enemies.iter().map(|e| e.damage_dealt).sum::<f32>();
        }
        (wins / 48.0, dmg / 48.0)
    };
    let (w0, d0) = score(&gen0);
    let (w1, d1) = score(&s);
    eprintln!("gen0: win {w0:.2} dmg {d0:.1} | evolved: win {w1:.2} dmg {d1:.1}");
    assert!(d1 > d0 * 1.1 || w1 > w0 + 0.1, "evolved squad should be stronger");
}


#[test]
fn insights_are_specific() {
    let a = Arena::foundry();
    let mut m = PlayerModel::default();
    let mut t = MatchTelemetry::default();
    for _ in 0..300 {
        t.sample(V2::new(20.0, 0.0), 0.0, false, false, 0.0);
        t.look(-std::f32::consts::FRAC_PI_2); // always looking north
    }
    for _ in 0..3 {
        t.death(V2::new(20.0, 0.0)); // east lane
    }
    m.ingest(&t);
    let ins = m.insights(&a, Some(&t), None, 500.0);
    assert!(ins.iter().any(|s| s.contains("deaths were in the east lane")), "{ins:?}");
    assert!(ins.iter().any(|s| s.contains("looked north")), "{ins:?}");
}

/// If the real player keeps winning, the ghost must get stronger (and vice versa).
/// Calibration tunes several ghost parameters, so measure the ghost's actual strength:
/// its round-win rate against a fixed reference squad on fixed seeds.
#[test]
fn ghost_calibrates_toward_real_outcomes() {
    let a = Arena::foundry();
    let reference = BrainState::new(99);
    let strength = |model: &PlayerModel| -> f32 {
        let wins = (0..24u64)
            .filter(|&k| sim::run_round(&a, model, Limits::for_generation(0), reference.squad_minds(k), 500 + k).player_won)
            .count();
        wins as f32 / 24.0
    };
    let mut s = BrainState::new(5);
    let cfg = EvolveConfig { budget: Duration::from_secs(30), max_neat_generations: 2, ..Default::default() };
    s.evolve(&MatchReport::default(), &a, &cfg, |_| {});

    let won = MatchReport { player_won: true, player_rounds: 2, enemy_rounds: 0, ..Default::default() };
    let before = strength(&s.player_model);
    let sum = s.evolve(&won, &a, &cfg, |_| {});
    let after = strength(&s.player_model);
    eprintln!("won: ghost sim {:?} strength {before:.2} -> {after:.2}", sum.ghost_player_win);
    if sum.ghost_player_win.unwrap() < 1.0 {
        assert!(after >= before - 0.05, "ghost should not get weaker after the player won: {before} -> {after}");
    }

    let lost = MatchReport { player_won: false, player_rounds: 0, enemy_rounds: 2, ..Default::default() };
    let before = strength(&s.player_model);
    let sum = s.evolve(&lost, &a, &cfg, |_| {});
    let after = strength(&s.player_model);
    eprintln!("lost: ghost sim {:?} strength {before:.2} -> {after:.2}", sum.ghost_player_win);
    if sum.ghost_player_win.unwrap() > 0.0 {
        assert!(after <= before + 0.05, "ghost should not get stronger after the player lost: {before} -> {after}");
    }
}

/// A trainer checkpoint saved before the maps grew (8x8 route grid) must load with grids
/// resized, or the trainer's ghost sims index past the end of the route table and panic.
#[test]
fn old_trainer_checkpoint_loads_with_current_grids() {
    use brain::trainer::Trainer;
    let mut t = Trainer::new(BrainState::new(7));
    t.state.player_model.routes = vec![0.0; 8 * 8 * 8 * 8];
    t.state.player_model.heat = vec![0.0; 32 * 32];
    let dir = std::env::temp_dir().join(format!("shroud-trainer-{}", std::process::id()));
    let path = dir.join("trainer.json");
    t.save(&path).unwrap();
    let back = Trainer::load(&path).expect("checkpoint loads");
    let fresh = PlayerModel::default();
    assert_eq!(back.state.player_model.routes.len(), fresh.routes.len());
    assert_eq!(back.state.player_model.heat.len(), fresh.heat.len());
    let _ = std::fs::remove_dir_all(dir);
}
