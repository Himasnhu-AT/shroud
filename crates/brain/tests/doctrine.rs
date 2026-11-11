//! Adaptive doctrine (bodycam spec §5, §7 check 6): the player's style profile, archetype
//! detection with hysteresis, and the squad's counters (technique only: jiggle-peeks,
//! flashes, wallbangs, reload rushes, off-angles, lean + stagger).
use brain::arena::{Arena, BlockKind, MapId};
use brain::difficulty::Difficulty;
use brain::doctrine::{AiTactic, Archetype, Archetypes, PlayerStyleProfile, StyleSummary, StyleTick, debrief_line};
use brain::evolve::BrainState;
use brain::genome::{TraitGenome, Traits, UNSET_DOCTRINE};
use brain::grenade::GrenadeKind;
use brain::math::{V2, angle_diff};
use brain::mind::{AllyInfo, EnemyMind, MindCtx, Observation};
use brain::player_model::{MatchTelemetry, PlayerModel};
use brain::rooms;
use brain::sim::{self, BODY_RADIUS, DT, SPRINT, VISION_HALF_ANGLE, VISION_RANGE, WALK};
use brain::squad::{MemberDoctrine, MemberView, Order, Role, SquadCoordinator};

/// vs a camper standing in a lamp's light (Office), one member walks to a spot that sees the
/// fixture but not him and taps the lamp: a `Role::Lights` order and aimed shots at that lamp
/// from a clear line. Maps without lamps never do it.
#[test]
fn camper_under_a_lamp_gets_his_light_shot_out() {
    let a = Arena::build(MapId::Office);
    assert!(!a.decor.lamps.is_empty());
    let mut tried = 0;
    for (id, l) in a.decor.lamps.iter().enumerate() {
        let player = a.nav.nearest_walkable(l.pos + V2::new(0.8, 0.6));
        if !a.lamps_lighting(player).contains(&id) {
            continue;
        }
        tried += 1;
        let mut w = World::new(&a, 5, 11, 0.8);
        let mut roles = Vec::new();
        let mut t = 0.0;
        while t < 30.0 && w.light_taps.is_empty() {
            t += DT;
            w.squad.observe_player(&a, StyleTick { dt: DT, pos: player, vel: V2::ZERO, yaw: 0.4 });
            for o in w.step(player).iter().flatten() {
                if !roles.contains(&o.role) {
                    roles.push(o.role);
                }
            }
        }
        let Some(&(i, from, lamp)) = w.light_taps.first() else {
            eprintln!("lamp {id} at {:?}: no tap in 30 s (roles {roles:?})", l.pos);
            continue;
        };
        assert!(roles.contains(&Role::Lights));
        assert!(a.lamps_lighting(player).contains(&lamp), "shoots a lamp that lights him");
        let (at, h) = a.decor.lamps[lamp].fixture();
        assert!(a.clear_line_3d(from, brain::doctrine::MUZZLE_H, at, h), "line of sight to the fixture");
        assert!(!a.line_of_sight(from, false, player, false), "from out of his sight");
        assert_eq!(w.squad.doctrine.tactic, AiTactic::JigglePeekAndFlash);
        eprintln!("lamp {lamp} shot at by m{i} from {from:?} after {t:.1} s (camper at {player:?})");
        // Once it's out, it isn't a target any more.
        let mut dark = Arena::build(MapId::Office);
        dark.dark_lamps.push(lamp);
        assert!(!dark.lamps_lighting(player).contains(&lamp));
        return;
    }
    panic!("no lamp shot out ({tried} lit spots tried)");
}

/// No lamps lighting him (or none on the map): nobody gets a Lights order.
#[test]
fn no_lights_no_light_shots() {
    let a = Arena::build(MapId::Sirocco);
    let player = a.nav.nearest_walkable(a.player_spawn * 0.45);
    if !a.lamps_lighting(player).is_empty() {
        return;
    }
    let mut w = World::new(&a, 5, 3, 0.8);
    let mut t = 0.0;
    while t < 20.0 {
        t += DT;
        w.squad.observe_player(&a, StyleTick { dt: DT, pos: player, vel: V2::ZERO, yaw: -std::f32::consts::FRAC_PI_2 });
        assert!(w.step(player).iter().flatten().all(|o| o.role != Role::Lights));
    }
    assert!(w.light_taps.is_empty());
}

