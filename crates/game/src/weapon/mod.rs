//! The player's arsenal: health and damage, guns, ballistics, viewmodels, effects, pickups.
//!
//! Stats come from the shared contract [`brain::weapons`] (one table for player, enemies and the
//! trainer). The player carries a primary + the Pistol sidearm ([`Arsenal`]); the gun in hand is
//! [`Weapon`] (alias [`Rifle`] for older code). Each frame, in [`WeaponSet`]:
//! `arsenal::switching` (1/2, wheel, Q, holster/draw) → `arsenal::aim` (ADS blend, spread) →
//! `fire::fire_and_reload` (trigger, pellets, penetration, recoil, reloads) → [`apply_damage`],
//! the single place that changes [`Health`].
//!
//! - `arsenal`: inventory, input, switching, ADS (FOV, sensitivity), loadout resets.
//! - `fire`: fire modes, spread cone, recoil patterns, damage falloff, headshots, penetration,
//!   tactical / empty / shell-by-shell reloads, surfaces.
//! - `viewmodel`: per-gun first-person models and their procedural animation.
//! - `fx`: muzzle flash, tracers, impacts per surface, shell casings, bullet holes (pooled).
//! - `pickups`: dropped guns (enemies drop theirs), hold E to swap, ammo from same-kind guns.
//! - `sound`: per-gun shots and foley from [`GunEvent`]s.

use avian3d::prelude::*;
use bevy::prelude::*;
use brain::math::V2;

pub mod arsenal;
mod casings;
#[cfg(feature = "dev")]
mod dev;
pub mod fire;
pub mod fx;
pub mod pickups;
mod sound;
pub mod viewmodel;

pub use crate::armed::{Armed, WeaponKind};
pub use arsenal::{Arsenal, Weapon};
pub use brain::weapons::{FireMode, Slot};
pub use fire::Surface;

use crate::assets::{AssetsDir, ENEMY_MODEL};
use crate::map::{ArenaRes, to_world};
use crate::player::{Player, PlayerSet};
use crate::state::GameState;

/// Older name for the gun in the player's hands (the AI, telemetry and HUD read
/// `ammo`, `mag` and `reloading`).
pub type Rifle = Weapon;

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Damage>()
            .add_message::<ShotFired>()
            .add_message::<Tracer>()
            .add_message::<Impact>()
            .add_message::<GunEvent>()
            .init_resource::<HitMarker>()
            .configure_sets(Update, WeaponSet.after(PlayerSet::Move).run_if(in_state(GameState::Playing)))
            .add_systems(Startup, spawn_dummies.run_if(|| std::env::args().any(|a| a == "--dummies")))
            .add_systems(Update, (apply_damage.in_set(WeaponSet).after(fire::fire_and_reload), tick_hit_marker));
        arsenal::build(app);
        fire::build(app);
        viewmodel::build(app);
        fx::build(app);
        pickups::build(app);
        sound::build(app);
        #[cfg(feature = "dev")]
        dev::build(app);
    }
}

/// Firing + damage application. Runs after `PlayerSet::Move` while Playing.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeaponSet;

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Health {
    pub hp: f32,
    pub max: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Self { hp: max, max }
    }
    pub fn is_dead(&self) -> bool {
        self.hp <= 0.0
    }
}

/// Request to damage `target` (an entity with [`Health`]).
#[derive(Message, Clone, Copy, Debug)]
pub struct Damage {
    pub target: Entity,
    pub amount: f32,
    pub source: Entity,
    pub headshot: bool,
}

/// A shot was fired (for hearing, telemetry, effects). One per trigger pull (a shotgun blast is
/// one shot of several pellets).
#[derive(Message, Clone, Copy, Debug)]
pub struct ShotFired {
    pub origin: Vec3,
    pub dir: Vec3,
    pub by: Entity,
    pub kind: WeaponKind,
    /// How far the shot is heard, metres (`kind.spec().loudness_m` unless suppressed). Read by
    /// enemy hearing.
    #[allow(dead_code)]
    pub loudness_m: f32,
}

impl ShotFired {
    pub fn new(origin: Vec3, dir: Vec3, by: Entity, kind: WeaponKind) -> Self {
        Self { origin, dir, by, kind, loudness_m: kind.spec().loudness_m }
    }
}

