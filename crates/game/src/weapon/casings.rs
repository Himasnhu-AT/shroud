//! Spent casings: a fixed pool of small cylinders (one shared mesh, a brass and a red-hull
//! material) thrown out of the ejection port, tumbling under gravity and bouncing off the
//! level (one ray per moving casing per frame). Nothing is spawned or allocated per shot: the
//! oldest casing is reused.

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

/// Radius and length (m) and whether it is a red shotgun hull.
fn shape(kind: WeaponKind) -> (f32, f32, bool) {
    match kind {
        WeaponKind::Pistol | WeaponKind::Smg => (0.006, 0.024, false),
        WeaponKind::Rifle => (0.0062, 0.048, false),
        WeaponKind::Marksman => (0.007, 0.058, false),
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
    live: bool,
    resting: bool,
}

#[derive(Resource)]
pub(super) struct Casings {
    slots: Vec<Slot>,
    next: usize,
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

pub(super) fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let mesh = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(10));
    let brass = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.74, 0.36),
        metallic: 0.65,
        perceptual_roughness: 0.38,
        // A touch of self-light (alpha 0: exposure-independent) so brass reads in shade.
        emissive: LinearRgba::new(0.10, 0.07, 0.02, 0.0),
        ..default()
    });
    let hull = materials.add(StandardMaterial { base_color: Color::srgb(0.62, 0.07, 0.05), perceptual_roughness: 0.5, ..default() });
    let slots = (0..POOL)
        .map(|_| {
            let e = commands
                .spawn((Name::new("casing"), Mesh3d(mesh.clone()), MeshMaterial3d(brass.clone()), Transform::default(), Visibility::Hidden, NotShadowCaster))
                .id();
            Slot { e, pos: Vec3::ZERO, vel: Vec3::ZERO, rot: Quat::IDENTITY, spin: Vec3::ZERO, age: 0.0, size: Vec3::ONE, live: false, resting: false }
        })
        .collect();
    commands.insert_resource(Casings { slots, next: 0, brass, hull });
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
    for s in casings.slots.iter_mut().filter(|s| s.live) {
        let Ok((mut tf, mut vis)) = q.get_mut(s.e) else { continue };
        s.age += dt;
        if s.age > LIFE {
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
            let radius = s.size.x;
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
        let k = ((LIFE - s.age) / 0.2).clamp(0.0, 1.0);
        let next = Transform { translation: s.pos, rotation: s.rot, scale: s.size * k };
        if *tf != next {
            *tf = next;
        }
        if *vis != Visibility::Visible {
            *vis = Visibility::Visible;
        }
    }
}
