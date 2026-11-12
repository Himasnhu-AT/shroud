//! Visual set dressing from `Arena::decor`: floor patches, lamps (fixtures plus lights),
//! windows (lintels, frames, breakable glass), door frames, and per-building trims (interior
//! linings, skirting, plinths, parapets or eaves) plus a baked reflection probe per building.
//! Only window lintels have colliders (they are wall); nothing here matters to the AI.
//!
//! Perf: small static pieces that share a material are merged into one mesh per building
//! (trims) or per map (frames) with [`Batch`]; big occluders stay separate entities.

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use brain::arena::{Arena, BlockKind, BuildingInfo, FloorSkin, LampStyle, MapId, Paint, SignMount, Window};
use brain::building::{EXTERIOR_HALF, SILL_HEIGHT};
use brain::math::V2;
use engine::look::ReflectionProbe;
use engine::meshkit::MeshKit;
use engine::settings::GraphicsQuality;

use super::glass::Glass;
use super::lamps::{LampGlow, LampLight};
use super::look::{MapLamp, Palette, fog_end};
use super::{ArenaPiece, Gfx};
use crate::gfx::world_uv_cuboid;

/// Floor patches sit this far above the ground slab (no coplanar faces with it).
const PATCH_THICKNESS: f32 = 0.012;

/// `walls`: small wall pieces already merged by the caller (sills, lintels); window lintels
/// are added to it here.
pub(crate) fn spawn(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette, mut walls: Batch) {
    floors(commands, arena, g, p);
    paint_and_signs(commands, arena, g, p);
    lamps(commands, arena, g, p);
    let detail = g.quality != GraphicsQuality::Low;
    let mut frames = Batch::default();
    windows(commands, arena, g, p, &mut frames, &mut walls, detail);
    walls.spawn(commands, g, p, "Wall pieces", true);
    if detail && matches!(arena.id, MapId::Office | MapId::Township | MapId::Sirocco | MapId::Gantry | MapId::Foundry) {
        door_frames(arena, g, p, &mut frames);
    }
    frames.spawn(commands, g, p, "Frames", false);
    for (bi, b) in arena.decor.buildings.iter().enumerate() {
        building_trims(commands, arena, bi, b, g, p, detail);
    }
}

// ---------------------------------------------------------------- merged meshes

/// Boxes merged into one mesh per material key (world-space UVs, so textures line up).
#[derive(Default)]
pub(crate) struct Batch {
    kits: HashMap<String, MeshKit>,
    order: Vec<String>,
}

impl Batch {
    /// A box of `size` centred at world `center`, in material `key` (a [`Palette::mat`] key,
    /// or a flat material registered with that key beforehand).
    pub fn add(&mut self, g: &mut Gfx, p: &mut Palette, key: &str, size: Vec3, center: Vec3) {
        if size.min_element() <= 0.0 {
            return;
        }
        let (_, tile) = p.mat(g, key);
        let kit = self.kits.entry(key.to_string()).or_insert_with(|| {
            self.order.push(key.to_string());
            MeshKit::new()
        });
        kit.add_mesh(&world_uv_cuboid(size, center, tile), Transform::from_translation(center), Color::WHITE);
    }

    /// Any shape (UVs as meshed) placed by `tf`, in material `key`.
    pub fn add_shape(&mut self, key: &str, shape: impl Into<Mesh>, tf: Transform) {
        let kit = self.kits.entry(key.to_string()).or_insert_with(|| {
            self.order.push(key.to_string());
            MeshKit::new()
        });
        kit.add(shape, tf, Color::WHITE);
    }

    /// One entity per material. `shadows`: whether the merged pieces cast shadows.
    pub fn spawn(self, commands: &mut Commands, g: &mut Gfx, p: &mut Palette, name: &str, shadows: bool) {
        self.spawn_with(commands, g, p, name, shadows, false);
    }

    /// [`Batch::spawn`] for small pieces scattered over the whole map (pipes, debris): the merged
    /// mesh's centre is far from most views, so it keeps full detail (`KeepDetail`) instead of
    /// the render budget's flat far version.
    pub fn spawn_scattered(self, commands: &mut Commands, g: &mut Gfx, p: &mut Palette, name: &str, shadows: bool) {
        self.spawn_with(commands, g, p, name, shadows, true);
    }

    fn spawn_with(self, commands: &mut Commands, g: &mut Gfx, p: &mut Palette, name: &str, shadows: bool, keep: bool) {
        let Batch { mut kits, order } = self;
        for key in order {
            let Some(kit) = kits.remove(&key) else { continue };
            let mut mesh = kit.build();
            let _ = mesh.generate_tangents();
            let (mat, _) = p.mat(g, &key);
            let mut e = commands.spawn((Name::new(format!("{name} ({key})")), ArenaPiece, Mesh3d(g.meshes.add(mesh)), MeshMaterial3d(mat), Transform::default()));
            if !shadows {
                e.insert(NotShadowCaster);
            }
            if keep {
                e.insert(engine::render_budget::KeepDetail);
            }
        }
    }
}

