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

fn section(title: &str, lines: Vec<String>) -> impl Bundle {
    let mut text = format!("{title}\n");
    for l in lines {
        text.push_str(&format!("  - {l}\n"));
    }
    (Text::new(text), TextFont::from_font_size(19.0), TextColor(Color::srgb(0.85, 0.88, 0.95)))
}

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
                    let arrow = if pct >= 0.0 { "▲" } else { "▼" };
                    format!("{} {arrow} {:+.0}%", d.label, pct)
                })
                .collect::<Vec<_>>(),
            format!(
                "Generation {} is ready | trained {} generations vs your ghost in {:.1}s | sim win rate {:.0}%",
                s.generation + 1,
                s.neat_generations,
                s.elapsed_ms as f32 / 1000.0,
                s.sim_win_rate * 100.0
            ),
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

    commands.spawn((
        DespawnOnExit(GameState::Debrief),
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(14),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.94)),
        children![
            (
                Text::new(format!("{title}  {pr}-{er}")),
                TextFont::from_font_size(52.0),
                TextColor(if won { Color::srgb(0.5, 1.0, 0.6) } else { Color::srgb(1.0, 0.4, 0.35) }),
            ),
            (Text::new(gen_line), TextFont::from_font_size(16.0), TextColor(Color::srgb(0.6, 0.65, 0.75))),
            section("What they learned", insights),
            section("How they changed", if deltas.is_empty() { vec!["—".into()] } else { deltas }),
            section("History", if history.is_empty() { vec!["—".into()] } else { history }),
            (
                Text::new("Enter / click: next match    R: reset AI    Esc: quit"),
                TextFont::from_font_size(18.0),
                TextColor(Color::srgb(0.6, 0.65, 0.75)),
            ),
        ],
    ));
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
