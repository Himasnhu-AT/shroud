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
use bevy::light::NotShadowCaster;

use crate::gfx::{FaceCamera, FxAssets};
use crate::player::{Player, PlayerIntent, PlayerSet};
use engine::fps::{FpsLook, FpsMotor, FpsViewKick};
use engine::gfx::MainCamera;
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
/// Viewmodel pivot (roughly the grip) relative to the camera.
const VIEWMODEL_POS: Vec3 = Vec3::new(0.2, -0.19, -0.3);
/// `rifle.glb` is 1.39 m long: stock end at its origin, barrel toward +Z
/// (glTF forward), so it is turned 180° about Y to point down −Z.
const RIFLE_MODEL_SCALE: f32 = 0.46;
const RIFLE_MODEL_OFFSET: Vec3 = Vec3::new(-0.045 * RIFLE_MODEL_SCALE, -0.02, 0.2);
/// Muzzle relative to the viewmodel pivot.
const MUZZLE_LOCAL: Vec3 = Vec3::new(0.0, -0.02, 0.2 - 1.40 * RIFLE_MODEL_SCALE);
/// Muzzle relative to the camera (tracer origin).
const MUZZLE_POS: Vec3 = Vec3::new(
    VIEWMODEL_POS.x + MUZZLE_LOCAL.x,
    VIEWMODEL_POS.y + MUZZLE_LOCAL.y,
    VIEWMODEL_POS.z + MUZZLE_LOCAL.z,
);
const FLASH_TIME: f32 = 0.05;
const TRACER_SPEED: f32 = 350.0;
const MAX_DECALS: usize = 64;

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Damage>()
            .add_message::<ShotFired>()
            .add_message::<Tracer>()
            .add_message::<Impact>()
            .init_resource::<HitMarker>()
            .init_resource::<Decals>()
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
                    (spawn_shot_fx, animate_viewmodel, update_fx, viewmodel_no_shadows)
                        .chain()
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

/// Visual-only: a bullet hit `pos` on a surface facing `normal`.
/// `flesh` hits get a red puff and no decal.
#[derive(Message, Clone, Copy, Debug)]
pub struct Impact {
    pub pos: Vec3,
    pub normal: Vec3,
    pub flesh: bool,
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

/// First-person rifle pivot (child of the camera). Holds animation state.
#[derive(Component, Default)]
struct ViewModel {
    /// Recoil spring (≈1 after one shot) and its velocity.
    kick: f32,
    kick_vel: f32,
    /// Random per-shot roll/yaw jitter, decays.
    jitter: f32,
    /// Smoothed look-rate sway (x = yaw, y = pitch).
    sway: Vec2,
    last_look: Option<Vec2>,
    bob_phase: f32,
    bob_amp: f32,
    /// 0..1 blend into the reload pose.
    reload: f32,
    /// Pulled back when the barrel would poke into a wall.
    wall_push: f32,
    /// Seconds of muzzle flash left.
    flash: f32,
}

#[derive(Component)]
struct Muzzle;

/// Muzzle flash sprites and light (children of [`Muzzle`]).
#[derive(Component)]
struct MuzzleFlash;

#[derive(Component)]
struct Fx {
    ttl: f32,
    age: f32,
    kind: FxKind,
}

enum FxKind {
    Tracer { from: Vec3, dir: Vec3, len: f32 },
    Spark { vel: Vec3 },
    Puff { vel: Vec3, size: f32, alpha: f32, mat: Handle<StandardMaterial> },
    Decal,
}

/// Live bullet-hole decals, oldest first.
#[derive(Resource, Default)]
struct Decals(std::collections::VecDeque<Entity>);

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
    mut players: Query<(Entity, &Transform, &mut Rifle, &mut FpsViewKick), With<Player>>,
    cams: Query<&Transform, (With<MainCamera>, Without<Player>)>,
    hitboxes: Query<&Hitbox>,
    healths: Query<(), With<Health>>,
    mut marker: ResMut<HitMarker>,
    mut shots: MessageWriter<ShotFired>,
    mut damage: MessageWriter<Damage>,
    mut tracers: MessageWriter<Tracer>,
    mut impacts: MessageWriter<Impact>,
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
    impacts.write(Impact {
        pos: end,
        normal: hit.normal,
        flesh: target.is_some(),
    });
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
    cams: Query<Entity, With<MainCamera>>,
    assets_dir: Res<AssetsDir>,
    asset_server: Res<AssetServer>,
    fx: Option<Res<FxAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(cam) = cams.single() else { return };
    commands.entity(cam).insert(Visibility::default());
    let vm = commands
        .spawn((
            Name::new("ViewModel"),
            ViewModel::default(),
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
            Transform::from_translation(RIFLE_MODEL_OFFSET)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
                .with_scale(Vec3::splat(RIFLE_MODEL_SCALE)),
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
            (Cuboid::new(0.07, 0.1, 0.45), Vec3::new(0.0, 0.0, -0.05)),
            (Cuboid::new(0.03, 0.03, 0.22), Vec3::new(0.0, 0.02, -0.37)),
            (Cuboid::new(0.05, 0.14, 0.06), Vec3::new(0.0, -0.1, 0.03)),
            (Cuboid::new(0.04, 0.12, 0.08), Vec3::new(0.0, -0.08, -0.13)),
        ];
        for (shape, pos) in parts {
            commands.spawn((
                Mesh3d(meshes.add(shape)),
                MeshMaterial3d(gun.clone()),
                Transform::from_translation(pos),
                ChildOf(vm),
            ));
        }
    }
    let muzzle = commands
        .spawn((
            Muzzle,
            Transform::from_translation(MUZZLE_LOCAL),
            Visibility::default(),
            ChildOf(vm),
        ))
        .id();
    commands.spawn((
        MuzzleFlash,
        PointLight {
            intensity: 0.0,
            range: 12.0,
            color: Color::srgb(1.0, 0.75, 0.4),
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, -0.1),
        ChildOf(muzzle),
    ));
    let Some(fx) = fx else { return };
    // Front-on star plus a side flame stretched along the barrel.
    for tf in [
        Transform::from_scale(Vec3::splat(0.2)),
        Transform::from_xyz(0.0, 0.0, -0.1)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::new(0.3, 0.12, 1.0)),
        Transform::from_xyz(0.0, 0.0, -0.1)
            .with_rotation(
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
                    * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            )
            .with_scale(Vec3::new(0.3, 0.12, 1.0)),
    ] {
        commands.spawn((
            MuzzleFlash,
            Mesh3d(fx.quad.clone()),
            MeshMaterial3d(fx.flash.clone()),
            tf,
            Visibility::Hidden,
            NotShadowCaster,
            ChildOf(muzzle),
        ));
    }
}

