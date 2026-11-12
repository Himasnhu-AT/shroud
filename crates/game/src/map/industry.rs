//! Foundry's industrial detail, all visual (no colliders, nothing the AI needs to know):
//!
//! - **Pipes and cables** along the tall walls, above head height, on wall brackets, with the odd
//!   downpipe flush to the wall. Merged into one mesh per material ([`Batch`]).
//! - **Wall-side clutter** from Poly Haven scans (drums, cement bags, boxes, jerrycans, tyres,
//!   gas bottles) only in concave corners where a wall meets another block, so nothing new stands
//!   in a lane; **wall boxes** (fuse and utility cabinets) hung flat on walls.
//!
//! Models are optional: without `assets/library/props/ph-*` the map keeps its pipes and plain
//! blocks. Small models cast no shadows. Placement is deterministic.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use brain::arena::{Arena, BlockKind, MapId, Skin};
use engine::render_budget::Scenery;
use engine::settings::GraphicsQuality;

use super::dressing::Batch;
use super::look::Palette;
use super::{ArenaPiece, Gfx};

/// A scanned model: library path, its bounding box in its own frame (metres, from the glTF), and
/// which local axis points away from the wall it stands against (`None`: any yaw).
pub(crate) struct Model {
    pub path: &'static str,
    pub min: Vec3,
    pub max: Vec3,
    pub back: Option<Vec3>,
    /// Draw distance (small clutter is culled past it), `None`: always.
    pub range: Option<std::ops::Range<f32>>,
}

impl Model {
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
    /// Local point at the middle of the bottom face.
    pub fn foot(&self) -> Vec3 {
        Vec3::new((self.min.x + self.max.x) * 0.5, self.min.y, (self.min.z + self.max.z) * 0.5)
    }
}

// Bounding boxes measured from the glTFs (accessor min/max through the node transforms).
pub(crate) const BARREL_RED: Model = Model { path: "library/props/ph-barrel-01/Barrel_01_1k.gltf", min: Vec3::new(-0.282, -0.007, -0.282), max: Vec3::new(0.282, 0.873, 0.282), back: None, range: None };
pub(crate) const BARREL_BLUE: Model = Model { path: "library/props/ph-barrel-03/barrel_03_1k.gltf", min: Vec3::new(-0.317, 0.0, -0.319), max: Vec3::new(0.317, 0.93, 0.319), back: None, range: None };
/// Lies flat.
pub(crate) const CEMENT: Model = Model { path: "library/props/ph-cement-bag/cement_bag_1k.gltf", min: Vec3::new(-0.229, 0.001, -0.348), max: Vec3::new(0.234, 0.181, 0.351), back: None, range: Some(0.0..35.0) };
pub(crate) const BOX: Model = Model { path: "library/props/ph-cardboard-box-01/cardboard_box_01_1k.gltf", min: Vec3::new(-0.193, -0.007, -0.225), max: Vec3::new(0.195, 0.335, 0.291), back: None, range: Some(0.0..35.0) };
pub(crate) const JERRYCAN: Model = Model { path: "library/props/ph-metal-jerrycan/metal_jerrycan_1k.gltf", min: Vec3::new(-0.17, 0.0, -0.089), max: Vec3::new(0.18, 0.455, 0.083), back: None, range: Some(0.0..35.0) };
/// Stands on its tread (axle along z): leaned against the wall.
pub(crate) const TYRE: Model = Model { path: "library/props/ph-old-tyre/old_tyre_1k.gltf", min: Vec3::new(-0.3, -0.3, -0.083), max: Vec3::new(0.3, 0.3, 0.082), back: Some(Vec3::Z), range: Some(0.0..35.0) };
pub(crate) const PROPANE: Model = Model { path: "library/props/ph-propane-tank/propane_tank_1k.gltf", min: Vec3::new(-0.169, 0.0, -0.169), max: Vec3::new(0.169, 0.554, 0.169), back: None, range: Some(0.0..35.0) };
/// Street cabinet, doors on +z.
pub(crate) const UTILITY_BOX: Model = Model { path: "library/props/ph-utility-box-02/utility_box_02_1k.gltf", min: Vec3::new(-0.46, 0.0, -0.21), max: Vec3::new(0.46, 1.12, 0.222), back: Some(Vec3::NEG_Z), range: Some(0.0..35.0) };
/// Caged bulkhead lamp: back plate on z = 0, glass towards +z.
pub(crate) const WALL_LAMP: Model = Model { path: "library/props/ph-industrial-wall-lamp/industrial_wall_lamp_1k.gltf", min: Vec3::new(-0.137, -0.215, 0.0), max: Vec3::new(0.137, 0.215, 0.14), back: Some(Vec3::NEG_Z), range: None };
/// Enamel high-bay shade on a chain; its top (the hook) at y = 0.
pub(crate) const HIGH_BAY: Model = Model { path: "library/props/ph-hanging-industrial-lamp/hanging_industrial_lamp_1k.gltf", min: Vec3::new(-0.275, -1.34, -0.275), max: Vec3::new(0.275, 0.015, 0.275), back: None, range: None };
/// Precast concrete barrier, 1.57 long (x) x 1.11 high x 0.44 deep: the same height as low cover.
/// The barrier's second LOD (asset-pipe clustering, 2.9 k triangles), past 12.5 m.
pub(crate) const BARRIER_LOD1: &str = "library/props/ph-concrete-road-barrier-02/barrier.lod1.glb";
pub(crate) const BARRIER: Model = Model { path: "library/props/ph-concrete-road-barrier-02/concrete_road_barrier_02_1k.gltf", min: Vec3::new(-0.776, -0.003, -0.254), max: Vec3::new(0.789, 1.109, 0.188), back: None, range: None };

