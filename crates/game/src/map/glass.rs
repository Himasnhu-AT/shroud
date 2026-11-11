//! Breakable window glass.
//!
//! Every glazed window opening (`brain::arena::Decor::windows` with `glass`) gets a thin pane
//! entity with a [`Glass`] component. Glass has **no collider**: it never stops a bullet, a
//! player or the AI (the 2D brain treats the window as plain Low cover). Shooting it shatters
//! it: the pane hides and a burst of shards falls out. Panes come back at the start of every
//! round.
//!
//! API for the weapon stream (or anything else):
//! - write [`ShatterGlass`]`(entity)` to break one pane;
//! - write [`BulletPath`]`{ origin, dir, max_dist }` for a shot (or a grenade blast ray):
//!   every intact pane the segment crosses shatters;
//! - nothing to do for plain shots: [`ShotFired`] is already turned into a `BulletPath` here
//!   (raycast against the world, excluding the shooter, to find where the bullet stops).
//!
//! Panes are axis-aligned boxes: `Glass::half` is the half extent in world axes.

use avian3d::prelude::*;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::state::GameState;
use crate::weapon::ShotFired;

/// An intact (or shattered, see [`Shattered`]) window pane. World-axis box, half extents.
#[derive(Component, Clone, Copy, Debug)]
pub struct Glass {
    pub half: Vec3,
}

/// On a pane that is broken this round (hidden).
#[derive(Component, Clone, Copy, Debug)]
pub struct Shattered;

/// Break this pane (no-op if it's already broken or not glass).
#[derive(Message, Clone, Copy, Debug)]
pub struct ShatterGlass(pub Entity);

/// A bullet (or anything) travelling `origin` → `origin + dir * max_dist`: every intact pane
/// it crosses shatters. `dir` should be unit length.
#[derive(Message, Clone, Copy, Debug)]
pub struct BulletPath {
    pub origin: Vec3,
    pub dir: Vec3,
    pub max_dist: f32,
}

/// A burst of small glass shards falling out of a box (a shot-out lamp, `map::lamps`).
#[derive(Message, Clone, Copy, Debug)]
pub struct ShardBurst {
    pub at: Vec3,
    pub half: Vec3,
    pub count: u8,
}

/// Falling shard (despawns when its time is up).
#[derive(Component)]
struct Shard {
    vel: Vec3,
    spin: Vec3,
    life: f32,
}

/// Shared shard mesh/material.
#[derive(Resource, Clone)]
struct ShardAssets {
    mesh: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
}

/// Bullets reach this far (beyond it nothing breaks).
const BULLET_RANGE: f32 = 300.0;

pub struct GlassPlugin;

impl Plugin for GlassPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ShatterGlass>()
            .add_message::<BulletPath>()
            .add_message::<ShardBurst>()
            .add_systems(Update, (paths_from_shots, break_along_paths, shatter, fall).chain())
            .add_systems(OnEnter(GameState::Playing), restore_glass);
        #[cfg(feature = "dev")]
        {
            use engine::devtools::{AddControlCommand, params};
            app.add_control_command("glass.info", "Window panes: total and broken", params(&[]), |world: &mut World, _| {
                let mut q = world.query::<(&Glass, Has<Shattered>)>();
                let (n, broken) = q.iter(world).fold((0, 0), |(n, b), (_, s)| (n + 1, b + s as i32));
                Ok(serde_json::json!({ "panes": n, "broken": broken }))
            });
        }
    }
}

/// Every `ShotFired` becomes a `BulletPath` up to the first solid thing it hits.
fn paths_from_shots(spatial: SpatialQuery, mut shots: MessageReader<ShotFired>, mut paths: MessageWriter<BulletPath>) {
    for s in shots.read() {
        let Ok(dir) = Dir3::new(s.dir) else { continue };
        let filter = SpatialQueryFilter::from_excluded_entities([s.by]);
        let dist = spatial.cast_ray(s.origin, dir, BULLET_RANGE, true, &filter).map_or(BULLET_RANGE, |h| h.distance);
        paths.write(BulletPath { origin: s.origin, dir: *dir, max_dist: dist });
    }
}

/// Slab test: does the segment cross the box?
pub fn segment_hits_box(origin: Vec3, dir: Vec3, max_dist: f32, center: Vec3, half: Vec3) -> bool {
    let (mut t0, mut t1) = (0.0f32, max_dist);
    for k in 0..3 {
        let (o, d, c, h) = (origin[k], dir[k], center[k], half[k]);
        if d.abs() < 1e-8 {
            if (o - c).abs() > h {
                return false;
            }
            continue;
        }
        let (mut a, mut b) = ((c - h - o) / d, (c + h - o) / d);
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return false;
        }
    }
    true
}

fn break_along_paths(mut paths: MessageReader<BulletPath>, panes: Query<(Entity, &GlobalTransform, &Glass), Without<Shattered>>, mut out: MessageWriter<ShatterGlass>) {
    for p in paths.read() {
        for (e, tf, g) in &panes {
            // A little slack so a shot that stops on the frame still breaks the pane.
            if segment_hits_box(p.origin, p.dir, p.max_dist + 0.05, tf.translation(), g.half + Vec3::splat(0.02)) {
                out.write(ShatterGlass(e));
            }
        }
    }
}

