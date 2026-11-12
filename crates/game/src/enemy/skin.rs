//! Operator skins for the enemy squad: the Quaternius mannequin dressed as a soldier.
//!
//! - The mannequin's two materials become fatigues (`M_Main`) and a dark base layer (`M_Joints`),
//!   coloured per variant.
//! - Gear is a handful of low-poly meshes parented to the rig's bones: a helmet with an NVG
//!   mount (Head), a plate carrier with pouches and, per variant, a backpack or a radio
//!   (spine_03), gloves (hands), boots (calves) and a red team armband (left upper arm) that
//!   reads at distance.
//! - Three variants by squad slot (`slot % 3`) so the squad isn't clones.
//!
//! Cost: every mesh and material is built once and shared by every enemy (same handles batch);
//! about 9 small meshes per enemy, ~1.2k triangles in all. Gear is tagged under the enemy's
//! `Actor` root. Hit detection uses the enemy's own capsule/sphere colliders, so the gear changes
//! nothing there (headshots still land on the head sphere).
//!
//! Placement: limbs use bone space (UAL bones run along their local +Y). Helmet and vest are
//! placed in the character's frame from the bones' pose a few frames after the scene spawns
//! (idle), then parented to the bone so they follow the animation.
//! Missing model: enemies stay primitives and none of this runs.
//!
//! MetaHuman enemy (`models/metahuman/enemy_mh.glb`, when generated): its body mesh takes the
//! fatigues (a full-body uniform), the head, eyes and teeth keep their own materials. Its bones
//! run along X rather than Y, so limb gear is placed along each bone's direction to its child.
//!
//! With `models/metahuman/gear_mh.glb` (`scripts/make-gear-mh.sh`, local only like the
//! MetaHuman) the MetaHuman wears real clothing instead of the primitive kit: a combat shirt,
//! cargo trousers with pockets and knee pads, boots, gloves, a balaclava, a plate carrier with
//! MOLLE and pouches, a belt, a high-cut helmet and the squad's pack. Each piece is a skinned
//! mesh bound to the enemy's own bones by name (`SkinnedMesh` with the enemy's joint entities),
//! so it deforms with every animation; materials are picked per squad look by the mesh name's
//! `<part>.<material>` suffix. About 28k triangles in 22 draws per enemy.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::gltf::{Gltf, GltfMesh, GltfNode, GltfSkin};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::Enemy;
use crate::state::has_visuals;

/// Bones the kit hangs on (Quaternius UAL names; MetaHuman: `head`, chest `spine_05`).
const HEADS: [&str; 2] = ["Head", "head"];
/// Chest bones, best last: a MetaHuman has spine_03 too, but lower down.
const CHESTS: [&str; 2] = ["spine_03", "spine_05"];
const HANDS: [&str; 2] = ["hand_l", "hand_r"];
const CALVES: [&str; 2] = ["calf_l", "calf_r"];
const THIGHS: [&str; 2] = ["thigh_l", "thigh_r"];
const UPPERARMS: [&str; 2] = ["upperarm_l", "upperarm_r"];
const LOWERARMS: [&str; 2] = ["lowerarm_l", "lowerarm_r"];
const FEET: [&str; 2] = ["foot_l", "foot_r"];
const BALLS: [&str; 2] = ["ball_l", "ball_r"];
const ARM_L: &str = "upperarm_l";
/// Frames after the rig appears before the pose is read (transforms have propagated, idle).
const SETTLE_FRAMES: u8 = 4;

/// One look: fatigues, base layer and gear colours (linear-ish sRGB).
struct Look {
    fatigues: Color,
    base: Color,
    gear: Color,
    helmet: Color,
    pack: Pack,
}

#[derive(Clone, Copy, PartialEq)]
enum Pack {
    Backpack,
    Radio,
    None,
}

fn looks() -> [Look; 3] {
    [
        // Navy fatigues, black plate carrier and helmet.
        Look {
            fatigues: Color::srgb(0.10, 0.12, 0.17),
            base: Color::srgb(0.05, 0.05, 0.06),
            gear: Color::srgb(0.06, 0.06, 0.065),
            helmet: Color::srgb(0.07, 0.075, 0.08),
            pack: Pack::None,
        },
        // Charcoal fatigues, ranger-green carrier, assault pack.
        Look {
            fatigues: Color::srgb(0.13, 0.13, 0.135),
            base: Color::srgb(0.05, 0.05, 0.05),
            gear: Color::srgb(0.13, 0.15, 0.11),
            helmet: Color::srgb(0.12, 0.14, 0.10),
            pack: Pack::Backpack,
        },
        // Dark olive fatigues, coyote carrier and helmet, radio.
        Look {
            fatigues: Color::srgb(0.14, 0.15, 0.10),
            base: Color::srgb(0.06, 0.055, 0.05),
            gear: Color::srgb(0.36, 0.29, 0.19),
            helmet: Color::srgb(0.33, 0.27, 0.18),
            pack: Pack::Radio,
        },
    ]
}

