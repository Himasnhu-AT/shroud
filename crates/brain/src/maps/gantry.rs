//! "Gantry": a bomb-defusal layout in a harbour rail yard on a grey sea morning.
//!
//! ```text
//!              north (defenders)
//!   +----------------------+---------+--+------------------+
//!   |  the warehouse       | harbour |  |  B site (quay,   |
//!   |  = A site (indoor,   | office  |con  gantry crane,   |
//!   |  shelving, office)   | def.    |  |  containers)     |
//!   |                 back>| spawn   +--+                  |
//!   +--roller door--door---+-gate----+--pump house--+      |
//!   |A yard |   A link     |   |   |   |  (B short)  | B  |
//!   |(con-  +--------------+ w | m | e +-------------+quay|
//!   |tainer |   sheds      | t | i | t |  customs    |    |
//!   |stacks)|              | r | d | r |             |    |
//!   +-------+--------------+---+---+---+-------------+----+
//!   |              attacker spawn (truck gate)             |
//!   +------------------------------------------------------+
//!              south (attackers)
//! ```
//!
//! Routes: **A yard** (the long way: container stacks up to the warehouse roller door),
//! **A link** (off the west tracks to the warehouse's small door), **mid** (three lanes
//! between two parked trains; the centre track is the sniper lane from the defender spawn
//! through the mid gate down to the parked trailer), **B short** (off the east tracks
//! through the pump house), **B quay** (the long open quay along the sea wall). Defenders
//! reach A through the warehouse back door and B through the B connector.

use crate::arena::{Arena, BuildingUse, Builder, FloorSkin, LampStyle, MapId, Paint, Roof, Skin, zone};
use crate::building::{Building, Opening, Side};
use crate::math::V2;

/// Half extent: outer walls on ±40, playable floor ±39.
pub const GANTRY_HALF: f32 = 40.0;

/// Track centre lines (x) through the yard.
const TRACKS: [f32; 3] = [-6.9, 0.0, 6.9];

