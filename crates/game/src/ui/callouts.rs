//! Squad callouts as subtitles: small lines at the bottom-left ("Bravo: Flanking north"),
//! newest at the bottom, each fading out over `LIFE_S`. Fed by [`Callout`] messages.

use bevy::prelude::*;

use crate::enemy::Callout;
use crate::state::{GameState, has_visuals};

const LIFE_S: f32 = 2.5;
const FADE_S: f32 = 0.6;
const MAX_LINES: usize = 4;

pub struct CalloutsPlugin;

impl Plugin for CalloutsPlugin {
    fn build(&self, app: &mut App) {
        // Like the HUD: spawned per round so it binds to the player camera.
        app.add_systems(OnEnter(GameState::Playing), spawn_root.run_if(has_visuals))
            .add_systems(Update, (add_lines, fade_lines).chain().run_if(has_visuals));
    }
}

#[derive(Component)]
struct CalloutRoot;

#[derive(Component)]
struct CalloutLine(f32);

fn spawn_root(mut commands: Commands) {
    commands.spawn((
        Name::new("Callouts"),
        CalloutRoot,
        DespawnOnExit(GameState::Playing),
        Node {
            position_type: PositionType::Absolute,
            left: px(24),
            bottom: px(64),
            flex_direction: FlexDirection::Column,
            row_gap: px(2),
            ..default()
        },
        Pickable::IGNORE,
    ));
}

fn add_lines(
    mut commands: Commands,
    mut msgs: MessageReader<Callout>,
    root: Query<Entity, With<CalloutRoot>>,
    lines: Query<(Entity, &CalloutLine)>,
) {
    let Ok(root) = root.single() else {
        msgs.clear();
        return;
    };
    let mut count = lines.iter().count();
    let mut oldest: Vec<(Entity, f32)> = lines.iter().map(|(e, l)| (e, l.0)).collect();
    oldest.sort_by(|a, b| b.1.total_cmp(&a.1));
    for m in msgs.read() {
        if count >= MAX_LINES
            && let Some((e, _)) = oldest.first().copied()
        {
            commands.entity(e).despawn();
            oldest.remove(0);
            count -= 1;
        }
        // The UI font is ASCII-only.
        let text: String = format!("{}: {}", m.speaker, m.text).chars().filter(|c| c.is_ascii()).collect();
        commands.spawn((
            CalloutLine(0.0),
            Text::new(text),
            TextFont::from_font_size(18.0),
            TextColor(Color::srgb(0.85, 0.95, 0.8)),
            TextShadow::default(),
            ChildOf(root),
        ));
        count += 1;
    }
}

fn fade_lines(mut commands: Commands, time: Res<Time>, mut lines: Query<(Entity, &mut CalloutLine, &mut TextColor)>) {
    for (e, mut l, mut color) in &mut lines {
        l.0 += time.delta_secs();
        if l.0 >= LIFE_S {
            commands.entity(e).despawn();
        } else {
            color.0.set_alpha(((LIFE_S - l.0) / FADE_S).min(1.0));
        }
    }
}
