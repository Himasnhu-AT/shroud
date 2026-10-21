//! Front end: main menu (Play / Settings / Quit) and match setup (difficulty + map cards).
//! Drawn over the live 3D scene, which slowly pans behind a dark scrim.
//! Screens are built by `ui::screens`; `--play` and `--headless` still skip straight to a match.

use bevy::prelude::*;
use brain::arena::MapId;
use brain::difficulty::Difficulty;

use crate::player::{Player, PlayerCamera};
use crate::state::{GameState, has_visuals};
use crate::ui::screens::ScreenCtx;
use crate::ui::theme::*;
use crate::ui::widgets::{Field, UiAction, action_button, card_row, nav_button};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, pan_camera.run_if(in_state(GameState::Menu).and_then(has_visuals)))
            .add_systems(Update, viewmodel_only_in_rounds.run_if(has_visuals));
    }
}

/// Slow cinematic yaw while the menu is up.
fn pan_camera(
    time: Res<Time<Real>>,
    mut player: Query<&mut Transform, (With<Player>, Without<PlayerCamera>)>,
    mut cam: Query<&mut Transform, With<PlayerCamera>>,
) {
    let t = time.elapsed_secs();
    for mut tf in &mut player {
        tf.rotation = Quat::from_rotation_y(0.35 + t * 0.025);
    }
    for mut tf in &mut cam {
        tf.rotation = Quat::from_rotation_x(-0.04 + (t * 0.11).sin() * 0.015);
    }
}

/// The rifle only shows in a round (not behind the menu, evolution or debrief screens).
fn viewmodel_only_in_rounds(state: Res<State<GameState>>, mut q: Query<(&Name, &mut Visibility)>, mut done: Local<Option<GameState>>) {
    if *done == Some(*state.get()) {
        return;
    }
    let want = if *state.get() == GameState::Playing { Visibility::Inherited } else { Visibility::Hidden };
    let mut found = false;
    for (name, mut v) in &mut q {
        if name.as_str() == "ViewModel" {
            *v = want;
            found = true;
        }
    }
    // The viewmodel spawns in PostStartup: retry until it exists.
    if found {
        *done = Some(*state.get());
    }
}

fn scrim(c: &mut Commands, root: Entity) {
    c.spawn((fullscreen(), menu_scrim(1.0), Pickable::IGNORE, ChildOf(root)));
    // Bottom fade so the footer reads.
    c.spawn((
        Node { position_type: PositionType::Absolute, left: px(0), right: px(0), bottom: px(0), height: percent(30), ..default() },
        BackgroundGradient::from(LinearGradient::new(
            0.0,
            vec![ColorStop::new(BG.with_alpha(0.85), percent(0)), ColorStop::new(BG.with_alpha(0.0), percent(100))],
        )),
        Pickable::IGNORE,
        ChildOf(root),
    ));
}

/// Brand block: eyebrow, logotype, tagline.
fn brand(p: &mut ChildSpawnerCommands, hero: f32) {
    p.spawn((Node { column_gap: px(GAP_S + 4.0), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE))
        .with_children(|r| {
            r.spawn((Node { width: px(36), height: px(3), ..default() }, BackgroundColor(ACCENT), Pickable::IGNORE));
            r.spawn(eyebrow("Adaptive squad shooter", ACCENT));
        });
    p.spawn((text("SHROUD", display(hero), TEXT), Node { margin: UiRect::new(px(-6), px(0), px(-8), px(-14)), ..default() }));
    p.spawn(text("They learn from you.", body(24.0), TEXT_MUTED));
}

pub fn spawn_main(c: &mut Commands, root: Entity, ctx: &ScreenCtx) {
    scrim(c, root);
    c.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(EDGE),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(root),
    ))
    .with_children(|col| {
        brand(col, SIZE_HERO);
        col.spawn((Node { height: px(64), ..default() }, Pickable::IGNORE));
        col.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(4), margin: UiRect::left(px(-26)), ..default() }, Pickable::IGNORE))
            .with_children(|nav| {
                nav_button(nav, "Play", UiAction::Play, 1);
                nav_button(nav, "Settings", UiAction::Settings, 2);
                nav_button(nav, "Quit", UiAction::Quit, 3);
            });
    });
    brain_card(c, root, ctx);
    c.spawn((Node::default(), Pickable::IGNORE, ChildOf(root))).with_children(|f| {
        spawn_footer(f, &[("W / S", "Navigate"), ("ENTER", "Select")]);
    });
}

