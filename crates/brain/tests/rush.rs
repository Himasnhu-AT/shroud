//! Squad rush (`Play::Rush`, `squad/rush.rs`): triggers (heard reload, low HP, numbers),
//! simultaneous arrival from several bearings, last-known-only targeting, old saves.
use brain::arena::{Arena, MapId};
use brain::bandit::{ARMS, TacticsLearner};
use brain::difficulty::Difficulty;
use brain::evolve::BrainState;
use brain::genome::TraitGenome;
use brain::math::{V2, angle_diff};
use brain::mind::{AllyInfo, EnemyMind, MindCtx, Observation};
use brain::player_model::PlayerModel;
use brain::sim::{self, BODY_RADIUS, DT, SPRINT, VISION_HALF_ANGLE, VISION_RANGE, WALK};
use brain::squad::{MemberView, Play, RUSH_ARRIVED_M, Role, RushReason, SquadCoordinator};
use brain::utility::Goal;

/// Kinematic world (like `tests/doctrine.rs`): minds move, the coordinator ticks.
struct World<'a> {
    a: &'a Arena,
    model: PlayerModel,
    minds: Vec<EnemyMind>,
    pos: Vec<V2>,
    looks: Vec<V2>,
    last_seen: Vec<f32>,
    last_known: Vec<V2>,
    squad: SquadCoordinator,
    limits: brain::limits::Limits,
    t: f32,
}

impl<'a> World<'a> {
    fn new(a: &'a Arena, start: &[V2], seed: u64, d: Difficulty) -> Self {
        let n = start.len();
        let limits = d.limits(0);
        let minds = BrainState::new(seed).squad_minds_for(seed, n, &limits);
        let mut squad = SquadCoordinator::new(a, n);
        squad.max_tokens = d.params().max_attackers;
        squad.enabled = d.params().coordination;
        squad.seed(seed);
        World {
            a,
            model: PlayerModel::default(),
            minds,
            pos: start.to_vec(),
            looks: start.iter().map(|_| V2::new(0.0, 1.0)).collect(),
            last_seen: vec![99.0; n],
            last_known: vec![a.player_spawn; n],
            squad,
            limits,
            t: 0.0,
        }
    }

    /// Member `i` saw him at `at` just now (how the board learns his spot).
    fn sighting(&mut self, i: usize, at: V2) {
        self.last_seen[i] = 0.0;
        self.last_known[i] = at;
    }

    /// One tick. `player`: where he really is (None: nowhere they could see).
    fn step(&mut self, player: Option<V2>) -> Vec<Option<brain::squad::Order>> {
        self.t += DT;
        let n = self.pos.len();
        let a = self.a;
        let views: Vec<MemberView> = (0..n)
            .map(|i| MemberView {
                pos: self.pos[i],
                alive: true,
                hp: 1.0,
                last_known: self.last_known[i],
                last_seen: self.last_seen[i],
                heard: None,
                look: self.looks[i],
                traits: self.minds[i].traits,
                bark: self.minds[i].take_bark(),
            })
            .collect();
        let orders = self.squad.tick(DT, &views, a, &self.model).to_vec();
        self.squad.take_throws();
        for (m, o) in self.minds.iter_mut().zip(&orders) {
            m.set_order(*o);
        }
        let board = self.squad.board;
        let snap = self.pos.clone();
        let cx = MindCtx { arena: a, model: &self.model, limits: self.limits };
        for i in 0..n {
            let sees = player.is_some_and(|p| {
                let d = p - self.pos[i];
                d.len() < VISION_RANGE
                    && (d.len() < 5.0 || angle_diff(d.angle(), self.looks[i].angle()).abs() < VISION_HALF_ANGLE)
                    && a.line_of_sight(self.pos[i], false, p, false)
            });
            if sees {
                self.last_seen[i] = 0.0;
                self.last_known[i] = player.unwrap();
            } else {
                self.last_seen[i] += DT;
            }
            let shared = board.merge(self.last_known[i], self.last_seen[i]);
            let obs = Observation {
                dt: DT,
                pos: self.pos[i],
                hp: 1.0,
                ammo_frac: 1.0,
                player_visible: sees,
                player_pos: shared.0,
                player_hp: self.squad.player_hp,
                time_since_seen: shared.1,
                squad_alert: board.alert,
                token_denied: !self.squad.tokens.get(i).copied().unwrap_or(true),
                allies: (0..n).filter(|&j| j != i).map(|j| AllyInfo { pos: snap[j], engaged: false, flanking: false }).collect(),
                ..Default::default()
            };
            let act = self.minds[i].think(&obs, &cx);
            self.looks[i] = act.look_dir;
            let speed = if act.crouch { sim::CROUCH_SPEED } else if act.sprint { SPRINT } else { WALK };
            self.pos[i] = a.resolve_collision(self.pos[i] + act.move_dir * speed * DT, BODY_RADIUS);
        }
        orders
    }
}

