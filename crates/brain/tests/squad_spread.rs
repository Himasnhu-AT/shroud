//! Squad spread metrics (playtest: "they come in a swarm and are predictable").
//!
//! - **Search**: with the player hidden and unknown, members split up: mean pairwise distance
//!   and the share of the map's walkable 4 m cells someone has looked at.
//! - **Encircle**: after first contact with a stationary player, members arrive from several
//!   sides (max angular gap around him) along different routes (shared path cells).
//!
//! All runs are seeded and use the same kinematic world as `tests/squad.rs` (no ghost), on
//! every map, with Regular (4) and Elite (6) sized squads.
use brain::arena::{ARENA_HALF, Arena, MapId};
use brain::difficulty::Difficulty;
use brain::evolve::BrainState;
use brain::math::{V2, angle_diff};
use brain::mind::{AllyInfo, EnemyMind, MindCtx, Observation};
use brain::player_model::PlayerModel;
use brain::sim::{self, BODY_RADIUS, DT, SPRINT, VISION_HALF_ANGLE, VISION_RANGE, WALK};
use brain::squad::SquadCoordinator;
use brain::squad::MemberView;
use std::f32::consts::TAU;

const CELL: f32 = 4.0;
const GW: usize = 16;

fn cell_center(i: usize) -> V2 {
    V2::new(-32.0 + ((i % GW) as f32 + 0.5) * CELL, -32.0 + ((i / GW) as f32 + 0.5) * CELL)
}

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
}

impl<'a> World<'a> {
    fn new(a: &'a Arena, n: usize, seed: u64, d: Difficulty) -> Self {
        let limits = d.limits(0);
        let minds = BrainState::new(seed).squad_minds_for(seed, n, &limits);
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
        }
    }

    /// One tick. `player`: where he is if he can be seen at all (None: never visible).
    /// Returns whether anyone saw him.
    fn step(&mut self, player: Option<V2>) -> bool {
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
        self.squad.drain_callouts();
        for (m, o) in self.minds.iter_mut().zip(&orders) {
            m.set_order(*o);
        }
        let board = self.squad.board;
        let snap = self.pos.clone();
        let cx = MindCtx { arena: a, model: &self.model, limits: self.limits };
        let mut any = false;
        for i in 0..n {
            let sees = player.is_some_and(|p| {
                let d = p - self.pos[i];
                d.len() < VISION_RANGE
                    && (d.len() < 5.0 || angle_diff(d.angle(), self.looks[i].angle()).abs() < VISION_HALF_ANGLE)
                    && a.line_of_sight(self.pos[i], false, p, false)
            });
            any |= sees;
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
                allies: (0..n).filter(|&j| j != i).map(|j| AllyInfo { pos: snap[j], engaged: false, flanking: false }).collect(),
                ..Default::default()
            };
            let act = self.minds[i].think(&obs, &cx);
            self.looks[i] = act.look_dir;
            let speed = if act.crouch { sim::CROUCH_SPEED } else if act.sprint { SPRINT } else { WALK };
            self.pos[i] = a.resolve_collision(self.pos[i] + act.move_dir * speed * DT, BODY_RADIUS);
        }
        any
    }
}

fn mean_pairwise(p: &[V2]) -> f32 {
    let (mut s, mut k) = (0.0, 0);
    for i in 0..p.len() {
        for j in i + 1..p.len() {
            s += p[i].dist(p[j]);
            k += 1;
        }
    }
    s / k.max(1) as f32
}

/// Walkable 4 m cells (centre walkable or any of four inner samples).
fn walkable_cells(a: &Arena) -> Vec<bool> {
    (0..GW * GW)
        .map(|i| {
            let c = cell_center(i);
            [V2::ZERO, V2::new(1.0, 1.0), V2::new(-1.0, 1.0), V2::new(1.0, -1.0), V2::new(-1.0, -1.0)].iter().any(|o| a.nav.walkable_at(c + *o))
        })
        .collect()
}

