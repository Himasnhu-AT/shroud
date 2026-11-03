//! Per-map look: material palettes (PBR with flat-colour fallbacks), how each block is
//! drawn, and the map's mood as an engine [`Look`] (sun, sky, fog, exposure, grading), plus
//! the lamp shadow budget.

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use brain::arena::{Arena, Block, BlockKind, BuildingUse, FloorSkin, MapId, Skin};
use engine::look::{AmbientLook, ExposureLook, FogLook, FogMode, GradeSection, GradingLook, Look, Rgb, SkyLook, SkyMode, SunLook, ToneMap};
use engine::render_budget::SceneryRanges;

use super::{ArenaRes, Gfx};
use crate::gfx::load_pbr;
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
        "concrete" => spec("concrete_4k", [0.95, 0.91, 0.84], [0.56, 0.55, 0.52], 0.9, 3.0),
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
        // Township: brick, painted render in four colours, corrugated cladding, paving.
        "town_ground" => spec("floor", [0.80, 0.77, 0.72], [0.46, 0.45, 0.43], 1.0, 4.0),
        "town_wall" => spec("brick", [0.78, 0.70, 0.66], [0.45, 0.30, 0.26], 0.95, 1.0),
        "brick" => spec("brick", [1.0, 0.94, 0.90], [0.55, 0.31, 0.25], 0.95, 0.9),
        "plaster0" => spec("stucco", [0.66, 0.78, 0.90], [0.55, 0.64, 0.74], 1.0, 3.0),
        // Peeling yellow paint over plaster (the set is already yellow: a light tint only).
        "plaster1" => spec("peeling_paint", [1.0, 0.97, 0.9], [0.86, 0.74, 0.46], 1.0, 3.0),
        "plaster2" => spec("stucco", [0.72, 0.86, 0.66], [0.58, 0.70, 0.52], 1.0, 3.0),
        "plaster3" => spec("stucco", [0.98, 0.93, 0.85], [0.84, 0.80, 0.72], 1.0, 3.0),
        "siding" => spec("container", [0.62, 0.68, 0.70], [0.46, 0.50, 0.52], 0.55, 2.4),
        "siding_b" => spec("container", [0.78, 0.74, 0.62], [0.58, 0.55, 0.46], 0.55, 2.4),
        "town_concrete" => spec("wall", [0.90, 0.88, 0.84], [0.62, 0.61, 0.58], 0.9, 2.5),
        "sidewalk" => spec("sidewalk", [0.92, 0.90, 0.86], [0.56, 0.55, 0.52], 1.0, 2.5),
        "wood_floor" => spec("wood_floor", [0.95, 0.88, 0.80], [0.46, 0.33, 0.22], 0.8, 2.0),
        "concrete_floor" => spec("floor", [0.95, 0.94, 0.92], [0.55, 0.55, 0.54], 0.7, 4.0),
        "shelf" => spec("crate", [0.80, 0.80, 0.82], [0.45, 0.46, 0.48], 0.8, 1.2),
        "plaster_in_house" => spec("office_wall", [0.98, 0.95, 0.88], [0.86, 0.83, 0.77], 1.0, 2.5),
        "plaster_in_shop" => spec("office_wall", [0.96, 0.97, 0.98], [0.86, 0.87, 0.88], 1.0, 2.5),
        "plaster_in_office" => spec("office_wall", [0.86, 0.90, 0.94], [0.76, 0.80, 0.84], 1.0, 2.5),
        "plaster_in_shed" => spec("wall", [0.88, 0.88, 0.86], [0.60, 0.60, 0.58], 0.9, 2.5),
        "ceiling_house" => spec("office_wall", [1.0, 0.99, 0.96], [0.90, 0.89, 0.86], 1.0, 2.5),
        "ceiling_shed" => spec("container", [0.70, 0.72, 0.74], [0.50, 0.52, 0.54], 0.6, 2.4),
        "canopy" => spec("", [1.0, 1.0, 1.0], [0.86, 0.86, 0.84], 0.5, 2.0),
        // Sirocco: whitewash in three tones, warm earth, cobbles, terracotta.
        "sirocco_ground" => spec("depot_ground", [1.0, 0.93, 0.82], [0.78, 0.68, 0.52], 1.0, 5.0),
        "white0" => spec("stucco", [1.0, 0.99, 0.96], [0.90, 0.88, 0.84], 1.0, 3.0),
        "white1" => spec("stucco", [0.99, 0.94, 0.84], [0.88, 0.82, 0.72], 1.0, 3.0),
        // Bare sandstone ashlar for every third house and the town's outer walls.
        "white2" => spec("sandstone", [1.08, 1.0, 0.92], [0.84, 0.70, 0.50], 1.0, 4.0),
        "sirocco_outer" => spec("sandstone", [0.98, 0.92, 0.84], [0.76, 0.66, 0.52], 1.0, 4.0),
        "cobble" => spec("cobblestone", [1.12, 1.04, 0.94], [0.62, 0.55, 0.46], 1.0, 2.4),
        "sirocco_paving" => spec("sidewalk", [1.0, 0.94, 0.84], [0.70, 0.64, 0.56], 1.0, 2.5),
        "terracotta_floor" => spec("tiles", [0.92, 0.58, 0.42], [0.66, 0.38, 0.26], 1.0, 1.2),
        "roof_terracotta" => spec("tiles", [0.74, 0.50, 0.40], [0.56, 0.36, 0.28], 0.9, 0.8),
        // Gantry: weathered harbour concrete, painted cladding, ballast, crane paint.
        // Wet asphalt after the night's rain; cast concrete quay walls (4K sets).
        "harbor_ground" => spec("asphalt_wet", [1.0, 1.0, 1.0], [0.22, 0.23, 0.24], 1.0, 5.0),
        "harbor_concrete" => spec("concrete_4k", [0.92, 0.93, 0.95], [0.54, 0.55, 0.56], 0.9, 3.0),
        "harbor_wall" => spec("concrete_4k", [0.74, 0.76, 0.79], [0.42, 0.43, 0.45], 0.9, 3.5),
        "siding_teal" => spec("container", [0.34, 0.55, 0.56], [0.22, 0.38, 0.40], 0.55, 2.4),
        "siding_rust" => spec("container", [0.66, 0.38, 0.26], [0.48, 0.26, 0.17], 0.6, 2.4),
        "gravel" => spec("roof_gravel", [0.86, 0.83, 0.80], [0.40, 0.38, 0.36], 1.0, 2.0),
        "crane_paint" => spec("tall", [1.0, 0.74, 0.16], [0.82, 0.58, 0.10], 0.5, 2.0),
        _ => spec("", [1.0, 1.0, 1.0], [0.6, 0.6, 0.6], 0.9, 2.0),
    }
}