/// Deterministic 0..1 hash of a few integers.
fn hash(a: i32, b: i32, salt: u32) -> f32 {
    let mut h = (a as u32).wrapping_mul(0x8da6_b343) ^ (b as u32).wrapping_mul(0xd816_3841) ^ salt.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// A vertical face of a block: ground point at its middle, outward normal, along-wall unit
/// vector, length, block height, block index.
struct Face {
    mid: Vec3,
    n: Vec3,
    t: Vec3,
    len: f32,
    height: f32,
    block: usize,
}

/// Faces of walls and tall buildings that look into the playable area.
fn faces(arena: &Arena) -> Vec<Face> {
    let lim = arena.half() + 0.01;
    let mut out = Vec::new();
    for (i, b) in arena.blocks.iter().enumerate() {
        let wallish = b.kind == BlockKind::Wall || (b.kind == BlockKind::Tall && b.height >= 4.0 && matches!(b.skin, Skin::Concrete | Skin::Brick | Skin::Siding));
        if !wallish {
            continue;
        }
        let c = Vec2::new(b.center.x, b.center.y);
        for (n, half_n, len) in [(Vec2::X, b.half.x, b.half.y * 2.0), (-Vec2::X, b.half.x, b.half.y * 2.0), (Vec2::Y, b.half.y, b.half.x * 2.0), (-Vec2::Y, b.half.y, b.half.x * 2.0)] {
            let p = c + n * half_n;
            if len < 1.5 || p.x.abs() > lim || p.y.abs() > lim || (b.kind == BlockKind::Wall && (-p).dot(n) <= 0.0) {
                continue;
            }
            let n3 = Vec3::new(n.x, 0.0, n.y);
            out.push(Face { mid: Vec3::new(p.x, 0.0, p.y), n: n3, t: Vec3::Y.cross(n3), len, height: b.height, block: i });
        }
    }
    out
}

/// Whether world point `p` (any height) is inside a block other than `skip` that reaches up to
/// `p.y + pad`, with `pad` slack sideways.
fn in_other_block(arena: &Arena, p: Vec3, pad: f32, skip: usize) -> bool {
    arena.blocks.iter().enumerate().any(|(i, b)| {
        i != skip && (p.x - b.center.x).abs() < b.half.x + pad && (p.z - b.center.y).abs() < b.half.y + pad && p.y < b.height + pad
    })
}

/// Near a lamp fixture, a sign plate or a doorway header (pipes keep clear of them).
fn busy(arena: &Arena, p: Vec3) -> bool {
    let flat = |v: Vec3| Vec2::new(v.x, v.z);
    arena.decor.lamps.iter().any(|l| Vec2::new(l.pos.x, l.pos.y).distance(flat(p)) < 1.4 && (l.height - p.y).abs() < 1.6)
        || arena.decor.signs.iter().any(|s| matches!(s.mount, brain::arena::SignMount::Wall { .. }) && Vec2::new(s.pos.x, s.pos.y).distance(flat(p)) < s.size * 2.5 + 0.8 && (s.y - p.y).abs() < s.size + 0.8)
        || arena.decor.headers.iter().any(|h| p.x > h.min.x - 0.6 && p.x < h.max.x + 0.6 && p.z > h.min.y - 0.6 && p.z < h.max.y + 0.6 && p.y > h.bottom - 0.6)
}

/// Rubble, paper and cable offcuts along wall feet, densest in corners, all under 0.25 m and
/// within 0.6 m of the wall, never in a doorway. Merged into `batch` (no shadows).
fn debris(arena: &Arena, batch: &mut Batch) {
    let doorway = |p: Vec3| arena.decor.headers.iter().any(|h| p.x > h.min.x - 1.0 && p.x < h.max.x + 1.0 && p.z > h.min.y - 1.0 && p.z < h.max.y + 1.0);
    for (fi, f) in faces(arena).iter().enumerate() {
        let n = (f.len / 0.35) as i32;
        for k in 0..n {
            let a = -f.len * 0.5 + (k as f32 + 0.5) * f.len / n as f32;
            let corner = 1.0 - ((f.len * 0.5 - a.abs()) / 2.0).clamp(0.0, 1.0); // 1 at the face ends
            let r = hash(fi as i32, k, 61);
            if r > 0.18 + 0.5 * corner {
                continue;
            }
            let out = 0.08 + hash(fi as i32, k, 63) * (0.25 + 0.3 * corner);
            let at = f.mid + f.t * a + f.n * out;
            if in_other_block(arena, at + Vec3::Y * 0.05, 0.12, f.block) || doorway(at) {
                continue;
            }
            let yaw = Quat::from_rotation_y(hash(k, fi as i32, 67) * 6.283);
            let kind = hash(fi as i32, k, 69);
            if kind < 0.62 {
                // A cluster of broken concrete: one chunk and a few crumbs.
                for c in 0..4 {
                    let sz = if c == 0 { 0.08 + 0.12 * hash(k, c, 71) } else { 0.025 + 0.04 * hash(k, c, 73) };
                    let off = if c == 0 { Vec3::ZERO } else { Vec3::new(hash(c, k, 75) - 0.5, 0.0, hash(k, c, 77) - 0.5) * 0.4 };
                    let tilt = Quat::from_euler(EulerRot::XYZ, hash(c, k, 79) * 0.8, hash(k, c, 81) * 3.1, hash(c, c + k, 83) * 0.8);
                    let dims = Vec3::new(sz, sz * (0.45 + 0.4 * hash(c, k, 85)), sz * (0.6 + 0.6 * hash(k, c, 87)));
                    batch.add_shape("foundry_rubble", Cuboid::from_size(dims), Transform::from_translation(at + yaw * off + Vec3::Y * dims.y * 0.35).with_rotation(yaw * tilt));
                }
            } else if kind < 0.85 {
                // A sheet of paper, lying flat.
                batch.add_shape("debris_paper", Cuboid::new(0.21, 0.002, 0.297), Transform::from_translation(at + Vec3::Y * 0.016).with_rotation(yaw * Quat::from_rotation_x(0.04)));
            } else {
                // A cable offcut: two kinked lengths.
                let l = 0.25 + 0.35 * hash(k, fi as i32, 89);
                let kink = Quat::from_rotation_y(0.5 + hash(fi as i32, k, 91));
                batch.add_shape("cable", Cylinder::new(0.009, l).mesh().resolution(6), Transform::from_translation(at + Vec3::Y * 0.01).with_rotation(yaw * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)));
                batch.add_shape("cable", Cylinder::new(0.009, l * 0.7).mesh().resolution(6), Transform::from_translation(at + yaw * Vec3::new(l * 0.5, 0.01, l * 0.25)).with_rotation(yaw * kink * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)));
            }
        }
    }
}

