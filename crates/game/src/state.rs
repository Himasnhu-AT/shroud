//! Shroud's flow inside the engine's `AppState::InGame`: `Playing → Evolving → Debrief →
//! Playing …`. The main menu is the engine's `AppState::MainMenu`; pause is the engine's.

use bevy::prelude::*;
pub use engine::flow::{AppState, Headless, has_visuals};
use engine::flow::PauseAllowed;
use engine::input::CaptureCursor;
pub use engine::pause::Paused;

/// Gameplay may run: not paused and no loading screen up (`loading`). Shroud's gameplay sets use
/// this instead of the engine's `not_paused`.
pub fn not_paused(p: Res<Paused>, loading: Option<Res<crate::loading::Loading>>) -> bool {
    !p.0 && !loading.is_some_and(|l| l.holds())
}

#[derive(SubStates, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[source(AppState = AppState::InGame)]
pub enum GameState {
    #[default]
    Playing,
    Evolving,
    Debrief,
}

pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        app.add_sub_state::<GameState>().add_systems(Update, rounds_only);
    }
}

/// Esc pauses and the cursor is captured only during a round; the evolving and debrief screens
/// keep the pointer free and handle Esc themselves. The buy menu frees the pointer too, and Esc
/// closes it instead of pausing. Nothing is captured and Esc does nothing under the loading screen.
fn rounds_only(
    state: Option<Res<State<GameState>>>,
    buy: Option<Res<crate::economy::BuyMenu>>,
    loading: Option<Res<crate::loading::Loading>>,
    mut pause: ResMut<PauseAllowed>,
    mut cursor: ResMut<CaptureCursor>,
) {
    let playing = state.is_some_and(|s| *s.get() == GameState::Playing) && !buy.is_some_and(|b| b.open) && !loading.is_some_and(|l| l.holds());
    if pause.0 != playing {
        pause.0 = playing;
    }
    if cursor.0 != playing {
        cursor.0 = playing;
    }
}