fn still(p: &mut PlayerStyleProfile, pos: V2, yaw: f32, secs: f32) {
    let mut t = 0.0;
    while t < secs {
        t += DT;
        p.tick(None, StyleTick { dt: DT, pos, vel: V2::ZERO, yaw: yaw + 0.05 * (t * 3.0).sin() });
    }
}

#[test]
fn angle_holder_needs_four_seconds_still_on_one_angle() {
    let mut p = PlayerStyleProfile::default();
    still(&mut p, V2::new(3.0, 4.0), 1.0, 3.8);
    assert!(!p.detect(Archetypes::NONE).has(Archetype::AngleHolder), "3.8 s is not a camp yet");
    still(&mut p, V2::new(3.0, 4.0), 1.0, 0.4);
    let on = p.detect(Archetypes::NONE);
    assert!(on.has(Archetype::AngleHolder), "hold {:.1} s speed {:.2}", p.stationary_angle_hold_secs, p.avg_move_speed);
    assert!(p.avg_move_speed < 0.1);
    // Swinging the view to a new angle restarts the hold, but the archetype holds on while he
    // stays put (hysteresis) ...
    still(&mut p, V2::new(3.0, 4.0), 2.5, 1.0);
    assert!(p.stationary_angle_hold_secs < 1.1);
    let on = p.detect(on);
    assert!(on.has(Archetype::AngleHolder));
    // ... and lets go once he really moves.
    let mut t = 0.0;
    while t < 3.0 {
        t += DT;
        p.tick(None, StyleTick { dt: DT, pos: V2::new(3.0 + 4.0 * t, 4.0), vel: V2::new(4.0, 0.0), yaw: 0.0 });
    }
    assert!(!p.detect(on).has(Archetype::AngleHolder));
}

/// No wallhack: time nobody sees him ages the windows but builds no angle hold; seen time on
/// the same angle before and after the gap still adds up.
#[test]
fn unseen_time_builds_no_angle_hold() {
    let mut p = PlayerStyleProfile::default();
    let at = V2::new(3.0, 4.0);
    still(&mut p, at, 1.0, 2.0);
    let mut t = 0.0;
    while t < 10.0 {
        t += DT;
        p.unseen(DT);
    }
    assert!(p.clock > 11.9, "clock {:.1}", p.clock);
    assert!(p.stationary_angle_hold_secs < 2.1, "unseen time counted: {:.1} s", p.stationary_angle_hold_secs);
    assert!(!p.detect(Archetypes::NONE).has(Archetype::AngleHolder), "12 s camping, only 2 s of it seen");
    still(&mut p, at, 1.0, 2.2);
    assert!(p.detect(Archetypes::NONE).has(Archetype::AngleHolder), "4.2 s seen on one angle: {:.1} s", p.stationary_angle_hold_secs);
}

#[test]
fn entry_fragger_runs_through_doors() {
    let a = Arena::build(MapId::Office);
    let doors = rooms::doorways(&a);
    let d = doors.first().copied().expect("Office has doorways");
    let mut p = PlayerStyleProfile::default();
    // Run 6 m/s straight through the door, 6 m either side.
    let start = d.pos - d.normal * 6.0;
    let mut t = 0.0;
    while t < 2.0 {
        t += DT;
        let pos = start + d.normal * (6.0 * t);
        p.tick(Some(&a), StyleTick { dt: DT, pos, vel: d.normal * 6.0, yaw: d.normal.angle() });
    }
    assert_eq!(p.rapid_entries, 1);
    assert!(p.avg_move_speed > 4.2);
    assert!(p.detect(Archetypes::NONE).has(Archetype::EntryFragger));
    // Walking through slowly is not an entry at a run.
    let mut q = PlayerStyleProfile::default();
    let mut t = 0.0;
    while t < 6.0 {
        t += DT;
        q.tick(Some(&a), StyleTick { dt: DT, pos: start + d.normal * (2.0 * t), vel: d.normal * 2.0, yaw: 0.0 });
    }
    assert_eq!(q.rapid_entries, 0);
    assert!(!q.detect(Archetypes::NONE).has(Archetype::EntryFragger));
}

