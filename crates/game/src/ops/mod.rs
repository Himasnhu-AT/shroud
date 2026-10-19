//! Test tooling and crash/perf reporting (see `docs/CONTROL_AND_TELEMETRY.md`).
//!
//! - `trace`: crash, hang and frame-time reporting through `ashlar-trace` (always built).
//! - `control` + `commands`: the test-only control port and MCP server (`control` feature).

#[cfg(feature = "control")]
pub mod commands;
#[cfg(feature = "control")]
pub mod control;
pub mod trace;

use std::path::PathBuf;

use bevy::prelude::*;

/// Adds tracing, the control port (with the `control` feature), [`GameSeed`] and [`DebugOverlay`].
/// Add it after `DefaultPlugins` so the trace panic hook runs before Bevy's.
pub struct OpsPlugin;

impl Plugin for OpsPlugin {
    fn build(&self, app: &mut App) {
        let seed = arg_value("seed")
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(rand::random);
        app.insert_resource(GameSeed(seed))
            .init_resource::<DebugOverlay>()
            .add_plugins(trace::TracePlugin);
        #[cfg(feature = "control")]
        app.add_plugins(control::ControlPlugin);
    }
}

/// RNG seed: `--seed <n>` for deterministic tests, otherwise random per run.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameSeed(pub u64);

/// Debug overlay toggle (`debug.overlay`). Read by whatever draws the overlay.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugOverlay(pub bool);

/// The game's data folder (`~/Library/Application Support/shroud` on macOS,
/// `~/.local/share/shroud` on Linux, `%APPDATA%\shroud\data` on Windows), created if missing.
/// Falls back to `./shroud-data` when the platform has no home folder.
pub fn data_dir() -> PathBuf {
    let dir = directories::ProjectDirs::from("", "", "shroud")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("shroud-data"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Value of `--<name> <value>` on the command line.
pub fn arg_value(name: &str) -> Option<String> {
    let flag = format!("--{name}");
    let mut args = std::env::args();
    while let Some(a) = args.next() {
        if a == flag {
            return args.next();
        }
        if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
    }
    None
}

/// Whether `--<name>` is on the command line.
pub fn has_flag(name: &str) -> bool {
    let flag = format!("--{name}");
    std::env::args().any(|a| a == flag)
}
