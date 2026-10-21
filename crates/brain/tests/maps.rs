//! Every map: in bounds, spawns walkable and connected, zones and cover, no overlapping
//! geometry (z-fighting), and the 2D sim plays a real round on it.

use brain::arena::{ARENA_HALF, Arena, BlockKind, MapId, WALL_THICKNESS};
use brain::evolve::BrainState;
use brain::limits::Limits;
use brain::math::V2;
use brain::player_model::PlayerModel;
use brain::sim;
use std::collections::VecDeque;

fn maps() -> Vec<Arena> {
    MapId::ALL.into_iter().map(Arena::build).collect()
}

/// Walkable cells reachable from `from` (8-neighbour flood, like A*'s moves).
fn reachable(a: &Arena, from: V2) -> Vec<bool> {
    let w = a.nav.w;
    let mut seen = vec![false; w * w];
    let start = a.nav.idx_of(from);
    assert!(a.nav.walkable[start]);
    seen[start] = true;
    let mut q = VecDeque::from([start]);
    while let Some(i) = q.pop_front() {
        let (x, z) = ((i % w) as i64, (i / w) as i64);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= w as i64 || nz >= w as i64 {
                continue;
            }
            let n = nz as usize * w + nx as usize;
            // No corner cutting: diagonal moves need both side cells open.
            let side_a = z as usize * w + nx as usize;
            let side_b = nz as usize * w + x as usize;
            if seen[n] || !a.nav.walkable[n] || (dx != 0 && dz != 0 && !(a.nav.walkable[side_a] && a.nav.walkable[side_b])) {
                continue;
            }
            seen[n] = true;
            q.push_back(n);
        }
    }
    seen
}

#[test]
fn build_returns_the_requested_map() {
    for id in MapId::ALL {
        assert_eq!(Arena::build(id).id, id);
    }
}

#[test]
fn maps_fit_inside_the_arena_bounds() {
    for a in maps() {
        let lim = ARENA_HALF + WALL_THICKNESS + 1e-3;
        for b in &a.blocks {
            let inner = if b.kind == BlockKind::Wall { lim } else { ARENA_HALF - WALL_THICKNESS + 1e-3 };
            assert!(
                (b.center.x.abs() + b.half.x) <= inner && (b.center.y.abs() + b.half.y) <= inner,
                "{:?}: block at {:?} pokes out",
                a.id,
                b.center
            );
            assert!(b.half.x > 0.0 && b.half.y > 0.0 && b.height > 0.0);
        }
        for s in a.enemy_spawns.iter().chain([&a.player_spawn]) {
            assert!(a.in_bounds(*s), "{:?}: spawn {s:?} out of bounds", a.id);
        }
    }
}

#[test]
fn spawns_are_walkable_spread_and_mutually_reachable() {
    for a in maps() {
        assert!(a.enemy_spawns.len() >= 6, "{:?}: Elite fields 6 enemies", a.id);
        let reach = reachable(&a, a.player_spawn);
        for (i, s) in a.enemy_spawns.iter().enumerate() {
            assert!(a.nav.walkable_at(*s), "{:?}: enemy spawn {i} {s:?} not walkable", a.id);
            assert!(reach[a.nav.idx_of(*s)], "{:?}: enemy spawn {i} unreachable from the player", a.id);
            assert!(a.nav.find_path(a.player_spawn, *s).is_some(), "{:?}: no path to spawn {i}", a.id);
            for (j, o) in a.enemy_spawns.iter().enumerate().skip(i + 1) {
                assert!(s.dist(*o) >= 3.0, "{:?}: spawns {i} and {j} too close", a.id);
            }
            assert!(s.dist(a.player_spawn) > 35.0, "{:?}: enemy spawn {i} too close to the player", a.id);
        }
        // No sealed-off pockets: (almost) every walkable cell is reachable.
        let walkable = a.nav.walkable.iter().filter(|w| **w).count();
        let reached = reach.iter().filter(|r| **r).count();
        assert!(reached as f32 >= walkable as f32 * 0.98, "{:?}: {reached}/{walkable} walkable cells reachable", a.id);
    }
}

#[test]
fn player_spawn_is_hidden_from_enemy_spawns() {
    for a in maps() {
        for (i, s) in a.enemy_spawns.iter().enumerate() {
            assert!(!a.line_of_sight(*s, false, a.player_spawn, false), "{:?}: enemy spawn {i} sees the player at round start", a.id);
        }
    }
}

