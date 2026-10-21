//! Difficulty levels. One knob the player picks in the menu; it scales the fairness
//! limits, squad size and which squad tactics are allowed. Evolution still runs on top.
//!
//! CONTRACT (shared by the AI, maps and UI streams): the enum, `ALL`, `label`, `blurb`
//! and `params()` field names are stable. The AI stream tunes the numbers.

use crate::limits::Limits;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Difficulty {
    Recruit,
    #[default]
    Regular,
    Veteran,
    Elite,
}

/// What a difficulty changes.
#[derive(Clone, Copy, Debug)]
pub struct DifficultyParams {
    /// Enemies per round (maps provide at least 6 enemy spawns).
    pub squad_size: usize,
    /// Multiplies `Limits::min_reaction_s` (higher = slower enemies).
    pub reaction_mult: f32,
    /// Multiplies `Limits::min_aim_error_deg` (higher = worse aim).
    pub aim_error_mult: f32,
    /// Multiplies `Limits::damage_per_hit`.
    pub damage_mult: f32,
    /// Squad coordinator (plays, roles, callouts) on/off.
    pub coordination: bool,
    /// Enemies may throw grenades / flashbangs.
    pub utility: bool,
    /// Max simultaneous attackers (attack tokens), on top of `squad::token_budget`.
    pub max_attackers: usize,
}

impl Difficulty {
    pub const ALL: [Difficulty; 4] = [Difficulty::Recruit, Difficulty::Regular, Difficulty::Veteran, Difficulty::Elite];

    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Recruit => "Recruit",
            Difficulty::Regular => "Regular",
            Difficulty::Veteran => "Veteran",
            Difficulty::Elite => "Elite",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Difficulty::Recruit => "3 enemies. Slow reactions, loose aim, no grenades.",
            Difficulty::Regular => "4 enemies. Fair fights; the squad coordinates.",
            Difficulty::Veteran => "5 enemies. Sharp aim, flanks and flashbangs.",
            Difficulty::Elite => "6 enemies. Every trick they've learned, used against you.",
        }
    }

    pub fn params(self) -> DifficultyParams {
        match self {
            Difficulty::Recruit => DifficultyParams { squad_size: 3, reaction_mult: 1.6, aim_error_mult: 1.8, damage_mult: 0.7, coordination: false, utility: false, max_attackers: 1 },
            Difficulty::Regular => DifficultyParams { squad_size: 4, reaction_mult: 1.15, aim_error_mult: 1.2, damage_mult: 0.9, coordination: true, utility: false, max_attackers: 2 },
            Difficulty::Veteran => DifficultyParams { squad_size: 5, reaction_mult: 1.0, aim_error_mult: 1.0, damage_mult: 1.0, coordination: true, utility: true, max_attackers: 3 },
            Difficulty::Elite => DifficultyParams { squad_size: 6, reaction_mult: 0.85, aim_error_mult: 0.8, damage_mult: 1.1, coordination: true, utility: true, max_attackers: 3 },
        }
    }

    /// Grenades the squad may throw per round (frags, flashbangs) when `params().utility`.
    pub fn grenades(self) -> (u32, u32) {
        match self {
            Difficulty::Recruit | Difficulty::Regular => (0, 0),
            Difficulty::Veteran => (2, 2),
            Difficulty::Elite => (3, 3),
        }
    }

    /// Generation limits scaled by this difficulty.
    pub fn limits(self, generation: u32) -> Limits {
        let base = Limits::for_generation(generation);
        let p = self.params();
        Limits {
            min_reaction_s: base.min_reaction_s * p.reaction_mult,
            min_aim_error_deg: base.min_aim_error_deg * p.aim_error_mult,
            damage_per_hit: base.damage_per_hit * p.damage_mult,
        }
    }
}
