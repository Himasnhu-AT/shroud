//! Spent casings: a fixed pool of small cartridge cases (one shared mesh, a brass and a red-hull
//! material) thrown out of the ejection port, tumbling under gravity and bouncing off the
//! level (one ray per moving casing per frame). Nothing is spawned or allocated per shot: the
//! oldest casing is reused. Magazine reloads drop the spent magazine the same way (its own
//! small pool; it stays on the floor a while).

use avian3d::prelude::*;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::WeaponKind;
use crate::player::Player;

/// Casings alive at once (the oldest is reused): about 2.5 s of full-auto fire.
const POOL: usize = 24;
/// Seconds a casing stays (moving or on the ground) before it is hidden.
const LIFE: f32 = 3.0;
const GRAVITY: f32 = 9.81;
const RESTITUTION: f32 = 0.35;
/// Fraction of the along-surface speed kept per bounce.
const FRICTION: f32 = 0.6;
/// Below this speed after a bounce on a floor, a casing lies down and stops.
const REST_SPEED: f32 = 0.45;
/// Dropped magazines kept at once, and how long one lies on the floor.
const MAG_POOL: usize = 4;
const MAG_LIFE: f32 = 12.0;

/// Size (m: thickness, height, depth) of a gun's magazine, if it has a detachable one.
fn mag_shape(kind: WeaponKind) -> Option<Vec3> {
    match kind {
        WeaponKind::Pistol | WeaponKind::HeavyPistol => Some(Vec3::new(0.022, 0.12, 0.034)),
        WeaponKind::Smg => Some(Vec3::new(0.024, 0.2, 0.04)),
        WeaponKind::Rifle | WeaponKind::BattleRifle => Some(Vec3::new(0.026, 0.17, 0.068)),
        WeaponKind::Marksman | WeaponKind::Sniper => Some(Vec3::new(0.03, 0.08, 0.09)),
        WeaponKind::Shotgun => None,
    }
}

/// Radius and length (m) and whether it is a red shotgun hull.
fn shape(kind: WeaponKind) -> (f32, f32, bool) {
    match kind {
        WeaponKind::Pistol | WeaponKind::Smg => (0.006, 0.024, false),
        WeaponKind::HeavyPistol => (0.0072, 0.03, false),
        WeaponKind::Rifle => (0.0062, 0.048, false),
        WeaponKind::BattleRifle => (0.0068, 0.05, false),
        WeaponKind::Marksman => (0.007, 0.058, false),
        WeaponKind::Sniper => (0.0078, 0.07, false),
        WeaponKind::Shotgun => (0.0105, 0.065, true),
    }
}

struct Slot {
    e: Entity,
    pos: Vec3,
    vel: Vec3,
    rot: Quat,
    /// Angular velocity (world, rad/s).
    spin: Vec3,
    age: f32,
    size: Vec3,
    /// Collision radius (m) and lifetime (s).
    radius: f32,
    life: f32,
    live: bool,
    resting: bool,
}

#[derive(Resource)]
pub(super) struct Casings {
    slots: Vec<Slot>,
    next: usize,
    mags: Vec<Slot>,
    next_mag: usize,
    brass: Handle<StandardMaterial>,
    hull: Handle<StandardMaterial>,
}

impl Casings {
    /// Throws a casing of `kind` from `at`, right and up relative to the camera `rot`.
    pub fn eject(&mut self, at: Vec3, rot: Quat, kind: WeaponKind, mats: &mut Query<&mut MeshMaterial3d<StandardMaterial>>) {
        if self.slots.is_empty() {
            return;
        }
        let i = self.next % self.slots.len();
        self.next = self.next.wrapping_add(1);
        let (r, len, hull) = shape(kind);
        let j = |a: f32| rand::random_range(-a..=a);
        let v_cam = Vec3::new(2.4 + j(0.5), 2.0 + j(0.6), 0.4 + j(0.4));
        let s = &mut self.slots[i];
        s.pos = at;
        s.vel = rot * v_cam;
        // Cylinder axis (+Y) along the barrel (camera -Z), tumbling end over end.
        s.rot = rot * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        s.spin = rot * Vec3::new(j(6.0), 14.0 + j(6.0), 18.0 + j(8.0));
        s.age = 0.0;
        s.size = Vec3::new(r, len, r);
        s.radius = r;
        s.life = LIFE;
        s.live = true;
        s.resting = false;
        let want = if hull { &self.hull } else { &self.brass };
        if let Ok(mut m) = mats.get_mut(s.e)
            && m.0 != *want
        {
            m.0 = want.clone();
        }
    }
}

impl Casings {
    /// Drops `kind`'s spent magazine from `at` (world), under the gun held at camera `rot`.
    pub fn drop_mag(&mut self, at: Vec3, rot: Quat, kind: WeaponKind) {
        let Some(size) = mag_shape(kind) else { return };
        if self.mags.is_empty() {
            return;
        }
        let i = self.next_mag % self.mags.len();
        self.next_mag = self.next_mag.wrapping_add(1);
        let j = |a: f32| rand::random_range(-a..=a);
        let s = &mut self.mags[i];
        s.pos = at;
        // Released, it falls clear of the well with a little push down and toward the body.
        s.vel = rot * Vec3::new(j(0.15), -1.2 + j(0.3), 0.3 + j(0.15));
        s.rot = rot;
        s.spin = rot * Vec3::new(j(3.0) + 2.0, j(1.5), j(2.0));
        s.age = 0.0;
        s.size = size;
        s.radius = size.x * 0.5;
        s.life = MAG_LIFE;
        s.live = true;
        s.resting = false;
    }
}

