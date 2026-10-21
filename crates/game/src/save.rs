//! The persisted enemy brain (`brain.json`) and where it lives.

use bevy::prelude::*;
use brain::evolve::{BrainState, EvolveSummary};
use std::path::PathBuf;

use crate::ops::trace::{GameEvent, GameTracer, trace_event};

pub use crate::ops::data_dir;

#[derive(Resource)]
pub struct Brain {
    pub state: BrainState,
    pub path: PathBuf,
}

impl Brain {
    pub fn load() -> Self {
        let path = data_dir().join("brain.json");
        let seed = crate::ops::arg_value("seed").and_then(|s| s.parse().ok()).unwrap_or_else(rand::random);
        let mut state = BrainState::load_or_new(&path, seed);
        if state.generation == 0 && !state.evaluated {
            let t = std::time::Instant::now();
            state.bootstrap(&brain::arena::Arena::build(crate::settings::Settings::load().map), &brain::evolve::EvolveConfig::default());
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
    pub fn reset(&mut self, tracer: Option<&GameTracer>) {
        if let Some(t) = tracer {
            trace_event(t, GameEvent::AiReset, &[("from_generation", ashlar_trace::Prop::Int(self.state.generation as i64))]);
        }
        let _ = std::fs::copy(&self.path, self.path.with_extension("json.bak"));
        self.state = BrainState::new(rand::random());
        self.state.bootstrap(&brain::arena::Arena::build(crate::settings::Settings::load().map), &brain::evolve::EvolveConfig::default());
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
