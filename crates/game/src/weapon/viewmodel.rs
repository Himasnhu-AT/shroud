//! First-person guns: one model per [`WeaponKind`] under a pivot on the camera, shown for the
//! gun in hand, and their procedural animation (draw / holster, reloads with the magazine
//! coming out, pump and bolt cycles, inspect, bob, look sway, landing dip, sprint pose, recoil
//! kick + muzzle rise, ADS to the sight line, pull-back near walls) plus the muzzle flash.
//!
//! Bodycam mode (`bodycam`): the root turns with the free aim (the gun moves on screen inside
//! the deadzone), a 6-DOF spring rig (spec §3.1) takes the recoil impulses and the wall
//! compression (high-ready fold), and the barrel's camera-space direction goes to
//! [`MuzzleCam`] so shots leave the muzzle along the barrel.
//!
//! The viewmodel keeps its own field of view: its root is scaled by
//! `tan(fov/2) / tan(VIEWMODEL_FOV/2)` in camera x/y, so the gun looks the same at any FOV
//! setting and does not balloon when the camera zooms for ADS.

use avian3d::prelude::*;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use engine::fps::{FpsLook, FpsMotor};
use engine::gfx::MainCamera;
use engine::settings::GraphicsQuality;

use super::arsenal::{HOLSTER_S, INSPECT_S, Phase, ReloadKind, Weapon};
use super::{GunEvent, WeaponKind, WeaponSet, approach, ease};
use crate::assets::AssetsDir;
use crate::gfx::FxAssets;
use crate::player::Player;
use crate::state::has_visuals;

/// The field of view the viewmodels were posed for.
pub const VIEWMODEL_FOV_DEG: f32 = 80.0;
/// Muzzle flash sprites and light: 45 ms.
const FLASH_TIME: f32 = 0.045;
/// Bodycam recoil: the spec's impulses (m/s, rad/s) times this, scaled per gun by its kick.
/// At the spec's raw numbers the rig moves under a centimetre per shot.
const RIG_GAIN: f32 = 5.0;
/// Bodycam wall compression: the barrel ray starts at the camera; the body keeps walls at least
/// `BODY_CLEARANCE` away, the gun folds over the next `BARREL_LEN` (spec: 0.65 m).
const BODY_CLEARANCE: f32 = 0.35;
const BARREL_LEN: f32 = 0.65;

pub(super) fn build(app: &mut App) {
    app.init_resource::<MuzzleCam>()
        .init_resource::<MuzzleShadows>()
        .add_systems(PreStartup, |dir: Option<Res<AssetsDir>>| {
            if let Some(dir) = dir {
                resolve_gun_models(&dir);
            }
        })
        .add_systems(PostUpdate, apply_gun_lods.run_if(has_visuals))
        .add_systems(PostStartup, spawn_viewmodel.run_if(has_visuals))
        .add_systems(
            Update,
            (find_mag_parts, show_held_gun, animate_viewmodel, muzzle_light_shadows, viewmodel_no_shadows).chain().after(WeaponSet).run_if(has_visuals),
        );
    #[cfg(feature = "dev")]
    {
        use engine::devtools::{AddControlCommand, params};
        app.add_control_command(
            "muzzle.shadows",
            "Muzzle-flash light shadows: force on/off (A/B perf checks), or omit `on` for the quality rule",
            params(&[("on", "boolean", "force (omit: automatic)")]),
            |world: &mut World, args| {
                let force = args.get("on").and_then(|v| v.as_bool());
                world.resource_mut::<MuzzleShadows>().force = force;
                Ok(serde_json::json!({ "force": force }))
            },
        );
    }
}

/// Override for the muzzle light's shadows (dev A/B); None = [`muzzle_shadows_allowed`].
#[derive(Resource, Default)]
pub struct MuzzleShadows {
    pub force: Option<bool>,
}

/// The muzzle light casts shadows on High and Ultra while the render governor is not cutting
/// game levels. A shadowed point light is six extra shadow views (512 cube) for the frames it
/// is lit; on High, full-auto, it moved p95 by -0.2 ms (Office) and +0.3 ms (Sirocco), p50 by
/// +0.2 / +0.7 ms (`scripts/muzzle-shadows-perf.json`).
pub fn muzzle_shadows_allowed(q: GraphicsQuality, governor_level: u8) -> bool {
    matches!(q, GraphicsQuality::High | GraphicsQuality::Ultra) && governor_level == 0
}

/// Shadows on the muzzle light only for the frames it is lit (an unlit shadowed light would
/// still render its cube every frame).
fn muzzle_light_shadows(
    settings: Option<Res<engine::settings::CoreSettings>>,
    governor: Option<Res<engine::render_budget::GovernorState>>,
    force: Res<MuzzleShadows>,
    mut lights: Query<&mut PointLight, With<MuzzleFlash>>,
) {
    let allowed = force.force.unwrap_or_else(|| {
        muzzle_shadows_allowed(settings.map(|s| s.quality).unwrap_or_default(), governor.map_or(0, |g| g.game_level))
    });
    for mut l in &mut lights {
        let want = allowed && l.intensity > 0.0;
        if l.shadow_maps_enabled != want {
            l.shadow_maps_enabled = want;
        }
    }
}

/// Where the muzzle of the gun in hand appears and where its barrel points, in camera space
/// (tracer and flash origin; bodycam shots).
#[derive(Resource, Clone, Copy, Debug)]
pub struct MuzzleCam(pub Vec3, pub Vec3);

impl Default for MuzzleCam {
    fn default() -> Self {
        let f = fit(WeaponKind::Rifle);
        MuzzleCam(f.hip + f.muzzle, Vec3::NEG_Z)
    }
}

/// How a gun's model sits in the hands. All in metres, camera space (x right, y up, -z ahead).
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    /// The high-detail model (first person, pickups near the player).
    pub path: &'static str,
    /// The low-detail model (enemy hands, pickups further away): a separate file in the same
    /// frame, else `path`.
    pub low: &'static str,
    pub scale: f32,
    /// Model origin relative to the pivot (after `yaw`).
    pub offset: Vec3,
    /// Model rotation about Y (0: the model's barrel already points along -Z).
    pub yaw: f32,
    /// Muzzle relative to the pivot.
    pub muzzle: Vec3,
    /// Pivot position at the hip.
    pub hip: Vec3,
    /// Pivot position aimed (sight line on the camera axis).
    pub ads: Vec3,
    /// Recoil impulse on the kick spring.
    pub kick: f32,
    pub flash: f32,
    /// Primitive fallback: body size.
    pub body: Vec3,
    /// Model-space centre of the model's bounds (pickups lie centred on it).
    pub center: Vec3,
    /// An optic model mounted on the gun: path and model-space position (the rail's
    /// `Attach_Scope` node).
    pub optic: Option<(&'static str, Vec3)>,
    /// The support hand relative to the pivot (the firing hand is the pivot).
    pub support: Vec3,
    /// Pistols: both hands on the grip.
    pub pistol: bool,
    /// Metres per mesh unit of the high model (photoscan tile size on box-projected UVs).
    pub mesh_m: f32,
    /// Metres per mesh unit of the low model.
    pub low_mesh_m: f32,
    /// The model brings its own textured materials (a downloaded pack): no flat finish swap.
    pub own_materials: bool,
    /// Pack material scales (base colour, roughness); (1, 1) = untouched.
    pub pack_tune: (f32, f32),
}

/// Detail tier of a gun model. First person is always [`Detail::High`]; enemies carry
/// [`Detail::Low`]; pickups and enemies within arm's reach cross-fade between the two by distance
/// ([`GunLod`]), so taking a gun off the floor never pops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    High,
    Low,
}

