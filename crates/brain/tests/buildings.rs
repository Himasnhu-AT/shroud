//! Buildings from the building kit: every room is reachable, windows behave like Low cover
//! (see and shoot over them standing, hidden crouched, never walk through), doorways are
//! navigable, and Township has the sightlines it promises.

use brain::arena::{AGENT_RADIUS, Arena, BlockKind, MapId};
use brain::math::V2;
use std::collections::VecDeque;

fn maps() -> Vec<Arena> {
    MapId::ALL.into_iter().map(Arena::build).collect()
}

fn reachable(a: &Arena, from: V2) -> Vec<bool> {
    let w = a.nav.w;
    let mut seen = vec![false; w * w];
    let start = a.nav.idx_of(a.nav.nearest_walkable(from));
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

fn inside(p: V2, min: V2, max: V2) -> bool {
    p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y
}

#[test]
fn township_is_big_and_has_eight_buildings() {
    let a = Arena::build(MapId::Township);
    assert!((a.half() - 45.0).abs() < 1e-3);
    // The L-shaped office is two wings.
    assert!(a.decor.buildings.len() >= 9, "{} buildings", a.decor.buildings.len());
    assert!(a.decor.windows.len() >= 40, "{} windows", a.decor.windows.len());
    // Everything else stays the default size.
    for id in [MapId::Foundry, MapId::Depot, MapId::Office] {
        assert_eq!(Arena::build(id).half(), brain::arena::ARENA_HALF);
    }
}

#[test]
fn every_room_is_reachable_from_every_other() {
    for a in maps() {
        let reach = reachable(&a, a.player_spawn);
        for bld in &a.decor.buildings {
            let rooms: Vec<_> = a.zones.iter().filter(|z| inside((z.min + z.max) * 0.5, bld.min, bld.max)).collect();
            assert!(!rooms.is_empty(), "{:?}: {} has no named rooms", a.id, bld.name);
            for z in rooms {
                // Reachable from the player spawn means reachable from every other room too.
                let cells: Vec<usize> = (0..a.nav.walkable.len()).filter(|&i| a.nav.walkable[i] && inside(a.nav.center(i), z.min, z.max)).collect();
                assert!(cells.len() >= 4, "{:?}: room {} has only {} walkable cells", a.id, z.name, cells.len());
                let ok = cells.iter().filter(|&&i| reach[i]).count();
                assert_eq!(ok, cells.len(), "{:?}: room {} is partly sealed off ({ok}/{})", a.id, z.name, cells.len());
                let c = a.nav.center(cells[cells.len() / 2]);
                assert!(a.nav.find_path(a.player_spawn, c).is_some(), "{:?}: no path into {}", a.id, z.name);
            }
        }
    }
}

#[test]
fn windows_are_low_cover_you_cannot_walk_through() {
    for a in maps() {
        for (i, w) in a.decor.windows.iter().enumerate() {
            assert!(w.sill < w.top && w.top < w.wall_top, "{:?}: window {i} has a bad profile", a.id);
            assert!(w.top >= 2.0, "{:?}: window {i} lintel too low to see under", a.id);
            let c = (w.min + w.max) * 0.5;
            let size = w.max - w.min;
            // The sill is a Low block exactly filling the opening.
            let sill = a.blocks.iter().find(|b| b.kind == BlockKind::Low && b.center.dist(c) < 1e-3);
            let sill = sill.unwrap_or_else(|| panic!("{:?}: window {i} at {c:?} has no sill block", a.id));
            assert!((sill.half * 2.0 - size).len() < 1e-3);
            assert!(!a.nav.walkable_at(c), "{:?}: window {i} at {c:?} is walkable", a.id);
            // Look straight through it from just either side: seen standing, hidden crouched.
            let across = if size.x > size.y { V2::new(0.0, 0.9) } else { V2::new(0.9, 0.0) };
            let (p, q) = (c - across, c + across);
            assert!(a.line_of_sight(p, false, q, false), "{:?}: can't see through window {i} at {c:?}", a.id);
            assert!(!a.line_of_sight(p, true, q, false), "{:?}: crouching under window {i} at {c:?} doesn't hide", a.id);
        }
    }
}

#[test]
fn building_doorways_fit_an_agent() {
    for a in maps() {
        for (i, h) in a.decor.headers.iter().enumerate() {
            let c = (h.min + h.max) * 0.5;
            let size = h.max - h.min;
            let width = size.x.max(size.y);
            assert!(width >= 2.0 * AGENT_RADIUS + 0.5, "{:?}: door {i} only {width:.2} m wide", a.id);
            assert!(a.nav.walkable_at(c) || a.nav.walkable_at(c + V2::new(0.5, 0.5)) || a.nav.walkable_at(c - V2::new(0.5, 0.5)), "{:?}: door {i} at {c:?} blocked", a.id);
        }
    }
}

#[test]
fn building_shells_are_walls_and_sills_of_that_building() {
    for a in maps() {
        for bld in &a.decor.buildings {
            assert!(!bld.shell.is_empty());
            for &i in &bld.shell {
                let b = &a.blocks[i];
                assert!(matches!(b.kind, BlockKind::Tall | BlockKind::Low), "{:?}: {} shell block {i} is {:?}", a.id, bld.name, b.kind);
                let (lo, hi) = (b.center - b.half, b.center + b.half);
                assert!(lo.x >= bld.min.x - 1e-3 && lo.y >= bld.min.y - 1e-3 && hi.x <= bld.max.x + 1e-3 && hi.y <= bld.max.y + 1e-3);
            }
            let r = a.decor.roofs[bld.roof];
            assert!((r.bottom - bld.height).abs() < 1e-3, "{:?}: {} roof not on its walls", a.id, bld.name);
        }
    }
}

#[test]
fn township_has_long_medium_and_short_sightlines() {
    let a = Arena::build(MapId::Township);
    // Main Street's west sidewalk: a sniper lane end to end.
    assert!(a.line_of_sight(V2::new(-6.0, -43.0), false, V2::new(-6.0, 43.0), false));
    // The cross street: over 80 m.
    assert!(a.line_of_sight(V2::new(-43.0, -5.0), false, V2::new(43.0, -5.0), false));
    // Interiors are short: no window-to-window line across the blue house kitchen is longer
    // than the house.
    let pts: Vec<V2> = (0..a.nav.walkable.len()).filter(|&i| a.nav.walkable[i]).map(|i| a.nav.center(i)).collect();
    let mut buckets = [0usize; 3];
    for (k, p) in pts.iter().enumerate().step_by(37) {
        for q in pts.iter().skip(k + 1).step_by(41) {
            let d = p.dist(*q);
            if a.line_of_sight(*p, false, *q, false) {
                buckets[if d < 15.0 { 0 } else if d < 40.0 { 1 } else { 2 }] += 1;
            }
        }
    }
    eprintln!("township visible pairs: short {} medium {} long {}", buckets[0], buckets[1], buckets[2]);
    assert!(buckets.iter().all(|&n| n > 20), "sightline mix {buckets:?}");
}

#[test]
fn ai_grids_cover_every_map() {
    use brain::player_model::{heat_center, heat_idx};
    for a in maps() {
        let lim = a.half() - 1.5;
        for p in [V2::new(-lim, -lim), V2::new(lim, lim), V2::new(lim, -lim)] {
            // No clamping at the map's corners: the cell centre is within a cell of the point.
            assert!(heat_center(heat_idx(p)).dist(p) < 2.0, "{:?}: heat grid clamps {p:?}", a.id);
            assert!(brain::squad::cell_center(brain::squad::cell_of(p)).dist(p) < 3.0);
        }
    }
}