/// Materials and shared meshes for one map build, created on first use.
pub(crate) struct Palette {
    pub id: MapId,
    /// The map's half extent (outer walls on ±half).
    pub half: f32,
    /// Wall signs as (centre, half width, half height): facade windows keep clear of them.
    signs: Vec<(Vec3, f32, f32)>,
    cache: HashMap<String, (Handle<StandardMaterial>, f32)>,
    cube: Option<Handle<Mesh>>,
    /// Block index → index of the building whose shell it is.
    pub shell_of: HashMap<usize, usize>,
    /// Building index → its facade colour variant.
    variant: Vec<usize>,
    /// Shared prop meshes by key (shape + size), so props of one kind and size share a handle.
    shapes: HashMap<String, Handle<Mesh>>,
}

impl Palette {
    pub fn new(_g: &mut Gfx, arena: &Arena) -> Self {
        let mut shell_of = HashMap::new();
        for (bi, b) in arena.decor.buildings.iter().enumerate() {
            for &i in &b.shell {
                shell_of.insert(i, bi);
            }
        }
        // Plaster houses cycle through the colours in build order.
        let mut n = 0;
        let variant = arena
            .decor
            .buildings
            .iter()
            .map(|b| {
                let v = n;
                if b.facade == Skin::Plaster {
                    n += 1;
                }
                v
            })
            .collect();
        let signs = arena
            .decor
            .signs
            .iter()
            .filter(|s| matches!(s.mount, brain::arena::SignMount::Wall { .. }))
            .map(|s| (Vec3::new(s.pos.x, s.y, s.pos.y), (s.text.chars().count() as f32 * 0.8 + 0.7) * s.size * 0.5, s.size * 0.75))
            .collect();
        Palette { id: arena.id, half: arena.half(), signs, cache: HashMap::new(), cube: None, shell_of, variant, shapes: HashMap::new() }
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

    /// A mesh made once per `key` (include the size in it) and shared by every prop that asks.
    pub fn shape(&mut self, g: &mut Gfx, key: &str, make: impl FnOnce() -> Mesh) -> Handle<Mesh> {
        if let Some(h) = self.shapes.get(key) {
            return h.clone();
        }
        let h = g.meshes.add(make());
        self.shapes.insert(key.to_string(), h.clone());
        h
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
            MapId::Township => "town_ground",
            MapId::Sirocco => "sirocco_ground",
            MapId::Gantry => "harbor_ground",
        };
        self.mat(g, key)
    }

    /// Material key of a floor patch, with per-map variants.
    pub fn floor_patch_key(&self, f: FloorSkin) -> String {
        match (self.id, f) {
            (MapId::Sirocco, FloorSkin::Pavement) => "cobble".into(),
            (MapId::Sirocco, FloorSkin::Sidewalk) => "sirocco_paving".into(),
            (MapId::Sirocco, FloorSkin::Tiles) => "terracotta_floor".into(),
            _ => super::dressing::floor_key(f).into(),
        }
    }

    /// Lintel material for a doorway: Office drywall, else concrete (Township picks the wall's
    /// own material in `map.rs`).
    pub fn header(&mut self, g: &mut Gfx) -> (Handle<StandardMaterial>, f32) {
        let key = match self.id {
            MapId::Office => "drywall",
            MapId::Sirocco => "white1",
            MapId::Gantry => "harbor_concrete",
            _ => "concrete",
        };
        self.mat(g, key)
    }

    pub fn roof(&mut self, g: &mut Gfx) -> (Handle<StandardMaterial>, f32) {
        let key = match self.id {
            MapId::Office => "ceiling",
            MapId::Township => "canopy",
            MapId::Sirocco => "roof_terracotta",
            MapId::Gantry => "crane_paint",
            _ => "roof_depot",
        };
        self.mat(g, key)
    }