/// Visual-only: draw a tracer from `from` to `to`.
#[derive(Message, Clone, Copy, Debug)]
pub struct Tracer {
    pub from: Vec3,
    pub to: Vec3,
}

/// Visual-only: a bullet hit `pos` on a surface facing `normal`. `exit` = the far side of
/// penetrated cover (no decal sound duplication).
#[derive(Message, Clone, Copy, Debug)]
pub struct Impact {
    pub pos: Vec3,
    pub normal: Vec3,
    pub flesh: bool,
    pub surface: Surface,
    pub exit: bool,
}

/// What the player's guns did this frame, for sound and viewmodel animation.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum GunEvent {
    Shot { kind: WeaponKind },
    /// Trigger pulled on an empty gun with nothing to reload.
    DryFire,
    MagOut { kind: WeaponKind },
    MagIn { kind: WeaponKind },
    /// Charging handle / slide after an empty reload.
    Chamber { kind: WeaponKind },
    ShellIn,
    /// Pump-action cycle after a shotgun shot.
    Pump,
    /// Bolt cycle after a marksman shot.
    Bolt,
    Draw { kind: WeaponKind },
    AdsIn,
    Pickup { kind: WeaponKind },
    Inspect,
}

/// Put on a collider entity to route hits to `owner`'s [`Health`].
/// Hits on colliders without a `Hitbox` damage the hit entity itself if it has `Health`.
#[derive(Component, Clone, Copy, Debug)]
pub struct Hitbox {
    pub owner: Entity,
    pub head: bool,
}

/// How long the hit marker shows after a hit, and after a kill (it pops and fades out).
pub const HIT_MARKER_S: f32 = 0.22;
pub const KILL_MARKER_S: f32 = 0.4;

/// Last time the player hit something; the HUD shows a marker while `timer > 0`.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct HitMarker {
    pub timer: f32,
    pub headshot: bool,
    pub kill: bool,
}

/// Static test target. Respawns at full HP when killed.
#[derive(Component)]
pub struct Dummy;

/// Applies every [`Damage`] message to its target's [`Health`].
pub fn apply_damage(
    mut msgs: MessageReader<Damage>,
    mut healths: Query<(&mut Health, Has<Dummy>)>,
    players: Query<(), With<Player>>,
    mut marker: ResMut<HitMarker>,
) {
    for d in msgs.read() {
        let Ok((mut h, dummy)) = healths.get_mut(d.target) else { continue };
        if h.is_dead() {
            continue;
        }
        h.hp = (h.hp - d.amount).max(0.0);
        if h.is_dead() {
            if players.contains(d.source) {
                marker.kill = true;
                marker.timer = KILL_MARKER_S;
            }
            if dummy {
                h.hp = h.max;
            }
        }
    }
}

fn tick_hit_marker(time: Res<Time>, mut marker: ResMut<HitMarker>) {
    if marker.timer > 0.0 {
        marker.timer = (marker.timer - time.delta_secs()).max(0.0);
    }
}

fn spawn_dummies(
    mut commands: Commands,
    arena: Res<ArenaRes>,
    assets_dir: Res<AssetsDir>,
    asset_server: Option<Res<AssetServer>>,
    headless: Res<crate::Headless>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let spawn = arena.0.player_spawn;
    let offsets = [V2::new(-5.0, -9.0), V2::new(0.0, -12.0), V2::new(5.0, -9.0)];

    let model = match (&asset_server, headless.0) {
        (Some(server), false) if assets_dir.has(ENEMY_MODEL) => Some(
            server.load::<bevy::world_serialization::WorldAsset>(bevy::gltf::GltfAssetLabel::Scene(0).from_asset(ENEMY_MODEL)),
        ),
        _ => None,
    };
    let prims = match (headless.0, model.is_some(), meshes, materials) {
        (false, false, Some(mut m), Some(mut s)) => Some((
            m.add(Capsule3d::new(0.35, 0.75)),
            m.add(Sphere::new(0.2)),
            s.add(StandardMaterial { base_color: Color::srgb(0.75, 0.2, 0.18), perceptual_roughness: 0.7, ..default() }),
        )),
        _ => None,
    };

    for (i, off) in offsets.into_iter().enumerate() {
        let ground = arena.0.resolve_collision(spawn + off, 0.6);
        let dummy = commands
            .spawn((
                Name::new(format!("Dummy {i}")),
                Dummy,
                Health::new(100.0),
                RigidBody::Static,
                Transform::from_translation(to_world(ground, 0.0)).looking_at(to_world(spawn, 0.0), Vec3::Y),
                Visibility::default(),
            ))
            .id();
        let mut body = commands.spawn((
            Hitbox { owner: dummy, head: false },
            Collider::capsule(0.35, 0.75),
            Transform::from_xyz(0.0, 0.725, 0.0),
            ChildOf(dummy),
        ));
        if let Some((cap, _, mat)) = &prims {
            body.insert((Mesh3d(cap.clone()), MeshMaterial3d(mat.clone())));
        }
        let mut head =
            commands.spawn((Hitbox { owner: dummy, head: true }, Collider::sphere(0.2), Transform::from_xyz(0.0, 1.62, 0.0), ChildOf(dummy)));
        if let Some((_, sph, mat)) = &prims {
            head.insert((Mesh3d(sph.clone()), MeshMaterial3d(mat.clone())));
        }
        if let Some(scene) = &model {
            commands.spawn((bevy::world_serialization::WorldAssetRoot(scene.clone()), Transform::default(), ChildOf(dummy)));
        }
    }
}

