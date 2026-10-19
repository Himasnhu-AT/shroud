//! Enemy gun presentation: a rifle in the mannequin's right hand that points where the enemy
//! aims, muzzle flashes, short tracers, a glint before an enemy opens fire, a team tint, and the
//! gun dropping when its owner dies. Gameplay only reads [`muzzle_point`]; everything else here
//! runs only with visuals.

use std::collections::HashMap;
use std::f32::consts::PI;

use bevy::prelude::*;

use super::{Corpse, Enemy, EnemyCombat};
use crate::assets::AssetsDir;
use crate::state::has_visuals;

pub const ENEMY_RIFLE_MODEL: &str = "models/enemy_rifle.glb";
/// Right-hand bone of the Quaternius mannequin (fingers point along its +Y).
const HAND_BONE: &str = "hand_r";
const GUN_SCALE: f32 = 0.85;
/// In gun model space (Kenney blaster-a: barrel along -Z, grip below the back half).
const GRIP: Vec3 = Vec3::new(0.0, -0.1, 0.2);
const MUZZLE: Vec3 = Vec3::new(0.0, 0.05, -0.42);
/// Gun pitch while not engaged (low ready), radians.
const LOW_READY: f32 = 0.45;

/// What the enemy FX layer should draw. Written by the AI every shot (headless too; unread there).
#[derive(Message, Clone, Copy, Debug)]
pub enum EnemyFx {
    /// A shot left `from`; draw a tracer to `to` if set (already kept clear of the player's camera).
    Shot { shooter: Entity, from: Vec3, to: Option<Vec3> },
    /// The enemy just acquired the player and is about to open fire.
    Glint { shooter: Entity, at: Vec3 },
}

/// On the enemy root once its gun is attached.
#[derive(Component)]
pub struct GunLink {
    pub gun: Entity,
    pub muzzle: Entity,
}

#[derive(Component)]
struct EnemyGun {
    owner: Entity,
    rot: Option<Quat>,
}

#[derive(Component)]
struct DroppedGun {
    vel: Vec3,
}

#[derive(Component)]
struct FxLife {
    ttl: f32,
    max: f32,
    /// Grow then shrink over the lifetime (glints); otherwise constant.
    pulse: bool,
}

/// Where a shot leaves: the gun muzzle if the model has one, else a point at the right shoulder.
pub fn muzzle_point(root: &Transform, crouched: bool, link: Option<&GunLink>, globals: &Query<&GlobalTransform>) -> Vec3 {
    link.and_then(|l| globals.get(l.muzzle).ok())
        .map(|g| g.translation())
        .filter(|m| m.distance(root.translation) < 2.5)
        .unwrap_or_else(|| root.transform_point(Vec3::new(0.22, if crouched { 0.85 } else { 1.4 }, -0.55)))
}

pub struct EnemyGunPlugin;

impl Plugin for EnemyGunPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<EnemyFx>().add_systems(
            Update,
            (attach_guns, tint_enemies, drop_guns, fall_dropped, spawn_fx, tick_fx).run_if(has_visuals),
        );
        app.add_systems(
            PostUpdate,
            aim_guns.after(bevy::transform::TransformSystems::Propagate).run_if(has_visuals),
        );
    }
}

fn owner_enemy(mut e: Entity, parents: &Query<&ChildOf>, enemies: &Query<(), With<Enemy>>) -> Option<Entity> {
    while let Ok(p) = parents.get(e) {
        e = p.parent();
        if enemies.contains(e) {
            return Some(e);
        }
    }
    None
}

/// When an enemy's hand bone appears, put a gun in it.
fn attach_guns(
    mut commands: Commands,
    names: Query<(Entity, &Name), Added<Name>>,
    parents: Query<&ChildOf>,
    enemies: Query<(), With<Enemy>>,
    dir: Res<AssetsDir>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (hand, name) in &names {
        if name.as_str() != HAND_BONE {
            continue;
        }
        let Some(owner) = owner_enemy(hand, &parents, &enemies) else { continue };
        let gun = commands
            .spawn((
                Name::new("enemy gun"),
                EnemyGun { owner, rot: None },
                Transform::from_scale(Vec3::splat(GUN_SCALE)),
                Visibility::default(),
                ChildOf(hand),
            ))
            .id();
        if dir.has(ENEMY_RIFLE_MODEL) {
            let scene = server.load::<bevy::world_serialization::WorldAsset>(
                bevy::gltf::GltfAssetLabel::Scene(0).from_asset(ENEMY_RIFLE_MODEL),
            );
            commands.entity(gun).insert(bevy::world_serialization::WorldAssetRoot(scene));
        } else {
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.08, 0.12, 0.75))),
                MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgb(0.1, 0.1, 0.12), ..default() })),
                Transform::from_xyz(0.0, 0.0, -0.02),
                ChildOf(gun),
            ));
        }
        let muzzle = commands.spawn((Transform::from_translation(MUZZLE), Visibility::default(), ChildOf(gun))).id();
        commands.entity(owner).insert(GunLink { gun, muzzle });
    }
}

