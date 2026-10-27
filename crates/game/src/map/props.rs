//! Props drawn from primitives (no model files needed): parked cars, dumpsters, steel drums,
//! cabinet fronts, goods on shelves. Each kind and size is one shared mesh (vertex-coloured,
//! built with the engine's `MeshKit`, cached by [`Palette::shape`]) plus a few shared
//! materials; the gameplay box (collider) stays the block.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use brain::arena::Block;
use engine::meshkit::MeshKit;

use super::Gfx;
use super::look::Palette;

/// Car paints (base colour × the mesh's vertex colours).
const PAINTS: [[f32; 3]; 6] = [[0.62, 0.08, 0.06], [0.86, 0.86, 0.84], [0.10, 0.16, 0.32], [0.55, 0.57, 0.60], [0.12, 0.30, 0.20], [0.80, 0.62, 0.20]];

/// A hatchback 1.9 m wide, 4.4 m long (along z, nose toward -z), centred on its block: body to
/// 1.1 m (the gameplay box), cabin above it (visual only: you can shoot over the bonnet). Raked
/// windscreen and rear glass, roof, pillars, door seams and handles, mirrors, grille, plates,
/// lamps, bumpers, side skirts and wheels with hubs.
fn car_mesh(h: f32) -> Mesh {
    let (w, l) = (1.86, 4.36);
    let y0 = -h * 0.5;
    let paint = Color::WHITE;
    let dark = Color::srgb(0.03, 0.03, 0.035);
    let trim = Color::srgb(0.08, 0.08, 0.09);
    let glass = Color::srgb(0.05, 0.06, 0.07);
    let chrome = Color::srgb(0.6, 0.6, 0.6);
    let plate = Color::srgb(0.9, 0.9, 0.86);
    let lamp = Color::srgb(3.0, 2.9, 2.6);
    let tail = Color::srgb(1.4, 0.05, 0.03);
    let mut k = MeshKit::new();
    // Wheels (axle along x): tyre, hub, hub cap.
    for (x, z) in [(-1.0, -1.35), (1.0, -1.35), (-1.0, 1.35), (1.0, 1.35)] {
        let tf = Transform::from_xyz(x * (w * 0.5 - 0.14), y0 + 0.33, z).with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        k.add(Cylinder::new(0.33, 0.24), tf, dark);
        k.add(Cylinder::new(0.2, 0.25), tf, chrome);
        k.add(Cylinder::new(0.07, 0.27), tf, trim);
    }
    // Lower body, bonnet / boot deck, dark side skirts between the wheels.
    k.add(Cuboid::new(w, 0.55, l), Transform::from_xyz(0.0, y0 + 0.62, 0.0), paint);
    k.add(Cuboid::new(w - 0.04, 0.16, l - 0.3), Transform::from_xyz(0.0, y0 + 0.97, 0.05), paint);
    k.add(Cuboid::new(w + 0.02, 0.09, 1.7), Transform::from_xyz(0.0, y0 + 0.39, 0.0), trim);
    // Cabin: side glass, roof, B and C pillars, raked windscreen and rear window.
    let (cab0, cab1) = (-0.38f32, 1.04f32);
    let roof_y = y0 + 1.57;
    k.add(Cuboid::new(w - 0.24, 0.46, cab1 - cab0), Transform::from_xyz(0.0, y0 + 1.3, (cab0 + cab1) * 0.5), glass);
    k.add(Cuboid::new(w - 0.18, 0.05, cab1 - cab0 + 0.06), Transform::from_xyz(0.0, roof_y, (cab0 + cab1) * 0.5), paint);
    k.add(Cuboid::new(w - 0.2, 0.46, 0.08), Transform::from_xyz(0.0, y0 + 1.3, 0.34), paint);
    k.add(Cuboid::new(w - 0.2, 0.46, 0.16), Transform::from_xyz(0.0, y0 + 1.3, cab1 - 0.08), paint);
    // A sloped pane from (z0, y_lo) up to (z1, y_hi), framed in paint at the sides.
    let slope = |k: &mut MeshKit, z0: f32, y_lo: f32, z1: f32, y_hi: f32| {
        let (dz, dy) = (z1 - z0, y_hi - y_lo);
        let len = (dz * dz + dy * dy).sqrt();
        let tilt = -dy.atan2(dz);
        let tf = Transform::from_xyz(0.0, (y_lo + y_hi) * 0.5, (z0 + z1) * 0.5).with_rotation(Quat::from_rotation_x(tilt));
        k.add(Cuboid::new(w - 0.22, 0.03, len), tf, glass);
        for x in [-1.0, 1.0] {
            k.add(Cuboid::new(0.06, 0.04, len), tf * Transform::from_xyz(x * (w * 0.5 - 0.12), 0.0, 0.0), paint);
        }
    };
    slope(&mut k, -1.08, y0 + 1.05, cab0, roof_y);
    slope(&mut k, 1.62, y0 + 1.05, cab1, roof_y);
    // Door seams and handles on both sides.
    for z in [-0.42, 0.36, 1.08] {
        k.add(Cuboid::new(w + 0.006, 0.5, 0.012), Transform::from_xyz(0.0, y0 + 0.72, z), trim);
    }
    for z in [-0.18, 0.62] {
        k.add(Cuboid::new(w + 0.024, 0.025, 0.13), Transform::from_xyz(0.0, y0 + 0.86, z), chrome);
    }
    // Mirrors.
    for x in [-1.0, 1.0] {
        k.add(Cuboid::new(0.16, 0.1, 0.08), Transform::from_xyz(x * (w * 0.5 + 0.06), y0 + 1.12, cab0 - 0.08), paint);
    }
    // Bumpers, lamps and plates at both ends; a grille in front.
    for (z, light) in [(-l * 0.5, lamp), (l * 0.5, tail)] {
        let out = z.signum();
        k.add(Cuboid::new(w + 0.02, 0.18, 0.12), Transform::from_xyz(0.0, y0 + 0.42, z), dark);
        k.add(Cuboid::new(0.52, 0.11, 0.02), Transform::from_xyz(0.0, y0 + 0.44, z + out * 0.07), plate);
        for x in [-0.62, 0.62] {
            k.add(Cuboid::new(0.32, 0.1, 0.04), Transform::from_xyz(x, y0 + 0.75, z * 1.004), light);
        }
    }
    k.add(Cuboid::new(0.8, 0.14, 0.03), Transform::from_xyz(0.0, y0 + 0.72, -l * 0.5 - 0.005), trim);
    k.build()
}