pub fn gantry() -> Arena {
    use Skin::*;
    let mut b = Builder::sized(GANTRY_HALF, 7.0, Concrete);

    // ------------------------------------------------------------------ attacker spawn (south)
    b.tall(-6.0, 36.0, 6.0, 39.0, 5.0, Concrete); // gatehouse
    b.tall(-5.0, 25.5, 5.0, 28.0, 3.0, Container); // parked trailer: the sniper lane ends here
    b.tall(-30.0, 30.0, -27.6, 37.0, 2.9, Container); // a truck
    b.low(-30.0, 28.2, -27.6, 30.0, Container);
    b.low(-18.0, 31.0, -14.0, 31.6, Sandbag);
    b.low(12.0, 30.0, 16.0, 30.6, Sandbag);
    b.tall(20.0, 33.0, 26.0, 35.4, 2.6, Container);
    b.tall(20.0, 35.4, 26.0, 37.8, 5.2, Container);
    b.tall(30.0, 30.0, 39.0, 39.0, 8.0, Siding); // truck shed
    b.tall(-39.0, 33.0, -33.0, 39.0, 6.0, Brick);
    b.low(8.0, 33.0, 9.2, 34.2, Crate);

    // ------------------------------------------------------------------ west: A yard, A link, the warehouse (A site)
    Building::new("the warehouse", BuildingUse::Warehouse, -39.0, -39.0, -12.0, -14.0, 7.5, Siding)
        .opening(Side::South, Opening::wide_door(-31.0, 4.0, 3.8))
        .door(Side::South, -16.0)
        .windows(Side::South, &[-22.0], 2.0)
        .door(Side::East, -31.0)
        .windows(Side::East, &[-21.0], 2.0)
        .partition_x(-31.0, -38.6, -32.0, &[Opening::door(-34.0)])
        .partition_z(-32.0, -38.6, -31.15, &[])
        .dark_room("the A office", -38.6, -38.6, -31.15, -32.15, None)
        .room("A site", -38.6, -38.6, -12.4, -14.4, Some(FloorSkin::Concrete))
        .lamp(-32.0, -20.0)
        .lamp(-18.0, -33.0)
        .lamp(-18.0, -20.0)
        .lamp(-35.0, -35.0)
        .tall(-37.0, -29.0, -31.0, -28.2, 3.0, Shelf)
        .tall(-37.0, -25.0, -31.0, -24.2, 3.0, Shelf)
        .tall(-22.0, -34.0, -16.0, -31.6, 2.6, Container)
        .tall(-26.0, -24.0, -23.6, -21.6, 2.4, Crate) // default boxes
        .low(-23.6, -22.8, -22.4, -21.6, Crate)
        .low(-17.0, -23.0, -15.4, -20.6, Cabinet) // forklift
        .low(-35.0, -18.0, -33.8, -16.8, Crate)
        .low(-21.0, -17.2, -19.2, -16.0, Crate)
        .low(-38.4, -31.0, -36.0, -30.4, Desk)
        .low(-29.0, -37.0, -26.6, -35.8, Crate)
        .emit(&mut b);
    // A link: a fenced loading apron off the west tracks, walled off from the A yard.
    b.tall(-27.4, -14.0, -27.0, -6.0, 3.6, Concrete);
    b.tall(-24.0, -11.0, -21.6, -8.6, 2.6, Crate);
    b.low(-17.0, -9.0, -15.8, -7.8, Barrel);
    // The sheds between the A yard and the tracks.
    b.tall(-27.0, -6.0, -12.0, 8.0, 6.0, Siding);
    b.tall(-27.0, 8.0, -12.0, 13.0, 7.0, Brick);
    b.tall(-27.0, 17.0, -12.0, 24.0, 7.0, Brick);
    // Cut-throughs from the tracks: west to the A yard, east to the quay.
    b.low(-22.0, 13.4, -20.8, 14.6, Crate);
    b.low(17.0, 15.6, 18.2, 16.8, Barrel);
    // A yard: container stacks (some two high) and pallets.
    b.tall(-37.0, 10.0, -34.6, 16.0, 2.6, Container);
    b.tall(-33.0, -2.0, -30.6, 4.0, 5.2, Container);
    b.tall(-31.0, 18.0, -28.6, 24.0, 2.6, Container);
    b.low(-38.6, -6.0, -37.0, -4.6, Crate);
    b.low(-36.0, -10.0, -34.8, -8.8, Barrel);
    b.low(-31.0, -11.0, -28.6, -10.4, Sandbag);
    b.low(-38.6, 21.0, -37.4, 22.2, Crate);

    // ------------------------------------------------------------------ mid: the rail yard
    // Two parked trains (box wagons and a flatbed each) make three lanes.
    b.tall(-8.2, -12.0, -5.6, 0.0, 2.9, Container);
    b.tall(-8.2, 3.0, -5.6, 13.0, 2.9, Container);
    b.low(-8.2, 19.0, -5.6, 20.6, Crate); // sleepers stacked by the track
    b.tall(5.6, -10.0, 8.2, 2.0, 2.9, Container);
    b.tall(5.6, 5.0, 8.2, 13.0, 2.9, Container);
    b.low(6.0, 20.0, 7.6, 21.6, Crate);
    b.low(-1.2, 24.0, 1.2, 24.6, Cabinet); // buffer stop
    b.low(-12.0, 18.0, -10.8, 19.2, Crate);
    b.low(10.8, 6.0, 12.0, 7.2, Barrel);
    // The mid gate: a wall across the yard with a signal gantry over the gap.
    b.tall(-12.0, -17.0, -2.0, -16.0, 4.0, Concrete);
    b.tall(2.0, -17.0, 12.0, -16.0, 4.0, Concrete);
    b.decor.headers.push(crate::arena::Header { min: V2::new(-2.0, -17.0), max: V2::new(2.0, -16.0), bottom: 3.4, top: 4.0 });

    // ------------------------------------------------------------------ east: the pump house (B short), customs, the quay
    b.tall(12.0, -39.0, 14.0, -34.0, 6.0, Concrete);
    b.tall(12.0, -29.5, 14.0, -12.0, 6.0, Concrete);
    Building::new("the pump house", BuildingUse::Workshop, 12.0, -12.0, 26.0, -2.0, 4.5, Brick)
        .door(Side::West, -7.0)
        .door(Side::North, 20.0)
        .windows(Side::North, &[16.0], 1.6)
        .windows(Side::East, &[-5.0], 1.6)
        .partition_x(19.0, -11.6, -2.4, &[Opening::door(-6.0)])
        .room("the pump house", 12.4, -11.6, 18.85, -2.4, Some(FloorSkin::Concrete))
        .room("the pump room", 19.15, -11.6, 25.6, -2.4, Some(FloorSkin::Tiles))
        .tall(22.0, -4.6, 25.6, -2.4, 2.2, Cabinet)
        .low(15.0, -11.6, 17.4, -10.8, Shelf)
        .low(22.4, -10.0, 24.0, -8.4, Barrel)
        .emit(&mut b);
    b.tall(12.0, -2.0, 26.0, 10.0, 7.5, Concrete);
    b.tall(12.0, 10.0, 26.0, 13.0, 6.0, Siding);
    b.tall(12.0, 17.0, 26.0, 24.0, 6.0, Siding);
    // The quay: containers, a barrier, bollards along the sea wall.
    b.tall(30.0, 12.0, 32.4, 18.0, 2.6, Container);
    b.tall(34.0, 0.0, 36.4, 6.0, 5.2, Container);
    b.tall(28.0, -10.0, 30.4, -6.0, 2.6, Container);
    b.low(27.0, 4.0, 29.4, 4.6, Sandbag);
    for z in [-4.0, 9.0, 22.0] {
        b.low(37.8, z, 38.6, z + 0.8, Barrel);
    }

    // ------------------------------------------------------------------ defenders (north)
    b.tall(-8.0, -39.0, 8.0, -35.0, 8.0, Concrete); // harbour office
    b.low(-6.0, -24.0, -2.0, -23.4, Sandbag);
    b.low(3.0, -28.0, 7.0, -27.4, Sandbag);
    b.low(8.0, -33.0, 9.6, -30.6, Cabinet); // forklift
    b.tall(-10.0, -21.0, -7.6, -18.6, 2.4, Crate);

    // ------------------------------------------------------------------ B site: the quay under the crane
    for (x, z) in [(18.0, -31.0), (35.0, -31.0), (18.0, -22.0), (35.0, -22.0)] {
        b.tall(x, z, x + 1.0, z + 1.0, 9.0, Cabinet); // crane legs
    }
    for z in [-31.0, -22.0] {
        b.decor.roofs.push(Roof { min: V2::new(17.5, z), max: V2::new(36.5, z + 1.0), bottom: 9.0, thickness: 0.6 });
    }
    b.decor.roofs.push(Roof { min: V2::new(25.0, -30.0), max: V2::new(28.0, -22.0), bottom: 8.4, thickness: 0.6 }); // trolley
    b.tall(22.0, -37.0, 28.0, -34.6, 2.6, Container);
    b.tall(28.0, -37.0, 34.0, -34.6, 5.2, Container);
    b.tall(24.0, -27.0, 26.4, -24.6, 2.4, Crate); // default boxes
    b.low(26.4, -25.8, 27.6, -24.6, Crate);
    b.low(31.0, -27.0, 33.4, -26.4, Sandbag);
    b.tall(30.0, -18.0, 32.4, -14.0, 2.6, Container);
    b.low(15.0, -20.0, 16.2, -18.8, Crate);
    for z in [-36.0, -28.0, -18.0] {
        b.low(37.8, z, 38.6, z + 0.8, Barrel);
    }

    // ------------------------------------------------------------------ lamps
    for (x, z, fx, fz) in [(26.0, 4.0, 1, 0), (26.0, 18.0, 1, 0), (-4.0, -35.0, 0, 1), (4.0, -35.0, 0, 1), (-27.0, 2.0, -1, 0), (-27.0, 20.0, -1, 0)] {
        b.lamp_styled(x, z, 4.2, LampStyle::Wall { facing_x: fx, facing_z: fz });
    }

    // ------------------------------------------------------------------ signs
    b.sign_wall("A", -24.0, -14.0, 4.6, 2.2, (0, 1), Paint::White, Some(Paint::Red));
    b.sign_wall("A", -22.0, -38.6, 4.2, 2.6, (0, 1), Paint::White, Some(Paint::Red));
    b.sign_wall("A", -12.0, -25.0, 4.6, 1.8, (1, 0), Paint::White, Some(Paint::Red));
    b.sign_wall("B", 27.0, -39.0, 3.8, 2.6, (0, 1), Paint::White, Some(Paint::Blue));
    b.sign_wall("B", 23.5, -12.0, 3.1, 1.3, (0, -1), Paint::White, Some(Paint::Blue));
    b.sign_floor("A", -28.0, -19.0, 3.0, (0, -1), Paint::Red);
    b.sign_floor("B", 21.0, -26.0, 3.0, (0, -1), Paint::Blue);
    b.sign_wall("< A", -20.0, 24.0, 2.6, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("B >", 19.0, 24.0, 2.6, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("MID", 0.0, 28.0, 1.9, 0.6, (0, 1), Paint::Black, Some(Paint::White));
    b.sign_wall("A >", -12.0, -3.0, 2.4, 0.8, (1, 0), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("< B", 12.0, 1.0, 2.4, 0.8, (-1, 0), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("A >", -12.0, -28.0, 2.2, 0.8, (1, 0), Paint::White, Some(Paint::Red));
    b.sign_wall("< B", 12.0, -25.0, 2.4, 0.8, (-1, 0), Paint::White, Some(Paint::Blue));
    b.sign_floor("A ^", -33.5, 8.0, 1.2, (0, -1), Paint::Yellow);
    b.sign_floor("B ^", 31.0, 20.5, 1.2, (0, -1), Paint::Yellow);

    // ------------------------------------------------------------------ ground
    b.floor(-39.0, 26.0, 39.0, 39.0, FloorSkin::Asphalt);
    b.floor(-39.0, -14.0, -27.4, 26.0, FloorSkin::Pavement);
    b.floor(-27.0, -14.0, -12.0, -6.0, FloorSkin::Concrete);
    b.floor(-27.0, 13.0, -12.0, 17.0, FloorSkin::Concrete);
    b.floor(12.0, 13.0, 26.0, 17.0, FloorSkin::Concrete);
    for x in TRACKS {
        let w = if x == 0.0 { 1.6 } else { 2.3 };
        b.floor(x - w, -16.0, x + w, 26.0, FloorSkin::Gravel);
    }
    b.floor(-12.0, -39.0, 12.0, -17.0, FloorSkin::Asphalt);
    b.floor(14.0, -39.0, 39.0, -12.0, FloorSkin::Pavement);
    b.floor(26.0, -12.0, 39.0, 26.0, FloorSkin::Pavement);
    // Rails set into the ballast, the quay edge line, spawn borders.
    for x in TRACKS {
        b.paint(x - 0.78, -16.0, x - 0.7, 26.0, Paint::Steel);
        b.paint(x + 0.7, -16.0, x + 0.78, 26.0, Paint::Steel);
    }
    b.paint(37.3, -39.0, 37.55, 26.0, Paint::Yellow);
    b.outline(-21.0, 28.5, 21.0, 38.5, 0.25, Paint::Orange);
    b.outline(-11.0, -34.6, 11.0, -20.0, 0.25, Paint::Blue);

    let zones = vec![
        zone("attacker spawn", -39.0, 24.0, 39.0, 39.0),
        zone("A yard", -39.0, -14.0, -27.0, 26.0),
        zone("A link", -27.0, -14.0, -12.0, -6.0),
        zone("yard cut", -27.0, 13.0, -12.0, 17.0),
        zone("quay cut", 12.0, 13.0, 26.0, 17.0),
        zone("west tracks", -12.0, -16.0, -5.6, 26.0),
        zone("mid", -5.6, -16.0, 5.6, 26.0),
        zone("east tracks", 5.6, -16.0, 12.0, 26.0),
        zone("B quay", 26.0, -12.0, 39.0, 26.0),
        zone("B site", 14.0, -39.0, 39.0, -12.0),
        zone("B connector", 12.0, -34.0, 14.0, -29.5),
        zone("defender spawn", -12.0, -39.0, 12.0, -26.0),
        zone("mid gate", -12.0, -26.0, 12.0, -16.0),
    ];
    b.finish(
        MapId::Gantry,
        zones,
        V2::new(14.0, 33.0),
        vec![
            V2::new(-5.0, -32.0),
            V2::new(0.0, -32.5),
            V2::new(5.0, -32.0),
            V2::new(-5.0, -27.0),
            V2::new(5.0, -25.0),
            V2::new(0.0, -21.0),
            V2::new(-20.0, -27.0),
            V2::new(30.0, -31.0),
        ],
    )
}
