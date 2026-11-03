//! Fairness limits: floors on reaction time and aim error, and enemy damage per hit.
//!
//! These are fixed: they do not change with generation. Evolution only improves *technique*
//! (positioning, flanking, cover, timing, teamwork, weapon choice), never raw mechanics, so a
//! generation-20 squad aims and hits exactly as hard as a generation-1 squad on the same
//! difficulty. Difficulty (`difficulty.rs`) is the only thing that scales these.

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub min_reaction_s: f32,
    pub min_aim_error_deg: f32,
    pub damage_per_hit: f32,
}

/// Enemy reaction time before difficulty scaling (s).
pub const REACTION_S: f32 = 0.44;
/// Enemy aim error before difficulty scaling (degrees).
pub const AIM_ERROR_DEG: f32 = 5.0;
/// Enemy rifle damage per hit before difficulty scaling.
pub const DAMAGE_PER_HIT: f32 = 8.0;

impl Limits {
    /// The limits in force at `generation`. The argument is kept so callers read naturally, but
    /// every generation gets the same mechanics: improvement comes from evolved technique only.
    pub fn for_generation(_generation: u32) -> Self {
        Limits { min_reaction_s: REACTION_S, min_aim_error_deg: AIM_ERROR_DEG, damage_per_hit: DAMAGE_PER_HIT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mechanics_do_not_grow_with_generation() {
        let a = Limits::for_generation(0);
        let b = Limits::for_generation(50);
        assert_eq!(a.min_reaction_s, b.min_reaction_s);
        assert_eq!(a.min_aim_error_deg, b.min_aim_error_deg);
        assert_eq!(a.damage_per_hit, b.damage_per_hit);
    }
}
