//! Grid navigation: bake walkable cells from the arena, A* with 8-neighbours,
//! then string-pull the path so agents walk straight lines where possible.

use crate::arena::{Block, BlockKind};
use crate::math::V2;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Clone, Debug)]
pub struct NavGrid {
    pub cell: f32,
    pub half: f32,
    pub w: usize,
    pub walkable: Vec<bool>,
}

#[derive(Copy, Clone, PartialEq)]
struct Node {
    f: f32,
    idx: usize,
}
impl Eq for Node {}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl NavGrid {
    pub fn bake(blocks: &[Block], half: f32, cell: f32, radius: f32) -> Self {
        let w = ((half * 2.0) / cell).round() as usize;
        let mut walkable = vec![true; w * w];
        for cz in 0..w {
            for cx in 0..w {
                let p = V2::new(-half + (cx as f32 + 0.5) * cell, -half + (cz as f32 + 0.5) * cell);
                walkable[cz * w + cx] = !blocks.iter().any(|b| {
                    let pad = if b.kind == BlockKind::Wall { radius } else { radius + 0.05 };
                    b.contains(p, pad)
                });
            }
        }
        NavGrid { cell, half, w, walkable }
    }

    pub fn cell_of(&self, p: V2) -> (usize, usize) {
        let cx = ((p.x + self.half) / self.cell).floor().clamp(0.0, (self.w - 1) as f32) as usize;
        let cz = ((p.y + self.half) / self.cell).floor().clamp(0.0, (self.w - 1) as f32) as usize;
        (cx, cz)
    }

    pub fn idx_of(&self, p: V2) -> usize {
        let (x, z) = self.cell_of(p);
        z * self.w + x
    }

    pub fn center(&self, idx: usize) -> V2 {
        let (cx, cz) = (idx % self.w, idx / self.w);
        V2::new(-self.half + (cx as f32 + 0.5) * self.cell, -self.half + (cz as f32 + 0.5) * self.cell)
    }

    pub fn walkable_at(&self, p: V2) -> bool {
        p.x.abs() < self.half && p.y.abs() < self.half && self.walkable[self.idx_of(p)]
    }

