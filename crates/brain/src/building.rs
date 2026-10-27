//! Building kit: describe a single-storey building as a footprint, openings per side (doors,
//! windows), interior partitions, rooms and furniture, and emit it into a map [`Builder`] as
//! plain arena data the 2D AI already understands:
//!
//! - exterior walls are `Tall` blocks, `2 * EXTERIOR_HALF` thick, cut by the openings;
//! - a **doorway** is a gap plus a [`Header`] (lintel) from the door head to the wall top;
//! - a **window** is a `Low` block (the sill, `SILL_HEIGHT` high: stand to see and shoot over
//!   it, crouch behind it to hide — exactly `Low` semantics) plus a [`Window`] decor entry
//!   (glass up to `WINDOW_TOP`, lintel above, drawn by the renderer);
//! - rooms become named zones (callouts), with an optional floor patch and a ceiling lamp;
//! - the roof is a [`Roof`](crate::arena::Roof) slab whose underside is the wall top.
//!
//! Everything is single-storey: nav, LOS and cover stay 2D.

use crate::arena::{Block, BlockKind, Builder, BuildingInfo, BuildingUse, DOOR_HEIGHT, DOOR_WIDTH, FloorSkin, Header, LampStyle, PARTITION_HALF, Skin, Window};
use crate::math::V2;

/// Half thickness of exterior walls (0.4 m walls).
pub const EXTERIOR_HALF: f32 = 0.2;
/// Window sill height: below eye height standing, above it crouched.
pub const SILL_HEIGHT: f32 = 1.0;
/// Underside of a window lintel.
pub const WINDOW_TOP: f32 = 2.2;
/// Default wall height of a house (ceiling underside).
pub const STOREY: f32 = 3.2;

/// A side of a building footprint. North = −z (`min.y`), South = +z, West = −x, East = +x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    North,
    South,
    West,
    East,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::North, Side::South, Side::West, Side::East];
    pub fn index(self) -> usize {
        self as usize
    }
}

/// A gap in a wall line. `at` is the centre along the wall (x for walls running along x,
/// z for walls running along z), in world metres.
#[derive(Clone, Copy, Debug)]
pub enum Opening {
    /// A doorway `width` wide whose lintel underside is at `head`.
    Door { at: f32, width: f32, head: f32 },
    /// A window `width` wide over a sill.
    Window { at: f32, width: f32, glass: bool },
}

impl Opening {
    /// A standard doorway (`DOOR_WIDTH`, lintel at `DOOR_HEIGHT`). Centre doors on whole
    /// metres so the 1 m nav grid gets two walkable cells through them.
    pub fn door(at: f32) -> Self {
        Opening::Door { at, width: DOOR_WIDTH, head: DOOR_HEIGHT }
    }
    /// A wide vehicle / roller door.
    pub fn wide_door(at: f32, width: f32, head: f32) -> Self {
        Opening::Door { at, width, head }
    }
    /// A glazed window.
    pub fn window(at: f32, width: f32) -> Self {
        Opening::Window { at, width, glass: true }
    }
    fn span(&self) -> (f32, f32) {
        match *self {
            Opening::Door { at, width, .. } | Opening::Window { at, width, .. } => (at - width * 0.5, at + width * 0.5),
        }
    }
}

/// Which way a wall line runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Along x, at constant z = `coord`.
    X,
    /// Along z, at constant x = `coord`.
    Z,
}

/// A straight wall from `a0` to `a1` along `axis`, centred on `coord`.
#[derive(Clone, Copy, Debug)]
pub struct WallLine {
    pub axis: Axis,
    pub coord: f32,
    pub a0: f32,
    pub a1: f32,
    pub half_thickness: f32,
    pub height: f32,
    pub skin: Skin,
    pub exterior: bool,
}

impl WallLine {
    fn rect(&self, a: f32, b: f32) -> (V2, V2) {
        let t = self.half_thickness;
        match self.axis {
            Axis::X => (V2::new(a, self.coord - t), V2::new(b, self.coord + t)),
            Axis::Z => (V2::new(self.coord - t, a), V2::new(self.coord + t, b)),
        }
    }
}

