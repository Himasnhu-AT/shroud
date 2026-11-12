//! Shroud's menus on the engine UI kit: main menu items + "Enemy brain" card, the match setup
//! screen (difficulty + map cards), the pause menu's match line and score, the crosshair
//! preview on the Gameplay settings tab, and what the game-specific buttons do. Drawn over the
//! live 3D scene, which slowly pans behind the main menu.

use bevy::prelude::*;
use engine::gfx::MainCamera;
use engine::settings::{SaveSettings, SettingsRegistry};
use engine::ui::prelude::*;
use engine::ui::screens::ScreenRequest;

use crate::match_flow::{MAX_ROUNDS, MatchFlow, ROUNDS_TO_WIN};
use crate::player::Player;
use crate::save::Brain;
use crate::settings::ShroudSettings;
use crate::state::{AppState, GameState, Paused, has_visuals};
use crate::ui::hud::spawn_crosshair;

pub struct MenuPlugin;

/// The perf overlay (F2) is an engine devtool: ship builds (no `dev`) only have F3.
#[cfg(feature = "dev")]
const DEBUG_KEYS: (&str, &str) = ("Perf / enemy debug view", "F2 / F3");
#[cfg(not(feature = "dev"))]
const DEBUG_KEYS: (&str, &str) = ("Enemy debug view", "F3");

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(wrap_map_cards);
        if let Some(mut b) = app.world_mut().get_resource_mut::<engine::input::Bindings>() {
            for (action, keys) in [
                ("Fire", "Left mouse"),
                ("Aim down sights", "Right mouse (hold)"),
                ("Reload", "R"),
                ("Primary / sidearm", "1 / 2, mouse wheel"),
                ("Last weapon / inspect", "Q / V"),
                ("Weapon light", "T"),
                ("Plant bomb (A / B) / pick up", "E (hold)"),
                ("Grenade: equip / next", "4 or G"),
                ("Throw: over / under", "Left / right mouse"),
                ("Buy menu", "B (freeze time)"),
                ("Scoreboard", "Tab (hold)"),
                DEBUG_KEYS,
            ] {
                b.set(action, keys);
            }
        }
        app.insert_resource(MenuConfig {
            eyebrow: "Adaptive squad shooter",
            main_items: vec![("Play", MenuAction::Open("setup")), ("Settings", MenuAction::Settings), ("Quit", MenuAction::Quit)],
            pause_items: vec![
                ("Resume", MenuAction::Resume),
                ("Settings", MenuAction::Settings),
                ("Restart match", MenuAction::Custom("restart")),
                ("Main menu", MenuAction::MainMenu),
                ("Quit game", MenuAction::Quit),
            ],
            pause_note: "Restarting or leaving abandons this match; the squad does not train on it and you earn no XP.",
            main_panel: Some(brain_card),
            pause_header: Some(pause_score),
            pause_eyebrow: Some(pause_eyebrow),
        })
        .register_screen("setup", build_setup)
        .settings_panel("Gameplay", crosshair_preview)
        .add_systems(OnEnter(GameState::Playing), close_screens)
        .add_systems(Update, menu_events)
        .add_systems(Update, pan_camera.run_if(in_state(AppState::MainMenu).and_then(has_visuals)))
        .add_systems(Update, viewmodel_only_in_rounds.run_if(has_visuals));
    }
}

/// Every round starts with no menu open (Restart from the pause menu re-enters `Playing`).
fn close_screens(mut requests: MessageWriter<ScreenRequest>, mut pause: engine::pause::PauseCtl) {
    requests.write(ScreenRequest::Replace(None));
    pause.set(false);
}

/// Game-specific menu buttons.
fn menu_events(
    mut events: MessageReader<MenuEvent>,
    mut flow: ResMut<MatchFlow>,
    mut app_state: ResMut<NextState<AppState>>,
    game_state: Option<ResMut<NextState<GameState>>>,
    mut save: MessageWriter<SaveSettings>,
) {
    let mut game_state = game_state;
    for MenuEvent(id) in events.read() {
        match *id {
            "start_match" => {
                save.write(SaveSettings);
                flow.abandon();
                app_state.set(AppState::InGame);
            }
            "restart" => {
                flow.abandon();
                if let Some(n) = game_state.as_mut() {
                    n.set(GameState::Playing);
                }
            }
            // The engine goes to the main menu itself.
            "main_menu" => flow.abandon(),
            _ => {}
        }
    }
}

/// Slow cinematic yaw while the main menu is up.
fn pan_camera(
    time: Res<Time<Real>>,
    mut player: Query<&mut Transform, (With<Player>, Without<MainCamera>)>,
    mut cam: Query<&mut Transform, With<MainCamera>>,
) {
    let t = time.elapsed_secs();
    for mut tf in &mut player {
        tf.rotation = Quat::from_rotation_y(0.35 + t * 0.025);
    }
    for mut tf in &mut cam {
        tf.rotation = Quat::from_rotation_x(-0.04 + (t * 0.11).sin() * 0.015);
    }
}