#[test]
fn reloader_and_precision_aimer() {
    let mut p = PlayerStyleProfile::default();
    p.reload(0.3); // a normal reload
    p.reload(0.8);
    assert_eq!(p.premature_reload_count, 1);
    assert!(!p.detect(Archetypes::NONE).has(Archetype::CompulsiveReloader));
    p.reload(0.6);
    let on = p.detect(Archetypes::NONE);
    assert!(on.has(Archetype::CompulsiveReloader));
    // Headshots: needs a few hits, then > 45%.
    for h in [true, true, false] {
        p.hit(h);
    }
    assert!(!p.detect(on).has(Archetype::PrecisionAimer), "3 hits is too few");
    p.hit(true);
    let on = p.detect(on);
    assert!(on.has(Archetype::PrecisionAimer) && on.has(Archetype::CompulsiveReloader), "several archetypes at once: {:?}", on.labels());
    // Shots: ADS share over the window.
    p.shot(true);
    p.shot(false);
    p.shot(true);
    p.shot(true);
    assert!((p.ads_usage_ratio - 0.75).abs() < 1e-4);
    // Old habits leave the window.
    still(&mut p, V2::ZERO, 0.0, 61.0);
    assert_eq!(p.premature_reload_count, 0);
    assert_eq!(p.hits, 0);
    assert_eq!(p.ads_usage_ratio, 0.0);
}

// ---------------------------------------------------------------------------------------------
// The squad.

fn traits(doctrine: f32) -> Traits {
    Traits { aggression: 0.6, team_cohesion: 0.7, preferred_range: 15.0, doctrine: [doctrine; 4], ..Traits::default() }
}

fn view(pos: V2, last_known: V2, last_seen: f32, doctrine: f32) -> MemberView {
    MemberView { pos, alive: true, hp: 1.0, last_known, last_seen, heard: None, look: (last_known - pos).norm(), traits: traits(doctrine), bark: None }
}

/// A doorway on `id` and a spot 4 m back from it on one side, facing the door.
fn doorway_hold(a: &Arena) -> (V2, f32, V2) {
    let doors = rooms::doorways(a);
    for d in &doors {
        for s in [1.0f32, -1.0] {
            let p = d.pos + d.normal * (4.0 * s);
            if a.nav.walkable_at(p) && a.nav.clear_line(p, d.pos) {
                return (p, (d.pos - p).angle(), d.pos);
            }
        }
    }
    panic!("no doorway to hold");
}

/// Spec check 6: standing still while aiming at a doorway for > 4 s makes the squad log a
/// transition to `AiTactic::JigglePeekAndFlash`.
#[test]
fn spec_check_6_holding_a_doorway_switches_to_jiggle_peek_and_flash() {
    let a = Arena::build(MapId::Sirocco);
    let (pos, yaw, door) = doorway_hold(&a);
    let model = PlayerModel::default();
    let mut coord = SquadCoordinator::new(&a, 4);
    let members: Vec<MemberView> = a.enemy_spawns.iter().take(4).map(|&p| view(p, pos, 99.0, 0.5)).collect();
    let mut changes = Vec::new();
    let mut t = 0.0;
    while t < 6.0 {
        t += DT;
        coord.observe_player(&a, StyleTick { dt: DT, pos, vel: V2::ZERO, yaw });
        coord.tick(DT, &members, &a, &model);
        for c in coord.drain_tactic_changes() {
            eprintln!("ai.tactic {{ from: {}, to: {}, reason: {} }} at {:.2} s (holding {pos:?}, door at {door:?})", c.from.name(), c.to.name(), c.reason, c.t);
            changes.push(c);
        }
    }
    assert_eq!(changes.len(), 1, "{changes:?}");
    let c = &changes[0];
    assert_eq!((c.from, c.to), (AiTactic::Default, AiTactic::JigglePeekAndFlash));
    assert!(c.t > 4.0 && c.t < 4.3, "switched at {:.2} s", c.t);
    assert!(c.reason.contains("held one angle"), "{}", c.reason);
    assert!(c.active.has(Archetype::AngleHolder));
    assert_eq!(coord.doctrine.tactic, AiTactic::JigglePeekAndFlash);
    assert!(coord.member_doctrine(0).counters.jiggle > 0.0);
}

/// Kinematic world (like `tests/squad_spread.rs`) with the doctrine wired as in the game.
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
    /// Taps at lamps: (member, its position, the lamp's id).
    light_taps: Vec<(usize, V2, usize)>,
}

