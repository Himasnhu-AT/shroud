//! Grime and wear so big flat walls and floors aren't uniform:
//!
//! - **Edge grime**: a dirty band along the foot of every wall, one merged mesh per map with a
//!   lit, alpha-blended dirt material on a texture that tiles along the wall. One draw call
//!   for the whole map.
//! - **Stains, streaks and floor marks**: the engine's forward decals (`engine::look::decal`),
//!   a few dozen per map on three shared materials (water streaks running down facades,
//!   blotchy floor stains, tyre marks / scuffs). Skipped on Low quality (they need a depth
//!   prepass).
//!
//! Textures and materials are made once at startup ([`GrimeAssets`]); a map build adds one
//! mesh and spawns decal entities on shared handles. Everything is an [`ArenaPiece`], so it
//! goes with the map. Placement is deterministic per map.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::decal::ForwardDecalMaterial;
use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use brain::arena::{Arena, Block, BlockKind, MapId, Skin};
use engine::look::{decal_image, decal_material, spawn_decal};
use engine::render_budget::KeepDetail;
use engine::settings::{CoreSettings, GraphicsQuality};

use super::{ArenaPiece, ArenaRes};
use crate::assets::AssetsDir;
use crate::gfx::PendingMips;

/// Edge-grime texture repeats every this many metres along a wall.
const STRIP_TILE: f32 = 2.5;

/// Shared grime textures and materials.
#[derive(Resource)]
pub(crate) struct GrimeAssets {
    strip_strong: Handle<StandardMaterial>,
    strip_light: Handle<StandardMaterial>,
    stain: Handle<ForwardDecalMaterial<StandardMaterial>>,
    streak: Handle<ForwardDecalMaterial<StandardMaterial>>,
    marks: Handle<ForwardDecalMaterial<StandardMaterial>>,
    /// Foundry draws all its grime as merged alpha-blended quads, one mesh per material
    /// ([`Kind`]): a few draws instead of one per decal.
    quad_mats: HashMap<Kind, Handle<StandardMaterial>>,
}

/// What a grime quad / decal shows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    Stain,
    Marks,
    Streak,
    /// Scanned rust / water runs (two variants).
    Leak0,
    Leak1,
    /// Scanned damp band at a wall foot.
    Damp,
    Manhole,
    Tyres,
    /// Big soft dirt at wall feet and soot at wall tops, varying along the wall.
    Wash,
    /// Yellow / black hazard stripes.
    Hazard,
    /// Yellow warning triangle plate.
    Warning,
    /// Dust drifts on the floor against walls.
    Dust,
}

// ---------------------------------------------------------------- noise