/// The MetaHuman's clothing and gear, skinned to its skeleton (local only, see the module docs).
pub const GEAR_MH: &str = "models/metahuman/gear_mh.glb";

/// Materials for the skinned gear (by the mesh name's material suffix).
struct GearMats {
    /// Fatigues on box-projected metre UVs (pockets).
    cloth_box: [Handle<StandardMaterial>; 3],
    cordura_box: [Handle<StandardMaterial>; 3],
    helmet_box: [Handle<StandardMaterial>; 3],
    /// Gloves on the body's UVs; boots on metre UVs.
    leather_mh: Handle<StandardMaterial>,
    leather_box: Handle<StandardMaterial>,
    webbing: Handle<StandardMaterial>,
    balaclava: Handle<StandardMaterial>,
    sole: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
    pads: Handle<StandardMaterial>,
}

/// Shared skin assets.
#[derive(Resource)]
struct Kit {
    /// The skinned gear file, when it exists.
    gear_mh: Option<Handle<Gltf>>,
    gear_mats: GearMats,
    fatigues: [Handle<StandardMaterial>; 3],
    /// Same fabric at the MetaHuman body's UV scale (one 0..1 map for the whole body).
    fatigues_mh: [Handle<StandardMaterial>; 3],
    base: [Handle<StandardMaterial>; 3],
    gear: [Handle<StandardMaterial>; 3],
    helmet_mat: [Handle<StandardMaterial>; 3],
    /// Gloves and boots: photoscanned black leather when present.
    leather: Handle<StandardMaterial>,
    /// Trouser legs and sleeves: the variant's fatigues at the tubes' UV scale.
    cloth: [Handle<StandardMaterial>; 3],
    accent: Handle<StandardMaterial>,
    helmet: Handle<Mesh>,
    vest: [Handle<Mesh>; 3],
    glove: Handle<Mesh>,
    boot: Handle<Mesh>,
    band: Handle<Mesh>,
    /// Unit tapered tubes, one per [`CLOTHS`] entry: y 0..1, radius 1 at y = 0, r1 / r0 at y = 1.
    tubes: [Handle<Mesh>; 4],
}

/// Loose cloth over a limb bone: tube radii (m) at the bone's start and end, and how much of
/// the bone it covers. Sized for a soldier's cargo trousers and combat shirt, a little looser
/// than the body under them so the silhouette reads as clothes, not a body suit.
struct Cloth {
    r0: f32,
    r1: f32,
    from: f32,
    to: f32,
}

const THIGH_CLOTH: Cloth = Cloth { r0: 0.098, r1: 0.07, from: -0.06, to: 1.04 };
const CALF_CLOTH: Cloth = Cloth { r0: 0.072, r1: 0.06, from: 0.0, to: 0.78 };
const UPPERARM_CLOTH: Cloth = Cloth { r0: 0.066, r1: 0.054, from: -0.05, to: 1.03 };
const LOWERARM_CLOTH: Cloth = Cloth { r0: 0.053, r1: 0.046, from: 0.0, to: 0.72 };
const CLOTHS: [&Cloth; 4] = [&THIGH_CLOTH, &CALF_CLOTH, &UPPERARM_CLOTH, &LOWERARM_CLOTH];

/// Bones found under an enemy, until it is dressed.
#[derive(Component, Default)]
struct Rig {
    head: Option<Entity>,
    chest: Option<Entity>,
    hands: [Option<Entity>; 2],
    calves: [Option<Entity>; 2],
    thighs: [Option<Entity>; 2],
    upperarms: [Option<Entity>; 2],
    lowerarms: [Option<Entity>; 2],
    feet: [Option<Entity>; 2],
    balls: [Option<Entity>; 2],
    arm: Option<Entity>,
    /// The MetaHuman body mesh's node (skinned gear is parented beside it).
    body: Option<Entity>,
    /// Rank in [`CHESTS`] of the chest found so far.
    chest_rank: usize,
    frames: u8,
}

#[derive(Component)]
struct Dressed;

/// A gear mesh (skipped by the body recolour).
#[derive(Component)]
struct Gear;

pub struct EnemySkinPlugin;

impl Plugin for EnemySkinPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup.run_if(has_visuals)).add_systems(
            Update,
            (find_bones, dress, recolour).chain().run_if(has_visuals).run_if(resource_exists::<Kit>),
        );
    }
}

/// The fabric set is off-white: scale its tint so the textured fatigues average the variant's
/// flat colour.
pub fn fabric_tint(c: Color) -> Color {
    let l = c.to_linear();
    // The ripstop colour map averages ~0.27 linear (`textures/ripstop/color.avg`); divide it out.
    let k = 1.0 / 0.27;
    Color::LinearRgba(LinearRgba::rgb(l.red * k, l.green * k, l.blue * k))
}

/// Linear albedo averages of the sets' colour maps (`color.avg`), divided out of the tints.
const CORDURA_AVG: f32 = 0.52;
const LEATHER_AVG: f32 = 0.009;

