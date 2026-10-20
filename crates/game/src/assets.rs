//! Asset root resolution and optional-model checks.
//!
//! Models under `assets/` are optional (CC0 downloads). Callers check
//! [`AssetsDir::has`] and fall back to primitives when a file is missing.

use std::path::PathBuf;

use bevy::prelude::*;

pub const RIFLE_MODEL: &str = "models/rifle.glb";
pub const ENEMY_MODEL: &str = "models/enemy.glb";

/// Absolute path of the workspace `assets/` directory.
#[derive(Resource, Clone, Debug)]
pub struct AssetsDir(pub PathBuf);

impl AssetsDir {
    /// True if `rel` (relative to the assets dir) exists on disk.
    pub fn has(&self, rel: &str) -> bool {
        self.0.join(rel).is_file()
    }
}

/// `./assets` if run from the workspace root, else the workspace `assets/`
/// relative to this crate's manifest (works for `cargo run -p game` anywhere).
pub fn assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SHROUD_ASSETS") {
        return PathBuf::from(dir);
    }
    // Inside a macOS app bundle: Shroud.app/Contents/MacOS/shroud -> Contents/Resources/assets.
    if let Ok(exe) = std::env::current_exe()
        && let Some(contents) = exe.parent().and_then(|p| p.parent())
    {
        let p = contents.join("Resources/assets");
        if p.is_dir() {
            return p;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("assets");
        if p.is_dir() {
            return p;
        }
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"))
}
