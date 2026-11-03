//! In-game screen-space UI: HUD, hit/damage feedback, squad callouts and the debrief. Menus,
//! pause and settings are the engine's UI kit, configured in `menu`.

pub mod buy_menu;
pub mod callouts;
pub mod debrief;
pub mod feedback;
pub mod hud;
pub mod killfeed;
pub mod perf_toast;
pub mod round_summary;
pub mod scoreboard;
pub mod theme;
pub mod weapon_hud;

use bevy::prelude::*;

use crate::enemy::EnemySet;
use crate::state::not_paused;
use crate::weapon::WeaponSet;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((callouts::CalloutsPlugin, hud::HudPlugin, debrief::DebriefPlugin, feedback::FeedbackPlugin, weapon_hud::WeaponHudPlugin))
            .add_plugins((killfeed::KillFeedPlugin, scoreboard::ScoreboardPlugin, round_summary::RoundSummaryPlugin, perf_toast::PerfToastPlugin, buy_menu::BuyMenuPlugin))
            // The pause menu freezes weapons and the enemy AI (the engine already stops the
            // player's movement). Time<Virtual> is paused too, but frame-based logic would tick.
            .configure_sets(Update, (WeaponSet, EnemySet).run_if(not_paused));
    }
}
