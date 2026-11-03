//! Report and encircle (The Last of Us, Game AI Pro 2 ch. 34; Voronoi capture,
//! arXiv:2312.01326 / 2406.19181).
//!
//! - **Approach sectors**: flood the nav grid from the player out to `RING_HI`; the reachable
//!   ring cells, grouped by bearing, are his exits and our approach sectors. Each sector's goal
//!   is its best ring cell (line of fire on him, cover nearby, mid-ring).
//! - **Joint assignment**: movers to distinct sectors minimising path length + k·angular
//!   crowding (Σ max(0, 360°/n − |θi − θj|), the shooters' bearings included). Exhaustive over
//!   injective assignments (≤ 5 movers × ≤ 12 sectors), then a softmax pick among the best 3.
//! - **Path diversity**: routes are planned one after another; each committed route raises the
//!   cost of a 2 m band around it, and the line from the player toward the squad's guns is
//!   expensive, so flankers swing wide and arrive from behind along different corridors. The
//!   mind gets the route as a via point (`Order::via`).

use crate::arena::Arena;
use crate::math::{BrainRng, V2, angle_diff};
use rand::Rng;
use std::f32::consts::TAU;

pub const RING_LO: f32 = 8.0;
pub const RING_HI: f32 = 15.0;
pub const BINS: usize = 12;
/// Angular crowding weight (cost per radian under the fair share, vs metres / 10).
pub const SPREAD_K: f32 = 2.5;
/// Extra cost per metre inside another member's route band.
pub const BAND_COST: f32 = 1.5;
pub const BAND_R: f32 = 2.0;
/// Extra cost per metre near the line of fire between him and the squad.
pub const VECTOR_COST: f32 = 2.0;
pub const VECTOR_R: f32 = 3.5;
/// Extra cost per metre in his line of sight.
pub const EXPOSED_COST: f32 = 0.8;
pub const TEMP: f32 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sector {
    /// Bearing from the player (radians).
    pub bearing: f32,
    /// Staging cell: out of his sight when possible, next to `peek`.
    pub goal: V2,
    /// A cell a step from `goal` with a line of fire on him (the synchronised peek).
    pub peek: V2,
    /// Path distance from the player to the goal.
    pub path_d: f32,
}

/// The player's approach sectors (at most `BINS`, one goal each).
pub fn sectors(arena: &Arena, player: V2) -> Vec<Sector> {
    let nav = &arena.nav;
    let field = nav.distance_field(&[player], RING_HI + 4.0);
    let mid = 0.5 * (RING_LO + RING_HI);
    // Line of fire on him from every cell in and around the ring.
    let los: Vec<Option<bool>> = field
        .iter()
        .enumerate()
        .map(|(i, &d)| (RING_LO - 4.0..=RING_HI + 4.0).contains(&d).then(|| arena.line_of_sight(nav.center(i), false, player, false)))
        .collect();
    let peek_near = |i: usize| -> Option<V2> {
        let c = nav.center(i);
        let (cx, cz) = nav.cell_of(c);
        let mut best: Option<(f32, V2)> = None;
        for dz in -3i64..=3 {
            for dx in -3i64..=3 {
                let (x, z) = (cx as i64 + dx, cz as i64 + dz);
                if x < 0 || z < 0 || x >= nav.w as i64 || z >= nav.w as i64 {
                    continue;
                }
                let j = z as usize * nav.w + x as usize;
                let q = nav.center(j);
                let d = q.dist(c);
                if d <= 3.2 && los[j] == Some(true) && nav.clear_line(c, q) && best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, q));
                }
            }
        }
        best.map(|b| b.1)
    };
    let mut best: Vec<Option<(f32, Sector)>> = vec![None; BINS];
    for (i, &d) in field.iter().enumerate() {
        if !(RING_LO..=RING_HI).contains(&d) {
            continue;
        }
        let c = nav.center(i);
        let bearing = (c - player).angle();
        let bin = ((bearing.rem_euclid(TAU) / TAU * BINS as f32) as usize).min(BINS - 1);
        let cover = arena.cover.iter().any(|cp| cp.pos.dist(c) < 1.5);
        let mut s = -(d - mid).abs() / 10.0 + if cover { 0.2 } else { 0.0 };
        if best[bin].is_some_and(|(b, _)| s + 1.0 <= b) {
            continue;
        }
        // Best: hidden, a step from a line of fire. Then: a line of fire. Last: hidden only.
        let peek = if los[i] == Some(true) {
            s += 0.5;
            c
        } else if let Some(q) = peek_near(i) {
            s += 1.0;
            q
        } else {
            c
        };
        if best[bin].is_none_or(|(b, _)| s > b) {
            best[bin] = Some((s, Sector { bearing, goal: c, peek, path_d: d }));
        }
    }
    best.into_iter().flatten().map(|b| b.1).collect()
}

