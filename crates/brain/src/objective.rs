//! Bomb defusal: the player (attacker) carries the bomb and plants it on site A or B; the squad
//! (defenders) holds the sites and must defuse a planted bomb before it goes off.
//!
//! Rules (CS-style, solo attacker):
//! - Plant: hold the plant key inside a site, standing, for [`PLANT_S`]. Letting go resets it.
//! - Planted: the bomb goes off after [`FUSE_S`] (attackers win). A defender standing within
//!   [`DEFUSE_REACH`] of it for [`DEFUSE_S`] defuses it (defenders win); moving off resets.
//! - Before a plant: the squad wiped → attackers win; the player dead or the clock out →
//!   defenders win.
//! - After a plant the round clock no longer matters, and the player dying does not end the
//!   round: only the bomb (or the squad wiped) does.
//!
//! The game owns the live state; the AI only sees [`PlantedBomb`] through
//! `SquadCoordinator::set_bomb`.

use crate::math::V2;

/// Seconds of holding the plant key to plant.
pub const PLANT_S: f32 = 3.2;
/// Seconds from plant to explosion.
pub const FUSE_S: f32 = 40.0;
/// Seconds a defender needs on the bomb to defuse (no kits).
pub const DEFUSE_S: f32 = 7.0;
/// How close (m, ground plane) a defender must stand to defuse.
pub const DEFUSE_REACH: f32 = 1.5;
/// Explosion lethal radius (m) and the radius past which it does no damage.
pub const BLAST_KILL_M: f32 = 8.0;
pub const BLAST_MAX_M: f32 = 22.0;

/// A plantable area: an axis-aligned rectangle with a letter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BombSite {
    pub letter: char,
    pub min: V2,
    pub max: V2,
}

impl BombSite {
    pub fn new(letter: char, x0: f32, z0: f32, x1: f32, z1: f32) -> Self {
        BombSite { letter, min: V2::new(x0.min(x1), z0.min(z1)), max: V2::new(x0.max(x1), z0.max(z1)) }
    }
    pub fn contains(&self, p: V2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
    pub fn center(&self) -> V2 {
        V2::new((self.min.x + self.max.x) * 0.5, (self.min.y + self.max.y) * 0.5)
    }
}

/// What the AI knows about a planted bomb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlantedBomb {
    pub pos: V2,
    /// Index into `Arena::sites`.
    pub site: usize,
    /// Seconds until it goes off.
    pub fuse_left: f32,
    /// A defender is on it right now (progress 0..1).
    pub defuse_progress: f32,
}

/// Damage the blast deals at distance `d` (m): lethal inside [`BLAST_KILL_M`], falling off
/// linearly to zero at [`BLAST_MAX_M`]. Cover is the caller's business.
pub fn blast_damage(d: f32) -> f32 {
    if d <= BLAST_KILL_M {
        500.0
    } else if d >= BLAST_MAX_M {
        0.0
    } else {
        let t = (d - BLAST_KILL_M) / (BLAST_MAX_M - BLAST_KILL_M);
        // 100 HP at the kill edge down to 0.
        100.0 * (1.0 - t)
    }
}

/// The site containing `p`, if any.
pub fn site_at(sites: &[BombSite], p: V2) -> Option<usize> {
    sites.iter().position(|s| s.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blast_falls_off() {
        assert!(blast_damage(2.0) >= 100.0);
        assert!(blast_damage(15.0) > 0.0 && blast_damage(15.0) < 100.0);
        assert_eq!(blast_damage(30.0), 0.0);
    }

    #[test]
    fn site_contains() {
        let s = BombSite::new('A', 10.0, 10.0, -10.0, -5.0);
        assert!(s.contains(V2::new(0.0, 0.0)));
        assert!(!s.contains(V2::new(0.0, 11.0)));
        assert_eq!(site_at(&[s], V2::new(1.0, 1.0)), Some(0));
    }
}