fn divided(c: Color, avg: f32) -> Color {
    let l = c.to_linear();
    Color::LinearRgba(LinearRgba::rgb(l.red / avg, l.green / avg, l.blue / avg))
}

/// `m` with MikkTSpace tangents (the kit's materials are normal-mapped photoscans).
fn tangents(m: Mesh) -> Mesh {
    m.clone().with_generated_tangents().unwrap_or(m)
}

/// Unit tapered tube along +Y (0..1), radius 1 at the bottom and `taper` at the top.
fn tube_mesh(taper: f32) -> Mesh {
    ConicalFrustum { radius_top: taper, radius_bottom: 1.0, height: 1.0 }
        .mesh()
        .resolution(14)
        .segments(1)
        .build()
        .translated_by(Vec3::Y * 0.5)
}

/// Glove in the hand bone's frame, unit size along +Y (wrist to fingertips): a flattened
/// mitt over the palm and fingers and a cuff at the wrist.
fn glove_mesh() -> Mesh {
    merge([
        (Sphere::new(1.0).mesh().uv(12, 8), Transform::from_xyz(0.0, 0.55, 0.0).with_scale(Vec3::new(0.55, 0.5, 0.5))),
        (Cylinder::new(1.0, 1.0).mesh().resolution(12).segments(1).build(), Transform::from_xyz(0.0, 0.05, 0.0).with_scale(Vec3::new(0.45, 0.3, 0.42))),
    ])
}

/// Combat boot in its own frame (metres; origin at the ankle joint, +Y up, -Z toward the toe),
/// for an ankle 0.085 m above the sole and the toe 0.2 m ahead of it; the dress step scales it
/// to the rig's foot.
fn boot_mesh() -> Mesh {
    let t = |x: f32, y: f32, z: f32| Transform::from_xyz(x, y, z);
    merge([
        // Shaft round the ankle and lower shin.
        (Cylinder::new(0.056, 0.17).mesh().resolution(14).segments(1).build(), t(0.0, 0.03, 0.005)),
        // Upper over the foot: a rounded box from the heel to the toe cap.
        (Capsule3d::new(0.045, 0.2).mesh().rings(1).latitudes(6).longitudes(12).build(), t(0.0, -0.045, -0.075).with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)).with_scale(Vec3::new(1.0, 1.0, 0.8))),
        // Lugged sole, a little wider than the upper.
        (cuboid(0.1, 0.025, 0.31), t(0.0, -0.075, -0.07)),
    ])
}

fn mat(materials: &mut Assets<StandardMaterial>, c: Color, rough: f32) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial { base_color: c, perceptual_roughness: rough, reflectance: 0.3, ..default() })
}

