//! Gun finishes: the GLB materials of every gun scene (first person, enemy hands, pickups) are
//! swapped at load for grounded firearm materials: blued/parkerized steel, black polymer, walnut,
//! brass, coated glass. Each gun gets its own furniture colour (black, OD green, FDE tan) so the
//! arsenal doesn't read as one gun in different sizes.
//!
//! The models (Pichuliru's CC0 gun set) ship flat colours named `M_PCL_Flat_<colour>`; the
//! mapping goes by that name (glTF material name), with a darkened fallback for anything else
//! (e.g. the older Kenney blasters, if a file is swapped back). One material per (gun palette,
//! source name), shared by every instance, so finishes cost no extra draw calls.
//!
//! Steel and polymer are photoscanned sets (`textures/gun_metal`: black powder-coated steel with
//! scratches; `textures/gun_polymer`: scuffed black plastic; ambientCG, CC0). The models' UVs
//! point into a colour palette, so their meshes get box-projected UVs at load ([`MeshPrep`]),
//! one texture tile per [`TILE_M`] of gun. Coatings (anodising, Cerakote, polymer) are dielectric:
//! only bare worn parts are metallic.

use std::collections::HashMap;

use bevy::gltf::GltfMaterialName;
use bevy::prelude::*;

use super::WeaponKind;
use crate::assets::AssetsDir;
use crate::meshprep::MeshPrep;

/// Real-world size of one texture tile on a gun (m).
const TILE_M: f32 = 0.22;
/// Linear albedo averages of the sets' colour maps (`color.avg`), divided out of the tints.
const METAL_AVG: f32 = 0.015;
const POLYMER_AVG: f32 = 0.018;

/// The photoscanned base materials (untinted), when the sets are present.
#[derive(Resource, Default)]
struct GunSets {
    metal: Option<StandardMaterial>,
    polymer: Option<StandardMaterial>,
}

fn load_sets(mut commands: Commands, server: Option<Res<AssetServer>>, dir: Option<Res<AssetsDir>>, pending: Option<ResMut<crate::gfx::PendingMips>>) {
    let mut sets = GunSets::default();
    if let (Some(s), Some(d), Some(mut p)) = (server, dir, pending) {
        sets.metal = crate::gfx::load_pbr(&s, &d, &mut p, "gun_metal", Color::WHITE);
        sets.polymer = crate::gfx::load_pbr(&s, &d, &mut p, "gun_polymer", Color::WHITE);
    }
    commands.insert_resource(sets);
}

/// On a gun scene root (or an optic mounted on it): its meshes get `kind`'s finish.
/// `mesh_m`: metres per mesh unit (Pichuliru's rigs carry a 100x scale; generated LODs are in
/// metres), so the photoscan tiles at [`TILE_M`] on every tier.
#[derive(Component, Clone, Copy)]
pub struct GunFinish {
    pub kind: WeaponKind,
    pub mesh_m: f32,
}

/// On a gun scene root whose model keeps its own (pack) materials: scale every material under it
/// (`guns.json` `albedo` / `roughness`). Downloaded packs are often authored bright and glossy
/// (a polished silver receiver): under the bodycam's interior exposure their metal mirrored the
/// sky probe and clipped. A coated or blued gun reflects little (F0 ~0.05-0.1) and is satin.
/// One tuned copy per source material, shared by every instance; the files aren't touched.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct PackFinish {
    pub albedo: f32,
    pub roughness: f32,
}

/// A pack material with its `PackFinish` scales applied.
pub fn tuned(m: &StandardMaterial, t: PackFinish) -> StandardMaterial {
    let c = m.base_color.to_linear();
    StandardMaterial {
        base_color: LinearRgba::new(c.red * t.albedo, c.green * t.albedo, c.blue * t.albedo, c.alpha).into(),
        // A factor on the texture's roughness, so it may exceed 1 (the shader clamps the product).
        perceptual_roughness: (m.perceptual_roughness * t.roughness).clamp(0.089, 4.0),
        ..m.clone()
    }
}

pub(super) fn build(app: &mut App) {
    app.add_systems(Startup, load_sets.run_if(crate::state::has_visuals))
        .add_systems(Update, apply_finish.run_if(crate::state::has_visuals).run_if(resource_exists::<GunSets>))
        .add_systems(Update, apply_pack_finish.run_if(crate::state::has_visuals));
}