impl<'a> World<'a> {
    fn new(a: &'a Arena, n: usize, seed: u64, gene: f32) -> Self {
        let d = Difficulty::Elite;
        let limits = d.limits(0);
        let mut minds = BrainState::new(seed).squad_minds_for(seed, n, &limits);
        for m in &mut minds {
            m.traits.doctrine = [gene; 4];
        }
        let n = minds.len();
        let mut squad = SquadCoordinator::new(a, n);
        squad.max_tokens = d.params().max_attackers;
        World {
            a,
            model: PlayerModel::default(),
            minds,
            pos: a.enemy_spawns.iter().take(n).copied().collect(),
            looks: vec![V2::new(0.0, 1.0); n],
            last_seen: vec![99.0; n],
            last_known: vec![a.player_spawn; n],
            squad,
            limits,
            light_taps: Vec::new(),
        }
    }

    fn step(&mut self, player: V2) -> Vec<Option<Order>> {
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
        for (i, (m, o)) in self.minds.iter_mut().zip(&orders).enumerate() {
            m.set_order(*o);
            m.set_doctrine(self.squad.member_doctrine(i));
        }
        let board = self.squad.board;
        let snap = self.pos.clone();
        let cx = MindCtx { arena: a, model: &self.model, limits: self.limits };
        for i in 0..n {
            let d = player - self.pos[i];
            let sees = d.len() < VISION_RANGE
                && (d.len() < 5.0 || angle_diff(d.angle(), self.looks[i].angle()).abs() < VISION_HALF_ANGLE)
                && a.line_of_sight(self.pos[i], false, player, false);
            if sees {
                self.last_seen[i] = 0.0;
                self.last_known[i] = player;
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
                time_since_seen: shared.1,
                squad_alert: board.alert,
                token_denied: !self.squad.tokens.get(i).copied().unwrap_or(true),
                allies: (0..n).filter(|&j| j != i).map(|j| AllyInfo { pos: snap[j], engaged: false, flanking: false }).collect(),
                ..Default::default()
            };
            let act = self.minds[i].think(&obs, &cx);
            if let Some(l) = act.light.filter(|_| act.fire) {
                self.light_taps.push((i, self.pos[i], l.lamp));
            }
            self.looks[i] = act.look_dir;
            let speed = if act.crouch { sim::CROUCH_SPEED } else if act.sprint { SPRINT } else { WALK };
            self.pos[i] = a.resolve_collision(self.pos[i] + act.move_dir * speed * DT, BODY_RADIUS);
        }
        orders
    }
}

struct CampRun {
    flashes: usize,
    frags: usize,
    roles: Vec<Role>,
    /// Jiggle samples: (member, offset from its hide spot) while jiggling.
    jiggle_offsets: Vec<f32>,
    calls: Vec<String>,
}

/// A player camping a spot on Sirocco for `secs`; `profile`: feed the doctrine his style.
fn camp(seed: u64, profile: bool, secs: f32) -> CampRun {
    let a = Arena::build(MapId::Sirocco);
    let mut w = World::new(&a, 5, seed, 0.8);
    let player = a.nav.nearest_walkable(a.player_spawn * 0.45);
    let mut out = CampRun { flashes: 0, frags: 0, roles: Vec::new(), jiggle_offsets: Vec::new(), calls: Vec::new() };
    let mut t = 0.0;
    while t < secs {
        t += DT;
        if profile {
            w.squad.observe_player(&a, StyleTick { dt: DT, pos: player, vel: V2::ZERO, yaw: -std::f32::consts::FRAC_PI_2 });
        }
        let orders = w.step(player);
        for o in orders.iter().flatten() {
            if !out.roles.contains(&o.role) {
                out.roles.push(o.role);
            }
        }
        if std::env::var("DBG").is_ok() && (t * 20.0).round() as i32 % 40 == 0 {
            let tgt = w.squad.board.pos;
            eprintln!("t {t:.1} board {:?} age {:.1} tactic {:?} still {:.1}", tgt, w.squad.board.age, w.squad.doctrine.tactic, w.squad.still_t);
            for i in 0..w.pos.len() {
                eprintln!("   m{i} {:?} d {:.1} spot {:?}", w.pos[i], w.pos[i].dist(tgt), brain::doctrine::jiggle_spot(&a, w.pos[i], tgt));
            }
        }
        if std::env::var("DBG").is_ok() && (t * 20.0).round() as i32 % 10 == 0 {
            if let Some((i, j)) = w.squad.jiggler {
                eprintln!("t {t:.1} jiggler {i} pos {:?} hide {:?} d {:.2} goal {:?} order {:?} dest {:?} path {:?}", w.pos[i], j.hide, w.pos[i].dist(j.hide), w.minds[i].goal(), w.minds[i].order().map(|o| o.role), w.minds[i].debug.dest, w.minds[i].debug.path);
            }
        }
        for (i, m) in w.minds.iter().enumerate() {
            if m.jiggling()
                && let Some(j) = w.squad.member_doctrine(i).jiggle
            {
                // (From the first time it stood on the spot: the walk in doesn't count.)
                let d = w.pos[i].dist(j.hide);
                if d < 0.1 || !out.jiggle_offsets.is_empty() {
                    out.jiggle_offsets.push(d);
                }
            }
        }
        for th in w.squad.take_throws() {
            match th.kind {
                GrenadeKind::Flash => out.flashes += 1,
                GrenadeKind::Frag => out.frags += 1,
                GrenadeKind::Smoke => {}
            }
        }
        out.calls.extend(w.squad.drain_callouts().into_iter().map(|c| c.text));
    }
    out
}

