//! Screen-space UI: front end, settings, pause menu, HUD, feedback and the debrief.
//! Shared look in [`theme`], reusable controls in [`widgets`], screen stack in [`screens`].

pub mod apply;
pub mod callouts;
pub mod debrief;
pub mod feedback;
pub mod hud;
pub mod pause;
pub mod perf_overlay;
pub mod screens;
pub mod settings_screen;
pub mod theme;
pub mod widgets;

use bevy::prelude::*;

use crate::enemy::EnemySet;
use crate::player::PlayerSet;
use crate::state::not_paused;
use crate::weapon::WeaponSet;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            theme::ThemePlugin,
            widgets::WidgetsPlugin,
            screens::ScreensPlugin,
            apply::ApplySettingsPlugin,
            callouts::CalloutsPlugin,
            hud::HudPlugin,
            debrief::DebriefPlugin,
            feedback::FeedbackPlugin,
            perf_overlay::PerfOverlayPlugin,
        ))
        // The pause menu freezes gameplay: input, movement, weapons and the enemy AI.
        // (Time<Virtual> is paused too, but frame-based logic would still tick.)
        .configure_sets(
            Update,
            (PlayerSet::Input, PlayerSet::Move, WeaponSet, EnemySet).run_if(not_paused),
        );
    }
}
