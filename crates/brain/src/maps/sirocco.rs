//! "Sirocco": a bomb-defusal layout in a whitewashed hill town at midday.
//!
//! ```text
//!                        north (defenders)
//!   +--------------+--+---------------------+------------------------+
//!   |  A site      |  |   defender spawn    |   the convent (solid)  |
//!   |  (fountain   |  |                     |                        |
//!   |   plaza)    west+---- the bell arch --+--+---------------------+
//!   |             arch   square  (( arch )) east   B site            |
//!   +--+-----+    |  |                      arch   (market, canopy)  |
//!   |bakery  |    |  +--+---D---------+-----+--+------+     +--------+
//!   |  D   high st.  |v |  the church  |priest's|house |dog- | house  |
//!   |baker's|      |  |i D  (pews,    | house  |      | leg | (solid)|
//!   +--D---+  gate |  |n |   altar)    |(solid) +------+  || |        |
//!   | lower +------+--+e +------D------+---+ dog-leg alley  ||        |
//!   | street| the cellars  procession road |-----------+    ||        |
//!   |       +------------+   (square)      |  house    |    ||        |
//!   +-------+------------+---[spawn wall]--+-----------+----++--------+
//!   |                     attacker spawn                              |
//!   +-----------------------------------------------------------------+
//!                        south (attackers)
//! ```
//!
//! Two lanes and a church. West: **the lower street** along the town wall, a jog east into
//! **the high street**, which opens into A from the south (or through **the baker's house**).
//! East: **the dog-leg**, an L-shaped alley off the square that weaves north into the side of B.
//! In the middle the attackers' square (**the procession road**) leads into **the church**
//! (side door onto **the vine alley**), whose north door opens on **the bell arch** square in
//! front of the defender spawn; from there **the west arch** leads to A and **the east arch** to
//! B. **The cellars** are a roofed cross-link from the lower street to the square. The player
//! (attacker) spawns south; the squad holds north.

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
    // A paved square under the south houses; a garden wall shields it from the church door.
    b.tall(-39.0, 35.0, -26.0, 39.0, 8.5, Stucco);
    b.tall(-9.0, 37.0, 9.0, 39.0, 6.0, Stucco);
    b.tall(25.0, 34.0, 39.0, 39.0, 7.5, Stucco);
    b.tall(-4.0, 26.5, 4.0, 27.7, 3.4, Stucco); // spawn wall
    b.low(-17.0, 31.0, -14.0, 34.0, Planter); // fountain
    b.low(13.0, 30.0, 14.2, 31.2, Crate);
    b.low(14.2, 30.0, 15.4, 31.2, Crate);
    b.low(21.0, 28.5, 22.9, 32.9, Car);
    b.low(-24.0, 29.0, -22.8, 30.2, Barrel);
    b.low(-30.0, 31.0, -27.0, 31.6, Stucco);

    // ------------------------------------------------------------------ west: lower street, high street, baker's house
    // The baker's house: between the lower street (south door), the high street (east door)
    // and A (north door).
    Building::new("the baker's house", BuildingUse::House, -39.0, -16.0, -30.0, -2.0, STOREY, Plaster)
        .door(Side::South, -35.0)
        .door(Side::East, -6.0)
        .windows(Side::East, &[-13.0], 1.4)
        .door(Side::North, -34.0)
        .partition_z(-9.0, -38.6, -30.4, &[Opening::door(-33.0)])
        .room("the bakery", -38.6, -15.6, -30.4, -9.15, Some(FloorSkin::Tiles))
        .room("the baker's house", -38.6, -8.85, -30.4, -2.4, Some(FloorSkin::Wood))
        .low(-38.6, -15.6, -35.6, -14.6, Desk)
        .low(-37.6, -6.0, -35.6, -4.8, Desk)
        .emit(&mut b);
    // The lower street: along the town wall from the spawn to the jog into the high street.
    b.tall(-33.0, 6.0, -31.0, 8.0, 2.4, Crate);
    b.low(-37.0, 15.0, -35.1, 19.4, Car);
    b.low(-38.6, 2.4, -37.4, 3.6, Barrel);
    b.low(-34.0, 22.0, -32.8, 23.2, Crate);
    // The high street: wide, opens into A from the south.
    b.tall(-24.0, -6.0, -22.0, -4.0, 2.4, Crate);
    b.low(-28.0, -12.0, -26.8, -10.8, Crate);
    b.low(-20.0, 0.0, -18.0, 2.0, Planter);
    b.low(-19.0, -14.0, -17.8, -12.8, Barrel);
    // House walls: the high street | the vine alley, and A | the defenders (the west arch).
    b.tall(-16.0, -16.0, -14.0, 4.0, 6.5, Stucco);
    b.tall(-16.0, -39.0, -14.0, -26.0, 6.5, Stucco);
    b.tall(-16.0, -22.0, -14.0, -16.0, 6.0, Stucco);
    arch(&mut b, -16.0, -26.0, -14.0, -22.0, 3.0, 6.0);
    // The houses either side of the cellars.
    b.tall(-30.0, 4.0, -14.0, 12.0, 7.0, Stucco);
    b.tall(-30.0, 17.0, -14.0, 27.0, 7.5, Stucco);
    // The cellars: a roofed cross-link, split down the middle for a stretch.
    b.roof(-30.0, 12.0, -14.0, 17.0, 3.4);
    b.tall(-24.0, 14.2, -20.0, 14.8, 3.45, Stucco);
    b.lamp_styled(-27.0, 14.5, 3.3, LampStyle::Bulkhead);
    b.lamp_styled(-17.0, 14.5, 3.3, LampStyle::Bulkhead);
    // The vine alley: between the church and the houses, under a pergola.
    b.decor.roofs.push(crate::arena::Roof { min: V2::new(-14.0, -10.0), max: V2::new(-10.0, -2.0), bottom: 3.0, thickness: 0.12 });
    b.low(-13.4, 6.0, -12.2, 7.2, Crate);

    // ------------------------------------------------------------------ centre: the church
    // Doors offset north/south (no line straight through) and an altar screen in the nave.
    // The south front stops at z 8 so the square in front of it is deep enough to fight in
    // (and to be flanked in) rather than a strip along one door.
    Building::new("the church", BuildingUse::House, -10.0, -12.0, 10.0, 8.0, 5.0, Stucco)
        .parapet(true)
        .opening(Side::North, Opening::wide_door(-4.0, 3.0, 3.0))
        .opening(Side::South, Opening::wide_door(5.0, 3.0, 3.0))
        .door(Side::West, -2.0)
        .room("the church", -9.6, -11.6, 9.6, 7.6, Some(FloorSkin::Tiles))
        .lamp(0.0, -7.0)
        .lamp(0.0, 3.5)
        .low(-2.0, -9.0, 2.0, -8.0, Desk) // altar
        .tall(-3.0, -1.0, 3.0, -0.4, 2.4, Stucco) // altar screen
        .tall(-6.5, -6.0, -5.5, -5.0, 5.0, Stucco)
        .tall(5.5, -6.0, 6.5, -5.0, 5.0, Stucco)
        .low(-8.0, 2.0, -3.0, 2.6, Desk) // pews
        .low(3.0, 2.0, 8.0, 2.6, Desk)
        .low(-8.0, 4.8, -3.0, 5.4, Desk)
        .low(3.0, 4.8, 8.0, 5.4, Desk)
        .emit(&mut b);
    b.tall(10.0, -12.0, 16.0, 8.0, 6.5, Stucco); // the priest's house (solid)
    // The procession road: the attackers' square in front of the church.
    b.tall(2.4, 20.0, 4.4, 22.0, 2.4, Crate);
    b.low(-6.0, 16.0, -4.8, 17.2, Crate);
    b.low(8.0, 18.0, 9.2, 19.2, Barrel);
    // Wine casks across the foot of the vine alley: no line from the bell arch into the spawn.
    b.tall(-14.0, 20.0, -10.0, 22.0, 2.4, Crate);

    // ------------------------------------------------------------------ east: the dog-leg
    b.tall(16.0, -4.0, 26.0, 14.0, 7.0, Stucco);
    b.tall(32.0, -4.0, 39.0, 27.0, 6.5, Stucco);
    b.tall(14.0, 20.0, 26.0, 27.0, 8.0, Stucco);
    // Two staggered stair houses: the lane weaves, no line from B into the spawn.
    b.tall(26.0, 10.0, 29.0, 12.0, 3.2, Stucco);
    b.tall(29.0, 2.0, 32.0, 4.0, 3.2, Stucco);
    b.low(18.0, 15.6, 19.2, 16.8, Crate);
    b.low(30.6, -2.0, 31.4, -1.2, Barrel);
    b.low(27.0, 22.0, 28.2, 23.2, Crate);

    // ------------------------------------------------------------------ defenders (north)
    b.tall(-6.0, -39.0, 6.0, -37.0, 9.0, Stucco); // the chapel front behind the spawn
    b.tall(14.0, -39.0, 39.0, -24.0, 7.0, Stucco); // the convent (solid)
    b.low(-12.4, -33.0, -11.2, -31.8, Barrel);
    // The bell arch square: a free-standing bell arch, a well, boxes.
    b.tall(-8.0, -20.0, -6.0, -18.0, 5.5, Stucco);
    b.tall(6.0, -20.0, 8.0, -18.0, 5.5, Stucco);
    arch(&mut b, -6.0, -20.0, 6.0, -18.0, 3.6, 5.5);
    b.low(-1.5, -27.5, 1.5, -24.5, Planter); // the well
    b.tall(-12.0, -16.0, -10.0, -14.0, 2.4, Crate);
    b.low(9.0, -16.0, 11.4, -15.4, Stucco);
    // The east arch: from the square into B.
    b.tall(14.0, -24.0, 16.0, -21.0, 6.0, Stucco);
    b.tall(14.0, -17.0, 16.0, -12.0, 6.0, Stucco);
    arch(&mut b, 14.0, -21.0, 16.0, -17.0, 3.0, 6.0);

    // ------------------------------------------------------------------ A site
    b.tall(-39.0, -39.0, -31.0, -35.0, 7.0, Stucco); // the back house
    b.tall(-34.0, -26.0, -31.0, -23.0, 2.4, Crate); // default boxes
    b.low(-31.0, -24.2, -29.8, -23.0, Crate);
    b.low(-26.0, -23.0, -23.0, -20.0, Planter); // fountain
    b.tall(-38.0, -20.0, -37.0, -18.0, 4.0, Stucco); // a broken arch pier
    b.low(-38.6, -30.0, -36.0, -29.4, Stucco);
    b.low(-22.0, -36.0, -20.1, -31.6, Car);
    b.low(-17.5, -30.0, -16.3, -28.8, Crate);
    b.low(-17.5, -28.8, -16.3, -27.6, Crate);
    b.tall(-27.0, -33.0, -25.0, -31.0, 2.4, Crate);

    // ------------------------------------------------------------------ B site
    b.tall(24.0, -18.0, 27.0, -15.0, 2.6, Crate); // default boxes
    b.low(27.0, -16.2, 28.2, -15.0, Crate);
    b.tall(33.0, -24.0, 39.0, -21.6, 2.6, Container);
    b.tall(20.0, -12.0, 21.0, -11.0, 4.2, Stucco); // market canopy piers
    b.tall(31.0, -12.0, 32.0, -11.0, 4.2, Stucco);
    b.decor.roofs.push(crate::arena::Roof { min: V2::new(19.5, -12.5), max: V2::new(32.5, -10.5), bottom: 4.2, thickness: 0.15 });
    b.low(18.0, -20.0, 20.4, -19.4, Stucco);
    b.low(37.4, -10.0, 38.6, -8.8, Barrel);
    b.low(37.4, -8.6, 38.6, -7.4, Barrel);
    b.low(34.0, -10.0, 35.9, -5.6, Car);
    b.low(21.0, -6.0, 24.0, -5.4, Stucco);

    // ------------------------------------------------------------------ signs
    // (The sign glyphs are A, B, D, I, M, T and arrows only: no callout words.)
    b.sign_wall("A", -35.0, -35.0, 3.6, 2.4, (0, 1), Paint::White, Some(Paint::Red));
    b.sign_wall("A", -14.0, -19.0, 3.0, 1.8, (1, 0), Paint::White, Some(Paint::Red));
    b.sign_wall("B", 30.0, -24.0, 3.6, 2.4, (0, 1), Paint::White, Some(Paint::Blue));
    b.sign_wall("B", 14.0, -14.5, 3.0, 1.8, (-1, 0), Paint::White, Some(Paint::Blue));
    b.sign_floor("A", -26.0, -30.0, 3.0, (0, -1), Paint::Red);
    b.sign_floor("B", 27.0, -21.0, 3.0, (0, -1), Paint::Blue);
    // Directions from spawn: A up the west lane, B up the dog-leg, both through the church.
    b.sign_wall("< A", -22.0, 27.0, 2.6, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("B >", 20.0, 27.0, 2.6, 1.0, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_wall("A B ^", 0.0, 27.7, 2.2, 0.6, (0, 1), Paint::Black, Some(Paint::White));
    b.sign_floor("A ^", -34.5, 10.0, 1.2, (0, -1), Paint::Yellow);
    b.sign_floor("A ^", -23.0, -10.0, 1.2, (0, -1), Paint::Yellow);
    b.sign_wall("B >", 21.0, 14.0, 2.4, 0.9, (0, 1), Paint::Black, Some(Paint::Yellow));
    b.sign_floor("B ^", 29.0, 24.0, 1.2, (0, -1), Paint::Yellow);
    // Defender rotations from the bell arch square.
    b.sign_floor("< A", -9.0, -23.0, 1.0, (0, -1), Paint::Yellow);
    b.sign_floor("B >", 9.0, -23.0, 1.0, (0, -1), Paint::Yellow);

    // ------------------------------------------------------------------ ground
    // Base: sun-baked earth. Cobbled streets, paved squares, tiled site plazas.
    b.floor(-39.0, 27.0, 39.0, 39.0, FloorSkin::Sidewalk);
    b.floor(-39.0, -2.0, -30.0, 27.0, FloorSkin::Pavement);
    b.floor(-30.0, -16.0, -16.0, 4.0, FloorSkin::Pavement);
    b.floor(-30.0, 12.0, -14.0, 17.0, FloorSkin::Concrete);
    b.floor(-14.0, -12.0, -10.0, 8.0, FloorSkin::Pavement);
    b.floor(-14.0, 8.0, 16.0, 27.0, FloorSkin::Sidewalk);
    b.floor(16.0, 14.0, 26.0, 20.0, FloorSkin::Pavement);
    b.floor(26.0, -4.0, 32.0, 27.0, FloorSkin::Pavement);
    b.floor(-39.0, -39.0, -16.0, -16.0, FloorSkin::Tiles);
    b.floor(16.0, -24.0, 39.0, -4.0, FloorSkin::Tiles);
    b.floor(-14.0, -39.0, 14.0, -12.0, FloorSkin::Sidewalk);
    // Spawn zones: painted borders (attackers orange, defenders blue).
    b.outline(-21.0, 28.5, 21.0, 38.5, 0.25, Paint::Orange);
    b.outline(-12.0, -36.6, 12.0, -28.5, 0.25, Paint::Blue);

    let zones = vec![
        zone("attacker spawn", -39.0, 27.0, 39.0, 39.0),
        // "A site" / "B site" are load-bearing: `Builder::finish` finds the plant zones by name.
        zone("the lower street", -39.0, -2.0, -30.0, 27.0),
        zone("the high street", -30.0, -16.0, -16.0, 4.0),
        zone("the cellars", -30.0, 12.0, -14.0, 17.0),
        zone("the west arch", -16.0, -26.0, -14.0, -22.0),
        zone("A site", -39.0, -39.0, -16.0, -16.0),
        zone("defender spawn", -14.0, -39.0, 14.0, -28.0),
        zone("the bell arch", -14.0, -28.0, 14.0, -12.0),
        zone("the vine alley", -14.0, -12.0, -10.0, 8.0),
        zone("the procession road", -14.0, 8.0, 16.0, 27.0),
        zone("the east arch", 14.0, -21.0, 16.0, -17.0),
        zone("B site", 16.0, -24.0, 39.0, -4.0),
        zone("the dog-leg", 14.0, -4.0, 32.0, 27.0),
    ];
    b.finish(
        MapId::Sirocco,
        zones,
        V2::new(0.0, 33.0),
        vec![
            V2::new(-5.0, -34.0),
            V2::new(0.0, -34.5),
            V2::new(5.0, -34.0),
            V2::new(-9.0, -30.0),
            V2::new(9.0, -30.0),
            V2::new(0.0, -30.5),
            V2::new(-30.0, -32.0),
            V2::new(27.0, -12.0),
        ],
    )
}
