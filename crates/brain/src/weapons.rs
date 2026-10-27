//! Shared weapon contract: the guns in Shroud and their ballistic stats.
//!
//! One table drives the player's arsenal (game `weapon/`), enemy loadouts (`mind`, `squad`) and the
//! offline trainer (`sim`, `ghost`), so a gun hits the same in a real match and in training.
//! Pure data; no engine types.

use serde::{Deserialize, Serialize};

/// Which slot a gun lives in on the player (1 = primary, 2 = sidearm).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Slot {
    Primary,
    Secondary,
}

/// Fire mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FireMode {
    Auto,
    Semi,
    /// Pump/bolt: one shot, then `cycle_s` before the next.
    Single,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum WeaponKind {
    /// "Vector-9" sidearm pistol. Everyone carries one.
    Pistol,
    /// "Hornet" SMG: close range, fast, inaccurate on the move.
    Smg,
    /// "Kestrel" assault rifle: the all-rounder (the original rifle).
    #[default]
    Rifle,
    /// "Breacher" pump shotgun: pellets, brutal inside 8 m.
    Shotgun,
    /// "Longbow" marksman rifle: scoped, slow, one-shot headshots.
    Marksman,
}

impl WeaponKind {
    pub const ALL: [WeaponKind; 5] =
        [WeaponKind::Pistol, WeaponKind::Smg, WeaponKind::Rifle, WeaponKind::Shotgun, WeaponKind::Marksman];
    pub const PRIMARIES: [WeaponKind; 4] =
        [WeaponKind::Smg, WeaponKind::Rifle, WeaponKind::Shotgun, WeaponKind::Marksman];

    pub fn spec(self) -> &'static WeaponSpec {
        match self {
            WeaponKind::Pistol => &PISTOL,
            WeaponKind::Smg => &SMG,
            WeaponKind::Rifle => &RIFLE,
            WeaponKind::Shotgun => &SHOTGUN,
            WeaponKind::Marksman => &MARKSMAN,
        }
    }

    pub fn label(self) -> &'static str {
        self.spec().name
    }
}

/// Ballistics and handling. Distances in metres, times in seconds, angles in radians.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponSpec {
    pub kind: WeaponKind,
    pub name: &'static str,
    pub slot: Slot,
    pub mode: FireMode,
    /// Damage per bullet (per pellet for shotguns) at or inside `falloff_start`.
    pub damage: f32,
    /// Damage at and beyond `falloff_end`, as a fraction of `damage`.
    pub falloff_min: f32,
    pub falloff_start: f32,
    pub falloff_end: f32,
    pub headshot_mult: f32,
    /// Bullets per trigger pull (pellets).
    pub pellets: u32,
    /// Rounds per second while the trigger is held (Auto) or max rate (Semi).
    pub fire_rate: f32,
    /// Time between shots for `Single` (pump/bolt).
    pub cycle_s: f32,
    pub mag: u32,
    /// Spare rounds carried at round start.
    pub reserve: u32,
    pub reload_s: f32,
    /// Reload when the mag is empty (adds a chamber/bolt action).
    pub reload_empty_s: f32,
    pub max_range: f32,
    /// Cone half-angle while standing still, hip fire.
    pub spread_hip: f32,
    /// Cone half-angle aiming down sights.
    pub spread_ads: f32,
    /// Extra spread at full run speed.
    pub spread_move: f32,
    /// Added per shot, decays at `spread_recover` rad/s.
    pub spread_per_shot: f32,
    pub spread_recover: f32,
    /// Vertical / horizontal camera kick per shot.
    pub kick_pitch: f32,
    pub kick_yaw: f32,
    /// FOV multiplier while aiming (0.9 = slight zoom, 0.35 = scope).
    pub ads_zoom: f32,
    pub ads_time_s: f32,
    /// Move speed multiplier carrying this gun.
    pub move_mult: f32,
    /// Bullets go through thin cover (low walls, partitions, crates) keeping up to this fraction of
    /// their damage (less through thicker cover; see the game's `weapon::fire`).
    /// 0 = no penetration.
    pub penetration: f32,
    /// How far an unsuppressed shot is heard, metres.
    pub loudness_m: f32,
}

