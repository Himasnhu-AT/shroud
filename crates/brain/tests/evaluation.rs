//! Evaluation hygiene (P0-3), team-spirit objective (P1-3) and background trainer (P1-4).
use brain::arena::Arena;
use brain::evolve::{BrainState, EvolveConfig, Halving, MatchReport, Parallel, RoundOutcome, round_seed, successive_halving};
use brain::limits::Limits;
use brain::sim;
use brain::trainer::{Trainer, TrainerConfig, TrainerHandle};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn unit(x: u64) -> f32 {
    (round_seed(x, 0) >> 40) as f32 / (1u64 << 24) as f32
}

/// Synthetic rounds: a shared per-round difficulty (what common random numbers share) plus
/// per-candidate luck. Candidate `n-1` has a planted edge: win probability 0.63 vs 0.5.
/// Returns (won, shaping); shaping is the round's margin, like damage it tracks strength noisily.
fn synthetic(seed: u64, c: usize, round: u64, n: usize) -> (bool, f32) {
    let threshold = if c == n - 1 { 0.6 } else { 0.5 };
    let shared = unit(seed ^ round.wrapping_mul(0x1234_5678));
    let own = unit(seed ^ (c as u64 + 1).wrapping_mul(0xABCD_EF01) ^ round.wrapping_mul(0x5555));
    let x = 0.75 * shared + 0.25 * own;
    (x < threshold, threshold - x)
}

#[test]
fn halving_with_crn_picks_planted_better_squad() {
    let n = 9;
    let par = Parallel { threads: 1, ..Default::default() };
    let plan = Halving { base_rounds: 3, rungs: 3 };
    let trials = 200u64;
    let (mut new_ok, mut old_ok) = (0, 0);
    for seed in 0..trials {
        let res = successive_halving(n, plan, &par, |c, r| {
            let (won, shaping) = synthetic(seed, c, r as u64, n);
            RoundOutcome { won, shaping, detail: () }
        })
        .unwrap();
        // Full ties go to the lower index, so the planted squad (last) never wins one.
        new_ok += usize::from(res.ranking[0] == n - 1);
        // Old scheme: each candidate gets 3 independent rounds (no shared seeds); same ordering.
        let score = |c: usize| {
            (0..3u64).map(|r| synthetic(seed.wrapping_mul(31) + c as u64 * 1000 + r, c, r, n)).fold((0, 0.0f32), |(w, s), (won, m)| (w + u32::from(won), s + m))
        };
        let best_old = (0..n).max_by(|&a, &b| score(a).0.cmp(&score(b).0).then(score(a).1.total_cmp(&score(b).1)).then(b.cmp(&a))).unwrap();
        old_ok += usize::from(best_old == n - 1);
    }
    let (new_rate, old_rate) = (new_ok as f32 / trials as f32, old_ok as f32 / trials as f32);
    eprintln!("planted squad selected: CRN + halving {new_rate:.2}, independent 3 rounds {old_rate:.2}");
    assert!(new_rate >= 0.9, "{new_rate}");
    assert!(new_rate > old_rate + 0.2);
}

/// With common random numbers two identical squads get identical scores, whatever the threads.
#[test]
fn common_random_numbers_are_deterministic() {
    let a = Arena::yard();
    let b = BrainState::new(21);
    let limits = Limits::for_generation(1);
    let plan = Halving { base_rounds: 2, rungs: 2 };
    let run = |threads| {
        let par = Parallel { threads, ..Default::default() };
        successive_halving(3, plan, &par, |c, r| {
            let seed = round_seed(77, r);
            let rr = sim::run_round(&a, &b.player_model, limits, b.squad_minds(seed), seed);
            let _ = c; // every candidate is the same squad
            RoundOutcome { won: !rr.player_won, shaping: rr.enemies.iter().map(sim::EnemyStats::fitness).sum::<f32>(), detail: rr.duration }
        })
        .unwrap()
    };
    let (r1, r4) = (run(1), run(4));
    assert_eq!(r1.scores[0], r1.scores[1]);
    assert_eq!(r1.details[0][..2], r1.details[2][..]);
    assert_eq!(r1.scores, r4.scores);

    let cfg = EvolveConfig { budget: Duration::from_secs(600), max_neat_generations: 2, threads: 1, ..Default::default() };
    let (mut s1, mut s2) = (BrainState::new(9), BrainState::new(9));
    s1.evolve(&MatchReport::default(), &a, &cfg, |_| {});
    s2.evolve(&MatchReport::default(), &a, &EvolveConfig { threads: 3, ..cfg.clone() }, |_| {});
    assert_eq!(serde_json::to_value(&s1).unwrap(), serde_json::to_value(&s2).unwrap());
    assert!(s1.sim_rounds > 0 && s1.champion.is_some());
}

