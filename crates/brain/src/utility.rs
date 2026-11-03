//! Utility AI: score tactical goals and pick one (docs/AI.md §2).

use crate::genome::Traits;
use crate::loadout;
use crate::weapons::WeaponKind;
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
    /// The player just took damage.
    pub player_hurt: bool,
    /// The squad is in combat (someone saw him, got shot, or heard a gunshot).
    pub combat: bool,
    /// Holds an attack token (may shoot/push); without one, reposition to a flank.
    pub token: bool,
}

/// Retreat only below this fraction of the genome's retreat health (aggressive baseline).
pub const RETREAT_SCALE: f32 = 0.6;

pub fn score(goal: Goal, i: &UtilityInput, t: &Traits) -> f32 {
    let seen_recently = i.visible || i.time_since_seen < 3.0 || i.combat;
    let range_fit = 1.0 - ((i.dist - t.preferred_range).abs() / 20.0).min(1.0);
    // Without a token, attacking/pushing yields to repositioning (unless point blank).
    let tok = if i.token || i.dist < 12.0 { 1.0 } else { 0.45 };
    // Windows to push: reloading, hurt, low.
    let window = if i.player_reloading { 0.6 } else { 0.0 } + if i.player_hurt { 0.25 } else { 0.0 } + if i.player_hp < 0.4 { 0.25 } else { 0.0 };
    // The gun: how well it works at this range (a shotgun at 30 m wants to close in, a
    // marksman at 5 m wants out).
    let gun_q = loadout::range_quality(t.weapon, i.dist);
    let (closer, rusher, marksman) = match t.weapon {
        WeaponKind::Shotgun => (1.0, 0.5, false),
        WeaponKind::Smg => (0.6, 1.0, false),
        WeaponKind::Marksman | WeaponKind::Sniper => (0.0, 0.0, true),
        _ => (0.0, 0.0, false),
    };
    match goal {
        Goal::Attack if i.visible => {
            // Well beyond preferred range: close in (Push) rather than plink from afar.
            let too_far = ((i.dist - t.preferred_range - 6.0).max(0.0) / 15.0).min(1.0);
            (0.45 + 0.3 * t.aggression + 0.15 * i.hp + 0.2 * range_fit - 0.45 * too_far) * tok * (0.55 + 0.45 * gun_q)
        }
        Goal::Push if seen_recently && marksman => {
            // Marksmen don't charge; they take the window from where they are.
            (0.1 + 0.2 * t.aggression + 0.5 * window) * tok
        }
        Goal::Push if seen_recently => {
            let far = if i.dist > t.preferred_range + 4.0 { 0.25 + 0.3 * closer * (1.0 - gun_q) } else { 0.0 };
            // He's badly hurt and just broke line of sight: run him down before he heals up
            // behind cover (any healthy member, token or not).
            let finish = !i.visible && i.player_hp < 0.35 && i.time_since_seen < 4.0 && i.hp > 0.4;
            if finish {
                0.25 + 0.5 * t.aggression + far + 0.15 * i.hp + window + 0.4
            } else {
                (0.25 + 0.5 * t.aggression * (1.0 - 0.5 * i.player_hp) + far + 0.15 * i.hp + window) * tok
            }
        }
        Goal::Flank if seen_recently && i.dist > 8.0 => {
            let support = if i.allies_engaged > 0 { 1.0 } else { 0.3 };
            let crowd = if i.flankers >= 2 { 0.6 } else { 0.0 };
            let reposition = if i.token { 0.0 } else { 0.4 };
            0.1 + 0.6 * t.flank_bias * support - crowd + reposition + 0.2 * rusher - if marksman { 0.3 } else { 0.0 }
        }
        Goal::TakeCover => {
            let pressure = if i.player_reloading { 0.0 } else if i.taking_fire { 0.8 } else { 0.25 * i.player_aiming_at_me.max(0.0) * i.visible as u8 as f32 };
            // A marksman caught close falls back to cover rather than duel at shotgun range.
            let caught_close = if marksman && i.visible && i.dist < 10.0 { 0.35 } else { 0.0 };
            pressure * (0.15 + 0.45 * t.cover_affinity) + (1.0 - i.hp) * 0.3 * t.cover_affinity + caught_close
        }
        Goal::Retreat if i.hp < t.retreat_health * RETREAT_SCALE => 0.95,
        Goal::Ambush if !i.visible && !i.combat && i.time_since_seen > 1.5 => 0.1 + 0.8 * t.prediction_trust * i.model_confidence,
        Goal::Hunt if !i.visible => {
            if i.combat { 0.5 } else { 0.35 }
        }
        _ => 0.0,
    }
}

/// Score of a goal the squad ordered: beats normal tactics (attacking included), but not
/// retreating at low health or a cover-loving enemy under heavy fire (survival instinct stays).
pub const ORDER_PRIORITY: f32 = 1.1;
/// Seconds without contact after which a retreated member follows squad orders again.
pub const REGROUP_S: f32 = 6.0;

/// Pick the best goal, with hysteresis toward the current one. `ordered`: the squad's order;
/// `sticky`: it holds even while this enemy is in contact.
pub fn choose(current: Goal, i: &UtilityInput, t: &Traits, ordered: Option<Goal>, sticky: bool) -> Goal {
    let mut best = (f32::MIN, current);
    for g in Goal::ALL {
        let raw = score(g, i, t);
        // Orders lapse while under fire and while in contact (fight first), except sticky ones
        // (flanking, following the entry), which keep going and shoot on the move.
        // In a fight, token holders fight on their own judgement; the rest run the play.
        // Static holds lapse in a fight too: pressure beats waiting.
        // An ordered push (the entry) keeps pushing in contact: that is the aggressive play.
        let pushing = ordered == Some(Goal::Push);
        let lapse = i.taking_fire || (i.in_contact && !sticky && !pushing) || (i.combat && !sticky && !pushing && (i.token || ordered == Some(Goal::Hold)));
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