fn register(g: &mut Gfx, p: &mut Palette) {
    p.flat(g, "debris_paper", || StandardMaterial { base_color: Color::srgb(0.62, 0.60, 0.55), perceptual_roughness: 0.9, ..default() });
    p.flat(g, "cable", || StandardMaterial { base_color: Color::srgb(0.025, 0.025, 0.028), perceptual_roughness: 0.55, ..default() });
    p.flat(g, "pipe_clamp", || StandardMaterial { base_color: Color::srgb(0.18, 0.17, 0.16), perceptual_roughness: 0.6, metallic: 0.8, ..default() });
}

/// Pipes, cables and their brackets along every face from [`faces`], merged per material.
pub(crate) fn spawn(commands: &mut Commands, arena: &Arena, g: &mut Gfx, p: &mut Palette) {
    if arena.id != MapId::Foundry || g.quality == GraphicsQuality::Low {
        return;
    }
    register(g, p);
    let mut pipes = Batch::default();
    let mut thin = Batch::default();
    let seg = 2.0;
    for (fi, f) in faces(arena).iter().enumerate() {
        if hash(fi as i32, f.block as i32, 3) > 0.72 {
            continue;
        }
        let paint = if hash(fi as i32, 1, 5) < 0.5 { "foundry_pipe" } else { "foundry_pipe_b" };
        let big = 0.07 + 0.06 * hash(fi as i32, 2, 7);
        let top = (f.height - 0.55).min(6.2);
        let runs = [(big, top, paint), (0.045, top - 0.32, "foundry_pipe")];
        let n = ((f.len - 0.6) / seg).floor() as i32;
        if n < 1 {
            continue;
        }
        let start = -(n as f32) * seg * 0.5;
        let mut last_ok: Option<Vec3> = None;
        for k in 0..n {
            let s = start + (k as f32 + 0.5) * seg;
            for (ri, &(r, y, key)) in runs.iter().enumerate() {
                if y < 3.0 {
                    continue;
                }
                let at = f.mid + f.t * s + f.n * (r + 0.07) + Vec3::Y * y;
                if in_other_block(arena, at, r + 0.1, f.block) || busy(arena, at) {
                    continue;
                }
                let rot = Quat::from_rotation_arc(Vec3::Y, f.t);
                pipes.add_shape(key, Cylinder::new(r, seg + 0.002).mesh().resolution(12), Transform::from_translation(at).with_rotation(rot));
                // A clamp at the start of each length, from the wall to round the pipe.
                let clamp = f.mid + f.t * (s - seg * 0.5 + 0.15) + f.n * ((r + 0.07) * 0.5 + 0.01) + Vec3::Y * y;
                let dims = Vec3::new(0.05, r * 2.0 + 0.06, r * 2.0 + 0.14);
                let (x, z) = if f.n.x.abs() > 0.5 { (dims.z, dims.x) } else { (dims.x, dims.z) };
                thin.add_shape("pipe_clamp", Cuboid::new(x, dims.y, z), Transform::from_translation(clamp));
                if ri == 0 {
                    last_ok = Some(at);
                }
            }
            // A bundle of cables under the pipes, on the same brackets.
            let y = top - 0.62;
            let at = f.mid + f.t * s + f.n * 0.05 + Vec3::Y * y;
            if y > 2.6 && !in_other_block(arena, at, 0.12, f.block) && !busy(arena, at) {
                for (c, dy) in [(0.0f32, 0.0f32), (0.035, -0.02), (0.07, 0.01)] {
                    let rot = Quat::from_rotation_arc(Vec3::Y, f.t);
                    thin.add_shape("cable", Cylinder::new(0.014, seg + 0.002).mesh().resolution(6), Transform::from_translation(at + f.n * c + Vec3::Y * dy).with_rotation(rot));
                }
            }
        }
        // Now and then a downpipe from the end of the big run to the ground, flush to the wall.
        if let Some(end) = last_ok.filter(|_| hash(fi as i32, 9, 11) < 0.35) {
            let r = big * 0.8;
            let foot = Vec3::new(end.x, 0.0, end.z) + f.t * (seg * 0.5 - 0.25);
            let h = end.y + big;
            let mid = foot + Vec3::Y * (h * 0.5);
            if !in_other_block(arena, mid.with_y(0.5), r + 0.3, f.block) && !busy(arena, mid) {
                pipes.add_shape(paint, Cylinder::new(r, h).mesh().resolution(12), Transform::from_translation(mid));
            }
        }
    }
    debris(arena, &mut thin);
    pipes.spawn_scattered(commands, g, p, "Pipes", false);
    thin.spawn_scattered(commands, g, p, "Pipe clamps, cables and debris", false);
}

