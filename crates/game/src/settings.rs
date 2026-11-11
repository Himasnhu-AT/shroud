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
    /// Classic (CS-style centre aim, full HUD) or Bodycam (chest-cam optics, free aim,
    /// stripped HUD; see `bodycam`).
    pub camera_style: CameraStyle,
    /// Bodycam only: where the camera is worn (head height or the chest rig).
    pub bodycam_mount: BodycamMount,
}

/// Where the bodycam sits (Bodycam style only; `bodycam::mount_offset`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodycamMount {
    #[default]
    Head,
    /// Spec §2.1 (`spine_02`): ~0.22 m below and 0.08 m right of the eyes, a little more sway.
    Chest,
}

impl BodycamMount {
    pub const ALL: [BodycamMount; 2] = [BodycamMount::Head, BodycamMount::Chest];
    pub fn label(self) -> &'static str {
        match self {
            BodycamMount::Head => "Head",
            BodycamMount::Chest => "Chest",
        }
    }
}

/// How the player's view is presented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraStyle {
    #[default]
    Classic,
    Bodycam,
}

impl CameraStyle {
    pub const ALL: [CameraStyle; 2] = [CameraStyle::Classic, CameraStyle::Bodycam];
    pub fn label(self) -> &'static str {
        match self {
            CameraStyle::Classic => "Classic",
            CameraStyle::Bodycam => "Bodycam",
        }
    }
}

impl Default for ShroudSettings {
    fn default() -> Self {
        ShroudSettings {
            // New players start on the easiest level; a saved choice is kept.
            difficulty: Difficulty::Recruit,
            map: MapId::default(),
            crosshair_color: [0.4, 1.0, 0.6],
            crosshair_size: 1.0,
            show_callouts: true,
            primary: WeaponKind::Rifle,
            camera_style: CameraStyle::Classic,
            bodycam_mount: BodycamMount::Head,
        }
    }
}

/// Shroud's mouse-sensitivity range (the engine slider goes to 3; ours to 4).
pub const SENSITIVITY_RANGE: (f32, f32) = (0.1, 4.0);

impl ShroudSettings {
    /// Make values loaded from settings.json safe: the crosshair size clamped to its slider
    /// range, the colour to 0..1, anything non-finite back to its default.
    pub fn sanitized(mut self) -> Self {
        let d = ShroudSettings::default();
        self.crosshair_size = if self.crosshair_size.is_finite() { self.crosshair_size.clamp(0.5, 2.0) } else { d.crosshair_size };
        if self.crosshair_color.iter().any(|c| !c.is_finite()) {
            self.crosshair_color = d.crosshair_color;
        }
        for c in &mut self.crosshair_color {
            *c = c.clamp(0.0, 1.0);
        }
        self
    }
}

/// Clamp the loaded settings to the ranges the settings screen allows (the engine already did
/// its own core fields, with a loose sensitivity bound we tighten to Shroud's slider here).
fn sanitize_loaded(world: &mut World) {
    let mut g = world.resource_mut::<ShroudSettings>();
    let clean = g.clone().sanitized();
    g.set_if_neq(clean);
    let mut core = world.resource_mut::<engine::settings::CoreSettings>();
    let sens = core.sensitivity.clamp(SENSITIVITY_RANGE.0, SENSITIVITY_RANGE.1);
    if core.sensitivity != sens {
        core.sensitivity = sens;
    }
}

pub const CROSSHAIR_COLORS: [[f32; 3]; 6] =
    [[0.4, 1.0, 0.6], [0.35, 0.9, 1.0], [1.0, 1.0, 1.0], [1.0, 0.92, 0.3], [1.0, 0.4, 0.95], [1.0, 0.3, 0.3]];

pub struct ShroudSettingsPlugin;

impl Plugin for ShroudSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SettingsPlugin::<ShroudSettings>::default());
        sanitize_loaded(app.world_mut());
        register_fields(app);
        app.add_systems(Update, show_mount_row);
    }
}

