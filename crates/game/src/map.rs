//! Builds the "Foundry" arena from `brain::arena::Arena::foundry()`.
//!
//! Ground plane mapping: `V2 { x, y }` ↔ world `(x, h, y)`.

use avian3d::prelude::*;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use brain::arena::{ARENA_HALF, Arena, BlockKind};
use brain::math::V2;

use crate::state::Headless;

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ArenaRes(Arena::foundry()))
            .add_systems(Startup, spawn_arena)
            .add_systems(Update, add_fog_to_cameras);
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

pub const SKY_COLOR: Color = Color::srgb(0.62, 0.68, 0.75);

/// Ground-plane point → world position at height `h`.
pub fn to_world(p: V2, h: f32) -> Vec3 {
    Vec3::new(p.x, h, p.y)
}

/// World position → ground-plane point (drops height).
pub fn to_ground(p: Vec3) -> V2 {
    V2::new(p.x, p.z)
}

fn spawn_arena(
    mut commands: Commands,
    arena: Res<ArenaRes>,
    headless: Res<Headless>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    // Visuals only when the renderer's asset types exist.
    let mut vis = match (headless.0, meshes, materials) {
        (false, Some(m), Some(s)) => Some((m, s)),
        _ => None,
    };

    // Floor: a thin static slab whose top is at y = 0.
    let floor_size = ARENA_HALF * 2.0 + 8.0;
    let mut floor = commands.spawn((
        Name::new("Floor"),
        ArenaPiece,
        RigidBody::Static,
        Collider::cuboid(floor_size, 1.0, floor_size),
        Transform::from_xyz(0.0, -0.5, 0.0),
    ));
    if let Some((meshes, mats)) = vis.as_mut() {
        floor.insert((
            Mesh3d(meshes.add(Cuboid::new(floor_size, 1.0, floor_size))),
            MeshMaterial3d(mats.add(StandardMaterial {
                base_color: Color::srgb(0.32, 0.33, 0.34),
                perceptual_roughness: 0.95,
                ..default()
            })),
        ));
    }

    let block_mats = vis.as_mut().map(|(_, mats)| {
        [
            // Wall: dark concrete.
            mats.add(StandardMaterial {
                base_color: Color::srgb(0.22, 0.23, 0.26),
                perceptual_roughness: 0.9,
                ..default()
            }),
            // Tall: rusty foundry steel.
            mats.add(StandardMaterial {
                base_color: Color::srgb(0.48, 0.30, 0.20),
                perceptual_roughness: 0.6,
                metallic: 0.3,
                ..default()
            }),
            // Low: yellow-ish crates/cover, easy to read as "crouch here".
            mats.add(StandardMaterial {
                base_color: Color::srgb(0.70, 0.58, 0.25),
                perceptual_roughness: 0.8,
                ..default()
            }),
        ]
    });

    for (i, b) in arena.0.blocks.iter().enumerate() {
        let size = Vec3::new(b.half.x * 2.0, b.height, b.half.y * 2.0);
        let kind_name = match b.kind {
            BlockKind::Wall => "Wall",
            BlockKind::Tall => "Tall",
            BlockKind::Low => "Low",
        };
        let mut e = commands.spawn((
            Name::new(format!("{kind_name} block {i}")),
            ArenaPiece,
            BlockIndex(i),
            RigidBody::Static,
            Collider::cuboid(size.x, size.y, size.z),
            Transform::from_translation(to_world(b.center, b.height * 0.5)),
        ));
        if let (Some((meshes, _)), Some(mats)) = (vis.as_mut(), block_mats.as_ref()) {
            let mat = match b.kind {
                BlockKind::Wall => mats[0].clone(),
                BlockKind::Tall => mats[1].clone(),
                BlockKind::Low => mats[2].clone(),
            };
            e.insert((Mesh3d(meshes.add(Cuboid::from_size(size))), MeshMaterial3d(mat)));
        }
    }

    if vis.is_some() {
        commands.spawn((
            Name::new("Sun"),
            DirectionalLight {
                illuminance: 9_000.0,
                shadow_maps_enabled: true,
                ..default()
            },
            Transform::from_xyz(20.0, 40.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        commands.insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.75, 0.8, 0.9),
            brightness: 400.0,
            ..default()
        });
        commands.insert_resource(ClearColor(SKY_COLOR));
    }
}

/// Light distance fog on every 3D camera (whoever spawns them).
fn add_fog_to_cameras(
    mut commands: Commands,
    headless: Res<Headless>,
    cams: Query<Entity, (With<Camera3d>, Without<DistanceFog>)>,
) {
    if headless.0 {
        return;
    }
    for e in &cams {
        commands.entity(e).insert(DistanceFog {
            color: SKY_COLOR,
            falloff: FogFalloff::Linear {
                start: 35.0,
                end: 140.0,
            },
            ..default()
        });
    }
}
