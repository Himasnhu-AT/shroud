//! Scoreboard (hold Tab during a round): you and every squad member — callsign, loadout role,
//! gun, kills, deaths, damage, accuracy — totalled over the match. [`Scoreboard`] is kept up to
//! date headless too (round summary, control port); the panel only exists with visuals and its
//! text is rebuilt only while open and when a number changed.

use bevy::prelude::*;
use brain::loadout::Role;
use brain::squad::{CALLSIGNS, callsign};
use brain::weapons::WeaponKind;

use super::hud::HideWhenPaused;
use super::killfeed::{KillEvent, detect_kills};
use super::theme::*;
use crate::armed::Armed;
use crate::enemy::{Enemy, EnemyCombat};
use crate::match_flow::MatchFlow;
use crate::player::Player;
use crate::state::{GameState, Paused, has_visuals};
use crate::telemetry::Telemetry;
use crate::weapon::{Damage, WeaponSet};

/// Squad rows the panel has room for (one per callsign).
const ROWS: usize = CALLSIGNS.len();
const COLS: [(&str, f32); 7] = [("NAME", 170.0), ("ROLE", 130.0), ("GUN", 130.0), ("K", 56.0), ("D", 56.0), ("DMG", 80.0), ("ACC", 80.0)];

/// One side's numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Line {
    pub kills: u32,
    pub deaths: u32,
    pub damage: f32,
    /// Trigger pulls and pulls that hit (enemies; the player's accuracy comes from telemetry).
    pub shots: u32,
    pub hits: u32,
}

impl Line {
    fn plus(&self, o: &Line) -> Line {
        Line { kills: self.kills + o.kills, deaths: self.deaths + o.deaths, damage: self.damage + o.damage, shots: self.shots + o.shots, hits: self.hits + o.hits }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnemyRow {
    pub slot: usize,
    pub callsign: &'static str,
    pub role: Role,
    /// The loadout's primary.
    pub gun: WeaponKind,
    pub alive: bool,
    /// Finished rounds of this match, and the current round.
    pub before: Line,
    pub round: Line,
}

impl EnemyRow {
    pub fn total(&self) -> Line {
        self.before.plus(&self.round)
    }
}

/// Match scoreboard. Squad slots keep their genomes (and loadouts) across a match's rounds,
/// so rows are per slot.
#[derive(Resource, Debug, Default)]
pub struct Scoreboard {
    pub enemies: Vec<EnemyRow>,
    pub player_before: Line,
    pub player_round: Line,
}

impl Scoreboard {
    pub fn player_total(&self) -> Line {
        self.player_before.plus(&self.player_round)
    }
}

pub struct ScoreboardPlugin;

impl Plugin for ScoreboardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Scoreboard>()
            .add_systems(OnEnter(GameState::Playing), (new_round.after(crate::match_flow::begin_round), spawn_board.run_if(has_visuals)))
            .add_systems(Update, track.after(detect_kills).after(WeaponSet).run_if(in_state(GameState::Playing)))
            .add_systems(Update, (toggle_board, fill_board).chain().after(track).run_if(has_visuals).run_if(in_state(GameState::Playing)));
    }
}

fn new_round(flow: Res<MatchFlow>, mut sb: ResMut<Scoreboard>) {
    if flow.round <= 1 {
        *sb = Scoreboard::default();
        return;
    }
    let sb = &mut *sb;
    sb.player_before = sb.player_total();
    sb.player_round = Line::default();
    for r in &mut sb.enemies {
        r.before = r.total();
        r.round = Line::default();
        r.alive = true;
    }
}

pub(crate) fn track(
    mut sb: ResMut<Scoreboard>,
    mut kills: MessageReader<KillEvent>,
    mut dmg: MessageReader<Damage>,
    players: Query<Entity, With<Player>>,
    enemies: Query<(Entity, &Enemy, &EnemyCombat)>,
) {
    let player = players.single().ok();
    let mut changed = false;
    let s = sb.bypass_change_detection();
    for k in kills.read() {
        if k.by_player {
            s.player_round.kills += 1;
        }
        if k.on_player {
            s.player_round.deaths += 1;
        }
        if let Some(r) = k.victim_slot.and_then(|v| s.enemies.iter_mut().find(|r| r.slot == v)) {
            r.round.deaths += 1;
        }
        changed = true;
    }
    for d in dmg.read() {
        if Some(d.source) == player && enemies.contains(d.target) {
            s.player_round.damage += d.amount;
            changed = true;
        }
    }
    let mut seen = [false; 16];
    for (_, en, c) in &enemies {
        if let Some(f) = seen.get_mut(en.slot) {
            *f = true;
        }
        let i = match s.enemies.iter().position(|r| r.slot == en.slot) {
            Some(i) => i,
            None => {
                s.enemies.push(EnemyRow {
                    slot: en.slot,
                    callsign: callsign(en.slot),
                    role: c.role,
                    gun: c.gun.primary,
                    alive: true,
                    before: Line::default(),
                    round: Line::default(),
                });
                s.enemies.sort_by_key(|r| r.slot);
                changed = true;
                s.enemies.iter().position(|r| r.slot == en.slot).unwrap_or(0)
            }
        };
        let r = &mut s.enemies[i];
        let round = Line { kills: c.report.kills, deaths: r.round.deaths, damage: c.report.damage_dealt, shots: c.shots, hits: c.shots_hit };
        let want = EnemyRow { role: c.role, gun: c.gun.primary, alive: !c.dead, round, ..*r };
        if *r != want {
            *r = want;
            changed = true;
        }
    }
    // Gone (dead and despawned): not alive.
    for r in &mut s.enemies {
        if r.alive && !seen.get(r.slot).copied().unwrap_or(false) {
            r.alive = false;
            changed = true;
        }
    }
    if changed {
        sb.set_changed();
    }
}

