//! Utility grenades: frags (flush a player out of cover), flashbangs (blind him before a
//! push) and smokes (a cloud that blocks sight; the player buys them, see [`SmokeCloud`]). The squad coordinator decides who throws what where (`squad::ThrowOrder`); this
//! module holds the shared numbers, the effect falloffs, and the 2D sim's rough model, so
//! evolution trains with the same utility the game uses.

use crate::arena::{Arena, BlockKind};
use crate::math::V2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GrenadeKind {
    Frag,
    Flash,
    Smoke,
}

impl GrenadeKind {
    pub fn label(self) -> &'static str {
        match self {
            GrenadeKind::Frag => "frag",
            GrenadeKind::Flash => "flashbang",
            GrenadeKind::Smoke => "smoke",
        }
    }
    pub const ALL: [GrenadeKind; 3] = [GrenadeKind::Frag, GrenadeKind::Flash, GrenadeKind::Smoke];
    /// Seconds from release to detonation.
    pub fn fuse(self) -> f32 {
        match self {
            GrenadeKind::Frag => FRAG_FUSE_S,
            GrenadeKind::Flash => FLASH_FUSE_S,
            GrenadeKind::Smoke => SMOKE_FUSE_S,
        }
    }
}

pub const FRAG_FUSE_S: f32 = 2.2;
pub const FLASH_FUSE_S: f32 = 1.5;
/// Smoke pops this long after release (or once it stops rolling, whichever is later).
pub const SMOKE_FUSE_S: f32 = 1.8;
/// Smoke cloud: radius at full size (m), how long it lasts (s) and how long it takes to bloom.
pub const SMOKE_RADIUS: f32 = 3.2;
pub const SMOKE_S: f32 = 15.0;
pub const SMOKE_BLOOM_S: f32 = 1.2;
/// Frag: full damage inside `FRAG_INNER`, falling to 0 at `FRAG_RADIUS` (quadratic).
pub const FRAG_RADIUS: f32 = 6.5;
pub const FRAG_INNER: f32 = 1.5;
pub const FRAG_DAMAGE: f32 = 90.0;
/// Flash: no effect beyond this.
pub const FLASH_RANGE: f32 = 22.0;
/// Longest whiteout (s), at full intensity.
pub const FLASH_MAX_S: f32 = 3.2;
/// Throw range limits (m).
pub const THROW_MIN: f32 = 5.0;
pub const THROW_MAX: f32 = 26.0;
/// The thrower faces the target this long before releasing (a visible wind-up).
pub const WINDUP_S: f32 = 0.55;
/// Squad-wide gap between throws.
pub const SQUAD_COOLDOWN_S: f32 = 7.0;

/// Frag damage at distance `d` from the blast (before LOS and difficulty scaling).
pub fn frag_damage(d: f32) -> f32 {
    if d <= FRAG_INNER {
        FRAG_DAMAGE
    } else if d >= FRAG_RADIUS {
        0.0
    } else {
        let f = 1.0 - (d - FRAG_INNER) / (FRAG_RADIUS - FRAG_INNER);
        FRAG_DAMAGE * f * f
    }
}

/// Flash intensity 0..1 for a viewer at distance `d` whose look direction makes `facing`
/// (cosine) with the direction to the flash. Needs line of sight (caller checks).
pub fn flash_intensity(facing: f32, d: f32) -> f32 {
    if d >= FLASH_RANGE {
        return 0.0;
    }
    // Looking at it: full; side-on: partial; back turned: a little glare.
    let look = if facing > 0.5 {
        1.0
    } else if facing > -0.2 {
        0.35 + 0.65 * (facing + 0.2) / 0.7
    } else {
        0.15
    };
    let near = 1.0 - (d / FLASH_RANGE).powi(2);
    (look * near).clamp(0.0, 1.0)
}

/// How long a flash of `intensity` whites the screen out.
pub fn flash_duration(intensity: f32) -> f32 {
    if intensity <= 0.05 { 0.0 } else { 0.4 + (FLASH_MAX_S - 0.4) * intensity }
}

/// A smoke cloud on the ground: a column that blocks sight lines through it (enemies' eyes, the
/// 2D line-of-sight checks via `Arena::smokes`). It blooms, holds, and thins out at the end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmokeCloud {
    pub at: V2,
    /// Seconds since it popped.
    pub age: f32,
}

impl SmokeCloud {
    pub fn new(at: V2) -> Self {
        SmokeCloud { at, age: 0.0 }
    }
    /// Current radius: grows over the bloom, shrinks over the last 1.5 s.
    pub fn radius(&self) -> f32 {
        let grow = (self.age / SMOKE_BLOOM_S).clamp(0.0, 1.0);
        let fade = ((SMOKE_S - self.age) / 1.5).clamp(0.0, 1.0);
        SMOKE_RADIUS * grow.sqrt() * fade
    }
    pub fn done(&self) -> bool {
        self.age >= SMOKE_S
    }
    /// Does the segment `a`–`b` pass through the opaque core (85% of the radius)? A viewer
    /// standing inside sees nothing beyond arm's length either.
    pub fn blocks(&self, a: V2, b: V2) -> bool {
        let r = self.radius() * 0.85;
        if r <= 0.3 {
            return false;
        }
        let d = b - a;
        let len2 = d.dot(d);
        let t = if len2 < 1e-9 { 0.0 } else { ((self.at - a).dot(d) / len2).clamp(0.0, 1.0) };
        let closest = a + d * t;
        closest.dist(self.at) < r && a.dist(b) > 1.0
    }
}

