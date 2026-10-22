//! Squad tactics added for 5-6 enemy squads: learned play selection (bandit), anchors that
//! pre-fire repeated peeks, breachers with frags and flashbangs, difficulty plumbing.
use brain::arena::Arena;
use brain::bandit::TacticsLearner;
use brain::difficulty::Difficulty;
use brain::evolve::BrainState;
use brain::genome::Traits;
use brain::grenade::GrenadeKind;
use brain::math::V2;
use brain::mind::{EnemyMind, MindCtx, Observation};
use brain::player_model::PlayerModel;
use brain::sim::DT;
use brain::squad::{Callout, MemberView, Order, Role, SquadCoordinator, ThrowOrder, UtilityBudget};

fn traits() -> Traits {
    Traits { aggression: 0.7, team_cohesion: 0.7, flank_bias: 0.3, preferred_range: 15.0, ..Traits::default() }
}

fn member(pos: V2, last_known: V2, last_seen: f32) -> MemberView {
    MemberView { pos, alive: true, hp: 1.0, last_known, last_seen, heard: None, look: (last_known - pos).norm(), traits: traits(), bark: None }
}

struct Run {
    orders: Vec<Vec<Option<Order>>>,
    calls: Vec<Callout>,
    throws: Vec<ThrowOrder>,
}

/// Tick a coordinator with members standing still; `seen(t)` says whether they all see the
/// player (at `player(t)`) at time t.
fn drive(coord: &mut SquadCoordinator, a: &Arena, pos: &[V2], secs: f32, t0: f32, seen: impl Fn(f32) -> bool, player: impl Fn(f32) -> V2) -> Run {
    let model = PlayerModel::default();
    let n = pos.len();
    let mut last_seen = vec![99.0f32; n];
    let mut last_known = vec![player(0.0); n];
    let mut run = Run { orders: Vec::new(), calls: Vec::new(), throws: Vec::new() };
    let mut t = t0;
    while t < t0 + secs {
        t += DT;
        for i in 0..n {
            if seen(t) {
                last_seen[i] = 0.0;
                last_known[i] = player(t);
            } else {
                last_seen[i] += DT;
            }
        }
        let views: Vec<MemberView> = (0..n).map(|i| member(pos[i], last_known[i], last_seen[i])).collect();
        run.orders.push(coord.tick(DT, &views, a, &model).to_vec());
        run.calls.extend(coord.drain_callouts());
        run.throws.extend(coord.take_throws());
    }
    run
}

/// Members on the east side with a clear view of the crates north of the south yard.
fn squad_pos() -> Vec<V2> {
    vec![V2::new(10.0, -5.0), V2::new(12.0, -8.0), V2::new(11.0, -2.0), V2::new(9.0, -9.0)]
}

#[test]
fn repeated_peeks_get_an_anchor_that_prefires() {
    let a = Arena::foundry();
    let spot = V2::new(7.0, 13.0);
    let pos = squad_pos();
    for p in &pos {
        assert!(a.line_of_sight(*p, false, spot, false), "setup: {p:?} must see the spot");
    }
    let mut coord = SquadCoordinator::new(&a, pos.len());
    coord.utility = UtilityBudget::NONE;
    // Peek 1 s, duck 1.5 s, repeated: three peeks = two re-peeks, then he ducks again.
    let cycle = 2.5;
    let seen = |t: f32| t < 3.0 * cycle && (t % cycle) < 1.0;
    let run = drive(&mut coord, &a, &pos, 3.0 * cycle + 2.5, 0.0, seen, |_| spot);
    assert!(coord.peeks.iter().any(|s| s.pos.dist(spot) < 1.0 && s.count >= 2), "peeks: {:?}", coord.peeks);
    let last_lost: Vec<&Vec<Option<Order>>> = run.orders.iter().skip((2.0 * cycle / DT) as usize + 25).collect();
    let anchors: Vec<Order> = last_lost.iter().flat_map(|o| o.iter().flatten().copied()).filter(|o| o.role == Role::Anchor).collect();
    assert!(!anchors.is_empty(), "an anchor holds the re-peeked angle");
    assert!(anchors.iter().all(|o| o.watch.is_some_and(|w| w.dist(spot) < 1.0) && o.sticky));
    let pre: Vec<V2> = anchors.iter().filter_map(|o| o.prefire).collect();
    assert!(!pre.is_empty(), "the anchor pre-fires when he is due back");
    assert!(pre.iter().all(|p| p.dist(spot) < 1.0));
    // Not the whole time he is gone: only in a window around his usual delay.
    assert!(pre.len() < anchors.len());
    assert!(run.calls.iter().any(|c| c.text.contains("re-peeking that crate")), "{:?}", run.calls);
}