#[derive(Clone, Copy, Default, Debug)]
struct SearchStats {
    spread: f32,
    coverage: f32,
}

/// Player hidden and unknown for `secs`: mean pairwise distance (from 5 s on) and the share of
/// walkable cells someone looked into (cone + LOS to a crouched player at the cell centre).
fn search_run(a: &Arena, n: usize, seed: u64, d: Difficulty, secs: f32) -> SearchStats {
    let mut w = World::new(a, n, seed, d);
    let walk = walkable_cells(a);
    let mut seen = vec![false; GW * GW];
    let (mut spread, mut samples) = (0.0, 0);
    let steps = (secs / DT) as usize;
    for k in 0..steps {
        w.step(None);
        if k % 10 == 0 {
            for i in 0..w.pos.len() {
                let eye = w.pos[i];
                for (c, s) in seen.iter_mut().enumerate() {
                    if *s || !walk[c] {
                        continue;
                    }
                    let p = cell_center(c);
                    let dd = p - eye;
                    if dd.len() < 40.0 && (dd.len() < 3.0 || angle_diff(dd.angle(), w.looks[i].angle()).abs() < 1.05) && a.line_of_sight(eye, false, p, true) {
                        *s = true;
                    }
                }
            }
            if k as f32 * DT >= 5.0 {
                spread += mean_pairwise(&w.pos);
                samples += 1;
            }
        }
    }
    let total = walk.iter().filter(|x| **x).count().max(1) as f32;
    SearchStats { spread: spread / samples.max(1) as f32, coverage: seen.iter().filter(|x| **x).count() as f32 / total }
}

#[derive(Clone, Copy, Default, Debug)]
struct EncircleStats {
    /// Largest angular gap (deg) between members around the player, `after` s after contact.
    max_gap: f32,
    /// Share of 2 m cells walked by 2+ members since contact.
    overlap: f32,
    /// Seconds to first contact (None: never found him).
    contact: Option<f32>,
}

fn max_gap_deg(player: V2, pos: &[V2]) -> f32 {
    let mut b: Vec<f32> = pos.iter().map(|p| (*p - player).angle().rem_euclid(TAU)).collect();
    b.sort_by(f32::total_cmp);
    let mut gap: f32 = 0.0;
    for k in 0..b.len() {
        let next = if k + 1 < b.len() { b[k + 1] } else { b[0] + TAU };
        gap = gap.max(next - b[k]);
    }
    gap.to_degrees()
}

/// A stationary, harmless player at `player`; the squad finds him and closes in.
fn encircle_run(a: &Arena, n: usize, seed: u64, d: Difficulty, player: V2, after: f32) -> EncircleStats {
    let mut w = World::new(a, n, seed, d);
    let mut contact: Option<f32> = None;
    let mut cells: Vec<std::collections::HashSet<(i32, i32)>> = vec![Default::default(); w.pos.len()];
    let mut t = 0.0;
    while t < 45.0 {
        t += DT;
        let saw = w.step(Some(player));
        if contact.is_none() && saw {
            contact = Some(t);
        }
        if let Some(c0) = contact {
            for (i, p) in w.pos.iter().enumerate() {
                cells[i].insert(((p.x / 2.0).floor() as i32, (p.y / 2.0).floor() as i32));
            }
            if t - c0 >= after {
                break;
            }
        }
    }
    let mut count = std::collections::HashMap::new();
    for s in &cells {
        for c in s {
            *count.entry(*c).or_insert(0) += 1;
        }
    }
    let shared = count.values().filter(|v| **v >= 2).count() as f32;
    EncircleStats { max_gap: max_gap_deg(player, &w.pos), overlap: shared / count.len().max(1) as f32, contact }
}

const SEEDS: [u64; 3] = [1, 2, 3];

fn sizes() -> [(Difficulty, usize); 2] {
    [(Difficulty::Regular, 4), (Difficulty::Elite, 6)]
}