/// Spawns `m` standing on the ground at `at`, its back against a wall whose outward normal is
/// `n` (or turned by `yaw` when it has no back), scaled by `scale`. Visual only.
pub(crate) fn place(commands: &mut Commands, g: &Gfx, m: &Model, at: Vec3, n: Vec3, yaw: f32, scale: f32, shadows: bool) {
    let rot = match m.back {
        // Turn the model's `back` axis to face away from the wall's normal (towards the wall).
        Some(back) => Quat::from_rotation_arc(Vec3::new(back.x, 0.0, back.z).normalize(), -n),
        None => Quat::from_rotation_y(yaw),
    };
    // Anchor: the middle of the bottom face, moved to the back face for wall-standing models.
    let mut anchor = m.foot();
    if let Some(b) = m.back {
        if b.x != 0.0 {
            anchor.x = if b.x > 0.0 { m.max.x } else { m.min.x };
        }
        if b.z != 0.0 {
            anchor.z = if b.z > 0.0 { m.max.z } else { m.min.z };
        }
    }
    let tf = Transform::from_translation(at - rot * (anchor * scale)).with_rotation(rot).with_scale(Vec3::splat(scale));
    spawn_model(commands, g, m, tf, shadows);
}

/// Spawns model `m` at `tf` (visual only, map scenery). `shadows: false`: its meshes cast none.
pub(crate) fn spawn_model(commands: &mut Commands, g: &Gfx, m: &Model, tf: Transform, shadows: bool) -> Option<Entity> {
    spawn_scene(commands, g, m.path, tf, ModelMeshes { shadows, range: m.range.clone() })
}