#[test]
fn single_peeks_do_not_prefire() {
    let a = Arena::foundry();
    let spot = V2::new(7.0, 13.0);
    let pos = squad_pos();
    let mut coord = SquadCoordinator::new(&a, pos.len());
    coord.utility = UtilityBudget::NONE;
    // He peeks from a new spot each time (8 m apart): no re-peeks.
    let run = drive(&mut coord, &a, &pos, 8.0, 0.0, |t| (t % 2.5) < 1.0, |t| spot + V2::new(-8.0 * (t / 2.5).floor(), 0.0));
    assert!(run.orders.iter().flatten().flatten().all(|o| o.prefire.is_none()));
}

#[test]
fn mind_prefires_only_at_the_spot_with_spread() {
    let a = Arena::foundry();
    let model = PlayerModel::default();
    let cx = MindCtx { arena: &a, model: &model, limits: Difficulty::Regular.limits(0) };
    let spot = V2::new(7.0, 13.0);
    let me = V2::new(10.0, -5.0);
    let mut mind: EnemyMind = BrainState::new(4).squad_minds(4).remove(0);
    let order = Order { role: Role::Anchor, goal: brain::utility::Goal::Hold, dest: None, watch: Some(spot), threat: None, sticky: true, via: None, prefire: Some(spot) };
    let mut fired = 0;
    let mut dirs = Vec::new();
    for k in 0..60 {
        mind.set_order(Some(order));
        let o = Observation {
            dt: DT,
            pos: me,
            hp: 1.0,
            ammo_frac: 1.0,
            player_pos: spot,
            player_hp: 1.0,
            time_since_seen: 2.0 + k as f32 * DT,
            squad_alert: brain::perception::Alert::Combat,
            ..Default::default()
        };
        let act = mind.think(&o, &cx);
        if act.fire {
            fired += 1;
            assert_eq!(act.prefire, Some(spot));
            dirs.push(act.shot_dir);
        }
    }
    // A short burst (the weapon's fire rate turns ~0.3 s of trigger into 3-4 rounds), not a hose.
    assert!(fired > 0 && fired as f32 * DT <= brain::squad::PREFIRE_BURST_S + DT, "fired {fired} ticks");
    let want = (spot - me).norm();
    assert!(dirs.iter().all(|d| d.dot(want) > 0.9), "shots go at the spot, with spread");
}

#[test]
fn camping_player_gets_fragged_and_called_out() {
    let a = Arena::foundry();
    // Camping behind the low crates south of the tower, in the south yard.
    let camp = V2::new(7.0, 11.9);
    let pos = squad_pos();
    let mut coord = SquadCoordinator::new(&a, pos.len());
    let run = drive(&mut coord, &a, &pos, 8.0, 0.0, |_| true, |_| camp);
    let frag = run.throws.iter().find(|t| t.kind == GrenadeKind::Frag).expect("a frag at the camper");
    assert!(frag.target.dist(camp) < 1.0);
    assert!(run.orders.iter().flatten().flatten().any(|o| o.role == Role::Breacher));
    assert!(run.calls.iter().any(|c| c.text.to_lowercase().contains("frag out")), "{:?}", run.calls);
    assert!(run.calls.iter().any(|c| c.text.starts_with("He's camping the south yard")), "{:?}", run.calls);
    // One frag per member per round, and the squad cooldown spaces them out.
    assert!(run.throws.iter().filter(|t| t.kind == GrenadeKind::Frag).count() <= 2);
}