/// One gun model and where its parts are, in its own model space (units after `scale` are
/// metres, barrel toward -Z after `yaw_deg`). The built-in table ([`builtin_models`]) is the CC0
/// set; `assets/models/weapons/guns.json` (a list of these) adds models, e.g. a downloaded
/// high-detail pack, and wins for its kind when its `high` file exists.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GunAsset {
    /// Gun kind (`parse_kind`: "rifle", "Kestrel", "smg" ...).
    pub kind: String,
    pub high: String,
    /// Separate low-detail file in the same frame (a pack's own LOD, or `make-gun-lods.sh`).
    #[serde(default)]
    pub low: Option<String>,
    #[serde(default = "one")]
    pub scale: f32,
    /// Turn about +Y (degrees, right-handed) that brings the barrel onto -Z: a barrel along -X
    /// takes -90, along +X 90, along +Z 180.
    #[serde(default)]
    pub yaw_deg: f32,
    /// Metres per mesh unit of the high file (1 for metre meshes; Pichuliru's rigs: 100).
    #[serde(default = "one")]
    pub mesh_m: f32,
    /// Keep the file's own materials (textured packs) instead of the flat-colour finishes.
    #[serde(default)]
    pub own_materials: bool,
    /// Firing hand on the grip (becomes the pivot).
    pub hand: [f32; 3],
    /// Front of the barrel.
    pub muzzle: [f32; 3],
    /// Height of the sight line (iron sights or the optic's axis).
    pub sight: f32,
    pub center: [f32; 3],
    /// Support hand (handguard / pump / under the firing hand on pistols).
    pub support: [f32; 3],
    /// Mounted optic: its file and model-space position.
    #[serde(default)]
    pub optic: Option<(String, [f32; 3])>,
    /// `own_materials` packs only: scale the pack's base colour (and so a metal's reflectance)
    /// and its roughness, for a finish that stays plausible under the game's lights (see
    /// `finish::PackFinish`). 1 = the file as it is.
    #[serde(default = "one")]
    pub albedo: f32,
    #[serde(default = "one")]
    pub roughness: f32,
}

fn one() -> f32 {
    1.0
}

/// Per-kind handling (not the model): hip pose, ADS depth, kick, flash, fallback size.
struct Handling {
    hip: Vec3,
    ads_z: f32,
    kick: f32,
    flash: f32,
    body: Vec3,
}

fn handling(kind: WeaponKind) -> Handling {
    const PISTOL_HIP: Vec3 = Vec3::new(0.13, -0.15, -0.4);
    const RIFLE_HIP: Vec3 = Vec3::new(0.17, -0.2, -0.34);
    let h = |hip, ads_z, kick, flash, body| Handling { hip, ads_z, kick, flash, body };
    match kind {
        WeaponKind::Pistol => h(PISTOL_HIP, -0.34, 22.0, 0.7, Vec3::new(0.03, 0.13, 0.19)),
        WeaponKind::HeavyPistol => h(PISTOL_HIP, -0.34, 40.0, 1.0, Vec3::new(0.03, 0.15, 0.21)),
        WeaponKind::Smg => h(Vec3::new(0.16, -0.19, -0.34), -0.26, 14.0, 0.6, Vec3::new(0.046, 0.12, 0.45)),
        WeaponKind::Rifle => h(RIFLE_HIP, -0.3, 30.0, 1.0, Vec3::new(0.05, 0.12, 0.65)),
        WeaponKind::BattleRifle => h(RIFLE_HIP, -0.3, 38.0, 1.1, Vec3::new(0.06, 0.13, 0.7)),
        WeaponKind::Shotgun => h(RIFLE_HIP, -0.28, 55.0, 1.5, Vec3::new(0.046, 0.1, 0.58)),
        WeaponKind::Marksman => h(RIFLE_HIP, -0.24, 60.0, 1.3, Vec3::new(0.054, 0.13, 0.84)),
        WeaponKind::Sniper => h(Vec3::new(0.18, -0.21, -0.31), -0.24, 75.0, 1.5, Vec3::new(0.06, 0.13, 0.96)),
    }
}

/// The CC0 set (Pichuliru, poly.pizza): real-world metres, barrel toward -Z, stock toward +Z,
/// measured from each GLB's rest-pose bounds and `Attach_*` nodes. Low tiers are made from these
/// by `scripts/make-gun-lods.sh` (the engine's asset-pipe) into `models/weapons/lod/`.
pub fn builtin_models() -> Vec<GunAsset> {
    let g = |kind: &str, file: &str, scale: f32, hand: [f32; 3], muzzle: [f32; 3], sight: f32, center: [f32; 3], support: [f32; 3], optic: Option<(&str, [f32; 3])>| GunAsset {
        kind: kind.into(),
        high: format!("models/weapons/{file}.glb"),
        low: Some(format!("models/weapons/lod/{file}.lod1.glb")),
        scale,
        yaw_deg: 0.0,
        mesh_m: 100.0,
        own_materials: false,
        hand,
        muzzle,
        sight,
        center,
        support,
        optic: optic.map(|(p, at)| (format!("models/weapons/{p}.glb"), at)),
        albedo: 1.0,
        roughness: 1.0,
    };
    vec![
        g("pistol", "pistol", 1.0, [0.0, -0.03, 0.05], [0.0, 0.025, -0.117], 0.042, [0.0, -0.025, -0.02], [-0.022, -0.045, 0.035], None),
        g("heavypistol", "heavy_pistol", 1.05, [0.0, -0.035, 0.05], [0.0, 0.024, -0.127], 0.04, [0.0, -0.036, -0.024], [-0.024, -0.05, 0.035], None),
        g("smg", "smg", 1.0, [0.0, -0.045, 0.085], [0.0, 0.017, -0.222], 0.064, [0.0, -0.057, 0.06], [0.0, -0.005, -0.13], None),
        g("rifle", "rifle", 1.0, [0.0, -0.05, 0.11], [0.0, 0.016, -0.339], 0.083, [0.0, -0.05, 0.01], [0.0, -0.012, -0.17], Some(("optic_magnified", [0.0, 0.041, 0.011]))),
        g("battlerifle", "battle_rifle", 1.0, [0.0, -0.06, 0.125], [0.0, 0.006, -0.34], 0.074, [0.0, -0.06, 0.02], [0.0, -0.01, -0.16], None),
        g("shotgun", "shotgun", 1.0, [0.0, -0.03, 0.175], [0.0, 0.03, -0.277], 0.052, [0.0, -0.01, 0.02], [0.0, -0.01, -0.087], None),
        g("marksman", "marksman", 1.0, [0.0, -0.06, 0.125], [0.0, 0.012, -0.452], 0.096, [0.0, -0.03, -0.03], [0.0, -0.01, -0.2], Some(("scope_short", [0.0, 0.052, 0.011]))),
        g("sniper", "sniper", 1.0, [0.0, -0.06, 0.2], [0.0, 0.015, -0.545], 0.083, [0.0, -0.02, -0.065], [0.0, -0.01, -0.26], Some(("scope_long", [0.0, 0.044, 0.011]))),
    ]
}

/// Extra gun models (local packs), relative to the assets folder.
pub const GUNS_JSON: &str = "models/weapons/guns.json";

/// The model each kind uses, chosen once at startup ([`resolve_gun_models`]); before that (tests,
/// headless tools) the built-in table.
static CHOSEN: std::sync::OnceLock<Vec<Fit>> = std::sync::OnceLock::new();

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

/// Turn a model entry into the hands' fit for `kind`. `has` says which files exist.
pub fn fit_of(kind: WeaponKind, m: &GunAsset, has: &dyn Fn(&str) -> bool) -> Fit {
    let h = handling(kind);
    let rot = Quat::from_rotation_y(m.yaw_deg.to_radians());
    let v = |a: [f32; 3]| rot * Vec3::from(a);
    let hand = v(m.hand);
    let low = m.low.as_deref().filter(|l| has(l));
    Fit {
        path: leak(&m.high),
        low: leak(low.unwrap_or(&m.high)),
        scale: m.scale,
        offset: -hand * m.scale,
        yaw: m.yaw_deg.to_radians(),
        muzzle: (v(m.muzzle) - hand) * m.scale,
        hip: h.hip,
        ads: Vec3::new(0.0, -(m.sight - hand.y) * m.scale, h.ads_z),
        kick: h.kick,
        flash: h.flash,
        body: h.body,
        center: v(m.center),
        optic: m.optic.as_ref().map(|(p, at)| (leak(p), v(*at))),
        support: (v(m.support) - hand) * m.scale,
        pistol: kind.spec().slot == brain::weapons::Slot::Secondary,
        mesh_m: m.mesh_m,
        // Generated LODs (asset-pipe) are written in metres.
        low_mesh_m: if low.is_some() && m.mesh_m != 1.0 && m.low.as_deref().is_some_and(|l| l.contains("/lod/")) { 1.0 } else { m.mesh_m },
        own_materials: m.own_materials,
        pack_tune: (m.albedo, m.roughness),
    }
}

