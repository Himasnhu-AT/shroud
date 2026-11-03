//! First-person guns: one model per [`WeaponKind`] under a pivot on the camera, shown for the
//! gun in hand, and their procedural animation (draw / holster, reloads with the magazine
//! coming out, pump and bolt cycles, inspect, bob, look sway, landing dip, sprint pose, recoil
//! kick + muzzle rise, ADS to the sight line, pull-back near walls) plus the muzzle flash.
//!
//! The viewmodel keeps its own field of view: its root is scaled by
//! `tan(fov/2) / tan(VIEWMODEL_FOV/2)` in camera x/y, so the gun looks the same at any FOV
//! setting and does not balloon when the camera zooms for ADS.

use avian3d::prelude::*;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use engine::fps::{FpsLook, FpsMotor};
use engine::gfx::MainCamera;

use super::arsenal::{HOLSTER_S, INSPECT_S, Phase, ReloadKind, Weapon};
use super::{GunEvent, WeaponKind, WeaponSet, approach, ease};
use crate::assets::AssetsDir;
use crate::gfx::FxAssets;
use crate::player::Player;
use crate::state::has_visuals;

/// The field of view the viewmodels were posed for.
pub const VIEWMODEL_FOV_DEG: f32 = 80.0;
const FLASH_TIME: f32 = 0.05;

pub(super) fn build(app: &mut App) {
    app.init_resource::<MuzzleCam>()
        .add_systems(PostStartup, spawn_viewmodel.run_if(has_visuals))
        .add_systems(
            Update,
            (find_mag_parts, show_held_gun, animate_viewmodel, viewmodel_no_shadows).chain().after(WeaponSet).run_if(has_visuals),
        );
}

/// Where the muzzle of the gun in hand appears, in camera space (tracer and flash origin).
#[derive(Resource, Clone, Copy, Debug)]
pub struct MuzzleCam(pub Vec3);

impl Default for MuzzleCam {
    fn default() -> Self {
        let f = fit(WeaponKind::Rifle);
        MuzzleCam(f.hip + f.muzzle)
    }
}

/// How a gun's model sits in the hands. All in metres, camera space (x right, y up, -z ahead).
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    pub path: &'static str,
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
}

