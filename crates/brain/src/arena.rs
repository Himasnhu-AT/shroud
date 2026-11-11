//! The arena layout, shared by the game (which builds meshes + colliders from it)
//! and the offline trainer (which simulates on it in 2D).

use crate::math::V2;
use crate::nav::NavGrid;

/// Height of low cover: you can shoot over it standing, hide behind it crouched.
pub const LOW_COVER_HEIGHT: f32 = 1.1;
pub const TALL_HEIGHT: f32 = 3.6;
/// Default half extent of a map (Depot, Office and the test yard are 64 m squares). Each map has its
/// own size: use [`Arena::half`]. Outer walls are centred on ±half, inner faces at
/// ±(half - WALL_THICKNESS).
pub const ARENA_HALF: f32 = 32.0;
/// Half extent covered by the AI's fixed world grids (player-model heat/routes, squad belief):
/// big enough for the largest map. Every map's `half()` must be ≤ this.
pub const GRID_HALF: f32 = 48.0;
pub const WALL_THICKNESS: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// Outer boundary wall.
    Wall,
    /// Blocks sight and bullets.
    Tall,
    /// Crouch cover: blocks sight only to/from a crouched agent.
    Low,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub center: V2,
    pub half: V2,
    pub height: f32,
    pub kind: BlockKind,
    /// How the renderer dresses it. No gameplay effect.
    pub skin: Skin,
}

/// Visual hint for the renderer (material and shape). Gameplay only looks at `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Skin {
    /// Pick from the kind (Foundry's look).
    #[default]
    Auto,
    /// Bare concrete (walls, pillars, barriers).
    Concrete,
    /// Sun-bleached rendered masonry (Depot buildings).
    Stucco,
    /// Corrugated shipping container / truck box.
    Container,
    /// Wooden crate or crate stack.
    Crate,
    /// Steel drum, drawn as a cylinder inside the block.
    Barrel,
    /// Sandbag / jersey barrier.
    Sandbag,
    /// Painted interior wall (Office).
    Drywall,
    /// Office desk / counter / table.
    Desk,
    /// Server rack, shelf or vending machine: dark painted metal.
    Cabinet,
    /// Red brick facade (also garden walls when Low).
    Brick,
    /// Painted render on houses; the renderer varies the colour per building.
    Plaster,
    /// Painted corrugated metal cladding (warehouses, workshops).
    Siding,
    /// Parked car (Low: shoot over the bonnet, hide behind it crouched).
    Car,
    /// Store shelving / workbench (Low or Tall).
    Shelf,
    /// Steel dumpster.
    Dumpster,
    /// Concrete planter with a shrub.
    Planter,
}

impl Block {
    fn new(cx: f32, cz: f32, hx: f32, hz: f32, kind: BlockKind) -> Self {
        let height = match kind {
            BlockKind::Low => LOW_COVER_HEIGHT,
            _ => TALL_HEIGHT,
        };
        Block { center: V2::new(cx, cz), half: V2::new(hx, hz), height, kind, skin: Skin::Auto }
    }

    /// Box from its ground-plane extents.
    pub(crate) fn rect(x0: f32, z0: f32, x1: f32, z1: f32, kind: BlockKind, height: f32, skin: Skin) -> Self {
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (z0, z1) = (z0.min(z1), z0.max(z1));
        Block {
            center: V2::new((x0 + x1) * 0.5, (z0 + z1) * 0.5),
            half: V2::new((x1 - x0) * 0.5, (z1 - z0) * 0.5),
            height,
            kind,
            skin,
        }
    }

    pub fn contains(&self, p: V2, pad: f32) -> bool {
        (p.x - self.center.x).abs() <= self.half.x + pad && (p.y - self.center.y).abs() <= self.half.y + pad
    }

    /// Does segment a→b pass through this box? (slab test)
    pub fn intersects_segment(&self, a: V2, b: V2) -> bool {
        let min = self.center - self.half;
        let max = self.center + self.half;
        let d = b - a;
        let mut t0 = 0.0f32;
        let mut t1 = 1.0f32;
        for (p, dp, lo, hi) in [(a.x, d.x, min.x, max.x), (a.y, d.y, min.y, max.y)] {
            if dp.abs() < 1e-9 {
                if p < lo || p > hi {
                    return false;
                }
            } else {
                let inv = 1.0 / dp;
                let (mut ta, mut tb) = ((lo - p) * inv, (hi - p) * inv);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }

    /// Length (m) of segment a->b inside this box (0 if it misses).
    pub fn segment_inside(&self, a: V2, b: V2) -> f32 {
        let min = self.center - self.half;
        let max = self.center + self.half;
        let d = b - a;
        let mut t0 = 0.0f32;
        let mut t1 = 1.0f32;
        for (p, dp, lo, hi) in [(a.x, d.x, min.x, max.x), (a.y, d.y, min.y, max.y)] {
            if dp.abs() < 1e-9 {
                if p < lo || p > hi {
                    return 0.0;
                }
            } else {
                let inv = 1.0 / dp;
                let (mut ta, mut tb) = ((lo - p) * inv, (hi - p) * inv);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
                if t0 > t1 {
                    return 0.0;
                }
            }
        }
        (t1 - t0).max(0.0) * d.len()
    }
}

/// A named rectangle used to describe player habits in plain words.
#[derive(Clone, Debug)]
pub struct Zone {
    pub name: &'static str,
    pub min: V2,
    pub max: V2,
}

/// A spot next to a block that an agent can hide behind.
#[derive(Clone, Copy, Debug)]
pub struct CoverPoint {
    pub pos: V2,
    pub block: usize,
}

/// Which map to play. CONTRACT: variants, `ALL`, `label`, `blurb` and `Arena::build` are
/// shared by the maps, AI and UI streams. Every map fits inside ±`Arena::half()` (≤ GRID_HALF,
/// which the player-model heat grids cover) and has at least 6 enemy spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MapId {
    /// Defusal layout in a steelworks: A in the casting hall, B at the loading docks, the ladle
    /// track between the boiler house and the scale house.
    #[default]
    Foundry,
    /// Medium-range outdoor defusal map around a hub: A in the coach yard, B in the roofed
    /// cistern hall, the arcade in the middle.
    Depot,
    /// Tight indoor close-quarters defusal map: A in the server room, B in the records room.
    Office,
    /// A 90 m town block: streets, alleys, a courtyard and eight enterable buildings.
    Township,
    /// Defusal layout in a whitewashed hill town, two lanes and a church: the high street to A,
    /// the dog-leg to B, the church between the procession road and the bell arch square.
    Sirocco,
    /// Defusal layout in a harbour rail yard: an indoor A site in a warehouse, parked trains
    /// across the rail yard, B on the open quay.
    Gantry,
}

impl MapId {
    /// Menu order: the bomb-site maps first. Saves store the variant name, so order is free.
    pub const ALL: [MapId; 6] = [MapId::Sirocco, MapId::Gantry, MapId::Depot, MapId::Township, MapId::Office, MapId::Foundry];
    pub fn label(self) -> &'static str {
        match self {
            MapId::Foundry => "Foundry",
            MapId::Depot => "Depot",
            MapId::Office => "Ledger",
            MapId::Township => "Township",
            MapId::Sirocco => "Sirocco",
            MapId::Gantry => "Gantry",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            MapId::Foundry => "Bomb sites in a steelworks. A in the casting hall, B at the loading docks, a ladle car on the track between them.",
            MapId::Depot => "Bomb sites at a sun-baked bus depot. A in the coach yard, B in the roofed cistern hall; the freight lane, the arcade and the gatehouse meet at the forecourt.",
            MapId::Office => "Bomb sites on a records-office floor. A in the server room, B in the records room; tight corridors, every corner a fight.",
            MapId::Township => "Bomb sites in a town block on a late afternoon. A in the warehouse loading yard, B in the office courtyard; long streets, alleys, houses with windows.",
            MapId::Sirocco => "Bomb sites in a whitewashed hill town. A on the fountain plaza, B in the market; the high street, the church between the procession road and the bell arch, the dog-leg, the cellars.",
            MapId::Gantry => "Bomb sites in a harbour rail yard. A inside the warehouse, parked trains across the rail yard, B out on the quay.",
        }
    }
}

