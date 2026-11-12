//! Loading screen: up at launch, when a match starts and when a map is (re)built, until the map
//! is built, its bake applied and every render pipeline it needs is compiled. Each later round
//! gets the short form: the black "camera power-on" cover, held by the same checks.
//!
//! Why: on macOS Bevy compiles render pipelines synchronously on the render thread (engine
//! docs/GAME_GUIDE.md 3.3, 3.6). Foundry with MetaHuman enemies needs ~200+ of them; on a cold
//! shader cache that was a single 17-20 s frame (the window froze, then the round was already
//! running). The engine's `WarmPipelines` draws every material on a *quad*, which compiles the
//! quad's vertex layout only: real meshes (tangents, lightmap UVs, skinning) still compiled on
//! first sight, mid-round, and the camera saw the whole map in one frame anyway.
//!
//! Stages ([`Stage`]):
//! 1. **Map**: the map's pieces exist, every glTF scene is spawned and every material texture
//!    has loaded (or failed), and the mesh count has been steady for a few frames.
//! 2. **Bake**: the map's lightmaps and probes are applied (`LoadedBake` for this level, every
//!    mesh matched). Lightmapped meshes have their own vertex layout, so this comes before 3.
//! 3. **Shaders**: one representative entity per (material, vertex layout, lightmap) is drawn,
//!    a few per frame, wherever it is: for those frames it is visible, unculled, out of any
//!    `VisibilityRange` and on [`LOAD_LAYER`], while every 3D camera and light sees only that
//!    layer. So each frame compiles a small batch (real progress, a responsive window) instead
//!    of everything at once, and hidden things (holstered guns, far LODs) are compiled too.
//!    The batch size adapts to keep frames near [`TARGET_FRAME_S`].
//! 4. **Settle**: cameras and lights are restored (the shot effects, played out of sight at the
//!    end of stage 1 by [`WarmShots`] / `weapon::warm`, are still there) and we wait until the pipeline cache has been quiet for
//!    [`QUIET_FRAMES`]. Whatever the representatives missed compiles here, still covered.
//!
//! From the first frame every 3D camera and light sees only [`LOAD_LAYER`], so the view behind
//! the cover draws (and compiles) nothing until the Shaders stage feeds it.
//!
//! The count shown is real: the render world's `PipelineCache` (compiled / known); the total is
//! an estimate from the pipelines per representative so far.
//!
//! While it is up nothing plays: virtual and physics time are paused until the Settle stage,
//! Shroud's `not_paused` run condition is false (weapons, AI, round timer, grenades, bomb), the pointer isn't captured, Esc
//! doesn't pause, look/move input is zeroed and the round's ambience and start sound wait for
//! [`Loaded`]. Then the cover fades out ([`FADE_S`]): the camera powers on.
//!
//! Each run logs `loading` to the session log (seconds, pipelines, worst frame) and the worst
//! frame of the next 5 s of play (`loading.after`): a stall there means something escaped.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers, VisibilityRange};
use bevy::prelude::*;
use bevy::render::render_resource::{CachedPipelineState, PipelineCache};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::window::RequestRedraw;
use bevy::world_serialization::WorldAssetRoot;
use engine::bake::{BakeApplied, BakeLevel, LoadedBake};
use engine::render_budget::DetailTier;

use crate::map::{ArenaPiece, ArenaRes};
use crate::state::{GameState, has_visuals};
use crate::ui::theme::*;

/// Render layer used only while warming (the engine's render scale uses 30).
pub const LOAD_LAYER: usize = 29;
/// Batch frames are sized to take about this long (the screen still updates).
const TARGET_FRAME_S: f32 = 0.12;
/// Frames a representative stays drawn (queue, specialise, compile).
const HOLD_FRAMES: u32 = 2;
/// Frames the pipeline cache must stay unchanged before the screen goes.
const QUIET_FRAMES: u32 = 4;
/// Fade from the cover to the picture.
pub const FADE_S: f32 = 0.35;
/// Shortest time the full screen shows (no one-frame flash on a warm cache).
const MIN_SHOW_S: f64 = 0.6;
/// Give up waiting on a stage after this long without progress (a missing asset must not trap
/// the player).
const STAGE_TIMEOUT_S: f64 = 25.0;
/// Practice: seconds of the real view behind the cover for the quality governor (it steps after
/// 0.5 s over budget; one step per session).
pub const PRACTICE_MEASURE_S: f64 = 2.5;
/// How long after loading we watch for stalls that escaped the warm-up.
const AFTER_S: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Launch: the menu's live map.
    Boot,
    /// A match starts or the map changes: the full screen.
    Match,
    /// A later round on the same map: the black power-on cover only.
    Round,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Map,
    Bake,
    Shaders,
    Settle,
    Fade,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Stage::Map => "MAP GEOMETRY",
            Stage::Bake => "BAKED LIGHT",
            Stage::Shaders => "SHADERS",
            Stage::Settle => "SENSOR WARM-UP",
            Stage::Fade => "SENSOR WARM-UP",
        }
    }
}