/// Pick each kind's model: the first `guns.json` entry for the kind whose high file exists, else
/// the built-in one. Logs what it chose. Call once at startup (later calls are ignored).
pub fn resolve_gun_models(dir: &AssetsDir) {
    let extra: Vec<GunAsset> = if dir.has(GUNS_JSON) {
        match std::fs::read_to_string(dir.0.join(GUNS_JSON)).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(v) => v,
            Err(e) => {
                warn!("guns: {GUNS_JSON}: {e}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let has = |p: &str| dir.has(p);
    let _ = CHOSEN.set(choose(&extra, &has));
    for k in WeaponKind::ALL {
        let f = fit(k);
        info!("guns: {k:?} high {} low {}", f.path, f.low);
    }
}

fn choose(extra: &[GunAsset], has: &dyn Fn(&str) -> bool) -> Vec<Fit> {
    let builtin = builtin_models();
    WeaponKind::ALL
        .iter()
        .map(|&k| {
            let mine = |m: &&GunAsset| super::parse_kind(&m.kind) == Some(k);
            let m = extra.iter().filter(mine).find(|m| has(&m.high)).or_else(|| builtin.iter().find(mine)).expect("every kind has a built-in model");
            fit_of(k, m, has)
        })
        .collect()
}

/// The fit of gun `kind`'s chosen model.
pub fn fit(kind: WeaponKind) -> Fit {
    let i = WeaponKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
    match CHOSEN.get() {
        Some(v) => v[i],
        None => {
            static BUILTIN: std::sync::OnceLock<Vec<Fit>> = std::sync::OnceLock::new();
            BUILTIN.get_or_init(|| choose(&[], &|p: &str| !p.contains("/lod/")))[i]
        }
    }
}

/// On a gun model root: every mesh under it gets this `VisibilityRange` (Bevy doesn't propagate
/// it), so the high and low tiers cross-fade by distance with a dither instead of popping.
#[derive(Component, Clone)]
pub struct GunLod(pub bevy::camera::visibility::VisibilityRange);

/// Where the tiers hand over: pickups show the high model within reach (a swap is 2.4 m), enemy
/// guns only right up close; both fade over a metre and a half.
pub const PICKUP_HIGH_M: f32 = 6.0;
pub const ENEMY_HIGH_M: f32 = 4.0;
const LOD_FADE_M: f32 = 1.5;

/// The pair of ranges for a high model shown within `near` metres and its low model beyond.
pub fn tier_ranges(near: f32) -> (GunLod, GunLod) {
    use bevy::camera::visibility::VisibilityRange;
    let fade = near..near + LOD_FADE_M;
    (
        GunLod(VisibilityRange { start_margin: 0.0..0.0, end_margin: fade.clone(), use_aabb: false }),
        GunLod(VisibilityRange { start_margin: fade, end_margin: 1.0e5..1.0e5, use_aabb: false }),
    )
}

/// Spawn gun `kind`'s model at `detail` (and its optic) under `parent` with transform `tf`;
/// unless the model brings its own materials they get the gun's finish (`finish.rs`). The
/// caller checks that `fit(kind).path` exists (the low tier falls back to it).
pub fn spawn_gun_model(commands: &mut Commands, server: &AssetServer, dir: &AssetsDir, kind: WeaponKind, detail: Detail, tf: Transform, parent: Entity) -> Entity {
    let f = fit(kind);
    let (path, mesh_m) = match detail {
        Detail::High => (f.path, f.mesh_m),
        Detail::Low => (f.low, f.low_mesh_m),
    };
    let scene = |p: &'static str| server.load::<bevy::world_serialization::WorldAsset>(bevy::gltf::GltfAssetLabel::Scene(0).from_asset(p));
    let model = commands.spawn((bevy::world_serialization::WorldAssetRoot(scene(path)), tf, Visibility::default(), ChildOf(parent))).id();
    if !f.own_materials {
        commands.entity(model).insert(super::finish::GunFinish { kind, mesh_m });
    } else if f.pack_tune != (1.0, 1.0) {
        commands.entity(model).insert(super::finish::PackFinish { albedo: f.pack_tune.0, roughness: f.pack_tune.1 });
    }
    if let Some((path, at)) = f.optic
        && dir.has(path)
    {
        let optic = commands.spawn((bevy::world_serialization::WorldAssetRoot(scene(path)), Transform::from_translation(at), Visibility::default(), ChildOf(model))).id();
        if !f.own_materials {
            // The optics are separate Pichuliru meshes in the rig's units.
            commands.entity(optic).insert(super::finish::GunFinish { kind, mesh_m: 100.0 });
        }
    }
    model
}

/// Both tiers of gun `kind` under `parent`, cross-fading at `near` metres: the high model within,
/// the low beyond. A model without a separate low file is spawned once, without ranges.
pub fn spawn_gun_tiers(commands: &mut Commands, server: &AssetServer, dir: &AssetsDir, kind: WeaponKind, near: f32, tf: Transform, parent: Entity) -> [Option<Entity>; 2] {
    let f = fit(kind);
    let high = spawn_gun_model(commands, server, dir, kind, Detail::High, tf, parent);
    if f.low == f.path {
        return [Some(high), None];
    }
    let low = spawn_gun_model(commands, server, dir, kind, Detail::Low, tf, parent);
    let (h, l) = tier_ranges(near);
    commands.entity(high).insert(h);
    commands.entity(low).insert(l);
    [Some(high), Some(low)]
}

/// Copy each [`GunLod`] root's range onto the meshes under it as they appear.
fn apply_gun_lods(
    mut commands: Commands,
    added: Query<Entity, (Added<Mesh3d>, Without<bevy::camera::visibility::VisibilityRange>)>,
    parents: Query<&ChildOf>,
    lods: Query<&GunLod>,
) {
    for e in &added {
        if let Some(l) = parents.iter_ancestors(e).find_map(|a| lods.get(a).ok()) {
            commands.entity(e).insert(l.0.clone());
        }
    }
}

/// First-person arms: gloves and sleeves in the player's (attacker) colours, built from a few
/// shared primitives (no rigged arm asset yet: see TODO.md, Phase 7). Children of each gun so
/// they follow every gun animation. Rounded forearms, gloved fingers wrapped round the grip and
/// the support hand, a field watch on the left wrist.
#[derive(Clone)]
struct HandKit {
    tube: Handle<Mesh>,
    ball: Handle<Mesh>,
    /// Unit capsule: radius 1, total height 4 along Y (scale to size).
    capsule: Handle<Mesh>,
    glove: Handle<StandardMaterial>,
    knuckle: Handle<StandardMaterial>,
    sleeve: Handle<StandardMaterial>,
    cuff: Handle<StandardMaterial>,
    watch_case: Handle<StandardMaterial>,
    watch_face: Handle<StandardMaterial>,
}

impl HandKit {
    fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        HandKit {
            // Tangents: the glove and shirt materials are normal-mapped photoscans.
            tube: meshes.add(tangents(Cylinder::new(1.0, 1.0).mesh().resolution(16).segments(1).build())),
            ball: meshes.add(tangents(Sphere::new(1.0).mesh().uv(16, 10))),
            capsule: meshes.add(tangents(Capsule3d::new(1.0, 2.0).mesh().rings(2).latitudes(8).longitudes(12).build())),
            // Black tactical gloves (synthetic leather palm, rubber knuckle guard), a charcoal
            // combat shirt with its cuff, a black field watch. Textured sets replace the glove
            // and shirt when present (`glove_leather`, `sleeve_fabric`).
            glove: materials.add(StandardMaterial { base_color: Color::srgb(0.06, 0.06, 0.065), perceptual_roughness: 0.72, ..default() }),
            knuckle: materials.add(StandardMaterial { base_color: Color::srgb(0.035, 0.035, 0.04), perceptual_roughness: 0.5, ..default() }),
            sleeve: materials.add(StandardMaterial { base_color: Color::srgb(0.085, 0.088, 0.095), perceptual_roughness: 0.9, ..default() }),
            cuff: materials.add(StandardMaterial { base_color: Color::srgb(0.07, 0.072, 0.078), perceptual_roughness: 0.9, ..default() }),
            watch_case: materials.add(StandardMaterial { base_color: Color::srgb(0.05, 0.05, 0.055), metallic: 0.6, perceptual_roughness: 0.4, ..default() }),
            watch_face: materials.add(StandardMaterial {
                base_color: Color::srgb(0.02, 0.025, 0.02),
                // Faint lume so the dial reads in shade.
                emissive: LinearRgba::rgb(0.02, 0.06, 0.03),
                perceptual_roughness: 0.15,
                ..default()
            }),
        }
    }

    fn part(&self, commands: &mut Commands, gun: Entity, mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>, tf: Transform) {
        commands.spawn((Name::new("arm"), Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), tf, ChildOf(gun)));
    }

    /// A rounded segment from `a` to `b` (radius `r`): a capsule stretched between them.
    fn segment(&self, commands: &mut Commands, gun: Entity, mat: &Handle<StandardMaterial>, a: Vec3, b: Vec3, r: f32) {
        let d = b - a;
        let len = d.length().max(1e-4);
        let rot = Quat::from_rotation_arc(Vec3::Y, d / len);
        // Unit capsule is 4 tall: the caps add `r` at each end.
        let tf = Transform::from_translation((a + b) * 0.5).with_rotation(rot).with_scale(Vec3::new(r, (len + 2.0 * r) / 4.0, r));
        self.part(commands, gun, &self.capsule, mat, tf);
    }
}

/// `m` with MikkTSpace tangents (kept as is if they can't be built).
fn tangents(m: Mesh) -> Mesh {
    m.clone().with_generated_tangents().unwrap_or(m)
}

/// A forearm from `wrist` running toward `elbow` (pivot space): glove gauntlet, shirt cuff, then a
/// sleeve that widens toward the elbow. `watch`: a field watch on top of the wrist, facing
/// `outward`.
fn arm(commands: &mut Commands, k: &HandKit, gun: Entity, wrist: Vec3, elbow: Vec3, watch: Option<Vec3>) {
    let d = elbow - wrist;
    let len = d.length();
    let dir = d / len;
    let rot = Quat::from_rotation_arc(Vec3::Y, dir);
    k.segment(commands, gun, &k.glove, wrist, wrist + dir * 0.05, 0.024);
    k.part(commands, gun, &k.tube, &k.cuff, Transform::from_translation(wrist + dir * 0.06).with_rotation(rot).with_scale(Vec3::new(0.029, 0.016, 0.029)));
    let s0 = 0.066;
    let mid = wrist + dir * (s0 + (len - s0) * 0.45);
    k.segment(commands, gun, &k.sleeve, wrist + dir * s0, mid, 0.03);
    k.segment(commands, gun, &k.sleeve, mid - dir * 0.03, elbow, 0.036);
    if let Some(out) = watch {
        // Outward normal of the wrist, square to the arm.
        let n = (out - dir * out.dot(dir)).normalize_or(Vec3::Y);
        let at = wrist + dir * 0.035;
        let face_rot = Quat::from_rotation_arc(Vec3::Y, n);
        // Strap round the glove cuff, case and dial on top.
        k.part(commands, gun, &k.tube, &k.watch_case, Transform::from_translation(at).with_rotation(rot).with_scale(Vec3::new(0.0265, 0.014, 0.0265)));
        k.part(commands, gun, &k.tube, &k.watch_case, Transform::from_translation(at + n * 0.026).with_rotation(face_rot).with_scale(Vec3::new(0.014, 0.008, 0.014)));
        k.part(commands, gun, &k.tube, &k.watch_face, Transform::from_translation(at + n * 0.0302).with_rotation(face_rot).with_scale(Vec3::new(0.011, 0.001, 0.011)));
    }
}

/// Four fingers wrapped across the front of a grip (`front`: knuckle-line centre), each a
/// rounded segment, then a fingertip curling back on the far side; the knuckle guard on top.
fn fingers(commands: &mut Commands, k: &HandKit, gun: Entity, front: Vec3, sc: f32, side: f32) {
    for i in 0..4 {
        let y = front.y - i as f32 * 0.0165 * sc;
        let r = (0.0085 - i as f32 * 0.0006) * sc;
        let a = Vec3::new(front.x + side * 0.02 * sc, y, front.z + 0.012 * sc);
        let b = Vec3::new(front.x - side * 0.016 * sc, y, front.z - 0.004 * sc);
        let tip = Vec3::new(front.x - side * 0.024 * sc, y, front.z + 0.016 * sc);
        k.segment(commands, gun, &k.glove, a, b, r);
        k.segment(commands, gun, &k.glove, b, tip, r * 0.9);
    }
    // Knuckle guard over the first joints.
    k.segment(
        commands,
        gun,
        &k.knuckle,
        Vec3::new(front.x + side * 0.022 * sc, front.y + 0.004 * sc, front.z + 0.014 * sc),
        Vec3::new(front.x + side * 0.022 * sc, front.y - 0.05 * sc, front.z + 0.014 * sc),
        0.007 * sc,
    );
}

fn spawn_hands(commands: &mut Commands, k: &HandKit, f: &Fit, gun: Entity) {
    let ball = |commands: &mut Commands, at: Vec3, size: Vec3| {
        k.part(commands, gun, &k.ball, &k.glove, Transform::from_translation(at).with_scale(size));
    };
    // Firing hand: palm on the right of the grip, fingers wrapped round its front, thumb along
    // the left side above them; the forearm runs back and down out of frame.
    let sc = if f.pistol { 0.75 } else { 1.0 };
    ball(commands, Vec3::new(0.02, -0.024, 0.004) * sc, Vec3::new(0.013, 0.04, 0.036) * sc);
    fingers(commands, k, gun, Vec3::new(0.0, -0.004, -0.03) * sc, sc, 1.0);
    k.segment(commands, gun, &k.glove, Vec3::new(0.008, 0.012, 0.012) * sc, Vec3::new(-0.016, 0.01, -0.03) * sc, 0.0085 * sc);
    arm(commands, k, gun, Vec3::new(0.018, -0.055, 0.04) * sc, Vec3::new(0.2, -0.36, 0.36), None);
    let s = f.support;
    if f.pistol {
        // Cupping the firing hand from the left: palm on the grip's left, fingers round the front.
        ball(commands, s + Vec3::new(-0.01, 0.0, -0.002), Vec3::new(0.011, 0.032, 0.03));
        fingers(commands, k, gun, s + Vec3::new(0.004, -0.004, -0.03), 0.8, -1.0);
        k.segment(commands, gun, &k.glove, s + Vec3::new(-0.012, 0.016, 0.0), s + Vec3::new(-0.008, 0.02, -0.03), 0.0075);
        arm(commands, k, gun, s + Vec3::new(-0.015, -0.035, 0.035), s + Vec3::new(-0.24, -0.36, 0.3), Some(Vec3::new(-1.0, 0.6, 0.0)));
    } else {
        // Under the handguard / on the pump: palm below, fingers curled up the right side,
        // thumb along the left.
        ball(commands, s + Vec3::new(0.0, -0.032, 0.004), Vec3::new(0.032, 0.014, 0.046));
        for i in 0..4 {
            let z = 0.022 - i as f32 * 0.016;
            let r = 0.0082 - i as f32 * 0.0005;
            k.segment(commands, gun, &k.glove, s + Vec3::new(0.022, -0.034, z), s + Vec3::new(0.032, -0.008, z - 0.002), r);
            k.segment(commands, gun, &k.glove, s + Vec3::new(0.032, -0.008, z - 0.002), s + Vec3::new(0.024, 0.012, z - 0.004), r * 0.9);
        }
        k.segment(commands, gun, &k.glove, s + Vec3::new(-0.024, -0.03, 0.03), s + Vec3::new(-0.03, -0.006, -0.02), 0.009);
        arm(commands, k, gun, s + Vec3::new(-0.005, -0.045, 0.04), s + Vec3::new(-0.22, -0.34, 0.36), Some(Vec3::new(-1.0, 0.5, 0.0)));
    }
}

/// MetaHuman hands for the first-person view (`models/metahuman/hands_mh.glb`, made by
/// `scripts/make-metahuman.sh`; Epic content, so optional). Each mesh sits in its grip frame:
/// origin at the palm centre, +Y hand forward (wrist → middle knuckle), +X the palm normal
/// (towards what it holds).
pub const MH_HANDS: &str = "models/metahuman/hands_mh.glb";

struct MhHands {
    right: Handle<Mesh>,
    left: Handle<Mesh>,
    glove: Handle<StandardMaterial>,
}

/// Grip frame → gun space: the hand points from `wrist` to `palm`, palm facing `palm_normal`.
fn grip_rot(palm_normal: Vec3, wrist: Vec3, palm: Vec3) -> Quat {
    let y = (palm - wrist).normalize();
    let x = (palm_normal - y * palm_normal.dot(y)).normalize();
    Quat::from_mat3(&Mat3::from_cols(x, y, x.cross(y)))
}

fn spawn_mh_hands(commands: &mut Commands, k: &HandKit, mh: &MhHands, f: &Fit, gun: Entity) {
    let sc = if f.pistol { 0.75 } else { 1.0 };
    let put = |commands: &mut Commands, mesh: &Handle<Mesh>, at: Vec3, rot: Quat, scale: f32| {
        commands.spawn((
            Name::new("arm"),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mh.glove.clone()),
            // A touch larger than the bare hand: the glove's thickness.
            Transform::from_translation(at).with_rotation(rot).with_scale(Vec3::splat(scale * 1.03)),
            crate::meshprep::MeshPrep::Tangents,
            ChildOf(gun),
        ));
    };
    // Firing hand: palm on the right of the grip facing it, the hand rising from the wrist.
    let (palm, wrist) = (Vec3::new(0.026, -0.03, 0.012) * sc, Vec3::new(0.022, -0.06, 0.05) * sc);
    put(commands, &mh.right, palm, grip_rot(-Vec3::X, wrist, palm), sc);
    arm(commands, k, gun, wrist, Vec3::new(0.2, -0.36, 0.36), None);
    let s = f.support;
    if f.pistol {
        // Cupping the firing hand from the left.
        let (palm, wrist) = (s + Vec3::new(-0.018, -0.004, 0.0), s + Vec3::new(-0.02, -0.04, 0.04));
        put(commands, &mh.left, palm, grip_rot(Vec3::X, wrist, palm), sc);
        arm(commands, k, gun, wrist, s + Vec3::new(-0.24, -0.36, 0.3), Some(Vec3::new(-1.0, 0.6, 0.0)));
    } else {
        // Palm up under the handguard, reaching forward.
        let (palm, wrist) = (s + Vec3::new(0.0, -0.04, 0.006), s + Vec3::new(-0.01, -0.05, 0.05));
        put(commands, &mh.left, palm, grip_rot(Vec3::Y, wrist, palm), 1.0);
        arm(commands, k, gun, wrist, s + Vec3::new(-0.22, -0.34, 0.36), Some(Vec3::new(-1.0, 0.5, 0.0)));
    }
}