pub struct Arena {
    pub id: MapId,
    /// Half extent: outer walls are centred on ±half (see [`Arena::half`]).
    pub half: f32,
    pub blocks: Vec<Block>,
    pub zones: Vec<Zone>,
    pub player_spawn: V2,
    pub enemy_spawns: Vec<V2>,
    pub nav: NavGrid,
    pub cover: Vec<CoverPoint>,
    /// Visual-only dressing (roofs, lamps, door headers, floor patches). No gameplay effect.
    pub decor: Decor,
    /// Live smoke clouds (the game keeps this up to date): they block [`Arena::line_of_sight`].
    pub smokes: Vec<crate::grenade::SmokeCloud>,
    /// Lamps shot out this round (indices into `decor.lamps`, the lamp ids). The game keeps it
    /// up to date and clears it at round start; the doctrine reads it (`Arena::lamps_lighting`).
    pub dark_lamps: Vec<usize>,
    /// Bomb sites (A first, then B). Every map has two; see [`crate::objective`].
    pub sites: Vec<crate::objective::BombSite>,
}

/// Agent collision radius used for nav inflation.
pub const AGENT_RADIUS: f32 = 0.45;

/// Width of a doorway cut by the map builder. With 1 m nav cells and AGENT_RADIUS
/// inflation this leaves two walkable cells when the door is centred on a whole metre.
pub const DOOR_WIDTH: f32 = 2.2;
/// Height of the lintel underside above a doorway.
pub const DOOR_HEIGHT: f32 = 2.25;
/// Half thickness of interior partition walls.
pub const PARTITION_HALF: f32 = 0.15;
/// Office: interior wall height, and the ceiling underside just below it (walls poke
/// into the ceiling slab so there are no light-leaking seams or coplanar faces).
pub const OFFICE_WALL_HEIGHT: f32 = 3.25;
pub const OFFICE_CEILING: f32 = 3.2;

/// Renderer-only layout data. The 2D AI never reads it, so nothing here may block movement
/// at standing height (lintels start at `DOOR_HEIGHT`, roofs and lamps are overhead).
#[derive(Clone, Debug, Default)]
pub struct Decor {
    pub roofs: Vec<Roof>,
    pub lamps: Vec<Lamp>,
    /// Lintels over doorways: ground rect plus the vertical span.
    pub headers: Vec<Header>,
    /// Floor surface patches drawn just above the ground (non-overlapping).
    pub floors: Vec<FloorPatch>,
    /// Window openings: a Low sill block below (gameplay), glass + lintel above (the lintel
    /// is bullet-solid, the glass stops nothing). Not in `headers`.
    pub windows: Vec<Window>,
    /// Enterable buildings, for the renderer (facades, linings, roofs, probes).
    pub buildings: Vec<BuildingInfo>,
    /// Painted lettering: site letters ("A", "B") and direction signs ("A >", "< B").
    pub signs: Vec<Sign>,
    /// Painted floor markings (spawn borders, site outlines, rails set into the ground). Drawn
    /// above floor patches; may overlap them but not each other.
    pub paint: Vec<PaintPatch>,
}

/// Paint colour of a sign or floor marking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Paint {
    White,
    Yellow,
    Red,
    Blue,
    Orange,
    Black,
    /// Dark steel (rails set into the ground).
    Steel,
}

/// Where a sign is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignMount {
    /// On a wall face; `facing` is the face's outward normal (unit ground axis).
    Wall { facing_x: i8, facing_z: i8 },
    /// On the floor; the tops of the letters point along `up` (unit ground axis).
    Floor { up_x: i8, up_z: i8 },
}

/// Painted text. Glyphs: A-Z (the renderer's subset), `<` `>` `^` arrows, space.
#[derive(Clone, Copy, Debug)]
pub struct Sign {
    pub text: &'static str,
    /// Centre of the text on the wall face (or floor).
    pub pos: V2,
    /// Height of the text centre above the floor (walls only).
    pub y: f32,
    /// Letter height in metres.
    pub size: f32,
    pub mount: SignMount,
    pub paint: Paint,
    /// Painted backing plate behind the letters.
    pub plate: Option<Paint>,
}

#[derive(Clone, Copy, Debug)]
pub struct PaintPatch {
    pub min: V2,
    pub max: V2,
    pub paint: Paint,
}

/// A window opening in a wall. `min`/`max` is the opening's ground rect (= its sill block's).
/// Between `sill` and `top` it is open (glass, which stops nothing); `top`..`wall_top` is
/// the lintel.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub min: V2,
    pub max: V2,
    pub sill: f32,
    pub top: f32,
    pub wall_top: f32,
    /// Glazed: the renderer adds a pane that shatters when shot.
    pub glass: bool,
    /// In an exterior wall (else an interior partition).
    pub exterior: bool,
}

/// What a building is used for (renderer hint: lamps, floors, colours).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildingUse {
    House,
    Shop,
    Warehouse,
    Office,
    Workshop,
}

/// One enterable building (renderer data; gameplay only sees its blocks).
#[derive(Clone, Debug)]
pub struct BuildingInfo {
    pub name: &'static str,
    /// Outer footprint.
    pub min: V2,
    pub max: V2,
    /// Wall top = ceiling underside = roof slab bottom.
    pub height: f32,
    pub facade: Skin,
    pub interior: Skin,
    pub use_: BuildingUse,
    /// Indices into `Arena::blocks` of the exterior wall segments and window sills.
    pub shell: Vec<usize>,
    /// Flat roof with a parapet (else a slab with eaves).
    pub parapet: bool,
    /// Index of its roof in `Decor::roofs`.
    pub roof: usize,
    /// Sides that are shared with a neighbouring wing (no wall of its own; N, S, W, E).
    pub open_sides: [bool; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct Roof {
    pub min: V2,
    pub max: V2,
    /// Height of the underside.
    pub bottom: f32,
    pub thickness: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Lamp {
    pub pos: V2,
    /// Mount height (just below the ceiling or roof).
    pub height: f32,
    pub style: LampStyle,
}

impl Lamp {
    /// Middle of the fixture (ground position, height): what a shot aims at, and the centre of
    /// its hit box. Matches the renderer's fixtures (`game::map::dressing::lamps`).
    pub fn fixture(&self) -> (V2, f32) {
        let top = self.height + 0.02;
        match self.style {
            LampStyle::Panel => (self.pos, top - 0.025),
            LampStyle::Bulkhead => (self.pos, top + 0.01),
            LampStyle::Pendant => (self.pos, top - 0.8),
            LampStyle::HighBay => (self.pos, top - 0.75),
            LampStyle::Wall { facing_x, facing_z } => (self.pos + V2::new(facing_x as f32, facing_z as f32) * 0.62, self.height - 0.05),
        }
    }

    /// Half extents (x, y, z) of the fixture's hit box around [`Lamp::fixture`].
    pub fn fixture_half(&self) -> [f32; 3] {
        match self.style {
            LampStyle::Panel => [0.63, 0.035, 0.33],
            LampStyle::Bulkhead => [0.25, 0.1, 0.13],
            LampStyle::Pendant => [0.22, 0.13, 0.22],
            LampStyle::HighBay => [0.38, 0.13, 0.38],
            LampStyle::Wall { .. } => [0.14, 0.15, 0.14],
        }
    }

    /// How far (m, on the ground) the lamp lights someone well enough to matter: inside this
    /// a player stands in its pool of light.
    pub fn reach(&self) -> f32 {
        match self.style {
            LampStyle::Panel => 4.5,
            LampStyle::Bulkhead => 5.0,
            LampStyle::Pendant => 3.5,
            LampStyle::HighBay => 7.0,
            LampStyle::Wall { .. } => 6.0,
        }
    }
}

/// Where segment a→b is inside the rect `min..max`, as fractions (t0, t1) of the segment.
fn segment_span(min: V2, max: V2, a: V2, b: V2) -> Option<(f32, f32)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, dp, lo, hi) in [(a.x, d.x, min.x, max.x), (a.y, d.y, min.y, max.y)] {
        if dp.abs() < 1e-9 {
            if p < lo || p > hi {
                return None;
            }
        } else {
            let (mut ta, mut tb) = ((lo - p) / dp, (hi - p) / dp);
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
            if t0 > t1 {
                return None;
            }
        }
    }
    Some((t0, t1))
}

/// Fixture the renderer draws for a lamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LampStyle {
    /// Caged bulkhead under a roof (warm point light).
    #[default]
    Bulkhead,
    /// Recessed ceiling panel (downward spot).
    Panel,
    /// Pendant lamp in a house (warm, small).
    Pendant,
    /// Industrial high-bay under a tall roof.
    HighBay,
    /// Street lamp on a facade bracket; the light hangs `facing` (unit ground axis) off the wall.
    Wall { facing_x: i8, facing_z: i8 },
}