/// Point each held gun at the enemy's aim target (or low ready), grip in the palm.
/// Runs after transform propagation so it sees this frame's animated hand.
fn aim_guns(
    time: Res<Time>,
    mut guns: Query<(&mut EnemyGun, &mut Transform, &ChildOf)>,
    globals: Query<&GlobalTransform>,
    enemies: Query<&EnemyCombat>,
) {
    let k = 1.0 - (-16.0 * time.delta_secs()).exp();
    for (mut g, mut tf, parent) in &mut guns {
        let (Ok(hand), Ok(root), Ok(c)) = (globals.get(parent.parent()), globals.get(g.owner), enemies.get(g.owner)) else {
            continue;
        };
        let palm = hand.translation() + hand.rotation() * Vec3::Y * 0.08;
        let fwd = root.forward().as_vec3();
        let low = (fwd * LOW_READY.cos() - Vec3::Y * LOW_READY.sin()).normalize();
        let dir = c.aim_at.map_or(low, |p| (p - palm).try_normalize().unwrap_or(fwd));
        let want = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
        let rot = g.rot.map_or(want, |r| r.slerp(want, k));
        g.rot = Some(rot);
        let world = Transform {
            translation: palm - rot * (GRIP * GUN_SCALE),
            rotation: rot,
            scale: Vec3::splat(GUN_SCALE),
        };
        *tf = GlobalTransform::from(world).reparented_to(hand);
    }
}

/// Team colour: tint every enemy body material red with a little glow so they pop.
fn tint_enemies(
    mut commands: Commands,
    added: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Added<MeshMaterial3d<StandardMaterial>>>,
    parents: Query<&ChildOf>,
    enemies: Query<(), With<Enemy>>,
    guns: Query<(), With<EnemyGun>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: Local<HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>>,
) {
    for (e, mat) in &added {
        if cache.values().any(|h| h.id() == mat.id()) {
            continue;
        }
        let mut a = e;
        let mut is_gun = false;
        while let Ok(p) = parents.get(a) {
            a = p.parent();
            is_gun |= guns.contains(a);
        }
        if is_gun || owner_enemy(e, &parents, &enemies).is_none() {
            continue;
        }
        let tinted = match cache.get(&mat.id()) {
            Some(h) => h.clone(),
            None => {
                let Some(orig) = materials.get(&mat.0).cloned() else { continue };
                let c = orig.base_color.to_linear();
                let lum = (c.red + c.green + c.blue) / 3.0;
                let tinted = materials.add(StandardMaterial {
                    base_color: LinearRgba::rgb(0.08 + lum * 0.9, 0.012 + lum * 0.08, 0.01 + lum * 0.06).into(),
                    emissive: LinearRgba::rgb(0.9, 0.06, 0.03),
                    perceptual_roughness: 0.55,
                    ..orig
                });
                cache.insert(mat.id(), tinted.clone());
                tinted
            }
        };
        commands.entity(e).insert(MeshMaterial3d(tinted));
    }
}

/// A dead enemy lets go of its gun.
fn drop_guns(
    mut commands: Commands,
    enemies: Query<(Entity, &EnemyCombat, &GunLink, &Transform)>,
    globals: Query<&GlobalTransform>,
) {
    for (e, c, link, tf) in &enemies {
        if !c.dead {
            continue;
        }
        commands.entity(e).remove::<GunLink>();
        let Ok(g) = globals.get(link.gun) else { continue };
        let side = tf.rotation * Vec3::X;
        commands
            .entity(link.gun)
            .remove::<(ChildOf, EnemyGun)>()
            .insert((g.compute_transform(), DroppedGun { vel: side * 1.2 + Vec3::Y * 1.5 }, Corpse(3.0)));
    }
}