pub(crate) fn car(commands: &mut Commands, e: Entity, i: usize, b: &Block, size: Vec3, g: &mut Gfx, p: &mut Palette) {
    let mesh = p.shape(g, &format!("car {:.3}", size.y), || car_mesh(size.y));
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

/// White base for vertex-coloured parts (dumpsters, drum rims, cabinet fronts), cached by key.
fn kit_material(g: &mut Gfx, p: &mut Palette, key: &str, metallic: f32, rough: f32) -> Handle<StandardMaterial> {
    p.flat(g, key, || StandardMaterial { base_color: Color::WHITE, metallic, perceptual_roughness: rough, ..default() })
}

/// A skip / big bin: tapered body, rolled top rim, two lids (one propped open), side ribs, fork
/// pockets and casters, inside the block's box.
fn dumpster_mesh(size: Vec3) -> Mesh {
    let green = Color::srgb(0.10, 0.26, 0.16);
    let ribs = Color::srgb(0.08, 0.21, 0.13);
    let lid = Color::srgb(0.04, 0.04, 0.045);
    let dark = Color::srgb(0.12, 0.12, 0.13);
    let along_x = size.x >= size.z;
    let (long, short) = if along_x { (size.x, size.z) } else { (size.z, size.x) };
    // (along, up, across) → local vector.
    let v = |a: f32, y: f32, c: f32| if along_x { Vec3::new(a, y, c) } else { Vec3::new(c, y, a) };
    let y0 = -size.y * 0.5;
    let h = size.y;
    let mut k = MeshKit::new();
    for a in [-1.0, 1.0] {
        for c in [-1.0, 1.0] {
            k.add(Cuboid::from_size(v(0.1, 0.12, 0.1)), Transform::from_translation(v(a * (long * 0.5 - 0.15), y0 + 0.06, c * (short * 0.5 - 0.15))), dark);
        }
    }
    // Body, narrower at the bottom.
    let half = (h - 0.18) * 0.5;
    k.add(Cuboid::from_size(v(long - 0.1, half, short - 0.2)), Transform::from_translation(v(0.0, y0 + 0.12 + half * 0.5, 0.0)), green);
    k.add(Cuboid::from_size(v(long - 0.02, half, short - 0.06)), Transform::from_translation(v(0.0, y0 + 0.12 + half * 1.5, 0.0)), green);
    k.add(Cuboid::from_size(v(long + 0.04, 0.06, short + 0.02)), Transform::from_translation(v(0.0, y0 + h - 0.03, 0.0)), ribs);
    for side in [-1.0f32, 1.0] {
        for a in [-0.33f32, 0.0, 0.33] {
            k.add(Cuboid::from_size(v(0.06, half - 0.08, 0.04)), Transform::from_translation(v(a * long, y0 + 0.12 + half * 1.5, side * (short * 0.5 - 0.02))), ribs);
        }
        for a in [-0.25f32, 0.25] {
            k.add(Cuboid::from_size(v(0.26, 0.12, 0.08)), Transform::from_translation(v(a * long, y0 + 0.12 + half * 0.75, side * (short * 0.5 - 0.06))), dark);
        }
    }
    // Lids hinged along the back edge (+across): one shut, one propped open.
    for (a, open) in [(-0.25f32, 0.0f32), (0.25, 0.22)] {
        let hinge = v(a * long, y0 + h + 0.02, short * 0.5 + 0.04);
        let rot = if along_x { Quat::from_rotation_x(open) } else { Quat::from_rotation_z(-open) };
        let tf = Transform::from_translation(hinge) * Transform::from_rotation(rot) * Transform::from_translation(v(0.0, 0.0, -(short + 0.08) * 0.5));
        k.add(Cuboid::from_size(v(long * 0.5 - 0.02, 0.04, short + 0.08)), tf, lid);
    }
    k.build()
}

pub(crate) fn dumpster(commands: &mut Commands, e: Entity, size: Vec3, g: &mut Gfx, p: &mut Palette) {
    let mat = kit_material(g, p, "dumpster_kit", 0.45, 0.6);
    let mesh = p.shape(g, &format!("dumpster {:.2} {:.2} {:.2}", size.x, size.y, size.z), || dumpster_mesh(size));
    commands.spawn((Name::new("Dumpster"), Mesh3d(mesh), MeshMaterial3d(mat), Transform::default(), ChildOf(e)));
}

/// Rolling hoops (in the drum's own paint) plus dark rolled rims and a lid with two bungs.
pub(crate) fn barrel(commands: &mut Commands, e: Entity, r: f32, h: f32, paint: &Handle<StandardMaterial>, g: &mut Gfx, p: &mut Palette) {
    let hoops = p.shape(g, &format!("barrel hoops {r:.3} {h:.3}"), || {
        let mut k = MeshKit::new();
        for y in [-h / 6.0, h / 6.0] {
            k.add(Cylinder::new(r + 0.014, 0.045).mesh().resolution(20), Transform::from_xyz(0.0, y, 0.0), Color::WHITE);
        }
        k.build()
    });
    let rims = p.shape(g, &format!("barrel rims {r:.3} {h:.3}"), || {
        let dark = Color::srgb(0.12, 0.13, 0.15);
        let mut k = MeshKit::new();
        for y in [h * 0.5 - 0.02, -h * 0.5 + 0.02] {
            k.add(Cylinder::new(r + 0.01, 0.04).mesh().resolution(20), Transform::from_xyz(0.0, y, 0.0), dark);
        }
        k.add(Cylinder::new(r - 0.03, 0.012).mesh().resolution(20), Transform::from_xyz(0.0, h * 0.5 + 0.004, 0.0), Color::srgb(0.2, 0.21, 0.23));
        for (x, rr) in [(r * 0.55, 0.035), (-r * 0.6, 0.022)] {
            k.add(Cylinder::new(rr, 0.03), Transform::from_xyz(x, h * 0.5 + 0.012, 0.0), dark);
        }
        k.build()
    });
    let rim_mat = kit_material(g, p, "barrel_rim_kit", 0.6, 0.5);
    commands.spawn((Mesh3d(hoops), MeshMaterial3d(paint.clone()), Transform::default(), NotShadowCaster, ChildOf(e)));
    commands.spawn((Mesh3d(rims), MeshMaterial3d(rim_mat), Transform::default(), NotShadowCaster, ChildOf(e)));
}

/// Fronts on both long faces of a cabinet: tall ones (lockers, fridges, vending machines) get
/// one or two doors with long handles and a vent; low ones (counters) a grid of drawers.
fn cabinet_mesh(size: Vec3) -> Mesh {
    let panel = Color::srgb(0.24, 0.25, 0.27);
    let handle = Color::srgb(0.75, 0.75, 0.74);
    let vent = Color::srgb(0.02, 0.02, 0.025);
    let along_x = size.x >= size.z;
    let (long, short) = if along_x { (size.x, size.z) } else { (size.z, size.x) };
    let v = |a: f32, y: f32, c: f32| if along_x { Vec3::new(a, y, c) } else { Vec3::new(c, y, a) };
    let y0 = -size.y * 0.5;
    let mut k = MeshKit::new();
    for side in [-1.0f32, 1.0] {
        let face = |d: f32| side * (short * 0.5 + d);
        if size.y >= 1.6 {
            let doors = if long >= 1.1 { 2 } else { 1 };
            let w = long / doors as f32;
            for d in 0..doors {
                let a = -long * 0.5 + w * (d as f32 + 0.5);
                k.add(Cuboid::from_size(v(w - 0.05, size.y - 0.22, 0.012)), Transform::from_translation(v(a, y0 + 0.08 + (size.y - 0.22) * 0.5, face(0.006))), panel);
                // Handle by the meeting edge of a pair, or the right edge of a single door.
                let ha = match (doors, d) {
                    (1, _) => long * 0.5 - 0.1,
                    (_, 0) => -0.08,
                    _ => 0.08,
                };
                k.add(Cuboid::from_size(v(0.025, 0.4, 0.03)), Transform::from_translation(v(ha, y0 + size.y * 0.52, face(0.02))), handle);
            }
            k.add(Cuboid::from_size(v(long * 0.6, 0.05, 0.008)), Transform::from_translation(v(0.0, y0 + size.y - 0.08, face(0.004))), vent);
        } else {
            let cols = (long / 0.6).floor().max(1.0) as i32;
            let w = long / cols as f32;
            let rows = 2;
            let rh = (size.y - 0.12) / rows as f32;
            for c in 0..cols {
                for r in 0..rows {
                    let a = -long * 0.5 + w * (c as f32 + 0.5);
                    let y = y0 + 0.1 + rh * (r as f32 + 0.5);
                    k.add(Cuboid::from_size(v(w - 0.04, rh - 0.04, 0.012)), Transform::from_translation(v(a, y, face(0.006))), panel);
                    k.add(Cuboid::from_size(v(0.2, 0.025, 0.03)), Transform::from_translation(v(a, y + rh * 0.25, face(0.02))), handle);
                }
            }
        }
    }
    k.build()
}

pub(crate) fn cabinet_fronts(commands: &mut Commands, e: Entity, size: Vec3, g: &mut Gfx, p: &mut Palette) {
    let mat = kit_material(g, p, "cabinet_kit", 0.5, 0.45);
    let mesh = p.shape(g, &format!("cabinet {:.2} {:.2} {:.2}", size.x, size.y, size.z), || cabinet_mesh(size));
    commands.spawn((Name::new("Cabinet fronts"), Mesh3d(mesh), MeshMaterial3d(mat), Transform::default(), NotShadowCaster, ChildOf(e)));
}

/// Steel shelving (a workbench when waist-high): uprights, boards, a back panel down the
/// middle (a Tall shelf blocks sight, so it must look solid through the middle) and goods on
/// the boards, as one vertex-coloured mesh per size. The block's collider is the gameplay shape.
pub(crate) fn shelf(commands: &mut Commands, e: Entity, size: Vec3, g: &mut Gfx, p: &mut Palette, detail: bool) {
    let white = p.flat(g, "goods", || StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.7, metallic: 0.1, ..default() });
    let mesh = p.shape(g, &format!("shelf {:.2} {:.2} {:.2} {detail}", size.x, size.y, size.z), || shelf_mesh(size, detail));
    commands.spawn((Name::new("Shelf"), Mesh3d(mesh), MeshMaterial3d(white), ChildOf(e)));
}

fn shelf_mesh(size: Vec3, detail: bool) -> Mesh {
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
    k.build()
}
