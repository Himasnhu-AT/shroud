//! Hitscan rifle, health and damage.
//!
//! Firing reads [`PlayerIntent`] (`fire`, `reload`), raycasts from the player
//! camera with Avian's `SpatialQuery`, and emits [`ShotFired`] and [`Damage`]
//! messages. [`apply_damage`] is the single place that changes [`Health`].

use avian3d::prelude::*;
use bevy::prelude::*;
use brain::math::V2;

use crate::assets::{AssetsDir, ENEMY_MODEL, RIFLE_MODEL};
use crate::map::{ArenaRes, to_world};
use crate::player::{Player, PlayerCamera, PlayerIntent, PlayerSet, ViewKick};
use crate::state::{GameState, has_visuals};

pub const RIFLE_DAMAGE: f32 = 25.0;
pub const HEADSHOT_MULT: f32 = 2.0;
pub const FIRE_RATE: f32 = 10.0;
pub const MAG_SIZE: u32 = 30;
pub const RELOAD_TIME: f32 = 2.0;
pub const RANGE: f32 = 200.0;
const KICK_PITCH: f32 = 0.018;
const KICK_YAW: f32 = 0.006;
const KICK_RECOVERY: f32 = 7.0;
/// Viewmodel offset from the camera.
const VIEWMODEL_POS: Vec3 = Vec3::new(0.2, -0.2, -0.45);
/// Scale for `rifle.glb` (unknown authoring scale; tweak once the model lands).
const RIFLE_MODEL_SCALE: f32 = 1.0;
const MUZZLE_POS: Vec3 = Vec3::new(0.2, -0.16, -0.78);

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Damage>()
            .add_message::<ShotFired>()
            .add_message::<Tracer>()
            .init_resource::<HitMarker>()
            .configure_sets(
                Update,
                WeaponSet.after(PlayerSet::Move).run_if(in_state(GameState::Playing)),
            )
            .add_systems(Startup, spawn_dummies.run_if(|| std::env::args().any(|a| a == "--dummies")))
            .add_systems(PostStartup, (attach_rifle, spawn_viewmodel.run_if(has_visuals)))
            .add_systems(OnEnter(GameState::Playing), reset_rifle)
            .add_systems(
                Update,
                (
                    (fire_rifle, apply_damage).chain().in_set(WeaponSet),
                    tick_hit_marker,
                    (spawn_tracers, animate_viewmodel, fade_fx)
                        .after(WeaponSet)
                        .run_if(has_visuals),
                ),
            );
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

/// A shot was fired (for hearing, telemetry, effects).
#[derive(Message, Clone, Copy, Debug)]
pub struct ShotFired {
    pub origin: Vec3,
    pub dir: Vec3,
    pub by: Entity,
}

/// Visual-only: draw a tracer from `from` to `to`.
#[derive(Message, Clone, Copy, Debug)]
pub struct Tracer {
    pub from: Vec3,
    pub to: Vec3,
}

/// Put on a collider entity to route hits to `owner`'s [`Health`].
/// Hits on colliders without a `Hitbox` damage the hit entity itself if it has `Health`.
#[derive(Component, Clone, Copy, Debug)]
pub struct Hitbox {
    pub owner: Entity,
    pub head: bool,
}

/// The player's rifle state (on the [`Player`] entity).
#[derive(Component, Clone, Copy, Debug)]
pub struct Rifle {
    pub ammo: u32,
    pub mag: u32,
    /// Seconds until the next shot is allowed.
    pub cooldown: f32,
    /// Seconds left on the current reload, if reloading.
    pub reloading: Option<f32>,
}

impl Default for Rifle {
    fn default() -> Self {
        Self {
            ammo: MAG_SIZE,
            mag: MAG_SIZE,
            cooldown: 0.0,
            reloading: None,
        }
    }
}

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

#[derive(Component)]
struct ViewModel {
    kick: f32,
}

#[derive(Component)]
struct Muzzle;

#[derive(Component)]
struct Fx {
    ttl: f32,
}

fn attach_rifle(mut commands: Commands, players: Query<Entity, (With<Player>, Without<Rifle>)>) {
    for e in &players {
        commands.entity(e).insert(Rifle::default());
    }
}

fn reset_rifle(mut q: Query<&mut Rifle>) {
    for mut r in &mut q {
        *r = Rifle::default();
    }
}

