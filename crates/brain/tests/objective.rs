//! Bomb defusal (squad side): pre-plant site setup, retake and defuse, save, defuser re-pick.
//!
//! Kinematic world as in `tests/squad_spread.rs` (no ghost, the player is never seen); the test
//! plays the game's part: `set_bomb` every tick and the defuse clock (`objective` rules).
use brain::arena::{Arena, MapId};
use brain::difficulty::Difficulty;
use brain::evolve::BrainState;
use brain::math::{V2, angle_diff};
use brain::mind::{AllyInfo, EnemyMind, MindCtx, Observation};
use brain::objective::{BLAST_MAX_M, DEFUSE_REACH, DEFUSE_S, PlantedBomb};
use brain::player_model::PlayerModel;
use brain::sim::{self, BODY_RADIUS, DT, SPRINT, VISION_HALF_ANGLE, VISION_RANGE, WALK};
use brain::squad::{MemberView, Play, Role, SquadCoordinator, bomb_site_dist};

struct World<'a> {
    a: &'a Arena,
    model: PlayerModel,
    minds: Vec<EnemyMind>,
    pos: Vec<V2>,
    looks: Vec<V2>,
    alive: Vec<bool>,
    last_seen: Vec<f32>,
    last_known: Vec<V2>,
    squad: SquadCoordinator,
    limits: brain::limits::Limits,
    /// The game's bomb state: (bomb, who is defusing).
    bomb: Option<PlantedBomb>,
    defusing: Option<usize>,
    defused: bool,
    /// Ticks on which some member wanted to defuse.
    wanted: usize,
}

impl<'a> World<'a> {
    fn new(a: &'a Arena, n: usize, seed: u64) -> Self {
        let d = Difficulty::Regular;
        let limits = d.limits(0);
        let minds = BrainState::new(seed).squad_minds_for(seed, n, &limits);
        let n = minds.len();
        let mut squad = SquadCoordinator::new(a, n);
        squad.seed(seed);
        World {
            a,
            model: PlayerModel::default(),
            minds,
            pos: a.enemy_spawns.iter().cycle().take(n).copied().collect(),
            looks: vec![V2::new(0.0, 1.0); n],
            alive: vec![true; n],
            last_seen: vec![99.0; n],
            last_known: vec![a.player_spawn; n],
            squad,
            limits,
            bomb: None,
            defusing: None,
            defused: false,
            wanted: 0,
        }
    }

    fn plant(&mut self, site: usize, fuse_left: f32) -> V2 {
        let pos = self.a.nav.nearest_walkable(self.a.sites[site].center());
        self.bomb = Some(PlantedBomb { pos, site, fuse_left, defuse_progress: 0.0 });
        pos
    }

