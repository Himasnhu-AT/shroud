//! Shroud's own settings: the `"game"` section of `<data dir>/settings.json` (the engine owns
//! `"core"`: controls, video, audio). A pre-engine flat settings file is read into both.
//!
//! CONTRACT (shared by the AI, maps and UI streams): field names below are stable.
//! - AI reads `difficulty` when spawning a squad (each round); maps read `map` when a new match
//!   starts; the HUD reads the crosshair fields; callouts read `show_callouts`. `primary` is kept
//!   only so older settings files load; guns now come from the buy menu (`economy`).

use bevy::prelude::*;
use brain::arena::MapId;
use brain::difficulty::Difficulty;
use brain::weapons::WeaponKind;
use engine::settings::{FieldDef, FieldKind, RegisterSettings, SettingsPlugin, SettingsRegistry};
use engine::ui::settings_screen::RegisterSettingsUi;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShroudSettings {
    pub difficulty: Difficulty,
    pub map: MapId,
    /// Crosshair colour as sRGB 0..1.
    pub crosshair_color: [f32; 3],
    pub crosshair_size: f32,
    pub show_callouts: bool,
    /// Unused since the buy menu (kept so older settings files round-trip).
    pub primary: WeaponKind,
}

impl Default for ShroudSettings {
    fn default() -> Self {
        ShroudSettings {
            difficulty: Difficulty::default(),
            map: MapId::default(),
            crosshair_color: [0.4, 1.0, 0.6],
            crosshair_size: 1.0,
            show_callouts: true,
            primary: WeaponKind::Rifle,
        }
    }
}

pub const CROSSHAIR_COLORS: [[f32; 3]; 6] =
    [[0.4, 1.0, 0.6], [0.35, 0.9, 1.0], [1.0, 1.0, 1.0], [1.0, 0.92, 0.3], [1.0, 0.4, 0.95], [1.0, 0.3, 0.3]];

pub struct ShroudSettingsPlugin;

impl Plugin for ShroudSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SettingsPlugin::<ShroudSettings>::default());
        register_fields(app);
    }
}

fn s(w: &World) -> &ShroudSettings {
    w.resource::<ShroudSettings>()
}
fn s_mut(w: &mut World) -> Mut<'_, ShroudSettings> {
    w.resource_mut::<ShroudSettings>()
}
fn pick<T: Copy>(all: &[T], v: f32) -> T {
    all[(v.round().max(0.0) as usize).min(all.len() - 1)]
}
fn index_of<T: PartialEq>(all: &[T], v: &T) -> f32 {
    all.iter().position(|x| x == v).unwrap_or(0) as f32
}
fn two_decimals(v: f32) -> String {
    format!("{v:.2}")
}

fn register_fields(app: &mut App) {
    let difficulties = Difficulty::ALL.iter().map(|d| (d.label(), d.blurb())).collect();
    let maps = MapId::ALL.iter().map(|m| (m.label(), m.blurb())).collect();
    app.register_setting(FieldDef {
        key: "difficulty",
        tab: "Gameplay",
        label: "Difficulty",
        hint: "Applies from the next round.",
        kind: FieldKind::Cards { options: difficulties },
        get: |w| index_of(&Difficulty::ALL, &s(w).difficulty),
        set: |w, v| s_mut(w).difficulty = pick(&Difficulty::ALL, v),
    })
    // Picked on the match-setup screen only (no settings tab).
    .register_setting(FieldDef {
        key: "map",
        tab: "",
        label: "Map",
        hint: "",
        kind: FieldKind::Cards { options: maps },
        get: |w| index_of(&MapId::ALL, &s(w).map),
        set: |w, v| s_mut(w).map = pick(&MapId::ALL, v),
    })
    .register_setting(FieldDef {
        key: "show_callouts",
        tab: "Gameplay",
        label: "Squad callouts",
        hint: "Show enemy radio chatter as subtitles.",
        kind: FieldKind::Toggle,
        get: |w| s(w).show_callouts as u8 as f32,
        set: |w, v| s_mut(w).show_callouts = v > 0.5,
    })
    .register_setting(FieldDef {
        key: "crosshair_color",
        tab: "Gameplay",
        label: "Crosshair colour",
        hint: "",
        kind: FieldKind::Swatches { colors: CROSSHAIR_COLORS.to_vec() },
        get: |w| {
            let c = Vec3::from(s(w).crosshair_color);
            let d = |i: usize| Vec3::from(CROSSHAIR_COLORS[i]).distance(c);
            (0..CROSSHAIR_COLORS.len()).min_by(|a, b| d(*a).total_cmp(&d(*b))).unwrap_or(0) as f32
        },
        set: |w, v| s_mut(w).crosshair_color = pick(&CROSSHAIR_COLORS, v),
    })
    .register_setting(FieldDef {
        key: "crosshair_size",
        tab: "Gameplay",
        label: "Crosshair size",
        hint: "",
        kind: FieldKind::Slider { min: 0.5, max: 2.0, step: 0.1, format: two_decimals },
        get: |w| s(w).crosshair_size,
        set: |w, v| s_mut(w).crosshair_size = v.clamp(0.5, 2.0),
    })
    .set_settings_tabs(&["Gameplay", "Controls", "Video", "Audio"])
    .settings_defaults::<ShroudSettings>();

    let mut reg = app.world_mut().resource_mut::<SettingsRegistry>();
    // No music or UI sounds in Shroud.
    reg.fields.retain(|f| !matches!(f.key, "music_volume" | "ui_volume"));
    // Shroud's sensitivity range goes to 4x.
    if let Some(f) = reg.fields.iter_mut().find(|f| f.key == "sensitivity") {
        f.kind = FieldKind::Slider { min: 0.1, max: 4.0, step: 0.05, format: two_decimals };
        f.set = |w, v| w.resource_mut::<engine::settings::CoreSettings>().sensitivity = v.clamp(0.1, 4.0);
    }
}