/// UV repeats of the ripstop weave across the mannequin's texture atlas (a fine weave, not a
/// pattern you notice at range).
const FABRIC_REPEAT: f32 = 4.0;
const FABRIC_REPEAT_MH: f32 = 14.0;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Option<Res<AssetServer>>,
    dir: Option<Res<crate::assets::AssetsDir>>,
    pending: Option<ResMut<crate::gfx::PendingMips>>,
) {
    let l = looks();
    // Fatigues: the ripstop fabric set tinted per variant (CC0, `textures/ripstop`), or flat.
    let mut pending = pending;
    let fatigues = std::array::from_fn(|i| {
        let fabric = match (&server, &dir, pending.as_deref_mut()) {
            (Some(s), Some(d), Some(p)) => crate::gfx::load_pbr(s, d, p, "ripstop", fabric_tint(l[i].fatigues)),
            _ => None,
        };
        match fabric {
            Some(m) => materials.add(StandardMaterial {
                uv_transform: bevy::math::Affine2::from_scale(Vec2::splat(FABRIC_REPEAT)),
                reflectance: 0.3,
                ..m
            }),
            None => mat(&mut materials, l[i].fatigues, 0.85),
        }
    });
    let fatigues_mh = std::array::from_fn(|i| {
        let m = materials.get(&fatigues[i]).cloned().unwrap_or_default();
        materials.add(StandardMaterial { uv_transform: bevy::math::Affine2::from_scale(Vec2::splat(FABRIC_REPEAT_MH)), ..m })
    });
    let base = std::array::from_fn(|i| mat(&mut materials, l[i].base, 0.8));
    // Photoscanned sets (CC0): nylon cordura for the carrier and the helmet's cover, leather for
    // gloves and boots; flat colours without them.
    let mut set = |materials: &mut Assets<StandardMaterial>, name: &str, c: Color, avg: f32, uv: Vec2, rough: f32| -> Handle<StandardMaterial> {
        let m = match (&server, &dir, pending.as_deref_mut()) {
            (Some(s), Some(d), Some(p)) => crate::gfx::load_pbr(s, d, p, name, divided(c, avg)),
            _ => None,
        };
        match m {
            Some(m) => materials.add(StandardMaterial { uv_transform: bevy::math::Affine2::from_scale(uv), perceptual_roughness: rough, reflectance: 0.35, ..m }),
            None => mat(materials, c, rough),
        }
    };
    let gear = std::array::from_fn(|i| set(&mut materials, "cordura", l[i].gear, CORDURA_AVG, Vec2::splat(3.0), 1.0));
    let helmet_mat = std::array::from_fn(|i| set(&mut materials, "cordura", l[i].helmet, CORDURA_AVG, Vec2::new(8.0, 2.0), 0.95));
    let leather = set(&mut materials, "glove_leather", Color::srgb(0.18, 0.175, 0.17), LEATHER_AVG, Vec2::splat(3.0), 0.9);
    let cloth = std::array::from_fn(|i| {
        let m = materials.get(&fatigues[i]).cloned().unwrap_or_default();
        materials.add(StandardMaterial { uv_transform: bevy::math::Affine2::from_scale(Vec2::new(5.0, 3.0)), ..m })
    });
    // Team colour: a red armband with a little glow so the squad reads at 30 m.
    let accent = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.05, 0.03),
        emissive: LinearRgba::rgb(1.6, 0.08, 0.03),
        perceptual_roughness: 0.6,
        ..default()
    });
    let vest = std::array::from_fn(|i| meshes.add(tangents(vest_mesh(l[i].pack))));
    let rescale = |materials: &mut Assets<StandardMaterial>, h: &Handle<StandardMaterial>, uv: Vec2| {
        let m = materials.get(h).cloned().unwrap_or_default();
        materials.add(StandardMaterial { uv_transform: bevy::math::Affine2::from_scale(uv), ..m })
    };
    // Hard gear is UV'd at one unit per metre: a weave of a few centimetres.
    let gear_mats = GearMats {
        cloth_box: std::array::from_fn(|i| rescale(&mut materials, &fatigues[i], Vec2::splat(7.0))),
        cordura_box: std::array::from_fn(|i| rescale(&mut materials, &gear[i], Vec2::splat(10.0))),
        helmet_box: std::array::from_fn(|i| rescale(&mut materials, &helmet_mat[i], Vec2::splat(10.0))),
        leather_mh: rescale(&mut materials, &leather, Vec2::splat(30.0)),
        leather_box: rescale(&mut materials, &leather, Vec2::splat(8.0)),
        webbing: set(&mut materials, "cordura", Color::srgb(0.035, 0.036, 0.034), CORDURA_AVG, Vec2::splat(14.0), 0.9),
        balaclava: set(&mut materials, "cordura", Color::srgb(0.03, 0.03, 0.032), CORDURA_AVG, Vec2::splat(25.0), 1.0),
        sole: mat(&mut materials, Color::srgb(0.025, 0.025, 0.025), 0.85),
        metal: materials.add(StandardMaterial { base_color: Color::srgb(0.06, 0.06, 0.065), metallic: 0.7, perceptual_roughness: 0.45, ..default() }),
        pads: mat(&mut materials, Color::srgb(0.05, 0.05, 0.052), 0.55),
    };
    let gear_mh = match (&server, &dir) {
        (Some(s), Some(d)) if d.has(GEAR_MH) && crate::assets::enemy_model(d) == crate::assets::ENEMY_MODEL_MH => Some(s.load(GEAR_MH)),
        _ => None,
    };
    commands.insert_resource(Kit {
        gear_mh,
        gear_mats,
        fatigues,
        fatigues_mh,
        base,
        gear,
        helmet_mat,
        leather,
        cloth,
        accent,
        helmet: meshes.add(tangents(helmet_mesh())),
        vest,
        glove: meshes.add(tangents(glove_mesh())),
        boot: meshes.add(tangents(boot_mesh())),
        band: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(12).segments(1)),
        tubes: CLOTHS.map(|c| meshes.add(tangents(tube_mesh(c.r1 / c.r0)))),
    });
}