#[allow(clippy::type_complexity)]
fn apply_pack_finish(
    mut commands: Commands,
    added: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Added<MeshMaterial3d<StandardMaterial>>>,
    parents: Query<&ChildOf>,
    roots: Query<&PackFinish>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: Local<HashMap<(AssetId<StandardMaterial>, u32, u32), Handle<StandardMaterial>>>,
    mut ours: Local<Vec<AssetId<StandardMaterial>>>,
) {
    for (e, mat) in &added {
        if ours.contains(&mat.id()) {
            continue;
        }
        let Some(t) = parents.iter_ancestors(e).find_map(|a| roots.get(a).ok()).copied() else { continue };
        let key = (mat.id(), t.albedo.to_bits(), t.roughness.to_bits());
        let handle = match cache.get(&key) {
            Some(h) => h.clone(),
            None => {
                let Some(src) = materials.get(&mat.0) else { continue };
                let new = tuned(src, t);
                let h = materials.add(new);
                ours.push(h.id());
                cache.insert(key, h.clone());
                h
            }
        };
        commands.entity(e).insert(MeshMaterial3d(handle));
    }
}

/// Furniture (polymer stock/grip/handguard) colour per gun.
fn furniture(kind: WeaponKind) -> Color {
    match kind {
        // Flat dark earth on the scout, OD green on the heavy sniper.
        WeaponKind::Marksman => Color::srgb(0.36, 0.30, 0.21),
        WeaponKind::Sniper => Color::srgb(0.17, 0.19, 0.13),
        // A two-tone Warden: tan frame under a black slide.
        WeaponKind::HeavyPistol => Color::srgb(0.33, 0.28, 0.2),
        _ => Color::srgb(0.15, 0.15, 0.155),
    }
}

/// Which photoscanned set a finish takes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Set {
    Flat,
    Metal,
    Polymer,
}

/// The finish for a source material name (None: keep the source but darken it), and the set it
/// is textured with when present.
fn finish(kind: WeaponKind, name: &str) -> Option<(StandardMaterial, Set)> {
    let m = |c: Color, metallic: f32, rough: f32| StandardMaterial { base_color: c, metallic, perceptual_roughness: rough, reflectance: 0.5, ..default() };
    let n = name.trim_start_matches("M_PCL_Flat_");
    let set = match n {
        "Black" | "Grey_Darker" | "Grey_Dark" => Set::Metal,
        "Black_Lighter" => Set::Polymer,
        _ => Set::Flat,
    };
    Some((match n {
        // Receiver, barrel, slide: black coated steel / anodised aluminium, a dark body with a
        // satin sheen (Cerakote / anodising are dielectric, roughness ~0.5): under a strong lamp
        // the energy goes into a highlight instead of a grey matte wash. Albedos 0.025-0.035
        // linear (tells-perf's lamp measurements, `light.viewmodel`).
        "Black" => m(Color::LinearRgba(LinearRgba::rgb(0.026, 0.026, 0.028)), 0.0, 0.5),
        // Polymer furniture: stays rough.
        "Black_Lighter" => m(furniture(kind), 0.0, 0.75),
        "Grey_Darker" => m(Color::LinearRgba(LinearRgba::rgb(0.03, 0.03, 0.032)), 0.0, 0.5),
        "Grey_Dark" => m(Color::LinearRgba(LinearRgba::rgb(0.035, 0.035, 0.037)), 0.0, 0.55),
        // Worn edges, bolt, small parts.
        "Grey_Light" | "Grey_Lighter" => m(Color::srgb(0.32, 0.32, 0.33), 0.9, 0.32),
        "White" => m(Color::srgb(0.55, 0.55, 0.52), 0.0, 0.6),
        "Yellow.Brass" => m(Color::srgb(0.72, 0.55, 0.25), 1.0, 0.3),
        // Wood furniture: oiled walnut.
        "Brown.Chestnut" => m(Color::srgb(0.27, 0.12, 0.055), 0.0, 0.5),
        "Brown.Bole" => m(Color::srgb(0.2, 0.1, 0.06), 0.0, 0.55),
        "Red.Brick" | "Red.Rosewood" => m(Color::srgb(0.22, 0.04, 0.03), 0.0, 0.6),
        // Lenses: coated, dark, glossy.
        "Glass" => m(Color::srgb(0.03, 0.045, 0.06), 0.3, 0.05),
        "Blue.Cerulean" => m(Color::srgb(0.05, 0.09, 0.18), 0.3, 0.08),
        _ => return None,
    }, set))
}

