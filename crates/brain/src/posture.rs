//! Readable body language at cover: leaning out past a tall block's edge, and ducking behind
//! low cover between peeks. Map-agnostic (works from `Arena::blocks`).

use crate::arena::{Arena, Block, BlockKind};
use crate::math::V2;

/// How far a lean moves the eye sideways (m).
pub const LEAN_M: f32 = 0.55;
/// Only lean / duck when this close to the block (m, from its surface).
pub const COVER_NEAR_M: f32 = 1.3;

fn block_dist(b: &Block, p: V2) -> f32 {
    let dx = ((p.x - b.center.x).abs() - b.half.x).max(0.0);
    let dz = ((p.y - b.center.y).abs() - b.half.y).max(0.0);
    dx.hypot(dz)
}

fn near(arena: &Arena, p: V2, kind: BlockKind) -> bool {
    arena.blocks.iter().any(|b| b.kind == kind && block_dist(b, p) < COVER_NEAR_M)
}

/// Lean side for an agent at `pos` looking toward `target` from beside tall cover:
/// +1 = lean right (`(target - pos).perp()`), -1 = left, 0 = no lean. Leans only when the
/// straight line is blocked and a lean of `LEAN_M` to that side clears it.
pub fn lean_side(arena: &Arena, pos: V2, target: V2) -> f32 {
    if !near(arena, pos, BlockKind::Tall) || pos.dist(target) < 2.0 {
        return 0.0;
    }
    if arena.line_of_sight(pos, false, target, false) {
        return 0.0;
    }
    let right = (target - pos).norm().perp();
    for s in [1.0, -1.0] {
        if arena.line_of_sight(pos + right * (LEAN_M * s), false, target, false) {
            return s;
        }
    }
    0.0
}

/// Low cover between `pos` (within `COVER_NEAR_M`) and `target`: crouching hides, standing peeks.
pub fn low_cover_toward(arena: &Arena, pos: V2, target: V2) -> bool {
    near(arena, pos, BlockKind::Low) && !arena.line_of_sight(pos, true, target, false) && pos.dist(target) > 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leans_past_a_corner() {
        let a = Arena::yard();
        // Tower is ±4 m around the origin. Standing at its south face, near the east edge,
        // looking at a point north-east past the corner.
        let pos = V2::new(3.6, 4.8);
        let target = V2::new(8.0, -10.0);
        let s = lean_side(&a, pos, target);
        assert_ne!(s, 0.0);
        let right = (target - pos).norm().perp();
        assert!(a.line_of_sight(pos + right * (LEAN_M * s), false, target, false));
        // In the open: no lean.
        assert_eq!(lean_side(&a, V2::new(0.0, 20.0), V2::new(0.0, 28.0)), 0.0);
    }

    #[test]
    fn low_cover_detected() {
        let a = Arena::yard();
        // Low crates at (7, 10) half 1.2: stand just south of them, player further north.
        assert!(low_cover_toward(&a, V2::new(7.0, 11.9), V2::new(7.0, 0.0 - 6.0)));
        assert!(!low_cover_toward(&a, V2::new(0.0, 20.0), V2::new(0.0, 28.0)));
    }
}
