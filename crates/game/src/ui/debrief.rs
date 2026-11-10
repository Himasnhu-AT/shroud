//! `GameState::Debrief`: match result, what the squad learned about you, how it changed.
//! Buttons: next match (Enter) | main menu (Esc) | reset the AI (R).

use bevy::prelude::*;

use crate::career::{CareerSave, MatchCareer};
use crate::match_flow::MatchFlow;
use brain::career::{Award, Rank};
use crate::save::{Brain, LastEvolve};
use super::theme::*;
use crate::state::{AppState, GameState, Headless};
use engine::ui::prelude::{MenuAction, MenuEvent, NavBack, action_button};

pub struct DebriefPlugin;

impl Plugin for DebriefPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Debrief), spawn_debrief)
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
    // What the squad will carry next match, and why (the learned loadout).
    let loadout_note = last.0.as_ref().and_then(|s| s.loadout_note.clone());
    let loadout_line = last.0.as_ref().map(|s| loadout_summary(&s.next_loadout)).filter(|l| !l.is_empty());
    let show_loadout = loadout_note.is_some() || loadout_line.is_some();
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
                    // The career panel takes the place of "How they changed / History" (the screen
                    // has no room for both; the main menu keeps the record).
                    let award = mc.award.as_ref().filter(|_| mc.match_no == flow.match_no);
                    if let Some(award) = award {
                        career_panel(root, &mc, award, mc.xp_before, save.career.xp);
                    }
                    if show_loadout {
                        root.spawn(card_panel()).with_children(|p| {
                            p.spawn(eyebrow("Their loadout", ENEMY));
                            if let Some(note) = loadout_note {
                                p.spawn(text(note, body_bold(SIZE_BODY + 2.0), TEXT));
                            }
                            if let Some(line) = loadout_line {
                                p.spawn(text(line, label(SIZE_LABEL), TEXT_MUTED));
                            }
                        });
                    }
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
                        // (One line fewer when the loadout panel takes its place on screen.)
                        // (One fewer per career objective past three: the career panel grows with them.)
                        let extra = if award.is_some() { mc.objectives.len().saturating_sub(3) } else { 0 };
                        let n = (if show_loadout { 3usize } else { 4 }).saturating_sub(extra).max(1);
                        for (i, line) in insights.iter().take(n).enumerate() {
                            p.spawn((Node { column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
                                r.spawn((Node { width: px(24), flex_shrink: 0.0, ..default() }, Pickable::IGNORE))
                                    .with_child(text(format!("0{}", i + 1), label(SIZE_LABEL), INTEL));
                                r.spawn((Node { width: px(PANEL_W - 40.0 - 24.0 - GAP_M), ..default() }, Pickable::IGNORE))
                                    .with_child(text(line.clone(), body(SIZE_BODY - 2.0), TEXT));
                            });
                        }
                    });
                    if award.is_none() {
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
                    }
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

