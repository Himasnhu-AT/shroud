//! `GameState::Debrief`: match result, the squad's plan and how it changed (the game's hook,
//! always first), what to work on, then a compact career strip.
//! Buttons: next match (Enter) | main menu (Esc) | retire this squad (resets the AI: click
//! twice, see [`RetireStep`]; deliberately no key shortcut, R is reload).

use bevy::prelude::*;

use crate::career::{CareerSave, MatchCareer};
use crate::match_flow::MatchFlow;
use brain::career::{Award, Rank};
use crate::save::{Brain, LastEvolve};
use super::theme::*;
use crate::state::{AppState, GameState, Headless};
use engine::ui::prelude::{MenuAction, MenuEvent, NavBack, Toast, UiAction, action_button};

pub struct DebriefPlugin;

impl Plugin for DebriefPlugin {
    fn build(&self, app: &mut App) {
        // Every visit starts un-armed (a click armed before "Next match" doesn't carry over).
        app.init_resource::<RetireStep>()
            .add_systems(OnEnter(GameState::Debrief), (spawn_debrief, |mut r: ResMut<RetireStep>| *r = RetireStep::Idle))
            .add_systems(Update, (input, animate_xp).run_if(in_state(GameState::Debrief)));
    }
}

#[derive(Resource)]
struct HeadlessAdvance(Timer);

fn spawn_debrief(
    mut commands: Commands,
    headless: Res<Headless>,
    flow: Res<MatchFlow>,
    last: Res<LastEvolve>,
    brain: Res<Brain>,
    mc: Res<MatchCareer>,
    save: Res<CareerSave>,
) {
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
            generation_line(s.generation + 1, s.sim_rounds, s.real_player_win, s.ghost_player_win),
        ),
        None => (vec!["Evolution failed this time; same squad again.".into()], vec![], String::new()),
    };
    let history: Vec<String> = brain
        .state
        .history
        .iter()
        .rev()
        .take(HISTORY_LINES)
        .map(|h| format!("Gen {}: you {} {}\u{2013}{}", h.generation, if h.player_won { "won" } else { "lost" }, h.player_rounds, h.enemy_rounds))
        .collect();

    let card = last.0.as_ref().and_then(|s| s.card.clone());
    // What the squad will carry next match, and why (the learned loadout): part of how they changed.
    let loadout_note = last.0.as_ref().and_then(|s| s.loadout_note.clone());
    let loadout_line = last.0.as_ref().map(|s| loadout_summary(&s.next_loadout)).filter(|l| !l.is_empty());
    let mut deltas: Vec<String> = deltas.into_iter().take(DELTA_LINES).collect();
    if deltas.is_empty() {
        deltas.push("No change".into());
    }
    let history = if history.is_empty() { vec!["-".into()] } else { history };
    let color = if won { FRIENDLY } else { ENEMY };
    let award = mc.award.as_ref().filter(|_| mc.match_no == flow.match_no);

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
                                    r.spawn(text(title, display(96.0), color));
                                    r.spawn(text(format!("{pr} \u{2013} {er}"), display(52.0), TEXT));
                                });
                            if !gen_line.is_empty() {
                                h.spawn(text(gen_line, body(SIZE_SMALL), TEXT_MUTED));
                            }
                        });
                    // The game's hook first: what the squad tried against you, and how it changed.
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
                    root.spawn((Node { width: px(PANEL_W), column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|row| {
                        side_panel(row, "How they changed", ENEMY, 2.0, |col| {
                            if let Some(note) = loadout_note {
                                col.spawn(text(note, body_bold(SIZE_BODY - 2.0), TEXT));
                            }
                            if let Some(line) = loadout_line {
                                col.spawn(text(line, label(SIZE_TINY), TEXT_MUTED));
                            }
                            for l in deltas {
                                col.spawn(text(l, body(SIZE_SMALL), TEXT));
                            }
                        });
                        side_panel(row, "History", TEXT_MUTED, 1.0, |col| {
                            for l in history {
                                col.spawn(text(l, body(SIZE_SMALL), TEXT));
                            }
                        });
                    });
                    root.spawn(card_panel()).with_children(|p| {
                        p.spawn(eyebrow("What to work on", INTEL));
                        // Two tips when the career strip also needs the room.
                        let n = if award.is_some() { 2 } else { 3 };
                        for (i, line) in insights.iter().take(n).enumerate() {
                            p.spawn((Node { column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
                                r.spawn((Node { width: px(24), flex_shrink: 0.0, ..default() }, Pickable::IGNORE))
                                    .with_child(text(format!("0{}", i + 1), label(SIZE_LABEL), INTEL));
                                r.spawn((Node { width: px(PANEL_W - 40.0 - 24.0 - GAP_M), ..default() }, Pickable::IGNORE))
                                    .with_child(text(line.clone(), body(SIZE_BODY - 2.0), TEXT));
                            });
                        }
                    });
                    if let Some(award) = award {
                        career_strip(root, &mc, award, mc.xp_before, save.career.xp);
                    }
                    root.spawn((Node { width: px(PANEL_W), column_gap: px(GAP_M), align_items: AlignItems::Center, margin: UiRect::top(px(GAP_XS)), ..default() }, Pickable::IGNORE))
                        .with_children(|b| {
                            let t = Theme::default();
                            action_button(b, &t, "Next match", MenuAction::Custom("next_match"), Some(1), true, 300.0);
                            action_button(b, &t, "Main menu", MenuAction::MainMenu, Some(2), false, 200.0);
                            action_button(b, &t, "Retire this squad", MenuAction::Custom("reset_ai"), Some(3), false, 260.0);
                        });
                });
            screen.spawn((Node::default(), Pickable::IGNORE)).with_children(|f| {
                spawn_footer(f, &Theme::default(), &[("ENTER", "Next match"), ("ESC", "Main menu")]);
            });
        });
}