/// The loading state (inactive outside loads).
#[derive(Resource, Debug)]
pub struct Loading {
    pub active: bool,
    pub kind: Kind,
    pub stage: Stage,
    started: f64,
    stage_started: f64,
    /// Mesh entity count and how many frames it has held still (Map stage).
    meshes: usize,
    steady: u32,
    /// Representatives to draw, how many are done, how many per frame.
    queue: Vec<Entity>,
    reps_total: usize,
    reps_done: usize,
    batch: usize,
    /// Representatives enabled last frame, and the seconds each costs (EMA).
    last_batch: usize,
    cost: f32,
    /// Pipelines known when the Shaders stage started, and the quiet-frame count.
    pipes_at_shaders: u32,
    last_total: u32,
    quiet: u32,
    stage_frames: u32,
    fade: f32,
    /// We paused `Time<Virtual>` (and physics) and must unpause it.
    paused_time: bool,
    /// Worst frame while loading (s), frames.
    worst: f32,
    frames: u32,
    /// Practice: the quality governor's measuring window ends here (real seconds).
    measure_until: Option<f64>,
    /// Watching the first seconds after a load.
    after_until: Option<f64>,
    after_worst: f32,
}

impl Default for Loading {
    fn default() -> Self {
        Loading {
            active: false,
            kind: Kind::Boot,
            stage: Stage::Map,
            started: 0.0,
            stage_started: 0.0,
            meshes: 0,
            steady: 0,
            queue: Vec::new(),
            reps_total: 0,
            reps_done: 0,
            batch: 1,
            last_batch: 0,
            cost: 0.05,
            pipes_at_shaders: 0,
            last_total: 0,
            quiet: 0,
            stage_frames: 0,
            fade: 0.0,
            paused_time: false,
            worst: 0.0,
            frames: 0,
            measure_until: None,
            after_until: None,
            after_worst: 0.0,
        }
    }
}

impl Loading {
    /// Gameplay is held (everything but the final fade).
    pub fn holds(&self) -> bool {
        self.active && self.stage != Stage::Fade
    }
    /// The quality governor may judge now: a practice session's measuring window (the scene is
    /// drawn and compiled, the cover still up).
    pub fn measuring(&self) -> bool {
        self.active && self.stage == Stage::Settle && self.measure_until.is_some()
    }
    /// 0..1 for the bar.
    fn progress(&self) -> f32 {
        let shaders = if self.reps_total == 0 { 1.0 } else { self.reps_done as f32 / self.reps_total as f32 };
        match self.stage {
            Stage::Map => 0.05 + 0.1 * (self.steady as f32 / 3.0).min(1.0),
            Stage::Bake => 0.18,
            Stage::Shaders => 0.25 + 0.65 * shaders,
            Stage::Settle => 0.92 + 0.08 * (self.quiet as f32 / QUIET_FRAMES as f32).min(1.0),
            Stage::Fade => 1.0,
        }
    }
}

/// Is gameplay held by a load? (For run conditions; false headless.)
pub fn loading_holds(l: Option<Res<Loading>>) -> bool {
    l.is_some_and(|l| l.holds())
}

/// Play one of each shot effect out of sight (`weapon::warm`) so their pipelines compile.
#[derive(Message, Clone, Copy, Debug, Default)]
pub struct WarmShots;