/// Spawns `m` with its LOD chain: `m` (LOD0) up to `near` metres, `lod1` (a sibling GLB made by the
/// engine's asset-pipe) beyond, cross-faded over 2 m. Falls back to LOD0 alone without `lod1`.
pub(crate) fn spawn_lod(commands: &mut Commands, g: &Gfx, m: &Model, lod1: &str, near: f32, tf: Transform, shadows: bool) -> bool {
    if !g.dir.has(lod1) {
        return spawn_model(commands, g, m, tf, shadows).is_some();
    }
    spawn_scene(commands, g, m.path, tf, ModelMeshes { shadows, range: Some(0.0..near) }).is_some()
        && spawn_scene(commands, g, lod1, tf, ModelMeshes { shadows, range: Some(near..f32::INFINITY) }).is_some()
}

fn spawn_scene(commands: &mut Commands, g: &Gfx, path: &str, tf: Transform, meshes: ModelMeshes) -> Option<Entity> {
    if !g.dir.has(path) {
        return None;
    }
    let scene = g.server.load(bevy::gltf::GltfAssetLabel::Scene(0).from_asset(path.to_string()));
    Some(commands.spawn((Name::new(format!("Prop {path}")), ArenaPiece, Scenery, meshes, bevy::world_serialization::WorldAssetRoot(scene), tf)).id())
}

/// On a model root: what its meshes get once the scene has spawned them ([`tag_model_meshes`]).
#[derive(Component, Clone)]
pub(crate) struct ModelMeshes {
    pub shadows: bool,
    /// Drawn only this far from the camera (metres), cross-faded over 2 m at each end.
    pub range: Option<std::ops::Range<f32>>,
}

