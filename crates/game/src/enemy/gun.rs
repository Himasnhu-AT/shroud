//! Enemy gun presentation: the gun in the mannequin's right hand (shaped per weapon kind) that
//! points where the enemy aims, muzzle flashes, short tracers, a glint before an enemy opens
//! fire. (Team colours and operator kit: `enemy::skin`.) (A dead enemy's gun becomes the arsenal's pickup, `weapon::pickups`.) Gameplay only reads
//! [`muzzle_point`]; everything else here runs only with visuals.
//!
//! Cost: every flash, tracer, glint and muzzle light comes from a small fixed pool spawned once
//! (shared meshes and materials, hidden when idle); nothing is spawned or allocated per shot.

use std::f32::consts::PI;

use bevy::prelude::*;
use brain::weapons::WeaponKind;

use super::{Enemy, EnemyCombat};
use crate::assets::AssetsDir;
use crate::state::has_visuals;
use crate::weapon::viewmodel;

/// Right-hand bone of the Quaternius mannequin (fingers point along its +Y).
const HAND_BONE: &str = "hand_r";
/// Enemy guns are the arsenal's own models (`weapon::viewmodel::fit`: per-gun path, hand point,
/// muzzle; barrel toward model -Z), this much bigger than the first-person scale so they read
/// from the front at 10–20 m in the mannequin's hand (a bit over life size, on purpose).
const THIRD_PERSON: f32 = 1.9;
/// Gun pitch while not engaged (low ready), radians.
const LOW_READY: f32 = 0.45;

/// Pool sizes.
const FLASHES: usize = 12;
const TRACERS: usize = 24;
const GLINTS: usize = 8;
const LIGHTS: usize = 4;
const LIGHT_INTENSITY: f32 = 60_000.0;

/// What the enemy FX layer should draw. Written by the AI every shot (headless too; unread there).
// The pooled FX flash at `shooter`'s drawn muzzle (else at `from`); `Glint.shooter` is for listeners.
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
pub(super) struct EnemyGun {
    owner: Entity,
    rot: Option<Quat>,
    /// The kind it is currently showing, and that model's entity.
    kind: Option<WeaponKind>,
    model: Option<Entity>,
    muzzle: Entity,
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

/// Shared primitive mesh/material for guns whose model file is missing.
#[derive(Resource)]
struct GunParts {
    body: Handle<Mesh>,
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
                (attach_guns, restyle_guns, drop_guns, spawn_fx, tick_fx).run_if(has_visuals).run_if(resource_exists::<GunParts>),
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
        body: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
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
                engine::render_budget::Actor,
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

/// When an enemy's hand bone appears, give it a gun pivot (the model comes in `restyle_guns`).
fn attach_guns(mut commands: Commands, names: Query<(Entity, &Name), Added<Name>>, parents: Query<&ChildOf>, enemies: Query<(), With<Enemy>>) {
    for (hand, name) in &names {
        if name.as_str() != HAND_BONE {
            continue;
        }
        let Some(owner) = owner_enemy(hand, &parents, &enemies) else { continue };
        let gun = commands.spawn((Name::new("enemy gun"), Transform::default(), Visibility::default(), ChildOf(hand))).id();
        let muzzle = commands.spawn((Transform::default(), Visibility::default(), ChildOf(gun))).id();
        commands.entity(gun).insert(EnemyGun { owner, rot: None, kind: None, model: None, muzzle });
        commands.entity(owner).insert(GunLink { gun, muzzle });
    }
}

/// Show the gun its owner holds (the primary, or the sidearm after a switch): the arsenal's
/// model for that kind, the hand point on the pivot, the muzzle where that model's barrel ends.
fn restyle_guns(
    mut commands: Commands,
    mut guns: Query<(Entity, &mut EnemyGun)>,
    owners: Query<&EnemyCombat>,
    mut transforms: Query<&mut Transform>,
    dir: Res<AssetsDir>,
    server: Res<AssetServer>,
    parts: Res<GunParts>,
) {
    for (gun, mut g) in &mut guns {
        let Ok(c) = owners.get(g.owner) else { continue };
        let kind = c.gun.held;
        if g.kind == Some(kind) {
            continue;
        }
        g.kind = Some(kind);
        if let Some(old) = g.model.take() {
            commands.entity(old).despawn();
        }
        let fit = viewmodel::fit(kind);
        let k = THIRD_PERSON;
        let model = if dir.has(fit.path) {
            let scene = server.load::<bevy::world_serialization::WorldAsset>(bevy::gltf::GltfAssetLabel::Scene(0).from_asset(fit.path));
            commands
                .spawn((
                    bevy::world_serialization::WorldAssetRoot(scene),
                    Transform::from_translation(fit.offset * k).with_scale(Vec3::splat(fit.scale * k)),
                    Visibility::default(),
                    ChildOf(gun),
                ))
                .id()
        } else {
            let size = fit.body * k;
            commands
                .spawn((
                    Mesh3d(parts.body.clone()),
                    MeshMaterial3d(parts.mat.clone()),
                    Transform::from_xyz(0.0, 0.0, -size.z * 0.4).with_scale(size),
                    ChildOf(gun),
                ))
                .id()
        };
        g.model = Some(model);
        if let Ok(mut m) = transforms.get_mut(g.muzzle) {
            m.translation = fit.muzzle * k;
        }
    }
}

/// Point each held gun at the enemy's aim target (or low ready), hand point in the palm.
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
        // The barrel is model -Z (Transform forward): point it at the target.
        let want = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
        let rot = g.rot.map_or(want, |r| r.slerp(want, k));
        g.rot = Some(rot);
        let world = Transform { translation: palm, rotation: rot, scale: Vec3::ONE };
        let local = GlobalTransform::from(world).reparented_to(hand);
        if *tf != local {
            *tf = local;
        }
    }
}

/// A dead enemy's gun leaves its hand: the arsenal drops a pickup of its `Armed` gun in its place.
fn drop_guns(mut commands: Commands, enemies: Query<(Entity, &EnemyCombat, &GunLink)>) {
    for (e, c, link) in &enemies {
        if c.dead {
            commands.entity(e).remove::<GunLink>();
            commands.entity(link.gun).despawn();
        }
    }
}

/// Light up pooled FX for this frame's shots and glints. A shooter with a gun in hand flashes
/// at that gun's muzzle (its last drawn pose), along its barrel; the tracer starts there too.
fn spawn_fx(
    mut fx: MessageReader<EnemyFx>,
    mut pool: ResMut<FxPool>,
    mut q: Query<(&mut PooledFx, &mut Transform, &mut Visibility)>,
    mut lights: Query<&mut PointLight>,
    links: Query<&GunLink>,
    globals: Query<&GlobalTransform>,
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
            EnemyFx::Shot { shooter, from, to } => {
                let drawn = links.get(shooter).ok().and_then(|l| globals.get(l.muzzle).ok()).filter(|m| m.translation().distance(from) < 1.5);
                let from = drawn.map_or(from, |m| m.translation());
                let to_dir = to.filter(|t| t.distance(from) > 0.05).map(|t| (t - from).normalize());
                let barrel = drawn.and_then(|m| m.forward().as_vec3().try_normalize());
                let aim = barrel.or(to_dir).unwrap_or(Vec3::NEG_Z);
                // Muzzle flash: a stretched glowing blob just past the barrel plus a brief light.
                let flash = Transform::from_translation(from + aim * 0.07).looking_to(aim, Vec3::Y).with_scale(Vec3::new(0.06, 0.06, 0.15));
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