fn fall_dropped(time: Res<Time>, mut q: Query<(&mut DroppedGun, &mut Transform)>) {
    let dt = time.delta_secs();
    for (mut d, mut tf) in &mut q {
        if d.vel == Vec3::ZERO {
            continue;
        }
        d.vel.y -= 9.81 * dt;
        tf.translation += d.vel * dt;
        // Settle on its side.
        let (yaw, _, _) = tf.rotation.to_euler(EulerRot::YXZ);
        let flat = Quat::from_euler(EulerRot::YXZ, yaw, 0.0, PI * 0.5);
        tf.rotation = tf.rotation.slerp(flat, 1.0 - (-8.0 * dt).exp());
        if tf.translation.y <= 0.07 {
            tf.translation.y = 0.07;
            tf.rotation = flat;
            d.vel = Vec3::ZERO;
        }
    }
}

fn spawn_fx(
    mut commands: Commands,
    mut fx: MessageReader<EnemyFx>,
    links: Query<&GunLink>,
    globals: Query<&GlobalTransform>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut assets: Local<Option<(Handle<Mesh>, Handle<Mesh>, Handle<StandardMaterial>, Handle<StandardMaterial>, Handle<StandardMaterial>)>>,
) {
    let (ball, cube, flash_mat, tracer_mat, glint_mat) = assets
        .get_or_insert_with(|| {
            let glow = |c: LinearRgba| StandardMaterial { base_color: Color::WHITE, emissive: c, unlit: true, ..default() };
            (
                meshes.add(Sphere::new(1.0)),
                meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
                materials.add(glow(LinearRgba::rgb(14.0, 8.0, 2.0))),
                materials.add(glow(LinearRgba::rgb(5.0, 3.2, 1.2))),
                materials.add(glow(LinearRgba::rgb(10.0, 12.0, 16.0))),
            )
        })
        .clone();
    // Flash and glint parent to the visual muzzle when there is one, so they ride the gun.
    for f in fx.read() {
        match *f {
            EnemyFx::Shot { shooter, from, to } => {
                let parent = links.get(shooter).ok().map(|l| l.muzzle).filter(|m| globals.contains(*m));
                // Muzzle flash: a stretched glowing blob along the barrel plus a brief light.
                let mut flash = commands.spawn((
                    FxLife { ttl: 0.05, max: 0.05, pulse: false },
                    Mesh3d(ball.clone()),
                    MeshMaterial3d(flash_mat.clone()),
                    bevy::light::NotShadowCaster,
                ));
                match parent {
                    Some(m) => flash.insert((Transform::from_xyz(0.0, 0.0, -0.06).with_scale(Vec3::new(0.05, 0.05, 0.12)), ChildOf(m))),
                    None => flash.insert(Transform::from_translation(from).with_scale(Vec3::splat(0.07))),
                };
                flash.with_child(PointLight {
                    color: Color::srgb(1.0, 0.7, 0.35),
                    intensity: 60_000.0,
                    range: 6.0,
                    ..default()
                });
                // Tracer: thin, gone in two frames.
                if let Some(to) = to {
                    let len = from.distance(to);
                    if len > 0.5 {
                        commands.spawn((
                            FxLife { ttl: 0.035, max: 0.035, pulse: false },
                            Mesh3d(cube.clone()),
                            MeshMaterial3d(tracer_mat.clone()),
                            Transform::from_translation((from + to) * 0.5)
                                .looking_at(to, Vec3::Y)
                                .with_scale(Vec3::new(0.012, 0.012, len)),
                            bevy::light::NotShadowCaster,
                        ));
                    }
                }
            }
            EnemyFx::Glint { shooter, at } => {
                let parent = links.get(shooter).ok().map(|l| l.muzzle).filter(|m| globals.contains(*m));
                let mut glint = commands.spawn((
                    FxLife { ttl: 0.35, max: 0.35, pulse: true },
                    Mesh3d(ball.clone()),
                    MeshMaterial3d(glint_mat.clone()),
                    bevy::light::NotShadowCaster,
                ));
                match parent {
                    Some(m) => glint.insert((Transform::from_scale(Vec3::ZERO), ChildOf(m))),
                    None => glint.insert(Transform::from_translation(at).with_scale(Vec3::ZERO)),
                };
            }
        }
    }
}

fn tick_fx(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut FxLife, &mut Transform)>) {
    for (e, mut f, mut tf) in &mut q {
        f.ttl -= time.delta_secs();
        if f.ttl <= 0.0 {
            commands.entity(e).despawn();
        } else if f.pulse {
            let s = (PI * (1.0 - f.ttl / f.max)).sin();
            tf.scale = Vec3::splat(0.09 * s);
        }
    }
}
