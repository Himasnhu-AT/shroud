//! Squad callouts as subtitles: small lines at the bottom-left ("BRAVO (MARKSMAN) Flanking
//! north"), newest at the bottom, each fading out over `LIFE_S`. Fed by [`Callout`] messages.
//! Also the player's own intel lines ("Marksman on the west lane", blue): when a squad member
//! with a special loadout first engages you, once per enemy per round, rate-limited.

use std::collections::HashMap;

use bevy::prelude::*;
use brain::loadout::Role;

use super::theme::*;
use crate::enemy::{Callout, Enemy, EnemyCombat};
use crate::map::{ArenaRes, to_ground};
use crate::settings::ShroudSettings;
use crate::state::{GameState, has_visuals};

const LIFE_S: f32 = 2.5;
const FADE_S: f32 = 0.6;
const MAX_LINES: usize = 4;
/// Least time between two intel lines (s).
const INTEL_GAP_S: f32 = 3.0;

/// A line of intel for the player (not a squad callout: no radio click, blue).
#[derive(Message, Clone, Debug)]
pub struct Intel(pub String);

pub struct CalloutsPlugin;

impl Plugin for CalloutsPlugin {
    fn build(&self, app: &mut App) {
        // Like the HUD: spawned per round so it binds to the player camera.
        app.add_message::<Intel>()
            .add_systems(OnEnter(GameState::Playing), spawn_root.run_if(has_visuals))
            .add_systems(Update, intel.run_if(in_state(GameState::Playing)).run_if(has_visuals))
            .add_systems(Update, (add_lines, fade_lines).chain().after(intel).run_if(has_visuals));
    }
}

#[derive(Component)]
struct CalloutRoot;

/// Age (s) and accent colour of a subtitle line.
#[derive(Component)]
struct CalloutLine(f32, Color);

fn spawn_root(mut commands: Commands) {
    commands.spawn((
        Name::new("Callouts"),
        CalloutRoot,
        super::hud::HideWhenPaused,
        DespawnOnExit(GameState::Playing),
        Node {
            position_type: PositionType::Absolute,
            left: px(40),
            bottom: px(182),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            ..default()
        },
        Pickable::IGNORE,
    ));
}

/// Intel: a rusher / breacher / marksman engaging you is called out with where it is.
fn intel(
    time: Res<Time>,
    arena: Res<ArenaRes>,
    enemies: Query<(Entity, &EnemyCombat, &Transform), With<Enemy>>,
    mut out: MessageWriter<Intel>,
    mut seen: Local<HashMap<Entity, bool>>,
    mut last: Local<f32>,
) {
    let now = time.elapsed_secs();
    for (e, c, tf) in &enemies {
        let sees = c.sees_player && !c.dead;
        // `false` = not called yet, `true` = called this round (entities are new each round).
        let called = seen.entry(e).or_insert(false);
        if !sees || *called || c.role == Role::Rifleman || now - *last < INTEL_GAP_S {
            continue;
        }
        *called = true;
        *last = now;
        let pos = to_ground(tf.translation);
        let text = match arena.0.zone_of(pos) {
            Some(z) => format!("{} on {}", c.role.label(), z.name),
            None => format!("{} has eyes on you", c.role.label()),
        };
        out.write(Intel(text));
    }
    if seen.len() > 16 {
        seen.retain(|e, _| enemies.contains(*e));
    }
}

fn add_lines(
    mut commands: Commands,
    mut msgs: MessageReader<Callout>,
    mut intel: MessageReader<Intel>,
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
        intel.clear();
        return;
    };
    if msgs.is_empty() && intel.is_empty() {
        return;
    }
    let mut count = lines.iter().count();
    let mut oldest: Vec<(Entity, f32)> = lines.iter().map(|(e, l)| (e, l.0)).collect();
    oldest.sort_by(|a, b| b.1.total_cmp(&a.1));
    // Callouts are ASCII by contract; keep it that way for the font.
    let clean = |t: &str| t.chars().filter(|c| c.is_ascii()).collect::<String>();
    let items: Vec<(String, String, Color)> = msgs
        .read()
        .map(|m| {
            log.log("callout", || serde_json::json!({ "speaker": m.speaker, "role": m.role, "text": m.text, "shown": true }));
            let who = match m.role {
                Some(r) => format!("{} ({})", clean(m.speaker), r),
                None => clean(m.speaker),
            };
            (who.to_uppercase(), clean(&m.text), ENEMY)
        })
        .chain(intel.read().map(|i| ("INTEL".to_string(), clean(&i.0), INTEL)))
        .collect();
    for (who, what, color) in items {
        if count >= MAX_LINES
            && let Some((e, _)) = oldest.first().copied()
        {
            commands.entity(e).despawn();
            oldest.remove(0);
            count -= 1;
        }
        commands
            .spawn((
                CalloutLine(0.0, color),
                // Dark backing so lines stay readable over the red damage vignette.
                Node {
                    padding: UiRect::new(px(10), px(12), px(4), px(5)),
                    column_gap: px(10),
                    align_items: AlignItems::Baseline,
                    border: UiRect::left(px(3)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.03, 0.05, BG_ALPHA)),
                BorderColor::all(color),
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .with_children(|l| {
                l.spawn((text(who, label(SIZE_LABEL + 1.0), color), line_shadow()));
                l.spawn((text(what, body(SIZE_BODY - 1.0), TEXT), line_shadow()));
            });
        count += 1;
    }
}

const BG_ALPHA: f32 = 0.6;

/// A tight drop shadow for legibility over bright scenes. Bevy's default (4 px offset) read as a
/// second, offset copy of every callout line.
fn line_shadow() -> TextShadow {
    TextShadow { offset: Vec2::new(1.0, 1.0), color: Color::srgba(0.0, 0.0, 0.0, 0.55) }
}

fn fade_lines(
    mut commands: Commands,
    time: Res<Time>,
    mut lines: Query<(Entity, &mut CalloutLine, &mut BackgroundColor, &mut BorderColor, &Children)>,
    mut texts: Query<(&mut TextColor, Option<&mut TextShadow>)>,
) {
    for (e, mut l, mut bg, mut border, kids) in &mut lines {
        l.0 += time.delta_secs();
        if l.0 >= LIFE_S {
            commands.entity(e).despawn();
            continue;
        }
        let a = ((LIFE_S - l.0) / FADE_S).min(1.0);
        // Fully visible until the fade starts: skip the writes so nothing is marked changed.
        if a >= 1.0 && bg.0.alpha() >= BG_ALPHA {
            continue;
        }
        bg.0.set_alpha(a * BG_ALPHA);
        *border = BorderColor::all(l.1.with_alpha(a));

        for k in kids.iter() {
            if let Ok((mut t, shadow)) = texts.get_mut(k) {
                t.0.set_alpha(a);
                // The shadow must fade with the text, or it reads as a second copy of the line.
                if let Some(mut sh) = shadow {
                    let base = line_shadow().color.alpha();
                    sh.color.set_alpha(base * a);
                }
            }
        }
    }
}