#[test]
fn no_utility_budget_no_throws() {
    let a = Arena::foundry();
    let camp = V2::new(7.0, 11.9);
    let pos = squad_pos();
    let mut coord = SquadCoordinator::new(&a, pos.len());
    coord.utility = UtilityBudget::NONE;
    let run = drive(&mut coord, &a, &pos, 10.0, 0.0, |_| true, |_| camp);
    assert!(run.throws.is_empty());
}

#[test]
fn forced_flashbang_is_thrown() {
    let a = Arena::foundry();
    let p = V2::new(7.0, 13.0);
    let pos = squad_pos();
    let mut coord = SquadCoordinator::new(&a, pos.len());
    coord.force_throw = Some(GrenadeKind::Flash);
    let run = drive(&mut coord, &a, &pos, 2.0, 0.0, |_| true, |_| p);
    let fl = run.throws.iter().find(|t| t.kind == GrenadeKind::Flash).expect("flash thrown");
    assert!(fl.target.dist(p) < 2.0);
    assert!(run.calls.iter().any(|c| c.text == "Flashbang!"));
}

#[test]
fn bandit_learns_from_engagement_windows() {
    let a = Arena::foundry();
    let pos = squad_pos();
    let p = V2::new(7.0, 13.0);
    let mut coord = SquadCoordinator::new(&a, pos.len());
    coord.utility = UtilityBudget::NONE;
    let before = coord.tactics.clone();
    // A long fight in which the squad keeps hitting him.
    let model = PlayerModel::default();
    let mut t = 0.0;
    while t < 25.0 {
        t += DT;
        let views: Vec<MemberView> = pos.iter().map(|q| member(*q, p, 0.0)).collect();
        coord.tick(DT, &views, &a, &model);
        if (t % 1.0) < DT {
            coord.player_damaged(10.0);
        }
    }
    coord.end_round(false);
    assert!(coord.tactics.episodes >= 2, "episodes {}", coord.tactics.episodes);
    assert_ne!(coord.tactics, before);
    assert!(!coord.last_scores.is_empty());
    let rewarded: f32 = coord.tactics.arms.iter().map(|a| a.reward_sum).sum();
    assert!(rewarded > 0.0, "damage dealt is a positive reward");
}

#[test]
fn bandit_persists_in_brain_state_and_old_saves_load() {
    let mut b = BrainState::new(3);
    b.tactics.update(brain::squad::Play::Trade, &[1.0; brain::bandit::CTX_DIM], 0.5);
    let json = serde_json::to_value(&b).unwrap();
    let back: BrainState = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back.tactics, b.tactics);
    // A brain.json from before the bandit existed.
    let mut old = json;
    old.as_object_mut().unwrap().remove("tactics");
    let back: BrainState = serde_json::from_value(old).unwrap();
    assert_eq!(back.tactics, TacticsLearner::default());
}

#[test]
fn six_minds_from_a_five_slot_squad() {
    let b = BrainState::new(9);
    let d = Difficulty::Elite;
    let p = d.params();
    let minds = b.squad_minds_for(11, p.squad_size, &d.limits(b.generation));
    assert_eq!(minds.len(), 6);
    // The extra member reuses a slot's genomes.
    assert_eq!(minds[5].trait_id, minds[0].trait_id);
    assert_eq!(brain::squad::callsign(5), "Foxtrot");
    // Difficulty limits apply to decoded traits (slower reactions on Recruit).
    let rec = b.squad_minds_for(11, 1, &Difficulty::Recruit.limits(0)).remove(0);
    let eli = b.squad_minds_for(11, 1, &Difficulty::Elite.limits(0)).remove(0);
    assert!(rec.traits.reaction_time >= eli.traits.reaction_time);
}

#[test]
fn attack_tokens_respect_max_attackers() {
    let a = Arena::foundry();
    let pos = vec![V2::new(10.0, -5.0), V2::new(12.0, -8.0), V2::new(16.0, -2.0), V2::new(9.0, -9.0), V2::new(14.0, -10.0), V2::new(18.0, -6.0)];
    let mut coord = SquadCoordinator::new(&a, pos.len());
    coord.max_tokens = 2;
    drive(&mut coord, &a, &pos, 2.0, 0.0, |_| true, |_| V2::new(7.0, 13.0));
    assert_eq!(coord.tokens.iter().filter(|t| **t).count(), 2);
}