/// Prints the table; returns per-(map, size) means.
fn search_table() -> Vec<(MapId, usize, SearchStats)> {
    let mut out = Vec::new();
    for id in MapId::ALL {
        let a = Arena::build(id);
        for (d, n) in sizes() {
            let mut m = SearchStats::default();
            for s in SEEDS {
                let r = search_run(&a, n, s, d, 30.0);
                m.spread += r.spread / SEEDS.len() as f32;
                m.coverage += r.coverage / SEEDS.len() as f32;
            }
            eprintln!("search {:<8} x{n}: spread {:5.1} m  coverage {:4.0}%", id.label(), m.spread, m.coverage * 100.0);
            out.push((id, n, m));
        }
    }
    out
}

/// An open spot between the middle and his spawn (the spawn itself backs onto a wall).
fn hideout(a: &Arena) -> V2 {
    a.nav.nearest_walkable(a.player_spawn * 0.45)
}

fn encircle_table(after: f32) -> Vec<(MapId, usize, EncircleStats, usize)> {
    let mut out = Vec::new();
    for id in MapId::ALL {
        let a = Arena::build(id);
        for (d, n) in sizes() {
            let mut m = EncircleStats::default();
            let mut found = 0;
            for s in SEEDS {
                let r = encircle_run(&a, n, s, d, hideout(&a), after);
                if r.contact.is_some() {
                    found += 1;
                    m.max_gap += r.max_gap;
                    m.overlap += r.overlap;
                }
            }
            let k = found.max(1) as f32;
            m.max_gap /= k;
            m.overlap /= k;
            eprintln!("encircle +{after}s {:<8} x{n}: found {found}/{}  max gap {:4.0} deg  shared cells {:4.0}%", id.label(), SEEDS.len(), m.max_gap, m.overlap * 100.0);
            out.push((id, n, m, found));
        }
    }
    out
}

#[test]
fn search_spreads_out() {
    // Before (one `Free` hunt toward the same last-known point, seeds 1-3, 30 s):
    //   Foundry x4 spread 8.2 m cov 81% | x6 8.3 m 85%
    //   Depot   x4        4.6 m     74% | x6 6.7 m 84%
    //   Office  x4        5.9 m     63% | x6 6.2 m 69%
    let old = [(MapId::Foundry, 4, 0.81), (MapId::Foundry, 6, 0.85), (MapId::Depot, 4, 0.74), (MapId::Depot, 6, 0.84), (MapId::Office, 4, 0.63), (MapId::Office, 6, 0.69)];
    for (id, n, m) in search_table() {
        // Tuned on the 64 m maps; big maps (Township, 90 m) are reported above, not graded yet.
        if Arena::build(id).half() > ARENA_HALF {
            continue;
        }
        let before = old.iter().find(|o| o.0 == id && o.1 == n).map_or(0.0, |o| o.2);
        assert!(m.spread > 18.0, "{id:?} x{n}: members should split up (mean pairwise {:.1} m)", m.spread);
        assert!(m.coverage >= before + 0.03, "{id:?} x{n}: coverage {:.2} vs {before:.2} before", m.coverage);
    }
}

#[test]
fn contact_encircles() {
    // Before (contact with a stationary player at `hideout`, seeds 1-3), max gap / shared cells:
    //   +6 s : Foundry x4 286 deg 4%, x6 276 deg 5% | Depot x4 280 deg 26%, x6 277 deg 27%
    //          Office  x4 126 deg 15% (found him 1/3), x6 155 deg 33% (2/3)
    //   +10 s: Foundry x4 232 deg 6%, x6 229 deg 10% | Depot x4 189 deg 26%, x6 247 deg 28%
    //          Office  x4 156 deg 26% (1/3), x6 155 deg 38% (2/3)
    let _ = encircle_table(6.0);
    let t10 = encircle_table(10.0);
    let mut overlap = 0.0;
    for (id, n, m, found) in t10.iter().filter(|r| Arena::build(r.0).half() <= ARENA_HALF) {
        assert_eq!(*found, SEEDS.len(), "{id:?} x{n}: the squad should find him every time");
        assert!(m.max_gap < 180.0, "{id:?} x{n}: they should come from several sides (max gap {:.0} deg)", m.max_gap);
        overlap += m.overlap / t10.iter().filter(|r| Arena::build(r.0).half() <= ARENA_HALF).count() as f32;
    }
    // Before: 22% of walked cells shared on average.
    assert!(overlap < 0.15, "routes should differ (shared cells {:.0}%)", overlap * 100.0);
}

