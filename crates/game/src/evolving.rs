//! `GameState::Evolving` placeholder: the between-match evolution pipeline
//! (brain::evolve on a background thread) will live here. For now it shows a
//! label and moves on to `Debrief` after a short delay.

use bevy::prelude::*;

use crate::state::{GameState, has_visuals};

pub struct EvolvingPlugin;

impl Plugin for EvolvingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Evolving), (start, spawn_label.run_if(has_visuals)))
            .add_systems(Update, tick.run_if(in_state(GameState::Evolving)));
    }
}

#[derive(Resource)]
struct EvolveTimer(Timer);

fn start(mut commands: Commands) {
    commands.insert_resource(EvolveTimer(Timer::from_seconds(1.0, TimerMode::Once)));
}

fn spawn_label(mut commands: Commands) {
    commands.spawn((
        DespawnOnExit(GameState::Evolving),
        Node {
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
        children![(Text::new("Evolving..."), TextFont::from_font_size(40.0))],
    ));
}

fn tick(time: Res<Time>, mut timer: ResMut<EvolveTimer>, mut next: ResMut<NextState<GameState>>) {
    if timer.0.tick(time.delta()).just_finished() {
        next.set(GameState::Debrief);
    }
}