/// Upper half of a unit sphere (y >= 0), with a slight flare at the rim.
fn dome(rings: u32, sectors: u32) -> Mesh {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uv = Vec::new();
    for r in 0..=rings {
        let lat = std::f32::consts::FRAC_PI_2 * (1.0 - r as f32 / rings as f32);
        for s in 0..=sectors {
            let lon = std::f32::consts::TAU * s as f32 / sectors as f32;
            let n = Vec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin());
            let flare = if r == rings { 1.06 } else { 1.0 };
            pos.push([n.x * flare, n.y, n.z * flare]);
            nrm.push(n.to_array());
            uv.push([s as f32 / sectors as f32, r as f32 / rings as f32]);
        }
    }
    let mut idx = Vec::new();
    let w = sectors + 1;
    for r in 0..rings {
        for s in 0..sectors {
            let (a, b) = (r * w + s, (r + 1) * w + s);
            idx.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

fn merge(parts: impl IntoIterator<Item = (Mesh, Transform)>) -> Mesh {
    let mut out: Option<Mesh> = None;
    for (m, t) in parts {
        let m = m.transformed_by(t);
        match &mut out {
            None => out = Some(m),
            Some(o) => o.merge(&m).expect("kit meshes share a layout"),
        }
    }
    out.expect("at least one part")
}

fn cuboid(x: f32, y: f32, z: f32) -> Mesh {
    Cuboid::new(x, y, z).mesh().build()
}

/// Helmet in the character frame (metres; -Z forward), origin at the head's centre line: a
/// dome, side rails and an NVG mount on the front.
fn helmet_mesh() -> Mesh {
    let t = |x: f32, y: f32, z: f32| Transform::from_xyz(x, y, z);
    merge([
        (dome(5, 14), Transform::from_scale(Vec3::new(0.135, 0.125, 0.145))),
        // Rails along both sides.
        (cuboid(0.012, 0.02, 0.12), t(0.13, 0.02, 0.0)),
        (cuboid(0.012, 0.02, 0.12), t(-0.13, 0.02, 0.0)),
        // NVG shroud on the brow and the counterweight at the back.
        (cuboid(0.05, 0.035, 0.03), t(0.0, 0.06, -0.14)),
        (cuboid(0.07, 0.04, 0.035), t(0.0, 0.05, 0.14)),
    ])
}

/// Plate carrier in the character frame, origin at the chest bone: front and back plates,
/// shoulder straps, a row of magazine pouches, and the variant's pack.
fn vest_mesh(pack: Pack) -> Mesh {
    let t = |x: f32, y: f32, z: f32| Transform::from_xyz(x, y, z);
    let mut parts = vec![
        // Front and back plates (wider than deep: they sit on the chest, not inside it).
        (cuboid(0.32, 0.34, 0.06), t(0.0, -0.02, -0.12)),
        (cuboid(0.32, 0.36, 0.05), t(0.0, -0.01, 0.12)),
        // Cummerbund round the sides.
        (cuboid(0.07, 0.2, 0.22), t(0.16, -0.08, 0.0)),
        (cuboid(0.07, 0.2, 0.22), t(-0.16, -0.08, 0.0)),
        // Shoulder straps.
        (cuboid(0.07, 0.03, 0.26), t(0.1, 0.16, 0.0)),
        (cuboid(0.07, 0.03, 0.26), t(-0.1, 0.16, 0.0)),
    ];
    // Three magazine pouches across the front, and an admin pouch above.
    for x in [-0.09, 0.0, 0.09] {
        parts.push((cuboid(0.075, 0.11, 0.045), t(x, -0.11, -0.165)));
    }
    parts.push((cuboid(0.14, 0.07, 0.03), t(0.0, 0.05, -0.16)));
    match pack {
        Pack::Backpack => parts.push((cuboid(0.28, 0.36, 0.15), t(0.0, -0.06, 0.22))),
        Pack::Radio => {
            parts.push((cuboid(0.11, 0.2, 0.08), t(-0.08, -0.04, 0.18)));
            parts.push((cuboid(0.012, 0.32, 0.012), t(-0.11, 0.2, 0.2)));
        }
        Pack::None => parts.push((cuboid(0.2, 0.12, 0.06), t(0.0, -0.15, 0.17))),
    }
    merge(parts)
}

fn owner(mut e: Entity, parents: &Query<&ChildOf>, enemies: &Query<(), With<Enemy>>) -> Option<Entity> {
    while let Ok(p) = parents.get(e) {
        e = p.parent();
        if enemies.contains(e) {
            return Some(e);
        }
    }
    None
}

/// Note the kit's bones as the enemy's scene spawns.
fn find_bones(
    mut commands: Commands,
    names: Query<(Entity, &Name), Added<Name>>,
    parents: Query<&ChildOf>,
    enemies: Query<(), With<Enemy>>,
    mut rigs: Query<&mut Rig>,
    mut fresh: Local<HashMap<Entity, Rig>>,
) {
    for (e, name) in &names {
        let n = name.as_str();
        let slot = |r: &mut Rig| match n {
            _ if HEADS.contains(&n) => r.head = Some(e),
            _ if CHESTS.contains(&n) => {
                let rank = CHESTS.iter().position(|c| *c == n).unwrap_or(0) + 1;
                if rank > r.chest_rank {
                    r.chest = Some(e);
                    r.chest_rank = rank;
                }
            }
            ARM_L => r.arm = Some(e),
            _ if n.starts_with("body_lod") => {
                if r.body.is_none() {
                    r.body = Some(e);
                }
            }
            _ => {}
        };
        let pair = |r: &mut Rig| {
            let lists: [(&[&str; 2], &mut [Option<Entity>; 2]); 7] = [
                (&HANDS, &mut r.hands),
                (&CALVES, &mut r.calves),
                (&THIGHS, &mut r.thighs),
                (&UPPERARMS, &mut r.upperarms),
                (&LOWERARMS, &mut r.lowerarms),
                (&FEET, &mut r.feet),
                (&BALLS, &mut r.balls),
            ];
            for (names, slots) in lists {
                if let Some(i) = names.iter().position(|h| *h == n) {
                    slots[i] = Some(e);
                }
            }
        };
        let paired = [HANDS, CALVES, THIGHS, UPPERARMS, LOWERARMS, FEET, BALLS].iter().any(|l| l.contains(&n));
        if !HEADS.contains(&n) && !CHESTS.contains(&n) && n != ARM_L && !paired && !n.starts_with("body_lod") {
            continue;
        }
        let Some(root) = owner(e, &parents, &enemies) else { continue };
        match rigs.get_mut(root) {
            Ok(mut r) => {
                slot(&mut r);
                pair(&mut r);
            }
            Err(_) => {
                let r = fresh.entry(root).or_default();
                slot(r);
                pair(r);
            }
        }
    }
    for (root, rig) in fresh.drain() {
        if let Ok(mut ec) = commands.get_entity(root) {
            ec.insert(rig);
        }
    }
}

/// The loaded skinned-gear file's parts.
#[derive(SystemParam)]
struct GearFiles<'w> {
    server: Res<'w, AssetServer>,
    gltfs: Res<'w, Assets<Gltf>>,
    nodes: Res<'w, Assets<GltfNode>>,
    meshes: Res<'w, Assets<GltfMesh>>,
    skins: Res<'w, Assets<GltfSkin>>,
}

/// Whether the skinned gear is ready (Some(true)), still loading (None), or unusable.
fn gear_ready(kit: &Kit, files: &GearFiles) -> Option<bool> {
    let h = kit.gear_mh.as_ref()?;
    match files.server.load_state(h) {
        bevy::asset::LoadState::Loaded => Some(files.gltfs.get(h).is_some()),
        bevy::asset::LoadState::Failed(_) => Some(false),
        _ => None,
    }
}

/// `<part>.<material>[.NNN]` (Blender object names) -> (part, material).
fn gear_name(n: &str) -> Option<(&str, &str)> {
    let mut it = n.split('.');
    Some((it.next()?, it.next()?))
}

/// The squad look's material for a gear piece.
fn gear_material<'a>(kit: &'a Kit, part: &str, mat: &str, v: usize) -> Option<&'a Handle<StandardMaterial>> {
    let g = &kit.gear_mats;
    Some(match mat {
        // Shirt and trousers keep the body's UVs: the body's fatigues material.
        "shirt" | "trousers" => &kit.fatigues_mh[v],
        "trousers_pocket" => &g.cloth_box[v],
        "leather" if part == "gloves" => &g.leather_mh,
        "leather" => &g.leather_box,
        "cordura" => &g.cordura_box[v],
        "helmet" => &g.helmet_box[v],
        "webbing" => &g.webbing,
        "balaclava" => &g.balaclava,
        "sole" => &g.sole,
        "metal" => &g.metal,
        "pads" => &g.pads,
        _ => return None,
    })
}

