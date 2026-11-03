//! "Sirocco": a bomb-defusal layout in a whitewashed hill town at midday.
//!
//! ```text
//!            north (defenders)
//!   +-------------+---+--------+---+-------------+
//!   |   A site    |   | def.   |   |   B site    |
//!   |  (plaza,    |con| spawn  |con|  (market,   |
//!   |  fountain)  |   |        |   |   stalls)   |
//!   +--+   +------+---+ mid    +---+--+    +-----+
//!   |  |   | short leg|  doors | B    |    | tun-|
//!   |A |   +----------+--------+ short|    | nel |
//!   |  |   house |   alley  mid |  alley    | exit|
//!   |long+-doors-+           |   +------+  |(roof|
//!   |  |  houses  |   mid    |  houses    | ed) |
//!   +--+----------+----------+------------+-----+
//!   |             attacker spawn               |
//!   +------------------------------------------+
//!            south (attackers)
//! ```
//!
//! Routes: **A long** (west street, a tall arched gate halfway), **A short** (an alley off
//! mid that turns north into A, or through **the A house** from long), **mid** (the sniper
//! lane from the defender spawn through the mid doors down to the spawn wall), **B short**
//! (an L-shaped alley off mid), **B tunnels** (two roofed corridors with a cross-link on
//! the east). Defenders rotate between sites through **A connector** and **B connector**
//! either side of their spawn. The player (attacker) spawns south; the squad holds north.

use crate::arena::{Arena, BuildingUse, Builder, FloorSkin, Header, LampStyle, MapId, Paint, Skin, zone};
use crate::building::{Building, Opening, Side, STOREY};
use crate::math::V2;

/// Half extent: outer walls on ±40, playable floor ±39.
pub const SIROCCO_HALF: f32 = 40.0;

/// An arch: a header between two wall ends, its underside at `bottom`.
fn arch(b: &mut Builder, x0: f32, z0: f32, x1: f32, z1: f32, bottom: f32, top: f32) {
    b.decor.headers.push(Header { min: V2::new(x0, z0), max: V2::new(x1, z1), bottom, top });
}