/// The rifle only shows in a live round (not behind menus, pause, evolution or debrief).
fn viewmodel_only_in_rounds(
    state: Option<Res<State<GameState>>>,
    paused: Res<Paused>,
    mut q: Query<(&Name, &mut Visibility)>,
    mut done: Local<Option<bool>>,
) {
    let show = state.is_some_and(|s| *s.get() == GameState::Playing) && !paused.0;
    if *done == Some(show) {
        return;
    }
    let want = if show { Visibility::Inherited } else { Visibility::Hidden };
    let mut found = false;
    for (name, mut v) in &mut q {
        if name.as_str() == "ViewModel" {
            *v = want;
            found = true;
        }
    }
    // The viewmodel spawns in PostStartup: retry until it exists.
    if found {
        *done = Some(show);
    }
}

// ---- screens and panels ----

/// "setup": map cards (first, large), difficulty, then into the match. Guns are bought in the
/// match (buy menu, B), not picked here.
fn build_setup(world: &mut World, root: Entity) {
    let t = world.resource::<Theme>().clone();
    let reg = world.resource::<SettingsRegistry>();
    let (Some(diff), Some(map)) = (reg.field("difficulty").cloned(), reg.field("map").cloned()) else { return };
    world.commands().entity(root).with_children(|p| {
        menu_backdrop(p, &t, 0.0, 1.0);
        p.spawn(menu_column(GAP_M)).with_children(|col| {
            col.spawn(eyebrow("Play", t.accent));
            col.spawn(title("Choose a map", SIZE_H1, t.text));
            col.spawn(text(
                format!("First to {ROUNDS_TO_WIN} rounds, at most {MAX_ROUNDS}. Start with a pistol and $800; buy with B during freeze time. Afterwards the squad trains against your ghost."),
                body(SIZE_BODY),
                t.text_muted,
            ));
            col.spawn(spacer(GAP_L - GAP_M));
            col.spawn(eyebrow("Map", t.text_muted));
            card_row(col, &t, &map, 1, MAP_CARD_W);
            col.spawn(spacer(0.0));
            col.spawn(eyebrow("Difficulty", t.text_muted));
            card_row(col, &t, &diff, 2, 272.0);
            col.spawn(spacer(GAP_L - GAP_M));
            col.spawn((Node { column_gap: px(GAP_M), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE)).with_children(|r| {
                action_button(r, &t, "Start match", MenuAction::Custom("start_match"), Some(3), true, 320.0);
                action_button(r, &t, "Back", UiAction::Back, Some(4), false, 160.0);
            });
        });
        spawn_footer(p, &t, &[("W / S", "Row"), ("A / D", "Choose"), ("Enter", "Start"), ("Esc", "Back")]);
    });
}

/// Map cards per row on the setup screen: the map row wraps so six or more maps fit.
const MAP_CARDS_PER_ROW: f32 = 3.0;
const MAP_CARD_W: f32 = 400.0;

/// The engine's card row is a single flex line; let the map row wrap into rows of three.
fn wrap_map_cards(add: On<Add<engine::ui::widgets::FieldRow>>, mut rows: Query<(&engine::ui::widgets::FieldRow, &mut Node)>) {
    if let Ok((row, mut node)) = rows.get_mut(add.entity)
        && row.0 == "map"
    {
        node.flex_wrap = FlexWrap::Wrap;
        node.row_gap = px(GAP_M);
        node.max_width = px(MAP_CARD_W * MAP_CARDS_PER_ROW + GAP_M * (MAP_CARDS_PER_ROW - 1.0));
    }
}

/// Main menu side card: generation and your record against it.
fn brain_card(world: &mut World, slot: Entity) {
    let t = world.resource::<Theme>().clone();
    let st = &world.resource::<Brain>().state;
    let wins = st.history.iter().filter(|h| h.player_won).count();
    let (generation, sim_rounds, matches) = (st.generation + 1, st.sim_rounds, st.history.len());
    let last = match st.history.last() {
        Some(h) => format!("Last match: you {} {}-{}", if h.player_won { "won" } else { "lost" }, h.player_rounds, h.enemy_rounds),
        None => "No matches yet. They are watching.".into(),
    };
    world.commands().entity(slot).with_children(|s| {
        s.spawn(accent_panel(&t, t.info, 22.0, GAP_S)).with_children(|p| {
            p.spawn(eyebrow("Enemy brain", t.info));
            p.spawn((Node { align_items: AlignItems::Baseline, column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
                r.spawn(text(format!("GEN {generation}"), display(52.0), t.text));
                r.spawn(text(format!("{sim_rounds} sim rounds"), body(SIZE_SMALL), t.text_muted));
            });
            p.spawn(rule(t.line));
            p.spawn((Node { column_gap: px(GAP_L), ..default() }, Pickable::IGNORE)).with_children(|r| {
                for (name, v, c) in [("Your wins", wins, t.good), ("Their wins", matches - wins, t.bad), ("Matches", matches, t.text)] {
                    r.spawn((Node { flex_direction: FlexDirection::Column, ..default() }, Pickable::IGNORE)).with_children(|st| {
                        st.spawn(text(v.to_string(), display(34.0), c));
                        st.spawn(text(name.to_uppercase(), label(SIZE_TINY), t.text_dim));
                    });
                }
            });
            p.spawn(text(last, body(SIZE_SMALL), t.text_muted));
        });
    });
    career_card(world, slot);
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Main menu: your rank, XP and how far to the next rank.
fn career_card(world: &mut World, slot: Entity) {
    let t = world.resource::<Theme>().clone();
    let c = world.resource::<crate::career::CareerSave>().career.clone();
    let rank = c.rank();
    let (frac, _) = crate::ui::career_hud::rank_line(&c);
    let next = match (c.progress(), rank.next()) {
        (Some((into, span)), Some(n)) => format!("{} XP to {}", span - into, n.name()),
        _ => "Top rank".into(),
    };
    world.commands().entity(slot).with_children(|s| {
        s.spawn((Node { height: px(GAP_M), ..default() }, Pickable::IGNORE));
        s.spawn(accent_panel(&t, t.accent, 22.0, GAP_S)).with_children(|p| {
            p.spawn(eyebrow("Your rank", t.accent));
            p.spawn((Node { align_items: AlignItems::Baseline, column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
                r.spawn(text(rank.name().to_uppercase(), display(44.0), t.text));
                r.spawn(text(format!("{} XP", c.xp), body(SIZE_SMALL), t.text_muted));
            });
            p.spawn((Node { width: percent(100), height: px(8), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.12)), Pickable::IGNORE))
                .with_child((Node { width: percent(frac * 100.0), height: percent(100), ..default() }, BackgroundColor(t.accent), Pickable::IGNORE));
            p.spawn(text(
                format!("{next} \u{b7} {} \u{b7} {} won", plural(c.matches, "match", "matches"), c.wins),
                body(SIZE_SMALL),
                t.text_muted,
            ));
        });
    });
}

/// "Match 2 · Round 3" above "PAUSED".
fn pause_eyebrow(world: &World) -> String {
    let f = world.resource::<MatchFlow>();
    format!("Match {} \u{b7} Round {}", f.match_no.max(1), f.round.max(1))
}

/// Score line under "PAUSED".
fn pause_score(world: &mut World, slot: Entity) {
    let t = world.resource::<Theme>().clone();
    let f = world.resource::<MatchFlow>();
    let (you, squad) = (f.player_rounds, f.enemy_rounds);
    let line = format!(
        "First to {ROUNDS_TO_WIN}, max {MAX_ROUNDS} rounds \u{b7} {} \u{b7} Gen {}",
        world.resource::<ShroudSettings>().difficulty.label(),
        world.resource::<Brain>().state.generation + 1
    );
    world.commands().entity(slot).with_children(|s| {
        s.spawn((Node { column_gap: px(GAP_M), align_items: AlignItems::Baseline, ..default() }, Pickable::IGNORE)).with_children(|r| {
            r.spawn(text("YOU", label(SIZE_LABEL), t.text_muted));
            r.spawn(text(you.to_string(), display(34.0), t.good));
            r.spawn(text("\u{2013}", display(34.0), t.text_dim));
            r.spawn(text(squad.to_string(), display(34.0), t.bad));
            r.spawn(text("SQUAD", label(SIZE_LABEL), t.text_muted));
            r.spawn(text(line, body(SIZE_SMALL), t.text_muted));
        });
    });
}

/// Gameplay tab: the real crosshair (kept in sync by `ui::hud`) on a sky-to-concrete strip.
fn crosshair_preview(world: &mut World, slot: Entity) {
    let t = world.resource::<Theme>().clone();
    let settings = world.resource::<ShroudSettings>().clone();
    world.commands().entity(slot).with_children(|s| {
        s.spawn(panel(&t, 22.0, GAP_M)).with_children(|card| {
            card.spawn(eyebrow("Crosshair preview", t.text_muted));
            card.spawn((
                Node {
                    width: percent(100),
                    height: px(220),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
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
            .with_children(|bx| spawn_crosshair(bx, &settings));
        });
    });
}