/// vs an angle holder the squad flashes him, jiggle-peeks his angle (out ~0.25 m and back)
/// and calls it; without the profile none of that happens.
#[test]
fn camper_gets_flashed_and_jiggle_peeked() {
    let with = camp(3, true, 25.0);
    let without = camp(3, false, 25.0);
    eprintln!("with: flashes {} frags {} roles {:?}", with.flashes, with.frags, with.roles);
    eprintln!("without: flashes {} frags {} roles {:?}", without.flashes, without.frags, without.roles);
    eprintln!("calls: {:?}", with.calls);
    assert!(with.calls.iter().any(|c| c.contains("jiggle it")), "the switch is called out");
    assert!(with.roles.contains(&Role::Jiggler), "someone jiggle-peeks");
    assert!(!without.roles.contains(&Role::Jiggler) && !without.roles.contains(&Role::Suppressor));
    assert!(with.flashes >= 1, "flash the angle");
    assert!(with.flashes > without.flashes || without.flashes == 0);
    let max = with.jiggle_offsets.iter().copied().fold(0.0f32, f32::max);
    assert!(!with.jiggle_offsets.is_empty(), "the jiggler reached its spot");
    assert!((0.15..0.6).contains(&max), "steps out ~0.25 m (max {max:.2} m)");
    // And back: most samples sit at the hide spot.
    let home = with.jiggle_offsets.iter().filter(|d| **d < 0.1).count() as f32 / with.jiggle_offsets.len() as f32;
    assert!(home > 0.5, "snaps back ({:.0}% at home)", home * 100.0);
}

/// A reloader's mag-out heard nearby sends the nearest one or two in at a sprint.
#[test]
fn mag_out_sound_triggers_a_rush() {
    let a = Arena::build(MapId::Sirocco);
    let model = PlayerModel::default();
    let player = a.nav.nearest_walkable(V2::new(0.0, 5.0));
    let pos = [V2::new(-6.0, 0.0), V2::new(7.0, -2.0), V2::new(0.0, -9.0), V2::new(-12.0, -14.0)].map(|p| a.nav.nearest_walkable(p));
    let run = |reloader: bool, gene: f32| {
        let mut coord = SquadCoordinator::new(&a, 4);
        if reloader {
            coord.player_reload(0.8);
            coord.player_reload(0.7);
        }
        let views: Vec<MemberView> = pos.iter().map(|&p| view(p, player, 2.0, gene)).collect();
        let mut rushers = Vec::new();
        let mut t = 0.0;
        while t < 1.5 {
            t += DT;
            coord.observe_player(&a, StyleTick { dt: DT, pos: player, vel: V2::ZERO, yaw: 0.0 });
            if (0.3..0.3 + DT).contains(&t) {
                for (i, &p) in pos.iter().enumerate() {
                    if brain::perception::hear(&a, player, p, brain::squad::MAG_OUT_RANGE).is_some() {
                        coord.heard_mag_out(i, player);
                    }
                }
            }
            let orders = coord.tick(DT, &views, &a, &model).to_vec();
            for (i, o) in orders.iter().enumerate() {
                if let Some(o) = o.filter(|o| o.role == Role::Rusher) {
                    assert_eq!(o.goal, brain::utility::Goal::Push);
                    assert!(o.sticky && o.dest.is_some_and(|d| d.dist(player) < 0.5));
                    if !rushers.contains(&i) {
                        rushers.push(i);
                    }
                }
            }
        }
        (rushers, coord.doctrine.tactic)
    };
    let (strong, tactic) = run(true, 1.0);
    assert_eq!(tactic, AiTactic::ReloadPunish);
    assert_eq!(strong.len(), 2, "a strong counter sends two: {strong:?}");
    // The nearest ones that heard it.
    let mut by_dist: Vec<usize> = (0..4).collect();
    by_dist.sort_by(|&x, &y| pos[x].dist(player).total_cmp(&pos[y].dist(player)));
    assert!(strong.iter().all(|i| by_dist[..2].contains(i)), "{strong:?} vs nearest {by_dist:?}");
    let (weak, _) = run(true, 0.0);
    assert_eq!(weak.len(), 1, "a weak counter sends one");
    let (none, tactic) = run(false, 1.0);
    assert!(none.is_empty() && tactic == AiTactic::Default, "no reload habit, no rush");
}

