//! Enemy loadouts: which gun each enemy carries, how it handles in an AI's hands, and the gun's
//! runtime state (magazine, reload, weapon switch). The game and the trainer sim share all of it,
//! so a squad trains with the guns it fights with.
//!
//! - Each trait genome carries one preference gene per primary (`WeaponKind::PRIMARIES`, six:
//!   SMG, Kestrel, shotgun, Longbow, Shrike, Osprey). Old saves with four load padded and the
//!   two new genes get seeded (`genome::TraitGenome::fill_loadout`).
//!   Evolution moves them like any other gene: if the player fights at 40 m, squads whose
//!   members prefer the Longbow win more sim rounds against his ghost and breed.
//! - A squad gets its guns from those preferences under per-difficulty caps ([`caps`],
//!   [`assign`]): the strongest preference claims a slot first; overflow carries a rifle.
//! - Everyone also carries the Vector-9 sidearm ([`GunState`] switches to it when the primary
//!   runs dry mid-fight).
//! - Fairness: enemy damage is the gun's ballistic table scaled by `Limits::damage_per_hit`
//!   (the rifle stays where it was before guns existed), aim error stays the evolved trait times
//!   a per-gun handling factor, and slow guns pay with long settle / cycle times.

use crate::difficulty::Difficulty;
use crate::limits::Limits;
use crate::weapons::{RIFLE, WeaponKind, WeaponSpec};
use serde::{Deserialize, Serialize};

/// Preference genes per genome (one per `WeaponKind::PRIMARIES`).
pub const PREF_COUNT: usize = WeaponKind::PRIMARIES.len();
/// Seconds to swap between primary and sidearm.
pub const SWITCH_S: f32 = 0.45;
/// The trainer's squads are equipped as on Veteran (the full kit, moderate caps).
pub const TRAINER_DIFFICULTY: Difficulty = Difficulty::Veteran;

/// What an enemy does in the squad, by the gun it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Role {
    /// Rifle: anchors and holds angles at mid range.
    #[default]
    Rifleman,
    /// SMG: flanks and rushes.
    Rusher,
    /// Shotgun: closes through cover, breaches rooms.
    Breacher,
    /// Marksman rifle: long sightlines, relocates after shots.
    Marksman,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Rifleman, Role::Rusher, Role::Breacher, Role::Marksman];

    pub fn of(k: WeaponKind) -> Role {
        match k {
            WeaponKind::Smg => Role::Rusher,
            WeaponKind::Shotgun => Role::Breacher,
            WeaponKind::Marksman | WeaponKind::Sniper => Role::Marksman,
            WeaponKind::Rifle | WeaponKind::BattleRifle | WeaponKind::Pistol | WeaponKind::HeavyPistol => Role::Rifleman,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Rifleman => "Rifleman",
            Role::Rusher => "Rusher",
            Role::Breacher => "Breacher",
            Role::Marksman => "Marksman",
        }
    }
}

/// Index of a primary in `WeaponKind::PRIMARIES` (pistols map to the rifle's slot).
pub fn primary_index(k: WeaponKind) -> usize {
    WeaponKind::PRIMARIES.iter().position(|&x| x == k).unwrap_or(1)
}

/// Index of a gun in `WeaponKind::ALL`.
pub fn kind_index(k: WeaponKind) -> usize {
    WeaponKind::ALL.iter().position(|&x| x == k).unwrap_or(2)
}

/// The preferred primary: the highest gene (ties go to the rifle, then table order).
pub fn preferred(prefs: &[f32; PREF_COUNT]) -> WeaponKind {
    let mut best = (prefs[1], WeaponKind::Rifle);
    for (i, &k) in WeaponKind::PRIMARIES.iter().enumerate() {
        if prefs[i] > best.0 {
            best = (prefs[i], k);
        }
    }
    best.1
}