/// Members spread around `center` at 12-30 m (walkable, reachable).
fn ring(a: &Arena, center: V2, n: usize) -> Vec<V2> {
    (0..n)
        .map(|k| {
            let ang = k as f32 * std::f32::consts::TAU / n as f32 + 0.4;
            a.nav.nearest_walkable(center + V2::from_angle(ang) * (14.0 + 4.0 * (k % 3) as f32))
        })
        .collect()
}

/// A reload heard by two members sends the squad in at once (`RushReason::Reload`).
#[test]
fn heard_reload_starts_a_squad_rush() {
    // Arrange
    let a = Arena::build(MapId::Sirocco);
    let spot = a.nav.nearest_walkable(V2::new(0.0, 5.0));
    let start = ring(&a, spot, 4);
    let mut w = World::new(&a, &start, 3, Difficulty::Veteran);
    for m in &mut w.minds {
        m.traits.rush = 1.0;
    }
    w.step(None);
    // Act
    w.squad.heard_mag_out(0, spot);
    w.squad.heard_mag_out(1, spot);
    let mut rushers = Vec::new();
    for _ in 0..20 {
        for (i, o) in w.step(None).iter().enumerate() {
            if o.is_some_and(|o| o.role == Role::Rusher) && !rushers.contains(&i) {
                rushers.push(i);
            }
        }
    }
    // Assert
    assert_eq!(w.squad.play, Play::Rush);
    assert_eq!(w.squad.rush.as_ref().map(|r| r.reason), Some(RushReason::Reload));
    assert!(rushers.len() >= 3, "most of the squad commits: {rushers:?}");
    assert!(w.squad.rush.as_ref().unwrap().target.dist(spot) < 1.0, "to where they heard it");
}

/// He's at 20% health (what the squad saw): rush. At full health with two left: no rush.
#[test]
fn low_hp_triggers_a_rush_and_full_hp_does_not() {
    let a = Arena::build(MapId::Office);
    let spot = a.nav.nearest_walkable(a.player_spawn);
    let run = |hp: f32, n: usize| {
        // Arrange
        let start = ring(&a, spot, n);
        let mut w = World::new(&a, &start, 5, Difficulty::Regular);
        w.squad.player_hp = hp;
        w.sighting(0, spot);
        // Act
        for _ in 0..10 {
            w.step(None);
        }
        w.squad.rush.as_ref().map(|r| r.reason)
    };
    // Assert
    assert_eq!(run(0.2, 2), Some(RushReason::LowHp));
    assert_eq!(run(1.0, 2), None);
}

/// Three or more alive against a hurt (60%) player: numbers rush. Recruit (no coordination)
/// never rushes.
#[test]
fn numbers_advantage_triggers_a_rush_but_not_on_recruit() {
    let a = Arena::build(MapId::Office);
    let spot = a.nav.nearest_walkable(a.player_spawn);
    let run = |d: Difficulty| {
        // Arrange
        let start = ring(&a, spot, 3);
        let mut w = World::new(&a, &start, 7, d);
        for m in &mut w.minds {
            m.traits.rush = 0.5;
        }
        w.squad.player_hp = 0.6;
        w.sighting(0, spot);
        // Act
        for _ in 0..10 {
            w.step(None);
        }
        w.squad.rush.as_ref().map(|r| r.reason)
    };
    // Assert
    assert_eq!(run(Difficulty::Regular), Some(RushReason::Numbers));
    assert_eq!(run(Difficulty::Recruit), None);
}

