//! Per-map look: material palettes (PBR with flat-colour fallbacks), how each block is
//! drawn, and the map's light rig (sun, ambient, exposure, fog, lamp shadow budget).

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::camera::Exposure;
use bevy::light::{AtmosphereEnvironmentMapLight, NotShadowCaster, light_consts::lux};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use brain::arena::{Block, BlockKind, MapId, Skin};

use super::{ArenaRes, Gfx};
use crate::gfx::{FOG_COLOR, Sun, load_pbr, world_uv_cuboid};
use engine::settings::{CoreSettings as Settings, GraphicsQuality};

/// A material recipe: texture set under `assets/textures/<tex>/`, tint, flat fallback
/// colour and roughness (used when the textures are missing), UV repeat in metres.
struct Spec {
    tex: &'static str,
    tint: Color,
    fallback: Color,
    rough: f32,
    tile: f32,
}

const fn spec(tex: &'static str, tint: [f32; 3], fallback: [f32; 3], rough: f32, tile: f32) -> Spec {
    Spec {
        tex,
        tint: Color::srgb(tint[0], tint[1], tint[2]),
        fallback: Color::srgb(fallback[0], fallback[1], fallback[2]),
        rough,
        tile,
    }
}

fn recipe(key: &str) -> Spec {
    match key {
        // Foundry (the original look).
        "foundry_floor" => spec("floor", [0.62, 0.62, 0.62], [0.32, 0.33, 0.34], 0.95, 4.0),
        "foundry_wall" => spec("wall", [0.42, 0.45, 0.50], [0.22, 0.23, 0.26], 0.9, 3.0),
        "foundry_tall" => spec("tall", [0.62, 0.72, 0.72], [0.48, 0.30, 0.20], 0.6, 2.0),
        "foundry_low" => spec("wall", [0.95, 0.80, 0.55], [0.70, 0.58, 0.25], 0.8, 1.5),
        // Depot: sand, sun-bleached render, painted containers, timber crates.
        "sand" => spec("depot_ground", [0.98, 0.90, 0.78], [0.74, 0.64, 0.48], 1.0, 5.0),
        "stucco" => spec("stucco", [0.97, 0.87, 0.70], [0.84, 0.73, 0.57], 1.0, 3.0),
        "stucco_b" => spec("stucco", [0.93, 0.76, 0.62], [0.80, 0.61, 0.48], 1.0, 3.0),
        "stucco_outer" => spec("stucco", [0.82, 0.74, 0.63], [0.66, 0.58, 0.48], 1.0, 3.0),
        "concrete" => spec("wall", [0.82, 0.79, 0.74], [0.56, 0.55, 0.52], 0.9, 2.5),
        "container0" => spec("container", [0.70, 0.26, 0.18], [0.55, 0.20, 0.14], 0.6, 2.6),
        "container1" => spec("container", [0.25, 0.42, 0.62], [0.18, 0.30, 0.46], 0.6, 2.6),
        "container2" => spec("container", [0.32, 0.50, 0.34], [0.22, 0.36, 0.24], 0.6, 2.6),
        "container3" => spec("container", [0.86, 0.52, 0.20], [0.70, 0.40, 0.14], 0.6, 2.6),
        "crate" => spec("crate", [0.95, 0.85, 0.70], [0.52, 0.38, 0.24], 1.0, 1.2),
        "sandbag" => spec("wall", [0.80, 0.71, 0.55], [0.58, 0.50, 0.38], 1.0, 1.0),
        "barrel" => spec("tall", [0.40, 0.52, 0.70], [0.20, 0.30, 0.45], 0.5, 1.0),
        "asphalt" => spec("asphalt", [0.85, 0.85, 0.85], [0.17, 0.17, 0.18], 1.0, 4.0),
        "pavement" => spec("floor", [0.86, 0.82, 0.76], [0.50, 0.48, 0.45], 1.0, 3.0),
        "roof_depot" => spec("wall", [0.62, 0.60, 0.57], [0.40, 0.39, 0.37], 0.9, 3.0),
        // Office: carpet, painted plaster, ceiling tiles, wood desks, steel cabinets.
        "carpet" => spec("carpet", [0.60, 0.64, 0.70], [0.30, 0.33, 0.38], 1.0, 2.0),
        "tiles" => spec("tiles", [0.90, 0.90, 0.88], [0.78, 0.78, 0.76], 1.0, 1.8),
        "drywall" => spec("office_wall", [0.94, 0.92, 0.88], [0.82, 0.80, 0.76], 1.0, 2.5),
        "drywall_outer" => spec("office_wall", [0.74, 0.79, 0.84], [0.62, 0.66, 0.70], 1.0, 2.5),
        "desk" => spec("crate", [0.88, 0.74, 0.56], [0.50, 0.38, 0.26], 1.0, 1.6),
        // Cubicle pods: fabric partition panels.
        "cubicle" => spec("carpet", [0.52, 0.57, 0.64], [0.36, 0.40, 0.46], 1.0, 1.0),
        "cabinet" => spec("", [1.0, 1.0, 1.0], [0.07, 0.075, 0.085], 0.45, 1.5),
        "planter" => spec("wall", [0.36, 0.36, 0.38], [0.25, 0.25, 0.27], 0.9, 1.0),
        "ceiling" => spec("ceiling", [1.0, 1.0, 1.0], [0.85, 0.85, 0.83], 1.0, 2.4),
        _ => spec("", [1.0, 1.0, 1.0], [0.6, 0.6, 0.6], 0.9, 2.0),
    }
}