    /// Ceiling (roof underside) material of a building.
    pub fn ceiling_key(use_: BuildingUse) -> &'static str {
        match use_ {
            BuildingUse::House => "ceiling_house",
            BuildingUse::Shop | BuildingUse::Office => "ceiling",
            BuildingUse::Warehouse | BuildingUse::Workshop => "ceiling_shed",
        }
    }

    /// Interior wall finish of a building.
    pub fn lining_key(use_: BuildingUse) -> &'static str {
        match use_ {
            BuildingUse::House => "plaster_in_house",
            BuildingUse::Shop => "plaster_in_shop",
            BuildingUse::Office => "plaster_in_office",
            BuildingUse::Warehouse | BuildingUse::Workshop => "plaster_in_shed",
        }
    }

    /// Facade material key of building `bi`.
    pub fn facade_key(&self, bi: usize, skin: Skin) -> String {
        match skin {
            Skin::Plaster => format!("plaster{}", self.variant.get(bi).copied().unwrap_or(0) % 4),
            Skin::Siding => if bi.is_multiple_of(2) { "siding" } else { "siding_b" }.into(),
            Skin::Brick => "brick".into(),
            Skin::Concrete if self.id == MapId::Township => "town_concrete".into(),
            Skin::Concrete if self.id == MapId::Gantry => "harbor_concrete".into(),
            Skin::Concrete => "concrete".into(),
            Skin::Stucco if self.id == MapId::Sirocco => "white0".into(),
            Skin::Stucco => "stucco".into(),
            _ => "town_concrete".into(),
        }
    }

    pub(crate) fn block_key(&self, i: usize, b: &Block) -> String {
        if let Some(&bi) = self.shell_of.get(&i) {
            return self.facade_key(bi, b.skin);
        }
        match b.skin {
            Skin::Auto => match b.kind {
                BlockKind::Wall => "foundry_wall",
                BlockKind::Tall => "foundry_tall",
                BlockKind::Low => "foundry_low",
            }
            .into(),
            Skin::Concrete if self.id == MapId::Township => "town_concrete".into(),
            Skin::Concrete if self.id == MapId::Gantry && b.kind == BlockKind::Wall => "harbor_wall".into(),
            Skin::Concrete if self.id == MapId::Gantry => "harbor_concrete".into(),
            Skin::Concrete => "concrete".into(),
            Skin::Stucco if self.id == MapId::Sirocco && b.kind == BlockKind::Wall => "sirocco_outer".into(),
            // Low garden walls take the creamy tone; houses cycle through all three.
            Skin::Stucco if self.id == MapId::Sirocco && b.kind == BlockKind::Low => "white1".into(),
            Skin::Stucco if self.id == MapId::Sirocco => format!("white{}", i % 3),
            Skin::Stucco if b.kind == BlockKind::Wall => "stucco_outer".into(),
            Skin::Stucco => if i.is_multiple_of(2) { "stucco" } else { "stucco_b" }.into(),
            Skin::Container => format!("container{}", i % 4),
            Skin::Crate => "crate".into(),
            Skin::Barrel if self.id == MapId::Office => "planter".into(),
            Skin::Barrel => "barrel".into(),
            Skin::Sandbag => "sandbag".into(),
            Skin::Drywall if b.kind == BlockKind::Wall => "drywall_outer".into(),
            Skin::Drywall => "drywall".into(),
            Skin::Desk if is_cubicle(b) && self.id == MapId::Office => "cubicle".into(),
            Skin::Desk => "desk".into(),
            Skin::Cabinet if is_crane(b) => "crane_paint".into(),
            Skin::Cabinet => "cabinet".into(),
            Skin::Brick if b.kind == BlockKind::Wall => "town_wall".into(),
            Skin::Brick => "brick".into(),
            Skin::Plaster => "plaster3".into(),
            Skin::Siding if self.id == MapId::Gantry => if i.is_multiple_of(2) { "siding_teal" } else { "siding_rust" }.into(),
            Skin::Siding => "siding".into(),
            // Cars and dumpsters get their own meshes; the box material is a fallback.
            Skin::Car => "cabinet".into(),
            Skin::Dumpster => "dumpster".into(),
            Skin::Shelf => "shelf".into(),
            Skin::Planter if self.id == MapId::Sirocco => "white1".into(),
            Skin::Planter => "planter".into(),
        }
    }
}

