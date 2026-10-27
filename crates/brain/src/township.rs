//! "Township": a 90 × 90 m town block on a clear late afternoon, built with the [building kit](crate::building).
//!
//! Main Street runs north–south through the middle (the long sniper lane, broken up by cars,
//! a checkpoint and a bus at the south end), the cross street runs east–west, and four blocks
//! hold eight enterable buildings: a warehouse with a mezzanine and a back office, a two-room
//! house, an L-shaped office (two wings), a corner shop with big windows, a brick house, a
//! workshop, and two more houses round a walled courtyard. Alleys, yards and a parking lot
//! with a fuel canopy give medium ranges; the interiors are close quarters. The player
//! spawns south behind the bus; the squad holds the north end.

use crate::arena::{Arena, BuildingUse, Builder, FloorSkin, LampStyle, MapId, Skin};
use crate::building::{Building, Opening, Side, STOREY};
use crate::math::V2;

/// Half extent: outer walls on ±45, playable floor ±44.
pub const TOWNSHIP_HALF: f32 = 45.0;

pub fn township() -> Arena {
    use BuildingUse::*;
    use Side::*;
    use Skin::*;
    let mut b = Builder::sized(TOWNSHIP_HALF, 5.0, Brick);

    // ------------------------------------------------------------------ north-west block
    // The warehouse: tall corrugated shed, two roller doors onto the loading yard, a back
    // office with an interior window, shelving aisles, and a mezzanine along the north wall.
    Building::new("the warehouse", Warehouse, -44.0, -44.0, -24.0, -28.0, 6.0, Siding)
        .opening(South, Opening::wide_door(-40.0, 4.0, 3.6))
        .opening(South, Opening::wide_door(-34.0, 4.0, 3.6))
        .windows(South, &[-31.0], 1.6)
        .windows(South, &[-26.0], 2.0)
        .door(East, -35.0)
        .windows(East, &[-40.0], 2.0)
        .partition_z(-32.5, -30.15, -24.4, &[Opening::door(-26.0)])
        .partition_x(-30.0, -32.35, -28.4, &[Opening::window(-30.5, 1.6)])
        .room("the warehouse office", -29.85, -32.35, -24.4, -28.4, None)
        .room("the warehouse", -43.6, -43.6, -24.4, -28.4, Some(FloorSkin::Concrete))
        .lamp(-38.0, -32.0)
        .lamp(-30.0, -38.0)
        .lamp(-33.0, -31.0)
        .tall(-42.5, -38.5, -34.0, -37.7, 2.8, Shelf)
        .tall(-42.5, -35.0, -34.0, -34.2, 2.8, Shelf)
        .tall(-31.0, -38.0, -29.0, -36.0, 2.4, Crate)
        .low(-31.0, -35.0, -29.8, -33.8, Crate)
        .low(-42.0, -33.5, -40.8, -32.3, Crate)
        .low(-37.5, -32.0, -36.3, -30.0, Cabinet)
        .low(-43.4, -31.0, -42.2, -29.8, Crate)
        .low(-29.0, -29.3, -27.0, -28.7, Desk)
        .emit(&mut b);
    // Mezzanine deck (overhead, visual + bullet-solid) on three pillars.
    b.decor.roofs.push(crate::arena::Roof { min: V2::new(-43.6, -43.6), max: V2::new(-30.0, -41.0), bottom: 3.0, thickness: 0.25 });
    for x in [-38.0, -33.5] {
        b.tall(x - 0.15, -41.3, x + 0.15, -41.0, 3.0, Concrete);
    }
    b.tall(-30.4, -41.3, -30.1, -41.0, 3.0, Concrete);

    // The blue house: kitchen and front room, windows on every side.
    Building::new("the blue house", House, -20.0, -40.0, -9.0, -30.0, STOREY, Plaster)
        .door(South, -11.0)
        .windows(South, &[-17.0], 1.6)
        .windows(North, &[-17.0, -12.0], 1.4)
        .door(West, -33.0)
        .windows(West, &[-37.0], 1.4)
        .windows(East, &[-37.0, -33.0], 1.6)
        .partition_x(-14.0, -39.6, -30.4, &[Opening::door(-35.0)])
        .room("the blue house kitchen", -19.6, -39.6, -14.15, -30.4, Some(FloorSkin::Tiles))
        .room("the blue house front room", -13.85, -39.6, -9.4, -30.4, Some(FloorSkin::Wood))
        .low(-19.6, -39.6, -15.5, -39.0, Desk)
        .low(-18.0, -36.5, -16.6, -35.0, Desk)
        .low(-12.5, -38.5, -10.0, -37.6, Desk)
        .tall(-13.85, -32.0, -13.25, -30.4, 2.0, Cabinet)
        .emit(&mut b);

    // The garden south of the house: a shed, a parked car, a low brick wall.
    b.tall(-23.5, -29.5, -21.5, -27.5, 2.6, Plaster);
    b.low(-19.0, -26.0, -17.2, -21.6, Car);
    b.low(-22.0, -12.2, -14.0, -11.8, Brick);
    b.low(-12.0, -20.0, -10.8, -18.8, Planter);

    // The loading yard: containers, a box truck, pallets, jersey barriers.
    b.tall(-43.0, -24.0, -37.0, -21.6, 2.6, Container);
    b.tall(-34.0, -20.0, -31.6, -14.0, 2.6, Container);
    b.tall(-28.0, -24.0, -25.6, -17.0, 3.0, Container);
    b.low(-28.0, -17.0, -25.6, -15.4, Container);
    b.low(-40.0, -12.0, -38.8, -10.8, Crate);
    b.low(-30.0, -10.0, -28.8, -8.8, Crate);
    b.low(-42.0, -8.0, -38.0, -7.4, Sandbag);

    // ------------------------------------------------------------------ north-east block
    // L-shaped office: wing A along the north lane, wing B down the east side, sharing
    // wing A's south wall (two doors through it).
    Building::new("the offices", Office, 10.0, -40.0, 34.0, -30.0, 3.6, Concrete)
        .door(West, -35.0)
        .door(North, 22.0)
        .windows(North, &[14.0, 26.0, 30.5], 2.0)
        .windows(South, &[12.5, 15.5, 21.0], 2.0)
        .door(South, 29.0)
        .windows(East, &[-35.0], 1.6)
        .partition_x(18.0, -39.6, -30.4, &[Opening::door(-35.0)])
        .room("the office lobby", 10.4, -39.6, 17.85, -30.4, Some(FloorSkin::Tiles))
        .room("the open-plan office", 18.15, -39.6, 33.6, -30.4, Some(FloorSkin::Carpet))
        .lamp(23.0, -35.0)
        .lamp(29.5, -35.0)
        .low(12.0, -33.4, 15.0, -32.6, Desk)
        .low(11.0, -39.4, 11.8, -38.6, Planter)
        .low(21.8, -37.3, 24.2, -35.7, Desk)
        .low(21.8, -33.8, 24.2, -32.2, Desk)
        .low(26.8, -37.3, 29.2, -35.7, Desk)
        .low(26.8, -33.8, 29.2, -32.2, Desk)
        .tall(31.6, -39.6, 33.6, -38.8, 2.0, Cabinet)
        .emit(&mut b);
    Building::new("the office wing", Office, 24.0, -30.0, 34.0, -14.0, 3.6, Concrete)
        .open_side(North)
        .door(West, -18.0)
        .windows(West, &[-28.5], 1.6)
        .windows(West, &[-25.0], 2.0)
        .windows(East, &[-26.0, -18.0], 1.6)
        .door(South, 29.0)
        .windows(South, &[26.0, 32.0], 1.4)
        .partition_z(-22.0, 24.4, 33.6, &[Opening::door(29.0)])
        .room("the meeting room", 24.4, -30.0, 33.6, -22.15, Some(FloorSkin::Carpet))
        .room("the server room", 24.4, -21.85, 33.6, -14.4, Some(FloorSkin::Tiles))
        .low(27.0, -27.5, 31.0, -25.0, Desk)
        .tall(24.8, -19.5, 27.6, -18.9, 2.2, Cabinet)
        .tall(30.4, -19.5, 33.2, -18.9, 2.2, Cabinet)
        .tall(24.8, -16.6, 27.6, -16.0, 2.2, Cabinet)
        .emit(&mut b);

    // The office courtyard: planters, benches, a fountain, a broken low wall to the street.
    for (x, z) in [(14.0, -26.0), (20.0, -26.0)] {
        b.low(x - 0.6, z - 0.6, x + 0.6, z + 0.6, Planter);
    }
    b.low(13.0, -20.4, 16.0, -19.8, Desk);
    b.low(16.0, -16.0, 18.0, -14.0, Planter);
    b.low(9.0, -8.2, 16.0, -7.8, Brick);
    b.low(18.0, -8.2, 23.0, -7.8, Brick);
    b.low(27.0, -11.0, 28.2, -9.8, Crate);

    // The east lot: parked cars and a dumpster.
    b.low(37.0, -36.0, 38.9, -31.6, Car);
    b.low(40.5, -28.0, 42.4, -23.6, Car);
    b.low(37.0, -20.0, 38.9, -15.6, Car);
    b.low(42.0, -12.0, 43.6, -10.2, Dumpster);
    // The north lane: a barrier.
    b.low(16.0, -42.3, 19.0, -41.7, Sandbag);

    // ------------------------------------------------------------------ south-east block
    // The corner shop: plate-glass windows onto both streets, aisles, a stockroom.
    Building::new("the corner shop", Shop, 9.0, 8.0, 23.0, 20.0, 3.8, Brick)
        .door(North, 14.0)
        .windows(North, &[11.0], 2.4)
        .windows(North, &[18.0], 2.8)
        .windows(North, &[21.0], 2.0)
        .windows(West, &[10.5], 2.2)
        .door(West, 13.0)
        .windows(East, &[12.0], 1.6)
        .door(South, 20.0)
        .windows(South, &[12.0], 1.4)
        .partition_z(16.0, 9.4, 22.6, &[Opening::door(19.0)])
        .room("the corner shop", 9.4, 8.4, 22.6, 15.85, Some(FloorSkin::Tiles))
        .room("the stockroom", 9.4, 16.15, 22.6, 19.6, Some(FloorSkin::Concrete))
        .lamp(18.0, 11.5)
        .low(10.0, 14.4, 12.5, 15.4, Desk)
        .low(15.6, 10.0, 21.4, 10.6, Shelf)
        .low(15.6, 13.2, 21.4, 13.8, Shelf)
        .tall(14.0, 18.9, 17.5, 19.6, 2.2, Shelf)
        .low(21.0, 16.6, 22.4, 18.0, Crate)
        .emit(&mut b);

    // The red house: hall and bedroom.
    Building::new("the red house", House, 9.0, 24.0, 21.0, 34.0, STOREY, Brick)
        .door(North, 12.0)
        .windows(North, &[18.0], 1.6)
        .door(East, 31.0)
        .windows(East, &[27.0], 1.4)
        .windows(South, &[12.0, 18.0], 1.4)
        .windows(West, &[27.0, 31.0], 1.6)
        .partition_x(15.0, 24.4, 33.6, &[Opening::door(29.0)])
        .room("the red house hall", 9.4, 24.4, 14.85, 33.6, Some(FloorSkin::Wood))
        .room("the red house bedroom", 15.15, 24.4, 20.6, 33.6, Some(FloorSkin::Wood))
        .low(16.0, 32.0, 18.4, 33.6, Desk)
        .tall(19.8, 24.4, 20.6, 26.0, 2.0, Cabinet)
        .low(10.5, 30.0, 12.5, 31.5, Desk)
        .emit(&mut b);

    // The gas station: a canopy on four pillars over two pump islands.
    for (x, z) in [(28.5, 12.5), (39.5, 12.5), (28.5, 21.5), (39.5, 21.5)] {
        b.tall(x - 0.25, z - 0.25, x + 0.25, z + 0.25, 4.4, Concrete);
    }
    b.decor.roofs.push(crate::arena::Roof { min: V2::new(27.5, 11.5), max: V2::new(40.5, 22.5), bottom: 4.4, thickness: 0.5 });
    b.low(31.5, 15.6, 36.5, 16.4, Cabinet);
    b.low(33.0, 18.2, 37.4, 20.1, Car);
    for (x, z) in [(32.0, 14.0), (36.0, 14.0)] {
        b.lamp_styled(x, z, 4.38, LampStyle::Panel);
    }
    // The parking lot and the south yard.
    b.low(41.0, 24.0, 42.9, 28.4, Car);
    b.low(41.0, 7.5, 42.9, 11.9, Car);
    b.low(26.0, 27.0, 27.9, 31.4, Car);
    b.low(24.0, 38.0, 26.0, 39.6, Dumpster);
    b.tall(30.0, 40.0, 36.0, 42.4, 2.6, Container);
    b.low(14.0, 38.0, 18.4, 39.9, Car);
    b.low(38.0, 36.0, 39.2, 37.2, Crate);

    // ------------------------------------------------------------------ south-west block
    // The workshop: a roller door onto the lane, a car on the floor, a parts store.
    Building::new("the workshop", Workshop, -44.0, 9.0, -30.0, 23.0, 4.5, Siding)
        .opening(East, Opening::wide_door(16.0, 4.0, 3.4))
        .windows(East, &[11.0, 20.5], 1.6)
        .door(North, -36.0)
        .windows(North, &[-41.0, -33.0], 1.6)
        .windows(South, &[-35.0], 1.6)
        .partition_x(-39.0, 9.4, 22.6, &[Opening::door(19.0)])
        .room("the parts store", -43.6, 9.4, -39.15, 22.6, Some(FloorSkin::Concrete))
        .room("the workshop", -38.85, 9.4, -30.4, 22.6, Some(FloorSkin::Concrete))
        .low(-35.0, 12.0, -33.1, 16.4, Car)
        .low(-37.5, 21.8, -34.5, 22.6, Shelf)
        .tall(-31.2, 9.4, -30.4, 10.6, 2.0, Cabinet)
        .tall(-43.6, 10.0, -43.0, 15.0, 2.4, Shelf)
        .low(-42.5, 17.0, -41.3, 18.2, Crate)
        .emit(&mut b);

    // The yellow house: front room on the cross street, kitchen onto the courtyard.
    Building::new("the yellow house", House, -22.0, 9.0, -10.0, 19.0, STOREY, Plaster)
        .door(North, -13.0)
        .windows(North, &[-19.0], 1.8)
        .windows(East, &[11.5, 16.5], 1.4)
        .windows(West, &[11.5], 1.4)
        .door(West, 17.0)
        .door(South, -13.0)
        .windows(South, &[-19.0], 1.6)
        .partition_z(14.0, -21.6, -10.4, &[Opening::door(-16.0)])
        .room("the yellow house front room", -21.6, 9.4, -10.4, 13.85, Some(FloorSkin::Wood))
        .room("the yellow house kitchen", -21.6, 14.15, -10.4, 18.6, Some(FloorSkin::Tiles))
        .low(-20.5, 11.8, -18.0, 12.6, Desk)
        .low(-21.4, 14.15, -18.6, 14.75, Desk)
        .low(-14.4, 15.6, -12.8, 16.8, Desk)
        .emit(&mut b);

    // The green house: two rooms backing onto the south wall.
    Building::new("the green house", House, -26.0, 34.0, -12.0, 44.0, STOREY, Plaster)
        .door(North, -15.0)
        .windows(North, &[-23.0, -17.0], 1.4)
        .windows(East, &[38.0], 1.4)
        .door(East, 41.0)
        .windows(West, &[39.0], 1.4)
        .partition_x(-19.0, 34.4, 43.6, &[Opening::door(39.0)])
        .room("the green house bedroom", -25.6, 34.4, -19.15, 43.6, Some(FloorSkin::Wood))
        .room("the green house living room", -18.85, 34.4, -12.4, 43.6, Some(FloorSkin::Wood))
        .low(-25.4, 41.6, -22.8, 43.4, Desk)
        .low(-15.5, 36.5, -13.5, 37.7, Desk)
        .emit(&mut b);

    // The courtyard: low walls, a fountain, a bench, planters.
    b.low(-27.2, 24.0, -26.8, 29.0, Brick);
    b.low(-24.0, 31.3, -18.0, 31.7, Brick);
    b.low(-19.0, 26.0, -17.0, 28.0, Planter);
    b.low(-24.0, 23.6, -21.6, 24.2, Desk);
    b.low(-12.0, 25.0, -11.0, 26.0, Planter);
    // The west yard behind the workshop.
    b.tall(-42.0, 27.0, -39.6, 33.0, 2.6, Container);
    b.low(-36.0, 36.0, -34.1, 40.4, Car);
    b.low(-31.0, 28.0, -29.4, 29.8, Dumpster);

    // ------------------------------------------------------------------ the streets
    // Main Street: a checkpoint, parked cars, a burnt-out car on the crossroads, and the bus
    // that shields the player's spawn.
    b.low(-4.0, -30.3, -1.0, -29.7, Sandbag);
    b.low(1.5, -26.3, 4.5, -25.7, Sandbag);
    b.low(-3.5, -20.0, -1.6, -15.6, Car);
    b.low(2.0, -12.0, 3.9, -7.6, Car);
    b.low(-2.2, -0.95, 2.2, 0.95, Car);
    b.low(-3.9, 10.0, -2.0, 14.4, Car);
    b.low(1.0, 20.0, 4.0, 20.6, Sandbag);
    b.tall(-3.5, 32.0, 3.5, 34.6, 3.0, Container);
    // The cross street.
    b.low(-30.0, -3.5, -25.6, -1.6, Car);
    b.low(-16.0, 1.5, -13.0, 2.1, Sandbag);
    b.low(22.0, 1.6, 26.4, 3.5, Car);
    b.tall(34.0, -3.2, 40.0, -0.8, 2.8, Container);
    b.low(14.0, -2.1, 17.0, -1.5, Sandbag);

    // Street lamps on the facades, facing the street.
    for (x, z, fx, fz) in [
        (-9.0, -36.0, 1, 0),
        (10.0, -38.0, -1, 0),
        (9.0, 16.5, -1, 0),
        (-10.0, 14.0, 1, 0),
        (-35.0, -28.0, 0, 1),
        (16.0, 8.0, 0, -1),
        (-16.0, 9.0, 0, -1),
        (31.0, -14.0, 0, 1),
        (9.0, 29.0, -1, 0),
    ] {
        b.lamp_styled(x, z, 3.6, LampStyle::Wall { facing_x: fx, facing_z: fz });
    }

    // Ground: asphalt roads, paving-slab sidewalks; the rest is the base pavement.
    b.floor(-5.0, -44.0, 5.0, 44.0, FloorSkin::Asphalt);
    b.floor(-44.0, -4.0, -5.0, 4.0, FloorSkin::Asphalt);
    b.floor(5.0, -4.0, 44.0, 4.0, FloorSkin::Asphalt);
    for (x0, z0, x1, z1) in [
        (-7.0, -44.0, -5.0, -4.0),
        (-7.0, 4.0, -5.0, 44.0),
        (5.0, -44.0, 7.0, -4.0),
        (5.0, 4.0, 7.0, 44.0),
        (-44.0, -6.0, -7.0, -4.0),
        (-44.0, 4.0, -7.0, 6.0),
        (7.0, -6.0, 44.0, -4.0),
        (7.0, 4.0, 44.0, 6.0),
        // The courtyards are paved too.
        (10.0, -29.6, 24.0, -8.5),
        (-26.6, 21.0, -10.0, 33.5),
    ] {
        b.floor(x0, z0, x1, z1, FloorSkin::Sidewalk);
    }

    let zones = vec![
        crate::arena::zone("the crossroads", -7.0, -6.0, 7.0, 6.0),
        crate::arena::zone("north Main Street", -9.0, -44.0, 10.0, -6.0),
        crate::arena::zone("south Main Street", -12.0, 6.0, 9.0, 44.0),
        crate::arena::zone("the west road", -44.0, -6.0, -7.0, 6.0),
        crate::arena::zone("the east road", 7.0, -6.0, 44.0, 6.0),
        crate::arena::zone("the north alley", -24.0, -44.0, -20.0, -28.0),
        crate::arena::zone("the back lot", -24.0, -44.0, -9.0, -40.0),
        crate::arena::zone("the loading yard", -44.0, -28.0, -24.0, -6.0),
        crate::arena::zone("the garden", -24.0, -30.0, -7.0, -6.0),
        crate::arena::zone("the north lane", 10.0, -44.0, 44.0, -40.0),
        crate::arena::zone("the office courtyard", 10.0, -30.0, 34.0, -6.0),
        crate::arena::zone("the east lot", 34.0, -40.0, 44.0, -6.0),
        crate::arena::zone("the gas station", 27.0, 11.0, 41.0, 23.0),
        crate::arena::zone("the back alley", 9.0, 20.0, 23.0, 24.0),
        crate::arena::zone("the parking lot", 21.0, 6.0, 44.0, 34.0),
        crate::arena::zone("the south yard", 9.0, 34.0, 44.0, 44.0),
        crate::arena::zone("the courtyard", -30.0, 19.0, -9.0, 34.0),
        crate::arena::zone("the workshop lane", -30.0, 6.0, -22.0, 19.0),
        crate::arena::zone("the west yard", -44.0, 23.0, -26.0, 44.0),
    ];
    b.finish(
        MapId::Township,
        zones,
        V2::new(0.0, 40.0),
        vec![
            V2::new(0.0, -41.0),
            V2::new(-4.0, -37.0),
            V2::new(4.0, -37.0),
            V2::new(-14.0, -42.0),
            V2::new(12.0, -42.0),
            V2::new(41.0, -40.0),
            V2::new(-34.0, -26.0),
        ],
    )
}