impl Builder {
    /// Emit a wall line with doors and windows cut in. Returns the indices of the blocks it
    /// added (wall segments and sills).
    pub(crate) fn wall_line(&mut self, w: &WallLine, openings: &[Opening]) -> Vec<usize> {
        let mut ops = openings.to_vec();
        ops.sort_by(|a, b| a.span().0.total_cmp(&b.span().0));
        let mut out = Vec::new();
        let mut start = w.a0;
        let solid = |b: &mut Builder, a: f32, z: f32, out: &mut Vec<usize>| {
            if z - a > 0.05 {
                let (min, max) = w.rect(a, z);
                out.push(b.blocks.len());
                b.blocks.push(Block::rect(min.x, min.y, max.x, max.y, BlockKind::Tall, w.height, w.skin));
            }
        };
        for o in ops {
            let (g0, g1) = o.span();
            debug_assert!(g0 >= start - 1e-3 && g1 <= w.a1 + 1e-3, "opening {o:?} outside its wall or overlapping another");
            solid(self, start, g0, &mut out);
            let (min, max) = w.rect(g0, g1);
            match o {
                Opening::Door { head, .. } => {
                    if w.height > head + 0.05 {
                        self.decor.headers.push(Header { min, max, bottom: head, top: w.height });
                    }
                }
                Opening::Window { glass, .. } => {
                    out.push(self.blocks.len());
                    self.blocks.push(Block::rect(min.x, min.y, max.x, max.y, BlockKind::Low, SILL_HEIGHT, w.skin));
                    self.decor.windows.push(Window {
                        min,
                        max,
                        sill: SILL_HEIGHT,
                        top: WINDOW_TOP.min(w.height - 0.1),
                        wall_top: w.height,
                        glass,
                        exterior: w.exterior,
                    });
                }
            }
            start = g1;
        }
        solid(self, start, w.a1, &mut out);
        out
    }
}

struct Room {
    name: &'static str,
    min: V2,
    max: V2,
    floor: Option<FloorSkin>,
    lamp: bool,
}

/// A single-storey building, emitted into a [`Builder`] with [`Building::emit`].
pub struct Building {
    pub name: &'static str,
    pub use_: BuildingUse,
    min: V2,
    max: V2,
    height: f32,
    facade: Skin,
    interior: Skin,
    parapet: bool,
    openings: [Vec<Opening>; 4],
    open: [bool; 4],
    partitions: Vec<(WallLine, Vec<Opening>)>,
    furniture: Vec<Block>,
    rooms: Vec<Room>,
    lamps: Vec<(V2, LampStyle)>,
}