/// Angular crowding of bearing `a` against `others`, for a fair share of `share` radians.
fn crowd(a: f32, others: &[f32], share: f32) -> f32 {
    others.iter().map(|b| (share - angle_diff(a, *b).abs()).max(0.0)).sum()
}

/// Assign movers to distinct sectors. `dist[m][s]`: path distance from mover m to sector s's
/// goal (INFINITY: unreachable). `fixed`: bearings already covered (the shooters). `prev`: the
/// current assignment (kept unless the best new one is > 15% cheaper).
pub fn assign(sectors: &[Sector], dist: &[Vec<f32>], fixed: &[f32], prev: Option<&[Option<usize>]>, rng: &mut BrainRng) -> Vec<Option<usize>> {
    let m = dist.len();
    if m == 0 || sectors.is_empty() {
        return vec![None; m];
    }
    let n_total = (m.min(sectors.len()) + fixed.len()).max(1);
    let share = TAU / n_total as f32;
    // Exhaustive search keeping the best 3 complete assignments.
    let mut top: Vec<(f32, Vec<Option<usize>>)> = Vec::new();
    let mut cur: Vec<Option<usize>> = Vec::with_capacity(m);
    let mut used = vec![false; sectors.len()];
    #[allow(clippy::too_many_arguments)]
    fn rec(
        k: usize,
        cost: f32,
        sectors: &[Sector],
        dist: &[Vec<f32>],
        fixed: &[f32],
        share: f32,
        cur: &mut Vec<Option<usize>>,
        used: &mut [bool],
        top: &mut Vec<(f32, Vec<Option<usize>>)>,
    ) {
        let worst = if top.len() >= 3 { top[2].0 } else { f32::INFINITY };
        if cost >= worst {
            return;
        }
        if k == dist.len() {
            top.push((cost, cur.clone()));
            top.sort_by(|a, b| a.0.total_cmp(&b.0));
            top.truncate(3);
            return;
        }
        let free = used.iter().filter(|u| !**u).count();
        let left = dist.len() - k;
        for s in 0..sectors.len() {
            if used[s] || !dist[k][s].is_finite() {
                continue;
            }
            let mut bearings: Vec<f32> = fixed.to_vec();
            bearings.extend(cur.iter().flatten().map(|&j| sectors[j].bearing));
            let c = dist[k][s] / 10.0 + SPREAD_K * crowd(sectors[s].bearing, &bearings, share);
            used[s] = true;
            cur.push(Some(s));
            rec(k + 1, cost + c, sectors, dist, fixed, share, cur, used, top);
            cur.pop();
            used[s] = false;
        }
        // More movers than sectors: someone may go without (costly, so only when forced).
        if left > free || (0..sectors.len()).all(|s| used[s] || !dist[k][s].is_finite()) {
            cur.push(None);
            rec(k + 1, cost + 5.0, sectors, dist, fixed, share, cur, used, top);
            cur.pop();
        }
    }
    rec(0, 0.0, sectors, dist, fixed, share, &mut cur, &mut used, &mut top);
    if top.is_empty() {
        return vec![None; m];
    }
    // Hysteresis: keep the previous assignment unless the best is clearly better.
    if let Some(p) = prev
        && p.len() == m
        && p.iter().flatten().all(|&s| s < sectors.len())
    {
        let mut c = 0.0;
        let mut seen: Vec<f32> = fixed.to_vec();
        let mut ok = true;
        for (k, s) in p.iter().enumerate() {
            match s {
                Some(s) if dist[k][*s].is_finite() => {
                    c += dist[k][*s] / 10.0 + SPREAD_K * crowd(sectors[*s].bearing, &seen, share);
                    seen.push(sectors[*s].bearing);
                }
                Some(_) => ok = false,
                None => c += 5.0,
            }
        }
        if ok && top[0].0 > 0.85 * c - 1e-3 {
            return p.to_vec();
        }
    }
    let best = top[0].0;
    let ws: Vec<f32> = top.iter().map(|(c, _)| (-(c - best) / TEMP).exp()).collect();
    let mut r = rng.random::<f32>() * ws.iter().sum::<f32>();
    for (w, (_, a)) in ws.iter().zip(&top) {
        if r <= *w {
            return a.clone();
        }
        r -= w;
    }
    top[0].1.clone()
}