/// Tall steel posts (crane legs): painted, not a rack or a cabinet.
fn is_crane(b: &Block) -> bool {
    b.height > 4.0 && b.half.x.max(b.half.y) <= 1.0
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
/// `walls` / `small` collect the plain box of a block into map-wide meshes, one per material
/// (shadow-casting / not): hundreds of unique world-UV boxes otherwise each cost a draw in every
/// view (main, prepass and up to ~25 shadow views) and can't be instanced.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dress_block(commands: &mut Commands, e: Entity, i: usize, b: &Block, size: Vec3, center: Vec3, g: &mut Gfx, p: &mut Palette, walls: &mut super::dressing::Batch, small: &mut super::dressing::Batch) {
    let detail = g.quality != GraphicsQuality::Low;
    // Props drawn from parts instead of their box (the block keeps only its collider).
    if matches!(b.skin, Skin::Car | Skin::Dumpster | Skin::Shelf) {
        commands.entity(e).insert(Visibility::default());
    }
    match b.skin {
        Skin::Car => {
            super::props::car(commands, e, i, b, size, g, p);
            return;
        }
        Skin::Dumpster => {
            super::props::dumpster(commands, e, size, g, p);
            return;
        }
        Skin::Shelf => {
            super::props::shelf(commands, e, size, g, p, detail);
            return;
        }
        _ => {}
    }
    let key = p.block_key(i, b);
    let (mat, _) = p.mat(g, &key);
    // Small props don't cast shadows (perf rule); walls and big cover do.
    let small_prop = b.kind == BlockKind::Low && size.x.max(size.z) < 1.6;
    if b.skin == Skin::Barrel {
        let (r, h) = (barrel_radius(b), b.height);
        let mesh = p.shape(g, &format!("barrel {r:.3} {h:.3}"), || Cylinder::new(r, h).mesh().resolution(20).into());
        commands.entity(e).insert((Mesh3d(mesh), MeshMaterial3d(mat.clone())));
        if small_prop {
            commands.entity(e).insert(NotShadowCaster);
        }
    } else {
        // The box joins a merged mesh; the block keeps its collider and parents its trims.
        if small_prop { small.add(g, p, &key, size, center) } else { walls.add(g, p, &key, size, center) }
        commands.entity(e).insert(Visibility::default());
    }
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
        // Steel drums: rolling hoops, rolled rims and a lid with bungs.
        (Skin::Barrel, _) if p.id != MapId::Office => super::props::barrel(commands, e, barrel_radius(b), size.y, &mat, g, p),
        // A big square planter is a fountain: water on top.
        (Skin::Planter, _) if b.half.x.min(b.half.y) > 0.9 => {
            let m = p.flat(g, "water", || StandardMaterial { base_color: Color::srgb(0.05, 0.12, 0.14), perceptual_roughness: 0.03, reflectance: 0.9, ..default() });
            child_box(commands, e, p.cube(g), m, Vec3::Y * (size.y * 0.5 + 0.004), Vec3::new(size.x - 0.3, 0.01, size.z - 0.3));
        }
        // Office planters and Township planters: a shrub on top (visual only; above cover height).
        (Skin::Barrel, _) | (Skin::Planter, _) => {
            let m = p.flat(g, "shrub", || StandardMaterial { base_color: Color::srgb(0.16, 0.32, 0.14), perceptual_roughness: 0.9, ..default() });
            let r = b.half.x.min(b.half.y).min(0.6);
            let shrub = p.shape(g, &format!("shrub {r:.3}"), || Sphere::new(r * 1.05).mesh().ico(3).unwrap_or_else(|_| Sphere::new(r * 1.05).into()));
            commands.spawn((Mesh3d(shrub), MeshMaterial3d(m), Transform::from_xyz(0.0, size.y * 0.5 + r * 0.45, 0.0).with_scale(Vec3::new(1.0, 0.9, 1.0)), NotShadowCaster, ChildOf(e)));
        }
        // Depot buildings: cornice cap, and windows on tall facades.
        (Skin::Stucco, BlockKind::Tall) => {
            let (cap, _) = p.mat(g, "concrete");
            child_box(commands, e, p.cube(g), cap, Vec3::new(0.0, size.y * 0.5 + 0.025, 0.0), Vec3::new(size.x + 0.2, 0.25, size.z + 0.2));
            if detail && size.y >= 5.0 {
                windows(commands, e, size, center, g, p);
            }
        }
        // Interior partitions: skirting boards along both long faces.
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
        (Skin::Cabinet, _) if is_crane(b) => {}
        (Skin::Cabinet, BlockKind::Tall) if detail && size.y >= 2.1 => rack_fronts(commands, e, size, g, p),
        // Lockers, fridges, vending machines and counters: door or drawer fronts with handles.
        (Skin::Cabinet, _) if detail => super::props::cabinet_fronts(commands, e, size, g, p),
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

pub(crate) fn child_box(commands: &mut Commands, parent: Entity, mesh: Handle<Mesh>, mat: Handle<StandardMaterial>, at: Vec3, scale: Vec3) {
    commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat), Transform::from_translation(at).with_scale(scale), NotShadowCaster, ChildOf(parent)));
}

/// Rows of dark glass windows with sills on a Depot building's long faces (above head height).
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
                if face.abs() > p.half - 1.1 {
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
                // Keep painted signs readable: no window over a sign plate.
                let world = center + pos + Vec3::Y * y;
                let flat = |v: Vec3| Vec2::new(v.x, v.z);
                if p.signs.iter().any(|&(c, hw, hh)| flat(c).distance(flat(world)) < hw + 0.8 && (c.y - world.y).abs() < hh + 0.8) {
                    continue;
                }
                child_box(commands, e, cube.clone(), glass.clone(), pos + Vec3::Y * y, scale);
                child_box(commands, e, cube.clone(), sill.clone(), sill_pos + Vec3::Y * (y - 0.79), sill_scale);
            }
        }
    }
}

// ---------------------------------------------------------------- the map's mood

/// Sun from a direction *towards* the sun (as the maps were first authored).
fn sun_towards(to_sun: Vec3, base: SunLook) -> SunLook {
    let s = to_sun.normalize();
    SunLook { azimuth_deg: s.x.atan2(-s.z).to_degrees().rem_euclid(360.0), elevation_deg: s.y.asin().to_degrees(), ..base }
}