impl Building {
    /// Footprint `(x0, z0)`–`(x1, z1)` (outer faces), wall height `height`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(name: &'static str, use_: BuildingUse, x0: f32, z0: f32, x1: f32, z1: f32, height: f32, facade: Skin) -> Self {
        Building {
            name,
            use_,
            min: V2::new(x0.min(x1), z0.min(z1)),
            max: V2::new(x0.max(x1), z0.max(z1)),
            height,
            facade,
            interior: Skin::Drywall,
            parapet: matches!(use_, BuildingUse::Shop | BuildingUse::Office | BuildingUse::Warehouse),
            openings: Default::default(),
            open: [false; 4],
            partitions: Vec::new(),
            furniture: Vec::new(),
            rooms: Vec::new(),
            lamps: Vec::new(),
        }
    }

    /// Flat roof with a parapet (true) or a slab with eaves (false).
    pub fn parapet(mut self, on: bool) -> Self {
        self.parapet = on;
        self
    }

    /// This side is the wall of an adjoining wing (built by that wing): no wall here.
    pub fn open_side(mut self, side: Side) -> Self {
        self.open[side.index()] = true;
        self
    }

    pub fn opening(mut self, side: Side, o: Opening) -> Self {
        self.openings[side.index()].push(o);
        self
    }

    pub fn door(self, side: Side, at: f32) -> Self {
        self.opening(side, Opening::door(at))
    }

    pub fn windows(mut self, side: Side, at: &[f32], width: f32) -> Self {
        for &a in at {
            self.openings[side.index()].push(Opening::window(a, width));
        }
        self
    }

    /// Inner faces of the exterior walls: `(min, max)` of the interior.
    pub fn inner(&self) -> (V2, V2) {
        let e = 2.0 * EXTERIOR_HALF;
        let k = |s: Side| if self.open[s.index()] { 0.0 } else { e };
        (
            V2::new(self.min.x + k(Side::West), self.min.y + k(Side::North)),
            V2::new(self.max.x - k(Side::East), self.max.y - k(Side::South)),
        )
    }

    /// Interior partition along z at `x`, from `z0` to `z1`.
    pub fn partition_x(mut self, x: f32, z0: f32, z1: f32, openings: &[Opening]) -> Self {
        let w = WallLine { axis: Axis::Z, coord: x, a0: z0, a1: z1, half_thickness: PARTITION_HALF, height: self.height, skin: self.interior, exterior: false };
        self.partitions.push((w, openings.to_vec()));
        self
    }

    /// Interior partition along x at `z`, from `x0` to `x1`.
    pub fn partition_z(mut self, z: f32, x0: f32, x1: f32, openings: &[Opening]) -> Self {
        let w = WallLine { axis: Axis::X, coord: z, a0: x0, a1: x1, half_thickness: PARTITION_HALF, height: self.height, skin: self.interior, exterior: false };
        self.partitions.push((w, openings.to_vec()));
        self
    }

    /// Waist-high furniture (counter, table, bed, low shelf): crouch cover.
    pub fn low(mut self, x0: f32, z0: f32, x1: f32, z1: f32, skin: Skin) -> Self {
        self.furniture.push(Block::rect(x0, z0, x1, z1, BlockKind::Low, crate::arena::LOW_COVER_HEIGHT, skin));
        self
    }

    /// Tall furniture (shelving, racks, wardrobes, crate stacks): blocks sight.
    pub fn tall(mut self, x0: f32, z0: f32, x1: f32, z1: f32, height: f32, skin: Skin) -> Self {
        self.furniture.push(Block::rect(x0, z0, x1, z1, BlockKind::Tall, height.min(self.height), skin));
        self
    }

    /// A named room (zone + optional floor patch + a ceiling lamp at its centre).
    pub fn room(mut self, name: &'static str, x0: f32, z0: f32, x1: f32, z1: f32, floor: Option<FloorSkin>) -> Self {
        self.rooms.push(Room { name, min: V2::new(x0.min(x1), z0.min(z1)), max: V2::new(x0.max(x1), z0.max(z1)), floor, lamp: true });
        self
    }

    /// A named room without its own lamp.
    pub fn dark_room(mut self, name: &'static str, x0: f32, z0: f32, x1: f32, z1: f32, floor: Option<FloorSkin>) -> Self {
        self.rooms.push(Room { name, min: V2::new(x0.min(x1), z0.min(z1)), max: V2::new(x0.max(x1), z0.max(z1)), floor, lamp: false });
        self
    }

    /// An extra ceiling lamp.
    pub fn lamp(mut self, x: f32, z: f32) -> Self {
        let style = self.lamp_style();
        self.lamps.push((V2::new(x, z), style));
        self
    }

    fn lamp_style(&self) -> LampStyle {
        match self.use_ {
            BuildingUse::House => LampStyle::Pendant,
            BuildingUse::Shop | BuildingUse::Office => LampStyle::Panel,
            BuildingUse::Warehouse | BuildingUse::Workshop => LampStyle::HighBay,
        }
    }

    /// Emit walls, openings, partitions, furniture, rooms, lamps and the roof. Returns the
    /// building's index in `decor.buildings`.
    pub(crate) fn emit(self, b: &mut Builder) -> usize {
        let e = EXTERIOR_HALF;
        let (min, max) = (self.min, self.max);
        let (imin, imax) = self.inner();
        let ext = |axis, coord, a0, a1| WallLine { axis, coord, a0, a1, half_thickness: e, height: self.height, skin: self.facade, exterior: true };
        let mut shell = Vec::new();
        // North/South walls span the full width (they own the corners); West/East fit between.
        let lines = [
            (Side::North, ext(Axis::X, min.y + e, min.x, max.x)),
            (Side::South, ext(Axis::X, max.y - e, min.x, max.x)),
            (Side::West, ext(Axis::Z, min.x + e, imin.y, imax.y)),
            (Side::East, ext(Axis::Z, max.x - e, imin.y, imax.y)),
        ];
        for (side, w) in lines {
            if !self.open[side.index()] {
                shell.extend(b.wall_line(&w, &self.openings[side.index()]));
            }
        }
        for (w, ops) in &self.partitions {
            b.wall_line(w, ops);
        }
        let style = self.lamp_style();
        b.blocks.extend(self.furniture);
        let lamp_h = self.height - 0.02;
        for r in &self.rooms {
            b.zone(r.name, r.min.x, r.min.y, r.max.x, r.max.y);
            if let Some(f) = r.floor {
                b.floor(r.min.x, r.min.y, r.max.x, r.max.y, f);
            }
            if r.lamp {
                let c = (r.min + r.max) * 0.5;
                b.lamp_styled(c.x, c.y, lamp_h, style);
            }
        }
        for (p, s) in self.lamps {
            b.lamp_styled(p.x, p.y, lamp_h, s);
        }
        let roof = b.decor.roofs.len();
        b.roof(min.x, min.y, max.x, max.y, self.height);
        b.decor.buildings.push(BuildingInfo {
            name: self.name,
            min,
            max,
            height: self.height,
            facade: self.facade,
            interior: self.interior,
            use_: self.use_,
            shell,
            parapet: self.parapet,
            roof,
            open_sides: self.open,
        });
        b.decor.buildings.len() - 1
    }
}
