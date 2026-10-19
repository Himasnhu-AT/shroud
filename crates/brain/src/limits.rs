//! Fairness limits: floors on reaction time and aim error that loosen slowly with
//! generation, so enemies get smarter rather than turning into aimbots (docs/AI.md §5).

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub min_reaction_s: f32,
    pub min_aim_error_deg: f32,
    pub damage_per_hit: f32,
}

impl Limits {
    pub fn for_generation(g: u32) -> Self {
        let g = g as f32;
        Limits {
            min_reaction_s: (0.45 - 0.015 * g).max(0.18),
            min_aim_error_deg: (5.0 - 0.25 * g).max(1.2),
            damage_per_hit: (8.0 + 0.25 * g).min(12.0),
        }
    }
}