#[test]
fn zones_name_most_of_the_floor_and_cover_is_plentiful() {
    for a in maps() {
        let mut names: Vec<_> = a.zones.iter().map(|z| z.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), a.zones.len(), "{:?}: duplicate zone names", a.id);
        assert!(a.zones.len() >= 8, "{:?}: too few zones", a.id);
        let cells: Vec<V2> = (0..a.nav.walkable.len()).filter(|&i| a.nav.walkable[i]).map(|i| a.nav.center(i)).collect();
        let named = cells.iter().filter(|p| a.zone_of(**p).is_some()).count();
        assert!(named as f32 >= cells.len() as f32 * 0.95, "{:?}: only {named}/{} cells in a zone", a.id, cells.len());
        // Every zone has some floor in it (no names for solid buildings only).
        for z in &a.zones {
            assert!(cells.iter().any(|p| a.zone_of(*p).is_some_and(|f| f.name == z.name)), "{:?}: zone {} has no floor", a.id, z.name);
        }
        assert!(a.cover.len() >= 80, "{:?}: only {} cover points", a.id, a.cover.len());
        let low = a.blocks.iter().filter(|b| b.kind == BlockKind::Low).count();
        assert!(low >= 8, "{:?}: only {low} waist-high cover blocks", a.id);
    }
}

/// Overlap area of two ground rects.
fn overlap(amin: V2, amax: V2, bmin: V2, bmax: V2) -> f32 {
    let w = (amax.x.min(bmax.x) - amin.x.max(bmin.x)).max(0.0);
    let h = (amax.y.min(bmax.y) - amin.y.max(bmin.y)).max(0.0);
    w * h
}

#[test]
fn geometry_does_not_overlap() {
    // Overlapping boxes give coplanar faces (z-fighting) and confuse cover generation.
    for a in maps() {
        let rect = |b: &brain::arena::Block| (b.center - b.half, b.center + b.half);
        for (i, x) in a.blocks.iter().enumerate() {
            for (j, y) in a.blocks.iter().enumerate().skip(i + 1) {
                if x.kind == BlockKind::Wall && y.kind == BlockKind::Wall {
                    continue; // outer wall corners
                }
                let ((a0, a1), (b0, b1)) = (rect(x), rect(y));
                assert!(overlap(a0, a1, b0, b1) < 1e-4, "{:?}: blocks {i} {:?} and {j} {:?} overlap", a.id, x.center, y.center);
            }
        }
        for (i, h) in a.decor.headers.iter().enumerate() {
            for b in &a.blocks {
                let (b0, b1) = rect(b);
                assert!(overlap(h.min, h.max, b0, b1) < 1e-4, "{:?}: door header {i} overlaps a block", a.id);
            }
            assert!(h.bottom >= 2.2 && h.top > h.bottom, "{:?}: header {i} hangs too low", a.id);
        }
        for (i, f) in a.decor.floors.iter().enumerate() {
            for g in a.decor.floors.iter().skip(i + 1) {
                assert!(overlap(f.min, f.max, g.min, g.max) < 1e-4, "{:?}: floor patches overlap", a.id);
            }
        }
    }
}

#[test]
fn doorways_are_wide_enough_for_nav() {
    // Each door header spans a gap the nav grid can path through (crossing it).
    for a in maps() {
        for (i, h) in a.decor.headers.iter().enumerate() {
            let c = (h.min + h.max) * 0.5;
            let size = h.max - h.min;
            assert!(size.x.max(size.y) >= 1.6, "{:?}: door {i} narrower than 1.6 m", a.id);
            let across = if size.x > size.y { V2::new(0.0, 1.5) } else { V2::new(1.5, 0.0) };
            let (p, q) = (c - across, c + across);
            assert!(a.nav.walkable_at(p) && a.nav.walkable_at(q), "{:?}: door {i} at {c:?} has blocked sides", a.id);
            let path = a.nav.find_path(p, q).expect("path through door");
            let len: f32 = std::iter::once(p).chain(path.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| w[0].dist(w[1])).sum();
            assert!(len < 4.5, "{:?}: door {i} at {c:?} is not passable (detour {len:.1} m)", a.id);
        }
    }
}

#[test]
fn a_sim_round_plays_out_on_every_map() {
    let brain = BrainState::new(3);
    let model = PlayerModel::default();
    for a in maps() {
        let mut fights = 0;
        let mut damage = 0.0;
        for seed in 0..6u64 {
            let minds = brain.squad_minds(seed);
            let r = sim::run_round(&a, &model, Limits::for_generation(0), minds, 10 + seed);
            assert!(r.duration > 0.0 && r.duration <= sim::ROUND_TIME + 0.1);
            if r.duration < sim::ROUND_TIME {
                fights += 1;
            }
            damage += r.enemies.iter().map(|e| e.damage_dealt).sum::<f32>();
        }
        eprintln!("{:?}: {fights}/6 rounds decided, {damage:.0} damage", a.id);
        assert!(fights >= 3, "{:?}: only {fights}/6 rounds were decided before time ran out", a.id);
        assert!(damage > 0.0, "{:?}: nobody ever got shot", a.id);
    }
}
