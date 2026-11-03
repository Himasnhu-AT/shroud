//! Gun finishes: the GLB materials of every gun scene (first person, enemy hands, pickups) are
//! swapped at load for grounded firearm materials: blued/parkerized steel, black polymer, walnut,
//! brass, coated glass. Each gun gets its own furniture colour (black, OD green, FDE tan) so the
//! arsenal doesn't read as one gun in different sizes.
//!
//! The models (Pichuliru's CC0 gun set) ship flat colours named `M_PCL_Flat_<colour>`; the
//! mapping goes by that name (glTF material name), with a darkened fallback for anything else
//! (e.g. the older Kenney blasters, if a file is swapped back). One material per (gun palette,
//! source name), shared by every instance, so finishes cost no extra draw calls.

use std::collections::HashMap;

use bevy::gltf::GltfMaterialName;
use bevy::prelude::*;

use super::WeaponKind;

/// On a gun scene root (or an optic mounted on it): its meshes get `kind`'s finish.
#[derive(Component, Clone, Copy)]
pub struct GunFinish(pub WeaponKind);

pub(super) fn build(app: &mut App) {
    app.add_systems(Update, apply_finish.run_if(crate::state::has_visuals));
}

/// Furniture (polymer stock/grip/handguard) colour per gun.
fn furniture(kind: WeaponKind) -> Color {
    match kind {
        // Flat dark earth on the scout, OD green on the heavy sniper.
        WeaponKind::Marksman => Color::srgb(0.36, 0.30, 0.21),
        WeaponKind::Sniper => Color::srgb(0.17, 0.19, 0.13),
        // A two-tone Warden: tan frame under a black slide.
        WeaponKind::HeavyPistol => Color::srgb(0.33, 0.28, 0.2),
        _ => Color::srgb(0.045, 0.045, 0.05),
    }
}

/// The finish for a source material name (None: keep the source but darken it).
fn finish(kind: WeaponKind, name: &str) -> Option<StandardMaterial> {
    let m = |c: Color, metallic: f32, rough: f32| StandardMaterial { base_color: c, metallic, perceptual_roughness: rough, reflectance: 0.45, ..default() };
    let n = name.trim_start_matches("M_PCL_Flat_");
    Some(match n {
        // Receiver, barrel, slide: blued steel.
        "Black" => m(Color::srgb(0.03, 0.031, 0.035), 0.8, 0.36),
        // Polymer furniture.
        "Black_Lighter" => m(furniture(kind), 0.0, 0.72),
        "Grey_Darker" => m(Color::srgb(0.065, 0.066, 0.07), 0.65, 0.48),
        "Grey_Dark" => m(Color::srgb(0.11, 0.11, 0.115), 0.6, 0.45),
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
    })
}

#[allow(clippy::type_complexity)]
fn apply_finish(
    mut commands: Commands,
    added: Query<(Entity, &MeshMaterial3d<StandardMaterial>, Option<&GltfMaterialName>), Added<MeshMaterial3d<StandardMaterial>>>,
    parents: Query<&ChildOf>,
    roots: Query<&GunFinish>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: Local<HashMap<(WeaponKind, String), Handle<StandardMaterial>>>,
    mut ours: Local<Vec<AssetId<StandardMaterial>>>,
) {
    for (e, mat, name) in &added {
        if ours.contains(&mat.id()) {
            continue;
        }
        let Some(kind) = parents.iter_ancestors(e).find_map(|a| roots.get(a).ok()).map(|f| f.0) else { continue };
        let key = (kind, name.map_or_else(|| format!("#{:?}", mat.id()), |n| n.0.clone()));
        let handle = match cache.get(&key) {
            Some(h) => h.clone(),
            None => {
                let new = match name.and_then(|n| finish(kind, &n.0)) {
                    Some(f) => f,
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
                cache.insert(key, h.clone());
                h
            }
        };
        commands.entity(e).insert(MeshMaterial3d(handle));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_materials_get_finishes() {
        for n in ["M_PCL_Flat_Black", "M_PCL_Flat_Black_Lighter", "M_PCL_Flat_Brown.Chestnut", "M_PCL_Flat_Glass"] {
            assert!(finish(WeaponKind::Rifle, n).is_some(), "{n}");
        }
        assert!(finish(WeaponKind::Rifle, "colormap").is_none());
        // Furniture differs per gun; steel doesn't.
        let a = finish(WeaponKind::Sniper, "M_PCL_Flat_Black_Lighter").unwrap().base_color;
        let b = finish(WeaponKind::Rifle, "M_PCL_Flat_Black_Lighter").unwrap().base_color;
        assert_ne!(a, b);
    }
}