    /// Nearest walkable cell centre to `p` (spiral search).
    pub fn nearest_walkable(&self, p: V2) -> V2 {
        if self.walkable_at(p) {
            return p;
        }
        let (cx, cz) = self.cell_of(p);
        for r in 1..self.w as i64 {
            let mut best: Option<(f32, usize)> = None;
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let (x, z) = (cx as i64 + dx, cz as i64 + dz);
                    if x < 0 || z < 0 || x >= self.w as i64 || z >= self.w as i64 {
                        continue;
                    }
                    let i = z as usize * self.w + x as usize;
                    if self.walkable[i] {
                        let d = self.center(i).dist(p);
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, i));
                        }
                    }
                }
            }
            if let Some((_, i)) = best {
                return self.center(i);
            }
        }
        p
    }

    /// Straight walkable line between two points (sampled every quarter cell).
    pub fn clear_line(&self, a: V2, b: V2) -> bool {
        let d = b.dist(a);
        let steps = (d / (self.cell * 0.25)).ceil().max(1.0) as usize;
        (0..=steps).all(|i| self.walkable_at(a.lerp(b, i as f32 / steps as f32)))
    }

    /// A* from `from` to `to`. Returns smoothed waypoints (excluding the start).
    pub fn find_path(&self, from: V2, to: V2) -> Option<Vec<V2>> {
        self.find_path_cost(from, to, None)
    }

    /// Tactical A*: `extra(p)` adds cost per metre at cell centre `p` (e.g. `squad::tactical_cost`,
    /// exposure to the player). Smoothing only shortcuts where the shortcut isn't costlier.
    pub fn find_path_cost(&self, from: V2, to: V2, extra: Option<&dyn Fn(V2) -> f32>) -> Option<Vec<V2>> {
        let start = self.idx_of(self.nearest_walkable(from));
        let goal_pos = self.nearest_walkable(to);
        let goal = self.idx_of(goal_pos);
        if start == goal {
            return Some(vec![goal_pos]);
        }
        let n = self.w * self.w;
        let mut g = vec![f32::INFINITY; n];
        let mut came = vec![usize::MAX; n];
        let mut open = BinaryHeap::new();
        g[start] = 0.0;
        let h = |i: usize| self.center(i).dist(goal_pos);
        open.push(Node { f: h(start), idx: start });
        let w = self.w as i64;
        while let Some(Node { idx, .. }) = open.pop() {
            if idx == goal {
                let mut cells = vec![goal];
                let mut c = goal;
                while came[c] != usize::MAX {
                    c = came[c];
                    cells.push(c);
                }
                cells.reverse();
                let pts: Vec<V2> = cells.iter().map(|&i| self.center(i)).collect();
                let mut path = if extra.is_some() { self.smooth_short(from, &pts) } else { self.smooth(from, &pts) };
                if let Some(last) = path.last_mut() {
                    *last = goal_pos;
                }
                return Some(path);
            }
            let (x, z) = ((idx % self.w) as i64, (idx / self.w) as i64);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= w || nz >= w {
                    continue;
                }
                let ni = (nz * w + nx) as usize;
                if !self.walkable[ni] {
                    continue;
                }
                // No corner cutting.
                if dx != 0 && dz != 0 && (!self.walkable[(z * w + nx) as usize] || !self.walkable[(nz * w + x) as usize]) {
                    continue;
                }
                let step = if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 } * self.cell;
                let cost = step * (1.0 + extra.map_or(0.0, |f| f(self.center(ni)).max(0.0)));
                let ng = g[idx] + cost;
                if ng < g[ni] {
                    g[ni] = ng;
                    came[ni] = idx;
                    open.push(Node { f: ng + h(ni), idx: ni });
                }
            }
        }
        None
    }

    /// Walkable 8-neighbours of cell `idx` (no corner cutting) with the step length in metres.
    pub fn neighbours(&self, idx: usize) -> impl Iterator<Item = (usize, f32)> + '_ {
        let w = self.w as i64;
        let (x, z) = ((idx % self.w) as i64, (idx / self.w) as i64);
        [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)].into_iter().filter_map(move |(dx, dz)| {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= w || nz >= w {
                return None;
            }
            let ni = (nz * w + nx) as usize;
            if !self.walkable[ni] || (dx != 0 && dz != 0 && (!self.walkable[(z * w + nx) as usize] || !self.walkable[(nz * w + x) as usize])) {
                return None;
            }
            Some((ni, if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 } * self.cell))
        })
    }

    /// Path distance (m) from the nearest of `from` to every cell (Dijkstra, the same moves as
    /// A*); `INFINITY` where unreachable or farther than `max`.
    pub fn distance_field(&self, from: &[V2], max: f32) -> Vec<f32> {
        let mut d = vec![f32::INFINITY; self.w * self.w];
        let mut open = BinaryHeap::new();
        for &p in from {
            let i = self.idx_of(self.nearest_walkable(p));
            d[i] = 0.0;
            open.push(Node { f: 0.0, idx: i });
        }
        while let Some(Node { f, idx }) = open.pop() {
            if f > d[idx] {
                continue;
            }
            for (ni, step) in self.neighbours(idx) {
                let nd = f + step;
                if nd < d[ni] && nd <= max {
                    d[ni] = nd;
                    open.push(Node { f: nd, idx: ni });
                }
            }
        }
        d
    }

    /// Path distance from a field (`distance_field`) to `p`.
    pub fn field_at(&self, field: &[f32], p: V2) -> f32 {
        field[self.idx_of(self.nearest_walkable(p))]
    }

    /// Smoothing for tactical paths: only shortcut a few cells so the route keeps its shape.
    fn smooth_short(&self, from: V2, pts: &[V2]) -> Vec<V2> {
        let mut out = Vec::new();
        let mut anchor = from;
        let mut i = 0;
        while i < pts.len() {
            let mut j = (i + 4).min(pts.len() - 1);
            while j > i && !self.clear_line(anchor, pts[j]) {
                j -= 1;
            }
            out.push(pts[j]);
            anchor = pts[j];
            i = j + 1;
        }
        out
    }

    fn smooth(&self, from: V2, pts: &[V2]) -> Vec<V2> {
        let mut out = Vec::new();
        let mut anchor = from;
        let mut i = 0;
        while i < pts.len() {
            let mut j = pts.len() - 1;
            while j > i && !self.clear_line(anchor, pts[j]) {
                j -= 1;
            }
            out.push(pts[j]);
            anchor = pts[j];
            i = j + 1;
        }
        out
    }
}
