//! The arena layout, shared by the game (which builds meshes + colliders from it)
//! and the offline trainer (which simulates on it in 2D).

use crate::math::V2;
use crate::nav::NavGrid;

/// Height of low cover: you can shoot over it standing, hide behind it crouched.
pub const LOW_COVER_HEIGHT: f32 = 1.1;
pub const TALL_HEIGHT: f32 = 3.6;
pub const ARENA_HALF: f32 = 32.0;
pub const WALL_THICKNESS: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// Outer boundary wall.
    Wall,
    /// Blocks sight and bullets.
    Tall,
    /// Crouch cover: blocks sight only to/from a crouched agent.
    Low,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub center: V2,
    pub half: V2,
    pub height: f32,
    pub kind: BlockKind,
}

impl Block {
    fn new(cx: f32, cz: f32, hx: f32, hz: f32, kind: BlockKind) -> Self {
        let height = match kind {
            BlockKind::Low => LOW_COVER_HEIGHT,
            _ => TALL_HEIGHT,
        };
        Block { center: V2::new(cx, cz), half: V2::new(hx, hz), height, kind }
    }

    pub fn contains(&self, p: V2, pad: f32) -> bool {
        (p.x - self.center.x).abs() <= self.half.x + pad && (p.y - self.center.y).abs() <= self.half.y + pad
    }

