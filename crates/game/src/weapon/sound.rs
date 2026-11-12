//! Per-gun sound from [`GunEvent`]s: shots, magazine out / in, chambering, pump, shells, bolt,
//! dry fire, draw, ADS rustle, pickups and casings tinkling a moment after each shot. Under a
//! gunshot, the space's tail: a room's reflections indoors (under a roof), the open-air rumble
//! and slap-back outdoors. Files
//! are `assets/audio/<name>.ogg`; any that are missing are silent (older fallbacks are tried).
//! Muted runs (`--mute`, headless, tests) load and play nothing.

use bevy::prelude::*;
use engine::audio::{AudioCategory, Volumes};

use super::{GunEvent, WeaponKind, WeaponSet};
use crate::assets::AssetsDir;
use crate::audio::Sfx;
use crate::state::GameState;

const SOUNDS: &[(&str, usize)] = &[
    ("pistol_shot", 1),
    ("smg_shot", 1),
    ("shotgun_shot", 1),
    ("marksman_shot", 1),
    ("heavy_pistol_shot", 1),
    ("battle_rifle_shot", 1),
    ("sniper_shot", 1),
    ("mag_out", 1),
    ("mag_in", 1),
    ("chamber", 1),
    ("shotgun_pump", 1),
    ("shell_insert", 1),
    ("bolt_cycle", 1),
    ("dry_fire", 1),
    ("weapon_draw", 1),
    ("ads_in", 1),
    ("pickup", 1),
    ("casing", 3),
    // The space round the shooter (`scripts/make-combat-sounds.py`): a room's early reflections
    // and short tail under a roof, the open-air rumble and slap-back outside.
    ("shot_tail_indoor", 1),
    ("shot_tail_outdoor", 1),
];

/// Whether `p` (world) is under one of the map's roofs: shots there get the room tail.
pub fn indoors(arena: &brain::arena::Arena, p: Vec3) -> bool {
    arena.decor.roofs.iter().any(|r| p.x >= r.min.x && p.x <= r.max.x && p.z >= r.min.y && p.z <= r.max.y && p.y < r.bottom + 0.5)
}

pub(super) fn build(app: &mut App) {
    if app.world().get_resource::<Volumes>().is_some_and(|v| v.muted) {
        return;
    }
    app.add_systems(Startup, load).add_systems(Update, gun_sounds.after(WeaponSet).run_if(in_state(GameState::Playing)));
}

fn load(sfx: Option<ResMut<Sfx>>, server: Option<Res<AssetServer>>, dir: Res<AssetsDir>) {
    let (Some(mut sfx), Some(server)) = (sfx, server) else { return };
    for &(name, n) in SOUNDS {
        sfx.load_variants(&server, &dir, name, &format!("audio/{name}"), n);
    }
}

fn shot_name(k: WeaponKind) -> &'static str {
    match k {
        WeaponKind::Pistol => "pistol_shot",
        WeaponKind::Smg => "smg_shot",
        WeaponKind::Rifle => "rifle_shot",
        WeaponKind::Shotgun => "shotgun_shot",
        WeaponKind::Marksman => "marksman_shot",
        WeaponKind::HeavyPistol => "heavy_pistol_shot",
        WeaponKind::BattleRifle => "battle_rifle_shot",
        WeaponKind::Sniper => "sniper_shot",
    }
}

