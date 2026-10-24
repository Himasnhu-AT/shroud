//! Squad callouts as subtitles: small lines at the bottom-left ("Bravo: Flanking north"),
//! newest at the bottom, each fading out over `LIFE_S`. Fed by [`Callout`] messages.

use bevy::prelude::*;

use super::theme::*;
use crate::enemy::Callout;
use crate::settings::ShroudSettings;
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
        super::hud::HideWhenPaused,
        DespawnOnExit(GameState::Playing),
        Node {
            position_type: PositionType::Absolute,
            left: px(40),
            bottom: px(150),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
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
    settings: Res<ShroudSettings>,
    log: crate::dev::Dev,
) {
    let root = root.single().ok().filter(|_| settings.show_callouts);
    let Some(root) = root else {
        for m in msgs.read() {
            log.log("callout", || serde_json::json!({ "speaker": m.speaker, "text": m.text, "shown": false }));
        }
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
        log.log("callout", || serde_json::json!({ "speaker": m.speaker, "text": m.text, "shown": true }));
        // Callouts are ASCII by contract; keep it that way for the font.
        let clean = |t: &str| t.chars().filter(|c| c.is_ascii()).collect::<String>();
        commands
            .spawn((
                CalloutLine(0.0),
                // Dark backing so lines stay readable over the red damage vignette.
                Node {
                    padding: UiRect::new(px(10), px(12), px(4), px(5)),
                    column_gap: px(10),
                    align_items: AlignItems::Baseline,
                    border: UiRect::left(px(3)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.05, BG_ALPHA)),
                BorderColor::all(ENEMY),
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .with_children(|l| {
                l.spawn((text(clean(m.speaker).to_uppercase(), label(SIZE_LABEL + 1.0), ENEMY), TextShadow::default()));
                l.spawn((text(clean(&m.text), body(SIZE_BODY - 1.0), TEXT), TextShadow::default()));
            });
        count += 1;
    }
}

const BG_ALPHA: f32 = 0.6;

fn fade_lines(
    mut commands: Commands,
    time: Res<Time>,
    mut lines: Query<(Entity, &mut CalloutLine, &mut BackgroundColor, &mut BorderColor, &Children)>,
    mut texts: Query<&mut TextColor>,
) {
    for (e, mut l, mut bg, mut border, kids) in &mut lines {
        l.0 += time.delta_secs();
        if l.0 >= LIFE_S {
            commands.entity(e).despawn();
        } else {
            let a = ((LIFE_S - l.0) / FADE_S).min(1.0);
            bg.0.set_alpha(a * BG_ALPHA);
            *border = BorderColor::all(ENEMY.with_alpha(a));
            for k in kids.iter() {
                if let Ok(mut t) = texts.get_mut(k) {
                    t.0.set_alpha(a);
                }
            }
        }
    }
}