#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub min: V2,
    pub max: V2,
    pub bottom: f32,
    pub top: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloorSkin {
    Asphalt,
    Tiles,
    Pavement,
    /// Paving slabs (sidewalks, courtyards).
    Sidewalk,
    /// Wooden boards (house interiors).
    Wood,
    /// Sealed concrete (warehouse, workshop).
    Concrete,
    /// Office carpet.
    Carpet,
    /// Loose stone (rail ballast).
    Gravel,
}

#[derive(Clone, Copy, Debug)]
pub struct FloorPatch {
    pub min: V2,
    pub max: V2,
    pub skin: FloorSkin,
}

/// Axis-aligned map authoring helper: boxes by extents, walls with doorways cut in.
pub(crate) struct Builder {
    pub(crate) half: f32,
    pub(crate) blocks: Vec<Block>,
    pub(crate) decor: Decor,
    /// Zones added while building (rooms); they precede the map's own list.
    pub(crate) zones: Vec<Zone>,
    /// Style for `lamp()`.
    pub(crate) lamp_style: LampStyle,
    /// Bomb sites added with `site()`. Empty: `finish` takes the zones named "A site" / "B site".
    pub(crate) sites: Vec<crate::objective::BombSite>,
}

impl Builder {
    /// Default-size map with the four outer walls.
    fn new(wall_height: f32, wall_skin: Skin) -> Self {
        Self::sized(ARENA_HALF, wall_height, wall_skin)
    }

    /// Starts with the four outer walls (inner faces at ±(half - WALL_THICKNESS)).
    pub(crate) fn sized(half: f32, wall_height: f32, wall_skin: Skin) -> Self {
        let h = half;
        let t = WALL_THICKNESS;
        let mut blocks = Vec::new();
        for (cx, cz, hx, hz) in [(0.0, -h, h + t, t), (0.0, h, h + t, t), (-h, 0.0, t, h + t), (h, 0.0, t, h + t)] {
            let mut b = Block::new(cx, cz, hx, hz, BlockKind::Wall);
            b.height = wall_height;
            b.skin = wall_skin;
            blocks.push(b);
        }
        Builder { half, blocks, decor: Decor::default(), zones: Vec::new(), lamp_style: LampStyle::Bulkhead, sites: Vec::new() }
    }

    pub(crate) fn low(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, skin: Skin) {
        self.blocks.push(Block::rect(x0, z0, x1, z1, BlockKind::Low, LOW_COVER_HEIGHT, skin));
    }

    pub(crate) fn tall(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, height: f32, skin: Skin) {
        self.blocks.push(Block::rect(x0, z0, x1, z1, BlockKind::Tall, height, skin));
    }

    /// A partition along z = `z` from x0 to x1 (x0/x1 are where it ends), with a
    /// doorway centred on each of `doors` and a lintel over it.
    pub(crate) fn wall_z(&mut self, z: f32, x0: f32, x1: f32, doors: &[f32], height: f32, skin: Skin) {
        for (a, b) in Self::segments(x0, x1, doors) {
            self.tall(a, z - PARTITION_HALF, b, z + PARTITION_HALF, height, skin);
        }
        for &d in doors {
            self.header(V2::new(d - DOOR_WIDTH * 0.5, z - PARTITION_HALF), V2::new(d + DOOR_WIDTH * 0.5, z + PARTITION_HALF), height);
        }
    }

    /// A partition along x = `x` from z0 to z1, with doorways at `doors` (z centres).
    pub(crate) fn wall_x(&mut self, x: f32, z0: f32, z1: f32, doors: &[f32], height: f32, skin: Skin) {
        for (a, b) in Self::segments(z0, z1, doors) {
            self.tall(x - PARTITION_HALF, a, x + PARTITION_HALF, b, height, skin);
        }
        for &d in doors {
            self.header(V2::new(x - PARTITION_HALF, d - DOOR_WIDTH * 0.5), V2::new(x + PARTITION_HALF, d + DOOR_WIDTH * 0.5), height);
        }
    }

    pub(crate) fn header(&mut self, min: V2, max: V2, top: f32) {
        self.decor.headers.push(Header { min, max, bottom: DOOR_HEIGHT, top });
    }

    /// Solid spans of [a, b] once door gaps are cut out.
    fn segments(a: f32, b: f32, doors: &[f32]) -> Vec<(f32, f32)> {
        let mut doors = doors.to_vec();
        doors.sort_by(f32::total_cmp);
        let mut out = Vec::new();
        let mut start = a;
        for d in doors {
            let (g0, g1) = (d - DOOR_WIDTH * 0.5, d + DOOR_WIDTH * 0.5);
            if g0 - start > 0.05 {
                out.push((start, g0));
            }
            start = g1;
        }
        if b - start > 0.05 {
            out.push((start, b));
        }
        out
    }

    pub(crate) fn roof(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, bottom: f32) {
        self.decor.roofs.push(Roof { min: V2::new(x0, z0), max: V2::new(x1, z1), bottom, thickness: 0.3 });
    }

    pub(crate) fn lamp(&mut self, x: f32, z: f32, height: f32) {
        let style = self.lamp_style;
        self.lamp_styled(x, z, height, style);
    }

    pub(crate) fn lamp_styled(&mut self, x: f32, z: f32, height: f32, style: LampStyle) {
        self.decor.lamps.push(Lamp { pos: V2::new(x, z), height, style });
    }

    pub(crate) fn zone(&mut self, name: &'static str, x0: f32, z0: f32, x1: f32, z1: f32) {
        self.zones.push(zone(name, x0, z0, x1, z1));
    }

    pub(crate) fn floor(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, skin: FloorSkin) {
        self.decor.floors.push(FloorPatch { min: V2::new(x0, z0), max: V2::new(x1, z1), skin });
    }

