//! Make a mesh ready for a tiling photoscanned PBR set: tangents for its normal map and, for
//! models that ship flat colours on palette UVs (the gun set), box-projected UVs so a texture
//! tiles across every face at a real-world size.
//!
//! The mesh asset is changed in place, once per mesh (every instance shares it and wants the
//! same result). An entity keeps its [`MeshPrep`] until its mesh has loaded, then it is removed;
//! the system only looks at entities still waiting. Meshes without CPU data (render-world-only
//! loads) are left alone.

use std::collections::HashSet;

use bevy::mesh::{MeshVertexAttribute, VertexAttributeValues};
use bevy::prelude::*;

/// What a mesh needs before its material is textured.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum MeshPrep {
    /// Generate tangents (MikkTSpace) if the mesh has none: without them Bevy skips the normal map.
    Tangents,
    /// Replace UV0 with a box projection (each vertex on the plane its normal faces most),
    /// `per_unit` texture repeats per mesh-space unit, then generate tangents.
    BoxUv { per_unit: f32 },
}

pub struct MeshPrepPlugin;

impl Plugin for MeshPrepPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, prep.run_if(crate::state::has_visuals));
    }
}

fn prep(mut commands: Commands, waiting: Query<(Entity, &Mesh3d, &MeshPrep)>, mut meshes: ResMut<Assets<Mesh>>, mut done: Local<HashSet<AssetId<Mesh>>>) {
    for (e, mesh, how) in &waiting {
        let id = mesh.id();
        if !done.contains(&id) {
            // Read first: `get_mut` marks the asset changed (a GPU re-upload) even if nothing is.
            let Some(m) = meshes.get(id) else { continue };
            let needs = match how {
                MeshPrep::Tangents => m.attribute(Mesh::ATTRIBUTE_TANGENT).is_none(),
                MeshPrep::BoxUv { .. } => true,
            };
            if needs && let Some(mut m) = meshes.get_mut(id) {
                if let MeshPrep::BoxUv { per_unit } = how {
                    box_uv(&mut m, *per_unit);
                }
                if let Err(err) = m.generate_tangents() {
                    debug!("mesh prep: no tangents for {id:?}: {err}");
                }
            }
            done.insert(id);
        }
        commands.entity(e).remove::<MeshPrep>();
    }
}

fn attr<'a>(m: &'a Mesh, a: MeshVertexAttribute) -> Option<&'a [[f32; 3]]> {
    match m.attribute(a)? {
        VertexAttributeValues::Float32x3(v) => Some(v),
        _ => None,
    }
}

/// Box projection: each vertex is mapped on the axis plane its normal faces most.
pub fn box_uv(m: &mut Mesh, per_unit: f32) {
    let (Some(pos), Some(nrm)) = (attr(m, Mesh::ATTRIBUTE_POSITION), attr(m, Mesh::ATTRIBUTE_NORMAL)) else { return };
    let uv: Vec<[f32; 2]> = pos.iter().zip(nrm).map(|(p, n)| box_uv_of(Vec3::from(*p), Vec3::from(*n), per_unit)).collect();
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
}

fn box_uv_of(p: Vec3, n: Vec3, k: f32) -> [f32; 2] {
    let a = n.abs();
    let (u, v) = if a.x >= a.y && a.x >= a.z {
        (p.z, p.y)
    } else if a.y >= a.z {
        (p.x, p.z)
    } else {
        (p.x, p.y)
    };
    [u * k, v * k]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    #[test]
    fn box_uv_follows_the_facing_plane_and_gives_tangents() {
        let mut m = Cuboid::new(2.0, 1.0, 0.5).mesh().build();
        box_uv(&mut m, 3.0);
        m.generate_tangents().unwrap();
        assert!(m.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
        // A +Z face maps x, y.
        assert_eq!(box_uv_of(Vec3::new(1.0, 0.5, 0.25), Vec3::Z, 3.0), [3.0, 1.5]);
        // A +Y face maps x, z; a -X face z, y.
        assert_eq!(box_uv_of(Vec3::new(1.0, 0.5, 0.25), Vec3::Y, 2.0), [2.0, 0.5]);
        assert_eq!(box_uv_of(Vec3::new(1.0, 0.5, 0.25), Vec3::NEG_X, 2.0), [0.5, 1.0]);
        let empty = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        let mut e = empty;
        box_uv(&mut e, 1.0); // no positions: left alone
        assert!(e.attribute(Mesh::ATTRIBUTE_UV_0).is_none());
    }
}