/// Sent when a load ends (the cover starts to fade).
#[derive(Message, Clone, Copy, Debug)]
pub struct Loaded(#[allow(dead_code)] pub Kind);

/// Pipeline counts from the render world's `PipelineCache` (shared with it).
#[derive(Resource, Clone, Default)]
pub struct PipeStats(Arc<PipeInner>);

#[derive(Default)]
pub struct PipeInner {
    watch: AtomicBool,
    total: AtomicU32,
    ready: AtomicU32,
    waiting: AtomicU32,
}

impl PipeStats {
    pub fn total(&self) -> u32 {
        self.0.total.load(Ordering::Relaxed)
    }
    pub fn ready(&self) -> u32 {
        self.0.ready.load(Ordering::Relaxed)
    }
    pub fn waiting(&self) -> u32 {
        self.0.waiting.load(Ordering::Relaxed)
    }
}

fn count_pipelines(stats: Res<PipeStats>, cache: Option<Res<PipelineCache>>, mut seen: Local<usize>) {
    let Some(cache) = cache else { return };
    // `SHROUD_LOADING_DEBUG=1`: log every new pipeline (label and shader defs).
    if std::env::var_os("SHROUD_LOADING_DEBUG").is_some() {
        use bevy::render::render_resource::PipelineDescriptor;
        for (i, p) in cache.pipelines().enumerate().skip(*seen) {
            let (label, defs) = match &p.descriptor {
                PipelineDescriptor::RenderPipelineDescriptor(d) => (d.label.clone(), d.vertex.shader_defs.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>()),
                PipelineDescriptor::ComputePipelineDescriptor(d) => (d.label.clone(), d.shader_defs.iter().map(|d| format!("{d:?}")).collect()),
                PipelineDescriptor::MeshPipelineDescriptor(_) => (None, Vec::new()),
            };
            info!("pipeline {i}: {:?} {}", label, defs.join(" "));
        }
        *seen = cache.pipelines().count();
    }
    if !stats.0.watch.load(Ordering::Relaxed) {
        return;
    }
    let (mut total, mut ready) = (0u32, 0u32);
    for p in cache.pipelines() {
        total += 1;
        if matches!(p.state, CachedPipelineState::Ok(_)) {
            ready += 1;
        }
    }
    stats.0.total.store(total, Ordering::Relaxed);
    stats.0.ready.store(ready, Ordering::Relaxed);
    stats.0.waiting.store(cache.waiting_pipelines().count() as u32, Ordering::Relaxed);
}

/// What a representative changed, to put back.
#[derive(Component)]
struct Warming {
    layers: Option<RenderLayers>,
    vis: Option<Visibility>,
    range: Option<VisibilityRange>,
    had_no_cull: bool,
    frames: u32,
}

/// On a camera or light moved to [`LOAD_LAYER`] while loading: its own layers.
#[derive(Component)]
struct ViewHeld(Option<RenderLayers>);

/// Representative keys already compiled this session (a later load skips them).
#[derive(Resource, Default)]
struct Warmed(HashSet<RepKey>);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct RepKey {
    material: AssetId<StandardMaterial>,
    /// Vertex attribute ids, or the mesh id when its data is GPU-only.
    layout: Vec<u64>,
    lightmap: bool,
    shadows: bool,
}

#[derive(Component)]
struct Cover;
#[derive(Component)]
struct CoverPanel;
#[derive(Component)]
struct CoverTitle;
#[derive(Component)]
struct CoverStep(Stage);
#[derive(Component)]
struct CoverTag(Stage);
#[derive(Component)]
struct CoverNote;
#[derive(Component)]
struct CoverBar;
#[derive(Component)]
struct CoverCount;
#[derive(Component)]
struct CoverDot;

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        let stats = PipeStats::default();
        app.init_resource::<Loading>()
            .init_resource::<Warmed>()
            .insert_resource(stats.clone())
            .add_message::<Loaded>()
            .add_message::<WarmShots>()
            .add_systems(Startup, (|mut c: Commands| c.queue(|w: &mut World| begin(w, Kind::Boot))).run_if(has_visuals))
            .init_resource::<LastEntry>()
            .add_systems(OnEnter(crate::state::AppState::MainMenu), |mut l: ResMut<LastEntry>| l.0 = None)
            .add_systems(
                OnEnter(GameState::Playing),
                begin_round_load.run_if(has_visuals).after(crate::map::MapSet).after(crate::match_flow::begin_round),
            )
            .add_systems(Update, (drive, draw_cover).chain().run_if(has_visuals))
            .add_systems(PreUpdate, zero_look.after(engine::input::InputSet).run_if(loading_holds))
            .add_systems(Update, zero_move.after(engine::fps::FpsSet::Input).before(engine::fps::FpsSet::Move).run_if(loading_holds));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(stats).add_systems(Render, count_pipelines.in_set(RenderSystems::Cleanup));
        }
        #[cfg(feature = "dev")]
        {
            use engine::devtools::{AddControlCommand, params};
            app.add_control_command("loading.status", "Loading screen: active, kind, stage, progress, pipelines", params(&[]), |world: &mut World, _| {
                let l = world.resource::<Loading>();
                let p = world.resource::<PipeStats>();
                Ok(serde_json::json!({
                    "active": l.active,
                    "holds": l.holds(),
                    "kind": format!("{:?}", l.kind),
                    "stage": format!("{:?}", l.stage),
                    "progress": l.progress(),
                    "reps": [l.reps_done, l.reps_total],
                    "pipelines": { "ready": p.ready(), "total": p.total(), "waiting": p.waiting() },
                    "worst_ms": l.worst * 1000.0,
                }))
            });
        }
    }
}

/// Entering `Playing`: the full screen for a new match or a rebuilt map, else the short cover.
/// What the last `Playing` entry was (map, practice), to tell a new session from a respawn.
/// Cleared on the main menu.
#[derive(Resource, Default)]
struct LastEntry(Option<(brain::arena::MapId, bool)>);

/// Entering `Playing`: the full screen for a new match, a new practice session or a rebuilt
/// map; the short cover for a later round, a practice respawn or a restart.
fn begin_round_load(mut commands: Commands, flow: Res<crate::match_flow::MatchFlow>, arena: Res<ArenaRes>, practice: Option<Res<crate::practice::Practice>>, mut last: ResMut<LastEntry>) {
    let entry = (arena.0.id, practice.is_some_and(|p| p.on));
    let kind = entry_kind(last.0, entry, flow.round);
    last.0 = Some(entry);
    commands.queue(move |w: &mut World| begin(w, kind));
}

/// Full screen or short cover for a `Playing` entry after `last` (practice has no rounds: only
/// a new session, or a map change, gets the full screen).
fn entry_kind(last: Option<(brain::arena::MapId, bool)>, now: (brain::arena::MapId, bool), round: u32) -> Kind {
    let new_session = last != Some(now);
    let new_match = !now.1 && round <= 1;
    if new_session || new_match { Kind::Match } else { Kind::Round }
}

