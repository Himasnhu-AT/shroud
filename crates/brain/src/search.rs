//! Split search while the player is unknown (Welsh, "Looking for Trouble", Game AI Pro 2
//! ch. 27; Isla's occupancy maps; frontier assignment, arXiv:2303.17459).
//!
//! - **Spots**: local maxima of the squad's belief grid (3×3-blurred mass), plus cover points
//!   near the last hot spot that no member can see into. Non-max suppressed, at most `MAX_SPOTS`.
//! - **Searched**: a spot a member has looked into (or reached) is skipped for `SEARCHED_S`.
//! - **Assignment**: greedy with reservations over score = mass − path distance − repulsion
//!   from the other members' targets (σ ≈ 10 m) + a bonus inside the member's own geodesic
//!   Voronoi region; each pick is sampled from the member's top 3 (softmax, seeded) so the
//!   squad is not a hive mind, and a member keeps its spot until it is searched (commitment).

use crate::arena::Arena;
use crate::math::{BrainRng, V2, angle_diff};
use crate::squad::{BELIEF_W, Belief, cell_center};
use rand::Rng;

pub const MAX_SPOTS: usize = 12;
/// Spots closer than this (m) merge.
pub const SPOT_SEP: f32 = 6.0;
/// A searched spot stays off the list this long (s).
pub const SEARCHED_S: f32 = 14.0;
/// Radius (m) around a searched point that counts as searched.
pub const SEARCHED_R: f32 = 4.0;
/// Repulsion width (m) between members' targets.
pub const SIGMA: f32 = 10.0;
/// Score weights: mass, path distance (per 30 m), repulsion, own-region bonus.
pub const W_MASS: f32 = 1.0;
pub const W_DIST: f32 = 0.55;
pub const W_REP: f32 = 0.9;
pub const W_OWN: f32 = 0.25;
/// Softmax temperature over a member's top-3 spots.
pub const TEMP: f32 = 0.08;
/// A member gives up on a spot it has not reached after this long (s).
pub const COMMIT_MAX_S: f32 = 14.0;
/// A member can see into a spot this far away (m).
pub const LOOK_RANGE: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub pos: V2,
    /// Belief mass, normalised so the best spot is 1.
    pub mass: f32,
}

fn searched(p: V2, searched: &[(V2, f32)], now: f32) -> bool {
    searched.iter().any(|(s, t)| now - t < SEARCHED_S && s.dist(p) < SEARCHED_R)
}

/// Candidate search spots. `hot`: where he was last seen or heard (adds hidden cover spots
/// around it); `eyes`: member positions (cover they can already see into is not a spot).
pub fn spots(arena: &Arena, belief: &Belief, hot: Option<V2>, eyes: &[V2], done: &[(V2, f32)], now: f32) -> Vec<Spot> {
    let w = BELIEF_W as i64;
    let p = &belief.p;
    let blur: Vec<f32> = (0..p.len())
        .map(|i| {
            let (x, z) = (i as i64 % w, i as i64 / w);
            let mut s = 0.0;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let (nx, nz) = (x + dx, z + dz);
                    if nx >= 0 && nz >= 0 && nx < w && nz < w {
                        s += p[(nz * w + nx) as usize] * if dx == 0 && dz == 0 { 1.0 } else { 0.5 };
                    }
                }
            }
            s
        })
        .collect();
    let mut cand: Vec<(f32, V2)> = Vec::new();
    for i in 0..p.len() {
        if !belief.walkable[i] || p[i] <= 0.0 {
            continue;
        }
        let (x, z) = (i as i64 % w, i as i64 / w);
        let peak = (-1..=1).all(|dz| {
            (-1..=1).all(|dx| {
                let (nx, nz) = (x + dx, z + dz);
                (dx == 0 && dz == 0) || nx < 0 || nz < 0 || nx >= w || nz >= w || blur[(nz * w + nx) as usize] <= blur[i]
            })
        });
        // Not only strict peaks: any cell with real mass is a candidate, peaks first.
        let score = blur[i] * if peak { 1.5 } else { 1.0 };
        cand.push((score, arena.nav.nearest_walkable(cell_center(i))));
    }
    // Hidden cover near where he vanished: where a human would duck.
    if let Some(h) = hot {
        for c in &arena.cover {
            let d = c.pos.dist(h);
            if !(2.0..16.0).contains(&d) || eyes.iter().any(|e| e.dist(c.pos) < LOOK_RANGE && arena.line_of_sight(*e, false, c.pos, true)) {
                continue;
            }
            let m = blur[crate::squad::cell_of(c.pos)] * 1.6 * (1.0 - d / 24.0);
            cand.push((m, c.pos));
        }
    }
    cand.retain(|(m, q)| *m > 1e-4 && !searched(*q, done, now));
    cand.sort_by(|a, b| b.0.total_cmp(&a.0));
    // Big maps spread spots (and searchers, below) in proportion to their size.
    let scale = map_scale(arena);
    let mut out: Vec<Spot> = Vec::new();
    for (m, q) in cand {
        if out.len() >= MAX_SPOTS {
            break;
        }
        if out.iter().all(|s| s.pos.dist(q) >= SPOT_SEP * scale) {
            out.push(Spot { pos: q, mass: m });
        }
    }
    let top = out.first().map_or(1.0, |s| s.mass.max(1e-9));
    out.iter_mut().for_each(|s| s.mass /= top);
    out
}