/// The engine [`Look`] for a map. Arenas want short shadows (they are ≤ 90 m across) and
/// three cascades; the fog end bounds what's drawn ([`fog_end`]).
pub fn look_of(id: MapId) -> Look {
    let base = Look::clear_day();
    let arena_sun = SunLook { shadow_distance: 70.0, cascades: 3, first_cascade: 9.0, ..SunLook::default() };
    let fog = |color: [f32; 3], start: f32, end: f32| FogLook {
        mode: FogMode::Linear,
        color: Rgb::new(color[0], color[1], color[2]),
        sun_color: Rgb::new(1.0, 0.85, 0.65),
        sun_amount: 0.35,
        sun_exponent: 24.0,
        start,
        end,
        ..FogLook::default()
    };
    let neutral = GradingLook { post_saturation: 1.05, midtones: GradeSection { contrast: 1.04, ..GradeSection::default() }, ..GradingLook::default() };
    match id {
        MapId::Foundry => Look {
            name: "foundry".into(),
            sun: sun_towards(Vec3::new(-30.0, 22.0, 26.0), SunLook { illuminance: 110_000.0, color: Rgb::new(1.0, 0.93, 0.84), ..arena_sun }),
            ambient: AmbientLook { color: Rgb::new(0.7, 0.78, 0.95), brightness: 800.0, env_intensity: 1.0 },
            fog: fog([0.58, 0.62, 0.70], 30.0, fog_end(id)),
            exposure: ExposureLook { ev100: 13.0, ..ExposureLook::default() },
            tonemapping: ToneMap::Agx,
            grading: neutral,
            ..base
        },
        // High, hot desert sun behind the attackers; dusty warm haze.
        MapId::Depot => Look {
            name: "depot".into(),
            sun: sun_towards(Vec3::new(-20.0, 38.0, 30.0), SunLook { illuminance: 110_000.0, color: Rgb::new(1.0, 0.94, 0.85), ..arena_sun }),
            ambient: AmbientLook { color: Rgb::new(0.86, 0.82, 0.78), brightness: 900.0, env_intensity: 1.0 },
            fog: fog([0.80, 0.74, 0.64], 40.0, fog_end(id)),
            exposure: ExposureLook { ev100: 13.2, ..ExposureLook::default() },
            tonemapping: ToneMap::Agx,
            grading: GradingLook { temperature: 0.03, ..neutral },
            ..base
        },
        // Indoors: no sun, ceiling lamps do the work. Exposure and fill are set for a lit
        // office (bright ceiling, light walls) without clipping the lamp panels.
        MapId::Office => Look {
            name: "office".into(),
            sun: SunLook { enabled: false, shadows: false, ..arena_sun },
            ambient: AmbientLook { color: Rgb::new(0.97, 0.95, 0.92), brightness: 400.0, env_intensity: 0.06 },
            fog: fog([0.30, 0.30, 0.32], 60.0, fog_end(id)),
            exposure: ExposureLook { ev100: 7.8, ..ExposureLook::default() },
            tonemapping: ToneMap::Agx,
            grading: GradingLook { midtones: GradeSection { contrast: 1.08, ..GradeSection::default() }, ..neutral },
            ..base
        },
        // Clear late afternoon: a warm sun from the west-south-west, low enough for long
        // shadows down Main Street, under a blue gradient sky (Foundry keeps the physical
        // atmosphere and a high white sun). Cool, slightly blue distance haze; a neutral fill
        // so plaster keeps its colour in shade.
        MapId::Township => Look {
            name: "township_afternoon".into(),
            sun: SunLook {
                azimuth_deg: 248.0,
                elevation_deg: 24.0,
                illuminance: 95_000.0,
                color: Rgb::new(1.0, 0.84, 0.64),
                shadow_distance: 80.0,
                cascades: 3,
                first_cascade: 10.0,
                ..SunLook::default()
            },
            sky: SkyLook {
                mode: SkyMode::Gradient,
                zenith: Rgb::new(0.06, 0.30, 0.85),
                horizon: Rgb::new(0.66, 0.80, 0.94),
                ground: Rgb::new(0.30, 0.28, 0.25),
                brightness: 7000.0,
                sun_glow: 0.8,
                ..SkyLook::default()
            },
            ambient: AmbientLook { color: Rgb::new(0.92, 0.90, 0.90), brightness: 900.0, env_intensity: 1.0 },
            fog: FogLook {
                sun_color: Rgb::new(1.0, 0.84, 0.62),
                sun_amount: 0.3,
                sun_exponent: 16.0,
                ..fog([0.66, 0.76, 0.88], 50.0, fog_end(id))
            },
            exposure: ExposureLook { ev100: 13.0, ..ExposureLook::default() },
            tonemapping: ToneMap::Agx,
            // AgX pushes saturated blues towards lavender: extra saturation keeps the sky blue.
            grading: GradingLook { post_saturation: 1.15, midtones: GradeSection { contrast: 1.06, ..GradeSection::default() }, ..GradingLook::default() },
            ..base
        },
        // Midday on a whitewashed hill town: a high, hot sun, deep blue sky, short hard
        // shadows, a pale warm haze. White walls carry the light, so exposure sits a touch high.
        MapId::Sirocco => Look {
            name: "sirocco_noon".into(),
            sun: SunLook {
                azimuth_deg: 205.0,
                elevation_deg: 56.0,
                illuminance: 110_000.0,
                color: Rgb::new(1.0, 0.95, 0.86),
                shadow_distance: 75.0,
                cascades: 3,
                first_cascade: 10.0,
                ..SunLook::default()
            },
            sky: SkyLook {
                mode: SkyMode::Gradient,
                zenith: Rgb::new(0.03, 0.22, 0.78),
                horizon: Rgb::new(0.72, 0.84, 0.96),
                ground: Rgb::new(0.44, 0.38, 0.30),
                brightness: 8000.0,
                sun_glow: 0.7,
                ..SkyLook::default()
            },
            ambient: AmbientLook { color: Rgb::new(0.88, 0.88, 0.90), brightness: 900.0, env_intensity: 1.0 },
            fog: FogLook { sun_color: Rgb::new(1.0, 0.9, 0.7), sun_amount: 0.25, ..fog([0.80, 0.84, 0.90], 60.0, fog_end(id)) },
            exposure: ExposureLook { ev100: 13.5, ..ExposureLook::default() },
            tonemapping: ToneMap::Agx,
            grading: GradingLook { temperature: 0.02, post_saturation: 1.15, midtones: GradeSection { contrast: 1.07, ..GradeSection::default() }, ..GradingLook::default() },
            ..base
        },
        // A grey sea morning: a low, weak sun from the water (east), a cool overcast sky,
        // a blue-grey haze that softens the far end of the yard; sodium lamps on the walls.
        MapId::Gantry => Look {
            name: "gantry_morning".into(),
            sun: SunLook {
                azimuth_deg: 105.0,
                elevation_deg: 30.0,
                illuminance: 55_000.0,
                color: Rgb::new(0.96, 0.95, 1.0),
                shadow_distance: 70.0,
                cascades: 3,
                first_cascade: 9.0,
                ..SunLook::default()
            },
            sky: SkyLook {
                mode: SkyMode::Gradient,
                zenith: Rgb::new(0.30, 0.40, 0.54),
                horizon: Rgb::new(0.70, 0.75, 0.80),
                ground: Rgb::new(0.26, 0.27, 0.28),
                brightness: 5000.0,
                sun_glow: 0.3,
                ..SkyLook::default()
            },
            ambient: AmbientLook { color: Rgb::new(0.80, 0.86, 0.96), brightness: 1100.0, env_intensity: 1.0 },
            fog: FogLook { sun_color: Rgb::new(0.95, 0.92, 0.85), sun_amount: 0.15, ..fog([0.60, 0.66, 0.72], 30.0, fog_end(id)) },
            exposure: ExposureLook { ev100: 12.4, ..ExposureLook::default() },
            tonemapping: ToneMap::Agx,
            grading: GradingLook { temperature: -0.03, post_saturation: 0.98, midtones: GradeSection { contrast: 1.06, ..GradeSection::default() }, ..GradingLook::default() },
            ..base
        },
    }
}