pub(super) fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let mesh = meshes.add(casing_mesh());
    // Fired brass: metal, a little dulled by powder residue (reflects the room rather than glowing).
    let brass = materials.add(StandardMaterial { base_color: Color::srgb(0.8, 0.6, 0.3), metallic: 1.0, perceptual_roughness: 0.32, ..default() });
    let hull = materials.add(StandardMaterial { base_color: Color::srgb(0.62, 0.07, 0.05), perceptual_roughness: 0.5, ..default() });
    let slot = |e| Slot { e, pos: Vec3::ZERO, vel: Vec3::ZERO, rot: Quat::IDENTITY, spin: Vec3::ZERO, age: 0.0, size: Vec3::ONE, radius: 0.01, life: LIFE, live: false, resting: false };
    let slots = (0..POOL)
        .map(|_| {
            slot(commands
                .spawn((Name::new("casing"), Mesh3d(mesh.clone()), MeshMaterial3d(brass.clone()), Transform::default(), Visibility::Hidden, NotShadowCaster, engine::render_budget::Actor))
                .id())
        })
        .collect();
    // Magazines: a black polymer box, shadowed (they lie on the floor for a while).
    let mag_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mag_mat = materials.add(StandardMaterial { base_color: Color::srgb(0.035, 0.035, 0.038), perceptual_roughness: 0.6, reflectance: 0.4, ..default() });
    let mags = (0..MAG_POOL)
        .map(|_| slot(commands.spawn((Name::new("dropped magazine"), Mesh3d(mag_mesh.clone()), MeshMaterial3d(mag_mat.clone()), Transform::default(), Visibility::Hidden, engine::render_budget::Actor)).id()))
        .collect();
    commands.insert_resource(Casings { slots, next: 0, mags, next_mag: 0, brass, hull });
}

/// A unit cartridge case along +Y (radius 1, height 1, centred): the body, a shoulder and neck
/// (rifle cases; on pistol cases it reads as the mouth), and the extractor groove above the rim.
fn casing_mesh() -> Mesh {
    let part = |r_bottom: f32, r_top: f32, h: f32, y: f32| {
        ConicalFrustum { radius_top: r_top, radius_bottom: r_bottom, height: h }.mesh().resolution(10).segments(1).build().translated_by(Vec3::Y * y)
    };
    let mut m = part(1.0, 1.0, 0.06, -0.47);
    for p in [part(0.82, 0.82, 0.05, -0.415), part(0.98, 0.95, 0.66, -0.06), part(0.95, 0.72, 0.08, 0.31), part(0.72, 0.72, 0.15, 0.425)] {
        m.merge(&p).expect("same layout");
    }
    m
}

/// Moves, bounces and retires live casings.
pub(super) fn update(
    time: Res<Time>,
    casings: Option<ResMut<Casings>>,
    spatial: SpatialQuery,
    player: Query<Entity, With<Player>>,
    mut q: Query<(&mut Transform, &mut Visibility)>,
) {
    let Some(mut casings) = casings else { return };
    let dt = time.delta_secs().min(0.05);
    if dt <= 0.0 {
        return;
    }
    let filter = SpatialQueryFilter::from_excluded_entities(player.iter());
    let casings = &mut *casings;
    for s in casings.slots.iter_mut().chain(casings.mags.iter_mut()).filter(|s| s.live) {
        let Ok((mut tf, mut vis)) = q.get_mut(s.e) else { continue };
        s.age += dt;
        if s.age > s.life {
            s.live = false;
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        if !s.resting {
            s.vel.y -= GRAVITY * dt;
            let step = s.vel * dt;
            let dist = step.length();
            let radius = s.radius;
            if let Ok(dir) = Dir3::new(step)
                && let Some(hit) = spatial.cast_ray(s.pos, dir, dist + radius, true, &filter)
            {
                let n = hit.normal;
                s.pos += dir * (hit.distance - radius).max(0.0) + n * radius;
                let vn = s.vel.dot(n);
                let vt = s.vel - n * vn;
                s.vel = vt * FRICTION - n * vn * RESTITUTION;
                s.spin *= 0.6;
                if n.y > 0.7 && s.vel.length() < REST_SPEED {
                    // Lie on the floor, axis horizontal, keeping its heading.
                    s.resting = true;
                    let axis = (s.rot * Vec3::Y).with_y(0.0).try_normalize().unwrap_or(Vec3::X);
                    s.rot = Quat::from_rotation_arc(Vec3::Y, axis);
                }
            } else {
                s.pos += step;
            }
            if !s.resting {
                let w = s.spin * dt;
                s.rot = (Quat::from_scaled_axis(w) * s.rot).normalize();
            }
        }
        // Shrink away over the last 0.2 s.
        let k = ((s.life - s.age) / 0.2).clamp(0.0, 1.0);
        let next = Transform { translation: s.pos, rotation: s.rot, scale: s.size * k };
        if *tf != next {
            *tf = next;
        }
        if *vis != Visibility::Visible {
            *vis = Visibility::Visible;
        }
    }
}