/// Start (or restart) a load.
pub fn begin(world: &mut World, kind: Kind) {
    restore_all(world);
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let was_active = world.resource::<Loading>().active;
    let old_kind = world.resource::<Loading>().kind;
    let paused_time = world.resource::<Loading>().paused_time;
    // A boot load still running when a match starts becomes the match's load.
    let kind = if was_active && old_kind != Kind::Round && kind == Kind::Round { old_kind } else { kind };
    *world.resource_mut::<Loading>() = Loading { active: true, kind, started: now, stage_started: now, paused_time, ..default() };
    // Hold time (the round timer, AI, physics) unless something else already paused it.
    if !paused_time {
        let mut virt = world.resource_mut::<Time<Virtual>>();
        if !virt.is_paused() {
            virt.pause();
            if let Some(mut p) = world.get_resource_mut::<Time<avian3d::prelude::Physics>>() {
                use avian3d::prelude::PhysicsTime;
                p.pause();
            }
            world.resource_mut::<Loading>().paused_time = true;
        }
    }
    world.resource::<PipeStats>().0.watch.store(true, Ordering::Relaxed);
    spawn_cover(world, kind);
    info!("loading: {kind:?} started");
}

fn spawn_cover(world: &mut World, kind: Kind) {
    let old: Vec<Entity> = world.query_filtered::<Entity, With<Cover>>().iter(world).collect();
    for e in old {
        world.entity_mut(e).despawn();
    }
    let map = world.get_resource::<ArenaRes>().map(|a| a.0.id.label().to_uppercase()).unwrap_or_default();
    let title = match kind {
        Kind::Boot => "UNIT 4 // BOOTING CAMERA".to_string(),
        Kind::Match if world.get_resource::<crate::practice::Practice>().is_some_and(|p| p.on) => format!("UNIT 4 // PRACTICE — {map}"),
        Kind::Match => format!("UNIT 4 // DEPLOYING — {map}"),
        Kind::Round => String::new(),
    };
    let mut root = world.spawn((
        Name::new("Loading cover"),
        Cover,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(1000),
    ));
    if kind == Kind::Round {
        return;
    }
    root.with_children(|r| {
        r.spawn((
            CoverPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(8.0),
                bottom: Val::Percent(14.0),
                width: Val::Px(560.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn(Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, ..default() }).with_children(|row| {
                row.spawn((CoverDot, Node { width: Val::Px(10.0), height: Val::Px(10.0), border_radius: BorderRadius::MAX, ..default() }, BackgroundColor(Color::srgb(0.95, 0.12, 0.1))));
                row.spawn((Text::new("STANDBY"), label(16.0), TextColor(Color::srgb(0.95, 0.2, 0.16))));
            });
            p.spawn((CoverTitle, Text::new(title), label(26.0), TextColor(TEXT)));
            // Bar.
            p.spawn((Node { width: Val::Percent(100.0), height: Val::Px(3.0), margin: UiRect::vertical(Val::Px(6.0)), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.1))))
                .with_children(|b| {
                    b.spawn((CoverBar, Node { width: Val::Percent(0.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(ACCENT)));
                });
            for s in [Stage::Map, Stage::Bake, Stage::Shaders, Stage::Settle] {
                p.spawn(Node { column_gap: Val::Px(12.0), align_items: AlignItems::Center, ..default() }).with_children(|row| {
                    // Fixed-width status tag: OK / .. / blank.
                    row.spawn((
                        Node { width: Val::Px(40.0), height: Val::Px(18.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border: UiRect::all(Val::Px(1.0)), ..default() },
                        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.12)),
                    ))
                    .with_children(|tag| {
                        tag.spawn((CoverTag(s), Text::new(""), label(13.0), TextColor(TEXT_DIM)));
                    });
                    row.spawn((CoverStep(s), Text::new(s.label()), label(15.0), TextColor(TEXT_DIM)));
                    if s == Stage::Shaders {
                        row.spawn((CoverCount, Text::new(""), label(15.0), TextColor(TEXT_MUTED)));
                    }
                });
            }
            p.spawn((CoverNote, Text::new(""), label(13.0), TextColor(TEXT_DIM), Node { margin: UiRect::top(Val::Px(8.0)), ..default() }));
        });
        r.spawn((
            Node { position_type: PositionType::Absolute, right: Val::Px(36.0), bottom: Val::Px(28.0), ..default() },
            Text::new(crate::bodycam::hud::DEVICE_ID),
            label(14.0),
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.45)),
        ));
    });
}

/// Representatives not warmed yet: one entity per (material, layout, lightmap, shadow casting).
fn collect_reps(world: &mut World) -> Vec<Entity> {
    let mut q = world.query_filtered::<(Entity, &Mesh3d, &MeshMaterial3d<StandardMaterial>, Has<bevy::pbr::Lightmap>, Has<bevy::light::NotShadowCaster>), Without<Warming>>();
    let meshes = world.resource::<Assets<Mesh>>();
    let warmed = world.resource::<Warmed>();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (e, mesh, mat, lightmap, no_shadow) in q.iter(world) {
        let layout = layout_of(meshes, &mesh.0);
        let key = RepKey { material: mat.0.id(), layout, lightmap, shadows: !no_shadow };
        if warmed.0.contains(&key) || !seen.insert(key) {
            continue;
        }
        out.push(e);
    }
    out
}

