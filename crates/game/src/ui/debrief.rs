//! `GameState::Debrief`: match result, what the squad learned about you, how it changed.
//! Enter/click: next match | R: reset the AI | Esc: quit.

use bevy::prelude::*;

use crate::match_flow::MatchFlow;
use crate::save::{Brain, LastEvolve};
use crate::state::{GameState, Headless};

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
    let deltas = if deltas.is_empty() { vec!["-".into()] } else { deltas };
    let history = if history.is_empty() { vec!["-".into()] } else { history };

    commands
        .spawn((
            DespawnOnExit(GameState::Debrief),
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(12),
                padding: UiRect::all(px(24)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.94)),
        ))
        .with_children(|root| {
            root.spawn((
                Text::new(format!("{title}  {pr}-{er}")),
                TextFont::from_font_size(48.0),
                TextColor(if won { Color::srgb(0.5, 1.0, 0.6) } else { Color::srgb(1.0, 0.4, 0.35) }),
            ));
            root.spawn((
                Text::new(gen_line),
                TextFont::from_font_size(15.0),
                TextLayout::justify(Justify::Center),
                TextColor(MUTED),
            ));
            if let Some(card) = card {
                root.spawn(panel()).with_children(|p| {
                    p.spawn(heading("THEIR PLAN THIS MATCH", ACCENT));
                    for (label, text) in
                        [("They learned", card.learned), ("They did", card.did), ("Did it work", card.worked), ("Next time", card.counter)]
                    {
                        // One text per row (the font is monospace, so a padded label lines up);
                        // side-by-side text nodes get mis-measured by the flex layout.
                        p.spawn((Text::default(), TextFont::from_font_size(17.0), TextColor(BODY))).with_children(|t| {
                            t.spawn((TextSpan::new(format!("{label:<13}")), TextFont::from_font_size(17.0), TextColor(ACCENT)));
                            t.spawn((TextSpan::new(text), TextFont::from_font_size(17.0), TextColor(BODY)));
                        });
                    }
                });
            }
            root.spawn(panel()).with_children(|p| {
                p.spawn(heading("WHAT TO WORK ON", INSIGHT));
                for (i, line) in insights.iter().take(4).enumerate() {
                    p.spawn((Text::new(format!("{}. {line}", i + 1)), TextFont::from_font_size(17.0), TextColor(BODY)));
                }
            });
            root.spawn(Node { width: px(PANEL_W), max_width: percent(96), flex_direction: FlexDirection::Row, column_gap: px(16), ..default() })
                .with_children(|row| {
                    for (title, lines) in [("HOW THEY CHANGED", deltas), ("HISTORY", history)] {
                        row.spawn(Node { flex_grow: 1.0, flex_basis: px(0), flex_direction: FlexDirection::Column, row_gap: px(3), ..default() })
                            .with_children(|col| {
                                col.spawn(heading(title, MUTED));
                                for l in lines {
                                    col.spawn((Text::new(l), TextFont::from_font_size(15.0), TextColor(BODY)));
                                }
                            });
                    }
                });
            root.spawn((Text::new("Enter / click: next match    R: reset AI    Esc: quit"), TextFont::from_font_size(16.0), TextColor(MUTED)));
        });
}

const PANEL_W: f32 = 980.0;
const BODY: Color = Color::srgb(0.86, 0.89, 0.95);
const MUTED: Color = Color::srgb(0.6, 0.65, 0.75);
const ACCENT: Color = Color::srgb(1.0, 0.75, 0.35);
const INSIGHT: Color = Color::srgb(0.55, 0.85, 1.0);

fn panel() -> impl Bundle {
    (
        Node {
            width: px(PANEL_W),
            max_width: percent(96),
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            padding: UiRect::axes(px(18), px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.08, 0.1, 0.14, 0.9)),
        BorderColor::all(Color::srgba(0.4, 0.45, 0.55, 0.5)),
    )
}

fn heading(text: &str, color: Color) -> impl Bundle {
    (Text::new(text), TextFont::from_font_size(14.0), TextColor(color))
}

fn input(
    time: Res<Time>,
    headless: Res<Headless>,
    mut auto: Option<ResMut<HeadlessAdvance>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut brain: ResMut<Brain>,
    tracer: Option<Res<crate::ops::trace::GameTracer>>,
    mut next: ResMut<NextState<GameState>>,
    mut exit: MessageWriter<AppExit>,
) {
    if headless.0 {
        if let Some(t) = auto.as_mut()
            && t.0.tick(time.delta()).just_finished()
        {
            next.set(GameState::Playing);
        }
        return;
    }
    let (Some(mouse), Some(keys)) = (mouse, keys) else { return };
    if mouse.just_pressed(MouseButton::Left) || keys.just_pressed(KeyCode::Enter) {
        next.set(GameState::Playing);
    } else if keys.just_pressed(KeyCode::KeyR) {
        brain.reset(tracer.as_deref());
        next.set(GameState::Menu);
    } else if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}
