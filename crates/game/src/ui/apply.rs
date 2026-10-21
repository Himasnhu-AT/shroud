//! Applies [`Settings`] that aren't read directly by their systems: FOV, window mode, vsync,
//! and the small FPS counter. (Sensitivity: `player.rs`; volumes: `audio.rs`; graphics
//! quality: `gfx::apply_camera_quality`; crosshair: `ui::hud`; callouts: `ui::callouts`.)

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, VideoModeSelection, WindowMode};

use super::theme::*;
use crate::player::PlayerCamera;
use crate::settings::{Settings, WindowModeSetting};
use crate::state::has_visuals;

pub struct ApplySettingsPlugin;

impl Plugin for ApplySettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_fps.run_if(has_visuals))
            .add_systems(Update, (apply_fov, apply_window, update_fps).run_if(has_visuals));
    }
}

fn apply_fov(settings: Res<Settings>, mut cams: Query<&mut Projection, With<PlayerCamera>>) {
    if !settings.is_changed() {
        return;
    }
    let fov = settings.fov_deg.clamp(50.0, 120.0).to_radians();
    for mut p in &mut cams {
        if let Projection::Perspective(pp) = p.as_mut()
            && (pp.fov - fov).abs() > 1e-4
        {
            pp.fov = fov;
        }
    }
}

/// Window mode + vsync. Test-driven / offscreen runs never change the window mode (a test must
/// not take over the screen).
fn apply_window(settings: Res<Settings>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if !settings.is_changed() {
        return;
    }
    let test_run = !crate::player::not_test_driven() || crate::ops::has_flag("offscreen");
    for mut w in &mut windows {
        let present = if settings.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
        if w.present_mode != present {
            w.present_mode = present;
        }
        if test_run {
            continue;
        }
        let mode = match settings.window_mode {
            WindowModeSetting::Windowed => WindowMode::Windowed,
            WindowModeSetting::Borderless => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            WindowModeSetting::Fullscreen => WindowMode::Fullscreen(MonitorSelection::Current, VideoModeSelection::Current),
        };
        if w.mode != mode {
            w.mode = mode;
        }
    }
}

#[derive(Component)]
struct FpsText;

fn spawn_fps(mut commands: Commands) {
    commands.spawn((
        FpsText,
        text("", body_bold(SIZE_TINY), FRIENDLY),
        Node {
            position_type: PositionType::Absolute,
            right: px(12),
            top: px(10),
            padding: UiRect::axes(px(8), px(2)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.6)),
        GlobalZIndex(30),
    ));
}

/// Smoothed over ~0.5 s of real time, refreshed 4× a second.
fn update_fps(
    settings: Res<Settings>,
    time: Res<Time<Real>>,
    mut q: Query<(&mut Text, &mut Node, &mut TextColor), With<FpsText>>,
    mut acc: Local<(f32, u32, f32)>,
) {
    let Ok((mut t, mut node, mut color)) = q.single_mut() else { return };
    let want = if settings.show_fps { Display::Flex } else { Display::None };
    if node.display != want {
        node.display = want;
    }
    if !settings.show_fps {
        return;
    }
    acc.0 += time.delta_secs();
    acc.1 += 1;
    if acc.0 >= 0.25 {
        let fps = acc.1 as f32 / acc.0;
        acc.2 = if acc.2 == 0.0 { fps } else { acc.2 * 0.5 + fps * 0.5 };
        *acc = (0.0, 0, acc.2);
        t.0 = format!("{:.0} FPS", acc.2);
        color.0 = if acc.2 >= 55.0 { FRIENDLY } else if acc.2 >= 30.0 { ACCENT } else { ENEMY };
    }
}