/// Plan diverse routes for `(from, to)` pairs (in priority order) around `player`, with the
/// squad's guns at `guns`. Returns a via point per route (None: short or direct route).
pub fn diverse_vias(arena: &Arena, player: V2, guns: Option<V2>, routes: &[(V2, V2)]) -> Vec<Option<V2>> {
    let nav = &arena.nav;
    let n = nav.w * nav.w;
    let mut band = vec![0.0f32; n];
    let exposed = std::cell::RefCell::new(vec![f32::NAN; n]);
    let fire_dir = guns.map(|g| (g - player).norm()).filter(|d| d.len() > 0.5);
    let base = |i: usize| -> f32 {
        let p = nav.center(i);
        let mut ex = exposed.borrow_mut();
        if ex[i].is_nan() {
            ex[i] = if p.dist(player) < 40.0 && arena.line_of_sight(player, false, p, false) { EXPOSED_COST } else { 0.0 };
        }
        let mut c = ex[i] + crate::squad::close_cost(p.dist(player));
        if let Some(fd) = fire_dir {
            let rel = p - player;
            let along = rel.dot(fd);
            if (0.0..30.0).contains(&along) && rel.cross(fd).abs() < VECTOR_R {
                c += VECTOR_COST;
            }
        }
        c
    };
    let mut out = Vec::with_capacity(routes.len());
    for &(from, to) in routes {
        let path = {
            let cost = |p: V2| {
                let i = nav.idx_of(p);
                band[i] + base(i)
            };
            nav.find_path_cost(from, to, Some(&cost))
        };
        let Some(path) = path else {
            out.push(None);
            continue;
        };
        // Rasterise the route and raise its band for the next members.
        let mut pts = vec![from];
        pts.extend(path.iter().copied());
        let mut len = 0.0;
        for w in pts.windows(2) {
            let seg = w[0].dist(w[1]);
            let steps = (seg / (nav.cell * 0.5)).ceil().max(1.0) as usize;
            for s in 0..=steps {
                let q = w[0].lerp(w[1], s as f32 / steps as f32);
                if q.dist(to) < 4.0 {
                    continue;
                }
                let r = (BAND_R / nav.cell).ceil() as i64;
                let (cx, cz) = nav.cell_of(q);
                for dz in -r..=r {
                    for dx in -r..=r {
                        let (x, z) = (cx as i64 + dx, cz as i64 + dz);
                        if x >= 0 && z >= 0 && x < nav.w as i64 && z < nav.w as i64 {
                            let i = z as usize * nav.w + x as usize;
                            if nav.center(i).dist(q) <= BAND_R {
                                band[i] = BAND_COST;
                            }
                        }
                    }
                }
            }
            len += seg;
        }
        // Via: the route's point about halfway, if the route is long enough to matter.
        let via = if len > 10.0 {
            let mut left = len * 0.5;
            let mut v = None;
            for w in pts.windows(2) {
                let seg = w[0].dist(w[1]);
                if seg >= left {
                    v = Some(w[0].lerp(w[1], left / seg.max(1e-6)));
                    break;
                }
                left -= seg;
            }
            // Only worth it when the route bends away from the straight line.
            v.filter(|v| {
                let d = (to - from).norm();
                (*v - from).cross(d).abs() > 3.0
            })
            .map(|v| nav.nearest_walkable(v))
        } else {
            None
        };
        out.push(via);
    }
    out
}