/// The rifle scene's meshes must not cast shadows into the world.
fn viewmodel_no_shadows(
    mut commands: Commands,
    added: Query<Entity, (Added<Mesh3d>, Without<NotShadowCaster>)>,
    parents: Query<&ChildOf>,
    vms: Query<(), With<ViewModel>>,
) {
    for e in &added {
        if parents.iter_ancestors(e).any(|a| vms.contains(a)) {
            commands.entity(e).insert(NotShadowCaster);
        }
    }
}

fn spawn_shot_fx(
    mut commands: Commands,
    fx: Option<Res<FxAssets>>,
    mut tracers: MessageReader<Tracer>,
    mut impacts: MessageReader<Impact>,
    mut shots: MessageReader<ShotFired>,
    players: Query<(), With<Player>>,
    mut vms: Query<&mut ViewModel>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut decals: ResMut<Decals>,
) {
    for s in shots.read() {
        if !players.contains(s.by) {
            continue;
        }
        for mut vm in &mut vms {
            vm.kick_vel += 30.0;
            vm.jitter = rand::random_range(-1.0..1.0);
            vm.flash = FLASH_TIME;
        }
    }
    let Some(fx) = fx else {
        tracers.clear();
        impacts.clear();
        return;
    };
    for t in tracers.read() {
        let len = t.from.distance(t.to);
        if len < 0.5 {
            continue;
        }
        commands.spawn((
            Fx {
                ttl: len / TRACER_SPEED + 0.05,
                age: 0.0,
                kind: FxKind::Tracer {
                    from: t.from,
                    dir: (t.to - t.from) / len,
                    len,
                },
            },
            Mesh3d(fx.cube.clone()),
            MeshMaterial3d(fx.tracer.clone()),
            Transform::from_translation(t.from).with_scale(Vec3::ZERO),
            NotShadowCaster,
        ));
    }
    for hit in impacts.read() {
        let n = hit.normal.try_normalize().unwrap_or(Vec3::Y);
        // Sparks off hard surfaces, a few droplets off flesh.
        let (count, color) = if hit.flesh { (4, None) } else { (7, Some(fx.spark.clone())) };
        for _ in 0..count {
            let r = Vec3::new(
                rand::random_range(-1.0..1.0),
                rand::random_range(-1.0..1.0),
                rand::random_range(-1.0..1.0),
            );
            let dir = (n * 1.2 + r).normalize_or(n);
            let vel = dir * rand::random_range(3.0..8.0);
            let mat = color.clone().unwrap_or_else(|| {
                mats.add(StandardMaterial {
                    base_color: Color::srgb(0.35, 0.02, 0.02),
                    perceptual_roughness: 0.4,
                    ..default()
                })
            });
            commands.spawn((
                Fx {
                    ttl: rand::random_range(0.12..0.3),
                    age: 0.0,
                    kind: FxKind::Spark { vel },
                },
                Mesh3d(fx.cube.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(hit.pos + n * 0.02).with_scale(Vec3::ZERO),
                NotShadowCaster,
            ));
        }
        // Dust (or blood) puff.
        let (tint, alpha, size) = if hit.flesh {
            (Color::srgba(0.45, 0.03, 0.03, 1.0), 0.8, 0.25)
        } else {
            (Color::srgba(0.62, 0.6, 0.56, 1.0), 0.6, 0.5)
        };
        let puff_mat = mats.add(StandardMaterial {
            base_color: tint,
            base_color_texture: Some(fx.puff_tex.clone()),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        });
        commands.spawn((
            Fx {
                ttl: 0.6,
                age: 0.0,
                kind: FxKind::Puff {
                    vel: n * 0.8 + Vec3::Y * 0.3,
                    size,
                    alpha,
                    mat: puff_mat.clone(),
                },
            },
            FaceCamera,
            Mesh3d(fx.quad.clone()),
            MeshMaterial3d(puff_mat),
            Transform::from_translation(hit.pos + n * 0.1)
                .with_rotation(Quat::from_rotation_z(rand::random_range(0.0..std::f32::consts::TAU)))
                .with_scale(Vec3::splat(size)),
            NotShadowCaster,
        ));
        if !hit.flesh {
            let up = if n.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
            let e = commands
                .spawn((
                    Fx {
                        ttl: 12.0,
                        age: 0.0,
                        kind: FxKind::Decal,
                    },
                    Mesh3d(fx.quad.clone()),
                    MeshMaterial3d(fx.decal.clone()),
                    Transform::from_translation(hit.pos + n * 0.004)
                        .looking_to(-n, up)
                        .with_scale(Vec3::splat(rand::random_range(0.14..0.2))),
                    NotShadowCaster,
                ))
                .id();
            decals.0.push_back(e);
            while decals.0.len() > MAX_DECALS {
                if let Some(old) = decals.0.pop_front() {
                    commands.entity(old).try_despawn();
                }
            }
        }
    }
}

fn animate_viewmodel(
    time: Res<Time>,
    spatial: SpatialQuery,
    players: Query<(Entity, &FpsLook, &FpsMotor, Option<&Rifle>), With<Player>>,
    cams: Query<&GlobalTransform, With<MainCamera>>,
    mut vms: Query<(&mut ViewModel, &mut Transform)>,
    mut flashes: Query<
        (&mut Visibility, &mut Transform, Option<&mut PointLight>),
        (With<MuzzleFlash>, Without<ViewModel>),
    >,
) {
    let dt = time.delta_secs().clamp(1e-4, 0.05);
    let Ok((player, look, motor, rifle)) = players.single() else { return };
    let Ok((mut vm, mut tf)) = vms.single_mut() else { return };
    let t = time.elapsed_secs();

    // Sway: the gun lags behind look rotation.
    let look_now = Vec2::new(look.yaw, look.pitch);
    let rate = vm.last_look.map_or(Vec2::ZERO, |l| (look_now - l) / dt);
    vm.last_look = Some(look_now);
    let target = (rate * 0.012).clamp(Vec2::splat(-0.06), Vec2::splat(0.06));
    let s = 1.0 - (-10.0 * dt).exp();
    vm.sway = vm.sway.lerp(target, s);

    // Bob: figure-eight while walking on the ground.
    let speed = Vec2::new(motor.velocity.x, motor.velocity.z).length();
    let amp_target = if motor.grounded { (speed / 6.0).min(1.3) } else { 0.0 };
    vm.bob_amp += (amp_target - vm.bob_amp) * (1.0 - (-8.0 * dt).exp());
    vm.bob_phase += dt * (speed * 1.8).max(0.0);
    let bob = Vec3::new(
        vm.bob_phase.cos() * 0.011,
        -vm.bob_phase.sin().abs() * 0.013,
        0.0,
    ) * vm.bob_amp
        + Vec3::Y * (t * 1.7).sin() * 0.0025;

    // Recoil: damped spring, kicked by each shot.
    let acc = -320.0 * vm.kick - 24.0 * vm.kick_vel;
    vm.kick_vel += acc * dt;
    vm.kick = (vm.kick + vm.kick_vel * dt).clamp(-0.5, 1.6);
    vm.jitter *= (-10.0 * dt).exp();

    // Reload: dip and roll the gun, with a "mag slap" bump near the end.
    let reloading = rifle.and_then(|r| r.reloading);
    let r_target = if reloading.is_some() { 1.0 } else { 0.0 };
    vm.reload += (r_target - vm.reload) * (1.0 - (-9.0 * dt).exp());
    let p = reloading.map_or(1.0, |left| 1.0 - left / RELOAD_TIME);
    let slap = (-((p - 0.62) / 0.05).powi(2)).exp() * vm.reload;

    // Pull the barrel back from walls instead of clipping through them.
    let push = cams.single().ok().and_then(|cam| {
        let filter = SpatialQueryFilter::from_excluded_entities([player]);
        spatial
            .cast_ray(cam.translation(), cam.forward(), 1.0, true, &filter)
            .map(|h| (0.85 - h.distance).clamp(0.0, 0.4))
    });
    let push_target = push.unwrap_or(0.0);
    vm.wall_push += (push_target - vm.wall_push) * (1.0 - (-14.0 * dt).exp());

    let (k, r, w) = (vm.kick, vm.reload, vm.wall_push);
    tf.translation = VIEWMODEL_POS
        + Vec3::new(vm.sway.x * 0.35, -vm.sway.y * 0.35, 0.0)
        + bob
        + Vec3::new(0.0, 0.012 * k, 0.05 * k)
        + Vec3::new(-0.03 * r, -0.09 * r + 0.02 * slap, 0.03 * r)
        + Vec3::new(0.0, -0.05 * w, w * 0.6);
    tf.rotation = Quat::from_euler(
        EulerRot::YXZ,
        vm.sway.x * 1.2 + vm.jitter * 0.012,
        -vm.sway.y * 1.0 + 0.07 * k + 0.12 * r + 0.06 * slap - 0.5 * w,
        vm.sway.x * 1.6 + vm.jitter * 0.03 + 0.5 * r,
    );

    // Muzzle flash.
    vm.flash = (vm.flash - dt).max(0.0);
    let f = vm.flash / FLASH_TIME;
    let on = f > 0.0;
    let roll = Quat::from_rotation_z(rand::random_range(0.0..std::f32::consts::TAU));
    let grow = rand::random_range(0.8..1.25);
    for (mut vis, mut ftf, light) in &mut flashes {
        if let Some(mut light) = light {
            light.intensity = if on { 2.5e6 * f.sqrt() } else { 0.0 };
            continue;
        }
        *vis = if on { Visibility::Inherited } else { Visibility::Hidden };
        if on && ftf.translation.z == 0.0 {
            // Front-on star: new roll and size each frame it is shown.
            ftf.rotation = roll;
            ftf.scale = Vec3::splat(0.2 * grow);
        }
    }
}

fn update_fx(
    mut commands: Commands,
    time: Res<Time>,
    mut fx: Query<(Entity, &mut Fx, &mut Transform)>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();
    for (e, mut f, mut tf) in &mut fx {
        f.age += dt;
        if f.age >= f.ttl {
            commands.entity(e).despawn();
            continue;
        }
        let (age, life) = (f.age, f.age / f.ttl);
        match &mut f.kind {
            FxKind::Tracer { from, dir, len } => {
                let head = (age * TRACER_SPEED).min(*len);
                let tail = (head - 6.0).max(0.0);
                let seg = head - tail;
                if seg <= 0.01 {
                    tf.scale = Vec3::ZERO;
                    continue;
                }
                let mid = *from + *dir * (tail + seg * 0.5);
                *tf = Transform::from_translation(mid)
                    .looking_to(*dir, Vec3::Y)
                    .with_scale(Vec3::new(0.012, 0.012, seg));
            }
            FxKind::Spark { vel } => {
                vel.y -= 9.8 * dt;
                tf.translation += *vel * dt;
                let v = *vel;
                let shrink = 1.0 - life;
                *tf = Transform::from_translation(tf.translation)
                    .looking_to(v.normalize_or(Vec3::Y), Vec3::Y)
                    .with_scale(Vec3::new(0.02 * shrink, 0.02 * shrink, 0.14 * shrink + 0.02));
            }
            FxKind::Puff { vel, size, alpha, mat } => {
                tf.translation += *vel * dt;
                *vel *= (-3.0 * dt).exp();
                tf.scale = Vec3::splat(*size * (1.0 + life * 2.0));
                if let Some(mut m) = mats.get_mut(mat.id()) {
                    m.base_color.set_alpha(*alpha * (1.0 - life) * (1.0 - life));
                }
            }
            FxKind::Decal => {}
        }
    }
}