/// Materials and shared meshes for one map build, created on first use.
pub(crate) struct Palette {
    pub id: MapId,
    cache: HashMap<String, (Handle<StandardMaterial>, f32)>,
    cube: Option<Handle<Mesh>>,
}

impl Palette {
    pub fn new(_g: &mut Gfx, id: MapId) -> Self {
        Palette { id, cache: HashMap::new(), cube: None }
    }

    /// A material from [`recipe`], cached by key.
    pub fn mat(&mut self, g: &mut Gfx, key: &str) -> (Handle<StandardMaterial>, f32) {
        if let Some(m) = self.cache.get(key) {
            return m.clone();
        }
        let s = recipe(key);
        let m = load_pbr(g.server, g.dir, g.pending, s.tex, s.tint).unwrap_or(StandardMaterial {
            base_color: s.fallback,
            perceptual_roughness: s.rough,
            ..default()
        });
        let out = (g.mats.add(m), s.tile);
        self.cache.insert(key.to_string(), out.clone());
        out
    }

    /// An untextured material, cached by key.
    pub fn flat(&mut self, g: &mut Gfx, key: &str, make: impl FnOnce() -> StandardMaterial) -> Handle<StandardMaterial> {
        if let Some((m, _)) = self.cache.get(key) {
            return m.clone();
        }
        let h = g.mats.add(make());
        self.cache.insert(key.to_string(), (h.clone(), 1.0));
        h
    }

    /// Self-lit material (exposure-independent: emissive alpha 0).
    pub fn glow(&mut self, g: &mut Gfx, key: &str, c: LinearRgba) -> Handle<StandardMaterial> {
        self.flat(g, key, || StandardMaterial { base_color: Color::BLACK, emissive: c, ..default() })
    }

    /// Unit cube, scaled per instance (for small props whose UVs don't matter).
    pub fn cube(&mut self, g: &mut Gfx) -> Handle<Mesh> {
        self.cube.get_or_insert_with(|| g.meshes.add(Cuboid::new(1.0, 1.0, 1.0))).clone()
    }

    pub fn floor(&mut self, g: &mut Gfx) -> (Handle<StandardMaterial>, f32) {
        let key = match self.id {
            MapId::Foundry => "foundry_floor",
            MapId::Depot => "sand",
            MapId::Office => "carpet",
        };
        self.mat(g, key)
    }

    pub fn header(&mut self, g: &mut Gfx) -> (Handle<StandardMaterial>, f32) {
        let key = match self.id {
            MapId::Office => "drywall",
            _ => "concrete",
        };
        self.mat(g, key)
    }

    pub fn roof(&mut self, g: &mut Gfx) -> (Handle<StandardMaterial>, f32) {
        let key = match self.id {
            MapId::Office => "ceiling",
            _ => "roof_depot",
        };
        self.mat(g, key)
    }