    fn step(&mut self, player: Option<V2>) {
        let n = self.pos.len();
        let a = self.a;
        // The game's side: the bomb clock and the defuse (CONTRACT: `wants_defuse` + reach).
        if let Some(b) = self.bomb.as_mut() {
            b.fuse_left = (b.fuse_left - DT).max(0.0);
            let on = (0..n).find(|&i| self.alive[i] && self.squad.wants_defuse(i) && self.pos[i].dist(b.pos) <= DEFUSE_REACH);
            if on.is_some() {
                self.wanted += 1;
            }
            match on {
                Some(i) if self.defusing.is_none_or(|d| d == i) => {
                    self.defusing = Some(i);
                    b.defuse_progress += DT / DEFUSE_S;
                }
                _ => {
                    self.defusing = None;
                    b.defuse_progress = 0.0;
                }
            }
            if b.defuse_progress >= 1.0 {
                self.defused = true;
            }
        }
        self.squad.set_bomb(self.bomb);
        let views: Vec<MemberView> = (0..n)
            .map(|i| {
                if !self.alive[i] {
                    return MemberView::gone();
                }
                MemberView {
                    pos: self.pos[i],
                    alive: true,
                    hp: 1.0,
                    last_known: self.last_known[i],
                    last_seen: self.last_seen[i],
                    heard: None,
                    look: self.looks[i],
                    traits: self.minds[i].traits,
                    bark: self.minds[i].take_bark(),
                }
            })
            .collect();
        let orders = self.squad.tick(DT, &views, a, &self.model).to_vec();
        for (m, o) in self.minds.iter_mut().zip(&orders) {
            m.set_order(*o);
        }
        let board = self.squad.board;
        let snap = self.pos.clone();
        let cx = MindCtx { arena: a, model: &self.model, limits: self.limits };
        for i in 0..n {
            if !self.alive[i] {
                continue;
            }
            let sees = player.is_some_and(|p| {
                let d = p - self.pos[i];
                d.len() < VISION_RANGE
                    && (d.len() < 5.0 || angle_diff(d.angle(), self.looks[i].angle()).abs() < VISION_HALF_ANGLE)
                    && a.line_of_sight(self.pos[i], false, p, false)
            });
            if sees {
                self.last_seen[i] = 0.0;
                self.last_known[i] = player.unwrap_or_default();
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
                player_hp: 1.0,
                player_facing: 0.0,
                time_since_seen: shared.1,
                squad_alert: board.alert,
                token_denied: !self.squad.tokens.get(i).copied().unwrap_or(true),
                allies: (0..n).filter(|&j| j != i && self.alive[j]).map(|j| AllyInfo { pos: snap[j], engaged: false, flanking: false }).collect(),
                ..Default::default()
            };
            let act = self.minds[i].think(&obs, &cx);
            self.looks[i] = act.look_dir;
            let speed = if act.crouch { sim::CROUCH_SPEED } else if act.sprint { SPRINT } else { WALK };
            self.pos[i] = a.resolve_collision(self.pos[i] + act.move_dir * speed * DT, BODY_RADIUS);
        }
    }

    fn run(&mut self, secs: f32) {
        for _ in 0..(secs / DT) as usize {
            self.step(None);
        }
    }

    /// Alive members within `m` of each site's rectangle.
    fn on_sites(&self, m: f32) -> Vec<usize> {
        self.a.sites.iter().map(|s| (0..self.pos.len()).filter(|&i| self.alive[i] && bomb_site_dist(s, self.pos[i]) < m).count()).collect()
    }
}

#[test]
fn setup_puts_a_defender_on_both_sites_on_every_map() {
    for id in MapId::ALL {
        let a = Arena::build(id);
        assert!(a.sites.len() >= 2, "{id:?}: needs two sites");
        for n in [2usize, 4, 5] {
            // Arrange
            let mut w = World::new(&a, n, 7 + n as u64);
            // Act: within 30 s (the biggest maps are ~80 m across).
            let mut ok_at = None;
            for k in 0..(30.0 / DT) as usize {
                w.step(None);
                let on = w.on_sites(4.0);
                if on.iter().all(|&c| c >= 1) {
                    ok_at = Some(k as f32 * DT);
                    break;
                }
            }
            // Assert
            assert_eq!(w.squad.play, Play::Setup, "{id:?} n={n}");
            assert!(ok_at.is_some(), "{id:?} n={n}: sites not both held after 30 s (on sites: {:?}, pos {:?})", w.on_sites(4.0), w.pos);
            // ...and by 25 s everyone stands on its spot with a line on the entry it holds.
            w.run(25.0 - ok_at.unwrap_or(0.0));
            for i in 0..n {
                let (spot, choke) = w.squad.site.hold[i].expect("a spot");
                assert!(w.pos[i].dist(spot) < 2.5, "{id:?} n={n}: member {i} at {:?}, spot {spot:?} order {:?} goal {:?} all {:?} holds {:?}", w.pos[i], w.squad.orders[i], w.minds[i].debug.goal, w.pos, w.squad.site.hold);
                if w.squad.site.duty[i] != Some(brain::squad::Duty::Rotator) {
                    assert!(a.line_of_sight(spot, false, choke, false), "{id:?}: spot {spot:?} can't see entry {choke:?}");
                }
            }
        }
    }
}

