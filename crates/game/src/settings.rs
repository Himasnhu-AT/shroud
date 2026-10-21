//! Player settings, persisted to `<data_dir>/settings.json`.
//!
//! CONTRACT (shared by the AI, maps and UI streams): field names below are stable.
//! - AI stream reads `difficulty` when spawning a squad (each round).
//! - Maps stream reads `map` when a new match starts.
//! - UI stream owns the menus that edit these, and applies the rest
//!   (sensitivity, FOV, volume, graphics) to the game.
//!
//! A corrupt or missing file falls back to defaults; unknown/missing fields use defaults.

use bevy::prelude::*;
use brain::arena::MapId;
use brain::difficulty::Difficulty;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GraphicsQuality {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WindowModeSetting {
    #[default]
    Windowed,
    Borderless,
    Fullscreen,
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Match
    pub difficulty: Difficulty,
    pub map: MapId,
    // Controls
    /// Mouse sensitivity multiplier (1.0 = default).
    pub sensitivity: f32,
    pub invert_y: bool,
    // Video
    pub fov_deg: f32,
    pub quality: GraphicsQuality,
    pub window_mode: WindowModeSetting,
    pub vsync: bool,
    pub show_fps: bool,
    // Audio (0..1)
    pub master_volume: f32,
    pub sfx_volume: f32,
    pub ambience_volume: f32,
    // HUD
    /// Crosshair colour as sRGB 0..1.
    pub crosshair_color: [f32; 3],
    pub crosshair_size: f32,
    pub show_callouts: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            difficulty: Difficulty::default(),
            map: MapId::default(),
            sensitivity: 1.0,
            invert_y: false,
            fov_deg: 80.0,
            quality: GraphicsQuality::default(),
            window_mode: WindowModeSetting::default(),
            vsync: true,
            show_fps: false,
            master_volume: 0.8,
            sfx_volume: 1.0,
            ambience_volume: 0.6,
            crosshair_color: [0.4, 1.0, 0.6],
            crosshair_size: 1.0,
            show_callouts: true,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::ops::data_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(Self::path(), s) {
                    warn!("settings: could not save: {e}");
                }
            }
            Err(e) => warn!("settings: could not encode: {e}"),
        }
    }
}

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Settings::load());
    }
}
