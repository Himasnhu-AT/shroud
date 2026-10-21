//! The arena layout, shared by the game (which builds meshes + colliders from it)
//! and the offline trainer (which simulates on it in 2D).

use crate::math::V2;
use crate::nav::NavGrid;

/// Height of low cover: you can shoot over it standing, hide behind it crouched.
pub const LOW_COVER_HEIGHT: f32 = 1.1;
pub const TALL_HEIGHT: f32 = 3.6;
pub const ARENA_HALF: f32 = 32.0;
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
    fn rect(x0: f32, z0: f32, x1: f32, z1: f32, kind: BlockKind, height: f32, skin: Skin) -> Self {
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
/// shared by the maps, AI and UI streams. Every map fits inside ±ARENA_HALF (the player-model
/// heat grids assume it) and has at least 6 enemy spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MapId {
    #[default]
    Foundry,
    /// Three-lane, medium-range outdoor map (Dust 2-like).
    Depot,
    /// Tight indoor close-quarters map (Office / Shoot House-like).
    Office,
}

impl MapId {
    pub const ALL: [MapId; 3] = [MapId::Foundry, MapId::Depot, MapId::Office];
    pub fn label(self) -> &'static str {
        match self {
            MapId::Foundry => "Foundry",
            MapId::Depot => "Depot",
            MapId::Office => "Office",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            MapId::Foundry => "Open industrial yard around a central tower.",
            MapId::Depot => "Three lanes, long sightlines, one contested middle.",
            MapId::Office => "Tight corridors and rooms. Every corner is a fight.",
        }
    }
}

pub struct Arena {
    pub id: MapId,
    pub blocks: Vec<Block>,
    pub zones: Vec<Zone>,
    pub player_spawn: V2,
    pub enemy_spawns: Vec<V2>,
    pub nav: NavGrid,
    pub cover: Vec<CoverPoint>,
    /// Visual-only dressing (roofs, lamps, door headers, floor patches). No gameplay effect.
    pub decor: Decor,
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
}

#[derive(Clone, Copy, Debug)]
pub struct FloorPatch {
    pub min: V2,
    pub max: V2,
    pub skin: FloorSkin,
}

/// Axis-aligned map authoring helper: boxes by extents, walls with doorways cut in.
struct Builder {
    blocks: Vec<Block>,
    decor: Decor,
}

impl Builder {
    /// Starts with the four outer walls (inner faces at ±(ARENA_HALF - WALL_THICKNESS)).
    fn new(wall_height: f32, wall_skin: Skin) -> Self {
        let h = ARENA_HALF;
        let t = WALL_THICKNESS;
        let mut blocks = Vec::new();
        for (cx, cz, hx, hz) in [(0.0, -h, h + t, t), (0.0, h, h + t, t), (-h, 0.0, t, h + t), (h, 0.0, t, h + t)] {
            let mut b = Block::new(cx, cz, hx, hz, BlockKind::Wall);
            b.height = wall_height;
            b.skin = wall_skin;
            blocks.push(b);
        }
        Builder { blocks, decor: Decor::default() }
    }

    fn low(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, skin: Skin) {
        self.blocks.push(Block::rect(x0, z0, x1, z1, BlockKind::Low, LOW_COVER_HEIGHT, skin));
    }

    fn tall(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, height: f32, skin: Skin) {
        self.blocks.push(Block::rect(x0, z0, x1, z1, BlockKind::Tall, height, skin));
    }

    /// A partition along z = `z` from x0 to x1 (x0/x1 are where it ends), with a
    /// doorway centred on each of `doors` and a lintel over it.
    fn wall_z(&mut self, z: f32, x0: f32, x1: f32, doors: &[f32], height: f32, skin: Skin) {
        for (a, b) in Self::segments(x0, x1, doors) {
            self.tall(a, z - PARTITION_HALF, b, z + PARTITION_HALF, height, skin);
        }
        for &d in doors {
            self.header(V2::new(d - DOOR_WIDTH * 0.5, z - PARTITION_HALF), V2::new(d + DOOR_WIDTH * 0.5, z + PARTITION_HALF), height);
        }
    }