/// Smoothstep for blends (ADS, poses).
pub(crate) fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Frame-rate independent approach factor for exponential smoothing at `rate` per second.
pub(crate) fn approach(rate: f32, dt: f32) -> f32 {
    1.0 - (-rate * dt).exp()
}

/// Display blurb per gun (match setup cards, pickups).
pub fn blurb(kind: WeaponKind) -> &'static str {
    match kind {
        WeaponKind::Pistol => "Sidearm. Quick to draw, accurate when you take your time.",
        WeaponKind::Smg => "Hornet SMG. 14 rounds a second; wins corners, loses lanes.",
        WeaponKind::Rifle => "Kestrel rifle. The all-rounder; punches through thin cover.",
        WeaponKind::Shotgun => "Breacher pump. Nine pellets; brutal inside 8 m.",
        WeaponKind::Marksman => "Longbow scout. Light scoped bolt rifle; quick on its feet, one-shot headshots.",
        WeaponKind::HeavyPistol => "Warden heavy pistol. Seven big rounds; one tap to the head up close.",
        WeaponKind::BattleRifle => "Shrike battle rifle. Hits harder than the Kestrel, kicks harder; one-tap headshots.",
        WeaponKind::Sniper => "Osprey sniper. Heavy bolt action; one body shot kills. Slow to scope and to move.",
    }
}

/// Short HUD class line.
pub fn class_label(kind: WeaponKind) -> &'static str {
    match kind {
        WeaponKind::Pistol => "PISTOL",
        WeaponKind::Smg => "SMG",
        WeaponKind::Rifle => "RIFLE",
        WeaponKind::Shotgun => "SHOTGUN",
        WeaponKind::Marksman => "SCOUT",
        WeaponKind::HeavyPistol => "HEAVY PISTOL",
        WeaponKind::BattleRifle => "BATTLE RIFLE",
        WeaponKind::Sniper => "SNIPER",
    }
}

/// Parse a gun name from the control port / CLI ("smg", "Hornet", "marksman" …).
pub fn parse_kind(s: &str) -> Option<WeaponKind> {
    let s = s.trim().to_ascii_lowercase();
    WeaponKind::ALL.into_iter().find(|k| {
        format!("{k:?}").to_ascii_lowercase() == s || k.label().to_ascii_lowercase() == s || class_label(*k).to_ascii_lowercase() == s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kind_accepts_names_and_labels() {
        assert_eq!(parse_kind("smg"), Some(WeaponKind::Smg));
        assert_eq!(parse_kind("Longbow"), Some(WeaponKind::Marksman));
        assert_eq!(parse_kind(" SHOTGUN "), Some(WeaponKind::Shotgun));
        assert_eq!(parse_kind("bazooka"), None);
    }

    #[test]
    fn shot_fired_new_takes_loudness_from_spec() {
        let s = ShotFired::new(Vec3::ZERO, Vec3::NEG_Z, Entity::PLACEHOLDER, WeaponKind::Marksman);
        assert_eq!(s.loudness_m, WeaponKind::Marksman.spec().loudness_m);
    }
}
