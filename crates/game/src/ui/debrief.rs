//! `GameState::Debrief` placeholder. The real screen (match result, "What they
//! learned", "How they changed", charts) is filled in later. For now: a title
//! and click/Enter → next match.

use bevy::prelude::*;

use crate::state::{GameState, has_visuals};

pub struct DebriefPlugin;

impl Plugin for DebriefPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Debrief), spawn_debrief.run_if(has_visuals))
            .add_systems(
                Update,
                next_match.run_if(in_state(GameState::Debrief).and_then(has_visuals)),
            );
    }
}

fn spawn_debrief(mut commands: Commands) {
    commands.spawn((
        DespawnOnExit(GameState::Debrief),
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(16),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.9)),
        children![
            (Text::new("Debrief"), TextFont::from_font_size(48.0)),
            (
                Text::new("Click for next match"),
                TextFont::from_font_size(22.0),
                TextColor(Color::srgb(0.6, 0.65, 0.75)),
            ),
        ],
    ));
}

fn next_match(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<GameState>>,
) {
    if mouse.just_pressed(MouseButton::Left) || keys.just_pressed(KeyCode::Enter) {
        next.set(GameState::Playing);
    }
}
