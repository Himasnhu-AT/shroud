//! Enemy gun presentation: the gun in the mannequin's right hand (shaped per weapon kind) that
//! points where the enemy aims, muzzle flashes, short tracers, a glint before an enemy opens
//! fire, a team tint, and the gun dropping when its owner dies. Gameplay only reads
//! [`muzzle_point`]; everything else here runs only with visuals.
//!
//! Cost: every flash, tracer, glint and muzzle light comes from a small fixed pool spawned once
//! (shared meshes and materials, hidden when idle); nothing is spawned or allocated per shot.

use std::collections::HashMap;
use std::f32::consts::PI;

use bevy::prelude::*;
use brain::weapons::WeaponKind;

use super::{Corpse, Enemy, EnemyCombat};
use crate::assets::AssetsDir;
use crate::state::has_visuals;

pub const ENEMY_RIFLE_MODEL: &str = "models/enemy_rifle.glb";
/// Right-hand bone of the Quaternius mannequin (fingers point along its +Y).
const HAND_BONE: &str = "hand_r";
/// In gun model space (Kenney blasters: muzzle toward -Z at z ~ -0.37, grip/stock toward +Z).
const GRIP: Vec3 = Vec3::new(0.0, -0.15, 0.2);
const MUZZLE: Vec3 = Vec3::new(0.0, 0.05, -0.39);
/// Gun pitch while not engaged (low ready), radians.
const LOW_READY: f32 = 0.45;

/// How each gun looks in an enemy's hand: the one blaster model, scaled per kind (x/y bulk,
/// z length), plus a scope on the marksman rifle.
#[derive(Clone, Copy)]
struct GunLook {
    scale: Vec3,
    scope: bool,
}

fn look(k: WeaponKind) -> GunLook {
    match k {
        WeaponKind::Pistol => GunLook { scale: Vec3::new(0.6, 0.6, 0.42), scope: false },
        WeaponKind::Smg => GunLook { scale: Vec3::new(0.72, 0.75, 0.62), scope: false },
        WeaponKind::Rifle => GunLook { scale: Vec3::splat(0.85), scope: false },
        WeaponKind::Shotgun => GunLook { scale: Vec3::new(1.1, 1.0, 0.9), scope: false },
        WeaponKind::Marksman => GunLook { scale: Vec3::new(0.78, 0.8, 1.3), scope: true },
    }
}

/// Pool sizes.
const FLASHES: usize = 12;
const TRACERS: usize = 24;
const GLINTS: usize = 8;
const LIGHTS: usize = 4;
const LIGHT_INTENSITY: f32 = 60_000.0;

/// What the enemy FX layer should draw. Written by the AI every shot (headless too; unread there).
// `shooter` is for listeners (audio, tests); the pooled FX place flashes at `from`.
#[allow(dead_code)]
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
    /// The kind it is currently shaped as, its scale and its scope child.
    kind: Option<WeaponKind>,
    scale: Vec3,
    scope: Entity,
}

#[derive(Component)]
struct DroppedGun {
    vel: Vec3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FxKind {
    Flash,
    Tracer,
    Glint,
    Light,
}

/// One pooled FX entity: hidden while `ttl <= 0`.
#[derive(Component)]
struct PooledFx {
    kind: FxKind,
    ttl: f32,
    max: f32,
}

#[derive(Resource, Default)]
struct FxPool {
    flashes: Vec<Entity>,
    tracers: Vec<Entity>,
    glints: Vec<Entity>,
    lights: Vec<Entity>,
    next: [usize; 4],
}

impl FxPool {
    fn take(&mut self, k: FxKind) -> Option<Entity> {
        let (list, i) = match k {
            FxKind::Flash => (&self.flashes, 0),
            FxKind::Tracer => (&self.tracers, 1),
            FxKind::Glint => (&self.glints, 2),
            FxKind::Light => (&self.lights, 3),
        };
        if list.is_empty() {
            return None;
        }
        let e = list[self.next[i] % list.len()];
        self.next[i] = self.next[i].wrapping_add(1);
        Some(e)
    }
}

/// Shared primitive meshes/materials for guns without the glTF model and for scopes.
#[derive(Resource)]
struct GunParts {
    body: Handle<Mesh>,
    scope: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
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
        app.add_message::<EnemyFx>()
            .init_resource::<FxPool>()
            .add_systems(Startup, setup.run_if(has_visuals))
            .add_systems(
                Update,
                (attach_guns, restyle_guns, tint_enemies, drop_guns, fall_dropped, spawn_fx, tick_fx).run_if(has_visuals).run_if(resource_exists::<GunParts>),
            );
        app.add_systems(
            PostUpdate,
            aim_guns.after(bevy::transform::TransformSystems::Propagate).run_if(has_visuals),
        );
    }
}