/// Most of each primary (in `PRIMARIES` order) a squad of `n` may field on a difficulty. Rifles
/// are unlimited, so every member always gets a gun.
/// Order: SMG, Kestrel, shotgun, Longbow, Shrike, Osprey. The Osprey (one-shot body kills)
/// only shows up from Veteran, one per squad. Long guns (Longbow + Osprey) together are capped by
/// the Longbow's number, so a squad never parks more shooters on sightlines than before.
pub fn caps(d: Difficulty, n: usize) -> [usize; PREF_COUNT] {
    match d {
        Difficulty::Recruit => [1, n, 0, 0, 0, 0],
        Difficulty::Regular => [1, n, 1, 1, 1, 0],
        Difficulty::Veteran => [2, n, 1, 1, 2, 1],
        Difficulty::Elite => [2, n, 2, 2, 2, 1],
    }
}

/// Hand out primaries: strongest preference first, within `caps`; the rest carry rifles.
pub fn assign(prefs: &[[f32; PREF_COUNT]], caps: [usize; PREF_COUNT]) -> Vec<WeaponKind> {
    let n = prefs.len();
    let mut want: Vec<(f32, usize, usize)> = Vec::with_capacity(n * PREF_COUNT);
    for (m, p) in prefs.iter().enumerate() {
        // Only a member's top two choices compete for capped slots (a weak third preference
        // shouldn't take the last shotgun from someone who loves it).
        let mut order: [usize; PREF_COUNT] = std::array::from_fn(|i| i);
        order.sort_by(|&a, &b| p[b].total_cmp(&p[a]).then(a.cmp(&b)));
        for &k in &order[..2] {
            want.push((p[k], m, k));
        }
    }
    want.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut left = caps;
    let mut long_left = caps[3];
    let mut out: Vec<Option<WeaponKind>> = vec![None; n];
    for (_, m, k) in want {
        let long = WeaponKind::PRIMARIES[k].long_gun();
        if out[m].is_none() && left[k] > 0 && (!long || long_left > 0) {
            left[k] -= 1;
            long_left -= usize::from(long);
            out[m] = Some(WeaponKind::PRIMARIES[k]);
        }
    }
    out.into_iter().map(|k| k.unwrap_or(WeaponKind::Rifle)).collect()
}

/// Give a squad its guns: each mind's weapon genes compete under the difficulty's caps.
pub fn equip(minds: &mut [crate::mind::EnemyMind], d: Difficulty) {
    let prefs: Vec<[f32; PREF_COUNT]> = minds.iter().map(|m| m.traits.loadout).collect();
    for (m, k) in minds.iter_mut().zip(assign(&prefs, caps(d, prefs.len()))) {
        m.set_weapon(k);
    }
}

/// Primaries of a squad by kind, in `PRIMARIES` order.
pub fn counts(kinds: &[WeaponKind]) -> [u32; PREF_COUNT] {
    let mut c = [0; PREF_COUNT];
    for &k in kinds {
        c[primary_index(k)] += 1;
    }
    c
}

/// How a gun handles in an enemy's hands (on top of the evolved aim trait).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handling {
    /// Multiplies the trait's aim error.
    pub aim_mult: f32,
    /// Extra spread per unit of movement (1 = full walk).
    pub move_mult: f32,
    /// First shots are wider until the aim has sat on the target this long (s).
    pub settle_s: f32,
    /// Must be on target this long before the first shot (scoping in).
    pub ads_gate_s: f32,
    /// Multiplies the damage scaling (keeps slow guns worth carrying).
    pub damage_mult: f32,
}

pub fn handling(k: WeaponKind) -> Handling {
    match k {
        WeaponKind::Pistol => Handling { aim_mult: 1.1, move_mult: 0.3, settle_s: 0.5, ads_gate_s: 0.0, damage_mult: 1.0 },
        WeaponKind::Smg => Handling { aim_mult: 1.25, move_mult: 0.12, settle_s: 0.35, ads_gate_s: 0.0, damage_mult: 1.0 },
        WeaponKind::Rifle => Handling { aim_mult: 1.0, move_mult: 0.35, settle_s: 0.6, ads_gate_s: 0.0, damage_mult: 1.0 },
        WeaponKind::Shotgun => Handling { aim_mult: 0.6, move_mult: 0.1, settle_s: 0.3, ads_gate_s: 0.0, damage_mult: 1.4 },
        WeaponKind::Marksman => Handling { aim_mult: 0.3, move_mult: 1.5, settle_s: 0.9, ads_gate_s: 0.45, damage_mult: 1.5 },
        WeaponKind::HeavyPistol => Handling { aim_mult: 1.2, move_mult: 0.4, settle_s: 0.55, ads_gate_s: 0.0, damage_mult: 1.0 },
        WeaponKind::BattleRifle => Handling { aim_mult: 1.15, move_mult: 0.45, settle_s: 0.65, ads_gate_s: 0.0, damage_mult: 1.0 },
        // Slow cycle and a long scope-in; damage scaled down so an enemy Osprey hurts (~46) but
        // never one-shots the player.
        WeaponKind::Sniper => Handling { aim_mult: 0.25, move_mult: 2.0, settle_s: 1.0, ads_gate_s: 0.6, damage_mult: 1.25 },
    }
}

