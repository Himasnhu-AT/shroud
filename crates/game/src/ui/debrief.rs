//! `GameState::Debrief`: match result, what the squad learned about you, how it changed.
//! Buttons: next match (Enter) | main menu (Esc) | reset the AI (R).

use bevy::prelude::*;

use crate::match_flow::MatchFlow;
use crate::save::{Brain, LastEvolve};
use super::theme::*;
use crate::state::{AppState, GameState, Headless};
use engine::ui::prelude::{MenuAction, MenuEvent, NavBack, action_button};

pub struct DebriefPlugin;

impl Plugin for DebriefPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Debrief), spawn_debrief)
            .add_systems(Update, input.run_if(in_state(GameState::Debrief)));
    }
}

#[derive(Resource)]
struct HeadlessAdvance(Timer);

fn spawn_debrief(mut commands: Commands, headless: Res<Headless>, flow: Res<MatchFlow>, last: Res<LastEvolve>, brain: Res<Brain>) {
    if headless.0 {
        commands.insert_resource(HeadlessAdvance(Timer::from_seconds(0.5, TimerMode::Once)));
        return;
    }
    let (won, pr, er) = flow.last_result.unwrap_or((false, 0, 0));
    let title = if won { "VICTORY" } else { "DEFEAT" };
    let (insights, deltas, gen_line) = match &last.0 {
        Some(s) => (
            s.insights.clone(),
            s.deltas
                .iter()
                .map(|d| {
                    let pct = (d.after - d.before) * 100.0;
                    let arrow = if pct >= 0.0 { "up" } else { "down" };
                    format!("{}: {arrow} {:.0}%", d.label, pct.abs())
                })
                .collect::<Vec<_>>(),
            format!(
                "Generation {} is ready | trained {} generations vs your ghost in {:.1}s | sim win rate {:.0}%",
                s.generation + 1,
                s.neat_generations,
                s.elapsed_ms as f32 / 1000.0,
                s.sim_win_rate * 100.0
            ) + &match (s.real_player_win, s.ghost_player_win) {
                (Some(r), Some(g)) => format!(
                    "\nYour ghost: you won {:.0}% of rounds, the ghost won {:.0}% vs the same squad -> ghost skill now {:.2}",
                    r * 100.0,
                    g * 100.0,
                    s.ghost_skill
                ),
                _ => String::new(),
            },
        ),
        None => (vec!["Evolution failed this time; same squad again.".into()], vec![], String::new()),
    };
    let history: Vec<String> = brain
        .state
        .history
        .iter()
        .rev()
        .take(6)
        .map(|h| {
            format!(
                "Gen {}: you {} {}-{} | sim win {:.0}%",
                h.generation,
                if h.player_won { "won" } else { "lost" },
                h.player_rounds,
                h.enemy_rounds,
                h.sim_win_rate * 100.0
            )
        })
        .collect();

    let card = last.0.as_ref().and_then(|s| s.card.clone());
    let deltas = if deltas.is_empty() { vec!["No change".into()] } else { deltas };
    let history = if history.is_empty() { vec!["-".into()] } else { history };
    let color = if won { FRIENDLY } else { ENEMY };

    commands
        .spawn((
            DespawnOnExit(GameState::Debrief),
            fullscreen(),
            BackgroundColor(BG.with_alpha(0.96)),
            GlobalZIndex(20),
        ))
        .with_children(|screen| {
            screen
                .spawn((
                    Node {
                        width: percent(100),
                        height: percent(100),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        row_gap: px(GAP_M),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|root| {
                    root.spawn((Node { width: px(PANEL_W), flex_direction: FlexDirection::Column, ..default() }, Pickable::IGNORE))
                        .with_children(|h| {
                            h.spawn(eyebrow(&format!("Match complete \u{b7} Squad generation {}", brain.state.generation), ACCENT));
                            h.spawn((Node { align_items: AlignItems::Baseline, column_gap: px(GAP_L), ..default() }, Pickable::IGNORE))
                                .with_children(|r| {
                                    r.spawn(text(title, display(104.0), color));
                                    r.spawn(text(format!("{pr} \u{2013} {er}"), display(56.0), TEXT));
                                });
                            if !gen_line.is_empty() {
                                h.spawn(text(gen_line, body(SIZE_SMALL), TEXT_MUTED));
                            }
                        });
                    if let Some(card) = card {
                        root.spawn(card_panel()).with_children(|p| {
                            p.spawn(eyebrow("Their plan this match", ACCENT));
                            for (label_text, body_text) in
                                [("They learned", card.learned), ("They did", card.did), ("Did it work", card.worked), ("Next time", card.counter)]
                            {
                                p.spawn((Node { column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
                                    // Text sits inside fixed-width boxes: text nodes as direct flex siblings
                                    // get mis-measured.
                                    r.spawn((Node { width: px(140), flex_shrink: 0.0, ..default() }, Pickable::IGNORE))
                                        .with_child(text(label_text.to_uppercase(), label(SIZE_LABEL), ACCENT.with_alpha(0.85)));
                                    r.spawn((Node { width: px(PANEL_W - 40.0 - 140.0 - GAP_M), ..default() }, Pickable::IGNORE))
                                        .with_child(text(body_text, body(SIZE_BODY - 2.0), TEXT));
                                });
                            }
                        });
                    }
                    root.spawn(card_panel()).with_children(|p| {
                        p.spawn(eyebrow("What to work on", INTEL));
                        for (i, line) in insights.iter().take(4).enumerate() {
                            p.spawn((Node { column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
                                r.spawn((Node { width: px(24), flex_shrink: 0.0, ..default() }, Pickable::IGNORE))
                                    .with_child(text(format!("0{}", i + 1), label(SIZE_LABEL), INTEL));
                                r.spawn((Node { width: px(PANEL_W - 40.0 - 24.0 - GAP_M), ..default() }, Pickable::IGNORE))
                                    .with_child(text(line.clone(), body(SIZE_BODY - 2.0), TEXT));
                            });
                        }
                    });
                    root.spawn((Node { width: px(PANEL_W), column_gap: px(GAP_M), ..default() }, Pickable::IGNORE))
                        .with_children(|row| {
                            for (title, lines) in [("How they changed", deltas), ("History", history)] {
                                row.spawn((
                                    Node {
                                        flex_grow: 1.0,
                                        flex_basis: px(0),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: px(4),
                                        padding: UiRect::all(px(20)),
                                        border: UiRect::all(px(1)),
                                        border_radius: BorderRadius::all(px(RADIUS)),
                                        ..default()
                                    },
                                    BackgroundColor(PANEL),
                                    BorderColor::all(LINE),
                                    Pickable::IGNORE,
                                ))
                                .with_children(|col| {
                                    col.spawn(eyebrow(title, TEXT_MUTED));
                                    for l in lines {
                                        col.spawn(text(l, body(SIZE_SMALL), TEXT));
                                    }
                                });
                            }
                        });
                    root.spawn((Node { width: px(PANEL_W), column_gap: px(GAP_M), align_items: AlignItems::Center, margin: UiRect::top(px(GAP_S)), ..default() }, Pickable::IGNORE))
                        .with_children(|b| {
                            let t = Theme::default();
                            action_button(b, &t, "Next match", MenuAction::Custom("next_match"), Some(1), true, 300.0);
                            action_button(b, &t, "Main menu", MenuAction::MainMenu, Some(2), false, 200.0);
                            action_button(b, &t, "Reset AI", MenuAction::Custom("reset_ai"), Some(3), false, 200.0);
                        });
                });
            screen.spawn((Node::default(), Pickable::IGNORE)).with_children(|f| {
                spawn_footer(f, &Theme::default(), &[("ENTER", "Next match"), ("ESC", "Main menu"), ("R", "Reset the AI")]);
            });
        });
}

const PANEL_W: f32 = 1100.0;

fn card_panel() -> impl Bundle {
    (
        Node {
            width: px(PANEL_W),
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            padding: UiRect::all(px(20)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(RADIUS)),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor::all(LINE),
        Pickable::IGNORE,
    )
}

fn input(
    mut commands: Commands,
    time: Res<Time>,
    headless: Res<Headless>,
    mut auto: Option<ResMut<HeadlessAdvance>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut events: MessageReader<MenuEvent>,
    mut backs: MessageReader<NavBack>,
    mut flow: ResMut<MatchFlow>,
    mut next: ResMut<NextState<GameState>>,
    mut app_state: ResMut<NextState<AppState>>,
) {
    if headless.0 {
        if let Some(t) = auto.as_mut()
            && t.0.tick(time.delta()).just_finished()
        {
            next.set(GameState::Playing);
        }
        return;
    }
    // Enter / clicks go through the buttons (engine UI kit); Esc and R are shortcuts.
    let mut reset = keys.is_some_and(|k| k.just_pressed(KeyCode::KeyR));
    let to_menu = backs.read().count() > 0;
    for MenuEvent(id) in events.read() {
        match *id {
            "next_match" => next.set(GameState::Playing),
            "reset_ai" => reset = true,
            _ => {}
        }
    }
    if reset {
        commands.queue(crate::save::reset_brain);
        app_state.set(AppState::MainMenu);
    } else if to_menu {
        flow.abandon();
        app_state.set(AppState::MainMenu);
    }
}