#[test]
fn setup_stacks_the_site_he_plants_on_more() {
    // Arrange: he has planted B six times.
    let a = Arena::build(MapId::ALL[0]);
    let mut w = World::new(&a, 5, 3);
    for _ in 0..6 {
        w.model.record_plant(1);
    }
    // Act
    w.run(1.0);
    // Assert: 4 anchors + 1 rotator, three of them on B.
    let duty = &w.squad.site.duty;
    let on = |s: usize| duty.iter().filter(|d| **d == Some(brain::squad::Duty::Anchor(s))).count();
    assert_eq!(on(1), 3, "duties {duty:?}");
    assert_eq!(on(0), 1, "duties {duty:?}");
}

/// Plant on A after a short setup and run until defused (or the fuse ends).
fn retake(id: MapId, n: usize, seed: u64, fuse: f32) -> World<'static> {
    let a: &'static Arena = Box::leak(Box::new(Arena::build(id)));
    let mut w = World::new(a, n, seed);
    w.run(4.0);
    w.plant(0, fuse);
    w
}

#[test]
fn retake_assigns_a_defuser_who_defuses_unopposed_on_every_map() {
    for id in MapId::ALL {
        // Arrange
        let mut w = retake(id, 4, 11, brain::objective::FUSE_S);
        // Act
        let mut t = 0.0;
        while t < brain::objective::FUSE_S && !w.defused {
            w.step(None);
            t += DT;
        }
        // Assert
        assert_eq!(w.squad.play, Play::Retake, "{id:?}");
        let d = w.squad.defuser.expect("a defuser");
        assert!(w.defused, "{id:?}: not defused in the fuse; defuser {d} at {:?}, bomb {:?}, order {:?}", w.pos[d], w.bomb.map(|b| b.pos), w.squad.orders[d]);
        // wants_defuse held on the bomb for (at least) the whole defuse.
        assert!(w.wanted as f32 * DT >= DEFUSE_S - 0.1, "{id:?}: wanted {:.1}s", w.wanted as f32 * DT);
        assert_eq!(w.squad.orders[d].map(|o| o.role), Some(Role::Defuser));
    }
}

#[test]
fn retake_saves_when_the_fuse_cannot_be_beaten() {
    for id in MapId::ALL {
        // Arrange: less fuse than a defuse takes.
        let mut w = retake(id, 4, 5, DEFUSE_S - 2.0);
        let bomb = w.bomb.unwrap().pos;
        // Act
        for _ in 0..(1.0 / DT) as usize {
            w.step(None);
        }
        // Assert: nobody defuses, everyone heads out of the blast.
        assert!(w.squad.site.saving, "{id:?}");
        assert_eq!(w.squad.defuser, None);
        assert!((0..4).all(|i| !w.squad.wants_defuse(i)));
        for i in 0..4 {
            let o = w.squad.orders[i].expect("an order");
            assert_eq!(o.role, Role::Saving, "{id:?}");
            assert!(o.dest.is_some_and(|d| d.dist(bomb) >= BLAST_MAX_M), "{id:?}: save spot {:?} too close", o.dest);
        }
        w.run(10.0);
        for i in 0..4 {
            assert!(w.pos[i].dist(bomb) >= BLAST_MAX_M, "{id:?}: member {i} still in the blast ({:.1} m)", w.pos[i].dist(bomb));
        }
        assert_eq!(w.wanted, 0);
    }
}

#[test]
fn retake_repicks_the_defuser_when_he_dies() {
    for id in MapId::ALL {
        // Arrange: a defuser is picked.
        let mut w = retake(id, 4, 21, brain::objective::FUSE_S);
        for _ in 0..(1.0 / DT) as usize {
            w.step(None);
        }
        let first = w.squad.defuser.expect("a defuser");
        // Act: he dies.
        w.alive[first] = false;
        let mut t = 0.0;
        while t < brain::objective::FUSE_S - 1.0 && !w.defused {
            w.step(None);
            t += DT;
        }
        // Assert
        let second = w.squad.defuser.expect("a new defuser");
        assert_ne!(first, second, "{id:?}");
        assert!(w.defused, "{id:?}: the new defuser didn't finish");
    }
}

