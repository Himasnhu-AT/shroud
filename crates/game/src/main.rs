//! Shroud: FPS whose enemies evolve from your past matches.
//!
//! `shroud` runs windowed; `shroud --headless` runs the same map/player/weapon
//! logic without a window or renderer (used by tests and the control port).

// Several pub items are API for modules other agents are adding (match flow, enemies, telemetry).
#![allow(dead_code)]
// Normal for Bevy systems.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

mod assets;
mod enemy;
mod evolving;
mod map;
mod match_flow;
mod menu;
mod ops;
mod player;
mod save;
mod state;
mod telemetry;
mod ui;
mod weapon;

use std::time::Duration;

use avian3d::prelude::*;
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;

pub use state::{GameState, Headless};

fn main() {
    let headless = std::env::args().any(|a| a == "--headless");
    let assets_dir = assets::assets_dir();
    let asset_plugin = AssetPlugin {
        file_path: assets_dir.to_string_lossy().into_owned(),
        ..default()
    };

    let mut app = App::new();
    if headless {
        app.add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 60.0,
            ))),
            bevy::log::LogPlugin::default(),
            TransformPlugin,
            bevy::diagnostic::DiagnosticsPlugin,
            asset_plugin,
            bevy::mesh::MeshPlugin,
            bevy::world_serialization::WorldSerializationPlugin,
            bevy::scene::ScenePlugin,
            bevy::state::app::StatesPlugin,
        ));
    } else {
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Shroud".into(),
                        // Control-port test runs: stay on top so macOS keeps drawing the
                        // window (an occluded window renders black screenshots).
                        // `--offscreen` (tests): hidden window, camera renders to an image.
                        visible: !std::env::args().any(|a| a == "--offscreen"),
                        window_level: if std::env::args().any(|a| a == "--control") {
                            bevy::window::WindowLevel::AlwaysOnTop
                        } else {
                            bevy::window::WindowLevel::Normal
                        },
                        ..default()
                    }),
                    ..default()
                })
                .set(asset_plugin),
        );
    }

    app.add_plugins(PhysicsPlugins::default())
        .insert_resource(Headless(headless))
        .insert_resource(assets::AssetsDir(assets_dir))
        .add_plugins((
            state::StatePlugin,
            menu::MenuPlugin,
            evolving::EvolvingPlugin,
            map::MapPlugin,
            player::PlayerPlugin,
            weapon::WeaponPlugin,
            ui::UiPlugin,
            // Last: its trace panic hook must wrap Bevy's.
            ops::OpsPlugin,
        ))
        .add_plugins((
            save::SavePlugin,
            enemy::EnemyPlugin,
            match_flow::MatchFlowPlugin,
            telemetry::TelemetryPlugin,
        ))
        .run();
}