/// Black tactical gloves: the photoscanned leather set (`textures/glove_leather`, CC0), a fine
/// grain at the hand's size.
pub fn glove_leather(server: &AssetServer, dir: &AssetsDir, pending: &mut crate::gfx::PendingMips) -> Option<StandardMaterial> {
    // The set averages 0.009 linear; x2.2 gives a black synthetic glove (~0.02), not a void,
    // without lifting its pale creases into a web. A fine grain (10 repeats over the hand).
    crate::gfx::load_pbr(server, dir, pending, "glove_leather", Color::LinearRgba(LinearRgba::rgb(2.2, 2.2, 2.3))).map(|m| StandardMaterial {
        uv_transform: bevy::math::Affine2::from_scale(Vec2::splat(10.0)),
        perceptual_roughness: 0.85,
        reflectance: 0.4,
        ..m
    })
}

/// The combat shirt sleeves in the ripstop fabric set (charcoal, like a patrol uniform under a
/// body camera), if it is there.
pub fn sleeve_fabric(server: &AssetServer, dir: &AssetsDir, pending: &mut crate::gfx::PendingMips) -> Option<StandardMaterial> {
    let tint = crate::enemy::skin::fabric_tint(Color::srgb(0.085, 0.088, 0.095));
    crate::gfx::load_pbr(server, dir, pending, "ripstop", tint)
        .map(|m| StandardMaterial { uv_transform: bevy::math::Affine2::from_scale(Vec2::new(2.0, 3.0)), perceptual_roughness: 1.0, ..m })
}

