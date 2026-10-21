//! Esc during a round: dims the game and offers Resume / Settings / Restart / Main menu / Quit.
//! Pausing itself (time, physics, `Paused`) lives in `ui::screens::PauseCtl`.

use bevy::prelude::*;

use super::screens::ScreenCtx;
use super::theme::*;
use super::widgets::{UiAction, nav_button};
use crate::match_flow::ROUNDS_TO_WIN;

pub fn spawn(c: &mut Commands, root: Entity, ctx: &ScreenCtx) {
    c.spawn((fullscreen(), BackgroundColor(BG.with_alpha(0.55)), Pickable::IGNORE, ChildOf(root)));
    c.spawn((fullscreen(), menu_scrim(0.95), Pickable::IGNORE, ChildOf(root)));
    let f = &ctx.flow;
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
        col.spawn(eyebrow(&format!("Match {} \u{b7} Round {}", f.match_no.max(1), f.round.max(1)), ACCENT));
        col.spawn((text("PAUSED", display(SIZE_H1 + 24.0), TEXT), Node { margin: UiRect::new(px(-3), px(0), px(-8), px(-6)), ..default() }));
        col.spawn((Node { column_gap: px(GAP_M), align_items: AlignItems::Baseline, ..default() }, Pickable::IGNORE))
            .with_children(|r| {
                r.spawn(text("YOU", label(SIZE_LABEL), TEXT_MUTED));
                r.spawn(text(f.player_rounds.to_string(), display(34.0), FRIENDLY));
                r.spawn(text("\u{2013}", display(34.0), TEXT_DIM));
                r.spawn(text(f.enemy_rounds.to_string(), display(34.0), ENEMY));
                r.spawn(text("SQUAD", label(SIZE_LABEL), TEXT_MUTED));
                r.spawn(text(
                    format!("First to {ROUNDS_TO_WIN} \u{b7} {} \u{b7} Gen {}", ctx.settings.difficulty.label(), ctx.brain.state.generation + 1),
                    body(SIZE_SMALL),
                    TEXT_MUTED,
                ));
            });
        col.spawn((Node { height: px(48), ..default() }, Pickable::IGNORE));
        col.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(4), margin: UiRect::left(px(-26)), ..default() }, Pickable::IGNORE))
            .with_children(|nav| {
                nav_button(nav, "Resume", UiAction::Resume, 1);
                nav_button(nav, "Settings", UiAction::Settings, 2);
                nav_button(nav, "Restart match", UiAction::Restart, 3);
                nav_button(nav, "Main menu", UiAction::MainMenu, 4);
                nav_button(nav, "Quit game", UiAction::Quit, 5);
            });
        col.spawn((Node { height: px(24), ..default() }, Pickable::IGNORE));
        col.spawn(text("Restarting or leaving abandons this match; the squad does not train on it.", body(SIZE_SMALL), TEXT_DIM));
    });
    c.spawn((Node::default(), Pickable::IGNORE, ChildOf(root))).with_children(|f| {
        spawn_footer(f, &[("W / S", "Navigate"), ("ENTER", "Select"), ("ESC", "Resume")]);
    });
}