/// Spawn every skinned gear piece for squad look `v` beside the body mesh (`parent`), bound to
/// the enemy's bones (`bones`: name -> entity). False if a piece can't be bound.
fn spawn_gear_mh(commands: &mut Commands, kit: &Kit, files: &GearFiles, v: usize, bones: &HashMap<String, Entity>, parent: Entity) -> bool {
    let Some(gltf) = kit.gear_mh.as_ref().and_then(|h| files.gltfs.get(h)) else { return false };
    let pack = match looks()[v].pack {
        Pack::Backpack => "pack_backpack",
        Pack::Radio => "pack_radio",
        Pack::None => "pack_none",
    };
    let mut spawned = 0;
    for nh in &gltf.nodes {
        let Some(node) = files.nodes.get(nh) else { continue };
        let (Some(mh), Some(sh)) = (&node.mesh, &node.skin) else { continue };
        let Some((part, m)) = gear_name(&node.name) else { continue };
        if part.starts_with("pack_") && part != pack {
            continue;
        }
        let (Some(mat), Some(mesh), Some(skin)) = (gear_material(kit, part, m, v), files.meshes.get(mh), files.skins.get(sh)) else {
            warn!("enemy gear: skipping {}", node.name);
            continue;
        };
        let joints: Option<Vec<Entity>> = skin.joints.iter().map(|j| files.nodes.get(j).and_then(|n| bones.get(&n.name).copied())).collect();
        let Some(joints) = joints else {
            warn!("enemy gear: {} has joints the enemy lacks", node.name);
            return false;
        };
        for prim in &mesh.primitives {
            commands.spawn((
                Name::new("kit"),
                Gear,
                Mesh3d(prim.mesh.clone()),
                MeshMaterial3d(mat.clone()),
                SkinnedMesh { inverse_bindposes: skin.inverse_bind_matrices.clone(), joints: joints.clone() },
                Transform::default(),
                ChildOf(parent),
            ));
            spawned += 1;
        }
    }
    spawned > 0
}

/// Every named entity under `root` (the enemy's bones).
fn bone_names(root: Entity, children: &Query<&Children>, names: &Query<&Name>) -> HashMap<String, Entity> {
    let mut out = HashMap::new();
    for e in children.iter_descendants(root) {
        if let Ok(n) = names.get(e) {
            out.entry(n.as_str().to_owned()).or_insert(e);
        }
    }
    out
}