/// Does any cloud in `smokes` block the sight line `a`–`b`?
pub fn smoke_blocks(smokes: &[SmokeCloud], a: V2, b: V2) -> bool {
    smokes.iter().any(|s| s.blocks(a, b))
}

/// Does a lob from `from` reach `to`? Low cover is cleared by the arc; tall blocks and walls are not.
pub fn lob_clear(arena: &Arena, from: V2, to: V2) -> bool {
    !arena.blocks.iter().any(|b| b.kind != BlockKind::Low && b.intersects_segment(from, to))
}

/// Where a grenade thrown from `from` at `target` comes to rest in 2D: the target, or short
/// of the first tall block in the way.
pub fn landing(arena: &Arena, from: V2, target: V2) -> V2 {
    let d = target - from;
    let len = d.len();
    if len < 0.1 {
        return target;
    }
    let steps = (len / 0.5).ceil() as usize;
    let mut last = from;
    for k in 1..=steps {
        let p = from + d * (k as f32 / steps as f32);
        if !lob_clear(arena, last, p) {
            return last;
        }
        last = p;
    }
    target
}

/// A grenade in the 2D trainer: flies straight to its landing point, then waits out its fuse.
#[derive(Clone, Copy, Debug)]
pub struct SimGrenade {
    pub kind: GrenadeKind,
    pub thrower: usize,
    pub from: V2,
    pub at: V2,
    pub fuse: f32,
}

impl SimGrenade {
    pub fn new(arena: &Arena, kind: GrenadeKind, thrower: usize, from: V2, target: V2) -> Self {
        SimGrenade { kind, thrower, from, at: landing(arena, from, target), fuse: kind.fuse() }
    }
    /// Where it is now (it travels for the first ~45% of the fuse).
    pub fn pos(&self) -> V2 {
        let f = (1.0 - self.fuse / self.kind.fuse()) / 0.45;
        self.from.lerp(self.at, f.clamp(0.0, 1.0))
    }
}

/// Frag damage to a target at `p` (crouched or not) from a blast at `at`, with cover: a tall
/// block in between stops it; low cover halves it for a crouched target.
pub fn frag_damage_at(arena: &Arena, at: V2, p: V2, crouched: bool) -> f32 {
    let d = at.dist(p);
    if d >= FRAG_RADIUS {
        return 0.0;
    }
    if !arena.line_of_sight(at, false, p, false) {
        return 0.0;
    }
    let base = frag_damage(d);
    if crouched && !arena.line_of_sight(at, true, p, true) { base * 0.5 } else { base }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloffs() {
        assert_eq!(frag_damage(0.5), FRAG_DAMAGE);
        assert_eq!(frag_damage(FRAG_RADIUS + 0.1), 0.0);
        assert!(frag_damage(3.0) < FRAG_DAMAGE && frag_damage(3.0) > frag_damage(5.0));
        assert!(flash_intensity(1.0, 3.0) > 0.9);
        assert!(flash_intensity(-1.0, 3.0) < 0.2);
        assert!(flash_intensity(1.0, 3.0) > flash_intensity(0.0, 3.0));
        assert_eq!(flash_intensity(1.0, FLASH_RANGE + 1.0), 0.0);
        assert_eq!(flash_duration(0.0), 0.0);
        assert!(flash_duration(1.0) > flash_duration(0.3));
    }

    #[test]
    fn smoke_blocks_sight_through_it() {
        let mut s = SmokeCloud::new(V2::new(0.0, 0.0));
        let (a, b) = (V2::new(-10.0, 0.5), V2::new(10.0, -0.5));
        assert!(!s.blocks(a, b), "not yet bloomed");
        s.age = 3.0;
        assert!(s.blocks(a, b), "through the middle");
        assert!(!s.blocks(V2::new(-10.0, 5.0), V2::new(10.0, 5.0)), "past the edge");
        assert!(!s.blocks(V2::new(-10.0, 0.0), V2::new(-5.0, 0.0)), "stops short of it");
        assert!(s.blocks(V2::new(0.5, 0.0), V2::new(8.0, 0.0)), "from inside");
        s.age = SMOKE_S;
        assert!(s.done() && !s.blocks(a, b), "gone");
        let mut arena = Arena::foundry();
        let (p, q) = (V2::new(-10.0, 18.0), V2::new(-10.0, 28.0));
        assert!(arena.line_of_sight(p, false, q, false));
        arena.smokes.push(SmokeCloud { at: V2::new(-10.0, 23.0), age: 5.0 });
        assert!(!arena.line_of_sight(p, false, q, false), "the arena's LOS respects smoke");
    }

    #[test]
    fn landing_stops_at_tall_cover() {
        let a = Arena::foundry();
        // Through the center tower (tall, ±4 m): lands short of it.
        let l = landing(&a, V2::new(0.0, -12.0), V2::new(0.0, 12.0));
        assert!(l.y < -3.9, "{l:?}");
        // Open line: reaches the target.
        let t = V2::new(-10.0, 28.0);
        assert_eq!(landing(&a, V2::new(-10.0, 18.0), t), t);
    }
}