impl WeaponSpec {
    /// Damage of one bullet/pellet at distance `d`, before the headshot multiplier.
    pub fn damage_at(&self, d: f32) -> f32 {
        if d <= self.falloff_start {
            return self.damage;
        }
        if d >= self.falloff_end {
            return self.damage * self.falloff_min;
        }
        let t = (d - self.falloff_start) / (self.falloff_end - self.falloff_start);
        self.damage * (1.0 + (self.falloff_min - 1.0) * t)
    }

    /// Seconds between trigger pulls at the fastest the gun allows.
    pub fn shot_interval(&self) -> f32 {
        match self.mode {
            FireMode::Single => self.cycle_s,
            _ => 1.0 / self.fire_rate,
        }
    }

    /// Expected damage per second at range `d` against a body, assuming `hit_rate` of
    /// bullets land. Ignores reloads. AI uses it to pick fights at good ranges.
    pub fn dps_at(&self, d: f32, hit_rate: f32) -> f32 {
        if d > self.max_range {
            return 0.0;
        }
        self.damage_at(d) * self.pellets as f32 * hit_rate / self.shot_interval()
    }

    /// The range band where this gun is at its best (start, end). AI prefers to fight inside it.
    pub fn sweet_spot(&self) -> (f32, f32) {
        match self.kind {
            WeaponKind::Shotgun => (0.0, 8.0),
            WeaponKind::Smg => (0.0, 15.0),
            WeaponKind::Pistol => (0.0, 18.0),
            WeaponKind::Rifle => (8.0, 35.0),
            WeaponKind::Marksman => (20.0, 80.0),
        }
    }
}

pub static PISTOL: WeaponSpec = WeaponSpec {
    kind: WeaponKind::Pistol,
    name: "Vector-9",
    slot: Slot::Secondary,
    mode: FireMode::Semi,
    damage: 22.0,
    falloff_min: 0.6,
    falloff_start: 12.0,
    falloff_end: 35.0,
    headshot_mult: 2.0,
    pellets: 1,
    fire_rate: 6.5,
    cycle_s: 0.0,
    mag: 15,
    reserve: 60,
    reload_s: 1.4,
    reload_empty_s: 1.7,
    max_range: 120.0,
    spread_hip: 0.012,
    spread_ads: 0.003,
    spread_move: 0.02,
    spread_per_shot: 0.008,
    spread_recover: 0.12,
    kick_pitch: 0.022,
    kick_yaw: 0.006,
    ads_zoom: 0.88,
    ads_time_s: 0.15,
    move_mult: 1.05,
    penetration: 0.0,
    loudness_m: 45.0,
};

pub static SMG: WeaponSpec = WeaponSpec {
    kind: WeaponKind::Smg,
    name: "Hornet",
    slot: Slot::Primary,
    mode: FireMode::Auto,
    damage: 18.0,
    falloff_min: 0.5,
    falloff_start: 10.0,
    falloff_end: 30.0,
    headshot_mult: 1.8,
    pellets: 1,
    fire_rate: 14.0,
    cycle_s: 0.0,
    mag: 32,
    reserve: 128,
    reload_s: 1.8,
    reload_empty_s: 2.2,
    max_range: 150.0,
    spread_hip: 0.02,
    spread_ads: 0.008,
    spread_move: 0.012,
    spread_per_shot: 0.004,
    spread_recover: 0.2,
    kick_pitch: 0.011,
    kick_yaw: 0.008,
    ads_zoom: 0.85,
    ads_time_s: 0.17,
    move_mult: 1.0,
    penetration: 0.0,
    loudness_m: 50.0,
};