/// A mesh's vertex layout as sorted attribute-id hashes (the mesh id when its data is GPU-only).
fn layout_of(meshes: &Assets<Mesh>, mesh: &Handle<Mesh>) -> Vec<u64> {
    use std::hash::{Hash, Hasher};
    fn hash(v: impl Hash) -> u64 {
        let mut d = std::collections::hash_map::DefaultHasher::new();
        v.hash(&mut d);
        d.finish()
    }
    match meshes.get(mesh).and_then(|m| m.try_attributes().ok().map(|a| a.map(|(attr, _)| hash(attr.id)).collect::<Vec<_>>())) {
        Some(mut ids) => {
            ids.sort_unstable();
            ids
        }
        None => vec![u64::MAX, hash(mesh.id())],
    }
}

fn rep_key(world: &World, e: Entity) -> Option<RepKey> {
    let er = world.get_entity(e).ok()?;
    let mesh = er.get::<Mesh3d>()?;
    let mat = er.get::<MeshMaterial3d<StandardMaterial>>()?;
    let meshes = world.resource::<Assets<Mesh>>();
    let layout = layout_of(meshes, &mesh.0);
    Some(RepKey { material: mat.0.id(), layout, lightmap: er.contains::<bevy::pbr::Lightmap>(), shadows: !er.contains::<bevy::light::NotShadowCaster>() })
}

fn warm_entity(world: &mut World, e: Entity) {
    let Ok(mut ec) = world.get_entity_mut(e) else { return };
    let w = Warming {
        layers: ec.get::<RenderLayers>().cloned(),
        vis: ec.get::<Visibility>().copied(),
        range: ec.get::<VisibilityRange>().cloned(),
        had_no_cull: ec.contains::<NoFrustumCulling>(),
        frames: HOLD_FRAMES,
    };
    let layers = w.layers.clone().unwrap_or_default().with(LOAD_LAYER);
    // A range that covers everything, crossfading when the real one does: crossfading ranges
    // are part of the pipeline key (`VISIBILITY_RANGE_DITHER`).
    let range = w.range.as_ref().map(|r| {
        if r.is_abrupt() {
            VisibilityRange::abrupt(0.0, 1.0e6)
        } else {
            VisibilityRange { start_margin: 0.0..0.0, end_margin: 1.0e6..1.0e6 + 1.0, use_aabb: r.use_aabb }
        }
    });
    ec.insert((layers, Visibility::Visible, NoFrustumCulling, w));
    if let Some(r) = range {
        ec.insert(r);
    }
}

fn unwarm_entity(world: &mut World, e: Entity) {
    let Ok(mut ec) = world.get_entity_mut(e) else { return };
    let Some(w) = ec.take::<Warming>() else { return };
    match w.layers {
        Some(l) => ec.insert(l),
        None => ec.remove::<RenderLayers>(),
    };
    match w.vis {
        Some(v) => ec.insert(v),
        None => ec.remove::<Visibility>(),
    };
    if let Some(r) = w.range {
        ec.insert(r);
    }
    if !w.had_no_cull {
        ec.remove::<NoFrustumCulling>();
    }
}

/// Every 3D camera, light and light probe sees only [`LOAD_LAYER`] (probes too: whether a view
/// sees an irradiance volume or reflection probe is part of every material's pipeline key) (run each frame: the map's lights and the
/// round's camera appear after the load starts). Until the Shaders stage nothing is on that
/// layer, so the view behind the cover draws nothing and compiles nothing at once.
fn hold_views(world: &mut World) {
    let mut q = world.query_filtered::<(Entity, Option<&RenderLayers>), (Or<(With<Camera3d>, With<PointLight>, With<SpotLight>, With<DirectionalLight>, With<bevy::light::LightProbe>)>, Without<ViewHeld>)>();
    let list: Vec<(Entity, Option<RenderLayers>)> = q.iter(world).map(|(e, l)| (e, l.cloned())).collect();
    for (e, layers) in list {
        world.entity_mut(e).insert((ViewHeld(layers), RenderLayers::layer(LOAD_LAYER)));
    }
}

fn restore_views(world: &mut World) {
    let held: Vec<Entity> = world.query_filtered::<Entity, With<ViewHeld>>().iter(world).collect();
    for e in held {
        let Ok(mut ec) = world.get_entity_mut(e) else { continue };
        let Some(ViewHeld(layers)) = ec.take::<ViewHeld>() else { continue };
        match layers {
            Some(l) => ec.insert(l),
            None => ec.remove::<RenderLayers>(),
        };
    }
}

fn restore_all(world: &mut World) {
    restore_views(world);
    let warming: Vec<Entity> = world.query_filtered::<Entity, With<Warming>>().iter(world).collect();
    for e in warming {
        unwarm_entity(world, e);
    }
}