/// vs an entry fragger, members out of contact hold off-angles inside the doors he'd come
/// through: 35-80 deg off the door line, out of sight from outside.
#[test]
fn entry_fragger_meets_off_angle_traps() {
    let a = Arena::build(MapId::Office);
    let model = PlayerModel::default();
    let doors = rooms::doorways(&a);
    let mut found = 0;
    'seed: for (k, d) in doors.iter().enumerate().take(12) {
        // He is 10 m out one side of this door; a member 5 m in the other side.
        let player = a.nav.nearest_walkable(d.pos - d.normal * 10.0);
        let me = a.nav.nearest_walkable(d.pos + d.normal * 5.0);
        if d.side(player) == d.side(me) {
            continue;
        }
        let mut coord = SquadCoordinator::new(&a, 3);
        // Profile: two doors at a run.
        let mut p = coord.style.clone();
        for _ in 0..2 {
            let mut t = 0.0;
            while t < 2.0 {
                t += DT;
                p.tick(Some(&a), StyleTick { dt: DT, pos: d.pos - d.normal * 6.0 + d.normal * (6.0 * t), vel: d.normal * 6.0, yaw: 0.0 });
            }
        }
        coord.style = p;
        let views = vec![view(me, player, 5.0, 1.0), view(a.nav.nearest_walkable(me + V2::new(20.0, 0.0)), player, 5.0, 1.0), view(a.enemy_spawns[0], player, 5.0, 1.0)];
        let mut t = 0.0;
        while t < 2.0 {
            t += DT;
            coord.observe_player(&a, StyleTick { dt: DT, pos: player, vel: V2::new(5.0, 0.0), yaw: 0.0 });
            let orders = coord.tick(DT, &views, &a, &model).to_vec();
            if let Some(o) = orders[0].filter(|o| o.role == Role::Trapper) {
                assert_eq!(coord.doctrine.tactic, AiTactic::OffAngleTrap);
                let spot = o.dest.unwrap();
                let door = o.watch.unwrap();
                let dd = doors.iter().find(|x| x.pos == door).unwrap();
                let inward = dd.normal * dd.side(me);
                let off = angle_diff((spot - dd.pos).angle(), inward.angle()).abs().to_degrees();
                eprintln!("door {k}: trap at {spot:?}, {off:.0} deg off the door line");
                assert!((34.0..81.0).contains(&off), "{off:.0} deg");
                assert!(!a.line_of_sight(dd.pos - inward * 2.5, false, spot, false), "hidden from outside the door");
                found += 1;
                continue 'seed;
            }
        }
    }
    assert!(found >= 2, "off-angle traps on Office doors: {found}");
}

