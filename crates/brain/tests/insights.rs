//! Insight accuracy (docs/research/adaptive-squad-ai.md, section 6.6): synthetic players with
//! planted weaknesses; the generator must find them and never invent others.

use brain::arena::Arena;
use brain::evolve::{BrainState, EnemyReport, MatchReport, MatchSummary};
use brain::insights::{InsightKind, MatchPosterior, beta_cdf, beta_quantile, insights_v2, squad_card, trend_insights};
use brain::math::rng_from_seed;
use brain::player_model::{Contrast, CoverState, DuelOutcome, DuelRecord, MatchTelemetry, PeekSide, PlayerModel};
use rand::Rng;

const TOWER: &str = "the center tower";
const ZONES: [(&str, f32); 7] = [
    (TOWER, 0.35),
    ("the north yard", 0.15),
    ("the south yard", 0.15),
    ("the west lane", 0.1),
    ("the east lane", 0.1),
    ("the inner west side", 0.075),
    ("the inner east side", 0.075),
];

/// One synthetic match. Base duel win rate 55%; if `planted`, left peeks at the tower win 25%.
fn synth_match(rng: &mut impl Rng, planted: bool) -> MatchTelemetry {
    let mut t = MatchTelemetry::default();
    t.samples = 600.0;
    t.rounds = 6;
    for i in 0..40 {
        let mut x: f32 = rng.random();
        let (zi, zone) = ZONES
            .iter()
            .enumerate()
            .find(|(_, (_, w))| {
                x -= w;
                x <= 0.0
            })
            .map_or((0, TOWER), |(i, (z, _))| (i, *z));
        let peek = match rng.random::<f32>() {
            p if p < 0.4 => PeekSide::Left,
            p if p < 0.8 => PeekSide::Right,
            _ => PeekSide::None,
        };
        let follow_up = rng.random::<f32>() < 0.3;
        let weak = planted && zone == TOWER && peek == PeekSide::Left;
        let p_win = if weak { 0.25 } else { 0.55 };
        let outcome = if rng.random::<f32>() < 0.1 {
            DuelOutcome::Disengage
        } else if rng.random::<f32>() < p_win {
            DuelOutcome::Win
        } else {
            DuelOutcome::Loss
        };
        t.push_duel(DuelRecord {
            t: i as f32 * 10.0,
            zone: zone.into(),
            cover: if peek == PeekSide::None { CoverState::Exposed } else { CoverState::TallEdge },
            peek,
            cover_block: (peek != PeekSide::None).then_some(zi as u16 + 1),
            range: rng.random_range(3.0..30.0),
            enemies_visible: [1, 1, 1, 1, 1, 1, 2, 2, 2, 3][rng.random_range(0..10)],
            crouched: rng.random::<f32>() < 0.3,
            follow_up,
            repeek: follow_up && rng.random::<bool>(),
            outcome,
            ..Default::default()
        });
        t.reaction(0.3 + 0.1 * rng.random::<f32>());
    }
    t
}

fn is_planted(kind: &InsightKind) -> bool {
    match kind {
        InsightKind::Situation(g) => g.zone.as_deref() == Some(TOWER) && g.peek == Some(PeekSide::Left),
        InsightKind::Contrast(Contrast::Side(b)) => *b == 1, // the tower's block in synth_match
        _ => false,
    }
}

fn run(seed: u64, planted: bool) -> Vec<(InsightKind, String)> {
    let arena = Arena::foundry();
    let mut rng = rng_from_seed(seed);
    let mut m = PlayerModel::default();
    for _ in 0..6 {
        m.ingest(&synth_match(&mut rng, planted));
    }
    insights_v2(&m, &arena, &[]).into_iter().map(|i| (i.kind, i.text)).collect()
}

#[test]
fn planted_weakness_is_found_with_high_precision() {
    let (mut tp, mut fp, mut found) = (0, 0, 0);
    for seed in 0..20 {
        let out = run(1000 + seed, true);
        let hits = out.iter().filter(|(k, _)| is_planted(k)).count();
        tp += hits;
        fp += out.len() - hits;
        if out.iter().any(|(k, _)| matches!(k, InsightKind::Situation(_)) && is_planted(k)) {
            found += 1;
        }
        for (k, text) in &out {
            assert!(text.is_ascii(), "non-ASCII insight: {text}");
            if !is_planted(k) {
                eprintln!("seed {seed}: false insight: {text}");
            }
        }
    }
    let precision = tp as f32 / (tp + fp).max(1) as f32;
    eprintln!("precision {precision:.2} ({tp} tp, {fp} fp), recall {found}/20");
    assert!(precision >= 0.9, "precision {precision}");
    // ~15 effective duels in the planted cell: power is limited by design (precision first).
    assert!(found >= 10, "planted weakness found in only {found}/20 seeds");
}