/// Map stage: pieces exist, scenes spawned, material textures loaded (else what's missing).
fn map_ready(world: &mut World) -> Result<(), String> {
    if world.query_filtered::<(), With<engine::gfx::MainCamera>>().iter(world).next().is_none() {
        return Err("no main camera".into());
    }
    if world.query_filtered::<(), With<ArenaPiece>>().iter(world).next().is_none() {
        return Err("no map pieces".into());
    }
    let server = world.resource::<AssetServer>().clone();
    let done = |id: bevy::asset::UntypedAssetId| match server.get_load_state(id) {
        None => true,
        Some(s) => s.is_loaded() || s.is_failed(),
    };
    let mut scenes = world.query::<(&WorldAssetRoot, Option<&Children>)>();
    for (root, children) in scenes.iter(world) {
        let id = root.0.id().untyped();
        let failed = server.get_load_state(id).is_some_and(|s| s.is_failed());
        if !failed && (!server.is_loaded_with_dependencies(id) || children.is_none()) {
            return Err(format!("scene {:?} ({:?}, spawned: {})", server.get_path(id), server.get_load_state(id), children.is_some()));
        }
    }
    for (_, m) in world.resource::<Assets<StandardMaterial>>().iter() {
        let maps = [&m.base_color_texture, &m.normal_map_texture, &m.metallic_roughness_texture, &m.occlusion_texture, &m.emissive_texture];
        if let Some(h) = maps.into_iter().flatten().find(|h| !done(h.id().untyped())) {
            return Err(format!("texture {:?} ({:?})", server.get_path(h.id()), server.get_load_state(h.id())));
        }
    }
    Ok(())
}

/// Bake stage: no bake wanted, or this level's bake loaded and every mesh matched against it.
fn bake_ready(world: &mut World) -> bool {
    let Some(level) = world.get_resource::<BakeLevel>().map(|l| l.id.clone()) else { return true };
    if world.get_resource::<engine::bake::BakeConfig>().is_some_and(|c| !c.apply) {
        return true;
    }
    if world.get_resource::<LoadedBake>().is_none_or(|b| b.level != level) {
        return false;
    }
    let mut q = world.query_filtered::<Option<&DetailTier>, (With<Mesh3d>, With<MeshMaterial3d<StandardMaterial>>, Without<BakeApplied>)>();
    q.iter(world).all(|t| t == Some(&DetailTier::Far))
}

fn set_stage(world: &mut World, stage: Stage) {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let mut l = world.resource_mut::<Loading>();
    info!("loading: {:?} -> {stage:?} after {:.2} s", l.stage, now - l.stage_started);
    l.stage = stage;
    l.stage_started = now;
    l.quiet = 0;
    l.stage_frames = 0;
}

