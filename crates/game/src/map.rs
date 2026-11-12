//! Builds the active map (`brain::arena::Arena::build(ShroudSettings.map)`) as meshes + colliders,
//! and swaps it when a new match starts on a different map.
//!
//! Ground plane mapping: `V2 { x, y }` ↔ world `(x, h, y)`.
//! Sky, sun and post-processing live in [`crate::gfx`]; per-map materials, lighting and set
//! dressing in [`look`], [`dressing`], [`props`] and [`grime`]; baked lighting in [`bake`].

pub mod bake;
mod dressing;
pub mod glass;
mod grime;
pub mod lamps;
mod look;
mod props;

use glass::GlassPlugin;

use avian3d::prelude::*;
use bevy::prelude::*;
use brain::arena::{Arena, BlockKind, MapId};
use brain::math::V2;

use crate::assets::AssetsDir;
use crate::gfx::PendingMips;
use crate::match_flow::{MatchFlow, ROUNDS_TO_WIN};
use crate::settings::ShroudSettings;
use engine::settings::{CoreSettings, GraphicsQuality};
use engine::render_budget::Scenery;
use crate::state::{AppState, GameState, Headless, has_visuals};

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        // SettingsPlugin is added before us, so the saved map is known at startup.
        let id = app.world().get_resource::<ShroudSettings>().map(|s| s.map).unwrap_or_default();
        app.insert_resource(ArenaRes(Arena::build(id)))
            .init_resource::<MapRequest>()
            .add_plugins((GlassPlugin, lamps::LampsPlugin))
            .add_plugins(bake::build)
            .add_systems(Startup, (spawn_current_map, grime::setup.run_if(has_visuals)))
            .add_systems(OnExit(AppState::MainMenu), |mut r: ResMut<MapRequest>| r.from_menu = true)
            // Map pieces are static scenery for the engine's render budget.
            .add_observer(|add: On<Add<ArenaPiece>>, mut c: Commands| {
                c.entity(add.entity).insert(Scenery);
            })
            // Before the round resets anything that reads `ArenaRes` (player, squad).
            .add_systems(
                OnEnter(GameState::Playing),
                sync_map
                    .in_set(MapSet)
                    .before(crate::match_flow::begin_round)
                    .before(crate::enemy::spawn_squad),
            )
            .add_systems(Update, (look::sync_look.run_if(look::look_inputs_changed), look::apply_light_budget, grime::spawn_grime).run_if(has_visuals));
        #[cfg(feature = "dev")]
        {
            use engine::devtools::{AddControlCommand, params};
            app.add_control_command(
                "map.load",
                "Switch map now (restarts the round)",
                params(&[("name", "string", "Sirocco | Gantry | Depot | Township | Office | Foundry")]),
                map_load_command,
            )
            .add_control_command("map.info", "Current map", params(&[]), map_info_command);
        }
    }
}

/// Runs on entering `Playing`, before the player and squad are placed.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapSet;

/// The arena layout shared with the AI (`brain`).
#[derive(Resource)]
pub struct ArenaRes(pub Arena);

/// Marker on every entity that belongs to the current map (geometry, props, map lights).
/// Despawned when the map changes.
#[derive(Component)]
pub struct ArenaPiece;

/// Pending map switch requests.
#[derive(Resource, Default)]
struct MapRequest {
    /// `map.load`: switch to this map on the next `Playing` entry, even mid-match.
    force: Option<MapId>,
    /// The next `Playing` entry comes from the menu (the player may have picked a new map).
    from_menu: bool,
}

/// Ground-plane point → world position at height `h`.
pub fn to_world(p: V2, h: f32) -> Vec3 {
    Vec3::new(p.x, h, p.y)
}

/// World position → ground-plane point (drops height).
pub fn to_ground(p: Vec3) -> V2 {
    V2::new(p.x, p.z)
}

/// Render-side resources for spawning map visuals (absent headless).
pub(crate) struct Gfx<'a> {
    pub meshes: &'a mut Assets<Mesh>,
    pub mats: &'a mut Assets<StandardMaterial>,
    pub server: &'a AssetServer,
    pub pending: &'a mut PendingMips,
    pub dir: &'a AssetsDir,
    pub quality: GraphicsQuality,
}

fn spawn_current_map(
    mut commands: Commands,
    arena: Res<ArenaRes>,
    headless: Res<Headless>,
    assets_dir: Res<AssetsDir>,
    core: Res<CoreSettings>,
    server: Option<Res<AssetServer>>,
    pending: Option<ResMut<PendingMips>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let quality = core.quality;
    match (headless.0, meshes, materials, server, pending) {
        (false, Some(mut m), Some(mut s), Some(a), Some(mut p)) => {
            let mut gfx = Gfx { meshes: &mut m, mats: &mut s, server: &a, pending: &mut p, dir: &assets_dir, quality };
            spawn_map(&mut commands, &arena.0, Some(&mut gfx));
        }
        _ => spawn_map(&mut commands, &arena.0, None),
    }
}