fn gun_sounds(
    mut commands: Commands,
    time: Res<Time>,
    sfx: Res<Sfx>,
    master: Res<Volumes>,
    mut events: MessageReader<GunEvent>,
    arena: Option<Res<crate::map::ArenaRes>>,
    player: Query<&GlobalTransform, With<crate::player::Player>>,
    // Casings waiting to land (seconds left).
    mut casings: Local<Vec<f32>>,
) {
    let inside = match (&arena, player.single()) {
        (Some(a), Ok(p)) => indoors(&a.0, p.translation()),
        _ => false,
    };
    let mut play = |name: &str, fallback: Option<&str>, v: f32| {
        let n = if sfx.has(name) { name } else { fallback.unwrap_or(name) };
        engine::audio::play(&mut commands, &sfx, &master, n, AudioCategory::Sfx, v, None);
    };
    let mut tail_played = false;
    for ev in events.read() {
        match *ev {
            GunEvent::Shot { kind } => {
                let v = match kind {
                    WeaponKind::Smg => 0.7,
                    WeaponKind::Pistol => 0.8,
                    _ => 0.9,
                };
                let fallback = match kind {
                    WeaponKind::HeavyPistol => "pistol_shot",
                    WeaponKind::Sniper => "marksman_shot",
                    _ => "rifle_shot",
                };
                play(shot_name(kind), Some(if sfx.has(fallback) { fallback } else { "rifle_shot" }), v);
                // Quieter tails for the small guns; one tail per frame is plenty at full auto.
                let tail = if inside { "shot_tail_indoor" } else { "shot_tail_outdoor" };
                if !tail_played && sfx.has(tail) {
                    tail_played = true;
                    play(tail, None, v * if inside { 0.55 } else { 0.45 });
                }
                if casings.len() < 4 && !matches!(kind, WeaponKind::Shotgun | WeaponKind::Marksman | WeaponKind::Sniper) {
                    casings.push(rand::random_range(0.35..0.55));
                }
            }
            GunEvent::DryFire => play("dry_fire", Some("empty_click"), 0.7),
            GunEvent::MagOut { .. } => play("mag_out", Some("reload_start"), 0.75),
            GunEvent::MagIn { .. } => play("mag_in", Some("reload_end"), 0.85),
            GunEvent::Chamber { .. } => play("chamber", None, 0.75),
            GunEvent::ShellIn => play("shell_insert", None, 0.7),
            GunEvent::Pump => {
                play("shotgun_pump", None, 0.8);
                casings.push(0.45);
            }
            GunEvent::Bolt => {
                play("bolt_cycle", None, 0.8);
                casings.push(0.45);
            }
            GunEvent::Draw { .. } => play("weapon_draw", None, 0.55),
            GunEvent::AdsIn => play("ads_in", None, 0.35),
            GunEvent::Pickup { .. } => play("pickup", None, 0.7),
            GunEvent::Inspect => play("weapon_draw", None, 0.3),
        }
    }
    let dt = time.delta_secs();
    let mut i = 0;
    while i < casings.len() {
        casings[i] -= dt;
        if casings[i] <= 0.0 {
            casings.swap_remove(i);
            play("casing", None, 0.25);
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::sound_files;

    #[test]
    fn shots_under_a_roof_are_indoors() {
        use brain::arena::{Arena, MapId};
        let a = Arena::build(MapId::Foundry);
        let r = a.decor.roofs.first().expect("Foundry has roofed buildings");
        let mid = Vec3::new((r.min.x + r.max.x) * 0.5, 1.6, (r.min.y + r.max.y) * 0.5);
        assert!(indoors(&a, mid));
        // Standing on that roof, or well outside it, is outdoors.
        assert!(!indoors(&a, mid + Vec3::Y * (r.bottom + 1.0)));
        assert!(!indoors(&a, Vec3::new(r.max.x + 50.0, 1.6, r.max.y + 50.0)));
    }

    #[test]
    fn every_gun_sound_has_its_files() {
        for &(name, n) in SOUNDS {
            for f in sound_files(name, n) {
                assert!(f.is_file(), "{name}: missing {}", f.display());
            }
        }
        // Each gun's shot resolves to a file (the Kestrel's lives with the shared sounds).
        for k in WeaponKind::ALL {
            let name = shot_name(k);
            assert!(sound_files(name, 1)[0].is_file(), "{k:?}: no {name}.ogg");
        }
    }
}
