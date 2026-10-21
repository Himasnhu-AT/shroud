//! Print a map's nav grid as ASCII: `cargo run -p brain --example map_ascii -- depot`.
//! `#` blocked, `.` walkable, `P` player spawn, digits enemy spawns. North (-z) is up.

use brain::arena::{Arena, MapId};

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "foundry".into());
    let id = MapId::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(&name)).unwrap_or_default();
    let a = Arena::build(id);
    let w = a.nav.w;
    let mut rows: Vec<Vec<char>> = (0..w)
        .map(|z| (0..w).map(|x| if a.nav.walkable[z * w + x] { '.' } else { '#' }).collect())
        .collect();
    let mut put = |p: brain::math::V2, c: char| {
        let (x, z) = a.nav.cell_of(p);
        rows[z][x] = c;
    };
    put(a.player_spawn, 'P');
    for (i, s) in a.enemy_spawns.iter().enumerate() {
        put(*s, char::from_digit(i as u32, 10).unwrap_or('E'));
    }
    println!("{} - {} blocks, {} cover points, {} zones", id.label(), a.blocks.len(), a.cover.len(), a.zones.len());
    for r in rows {
        println!("{}", r.into_iter().flat_map(|c| [c, c]).collect::<String>());
    }
}