pub fn sirocco() -> Arena {
    use Skin::*;
    let mut b = Builder::sized(SIROCCO_HALF, 7.0, Stucco);

    // ------------------------------------------------------------------ attacker spawn (south)
    // A paved square under the south houses; a garden wall shields it from the mid lane.
    b.tall(-39.0, 35.0, -26.0, 39.0, 8.5, Stucco);
    b.tall(-9.0, 37.0, 9.0, 39.0, 6.0, Stucco);
    b.tall(25.0, 34.0, 39.0, 39.0, 7.5, Stucco);
    b.tall(-4.0, 26.5, 4.0, 27.7, 3.4, Stucco); // spawn wall: mid's sniper lane ends here
    b.low(-17.0, 31.0, -14.0, 34.0, Planter); // fountain
    b.low(13.0, 30.0, 14.2, 31.2, Crate);
    b.low(14.2, 30.0, 15.4, 31.2, Crate);
    b.low(21.0, 28.5, 22.9, 32.9, Car);
    b.low(-24.0, 29.0, -22.8, 30.2, Barrel);
    b.low(-30.0, 31.0, -27.0, 31.6, Stucco);

    // ------------------------------------------------------------------ west: A long, the A house, A short
    // The houses between A long and mid: a tall row (south) and a lower one (north).
    b.tall(-27.0, 17.0, -15.0, 26.0, 7.5, Stucco);
    b.tall(-27.0, 9.0, -15.0, 13.0, 6.0, Stucco);
    b.tall(-15.0, 9.0, -5.0, 13.0, 7.0, Stucco);
    b.tall(-15.0, 17.0, -5.0, 26.0, 6.5, Stucco);
    // The mid alleys: side streets off top mid, west to outside long and east into the lower tunnel.
    arch(&mut b, -5.6, 13.0, -5.0, 17.0, 3.0, 5.0);
    arch(&mut b, 5.0, 13.0, 5.6, 17.0, 3.0, 5.0);
    b.low(-20.0, 13.4, -18.8, 14.6, Crate);
    b.low(18.0, 15.6, 19.2, 16.8, Barrel);
    b.tall(-17.0, -2.0, -5.0, 9.0, 5.5, Stucco);
    // The A house: a two-room house between long and the alley (a third way into A short).
    Building::new("the A house", BuildingUse::House, -27.0, -2.0, -17.0, 9.0, STOREY, Plaster)
        .door(Side::West, 3.0)
        .windows(Side::West, &[6.5], 1.4)
        .door(Side::North, -21.0)
        .windows(Side::North, &[-18.6], 1.2)
        .partition_z(5.0, -26.6, -17.4, &[Opening::door(-20.0)])
        .room("the A house", -26.6, -1.6, -17.4, 4.85, Some(FloorSkin::Wood))
        .room("the A house kitchen", -26.6, 5.15, -17.4, 8.6, Some(FloorSkin::Tiles))
        .low(-26.6, 7.6, -23.0, 8.6, Desk)
        .low(-25.6, -0.4, -23.6, 0.8, Desk)
        .tall(-18.2, 1.8, -17.4, 3.6, 2.0, Cabinet)
        .emit(&mut b);
    // A long gate: a thick wall across the street with a tall arch.
    b.tall(-39.0, 0.0, -35.1, 1.4, 5.5, Stucco);
    b.tall(-30.9, 0.0, -27.0, 1.4, 5.5, Stucco);
    arch(&mut b, -35.1, 0.0, -30.9, 1.4, 3.4, 5.5);
    // Outside long: a parked car, a cart, crates by the gate.
    b.low(-37.0, 15.0, -35.1, 19.4, Car);
    b.low(-30.0, 21.0, -28.8, 22.2, Crate);
    b.low(-30.0, 6.0, -28.0, 6.6, Stucco);
    b.low(-38.6, 2.4, -37.4, 3.6, Barrel);
    // Inside long: a low wall and boxes on the corner into A.
    b.tall(-31.0, -7.0, -29.0, -5.0, 2.4, Crate);
    b.low(-29.0, -6.2, -27.8, -5.0, Crate);
    b.low(-38.6, -9.0, -36.0, -8.4, Stucco);
    // Between long and the short's north leg; between the leg and mid.
    b.tall(-27.0, -12.0, -23.0, -2.0, 6.0, Stucco);
    b.tall(-19.0, -15.0, -5.0, -6.0, 5.0, Stucco);
    // Alley arch where A short leaves mid, and a vine pergola over the alley.
    arch(&mut b, -5.6, -6.0, -5.0, -2.0, 3.0, 5.0);
    b.decor.roofs.push(crate::arena::Roof { min: V2::new(-16.0, -6.0), max: V2::new(-9.0, -2.0), bottom: 3.0, thickness: 0.12 });
    b.low(-14.0, -3.4, -12.8, -2.2, Crate);
    b.low(-22.8, -11.0, -21.6, -9.8, Barrel);

    // ------------------------------------------------------------------ mid
    b.tall(2.4, 20.0, 4.4, 22.0, 2.4, Crate); // mid boxes
    b.low(-4.6, 6.0, -3.4, 7.2, Crate);
    b.low(-1.2, -9.0, 1.2, -8.4, Stucco);
    b.low(3.6, 18.0, 4.8, 19.2, Barrel);
    // Mid doors: two thick pillars and an arch, the defenders' end of the lane.
    b.tall(-5.0, -15.0, -1.6, -13.5, 4.8, Stucco);
    b.tall(1.6, -15.0, 5.0, -13.5, 4.8, Stucco);
    arch(&mut b, -1.6, -15.0, 1.6, -13.5, 2.8, 4.8);

    // ------------------------------------------------------------------ east: B short, B tunnels
    b.tall(5.0, 17.0, 15.0, 26.0, 6.5, Stucco);
    b.tall(5.0, 8.0, 15.0, 13.0, 7.0, Stucco);
    b.tall(15.0, 17.0, 26.0, 26.0, 8.0, Stucco);
    b.tall(15.0, 8.0, 26.0, 13.0, 6.0, Stucco);
    b.tall(5.0, -15.0, 14.0, 4.0, 6.0, Stucco);
    b.tall(18.0, -13.0, 26.0, 8.0, 6.5, Stucco);
    // B short: the alley leaves mid under an arch, turns north at the corner.
    arch(&mut b, 5.0, 4.0, 5.6, 8.0, 3.0, 5.0);
    b.low(16.2, 5.0, 17.4, 6.2, Crate);
    b.low(14.6, -6.0, 15.8, -4.8, Barrel);
    b.decor.roofs.push(crate::arena::Roof { min: V2::new(14.0, -4.0), max: V2::new(18.0, 2.0), bottom: 3.0, thickness: 0.12 });
    // B tunnels: two roofed corridors split by a wall with a cross-link; the lower one
    // joins the upper before the exit.
    b.roof(26.0, -4.0, 39.0, 20.0, 3.4);
    b.tall(31.0, -1.0, 32.0, 6.0, 3.45, Stucco);
    b.tall(31.0, 10.0, 32.0, 20.0, 3.45, Stucco);
    b.tall(26.0, -5.0, 33.0, -4.0, 3.45, Stucco);
    b.low(37.4, 12.0, 38.6, 13.2, Barrel);
    b.low(37.4, 13.4, 38.6, 14.6, Barrel);
    b.low(26.0, 2.0, 27.2, 3.2, Crate);
    for (x, z) in [(28.5, 15.0), (35.5, 15.0), (35.5, 3.0), (28.5, 1.0), (35.5, -2.5)] {
        b.lamp_styled(x, z, 3.3, LampStyle::Bulkhead);
    }
    // Outside the tunnels, the B side of spawn.
    b.low(27.0, 22.0, 29.4, 22.6, Stucco);

    // ------------------------------------------------------------------ defenders (north)
    // Thick house walls split the north into A site | defender spawn | B site, with a
    // connector arch through each.
    b.tall(-14.0, -39.0, -10.0, -34.0, 6.5, Stucco);
    b.tall(-14.0, -29.6, -10.0, -15.0, 6.0, Stucco);
    arch(&mut b, -14.0, -34.0, -10.0, -29.6, 3.0, 6.0);
    b.tall(10.0, -39.0, 14.0, -34.0, 6.5, Stucco);
    b.tall(10.0, -29.6, 14.0, -15.0, 6.0, Stucco);
    arch(&mut b, 10.0, -34.0, 14.0, -29.6, 3.0, 6.0);
    // The church front behind the spawn, a well, benches.
    b.tall(-6.0, -39.0, 6.0, -37.0, 9.0, Stucco);
    b.low(-1.5, -27.5, 1.5, -24.5, Planter); // the well
    b.tall(-8.0, -21.0, -6.0, -19.0, 2.4, Crate);
    b.low(5.0, -21.0, 7.4, -20.4, Stucco);
    b.low(-9.4, -33.0, -8.2, -31.8, Barrel);

    // ------------------------------------------------------------------ A site
    b.tall(-39.0, -39.0, -31.0, -35.0, 7.0, Stucco); // the back house
    b.tall(-30.0, -27.0, -27.0, -24.0, 2.4, Crate); // default boxes
    b.low(-27.0, -25.2, -25.8, -24.0, Crate);
    b.low(-22.0, -23.0, -19.0, -20.0, Planter); // fountain
    b.tall(-35.0, -20.0, -34.0, -17.0, 4.0, Stucco); // a broken arch pier
    b.low(-38.6, -30.0, -36.0, -29.4, Stucco);
    b.low(-24.0, -36.0, -22.1, -31.6, Car);
    b.low(-17.0, -30.0, -15.8, -28.8, Crate);
    b.low(-17.0, -28.8, -15.8, -27.6, Crate);
    b.tall(-27.0, -33.0, -25.0, -31.0, 2.4, Crate);

    // ------------------------------------------------------------------ B site
    b.tall(31.0, -39.0, 39.0, -33.0, 7.0, Stucco); // the back house
    b.tall(24.0, -27.0, 27.0, -24.0, 2.6, Crate); // default boxes
    b.low(27.0, -25.2, 28.2, -24.0, Crate);
    b.tall(16.0, -37.0, 22.0, -34.6, 2.6, Container); // a storage container
    b.tall(20.0, -20.0, 21.0, -19.0, 4.2, Stucco); // market canopy piers
    b.tall(29.0, -20.0, 30.0, -19.0, 4.2, Stucco);
    b.decor.roofs.push(crate::arena::Roof { min: V2::new(19.5, -20.5), max: V2::new(30.5, -18.5), bottom: 4.2, thickness: 0.15 });
    b.low(31.0, -21.0, 34.0, -20.4, Stucco);
    b.low(36.6, -27.0, 37.8, -25.8, Barrel);
    b.low(37.8, -27.0, 39.0, -25.8, Barrel);
    b.low(15.0, -24.0, 17.4, -23.4, Stucco);
    b.low(32.0, -31.0, 33.9, -26.6, Car);

    // ------------------------------------------------------------------ signs
    // Site letters, big, where attackers arrive and where defenders hold.
    b.sign_wall("A", -35.0, -35.0, 3.6, 2.4, (0, 1), Paint::White, Some(Paint::Red));
    b.sign_wall("A", -14.0, -22.0, 3.0, 1.8, (-1, 0), Paint::White, Some(Paint::Red));
    b.sign_wall("B", 35.0, -33.0, 3.6, 2.4, (0, 1), Paint::White, Some(Paint::Blue));
    b.sign_wall("B", 14.0, -22.0, 3.0, 1.8, (1, 0), Paint::White, Some(Paint::Blue));
    b.sign_floor("A", -26.0, -19.0, 3.0, (0, -1), Paint::Red);
    b.sign_floor("B", 25.0, -30.0, 3.0, (0, -1), Paint::Blue);
    // Directions from spawn.
    b.sign_wall("< A", -16.0, 26.0, 2.4, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("B >", 16.0, 26.0, 2.4, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("MID", 0.0, 27.7, 2.2, 0.6, (0, 1), Paint::Black, Some(Paint::White));
    // A long, mid, B short, tunnels.
    b.sign_wall("< A", -27.0, 12.0, 2.4, 0.9, (-1, 0), Paint::Black, Some(Paint::Yellow));
    b.sign_floor("A ^", -33.0, 4.0, 1.2, (0, -1), Paint::Yellow);
    b.sign_wall("A >", -5.0, 2.5, 2.4, 0.8, (1, 0), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("B >", 5.0, 1.0, 2.4, 0.8, (-1, 0), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("A >", -23.0, -4.0, 2.4, 0.8, (1, 0), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("< B", 18.0, 6.0, 2.4, 0.8, (-1, 0), Paint::Black, Some(Paint::Yellow));
    b.sign_floor("B ^", 35.5, 22.0, 1.2, (0, -1), Paint::Yellow);
    // Defender rotations.
    b.sign_wall("A >", -10.0, -24.0, 2.6, 0.8, (1, 0), Paint::White, Some(Paint::Red));
    b.sign_wall("< B", 10.0, -24.0, 2.6, 0.8, (-1, 0), Paint::White, Some(Paint::Blue));
    b.sign_wall("MID", 0.0, -37.0, 2.6, 0.6, (0, 1), Paint::Black, Some(Paint::White));

    // ------------------------------------------------------------------ ground
    // Base: sun-baked earth. Cobbled streets, a paved spawn square, tiled site plazas.
    b.floor(-39.0, 26.0, 39.0, 39.0, FloorSkin::Sidewalk);
    b.floor(-39.0, -12.0, -27.0, 26.0, FloorSkin::Pavement);
    b.floor(-5.0, -15.0, 5.0, 26.0, FloorSkin::Pavement);
    b.floor(-23.0, -6.0, -5.0, -2.0, FloorSkin::Pavement);
    b.floor(-27.0, 13.0, -5.0, 17.0, FloorSkin::Pavement);
    b.floor(5.0, 13.0, 26.0, 17.0, FloorSkin::Pavement);
    b.floor(-23.0, -12.0, -19.0, -6.0, FloorSkin::Pavement);
    b.floor(5.0, 4.0, 18.0, 8.0, FloorSkin::Pavement);
    b.floor(14.0, -13.0, 18.0, 4.0, FloorSkin::Pavement);
    b.floor(26.0, -4.0, 39.0, 20.0, FloorSkin::Concrete);
    b.floor(-39.0, -39.0, -14.0, -12.0, FloorSkin::Tiles);
    b.floor(14.0, -39.0, 39.0, -13.0, FloorSkin::Tiles);
    b.floor(-10.0, -39.0, 10.0, -15.0, FloorSkin::Sidewalk);
    // Spawn zones: painted borders (attackers orange, defenders blue).
    b.outline(-21.0, 28.5, 21.0, 38.5, 0.25, Paint::Orange);
    b.outline(-9.6, -36.6, 9.6, -28.0, 0.25, Paint::Blue);

    let zones = vec![
        zone("attacker spawn", -39.0, 26.0, 39.0, 39.0),
        zone("outside long", -39.0, 1.4, -27.0, 26.0),
        zone("A long", -39.0, -12.0, -27.0, 1.4),
        zone("A short", -27.0, -12.0, -5.0, -2.0),
        zone("A site", -39.0, -39.0, -14.0, -12.0),
        zone("A connector", -14.0, -34.0, -10.0, -29.6),
        zone("defender spawn", -10.0, -39.0, 10.0, -23.0),
        zone("mid doors", -10.0, -23.0, 10.0, -13.5),
        zone("mid", -5.0, -13.5, 5.0, 26.0),
        zone("west mid alley", -27.0, 13.0, -5.0, 17.0),
        zone("east mid alley", 5.0, 13.0, 26.0, 17.0),
        zone("B short", 5.0, -13.0, 18.0, 8.0),
        zone("B connector", 10.0, -34.0, 14.0, -29.6),
        zone("B tunnels", 26.0, -4.0, 39.0, 26.0),
        zone("B entrance", 26.0, -13.0, 39.0, -4.0),
        zone("B site", 14.0, -39.0, 39.0, -13.0),
    ];
    b.finish(
        MapId::Sirocco,
        zones,
        V2::new(0.0, 33.0),
        vec![
            V2::new(-5.0, -34.0),
            V2::new(0.0, -34.5),
            V2::new(5.0, -34.0),
            V2::new(-6.0, -29.0),
            V2::new(6.0, -29.0),
            V2::new(0.0, -21.0),
            V2::new(-30.0, -33.0),
            V2::new(28.5, -31.0),
        ],
    )
}