/// Where a map's fog is opaque (nothing is drawn beyond it).
pub fn fog_end(id: MapId) -> f32 {
    match id {
        MapId::Foundry => 180.0,
        MapId::Depot => 240.0,
        MapId::Office => 400.0,
        MapId::Township => 300.0,
        MapId::Sirocco => 260.0,
        MapId::Gantry => 200.0,
    }
}

/// Render-budget ranges that fit a map: nothing inside it fades, loses its shadow or goes
/// flat, even on Low (×0.6); scenery stops at the fog.
pub fn scenery_ranges(arena: &Arena) -> SceneryRanges {
    let diagonal = arena.half() * 2.0 * std::f32::consts::SQRT_2;
    let near = (diagonal / 0.6).max(160.0);
    SceneryRanges { shadow: near, small_props: near, far: near, view: fog_end(arena.id), ..default() }
}

/// The HDRI sky of a map (Poly Haven CC0, converted to a KTX2 cubemap under `assets/sky/`,
/// see `assets/CREDITS.md`): the skybox and the image-based light. The sun glow in each sky
/// was rotated to the map's sun azimuth and the sun itself clamped out (the `DirectionalLight`
/// is the sun), so shadows and sky agree. Maps without one keep their gradient/atmosphere.
pub fn hdri_of(id: MapId) -> Option<&'static str> {
    match id {
        MapId::Sirocco => Some("sky/sirocco_kloofendal_48d.ktx2"),
        MapId::Gantry => Some("sky/gantry_overcast_soil.ktx2"),
        // Indoors: never seen, but the reflections and fill of a real interior.
        MapId::Office => Some("sky/office_factory_canteen.ktx2"),
        _ => None,
    }
}

/// [`look_of`] with the map's HDRI sky when its cubemap is present (otherwise the procedural
/// sky stays: the game runs without assets).
pub fn look_with_sky(id: MapId, has: impl Fn(&str) -> bool) -> Look {
    let mut l = look_of(id);
    if let Some(path) = hdri_of(id).filter(|p| has(p)) {
        l.sky.mode = SkyMode::Cubemap;
        l.sky.cubemap = path.into();
        // Same cd/m² per unit as the gradient: the cubemaps are normalised to a sky mean of ~0.55.
        l.sky.brightness = match id {
            MapId::Sirocco => 9000.0,
            MapId::Gantry => 5200.0,
            // Office: only its reflections and fill (it is never seen); kept dim.
            _ => 2500.0,
        };
        l.name = format!("{}_hdri", l.name).into();
    }
    l
}