/// The "Bodycam mount" row sits on the Video tab only while the camera style is Bodycam (the
/// open settings screen is rebuilt when it appears or goes).
fn show_mount_row(settings: Res<ShroudSettings>, reg: Option<ResMut<SettingsRegistry>>, rebuild: Option<ResMut<Messages<engine::ui::prelude::RebuildScreen>>>) {
    if !settings.is_changed() {
        return;
    }
    let Some(mut reg) = reg else { return };
    let want = if settings.camera_style == CameraStyle::Bodycam { "Video" } else { "" };
    let Some(i) = reg.fields.iter().position(|f| f.key == "bodycam_mount") else { return };
    if reg.fields[i].tab != want {
        reg.fields[i].tab = want;
        if let Some(mut r) = rebuild {
            r.write(engine::ui::prelude::RebuildScreen);
        }
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
    .register_setting(FieldDef {
        key: "camera_style",
        tab: "Video",
        label: "Camera style",
        hint: "Bodycam: wide chest-cam lens, free aim, no crosshair or health/ammo numbers (press R on a full mag to check ammo).",
        kind: FieldKind::Choice { options: CameraStyle::ALL.iter().map(|c| c.label()).collect() },
        get: |w| index_of(&CameraStyle::ALL, &s(w).camera_style),
        set: |w, v| s_mut(w).camera_style = pick(&CameraStyle::ALL, v),
    })
    // Shown on the Video tab only while the camera style is Bodycam (`show_mount_row`).
    .register_setting(FieldDef {
        key: "bodycam_mount",
        tab: "",
        label: "Bodycam mount",
        hint: "Head: the camera at eye height. Chest: worn on the vest, lower and a little right, with more sway. Shots still leave the barrel; enemies still see your head.",
        kind: FieldKind::Choice { options: BodycamMount::ALL.iter().map(|c| c.label()).collect() },
        get: |w| index_of(&BodycamMount::ALL, &s(w).bodycam_mount),
        set: |w, v| s_mut(w).bodycam_mount = pick(&BodycamMount::ALL, v),
    })
    .set_settings_tabs(&["Gameplay", "Controls", "Video", "Audio"])
    .settings_defaults::<ShroudSettings>();

    let mut reg = app.world_mut().resource_mut::<SettingsRegistry>();
    // No music or UI sounds in Shroud.
    reg.fields.retain(|f| !matches!(f.key, "music_volume" | "ui_volume"));
    // Shroud's sensitivity range goes to 4x.
    if let Some(f) = reg.fields.iter_mut().find(|f| f.key == "sensitivity") {
        f.kind = FieldKind::Slider { min: SENSITIVITY_RANGE.0, max: SENSITIVITY_RANGE.1, step: 0.05, format: two_decimals };
        f.set = |w, v| w.resource_mut::<engine::settings::CoreSettings>().sensitivity = v.clamp(SENSITIVITY_RANGE.0, SENSITIVITY_RANGE.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_settings_are_clamped_and_finite() {
        let s = ShroudSettings { crosshair_size: 1e39_f64 as f32, crosshair_color: [f32::NAN, 0.5, 0.5], ..Default::default() }.sanitized();
        assert_eq!(s.crosshair_size, ShroudSettings::default().crosshair_size);
        assert_eq!(s.crosshair_color, ShroudSettings::default().crosshair_color);
        let s = ShroudSettings { crosshair_size: 0.0, crosshair_color: [2.0, -1.0, 0.5], ..Default::default() }.sanitized();
        assert_eq!(s.crosshair_size, 0.5);
        assert_eq!(s.crosshair_color, [1.0, 0.0, 0.5]);
        assert_eq!(ShroudSettings::default().sanitized(), ShroudSettings::default());
    }

    #[test]
    fn plugin_clamps_a_hand_edited_settings_file() {
        let dir = std::env::temp_dir().join(format!("shroud-settings-clamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), r#"{"core":{"fov_deg":0.0,"sensitivity":9.0},"game":{"crosshair_size":50.0}}"#).unwrap();
        let mut app = App::new();
        app.insert_resource(engine::paths::DataDir(dir.clone()));
        app.add_plugins(engine::settings::CoreSettingsPlugin);
        sanitize_after_load(&mut app);
        let core = app.world().resource::<engine::settings::CoreSettings>();
        assert_eq!(core.fov_deg, 60.0);
        assert_eq!(core.sensitivity, SENSITIVITY_RANGE.1);
        assert_eq!(app.world().resource::<ShroudSettings>().crosshair_size, 2.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The part of `ShroudSettingsPlugin::build` that loads and clamps (without the UI rows).
    fn sanitize_after_load(app: &mut App) {
        app.add_plugins(SettingsPlugin::<ShroudSettings>::default());
        sanitize_loaded(app.world_mut());
    }
}