/// Hang the kit on the bones once the rig has a pose.
#[allow(clippy::too_many_arguments)]
fn dress(
    mut commands: Commands,
    kit: Res<Kit>,
    files: GearFiles,
    mut rigs: Query<(Entity, &Enemy, &mut Rig, &GlobalTransform), Without<Dressed>>,
    globals: Query<&GlobalTransform>,
    children: Query<&Children>,
    locals: Query<&Transform>,
    names: Query<&Name>,
) {
    // A bone's direction and length in its own space: towards its farthest child (UAL bones run
    // along +Y, MetaHuman bones along ±X).
    let axis = |bone: Entity| -> (Vec3, f32) {
        let far = children
            .get(bone)
            .ok()
            .and_then(|c| c.iter().filter_map(|c| locals.get(c).ok().map(|t| t.translation)).max_by(|a, b| a.length().total_cmp(&b.length())));
        match far {
            Some(t) if t.length() > 1e-4 => (t.normalize(), t.length()),
            _ => (Vec3::Y, 0.0),
        }
    };
    // Along the bone: `along` metres from its origin (or `frac` of its length when it has one),
    // a unit mesh's +Y on the bone's axis.
    let on_bone = |bone: Entity, along: f32, frac: Option<f32>, scale: Vec3| -> Transform {
        let (dir, len) = axis(bone);
        let d = match frac {
            Some(f) if len > 0.0 => f * len,
            _ => along,
        };
        Transform::from_translation(dir * d).with_rotation(Quat::from_rotation_arc(Vec3::Y, dir)).with_scale(scale)
    };
    for (e, enemy, mut rig, root) in &mut rigs {
        let (Some(head), Some(chest)) = (rig.head, rig.chest) else { continue };
        rig.frames = rig.frames.saturating_add(1);
        if rig.frames < SETTLE_FRAMES {
            continue;
        }
        let v = enemy.slot % 3;
        let mh = rig.chest_rank == CHESTS.len();
        // MetaHuman with the skinned gear: real clothing instead of the primitive kit.
        let mut skinned = false;
        if mh && kit.gear_mh.is_some() {
            match gear_ready(&kit, &files) {
                None => continue,
                Some(true) => {
                    if let Some(body) = rig.body {
                        let bones = bone_names(e, &children, &names);
                        skinned = spawn_gear_mh(&mut commands, &kit, &files, v, &bones, body);
                    }
                }
                Some(false) => {}
            }
        }
        if skinned {
            // The team armband still goes on, over the sleeve.
            if let Some(arm) = rig.arm {
                let (dir, _) = axis(arm);
                let tf = Transform::from_translation(dir * 0.12).with_rotation(Quat::from_rotation_arc(Vec3::Y, dir)).with_scale(Vec3::new(0.068, 0.05, 0.068));
                commands.spawn((Name::new("kit"), Gear, Mesh3d(kit.band.clone()), MeshMaterial3d(kit.accent.clone()), tf, ChildOf(arm)));
            }
            commands.entity(e).insert(Dressed);
            continue;
        }
        // Helmet and vest: placed in the character's frame (upright, facing its forward) at the
        // bone's posed position, then expressed in the bone's space.
        let upright = |bone: Entity, local: Vec3| -> Option<Transform> {
            let g = globals.get(bone).ok()?;
            let at = g.translation() + root.rotation() * local;
            let world = GlobalTransform::from(Transform::from_translation(at).with_rotation(root.rotation()));
            Some(world.reparented_to(g))
        };
        let mut put = |bone: Entity, tf: Transform, mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>| {
            commands.spawn((Name::new("kit"), Gear, Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), tf, ChildOf(bone)));
        };
        // The MetaHuman's head bone sits lower in the skull than the mannequin's.
        let helmet_y = if rig.chest_rank == CHESTS.len() { 0.07 } else { 0.1 };
        if let Some(tf) = upright(head, Vec3::new(0.0, helmet_y, 0.005)) {
            put(head, tf, &kit.helmet, &kit.helmet_mat[v]);
        }
        if let Some(tf) = upright(chest, Vec3::new(0.0, 0.02, -0.01)) {
            put(chest, tf, &kit.vest[v], &kit.gear[v]);
        }
        // Gloves along the hand bones: a mitt from the wrist over the fingers, flat across the
        // palm (its width toward the thumb).
        for hand in rig.hands.into_iter().flatten() {
            let (dir, _) = axis(hand);
            let thumb = children
                .get(hand)
                .ok()
                .and_then(|c| c.iter().find(|c| names.get(*c).is_ok_and(|n| n.as_str().starts_with("thumb_01"))))
                .and_then(|t| locals.get(t).ok())
                .map(|t| t.translation.reject_from(dir));
            let x = thumb.and_then(|t| t.try_normalize()).unwrap_or_else(|| dir.any_orthonormal_vector());
            let rot = Quat::from_mat3(&Mat3::from_cols(x, dir, x.cross(dir)));
            put(hand, Transform::from_rotation(rot).with_scale(Vec3::new(0.078, 0.15, 0.062)), &kit.glove, &kit.leather);
        }
        // Trouser legs and sleeves: loose tubes along the limb bones, over the body's uniform.
        let limbs = [rig.thighs, rig.calves, rig.upperarms, rig.lowerarms];
        for (i, bones) in limbs.into_iter().enumerate() {
            let c = CLOTHS[i];
            for bone in bones.into_iter().flatten() {
                let (dir, len) = axis(bone);
                if len <= 0.0 {
                    continue;
                }
                let l = (c.to - c.from) * len;
                let tf = Transform::from_translation(dir * c.from * len)
                    .with_rotation(Quat::from_rotation_arc(Vec3::Y, dir))
                    .with_scale(Vec3::new(c.r0, l, c.r0));
                put(bone, tf, &kit.tubes[i], &kit.cloth[v]);
            }
        }
        // Boots: built in the character's frame from the idle pose (ankle above the ball of the
        // foot, toe ahead of it), then parented to the foot so they follow the animation.
        for (foot, ball) in rig.feet.into_iter().zip(rig.balls) {
            let (Some(foot), Some(ball)) = (foot, ball) else { continue };
            let (Ok(f), Ok(b)) = (globals.get(foot), globals.get(ball)) else { continue };
            let (ankle, toe) = (f.translation(), b.translation());
            let up = Vec3::Y;
            let ahead = (toe - ankle).reject_from(up);
            let Some(fwd) = ahead.try_normalize() else { continue };
            // The boot mesh is posed for an ankle 0.085 m up and the ball 0.14 m ahead.
            let height = ((ankle - toe).dot(up) + 0.03).clamp(0.05, 0.14);
            let length = ahead.length().clamp(0.08, 0.2);
            let scale = Vec3::new(1.0, height / 0.085, length / 0.14);
            let world = GlobalTransform::from(Transform::from_translation(ankle).looking_to(fwd, up).with_scale(scale));
            put(foot, world.reparented_to(f), &kit.boot, &kit.leather);
        }
        if let Some(arm) = rig.arm {
            put(arm, on_bone(arm, 0.12, None, Vec3::new(0.062, 0.06, 0.062)), &kit.band, &kit.accent);
        }
        commands.entity(e).insert(Dressed);
    }
}

