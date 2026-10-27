//! Props drawn from primitives (no model files needed): parked cars, dumpsters, goods on
//! shelves. Each kind is one shared mesh (vertex-coloured, built with the engine's
//! `MeshKit`) plus a few shared paint materials; the gameplay box (collider) stays the block.

use bevy::prelude::*;
use brain::arena::Block;
use engine::meshkit::MeshKit;

use super::Gfx;
use super::look::{Palette, child_box};

/// Car paints (base colour × the mesh's vertex colours).
const PAINTS: [[f32; 3]; 6] = [[0.62, 0.08, 0.06], [0.86, 0.86, 0.84], [0.10, 0.16, 0.32], [0.55, 0.57, 0.60], [0.12, 0.30, 0.20], [0.80, 0.62, 0.20]];

/// A hatchback 1.9 m wide, 4.4 m long (along z), centred on its block: body to 1.1 m (the
/// gameplay box), glasshouse above it (visual only: you can shoot over the bonnet).
fn car_mesh(h: f32) -> Mesh {
    let (w, l) = (1.86, 4.36);
    let y0 = -h * 0.5;
    let paint = Color::WHITE;
    let dark = Color::srgb(0.03, 0.03, 0.035);
    let glass = Color::srgb(0.05, 0.06, 0.07);
    let chrome = Color::srgb(0.6, 0.6, 0.6);
    let lamp = Color::srgb(3.0, 2.9, 2.6);
    let tail = Color::srgb(1.4, 0.05, 0.03);
    let mut k = MeshKit::new();
    // Wheels (axle along x).
    for (x, z) in [(-1.0, -1.35), (1.0, -1.35), (-1.0, 1.35), (1.0, 1.35)] {
        let tf = Transform::from_xyz(x * (w * 0.5 - 0.14), y0 + 0.33, z).with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        k.add(Cylinder::new(0.33, 0.24), tf, dark);
        k.add(Cylinder::new(0.2, 0.25), tf, chrome);
    }
    // Lower body, bonnet, boot.
    k.add(Cuboid::new(w, 0.55, l), Transform::from_xyz(0.0, y0 + 0.62, 0.0), paint);
    k.add(Cuboid::new(w - 0.04, 0.16, l - 0.3), Transform::from_xyz(0.0, y0 + 0.97, 0.05), paint);
    // Glasshouse: pillars in paint, windows dark.
    k.add(Cuboid::new(w - 0.22, 0.5, 2.2), Transform::from_xyz(0.0, y0 + 1.3, 0.25), glass);
    k.add(Cuboid::new(w - 0.18, 0.06, 2.0), Transform::from_xyz(0.0, y0 + 1.57, 0.3), paint);
    for z in [-0.84, 1.34] {
        k.add(Cuboid::new(w - 0.18, 0.5, 0.08), Transform::from_xyz(0.0, y0 + 1.3, z), paint);
    }
    // Bumpers, lamps, wheel arches shadow.
    for (z, light) in [(-l * 0.5, lamp), (l * 0.5, tail)] {
        k.add(Cuboid::new(w + 0.02, 0.18, 0.12), Transform::from_xyz(0.0, y0 + 0.42, z), dark);
        for x in [-0.62, 0.62] {
            k.add(Cuboid::new(0.32, 0.1, 0.04), Transform::from_xyz(x, y0 + 0.75, z * 1.004), light);
        }
    }
    k.build()
}

pub(crate) fn car(commands: &mut Commands, e: Entity, i: usize, b: &Block, size: Vec3, g: &mut Gfx, p: &mut Palette) {
    if p.car.is_none() {
        p.car = Some(g.meshes.add(car_mesh(size.y)));
    }
    let Some(mesh) = p.car.clone() else { return };
    let k = i % PAINTS.len();
    let c = PAINTS[k];
    let mat = p.flat(g, &format!("car_paint{k}"), || StandardMaterial {
        base_color: Color::srgb(c[0], c[1], c[2]),
        perceptual_roughness: 0.3,
        metallic: 0.55,
        reflectance: 0.6,
        ..default()
    });
    let along_x = b.half.x > b.half.y;
    let rot = if along_x { Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) } else { Quat::IDENTITY };
    commands.spawn((Name::new("Car"), Mesh3d(mesh), MeshMaterial3d(mat), Transform::from_rotation(rot), ChildOf(e)));
}