/// The camera child that carries the viewmodel (named "ViewModel": the menu hides it outside
/// rounds). Scaled for the viewmodel FOV.
#[derive(Component)]
pub struct ViewRoot;

/// Animated pivot under [`ViewRoot`]. Holds animation state.
#[derive(Component, Default)]
pub struct ViewModel {
    kick: f32,
    kick_vel: f32,
    jitter: f32,
    sway: Vec2,
    sway_vel: Vec2,
    /// Body motion the gun lags behind (smoothed, -1..1): sideways and vertical.
    strafe: f32,
    rise: f32,
    last_look: Option<Vec2>,
    bob_phase: f32,
    bob_amp: f32,
    reload: f32,
    wall_push: f32,
    flash: f32,
    flash_scale: f32,
    /// Landing dip spring.
    land: f32,
    land_vel: f32,
    air_time: f32,
    was_grounded: bool,
    /// Short bump for mag-in / shell-in / chamber events.
    bump: f32,
    t: f32,
    /// Bodycam 6-DOF rig: translation (camera space, m) and rotation (pitch, yaw, roll).
    rig_t: Spring3,
    rig_r: Spring3,
    /// Bodycam wall compression 0..1.
    compress: f32,
}

/// Second-order spring (spec §3.1): semi-implicit Euler, substepped for stability.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring3 {
    pub value: Vec3,
    pub velocity: Vec3,
    pub target: Vec3,
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring3 {
    pub const fn new(stiffness: f32, damping: f32) -> Self {
        Spring3 { value: Vec3::ZERO, velocity: Vec3::ZERO, target: Vec3::ZERO, stiffness, damping }
    }

    pub fn add_impulse(&mut self, impulse: Vec3) {
        self.velocity += impulse;
    }

    pub fn step(&mut self, dt: f32) {
        let n = (dt / (1.0 / 240.0)).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as u32 {
            let force = (self.target - self.value) * self.stiffness - self.velocity * self.damping;
            self.velocity += force * h;
            self.value += self.velocity * h;
        }
    }
}

impl Default for Spring3 {
    fn default() -> Self {
        Spring3::new(1.0, 1.0)
    }
}

/// Look-sway spring stiffness per gun: light guns snap back, long heavy ones swing.
fn sway_stiffness(kind: WeaponKind) -> f32 {
    match kind {
        WeaponKind::Pistol | WeaponKind::HeavyPistol => 150.0,
        WeaponKind::Smg => 125.0,
        WeaponKind::Rifle | WeaponKind::BattleRifle => 100.0,
        WeaponKind::Shotgun => 90.0,
        WeaponKind::Marksman | WeaponKind::Sniper => 70.0,
    }
}

/// Spec §3.1: translation k 210 d 16, rotation k 180 d 14.
fn rig_springs() -> (Spring3, Spring3) {
    (Spring3::new(210.0, 16.0), Spring3::new(180.0, 14.0))
}

/// One gun's model (child of the pivot).
#[derive(Component)]
pub struct GunModel(pub WeaponKind);

/// A magazine mesh inside a gun model, animated out and in on reloads.
#[derive(Component)]
struct MagPart {
    rest: Transform,
}

#[derive(Component)]
struct Muzzle;

#[derive(Component)]
struct MuzzleFlash;