    /// A partition along x = `x` from z0 to z1, with doorways at `doors` (z centres).
    fn wall_x(&mut self, x: f32, z0: f32, z1: f32, doors: &[f32], height: f32, skin: Skin) {
        for (a, b) in Self::segments(z0, z1, doors) {
            self.tall(x - PARTITION_HALF, a, x + PARTITION_HALF, b, height, skin);
        }
        for &d in doors {
            self.header(V2::new(x - PARTITION_HALF, d - DOOR_WIDTH * 0.5), V2::new(x + PARTITION_HALF, d + DOOR_WIDTH * 0.5), height);
        }
    }

    fn header(&mut self, min: V2, max: V2, top: f32) {
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

    fn roof(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, bottom: f32) {
        self.decor.roofs.push(Roof { min: V2::new(x0, z0), max: V2::new(x1, z1), bottom, thickness: 0.3 });
    }

    fn lamp(&mut self, x: f32, z: f32, height: f32) {
        self.decor.lamps.push(Lamp { pos: V2::new(x, z), height });
    }

    fn floor(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, skin: FloorSkin) {
        self.decor.floors.push(FloorPatch { min: V2::new(x0, z0), max: V2::new(x1, z1), skin });
    }

    fn finish(self, id: MapId, zones: Vec<Zone>, player_spawn: V2, enemy_spawns: Vec<V2>) -> Arena {
        let nav = NavGrid::bake(&self.blocks, ARENA_HALF, 1.0, AGENT_RADIUS);
        let mut arena = Arena { id, blocks: self.blocks, zones, player_spawn, enemy_spawns, nav, cover: Vec::new(), decor: self.decor };
        arena.cover = arena.compute_cover();
        arena
    }
}

fn zone(name: &'static str, x0: f32, z0: f32, x1: f32, z1: f32) -> Zone {
    Zone { name, min: V2::new(x0, z0), max: V2::new(x1, z1) }
}

impl Arena {
    /// Build the given map.
    pub fn build(id: MapId) -> Self {
        match id {
            MapId::Foundry => Arena::foundry(),
            MapId::Depot => Arena::depot(),
            MapId::Office => Arena::office(),
        }
    }