/// Meshes a model scene just spawned are static map scenery (`Scenery`, for the render budget
/// and the light bake), lose shadows on small props, and get their LOD / draw range.
pub(crate) fn tag_model_meshes(mut commands: Commands, added: Query<Entity, Added<Mesh3d>>, parents: Query<&ChildOf>, roots: Query<&ModelMeshes>) {
    for e in &added {
        let Some(m) = parents.iter_ancestors(e).find_map(|a| roots.get(a).ok()) else { continue };
        let mut c = commands.entity(e);
        c.insert(Scenery);
        if !m.shadows {
            c.insert(NotShadowCaster);
        }
        if let Some(r) = &m.range {
            let start = if r.start > 0.0 { r.start - 1.0..r.start + 1.0 } else { 0.0..0.0 };
            c.insert(bevy::camera::visibility::VisibilityRange { start_margin: start, end_margin: r.end - 1.0..r.end + 1.0, use_aabb: false });
        }
    }
}

/// Clutter in concave corners (a wall face whose end runs into another block) and street
/// cabinets against long walls (0.43 m deep: flush, like a pipe, not cover).
pub(crate) fn clutter(commands: &mut Commands, arena: &Arena, g: &mut Gfx) {
    if arena.id != MapId::Foundry || g.quality == GraphicsQuality::Low {
        return;
    }
    let piles: [&[(&Model, f32, f32)]; 6] = [
        // (model, offset along the wall from the corner, offset out from the wall)
        &[(&BARREL_RED, 0.35, 0.33), (&BARREL_BLUE, 0.98, 0.33)],
        &[(&CEMENT, 0.4, 0.12), (&CEMENT, 0.95, 0.12), (&BOX, 0.6, 0.55)],
        &[(&BARREL_BLUE, 0.35, 0.33), (&JERRYCAN, 0.85, 0.14)],
        &[(&TYRE, 0.45, 0.09), (&PROPANE, 1.05, 0.2)],
        &[(&BOX, 0.4, 0.34), (&BOX, 0.5, 0.3)],
        &[(&BARREL_RED, 0.35, 0.33), (&PROPANE, 0.85, 0.2), (&PROPANE, 1.15, 0.22)],
    ];
    let mut placed = 0;
    for (fi, f) in faces(arena).iter().enumerate() {
        for end in [-1.0f32, 1.0] {
            let corner = f.mid + f.t * (end * f.len * 0.5);
            // Concave: just past the end, beside the wall, is another block.
            let beyond = corner + f.t * (end * 0.25) + f.n * 0.4 + Vec3::Y * 0.5;
            if !in_other_block(arena, beyond, 0.0, f.block) {
                continue;
            }
            let pick = (hash(fi as i32, end as i32, 21) * 9.0) as usize;
            let Some(pile) = piles.get(pick) else { continue };
            let mut ok = true;
            let spots: Vec<(&Model, Vec3)> = pile.iter().map(|&(m, a, o)| (m, corner - f.t * (end * a) + f.n * o)).collect();
            for &(m, s) in &spots {
                let r = m.size().x.max(m.size().z) * 0.5;
                if in_other_block(arena, s + Vec3::Y * 0.3, r - 0.05, usize::MAX) || busy(arena, s + Vec3::Y * 2.0) {
                    ok = false;
                }
            }
            if !ok {
                continue;
            }
            for (k, (m, s)) in spots.into_iter().enumerate() {
                let yaw = hash(fi as i32, k as i32, 31) * std::f32::consts::TAU;
                // Stack the second box on the first.
                let lift = if std::ptr::eq(m, &BOX) && k == 1 && pick == 4 { BOX.size().y } else { 0.0 };
                place(commands, g, m, s + Vec3::Y * lift, f.n, yaw, 1.0, false);
            }
            placed += 1;
        }
        // A fuse or utility cabinet on some long faces, mid-wall, clear of everything.
        if f.len >= 4.0 && hash(fi as i32, 5, 41) < 0.3 {
            let s = f.mid + f.t * ((hash(fi as i32, 7, 47) - 0.5) * (f.len - 2.0)) + f.n * 0.01;
            let clear = !in_other_block(arena, s + f.n * 0.4 + Vec3::Y * 0.6, 0.3, f.block) && !busy(arena, s + Vec3::Y * 1.4);
            if clear {
                place(commands, g, &UTILITY_BOX, s, f.n, 0.0, 1.0, false);
            }
        }
    }
    debug!("foundry clutter: {placed} corner piles");
}
