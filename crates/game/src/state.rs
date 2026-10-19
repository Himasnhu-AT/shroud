//! Top-level game state machine: `Menu → Playing → Evolving → Debrief → Playing …`.

use bevy::prelude::*;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    #[default]
    Menu,
    Playing,
    Evolving,
    Debrief,
}

/// `true` when running with `--headless` (no window, renderer, input or UI).
#[derive(Resource, Clone, Copy, Debug)]
pub struct Headless(pub bool);

/// Run condition: rendering/UI/input exist.
pub fn has_visuals(h: Res<Headless>) -> bool {
    !h.0
}

pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .add_systems(Startup, skip_menu_when_headless);
    }
}

/// Headless runs have no menu to click, so go straight to `Playing`.
/// `--play` does the same in a window (dev shortcut).
fn skip_menu_when_headless(h: Res<Headless>, mut next: ResMut<NextState<GameState>>) {
    if h.0 || std::env::args().any(|a| a == "--play") {
        next.set(GameState::Playing);
    }
}