    /// Does segment a→b pass through this box? (slab test)
    pub fn intersects_segment(&self, a: V2, b: V2) -> bool {
        let min = self.center - self.half;
        let max = self.center + self.half;
        let d = b - a;
        let mut t0 = 0.0f32;
        let mut t1 = 1.0f32;
        for (p, dp, lo, hi) in [(a.x, d.x, min.x, max.x), (a.y, d.y, min.y, max.y)] {
            if dp.abs() < 1e-9 {
                if p < lo || p > hi {
                    return false;
                }
            } else {
                let inv = 1.0 / dp;
                let (mut ta, mut tb) = ((lo - p) * inv, (hi - p) * inv);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }
}

/// A named rectangle used to describe player habits in plain words.
#[derive(Clone, Debug)]
pub struct Zone {
    pub name: &'static str,
    pub min: V2,
    pub max: V2,
}

/// A spot next to a block that an agent can hide behind.
#[derive(Clone, Copy, Debug)]
pub struct CoverPoint {
    pub pos: V2,
    pub block: usize,
}

pub struct Arena {
    pub blocks: Vec<Block>,
    pub zones: Vec<Zone>,
    pub player_spawn: V2,
    pub enemy_spawns: Vec<V2>,
    pub nav: NavGrid,
    pub cover: Vec<CoverPoint>,
}

/// Agent collision radius used for nav inflation.
pub const AGENT_RADIUS: f32 = 0.45;

impl Arena {
    /// The one arena of the prototype: "Foundry".
    pub fn foundry() -> Self {
        use BlockKind::*;
        let h = ARENA_HALF;
        let t = WALL_THICKNESS;
        let mut blocks = vec![
            // Outer walls.
            Block::new(0.0, -h, h + t, t, Wall),
            Block::new(0.0, h, h + t, t, Wall),
            Block::new(-h, 0.0, t, h + t, Wall),
            Block::new(h, 0.0, t, h + t, Wall),
            // Center tower.
            Block::new(0.0, 0.0, 4.0, 4.0, Tall),
        ];
        // Mirror-symmetric pieces (x mirrored, and z mirrored).
        let quad: &[(f32, f32, f32, f32, BlockKind)] = &[
            (14.0, 6.5, 0.75, 4.5, Tall),  // lane walls with a gap at z=0
            (7.0, 10.0, 1.2, 1.2, Low),    // crates near center
            (8.0, 20.0, 1.5, 1.5, Tall),   // yard pillars
            (22.0, 12.0, 2.0, 1.0, Tall),  // lane blockers
            (24.0, 22.0, 1.5, 1.0, Low),   // corner crates
            (4.0, 26.0, 4.0, 0.5, Low),    // long low walls near spawns
            (20.0, 0.0, 1.0, 2.5, Low),    // lane mid crates
            (27.0, 6.0, 1.0, 1.0, Low),
        ];
        for &(x, z, hx, hz, k) in quad {
            for (sx, sz) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                let b = Block::new(x * sx, z * sz, hx, hz, k);
                if !blocks.iter().any(|o| o.center.dist(b.center) < 0.01) {
                    blocks.push(b);
                }
            }
        }
        let zones = vec![
            Zone { name: "the center tower", min: V2::new(-8.0, -8.0), max: V2::new(8.0, 8.0) },
            Zone { name: "the north yard", min: V2::new(-14.0, -32.0), max: V2::new(14.0, -8.0) },
            Zone { name: "the south yard", min: V2::new(-14.0, 8.0), max: V2::new(14.0, 32.0) },
            Zone { name: "the west lane", min: V2::new(-32.0, -14.0), max: V2::new(-14.0, 14.0) },
            Zone { name: "the east lane", min: V2::new(14.0, -14.0), max: V2::new(32.0, 14.0) },
            Zone { name: "the north-west corner", min: V2::new(-32.0, -32.0), max: V2::new(-14.0, -14.0) },
            Zone { name: "the north-east corner", min: V2::new(14.0, -32.0), max: V2::new(32.0, -14.0) },
            Zone { name: "the south-west corner", min: V2::new(-32.0, 14.0), max: V2::new(-14.0, 32.0) },
            Zone { name: "the south-east corner", min: V2::new(14.0, 14.0), max: V2::new(32.0, 32.0) },
            Zone { name: "the inner west side", min: V2::new(-14.0, -8.0), max: V2::new(-8.0, 8.0) },
            Zone { name: "the inner east side", min: V2::new(8.0, -8.0), max: V2::new(14.0, 8.0) },
        ];
        let nav = NavGrid::bake(&blocks, ARENA_HALF, 1.0, AGENT_RADIUS);
        let mut arena = Arena {
            blocks,
            zones,
            player_spawn: V2::new(0.0, 29.0),
            enemy_spawns: vec![
                V2::new(-20.0, -28.0),
                V2::new(-10.0, -29.0),
                V2::new(0.0, -29.5),
                V2::new(10.0, -29.0),
                V2::new(20.0, -28.0),
            ],
            nav,
            cover: Vec::new(),
        };
        arena.cover = arena.compute_cover();
        arena
    }

    fn compute_cover(&self) -> Vec<CoverPoint> {
        let mut out = Vec::new();
        let off = AGENT_RADIUS + 0.35;
        for (i, b) in self.blocks.iter().enumerate() {
            if b.kind == BlockKind::Wall {
                continue;
            }
            let (hx, hz) = (b.half.x + off, b.half.y + off);
            let mut pts = Vec::new();
            let step = 1.5;
            let mut x = -b.half.x;
            while x <= b.half.x + 0.01 {
                pts.push(V2::new(x, -hz));
                pts.push(V2::new(x, hz));
                x += step;
            }
            let mut z = -b.half.y;
            while z <= b.half.y + 0.01 {
                pts.push(V2::new(-hx, z));
                pts.push(V2::new(hx, z));
                z += step;
            }
            for p in pts {
                let pos = b.center + p;
                if self.nav.walkable_at(pos) {
                    out.push(CoverPoint { pos, block: i });
                }
            }
        }
        out
    }

    /// Line of sight on the ground plane. Low cover only blocks if either end is crouched.
    pub fn line_of_sight(&self, a: V2, a_crouched: bool, b: V2, b_crouched: bool) -> bool {
        let crouched = a_crouched || b_crouched;
        !self.blocks.iter().any(|blk| {
            (blk.kind != BlockKind::Low || crouched) && blk.intersects_segment(a, b)
        })
    }

    pub fn zone_of(&self, p: V2) -> Option<&Zone> {
        self.zones
            .iter()
            .find(|z| p.x >= z.min.x && p.x <= z.max.x && p.y >= z.min.y && p.y <= z.max.y)
    }

    pub fn in_bounds(&self, p: V2) -> bool {
        p.x.abs() < ARENA_HALF - AGENT_RADIUS && p.y.abs() < ARENA_HALF - AGENT_RADIUS
    }

    /// Push a point out of any block it overlaps (used by the 2D sim for collision).
    pub fn resolve_collision(&self, mut p: V2, radius: f32) -> V2 {
        for b in &self.blocks {
            if !b.contains(p, radius) {
                continue;
            }
            let d = p - b.center;
            let px = b.half.x + radius - d.x.abs();
            let pz = b.half.y + radius - d.y.abs();
            if px < pz {
                p.x += px * d.x.signum();
            } else {
                p.y += pz * d.y.signum();
            }
        }
        let lim = ARENA_HALF - radius;
        V2::new(p.x.clamp(-lim, lim), p.y.clamp(-lim, lim))
    }
}