/// The mannequin's own materials become the variant's fatigues and base layer (one material per
/// variant and source, shared by every enemy).
fn recolour(
    mut commands: Commands,
    kit: Res<Kit>,
    added: Query<(Entity, &MeshMaterial3d<StandardMaterial>), (Added<MeshMaterial3d<StandardMaterial>>, Without<Gear>)>,
    parents: Query<&ChildOf>,
    enemies: Query<&Enemy>,
    guns: Query<(), With<super::gun::EnemyGun>>,
    names: Query<&Name>,
    materials: Res<Assets<StandardMaterial>>,
) {
    for (e, m) in &added {
        // MetaHuman parts by mesh name (on the entity or its node).
        let named = |prefix: &str| {
            names.get(e).is_ok_and(|n| n.as_str().starts_with(prefix))
                || parents.get(e).ok().and_then(|p| names.get(p.parent()).ok()).is_some_and(|n| n.as_str().starts_with(prefix))
        };
        let mh_body = named("body_lod") || named("combined_lod");
        let mh_other = ["head_lod", "teeth_lod", "eyeLeft_lod", "eyeRight_lod", "eyelashes_lod"].iter().any(|p| named(p));
        let mut a = e;
        let mut enemy = None;
        let mut gun = false;
        while let Ok(p) = parents.get(a) {
            a = p.parent();
            gun |= guns.contains(a);
            if let Ok(en) = enemies.get(a) {
                enemy = Some(en.slot % 3);
                break;
            }
        }
        let Some(v) = enemy else { continue };
        if gun || mh_other || kit.fatigues.contains(&m.0) || kit.fatigues_mh.contains(&m.0) || kit.base.contains(&m.0) {
            continue;
        }
        if mh_body {
            if m.0 != kit.fatigues_mh[v] {
                commands.entity(e).insert(MeshMaterial3d(kit.fatigues_mh[v].clone()));
            }
            continue;
        }
        // M_Main is the light (orange) body, M_Joints the darker segments.
        let joints = materials.get(&m.0).is_some_and(|s| {
            let c = s.base_color.to_srgba();
            c.blue > c.green
        });
        let want = if joints { &kit.base[v] } else { &kit.fatigues[v] };
        if m.0 != *want {
            commands.entity(e).insert(MeshMaterial3d(want.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gear_names_split_into_part_and_material() {
        assert_eq!(gear_name("helmet.webbing.001"), Some(("helmet", "webbing")));
        assert_eq!(gear_name("pack_radio.webbing"), Some(("pack_radio", "webbing")));
        assert_eq!(gear_name("loose"), None);
    }
}
