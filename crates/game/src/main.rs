//! Shroud: FPS whose enemies evolve from your past matches.
//!
//! The shell (window, settings, menus, pause, UI kit, input, audio, FPS controller, render
//! budget, devtools) is the `engine` crate; everything here is the game. `shroud --headless`
//! runs the same map/player/weapon/AI logic without a window (tests, control port).

// Normal for Bevy systems.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

mod assets;
mod armed;
mod armor;
mod bomb;
mod audio;
mod bodycam;
mod career;
mod dev;
mod economy;
mod enemy;
mod evolving;
mod gfx;
mod map;
mod match_flow;
mod menu;
mod player;
mod save;
mod settings;
mod state;
mod telemetry;
mod trainer_task;
mod ui;
mod weapon;

use engine::GameInfo;

pub use state::{GameState, Headless};

fn info() -> GameInfo {
    // `id` "shroud" names the save folder and the SHROUD_* env vars: keep it, or players lose
    // their saves. `title` is only the player-facing name (window, menu).
    GameInfo::new("shroud", "Pattern of Life")
        // Second line: the fairness promise (aim/reaction are fixed per difficulty, see brain::limits).
        .tagline("They learn from you.\nThey never aim faster. They play smarter.")
        .version(env!("CARGO_PKG_VERSION"))
        .assets_fallback(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"))
        // Packaged builds: the commit, and the telemetry server (scripts/release.sh sets these).
        .build(option_env!("SHROUD_BUILD_SHA"))
        .trace(option_env!("SHROUD_BUILD_TRACE_ENDPOINT"), option_env!("SHROUD_BUILD_TRACE_KEY"))
}

fn main() {
    engine::app(info())
        .add_plugins((
            // First: the map and the brain read the saved settings while building.
            settings::ShroudSettingsPlugin,
            state::StatePlugin,
            engine::fps::FpsPlugin,
            gfx::GfxPlugin,
            map::MapPlugin,
            player::PlayerPlugin,
            weapon::WeaponPlugin,
            save::SavePlugin,
            evolving::EvolvingPlugin,
            menu::MenuPlugin,
            ui::UiPlugin,
            bodycam::BodycamPlugin,
        ))
        .add_plugins((enemy::EnemyPlugin, match_flow::MatchFlowPlugin, bomb::BombPlugin, career::CareerPlugin, telemetry::TelemetryPlugin, audio::AudioPlugin, dev::DevPlugin))
        .run();
}