/// Per-gun numbers, model space (metres, barrel toward -Z) unless noted.
struct Raw {
    path: &'static str,
    scale: f32,
    /// Firing hand on the grip (becomes the pivot).
    hand: Vec3,
    /// Front of the barrel.
    muzzle: Vec3,
    /// Height of the sight line (iron sights or the optic's axis).
    sight: f32,
    center: Vec3,
    /// Support hand (handguard / pump / under the firing hand on pistols).
    support: Vec3,
    optic: Option<(&'static str, Vec3)>,
    /// Camera space: hip pivot, ADS depth.
    hip: Vec3,
    ads_z: f32,
    kick: f32,
    flash: f32,
    body: Vec3,
}

/// Per-gun placement, from each GLB's measured rest-pose bounds and attachment nodes
/// (Pichuliru's CC0 gun set: real-world metres, barrel toward -Z, stock toward +Z, so no turn
/// and scale ~1). The pivot sits at the firing hand on the grip.
pub fn fit(kind: WeaponKind) -> Fit {
    const PISTOL_HIP: Vec3 = Vec3::new(0.13, -0.15, -0.4);
    const RIFLE_HIP: Vec3 = Vec3::new(0.17, -0.2, -0.34);
    let r = match kind {
        WeaponKind::Pistol => Raw {
            path: "models/weapons/pistol.glb",
            scale: 1.0,
            hand: Vec3::new(0.0, -0.03, 0.05),
            muzzle: Vec3::new(0.0, 0.025, -0.117),
            sight: 0.042,
            center: Vec3::new(0.0, -0.025, -0.02),
            support: Vec3::new(-0.022, -0.045, 0.035),
            optic: None,
            hip: PISTOL_HIP,
            ads_z: -0.34,
            kick: 22.0,
            flash: 0.7,
            body: Vec3::new(0.03, 0.13, 0.19),
        },
        WeaponKind::HeavyPistol => Raw {
            path: "models/weapons/heavy_pistol.glb",
            scale: 1.05,
            hand: Vec3::new(0.0, -0.035, 0.05),
            muzzle: Vec3::new(0.0, 0.024, -0.127),
            sight: 0.04,
            center: Vec3::new(0.0, -0.036, -0.024),
            support: Vec3::new(-0.024, -0.05, 0.035),
            optic: None,
            hip: PISTOL_HIP,
            ads_z: -0.34,
            kick: 40.0,
            flash: 1.0,
            body: Vec3::new(0.03, 0.15, 0.21),
        },
        WeaponKind::Smg => Raw {
            path: "models/weapons/smg.glb",
            scale: 1.0,
            hand: Vec3::new(0.0, -0.045, 0.085),
            muzzle: Vec3::new(0.0, 0.017, -0.222),
            sight: 0.064,
            center: Vec3::new(0.0, -0.057, 0.06),
            support: Vec3::new(0.0, -0.005, -0.13),
            optic: None,
            hip: Vec3::new(0.16, -0.19, -0.34),
            ads_z: -0.26,
            kick: 14.0,
            flash: 0.6,
            body: Vec3::new(0.046, 0.12, 0.45),
        },
        WeaponKind::Rifle => Raw {
            path: "models/weapons/rifle.glb",
            scale: 1.0,
            hand: Vec3::new(0.0, -0.05, 0.11),
            muzzle: Vec3::new(0.0, 0.016, -0.339),
            sight: 0.083,
            center: Vec3::new(0.0, -0.05, 0.01),
            support: Vec3::new(0.0, -0.012, -0.17),
            optic: Some(("models/weapons/optic_magnified.glb", Vec3::new(0.0, 0.041, 0.011))),
            hip: RIFLE_HIP,
            ads_z: -0.3,
            kick: 30.0,
            flash: 1.0,
            body: Vec3::new(0.05, 0.12, 0.65),
        },
        WeaponKind::BattleRifle => Raw {
            path: "models/weapons/battle_rifle.glb",
            scale: 1.0,
            hand: Vec3::new(0.0, -0.06, 0.125),
            muzzle: Vec3::new(0.0, 0.006, -0.34),
            sight: 0.074,
            center: Vec3::new(0.0, -0.06, 0.02),
            support: Vec3::new(0.0, -0.01, -0.16),
            optic: None,
            hip: RIFLE_HIP,
            ads_z: -0.3,
            kick: 38.0,
            flash: 1.1,
            body: Vec3::new(0.06, 0.13, 0.7),
        },
        WeaponKind::Shotgun => Raw {
            path: "models/weapons/shotgun.glb",
            scale: 1.0,
            hand: Vec3::new(0.0, -0.03, 0.175),
            muzzle: Vec3::new(0.0, 0.03, -0.277),
            sight: 0.052,
            center: Vec3::new(0.0, -0.01, 0.02),
            support: Vec3::new(0.0, -0.01, -0.087),
            optic: None,
            hip: RIFLE_HIP,
            ads_z: -0.28,
            kick: 55.0,
            flash: 1.5,
            body: Vec3::new(0.046, 0.1, 0.58),
        },
        WeaponKind::Marksman => Raw {
            path: "models/weapons/marksman.glb",
            scale: 1.0,
            hand: Vec3::new(0.0, -0.06, 0.125),
            muzzle: Vec3::new(0.0, 0.012, -0.452),
            sight: 0.096,
            center: Vec3::new(0.0, -0.03, -0.03),
            support: Vec3::new(0.0, -0.01, -0.2),
            optic: Some(("models/weapons/scope_short.glb", Vec3::new(0.0, 0.052, 0.011))),
            hip: RIFLE_HIP,
            ads_z: -0.24,
            kick: 60.0,
            flash: 1.3,
            body: Vec3::new(0.054, 0.13, 0.84),
        },
        WeaponKind::Sniper => Raw {
            path: "models/weapons/sniper.glb",
            scale: 1.0,
            hand: Vec3::new(0.0, -0.06, 0.2),
            muzzle: Vec3::new(0.0, 0.015, -0.545),
            sight: 0.083,
            center: Vec3::new(0.0, -0.02, -0.065),
            support: Vec3::new(0.0, -0.01, -0.26),
            optic: Some(("models/weapons/scope_long.glb", Vec3::new(0.0, 0.044, 0.011))),
            hip: Vec3::new(0.18, -0.21, -0.31),
            ads_z: -0.24,
            kick: 75.0,
            flash: 1.5,
            body: Vec3::new(0.06, 0.13, 0.96),
        },
    };
    Fit {
        path: r.path,
        scale: r.scale,
        offset: -r.hand * r.scale,
        yaw: 0.0,
        muzzle: (r.muzzle - r.hand) * r.scale,
        hip: r.hip,
        ads: Vec3::new(0.0, -(r.sight - r.hand.y) * r.scale, r.ads_z),
        kick: r.kick,
        flash: r.flash,
        body: r.body,
        center: r.center,
        optic: r.optic,
        support: (r.support - r.hand) * r.scale,
        pistol: kind.spec().slot == brain::weapons::Slot::Secondary,
    }
}

/// Spawn gun `kind`'s model (and its optic) under `parent` with transform `tf`; its materials
/// get the gun's finish (`finish.rs`). The caller checks that the model file exists.
pub fn spawn_gun_model(commands: &mut Commands, server: &AssetServer, dir: &AssetsDir, kind: WeaponKind, tf: Transform, parent: Entity) -> Entity {
    let f = fit(kind);
    let scene = |p: &'static str| server.load::<bevy::world_serialization::WorldAsset>(bevy::gltf::GltfAssetLabel::Scene(0).from_asset(p));
    let model = commands
        .spawn((bevy::world_serialization::WorldAssetRoot(scene(f.path)), super::finish::GunFinish(kind), tf, Visibility::default(), ChildOf(parent)))
        .id();
    if let Some((path, at)) = f.optic
        && dir.has(path)
    {
        commands.spawn((
            bevy::world_serialization::WorldAssetRoot(scene(path)),
            super::finish::GunFinish(kind),
            Transform::from_translation(at),
            Visibility::default(),
            ChildOf(model),
        ));
    }
    model
}

/// First-person arms: gloves and sleeves in the player's (attacker) colours, built from a few
/// shared primitives. Children of each gun so they follow every gun animation.
struct HandKit {
    cube: Handle<Mesh>,
    tube: Handle<Mesh>,
    glove: Handle<StandardMaterial>,
    sleeve: Handle<StandardMaterial>,
    cuff: Handle<StandardMaterial>,
}

impl HandKit {
    fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        HandKit {
            cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            tube: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(12).segments(1)),
            // Black tactical gloves, coyote combat shirt, the player team's teal cuff band.
            glove: materials.add(StandardMaterial { base_color: Color::srgb(0.06, 0.06, 0.065), perceptual_roughness: 0.75, ..default() }),
            sleeve: materials.add(StandardMaterial { base_color: Color::srgb(0.34, 0.29, 0.21), perceptual_roughness: 0.9, ..default() }),
            cuff: materials.add(StandardMaterial {
                base_color: Color::srgb(0.08, 0.33, 0.3),
                emissive: LinearRgba::rgb(0.01, 0.05, 0.045),
                perceptual_roughness: 0.7,
                ..default()
            }),
        }
    }
}