/// The loading state machine (exclusive: it inserts and removes components on many entities).
fn drive(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let dt = world.resource::<Time<Real>>().delta_secs();
    // After a load: the worst frame of the first seconds of play.
    if let Some(until) = world.resource::<Loading>().after_until {
        let mut l = world.resource_mut::<Loading>();
        l.after_worst = l.after_worst.max(dt);
        if now >= until {
            l.after_until = None;
            let worst = l.after_worst;
            info!("loading: worst frame in the {AFTER_S} s after loading: {:.0} ms", worst * 1000.0);
            #[cfg(feature = "dev")]
            if let Some(log) = world.get_resource::<engine::devtools::SessionLog>() {
                log.event("loading.after", serde_json::json!({ "worst_ms": worst * 1000.0, "seconds": AFTER_S }));
            }
        }
    }
    if !world.resource::<Loading>().active {
        return;
    }
    world.write_message(RequestRedraw);
    {
        let mut l = world.resource_mut::<Loading>();
        l.frames += 1;
        // The first frame's delta covers startup; count the rest.
        if l.frames > 1 {
            l.worst = l.worst.max(dt);
        }
        if dt > 1.0 {
            info!("loading: {:.2} s frame in {:?} (batch {}, {} / {} drawn)", dt, l.stage, l.batch, l.reps_done, l.reps_total);
        }
    }
    let (stage, stage_started) = {
        let mut l = world.resource_mut::<Loading>();
        l.stage_frames += 1;
        (l.stage, l.stage_started)
    };
    let timed_out = now - stage_started > STAGE_TIMEOUT_S;
    if stage < Stage::Settle {
        hold_views(world);
    }
    match stage {
        Stage::Map => {
            let n = world.query_filtered::<(), With<Mesh3d>>().iter(world).count();
            let ready = map_ready(world);
            let mut l = world.resource_mut::<Loading>();
            if n != l.meshes || ready.is_err() {
                l.meshes = n;
                l.steady = 0;
            } else {
                l.steady += 1;
            }
            if l.steady >= 3 || timed_out {
                if timed_out {
                    warn!("loading: map stage timed out waiting for {}", ready.err().unwrap_or_else(|| format!("the mesh count to settle ({n})")));
                }
                set_stage(world, Stage::Bake);
            }
        }
        Stage::Bake => {
            if bake_ready(world) || timed_out {
                if timed_out {
                    warn!("loading: bake stage timed out");
                }
                let reps = collect_reps(world);
                let total = world.resource::<PipeStats>().total();
                let mut l = world.resource_mut::<Loading>();
                l.reps_total = reps.len();
                l.queue = reps;
                l.pipes_at_shaders = total;
                set_stage(world, Stage::Shaders);
            }
        }
        Stage::Shaders => {
            // Retire the representatives that have been drawn long enough.
            let mut held = world.query::<(Entity, &mut Warming)>();
            let mut done = Vec::new();
            for (e, mut w) in held.iter_mut(world) {
                if w.frames == 0 {
                    done.push(e);
                } else {
                    w.frames -= 1;
                }
            }
            if !done.is_empty() {
                world.resource_mut::<Loading>().stage_started = now;
            }
            for e in done {
                if let Some(k) = rep_key(world, e) {
                    world.resource_mut::<Warmed>().0.insert(k);
                }
                unwarm_entity(world, e);
                world.resource_mut::<Loading>().reps_done += 1;
            }
            // Size the batch from the measured cost per representative (the last frames hold
            // the last batch's compiles): on a cold cache one at a time, on a warm one dozens.
            let next: Vec<Entity> = {
                let mut l = world.resource_mut::<Loading>();
                if l.last_batch > 0 {
                    let per = dt / l.last_batch as f32;
                    l.cost = if l.cost > 0.0 { l.cost * 0.5 + per * 0.5 } else { per };
                }
                l.batch = ((TARGET_FRAME_S / l.cost.max(1e-4)) as usize).clamp(1, 256);
                let take = l.batch.min(l.queue.len());
                l.last_batch = take;
                l.queue.drain(..take).collect()
            };
            for e in next {
                warm_entity(world, e);
            }
            let left = world.resource::<Loading>().queue.len() + world.query_filtered::<(), With<Warming>>().iter(world).count();
            if left == 0 || timed_out {
                if timed_out {
                    warn!("loading: shader stage made no progress for {STAGE_TIMEOUT_S} s");
                    restore_all(world);
                }
                restore_views(world);
                let p = world.resource::<PipeStats>().total();
                world.resource_mut::<Loading>().last_total = p;
                info!("loading: {} pipelines after the representatives", p);
                // Time runs again from here (gameplay stays held by `not_paused`): the shot
                // effects play hidden in the floor in front of the camera and their particles
                // need time to emit.
                resume_time(world);
                world.write_message(WarmShots);
                // (The engine's `WarmPipelines` quads are not sent: they compile every material
                // again on a quad layout that real meshes don't use, ~20 s more on a cold cache.)
                set_stage(world, Stage::Settle);
            }
        }
        Stage::Settle => {
            let p = world.resource::<PipeStats>().clone();
            let (total, waiting) = (p.total(), p.waiting());
            let mut l = world.resource_mut::<Loading>();
            if total == l.last_total && waiting == 0 {
                l.quiet += 1;
            } else {
                l.quiet = 0;
            }
            l.last_total = total;
            let min_show = if l.kind == Kind::Round { 0.0 } else { MIN_SHOW_S };
            // The engine's warm-up draws for 3 frames; wait it out, then for quiet.
            let quiet = l.stage_frames > 4 && l.quiet >= QUIET_FRAMES;
            // A practice session has no freeze time for the quality governor to judge in: it
            // gets [`PRACTICE_MEASURE_S`] of the real view behind the cover, once compiles are
            // done, and picks the quality there (`quality::hold_in_rounds`).
            let practice = world.get_resource::<crate::practice::Practice>().is_some_and(|p| p.on);
            let mut l = world.resource_mut::<Loading>();
            if quiet && practice && l.kind == Kind::Match && l.measure_until.is_none() {
                l.measure_until = Some(now + PRACTICE_MEASURE_S);
            }
            let measured = l.measure_until.is_none_or(|t| now >= t);
            if (quiet && measured && now - l.started >= min_show) || timed_out {
                finish(world, now);
            }
        }
        Stage::Fade => {
            let mut l = world.resource_mut::<Loading>();
            l.fade += dt.min(0.05);
            if l.fade >= FADE_S {
                l.active = false;
                l.after_until = Some(now + AFTER_S);
                l.after_worst = 0.0;
                world.resource::<PipeStats>().0.watch.store(false, Ordering::Relaxed);
                let covers: Vec<Entity> = world.query_filtered::<Entity, With<Cover>>().iter(world).collect();
                for e in covers {
                    world.entity_mut(e).despawn();
                }
            }
        }
    }
}

/// Unpause virtual and physics time if the load paused them.
fn resume_time(world: &mut World) {
    if !std::mem::take(&mut world.resource_mut::<Loading>().paused_time) {
        return;
    }
    world.resource_mut::<Time<Virtual>>().unpause();
    if let Some(mut t) = world.get_resource_mut::<Time<avian3d::prelude::Physics>>() {
        use avian3d::prelude::PhysicsTime;
        t.unpause();
    }
}

fn finish(world: &mut World, now: f64) {
    let p = world.resource::<PipeStats>().clone();
    let (kind, secs, worst, reps, at_shaders) = {
        let l = world.resource::<Loading>();
        (l.kind, now - l.started, l.worst, l.reps_total, l.pipes_at_shaders)
    };
    info!(
        "loading: {kind:?} done in {secs:.2} s: {} pipelines ({} new), {reps} representatives, worst frame {:.0} ms",
        p.total(),
        p.total().saturating_sub(at_shaders),
        worst * 1000.0
    );
    #[cfg(feature = "dev")]
    if let Some(log) = world.get_resource::<engine::devtools::SessionLog>() {
        log.event(
            "loading",
            serde_json::json!({ "kind": format!("{kind:?}"), "seconds": secs, "pipelines": p.total(), "new_pipelines": p.total().saturating_sub(at_shaders), "representatives": reps, "worst_ms": worst * 1000.0 }),
        );
    }
    resume_time(world);
    set_stage(world, Stage::Fade);
    world.write_message(Loaded(kind));
}

