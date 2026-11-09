//! "Foundry": a bomb-defusal layout in a steelworks on a hazy evening.
//!
//! ```text
//!               north (defenders)
//!   +--------------------+------------+--+-------------------+
//!   | the casting hall   |  defender  |  |  loading docks    |
//!   | = A site (furnace, |  spawn     |B |  = B site (trucks |
//!   | ladles, moulds,  ctl  (offices) |con  at the dock shed,|
//!   | control room)    <door          |  |  coils, container)|
//!   +-roller--door-------+mid doors---+--+--scale--+ramp+----+
//!   | A long |  A apron  |            |  house   |  |  B   |
//!   |  (chi- | boiler    |            | (B short)|  | ramp |
//!   |  cane  | house     |    mid     +----------+  |      |
//!   |  of    +-west cut--+  (ladle    |  east cut    | rail |
//!   | balers)| pattern   |   car on   +----------+  | spur |
//!   | scrap  | shop      |   rails)   | compressor|  |(wag- |
//!   | yard   |           |            | house     |  | ons) |
//!   +--------+-----------+--spawn wall+-----------+--+------+
//!   |               attacker spawn (works gate)             |
//!   +-------------------------------------------------------+
//!               south (attackers)
//! ```
//!
//! Routes: **A long** (the scrap yard, an S-bend between two scrap balers, up to the hall's
//! roller door), **A short** (off mid through the boiler house and the A apron to the hall's
//! side door; the boiler house also opens west onto A long), **mid** (an 18 m yard with a
//! ladle car on rails; the defenders hold it from the mid doors under the conveyor),
//! **B short** (off mid through the scale house into the docks), **B ramp** (the rail spur
//! along the east wall, through the ramp gate). Two cuts link the lanes: the **west cut**
//! (A long ↔ mid, under a conveyor) and the **east cut** (mid ↔ rail spur). Defenders reach A
//! through the hall's east door and B through the B connector arch.

use crate::arena::{Arena, BuildingUse, Builder, FloorSkin, Header, LampStyle, MapId, Paint, Roof, Skin, zone};
use crate::building::{Building, Opening, Side};
use crate::math::V2;

/// Half extent: outer walls on ±40, playable floor ±39.
pub const FOUNDRY_HALF: f32 = 40.0;

/// Overhead header (arch, gate beam) between two wall ends.
fn beam(b: &mut Builder, x0: f32, z0: f32, x1: f32, z1: f32, bottom: f32, top: f32) {
    b.decor.headers.push(Header { min: V2::new(x0, z0), max: V2::new(x1, z1), bottom, top });
}

/// A conveyor or pipe bridge overhead (visual; above any head).
fn bridge(b: &mut Builder, x0: f32, z0: f32, x1: f32, z1: f32, bottom: f32) {
    b.decor.roofs.push(Roof { min: V2::new(x0, z0), max: V2::new(x1, z1), bottom, thickness: 0.5 });
}

