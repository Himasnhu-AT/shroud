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
            .add_systems(Startup, skip_menu_when_headless)
            .add_systems(Last, quit_after);
    }
}

/// Headless runs have no menu to click, so go straight to `Playing`.
/// `--play` does the same in a window (dev shortcut).
fn skip_menu_when_headless(h: Res<Headless>, mut next: ResMut<NextState<GameState>>) {
    if h.0 || std::env::args().any(|a| a == "--play") {
        next.set(GameState::Playing);
    }
}

/// `--quit-after <seconds>`: exit after that much real time (soak/smoke runs).
fn quit_after(time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>, mut limit: Local<Option<Option<f32>>>) {
    let limit = *limit.get_or_insert_with(|| crate::ops::arg_value("quit-after").and_then(|s| s.parse().ok()));
    if limit.is_some_and(|l| time.elapsed_secs() > l) {
        info!("quit-after reached");
        exit.write(AppExit::Success);
    }
}