fn spawn_viewmodel(
    mut commands: Commands,
    cams: Query<Entity, With<MainCamera>>,
    assets_dir: Res<AssetsDir>,
    asset_server: Res<AssetServer>,
    fx: Option<Res<FxAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    pending: Option<ResMut<crate::gfx::PendingMips>>,
) {
    let Ok(cam) = cams.single() else { return };
    commands.entity(cam).insert(Visibility::default());
    let root = commands.spawn((Name::new("ViewModel"), ViewRoot, Transform::default(), Visibility::default(), ChildOf(cam))).id();
    let rifle = fit(WeaponKind::Rifle);
    let (rig_t, rig_r) = rig_springs();
    let pivot = commands
        .spawn((
            Name::new("ViewModel pivot"),
            ViewModel { rig_t, rig_r, ..default() },
            Transform::from_translation(rifle.hip),
            Visibility::default(),
            ChildOf(root),
        ))
        .id();
    let mut hands = HandKit::new(&mut meshes, &mut materials);
    let mut pending = pending;
    if let Some(p) = pending.as_deref_mut() {
        if let Some(m) = sleeve_fabric(&asset_server, &assets_dir, p) {
            hands.sleeve = materials.add(m);
            // The cuff is the same shirt, a shade darker where the glove gauntlet tucks in.
            let mut cuff = materials.get(&hands.sleeve).cloned().unwrap_or_default();
            cuff.base_color = cuff.base_color.darker(0.02);
            hands.cuff = materials.add(cuff);
        }
        if let Some(m) = glove_leather(&asset_server, &assets_dir, p) {
            hands.glove = materials.add(m);
        }
    }
    let mh = assets_dir.has(MH_HANDS).then(|| MhHands {
        right: asset_server.load(bevy::gltf::GltfAssetLabel::Primitive { mesh: 0, primitive: 0 }.from_asset(MH_HANDS)),
        left: asset_server.load(bevy::gltf::GltfAssetLabel::Primitive { mesh: 1, primitive: 0 }.from_asset(MH_HANDS)),
        // Gloved: black synthetic leather over the MetaHuman hand's shape.
        glove: hands.glove.clone(),
    });
    // Primitive fallback, shared by every gun that has no model file.
    let mut prim: Option<(Handle<StandardMaterial>, Handle<StandardMaterial>)> = None;
    for kind in WeaponKind::ALL {
        let f = fit(kind);
        let gun = commands
            .spawn((Name::new(format!("Gun {kind:?}")), GunModel(kind), Transform::default(), Visibility::Hidden, ChildOf(pivot)))
            .id();
        match &mh {
            Some(mh) => spawn_mh_hands(&mut commands, &hands, mh, &f, gun),
            None => spawn_hands(&mut commands, &hands, &f, gun),
        }
        if assets_dir.has(f.path) {
            let tf = Transform::from_translation(f.offset).with_rotation(Quat::from_rotation_y(f.yaw)).with_scale(Vec3::splat(f.scale));
            spawn_gun_model(&mut commands, &asset_server, &assets_dir, kind, Detail::High, tf, gun);
            continue;
        }
        let (dark, accent) = prim
            .get_or_insert_with(|| {
                (
                    materials.add(StandardMaterial { base_color: Color::srgb(0.12, 0.12, 0.13), perceptual_roughness: 0.5, metallic: 0.6, ..default() }),
                    materials.add(StandardMaterial { base_color: Color::srgb(0.55, 0.42, 0.22), perceptual_roughness: 0.6, ..default() }),
                )
            })
            .clone();
        let b = f.body;
        let len = -f.muzzle.z;
        let parts = [
            (Cuboid::new(b.x, b.y, b.z), Vec3::new(0.0, 0.0, -len + b.z * 0.5 + 0.06), dark.clone()),
            (Cuboid::new(b.x * 0.4, b.x * 0.4, 0.1), Vec3::new(0.0, b.y * 0.15, -len + 0.05), dark.clone()),
            (Cuboid::new(b.x * 0.8, 0.13, 0.06), Vec3::new(0.0, -0.09, 0.0), accent.clone()),
            (Cuboid::new(b.x * 0.7, 0.11, 0.07), Vec3::new(0.0, -0.08, -0.12), dark.clone()),
        ];
        for (shape, pos, mat) in parts {
            let mut e = commands.spawn((Mesh3d(meshes.add(shape)), MeshMaterial3d(mat), Transform::from_translation(pos), ChildOf(gun)));
            if pos.y < -0.075 && pos.z < -0.05 {
                e.insert((MagPart { rest: Transform::from_translation(pos) }, Name::new("magazine")));
            }
        }
    }
    let muzzle = commands.spawn((Muzzle, Transform::from_translation(rifle.muzzle), Visibility::default(), ChildOf(pivot))).id();
    commands.spawn((
        MuzzleFlash,
        PointLight { intensity: 0.0, range: 12.0, color: Color::srgb(1.0, 0.75, 0.4), shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(0.0, 0.0, -0.1),
        ChildOf(muzzle),
    ));
    let Some(fx) = fx else { return };
    // Front-on star plus side flames stretched along the barrel.
    for tf in [
        Transform::from_scale(Vec3::splat(0.2)),
        Transform::from_xyz(0.0, 0.0, -0.1)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::new(0.3, 0.12, 1.0)),
        Transform::from_xyz(0.0, 0.0, -0.1)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::new(0.3, 0.12, 1.0)),
    ] {
        commands.spawn((MuzzleFlash, Mesh3d(fx.quad.clone()), MeshMaterial3d(fx.flash.clone()), tf, Visibility::Hidden, NotShadowCaster, ChildOf(muzzle)));
    }
}

/// Magazine meshes inside loaded gun scenes (glTF node names containing "mag").
fn find_mag_parts(
    mut commands: Commands,
    added: Query<(Entity, &Name, &Transform), (Added<Name>, Without<MagPart>)>,
    parents: Query<&ChildOf>,
    guns: Query<(), With<GunModel>>,
) {
    for (e, name, tf) in &added {
        let n = name.as_str().to_ascii_lowercase();
        // "magazine" (not "Magazine Release", the button).
        if n.contains("mag") && !n.contains("release") && !n.ends_with("_end") && parents.iter_ancestors(e).any(|a| guns.contains(a)) {
            commands.entity(e).insert(MagPart { rest: *tf });
        }
    }
}