    /// Text painted on a wall face at (x, z), centre `y` up, facing the unit axis `facing`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sign_wall(&mut self, text: &'static str, x: f32, z: f32, y: f32, size: f32, facing: (i8, i8), paint: Paint, plate: Option<Paint>) {
        self.decor.signs.push(Sign { text, pos: V2::new(x, z), y, size, mount: SignMount::Wall { facing_x: facing.0, facing_z: facing.1 }, paint, plate });
    }

    /// Text painted on the floor at (x, z), letter tops pointing along `up`.
    pub(crate) fn sign_floor(&mut self, text: &'static str, x: f32, z: f32, size: f32, up: (i8, i8), paint: Paint) {
        self.decor.signs.push(Sign { text, pos: V2::new(x, z), y: 0.0, size, mount: SignMount::Floor { up_x: up.0, up_z: up.1 }, paint, plate: None });
    }

    pub(crate) fn paint(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, paint: Paint) {
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (z0, z1) = (z0.min(z1), z0.max(z1));
        self.decor.paint.push(PaintPatch { min: V2::new(x0, z0), max: V2::new(x1, z1), paint });
    }

    /// A painted rectangle outline `w` wide (four strips that don't overlap).
    pub(crate) fn outline(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, w: f32, paint: Paint) {
        self.paint(x0, z0, x1, z0 + w, paint);
        self.paint(x0, z1 - w, x1, z1, paint);
        self.paint(x0, z0 + w, x0 + w, z1 - w, paint);
        self.paint(x1 - w, z0 + w, x1, z1 - w, paint);
    }

    /// A bomb site (plantable rectangle). Add A before B.
    pub(crate) fn site(&mut self, letter: char, x0: f32, z0: f32, x1: f32, z1: f32) {
        self.sites.push(crate::objective::BombSite::new(letter, x0, z0, x1, z1));
    }

    pub(crate) fn finish(mut self, id: MapId, zones: Vec<Zone>, player_spawn: V2, enemy_spawns: Vec<V2>) -> Arena {
        let nav = NavGrid::bake(&self.blocks, self.half, 1.0, AGENT_RADIUS);
        // Rooms added with `zone()` come first: `zone_of` returns the first match.
        self.zones.extend(zones);
        if self.sites.is_empty() {
            for (letter, name) in [('A', "A site"), ('B', "B site")] {
                if let Some(z) = self.zones.iter().find(|z| z.name == name) {
                    self.sites.push(crate::objective::BombSite::new(letter, z.min.x, z.min.y, z.max.x, z.max.y));
                }
            }
        }
        debug_assert!(self.sites.len() == 2, "{id:?}: a map declares sites A and B (`site()` or zones \"A site\" / \"B site\")");
        let mut arena = Arena {
            id,
            half: self.half,
            blocks: self.blocks,
            zones: self.zones,
            player_spawn,
            enemy_spawns,
            nav,
            cover: Vec::new(),
            decor: self.decor,
            smokes: Vec::new(),
            dark_lamps: Vec::new(),
            sites: self.sites,
        };
        arena.cover = arena.compute_cover();
        arena
    }
}

pub(crate) fn zone(name: &'static str, x0: f32, z0: f32, x1: f32, z1: f32) -> Zone {
    Zone { name, min: V2::new(x0, z0), max: V2::new(x1, z1) }
}

impl Arena {
    /// Build the given map.
    pub fn build(id: MapId) -> Self {
        match id {
            MapId::Foundry => crate::maps::foundry::foundry(),
            MapId::Depot => Arena::depot(),
            MapId::Office => Arena::office(),
            MapId::Township => crate::township::township(),
            MapId::Sirocco => crate::maps::sirocco::sirocco(),
            MapId::Gantry => crate::maps::gantry::gantry(),
        }
    }

    /// Half extent of this map: outer walls are centred on ±half, so the playable floor is
    /// ±(half - WALL_THICKNESS). Use this, not `ARENA_HALF` (the default size).
    pub fn half(&self) -> f32 {
        self.half
    }

