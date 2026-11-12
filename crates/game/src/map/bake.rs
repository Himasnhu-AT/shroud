//! Baked lighting for the maps (engine `bake`, engine docs/BAKE.md): lightmaps on the static
//! level and one light-probe volume per map for everything that moves, baked offline by headless
//! Blender and cached in `assets/baked/<map>/`.
//!
//! - **Which bake**: `BakeLevel` follows the map ([`bake_id`]: `MapId::Office` → `"office"`), set
//!   when the map's visuals are built (`spawn_map`, so both `spawn_current_map` and `sync_map`).
//!   A map without a cache folder simply has no bake: it keeps the look's ambient and sky light,
//!   as before baking existed.
//! - **What is static**: every `ArenaPiece` is `Scenery` (merged level meshes, blocks, roofs,
//!   dressing), so the bake sees exactly the level. Moving things are `Actor` (player, enemies,
//!   pickups, FX, grenades, glass shards, casings, bullet holes) and never baked. Glass panes and
//!   edge grime are alpha-blended (light passes through them in the bake); decals are not
//!   `StandardMaterial`s. The glowing part of each lamp is [`NoBake`]: its light is the lamp's
//!   own light below, and its material swaps to dark glass when it's shot out.
//! - **Lamps** ([`LAMP_BAKE`]): map lamps can be shot out, so their direct light stays realtime.
//!   They are baked `Indirect`: the bake holds their bounce (the ceiling over a lit office, the
//!   wall beside a bulkhead), and a lamp that dies keeps its baked bounce until the round
//!   ends. That residue is a fraction of the bounce light at the spot (neighbouring lamps
//!   overlap on every lamp-lit map): checked by eye in `scripts/bake-lamp-out.json`, it reads
//!   as the room's fill, not as a lit lamp. `Ignore` would leave lamp-lit interiors (all of
//!   Ledger) with no bounce at all: black ceilings under every lamp.
//! - **Sun and sky** are baked (the sun `Indirect`: its direct light and shadows stay realtime).
//! - **Probes**: one [`BakeVolume`] over the playable area, floor to the highest roof
//!   ([`probe_volume`]). macOS binds only one irradiance volume, so it's one box per map.
//!
//! Baking a map (needs Blender 4.2+; the game's dev build):
//! `python3 -I scripts/play.py scripts/bake-foundry.json --out .tmp/bake` (a `map.load`, then
//! `bake.run` and a wait on `bake.status`), then commit `assets/baked/foundry/`.

use bevy::prelude::*;
use brain::arena::{Arena, MapId};
use engine::bake::{BakeConfig, BakeLevel, BakeLight, BakeVolume, NoBake};

use super::lamps::LampGlow;
use super::look::MapLamp;

/// How the map lamps take part in the bake (see the module docs).
pub const LAMP_BAKE: BakeLight = BakeLight::Indirect;

/// Probe spacing (m) we aim for: walls are 0.2-0.4 m thick, so finer is better against light
/// leaking through them, but a map has to fit in [`MAX_PROBES`].
pub const PROBE_SPACING: f32 = 1.0;
/// Probe budget for one map (six RGBA16F sides each: 60 000 probes ≈ 2.9 MB).
pub const MAX_PROBES: u32 = 60_000;

pub(super) fn build(app: &mut App) {
    app.add_observer(|add: On<Add<MapLamp>>, mut c: Commands| {
        c.entity(add.entity).insert(LAMP_BAKE);
    })
    .add_observer(|add: On<Add<LampGlow>>, mut c: Commands| {
        c.entity(add.entity).insert(NoBake);
    });
    app.add_systems(Startup, |config: Option<ResMut<BakeConfig>>| {
        let Some(mut c) = config else { return };
        if c.settings.max_probes < MAX_PROBES {
            c.settings.max_probes = MAX_PROBES;
        }
        // Dev builds: bake with another copy of the engine's Blender script (to try a fix).
        #[cfg(feature = "dev")]
        if let Some(p) = std::env::var_os("SHROUD_BAKE_SCRIPT") {
            c.script = Some(p.into());
        }
    });
}

/// The bake cache folder of a map: its variant in lower case (as saves and the website key it,
/// so a display rename never orphans a bake).
pub fn bake_id(id: MapId) -> String {
    format!("{id:?}").to_ascii_lowercase()
}

/// The map's probe box (centre, size) and spacing: the whole playable square inside the outer
/// walls, from the floor to just over the highest roof or block.
pub fn probe_volume(arena: &Arena) -> (Vec3, Vec3, f32) {
    let half = arena.half();
    let blocks = arena.blocks.iter().map(|b| b.height);
    let roofs = arena.decor.roofs.iter().map(|r| r.bottom + r.thickness);
    let headers = arena.decor.headers.iter().map(|h| h.top);
    let top = blocks.chain(roofs).chain(headers).fold(3.0f32, f32::max).min(16.0) + 0.5;
    let size = Vec3::new(half * 2.0, top, half * 2.0);
    // Grow the spacing only as far as the budget needs.
    let cells = |s: f32| (size / s).ceil() + Vec3::ONE;
    let mut spacing = PROBE_SPACING;
    while { let c = cells(spacing); (c.x * c.y * c.z) as u32 } > MAX_PROBES {
        spacing += 0.1;
    }
    (Vec3::new(0.0, top * 0.5, 0.0), size, spacing)
}

/// Point the bake at this map and give it its probe volume (an `ArenaPiece`, so it goes with
/// the map). Visual builds only.
pub(super) fn spawn(commands: &mut Commands, arena: &Arena) {
    commands.insert_resource(BakeLevel::new(bake_id(arena.id)));
    let (center, size, spacing) = probe_volume(arena);
    commands.spawn((
        Name::new("Bake probe volume"),
        super::ArenaPiece,
        BakeVolume { spacing },
        Transform::from_translation(center).with_scale(size),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_map_has_a_bake_id_and_a_probe_box_in_budget() {
        for id in MapId::ALL {
            let a = Arena::build(id);
            assert_eq!(bake_id(id), format!("{id:?}").to_lowercase());
            let (c, size, spacing) = probe_volume(&a);
            let n = ((size / spacing).ceil() + Vec3::ONE).element_product() as u32;
            assert!(n <= MAX_PROBES, "{id:?}: {n} probes");
            assert!(spacing <= 1.5, "{id:?}: spacing {spacing}");
            assert!(c.y > 1.5 && size.x >= a.half() * 2.0);
        }
        assert_eq!(bake_id(MapId::Office), "office");
    }
}