/// A forearm from `wrist` running toward `elbow` (pivot space): sleeve tube plus cuff band.
fn arm(commands: &mut Commands, k: &HandKit, gun: Entity, wrist: Vec3, elbow: Vec3) {
    let d = elbow - wrist;
    let len = d.length();
    let rot = Quat::from_rotation_arc(Vec3::Y, d / len);
    let mut part = |mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>, tf: Transform| {
        commands.spawn((Name::new("arm"), Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), tf, ChildOf(gun)));
    };
    // The glove's wrist, then the cuff, then the sleeve.
    part(&k.tube, &k.glove, Transform::from_translation(wrist + d / len * 0.02).with_rotation(rot).with_scale(Vec3::new(0.024, 0.05, 0.024)));
    part(&k.tube, &k.cuff, Transform::from_translation(wrist + d / len * 0.055).with_rotation(rot).with_scale(Vec3::new(0.03, 0.018, 0.03)));
    let s0 = 0.064;
    part(&k.tube, &k.sleeve, Transform::from_translation(wrist + d / len * (s0 + (len - s0) * 0.5)).with_rotation(rot).with_scale(Vec3::new(0.032, len - s0, 0.032)));
}

fn spawn_hands(commands: &mut Commands, k: &HandKit, f: &Fit, gun: Entity) {
    let box_ = |commands: &mut Commands, at: Vec3, size: Vec3, rot: Quat| {
        commands.spawn((Name::new("glove"), Mesh3d(k.cube.clone()), MeshMaterial3d(k.glove.clone()), Transform::from_translation(at).with_rotation(rot).with_scale(size), ChildOf(gun)));
    };
    // Firing hand: palm on the right of the grip, fingers wrapped round its front, thumb on top
    // of the left side; the forearm runs back and down out of frame.
    let sc = if f.pistol { 0.75 } else { 1.0 };
    box_(commands, Vec3::new(0.02, -0.02, 0.0) * sc, Vec3::new(0.018, 0.075, 0.065) * sc, Quat::IDENTITY);
    box_(commands, Vec3::new(0.0, -0.028, -0.03) * sc, Vec3::new(0.042, 0.06, 0.018) * sc, Quat::IDENTITY);
    box_(commands, Vec3::new(-0.015, 0.01, -0.01) * sc, Vec3::new(0.014, 0.016, 0.05) * sc, Quat::IDENTITY);
    arm(commands, k, gun, Vec3::new(0.018, -0.055, 0.04) * sc, Vec3::new(0.2, -0.36, 0.36));
    let s = f.support;
    if f.pistol {
        // Cupping the firing hand from the left.
        box_(commands, s + Vec3::new(-0.008, 0.0, -0.004), Vec3::new(0.016, 0.06, 0.055), Quat::IDENTITY);
        box_(commands, s + Vec3::new(0.008, -0.01, -0.03), Vec3::new(0.034, 0.045, 0.016), Quat::IDENTITY);
        arm(commands, k, gun, s + Vec3::new(-0.015, -0.035, 0.035), s + Vec3::new(-0.24, -0.36, 0.3));
    } else {
        // Under the handguard / on the pump: palm below, fingers up the right side, thumb left.
        box_(commands, s + Vec3::new(0.0, -0.032, 0.0), Vec3::new(0.06, 0.024, 0.09), Quat::IDENTITY);
        box_(commands, s + Vec3::new(0.03, -0.012, 0.0), Vec3::new(0.018, 0.045, 0.08), Quat::IDENTITY);
        box_(commands, s + Vec3::new(-0.03, -0.014, 0.01), Vec3::new(0.016, 0.035, 0.06), Quat::IDENTITY);
        arm(commands, k, gun, s + Vec3::new(-0.005, -0.045, 0.04), s + Vec3::new(-0.22, -0.34, 0.36));
    }
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
) {
    let Ok(cam) = cams.single() else { return };
    commands.entity(cam).insert(Visibility::default());
    let root = commands.spawn((Name::new("ViewModel"), ViewRoot, Transform::default(), Visibility::default(), ChildOf(cam))).id();
    let rifle = fit(WeaponKind::Rifle);
    let pivot = commands
        .spawn((Name::new("ViewModel pivot"), ViewModel::default(), Transform::from_translation(rifle.hip), Visibility::default(), ChildOf(root)))
        .id();
    let hands = HandKit::new(&mut meshes, &mut materials);
    // Primitive fallback, shared by every gun that has no model file.
    let mut prim: Option<(Handle<StandardMaterial>, Handle<StandardMaterial>)> = None;
    for kind in WeaponKind::ALL {
        let f = fit(kind);
        let gun = commands
            .spawn((Name::new(format!("Gun {kind:?}")), GunModel(kind), Transform::default(), Visibility::Hidden, ChildOf(pivot)))
            .id();
        spawn_hands(&mut commands, &hands, &f, gun);
        if assets_dir.has(f.path) {
            let tf = Transform::from_translation(f.offset).with_rotation(Quat::from_rotation_y(f.yaw)).with_scale(Vec3::splat(f.scale));
            spawn_gun_model(&mut commands, &asset_server, &assets_dir, kind, tf, gun);
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
) {
    let dt = time.delta_secs().clamp(1e-4, 0.05);
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
    if let Ok(mut rtf) = roots.single_mut()
        && (rtf.scale - root_scale).length_squared() > 1e-8
    {
        rtf.scale = root_scale;
    }

    for ev in events.read() {
        match *ev {
            GunEvent::Shot { kind } => {
                let k = fit(kind);
                vm.kick_vel += k.kick * (1.0 - 0.45 * w.ads);
                vm.jitter = rand::random_range(-1.0..1.0);
                vm.flash = FLASH_TIME;
                vm.flash_scale = k.flash;
            }
            GunEvent::MagIn { .. } | GunEvent::ShellIn | GunEvent::Chamber { .. } => vm.bump = 1.0,
            GunEvent::Pump | GunEvent::Bolt => vm.bump = 0.6,
            _ => {}
        }
    }

    // Look sway: the gun lags behind look rotation.
    let look_now = Vec2::new(look.yaw, look.pitch);
    let rate = vm.last_look.map_or(Vec2::ZERO, |l| (look_now - l) / dt);
    vm.last_look = Some(look_now);
    let target = (rate * 0.012).clamp(Vec2::splat(-0.06), Vec2::splat(0.06));
    vm.sway = vm.sway.lerp(target, approach(10.0, dt));

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
    let push = cam.and_then(|(g, _)| {
        let filter = SpatialQueryFilter::from_excluded_entities([player]);
        spatial.cast_ray(g.translation(), g.forward(), 1.0, true, &filter).map(|h| (0.85 - h.distance).clamp(0.0, 0.4))
    });
    vm.wall_push += (push.unwrap_or(0.0) - vm.wall_push) * approach(14.0, dt);

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
        + Vec3::new(0.0, -0.05 * w_push, w_push * 0.6);
    let rot = Quat::from_euler(
        EulerRot::YXZ,
        vm.sway.x * 1.2 * (1.0 - 0.7 * a) + vm.jitter * 0.012 + 0.55 * sprint + 0.9 * insp + insp_wiggle * insp,
        -vm.sway.y * (1.0 - 0.7 * a) + 0.07 * k + 0.12 * rl + 0.05 * vm.bump - 0.5 * w_push - 0.85 * lowered - 0.2 * sprint
            + if bolt { 0.06 * cyc } else { 0.1 * cyc }
            - 0.04 * vm.land,
        vm.sway.x * 1.6 * (1.0 - 0.8 * a) + vm.jitter * 0.03 + 0.5 * rl + if bolt { 0.35 * cyc } else { 0.0 } - 0.35 * lowered + 0.25 * sprint
            - 0.45 * insp,
    );
    let new_tf = Transform { translation: pos, rotation: rot, scale: Vec3::ONE };
    if *tf != new_tf {
        *tf = new_tf;
    }
    // Hidden behind the optic (its overlay is the sight picture).
    let want_vis = if w.sighted().is_some() { Visibility::Hidden } else { Visibility::Inherited };
    if *vis != want_vis {
        *vis = want_vis;
    }
    let m = root_scale * new_tf.transform_point(f.muzzle);
    if muzzle_cam.0 != m {
        muzzle_cam.0 = m;
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
