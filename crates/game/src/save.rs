//! The persisted enemy brain (`brain.json`) and where it lives.

use bevy::prelude::*;
use brain::evolve::{BrainState, EvolveSummary};
use std::path::PathBuf;

/// `SHROUD_DATA_DIR` overrides (tests); otherwise the OS app-data folder.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SHROUD_DATA_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/shroud")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(home).join("shroud")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("shroud")
    }
}

#[derive(Resource)]
pub struct Brain {
    pub state: BrainState,
    pub path: PathBuf,
}

impl Brain {
    pub fn load() -> Self {
        let path = data_dir().join("brain.json");
        let seed = std::env::args()
            .skip_while(|a| a != "--seed")
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(rand::random);
        let mut state = BrainState::load_or_new(&path, seed);
        if state.generation == 0 && !state.evaluated {
            let t = std::time::Instant::now();
            state.bootstrap(&brain::arena::Arena::foundry(), &brain::evolve::EvolveConfig::default());
            info!("brain: bootstrap training took {} ms", t.elapsed().as_millis());
        }
        info!("brain: generation {} from {}", state.generation, path.display());
        Brain { state, path }
    }

    pub fn save(&self) {
        if let Err(e) = self.state.save(&self.path) {
            warn!("brain: could not save {}: {e}", self.path.display());
        }
    }

    /// Back up the current brain and start over.
    pub fn reset(&mut self) {
        let _ = std::fs::copy(&self.path, self.path.with_extension("json.bak"));
        self.state = BrainState::new(rand::random());
        self.state.bootstrap(&brain::arena::Arena::foundry(), &brain::evolve::EvolveConfig::default());
        self.save();
    }
}

/// Result of the last evolution, shown on the debrief.
#[derive(Resource, Clone, Debug, Default)]
pub struct LastEvolve(pub Option<EvolveSummary>);

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Brain::load()).init_resource::<LastEvolve>();
    }
}