/// "Career": the XP lines, total and an XP bar that fills from the old XP to the new (wrapping
/// at each rank), with a big PROMOTED line when it crosses one; and the objectives, done or not.
fn career_panel(root: &mut ChildSpawnerCommands, mc: &MatchCareer, award: &Award, from: u32, to: u32) {
    const LEFT_W: f32 = 520.0;
    let right_w = PANEL_W - 40.0 - LEFT_W - GAP_L;
    root.spawn((card_panel(), XpAnim { from, to, rank_before: award.rank_before, t: -XP_ANIM_DELAY, promoted_at: None })).with_children(|p| {
        p.spawn((Node { column_gap: px(GAP_L), ..default() }, Pickable::IGNORE)).with_children(|row| {
            // Left: XP breakdown, total, rank bar.
            row.spawn((Node { width: px(LEFT_W), flex_direction: FlexDirection::Column, row_gap: px(5), ..default() }, Pickable::IGNORE)).with_children(|l| {
                l.spawn(eyebrow("Career", ACCENT));
                // Match and rounds lines, then the objectives as one line (the right column lists them).
                let done = mc.status().iter().filter(|s| s.0).count();
                let fixed = award.lines.len().saturating_sub(done);
                let obj_xp: u32 = award.lines[fixed..].iter().map(|l| l.xp).sum();
                let mut lines: Vec<(String, u32)> = award.lines[..fixed].iter().map(|l| (l.label.clone(), l.xp)).collect();
                if done > 0 {
                    lines.push((format!("Objectives \u{d7}{done}"), obj_xp));
                }
                for (label_text, xp) in lines {
                    l.spawn((Node { width: px(LEFT_W), justify_content: JustifyContent::SpaceBetween, ..default() }, Pickable::IGNORE)).with_children(|r| {
                        r.spawn((Node { width: px(LEFT_W - 110.0), ..default() }, Pickable::IGNORE)).with_child(text(label_text, body(SIZE_SMALL), TEXT));
                        r.spawn(text(format!("+{xp}"), label(SIZE_SMALL), FRIENDLY));
                    });
                }
                l.spawn((Node { width: px(LEFT_W), height: px(1), ..default() }, BackgroundColor(LINE), Pickable::IGNORE));
                l.spawn((Node { width: px(LEFT_W), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Baseline, ..default() }, Pickable::IGNORE)).with_children(|r| {
                    r.spawn(text("TOTAL", label(SIZE_LABEL), TEXT_MUTED));
                    r.spawn(text(format!("+{} XP", award.total), display(34.0), ACCENT));
                });
                l.spawn((Node { width: px(LEFT_W), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Baseline, margin: UiRect::top(px(4)), ..default() }, Pickable::IGNORE)).with_children(|r| {
                    r.spawn((XpRankText, text(award.rank_before.name().to_uppercase(), display(30.0), TEXT)));
                    r.spawn((XpNumbers, text("", label(SIZE_TINY), TEXT_MUTED)));
                });
                l.spawn((Node { width: px(LEFT_W), height: px(10), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.12)), Pickable::IGNORE))
                    .with_child((XpFill, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(ACCENT), Pickable::IGNORE));
            });
            // Right: the objectives.
            row.spawn((Node { width: px(right_w), flex_direction: FlexDirection::Column, row_gap: px(7), ..default() }, Pickable::IGNORE)).with_children(|r| {
                let done = mc.status().iter().filter(|s| s.0).count();
                r.spawn(eyebrow(&format!("Objectives {done}/{}", mc.objectives.len()), TEXT_MUTED));
                for (o, (ok, prog, target)) in mc.objectives.iter().zip(mc.status()) {
                    r.spawn((Node { column_gap: px(GAP_S), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE)).with_children(|line| {
                        super::career_hud::check_box(line, 16.0, ok);
                        line.spawn((Node { width: px(right_w - 16.0 - 90.0 - 2.0 * GAP_S), ..default() }, Pickable::IGNORE))
                            .with_child(text(o.goal.describe(), body(SIZE_SMALL), if ok { TEXT } else { TEXT_MUTED }));
                        let (s, c) = if ok { (format!("+{} XP", o.xp), FRIENDLY) } else { (format!("{prog}/{target}"), TEXT_DIM) };
                        line.spawn((Node { width: px(90), justify_content: JustifyContent::FlexEnd, ..default() }, Pickable::IGNORE)).with_child(text(s, label(SIZE_TINY), c));
                    });
                }
                if award.promoted() {
                    // Shown (and popped in) when the XP bar crosses into the new rank.
                    r.spawn((
                        Promoted,
                        text(format!("PROMOTED TO {}", award.rank_after.name().to_uppercase()), display(48.0), ACCENT),
                        TextShadow::default(),
                        Node { margin: UiRect::top(px(GAP_S)), ..default() },
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

#[cfg(test)]
mod tests {
    use brain::weapons::WeaponKind;

    #[test]
    fn loadout_summary_counts_roles() {
        let s = super::loadout_summary(&[WeaponKind::Rifle, WeaponKind::Marksman, WeaponKind::Rifle, WeaponKind::Shotgun]);
        assert_eq!(s, "NEXT SQUAD: 2 RIFLEMEN  \u{b7}  1 BREACHER  \u{b7}  1 MARKSMAN");
        assert!(super::loadout_summary(&[]).is_empty());
    }
}