    /// The original open, mirror-symmetric yard (Foundry before it became a defusal map).
    /// Not in the menu: a fixed, simple arena for AI tests and benches, whose coordinates the
    /// tests rely on. Its id is `MapId::Foundry` only because every arena needs one.
    pub fn yard() -> Self {
        use BlockKind::*;
        let h = ARENA_HALF;
        let t = WALL_THICKNESS;
        let mut blocks = vec![
            // Outer walls.
            Block::new(0.0, -h, h + t, t, Wall),
            Block::new(0.0, h, h + t, t, Wall),
            Block::new(-h, 0.0, t, h + t, Wall),
            Block::new(h, 0.0, t, h + t, Wall),
            // Center tower.
            Block::new(0.0, 0.0, 4.0, 4.0, Tall),
        ];
        // Mirror-symmetric pieces (x mirrored, and z mirrored).
        let quad: &[(f32, f32, f32, f32, BlockKind)] = &[
            (14.0, 6.5, 0.75, 4.5, Tall),  // lane walls with a gap at z=0
            (7.0, 10.0, 1.2, 1.2, Low),    // crates near center
            (8.0, 20.0, 1.5, 1.5, Tall),   // yard pillars
            (22.0, 12.0, 2.0, 1.0, Tall),  // lane blockers
            (24.0, 22.0, 1.5, 1.0, Low),   // corner crates
            (4.0, 26.0, 4.0, 0.5, Low),    // long low walls near spawns
            (20.0, 0.0, 1.0, 2.5, Low),    // lane mid crates
            (27.0, 6.0, 1.0, 1.0, Low),
        ];
        for &(x, z, hx, hz, k) in quad {
            for (sx, sz) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                let b = Block::new(x * sx, z * sz, hx, hz, k);
                if !blocks.iter().any(|o| o.center.dist(b.center) < 0.01) {
                    blocks.push(b);
                }
            }
        }
        let zones = vec![
            zone("the center tower", -8.0, -8.0, 8.0, 8.0),
            zone("the north yard", -14.0, -32.0, 14.0, -8.0),
            zone("the south yard", -14.0, 8.0, 14.0, 32.0),
            zone("the west lane", -32.0, -14.0, -14.0, 14.0),
            zone("the east lane", 14.0, -14.0, 32.0, 14.0),
            zone("the north-west corner", -32.0, -32.0, -14.0, -14.0),
            zone("the north-east corner", 14.0, -32.0, 32.0, -14.0),
            zone("the south-west corner", -32.0, 14.0, -14.0, 32.0),
            zone("the south-east corner", 14.0, 14.0, 32.0, 32.0),
            zone("the inner west side", -14.0, -8.0, -8.0, 8.0),
            zone("the inner east side", 8.0, -8.0, 14.0, 8.0),
        ];
        let mut b = Builder { half: ARENA_HALF, blocks, decor: Decor::default(), zones: Vec::new(), lamp_style: LampStyle::Bulkhead, sites: Vec::new() };
        b.site('A', -31.0, -31.0, -15.0, -15.0);
        b.site('B', 15.0, -31.0, 31.0, -15.0);
        b.finish(
            MapId::Foundry,
            zones,
            V2::new(0.0, 29.0),
            vec![
                V2::new(-20.0, -28.0),
                V2::new(-10.0, -29.0),
                V2::new(0.0, -29.5),
                V2::new(10.0, -29.0),
                V2::new(20.0, -28.0),
                // Flank spawns (the 6th+ enemy on Elite).
                V2::new(-28.0, -20.0),
                V2::new(28.0, -20.0),
            ],
        )
    }

    /// "Depot": a sun-baked bus depot built around a hub, not parallel lanes.
    ///
    /// ```text
    ///              north (defenders)
    ///   +-----------+------------------------+-----------+
    ///   | depot     |  north spawn            |  B site   |
    ///   | office    |                        D  (the    |
    ///   +-----+ +---+   A site: the coach     |  cistern |
    ///   |     gate  |   yard, two coaches     |  hall,   |
    ///   |  freight  |                        D  roofed)  |
    ///   |  lane     +------ forecourt --------+---D------+
    ///   | (open     | arcade | ticket |gate-  | bus  |under-
    ///   |  yard,    D shops  D hall   Dhouse  |garage| pass
    ///   |  stacked  +---D----+----D---+-------+------+ (road,
    ///   |  boxes)   |      bus bays (open lot)       | bridge)
    ///   +-----------+---------- south spawn ----------+-----+
    ///              south (attackers)
    /// ```
    ///
    /// A is the coach yard in the middle of the north, straight in front of the defender spawn;
    /// B is the cistern hall, a roofed pump hall on the north-east. Attackers reach the
    /// **forecourt** (A's front) three ways: the open **freight lane** yard on the west (which
    /// also enters A from the side through the freight gate), through **the arcade** (two shops
    /// with doors on every side), or up **the gatehouse** passage beside it. B has three doors:
    /// **the underpass** road from the south-east (under a road bridge), a side door off the
    /// forecourt corner of A, and the defenders' back door. The player spawns south; the squad
    /// defends north.
    pub fn depot() -> Self {
        use crate::building::{Building, Opening, Side};
        use Skin::*;
        let mut b = Builder::new(4.5, Stucco);

        // --- North-west: the depot office (solid) and the fence of the freight gate.
        b.tall(-31.0, -31.0, -16.0, -24.0, 6.5, Stucco);
        b.tall(-17.0, -24.0, -16.0, -21.0, 3.6, Stucco);
        b.tall(-17.0, -16.0, -16.0, -9.0, 3.6, Stucco); // the gate is z -21..-16

        // --- The freight lane: an open yard of stacked containers (no straight line end to end).
        b.tall(-26.0, -14.0, -20.0, -11.6, 2.6, Container);
        b.tall(-25.0, -1.2, -19.0, 1.2, 2.6, Container);
        b.tall(-31.0, 6.0, -28.6, 12.0, 2.6, Container);
        b.tall(-21.0, 15.0, -19.0, 17.0, 2.4, Crate);
        b.low(-29.0, 18.0, -27.8, 19.2, Crate);
        b.low(-24.0, 8.0, -21.6, 8.6, Sandbag);
        b.low(-30.2, -8.0, -29.4, -7.2, Barrel);
        b.low(-22.0, -20.0, -20.8, -18.8, Crate);
        b.low(-28.4, -18.0, -27.2, -16.8, Crate);

        // --- The arcade: two shops between the bus bays and the forecourt, doors on all sides.
        Building::new("the arcade", BuildingUse::Shop, -16.0, -3.0, 7.0, 9.0, 3.6, Stucco)
            .door(Side::North, -10.0)
            .door(Side::North, 2.0)
            .door(Side::South, -11.0)
            .door(Side::South, 3.0)
            .door(Side::West, 3.0)
            .door(Side::East, 6.0)
            .partition_x(-5.0, -2.6, 8.6, &[Opening::door(3.0)])
            .room("the arcade", -15.6, -2.6, -5.15, 8.6, Some(FloorSkin::Tiles))
            .room("the ticket hall", -4.85, -2.6, 6.6, 8.6, Some(FloorSkin::Tiles))
            .low(-13.0, -1.0, -11.8, 2.0, Shelf)
            .tall(-15.6, 6.0, -14.4, 8.6, 2.2, Shelf)
            .low(-9.0, 4.5, -7.0, 5.7, Desk)
            .low(-1.0, 1.0, 3.0, 2.2, Desk) // ticket counter
            .tall(-0.5, 3.5, 4.0, 4.1, 2.2, Shelf) // timetable rack: no line through both doors
            .low(-3.0, 6.0, -1.0, 6.8, Desk)
            .tall(4.4, -2.6, 6.6, -1.6, 2.0, Cabinet)
            .emit(&mut b);

        // --- The gatehouse: a passage between the arcade and the bus garage, under a gate arch.
        b.tall(7.0, -1.0, 8.0, 0.0, 4.0, Concrete);
        b.tall(12.0, -1.0, 13.0, 0.0, 4.0, Concrete);
        b.header(V2::new(8.0, -1.0), V2::new(12.0, 0.0), 4.0);
        b.tall(10.5, 3.0, 13.0, 6.0, 3.0, Stucco); // the gatekeeper's booth
        b.tall(13.0, -6.0, 20.0, 14.0, 6.0, Stucco); // the bus garage (solid)

        // --- B: the cistern hall, roofed. Doors: west (defenders), west (off A), south (underpass).
        Building::new("the cistern hall", BuildingUse::Warehouse, 13.0, -31.0, 31.0, -6.0, 4.5, Stucco)
            .opening(Side::West, Opening::wide_door(-27.0, 3.0, 3.0))
            .opening(Side::West, Opening::wide_door(-11.0, 3.0, 3.0))
            .opening(Side::South, Opening::wide_door(25.0, 4.0, 3.2))
            .room("B site", 13.4, -30.6, 30.6, -14.0, Some(FloorSkin::Concrete))
            .room("the cistern", 13.4, -14.0, 30.6, -6.4, Some(FloorSkin::Concrete))
            .lamp(17.0, -26.0)
            .lamp(27.0, -15.0)
            .tall(20.0, -26.0, 22.4, -23.6, 2.4, Crate) // default plant boxes
            .low(22.4, -24.8, 23.6, -23.6, Crate)
            .tall(26.0, -30.6, 30.6, -28.2, 2.6, Container)
            .tall(18.0, -15.0, 22.0, -11.0, 3.0, Concrete) // the cistern tank
            .tall(26.0, -19.0, 27.0, -18.0, 4.5, Concrete) // roof pillar
            .low(15.0, -20.0, 15.8, -19.2, Barrel)
            .low(15.0, -19.0, 15.8, -18.2, Barrel)
            .low(27.0, -11.0, 29.4, -10.4, Sandbag)
            .low(17.0, -9.0, 18.2, -7.8, Crate)
            .emit(&mut b);

        // --- The underpass: the east road, under a road bridge halfway.
        b.roof(20.0, 0.0, 31.0, 8.0, 3.4);
        b.tall(24.5, 3.0, 26.5, 5.0, 3.45, Concrete); // bridge pier
        b.lamp(22.0, 2.0, 3.3);
        b.lamp(29.0, 6.0, 3.3);
        b.tall(20.0, 9.0, 22.4, 11.4, 2.4, Crate);
        b.low(22.0, 15.0, 23.9, 19.4, Car);
        b.low(29.4, 4.0, 30.2, 4.8, Barrel);
        b.tall(27.0, -3.6, 29.4, -1.2, 2.4, Crate);
        b.low(21.0, -3.0, 23.4, -2.4, Sandbag);

        // --- A site: the coach yard.
        b.tall(-10.0, -22.0, -7.6, -13.0, 2.8, Container); // parked coach
        b.tall(1.0, -21.0, 3.4, -12.0, 2.8, Container); // parked coach
        b.tall(-4.0, -16.0, -2.0, -14.0, 2.4, Crate); // default plant boxes
        b.low(-2.0, -15.2, -0.8, -14.0, Crate);
        b.low(-14.0, -12.2, -12.8, -11.0, Crate);
        b.low(6.0, -20.0, 7.2, -18.8, Crate);
        b.low(10.0, -14.0, 10.8, -13.2, Barrel);
        b.low(-6.0, -25.4, -2.0, -24.8, Sandbag);
        b.low(5.0, -25.4, 9.0, -24.8, Sandbag);

        // --- The forecourt.
        b.low(-10.0, -7.0, -7.0, -6.4, Sandbag);
        b.low(4.5, -7.0, 5.7, -5.8, Crate);
        b.low(9.0, -8.0, 10.2, -6.8, Crate);
        b.tall(-14.0, -8.0, -12.0, -6.0, 2.4, Crate);

        // --- The bus bays and the south (player) spawn.
        b.tall(-12.0, 13.0, -9.6, 20.0, 2.8, Container); // parked bus
        b.tall(8.0, 12.0, 10.4, 19.0, 2.8, Container); // parked bus
        b.low(-5.0, 15.0, -3.8, 16.2, Crate);
        b.low(4.0, 15.5, 5.2, 16.7, Crate);
        b.low(16.0, 18.0, 16.8, 18.8, Barrel);
        b.tall(-3.0, 20.0, 3.0, 22.4, 2.8, Container); // parked truck: shields the spawn
        b.low(-14.0, 22.0, -10.0, 22.6, Sandbag);
        b.low(10.0, 22.0, 14.0, 22.6, Sandbag);
        b.tall(-31.0, 25.0, -25.0, 31.0, 6.0, Stucco);
        b.tall(25.0, 25.0, 31.0, 31.0, 6.0, Stucco);
        b.low(-20.0, 26.0, -18.8, 27.2, Crate);
        b.low(18.8, 26.0, 20.0, 27.2, Crate);

        // --- Ground: sand in the freight lane, asphalt yards and streets, pavement elsewhere.
        b.floor(-31.0, 23.0, 31.0, 31.0, FloorSkin::Asphalt);
        b.floor(-16.0, 9.0, 20.0, 23.0, FloorSkin::Asphalt);
        b.floor(-16.0, -26.0, 13.0, -9.0, FloorSkin::Asphalt);
        b.floor(-16.0, -9.0, 13.0, -3.0, FloorSkin::Pavement);
        b.floor(-16.0, -31.0, 13.0, -26.0, FloorSkin::Pavement);
        b.floor(7.0, -3.0, 13.0, 9.0, FloorSkin::Pavement);
        b.floor(20.0, -6.0, 31.0, 23.0, FloorSkin::Concrete);

        // --- Site letters and direction signs.
        b.sign_wall("A", -16.0, -27.5, 2.4, 1.6, (1, 0), Paint::White, Some(Paint::Red));
        b.sign_floor("A", -5.5, -19.5, 3.0, (0, -1), Paint::Red);
        b.sign_wall("B", 13.0, -18.5, 2.4, 1.6, (-1, 0), Paint::White, Some(Paint::Blue));
        b.sign_wall("B", 29.0, -6.0, 2.4, 1.6, (0, 1), Paint::White, Some(Paint::Blue));
        b.sign_floor("B", 22.0, -20.0, 3.0, (0, -1), Paint::Blue);
        // (The sign glyphs are A, B, D, I, M, T and arrows only: no callout words.)
        b.sign_wall("A B ^", 0.0, 22.4, 2.0, 0.6, (0, 1), Paint::Black, Some(Paint::White));
        b.sign_wall("< A", -14.0, 9.0, 2.6, 0.9, (0, 1), Paint::Black, Some(Paint::Yellow));
        b.sign_wall("B >", 16.5, 14.0, 2.6, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
        b.sign_floor("A ^", -23.5, 20.0, 1.2, (0, -1), Paint::Yellow);
        b.sign_floor("B ^", 25.5, 20.0, 1.2, (0, -1), Paint::Yellow);
        b.sign_floor("B >", 10.0, -27.5, 1.0, (0, -1), Paint::Yellow);
        // Spawn borders: attackers orange, defenders blue.
        b.outline(-16.0, 23.4, 16.0, 31.0, 0.25, Paint::Orange);
        b.outline(-12.0, -30.6, 10.0, -26.4, 0.25, Paint::Blue);
        // Plant zones: the west half of the coach yard (A) and the back of the cistern hall
        // (B). They stay 14 m apart: the squad's rotation calls need to tell the sites apart,
        // and the hall's west aisle and the yard's east end are the approaches, not the site.
        b.site('A', -14.0, -24.0, 5.0, -11.0);
        b.site('B', 19.0, -30.0, 29.5, -15.0);
        let zones = vec![
            zone("the north spawn", -16.0, -31.0, 13.0, -26.0),
            zone("A site", -16.0, -26.0, 13.0, -9.0),
            zone("the freight lane", -31.0, -24.0, -16.0, 23.0),
            zone("the forecourt", -16.0, -9.0, 13.0, -3.0),
            zone("the gatehouse", 7.0, -3.0, 13.0, 9.0),
            zone("the bus bays", -16.0, 9.0, 20.0, 23.0),
            zone("the underpass", 20.0, -6.0, 31.0, 23.0),
            zone("the south spawn", -31.0, 23.0, 31.0, 31.0),
        ];
        b.finish(
            MapId::Depot,
            zones,
            V2::new(0.0, 27.0),
            vec![
                V2::new(-9.0, -29.0),
                V2::new(-3.0, -29.5),
                V2::new(3.0, -29.0),
                V2::new(8.0, -28.5),
                V2::new(-13.0, -28.0),
                V2::new(0.0, -26.8),
                V2::new(24.0, -27.0),
            ],
        )
    }

    /// "Office": a single roofed floor of rooms and corridors. Rooms on both flanks off
    /// two long corridors, open-plan cubicles with a conference room in the middle, a break
    /// room north and reception south. Doorways everywhere, so any room can be flanked.
    pub fn office() -> Self {
        use Skin::*;
        let h = OFFICE_WALL_HEIGHT;
        let t = PARTITION_HALF;
        let mut b = Builder::new(h, Drywall);
        b.lamp_style = LampStyle::Panel;
        b.roof(-31.0, -31.0, 31.0, 31.0, OFFICE_CEILING);
        // --- Separators between the back offices (north), the floor, and the lobby (south).
        b.wall_z(-22.0, -31.0, 31.0, &[-25.0, -17.0, -8.0, 8.0, 17.0, 25.0], h, Drywall);
        b.wall_z(22.0, -31.0, 31.0, &[-25.0, -17.0, -5.0, 5.0, 17.0, 25.0], h, Drywall);
        b.wall_x(-12.0, -31.0, -22.0 - t, &[-27.0], h, Drywall);
        b.wall_x(12.0, -31.0, -22.0 - t, &[-26.0], h, Drywall);
        b.wall_x(-12.0, 22.0 + t, 31.0, &[27.0], h, Drywall);
        b.wall_x(12.0, 22.0 + t, 31.0, &[26.0], h, Drywall);
        // --- Corridor walls (full length of the floor).
        let (fz0, fz1) = (-22.0 + t, 22.0 - t);
        b.wall_x(-19.0, fz0, fz1, &[-16.0, -5.0, 6.0, 17.0], h, Drywall);
        b.wall_x(-15.0, fz0, fz1, &[-14.0, -5.0, 5.0, 16.0], h, Drywall);
        b.wall_x(15.0, fz0, fz1, &[-16.0, 0.0, 14.0], h, Drywall);
        b.wall_x(19.0, fz0, fz1, &[-17.0, -6.0, 5.0, 16.0], h, Drywall);
        // --- West rooms.
        b.wall_z(-11.0, -31.0, -19.0 - t, &[-27.0], h, Drywall);
        b.wall_z(0.0, -31.0, -19.0 - t, &[], h, Drywall);
        b.wall_z(11.0, -31.0, -19.0 - t, &[-25.0], h, Drywall);
        // --- East rooms.
        b.wall_z(-11.0, 19.0 + t, 31.0, &[27.0], h, Drywall); // kitchen door into the server room (A)
        b.wall_z(0.0, 19.0 + t, 31.0, &[25.0], h, Drywall);
        b.wall_z(11.0, 19.0 + t, 31.0, &[27.0], h, Drywall);
        // --- Centre: break room | cubicles | reception.
        b.wall_z(-11.0, -15.0 + t, 15.0 - t, &[-10.0, 0.0, 10.0], h, Drywall);
        b.wall_z(11.0, -15.0 + t, 15.0 - t, &[-8.0, 8.0], h, Drywall);
        // Conference room in the middle of the cubicles.
        b.wall_x(-5.0, -4.0 - t, 4.0 + t, &[0.0], h, Drywall);
        b.wall_x(5.0, -4.0 - t, 4.0 + t, &[-1.0], h, Drywall);
        // Its north wall is glazed: two interior windows onto the cubicles.
        {
            use crate::building::{Axis, Opening, WallLine};
            let w = WallLine { axis: Axis::X, coord: -4.0, a0: -5.0 + t, a1: 5.0 - t, half_thickness: t, height: h, skin: Drywall, exterior: false };
            b.wall_line(&w, &[Opening::window(-2.3, 3.0), Opening::window(2.3, 3.0)]);
        }
        b.wall_z(4.0, -5.0 + t, 5.0 - t, &[2.0], h, Drywall);
        b.low(-2.5, -1.0, 2.5, 1.0, Desk);
        // Cubicle pods (desks with partitions).
        for (x, z) in [(-10.0, -7.5), (-10.0, 0.0), (-10.0, 7.5), (10.0, -7.5), (10.0, 1.0), (10.0, 7.5)] {
            b.low(x - 1.2, z - 0.8, x + 1.2, z + 0.8, Desk);
        }
        b.low(-2.0, -8.6, 2.0, -7.4, Desk);
        b.low(-2.0, 7.4, 2.0, 8.6, Desk);
        // Break room.
        for x in [-8.0, 0.0, 8.0] {
            b.low(x - 1.0, -18.0, x + 1.0, -16.0, Desk);
        }
        b.low(-6.0, -22.0 + t, 6.0, -21.0, Desk); // counter along the north wall
        b.tall(11.0, -22.0 + t, 12.4, -21.0, 2.0, Cabinet); // vending machine
        // A delivery of boxed kit stacked in the corners.
        b.tall(12.6, -22.0 + t, 14.6, -19.85, 2.0, Crate);
        b.low(-15.0 + t, -22.0 + t, -13.65, -20.65, Crate);
        b.low(-15.0 + t, -20.65, -13.65, -19.45, Crate);
        // Reception.
        b.low(-3.0, 15.0, 3.0, 16.0, Desk);
        b.low(-14.4, 20.4, -13.6, 21.2, Barrel); // planters
        b.low(13.6, 20.4, 14.4, 21.2, Barrel);
        // Corridor chicanes (water coolers / cabinets).
        b.tall(-19.0 + t, -0.5, -17.8, 0.5, 2.0, Cabinet);
        b.tall(-16.2, 9.5, -15.0 - t, 10.5, 2.0, Cabinet);
        b.tall(15.0 + t, -9.5, 16.2, -8.5, 2.0, Cabinet);
        b.tall(17.8, 8.5, 19.0 - t, 9.5, 2.0, Cabinet);
        // West rooms: records (shelves; B site), office, copy room, manager's office.
        b.tall(-28.0, -19.0, -22.0, -18.4, 2.2, Cabinet);
        b.tall(-28.0, -15.0, -22.0, -14.4, 2.2, Cabinet);
        // B site dressing: archive boxes (default plant cover) and a shredder bin.
        b.tall(-31.0, -13.4, -29.4, -11.15, 2.0, Crate);
        b.low(-29.4, -12.35, -28.2, -11.15, Crate);
        b.low(-20.35, -21.85, -19.15, -20.65, Barrel);
        b.low(-28.0, -8.0, -25.6, -7.2, Desk);
        b.low(-24.0, -4.0, -21.6, -3.2, Desk);
        b.low(-31.0, 4.0, -29.8, 6.0, Cabinet);
        b.tall(-22.0, 9.9, -20.0, 11.0 - t, 2.0, Cabinet);
        b.low(-27.0, 16.0, -24.6, 17.2, Desk);
        b.low(-31.0, 19.0, -30.0, 21.5, Desk);
        // East rooms: server racks, kitchen, storage, IT.
        for z in [-18.0, -14.5] {
            b.tall(22.0, z - 0.4, 29.0, z + 0.4, 2.2, Cabinet);
        }
        // A site dressing: shipping crates for new racks (default plant cover), a cable drum.
        b.low(29.6, -21.85, 31.0, -20.45, Crate);
        b.low(29.6, -20.45, 31.0, -19.05, Crate);
        b.low(19.15, -12.6, 20.35, -11.4, Barrel);
        b.low(30.0, -9.0, 31.0, -2.0, Desk);
        b.low(23.0, -6.5, 27.0, -4.5, Desk);
        b.tall(22.0, 2.0, 24.0, 4.0, 2.0, Crate);
        b.low(27.0, 7.0, 28.2, 8.2, Crate);
        b.tall(28.0, 1.0, 31.0, 3.0, 2.4, Crate);
        b.low(22.0, 15.0, 24.4, 15.8, Desk);
        b.low(26.0, 18.0, 28.4, 18.8, Desk);
        // Back offices (north) and lobby (south).
        b.low(-27.0, -28.0, -24.6, -27.2, Desk);
        b.low(-19.0, -25.0, -16.6, -24.2, Desk);
        b.low(16.0, -28.0, 18.4, -27.2, Desk);
        b.low(24.0, -25.0, 26.4, -24.2, Desk);
        b.tall(-6.4, -26.9, -5.6, -26.1, h, Concrete);
        b.tall(5.6, -26.9, 6.4, -26.1, h, Concrete);
        b.tall(-6.4, 26.1, -5.6, 26.9, h, Concrete);
        b.tall(5.6, 26.1, 6.4, 26.9, h, Concrete);
        b.low(-2.0, 29.6, 2.0, 30.2, Desk); // lobby bench
        b.low(-28.0, 27.0, -26.8, 28.2, Crate);
        b.low(-20.0, 24.0, -18.8, 25.2, Crate);
        b.low(26.8, 27.0, 28.0, 28.2, Crate);
        b.low(18.8, 24.0, 20.0, 25.2, Crate);
        // Ceiling lamps, most important first (the renderer gives the first few shadows).
        let lamp_h = OFFICE_CEILING - 0.02;
        for (x, z) in [
            (0.0, 0.0),
            (-10.0, -4.0),
            (10.0, 4.0),
            (0.0, 26.5),
            (0.0, -26.5),
            (-25.0, -5.5),
            (25.0, -16.5),
            (-17.0, -6.0),
            (17.0, 6.0),
            (-8.0, -16.5),
            (8.0, -16.5),
            (-8.0, 16.5),
            (8.0, 16.5),
            (-10.0, 6.0),
            (10.0, -6.0),
            (-25.0, -16.5),
            (-25.0, 5.5),
            (-25.0, 16.5),
            (25.0, -5.5),
            (25.0, 5.5),
            (25.0, 16.5),
            (-17.0, -16.0),
            (-17.0, 4.0),
            (-17.0, 15.0),
            (17.0, -16.0),
            (17.0, -5.0),
            (17.0, 15.0),
            (-21.0, -26.5),
            (21.0, -26.5),
            (-21.0, 26.5),
            (21.0, 26.5),
        ] {
            b.lamp(x, z, lamp_h);
        }
        // Floors: carpet is the base; tiles in the lobby strip, kitchen and corridors.
        b.floor(-31.0, 22.0, 31.0, 31.0, FloorSkin::Tiles);
        b.floor(19.0, -11.0, 31.0, 0.0, FloorSkin::Tiles);
        b.floor(-19.0, -22.0, -15.0, 22.0, FloorSkin::Tiles);
        b.floor(15.0, -22.0, 19.0, 22.0, FloorSkin::Tiles);
        // --- Bomb sites on the flanks: A is the server room (east), B the records room (west).
        //     The squad spawns in the back offices behind both; the player in the lobby.
        b.site('A', 19.15, -21.85, 31.0, -11.15);
        b.site('B', -31.0, -21.85, -19.15, -11.15);
        b.sign_wall("A", 21.0, -21.85, 2.0, 1.3, (0, 1), Paint::White, Some(Paint::Red));
        b.sign_wall("A", 18.85, -13.0, 2.0, 1.1, (-1, 0), Paint::White, Some(Paint::Red));
        b.sign_wall("A", 23.5, -10.85, 2.0, 1.1, (0, 1), Paint::White, Some(Paint::Red));
        b.sign_floor("A", 25.0, -12.6, 2.0, (0, -1), Paint::Red);
        b.sign_wall("B", -21.0, -21.85, 2.0, 1.3, (0, 1), Paint::White, Some(Paint::Blue));
        b.sign_wall("B", -18.85, -13.0, 2.0, 1.1, (1, 0), Paint::White, Some(Paint::Blue));
        b.sign_wall("B", -23.0, -10.85, 2.0, 1.1, (0, 1), Paint::White, Some(Paint::Blue));
        b.sign_floor("B", -25.0, -12.6, 2.0, (0, -1), Paint::Blue);
        // Directions from the lobby, and down the corridors.
        b.sign_wall("< B", -9.0, 22.15, 2.1, 0.8, (0, 1), Paint::Black, Some(Paint::Yellow));
        b.sign_wall("A >", 9.0, 22.15, 2.1, 0.8, (0, 1), Paint::Black, Some(Paint::Yellow));
        b.sign_floor("A ^", 17.0, 12.0, 1.0, (0, -1), Paint::Yellow);
        b.sign_floor("B ^", -17.0, 12.0, 1.0, (0, -1), Paint::Yellow);
        // Spawn borders: attackers orange (lobby), defenders blue (back hallway).
        b.outline(-10.0, 24.5, 10.0, 30.8, 0.2, Paint::Orange);
        b.outline(-11.6, -30.8, 11.6, -23.2, 0.2, Paint::Blue);
        let zones = vec![
            zone("the north-west office", -32.0, -32.0, -12.0, -22.0),
            zone("the back hallway", -12.0, -32.0, 12.0, -22.0),
            zone("the north-east office", 12.0, -32.0, 32.0, -22.0),
            zone("the records room", -32.0, -22.0, -19.0, -11.0),
            zone("the west office", -32.0, -11.0, -19.0, 0.0),
            zone("the copy room", -32.0, 0.0, -19.0, 11.0),
            zone("the manager's office", -32.0, 11.0, -19.0, 22.0),
            zone("the west corridor", -19.0, -22.0, -15.0, 22.0),
            zone("the east corridor", 15.0, -22.0, 19.0, 22.0),
            zone("the break room", -15.0, -22.0, 15.0, -11.0),
            zone("the conference room", -5.0, -4.0, 5.0, 4.0),
            zone("the cubicles", -15.0, -11.0, 15.0, 11.0),
            zone("the reception", -15.0, 11.0, 15.0, 22.0),
            zone("the server room", 19.0, -22.0, 32.0, -11.0),
            zone("the kitchen", 19.0, -11.0, 32.0, 0.0),
            zone("the storage room", 19.0, 0.0, 32.0, 11.0),
            zone("the IT office", 19.0, 11.0, 32.0, 22.0),
            zone("the west entrance", -32.0, 22.0, -12.0, 32.0),
            zone("the lobby", -12.0, 22.0, 12.0, 32.0),
            zone("the east entrance", 12.0, 22.0, 32.0, 32.0),
        ];
        b.finish(
            MapId::Office,
            zones,
            V2::new(0.0, 28.0),
            vec![
                V2::new(-3.0, -28.0),
                V2::new(3.0, -28.0),
                V2::new(-20.0, -28.5),
                V2::new(20.0, -28.5),
                V2::new(-28.0, -24.0),
                V2::new(28.0, -24.0),
                V2::new(0.0, -24.0),
            ],
        )
    }

    fn compute_cover(&self) -> Vec<CoverPoint> {
        let mut out = Vec::new();
        let off = AGENT_RADIUS + 0.35;
        for (i, b) in self.blocks.iter().enumerate() {
            if b.kind == BlockKind::Wall {
                continue;
            }
            let (hx, hz) = (b.half.x + off, b.half.y + off);
            let mut pts = Vec::new();
            let step = 1.5;
            let mut x = -b.half.x;
            while x <= b.half.x + 0.01 {
                pts.push(V2::new(x, -hz));
                pts.push(V2::new(x, hz));
                x += step;
            }
            let mut z = -b.half.y;
            while z <= b.half.y + 0.01 {
                pts.push(V2::new(-hx, z));
                pts.push(V2::new(hx, z));
                z += step;
            }
            for p in pts {
                let pos = b.center + p;
                if self.nav.walkable_at(pos) {
                    out.push(CoverPoint { pos, block: i });
                }
            }
        }
        out
    }

    /// Line of sight on the ground plane. Low cover only blocks if either end is crouched.
    pub fn line_of_sight(&self, a: V2, a_crouched: bool, b: V2, b_crouched: bool) -> bool {
        let crouched = a_crouched || b_crouched;
        !self.blocks.iter().any(|blk| {
            (blk.kind != BlockKind::Low || crouched) && blk.intersects_segment(a, b)
        }) && !crate::grenade::smoke_blocks(&self.smokes, a, b)
    }

    /// Is lamp `id` still lit (not shot out this round)?
    pub fn lamp_lit(&self, id: usize) -> bool {
        id < self.decor.lamps.len() && !self.dark_lamps.contains(&id)
    }

    /// Lit lamps whose pool of light covers `p` (within [`Lamp::reach`], nothing solid between
    /// the fixture and `p`), nearest first. Empty on maps without lamps.
    pub fn lamps_lighting(&self, p: V2) -> Vec<usize> {
        let mut out: Vec<(f32, usize)> = self
            .decor
            .lamps
            .iter()
            .enumerate()
            .filter(|&(i, _)| self.lamp_lit(i))
            .filter_map(|(i, l)| {
                let (at, h) = l.fixture();
                let d = at.dist(p);
                (d <= l.reach() && self.clear_line_3d(at, h, p, 1.5)).then_some((d, i))
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out.into_iter().map(|(_, i)| i).collect()
    }

    /// A straight line from (`a`, height `ha`) to (`b`, height `hb`) passes nothing solid:
    /// blocks (from the floor to their height), door lintels, window lintels and smoke. For
    /// shots at things off the floor (a lamp under the ceiling over a doorway's lintel).
    pub fn clear_line_3d(&self, a: V2, ha: f32, b: V2, hb: f32) -> bool {
        let h = |t: f32| ha + (hb - ha) * t;
        let crosses = |min: V2, max: V2, lo: f32, hi: f32| {
            segment_span(min, max, a, b).is_some_and(|(t0, t1)| {
                let (h0, h1) = (h(t0), h(t1));
                h0.min(h1) < hi && h0.max(h1) > lo
            })
        };
        !self.blocks.iter().any(|blk| crosses(blk.center - blk.half, blk.center + blk.half, -1.0, blk.height))
            && !self.decor.headers.iter().any(|hd| crosses(hd.min, hd.max, hd.bottom, hd.top))
            && !self.decor.windows.iter().any(|w| crosses(w.min, w.max, w.top, w.wall_top))
            && !crate::grenade::smoke_blocks(&self.smokes, a, b)
    }

    /// Damage fraction a bullet from a gun with `penetration` keeps from `a` to `b` through
    /// thin cover (the same rule as the game's bullets, `weapons::penetration_mult`): 1 in the
    /// open, 0 when a wall or anything too thick is in the way. Low cover counts (a crouched
    /// target behind a crate is shot through it).
    pub fn penetration(&self, a: V2, b: V2, penetration: f32) -> f32 {
        let mut keep = 1.0;
        for blk in &self.blocks {
            let thick = blk.segment_inside(a, b);
            if thick <= 0.0 {
                continue;
            }
            if !crate::weapons::penetrable(blk) {
                return 0.0;
            }
            keep *= crate::weapons::penetration_mult(penetration, thick);
            if keep <= 0.05 {
                return 0.0;
            }
        }
        keep
    }

    pub fn zone_of(&self, p: V2) -> Option<&Zone> {
        self.zones
            .iter()
            .find(|z| p.x >= z.min.x && p.x <= z.max.x && p.y >= z.min.y && p.y <= z.max.y)
    }

    pub fn in_bounds(&self, p: V2) -> bool {
        p.x.abs() < self.half - AGENT_RADIUS && p.y.abs() < self.half - AGENT_RADIUS
    }

    /// Push a point out of any block it overlaps (used by the 2D sim for collision).
    pub fn resolve_collision(&self, mut p: V2, radius: f32) -> V2 {
        for b in &self.blocks {
            if !b.contains(p, radius) {
                continue;
            }
            let d = p - b.center;
            // Off a corner: a circle against the box (rounded corners), pushed out radially,
            // so a body sliding past a door jamb rounds it instead of sticking on the square
            // corner of the padded box (like the game's capsules).
            let (ox, oz) = (d.x.abs() - b.half.x, d.y.abs() - b.half.y);
            if ox > 0.0 && oz > 0.0 {
                let r = V2::new(ox * d.x.signum(), oz * d.y.signum());
                let l = r.len();
                if l >= radius {
                    continue;
                }
                p = p + r * ((radius - l) / l.max(1e-6));
                continue;
            }
            let px = b.half.x + radius - d.x.abs();
            let pz = b.half.y + radius - d.y.abs();
            if px < pz {
                p.x += px * d.x.signum();
            } else {
                p.y += pz * d.y.signum();
            }
        }
        let lim = self.half - radius;
        V2::new(p.x.clamp(-lim, lim), p.y.clamp(-lim, lim))
    }
}