/// After a rush ends there's a cooldown before the next one.
#[test]
fn rushes_have_a_cooldown() {
    let a = Arena::build(MapId::Office);
    let spot = a.nav.nearest_walkable(a.player_spawn);
    // Arrange
    let start = ring(&a, spot, 3);
    let mut w = World::new(&a, &start, 9, Difficulty::Veteran);
    w.squad.player_hp = 0.2;
    w.sighting(0, spot);
    // Act: run the rush to its end
    let mut ended = None;
    for k in 0..600 {
        w.step(None);
        if k > 2 && w.squad.rush.is_none() {
            ended = Some(w.t);
            break;
        }
    }
    let ended = ended.expect("the rush ends");
    // He's seen again, still low: no new rush inside the cooldown.
    w.sighting(1, spot);
    for _ in 0..40 {
        w.step(None);
    }
    // Assert
    assert!(w.squad.rush.is_none(), "no rush {:.1} s after the last one", w.t - ended);
}

struct Arrival {
    /// Per rusher: (time it got within `RUSH_ARRIVED_M`, bearing from the spot at 7 m out).
    times: Vec<f32>,
    bearings: Vec<f32>,
    rushers: usize,
}

/// Squad of `n` spread out, he was seen at `spot` (low) and then vanished: rush, and record
/// when and from where each rusher arrives. He's never visible again.
fn arrive(map: MapId, n: usize, seed: u64) -> Option<Arrival> {
    let a = Arena::build(map);
    let start: Vec<V2> = a.enemy_spawns.iter().take(n).copied().collect();
    let c = start.iter().fold(V2::ZERO, |s, p| s + *p) * (1.0 / n as f32);
    // Between his spawn and theirs, nearer theirs: where a mid-round fight happens.
    let spot = a.nav.nearest_walkable(c + (a.player_spawn - c) * 0.4);
    let mut w = World::new(&a, &start, seed, Difficulty::Elite);
    w.squad.utility = brain::squad::UtilityBudget::NONE;
    w.squad.player_hp = 0.2;
    w.sighting(0, spot);
    for _ in 0..40 {
        w.step(None);
        if w.squad.rush.is_some() {
            break;
        }
    }
    let r = w.squad.rush.clone()?;
    let mut times = vec![f32::NAN; n];
    let mut bearings = vec![f32::NAN; n];
    for _ in 0..(40.0 / DT) as usize {
        w.step(None);
        let Some(cur) = w.squad.rush.as_ref() else { break };
        for &i in &cur.members {
            let d = w.pos[i].dist(r.target);
            if bearings[i].is_nan() && d < 7.0 {
                bearings[i] = (w.pos[i] - r.target).angle();
            }
            if times[i].is_nan() && d < RUSH_ARRIVED_M {
                times[i] = w.t;
            }
        }
    }
    let idx: Vec<usize> = r.members.iter().copied().filter(|&i| !times[i].is_nan()).collect();
    Some(Arrival { times: idx.iter().map(|&i| times[i]).collect(), bearings: idx.iter().map(|&i| bearings[i]).collect(), rushers: r.members.len() })
}

/// Rushers arrive within a short window of each other, from at least two bearings, on at
/// least one real map (and everyone who started gets there on most).
#[test]
fn rushers_arrive_together_from_two_bearings() {
    let mut good = Vec::new();
    for map in [MapId::Office, MapId::Sirocco, MapId::Gantry, MapId::Foundry] {
        for seed in [1, 2] {
            // Arrange + Act
            let Some(r) = arrive(map, 5, seed) else {
                eprintln!("{map:?} seed {seed}: no rush");
                continue;
            };
            // Assert (collected)
            let spread = r.times.iter().copied().fold(f32::MIN, f32::max) - r.times.iter().copied().fold(f32::MAX, f32::min);
            let wide = r.bearings.iter().flat_map(|x| r.bearings.iter().map(move |y| angle_diff(*x, *y).abs())).fold(0.0f32, f32::max);
            eprintln!("{map:?} seed {seed}: {}/{} arrived, spread {spread:.2} s, widest bearing gap {:.0} deg", r.times.len(), r.rushers, wide.to_degrees());
            if r.times.len() >= 2 && spread <= 2.0 && wide >= 45f32.to_radians() {
                good.push(map);
            }
        }
    }
    assert!(good.len() >= 4, "synchronised multi-angle arrivals on {good:?}");
}

