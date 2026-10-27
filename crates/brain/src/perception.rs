//! Stimulus perception shared by the game and the 2D sim: alert states, hearing with distance
//! falloff and occlusion, near-miss detection (docs/research/adaptive-squad-ai.md, addendum).

use crate::arena::{Arena, BlockKind};
use crate::math::V2;

/// Awareness, per enemy and for the whole squad. Ordered: `max` is "most alert".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Alert {
    #[default]
    Unaware,
    /// Heard something (footsteps, a reload): investigate, face the sound.
    Suspicious,
    /// Saw the player, got shot or shot at, or heard a gunshot: fight.
    Combat,
}

impl Alert {
    pub fn label(self) -> &'static str {
        match self {
            Alert::Unaware => "unaware",
            Alert::Suspicious => "suspicious",
            Alert::Combat => "combat",
        }
    }
}

/// Audible range (m) of each sound in the open.
pub const GUNSHOT_RANGE: f32 = 45.0;
pub const RUN_STEP_RANGE: f32 = 12.0;
pub const WALK_STEP_RANGE: f32 = 6.0;
pub const CROUCH_STEP_RANGE: f32 = 3.0;
pub const RELOAD_RANGE: f32 = 8.0;
/// A bullet passing within this distance (or striking within it) is "being shot at".
pub const NEAR_MISS_RADIUS: f32 = 2.5;
/// Each tall block between source and listener multiplies loudness by this.
pub const OCCLUSION: f32 = 0.3;
/// Loudness below this is not heard.
pub const HEARD_MIN: f32 = 0.05;
/// Combat calms to Suspicious after this long without fresh information, then to Unaware.
pub const COMBAT_CALM_S: f32 = 15.0;
pub const SUSPICIOUS_CALM_S: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Gunshot,
    Footstep,
    Reload,
}

impl Sound {
    /// Alert level a sound raises the listener to.
    pub fn alert(self) -> Alert {
        match self {
            Sound::Gunshot => Alert::Combat,
            _ => Alert::Suspicious,
        }
    }
}

/// Open-air range (m) at which a shot from gun `kind` is still heard. Scaled from the gun's
/// `loudness_m` so the rifle keeps the original `GUNSHOT_RANGE` (a pistol carries less, a
/// shotgun or marksman rifle more).
pub fn gunshot_range(kind: crate::weapons::WeaponKind) -> f32 {
    gunshot_range_m(kind.spec().loudness_m)
}

/// `gunshot_range` from a raw `loudness_m` (e.g. the game's `ShotFired::loudness_m`).
pub fn gunshot_range_m(loudness_m: f32) -> f32 {
    loudness_m * GUNSHOT_RANGE / crate::weapons::RIFLE.loudness_m
}

/// A sound a listener picked up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heard {
    pub sound: Sound,
    pub pos: V2,
    /// 0..1 after falloff and occlusion.
    pub loud: f32,
}

/// The loudest of `sounds` (kind, source, open-air range) audible at `listener`.
pub fn listen(arena: &Arena, sounds: &[(Sound, V2, f32)], listener: V2) -> Option<Heard> {
    sounds
        .iter()
        .filter_map(|&(sound, pos, range)| hear(arena, pos, listener, range).map(|loud| Heard { sound, pos, loud }))
        .max_by(|a, b| (a.loud * a.sound.weight()).total_cmp(&(b.loud * b.sound.weight())))
}

impl Sound {
    fn weight(self) -> f32 {
        if self == Sound::Gunshot { 2.0 } else { 1.0 }
    }
}

/// How far the player's footsteps carry at ground speed `speed` (m/s). Crouched and slow
/// (sneaking) is silent; walking is quiet; running is loud.
pub fn footstep_range(speed: f32, crouched: bool) -> f32 {
    if speed < 0.5 {
        0.0
    } else if crouched {
        if speed < 1.6 { 0.0 } else { CROUCH_STEP_RANGE }
    } else if speed > 4.5 {
        RUN_STEP_RANGE
    } else {
        WALK_STEP_RANGE
    }
}

/// Loudness (0..1) at `listener` of a sound at `src` that carries `range` metres in the open:
/// linear distance falloff, times `OCCLUSION` per tall block or wall in between.
pub fn loudness(arena: &Arena, src: V2, listener: V2, range: f32) -> f32 {
    let d = src.dist(listener);
    if range <= 0.0 || d >= range {
        return 0.0;
    }
    let walls = arena.blocks.iter().filter(|b| b.kind != BlockKind::Low && b.intersects_segment(src, listener)).count();
    (1.0 - d / range) * OCCLUSION.powi(walls as i32)
}

/// Is `listener` heard-able for this sound? Returns the loudness if so.
pub fn hear(arena: &Arena, src: V2, listener: V2, range: f32) -> Option<f32> {
    let l = loudness(arena, src, listener, range);
    (l >= HEARD_MIN).then_some(l)
}

/// Did a shot from `origin` along unit `dir` pass close to (or hit near) `listener`?
pub fn near_miss(arena: &Arena, origin: V2, dir: V2, listener: V2) -> bool {
    let to = listener - origin;
    let along = to.dot(dir);
    if along <= 0.0 || along > 60.0 {
        return false;
    }
    let closest = origin + dir * along;
    closest.dist(listener) < NEAR_MISS_RADIUS && arena.line_of_sight(origin, false, closest, false)
}

/// Uncertainty (m) of a heard position: quiet sounds are hard to place.
pub fn heard_sigma(loud: f32) -> f32 {
    1.5 + 6.0 * (1.0 - loud.clamp(0.0, 1.0))
}