    fn block_key(&self, i: usize, b: &Block) -> String {
        match b.skin {
            Skin::Auto => match b.kind {
                BlockKind::Wall => "foundry_wall",
                BlockKind::Tall => "foundry_tall",
                BlockKind::Low => "foundry_low",
            }
            .into(),
            Skin::Concrete => "concrete".into(),
            Skin::Stucco if b.kind == BlockKind::Wall => "stucco_outer".into(),
            Skin::Stucco => if i.is_multiple_of(2) { "stucco" } else { "stucco_b" }.into(),
            Skin::Container => format!("container{}", i % 4),
            Skin::Crate => "crate".into(),
            Skin::Barrel if self.id == MapId::Office => "planter".into(),
            Skin::Barrel => "barrel".into(),
            Skin::Sandbag => "sandbag".into(),
            Skin::Drywall if b.kind == BlockKind::Wall => "drywall_outer".into(),
            Skin::Drywall => "drywall".into(),
            Skin::Desk if is_cubicle(b) => "cubicle".into(),
            Skin::Desk => "desk".into(),
            Skin::Cabinet => "cabinet".into(),
        }
    }
}

/// Office cubicle pods and long desks (fabric partitions, monitors on top).
fn is_cubicle(b: &Block) -> bool {
    let (w, d) = (b.half.x.max(b.half.y) * 2.0, b.half.x.min(b.half.y) * 2.0);
    b.kind == BlockKind::Low && d >= 1.2 && w > d + 0.1 && w <= 4.0
}

/// Barrels are cylinders; everything else is its box.
pub(crate) fn block_collider(b: &Block) -> Collider {
    match b.skin {
        Skin::Barrel => Collider::cylinder(barrel_radius(b), b.height),
        _ => Collider::cuboid(b.half.x * 2.0, b.height, b.half.y * 2.0),
    }
}

fn barrel_radius(b: &Block) -> f32 {
    b.half.x.min(b.half.y) * 0.95
}