/// A flat finish on its photoscanned set: the set's maps, the finish's colour (the set's average
/// albedo divided out) and roughness scale.
fn textured(flat: StandardMaterial, base: &StandardMaterial, avg: f32) -> StandardMaterial {
    let c = flat.base_color.to_linear();
    StandardMaterial {
        base_color: LinearRgba::rgb(c.red / avg, c.green / avg, c.blue / avg).into(),
        metallic: flat.metallic,
        perceptual_roughness: flat.perceptual_roughness,
        reflectance: flat.reflectance,
        ..base.clone()
    }
}

#[allow(clippy::type_complexity)]
fn apply_finish(
    mut commands: Commands,
    added: Query<(Entity, &MeshMaterial3d<StandardMaterial>, Option<&GltfMaterialName>), Added<MeshMaterial3d<StandardMaterial>>>,
    parents: Query<&ChildOf>,
    roots: Query<&GunFinish>,
    sets: Res<GunSets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: Local<HashMap<(WeaponKind, String), (Handle<StandardMaterial>, bool)>>,
    mut ours: Local<Vec<AssetId<StandardMaterial>>>,
) {
    for (e, mat, name) in &added {
        if ours.contains(&mat.id()) {
            continue;
        }
        let Some(root) = parents.iter_ancestors(e).find_map(|a| roots.get(a).ok()).copied() else { continue };
        let kind = root.kind;
        let key = (kind, name.map_or_else(|| format!("#{:?}", mat.id()), |n| n.0.clone()));
        let (handle, tex) = match cache.get(&key) {
            Some(h) => h.clone(),
            None => {
                let mut tex = false;
                let new = match name.and_then(|n| finish(kind, &n.0)) {
                    Some((f, set)) => match (set, &sets.metal, &sets.polymer) {
                        (Set::Metal, Some(b), _) => {
                            tex = true;
                            textured(f, b, METAL_AVG)
                        }
                        (Set::Polymer, _, Some(b)) => {
                            tex = true;
                            textured(f, b, POLYMER_AVG)
                        }
                        _ => f,
                    },
                    None => {
                        // Unknown source: keep its texture, pull it toward dark gunmetal.
                        let Some(orig) = materials.get(&mat.0).cloned() else { continue };
                        let c = orig.base_color.to_linear();
                        StandardMaterial {
                            base_color: LinearRgba::rgb(c.red * 0.18 + 0.01, c.green * 0.18 + 0.01, c.blue * 0.18 + 0.012).into(),
                            metallic: 0.6,
                            perceptual_roughness: 0.45,
                            ..orig
                        }
                    }
                };
                let h = materials.add(new);
                ours.push(h.id());
                cache.insert(key, (h.clone(), tex));
                (h, tex)
            }
        };
        let mut ec = commands.entity(e);
        ec.insert(MeshMaterial3d(handle));
        if tex {
            ec.insert(MeshPrep::BoxUv { per_unit: root.mesh_m / TILE_M });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_finish_scales_colour_and_roughness_within_range() {
        let m = StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.9, metallic: 1.0, ..default() };
        let t = tuned(&m, PackFinish { albedo: 0.5, roughness: 1.3 });
        assert!((t.base_color.to_linear().red - 0.5).abs() < 1e-6);
        assert!((t.perceptual_roughness - 1.17).abs() < 1e-5);
        assert_eq!(t.metallic, 1.0);
    }

    #[test]
    fn known_materials_get_finishes() {
        for n in ["M_PCL_Flat_Black", "M_PCL_Flat_Black_Lighter", "M_PCL_Flat_Brown.Chestnut", "M_PCL_Flat_Glass"] {
            assert!(finish(WeaponKind::Rifle, n).is_some(), "{n}");
        }
        assert!(finish(WeaponKind::Rifle, "colormap").is_none());
        // Furniture differs per gun; steel doesn't.
        let a = finish(WeaponKind::Sniper, "M_PCL_Flat_Black_Lighter").unwrap().0.base_color;
        let b = finish(WeaponKind::Rifle, "M_PCL_Flat_Black_Lighter").unwrap().0.base_color;
        assert_ne!(a, b);
        // Steel and polymer take the photoscanned sets; lenses and wood stay flat.
        assert_eq!(finish(WeaponKind::Rifle, "M_PCL_Flat_Black").unwrap().1, Set::Metal);
        assert_eq!(finish(WeaponKind::Rifle, "M_PCL_Flat_Black_Lighter").unwrap().1, Set::Polymer);
        assert_eq!(finish(WeaponKind::Rifle, "M_PCL_Flat_Glass").unwrap().1, Set::Flat);
    }
}