pub fn foundry() -> Arena {
    use Skin::*;
    let mut b = Builder::sized(FOUNDRY_HALF, 7.5, Concrete);

    // ------------------------------------------------------------------ attacker spawn (south)
    // The works gate: a gatehouse, a parked flatbed, a garage and a brick store.
    b.tall(-6.0, 36.0, 6.0, 39.0, 5.0, Concrete); // gatehouse
    b.tall(-4.0, 24.0, 4.0, 25.4, 3.4, Concrete); // spawn wall: mid's long sightline ends here
    b.tall(-30.0, 29.0, -27.6, 36.0, 2.9, Container); // flatbed with a load
    b.low(-18.0, 30.0, -14.0, 30.6, Sandbag);
    b.low(12.0, 30.0, 16.0, 30.6, Sandbag);
    b.tall(30.0, 31.0, 39.0, 39.0, 7.0, Siding); // garage
    b.tall(-39.0, 33.0, -33.0, 39.0, 6.0, Brick); // store
    b.low(8.0, 33.0, 9.2, 34.2, Crate);
    b.low(-10.0, 27.0, -8.8, 28.2, Barrel);
    b.low(22.0, 27.0, 24.4, 28.2, Crate);

    // ------------------------------------------------------------------ west: scrap yard, A long
    // Scrap balers make an S-bend halfway up the lane (the A long choke).
    b.tall(-39.0, 2.0, -32.0, 4.0, 3.6, Container);
    b.tall(-34.0, -4.0, -27.0, -2.0, 3.6, Container);
    // Scrap piles and skips south of the balers.
    b.tall(-37.0, 10.0, -34.6, 16.0, 2.6, Container);
    b.tall(-31.0, 16.0, -28.6, 21.0, 5.2, Container);
    b.low(-30.0, 8.0, -28.8, 9.2, Crate);
    b.low(-38.6, 20.0, -37.4, 21.2, Barrel);
    b.low(-38.6, 21.2, -37.4, 22.4, Barrel);
    b.low(-33.0, 6.0, -31.0, 6.6, Sandbag);
    // A long proper: boxes on the corner into the roller door.
    b.tall(-31.0, -12.0, -29.0, -10.0, 2.4, Crate);
    b.low(-29.0, -11.2, -27.8, -10.0, Crate);
    b.low(-38.6, -9.0, -37.4, -7.8, Barrel);

    // ------------------------------------------------------------------ the casting hall (A site)
    Building::new("the casting hall", BuildingUse::Warehouse, -39.0, -39.0, -14.0, -15.0, 8.0, Siding)
        .opening(Side::South, Opening::wide_door(-31.0, 4.0, 4.0)) // roller door off A long
        .door(Side::South, -20.0) // side door off the A apron
        .windows(Side::South, &[-36.0], 2.0)
        .door(Side::East, -29.0) // defenders' door
        .windows(Side::East, &[-20.0], 2.0)
        .partition_x(-21.0, -38.6, -32.0, &[Opening::window(-35.5, 1.6)])
        .partition_z(-32.0, -20.85, -14.4, &[Opening::door(-17.5)])
        .dark_room("the control room", -20.85, -38.6, -14.4, -32.15, None)
        .room("the casting hall", -38.6, -38.6, -14.4, -15.4, Some(FloorSkin::Concrete))
        .lamp(-34.0, -20.0)
        .lamp(-19.0, -22.0)
        .lamp(-17.5, -35.5)
        .tall(-38.6, -38.6, -33.0, -33.0, 6.0, Concrete) // the furnace
        .tall(-30.0, -36.0, -27.4, -33.4, 2.4, Barrel) // ladles
        .tall(-26.0, -36.0, -23.4, -33.4, 2.4, Barrel)
        .low(-34.0, -22.0, -30.0, -21.2, Container) // ingot moulds
        .low(-34.0, -27.0, -30.0, -26.2, Container)
        .tall(-26.0, -26.0, -23.6, -23.6, 2.4, Crate) // default boxes
        .low(-23.6, -24.8, -22.4, -23.6, Crate)
        .low(-19.0, -22.0, -17.8, -20.8, Crate)
        .low(-38.4, -26.0, -37.2, -23.0, Cabinet) // tool lockers
        .low(-20.5, -38.6, -17.0, -37.8, Desk) // control desk
        .emit(&mut b);
    b.site('A', -35.0, -31.0, -19.0, -16.0);

    // ------------------------------------------------------------------ the boiler house (A short)
    Building::new("the boiler house", BuildingUse::Workshop, -27.0, -12.0, -9.0, -2.0, 4.5, Brick)
        .door(Side::East, -7.0)
        .door(Side::North, -20.0)
        .door(Side::West, -7.0)
        .windows(Side::South, &[-13.0], 1.6)
        .partition_x(-18.0, -11.6, -2.4, &[Opening::door(-7.0)])
        .room("the boiler room", -26.6, -11.6, -18.15, -2.4, Some(FloorSkin::Concrete))
        .room("the boiler house", -17.85, -11.6, -9.4, -2.4, Some(FloorSkin::Tiles))
        .tall(-26.6, -11.6, -23.6, -9.6, 3.2, Barrel) // the boiler
        .low(-14.0, -3.2, -11.0, -2.4, Shelf) // workbench
        .low(-11.0, -11.6, -9.8, -10.4, Crate)
        .emit(&mut b);
    // Yard wall between the A apron and top mid.
    b.tall(-14.0, -16.6, -9.0, -12.0, 4.0, Brick);

    // ------------------------------------------------------------------ west cut, pattern shop
    b.tall(-27.0, 2.0, -9.0, 24.0, 6.5, Brick); // the pattern shop
    bridge(&mut b, -27.0, -1.0, -9.0, 1.0, 4.8); // conveyor over the west cut
    b.low(-20.0, -1.6, -18.8, -0.4, Crate);

    // ------------------------------------------------------------------ mid
    b.tall(-1.3, 4.0, 1.3, 10.0, 3.0, Container); // the ladle car on its rails
    b.low(-7.0, 14.0, -5.8, 15.2, Crate);
    b.tall(4.0, 16.0, 6.4, 18.4, 2.4, Crate);
    b.low(5.0, -8.0, 6.2, -6.8, Barrel);
    b.low(-2.0, -12.0, 2.0, -11.4, Sandbag);
    b.low(-7.6, -4.0, -6.4, -2.8, Crate);
    bridge(&mut b, -9.0, -7.0, 9.0, -5.4, 5.0); // conveyor across mid
    // Mid doors: the defenders' end of mid, a beam over the gap.
    b.tall(-14.0, -18.0, -1.6, -16.6, 5.0, Concrete);
    b.tall(1.6, -18.0, 13.0, -16.6, 5.0, Concrete);
    beam(&mut b, -1.6, -18.0, 1.6, -16.6, 3.4, 5.0);
    b.tall(9.0, -16.6, 13.0, -15.0, 4.0, Brick);

    // ------------------------------------------------------------------ east: scale house (B short), east cut, compressor house
    Building::new("the scale house", BuildingUse::Workshop, 9.0, -15.0, 26.0, -4.0, 4.5, Brick)
        .door(Side::West, -10.0)
        .door(Side::North, 22.5)
        .windows(Side::North, &[18.0], 1.6)
        .windows(Side::East, &[-8.0], 1.6)
        .partition_x(16.0, -14.6, -4.4, &[Opening::door(-10.0)])
        .room("the weigh office", 9.4, -14.6, 15.85, -4.4, Some(FloorSkin::Tiles))
        .room("the scale house", 16.15, -14.6, 25.6, -4.4, Some(FloorSkin::Concrete))
        .low(10.0, -14.6, 12.4, -13.8, Desk)
        .tall(15.0, -6.0, 15.85, -4.4, 2.0, Cabinet)
        .low(20.0, -7.0, 22.4, -5.8, Crate)
        .emit(&mut b);
    b.tall(9.0, 0.0, 26.0, 24.0, 7.0, Concrete); // the compressor house
    b.low(16.0, -3.4, 17.2, -2.2, Barrel);
    b.low(23.0, -1.6, 24.2, -0.4, Crate);

    // ------------------------------------------------------------------ rail spur (B ramp)
    b.tall(31.8, -8.0, 34.2, 2.0, 3.2, Container); // hopper wagons
    b.tall(31.8, 8.0, 34.2, 18.0, 3.2, Container);
    b.low(28.0, -11.0, 29.2, -9.8, Crate);
    b.low(37.8, 20.0, 38.6, 20.8, Barrel);
    b.low(37.8, 4.0, 38.6, 4.8, Barrel);
    b.low(27.0, 12.0, 29.4, 12.6, Sandbag);
    // The ramp gate into the docks.
    b.tall(26.0, -16.0, 31.0, -14.0, 4.6, Concrete);
    b.tall(35.0, -16.0, 39.0, -14.0, 4.6, Concrete);
    beam(&mut b, 31.0, -16.0, 35.0, -14.0, 4.0, 4.6);

    // ------------------------------------------------------------------ defenders (north)
    b.tall(-7.0, -39.0, 7.0, -36.0, 8.0, Concrete); // the works office
    b.low(-6.0, -25.0, -2.0, -24.4, Sandbag);
    b.low(3.0, -28.0, 7.0, -27.4, Sandbag);
    b.low(8.0, -33.0, 9.6, -30.6, Cabinet); // forklift
    b.tall(-11.0, -22.0, -8.6, -19.6, 2.4, Crate);
    // The wall between the defender spawn and the docks, with the B connector arch.
    b.tall(13.0, -39.0, 15.0, -34.0, 6.5, Brick);
    b.tall(13.0, -29.0, 15.0, -15.0, 6.0, Brick);
    beam(&mut b, 13.0, -34.0, 15.0, -29.0, 3.0, 6.0);

    // ------------------------------------------------------------------ loading docks (B site)
    b.tall(15.0, -39.0, 39.0, -35.0, 7.0, Siding); // the dock shed
    bridge(&mut b, 15.0, -35.0, 26.0, -32.0, 4.2); // dock canopy
    b.tall(18.0, -35.0, 20.4, -28.0, 2.9, Container); // trailers backed onto the dock
    b.tall(23.0, -35.0, 25.4, -29.0, 2.9, Container);
    b.tall(27.0, -26.0, 29.4, -23.6, 2.4, Crate); // default boxes
    b.low(29.4, -24.8, 30.6, -23.6, Crate);
    b.low(34.0, -30.0, 35.2, -28.8, Barrel); // steel coils
    b.low(35.2, -30.0, 36.4, -28.8, Barrel);
    b.tall(34.0, -22.0, 39.0, -19.6, 2.6, Container);
    b.low(16.0, -21.0, 17.6, -18.6, Cabinet); // forklift
    b.low(20.0, -18.0, 23.0, -17.4, Sandbag);
    b.low(37.4, -34.6, 38.6, -33.4, Crate);
    b.site('B', 18.0, -31.0, 37.0, -17.0);

    // ------------------------------------------------------------------ lamps
    for (x, z, fx, fz) in [
        (-24.0, -15.0, 0, 1),
        (31.0, -35.0, 0, 1),
        (26.0, -9.0, 1, 0),
        (-9.0, 12.0, 1, 0),
        (9.0, 12.0, -1, 0),
        (-27.0, -9.0, -1, 0),
        (-4.0, -36.0, 0, 1),
        (4.0, -36.0, 0, 1),
        (-18.0, 24.0, 0, 1),
        (18.0, 24.0, 0, 1),
    ] {
        b.lamp_styled(x, z, 4.2, LampStyle::Wall { facing_x: fx, facing_z: fz });
    }
    b.lamp_styled(20.5, -33.5, 4.15, LampStyle::Bulkhead); // under the dock canopy

    // ------------------------------------------------------------------ signs
    b.sign_wall("A", -25.0, -15.0, 5.6, 2.4, (0, 1), Paint::White, Some(Paint::Red));
    b.sign_wall("A", -28.0, -38.6, 4.8, 2.6, (0, 1), Paint::White, Some(Paint::Red));
    b.sign_wall("A", -14.0, -24.0, 3.6, 1.8, (1, 0), Paint::White, Some(Paint::Red));
    b.sign_floor("A", -28.0, -19.0, 3.0, (0, -1), Paint::Red);
    b.sign_wall("B", 31.0, -35.0, 4.4, 2.6, (0, 1), Paint::White, Some(Paint::Blue));
    b.sign_wall("B", 13.0, -24.0, 3.4, 1.8, (-1, 0), Paint::White, Some(Paint::Blue));
    b.sign_floor("B", 24.0, -21.0, 3.0, (0, -1), Paint::Blue);
    // Directions from spawn.
    b.sign_wall("< A", -21.0, 24.0, 2.6, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("B >", 21.0, 24.0, 2.6, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("MID", 0.0, 25.4, 2.0, 0.6, (0, 1), Paint::Black, Some(Paint::White));
    b.sign_floor("A ^", -33.0, 12.0, 1.2, (0, -1), Paint::Yellow);
    b.sign_floor("B ^", 29.0, 6.0, 1.2, (0, -1), Paint::Yellow);
    // Off mid.
    b.sign_wall("A >", -9.0, -10.0, 2.4, 0.8, (1, 0), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("< B", 9.0, -13.0, 2.4, 0.8, (-1, 0), Paint::Black, Some(Paint::Yellow));
    // Defenders.
    b.sign_wall("MID", 0.0, -36.0, 2.6, 0.6, (0, 1), Paint::Black, Some(Paint::White));

    // ------------------------------------------------------------------ ground
    b.floor(-39.0, 24.0, 39.0, 39.0, FloorSkin::Asphalt);
    b.floor(-9.0, -16.6, 9.0, 24.0, FloorSkin::Pavement);
    b.floor(-14.0, -36.0, 13.0, -16.6, FloorSkin::Pavement);
    b.floor(-39.0, -15.0, -27.0, 24.0, FloorSkin::Gravel);
    b.floor(26.0, -14.0, 39.0, 24.0, FloorSkin::Gravel);
    b.floor(15.0, -35.0, 39.0, -16.0, FloorSkin::Concrete);
    b.floor(-27.0, -15.0, -14.0, -12.0, FloorSkin::Concrete);
    b.floor(-27.0, -2.0, -9.0, 2.0, FloorSkin::Concrete);
    b.floor(9.0, -4.0, 26.0, 0.0, FloorSkin::Concrete);
    // Rails set into the ground (mid's ladle track, the spur), the dock edge, spawn borders.
    for (x, z0, z1) in [(0.0, -16.6, 24.0), (33.0, -14.0, 24.0)] {
        b.paint(x - 0.78, z0, x - 0.7, z1, Paint::Steel);
        b.paint(x + 0.7, z0, x + 0.78, z1, Paint::Steel);
    }
    b.paint(15.0, -34.9, 39.0, -34.6, Paint::Yellow);
    b.outline(-22.0, 27.0, 22.0, 35.6, 0.25, Paint::Orange);
    b.outline(-12.0, -35.6, 12.0, -23.0, 0.25, Paint::Blue);

    let zones = vec![
        zone("attacker spawn", -39.0, 24.0, 39.0, 39.0),
        zone("the scrap yard", -39.0, 4.0, -27.0, 24.0),
        zone("the balers", -39.0, -4.0, -27.0, 4.0),
        zone("A long", -39.0, -15.0, -27.0, -4.0),
        zone("A apron", -27.0, -15.0, -14.0, -12.0),
        zone("west cut", -27.0, -2.0, -9.0, 2.0),
        zone("mid", -9.0, -16.6, 9.0, 24.0),
        zone("mid doors", -14.0, -24.0, 13.0, -16.6),
        zone("defender spawn", -14.0, -39.0, 13.0, -24.0),
        zone("B connector", 13.0, -34.0, 15.0, -29.0),
        zone("the loading docks", 15.0, -39.0, 39.0, -14.0),
        zone("east cut", 9.0, -4.0, 26.0, 0.0),
        zone("B ramp", 26.0, -14.0, 39.0, -8.0),
        zone("the rail spur", 26.0, -8.0, 39.0, 24.0),
    ];
    b.finish(
        MapId::Foundry,
        zones,
        V2::new(0.0, 32.0),
        vec![
            V2::new(-5.0, -32.0),
            V2::new(0.0, -32.5),
            V2::new(5.0, -32.0),
            V2::new(-6.0, -28.0),
            V2::new(6.0, -25.0),
            V2::new(0.0, -21.0),
            V2::new(-27.0, -29.0),
            V2::new(29.0, -29.0),
        ],
    )
}
