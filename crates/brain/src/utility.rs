//! Utility AI: score tactical goals and pick one (docs/AI.md §2).

use crate::genome::Traits;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Goal {
    Attack,
    Push,
    Flank,
    TakeCover,
    Retreat,
    Ambush,
    Hunt,
}

impl Goal {
    pub const ALL: [Goal; 7] = [Goal::Attack, Goal::Push, Goal::Flank, Goal::TakeCover, Goal::Retreat, Goal::Ambush, Goal::Hunt];
    pub fn label(self) -> &'static str {
        match self {
            Goal::Attack => "attack",
            Goal::Push => "push",
            Goal::Flank => "flank",
            Goal::TakeCover => "cover",
            Goal::Retreat => "retreat",
            Goal::Ambush => "ambush",
            Goal::Hunt => "hunt",
        }
    }
}

/// Everything the scorer needs, already reduced to scalars.
pub struct UtilityInput {
    pub visible: bool,
    pub dist: f32,
    pub hp: f32,
    pub player_hp: f32,
    pub taking_fire: bool,
    pub time_since_seen: f32,
    pub allies_engaged: usize,
    pub flankers: usize,
    pub model_confidence: f32,
    pub player_aiming_at_me: f32,
}

pub fn score(goal: Goal, i: &UtilityInput, t: &Traits) -> f32 {
    let seen_recently = i.visible || i.time_since_seen < 3.0;
    let range_fit = 1.0 - ((i.dist - t.preferred_range).abs() / 20.0).min(1.0);
    match goal {
        Goal::Attack if i.visible => 0.35 + 0.25 * t.aggression + 0.2 * i.hp + 0.2 * range_fit,
        Goal::Push if seen_recently => {
            let far = if i.dist > t.preferred_range + 4.0 { 0.25 } else { 0.0 };
            0.15 + 0.45 * t.aggression * (1.0 - i.player_hp) + far + 0.15 * i.hp
        }
        Goal::Flank if seen_recently && i.dist > 8.0 => {
            let support = if i.allies_engaged > 0 { 1.0 } else { 0.3 };
            let crowd = if i.flankers >= 2 { 0.6 } else { 0.0 };
            0.1 + 0.6 * t.flank_bias * support - crowd
        }
        Goal::TakeCover => {
            let pressure = if i.taking_fire { 1.0 } else { 0.4 * i.player_aiming_at_me.max(0.0) * i.visible as u8 as f32 };
            pressure * (0.25 + 0.55 * t.cover_affinity) + (1.0 - i.hp) * 0.35 * t.cover_affinity
        }
        Goal::Retreat if i.hp < t.retreat_health => 0.95,
        Goal::Ambush if !i.visible && i.time_since_seen > 1.5 => 0.1 + 0.8 * t.prediction_trust * i.model_confidence,
        Goal::Hunt if !i.visible => 0.35,
        _ => 0.0,
    }
}

/// Pick the best goal, with hysteresis toward the current one.
pub fn choose(current: Goal, i: &UtilityInput, t: &Traits) -> Goal {
    let mut best = (f32::MIN, current);
    for g in Goal::ALL {
        let s = score(g, i, t) + if g == current { 0.1 } else { 0.0 };
        if s > best.0 {
            best = (s, g);
        }
    }
    best.1
}