/// On entering `Playing`: if a new match starts (or we came from the menu) and the chosen
/// map differs from the built one, tear the old map down and build the new one.
fn sync_map(
    mut commands: Commands,
    mut arena: ResMut<ArenaRes>,
    mut request: ResMut<MapRequest>,
    flow: Res<MatchFlow>,
    pieces: Query<Entity, With<ArenaPiece>>,
    headless: Res<Headless>,
    assets_dir: Res<AssetsDir>,
    settings: Res<ShroudSettings>,
    core: Res<CoreSettings>,
    server: Option<Res<AssetServer>>,
    pending: Option<ResMut<PendingMips>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let new_match = flow.round == 0 || flow.player_rounds >= ROUNDS_TO_WIN || flow.enemy_rounds >= ROUNDS_TO_WIN;
    let from_menu = std::mem::take(&mut request.from_menu);
    let wanted = match request.force.take() {
        Some(id) => id,
        None if new_match || from_menu => settings.map,
        None => return,
    };
    if wanted == arena.0.id {
        return;
    }
    info!("map: {} -> {}", arena.0.id.label(), wanted.label());
    for e in &pieces {
        commands.entity(e).despawn();
    }
    arena.0 = Arena::build(wanted);
    let quality = core.quality;
    match (headless.0, meshes, materials, server, pending) {
        (false, Some(mut m), Some(mut s), Some(a), Some(mut p)) => {
            let mut gfx = Gfx { meshes: &mut m, mats: &mut s, server: &a, pending: &mut p, dir: &assets_dir, quality };
            spawn_map(&mut commands, &arena.0, Some(&mut gfx));
        }
        _ => spawn_map(&mut commands, &arena.0, None),
    }
}

/// Spawn floor, blocks, door headers and roofs (with colliders), plus visuals and map lights
/// when `gfx` is given. Everything gets [`ArenaPiece`].
fn spawn_map(commands: &mut Commands, arena: &Arena, mut gfx: Option<&mut Gfx>) {
    let mut pal = gfx.as_deref_mut().map(|g| look::Palette::new(g, arena));

    // Floor: a thick static slab whose top is at y = 0.
    let floor_size = arena.half() * 2.0 + 8.0;
    let floor_center = Vec3::new(0.0, -0.5, 0.0);
    let mut floor = commands.spawn((
        Name::new("Floor"),
        ArenaPiece,
        RigidBody::Static,
        Collider::cuboid(floor_size, 1.0, floor_size),
        Transform::from_translation(floor_center),
    ));
    if let (Some(g), Some(p)) = (gfx.as_deref_mut(), pal.as_mut()) {
        let (mat, tile) = p.floor(g);
        floor.insert((
            Mesh3d(g.meshes.add(crate::gfx::world_uv_cuboid(Vec3::new(floor_size, 1.0, floor_size), floor_center, tile))),
            MeshMaterial3d(mat),
        ));
    }

    // Small static wall pieces (window sills, Township lintels) merged per material.
    let mut merged = dressing::Batch::default();
    // Small low cover merged without shadows.
    let mut small = dressing::Batch::default();
    for (i, b) in arena.blocks.iter().enumerate() {
        let size = Vec3::new(b.half.x * 2.0, b.height, b.half.y * 2.0);
        let kind_name = match b.kind {
            BlockKind::Wall => "Wall",
            BlockKind::Tall => "Tall",
            BlockKind::Low => "Low",
        };
        let center = to_world(b.center, b.height * 0.5);
        let id = commands
            .spawn((
                Name::new(format!("{kind_name} block {i}")),
                ArenaPiece,
                RigidBody::Static,
                look::block_collider(b),
                Transform::from_translation(center),
            ))
            .id();
        if let (Some(g), Some(p)) = (gfx.as_deref_mut(), pal.as_mut()) {
            if b.kind == BlockKind::Low && p.shell_of.contains_key(&i) {
                let key = p.block_key(i, b);
                merged.add(g, p, &key, size, center);
            } else {
                look::dress_block(commands, id, i, b, size, center, g, p, &mut merged, &mut small);
            }
        }
    }

    // Door lintels and roofs: solid (bullets stop on them) but overhead, so the 2D AI
    // never needs to know about them.
    for (i, h) in arena.decor.headers.iter().enumerate() {
        let (min, max) = (h.min, h.max);
        let size = Vec3::new(max.x - min.x, h.top - h.bottom, max.y - min.y);
        let center = Vec3::new((min.x + max.x) * 0.5, (h.top + h.bottom) * 0.5, (min.y + max.y) * 0.5);
        let mut e = commands.spawn((
            Name::new(format!("Door header {i}")),
            ArenaPiece,
            RigidBody::Static,
            Collider::cuboid(size.x, size.y, size.z),
            Transform::from_translation(center),
        ));
        if let (Some(g), Some(p)) = (gfx.as_deref_mut(), pal.as_mut()) {
            // Lintels and arches on maps with buildings are part of their wall: same material
            // as the wall beside them.
            let wall = matches!(arena.id, MapId::Township | MapId::Sirocco | MapId::Gantry | MapId::Foundry).then(|| wall_beside(arena, min, max)).flatten();
            match wall {
                Some(k) => {
                    let key = p.block_key(k, &arena.blocks[k]);
                    merged.add(g, p, &key, size, center);
                }
                None => {
                    let (mat, tile) = p.header(g);
                    e.insert((Mesh3d(g.meshes.add(crate::gfx::world_uv_cuboid(size, center, tile))), MeshMaterial3d(mat)));
                }
            }
        }
    }
    for (i, r) in arena.decor.roofs.iter().enumerate() {
        let size = Vec3::new(r.max.x - r.min.x, r.thickness, r.max.y - r.min.y);
        let center = Vec3::new((r.min.x + r.max.x) * 0.5, r.bottom + r.thickness * 0.5, (r.min.y + r.max.y) * 0.5);
        let mut e = commands.spawn((
            Name::new(format!("Roof {i}")),
            ArenaPiece,
            RigidBody::Static,
            Collider::cuboid(size.x, size.y, size.z),
            Transform::from_translation(center),
        ));
        if let (Some(g), Some(p)) = (gfx.as_deref_mut(), pal.as_mut()) {
            // A building's roof slab is its ceiling inside.
            let (mat, tile) = match arena.decor.buildings.iter().find(|b| b.roof == i) {
                Some(b) => p.mat(g, look::Palette::ceiling_key(b.use_)),
                None => p.roof(g),
            };
            e.insert((Mesh3d(g.meshes.add(crate::gfx::world_uv_cuboid(size, center, tile))), MeshMaterial3d(mat)));
        }
    }

    if let (Some(g), Some(p)) = (gfx, pal.as_mut()) {
        small.spawn(commands, g, p, "Low cover", false);
        dressing::spawn(commands, arena, g, p, merged);
        // Baked lighting for this map, when its cache exists (`assets/baked/<map>/`).
        bake::spawn(commands, arena);
    }
}