fn fire_rifle(
    time: Res<Time>,
    spatial: SpatialQuery,
    intent: Res<PlayerIntent>,
    mut players: Query<(Entity, &Transform, &mut Rifle, &mut ViewKick), With<Player>>,
    cams: Query<&Transform, (With<PlayerCamera>, Without<Player>)>,
    hitboxes: Query<&Hitbox>,
    healths: Query<(), With<Health>>,
    mut marker: ResMut<HitMarker>,
    mut shots: MessageWriter<ShotFired>,
    mut damage: MessageWriter<Damage>,
    mut tracers: MessageWriter<Tracer>,
) {
    let dt = time.delta_secs();
    let Ok((player, ptf, mut rifle, mut kick)) = players.single_mut() else {
        return;
    };

    // Recoil recovery.
    let k = (-KICK_RECOVERY * dt).exp();
    kick.pitch *= k;
    kick.yaw *= k;

    rifle.cooldown = (rifle.cooldown - dt).max(0.0);
    if let Some(left) = rifle.reloading {
        let left = left - dt;
        if left <= 0.0 {
            rifle.ammo = rifle.mag;
            rifle.reloading = None;
        } else {
            rifle.reloading = Some(left);
            return;
        }
    }
    if (intent.reload && rifle.ammo < rifle.mag) || (intent.fire && rifle.ammo == 0) {
        rifle.reloading = Some(RELOAD_TIME);
        return;
    }
    if !intent.fire || rifle.cooldown > 0.0 {
        return;
    }

    let Ok(cam_local) = cams.single() else { return };
    // Compose from local transforms: GlobalTransform lags a frame behind movement.
    let cam = ptf.mul_transform(*cam_local);
    let origin = cam.translation;
    let dir = cam.forward();
    rifle.ammo -= 1;
    rifle.cooldown += 1.0 / FIRE_RATE;
    kick.pitch += KICK_PITCH;
    kick.yaw += rand::random_range(-KICK_YAW..KICK_YAW);

    shots.write(ShotFired {
        origin,
        dir: *dir,
        by: player,
    });

    let filter = SpatialQueryFilter::from_excluded_entities([player]);
    let hit = spatial.cast_ray(origin, dir, RANGE, true, &filter);
    let end = hit.map_or(origin + *dir * RANGE, |h| origin + *dir * h.distance);
    tracers.write(Tracer {
        from: cam.transform_point(MUZZLE_POS),
        to: end,
    });

    let Some(hit) = hit else { return };
    let target = match hitboxes.get(hit.entity) {
        Ok(hb) => Some((hb.owner, hb.head)),
        Err(_) if healths.contains(hit.entity) => Some((hit.entity, false)),
        Err(_) => None,
    };
    if let Some((target, headshot)) = target {
        let amount = RIFLE_DAMAGE * if headshot { HEADSHOT_MULT } else { 1.0 };
        damage.write(Damage {
            target,
            amount,
            source: player,
            headshot,
        });
        *marker = HitMarker {
            timer: 0.18,
            headshot,
            kill: false,
        };
    }
}

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
                marker.timer = 0.3;
            }
            if dummy {
                h.hp = h.max;
            }
        }
    }
}