fn hash(x: i32, y: i32, salt: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ salt.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Smooth value noise in 0..1; `period` > 0 wraps x (tiles horizontally).
fn noise(x: f32, y: f32, period: i32, salt: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let w = |i: i32| if period > 0 { i.rem_euclid(period) } else { i };
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let a = hash(w(xi), yi, salt);
    let b = hash(w(xi + 1), yi, salt);
    let c = hash(w(xi), yi + 1, salt);
    let d = hash(w(xi + 1), yi + 1, salt);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

/// Four octaves of [`noise`], 0..1. `period` is the wrap of the first octave.
fn fbm(x: f32, y: f32, period: i32, salt: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 0.5, 0.0, 1.0);
    for o in 0..4 {
        let p = if period > 0 { period * f as i32 } else { 0 };
        sum += amp * noise(x * f, y * f, p, salt.wrapping_add(o * 977));
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------- textures

/// The wall-foot band: dirt colour whose alpha is clear at the top and fades in, noisy, to
/// the bottom, with drip streaks reaching up. Tiles along u; v = 0 at the top of the band.
fn strip_image(strength: f32) -> Image {
    let (w, h) = (256u32, 64u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    let dirt = [0.20f32, 0.17, 0.13];
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let v = (y as f32 + 0.5) / h as f32;
            let up = 1.0 - v; // height in the band, 0 at the floor
            // Ragged top edge, plus thin streaks climbing higher.
            let edge = 0.45 + 0.5 * (fbm(u * 8.0, 0.5, 8, 11) - 0.5) * 2.0;
            let streak = smoothstep(0.55, 0.75, fbm(u * 48.0, 0.3, 48, 23)) * 0.4;
            let reach = (edge + streak).clamp(0.2, 1.0);
            let mut d = (1.0 - up / reach).clamp(0.0, 1.0).powf(0.9);
            // Blotchy variation and a darker line right at the floor.
            d *= 0.7 + 0.3 * fbm(u * 24.0, v * 6.0, 24, 37);
            d = d.max(smoothstep(0.93, 1.0, v) * 0.8);
            let k = (d * strength).clamp(0.0, 1.0);
            for c in dirt {
                data.push((c.powf(1.0 / 2.2) * 255.0 + 0.5) as u8);
            }
            data.push((k * 0.85 * 255.0 + 0.5) as u8);
        }
    }
    let mut img = Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    engine::gfx::build_mips(&mut img);
    img
}

/// Blotchy floor stain (oil, water marks), soft ragged edge.
fn stain_image() -> Image {
    decal_image(128, |uv| {
        let p = uv * 2.0 - Vec2::ONE;
        let r = p.length() + (fbm(uv.x * 5.0, uv.y * 5.0, 0, 51) - 0.5) * 0.7;
        let body = smoothstep(0.85, 0.35, r);
        let mottled = 0.55 + 0.45 * fbm(uv.x * 14.0, uv.y * 14.0, 0, 53);
        let ring = smoothstep(0.5, 0.75, r) * smoothstep(0.95, 0.75, r) * 0.35;
        let a = ((body * mottled + ring) * 0.8).clamp(0.0, 1.0);
        [0.13, 0.12, 0.10, a]
    })
}

/// Water streaks running down from the top edge (v = 0 at the top), fading at the sides.
fn streak_image() -> Image {
    decal_image(128, |uv| {
        let lane = fbm(uv.x * 12.0, 0.5, 0, 61);
        let len = 0.35 + 0.6 * fbm(uv.x * 6.0, 2.5, 0, 67);
        let run = smoothstep(0.42, 0.7, lane) * (1.0 - smoothstep(len * 0.6, len, uv.y));
        let wash = (1.0 - uv.y).powf(2.0) * 0.35 * fbm(uv.x * 4.0, uv.y * 3.0, 0, 71);
        let sides = smoothstep(0.0, 0.18, uv.x) * smoothstep(1.0, 0.82, uv.x);
        let top = smoothstep(0.0, 0.04, uv.y);
        let a = ((run * 0.75 + wash) * sides * top).clamp(0.0, 1.0);
        [0.16, 0.15, 0.13, a]
    })
}

/// Two parallel worn bands (tyre marks on streets, scuffs indoors), patchy along their length.
fn marks_image() -> Image {
    decal_image(128, |uv| {
        let band = |c: f32| smoothstep(0.09, 0.04, (uv.x - c).abs());
        let b = band(0.3).max(band(0.7));
        let patchy = 0.4 + 0.6 * fbm(uv.x * 3.0, uv.y * 10.0, 0, 81);
        let ends = smoothstep(0.0, 0.2, uv.y) * smoothstep(1.0, 0.8, uv.y);
        [0.07, 0.07, 0.07, (b * patchy * ends * 0.7).clamp(0.0, 1.0)]
    })
}

/// Wash texture repeats every this many metres along a wall.
const WASH_TILE: f32 = 9.0;

/// Large-scale wall variation, one image for the full wall height (v = 0 at the top): soot under
/// the top, dirt rising from the foot, each with a ragged, low-frequency edge that varies along u
/// (tiles in u).
fn wash_image() -> Image {
    decal_image(256, |uv| {
        let (u, v) = (uv.x, uv.y);
        let up = 1.0 - v;
        let foot_reach = 0.22 + 0.28 * fbm(u * 4.0, 0.5, 4, 101);
        let foot = (1.0 - smoothstep(0.0, foot_reach, up)).powf(1.4) * (0.55 + 0.45 * fbm(u * 12.0, v * 5.0, 12, 103));
        let soot_reach = 0.08 + 0.18 * fbm(u * 3.0, 2.5, 3, 107);
        let soot = (1.0 - smoothstep(0.0, soot_reach, v)).powf(1.2) * (0.5 + 0.5 * fbm(u * 10.0, v * 4.0, 10, 109));
        let blotch = smoothstep(0.55, 0.85, fbm(u * 6.0, v * 3.0, 6, 113)) * 0.25;
        let a = (foot * 0.95 + soot * 0.8 + blotch).clamp(0.0, 0.9);
        // Darker than the scans themselves (Foundry's cast concrete and brick are ~0.2 albedo);
        // decal_image stores the values as given in an sRGB texture.
        if soot > foot { [0.05, 0.048, 0.045, a] } else { [0.12, 0.105, 0.09, a] }
    })
}

/// Worn yellow / black diagonal stripes (one stripe pair per u unit), chipped by noise.
fn hazard_image() -> Image {
    let mut img = decal_image(128, |uv| {
        let stripe = ((uv.x * 2.0 + uv.y * 0.5).fract() < 0.5) as i32;
        let wear = fbm(uv.x * 9.0, uv.y * 9.0, 9, 131);
        let a = smoothstep(0.25, 0.45, wear) * 0.92;
        if stripe == 1 { [0.78, 0.56, 0.06, a] } else { [0.03, 0.03, 0.03, a] }
    });
    engine::gfx::build_mips(&mut img);
    img
}

/// A yellow warning triangle with a black border and exclamation mark on a white plate.
fn warning_image() -> Image {
    decal_image(128, |uv| {
        let p = Vec2::new(uv.x - 0.5, 0.92 - uv.y); // y up from the plate's bottom margin
        let inside = |m: f32| p.y >= m && p.y <= 0.84 - m * 1.6 && p.x.abs() <= (0.84 - p.y) * 0.62 - m;
        let mark = (p.x.abs() < 0.035 && p.y > 0.22 && p.y < 0.58) || (p.x.abs() < 0.04 && p.y > 0.11 && p.y < 0.17);
        let grime = 0.85 + 0.15 * fbm(uv.x * 8.0, uv.y * 8.0, 0, 137);
        let c = if inside(0.07) && !mark { [0.85 * grime, 0.62 * grime, 0.05] } else if inside(0.0) || mark { [0.03, 0.03, 0.03] } else { [0.8 * grime, 0.8 * grime, 0.76 * grime] };
        [c[0], c[1], c[2], 1.0]
    })
}

/// A drift of grey dust along a wall foot: dense at v = 1 (the wall), fading out by v = 0.
fn dust_image() -> Image {
    let mut img = decal_image(128, |uv| {
        let edge = 0.35 + 0.5 * fbm(uv.x * 6.0, 0.5, 0, 141);
        let a = smoothstep(1.0 - edge, 1.0, uv.y) * (0.55 + 0.45 * fbm(uv.x * 20.0, uv.y * 8.0, 0, 143)) * 0.8;
        [0.36, 0.34, 0.31, a]
    });
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor { address_mode_u: ImageAddressMode::Repeat, ..ImageSamplerDescriptor::linear() });
    img
}

pub(crate) fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut decals: ResMut<Assets<ForwardDecalMaterial<StandardMaterial>>>,
    server: Option<Res<AssetServer>>,
    dir: Option<Res<AssetsDir>>,
    pending: Option<ResMut<PendingMips>>,
) {
    let mut quad_mats = HashMap::new();
    let blend = |m: StandardMaterial| StandardMaterial { alpha_mode: AlphaMode::Blend, depth_map: None, double_sided: false, cull_mode: Some(bevy::render::render_resource::Face::Back), ..m };
    if let (Some(server), Some(dir), Some(mut pending)) = (server, dir, pending) {
        for (k, name) in [(Kind::Leak0, "acg-leaking001"), (Kind::Leak1, "acg-leaking003"), (Kind::Damp, "acg-leaking007"), (Kind::Manhole, "acg-manholecover009"), (Kind::Tyres, "acg-tiretracks001")] {
            // The leak scans were shot on pale walls: darken them to read on dark concrete.
            let tint = match k {
                Kind::Leak0 | Kind::Leak1 => Color::srgb(0.30, 0.27, 0.24),
                Kind::Damp => Color::srgb(0.45, 0.42, 0.38),
                _ => Color::WHITE,
            };
            if let Some(m) = engine::gfx::load_pbr_at(&server, &dir, &mut pending, &format!("library/decals/{name}"), tint) {
                quad_mats.insert(k, mats.add(blend(m)));
            }
        }
    }
    let tiled = |img: Image| {
        let mut img = img;
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor { address_mode_u: ImageAddressMode::Repeat, address_mode_v: ImageAddressMode::ClampToEdge, anisotropy_clamp: 8, ..ImageSamplerDescriptor::linear() });
        img
    };
    for (k, img, rough) in [(Kind::Wash, tiled(wash_image()), 1.0), (Kind::Hazard, tiled(hazard_image()), 0.7), (Kind::Warning, warning_image(), 0.5), (Kind::Dust, dust_image(), 1.0)] {
        let h = images.add(img);
        quad_mats.insert(k, mats.add(StandardMaterial { base_color_texture: Some(h), perceptual_roughness: rough, reflectance: 0.3, alpha_mode: AlphaMode::Blend, ..default() }));
    }
    let strip = |img: Handle<Image>| StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(img),
        perceptual_roughness: 1.0,
        reflectance: 0.2,
        alpha_mode: AlphaMode::Blend,
        ..default()
    };
    let strong = images.add(strip_image(1.0));
    let light = images.add(strip_image(0.7));
    let decal = |img: Image, images: &mut Assets<Image>| StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(img)),
        perceptual_roughness: 0.95,
        ..default()
    };
    let stain = decal(stain_image(), &mut images);
    let streak = decal(streak_image(), &mut images);
    let marks = decal(marks_image(), &mut images);
    for (k, m) in [(Kind::Stain, &stain), (Kind::Streak, &streak), (Kind::Marks, &marks)] {
        quad_mats.insert(k, mats.add(blend(m.clone())));
    }
    commands.insert_resource(GrimeAssets {
        strip_strong: mats.add(strip(strong)),
        strip_light: mats.add(strip(light)),
        stain: decals.add(decal_material(stain, 0.06)),
        streak: decals.add(decal_material(streak, 0.06)),
        marks: decals.add(decal_material(marks, 0.06)),
        quad_mats,
    });
}