// ---------------------------------------------------------------- floors

pub(crate) fn floor_key(f: FloorSkin) -> &'static str {
    match f {
        FloorSkin::Gravel => "gravel",
        FloorSkin::Asphalt => "asphalt",
        FloorSkin::Tiles => "tiles",
        FloorSkin::Pavement => "pavement",
        FloorSkin::Sidewalk => "sidewalk",
        FloorSkin::Wood => "wood_floor",
        FloorSkin::Concrete => "concrete_floor",
        FloorSkin::Carpet => "carpet",
    }
}

fn floors(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette) {
    for (i, f) in arena.decor.floors.iter().enumerate() {
        let (mat, tile) = p.mat(g, &p.floor_patch_key(f.skin));
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

// ---------------------------------------------------------------- paint and signs

/// Floor paint sits on the floor patches; sign strokes sit on the paint.
const PAINT_Y: (f32, f32) = (PATCH_THICKNESS + 0.0006, PATCH_THICKNESS + 0.003);
const FLOOR_SIGN_Y: (f32, f32) = (PATCH_THICKNESS + 0.0036, PATCH_THICKNESS + 0.006);

fn paint_key(paint: Paint) -> &'static str {
    match paint {
        Paint::White => "paint_white",
        Paint::Yellow => "paint_yellow",
        Paint::Red => "paint_red",
        Paint::Blue => "paint_blue",
        Paint::Orange => "paint_orange",
        Paint::Black => "paint_black",
        Paint::Steel => "paint_steel",
    }
}

fn register_paints(g: &mut Gfx, p: &mut Palette) {
    let flat = |c: [f32; 3], rough: f32, metal: f32| StandardMaterial { base_color: Color::srgb(c[0], c[1], c[2]), perceptual_roughness: rough, metallic: metal, ..default() };
    for (paint, c, rough, metal) in [
        (Paint::White, [0.86, 0.85, 0.82], 0.75, 0.0),
        (Paint::Yellow, [0.90, 0.68, 0.10], 0.7, 0.0),
        (Paint::Red, [0.66, 0.12, 0.08], 0.7, 0.0),
        (Paint::Blue, [0.10, 0.26, 0.58], 0.7, 0.0),
        (Paint::Orange, [0.92, 0.42, 0.08], 0.7, 0.0),
        (Paint::Black, [0.04, 0.04, 0.045], 0.7, 0.0),
        (Paint::Steel, [0.30, 0.29, 0.28], 0.35, 0.9),
    ] {
        p.flat(g, paint_key(paint), || flat(c, rough, metal));
    }
}

/// Strokes of a glyph in a cell 1 high, `(width, [(x0, y0, x1, y1)])`. Unknown characters
/// draw nothing (a space).
fn glyph(c: char) -> (f32, &'static [(f32, f32, f32, f32)]) {
    match c {
        'A' => (0.66, &[(0.0, 0.0, 0.33, 1.0), (0.33, 1.0, 0.66, 0.0), (0.14, 0.38, 0.52, 0.38)]),
        'B' => (
            0.6,
            &[
                (0.0, 0.0, 0.0, 1.0),
                (0.0, 1.0, 0.42, 1.0),
                (0.42, 1.0, 0.54, 0.88),
                (0.54, 0.88, 0.54, 0.62),
                (0.54, 0.62, 0.42, 0.5),
                (0.0, 0.5, 0.46, 0.5),
                (0.46, 0.5, 0.6, 0.38),
                (0.6, 0.38, 0.6, 0.12),
                (0.6, 0.12, 0.48, 0.0),
                (0.48, 0.0, 0.0, 0.0),
            ],
        ),
        'D' => (0.6, &[(0.0, 0.0, 0.0, 1.0), (0.0, 1.0, 0.38, 1.0), (0.38, 1.0, 0.6, 0.75), (0.6, 0.75, 0.6, 0.25), (0.6, 0.25, 0.38, 0.0), (0.38, 0.0, 0.0, 0.0)]),
        'I' => (0.4, &[(0.2, 0.0, 0.2, 1.0), (0.0, 1.0, 0.4, 1.0), (0.0, 0.0, 0.4, 0.0)]),
        'M' => (0.7, &[(0.0, 0.0, 0.0, 1.0), (0.0, 1.0, 0.35, 0.45), (0.35, 0.45, 0.7, 1.0), (0.7, 1.0, 0.7, 0.0)]),
        'T' => (0.6, &[(0.0, 1.0, 0.6, 1.0), (0.3, 1.0, 0.3, 0.0)]),
        '>' => (0.9, &[(0.0, 0.5, 0.86, 0.5), (0.9, 0.5, 0.52, 0.86), (0.9, 0.5, 0.52, 0.14)]),
        '<' => (0.9, &[(0.04, 0.5, 0.9, 0.5), (0.0, 0.5, 0.38, 0.86), (0.0, 0.5, 0.38, 0.14)]),
        '^' => (0.7, &[(0.35, 0.0, 0.35, 0.96), (0.35, 1.0, 0.0, 0.62), (0.35, 1.0, 0.7, 0.62)]),
        _ => (0.5, &[]),
    }
}

/// Painted floor markings and signs: merged into one mesh per paint colour, no shadows.
fn paint_and_signs(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette) {
    if arena.decor.paint.is_empty() && arena.decor.signs.is_empty() {
        return;
    }
    register_paints(g, p);
    let mut batch = Batch::default();
    for patch in &arena.decor.paint {
        let (y0, y1) = PAINT_Y;
        let size = Vec3::new(patch.max.x - patch.min.x, y1 - y0, patch.max.y - patch.min.y);
        let at = Vec3::new((patch.min.x + patch.max.x) * 0.5, (y0 + y1) * 0.5, (patch.min.y + patch.max.y) * 0.5);
        batch.add_shape(paint_key(patch.paint), Cuboid::from_length(1.0), Transform::from_translation(at).with_scale(size));
    }
    for s in &arena.decor.signs {
        // Text frame: origin at the text centre, `right` along the text, `up` along the letters,
        // `normal` out of the surface; `lift(d)` is a point d off the surface.
        let (origin, right, up, normal, plate_d, stroke_d) = match s.mount {
            SignMount::Wall { facing_x, facing_z } => {
                let n = Vec3::new(facing_x as f32, 0.0, facing_z as f32);
                (Vec3::new(s.pos.x, s.y, s.pos.y), Vec3::new(n.z, 0.0, -n.x), Vec3::Y, n, (0.006, 0.016), if s.plate.is_some() { (0.018, 0.026) } else { (0.006, 0.014) })
            }
            SignMount::Floor { up_x, up_z } => {
                let u = Vec3::new(up_x as f32, 0.0, up_z as f32);
                (Vec3::new(s.pos.x, 0.0, s.pos.y), Vec3::new(-u.z, 0.0, u.x), u, Vec3::Y, (0.0, 0.0), FLOOR_SIGN_Y)
            }
        };
        let h = s.size;
        let gap = 0.18;
        let glyphs: Vec<_> = s.text.chars().map(glyph).collect();
        let width: f32 = glyphs.iter().map(|(w, _)| w).sum::<f32>() + gap * glyphs.len().saturating_sub(1) as f32;
        let thick = 0.15;
        if let Some(plate) = s.plate {
            let (d0, d1) = plate_d;
            let (pw, ph) = ((width + 0.7) * h, h * 1.5);
            let rot = Quat::from_mat3(&Mat3::from_cols(right, up, normal));
            let at = origin + normal * ((d0 + d1) * 0.5);
            batch.add_shape(paint_key(plate), Cuboid::from_length(1.0), Transform { translation: at, rotation: rot, scale: Vec3::new(pw, ph, d1 - d0) });
        }
        let (d0, d1) = stroke_d;
        let mut x = -width * 0.5;
        for (w, strokes) in glyphs {
            for &(x0, y0, x1, y1) in strokes {
                let a = right * ((x + x0) * h) + up * ((y0 - 0.5) * h);
                let b = right * ((x + x1) * h) + up * ((y1 - 0.5) * h);
                let along = b - a;
                let len = along.length();
                if len < 1e-4 {
                    continue;
                }
                let dir = along / len;
                let side = normal.cross(dir);
                let rot = Quat::from_mat3(&Mat3::from_cols(dir, side, normal));
                let centre = origin + (a + b) * 0.5 + normal * ((d0 + d1) * 0.5);
                batch.add_shape(paint_key(s.paint), Cuboid::from_length(1.0), Transform { translation: centre, rotation: rot, scale: Vec3::new(len + thick * h, thick * h, d1 - d0) });
            }
            x += w + gap;
        }
    }
    batch.spawn(commands, g, p, "Paint", false);
}

// ---------------------------------------------------------------- lamps

fn lamps(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette) {
    p.flat(g, "lamp_housing", || StandardMaterial {
        base_color: Color::srgb(0.82, 0.82, 0.80),
        perceptual_roughness: 0.4,
        metallic: 0.3,
        ..default()
    });
    p.flat(g, "lamp_cage", || StandardMaterial {
        base_color: Color::srgb(0.08, 0.08, 0.09),
        perceptual_roughness: 0.5,
        metallic: 0.7,
        ..default()
    });
    let panel = p.glow(g, "lamp_panel", LinearRgba::new(7.0, 6.8, 6.3, 0.0));
    let bulb = p.glow(g, "lamp_bulb", LinearRgba::new(9.0, 6.0, 2.6, 0.0));
    // A shot-out lamp: dark, cracked-looking glass (`super::lamps`).
    let dead = p.flat(g, "lamp_dead", || StandardMaterial { base_color: Color::srgb(0.10, 0.10, 0.11), perceptual_roughness: 0.35, metallic: 0.1, ..default() });
    let cube = p.cube(g);
    // Housings are merged per material (one mesh each for the whole map); the glowing part of
    // each lamp stays its own entity (it goes dark when the lamp is shot out), as do the lights.
    let mut fx = Batch::default();
    let glow = |commands: &mut Commands, id: usize, lit: &Handle<StandardMaterial>, tf: Transform| {
        commands.spawn((
            Name::new(format!("Lamp glow {id}")),
            ArenaPiece,
            LampGlow { id, lit: lit.clone(), dead: dead.clone() },
            Mesh3d(cube.clone()),
            MeshMaterial3d(lit.clone()),
            tf,
            NotShadowCaster,
        ));
    };
    // Foundry: scanned fixtures (caged bulkheads, enamel high-bays) instead of boxes.
    let scanned = arena.id == MapId::Foundry && g.quality != GraphicsQuality::Low;
    let bulkhead = |commands: &mut Commands, g: &Gfx, at: Vec3, along: Vec3| {
        // Back plate up against what it hangs from, glass facing down, long side along `along`.
        let rot = Quat::from_mat3(&Mat3::from_cols(along.cross(Vec3::NEG_Y), along, Vec3::NEG_Y));
        super::industry::spawn_model(commands, g, &super::industry::WALL_LAMP, Transform::from_translation(at).with_rotation(rot), false).is_some()
    };
    for (rank, l) in arena.decor.lamps.iter().enumerate() {
        let top = l.height + 0.02;
        let at = |y: f32| Vec3::new(l.pos.x, y, l.pos.y);
        match l.style {
            // Recessed ceiling panel + a downward spot.
            LampStyle::Panel => {
                fx.add_shape("lamp_housing", Cuboid::from_length(1.0), Transform::from_translation(at(top - 0.015)).with_scale(Vec3::new(1.26, 0.03, 0.66)));
                glow(commands, rank, &panel, Transform::from_translation(at(top - 0.02)).with_scale(Vec3::new(1.2, 0.04, 0.6)));
                commands.spawn((
                    Name::new(format!("Lamp light {rank}")),
                    ArenaPiece,
                    MapLamp,
                    LampLight { id: rank, power: 200_000.0 },
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
            // Caged bulkhead under a roof + a warm point light.
            LampStyle::Bulkhead if scanned && bulkhead(commands, g, at(top + 0.12), Vec3::X) => {
                glow(commands, rank, &bulb, Transform::from_translation(at(top - 0.03)).with_scale(Vec3::new(0.3, 0.02, 0.16)));
                point(commands, rank, at(top - 0.2), Color::srgb(1.0, 0.78, 0.5), 1_500_000.0, 12.0);
            }
            LampStyle::Bulkhead => {
                fx.add_shape("lamp_cage", Cuboid::from_length(1.0), Transform::from_translation(at(top + 0.06)).with_scale(Vec3::new(0.5, 0.12, 0.26)));
                glow(commands, rank, &bulb, Transform::from_translation(at(top - 0.025)).with_scale(Vec3::new(0.4, 0.05, 0.16)));
                point(commands, rank, at(top - 0.2), Color::srgb(1.0, 0.78, 0.5), 1_500_000.0, 12.0);
            }
            // House pendant: cord, shade, warm bulb.
            LampStyle::Pendant => {
                let drop = 0.7;
                fx.add_shape("lamp_cage", Cuboid::from_length(1.0), Transform::from_translation(at(top - drop * 0.5)).with_scale(Vec3::new(0.015, drop, 0.015)));
                fx.add_shape("lamp_housing", Cylinder::new(0.22, 0.2), Transform::from_translation(at(top - drop - 0.05)));
                glow(commands, rank, &bulb, Transform::from_translation(at(top - drop - 0.16)).with_scale(Vec3::splat(0.09)));
                point(commands, rank, at(top - drop - 0.3), Color::srgb(1.0, 0.8, 0.58), 600_000.0, 10.0);
            }
            // Industrial high-bay under a tall roof.
            LampStyle::HighBay if scanned && super::industry::spawn_model(commands, g, &super::industry::HIGH_BAY, Transform::from_translation(at(top)).with_scale(Vec3::splat(0.75)), false).is_some() => {
                glow(commands, rank, &bulb, Transform::from_translation(at(top - 0.97)).with_scale(Vec3::new(0.3, 0.02, 0.3)));
                point(commands, rank, at(top - 1.15), Color::srgb(1.0, 0.88, 0.7), 3_000_000.0, 18.0);
            }
            LampStyle::HighBay => {
                fx.add_shape("lamp_cage", Cuboid::from_length(1.0), Transform::from_translation(at(top - 0.3)).with_scale(Vec3::new(0.02, 0.6, 0.02)));
                fx.add_shape("lamp_cage", Cylinder::new(0.38, 0.22), Transform::from_translation(at(top - 0.7)));
                glow(commands, rank, &bulb, Transform::from_translation(at(top - 0.82)).with_scale(Vec3::new(0.5, 0.03, 0.5)));
                point(commands, rank, at(top - 1.1), Color::srgb(1.0, 0.88, 0.7), 3_000_000.0, 18.0);
            }
            // Street lamp on a wall bracket.
            LampStyle::Wall { facing_x, facing_z } => {
                let f = Vec3::new(facing_x as f32, 0.0, facing_z as f32);
                let base = at(l.height);
                let arm = base + f * 0.3 + Vec3::Y * 0.1;
                let arm_scale = if facing_x != 0 { Vec3::new(0.6, 0.05, 0.05) } else { Vec3::new(0.05, 0.05, 0.6) };
                fx.add_shape("lamp_cage", Cuboid::from_length(1.0), Transform::from_translation(arm).with_scale(arm_scale));
                if scanned && bulkhead(commands, g, base + f * 0.62 + Vec3::Y * 0.075, f) {
                    glow(commands, rank, &bulb, Transform::from_translation(base + f * 0.62 - Vec3::Y * 0.075).with_scale(Vec3::new(0.16, 0.015, 0.16)));
                } else {
                    fx.add_shape("lamp_cage", Cuboid::from_length(1.0), Transform::from_translation(base + f * 0.62).with_scale(Vec3::new(0.26, 0.2, 0.26)));
                    glow(commands, rank, &bulb, Transform::from_translation(base + f * 0.62 - Vec3::Y * 0.13).with_scale(Vec3::new(0.2, 0.06, 0.2)));
                }
                point(commands, rank, base + f * 0.62 - Vec3::Y * 0.3, Color::srgb(1.0, 0.72, 0.42), 900_000.0, 14.0);
            }
        }
    }
    fx.spawn(commands, g, p, "Lamp fixtures", false);
}

fn point(commands: &mut Commands, rank: usize, pos: Vec3, color: Color, intensity: f32, range: f32) {
    commands.spawn((
        Name::new(format!("Lamp light {rank}")),
        ArenaPiece,
        MapLamp,
        LampLight { id: rank, power: intensity },
        PointLight { color, intensity, range, radius: 0.1, shadow_maps_enabled: false, ..default() },
        Transform::from_translation(pos),
    ));
}

// ---------------------------------------------------------------- windows

/// Which way a wall opening runs, and its outward normal (away from its building) if known.
fn opening_axes(arena: &Arena, min: V2, max: V2) -> (bool, Option<Vec3>) {
    let along_x = max.x - min.x > max.y - min.y;
    let c = (min + max) * 0.5;
    let out = arena.decor.buildings.iter().find(|b| c.x >= b.min.x - 0.01 && c.x <= b.max.x + 0.01 && c.y >= b.min.y - 0.01 && c.y <= b.max.y + 0.01).map(|b| {
        let bc = (b.min + b.max) * 0.5;
        if along_x { Vec3::Z * (c.y - bc.y).signum() } else { Vec3::X * (c.x - bc.x).signum() }
    });
    (along_x, out)
}

fn windows(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette, frames: &mut Batch, walls: &mut Batch, detail: bool) {
    let glass = p.flat(g, "window_glass", || StandardMaterial {
        base_color: Color::srgba(0.55, 0.66, 0.72, 0.22),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.04,
        metallic: 0.0,
        reflectance: 0.6,
        ..default()
    });
    p.flat(g, "window_frame", || StandardMaterial { base_color: Color::srgb(0.88, 0.88, 0.86), perceptual_roughness: 0.45, ..default() });
    p.flat(g, "window_sill", || StandardMaterial { base_color: Color::srgb(0.70, 0.68, 0.64), perceptual_roughness: 0.8, ..default() });
    let cube = p.cube(g);
    let range = VisibilityRange::abrupt(0.0, fog_end(arena.id));
    for (i, w) in arena.decor.windows.iter().enumerate() {
        let (min, max) = (w.min, w.max);
        let (along_x, out) = opening_axes(arena, min, max);
        let c = (min + max) * 0.5;
        // Lintel: the wall above the opening, in the wall's own material. Bullet-solid.
        let sill_block = arena.blocks.iter().position(|b| b.kind == BlockKind::Low && b.center.dist(c) < 1e-3);
        let key = sill_block.map(|k| p.block_key(k, &arena.blocks[k])).unwrap_or_else(|| "concrete".into());
        let size = Vec3::new(max.x - min.x, w.wall_top - w.top, max.y - min.y);
        let center = Vec3::new(c.x, (w.top + w.wall_top) * 0.5, c.y);
        walls.add(g, p, &key, size, center);
        commands.spawn((Name::new(format!("Window lintel {i}")), ArenaPiece, RigidBody::Static, Collider::cuboid(size.x, size.y, size.z), Transform::from_translation(center)));
        window_frame(g, p, frames, w, along_x, out, detail);
        if w.glass {
            let inset = 0.06;
            let (len, depth) = if along_x { (max.x - min.x, max.y - min.y) } else { (max.y - min.y, max.x - min.x) };
            let _ = depth;
            let h = w.top - w.sill - 0.1;
            let half = if along_x { Vec3::new(len * 0.5 - inset, h * 0.5, 0.012) } else { Vec3::new(0.012, h * 0.5, len * 0.5 - inset) };
            commands.spawn((
                Name::new(format!("Glass {i}")),
                ArenaPiece,
                Glass { half },
                Mesh3d(cube.clone()),
                MeshMaterial3d(glass.clone()),
                Transform::from_xyz(c.x, w.sill + 0.05 + h * 0.5, c.y).with_scale(half * 2.0),
                NotShadowCaster,
                range.clone(),
            ));
        }
    }
}

/// Jambs, head, sill bar and a mullion around a window opening; a stone sill outside.
fn window_frame(g: &mut Gfx, p: &mut Palette, frames: &mut Batch, w: &Window, along_x: bool, out: Option<Vec3>, detail: bool) {
    let (a0, a1, c0, c1) = if along_x { (w.min.x, w.max.x, w.min.y, w.max.y) } else { (w.min.y, w.max.y, w.min.x, w.max.x) };
    let proud = 0.02;
    let bar = 0.07;
    let mut put = |frames: &mut Batch, key: &str, a0: f32, a1: f32, c0: f32, c1: f32, y0: f32, y1: f32| {
        let (x0, x1, z0, z1) = if along_x { (a0, a1, c0, c1) } else { (c0, c1, a0, a1) };
        frames.add(g, p, key, Vec3::new(x1 - x0, y1 - y0, z1 - z0), Vec3::new((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5));
    };
    let (d0, d1) = (c0 - proud, c1 + proud);
    put(frames, "window_frame", a0, a0 + bar, d0, d1, w.sill, w.top);
    put(frames, "window_frame", a1 - bar, a1, d0, d1, w.sill, w.top);
    put(frames, "window_frame", a0 + bar, a1 - bar, d0, d1, w.top - bar, w.top);
    put(frames, "window_frame", a0 + bar, a1 - bar, d0, d1, w.sill, w.sill + 0.05);
    if !detail {
        return;
    }
    let mid = (c0 + c1) * 0.5;
    if a1 - a0 > 1.7 {
        let m = (a0 + a1) * 0.5;
        put(frames, "window_frame", m - 0.03, m + 0.03, mid - 0.035, mid + 0.035, w.sill + 0.05, w.top - bar);
    }
    // Transom bar on tall shop windows.
    if a1 - a0 > 2.1 {
        put(frames, "window_frame", a0 + bar, a1 - bar, mid - 0.035, mid + 0.035, w.top - 0.45, w.top - 0.4);
    }
    if let (true, Some(o)) = (w.exterior, out) {
        // Stone sill sticking out of the facade below the frame.
        let o = if along_x { o.z } else { o.x };
        let face = if o > 0.0 { c1 } else { c0 };
        let (s0, s1) = if o > 0.0 { (face - 0.02, face + 0.1) } else { (face - 0.1, face + 0.02) };
        put(frames, "window_sill", a0 - 0.08, a1 + 0.08, s0, s1, w.sill - 0.06, w.sill);
    }
}

/// Jambs and a head piece around each doorway, proud of the wall so nothing is coplanar.
fn door_frames(arena: &Arena, g: &mut Gfx, p: &mut Palette, frames: &mut Batch) {
    p.flat(g, "door_frame", || StandardMaterial { base_color: Color::srgb(0.30, 0.31, 0.33), perceptual_roughness: 0.45, ..default() });
    for h in &arena.decor.headers {
        let (min, max) = (h.min, h.max);
        let along_x = max.x - min.x > max.y - min.y;
        let (a, b, c0, c1) = if along_x { (min.x, max.x, min.y, max.y) } else { (min.y, max.y, min.x, max.x) };
        if c1 - c0 > 0.6 {
            continue; // Depot's thick concrete gateways have no frames.
        }
        let mut put = |a0: f32, a1: f32, c0: f32, c1: f32, y0: f32, y1: f32| {
            let (x0, x1, z0, z1) = if along_x { (a0, a1, c0, c1) } else { (c0, c1, a0, a1) };
            frames.add(g, p, "door_frame", Vec3::new(x1 - x0, y1 - y0, z1 - z0), Vec3::new((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5));
        };
        put(a - 0.06, a + 0.02, c0 - 0.02, c1 + 0.02, 0.0, h.bottom);
        put(b - 0.02, b + 0.06, c0 - 0.02, c1 + 0.02, 0.0, h.bottom);
        put(a - 0.06, b + 0.06, c0 - 0.025, c1 + 0.025, h.bottom - 0.06, h.bottom + 0.04);
    }
}

// ---------------------------------------------------------------- buildings

/// Linings, skirting, plinth, parapet or eaves, and the reflection probe of building `bi`.
fn building_trims(commands: &mut Commands, arena: &Arena, bi: usize, b: &BuildingInfo, g: &mut Gfx, p: &mut Palette, detail: bool) {
    let e = EXTERIOR_HALF;
    let t2 = 2.0 * e;
    let [on, os, ow, oe] = b.open_sides;
    let k = |open: bool| if open { 0.0 } else { t2 };
    let imin = V2::new(b.min.x + k(ow), b.min.y + k(on));
    let imax = V2::new(b.max.x - k(oe), b.max.y - k(os));
    let lining = p.lining_of(b.use_);
    let facade = p.facade_key(bi, b.facade);
    p.flat(g, "skirting", || StandardMaterial { base_color: Color::srgb(0.16, 0.16, 0.17), perceptual_roughness: 0.6, ..default() });
    p.flat(g, "plinth", || StandardMaterial { base_color: Color::srgb(0.24, 0.23, 0.22), perceptual_roughness: 0.9, ..default() });
    p.flat(g, "roof_edge", || StandardMaterial { base_color: Color::srgb(0.20, 0.19, 0.19), perceptual_roughness: 0.8, ..default() });
    p.flat(g, "coping", || StandardMaterial { base_color: Color::srgb(0.66, 0.64, 0.60), perceptual_roughness: 0.85, ..default() });
    let mut inside = Batch::default();
    let mut outside = Batch::default();
    for &i in &b.shell {
        let blk = &arena.blocks[i];
        let (lo, hi) = (blk.center - blk.half, blk.center + blk.half);
        let h = blk.height;
        let sill = blk.kind == BlockKind::Low;
        // Which wall: N/S walls sit at the footprint's z edges.
        let (along_x, inward) = if (blk.center.y - (b.min.y + e)).abs() < 0.01 {
            (true, 1.0)
        } else if (blk.center.y - (b.max.y - e)).abs() < 0.01 {
            (true, -1.0)
        } else if (blk.center.x - (b.min.x + e)).abs() < 0.01 {
            (false, 1.0)
        } else {
            (false, -1.0)
        };
        // Span along the wall, clipped to the interior; faces across it.
        let (a0, a1) = if along_x { (lo.x.max(imin.x), hi.x.min(imax.x)) } else { (lo.y.max(imin.y), hi.y.min(imax.y)) };
        let (f_in, f_out) = match (along_x, inward > 0.0) {
            (true, true) => (hi.y, lo.y),
            (true, false) => (lo.y, hi.y),
            (false, true) => (hi.x, lo.x),
            (false, false) => (lo.x, hi.x),
        };
        let mut slab = |batch: &mut Batch, key: &str, a0: f32, a1: f32, face: f32, dir: f32, thick: f32, y0: f32, y1: f32| {
            let (c0, c1) = if dir > 0.0 { (face, face + thick) } else { (face - thick, face) };
            let (x0, x1, z0, z1) = if along_x { (a0, a1, c0, c1) } else { (c0, c1, a0, a1) };
            batch.add(g, p, key, Vec3::new(x1 - x0, y1 - y0, z1 - z0), Vec3::new((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5));
        };
        // Interior plaster (just proud of the wall), with skirting on full-height walls.
        if a1 - a0 > 0.02 {
            slab(&mut inside, lining, a0, a1, f_in, inward, 0.01, 0.0, if sill { SILL_HEIGHT } else { h - 0.003 });
            if !sill && detail {
                slab(&mut inside, "skirting", a0, a1, f_in, inward, 0.022, 0.0, 0.1);
            }
        }
        // Plinth along the outside of the wall (full block length).
        let (o0, o1) = if along_x { (lo.x, hi.x) } else { (lo.y, hi.y) };
        if detail {
            slab(&mut outside, "plinth", o0, o1, f_out, -inward, 0.03, 0.0, 0.32);
        }
    }
    // Lintels over windows and doorways in the outer walls: interior plaster on them too.
    let bc = (b.min + b.max) * 0.5;
    let in_footprint = |c: V2| c.x >= b.min.x - 0.01 && c.x <= b.max.x + 0.01 && c.y >= b.min.y - 0.01 && c.y <= b.max.y + 0.01;
    let spans: Vec<(V2, V2, f32, f32)> =
        arena.decor.windows.iter().filter(|w| w.exterior).map(|w| (w.min, w.max, w.top, w.wall_top)).chain(arena.decor.headers.iter().map(|h| (h.min, h.max, h.bottom, h.top))).collect();
    for (lo, hi, y0, y1) in spans {
        let c = (lo + hi) * 0.5;
        let along_x = hi.x - lo.x > hi.y - lo.y;
        let thick = if along_x { hi.y - lo.y } else { hi.x - lo.x };
        if !in_footprint(c) || (thick - t2).abs() > 0.01 {
            continue;
        }
        let (x0, x1, z0, z1) = if along_x {
            let face = if bc.y > c.y { hi.y } else { lo.y };
            let (f0, f1) = if bc.y > c.y { (face, face + 0.01) } else { (face - 0.01, face) };
            (lo.x, hi.x, f0, f1)
        } else {
            let face = if bc.x > c.x { hi.x } else { lo.x };
            let (f0, f1) = if bc.x > c.x { (face, face + 0.01) } else { (face - 0.01, face) };
            (f0, f1, lo.y, hi.y)
        };
        let (ya, yb) = (y0, y1 - 0.003);
        inside.add(g, p, lining, Vec3::new(x1 - x0, yb - ya, z1 - z0), Vec3::new((x0 + x1) * 0.5, (ya + yb) * 0.5, (z0 + z1) * 0.5));
    }
    // Roof edge: a parapet with coping (flat roofs) or an overhanging eave (houses, sheds).
    let r = arena.decor.roofs[b.roof];
    let top = r.bottom + r.thickness;
    let (x0, z0, x1, z1) = (b.min.x, b.min.y, b.max.x, b.max.y);
    if b.parapet {
        let (pt, ph) = (0.25, 0.75);
        let y0 = b.height;
        let y1 = top + ph;
        let edges = [
            (!on, Vec3::new(x1 - x0 + 0.04, y1 - y0, pt), Vec3::new((x0 + x1) * 0.5, (y0 + y1) * 0.5, z0 - 0.02 + pt * 0.5)),
            (!os, Vec3::new(x1 - x0 + 0.04, y1 - y0, pt), Vec3::new((x0 + x1) * 0.5, (y0 + y1) * 0.5, z1 + 0.02 - pt * 0.5)),
            (!ow, Vec3::new(pt, y1 - y0, z1 - z0 - 2.0 * pt + 0.04), Vec3::new(x0 - 0.02 + pt * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5)),
            (!oe, Vec3::new(pt, y1 - y0, z1 - z0 - 2.0 * pt + 0.04), Vec3::new(x1 + 0.02 - pt * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5)),
        ];
        for (on, size, c) in edges {
            if on {
                outside.add(g, p, &facade, size, c);
                if detail {
                    outside.add(g, p, "coping", Vec3::new(size.x + 0.08, 0.07, size.z + 0.08), c + Vec3::Y * (size.y * 0.5 + 0.035));
                }
            }
        }
    } else {
        let over = 0.4;
        let grow = |open: bool| if open { 0.0 } else { over };
        let (ex0, ez0, ex1, ez1) = (x0 - grow(ow), z0 - grow(on), x1 + grow(oe), z1 + grow(os));
        let (y0, y1) = (b.height + 0.004, top + 0.08);
        outside.add(g, p, "roof_edge", Vec3::new(ex1 - ex0, y1 - y0, ez1 - ez0), Vec3::new((ex0 + ex1) * 0.5, (y0 + y1) * 0.5, (ez0 + ez1) * 0.5));
    }
    inside.spawn(commands, g, p, &format!("{} interior trims", b.name), false);
    outside.spawn(commands, g, p, &format!("{} exterior trims", b.name), true);
    // A baked reflection probe filling the interior (captures once, then frozen).
    let size = Vec3::new(imax.x - imin.x, b.height, imax.y - imin.y);
    let mut probe = ReflectionProbe::default();
    probe.resolution = 64;
    probe.refresh = Some(0);
    probe.far = 120.0;
    commands.spawn((
        Name::new(format!("{} probe", b.name)),
        ArenaPiece,
        probe,
        Transform::from_xyz((imin.x + imax.x) * 0.5, b.height * 0.5, (imin.y + imax.y) * 0.5).with_scale(size),
    ));
}