fn tick_hit_marker(time: Res<Time>, mut marker: ResMut<HitMarker>) {
    marker.timer = (marker.timer - time.delta_secs()).max(0.0);
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
            server.load::<bevy::world_serialization::WorldAsset>(
                bevy::gltf::GltfAssetLabel::Scene(0).from_asset(ENEMY_MODEL),
            ),
        ),
        _ => None,
    };
    let prims = match (headless.0, model.is_some(), meshes, materials) {
        (false, false, Some(mut m), Some(mut s)) => Some((
            m.add(Capsule3d::new(0.35, 0.75)),
            m.add(Sphere::new(0.2)),
            s.add(StandardMaterial {
                base_color: Color::srgb(0.75, 0.2, 0.18),
                perceptual_roughness: 0.7,
                ..default()
            }),
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
                Transform::from_translation(to_world(ground, 0.0))
                    .looking_at(to_world(spawn, 0.0), Vec3::Y),
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
        let mut head = commands.spawn((
            Hitbox { owner: dummy, head: true },
            Collider::sphere(0.2),
            Transform::from_xyz(0.0, 1.62, 0.0),
            ChildOf(dummy),
        ));
        if let Some((_, sph, mat)) = &prims {
            head.insert((Mesh3d(sph.clone()), MeshMaterial3d(mat.clone())));
        }
        if let Some(scene) = &model {
            commands.spawn((
                bevy::world_serialization::WorldAssetRoot(scene.clone()),
                Transform::default(),
                ChildOf(dummy),
            ));
        }
    }
}

fn spawn_viewmodel(
    mut commands: Commands,
    cams: Query<Entity, With<PlayerCamera>>,
    assets_dir: Res<AssetsDir>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(cam) = cams.single() else { return };
    commands.entity(cam).insert(Visibility::default());
    let vm = commands
        .spawn((
            ViewModel { kick: 0.0 },
            Transform::from_translation(VIEWMODEL_POS),
            Visibility::default(),
            ChildOf(cam),
        ))
        .id();
    if assets_dir.has(RIFLE_MODEL) {
        let scene = asset_server.load::<bevy::world_serialization::WorldAsset>(
            bevy::gltf::GltfAssetLabel::Scene(0).from_asset(RIFLE_MODEL),
        );
        commands.spawn((
            bevy::world_serialization::WorldAssetRoot(scene),
            Transform::from_scale(Vec3::splat(RIFLE_MODEL_SCALE)),
            ChildOf(vm),
        ));
    } else {
        let gun = materials.add(StandardMaterial {
            base_color: Color::srgb(0.12, 0.12, 0.13),
            perceptual_roughness: 0.5,
            metallic: 0.6,
            ..default()
        });
        let parts = [
            (Cuboid::new(0.07, 0.1, 0.45), Vec3::new(0.0, 0.0, 0.0)),
            (Cuboid::new(0.03, 0.03, 0.25), Vec3::new(0.0, 0.03, -0.33)),
            (Cuboid::new(0.05, 0.14, 0.06), Vec3::new(0.0, -0.1, 0.08)),
            (Cuboid::new(0.04, 0.12, 0.08), Vec3::new(0.0, -0.08, -0.08)),
        ];
        for (shape, pos) in parts {
            commands.spawn((
                Mesh3d(meshes.add(shape)),
                MeshMaterial3d(gun.clone()),
                Transform::from_translation(pos),
                bevy::light::NotShadowCaster,
                ChildOf(vm),
            ));
        }
    }
    commands.spawn((
        Muzzle,
        Transform::from_translation(MUZZLE_POS),
        Visibility::default(),
        ChildOf(cam),
    ));
}

fn spawn_tracers(
    mut commands: Commands,
    mut tracers: MessageReader<Tracer>,
    muzzles: Query<Entity, With<Muzzle>>,
    mut vms: Query<&mut ViewModel>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for t in tracers.read() {
        let len = t.from.distance(t.to);
        if len > 0.5 {
            let mid = (t.from + t.to) * 0.5;
            commands.spawn((
                Fx { ttl: 0.05 },
                Mesh3d(meshes.add(Cuboid::new(0.015, 0.015, len))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(1.0, 0.85, 0.5),
                    emissive: LinearRgba::rgb(6.0, 4.0, 1.5),
                    unlit: true,
                    ..default()
                })),
                Transform::from_translation(mid).looking_at(t.to, Vec3::Y),
                bevy::light::NotShadowCaster,
            ));
        }
        if let Ok(m) = muzzles.single() {
            commands.spawn((
                Fx { ttl: 0.04 },
                Mesh3d(meshes.add(Sphere::new(0.05))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(1.0, 0.8, 0.4),
                    emissive: LinearRgba::rgb(12.0, 8.0, 2.0),
                    unlit: true,
                    ..default()
                })),
                Transform::default(),
                bevy::light::NotShadowCaster,
                ChildOf(m),
            ));
        }
        for mut vm in &mut vms {
            vm.kick = (vm.kick + 0.03).min(0.08);
        }
    }
}

fn animate_viewmodel(time: Res<Time>, mut vms: Query<(&mut ViewModel, &mut Transform)>) {
    let k = (-14.0 * time.delta_secs()).exp();
    for (mut vm, mut tf) in &mut vms {
        vm.kick *= k;
        tf.translation = VIEWMODEL_POS + Vec3::new(0.0, vm.kick * 0.3, vm.kick);
        tf.rotation = Quat::from_rotation_x(vm.kick * 1.5);
    }
}

fn fade_fx(mut commands: Commands, time: Res<Time>, mut fx: Query<(Entity, &mut Fx)>) {
    for (e, mut f) in &mut fx {
        f.ttl -= time.delta_secs();
        if f.ttl <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}