#[derive(Component)]
struct BoardRoot;
#[derive(Component)]
struct BoardTitle;
#[derive(Component)]
struct BoardRow(usize);
#[derive(Component)]
struct Cell {
    row: usize,
    col: usize,
}

fn spawn_board(mut commands: Commands) {
    commands
        .spawn((
            Name::new("Scoreboard"),
            BoardRoot,
            HideWhenPaused,
            DespawnOnExit(GameState::Playing),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            GlobalZIndex(12),
            Pickable::IGNORE,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                    padding: UiRect::all(px(24)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(RADIUS)),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(LINE),
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                p.spawn((BoardTitle, eyebrow("Scoreboard", ACCENT)));
                board_row(p, None, true);
                for r in 0..=ROWS {
                    board_row(p, Some(r), false);
                }
            });
        });
}

fn board_row(p: &mut ChildSpawnerCommands, marker: Option<usize>, header: bool) {
    let mut e = p.spawn((
        Node { align_items: AlignItems::Center, padding: UiRect::axes(px(8), px(5)), ..default() },
        BackgroundColor(if header { Color::NONE } else { Color::srgba(1.0, 1.0, 1.0, 0.03) }),
        Pickable::IGNORE,
    ));
    if let Some(r) = marker {
        e.insert(BoardRow(r));
    }
    e.with_children(|r| {
        for (col, (name, w)) in COLS.iter().enumerate() {
            r.spawn((Node { width: px(*w), flex_shrink: 0.0, ..default() }, Pickable::IGNORE)).with_children(|c| {
                if header {
                    c.spawn(text(*name, label(SIZE_TINY + 1.0), TEXT_MUTED));
                } else if let Some(row) = marker {
                    c.spawn((Cell { row, col }, text("", label(SIZE_LABEL), TEXT)));
                }
            });
        }
    });
}

fn toggle_board(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    paused: Res<Paused>,
    mut root: Query<&mut Visibility, With<BoardRoot>>,
) {
    let held = keys.is_some_and(|k| k.pressed(KeyCode::Tab)) && !paused.0;
    let want = if held { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut root {
        if *v != want {
            *v = want;
        }
    }
}

fn fill_board(
    sb: Res<Scoreboard>,
    flow: Res<MatchFlow>,
    telemetry: Res<Telemetry>,
    player: Query<Option<&Armed>, With<Player>>,
    root: Query<&Visibility, With<BoardRoot>>,
    mut title: Query<&mut Text, (With<BoardTitle>, Without<Cell>)>,
    mut rows: Query<(&BoardRow, &mut Node)>,
    mut cells: Query<(&Cell, &mut Text, &mut TextColor)>,
    mut last: Local<(bool, u32, u32, u32)>,
) {
    let open = root.single().is_ok_and(|v| *v != Visibility::Hidden);
    let shots = telemetry.0.shots.iter().sum::<f32>() as u32;
    let hits = telemetry.0.hits.iter().sum::<f32>() as u32;
    let fresh = open && (!last.0 || sb.is_changed() || last.1 != shots || last.2 != hits || last.3 != flow.round);
    *last = (open, shots, hits, flow.round);
    if !fresh {
        return;
    }
    if let Ok(mut t) = title.single_mut() {
        let s = format!("SCOREBOARD \u{b7} ROUND {} \u{b7} YOU {} - {} SQUAD", flow.round.max(1), flow.player_rounds, flow.enemy_rounds);
        if t.0 != s {
            t.0 = s;
        }
    }
    for (r, mut n) in &mut rows {
        let show = r.0 == 0 || r.0 <= sb.enemies.len();
        let d = if show { Display::Flex } else { Display::None };
        if n.display != d {
            n.display = d;
        }
    }
    let acc = |h: u32, s: u32| if s == 0 { "-".to_string() } else { format!("{:.0}%", 100.0 * h as f32 / s as f32) };
    let p = sb.player_total();
    let pgun = player.single().ok().flatten().map_or(WeaponKind::Rifle, |a| a.0);
    for (c, mut t, mut col) in &mut cells {
        let (s, color) = if c.row == 0 {
            let s = match c.col {
                0 => "YOU".to_string(),
                1 => "-".to_string(),
                2 => pgun.label().to_uppercase(),
                3 => p.kills.to_string(),
                4 => p.deaths.to_string(),
                5 => format!("{:.0}", p.damage),
                _ => acc(hits, shots),
            };
            (s, FRIENDLY)
        } else if let Some(r) = sb.enemies.get(c.row - 1) {
            let tot = r.total();
            let s = match c.col {
                0 => r.callsign.to_uppercase(),
                1 => r.role.label().to_uppercase(),
                2 => r.gun.label().to_uppercase(),
                3 => tot.kills.to_string(),
                4 => tot.deaths.to_string(),
                5 => format!("{:.0}", tot.damage),
                _ => acc(tot.hits, tot.shots),
            };
            (s, if !r.alive { TEXT_DIM } else if c.col == 0 { ENEMY } else { TEXT })
        } else {
            (String::new(), TEXT)
        };
        if t.0 != s {
            t.0 = s;
        }
        if col.0 != color {
            col.0 = color;
        }
    }
}