/// Aim error multiplier with gun `k` at `d` metres: the handling factor, and a scoped rifle is
/// clumsy up close (tunnel vision inside 12 m).
pub fn aim_mult_at(k: WeaponKind, d: f32) -> f32 {
    let h = handling(k).aim_mult;
    if k.long_gun() && d < 12.0 { h * (1.0 + (12.0 - d) / 4.0) } else { h }
}

/// Damage one enemy bullet/pellet of gun `k` does at `d` metres under `limits`. The rifle does
/// exactly `limits.damage_per_hit` inside its falloff (the pre-loadout balance).
pub fn enemy_damage(k: WeaponKind, d: f32, limits: &Limits) -> f32 {
    k.spec().damage_at(d) * limits.damage_per_hit / RIFLE.damage * handling(k).damage_mult
}

/// Per-pellet cone (1 sigma, radians) of an enemy shot with gun `k`: shotguns spray, the rest
/// add their aimed spread to the aim error.
pub fn pellet_spread(k: WeaponKind) -> f32 {
    let s = k.spec();
    if s.pellets > 1 { s.spread_hip * 0.5 } else { s.spread_ads }
}

/// Preferred fighting range (m) for gun `k`, given the genome's own preference `r` (6..30 m).
/// The genome still shifts it within the gun's band.
pub fn fit_range(k: WeaponKind, r: f32) -> f32 {
    let t = ((r - 6.0) / 24.0).clamp(0.0, 1.0);
    let (lo, hi) = match k {
        WeaponKind::Shotgun => (3.5, 7.0),
        WeaponKind::Smg => (6.0, 13.0),
        WeaponKind::Pistol => (6.0, 15.0),
        WeaponKind::Rifle => (6.0, 30.0),
        WeaponKind::Marksman => (24.0, 42.0),
        WeaponKind::HeavyPistol => (6.0, 15.0),
        WeaponKind::BattleRifle => (7.0, 30.0),
        WeaponKind::Sniper => (28.0, 45.0),
    };
    lo + (hi - lo) * t
}

/// How good gun `k` is at `d` metres relative to its own best (0..1), for tactics.
pub fn range_quality(k: WeaponKind, d: f32) -> f32 {
    let s = k.spec();
    let (lo, hi) = s.sweet_spot();
    if (lo..=hi).contains(&d) {
        return 1.0;
    }
    let mid = (lo + hi) * 0.5;
    let best = s.dps_at(mid, 0.5).max(1e-3);
    // A rough hit-rate model: further is harder to hit for everyone.
    let hit = |x: f32| (0.9 - 0.012 * x).clamp(0.15, 0.9);
    (s.dps_at(d, hit(d)) / (best * hit(mid) / 0.5)).clamp(0.0, 1.0)
}

/// One gun pair (primary + Vector-9 sidearm): magazines, reload, cycle and switch timers.
/// Enemy reserves are unlimited (they never run out of spare magazines).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunState {
    pub primary: WeaponKind,
    pub held: WeaponKind,
    /// Rounds in the primary's and the sidearm's magazines.
    pub mags: [u32; 2],
    /// Seconds left on the current reload (per shell for shotguns).
    pub reload_t: f32,
    /// Shell-by-shell reload in progress (firing interrupts it).
    pub shells: bool,
    pub cooldown: f32,
    /// Seconds left on a weapon switch (no firing meanwhile).
    pub switch_t: f32,
}

