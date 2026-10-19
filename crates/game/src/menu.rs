//! Main menu: a single "Click to play" screen.

use bevy::prelude::*;

use crate::state::{GameState, has_visuals};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Menu), spawn_menu.run_if(has_visuals))
            .add_systems(
                Update,
                click_to_play.run_if(in_state(GameState::Menu).and_then(has_visuals)),
            );
    }
}

#[derive(Component)]
struct PlayButton;

fn spawn_menu(mut commands: Commands) {
    commands.spawn((
        DespawnOnExit(GameState::Menu),
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(24),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.85)),
        children![
            (
                Text::new("SHROUD"),
                TextFont::from_font_size(72.0),
                TextColor(Color::srgb(0.85, 0.9, 1.0)),
            ),
            (
                Text::new("They learn from you."),
                TextFont::from_font_size(20.0),
                TextColor(Color::srgb(0.55, 0.6, 0.7)),
            ),
            (
                PlayButton,
                Button,
                Node {
                    padding: UiRect::axes(px(36), px(14)),
                    margin: UiRect::top(px(24)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.15, 0.2, 0.28)),
                children![(
                    Text::new("Click to play"),
                    TextFont::from_font_size(28.0),
                    TextColor(Color::WHITE),
                )],
            ),
        ],
    ));
}

fn click_to_play(
    mut buttons: Query<(&Interaction, &mut BackgroundColor), (With<PlayButton>, Changed<Interaction>)>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<GameState>>,
) {
    for (interaction, mut bg) in &mut buttons {
        match interaction {
            Interaction::Pressed => next.set(GameState::Playing),
            Interaction::Hovered => bg.0 = Color::srgb(0.22, 0.3, 0.42),
            Interaction::None => bg.0 = Color::srgb(0.15, 0.2, 0.28),
        }
    }
    // Clicking anywhere (or Enter) also starts.
    if mouse.just_pressed(MouseButton::Left) || keys.just_pressed(KeyCode::Enter) {
        next.set(GameState::Playing);
    }
}