fn small_cfg() -> TrainerConfig {
    TrainerConfig { eval: EvolveConfig { squads: 4, rounds_per_genome: 2, rungs: 2, ..Default::default() }, threads: 2, ..Default::default() }
}

#[test]
fn trainer_resumes_from_checkpoint_bit_for_bit() {
    let a = Arena::yard();
    let cfg = small_cfg();
    let mut straight = Trainer::new(BrainState::new(4));
    for _ in 0..4 {
        straight.step(&a, &cfg, None).unwrap();
    }
    let dir = std::env::temp_dir().join(format!("shroud-trainer-{}", std::process::id()));
    let path = dir.join("trainer.json");
    let mut first = Trainer::new(BrainState::new(4));
    for _ in 0..2 {
        first.step(&a, &cfg, None).unwrap();
    }
    first.save(&path).unwrap();
    let mut resumed = Trainer::load(&path).unwrap();
    for _ in 0..2 {
        resumed.step(&a, &cfg, None).unwrap();
    }
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(resumed.steps, 4);
    assert_eq!(serde_json::to_value(&straight).unwrap(), serde_json::to_value(&resumed).unwrap());

    // A cancelled step leaves the trainer untouched.
    let before = serde_json::to_value(&resumed).unwrap();
    assert!(resumed.step(&a, &cfg, Some(&AtomicBool::new(true))).is_none());
    assert_eq!(before, serde_json::to_value(&resumed).unwrap());
}

#[test]
fn background_trainer_cancels_promptly_and_merges() {
    let dir = std::env::temp_dir().join(format!("shroud-trainer-bg-{}", std::process::id()));
    let path = dir.join("trainer.json");
    let live = BrainState::new(6);
    let h = TrainerHandle::spawn(Trainer::new(live.clone()), TrainerConfig { checkpoint_every: 1, ..small_cfg() }, Some(path.clone())).unwrap();
    let t0 = Instant::now();
    while h.progress.steps.load(std::sync::atomic::Ordering::Relaxed) < 2 && t0.elapsed() < Duration::from_secs(60) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let rounds = h.rounds();
    let t = Instant::now();
    let trainer = h.finish().unwrap();
    let stop_ms = t.elapsed().as_millis();
    eprintln!("trainer: {} steps, {rounds} rounds before cancel, stopped in {stop_ms} ms", trainer.steps);
    assert!(trainer.steps >= 2 && trainer.session_rounds >= rounds);
    assert!(stop_ms < 1000, "cancel took {stop_ms} ms");
    let saved = Trainer::load(&path).expect("checkpoint written");
    assert_eq!(saved.steps, trainer.steps);
    let _ = std::fs::remove_dir_all(dir);

    // Merge between matches: only into the brain it started from.
    let mut brain = live.clone();
    assert!(trainer.matches(&brain));
    assert!(brain.adopt(trainer.state.clone()));
    assert_eq!(brain.sim_rounds, trainer.state.sim_rounds);
    let champ = brain.champion.clone().unwrap();
    for (k, s) in brain.squad.iter().enumerate() {
        assert_eq!(brain.traits.genomes[s.trait_id].genes, champ.traits[k].genes, "deployed squad is the champion");
    }
    let mut moved_on = live.clone();
    moved_on.generation += 1;
    assert!(!moved_on.adopt(trainer.state));
}

/// Sim-vs-real is logged for the deployed squad after each real match.
#[test]
fn history_logs_sim_vs_real() {
    let a = Arena::yard();
    let cfg = EvolveConfig { budget: Duration::from_secs(600), max_neat_generations: 1, ..Default::default() };
    let mut s = BrainState::new(8);
    s.evolve(&MatchReport::default(), &a, &cfg, |_| {});
    let deployed = s.champion.as_ref().unwrap().score.win_rate();
    let report = MatchReport { player_rounds: 1, enemy_rounds: 3, ..Default::default() };
    let sum = s.evolve(&report, &a, &cfg, |_| {});
    let h = s.history.last().unwrap();
    assert_eq!(h.real_win, Some(0.75));
    assert_eq!(h.deployed_sim_win, Some(deployed));
    assert_eq!(sum.total_sim_rounds, s.sim_rounds);
    assert!(sum.sim_rounds > 0);
}
