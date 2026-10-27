//! Optional CC0 models under `assets/`: callers check [`AssetsDir::has`] and fall back to
//! primitives when a file is missing.

pub use engine::paths::AssetsDir;

pub const ENEMY_MODEL: &str = "models/enemy.glb";