#[test]
fn defuser_waits_while_he_watches_the_bomb_with_time_to_spare() {
    // Arrange: he stands on the site in the open, seen by the squad.
    let a = Arena::build(MapId::ALL[0]);
    let mut w = World::new(&a, 4, 9);
    w.run(4.0);
    let bomb = w.plant(0, brain::objective::FUSE_S);
    let site = a.sites[0];
    let him = (0..400)
        .map(|k| V2::new(site.min.x + (k % 20) as f32 * (site.max.x - site.min.x) / 20.0, site.min.y + (k / 20) as f32 * (site.max.y - site.min.y) / 20.0))
        .find(|p| a.nav.walkable_at(*p) && p.dist(bomb) > 4.0 && a.line_of_sight(*p, false, bomb, true))
        .expect("a spot watching the bomb");
    // Act: the squad keeps seeing him (radio board fresh) for 3 s.
    for _ in 0..(3.0 / DT) as usize {
        w.squad.board.pos = him;
        w.squad.board.age = 0.0;
        w.step(Some(him));
    }
    // Assert: the defuser hasn't gone for the bomb yet.
    assert!(!w.squad.site.going, "defuser should wait for the angle to be cleared");
    assert_eq!(w.wanted, 0);
}

#[test]
fn rotation_keeps_one_anchor_until_info_is_confirmed() {
    for id in MapId::ALL {
        // Arrange: five defenders set up.
        let a = Arena::build(id);
        let mut w = World::new(&a, 5, 13);
        w.run(5.0);
        let on_b = |w: &World| (0..5).filter(|&i| w.squad.site.hold[i].is_some_and(|h| bomb_site_dist(&a.sites[1], h.0) < 6.0)).count();
        let b_before = on_b(&w);
        assert!(b_before >= 2, "{id:?}: B needs two anchors for this test ({b_before})");
        let at_a = a.nav.nearest_walkable(a.sites[0].center());
        // Act 1: stale-ish info (4 s old, so he's not "known") puts him at A.
        for _ in 0..(2.0 / DT) as usize {
            w.squad.board.pos = at_a;
            w.squad.board.age = 4.0;
            w.step(None);
        }
        // Assert 1: rotating to A, but B keeps one anchor (it could be a fake).
        assert_eq!(w.squad.site.rotate.map(|r| r.0), Some(0), "{id:?}");
        assert_eq!(on_b(&w), 1, "{id:?}: B should keep exactly one anchor");
        // Act 2: he is seen on A for a while (confirmed).
        for _ in 0..(5.0 / DT) as usize {
            w.squad.board.pos = at_a;
            w.squad.board.age = 0.2;
            w.step(None);
        }
        // Assert 2: the last anchor rotates too.
        assert!(w.squad.site.rotate.is_some_and(|r| r.3), "{id:?}: not confirmed");
        assert_eq!(on_b(&w), 0, "{id:?}: B anchor should have rotated");
    }
}

#[test]
fn sim_defusal_rounds_plant_and_resolve_by_the_rules() {
    let (mut planted, mut defused, mut exploded, mut rounds) = (0, 0, 0, 0);
    for id in MapId::ALL {
        let a = Arena::build(id);
        let state = BrainState::new(1);
        let limits = Difficulty::Regular.limits(0);
        for seed in 0..6u64 {
            // Arrange
            let minds = state.squad_minds_for(seed, 4, &limits);
            // Act
            let r = sim::run_round_objective(&a, &PlayerModel::default(), limits, minds, seed);
            // Assert: outcomes follow `objective`.
            rounds += 1;
            assert!(!(r.defused && r.exploded), "{id:?}");
            if r.defused {
                assert!(r.planted && !r.player_won, "{id:?} seed {seed}");
            }
            if r.exploded {
                assert!(r.planted && r.player_won, "{id:?} seed {seed}");
            }
            if !r.planted {
                assert!(r.duration <= sim::ROUND_TIME + DT, "{id:?}");
            }
            planted += r.planted as u32;
            defused += r.defused as u32;
            exploded += r.exploded as u32;
        }
    }
    eprintln!("defusal sim: {rounds} rounds, {planted} plants, {defused} defused, {exploded} exploded");
    assert!(planted > 0, "the ghost never planted");
}