/// Mesh, material and trims for block `i` (entity `e`, already spawned with its collider).
pub(crate) fn dress_block(commands: &mut Commands, e: Entity, i: usize, b: &Block, size: Vec3, center: Vec3, g: &mut Gfx, p: &mut Palette) {
    let (mat, tile) = p.mat(g, &p.block_key(i, b));
    let mesh = match b.skin {
        Skin::Barrel => g.meshes.add(Cylinder::new(barrel_radius(b), b.height)),
        _ => g.meshes.add(world_uv_cuboid(size, center, tile)),
    };
    commands.entity(e).insert((Mesh3d(mesh), MeshMaterial3d(mat)));
    let detail = g.quality != GraphicsQuality::Low;
    match (b.skin, b.kind) {
        // Foundry: glowing amber lip on low cover, cyan strip along the outer walls.
        (Skin::Auto, BlockKind::Low) => {
            let m = p.glow(g, "trim_amber", LinearRgba::new(5.0, 2.2, 0.4, 0.0));
            child_box(commands, e, p.cube(g), m, Vec3::new(0.0, size.y * 0.5 - 0.0175, 0.0), Vec3::new(size.x + 0.02, 0.035, size.z + 0.02));
        }
        (Skin::Auto, BlockKind::Wall) => {
            let m = p.glow(g, "trim_cyan", LinearRgba::new(0.4, 2.6, 4.0, 0.0));
            child_box(commands, e, p.cube(g), m, Vec3::new(0.0, -size.y * 0.5 + 0.35, 0.0), Vec3::new(size.x + 0.02, 0.05, size.z + 0.02));
        }
        // Barrels: a darker rim band near the top.
        (Skin::Barrel, _) if p.id == MapId::Depot => {
            let m = p.flat(g, "barrel_rim", || StandardMaterial { base_color: Color::srgb(0.12, 0.13, 0.15), perceptual_roughness: 0.5, metallic: 0.6, ..default() });
            let r = barrel_radius(b);
            let ring = g.meshes.add(Cylinder::new(r + 0.012, 0.05));
            for y in [size.y * 0.5 - 0.12, -size.y * 0.5 + 0.2] {
                commands.spawn((Mesh3d(ring.clone()), MeshMaterial3d(m.clone()), Transform::from_xyz(0.0, y, 0.0), NotShadowCaster, ChildOf(e)));
            }
        }
        // Office planters: soil and a shrub on top (visual only; above cover height).
        (Skin::Barrel, _) => {
            let m = p.flat(g, "shrub", || StandardMaterial { base_color: Color::srgb(0.16, 0.32, 0.14), perceptual_roughness: 0.9, ..default() });
            let shrub = g.meshes.add(Sphere::new(barrel_radius(b) * 1.1));
            commands.spawn((Mesh3d(shrub), MeshMaterial3d(m), Transform::from_xyz(0.0, size.y * 0.5 + 0.25, 0.0).with_scale(Vec3::new(1.0, 1.3, 1.0)), ChildOf(e)));
        }
        // Depot buildings: cornice cap, and windows on tall facades.
        (Skin::Stucco, BlockKind::Tall) => {
            let (cap, _) = p.mat(g, "concrete");
            child_box(commands, e, p.cube(g), cap, Vec3::new(0.0, size.y * 0.5 + 0.025, 0.0), Vec3::new(size.x + 0.2, 0.25, size.z + 0.2));
            if detail && size.y >= 5.0 {
                windows(commands, e, size, center, g, p);
            }
        }
        // Office walls: skirting boards along both long faces.
        (Skin::Drywall, BlockKind::Tall) => {
            let m = p.flat(g, "skirting", || StandardMaterial { base_color: Color::srgb(0.16, 0.16, 0.17), perceptual_roughness: 0.6, ..default() });
            // Different heights for x- and z-running walls so crossing boards never share a top face.
            let along_x = size.x >= size.z;
            let h = if along_x { 0.10 } else { 0.11 };
            let s = if along_x { Vec3::new(size.x - 0.02, h, size.z + 0.024) } else { Vec3::new(size.x + 0.024, h, size.z - 0.02) };
            child_box(commands, e, p.cube(g), m, Vec3::new(0.0, -size.y * 0.5 + h * 0.5, 0.0), s);
        }
        // Office desks: monitors on the cubicle pods (visual only, no collider).
        (Skin::Desk, BlockKind::Low) if detail && p.id == MapId::Office && is_cubicle(b) => {
            let frame = p.flat(g, "monitor", || StandardMaterial { base_color: Color::srgb(0.03, 0.03, 0.035), perceptual_roughness: 0.35, ..default() });
            let screen = p.glow(g, "screen", LinearRgba::new(0.10, 0.20, 0.38, 0.0));
            // Two monitors per side, back to back along the pod's long axis.
            let along_x = size.x >= size.z;
            let (long, short) = if along_x { (size.x, size.z) } else { (size.z, size.x) };
            let place = |a: f32, c: f32| if along_x { Vec3::new(a, 0.0, c) } else { Vec3::new(c, 0.0, a) };
            let dims = |w: f32, h: f32, d: f32| if along_x { Vec3::new(w, h, d) } else { Vec3::new(d, h, w) };
            for sa in [-0.25f32, 0.25] {
                for sc in [-1.0f32, 1.0] {
                    let base = place(sa * long, sc * short * 0.25) + Vec3::Y * size.y * 0.5;
                    child_box(commands, e, p.cube(g), frame.clone(), base + Vec3::Y * 0.22, dims(0.52, 0.32, 0.04));
                    child_box(commands, e, p.cube(g), screen.clone(), base + Vec3::Y * 0.22 + place(0.0, -sc * 0.024), dims(0.48, 0.28, 0.005));
                    child_box(commands, e, p.cube(g), frame.clone(), base + Vec3::Y * 0.03, dims(0.06, 0.06, 0.06));
                }
            }
        }
        // Server racks / shelving: vented door panels with status LEDs.
        (Skin::Cabinet, BlockKind::Tall) if detail && size.y >= 2.1 => rack_fronts(commands, e, size, g, p),
        _ => {}
    }
}

fn rack_fronts(commands: &mut Commands, e: Entity, size: Vec3, g: &mut Gfx, p: &mut Palette) {
    let panel = p.flat(g, "rack_panel", || StandardMaterial { base_color: Color::srgb(0.13, 0.135, 0.15), perceptual_roughness: 0.35, metallic: 0.7, ..default() });
    let green = p.glow(g, "led_green", LinearRgba::new(0.2, 3.0, 0.6, 0.0));
    let blue = p.glow(g, "led_blue", LinearRgba::new(0.3, 0.9, 3.5, 0.0));
    let cube = p.cube(g);
    let along_x = size.x >= size.z;
    let (long, short) = if along_x { (size.x, size.z) } else { (size.z, size.x) };
    let n = (long / 0.62).floor().max(1.0) as i32;
    let step = long / n as f32;
    for k in 0..n {
        let a = -long * 0.5 + step * (k as f32 + 0.5);
        for side in [-1.0f32, 1.0] {
            let at = |depth: f32, y: f32| if along_x { Vec3::new(a, y, side * (short * 0.5 + depth)) } else { Vec3::new(side * (short * 0.5 + depth), y, a) };
            let dims = |w: f32, h: f32, d: f32| if along_x { Vec3::new(w, h, d) } else { Vec3::new(d, h, w) };
            child_box(commands, e, cube.clone(), panel.clone(), at(0.008, 0.05), dims(step - 0.06, size.y - 0.3, 0.016));
            for (j, y) in [0.55f32, 0.15, -0.3].into_iter().enumerate() {
                let m = if (k + j as i32) % 3 == 0 { blue.clone() } else { green.clone() };
                child_box(commands, e, cube.clone(), m, at(0.019, y), dims(0.18, 0.02, 0.006));
            }
        }
    }
}