/// The wall block that a doorway lintel spanning `min`..`max` is cut into.
fn wall_beside(arena: &Arena, min: V2, max: V2) -> Option<usize> {
    let along_x = max.x - min.x > max.y - min.y;
    arena.blocks.iter().position(|b| {
        b.kind == BlockKind::Tall && {
            let (lo, hi) = (b.center - b.half, b.center + b.half);
            if along_x {
                (lo.y - min.y).abs() < 0.01 && (hi.y - max.y).abs() < 0.01 && ((hi.x - min.x).abs() < 0.01 || (lo.x - max.x).abs() < 0.01)
            } else {
                (lo.x - min.x).abs() < 0.01 && (hi.x - max.x).abs() < 0.01 && ((hi.y - min.y).abs() < 0.01 || (lo.y - max.y).abs() < 0.01)
            }
        }
    })
}

// ---------------------------------------------------------------- control port

#[cfg(any(feature = "dev", test))]
fn parse_map(name: &str) -> Option<MapId> {
    // Match the display label or the stable variant name ("office" still works after a rename).
    let n = name.trim();
    MapId::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(n) || format!("{m:?}").eq_ignore_ascii_case(n))
}

#[cfg(feature = "dev")]
fn map_load_command(world: &mut World, p: serde_json::Value) -> Result<serde_json::Value, String> {
    let name = p.get("name").and_then(serde_json::Value::as_str).ok_or("`name` (string) is required")?;
    let id = parse_map(name).ok_or_else(|| format!("unknown map `{name}` (Sirocco, Gantry, Depot, Township, Office, Foundry)"))?;
    if let Some(mut s) = world.get_resource_mut::<ShroudSettings>() {
        s.map = id;
    }
    world.resource_mut::<MapRequest>().force = Some(id);
    // Re-entering Playing rebuilds the map, then resets the player and squad on it.
    world.resource_mut::<NextState<GameState>>().set(GameState::Playing);
    Ok(serde_json::json!({ "map": id.label() }))
}

#[cfg(feature = "dev")]
fn map_info_command(world: &mut World, _: serde_json::Value) -> Result<serde_json::Value, String> {
    let a = &world.resource::<ArenaRes>().0;
    Ok(serde_json::json!({
        "map": a.id.label(),
        "blocks": a.blocks.len(),
        "cover": a.cover.len(),
        "player_spawn": [a.player_spawn.x, a.player_spawn.y],
        "enemy_spawns": a.enemy_spawns.iter().map(|s| [s.x, s.y]).collect::<Vec<_>>(),
        "zones": a.zones.iter().map(|z| z.name).collect::<Vec<_>>(),
        "lamps": a.decor.lamps.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_names_parse_case_insensitively() {
        assert_eq!(parse_map("office"), Some(MapId::Office));
        assert_eq!(parse_map(" Depot "), Some(MapId::Depot));
        assert_eq!(parse_map("township"), Some(MapId::Township));
        assert_eq!(parse_map("sirocco"), Some(MapId::Sirocco));
        assert_eq!(parse_map("Gantry"), Some(MapId::Gantry));
        assert_eq!(parse_map("nuke"), None);
    }
}
