//! The persisted enemy brain (`<data dir>/brain.json`).

use bevy::prelude::*;
use brain::arena::MapId;
use brain::evolve::{BrainState, EvolveSummary};
use engine::cli::GameSeed;
use engine::paths::DataDir;
use std::path::PathBuf;

use crate::settings::ShroudSettings;

#[derive(Resource)]
pub struct Brain {
    pub state: BrainState,
    pub path: PathBuf,
}

impl Brain {
    pub fn load(dir: &DataDir, seed: u64, map: MapId) -> Self {
        let path = dir.file("brain.json");
        let (mut state, note) = BrainState::load_checked(&path, seed);
        if let Some(note) = note {
            warn!("brain: {note}");
        }
        if state.generation == 0 && !state.evaluated {
            let t = std::time::Instant::now();
            state.bootstrap(&brain::arena::Arena::build(map), &brain::evolve::EvolveConfig::default());
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

    /// Back up the current brain and start over (bootstrapped on `map`). Returns the
    /// generation that was dropped.
    pub fn reset(&mut self, map: MapId) -> u32 {
        let old = self.state.generation;
        let _ = std::fs::copy(&self.path, self.path.with_extension("json.bak"));
        self.state = BrainState::new(rand::random());
        self.state.bootstrap(&brain::arena::Arena::build(map), &brain::evolve::EvolveConfig::default());
        self.save();
        old
    }
}

/// Reset the brain and report it (`ai.reset` trace event).
pub fn reset_brain(world: &mut World) {
    let map = world.resource::<ShroudSettings>().map;
    let from = world.resource_mut::<Brain>().reset(map);
    crate::dev::trace(world, "ai.reset", &[("from_generation", crate::dev::Prop::Int(from as i64))]);
}

/// Result of the last evolution, shown on the debrief.
#[derive(Resource, Clone, Debug, Default)]
pub struct LastEvolve(pub Option<EvolveSummary>);

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        let w = app.world();
        let brain = Brain::load(w.resource::<DataDir>(), w.resource::<GameSeed>().0, w.resource::<ShroudSettings>().map);
        app.insert_resource(brain).init_resource::<LastEvolve>();
    }
}
