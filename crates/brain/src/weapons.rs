//! Shared weapon contract: the guns in Shroud and their ballistic stats.
//!
//! One table drives the player's arsenal (game `weapon/`), enemy loadouts (`mind`, `squad`) and the
//! offline trainer (`sim`, `ghost`), so a gun hits the same in a real match and in training.
//! Pure data; no engine types.

use serde::{Deserialize, Serialize};

use crate::arena::{Block, BlockKind, Skin};

/// Bullets go through this block (if the gun can penetrate and it is thin enough there). Shared
/// by the player's bullets (game `weapon/fire.rs`), enemy bullets and the trainer.
pub fn penetrable(b: &Block) -> bool {
    b.kind != BlockKind::Wall && (b.kind == BlockKind::Low || matches!(b.skin, Skin::Crate | Skin::Desk | Skin::Drywall | Skin::Cabinet | Skin::Shelf | Skin::Car))
}

/// Thickest cover (metres along the bullet) a gun with this `penetration` gets through.
pub fn max_thickness(penetration: f32) -> f32 {
    if penetration <= 0.0 { 0.0 } else { 0.6 + 3.0 * penetration }
}

/// Fraction of damage left after passing `thickness` metres of cover.
pub fn penetration_mult(penetration: f32, thickness: f32) -> f32 {
    let max = max_thickness(penetration);
    if max <= 0.0 || thickness > max {
        return 0.0;
    }
    penetration * (1.0 - 0.5 * thickness / max)
}

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
    /// "Longbow" scout: light scoped bolt rifle, mobile, one-shot headshots. The cheap long gun.
    Marksman,
    /// "Warden" heavy pistol: seven big rounds, slow and kicky, one-tap headshots.
    HeavyPistol,
    /// "Shrike" battle rifle: the attacker rifle. Harder hitting (one-tap headshots), slower,
    /// more recoil and a bit cheaper than the Kestrel.
    BattleRifle,
    /// "Osprey" heavy bolt-action sniper: one-shot body kills, the most expensive gun.
    Sniper,
}

impl WeaponKind {
    /// Every gun. New kinds are appended (per-gun arrays in saves are indexed by this order).
    pub const ALL: [WeaponKind; 8] = [
        WeaponKind::Pistol,
        WeaponKind::Smg,
        WeaponKind::Rifle,
        WeaponKind::Shotgun,
        WeaponKind::Marksman,
        WeaponKind::HeavyPistol,
        WeaponKind::BattleRifle,
        WeaponKind::Sniper,
    ];
    /// Primaries, in the order of the enemy weapon-preference genes (`loadout.rs`). Appended only.
    pub const PRIMARIES: [WeaponKind; 6] = [
        WeaponKind::Smg,
        WeaponKind::Rifle,
        WeaponKind::Shotgun,
        WeaponKind::Marksman,
        WeaponKind::BattleRifle,
        WeaponKind::Sniper,
    ];

    pub fn spec(self) -> &'static WeaponSpec {
        match self {
            WeaponKind::Pistol => &PISTOL,
            WeaponKind::Smg => &SMG,
            WeaponKind::Rifle => &RIFLE,
            WeaponKind::Shotgun => &SHOTGUN,
            WeaponKind::Marksman => &MARKSMAN,
            WeaponKind::HeavyPistol => &HEAVY_PISTOL,
            WeaponKind::BattleRifle => &BATTLE_RIFLE,
            WeaponKind::Sniper => &SNIPER,
        }
    }

    /// Scoped long guns (the Longbow and the Osprey): fight from long sightlines, never entry.
    pub fn long_gun(self) -> bool {
        matches!(self, WeaponKind::Marksman | WeaponKind::Sniper)
    }

    pub fn label(self) -> &'static str {
        self.spec().name
    }

    /// Buy-menu price in credits (the economy reads this; enemies buy from the same table).
    pub fn price(self) -> u32 {
        match self {
            WeaponKind::Pistol => 200,
            WeaponKind::Smg => 1250,
            WeaponKind::Rifle => 2900,
            WeaponKind::Shotgun => 1100,
            WeaponKind::Marksman => 1700,
            WeaponKind::HeavyPistol => 700,
            WeaponKind::BattleRifle => 2700,
            WeaponKind::Sniper => 4750,
        }
    }

    /// Credits for a kill with this gun (cheap guns pay more, the sniper pays least).
    pub fn kill_reward(self) -> u32 {
        match self {
            WeaponKind::Pistol | WeaponKind::HeavyPistol | WeaponKind::Rifle | WeaponKind::BattleRifle => 300,
            WeaponKind::Marksman => 300,
            WeaponKind::Smg => 600,
            WeaponKind::Shotgun => 900,
            WeaponKind::Sniper => 100,
        }
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
            WeaponKind::HeavyPistol => (0.0, 20.0),
            WeaponKind::BattleRifle => (8.0, 35.0),
            WeaponKind::Sniper => (25.0, 120.0),
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
    ads_time_s: 0.28,
    move_mult: 0.95,
    penetration: 0.6,
    loudness_m: 110.0,
};

