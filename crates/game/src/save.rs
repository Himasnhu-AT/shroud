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
        match back_up(&self.path) {
            Ok(Some(to)) => info!("brain: generation {old} backed up to {}", to.display()),
            Ok(None) => {}
            Err(e) => warn!("brain: could not back up {}: {e}", self.path.display()),
        }
        self.state = BrainState::new(rand::random());
        self.state.bootstrap(&brain::arena::Arena::build(map), &brain::evolve::EvolveConfig::default());
        self.save();
        old
    }
}

/// Copy `path` to `<path>.bak-<unix secs>` (`-2`, `-3`… if taken), never over an earlier backup.
/// `None` when there is nothing to back up.
fn back_up(path: &std::path::Path) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let name = path.file_name().map_or_else(|| "brain.json".into(), |n| n.to_string_lossy().into_owned());
    let to = (1u32..)
        .map(|n| path.with_file_name(if n == 1 { format!("{name}.bak-{secs}") } else { format!("{name}.bak-{secs}-{n}") }))
        .find(|p| !p.exists())
        .expect("a free backup name");
    std::fs::copy(path, &to)?;
    Ok(Some(to))
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

#[cfg(test)]
mod tests {
    #[test]
    fn back_up_twice_keeps_both_backups() {
        // Arrange: a brain file and a scratch dir.
        let dir = std::env::temp_dir().join(format!("shroud-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("brain.json");
        std::fs::write(&path, b"gen 7").unwrap();
        // Act: two resets in the same second.
        let first = super::back_up(&path).unwrap().unwrap();
        std::fs::write(&path, b"gen 0").unwrap();
        let second = super::back_up(&path).unwrap().unwrap();
        // Assert
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"gen 7", "the first backup survives the second reset");
        assert_eq!(std::fs::read(&second).unwrap(), b"gen 0");
        assert!(super::back_up(&dir.join("missing.json")).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