/// vs a precision aimer, members who adopt the counter strafe in short irregular bursts.
#[test]
fn precision_aimer_meets_micro_strafes() {
    let a = Arena::build(MapId::Sirocco);
    let model = PlayerModel::default();
    let limits = Difficulty::Regular.limits(0);
    // Open ground to strafe on: the procession road square in front of the church.
    let player = a.nav.nearest_walkable(V2::new(0.0, 18.0));
    let me = (0..40)
        .map(|k| a.nav.nearest_walkable(player + V2::from_angle(k as f32 * 0.157) * 14.0))
        .find(|p| a.line_of_sight(*p, false, player, false) && p.dist(player) > 10.0)
        .expect("an open spot");
    let flips = |stagger: f32| {
        let mut m = BrainState::new(5).squad_minds_for(5, 1, &limits).remove(0);
        let cx = MindCtx { arena: &a, model: &model, limits };
        let mut pos = me;
        let mut last = V2::ZERO;
        let mut n = 0;
        let mut t = 0.0;
        while t < 8.0 {
            t += DT;
            m.set_order(None);
            m.set_doctrine(MemberDoctrine { counters: brain::doctrine::Counters { stagger, ..Default::default() }, jiggle: None, light: None });
            let o = Observation {
                dt: DT,
                pos,
                hp: 1.0,
                ammo_frac: 1.0,
                player_visible: true,
                player_pos: player,
                player_hp: 1.0,
                squad_alert: brain::perception::Alert::Combat,
                allies: vec![AllyInfo { pos: pos + V2::new(3.0, 0.0), engaged: true, flanking: false }],
                ..Default::default()
            };
            let act = m.think(&o, &cx);
            let lateral = (player - pos).norm().perp();
            let side = act.move_dir.dot(lateral);
            if side.abs() > 0.2 {
                let s = V2::new(side.signum(), 0.0);
                if last.x != 0.0 && s.x != last.x {
                    n += 1;
                }
                last = s;
            }
            pos = a.resolve_collision(pos + act.move_dir * WALK * DT, BODY_RADIUS);
        }
        n
    };
    let base = flips(0.0);
    let stagger = flips(1.0);
    eprintln!("strafe direction changes in 8 s: {base} -> {stagger}");
    assert!(stagger as f32 >= base as f32 * 1.5 && stagger > base + 3, "{base} -> {stagger}");
}

/// Wallbangs: a rifle gets through a crate, nothing gets through a wall.
#[test]
fn penetration_through_thin_cover_only() {
    let a = Arena::build(MapId::Sirocco);
    let rifle = brain::weapons::WeaponKind::Rifle.spec().penetration;
    let smg = brain::weapons::WeaponKind::Smg.spec().penetration;
    // A lone crate: the segment across its thin side crosses nothing else.
    let (p, q) = a
        .blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Low && b.half.x.min(b.half.y) < 0.8)
        .map(|low| {
            let axis = if low.half.x < low.half.y { V2::new(1.0, 0.0) } else { V2::new(0.0, 1.0) };
            let half = low.half.x.min(low.half.y);
            (low.center - axis * (half + 0.3), low.center + axis * (half + 0.3))
        })
        .find(|(p, q)| a.blocks.iter().filter(|b| b.segment_inside(*p, *q) > 0.0).count() == 1)
        .expect("a lone crate");
    let keep = a.penetration(p, q, rifle);
    assert!(keep > 0.15 && keep < 1.0, "rifle through a crate keeps {keep:.2}");
    if smg <= 0.0 {
        assert_eq!(a.penetration(p, q, smg), 0.0);
    }
    let wall = a.blocks.iter().find(|b| b.kind == BlockKind::Wall).unwrap();
    let axis = if wall.half.x < wall.half.y { V2::new(1.0, 0.0) } else { V2::new(0.0, 1.0) };
    assert_eq!(a.penetration(wall.center - axis * 2.0, wall.center + axis * 2.0, 1.0), 0.0);
}

/// Old saves: genomes without doctrine genes load as unset and get seeded; models without a
/// style summary load with an empty one.
#[test]
fn old_saves_load_and_seed_doctrine_genes() {
    let mut rng = brain::math::rng_from_seed(9);
    let g = TraitGenome::random(&mut rng);
    let mut v = serde_json::to_value(&g).unwrap();
    v.as_object_mut().unwrap().remove("doctrine");
    let mut old: TraitGenome = serde_json::from_value(v).unwrap();
    assert_eq!(old.doctrine, UNSET_DOCTRINE);
    old.fill_loadout();
    assert!(old.doctrine.iter().all(|x| (0.0..=1.0).contains(x)));
    assert_eq!(old.doctrine, g.doctrine, "seeded from the trait genes, like a fresh genome");
    let decoded = old.decode(&brain::limits::Limits::for_generation(0));
    assert_eq!(decoded.doctrine, g.doctrine);

    let m = PlayerModel::default();
    let mut v = serde_json::to_value(&m).unwrap();
    v.as_object_mut().unwrap().remove("style");
    let back: PlayerModel = serde_json::from_value(v).unwrap();
    assert_eq!(back.style, StyleSummary::default());
    let mut t = serde_json::to_value(MatchTelemetry::default()).unwrap();
    t.as_object_mut().unwrap().remove("style_time");
    t.as_object_mut().unwrap().remove("style_play_s");
    let back: MatchTelemetry = serde_json::from_value(t).unwrap();
    assert_eq!(back.style_play_s, 0.0);
}