pub static RIFLE: WeaponSpec = WeaponSpec {
    kind: WeaponKind::Rifle,
    name: "Kestrel",
    slot: Slot::Primary,
    mode: FireMode::Auto,
    damage: 25.0,
    falloff_min: 0.75,
    falloff_start: 25.0,
    falloff_end: 70.0,
    headshot_mult: 2.0,
    pellets: 1,
    fire_rate: 10.0,
    cycle_s: 0.0,
    mag: 30,
    reserve: 90,
    reload_s: 2.0,
    reload_empty_s: 2.5,
    max_range: 200.0,
    spread_hip: 0.025,
    spread_ads: 0.002,
    spread_move: 0.035,
    spread_per_shot: 0.006,
    spread_recover: 0.15,
    kick_pitch: 0.018,
    kick_yaw: 0.006,
    ads_zoom: 0.78,
    ads_time_s: 0.22,
    move_mult: 0.95,
    penetration: 0.35,
    loudness_m: 70.0,
};

pub static SHOTGUN: WeaponSpec = WeaponSpec {
    kind: WeaponKind::Shotgun,
    name: "Breacher",
    slot: Slot::Primary,
    mode: FireMode::Single,
    damage: 13.0,
    falloff_min: 0.15,
    falloff_start: 6.0,
    falloff_end: 22.0,
    headshot_mult: 1.5,
    pellets: 9,
    fire_rate: 1.2,
    cycle_s: 0.85,
    mag: 6,
    reserve: 30,
    // Shells load one at a time: this is per shell; the game may interrupt between shells.
    reload_s: 0.5,
    reload_empty_s: 0.8,
    max_range: 60.0,
    spread_hip: 0.07,
    spread_ads: 0.055,
    spread_move: 0.01,
    spread_per_shot: 0.0,
    spread_recover: 1.0,
    kick_pitch: 0.06,
    kick_yaw: 0.012,
    ads_zoom: 0.9,
    ads_time_s: 0.2,
    move_mult: 0.97,
    penetration: 0.0,
    loudness_m: 75.0,
};

pub static MARKSMAN: WeaponSpec = WeaponSpec {
    kind: WeaponKind::Marksman,
    name: "Longbow",
    slot: Slot::Primary,
    mode: FireMode::Single,
    damage: 70.0,
    falloff_min: 0.85,
    falloff_start: 60.0,
    falloff_end: 150.0,
    headshot_mult: 2.5,
    pellets: 1,
    fire_rate: 1.0,
    cycle_s: 1.1,
    mag: 5,
    reserve: 25,
    reload_s: 2.6,
    reload_empty_s: 3.1,
    max_range: 300.0,
    spread_hip: 0.06,
    spread_ads: 0.0,
    spread_move: 0.08,
    spread_per_shot: 0.0,
    spread_recover: 1.0,
    kick_pitch: 0.05,
    kick_yaw: 0.008,
    ads_zoom: 0.3,
    ads_time_s: 0.32,
    move_mult: 0.88,
    penetration: 0.6,
    loudness_m: 110.0,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_match_kinds() {
        for k in WeaponKind::ALL {
            assert_eq!(k.spec().kind, k);
            assert!(k.spec().damage_at(0.0) >= k.spec().damage_at(1000.0));
        }
    }

    #[test]
    fn falloff_is_linear_between_bounds() {
        let r = WeaponKind::Rifle.spec();
        let mid = (r.falloff_start + r.falloff_end) / 2.0;
        let want = r.damage * (1.0 + r.falloff_min) / 2.0;
        assert!((r.damage_at(mid) - want).abs() < 1e-4);
    }

    #[test]
    fn guns_win_in_their_own_band() {
        let close = 4.0;
        let far = 50.0;
        let sg = WeaponKind::Shotgun.spec();
        let mk = WeaponKind::Marksman.spec();
        assert!(sg.dps_at(close, 0.8) > mk.dps_at(close, 0.8));
        assert!(mk.dps_at(far, 0.8) > sg.dps_at(far, 0.8));
    }
}
