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
    assert!(m.insights(&a).iter().any(|s| s.contains("south yard")), "{:?}", m.insights(&a));
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
    let limits = Limits::for_generation(0);
    let mut s = BrainState::new(11);
    let gen0 = s.clone();
    let cfg = EvolveConfig { budget: Duration::from_secs(60), max_neat_generations: 4, ..Default::default() };
    for _ in 0..3 {
        s.evolve(&MatchReport::default(), &a, &cfg, |_| {});
    }
    let score = |b: &BrainState| -> (f32, f32) {
        let (mut wins, mut dmg) = (0.0, 0.0);
        for seed in 0..24 {
            let r = sim::run_round(&a, &model, limits, b.squad_minds(seed), 1000 + seed);
            if !r.player_won {
                wins += 1.0;
            }
            dmg += r.enemies.iter().map(|e| e.damage_dealt).sum::<f32>();
        }
        (wins / 24.0, dmg / 24.0)
    };
    let (w0, d0) = score(&gen0);
    let (w1, d1) = score(&s);
    eprintln!("gen0: win {w0:.2} dmg {d0:.1} | evolved: win {w1:.2} dmg {d1:.1}");
    assert!(d1 > d0 * 1.1 || w1 > w0 + 0.1, "evolved squad should be stronger");
}