/// Show the model of the gun in hand; move the muzzle to it.
fn show_held_gun(
    player: Query<&Weapon, With<Player>>,
    mut guns: Query<(&GunModel, &mut Visibility)>,
    mut muzzle: Query<&mut Transform, With<Muzzle>>,
    mut shown: Local<Option<WeaponKind>>,
    added: Query<(), Added<GunModel>>,
) {
    let Ok(w) = player.single() else { return };
    if *shown == Some(w.kind) && added.is_empty() {
        return;
    }
    *shown = Some(w.kind);
    for (g, mut v) in &mut guns {
        let want = if g.0 == w.kind { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    for mut tf in &mut muzzle {
        tf.translation = fit(w.kind).muzzle;
    }
}

fn animate_viewmodel(
    time: Res<Time>,
    spatial: SpatialQuery,
    players: Query<(Entity, &FpsLook, &FpsMotor, Option<&Weapon>), With<Player>>,
    cams: Query<(&GlobalTransform, &Projection), With<MainCamera>>,
    mut roots: Query<&mut Transform, (With<ViewRoot>, Without<ViewModel>)>,
    mut vms: Query<(&mut ViewModel, &mut Transform, &mut Visibility), Without<ViewRoot>>,
    mut mags: Query<(&MagPart, &mut Transform, &ChildOf), (Without<ViewModel>, Without<ViewRoot>, Without<MuzzleFlash>)>,
    parents: Query<&ChildOf>,
    guns: Query<&GunModel>,
    globals: Query<&GlobalTransform>,
    mut flashes: Query<(&mut Visibility, &mut Transform, Option<&mut PointLight>), (With<MuzzleFlash>, Without<ViewModel>, Without<ViewRoot>)>,
    mut events: MessageReader<GunEvent>,
    mut muzzle_cam: ResMut<MuzzleCam>,
    aim: Res<crate::bodycam::BodycamAim>,
    grenade: Option<Res<super::throwing::GrenadeHand>>,
    bomb: Option<Res<crate::bomb::Bomb>>,
) {
    let dt = time.delta_secs().clamp(1e-4, 0.05);
    let body = aim.active;
    let Ok((player, look, motor, weapon)) = players.single() else { return };
    let Ok((mut vm, mut tf, mut vis)) = vms.single_mut() else { return };
    let Some(w) = weapon else { return };
    let f = fit(w.kind);
    vm.t += dt;
    let t = vm.t;

    // Viewmodel FOV: scale camera x/y so the gun keeps its size at any camera FOV.
    let cam = cams.single().ok();
    let fov = cam.and_then(|(_, p)| if let Projection::Perspective(pp) = p { Some(pp.fov) } else { None }).unwrap_or(VIEWMODEL_FOV_DEG.to_radians());
    let r = (fov * 0.5).tan() / (VIEWMODEL_FOV_DEG.to_radians() * 0.5).tan();
    let root_scale = Vec3::new(r, r, 1.0);
    // Bodycam: the whole viewmodel turns about the camera to where the gun aims.
    let root_rot = if body { aim.weapon_in_camera } else { Quat::IDENTITY };
    if let Ok(mut rtf) = roots.single_mut()
        && ((rtf.scale - root_scale).length_squared() > 1e-8 || rtf.rotation != root_rot)
    {
        rtf.scale = root_scale;
        rtf.rotation = root_rot;
    }

    for ev in events.read() {
        match *ev {
            GunEvent::Shot { kind } => {
                let k = fit(kind);
                if body {
                    // Spec §3.2: kick back into the chest, muzzle climb, wrist twist.
                    let s = k.kick / 30.0 * RIG_GAIN * (1.0 - 0.45 * w.ads);
                    let r = |a: f32| rand::random_range(-a..a);
                    vm.rig_t.add_impulse(Vec3::new(r(0.02), rand::random_range(0.01..0.03), 0.18) * s);
                    vm.rig_r.add_impulse(Vec3::new(0.14, r(0.06), r(0.12)) * s);
                } else {
                    vm.kick_vel += k.kick * (1.0 - 0.45 * w.ads);
                }
                vm.jitter = rand::random_range(-1.0..1.0);
                vm.flash = FLASH_TIME;
                vm.flash_scale = k.flash;
            }
            GunEvent::MagIn { .. } | GunEvent::ShellIn | GunEvent::Chamber { .. } => vm.bump = 1.0,
            GunEvent::Pump | GunEvent::Bolt => vm.bump = 0.6,
            _ => {}
        }
    }

    // Look sway: the gun lags behind look rotation on a spring, so a heavy gun swings past and
    // settles (a little overshoot) where a pistol snaps back.
    let look_now = Vec2::new(look.yaw, look.pitch);
    let rate = vm.last_look.map_or(Vec2::ZERO, |l| (look_now - l) / dt);
    vm.last_look = Some(look_now);
    let target = (rate * 0.012).clamp(Vec2::splat(-0.06), Vec2::splat(0.06));
    let stiff = sway_stiffness(w.kind);
    let damp = 2.0 * 0.72 * stiff.sqrt();
    let acc = (target - vm.sway) * stiff - vm.sway_vel * damp;
    vm.sway_vel += acc * dt;
    let sway_vel = vm.sway_vel;
    vm.sway += sway_vel * dt;
    // Body inertia: moving sideways leaves the gun behind and tilts it; falling lifts it.
    let local_v = Quat::from_rotation_y(-look.yaw) * motor.velocity;
    vm.strafe += ((local_v.x / 6.0).clamp(-1.0, 1.0) - vm.strafe) * approach(6.0, dt);
    vm.rise += ((motor.velocity.y / 6.0).clamp(-1.0, 1.0) - vm.rise) * approach(8.0, dt);

    // Bob: figure-eight while walking on the ground; bigger sprinting, tiny aimed.
    let speed = Vec2::new(motor.velocity.x, motor.velocity.z).length();
    let amp_target = if motor.grounded { (speed / 6.0).min(1.4) } else { 0.0 };
    vm.bob_amp += (amp_target - vm.bob_amp) * approach(8.0, dt);
    vm.bob_phase += dt * (speed * 1.8).max(0.0);
    let a = ease(w.ads);
    let breathe = Vec3::new((t * 0.9).sin() * 0.0015, (t * 1.7).sin() * 0.0025, 0.0);
    let bob = (Vec3::new(vm.bob_phase.cos() * 0.011, -vm.bob_phase.sin().abs() * 0.013, 0.0) * vm.bob_amp) * (1.0 - 0.85 * a) + breathe * (1.0 - 0.7 * a);

    // Landing dip: a spring kicked by the fall.
    if !motor.grounded {
        vm.air_time += dt;
    } else if !vm.was_grounded {
        vm.land_vel -= (vm.air_time * 2.2).min(1.6) * 6.0;
        vm.air_time = 0.0;
    }
    vm.was_grounded = motor.grounded;
    let land_acc = -160.0 * vm.land - 18.0 * vm.land_vel;
    vm.land_vel += land_acc * dt;
    vm.land = (vm.land + vm.land_vel * dt).clamp(-1.0, 1.0);

    // Recoil spring.
    let acc = -320.0 * vm.kick - 24.0 * vm.kick_vel;
    vm.kick_vel += acc * dt;
    vm.kick = (vm.kick + vm.kick_vel * dt).clamp(-0.5, 2.2);
    vm.jitter *= (-10.0 * dt).exp();
    vm.bump = (vm.bump - dt * 6.0).max(0.0);

    // Reload envelope (0..1 while reloading) and stage progress.
    let progress = w.reload_progress();
    let r_target = progress.is_some() as u8 as f32;
    vm.reload += (r_target - vm.reload) * approach(9.0, dt);
    let p = progress.unwrap_or(1.0);
    let shells = w.reload_kind == ReloadKind::Shells;
    let rl = vm.reload * if shells { 0.6 } else { 1.0 };
    // Magazine: out by 0.35, back in by 0.6 (mag reloads only).
    let mag_drop = if progress.is_some() && !shells {
        let out = ((p - 0.12) / 0.15).clamp(0.0, 1.0);
        let back = ((p - 0.42) / 0.16).clamp(0.0, 1.0);
        ease(out) * (1.0 - ease(back))
    } else {
        0.0
    };
    let chamber = if progress.is_some() && w.reload_kind == ReloadKind::Empty { (-((p - 0.86) / 0.04).powi(2)).exp() } else { 0.0 };

    // Draw / holster: the gun comes up from below.
    let lowered = match w.phase {
        Phase::Ready => 0.0,
        Phase::Drawing { left, total } => ease(left / total.max(1e-3)),
        Phase::Holstering { left, .. } => ease(1.0 - left / HOLSTER_S),
    };
    // Planting the bomb (or down after it): the gun goes out of the way.
    let lowered = lowered.max(bomb.map_or(0.0, |b| ease(b.lower)));
    // Pump / bolt cycle.
    let spec = w.kind.spec();
    let cyc = if w.cycling > 0.0 && spec.cycle_s > 0.0 {
        let c = 1.0 - w.cycling / spec.cycle_s;
        (c * std::f32::consts::PI).sin().max(0.0) * ((c - 0.1) / 0.15).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let bolt = matches!(w.kind, WeaponKind::Marksman | WeaponKind::Sniper);
    // Inspect: turn the gun to show its side.
    let insp = if w.inspect > 0.0 {
        let u = 1.0 - w.inspect / INSPECT_S;
        ease((u / 0.2).min(1.0)) * ease(((1.0 - u) / 0.2).min(1.0))
    } else {
        0.0
    };
    let insp_wiggle = if w.inspect > 0.0 { ((1.0 - w.inspect / INSPECT_S) * 6.0).sin() * 0.15 } else { 0.0 };
    let sprint = ease(w.sprint);

    // Pull the barrel back from walls instead of clipping through them.
    let filter = SpatialQueryFilter::from_excluded_entities([player]);
    if body {
        // Bodycam: a ray along the gun's aim; the closer the wall, the further the gun folds up
        // and back to high ready (spec §3.2, measured past the body's own clearance).
        let alpha = cam
            .and_then(|(g, _)| {
                // The aim direction, not the folded barrel (that would point over the wall).
                let dir = Dir3::new(g.rotation() * (root_rot * Vec3::NEG_Z)).ok()?;
                spatial.cast_ray(g.translation(), dir, BODY_CLEARANCE + BARREL_LEN, true, &filter).map(|h| 1.0 - (h.distance - BODY_CLEARANCE).max(0.0) / BARREL_LEN)
            })
            .unwrap_or(0.0);
        vm.compress += (alpha - vm.compress) * approach(12.0, dt);
        let c = vm.compress;
        // Spec: 0.22 m back and 65° up. The pivot is the grip, so pulling it all the way back
        // would put the handguard on the lens: back 0.12, down and right a little, turned in.
        vm.rig_t.target = Vec3::new(0.05 * c, -0.07 * c, 0.12 * c);
        vm.rig_r.target = Vec3::new(65f32.to_radians() * c, 0.2 * c, 0.3 * c);
        vm.rig_t.step(dt);
        vm.rig_r.step(dt);
        vm.wall_push = 0.0;
    } else {
        let push = cam.and_then(|(g, _)| spatial.cast_ray(g.translation(), g.forward(), 1.0, true, &filter).map(|h| (0.85 - h.distance).clamp(0.0, 0.4)));
        vm.wall_push += (push.unwrap_or(0.0) - vm.wall_push) * approach(14.0, dt);
        if vm.rig_t.value != Vec3::ZERO || vm.rig_r.value != Vec3::ZERO || vm.compress != 0.0 {
            (vm.rig_t, vm.rig_r) = rig_springs();
            vm.compress = 0.0;
        }
    }

    let (k, w_push) = (vm.kick, vm.wall_push * (1.0 - a));
    let hip_ads = f.hip.lerp(f.ads, a);
    let pos = hip_ads
        + Vec3::new(vm.sway.x * 0.35, -vm.sway.y * 0.35, 0.0) * (1.0 - 0.75 * a)
        + bob
        + Vec3::new(0.0, 0.01 * k * (1.0 - 0.5 * a), 0.05 * k)
        + Vec3::new(-0.03 * rl, -0.09 * rl + 0.015 * vm.bump, 0.03 * rl)
        + Vec3::new(0.0, -0.028 * vm.land, 0.0)
        + Vec3::new(0.0, 0.0, 0.035 * chamber)
        + Vec3::new(0.0, 0.0, if bolt { 0.0 } else { 0.05 * cyc })
        + Vec3::new(-0.05 * sprint, -0.07 * sprint, 0.04 * sprint)
        + Vec3::new(-0.08 * insp, 0.03 * insp, 0.02 * insp)
        + Vec3::new(0.0, -0.28 * lowered, 0.06 * lowered)
        + Vec3::new(0.0, -0.05 * w_push, w_push * 0.6)
        + Vec3::new(-0.012 * vm.strafe, -0.014 * vm.rise, 0.0) * (1.0 - 0.6 * a);
    let rot = Quat::from_euler(
        EulerRot::YXZ,
        vm.sway.x * 1.2 * (1.0 - 0.7 * a) + vm.jitter * 0.012 + 0.55 * sprint + 0.9 * insp + insp_wiggle * insp,
        -vm.sway.y * (1.0 - 0.7 * a) + 0.07 * k + 0.12 * rl + 0.05 * vm.bump - 0.5 * w_push - 0.85 * lowered - 0.2 * sprint
            + if bolt { 0.06 * cyc } else { 0.1 * cyc }
            - 0.04 * vm.land,
        vm.sway.x * 1.6 * (1.0 - 0.8 * a) + vm.jitter * 0.03 + 0.5 * rl + if bolt { 0.35 * cyc } else { 0.0 } - 0.35 * lowered + 0.25 * sprint
            - 0.45 * insp
            - 0.07 * vm.strafe * (1.0 - 0.6 * a),
    );
    let (pos, rot) = if body {
        // Chest cam: the gun sits nearer the middle of the frame, held lower.
        let chest = Vec3::new(-0.06, -0.025, 0.0) * (1.0 - a);
        let r = vm.rig_r.value;
        (pos + chest + vm.rig_t.value, Quat::from_euler(EulerRot::YXZ, r.y, r.x, r.z) * rot)
    } else {
        (pos, rot)
    };
    let new_tf = Transform { translation: pos, rotation: rot, scale: Vec3::ONE };
    if *tf != new_tf {
        *tf = new_tf;
    }
    // Hidden behind the optic (its overlay is the sight picture), and put away while a grenade is out.
    let want_vis = if w.sighted().is_some() || grenade.is_some_and(|g| g.out()) { Visibility::Hidden } else { Visibility::Inherited };
    if *vis != want_vis {
        *vis = want_vis;
    }
    let m = root_rot * (root_scale * new_tf.transform_point(f.muzzle));
    let barrel = (root_rot * (root_scale * (new_tf.rotation * Vec3::NEG_Z))).normalize_or(Vec3::NEG_Z);
    if muzzle_cam.0 != m || muzzle_cam.1 != barrel {
        *muzzle_cam = MuzzleCam(m, barrel);
    }

    // Magazines of the held gun: ~12 cm down (gun space) and out of frame. The magazine node's
    // parent can be any frame (Kenney meshes, skinned Pichuliru bones under a 100x rig), so the
    // drop is carried into the parent's space.
    for (part, mut mtf, parent) in &mut mags {
        let Some(gun_e) = std::iter::once(parent.parent()).chain(parents.iter_ancestors(parent.parent())).find(|a| guns.get(*a).is_ok_and(|g| g.0 == w.kind)) else {
            continue;
        };
        let target = if mag_drop == 0.0 {
            part.rest
        } else {
            let drop = match (globals.get(gun_e), globals.get(parent.parent())) {
                (Ok(g), Ok(p)) => p.affine().inverse().transform_vector3(g.affine().transform_vector3(Vec3::new(0.0, -0.12, 0.02))),
                _ => Vec3::ZERO,
            };
            Transform { translation: part.rest.translation + drop * mag_drop, ..part.rest }
        };
        if *mtf != target {
            *mtf = target;
        }
    }

    // Muzzle flash.
    vm.flash = (vm.flash - dt).max(0.0);
    let fl = vm.flash / FLASH_TIME;
    let on = fl > 0.0;
    let roll = Quat::from_rotation_z(rand::random_range(0.0..std::f32::consts::TAU));
    let grow = rand::random_range(0.8..1.25) * vm.flash_scale.max(0.3);
    for (mut fvis, mut ftf, light) in &mut flashes {
        if let Some(mut light) = light {
            let want = if on { 2.5e6 * fl.sqrt() * vm.flash_scale } else { 0.0 };
            if light.intensity != want {
                light.intensity = want;
            }
            continue;
        }
        let want = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *fvis != want {
            *fvis = want;
        }
        if on {
            if ftf.translation.z == 0.0 {
                // Front-on star: new roll and size each frame it is shown.
                ftf.rotation = roll;
                ftf.scale = Vec3::splat(0.2 * grow);
            } else {
                ftf.scale = Vec3::new(0.3 * grow, 0.12 * grow, 1.0);
            }
        }
    }
}

/// The gun scenes' meshes must not cast shadows into the world.
fn viewmodel_no_shadows(
    mut commands: Commands,
    added: Query<Entity, (Added<Mesh3d>, Without<NotShadowCaster>)>,
    parents: Query<&ChildOf>,
    vms: Query<(), With<ViewRoot>>,
) {
    for e in &added {
        if parents.iter_ancestors(e).any(|a| vms.contains(a)) {
            commands.entity(e).insert(NotShadowCaster);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_fit_keeps_the_measured_anchors() {
        let f = choose(&[], &|_: &str| true)[WeaponKind::ALL.iter().position(|k| *k == WeaponKind::Rifle).unwrap()];
        assert_eq!(f.path, "models/weapons/rifle.glb");
        assert_eq!(f.low, "models/weapons/lod/rifle.lod1.glb");
        // Muzzle and ADS relative to the grip (the pivot), as before the table moved to data.
        assert!((f.muzzle - Vec3::new(0.0, 0.066, -0.449)).length() < 1e-5);
        assert!((f.ads - Vec3::new(0.0, -0.133, -0.3)).length() < 1e-5);
        // The generated LOD is in metres; the rig in hectometre units.
        assert_eq!((f.mesh_m, f.low_mesh_m), (100.0, 1.0));
    }

    #[test]
    fn a_pack_model_wins_only_when_its_file_exists() {
        let pack: Vec<GunAsset> = serde_json::from_str(
            r#"[{"kind": "Kestrel", "high": "library/misc/pack/ar.glb", "yaw_deg": -90, "own_materials": true,
                 "hand": [0.1, -0.05, 0], "muzzle": [-0.4, 0.02, 0], "sight": 0.08, "center": [0, 0, 0], "support": [-0.2, 0, 0]}]"#,
        )
        .unwrap();
        let rifle = WeaponKind::ALL.iter().position(|k| *k == WeaponKind::Rifle).unwrap();
        let without = choose(&pack, &|p: &str| !p.starts_with("library/"));
        assert_eq!(without[rifle].path, "models/weapons/rifle.glb");
        let with = choose(&pack, &|_: &str| true);
        let f = with[rifle];
        assert_eq!(f.path, "library/misc/pack/ar.glb");
        // No separate low file: the low tier is the same model.
        assert_eq!(f.low, f.path);
        assert!(f.own_materials);
        // Barrel along -X turned -90 degrees (about +Y) onto -Z: the muzzle ends up ahead of the grip.
        assert!(f.muzzle.z < -0.45 && f.muzzle.x.abs() < 1e-5, "{:?}", f.muzzle);
        // Other kinds keep the built-in set.
        assert_eq!(with[0].path, without[0].path);
    }

    #[test]
    fn tiers_hand_over_without_a_gap() {
        let (h, l) = tier_ranges(PICKUP_HIGH_M);
        assert_eq!(h.0.start_margin, 0.0..0.0);
        assert_eq!(h.0.end_margin, l.0.start_margin);
        assert!(PICKUP_HIGH_M > super::super::pickups::SWAP_RANGE, "a gun in reach must already be the high model");
    }

    #[test]
    fn muzzle_shadows_only_high_up_and_unthrottled() {
        assert!(!muzzle_shadows_allowed(GraphicsQuality::Low, 0));
        assert!(!muzzle_shadows_allowed(GraphicsQuality::Medium, 0));
        assert!(muzzle_shadows_allowed(GraphicsQuality::High, 0));
        assert!(muzzle_shadows_allowed(GraphicsQuality::Ultra, 0));
        assert!(!muzzle_shadows_allowed(GraphicsQuality::Ultra, 1));
    }
}