/// "Enemy brain" summary: generation and your record against it.
fn brain_card(c: &mut Commands, root: Entity, ctx: &ScreenCtx) {
    let st = &ctx.brain.state;
    let wins = st.history.iter().filter(|h| h.player_won).count();
    let losses = st.history.len() - wins;
    let last = st.history.last().map(|h| {
        format!("Last match: you {} {}-{}", if h.player_won { "won" } else { "lost" }, h.player_rounds, h.enemy_rounds)
    });
    c.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: px(EDGE),
            bottom: px(44),
            width: px(380),
            flex_direction: FlexDirection::Column,
            row_gap: px(GAP_S),
            padding: UiRect::all(px(22)),
            border: UiRect::left(px(3)),
            border_radius: BorderRadius::all(px(RADIUS)),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor::all(INTEL),
        Pickable::IGNORE,
        ChildOf(root),
    ))
    .with_children(|p| {
        p.spawn(eyebrow("Enemy brain", INTEL));
        p.spawn((Node { align_items: AlignItems::Baseline, column_gap: px(GAP_M), ..default() }, Pickable::IGNORE))
            .with_children(|r| {
                r.spawn(text(format!("GEN {}", st.generation + 1), display(52.0), TEXT));
                r.spawn(text(format!("{} sim rounds", st.sim_rounds), body(SIZE_SMALL), TEXT_MUTED));
            });
        p.spawn(rule(LINE));
        p.spawn((Node { column_gap: px(GAP_L), ..default() }, Pickable::IGNORE)).with_children(|r| {
            stat(r, "Your wins", wins.to_string(), FRIENDLY);
            stat(r, "Their wins", losses.to_string(), ENEMY);
            stat(r, "Matches", st.history.len().to_string(), TEXT);
        });
        if let Some(l) = last {
            p.spawn(text(l, body(SIZE_SMALL), TEXT_MUTED));
        } else {
            p.spawn(text("No matches yet. They are watching.", body(SIZE_SMALL), TEXT_MUTED));
        }
    });
}

fn stat(p: &mut ChildSpawnerCommands, name: &str, value: String, color: Color) {
    p.spawn((Node { flex_direction: FlexDirection::Column, ..default() }, Pickable::IGNORE)).with_children(|s| {
        s.spawn(text(value, display(34.0), color));
        s.spawn(text(name.to_uppercase(), label(SIZE_TINY), TEXT_DIM));
    });
}

pub fn spawn_setup(c: &mut Commands, root: Entity, _ctx: &ScreenCtx) {
    scrim(c, root);
    c.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(EDGE),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            row_gap: px(GAP_M),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(root),
    ))
    .with_children(|col| {
        col.spawn(eyebrow("Play", ACCENT));
        col.spawn((text("MATCH SETUP", display(SIZE_H1), TEXT), Node { margin: UiRect::top(px(-10)), ..default() }));
        col.spawn(text("Best of three rounds. Afterwards the squad trains against your ghost.", body(SIZE_BODY), TEXT_MUTED));

        col.spawn((Node { height: px(GAP_L), ..default() }, Pickable::IGNORE));
        col.spawn(eyebrow("Difficulty", TEXT_MUTED));
        let diffs: Vec<(&str, &str)> = Difficulty::ALL.iter().map(|d| (d.label(), d.blurb())).collect();
        card_row(col, Field::Difficulty, 1, &diffs, 272.0);

        col.spawn((Node { height: px(GAP_M), ..default() }, Pickable::IGNORE));
        col.spawn(eyebrow("Map", TEXT_MUTED));
        let maps: Vec<(&str, &str)> = MapId::ALL.iter().map(|m| (m.label(), m.blurb())).collect();
        card_row(col, Field::Map, 2, &maps, 368.0);

        col.spawn((Node { height: px(GAP_L), ..default() }, Pickable::IGNORE));
        col.spawn((Node { column_gap: px(GAP_M), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE))
            .with_children(|r| {
                action_button(r, "Start match", UiAction::StartMatch, Some(3), true, 320.0);
                action_button(r, "Back", UiAction::Back, Some(4), false, 160.0);
            });
    });
    c.spawn((Node::default(), Pickable::IGNORE, ChildOf(root))).with_children(|f| {
        spawn_footer(f, &[("W / S", "Row"), ("A / D", "Choose"), ("ENTER", "Start"), ("ESC", "Back")]);
    });
}