fn child_box(commands: &mut Commands, parent: Entity, mesh: Handle<Mesh>, mat: Handle<StandardMaterial>, at: Vec3, scale: Vec3) {
    commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat), Transform::from_translation(at).with_scale(scale), NotShadowCaster, ChildOf(parent)));
}

/// Rows of dark glass windows with sills on a building's long faces (above head height).
fn windows(commands: &mut Commands, e: Entity, size: Vec3, center: Vec3, g: &mut Gfx, p: &mut Palette) {
    let glass = p.flat(g, "glass", || StandardMaterial {
        base_color: Color::srgb(0.06, 0.08, 0.10),
        perceptual_roughness: 0.08,
        metallic: 0.0,
        reflectance: 0.6,
        ..default()
    });
    let (sill, _) = p.mat(g, "concrete");
    let cube = p.cube(g);
    for (axis_len, other, along_x) in [(size.x, size.z, true), (size.z, size.x, false)] {
        let n = ((axis_len - 1.5) / 3.2).floor() as i32;
        if n < 1 {
            continue;
        }
        let span = (n - 1) as f32 * 3.2;
        for k in 0..n {
            let t = -span * 0.5 + k as f32 * 3.2;
            for side in [-1.0f32, 1.0] {
                // Skip faces pressed against the outer wall.
                let face = if along_x { center.z + side * other * 0.5 } else { center.x + side * other * 0.5 };
                if face.abs() > 30.9 {
                    continue;
                }
                let off = other * 0.5 + 0.02;
                let (pos, scale, sill_pos, sill_scale) = if along_x {
                    (Vec3::new(t, 0.0, side * off), Vec3::new(1.2, 1.5, 0.04), Vec3::new(t, 0.0, side * (other * 0.5 + 0.06)), Vec3::new(1.4, 0.08, 0.12))
                } else {
                    (Vec3::new(side * off, 0.0, t), Vec3::new(0.04, 1.5, 1.2), Vec3::new(side * (other * 0.5 + 0.06), 0.0, t), Vec3::new(0.12, 0.08, 1.4))
                };
                let y = size.y * 0.5 - 1.8;
                if y - 0.75 < -size.y * 0.5 + 2.6 {
                    continue;
                }
                child_box(commands, e, cube.clone(), glass.clone(), pos + Vec3::Y * y, scale);
                child_box(commands, e, cube.clone(), sill.clone(), sill_pos + Vec3::Y * (y - 0.79), sill_scale);
            }
        }
    }
}

// ---------------------------------------------------------------- light rig

/// A map's lighting: sun, sky fill, exposure, fog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapLook {
    /// Where the sun shines from (direction towards the sun), or `None` indoors.
    pub sun_from: Option<Vec3>,
    pub sun_lux: f32,
    pub sun_color: Color,
    pub ambient: Color,
    pub ambient_brightness: f32,
    pub ev100: f32,
    pub env_intensity: f32,
    pub fog: Color,
    pub fog_start: f32,
    pub fog_end: f32,
}

impl MapLook {
    pub fn of(id: MapId) -> Self {
        match id {
            MapId::Foundry => MapLook {
                sun_from: Some(Vec3::new(-30.0, 22.0, 26.0)),
                sun_lux: lux::RAW_SUNLIGHT,
                sun_color: Color::srgb(1.0, 0.93, 0.84),
                ambient: Color::srgb(0.7, 0.78, 0.95),
                ambient_brightness: 800.0,
                ev100: 13.0,
                env_intensity: 1.0,
                fog: FOG_COLOR,
                fog_start: 30.0,
                fog_end: 180.0,
            },
            // High, hot desert sun behind the attackers; dusty warm haze.
            MapId::Depot => MapLook {
                sun_from: Some(Vec3::new(-20.0, 38.0, 30.0)),
                sun_lux: lux::RAW_SUNLIGHT,
                sun_color: Color::srgb(1.0, 0.94, 0.85),
                ambient: Color::srgb(0.86, 0.82, 0.78),
                ambient_brightness: 900.0,
                ev100: 13.2,
                env_intensity: 1.0,
                fog: Color::srgb(0.80, 0.74, 0.64),
                fog_start: 40.0,
                fog_end: 240.0,
            },
            // Indoors: no sun, ceiling lamps do the work, exposure set for interiors.
            MapId::Office => MapLook {
                sun_from: None,
                sun_lux: 0.0,
                sun_color: Color::WHITE,
                ambient: Color::srgb(0.95, 0.93, 0.90),
                ambient_brightness: 150.0,
                ev100: 8.6,
                env_intensity: 0.08,
                fog: Color::srgb(0.30, 0.30, 0.32),
                fog_start: 60.0,
                fog_end: 400.0,
            },
        }
    }
}

