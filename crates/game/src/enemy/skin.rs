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

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::Enemy;
use crate::state::has_visuals;

/// Bones the kit hangs on (Quaternius UAL names).
const HEAD: &str = "Head";
const CHEST: &str = "spine_03";
const HANDS: [&str; 2] = ["hand_l", "hand_r"];
const CALVES: [&str; 2] = ["calf_l", "calf_r"];
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

/// Shared skin assets.
#[derive(Resource)]
struct Kit {
    fatigues: [Handle<StandardMaterial>; 3],
    base: [Handle<StandardMaterial>; 3],
    gear: [Handle<StandardMaterial>; 3],
    helmet_mat: [Handle<StandardMaterial>; 3],
    black: Handle<StandardMaterial>,
    accent: Handle<StandardMaterial>,
    helmet: Handle<Mesh>,
    vest: [Handle<Mesh>; 3],
    glove: Handle<Mesh>,
    boot: Handle<Mesh>,
    band: Handle<Mesh>,
}

/// Bones found under an enemy, until it is dressed.
#[derive(Component, Default)]
struct Rig {
    head: Option<Entity>,
    chest: Option<Entity>,
    hands: [Option<Entity>; 2],
    calves: [Option<Entity>; 2],
    arm: Option<Entity>,
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

fn mat(materials: &mut Assets<StandardMaterial>, c: Color, rough: f32) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial { base_color: c, perceptual_roughness: rough, reflectance: 0.3, ..default() })
}

/// UV repeats of the ripstop weave across the mannequin's texture atlas (a fine weave, not a
/// pattern you notice at range).
const FABRIC_REPEAT: f32 = 4.0;

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
    let base = std::array::from_fn(|i| mat(&mut materials, l[i].base, 0.8));
    let gear = std::array::from_fn(|i| mat(&mut materials, l[i].gear, 0.75));
    let helmet_mat = std::array::from_fn(|i| mat(&mut materials, l[i].helmet, 0.6));
    let black = mat(&mut materials, Color::srgb(0.03, 0.03, 0.035), 0.7);
    // Team colour: a red armband with a little glow so the squad reads at 30 m.
    let accent = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.05, 0.03),
        emissive: LinearRgba::rgb(1.6, 0.08, 0.03),
        perceptual_roughness: 0.6,
        ..default()
    });
    let vest = std::array::from_fn(|i| meshes.add(vest_mesh(l[i].pack)));
    commands.insert_resource(Kit {
        fatigues,
        base,
        gear,
        helmet_mat,
        black,
        accent,
        helmet: meshes.add(helmet_mesh()),
        vest,
        glove: meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap()),
        boot: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(10).segments(1)),
        band: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(12).segments(1)),
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
            HEAD => r.head = Some(e),
            CHEST => r.chest = Some(e),
            ARM_L => r.arm = Some(e),
            _ => {
                if let Some(i) = HANDS.iter().position(|h| *h == n) {
                    r.hands[i] = Some(e);
                } else if let Some(i) = CALVES.iter().position(|c| *c == n) {
                    r.calves[i] = Some(e);
                }
            }
        };
        if n != HEAD && n != CHEST && n != ARM_L && !HANDS.contains(&n) && !CALVES.contains(&n) {
            continue;
        }
        let Some(root) = owner(e, &parents, &enemies) else { continue };
        match rigs.get_mut(root) {
            Ok(mut r) => slot(&mut r),
            Err(_) => slot(fresh.entry(root).or_default()),
        }
    }
    for (root, rig) in fresh.drain() {
        if let Ok(mut ec) = commands.get_entity(root) {
            ec.insert(rig);
        }
    }
}

/// Hang the kit on the bones once the rig has a pose.
fn dress(
    mut commands: Commands,
    kit: Res<Kit>,
    mut rigs: Query<(Entity, &Enemy, &mut Rig, &GlobalTransform), Without<Dressed>>,
    globals: Query<&GlobalTransform>,
) {
    for (e, enemy, mut rig, root) in &mut rigs {
        let (Some(head), Some(chest)) = (rig.head, rig.chest) else { continue };
        rig.frames = rig.frames.saturating_add(1);
        if rig.frames < SETTLE_FRAMES {
            continue;
        }
        let v = enemy.slot % 3;
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
        if let Some(tf) = upright(head, Vec3::new(0.0, 0.1, 0.005)) {
            put(head, tf, &kit.helmet, &kit.helmet_mat[v]);
        }
        if let Some(tf) = upright(chest, Vec3::new(0.0, 0.02, -0.01)) {
            put(chest, tf, &kit.vest[v], &kit.gear[v]);
        }
        // Limbs in bone space: the bones run along +Y.
        for hand in rig.hands.into_iter().flatten() {
            put(hand, Transform::from_xyz(0.0, 0.05, 0.0).with_scale(Vec3::new(0.05, 0.075, 0.05)), &kit.glove, &kit.black);
        }
        for calf in rig.calves.into_iter().flatten() {
            put(calf, Transform::from_xyz(0.0, 0.37, 0.0).with_scale(Vec3::new(0.066, 0.13, 0.066)), &kit.boot, &kit.black);
        }
        if let Some(arm) = rig.arm {
            put(arm, Transform::from_xyz(0.0, 0.12, 0.0).with_scale(Vec3::new(0.062, 0.06, 0.062)), &kit.band, &kit.accent);
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
    materials: Res<Assets<StandardMaterial>>,
) {
    for (e, m) in &added {
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
        if gun || kit.fatigues.contains(&m.0) || kit.base.contains(&m.0) {
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