const PANEL_W: f32 = 1100.0;
/// Lines in "How they changed" (trait changes) and "History" (past matches).
const DELTA_LINES: usize = 4;
const HISTORY_LINES: usize = 5;

/// The header's plain-words line about the new squad and the player's ghost:
/// "Generation 5 is ready. They trained 1,240 rounds against your ghost (a copy of how you play)."
fn generation_line(generation: u32, sim_rounds: u64, real_win: Option<f32>, ghost_win: Option<f32>) -> String {
    let mut s = format!("Generation {generation} is ready. They trained {} rounds against your ghost (a copy of how you play).", thousands(sim_rounds));
    if let (Some(r), Some(g)) = (real_win, ghost_win) {
        s += &format!(" Against this squad you won {:.0}% of rounds; your ghost won {:.0}%.", r * 100.0, g * 100.0);
    }
    s
}

/// 1240 -> "1,240".
fn thousands(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::with_capacity(d.len() + d.len() / 3);
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One column of the "How they changed / History" row (`grow`: its share of the width).
fn side_panel(row: &mut ChildSpawnerCommands, title: &str, color: Color, grow: f32, fill: impl FnOnce(&mut ChildSpawnerCommands)) {
    row.spawn((
        Node {
            flex_grow: grow,
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
        col.spawn(eyebrow(title, color));
        fill(col);
    });
}

/// Seconds the XP bar takes to fill from the old XP to the new.
const XP_ANIM_S: f32 = 1.8;
/// Delay before it starts.
const XP_ANIM_DELAY: f32 = 0.5;

#[derive(Component)]
struct XpAnim {
    from: u32,
    to: u32,
    rank_before: Rank,
    t: f32,
    /// Animation time the bar crossed into the new rank.
    promoted_at: Option<f32>,
}
#[derive(Component)]
struct XpFill;
#[derive(Component)]
struct XpRankText;
#[derive(Component)]
struct XpNumbers;
#[derive(Component)]
struct Promoted;

/// The career as one compact strip: XP earned, the rank and an XP bar that fills from the old
/// XP to the new (wrapping at each rank), objectives done, and a PROMOTED line that pops in when
/// the bar crosses into a new rank. The main menu keeps the full record.
fn career_strip(root: &mut ChildSpawnerCommands, mc: &MatchCareer, award: &Award, from: u32, to: u32) {
    const BAR_W: f32 = 440.0;
    let status = mc.status();
    let done = status.iter().filter(|s| s.0).count();
    root.spawn((card_panel(), XpAnim { from, to, rank_before: award.rank_before, t: -XP_ANIM_DELAY, promoted_at: None })).with_children(|p| {
        p.spawn((Node { column_gap: px(GAP_L), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE)).with_children(|row| {
            // XP earned.
            row.spawn((Node { width: px(170), flex_shrink: 0.0, flex_direction: FlexDirection::Column, ..default() }, Pickable::IGNORE)).with_children(|c| {
                c.spawn(eyebrow("Career", ACCENT));
                c.spawn(text(format!("+{} XP", award.total), display(30.0), ACCENT));
            });
            // Rank and bar.
            row.spawn((Node { width: px(BAR_W), flex_shrink: 0.0, flex_direction: FlexDirection::Column, row_gap: px(5), ..default() }, Pickable::IGNORE)).with_children(|c| {
                c.spawn((Node { width: px(BAR_W), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Baseline, ..default() }, Pickable::IGNORE)).with_children(|r| {
                    r.spawn((XpRankText, text(award.rank_before.name().to_uppercase(), display(24.0), TEXT)));
                    r.spawn((XpNumbers, text("", label(SIZE_TINY), TEXT_MUTED)));
                });
                c.spawn((Node { width: px(BAR_W), height: px(8), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.12)), Pickable::IGNORE))
                    .with_child((XpFill, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(ACCENT), Pickable::IGNORE));
            });
            // Objectives, then the promotion (shown when the bar crosses into the new rank).
            row.spawn((Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, align_items: AlignItems::FlexEnd, ..default() }, Pickable::IGNORE)).with_children(|c| {
                c.spawn(text(format!("OBJECTIVES {done}/{}", mc.objectives.len()), label(SIZE_LABEL), if done > 0 { FRIENDLY } else { TEXT_MUTED }));
                if award.promoted() {
                    c.spawn((
                        Promoted,
                        text(format!("PROMOTED TO {}", award.rank_after.name().to_uppercase()), display(30.0), ACCENT),
                        TextShadow::default(),
                        UiTransform::IDENTITY,
                        Visibility::Hidden,
                    ));
                }
            });
        });
    });
}

/// Fill the XP bar from the old XP to the new; the rank name follows, and PROMOTED pops when the
/// bar crosses into a new rank.
fn animate_xp(
    time: Res<Time>,
    mut anims: Query<&mut XpAnim>,
    mut fill: Query<&mut Node, With<XpFill>>,
    mut texts: ParamSet<(Query<&mut Text, With<XpRankText>>, Query<&mut Text, With<XpNumbers>>)>,
    mut promoted: Query<(&mut Visibility, &mut UiTransform), With<Promoted>>,
) {
    let Ok(mut a) = anims.single_mut() else { return };
    let before = a.t;
    a.t += time.delta_secs();
    if before >= XP_ANIM_S + 1.0 {
        return;
    }
    let k = (a.t / XP_ANIM_S).clamp(0.0, 1.0);
    let ease = 1.0 - (1.0 - k).powi(3);
    let xp = a.from + ((a.to.saturating_sub(a.from)) as f32 * ease).round() as u32;
    let c = brain::career::Career { xp, ..Default::default() };
    let rank = c.rank();
    let (frac, _) = super::career_hud::rank_line(&c);
    if let Ok(mut n) = fill.single_mut() {
        n.width = percent(frac * 100.0);
    }
    if let Ok(mut t) = texts.p0().single_mut() {
        let s = rank.name().to_uppercase();
        if t.0 != s {
            t.0 = s;
        }
    }
    if let Ok(mut t) = texts.p1().single_mut() {
        let s = match (c.progress(), rank.next()) {
            (Some((into, span)), Some(next)) => format!("{into} / {span} XP TO {}", next.name().to_uppercase()),
            _ => format!("{xp} XP \u{b7} TOP RANK"),
        };
        if t.0 != s {
            t.0 = s;
        }
    }
    if rank > a.rank_before
        && let Ok((mut v, mut tf)) = promoted.single_mut()
    {
        let now = a.t;
        let at = *a.promoted_at.get_or_insert(now);
        if *v != Visibility::Inherited {
            *v = Visibility::Inherited;
        }
        // Pop in: 1.6x settling to 1x over 0.35 s.
        let p = ((now - at) / 0.35).clamp(0.0, 1.0);
        tf.scale = Vec2::splat(1.0 + 0.6 * (1.0 - p).powi(3));
    }
}

/// "NEXT SQUAD: 2 RIFLEMEN  ·  1 BREACHER  ·  1 MARKSMAN" (most common role first).
fn loadout_summary(kinds: &[brain::weapons::WeaponKind]) -> String {
    use brain::loadout::Role;
    let mut counts: Vec<(Role, usize)> = Role::ALL.iter().map(|&r| (r, kinds.iter().filter(|&&k| Role::of(k) == r).count())).filter(|(_, n)| *n > 0).collect();
    if counts.is_empty() {
        return String::new();
    }
    // Most common first; ties keep role order (rifleman, rusher, breacher, marksman).
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    let parts: Vec<String> = counts
        .iter()
        .map(|(r, n)| {
            let name = r.label();
            // Rifleman -> Riflemen, Marksman -> Marksmen.
            let plural = if *n == 1 { name.to_string() } else if let Some(s) = name.strip_suffix("man") { format!("{s}men") } else { format!("{name}s") };
            format!("{n} {plural}")
        })
        .collect();
    format!("NEXT SQUAD: {}", parts.join("  \u{b7}  ").to_uppercase())
}


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

/// Seconds the first "Retire this squad" click stays armed.
const RETIRE_CONFIRM_S: f32 = 4.0;
const RETIRE_LABEL: &str = "Retire this squad";

/// Retiring the squad wipes the evolved brain, so it asks twice: the first click arms it (the
/// button reads "Click again to retire") for [`RETIRE_CONFIRM_S`]; a second click in that window
/// retires it. The button then reads "Retiring…" for a couple of frames before the blocking
/// retrain, so the freeze isn't silent.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
enum RetireStep {
    #[default]
    Idle,
    /// Seconds left to confirm.
    Armed(f32),
    /// Frames left before the reset runs.
    Running(u8),
}

impl RetireStep {
    /// After a click on the button.
    fn click(self) -> Self {
        match self {
            RetireStep::Idle => RetireStep::Armed(RETIRE_CONFIRM_S),
            RetireStep::Armed(_) => RetireStep::Running(2),
            r => r,
        }
    }
    /// After `dt` seconds (one frame). `true` when the reset should run now.
    fn tick(self, dt: f32) -> (Self, bool) {
        match self {
            RetireStep::Armed(t) if t - dt <= 0.0 => (RetireStep::Idle, false),
            RetireStep::Armed(t) => (RetireStep::Armed(t - dt), false),
            RetireStep::Running(0) => (RetireStep::Idle, true),
            RetireStep::Running(n) => (RetireStep::Running(n - 1), false),
            RetireStep::Idle => (RetireStep::Idle, false),
        }
    }
    fn label(self) -> String {
        match self {
            RetireStep::Idle => engine::ui::tl(RETIRE_LABEL).to_uppercase(),
            RetireStep::Armed(_) => engine::ui::tl("Click again to retire").to_uppercase(),
            RetireStep::Running(_) => engine::ui::tl("Retiring…").to_uppercase(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn input(
    mut commands: Commands,
    time: Res<Time>,
    headless: Res<Headless>,
    mut auto: Option<ResMut<HeadlessAdvance>>,
    mut retire: ResMut<RetireStep>,
    brain: Res<Brain>,
    buttons: Query<(&engine::ui::widgets::ActionButton, &Children)>,
    mut texts: Query<&mut Text>,
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
    // Enter / clicks go through the buttons (engine UI kit); Esc is a shortcut. Retiring the
    // squad has no key: R is reload, too easy to hit by habit.
    let before = *retire;
    let (step, reset) = retire.tick(time.delta_secs());
    *retire = step;
    let busy = matches!(*retire, RetireStep::Running(_));
    let to_menu = backs.read().count() > 0 && !busy;
    for MenuEvent(id) in events.read() {
        if busy {
            continue;
        }
        match *id {
            "next_match" => next.set(GameState::Playing),
            "reset_ai" => *retire = retire.click(),
            _ => {}
        }
    }
    if std::mem::discriminant(&before) != std::mem::discriminant(&*retire) {
        let label = retire.label();
        for (b, children) in &buttons {
            if b.0 != UiAction::Menu(MenuAction::Custom("reset_ai")) {
                continue;
            }
            for c in children.iter() {
                if let Ok(mut t) = texts.get_mut(c) {
                    t.0 = label.clone();
                }
            }
        }
    }
    if reset {
        let generation = brain.state.generation;
        commands.queue(crate::save::reset_brain);
        commands.write_message(Toast::new("Squad retired").body(format!("Generation {generation} backed up. A fresh squad is ready.")));
        app_state.set(AppState::MainMenu);
    } else if to_menu {
        flow.abandon();
        app_state.set(AppState::MainMenu);
    }
}

#[cfg(test)]
mod tests {
    use brain::weapons::WeaponKind;

    #[test]
    fn retire_needs_a_second_click_in_time() {
        use super::{RETIRE_CONFIRM_S, RetireStep};
        // Arrange / Act: one click, then the window runs out.
        let armed = RetireStep::Idle.click();
        let (expired, ran) = armed.tick(RETIRE_CONFIRM_S + 0.1);
        // Assert
        assert!(matches!(armed, RetireStep::Armed(_)), "the first click only arms it");
        assert_eq!((expired, ran), (RetireStep::Idle, false), "no second click: nothing happens");
        // Act: two clicks, then frames pass.
        let mut step = RetireStep::Idle.click().click();
        let mut frames = 0;
        let ran = loop {
            let (s, ran) = step.tick(0.016);
            step = s;
            frames += 1;
            if ran || frames > 10 {
                break ran;
            }
        };
        // Assert: runs only after the "Retiring…" label had a frame to show, and ends idle.
        assert!(ran && frames > 1);
        assert_eq!(step, RetireStep::Idle);
    }

    #[test]
    fn loadout_summary_counts_roles() {
        let s = super::loadout_summary(&[WeaponKind::Rifle, WeaponKind::Marksman, WeaponKind::Rifle, WeaponKind::Shotgun]);
        assert_eq!(s, "NEXT SQUAD: 2 RIFLEMEN  \u{b7}  1 BREACHER  \u{b7}  1 MARKSMAN");
        assert!(super::loadout_summary(&[]).is_empty());
    }

    #[test]
    fn generation_line_is_plain_words() {
        let s = super::generation_line(5, 1240, Some(0.6), Some(0.55));
        assert_eq!(s, "Generation 5 is ready. They trained 1,240 rounds against your ghost (a copy of how you play). Against this squad you won 60% of rounds; your ghost won 55%.");
        assert!(!super::generation_line(2, 7, None, None).contains("ghost won"));
        assert_eq!(super::thousands(1_234_567), "1,234,567");
        assert_eq!(super::thousands(999), "999");
    }
}