#[test]
fn no_weakness_means_no_insights() {
    for seed in 0..20 {
        let out = run(5000 + seed, false);
        assert!(out.is_empty(), "seed {seed}: invented {:?}", out.iter().map(|o| &o.1).collect::<Vec<_>>());
    }
}

#[test]
fn trends_need_significance() {
    let p = |w: f32, n: f32, rt: f32| MatchPosterior { duel_wins: w, duels: n, rt_n: 20.0, rt_mean_log: rt.ln(), rt_var_log: 0.04, hits: 0.0, shots: 0.0 };
    // Big, well-sampled change in reaction time: reported.
    let h = vec![p(10.0, 20.0, 0.40), p(10.0, 20.0, 0.40), p(10.0, 20.0, 0.30)];
    let t = trend_insights(&h);
    assert!(t.iter().any(|i| i.text.contains("reaction time went from 400 to 300 ms")), "{t:?}");
    // Noise-sized change: nothing.
    let h = vec![p(10.0, 20.0, 0.40), p(11.0, 20.0, 0.39)];
    assert!(trend_insights(&h).is_empty());
}

#[test]
fn beta_numerics() {
    assert!((beta_cdf(1.0, 1.0, 0.3) - 0.3).abs() < 0.01);
    assert!((beta_quantile(5.0, 5.0, 0.5) - 0.5).abs() < 0.01);
    assert!(beta_cdf(2.0, 8.0, 0.5) > 0.97);
}

#[test]
fn card_is_ascii_and_complete() {
    let arena = Arena::foundry();
    let mut rng = rng_from_seed(3);
    let mut before = PlayerModel::default();
    for _ in 0..4 {
        before.ingest(&synth_match(&mut rng, true));
    }
    let t = synth_match(&mut rng, true);
    let mut e = EnemyReport::default();
    e.goal_time[2] = 30.0; // flank
    e.goal_time[5] = 20.0; // ambush
    e.kills_by_goal[2] = 2;
    let after = {
        let mut m = before.clone();
        m.ingest(&t);
        m
    };
    let top = insights_v2(&after, &arena, &[]).into_iter().next();
    let c = squad_card(&before, &arena, &t, &[e], 2, 4, top.as_ref());
    for s in [&c.learned, &c.did, &c.worked, &c.counter] {
        assert!(!s.is_empty() && s.is_ascii(), "{c:?}");
    }
    assert!(c.did.contains("flanking"), "{}", c.did);
    assert!(c.worked.contains("4 of 6 rounds"), "{}", c.worked);
}

#[test]
fn old_saves_still_load() {
    let arena = Arena::foundry();
    let mut s = BrainState::new(7);
    s.history.push(MatchSummary {
        generation: 1,
        player_won: true,
        player_rounds: 4,
        enemy_rounds: 2,
        sim_win_rate: 0.5,
        avg_fitness: 1.0,
        stats: Default::default(),
        posterior: Default::default(),
        squad: vec![],
        outcome_gap: None,
        calibration_error: None,
    });
    let mut v = serde_json::to_value(&s).unwrap();
    // Strip everything this change added: an old save has none of it.
    v["player_model"].as_object_mut().unwrap().remove("situations");
    v["player_model"].as_object_mut().unwrap().remove("ghost");
    v["history"][0].as_object_mut().unwrap().remove("posterior");
    let back: BrainState = serde_json::from_value(v).expect("old brain.json loads");
    assert_eq!(back.history.len(), 1);

    let mut r = serde_json::to_value(MatchReport { enemies: vec![EnemyReport::default()], ..Default::default() }).unwrap();
    for k in ["duels", "trajectory", "shot_log", "reactions", "rounds", "last_route"] {
        r["telemetry"].as_object_mut().unwrap().remove(k);
    }
    for k in ["goal_time", "kills_by_goal"] {
        r["enemies"][0].as_object_mut().unwrap().remove(k);
    }
    let rep: MatchReport = serde_json::from_value(r).expect("old report loads");
    // And evolving on an old-style report still works.
    let mut s2 = back;
    let _ = s2.evolve(&rep, &arena, &brain::evolve::EvolveConfig { budget: std::time::Duration::from_millis(1), max_neat_generations: 1, ..Default::default() }, |_| {});
}