    /// The original arena: "Foundry", an open mirror-symmetric yard.
    pub fn foundry() -> Self {
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
        let b = Builder { blocks, decor: Decor::default() };
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

    /// "Depot": three lanes between sun-baked buildings. Long A on the west with a door
    /// choke, a contested mid with double doors, roofed B tunnels on the east, and two
    /// crate-strewn sites at the north end. The player spawns south; the squad defends north.
    pub fn depot() -> Self {
        use Skin::*;
        let mut b = Builder::new(4.5, Stucco);
        // --- Buildings that separate the lanes (tall facades for the skyline).
        b.tall(-21.0, -6.0, -7.0, 3.0, 5.5, Stucco); // west block, north half
        b.tall(-21.0, 6.5, -7.0, 17.0, 6.5, Stucco); // west block, south half (A short runs between)
        b.tall(7.0, -6.0, 21.0, 8.0, 6.0, Stucco); // east block, north half
        b.tall(7.0, 11.5, 21.0, 17.0, 5.0, Stucco); // east block, south half (lower tunnel between)
        b.roof(7.0, 8.0, 21.0, 11.5, 3.4); // the lower tunnel is covered
        // --- Mid doors: the contested choke between mid and the north.
        b.tall(-7.0, -7.5, -1.2, -6.0, 4.0, Concrete);
        b.tall(1.2, -7.5, 7.0, -6.0, 4.0, Concrete);
        b.header(V2::new(-1.2, -7.5), V2::new(1.2, -6.0), 4.0);
        // --- Long A doors.
        b.tall(-31.0, -3.0, -27.2, -1.5, 4.0, Concrete);
        b.tall(-24.8, -3.0, -21.0, -1.5, 4.0, Concrete);
        b.header(V2::new(-27.2, -3.0), V2::new(-24.8, -1.5), 4.0);
        // --- B tunnels: a roofed serpentine between the east block and the outer wall.
        b.roof(21.0, -2.5, 31.0, 17.0, 3.4);
        b.tall(25.0, 5.0, 26.0, 14.0, 3.45, Concrete); // splits upper/lower tunnel
        b.tall(21.0, 2.0, 26.5, 3.0, 3.45, Concrete);
        b.tall(25.5, -2.0, 31.0, -1.0, 3.45, Concrete);
        b.low(29.4, 9.0, 30.2, 9.8, Barrel);
        b.low(29.4, 10.0, 30.2, 10.8, Barrel);
        b.low(21.0, 15.0, 22.2, 16.2, Crate);
        b.lamp(23.0, 12.0, 3.3);
        b.lamp(28.5, 8.0, 3.3);
        b.lamp(28.5, 0.5, 3.3);
        b.lamp(23.0, -0.5, 3.3);
        b.lamp(14.0, 9.75, 3.3);
        // --- Site dividers (CT side).
        b.tall(-13.5, -24.0, -12.0, -12.0, 3.6, Stucco);
        b.tall(12.0, -24.0, 13.5, -12.0, 3.6, Stucco);
        // --- A site.
        b.tall(-25.0, -20.0, -22.0, -17.0, 2.4, Crate); // default box stack
        b.low(-22.0, -18.2, -20.8, -17.0, Crate);
        b.low(-18.0, -13.0, -16.8, -11.8, Crate);
        b.tall(-31.0, -29.0, -28.6, -23.0, 2.6, Container);
        b.low(-17.0, -22.0, -15.0, -21.4, Sandbag);
        b.low(-29.0, -9.0, -26.6, -8.4, Sandbag);
        // --- B site.
        b.tall(22.0, -22.4, 28.0, -20.0, 2.6, Container);
        b.tall(17.0, -17.0, 19.4, -14.6, 2.4, Crate);
        b.low(19.4, -15.8, 20.6, -14.6, Crate);
        b.low(27.6, -12.4, 28.4, -11.6, Barrel);
        b.low(28.6, -12.4, 29.4, -11.6, Barrel);
        b.low(23.0, -9.0, 25.4, -8.4, Sandbag);
        b.low(15.0, -9.0, 17.4, -8.4, Sandbag);
        // --- North (CT) area.
        b.low(-4.0, -22.6, 4.0, -21.8, Sandbag);
        b.tall(-6.6, -15.0, -4.2, -12.6, 2.4, Crate);
        b.tall(3.0, -17.0, 5.0, -15.0, 3.6, Concrete);
        b.low(-1.0, -11.0, 1.0, -10.2, Crate);
        b.low(6.0, -11.0, 7.4, -9.6, Crate);
        // --- Mid.
        b.low(-1.0, 3.0, 1.0, 5.0, Crate); // the mid box
        b.low(3.6, 12.0, 5.0, 13.4, Crate);
        b.low(-6.0, 9.0, -4.8, 10.2, Barrel);
        // --- Long A.
        b.low(-29.0, 6.0, -27.6, 7.4, Crate);
        b.low(-24.0, 12.0, -22.8, 13.2, Crate);
        b.tall(-31.0, 9.0, -28.6, 15.0, 2.6, Container);
        // --- South (player) spawn.
        b.tall(-3.0, 20.0, 3.0, 22.4, 2.8, Container); // parked truck shields spawn from mid
        b.low(-14.0, 22.0, -10.0, 22.6, Sandbag);
        b.low(10.0, 22.0, 14.0, 22.6, Sandbag);
        b.tall(-31.0, 25.0, -25.0, 31.0, 6.0, Stucco);
        b.tall(25.0, 25.0, 31.0, 31.0, 6.0, Stucco);
        b.low(-20.0, 26.0, -18.8, 27.2, Crate);
        b.low(18.8, 26.0, 20.0, 27.2, Crate);
        // --- Ground: sand everywhere, asphalt roads through mid and the spawn street,
        //     concrete pads on the sites.
        b.floor(-7.0, -7.5, 7.0, 17.0, FloorSkin::Asphalt);
        b.floor(-31.0, 17.0, 31.0, 24.0, FloorSkin::Asphalt);
        b.floor(-31.0, -31.0, -13.5, -10.0, FloorSkin::Pavement);
        b.floor(13.5, -31.0, 31.0, -10.0, FloorSkin::Pavement);
        let zones = vec![
            zone("A site", -32.0, -32.0, -13.5, -7.5),
            zone("B site", 13.5, -32.0, 32.0, -6.0),
            zone("the north spawn", -13.5, -32.0, 13.5, -20.0),
            zone("north mid", -13.5, -20.0, 13.5, -6.0),
            zone("long A", -32.0, -7.5, -21.0, 17.0),
            zone("the A short", -21.0, -6.0, -7.0, 17.0),
            zone("mid", -7.0, -6.0, 7.0, 17.0),
            zone("the B tunnels", 21.0, -6.0, 32.0, 17.0),
            zone("the lower tunnel", 7.0, -6.0, 21.0, 17.0),
            zone("the south spawn", -32.0, 17.0, 32.0, 32.0),
        ];
        b.finish(
            MapId::Depot,
            zones,
            V2::new(0.0, 27.0),
            vec![
                V2::new(-4.0, -28.0),
                V2::new(4.0, -28.0),
                V2::new(-18.0, -27.0),
                V2::new(18.0, -27.0),
                V2::new(0.0, -25.0),
                V2::new(-26.5, -21.5),
                V2::new(26.0, -26.0),
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
        b.wall_z(-11.0, 19.0 + t, 31.0, &[], h, Drywall);
        b.wall_z(0.0, 19.0 + t, 31.0, &[25.0], h, Drywall);
        b.wall_z(11.0, 19.0 + t, 31.0, &[27.0], h, Drywall);
        // --- Centre: break room | cubicles | reception.
        b.wall_z(-11.0, -15.0 + t, 15.0 - t, &[-10.0, 0.0, 10.0], h, Drywall);
        b.wall_z(11.0, -15.0 + t, 15.0 - t, &[-8.0, 8.0], h, Drywall);
        // Conference room in the middle of the cubicles.
        b.wall_x(-5.0, -4.0 - t, 4.0 + t, &[0.0], h, Drywall);
        b.wall_x(5.0, -4.0 - t, 4.0 + t, &[-1.0], h, Drywall);
        b.wall_z(-4.0, -5.0 + t, 5.0 - t, &[], h, Drywall);
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
        // Reception.
        b.low(-3.0, 15.0, 3.0, 16.0, Desk);
        b.low(-14.4, 20.4, -13.6, 21.2, Barrel); // planters
        b.low(13.6, 20.4, 14.4, 21.2, Barrel);
        // Corridor chicanes (water coolers / cabinets).
        b.tall(-19.0 + t, -0.5, -17.8, 0.5, 2.0, Cabinet);
        b.tall(-16.2, 9.5, -15.0 - t, 10.5, 2.0, Cabinet);
        b.tall(15.0 + t, -9.5, 16.2, -8.5, 2.0, Cabinet);
        b.tall(17.8, 8.5, 19.0 - t, 9.5, 2.0, Cabinet);
        // West rooms: records (shelves), office, copy room, manager's office.
        b.tall(-28.0, -19.0, -22.0, -18.4, 2.2, Cabinet);
        b.tall(-28.0, -15.0, -22.0, -14.4, 2.2, Cabinet);
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
        })
    }

    pub fn zone_of(&self, p: V2) -> Option<&Zone> {
        self.zones
            .iter()
            .find(|z| p.x >= z.min.x && p.x <= z.max.x && p.y >= z.min.y && p.y <= z.max.y)
    }

    pub fn in_bounds(&self, p: V2) -> bool {
        p.x.abs() < ARENA_HALF - AGENT_RADIUS && p.y.abs() < ARENA_HALF - AGENT_RADIUS
    }

    /// Push a point out of any block it overlaps (used by the 2D sim for collision).
    pub fn resolve_collision(&self, mut p: V2, radius: f32) -> V2 {
        for b in &self.blocks {
            if !b.contains(p, radius) {
                continue;
            }
            let d = p - b.center;
            let px = b.half.x + radius - d.x.abs();
            let pz = b.half.y + radius - d.y.abs();
            if px < pz {
                p.x += px * d.x.signum();
            } else {
                p.y += pz * d.y.signum();
            }
        }
        let lim = ARENA_HALF - radius;
        V2::new(p.x.clamp(-lim, lim), p.y.clamp(-lim, lim))
    }
}