impl GunState {
    pub fn new(primary: WeaponKind) -> Self {
        GunState {
            primary,
            held: primary,
            mags: [primary.spec().mag, WeaponKind::Pistol.spec().mag],
            reload_t: 0.0,
            shells: false,
            cooldown: 0.0,
            switch_t: 0.0,
        }
    }

    fn slot(&self) -> usize {
        usize::from(self.held != self.primary)
    }

    pub fn spec(&self) -> &'static WeaponSpec {
        self.held.spec()
    }

    pub fn ammo(&self) -> u32 {
        self.mags[self.slot()]
    }

    pub fn reloading(&self) -> bool {
        self.reload_t > 0.0
    }

    pub fn switching(&self) -> bool {
        self.switch_t > 0.0
    }

    /// Seconds until the held gun can fire again after a reload/switch.
    pub fn busy_s(&self) -> f32 {
        let reload = if self.reload_t > 0.0 && !self.shells { self.reload_t } else { 0.0 };
        reload.max(self.switch_t)
    }

    /// Held magazine 0..1; 0 while a full reload is running (shells still count while loading).
    pub fn ammo_frac(&self) -> f32 {
        if self.reload_t > 0.0 && !self.shells {
            0.0
        } else {
            self.ammo() as f32 / self.spec().mag.max(1) as f32
        }
    }

    /// The primary's magazine 0..1 (0 while it reloads).
    pub fn primary_frac(&self) -> f32 {
        if self.held == self.primary {
            self.ammo_frac()
        } else {
            self.mags[0] as f32 / self.primary.spec().mag.max(1) as f32
        }
    }

    /// Start reloading the held gun. False if full or busy.
    pub fn start_reload(&mut self) -> bool {
        let s = self.spec();
        if self.reload_t > 0.0 || self.switch_t > 0.0 || self.ammo() >= s.mag {
            return false;
        }
        self.shells = s.pellets > 1;
        self.reload_t = if self.ammo() == 0 { s.reload_empty_s } else { s.reload_s };
        true
    }

    /// Switch to `kind` (the primary or the sidearm). Cancels a reload. False if already held.
    pub fn switch_to(&mut self, kind: WeaponKind) -> bool {
        if kind == self.held || (kind != self.primary && kind != WeaponKind::Pistol) {
            return false;
        }
        self.held = kind;
        self.switch_t = SWITCH_S;
        self.reload_t = 0.0;
        self.shells = false;
        true
    }

    /// Advance `dt`; pull the trigger if `fire` and the gun can. True if a shot left.
    /// An empty magazine starts a reload on its own.
    pub fn tick(&mut self, dt: f32, fire: bool) -> bool {
        self.cooldown -= dt;
        if self.switch_t > 0.0 {
            self.switch_t -= dt;
            if self.switch_t > 0.0 {
                return false;
            }
        }
        let slot = self.slot();
        let s = self.spec();
        if self.reload_t > 0.0 {
            if self.shells && fire && self.mags[slot] > 0 {
                // Pump guns fire out of a partial reload.
                self.reload_t = 0.0;
                self.shells = false;
            } else {
                self.reload_t -= dt;
                if self.reload_t <= 0.0 {
                    if self.shells {
                        self.mags[slot] += 1;
                        if self.mags[slot] < s.mag {
                            self.reload_t += s.reload_s;
                        } else {
                            self.shells = false;
                        }
                    } else {
                        self.mags[slot] = s.mag;
                    }
                }
                if self.reload_t > 0.0 {
                    return false;
                }
                self.reload_t = 0.0;
            }
        }
        if fire && self.cooldown <= 0.0 && self.mags[slot] > 0 {
            self.mags[slot] -= 1;
            self.cooldown = self.cooldown.max(-dt) + s.shot_interval();
            if self.mags[slot] == 0 {
                self.start_reload();
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_respects_caps_and_preferences() {
        // Everyone loves the marksman rifle; Regular allows one.
        let prefs = vec![[0.1, 0.2, 0.1, 0.9, 0.0, 0.0], [0.1, 0.2, 0.1, 0.95, 0.0, 0.0], [0.8, 0.1, 0.1, 0.7, 0.0, 0.0], [0.1, 0.1, 0.9, 0.2, 0.0, 0.0]];
        let k = assign(&prefs, caps(Difficulty::Regular, 4));
        assert_eq!(k[1], WeaponKind::Marksman, "strongest preference wins the slot");
        assert_eq!(k.iter().filter(|&&x| x == WeaponKind::Marksman).count(), 1);
        assert_eq!(k[2], WeaponKind::Smg);
        assert_eq!(k[3], WeaponKind::Shotgun);
        assert_eq!(k[0], WeaponKind::Rifle, "overflow carries a rifle");
        // Recruit: no shotguns or marksmen at all.
        let r = assign(&prefs, caps(Difficulty::Recruit, 4));
        assert!(r.iter().all(|&x| matches!(x, WeaponKind::Rifle | WeaponKind::Smg)));
        // One Osprey per squad from Veteran; never on Regular.
        let snipers = vec![[0.0, 0.1, 0.0, 0.2, 0.3, 0.9]; 4];
        let v = assign(&snipers, caps(Difficulty::Veteran, 4));
        assert_eq!(v.iter().filter(|&&x| x == WeaponKind::Sniper).count(), 1);
        assert_eq!(v.iter().filter(|&&x| x == WeaponKind::BattleRifle).count(), 2);
        let g = assign(&snipers, caps(Difficulty::Regular, 4));
        assert!(!g.contains(&WeaponKind::Sniper));
        // Long guns share the Longbow's cap: on Veteran the Osprey takes the only long slot.
        assert!(!v.contains(&WeaponKind::Marksman));
        let e = assign(&vec![[0.0, 0.1, 0.0, 0.8, 0.0, 0.9]; 6], caps(Difficulty::Elite, 6));
        assert_eq!(e.iter().filter(|k| k.long_gun()).count(), 2);
    }

    #[test]
    fn rifle_damage_is_unchanged() {
        let l = Limits::for_generation(0);
        assert!((enemy_damage(WeaponKind::Rifle, 10.0, &l) - l.damage_per_hit).abs() < 1e-4);
        assert!(enemy_damage(WeaponKind::Marksman, 40.0, &l) > enemy_damage(WeaponKind::Rifle, 40.0, &l) * 2.0);
    }

    #[test]
    fn gun_cycles_reloads_and_switches() {
        let mut g = GunState::new(WeaponKind::Marksman);
        let dt = 0.05;
        let mut shots = 0;
        for _ in 0..40 {
            shots += g.tick(dt, true) as u32;
        }
        // 2 s of trigger at a 1.1 s cycle: two shots.
        assert_eq!(shots, 2);
        // Empty it: it reloads on its own.
        g.mags[0] = 1;
        g.cooldown = 0.0;
        assert!(g.tick(dt, true));
        assert!(g.reloading());
        assert_eq!(g.ammo_frac(), 0.0);
        // Switch to the pistol mid-reload: the reload is dropped, the pistol fires after the swap.
        assert!(g.switch_to(WeaponKind::Pistol));
        assert!(!g.tick(dt, true));
        let mut fired = false;
        for _ in 0..20 {
            fired |= g.tick(dt, true);
        }
        assert!(fired);
        assert_eq!(g.held, WeaponKind::Pistol);
        assert_eq!(g.mags[0], 0, "the primary is still empty");
    }

    #[test]
    fn shotgun_loads_shells_and_fires_out_of_a_reload() {
        let mut g = GunState::new(WeaponKind::Shotgun);
        g.mags[0] = 2;
        assert!(g.start_reload());
        for _ in 0..30 {
            g.tick(0.05, false);
        }
        assert!(g.mags[0] > 2, "shells went in one at a time: {}", g.mags[0]);
        g.mags[0] = 3;
        g.reload_t = 0.3;
        g.shells = true;
        assert!(g.tick(0.05, true), "firing interrupts a shell reload");
    }

    #[test]
    fn ranges_fit_the_gun() {
        assert!(fit_range(WeaponKind::Shotgun, 30.0) <= 8.0);
        assert!(fit_range(WeaponKind::Marksman, 6.0) >= 20.0);
        assert!(range_quality(WeaponKind::Shotgun, 4.0) > range_quality(WeaponKind::Shotgun, 30.0));
        assert!(range_quality(WeaponKind::Marksman, 40.0) > 0.9);
    }
}