/// Shared gun parts and the FX pool, once.
fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>, mut pool: ResMut<FxPool>) {
    commands.insert_resource(GunParts {
        body: meshes.add(Cuboid::new(0.08, 0.12, 0.75)),
        scope: meshes.add(Cuboid::new(0.055, 0.06, 0.3)),
        mat: materials.add(StandardMaterial { base_color: Color::srgb(0.1, 0.1, 0.12), perceptual_roughness: 0.5, ..default() }),
    });
    // Unlit ignores emissive, so the HDR colour goes in base_color (bloom picks up > 1).
    let glow = |c: LinearRgba| StandardMaterial { base_color: c.into(), unlit: true, ..default() };
    let ball = meshes.add(Sphere::new(1.0));
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let flash_mat = materials.add(glow(LinearRgba::rgb(9.0, 5.0, 1.2)));
    let tracer_mat = materials.add(glow(LinearRgba::rgb(3.0, 1.7, 0.5)));
    let glint_mat = materials.add(glow(LinearRgba::rgb(6.0, 7.5, 10.0)));
    let mut fx = |kind: FxKind, mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>| {
        commands
            .spawn((
                Name::new("enemy fx"),
                PooledFx { kind, ttl: 0.0, max: 1.0 },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::default(),
                Visibility::Hidden,
                bevy::light::NotShadowCaster,
            ))
            .id()
    };
    pool.flashes = (0..FLASHES).map(|_| fx(FxKind::Flash, &ball, &flash_mat)).collect();
    pool.tracers = (0..TRACERS).map(|_| fx(FxKind::Tracer, &cube, &tracer_mat)).collect();
    pool.glints = (0..GLINTS).map(|_| fx(FxKind::Glint, &ball, &glint_mat)).collect();
    pool.lights = (0..LIGHTS)
        .map(|_| {
            commands
                .spawn((
                    Name::new("enemy muzzle light"),
                    PooledFx { kind: FxKind::Light, ttl: 0.0, max: 1.0 },
                    PointLight { color: Color::srgb(1.0, 0.7, 0.35), intensity: LIGHT_INTENSITY, range: 6.0, ..default() },
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
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
    parts: Res<GunParts>,
) {
    for (hand, name) in &names {
        if name.as_str() != HAND_BONE {
            continue;
        }
        let Some(owner) = owner_enemy(hand, &parents, &enemies) else { continue };
        let gun = commands.spawn((Name::new("enemy gun"), Transform::from_scale(Vec3::splat(0.85)), Visibility::default(), ChildOf(hand))).id();
        if dir.has(ENEMY_RIFLE_MODEL) {
            let scene = server.load::<bevy::world_serialization::WorldAsset>(bevy::gltf::GltfAssetLabel::Scene(0).from_asset(ENEMY_RIFLE_MODEL));
            commands.entity(gun).insert(bevy::world_serialization::WorldAssetRoot(scene));
        } else {
            commands.spawn((Mesh3d(parts.body.clone()), MeshMaterial3d(parts.mat.clone()), Transform::from_xyz(0.0, 0.0, -0.02), ChildOf(gun)));
        }
        let scope = commands
            .spawn((
                Mesh3d(parts.scope.clone()),
                MeshMaterial3d(parts.mat.clone()),
                Transform::from_xyz(0.0, 0.2, 0.02),
                Visibility::Hidden,
                ChildOf(gun),
            ))
            .id();
        let muzzle = commands.spawn((Transform::from_translation(MUZZLE), Visibility::default(), ChildOf(gun))).id();
        commands.entity(gun).insert(EnemyGun { owner, rot: None, kind: None, scale: Vec3::splat(0.85), scope });
        commands.entity(owner).insert(GunLink { gun, muzzle });
    }
}

/// Shape the gun after what its owner holds (the primary, or the sidearm after a switch).
fn restyle_guns(mut guns: Query<&mut EnemyGun>, owners: Query<&EnemyCombat>, mut vis: Query<&mut Visibility>) {
    for mut g in &mut guns {
        let Ok(c) = owners.get(g.owner) else { continue };
        let kind = c.gun.held;
        if g.kind == Some(kind) {
            continue;
        }
        let l = look(kind);
        g.kind = Some(kind);
        g.scale = l.scale;
        if let Ok(mut v) = vis.get_mut(g.scope) {
            let want = if l.scope { Visibility::Inherited } else { Visibility::Hidden };
            if *v != want {
                *v = want;
            }
        }
    }
}

/// Point each held gun at the enemy's aim target (or low ready), grip in the palm.
/// Runs after transform propagation so it sees this frame's animated hand.
fn aim_guns(time: Res<Time>, mut guns: Query<(&mut EnemyGun, &mut Transform, &ChildOf)>, globals: Query<&GlobalTransform>, enemies: Query<&EnemyCombat>) {
    let k = 1.0 - (-16.0 * time.delta_secs()).exp();
    for (mut g, mut tf, parent) in &mut guns {
        let (Ok(hand), Ok(root), Ok(c)) = (globals.get(parent.parent()), globals.get(g.owner), enemies.get(g.owner)) else {
            continue;
        };
        let palm = hand.translation() + hand.rotation() * Vec3::Y * 0.08;
        let fwd = root.forward().as_vec3();
        let low = (fwd * LOW_READY.cos() - Vec3::Y * LOW_READY.sin()).normalize();
        let dir = c.aim_at.map_or(low, |p| (p - palm).try_normalize().unwrap_or(fwd));
        // The muzzle is at model -Z: point -Z (Transform forward) at the target.
        let want = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
        let rot = g.rot.map_or(want, |r| r.slerp(want, k));
        g.rot = Some(rot);
        let world = Transform { translation: palm - rot * (GRIP * g.scale), rotation: rot, scale: g.scale };
        let local = GlobalTransform::from(world).reparented_to(hand);
        if *tf != local {
            *tf = local;
        }
    }
}

/// Team colour: tint every enemy body material red with a little glow so they pop. One tinted
/// material per source material, shared by every enemy.
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
fn drop_guns(mut commands: Commands, enemies: Query<(Entity, &EnemyCombat, &GunLink, &Transform)>, globals: Query<&GlobalTransform>) {
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

/// Light up pooled FX for this frame's shots and glints.
fn spawn_fx(
    mut fx: MessageReader<EnemyFx>,
    mut pool: ResMut<FxPool>,
    mut q: Query<(&mut PooledFx, &mut Transform, &mut Visibility)>,
    mut lights: Query<&mut PointLight>,
) {
    let mut show = |pool: &mut FxPool, kind: FxKind, ttl: f32, tf: Transform| {
        let e = pool.take(kind)?;
        let Ok((mut p, mut t, mut v)) = q.get_mut(e) else { return None };
        p.ttl = ttl;
        p.max = ttl;
        *t = tf;
        if *v != Visibility::Visible {
            *v = Visibility::Visible;
        }
        Some(e)
    };
    for f in fx.read() {
        match *f {
            EnemyFx::Shot { from, to, .. } => {
                let aim = to.filter(|t| t.distance(from) > 0.05).map_or(Vec3::NEG_Z, |t| (t - from).normalize());
                // Muzzle flash: a stretched glowing blob along the barrel plus a brief light.
                let flash = Transform::from_translation(from + aim * 0.06).looking_to(aim, Vec3::Y).with_scale(Vec3::new(0.05, 0.05, 0.12));
                show(&mut pool, FxKind::Flash, 0.05, flash);
                if let Some(l) = show(&mut pool, FxKind::Light, 0.05, Transform::from_translation(from))
                    && let Ok(mut pl) = lights.get_mut(l)
                    && pl.intensity != LIGHT_INTENSITY
                {
                    pl.intensity = LIGHT_INTENSITY;
                }
                // Tracer: thin, gone in two frames.
                if let Some(to) = to {
                    let len = from.distance(to);
                    if len > 0.5 {
                        let t = Transform::from_translation((from + to) * 0.5).looking_at(to, Vec3::Y).with_scale(Vec3::new(0.012, 0.012, len));
                        show(&mut pool, FxKind::Tracer, 0.035, t);
                    }
                }
            }
            EnemyFx::Glint { at, .. } => {
                show(&mut pool, FxKind::Glint, 0.35, Transform::from_translation(at).with_scale(Vec3::ZERO));
            }
        }
    }
}

fn tick_fx(time: Res<Time>, mut q: Query<(&mut PooledFx, &mut Transform, &mut Visibility)>) {
    let dt = time.delta_secs();
    for (mut f, mut tf, mut v) in &mut q {
        if f.ttl <= 0.0 {
            continue;
        }
        f.ttl -= dt;
        if f.ttl <= 0.0 {
            *v = Visibility::Hidden;
        } else if f.kind == FxKind::Glint {
            let s = (PI * (1.0 - f.ttl / f.max)).sin();
            tf.scale = Vec3::splat(0.09 * s);
        }
    }
}