/// He is seen, then vanishes: at most two members check where he was; the rest spread out
/// over other spots; calls go out on the radio.
#[test]
fn lost_contact_sends_two_to_check_and_spreads_the_rest() {
    use brain::squad::{Play, Role};
    for id in MapId::ALL {
        let a = Arena::build(id);
        let mut w = World::new(&a, 6, 4, Difficulty::Elite);
        let player = hideout(&a);
        let mut seen_at: Option<f32> = None;
        let mut t = 0.0;
        let (mut searched_ticks, mut max_inv, mut spread_dests) = (0, 0, 0.0f32);
        let mut calls = Vec::new();
        while t < 40.0 {
            t += DT;
            // Visible until 2 s after first contact, then gone for good.
            let visible = seen_at.is_none_or(|s| t - s < 2.0);
            if w.step(visible.then_some(player)) && seen_at.is_none() {
                seen_at = Some(t);
            }
            calls.extend(w.squad.drain_callouts().into_iter().map(|c| c.text));
            if seen_at.is_some() && w.squad.play == Play::Search {
                searched_ticks += 1;
                let inv = w.squad.orders.iter().flatten().filter(|o| o.role == Role::Collapser).count();
                max_inv = max_inv.max(inv);
                let dests: Vec<V2> = w.squad.orders.iter().flatten().filter(|o| o.role == Role::Searcher).filter_map(|o| o.dest).collect();
                spread_dests = spread_dests.max(mean_pairwise(&dests));
            }
        }
        eprintln!("{id:?}: {searched_ticks} search ticks, max investigators {max_inv}, searcher dest spread {spread_dests:.1} m");
        assert!(seen_at.is_some() && searched_ticks > 0, "{id:?}: should find him, then lose him");
        assert!(max_inv <= 2, "{id:?}: {max_inv} members ran to the last known spot");
        assert!(spread_dests > 12.0, "{id:?}: searchers should head to different spots ({spread_dests:.1} m)");
        assert!(calls.iter().all(|c| c.is_ascii()));
    }
}

#[test]
#[ignore]
fn debug_encircle_trace() {
    let id = match std::env::var("MAP").as_deref() {
        Ok("depot") => MapId::Depot,
        Ok("office") => MapId::Office,
        _ => MapId::Foundry,
    };
    let a = Arena::build(id);
    let mut w = World::new(&a, 4, 1, Difficulty::Regular);
    let player = hideout(&a);
    let mut t = 0.0;
    let mut c0: Option<f32> = None;
    while t < 40.0 {
        t += DT;
        let saw = w.step(Some(player));
        if c0.is_none() && saw {
            c0 = Some(t);
        }
        let k = (t / DT).round() as usize;
        if k.is_multiple_of(10) && c0.is_some() {
            let roles: Vec<String> =
                w.squad.orders.iter().map(|o| o.map_or("-".to_string(), |o| format!("{}:{:?}", o.role.label(), o.dest.map(|d| (d.x.round(), d.y.round()))))).collect();
            eprintln!(
                "t {t:5.1} {:<9} age {:4.1} pos {:?} goals {:?} {:?}",
                w.squad.play.label(),
                w.squad.board.age,
                w.pos.iter().map(|p| (p.x.round(), p.y.round())).collect::<Vec<_>>(),
                w.minds.iter().map(|m| m.goal().label()).collect::<Vec<_>>(),
                roles,
            );
            if t - c0.unwrap_or(0.0) > 10.0 {
                break;
            }
        }
    }
    eprintln!("player {player:?} contact {c0:?} spawns {:?}", a.enemy_spawns);
}