/// Map size relative to the 64 m maps the weights were tuned on (Township: ~1.4).
pub fn map_scale(arena: &Arena) -> f32 {
    (arena.half() / crate::arena::ARENA_HALF).max(1.0)
}

/// Can a member at `eye` looking along `look` see into `spot` (a crouched player there)?
pub fn sees_spot(arena: &Arena, eye: V2, look: V2, spot: V2) -> bool {
    let d = spot - eye;
    let l = d.len();
    l < 3.0 || (l < LOOK_RANGE && angle_diff(d.angle(), look.angle()).abs() < 1.0 && arena.line_of_sight(eye, false, spot, true))
}

/// One searcher: slot, position, path-distance field (`NavGrid::distance_field`), current
/// target (kept if still a spot).
pub struct Searcher<'a> {
    pub slot: usize,
    pub pos: V2,
    pub field: &'a [f32],
    pub current: Option<V2>,
}

/// Assign a spot to each searcher. `reserved`: targets already taken (investigators).
/// Returns per searcher (same order) the spot to go to.
pub fn assign(arena: &Arena, spots: &[Spot], who: &[Searcher], reserved: &[V2], rng: &mut BrainRng) -> Vec<Option<V2>> {
    let scale = map_scale(arena);
    let sigma = SIGMA * scale;
    let mut out: Vec<Option<V2>> = vec![None; who.len()];
    let mut taken: Vec<V2> = reserved.to_vec();
    let mut used = vec![false; spots.len()];
    // Commitment: keep a current target that is still a spot.
    for (k, s) in who.iter().enumerate() {
        if let Some(c) = s.current
            && let Some(j) = (0..spots.len()).find(|&j| !used[j] && spots[j].pos.dist(c) < SPOT_SEP * 0.5)
        {
            used[j] = true;
            out[k] = Some(spots[j].pos);
            taken.push(spots[j].pos);
        }
    }
    let dist = |k: usize, p: V2| arena.nav.field_at(who[k].field, p);
    // Geodesic Voronoi: the searcher nearest (by path) owns a spot.
    let owner = |p: V2| (0..who.len()).min_by(|&a, &b| dist(a, p).total_cmp(&dist(b, p)));
    loop {
        let open: Vec<usize> = (0..who.len()).filter(|&k| out[k].is_none()).collect();
        if open.is_empty() {
            break;
        }
        let all_used = used.iter().all(|u| *u);
        // Score every (searcher, spot) given what's taken.
        type Pick = (f32, usize, Vec<(f32, usize)>);
        let mut best: Option<Pick> = None;
        for &k in &open {
            let mut opts: Vec<(f32, usize)> = (0..spots.len())
                .filter(|&j| all_used || !used[j])
                .filter_map(|j| {
                    let s = spots[j];
                    let d = dist(k, s.pos);
                    if !d.is_finite() {
                        return None;
                    }
                    let rep: f32 = taken.iter().map(|t| (-t.dist(s.pos).powi(2) / (sigma * sigma)).exp()).sum();
                    let own = if owner(s.pos) == Some(k) { W_OWN } else { 0.0 };
                    Some((W_MASS * s.mass - W_DIST * d / (30.0 * scale) - W_REP * rep + own, j))
                })
                .collect();
            opts.sort_by(|a, b| b.0.total_cmp(&a.0));
            opts.truncate(3);
            if let Some(&(s, _)) = opts.first()
                && best.as_ref().is_none_or(|b| s > b.0)
            {
                best = Some((s, k, opts));
            }
        }
        let Some((_, k, opts)) = best else { break };
        // Softmax over the top 3.
        let top = opts[0].0;
        let ws: Vec<f32> = opts.iter().map(|(s, _)| ((s - top) / TEMP).exp()).collect();
        let mut r = rng.random::<f32>() * ws.iter().sum::<f32>();
        let mut pick = opts[0].1;
        for (w, (_, j)) in ws.iter().zip(&opts) {
            if r <= *w {
                pick = *j;
                break;
            }
            r -= w;
        }
        used[pick] = true;
        out[k] = Some(spots[pick].pos);
        taken.push(spots[pick].pos);
    }
    out
}
