//! Builds the "Foundry" arena from `brain::arena::Arena::foundry()`.
//!
//! Ground plane mapping: `V2 { x, y }` ↔ world `(x, h, y)`.
//! Lighting, sky and post-processing live in [`crate::gfx`].

use avian3d::prelude::*;
use bevy::prelude::*;
use brain::arena::{ARENA_HALF, Arena, BlockKind};
use brain::math::V2;

use crate::assets::AssetsDir;
use crate::gfx::{PendingMips, load_pbr, world_uv_cuboid};
use crate::state::Headless;

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ArenaRes(Arena::foundry()))
            .add_systems(Startup, spawn_arena);
    }
}

/// The arena layout shared with the AI (`brain`).
#[derive(Resource)]
pub struct ArenaRes(pub Arena);

/// Marker on every arena piece (floor and blocks).
#[derive(Component)]
pub struct ArenaPiece;

/// Which block of `ArenaRes.0.blocks` this entity is.
#[derive(Component, Clone, Copy, Debug)]
pub struct BlockIndex(pub usize);

/// Ground-plane point → world position at height `h`.
pub fn to_world(p: V2, h: f32) -> Vec3 {
    Vec3::new(p.x, h, p.y)
}

/// World position → ground-plane point (drops height).
pub fn to_ground(p: Vec3) -> V2 {
    V2::new(p.x, p.z)
}

/// Texture repeat size in metres for [floor, wall, tall, low].
const TILE: [f32; 4] = [4.0, 3.0, 2.0, 1.5];

fn spawn_arena(
    mut commands: Commands,
    arena: Res<ArenaRes>,
    headless: Res<Headless>,
    assets_dir: Res<AssetsDir>,
    server: Option<Res<AssetServer>>,
    pending: Option<ResMut<PendingMips>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    // Visuals only when the renderer's asset types exist.
    let mut vis = match (headless.0, meshes, materials, server, pending) {
        (false, Some(m), Some(s), Some(a), Some(p)) => Some((m, s, a, p)),
        _ => None,
    };

    // Textured PBR when `assets/textures/*` exists, flat colours otherwise.
    let mats = vis.as_mut().map(|(_, mats, server, pending)| {
        let mut mk = |name: &str, tint: Color, fallback: StandardMaterial| {
            let m = load_pbr(server, &assets_dir, pending, name, tint).unwrap_or(fallback);
            mats.add(m)
        };
        let flat = |c: Color, rough: f32| StandardMaterial {
            base_color: c,
            perceptual_roughness: rough,
            ..default()
        };
        [
            // Floor: weathered concrete.
            mk("floor", Color::srgb(0.62, 0.62, 0.62), flat(Color::srgb(0.32, 0.33, 0.34), 0.95)),
            // Wall: cool dark concrete framing the arena.
            mk("wall", Color::srgb(0.42, 0.45, 0.50), flat(Color::srgb(0.22, 0.23, 0.26), 0.9)),
            // Tall: teal painted steel with rust.
            mk("tall", Color::WHITE, flat(Color::srgb(0.48, 0.30, 0.20), 0.6)),
            // Low: red painted cover, reads as "crouch here".
            mk("low", Color::srgb(0.95, 0.9, 0.85), flat(Color::srgb(0.70, 0.58, 0.25), 0.8)),
        ]
    });

    // Floor: a thin static slab whose top is at y = 0.
    let floor_size = ARENA_HALF * 2.0 + 8.0;
    let floor_center = Vec3::new(0.0, -0.5, 0.0);
    let mut floor = commands.spawn((
        Name::new("Floor"),
        ArenaPiece,
        RigidBody::Static,
        Collider::cuboid(floor_size, 1.0, floor_size),
        Transform::from_translation(floor_center),
    ));
    if let (Some((meshes, ..)), Some(mats)) = (vis.as_mut(), mats.as_ref()) {
        let size = Vec3::new(floor_size, 1.0, floor_size);
        floor.insert((
            Mesh3d(meshes.add(world_uv_cuboid(size, floor_center, TILE[0]))),
            MeshMaterial3d(mats[0].clone()),
        ));
    }

    for (i, b) in arena.0.blocks.iter().enumerate() {
        let size = Vec3::new(b.half.x * 2.0, b.height, b.half.y * 2.0);
        let (kind_name, k) = match b.kind {
            BlockKind::Wall => ("Wall", 1),
            BlockKind::Tall => ("Tall", 2),
            BlockKind::Low => ("Low", 3),
        };
        let center = to_world(b.center, b.height * 0.5);
        let mut e = commands.spawn((
            Name::new(format!("{kind_name} block {i}")),
            ArenaPiece,
            BlockIndex(i),
            RigidBody::Static,
            Collider::cuboid(size.x, size.y, size.z),
            Transform::from_translation(center),
        ));
        if let (Some((meshes, ..)), Some(mats)) = (vis.as_mut(), mats.as_ref()) {
            e.insert((
                Mesh3d(meshes.add(world_uv_cuboid(size, center, TILE[k]))),
                MeshMaterial3d(mats[k].clone()),
            ));
        }
    }
}
