//! Visual-only set dressing from `Arena::decor`: floor surface patches, lamps (fixtures plus
//! lights) and door frames. Nothing here has a collider or matters to the AI.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use brain::arena::{Arena, FloorSkin, MapId};

use super::look::{MapLamp, Palette};
use super::{ArenaPiece, Gfx};
use crate::gfx::world_uv_cuboid;
use engine::settings::GraphicsQuality;

/// Floor patches sit this far above the ground slab (no coplanar faces with it).
const PATCH_THICKNESS: f32 = 0.012;

pub(crate) fn spawn(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette) {
    floors(commands, arena, g, p);
    lamps(commands, arena, g, p);
    if g.quality != GraphicsQuality::Low && arena.id == MapId::Office {
        door_frames(commands, arena, g, p);
    }
}

fn floors(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette) {
    for (i, f) in arena.decor.floors.iter().enumerate() {
        let key = match f.skin {
            FloorSkin::Asphalt => "asphalt",
            FloorSkin::Tiles => "tiles",
            FloorSkin::Pavement => "pavement",
        };
        let (mat, tile) = p.mat(g, key);
        let size = Vec3::new(f.max.x - f.min.x, PATCH_THICKNESS, f.max.y - f.min.y);
        let center = Vec3::new((f.min.x + f.max.x) * 0.5, PATCH_THICKNESS * 0.5, (f.min.y + f.max.y) * 0.5);
        commands.spawn((
            Name::new(format!("Floor patch {i}")),
            ArenaPiece,
            Mesh3d(g.meshes.add(world_uv_cuboid(size, center, tile))),
            MeshMaterial3d(mat),
            Transform::from_translation(center),
            NotShadowCaster,
        ));
    }
}

fn lamps(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette) {
    let cube = p.cube(g);
    let housing = p.flat(g, "lamp_housing", || StandardMaterial {
        base_color: Color::srgb(0.82, 0.82, 0.80),
        perceptual_roughness: 0.4,
        metallic: 0.3,
        ..default()
    });
    for (rank, l) in arena.decor.lamps.iter().enumerate() {
        let top = l.height + 0.02;
        let at = |y: f32| Vec3::new(l.pos.x, y, l.pos.y);
        match arena.id {
            // Recessed ceiling panel + a downward spot.
            MapId::Office => {
                let panel = p.glow(g, "lamp_panel", LinearRgba::new(7.0, 6.8, 6.3, 0.0));
                commands.spawn((
                    Name::new(format!("Lamp {rank}")),
                    ArenaPiece,
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(housing.clone()),
                    Transform::from_translation(at(top - 0.015)).with_scale(Vec3::new(1.26, 0.03, 0.66)),
                    NotShadowCaster,
                ));
                commands.spawn((
                    ArenaPiece,
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(panel),
                    Transform::from_translation(at(top - 0.02)).with_scale(Vec3::new(1.2, 0.04, 0.6)),
                    NotShadowCaster,
                ));
                commands.spawn((
                    Name::new(format!("Lamp light {rank}")),
                    ArenaPiece,
                    MapLamp { rank },
                    SpotLight {
                        color: Color::srgb(1.0, 0.95, 0.88),
                        intensity: 200_000.0,
                        range: 14.0,
                        radius: 0.3,
                        shadow_maps_enabled: false,
                        outer_angle: 1.2,
                        inner_angle: 0.55,
                        ..default()
                    },
                    Transform::from_translation(at(top - 0.08)).looking_to(Vec3::NEG_Y, Vec3::X),
                ));
            }
            // Caged bulkhead lamp under a roof + a warm point light.
            _ => {
                let bulb = p.glow(g, "lamp_bulb", LinearRgba::new(9.0, 6.0, 2.6, 0.0));
                let dark = p.flat(g, "lamp_cage", || StandardMaterial {
                    base_color: Color::srgb(0.08, 0.08, 0.09),
                    perceptual_roughness: 0.5,
                    metallic: 0.7,
                    ..default()
                });
                commands.spawn((
                    Name::new(format!("Lamp {rank}")),
                    ArenaPiece,
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(dark),
                    Transform::from_translation(at(top + 0.06)).with_scale(Vec3::new(0.5, 0.12, 0.26)),
                    NotShadowCaster,
                ));
                commands.spawn((
                    ArenaPiece,
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(bulb),
                    Transform::from_translation(at(top - 0.025)).with_scale(Vec3::new(0.4, 0.05, 0.16)),
                    NotShadowCaster,
                ));
                commands.spawn((
                    Name::new(format!("Lamp light {rank}")),
                    ArenaPiece,
                    MapLamp { rank },
                    PointLight {
                        color: Color::srgb(1.0, 0.78, 0.5),
                        intensity: 1_500_000.0,
                        range: 12.0,
                        radius: 0.1,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(at(top - 0.2)),
                ));
            }
        }
    }
}

/// Jambs and a head piece around each doorway, proud of the wall so nothing is coplanar.
fn door_frames(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette) {
    let cube = p.cube(g);
    let wood = p.flat(g, "door_frame", || StandardMaterial {
        base_color: Color::srgb(0.30, 0.31, 0.33),
        perceptual_roughness: 0.45,
        ..default()
    });
    for h in &arena.decor.headers {
        let (min, max) = (h.min, h.max);
        let along_x = max.x - min.x > max.y - min.y;
        // (along-axis span, across-axis span) → world box.
        let mut put = |a0: f32, a1: f32, c0: f32, c1: f32, y0: f32, y1: f32| {
            let (x0, x1, z0, z1) = if along_x { (a0, a1, c0, c1) } else { (c0, c1, a0, a1) };
            commands.spawn((
                ArenaPiece,
                Mesh3d(cube.clone()),
                MeshMaterial3d(wood.clone()),
                Transform::from_xyz((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5).with_scale(Vec3::new(x1 - x0, y1 - y0, z1 - z0)),
                NotShadowCaster,
            ));
        };
        let (a, b, c0, c1) = if along_x { (min.x, max.x, min.y, max.y) } else { (min.y, max.y, min.x, max.x) };
        put(a - 0.06, a + 0.02, c0 - 0.02, c1 + 0.02, 0.0, h.bottom);
        put(b - 0.02, b + 0.06, c0 - 0.02, c1 + 0.02, 0.0, h.bottom);
        put(a, b, c0 - 0.025, c1 + 0.025, h.bottom - 0.06, h.bottom);
    }
}
