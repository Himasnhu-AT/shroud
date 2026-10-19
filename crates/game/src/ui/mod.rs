//! Screen-space UI: in-game HUD and the debrief screen.

pub mod debrief;
pub mod feedback;
pub mod hud;
pub mod perf_overlay;

use bevy::prelude::*;


pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((hud::HudPlugin, debrief::DebriefPlugin, feedback::FeedbackPlugin, perf_overlay::PerfOverlayPlugin));
    }
}
