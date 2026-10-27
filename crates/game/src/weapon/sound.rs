//! Per-gun sound from [`GunEvent`]s: shots, magazine out / in, chambering, pump, shells, bolt,
//! dry fire, draw, ADS rustle, pickups and casings tinkling a moment after each shot. Files
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
];

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
    }
}

fn gun_sounds(
    mut commands: Commands,
    time: Res<Time>,
    sfx: Res<Sfx>,
    master: Res<Volumes>,
    mut events: MessageReader<GunEvent>,
    // Casings waiting to land (seconds left).
    mut casings: Local<Vec<f32>>,
) {
    let mut play = |name: &str, fallback: Option<&str>, v: f32| {
        let n = if sfx.has(name) { name } else { fallback.unwrap_or(name) };
        engine::audio::play(&mut commands, &sfx, &master, n, AudioCategory::Sfx, v, None);
    };
    for ev in events.read() {
        match *ev {
            GunEvent::Shot { kind } => {
                let v = match kind {
                    WeaponKind::Smg => 0.7,
                    WeaponKind::Pistol => 0.8,
                    _ => 0.9,
                };
                play(shot_name(kind), Some("rifle_shot"), v);
                if casings.len() < 4 && !matches!(kind, WeaponKind::Shotgun | WeaponKind::Marksman) {
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