/// Keeps the sun, ambient light and camera exposure/fog on the current map's look.
/// Cheap: only writes what differs (other code may re-add camera components).
pub(crate) fn apply_look(
    arena: Res<ArenaRes>,
    ambient: Option<ResMut<GlobalAmbientLight>>,
    mut suns: Query<(&mut DirectionalLight, &mut Transform), With<Sun>>,
    mut cams: Query<(Option<&mut Exposure>, Option<&mut DistanceFog>, Option<&mut AtmosphereEnvironmentMapLight>), With<Camera3d>>,
) {
    let look = MapLook::of(arena.0.id);
    for (mut sun, mut tf) in &mut suns {
        let shadows = look.sun_from.is_some();
        if sun.illuminance != look.sun_lux || sun.shadow_maps_enabled != shadows || sun.color != look.sun_color {
            sun.illuminance = look.sun_lux;
            sun.shadow_maps_enabled = shadows;
            sun.color = look.sun_color;
        }
        if let Some(from) = look.sun_from {
            let want = Transform::from_translation(from).looking_at(Vec3::ZERO, Vec3::Y);
            if tf.rotation.angle_between(want.rotation) > 1e-3 {
                *tf = want;
            }
        }
    }
    if let Some(mut a) = ambient
        && (a.color != look.ambient || a.brightness != look.ambient_brightness)
    {
        a.color = look.ambient;
        a.brightness = look.ambient_brightness;
    }
    for (exposure, fog, env) in &mut cams {
        if let Some(mut x) = exposure
            && x.ev100 != look.ev100
        {
            x.ev100 = look.ev100;
        }
        if let Some(mut f) = fog {
            let falloff_ok = matches!(f.falloff, FogFalloff::Linear { start, end } if start == look.fog_start && end == look.fog_end);
            if f.color != look.fog || !falloff_ok {
                f.color = look.fog;
                f.falloff = FogFalloff::Linear { start: look.fog_start, end: look.fog_end };
            }
        }
        if let Some(mut e) = env
            && e.intensity != look.env_intensity
        {
            e.intensity = look.env_intensity;
        }
    }
}

/// A map lamp; the first `budget` (by `rank`) cast shadows.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct MapLamp {
    pub rank: usize,
}

/// How many map lamps may cast shadows at a quality level.
pub(crate) fn shadow_budget(q: GraphicsQuality) -> usize {
    match q {
        GraphicsQuality::Low => 0,
        GraphicsQuality::Medium => 3,
        GraphicsQuality::High => 6,
        GraphicsQuality::Ultra => 10,
    }
}

/// Applies the shadow budget when lamps spawn or the quality setting changes.
pub(crate) fn apply_light_budget(
    settings: Option<Res<Settings>>,
    added: Query<(), Added<MapLamp>>,
    mut spots: Query<(&MapLamp, &mut SpotLight)>,
    mut points: Query<(&MapLamp, &mut PointLight)>,
) {
    let changed = settings.as_ref().is_some_and(|s| s.is_changed());
    if !changed && added.is_empty() {
        return;
    }
    let budget = shadow_budget(settings.map(|s| s.quality).unwrap_or_default());
    for (lamp, mut l) in &mut spots {
        let on = lamp.rank < budget;
        if l.shadow_maps_enabled != on {
            l.shadow_maps_enabled = on;
        }
    }
    // Point-light shadows cost six renders each: only on Ultra, and only the first two.
    for (lamp, mut l) in &mut points {
        let on = budget >= 10 && lamp.rank < 2;
        if l.shadow_maps_enabled != on {
            l.shadow_maps_enabled = on;
        }
    }
}