/// Across matches: a habit goes into the player model, the next round starts countering it,
/// and the debrief says so.
#[test]
fn habits_carry_over_and_reach_the_debrief() {
    let mut t = MatchTelemetry::default();
    t.samples = 600.0;
    t.style_play_s = 120.0;
    t.style_time[Archetype::AngleHolder as usize] = 70.0;
    let line = debrief_line(&t).unwrap();
    assert!(line.contains("hold angles") && line.contains("jiggle-peeking and flashing"), "{line}");
    let mut model = PlayerModel::default();
    model.ingest(&t);
    assert!(model.style.share[0] > 0.5);
    let a = Arena::build(MapId::Depot);
    let mut coord = SquadCoordinator::new(&a, 3);
    let views: Vec<MemberView> = a.enemy_spawns.iter().take(3).map(|&p| view(p, a.player_spawn, 99.0, 0.5)).collect();
    coord.tick(DT, &views, &a, &model);
    assert_eq!(coord.doctrine.tactic, AiTactic::JigglePeekAndFlash, "starts the round on the habit");
    let c = coord.drain_tactic_changes();
    assert!(c[0].reason.contains("earlier matches"), "{:?}", c[0].reason);
    assert!(debrief_line(&MatchTelemetry::default()).is_none());
}

/// Technique only: the doctrine genes change no mechanics (reaction, aim error).
#[test]
fn doctrine_genes_are_technique_only() {
    let limits = brain::limits::Limits::for_generation(0);
    let mut rng = brain::math::rng_from_seed(4);
    let mut a = TraitGenome::random(&mut rng);
    let before = a.decode(&limits);
    a.doctrine = [1.0; 4];
    let after = a.decode(&limits);
    assert_eq!(before.reaction_time, after.reaction_time);
    assert_eq!(before.aim_error_deg, after.aim_error_deg);
    assert_eq!(after.doctrine, [1.0; 4]);
}

/// vs an angle holder behind thin cover: a member with a rifle sprays through it (a short
/// pre-fire burst at his spot), and the mind fires it.
#[test]
fn camper_behind_thin_cover_gets_wallbanged() {
    let model = PlayerModel::default();
    let rifle = brain::weapons::WeaponKind::Rifle.spec().penetration;
    let mut tried = 0;
    for id in MapId::ALL {
        let a = Arena::build(id);
        for b in a.blocks.iter().filter(|b| b.kind == BlockKind::Tall && brain::weapons::penetrable(b)) {
            let (axis, half) = if b.half.x < b.half.y { (V2::new(1.0, 0.0), b.half.x) } else { (V2::new(0.0, 1.0), b.half.y) };
            let target = b.center + axis * (half + 1.0);
            let me = b.center - axis * (half + 8.0);
            if !a.nav.walkable_at(target) || !a.nav.walkable_at(me) || a.line_of_sight(me, false, target, false) || a.penetration(me, target, rifle) < 0.15 {
                continue;
            }
            tried += 1;
            let mut coord = SquadCoordinator::new(&a, 2);
            let mut tr = traits(1.0);
            tr.weapon = brain::weapons::WeaponKind::Rifle;
            let far = a.nav.nearest_walkable(target + axis * 25.0);
            let views = vec![
                MemberView { pos: me, alive: true, hp: 1.0, last_known: target, last_seen: 2.0, heard: None, look: (target - me).norm(), traits: tr, bark: None },
                MemberView { pos: far, alive: true, hp: 1.0, last_known: target, last_seen: 0.0, heard: None, look: (target - far).norm(), traits: tr, bark: None },
            ];
            let mut prefire = None;
            let mut t = 0.0;
            while t < 8.0 && prefire.is_none() {
                t += DT;
                coord.observe_player(&a, StyleTick { dt: DT, pos: target, vel: V2::ZERO, yaw: 0.3 });
                let o = coord.tick(DT, &views, &a, &model).to_vec();
                prefire = o[0].filter(|o| o.role == Role::Suppressor).and_then(|o| o.prefire);
            }
            let Some(at) = prefire else { continue };
            assert!(at.dist(target) < 0.5);
            eprintln!("{id:?}: wallbang through {:?} at {at:?} after {t:.1} s", b.skin);
            return;
        }
    }
    panic!("no wallbang on any map ({tried} thin-cover setups)");
}