fn draw_cover(
    loading: Res<Loading>,
    pipes: Res<PipeStats>,
    time: Res<Time<Real>>,
    mut cover: Query<&mut BackgroundColor, (With<Cover>, Without<CoverBar>, Without<CoverDot>)>,
    mut panel: Query<&mut Visibility, With<CoverPanel>>,
    mut bar: Query<&mut Node, With<CoverBar>>,
    mut steps: Query<(&CoverStep, &mut TextColor), (Without<CoverTag>, Without<CoverCount>, Without<CoverNote>)>,
    mut tags: Query<(&CoverTag, &mut Text, &mut TextColor), (Without<CoverStep>, Without<CoverCount>, Without<CoverNote>)>,
    mut count: Query<&mut Text, (With<CoverCount>, Without<CoverTag>, Without<CoverNote>)>,
    mut note: Query<&mut Text, (With<CoverNote>, Without<CoverTag>, Without<CoverCount>)>,
    mut dot: Query<&mut BackgroundColor, (With<CoverDot>, Without<Cover>)>,
) {
    if !loading.active {
        return;
    }
    if loading.stage == Stage::Fade {
        let a = 1.0 - (loading.fade / FADE_S).clamp(0.0, 1.0);
        for mut bg in &mut cover {
            bg.0 = Color::BLACK.with_alpha(a);
        }
        for mut v in &mut panel {
            *v = Visibility::Hidden;
        }
        return;
    }
    for mut n in &mut bar {
        n.width = Val::Percent(loading.progress() * 100.0);
    }
    let blink = (time.elapsed_secs() * 3.0).fract() < 0.5;
    for (s, mut c) in &mut steps {
        c.0 = if s.0 < loading.stage { TEXT_MUTED } else if s.0 == loading.stage { TEXT } else { TEXT_DIM };
    }
    for (tag, mut t, mut c) in &mut tags {
        let (want, color) = if tag.0 < loading.stage {
            ("OK", FRIENDLY)
        } else if tag.0 == loading.stage {
            (if blink { "··" } else { "" }, ACCENT)
        } else {
            ("", TEXT_DIM)
        };
        if t.0 != want {
            t.0 = want.to_string();
        }
        c.0 = color;
    }
    // A long load is the shader cache filling (first launch, new driver, new build).
    let slow = time.elapsed_secs_f64() - loading.started > 4.0 && loading.stage <= Stage::Shaders;
    for mut t in &mut note {
        let want = if slow { "First run on this Mac: compiling shaders. Later loads are quick." } else { "" };
        if t.0 != want {
            t.0 = want.to_string();
        }
    }
    let ready = pipes.ready().saturating_sub(loading.pipes_at_shaders);
    let estimate = if loading.reps_done > 0 && loading.reps_total > loading.reps_done {
        let per = ready as f32 / loading.reps_done as f32;
        ready + (per * (loading.reps_total - loading.reps_done) as f32).round() as u32
    } else {
        ready
    };
    for mut t in &mut count {
        let want = match loading.stage {
            Stage::Map | Stage::Bake => String::new(),
            Stage::Shaders if loading.reps_total > 0 => format!("{ready} / ~{estimate} COMPILED"),
            _ => format!("{} READY", pipes.ready()),
        };
        if t.0 != want {
            t.0 = want;
        }
    }
    let on = (time.elapsed_secs() * 1.6).fract() < 0.55;
    for mut d in &mut dot {
        d.0 = Color::srgb(0.95, 0.12, 0.1).with_alpha(if on { 1.0 } else { 0.15 });
    }
}

fn zero_look(mut look: ResMut<engine::input::LookInput>) {
    *look = default();
}

fn zero_move(mut q: Query<&mut engine::fps::FpsInput>) {
    for mut i in &mut q {
        *i = default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_monotonic_through_the_stages() {
        let mut l = Loading { active: true, reps_total: 10, ..default() };
        let mut last = -1.0;
        for (stage, done) in [(Stage::Map, 0), (Stage::Bake, 0), (Stage::Shaders, 0), (Stage::Shaders, 5), (Stage::Shaders, 10), (Stage::Settle, 10), (Stage::Fade, 10)] {
            l.stage = stage;
            l.reps_done = done;
            let p = l.progress();
            assert!(p >= last && p <= 1.0, "{stage:?} {done}: {p} < {last}");
            last = p;
        }
        assert!(l.holds() == false && Loading { active: true, ..default() }.holds());
    }

    #[test]
    fn full_screen_for_new_sessions_short_cover_for_respawns() {
        use brain::arena::MapId::*;
        // A match: round 1 full, later rounds short, a map change full.
        assert_eq!(entry_kind(None, (Foundry, false), 1), Kind::Match);
        assert_eq!(entry_kind(Some((Foundry, false)), (Foundry, false), 2), Kind::Round);
        assert_eq!(entry_kind(Some((Foundry, false)), (Office, false), 3), Kind::Match);
        // Practice (no rounds, the round counter stays put): session start full, respawns short.
        assert_eq!(entry_kind(Some((Foundry, false)), (Foundry, true), 0), Kind::Match);
        assert_eq!(entry_kind(Some((Foundry, true)), (Foundry, true), 0), Kind::Round);
        // Back to a match from practice: full.
        assert_eq!(entry_kind(Some((Foundry, true)), (Foundry, false), 1), Kind::Match);
    }
}