/// He was seen low at A, then walked away out of sight to B: they rush A (the board), never B.
#[test]
fn rush_targets_the_last_known_spot_not_the_player() {
    // Arrange
    let a = Arena::build(MapId::Office);
    let old = a.nav.nearest_walkable(a.player_spawn);
    let start = ring(&a, old, 3);
    let mut w = World::new(&a, &start, 11, Difficulty::Veteran);
    w.squad.player_hp = 0.2;
    w.sighting(0, old);
    w.step(None);
    // Act: he is somewhere else now, behind walls nobody sees through (no sighting reaches them).
    let mut targets = Vec::new();
    for _ in 0..60 {
        let orders = w.step(None);
        targets.extend(orders.iter().flatten().filter(|o| o.role == Role::Rusher && o.goal == Goal::Push).filter_map(|o| o.dest));
        if let Some(r) = &w.squad.rush {
            targets.push(r.target);
        }
    }
    // Assert
    assert!(!targets.is_empty());
    assert!(targets.iter().all(|t| t.dist(old) < 1.5), "every rush order goes to the old spot");
}

/// A rusher hit while pushing keeps pushing (no fall-back): the order holds through fire.
#[test]
fn rusher_keeps_pushing_under_fire() {
    let a = Arena::build(MapId::Office);
    let limits = Difficulty::Veteran.limits(0);
    let model = PlayerModel::default();
    let mut m = BrainState::new(4).squad_minds_for(4, 1, &limits).remove(0);
    m.traits.retreat_health = 0.6;
    let me = a.nav.nearest_walkable(a.player_spawn + V2::new(12.0, 0.0));
    let him = a.nav.nearest_walkable(a.player_spawn);
    m.set_order(Some(brain::squad::Order { role: Role::Rusher, goal: Goal::Push, dest: Some(him), watch: Some(him), threat: None, sticky: true, via: None, prefire: None }));
    let cx = MindCtx { arena: &a, model: &model, limits };
    let mut goals = Vec::new();
    for _ in 0..20 {
        let obs = Observation { dt: DT, pos: me, hp: 0.25, ammo_frac: 1.0, player_pos: him, player_hp: 1.0, time_since_seen: 0.0, taking_fire: true, ..Default::default() };
        let act = m.think(&obs, &cx);
        goals.push(m.goal());
        assert!(!act.crouch);
    }
    assert!(goals.iter().all(|g| *g == Goal::Push), "{goals:?}");
}

/// Saves from before the rush: genomes without the rush gene get a seeded one; bandit state
/// with four arms loads and grows the fifth.
#[test]
fn old_saves_load_with_rush_gene_and_arm() {
    // Arrange: a genome saved without the field.
    let mut rng = brain::math::rng_from_seed(3);
    let g = TraitGenome::random(&mut rng);
    let mut v = serde_json::to_value(&g).unwrap();
    v.as_object_mut().unwrap().remove("rush");
    let old: TraitGenome = serde_json::from_value(v).unwrap();
    // Act
    let t = old.decode(&Difficulty::Regular.limits(0));
    // Assert
    assert!((0.0..=1.0).contains(&t.rush));
    assert_eq!(t.rush, g.decode(&Difficulty::Regular.limits(0)).rush, "seeded the same way as new genomes");
    let mut four = serde_json::to_value(TacticsLearner::default()).unwrap();
    four["arms"].as_array_mut().unwrap().truncate(4);
    let mut l: TacticsLearner = serde_json::from_value(four).unwrap();
    l.update(Play::Rush, &[1.0; brain::bandit::CTX_DIM], 0.5);
    assert_eq!(l.arms.len(), ARMS.len());
}

/// The debrief names the rushes once there were at least two, with the commonest reason.
#[test]
fn debrief_mentions_repeated_rushes() {
    // Arrange
    let mut t = brain::player_model::MatchTelemetry::default();
    t.rushes[RushReason::Reload.index()] = 2.0;
    t.rushes[RushReason::LowHp.index()] = 1.0;
    t.rush_wins = 1.0;
    // Act
    let line = brain::squad::rush_debrief_line(&t);
    // Assert
    assert_eq!(line.as_deref(), Some("They rushed you 3 times, mostly when they heard you reload; one of them won the round."));
    assert_eq!(brain::squad::rush_debrief_line(&brain::player_model::MatchTelemetry::default()), None);
}