pub(crate) fn dumpster(commands: &mut Commands, e: Entity, size: Vec3, g: &mut Gfx, p: &mut Palette) {
    let body = p.flat(g, "dumpster", || StandardMaterial { base_color: Color::srgb(0.10, 0.26, 0.16), perceptual_roughness: 0.55, metallic: 0.5, ..default() });
    let lid = p.flat(g, "dumpster_lid", || StandardMaterial { base_color: Color::srgb(0.05, 0.05, 0.05), perceptual_roughness: 0.7, ..default() });
    let cube = p.cube(g);
    child_box(commands, e, cube.clone(), body, Vec3::ZERO, size);
    child_box(commands, e, cube, lid, Vec3::Y * (size.y * 0.5 + 0.03), Vec3::new(size.x + 0.06, 0.06, size.z + 0.06));
}

/// Steel shelving (a workbench when waist-high): uprights, boards, a back panel down the
/// middle (a Tall shelf blocks sight, so it must look solid through the middle) and goods on
/// the boards, as one vertex-coloured mesh. The block's collider is the gameplay shape.
pub(crate) fn shelf(commands: &mut Commands, e: Entity, size: Vec3, g: &mut Gfx, p: &mut Palette, detail: bool) {
    let white = p.flat(g, "goods", || StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.7, metallic: 0.1, ..default() });
    let goods = [[0.75, 0.2, 0.15], [0.2, 0.45, 0.7], [0.85, 0.7, 0.2], [0.3, 0.55, 0.3], [0.9, 0.9, 0.85], [0.55, 0.38, 0.22]];
    let steel = Color::srgb(0.22, 0.24, 0.27);
    let board = Color::srgb(0.62, 0.62, 0.64);
    let back = Color::srgb(0.42, 0.44, 0.47);
    let along_x = size.x >= size.z;
    let (long, short) = if along_x { (size.x, size.z) } else { (size.z, size.x) };
    // (along, up, across) → local vector.
    let v = |a: f32, y: f32, c: f32| if along_x { Vec3::new(a, y, c) } else { Vec3::new(c, y, a) };
    let y0 = -size.y * 0.5;
    let mut k = MeshKit::new();
    let bays = (long / 1.2).ceil().max(1.0) as i32;
    for i in 0..=bays {
        let a = -long * 0.5 + 0.03 + (long - 0.06) * i as f32 / bays as f32;
        for c in [-short * 0.5 + 0.03, short * 0.5 - 0.03] {
            k.add(Cuboid::from_size(v(0.05, size.y, 0.05)), Transform::from_translation(v(a, 0.0, c)), steel);
        }
    }
    k.add(Cuboid::from_size(v(long - 0.04, size.y - 0.04, 0.04)), Transform::default(), back);
    let levels: Vec<f32> = if size.y > 1.6 { vec![0.12, size.y * 0.36, size.y * 0.66, size.y - 0.03] } else { vec![0.12, size.y - 0.03] };
    for (li, &ly) in levels.iter().enumerate() {
        k.add(Cuboid::from_size(v(long, 0.03, short)), Transform::from_translation(v(0.0, y0 + ly - 0.015, 0.0)), board);
        // Goods on both faces of the back panel (none on a tall shelf's top board).
        if !detail || (size.y > 1.6 && li == levels.len() - 1) {
            continue;
        }
        let n = (long / 0.42).floor() as i32;
        for side in [-1.0f32, 1.0] {
            for j in 0..n {
                let s = (side > 0.0) as usize;
                if (j as usize * 7 + li * 3 + s).is_multiple_of(5) {
                    continue;
                }
                let a = -long * 0.5 + (j as f32 + 0.5) * long / n as f32;
                let h = 0.16 + 0.14 * (((j as usize + li) * 13 % 5) as f32 / 4.0);
                let depth = short * 0.5 - 0.08;
                let col = goods[(j as usize + li * 2 + s) % goods.len()];
                k.add(Cuboid::from_size(v(0.3, h, depth)), Transform::from_translation(v(a, y0 + ly + h * 0.5, side * (depth * 0.5 + 0.03))), Color::srgb(col[0], col[1], col[2]));
            }
        }
    }
    commands.spawn((Name::new("Shelf"), Mesh3d(g.meshes.add(k.build())), MeshMaterial3d(white), ChildOf(e)));
}