fn shatter(
    mut commands: Commands,
    mut reqs: MessageReader<ShatterGlass>,
    mut bursts: MessageReader<ShardBurst>,
    panes: Query<(&GlobalTransform, &Glass), Without<Shattered>>,
    assets: Option<Res<ShardAssets>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mats: Option<ResMut<Assets<StandardMaterial>>>,
) {
    if reqs.is_empty() && bursts.is_empty() {
        return;
    }
    let assets = match (assets, meshes, mats) {
        (Some(a), _, _) => Some(a.clone()),
        (None, Some(mut m), Some(mut s)) => {
            let a = ShardAssets {
                mesh: m.add(Cuboid::new(1.0, 1.0, 1.0)),
                mat: s.add(StandardMaterial {
                    base_color: Color::srgba(0.75, 0.85, 0.9, 0.45),
                    alpha_mode: AlphaMode::Blend,
                    perceptual_roughness: 0.05,
                    reflectance: 0.7,
                    ..default()
                }),
            };
            commands.insert_resource(a.clone());
            Some(a)
        }
        _ => None,
    };
    for b in bursts.read() {
        let Some(a) = &assets else { continue };
        let mut seed = (b.at.x * 92.7 + b.at.z * 13.1 + b.at.y * 7.3).to_bits() | 1;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 10_000) as f32 / 10_000.0
        };
        for _ in 0..b.count {
            let at = b.at + Vec3::new((rnd() * 2.0 - 1.0) * b.half.x, -b.half.y, (rnd() * 2.0 - 1.0) * b.half.z);
            let vel = Vec3::new((rnd() - 0.5) * 2.0, -rnd() * 1.5, (rnd() - 0.5) * 2.0);
            let s = 0.03 + rnd() * 0.06;
            commands.spawn((
                Name::new("Glass shard"),
                Shard { vel, spin: Vec3::new(rnd() * 9.0, rnd() * 9.0, rnd() * 9.0), life: 1.4 + rnd() * 0.8 },
                Mesh3d(a.mesh.clone()),
                MeshMaterial3d(a.mat.clone()),
                Transform::from_translation(at).with_scale(Vec3::new(s, s * (0.6 + rnd()), 0.006)).with_rotation(Quat::from_rotation_y(rnd() * 6.3)),
                NotShadowCaster,
                engine::render_budget::Actor,
            ));
        }
    }
    let mut done = Vec::new();
    for ShatterGlass(e) in reqs.read() {
        if done.contains(e) {
            continue;
        }
        let Ok((tf, g)) = panes.get(*e) else { continue };
        done.push(*e);
        commands.entity(*e).insert((Shattered, Visibility::Hidden));
        let Some(a) = &assets else { continue };
        let c = tf.translation();
        // Normal: the pane's thin axis.
        let n = if g.half.x < g.half.z { Vec3::X } else { Vec3::Z };
        let across = if n == Vec3::X { Vec3::Z } else { Vec3::X };
        let mut seed = (c.x * 92.7 + c.z * 13.1 + c.y * 7.3).to_bits() | 1; // xorshift never leaves 0
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 10_000) as f32 / 10_000.0
        };
        for _ in 0..14 {
            let w = g.half.x.max(g.half.z);
            let at = c + across * (rnd() * 2.0 - 1.0) * w + Vec3::Y * (rnd() * 2.0 - 1.0) * g.half.y;
            let side = if rnd() < 0.5 { -1.0 } else { 1.0 };
            let vel = n * side * (0.6 + rnd() * 1.8) + across * (rnd() - 0.5) * 1.2 + Vec3::Y * rnd() * 0.8;
            let s = 0.05 + rnd() * 0.12;
            commands.spawn((
                Name::new("Glass shard"),
                Shard { vel, spin: Vec3::new(rnd() * 9.0, rnd() * 9.0, rnd() * 9.0), life: 1.6 + rnd() * 0.8 },
                Mesh3d(a.mesh.clone()),
                MeshMaterial3d(a.mat.clone()),
                Transform::from_translation(at).with_scale(Vec3::new(s, s * (0.6 + rnd()), 0.006)).with_rotation(Quat::from_rotation_y(rnd() * 6.3)),
                NotShadowCaster,
                engine::render_budget::Actor,
            ));
        }
    }
}

fn fall(time: Res<Time>, mut commands: Commands, mut q: Query<(Entity, &mut Shard, &mut Transform)>) {
    let dt = time.delta_secs();
    for (e, mut s, mut tf) in &mut q {
        s.life -= dt;
        if s.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        if tf.translation.y > 0.01 {
            s.vel.y -= 9.81 * dt;
            tf.translation += s.vel * dt;
            let spin = s.spin * dt;
            tf.rotate(Quat::from_euler(EulerRot::XYZ, spin.x, spin.y, spin.z));
        } else {
            // Lie on the floor, then fade out by shrinking.
            tf.translation.y = 0.01;
            if s.life < 0.4 {
                tf.scale *= 1.0 - dt * 4.0;
            }
        }
    }
}

fn restore_glass(mut commands: Commands, broken: Query<Entity, With<Shattered>>) {
    for e in &broken {
        commands.entity(e).remove::<Shattered>().insert(Visibility::Inherited);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_box_test() {
        let half = Vec3::new(0.6, 0.6, 0.01);
        assert!(segment_hits_box(Vec3::new(0.0, 1.5, -5.0), Vec3::Z, 10.0, Vec3::new(0.0, 1.5, 0.0), half));
        assert!(!segment_hits_box(Vec3::new(0.0, 1.5, -5.0), Vec3::Z, 4.0, Vec3::new(0.0, 1.5, 0.0), half), "stops short");
        assert!(!segment_hits_box(Vec3::new(2.0, 1.5, -5.0), Vec3::Z, 10.0, Vec3::new(0.0, 1.5, 0.0), half), "misses to the side");
    }
}