pub static HEAVY_PISTOL: WeaponSpec = WeaponSpec {
    kind: WeaponKind::HeavyPistol,
    name: "Warden",
    slot: Slot::Secondary,
    mode: FireMode::Semi,
    damage: 48.0,
    falloff_min: 0.7,
    falloff_start: 15.0,
    falloff_end: 40.0,
    // 115 to the head inside 15 m: one tap without a helmet.
    headshot_mult: 2.4,
    pellets: 1,
    fire_rate: 2.6,
    cycle_s: 0.0,
    mag: 7,
    reserve: 35,
    reload_s: 2.0,
    reload_empty_s: 2.3,
    max_range: 150.0,
    spread_hip: 0.018,
    spread_ads: 0.004,
    spread_move: 0.04,
    spread_per_shot: 0.03,
    spread_recover: 0.1,
    kick_pitch: 0.06,
    kick_yaw: 0.012,
    ads_zoom: 0.85,
    ads_time_s: 0.18,
    move_mult: 1.0,
    penetration: 0.25,
    loudness_m: 70.0,
};

pub static BATTLE_RIFLE: WeaponSpec = WeaponSpec {
    kind: WeaponKind::BattleRifle,
    name: "Shrike",
    slot: Slot::Primary,
    mode: FireMode::Auto,
    damage: 34.0,
    falloff_min: 0.7,
    falloff_start: 20.0,
    falloff_end: 65.0,
    // 102 to the head: one tap at any range inside falloff.
    headshot_mult: 3.0,
    pellets: 1,
    fire_rate: 8.5,
    cycle_s: 0.0,
    mag: 30,
    reserve: 90,
    reload_s: 2.3,
    reload_empty_s: 2.7,
    max_range: 200.0,
    spread_hip: 0.03,
    spread_ads: 0.003,
    spread_move: 0.045,
    spread_per_shot: 0.009,
    spread_recover: 0.13,
    kick_pitch: 0.026,
    kick_yaw: 0.011,
    ads_zoom: 0.8,
    ads_time_s: 0.24,
    move_mult: 0.93,
    penetration: 0.4,
    loudness_m: 75.0,
};

pub static SNIPER: WeaponSpec = WeaponSpec {
    kind: WeaponKind::Sniper,
    name: "Osprey",
    slot: Slot::Primary,
    mode: FireMode::Single,
    // One-shot body kill (100 HP) out to `falloff_start`, and still 103 at the far end.
    damage: 115.0,
    falloff_min: 0.9,
    falloff_start: 80.0,
    falloff_end: 200.0,
    headshot_mult: 2.0,
    pellets: 1,
    fire_rate: 0.7,
    cycle_s: 1.5,
    mag: 5,
    reserve: 20,
    reload_s: 3.0,
    reload_empty_s: 3.6,
    max_range: 400.0,
    spread_hip: 0.09,
    spread_ads: 0.0,
    spread_move: 0.12,
    spread_per_shot: 0.0,
    spread_recover: 1.0,
    kick_pitch: 0.07,
    kick_yaw: 0.01,
    ads_zoom: 0.22,
    ads_time_s: 0.38,
    move_mult: 0.84,
    penetration: 0.8,
    loudness_m: 130.0,
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

    #[test]
    fn new_guns_fill_their_roles() {
        // The Osprey kills with one body shot; the Longbow doesn't.
        assert!(SNIPER.damage_at(80.0) >= 100.0 && SNIPER.damage_at(1000.0) >= 100.0);
        assert!(MARKSMAN.damage_at(10.0) < 100.0);
        // One-tap headshots: Warden up close, Shrike at range; the Kestrel needs two.
        assert!(HEAVY_PISTOL.damage_at(10.0) * HEAVY_PISTOL.headshot_mult >= 100.0);
        assert!(BATTLE_RIFLE.damage_at(20.0) * BATTLE_RIFLE.headshot_mult >= 100.0);
        assert!(RIFLE.damage_at(10.0) * RIFLE.headshot_mult < 100.0);
        // The attacker rifle: cheaper, harder kick, slower than the Kestrel.
        assert!(WeaponKind::BattleRifle.price() < WeaponKind::Rifle.price());
        assert!(BATTLE_RIFLE.kick_pitch > RIFLE.kick_pitch && BATTLE_RIFLE.fire_rate < RIFLE.fire_rate);
        // The sniper is the top price; the scout is a cheap long gun.
        let top = WeaponKind::ALL.iter().map(|k| k.price()).max().unwrap();
        assert_eq!(WeaponKind::Sniper.price(), top);
        assert!(WeaponKind::Marksman.price() < WeaponKind::Rifle.price());
        assert_eq!(WeaponKind::HeavyPistol.spec().slot, Slot::Secondary);
        for k in WeaponKind::PRIMARIES {
            assert_eq!(k.spec().slot, Slot::Primary);
        }
    }
}
