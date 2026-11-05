//! Optional CC0 models under `assets/`: callers check [`AssetsDir::has`] and fall back to
//! primitives when a file is missing.

pub use engine::paths::AssetsDir;

pub const ENEMY_MODEL: &str = "models/enemy.glb";

/// The MetaHuman enemy, generated locally by `scripts/make-metahuman.sh` (Epic's MetaHuman license:
/// gitignored, never shipped). Same bone names and clip names as [`ENEMY_MODEL`].
pub const ENEMY_MODEL_MH: &str = "models/metahuman/enemy_mh.glb";

/// The enemy model to load: the MetaHuman when it was generated, else the CC0 mannequin.
pub fn enemy_model(dir: &AssetsDir) -> &'static str {
    if dir.has(ENEMY_MODEL_MH) { ENEMY_MODEL_MH } else { ENEMY_MODEL }
}

/// Load an enemy model file. The MetaHuman's meshes and textures live on the GPU only (no CPU
/// copies: they are large and nothing reads them back). Every load of a path must use the same
/// settings, so both the scene and the clip loads go through here.
pub fn load_enemy<A: bevy::asset::Asset>(server: &bevy::asset::AssetServer, path: &'static str, label: Option<bevy::gltf::GltfAssetLabel>) -> bevy::asset::Handle<A> {
    let asset = match label {
        Some(l) => l.from_asset(path),
        None => path.into(),
    };
    if path == ENEMY_MODEL_MH {
        server
            .load_builder()
            .with_settings(|s: &mut bevy::gltf::GltfLoaderSettings| {
                s.load_meshes = bevy::asset::RenderAssetUsages::RENDER_WORLD;
                s.load_materials = bevy::asset::RenderAssetUsages::RENDER_WORLD;
            })
            .load(asset)
    } else {
        server.load(asset)
    }
}

/// Uniform scale of the enemy model so its head sits in the head hitbox (the MetaHuman archetype
/// is 1.60 m; this puts its head bone at the mannequin's height).
pub fn enemy_model_scale(path: &str) -> f32 {
    if path == ENEMY_MODEL_MH { 1.08 } else { 1.0 }
}
