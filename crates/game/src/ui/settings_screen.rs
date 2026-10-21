//! Settings: Gameplay / Controls / Video / Audio tabs. Every change applies live
//! (see `ui::apply`) and is saved shortly after, and again on Back.

use bevy::prelude::*;

use super::hud::spawn_crosshair;
use super::screens::{ScreenCtx, TABS};
use super::theme::*;
use super::widgets::{Field, UiAction, action_button, field_row, tab_button};
use crate::state::GameState;

const ROWS_W: f32 = 1000.0;
const SIDE_W: f32 = 380.0;

pub fn tab_fields(tab: usize) -> &'static [Field] {
    match tab {
        0 => &[Field::Difficulty, Field::ShowCallouts, Field::CrosshairColor, Field::CrosshairSize],
        1 => &[Field::Sensitivity, Field::InvertY],
        2 => &[Field::Quality, Field::Fov, Field::WindowMode, Field::Vsync, Field::ShowFps],
        _ => &[Field::Master, Field::Sfx, Field::Ambience],
    }
}

const BINDINGS: [(&str, &str); 10] = [
    ("Move", "W A S D"),
    ("Look", "Mouse"),
    ("Fire", "Left mouse"),
    ("Reload", "R"),
    ("Jump", "Space"),
    ("Crouch", "Ctrl / C"),
    ("Sprint", "Shift"),
    ("Pause", "Esc"),
    ("Perf overlay", "F2"),
    ("Enemy debug view", "F3"),
];

pub fn spawn(c: &mut Commands, root: Entity, tab: usize, ctx: &ScreenCtx) {
    let in_match = *ctx.state.get() == GameState::Playing;
    // Stronger scrim than the main menu: this is a reading screen.
    c.spawn((fullscreen(), BackgroundColor(BG.with_alpha(if in_match { 0.80 } else { 0.62 })), Pickable::IGNORE, ChildOf(root)));
    c.spawn((fullscreen(), menu_scrim(0.9), Pickable::IGNORE, ChildOf(root)));

    c.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(EDGE),
            top: px(84),
            flex_direction: FlexDirection::Column,
            row_gap: px(GAP_M),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(root),
    ))
    .with_children(|col| {
        col.spawn(eyebrow(if in_match { "Paused" } else { "Options" }, ACCENT));
        col.spawn((text("SETTINGS", display(SIZE_H1), TEXT), Node { margin: UiRect::top(px(-10)), ..default() }));

        // Tabs.
        col.spawn((
            Node { column_gap: px(GAP_XS), margin: UiRect::new(px(-22), px(0), px(GAP_S), px(0)), ..default() },
            Pickable::IGNORE,
        ))
        .with_children(|t| {
            for (i, name) in TABS.iter().enumerate() {
                tab_button(t, name, i, i == tab);
            }
        });
        col.spawn((Node { width: px(ROWS_W + GAP_XL + SIDE_W), ..default() }, Pickable::IGNORE))
            .with_child(rule(LINE));

        col.spawn((Node { column_gap: px(GAP_XL), margin: UiRect::top(px(GAP_M)), ..default() }, Pickable::IGNORE))
            .with_children(|body_row| {
                body_row
                    .spawn((Node { width: px(ROWS_W), flex_direction: FlexDirection::Column, row_gap: px(6), ..default() }, Pickable::IGNORE))
                    .with_children(|rows| {
                        let fields = tab_fields(tab);
                        for (i, f) in fields.iter().enumerate() {
                            field_row(rows, *f, i as u32 + 1);
                        }
                        rows.spawn((
                            Node { column_gap: px(GAP_M), margin: UiRect::top(px(GAP_L)), ..default() },
                            Pickable::IGNORE,
                        ))
                        .with_children(|b| {
                            let n = fields.len() as u32;
                            action_button(b, "Back", UiAction::Back, Some(n + 1), false, 180.0);
                            action_button(b, "Reset tab", UiAction::ResetTab, Some(n + 2), false, 180.0);
                        });
                    });
                body_row
                    .spawn((Node { width: px(SIDE_W), flex_direction: FlexDirection::Column, row_gap: px(GAP_M), ..default() }, Pickable::IGNORE))
                    .with_children(|side| side_panel(side, tab, ctx));
            });
    });

    c.spawn((Node::default(), Pickable::IGNORE, ChildOf(root))).with_children(|f| {
        spawn_footer(f, &[("Q / E", "Switch tab"), ("W / S", "Select"), ("A / D", "Adjust"), ("ESC", "Back")]);
    });
}

fn side_panel(p: &mut ChildSpawnerCommands, tab: usize, ctx: &ScreenCtx) {
    match tab {
        0 => {
            p.spawn(panel(22.0, GAP_M)).with_children(|card| {
                card.spawn(eyebrow("Crosshair preview", TEXT_MUTED));
                card.spawn((
                    Node {
                        width: percent(100),
                        height: px(220),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(px(3)),
                        ..default()
                    },
                    // A sky-to-concrete strip so the colour reads against both.
                    BackgroundGradient::from(LinearGradient::new(
                        std::f32::consts::PI,
                        vec![
                            ColorStop::new(Color::srgb(0.62, 0.66, 0.72), percent(0)),
                            ColorStop::new(Color::srgb(0.46, 0.47, 0.48), percent(50)),
                            ColorStop::new(Color::srgb(0.2, 0.21, 0.22), percent(100)),
                        ],
                    )),
                    Pickable::IGNORE,
                ))
                .with_children(|bx| spawn_crosshair(bx, &ctx.settings));
            });
        }
        1 => {
            p.spawn(panel(22.0, GAP_S)).with_children(|card| {
                card.spawn(eyebrow("Key bindings", TEXT_MUTED));
                for (action, key) in BINDINGS {
                    card.spawn((
                        Node { justify_content: JustifyContent::SpaceBetween, padding: UiRect::vertical(px(4)), ..default() },
                        Pickable::IGNORE,
                    ))
                    .with_children(|r| {
                        r.spawn(text(action, body(SIZE_SMALL + 1.0), TEXT_MUTED));
                        r.spawn(text(key, body_bold(SIZE_SMALL + 1.0), TEXT));
                    });
                }
                card.spawn(text("Rebinding is not available yet.", body(SIZE_TINY), TEXT_DIM));
            });
        }
        2 => {
            p.spawn(panel(22.0, GAP_S)).with_children(|card| {
                card.spawn(eyebrow("Presets", TEXT_MUTED));
                for (q, what) in [
                    ("Low", "No ambient occlusion, FXAA, no bloom, small shadow maps."),
                    ("Medium", "Temporal AA, no ambient occlusion."),
                    ("High", "Temporal AA and ambient occlusion."),
                    ("Ultra", "Best ambient occlusion, sharpest shadows."),
                ] {
                    card.spawn((Node { flex_direction: FlexDirection::Column, padding: UiRect::vertical(px(3)), ..default() }, Pickable::IGNORE))
                        .with_children(|r| {
                            r.spawn(text(q.to_uppercase(), label(SIZE_LABEL), TEXT));
                            r.spawn(text(what, body(SIZE_TINY + 1.0), TEXT_MUTED));
                        });
                }
            });
        }
        _ => {}
    }
}