// ---------------------------------------------------------------- placement

/// Deterministic 0..1 sequence per map.
struct Seq(u32);

impl Seq {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(1);
        hash(self.0 as i32, 7, 0x51ed_270b)
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

/// Blocks that read as walls (get edge grime and streaks); props don't.
fn is_wall(b: &Block) -> bool {
    matches!(b.skin, Skin::Auto | Skin::Concrete | Skin::Stucco | Skin::Drywall | Skin::Brick | Skin::Plaster | Skin::Siding | Skin::Sandbag | Skin::Container)
}

/// A vertical block face: centre on the ground, outward normal, length along it, the block.
struct Face {
    at: Vec3,
    n: Vec3,
    len: f32,
    height: f32,
    /// Faces away from its building / toward the playable area (gets streaks).
    outer: bool,
}

fn faces(arena: &Arena) -> Vec<Face> {
    let mut shell_center = vec![None; arena.blocks.len()];
    let mut sill = vec![false; arena.blocks.len()];
    for b in &arena.decor.buildings {
        let c = (b.min + b.max) * 0.5;
        for &i in &b.shell {
            shell_center[i] = Some(Vec2::new(c.x, c.y));
            sill[i] = arena.blocks[i].kind == BlockKind::Low;
        }
    }
    let lim = arena.half() + 0.01;
    let mut out = Vec::new();
    for (i, b) in arena.blocks.iter().enumerate() {
        if !is_wall(b) || sill[i] || (b.kind == BlockKind::Low && b.height < 0.6) {
            continue;
        }
        let c = Vec2::new(b.center.x, b.center.y);
        for (n, half_n, len) in [(Vec2::X, b.half.x, b.half.y * 2.0), (-Vec2::X, b.half.x, b.half.y * 2.0), (Vec2::Y, b.half.y, b.half.x * 2.0), (-Vec2::Y, b.half.y, b.half.x * 2.0)] {
            let p = c + n * half_n;
            // Outer faces of the perimeter wall look out of the map: nobody sees them.
            if len < 0.8 || p.x.abs() > lim || p.y.abs() > lim {
                continue;
            }
            let outer = match shell_center[i] {
                Some(bc) => (p - bc).dot(n) > 0.0,
                None => b.kind != BlockKind::Wall || (-p).dot(n) > 0.0,
            };
            out.push(Face { at: Vec3::new(p.x, 0.0, p.y), n: Vec3::new(n.x, 0.0, n.y), len, height: b.height, outer });
        }
    }
    out
}

/// One quad per wall face for the foot band, merged into one mesh (world-space u so the
/// texture runs continuously along a wall).
fn strip_mesh(faces: &[Face], band: f32) -> Mesh {
    let mut pos = Vec::with_capacity(faces.len() * 4);
    let mut nrm = Vec::with_capacity(faces.len() * 4);
    let mut uv = Vec::with_capacity(faces.len() * 4);
    let mut idx = Vec::with_capacity(faces.len() * 6);
    for f in faces {
        // Just proud of the wall and of interior linings (1 cm), behind skirting and plinths.
        let base = f.at + f.n * 0.014;
        let t = Vec3::Y.cross(f.n); // along the wall, so (t, up, n) faces outward
        let h = band.min(f.height * 0.6);
        let along = base.x * t.x + base.z * t.z; // world coordinate along t
        let (u0, u1) = ((along - f.len * 0.5) / STRIP_TILE, (along + f.len * 0.5) / STRIP_TILE);
        let i0 = pos.len() as u32;
        for (s, y, u, v) in [(-0.5, 0.0, u0, 1.0), (0.5, 0.0, u1, 1.0), (0.5, h, u1, 1.0 - h / band), (-0.5, h, u0, 1.0 - h / band)] {
            let p = base + t * (s * f.len) + Vec3::Y * y;
            pos.push(p.to_array());
            nrm.push(f.n.to_array());
            uv.push([u, v]);
        }
        idx.extend_from_slice(&[i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

/// How dirty each map is: band height, strong or light band, stains, marks, streaks.
fn budget(id: MapId) -> (f32, bool, usize, usize, usize) {
    match id {
        MapId::Foundry => (1.1, true, 28, 10, 26),
        MapId::Depot => (0.9, true, 30, 12, 24),
        MapId::Office => (0.32, false, 26, 18, 0),
        MapId::Township => (0.85, true, 40, 14, 40),
        MapId::Sirocco => (0.8, true, 34, 12, 30),
        MapId::Gantry => (1.0, true, 40, 16, 34),
    }
}

/// Whether ground point `p` is clear of every block (with a margin).
fn clear_of_blocks(arena: &Arena, p: Vec2, margin: f32) -> bool {
    arena.blocks.iter().all(|b| (p.x - b.center.x).abs() > b.half.x + margin || (p.y - b.center.y).abs() > b.half.y + margin)
}

/// Spawns grime for the current map once per map build.
pub(crate) fn spawn_grime(
    mut commands: Commands,
    arena: Res<ArenaRes>,
    assets: Option<Res<GrimeAssets>>,
    core: Res<CoreSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut done: Local<Option<MapId>>,
) {
    let Some(g) = assets else { return };
    let arena = &arena.0;
    if *done == Some(arena.id) {
        return;
    }
    *done = Some(arena.id);
    let (band, strong, n_stains, n_marks, n_streaks) = budget(arena.id);
    let faces = faces(arena);
    commands.spawn((
        Name::new("Edge grime"),
        ArenaPiece,
        Mesh3d(meshes.add(strip_mesh(&faces, band))),
        MeshMaterial3d(if strong { g.strip_strong.clone() } else { g.strip_light.clone() }),
        Transform::default(),
        NotShadowCaster,
        // A map-wide mesh: its centre is far from most views, so the render budget would draw
        // its flat far version (the average colour, nearly invisible) everywhere.
        KeepDetail,
    ));
    if core.quality == GraphicsQuality::Low {
        return;
    }
    let mut rng = Seq(arena.id as u32 * 7919 + 17);
    let half = arena.half() - 1.0;
    let mut sink = Sink { g: &g, quads: (arena.id == MapId::Foundry).then(HashMap::new) };
    // Floor stains and marks, on open ground (inside buildings too).
    for (count, kind) in [(n_stains, Kind::Stain), (n_marks, Kind::Marks)] {
        let mut placed = 0;
        for _ in 0..count * 8 {
            if placed == count {
                break;
            }
            let p = Vec2::new(rng.range(-half, half), rng.range(-half, half));
            let (w, l) = match (kind, arena.id) {
                (Kind::Stain, _) => {
                    let s = rng.range(0.8, 2.6);
                    (s, s * rng.range(0.7, 1.3))
                }
                (_, MapId::Office) => (rng.range(0.5, 0.8), rng.range(0.5, 1.0)),
                _ => (rng.range(0.7, 0.9), rng.range(3.0, 6.0)),
            };
            if !clear_of_blocks(arena, p, 0.2 + w.max(l) * 0.3) {
                continue;
            }
            let yaw = rng.range(0.0, std::f32::consts::TAU);
            // Between the ground slab (y 0) and floor patches (y 0.012).
            let tf = Transform::from_xyz(p.x, 0.006, p.y).with_rotation(Quat::from_rotation_y(yaw)).with_scale(Vec3::new(w, 0.1, l));
            sink.put(&mut commands, kind, tf);
            placed += 1;
        }
    }
    // Water streaks running down from the top of outer wall faces.
    let tall: Vec<&Face> = faces.iter().filter(|f| f.outer && f.len >= 2.0 && f.height >= 2.4).collect();
    for _ in 0..if tall.is_empty() { 0 } else { n_streaks } {
        let f = tall[(rng.next() * tall.len() as f32) as usize % tall.len()];
        let w = rng.range(1.0, 2.2).min(f.len - 0.2);
        let h = rng.range(1.2, 2.4).min(f.height - 0.3);
        let t = Vec3::Y.cross(f.n);
        let s = rng.range(-0.5, 0.5) * (f.len - w);
        let top = f.height - rng.range(0.02, 0.4);
        let at = f.at + t * s + f.n * 0.005 + Vec3::Y * (top - h * 0.5);
        sink.put(&mut commands, Kind::Streak, Transform::from_translation(at).with_rotation(wall_rot(f.n)).with_scale(Vec3::new(w, 0.1, h)));
    }
    if arena.id == MapId::Foundry {
        scanned(&mut commands, arena, &mut sink, &faces, &mut rng);
        signage(&mut commands, arena, &mut sink, &faces);
    }
    if let Some(quads) = sink.quads {
        let mut kinds: Vec<_> = quads.keys().copied().collect();
        kinds.sort_by_key(|k| *k as u8);
        for k in kinds {
            let Some(m) = g.quad_mats.get(&k) else { continue };
            commands.spawn((Name::new(format!("Grime {k:?}")), ArenaPiece, Mesh3d(meshes.add(quad_mesh(&quads[&k]))), MeshMaterial3d(m.clone()), Transform::default(), NotShadowCaster, KeepDetail));
        }
    }
}

/// Wall decal frame: local +Y = the face normal, -Z = up (the texture's top edge).
fn wall_rot(n: Vec3) -> Quat {
    Quat::from_mat3(&Mat3::from_cols(n.cross(Vec3::NEG_Y), n, Vec3::NEG_Y))
}

/// Where grime goes: forward decals, or (Foundry) quads collected per [`Kind`] and merged into
/// one mesh each. A placement is a decal transform (unit quad in x/z, +Y out of the surface) and
/// the u range across it (tiling textures run along a wall in world metres).
struct Sink<'a> {
    g: &'a GrimeAssets,
    quads: Option<HashMap<Kind, Vec<(Transform, [f32; 2])>>>,
}

impl Sink<'_> {
    fn put(&mut self, commands: &mut Commands, k: Kind, tf: Transform) {
        self.put_u(commands, k, tf, [0.0, 1.0]);
    }
    fn put_u(&mut self, commands: &mut Commands, k: Kind, tf: Transform, u: [f32; 2]) {
        if let Some(q) = &mut self.quads {
            q.entry(k).or_default().push((tf, u));
            return;
        }
        let m = match k {
            Kind::Stain => &self.g.stain,
            Kind::Marks => &self.g.marks,
            Kind::Streak => &self.g.streak,
            _ => return,
        };
        let e = spawn_decal(commands, m.clone(), tf);
        commands.entity(e).insert((ArenaPiece, NotShadowCaster));
    }
}

/// One mesh of quads from decal transforms. Quads are lifted off their surface: floor ones just
/// above the floor patches (under the paint), wall ones in front of linings and the edge-grime band.
fn quad_mesh(quads: &[(Transform, [f32; 2])]) -> Mesh {
    let (mut pos, mut nrm, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (k, (tf, [u0, u1])) in quads.iter().enumerate() {
        let n = tf.rotation * Vec3::Y;
        let r = tf.rotation * Vec3::X * (tf.scale.x * 0.5);
        let d = tf.rotation * Vec3::Z * (tf.scale.z * 0.5);
        let mut c = tf.translation;
        if n.y > 0.9 {
            c.y = 0.0135 + (k % 7) as f32 * 0.0001;
        } else {
            c += n * (0.0165 + (k % 5) as f32 * 0.0003);
        }
        let i0 = pos.len() as u32;
        for (p, t) in [(c - r - d, [*u0, 0.0]), (c + r - d, [*u1, 0.0]), (c + r + d, [*u1, 1.0]), (c - r + d, [*u0, 1.0])] {
            pos.push(p.to_array());
            nrm.push(n.to_array());
            uv.push(t);
        }
        // Counter-clockwise seen from the front (n).
        if r.cross(d).dot(n) > 0.0 { idx.extend_from_slice(&[i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]) } else { idx.extend_from_slice(&[i0, i0 + 2, i0 + 1, i0, i0 + 3, i0 + 2]) }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx));
    let _ = mesh.generate_tangents();
    mesh
}

/// Foundry's photoscanned and large-scale grime: a soft dirt wash up every wall foot and soot under
/// every wall top (varying along the wall, so tiling breaks up), rust and water runs, damp wall
/// feet, dust drifts in the floor corners, manholes and tyre tracks. Deterministic.
fn scanned(commands: &mut Commands, arena: &Arena, s: &mut Sink, faces: &[Face], rng: &mut Seq) {
    for f in faces.iter().filter(|f| f.len >= 1.0) {
        let t = Vec3::Y.cross(f.n);
        let along = f.at.x * t.x + f.at.z * t.z;
        let u = [(along - f.len * 0.5) / WASH_TILE, (along + f.len * 0.5) / WASH_TILE];
        // The wash image holds the foot band in its lower half and the soot band in its upper half.
        let h = f.height;
        let at = f.at + Vec3::Y * (h * 0.5);
        s.put_u(commands, Kind::Wash, Transform::from_translation(at).with_rotation(wall_rot(f.n)).with_scale(Vec3::new(f.len, 0.1, h)), u);
        // Dust drift on the floor along the foot of the wall.
        let dust_at = f.at + f.n * 0.35;
        if clear_of_blocks(arena, Vec2::new(dust_at.x, dust_at.z), 0.05) || f.height >= 2.0 {
            let rot = Quat::from_mat3(&Mat3::from_cols(t, Vec3::Y, -f.n));
            s.put_u(commands, Kind::Dust, Transform::from_translation(f.at + f.n * 0.4).with_rotation(rot).with_scale(Vec3::new(f.len, 0.1, 0.8)), [u[0] * 3.0, u[1] * 3.0]);
        }
    }
    let tall: Vec<&Face> = faces.iter().filter(|f| f.len >= 2.0 && f.height >= 2.4).collect();
    for k in 0..if tall.is_empty() { 0 } else { 70 } {
        let f = tall[(rng.next() * tall.len() as f32) as usize % tall.len()];
        let w = rng.range(1.4, 3.2).min(f.len - 0.1);
        let h = rng.range(1.6, 3.4).min(f.height - 0.2);
        let t = Vec3::Y.cross(f.n);
        let along = rng.range(-0.5, 0.5) * (f.len - w);
        let top = f.height - rng.range(0.0, 0.25);
        let at = f.at + t * along + Vec3::Y * (top - h * 0.5);
        s.put(commands, if k % 2 == 0 { Kind::Leak0 } else { Kind::Leak1 }, Transform::from_translation(at).with_rotation(wall_rot(f.n)).with_scale(Vec3::new(w, 0.1, h)));
    }
    for f in faces.iter().filter(|f| f.len >= 1.5) {
        if rng.next() > 0.55 {
            continue;
        }
        let w = rng.range(2.0, 4.0).min(f.len);
        let h = rng.range(0.7, 1.3).min(f.height * 0.6);
        let t = Vec3::Y.cross(f.n);
        let at = f.at + t * (rng.range(-0.5, 0.5) * (f.len - w)) + Vec3::Y * (h * 0.5);
        s.put(commands, Kind::Damp, Transform::from_translation(at).with_rotation(wall_rot(f.n)).with_scale(Vec3::new(w, 0.1, h)));
    }
    let half = arena.half() - 1.5;
    let ground = |commands: &mut Commands, s: &mut Sink, k: Kind, count: usize, size: (f32, f32), aspect: f32, rng: &mut Seq| {
        let mut placed = 0;
        for _ in 0..count * 10 {
            if placed == count {
                break;
            }
            let p = Vec2::new(rng.range(-half, half), rng.range(-half, half));
            let w = rng.range(size.0, size.1);
            let l = w * aspect;
            if !clear_of_blocks(arena, p, 0.1 + w.max(l) * 0.5) {
                continue;
            }
            let yaw = rng.range(0.0, std::f32::consts::TAU);
            s.put(commands, k, Transform::from_xyz(p.x, 0.006, p.y).with_rotation(Quat::from_rotation_y(yaw)).with_scale(Vec3::new(w, 0.1, l)));
            placed += 1;
        }
    };
    ground(commands, s, Kind::Manhole, 5, (0.9, 0.9), 1.0, rng);
    ground(commands, s, Kind::Tyres, 10, (1.0, 1.3), 3.5, rng);
}

/// Safety paint: hazard stripes round the foot of the hall's columns and the furnace, along the
/// ladle car's flanks on the floor, and a warning plate beside every doorway with a frame.
fn signage(commands: &mut Commands, arena: &Arena, s: &mut Sink, faces: &[Face]) {
    for b in &arena.blocks {
        let column = b.skin == Skin::Cabinet && b.height > 4.0 && b.half.x.max(b.half.y) <= 1.0;
        let furnace = b.kind == BlockKind::Tall && b.skin == Skin::Concrete && b.half.x > 2.5 && (b.half.x - b.half.y).abs() < 0.01 && (b.height - 6.0).abs() < 0.01;
        let ladle_car = b.skin == Skin::Container && b.kind == BlockKind::Tall && (b.height - 3.0).abs() < 0.01 && (b.half.x - 1.3).abs() < 0.01;
        let c = Vec3::new(b.center.x, 0.0, b.center.y);
        if column || furnace {
            // A 1.2 m (columns) / 0.4 m (furnace) striped band on each face.
            let band = if column { 1.2 } else { 0.4 };
            for (n, hn, len) in [(Vec3::X, b.half.x, b.half.y * 2.0), (-Vec3::X, b.half.x, b.half.y * 2.0), (Vec3::Z, b.half.y, b.half.x * 2.0), (-Vec3::Z, b.half.y, b.half.x * 2.0)] {
                let at = c + n * (hn - 0.0145) + Vec3::Y * (band * 0.5);
                s.put_u(commands, Kind::Hazard, Transform::from_translation(at).with_rotation(wall_rot(n)).with_scale(Vec3::new(len, 0.1, band)), [0.0, len / 0.6]);
            }
        }
        if ladle_car {
            for side in [-1.0f32, 1.0] {
                let len = b.half.y * 2.0 + 0.6;
                let at = c + Vec3::X * side * (b.half.x + 0.35);
                let rot = Quat::from_mat3(&Mat3::from_cols(Vec3::Z, Vec3::Y, -Vec3::X));
                s.put_u(commands, Kind::Hazard, Transform::from_translation(at).with_rotation(rot).with_scale(Vec3::new(len, 0.1, 0.22)), [0.0, len / 0.6]);
            }
        }
    }
    // Warning plates at 1.7 m beside doorways (on the wall face the header sits in).
    for h in &arena.decor.headers {
        if h.bottom > 3.5 {
            continue;
        }
        for f in faces {
            let t = Vec3::Y.cross(f.n);
            let end = f.at + t * (f.len * 0.5);
            let mid = Vec3::new((h.min.x + h.max.x) * 0.5, 0.0, (h.min.y + h.max.y) * 0.5);
            // The face whose positive end touches this opening, facing out of its gap's sides.
            if end.distance(mid) > (h.max.x - h.min.x).max(h.max.y - h.min.y) * 0.5 + 0.8 || f.len < 1.5 || f.height < 2.2 {
                continue;
            }
            let at = end - t * 0.6 + Vec3::Y * 1.7;
            s.put(commands, Kind::Warning, Transform::from_translation(at).with_rotation(wall_rot(f.n)).with_scale(Vec3::new(0.4, 0.1, 0.36)));
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_tiles_and_stays_in_range() {
        for i in 0..50 {
            let y = i as f32 * 0.13;
            let a = fbm(0.0, y, 8, 3);
            let b = fbm(8.0, y, 8, 3);
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
            assert!((0.0..=1.0).contains(&a));
        }
    }

    #[test]
    fn strip_is_dirty_at_the_floor_and_clear_at_the_top() {
        let img = strip_image(1.0);
        let d = img.data.as_ref().unwrap();
        let (w, h) = (256usize, 64usize);
        let row_alpha = |y: usize| (0..w).map(|x| d[(y * w + x) * 4 + 3] as f32).sum::<f32>() / w as f32;
        eprintln!("alpha top {} mid {} bottom {}", row_alpha(0), row_alpha(h / 2), row_alpha(h - 1));
        assert!(row_alpha(h - 1) > 120.0, "bottom {}", row_alpha(h - 1));
        assert!(row_alpha(h * 3 / 4) > 60.0, "lower quarter {}", row_alpha(h * 3 / 4));
        assert!(row_alpha(0) < 30.0, "top {}", row_alpha(0));
    }

    #[test]
    fn every_map_has_wall_faces_inside_the_arena() {
        for id in MapId::ALL {
            let arena = Arena::build(id);
            let f = faces(&arena);
            assert!(f.len() > 8, "{id:?}: {}", f.len());
            assert!(f.iter().all(|f| f.at.x.abs() <= arena.half() + 0.02 && f.at.z.abs() <= arena.half() + 0.02));
            let m = strip_mesh(&f, 0.8);
            assert_eq!(m.count_vertices(), f.len() * 4);
        }
    }
}
