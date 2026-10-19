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
    /// Squad order: go to a position (if any) and hold the angle there.
    Hold,
}

impl Goal {
    pub const COUNT: usize = 8;
    pub const ALL: [Goal; Goal::COUNT] = [Goal::Attack, Goal::Push, Goal::Flank, Goal::TakeCover, Goal::Retreat, Goal::Ambush, Goal::Hunt, Goal::Hold];
    pub fn label(self) -> &'static str {
        match self {
            Goal::Attack => "attack",
            Goal::Push => "push",
            Goal::Flank => "flank",
            Goal::TakeCover => "cover",
            Goal::Retreat => "retreat",
            Goal::Ambush => "ambush",
            Goal::Hunt => "hunt",
            Goal::Hold => "hold",
        }
    }
}

/// Everything the scorer needs, already reduced to scalars.
pub struct UtilityInput {
    /// This enemy itself saw the player within the last ~1.5 s (not just the radio).
    pub in_contact: bool,
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
    pub player_reloading: bool,
}

pub fn score(goal: Goal, i: &UtilityInput, t: &Traits) -> f32 {
    let seen_recently = i.visible || i.time_since_seen < 3.0;
    let range_fit = 1.0 - ((i.dist - t.preferred_range).abs() / 20.0).min(1.0);
    match goal {
        Goal::Attack if i.visible => {
            // Well beyond preferred range: close in (Push) rather than plink from afar.
            let too_far = ((i.dist - t.preferred_range - 6.0).max(0.0) / 15.0).min(1.0);
            0.35 + 0.25 * t.aggression + 0.2 * i.hp + 0.2 * range_fit - 0.5 * too_far
        }
        Goal::Push if seen_recently => {
            let far = if i.dist > t.preferred_range + 4.0 { 0.25 } else { 0.0 };
            // The player reloading is the classic window to push.
            let reload = if i.player_reloading { 0.35 } else { 0.0 };
            0.15 + 0.45 * t.aggression * (1.0 - i.player_hp) + far + 0.15 * i.hp + reload
        }
        Goal::Flank if seen_recently && i.dist > 8.0 => {
            let support = if i.allies_engaged > 0 { 1.0 } else { 0.3 };
            let crowd = if i.flankers >= 2 { 0.6 } else { 0.0 };
            0.1 + 0.6 * t.flank_bias * support - crowd
        }
        Goal::TakeCover => {
            let pressure = if i.player_reloading { 0.0 } else if i.taking_fire { 1.0 } else { 0.4 * i.player_aiming_at_me.max(0.0) * i.visible as u8 as f32 };
            pressure * (0.25 + 0.55 * t.cover_affinity) + (1.0 - i.hp) * 0.35 * t.cover_affinity
        }
        Goal::Retreat if i.hp < t.retreat_health => 0.95,
        Goal::Ambush if !i.visible && i.time_since_seen > 1.5 => 0.1 + 0.8 * t.prediction_trust * i.model_confidence,
        Goal::Hunt if !i.visible => 0.35,
        _ => 0.0,
    }
}

/// Score of a goal the squad ordered: beats normal tactics (attacking included), but not
/// retreating at low health or a cover-loving enemy under heavy fire (survival instinct stays).
pub const ORDER_PRIORITY: f32 = 1.1;
/// Seconds without contact after which a retreated member follows squad orders again.
pub const REGROUP_S: f32 = 6.0;

/// Pick the best goal, with hysteresis toward the current one. `ordered`: the squad's order.
pub fn choose(current: Goal, i: &UtilityInput, t: &Traits, ordered: Option<Goal>) -> Goal {
    let mut best = (f32::MIN, current);
    for g in Goal::ALL {
        let raw = score(g, i, t);
        // Orders lapse while under fire and while in contact (fight first), except movement
        // orders (flank, push/follow), which keep going and shoot on the move.
        let lapse = i.taking_fire || (i.in_contact && !matches!(g, Goal::Flank | Goal::Push));
        let base = match ordered {
            Some(o) if o == g && !lapse => raw.max(ORDER_PRIORITY),
            // Hurt members fall back while the fight is hot; once it has gone quiet the
            // squad's order (regroup, sweep) wins again instead of cowering for the round.
            Some(_) if g == Goal::Retreat && raw > 0.0 && (i.taking_fire || i.time_since_seen < REGROUP_S) => raw.max(ORDER_PRIORITY + 0.1),
            _ => raw,
        };
        let s = base + if g == current { 0.1 } else { 0.0 };
        if s > best.0 {
            best = (s, g);
        }
    }
    best.1
}