/// Keeps the engine `Look` and the render-budget ranges on the current map.
/// Bodycam mode swaps in the map's bodycam look (`bodycam::lens::bodycam_look`).
pub(crate) fn sync_look(
    arena: Res<ArenaRes>,
    settings: Res<crate::settings::ShroudSettings>,
    look: Option<Res<Look>>,
    dir: Option<Res<crate::assets::AssetsDir>>,
    mut ranges: ResMut<SceneryRanges>,
    mut commands: Commands,
) {
    let mut want = look_with_sky(arena.0.id, |p| dir.as_ref().is_some_and(|d| d.has(p)));
    if crate::bodycam::is_bodycam(&settings) {
        want = crate::bodycam::lens::bodycam_look(want);
    }
    if look.is_none_or(|l| l.name != want.name) {
        commands.insert_resource(want);
    }
    let r = scenery_ranges(&arena.0);
    if *ranges != r {
        *ranges = r;
    }
}

/// A map lamp; the nearest few cast shadows (see [`apply_light_budget`]).
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct MapLamp;

/// Shadow budget for map lamps at a quality level: (spot shadows, point-light cube shadows).
/// Spot shadows are layers of the sun's shadow texture (same resolution), and a cube costs six
/// views, so both stay small. The picks go to the lamps nearest the camera (see [`pick_shadowed`]).
pub(crate) fn shadow_budget(q: GraphicsQuality) -> (usize, usize) {
    match q {
        GraphicsQuality::Low => (0, 0),
        GraphicsQuality::Medium => (2, 0),
        GraphicsQuality::High => (4, 0),
        GraphicsQuality::Ultra => (6, 1),
    }
}

/// Lamp shadow budget after the governor's game level: level 1 keeps two thirds of the spots and
/// drops the point cube, level 2 and up keeps a third.
pub(crate) fn governed(k_spot: usize, k_point: usize, game_level: u8) -> (usize, usize) {
    match game_level {
        0 => (k_spot, k_point),
        1 => (k_spot * 2 / 3, 0),
        _ => (k_spot / 3, 0),
    }
}

/// Lamps farther than this from the camera never get a shadow (a lamp's range is 12-14 m).
const LAMP_SHADOW_DIST: f32 = 25.0;
/// A shadowed lamp keeps its slot until it is this much farther than a challenger (hysteresis,
/// so shadows don't swap back and forth while the player stands between two lamps).
const SWAP_MARGIN: f32 = 1.25;
/// How often the picks are re-ranked.
const RERANK_S: f32 = 0.25;

/// Choose up to `k` lamps to shadow from `(lamp, distance)` pairs: nearest first, but lamps that
/// already have a shadow (`current`) keep it unless a candidate is clearly closer. Lamps beyond
/// `max_d` are dropped (with the same margin for ones already shadowed).
pub(crate) fn pick_shadowed(current: &[usize], dists: &[(usize, f32)], k: usize, max_d: f32) -> Vec<usize> {
    if k == 0 {
        return Vec::new();
    }
    let held = |i: usize| current.contains(&i);
    // Held lamps compete as if they were closer by the margin.
    let mut ranked: Vec<(usize, f32)> = dists
        .iter()
        .filter(|&&(i, d)| d <= if held(i) { max_d * SWAP_MARGIN } else { max_d })
        .map(|&(i, d)| (i, if held(i) { d / SWAP_MARGIN } else { d }))
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut out: Vec<usize> = ranked.into_iter().take(k).map(|(i, _)| i).collect();
    out.sort_unstable();
    out
}

