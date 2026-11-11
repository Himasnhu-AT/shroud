use brain::arena::{Arena, MapId};
use serde_json::{json, Value};

fn r(v: f32) -> f64 { (v as f64 * 100.0).round() / 100.0 }

fn main() {
    let maps: Vec<Value> = MapId::ALL.iter().map(|&id| {
        let a = Arena::build(id);
        json!({
            // The variant name, not the display label: URLs and site keys stay put when a map is renamed.
            "id": format!("{id:?}").to_lowercase(),
            "label": id.label(),
            "blurb": id.blurb(),
            // Each map's own size (64 to 90 m across); the top-level "half" is only the default.
            "half": r(a.half()),
            "blocks": a.blocks.iter().map(|b| json!({
                "x": r(b.center.x), "y": r(b.center.y), "hx": r(b.half.x), "hy": r(b.half.y),
                "h": r(b.height), "kind": format!("{:?}", b.kind), "skin": format!("{:?}", b.skin),
            })).collect::<Vec<_>>(),
            "zones": a.zones.iter().map(|z| json!({
                "name": z.name, "x0": r(z.min.x), "y0": r(z.min.y), "x1": r(z.max.x), "y1": r(z.max.y),
            })).collect::<Vec<_>>(),
            "playerSpawn": [r(a.player_spawn.x), r(a.player_spawn.y)],
            "enemySpawns": a.enemy_spawns.iter().map(|s| [r(s.x), r(s.y)]).collect::<Vec<_>>(),
        })
    }).collect();
    println!("{}", serde_json::to_string(&json!({ "half": brain::arena::ARENA_HALF, "maps": maps })).unwrap());
}