/// Gives the lamp shadow budget to the lamps nearest the camera, re-ranked a few times a second
/// with hysteresis (Unreal and Godot both size local-light shadows by how close or big the light
/// is on screen; authoring order put shadows on far corners of the map while nearby lamps had none).
pub(crate) fn apply_light_budget(
    time: Res<Time<Real>>,
    settings: Option<Res<Settings>>,
    added: Query<(), Added<MapLamp>>,
    mut shot_out: Query<(Option<&mut SpotLight>, Option<&mut PointLight>), (With<MapLamp>, With<super::lamps::LampOut>, Added<super::lamps::LampOut>)>,
    mut relit: RemovedComponents<super::lamps::LampOut>,
    cam: Query<&GlobalTransform, With<engine::gfx::MainCamera>>,
    mut spots: Query<(Entity, &GlobalTransform, &mut SpotLight), (With<MapLamp>, Without<super::lamps::LampOut>)>,
    mut points: Query<(Entity, &GlobalTransform, &mut PointLight), (With<MapLamp>, Without<SpotLight>, Without<super::lamps::LampOut>)>,
    governor: Option<Res<engine::render_budget::GovernorState>>,
    mut next: Local<f32>,
) {
    let now = time.elapsed_secs();
    // Shot-out lamps leave the candidates (`map::lamps`): re-rank at once so the next lamp in
    // line takes the freed shadow slot.
    let mut lamps_changed = relit.read().count() > 0;
    for (spot, point) in &mut shot_out {
        lamps_changed = true;
        // (Off again in case a re-rank gave it a shadow the frame it broke.)
        if let Some(mut l) = spot.filter(|l| l.shadow_maps_enabled) {
            l.shadow_maps_enabled = false;
        }
        if let Some(mut l) = point.filter(|l| l.shadow_maps_enabled) {
            l.shadow_maps_enabled = false;
        }
    }
    let changed = settings.as_ref().is_some_and(|s| s.is_changed()) || !added.is_empty() || lamps_changed || governor.as_ref().is_some_and(|g| g.is_changed());
    if !changed && now < *next {
        return;
    }
    *next = now + RERANK_S;
    let Some(eye) = cam.iter().next().map(|t| t.translation()) else {
        return;
    };
    let (k_spot, k_point) = shadow_budget(settings.map(|s| s.quality).unwrap_or_default());
    // The engine governor's game steps (`GovernorStep::Game(n)`) trim lamp shadows under load.
    let (k_spot, k_point) = governed(k_spot, k_point, governor.map_or(0, |g| g.game_level));
    // Entities index the candidates; `index()` is stable for a lamp's lifetime.
    let rank = |q: &mut dyn Iterator<Item = (Entity, Vec3, bool)>, k: usize| -> Vec<Entity> {
        let all: Vec<(Entity, Vec3, bool)> = q.collect();
        let current: Vec<usize> = all.iter().enumerate().filter(|(_, c)| c.2).map(|(i, _)| i).collect();
        let dists: Vec<(usize, f32)> = all.iter().enumerate().map(|(i, c)| (i, c.1.distance(eye))).collect();
        pick_shadowed(&current, &dists, k, LAMP_SHADOW_DIST).into_iter().map(|i| all[i].0).collect()
    };
    let on_spots = rank(&mut spots.iter().map(|(e, t, l)| (e, t.translation(), l.shadow_maps_enabled)), k_spot);
    for (e, _, mut l) in &mut spots {
        let on = on_spots.contains(&e);
        if l.shadow_maps_enabled != on {
            l.shadow_maps_enabled = on;
        }
    }
    let on_points = rank(&mut points.iter().map(|(e, t, l)| (e, t.translation(), l.shadow_maps_enabled)), k_point);
    for (e, _, mut l) in &mut points {
        let on = on_points.contains(&e);
        if l.shadow_maps_enabled != on {
            l.shadow_maps_enabled = on;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamp_shadows_go_to_the_nearest_with_hysteresis() {
        let d = [(0, 3.0), (1, 10.0), (2, 5.0), (3, 30.0)];
        // Nearest two, far one excluded.
        assert_eq!(pick_shadowed(&[], &d, 2, 25.0), vec![0, 2]);
        assert_eq!(pick_shadowed(&[], &d, 9, 25.0), vec![0, 1, 2]);
        assert!(pick_shadowed(&[0], &d, 0, 25.0).is_empty());
        // Lamp 1 holds its slot against lamp 2 that is only a little closer...
        let near = [(1, 6.0), (2, 5.5)];
        assert_eq!(pick_shadowed(&[1], &near, 1, 25.0), vec![1]);
        // ...but loses it to one that is clearly closer.
        let far = [(1, 9.0), (2, 5.0)];
        assert_eq!(pick_shadowed(&[1], &far, 1, 25.0), vec![2]);
        // A held lamp slightly past the cutoff keeps its shadow; a new one doesn't get one.
        assert_eq!(pick_shadowed(&[3], &[(3, 27.0)], 1, 25.0), vec![3]);
        assert!(pick_shadowed(&[], &[(3, 27.0)], 1, 25.0).is_empty());
    }

    #[test]
    fn governor_game_levels_trim_lamp_shadows() {
        assert_eq!(governed(6, 1, 0), (6, 1));
        assert_eq!(governed(6, 1, 1), (4, 0));
        assert_eq!(governed(6, 1, 2), (2, 0));
        assert_eq!(governed(0, 0, 2), (0, 0));
    }

    #[test]
    fn sun_conversion_matches_the_authored_direction() {
        let s = sun_towards(Vec3::new(-30.0, 22.0, 26.0), SunLook::default());
        let travel = s.direction();
        assert!(travel.dot(-Vec3::new(-30.0, 22.0, 26.0).normalize()) > 0.999, "{travel:?}");
    }

    #[test]
    fn every_map_has_a_look_and_ranges_cover_it() {
        for id in MapId::ALL {
            let l = look_of(id);
            assert!(l.fog.end > l.fog.start);
            let r = scenery_ranges(&Arena::build(id));
            assert!(r.far * 0.6 >= Arena::build(id).half() * 2.0 * std::f32::consts::SQRT_2 - 1.0);
            assert_eq!(r.view, fog_end(id));
            // Without the sky files the procedural sky stays; with them, the HDRI cubemap.
            assert_eq!(look_with_sky(id, |_| false).sky.mode, l.sky.mode);
            if let Some(p) = hdri_of(id) {
                let h = look_with_sky(id, |_| true);
                assert_eq!((h.sky.mode, h.sky.cubemap.as_ref()), (SkyMode::Cubemap, p));
            }
        }
    }
}
