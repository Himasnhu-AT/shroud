//! Career on the HUD: the objectives tracker (top left, under the squad generation line) and the
//! "OBJECTIVE COMPLETE" toast.
//!
//! The tracker lists this match's objectives with live progress ("3/5", a green check when
//! done) and the rank with a thin XP bar. At the start of a match (round 1, freeze time and the
//! first seconds of live play) it is expanded: a title, bigger rows and each objective's XP.
//! It redraws only when the match log changes or it expands / collapses.

use bevy::prelude::*;
use brain::career::Career;

use super::hud::{HideWhenPaused, RoundClock, banner_root};
use super::theme::*;
use crate::career::{CareerSave, MatchCareer, ObjectiveDone};
use crate::economy::RoundPhase;
use crate::match_flow::MatchFlow;
use crate::state::{GameState, has_visuals};

/// Seconds of live play the tracker stays expanded in round 1.
const EXPANDED_LIVE_S: f32 = 6.0;
const TOAST_S: f32 = 3.4;
const W_COMPACT: f32 = 360.0;
const W_EXPANDED: f32 = 470.0;

pub struct CareerHudPlugin;

impl Plugin for CareerHudPlugin {
    fn build(&self, app: &mut App) {
        // Practice has no objectives.
        app.add_systems(OnEnter(GameState::Playing), spawn_tracker.run_if(has_visuals.and_then(crate::practice::in_match)).after(crate::career::start_round))
            .add_systems(Update, (redraw_tracker, toast).run_if(has_visuals.and_then(in_state(GameState::Playing)).and_then(crate::practice::in_match)));
    }
}

#[derive(Component)]
struct Tracker {
    /// Expanded when last drawn (None = never drawn).
    drawn: Option<bool>,
}

#[derive(Component)]
struct Toast;

fn spawn_tracker(mut commands: Commands) {
    commands.spawn((
        Name::new("CareerTracker"),
        Tracker { drawn: None },
        DespawnOnExit(GameState::Playing),
        HideWhenPaused,
        Node {
            position_type: PositionType::Absolute,
            left: px(40),
            // Under the money (`ui::buy_menu`, top 66).
            top: px(124),
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            padding: UiRect::new(px(14), px(14), px(10), px(12)),
            border: UiRect::left(px(3)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.78)),
        BorderColor::all(ACCENT),
        Pickable::IGNORE,
    ));
}

/// Expanded: round 1, during freeze time or the first seconds of play.
fn expanded(flow: &MatchFlow, phase: &RoundPhase, clock: &RoundClock) -> bool {
    flow.round <= 1 && flow.round_over.is_none() && (phase.frozen() || clock.0 < EXPANDED_LIVE_S)
}

fn redraw_tracker(
    mut commands: Commands,
    mc: Res<MatchCareer>,
    save: Res<CareerSave>,
    flow: Res<MatchFlow>,
    phase: Res<RoundPhase>,
    clock: Res<RoundClock>,
    mut q: Query<(Entity, &mut Tracker, &mut Node)>,
) {
    let big = expanded(&flow, &phase, &clock);
    for (e, mut tr, mut node) in &mut q {
        if tr.drawn == Some(big) && !mc.is_changed() && !save.is_changed() {
            continue;
        }
        tr.drawn = Some(big);
        node.width = px(if big { W_EXPANDED } else { W_COMPACT });
        commands.entity(e).despawn_related::<Children>().with_children(|p| draw(p, &mc, &save.career, big));
    }
}

fn draw(p: &mut ChildSpawnerCommands, mc: &MatchCareer, career: &Career, big: bool) {
    let w = if big { W_EXPANDED } else { W_COMPACT } - 28.0;
    let rank = career.rank();
    // Header: title + rank.
    p.spawn((Node { width: px(w), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, ..default() }, Pickable::IGNORE))
        .with_children(|r| {
            r.spawn(text(if big { "MATCH OBJECTIVES" } else { "OBJECTIVES" }, label(if big { SIZE_LABEL } else { SIZE_TINY }), ACCENT));
            r.spawn(text(rank.name().to_uppercase(), label(SIZE_TINY), TEXT));
        });
    let (row_font, check) = if big { (SIZE_BODY - 2.0, 16.0) } else { (SIZE_SMALL - 1.0, 13.0) };
    let right_w = if big { 78.0 } else { 44.0 };
    let text_w = w - check - right_w - 2.0 * GAP_S;
    for (o, (done, prog, target)) in mc.objectives.iter().zip(mc.status()) {
        p.spawn((Node { width: px(w), align_items: AlignItems::Center, column_gap: px(GAP_S), ..default() }, Pickable::IGNORE)).with_children(|r| {
            check_box(r, check, done);
            r.spawn((Node { width: px(text_w), flex_shrink: 0.0, ..default() }, Pickable::IGNORE))
                .with_child(text(o.goal.describe(), body(row_font), if done { TEXT_MUTED } else { TEXT }));
            let (right, color) = if big {
                (format!("+{} XP", o.xp), if done { FRIENDLY } else { ACCENT })
            } else if done {
                (format!("+{}", o.xp), FRIENDLY)
            } else {
                (format!("{prog}/{target}"), TEXT)
            };
            r.spawn((Node { width: px(right_w), flex_shrink: 0.0, justify_content: JustifyContent::FlexEnd, ..default() }, Pickable::IGNORE))
                .with_child(text(right, label(if big { SIZE_SMALL } else { SIZE_TINY }), color));
        });
    }
    // Rank progress: a thin bar and the numbers.
    let (frac, line) = rank_line(career);
    p.spawn((Node { width: px(w), height: px(4), margin: UiRect::top(px(4)), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.14)), Pickable::IGNORE))
        .with_child((Node { width: percent(frac * 100.0), height: percent(100), ..default() }, BackgroundColor(ACCENT), Pickable::IGNORE));
    p.spawn(text(line, label(SIZE_TINY), TEXT.with_alpha(0.8)));
}

/// (fraction of this rank done, "240 / 1,000 XP TO PRIVATE").
pub fn rank_line(c: &Career) -> (f32, String) {
    match (c.progress(), c.rank().next()) {
        (Some((into, span)), Some(next)) => (into as f32 / span.max(1) as f32, format!("{} XP \u{b7} {} TO {}", c.xp, span - into, next.name().to_uppercase())),
        _ => (1.0, format!("{} XP \u{b7} TOP RANK", c.xp)),
    }
}

/// A square box; done = filled green with a dark check drawn from two bars.
pub fn check_box(p: &mut ChildSpawnerCommands, size: f32, done: bool) {
    let (bg, border) = if done { (FRIENDLY, FRIENDLY) } else { (Color::NONE, TEXT_MUTED) };
    p.spawn((
        Node { width: px(size), height: px(size), flex_shrink: 0.0, border: UiRect::all(px(2)), border_radius: BorderRadius::all(px(2)), ..default() },
        BackgroundColor(bg),
        BorderColor::all(border),
        Pickable::IGNORE,
    ))
    .with_children(|b| {
        if !done {
            return;
        }
        let dark = Color::srgb(0.03, 0.05, 0.06);
        let s = size / 16.0;
        // Short stroke (down-right) then the long one (up-right).
        b.spawn((
            Node { position_type: PositionType::Absolute, left: px(1.5 * s), top: px(6.0 * s), width: px(5.0 * s), height: px(2.2 * s), ..default() },
            UiTransform::from_rotation(Rot2::degrees(45.0)),
            BackgroundColor(dark),
            Pickable::IGNORE,
        ));
        b.spawn((
            Node { position_type: PositionType::Absolute, left: px(3.6 * s), top: px(4.6 * s), width: px(8.5 * s), height: px(2.2 * s), ..default() },
            UiTransform::from_rotation(Rot2::degrees(-50.0)),
            BackgroundColor(dark),
            Pickable::IGNORE,
        ));
    });
}

/// "OBJECTIVE COMPLETE  +200 XP" with the objective under it; several at once share one toast.
fn toast(mut commands: Commands, mut done: MessageReader<ObjectiveDone>, old: Query<Entity, With<Toast>>) {
    let list: Vec<ObjectiveDone> = done.read().cloned().collect();
    if list.is_empty() {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    let root = banner_root(&mut commands, Some(TOAST_S), 70.0);
    commands.entity(root).insert(Toast).with_children(|b| {
        let xp: u32 = list.iter().map(|d| d.xp).sum();
        let title = if list.len() > 1 { format!("{} OBJECTIVES COMPLETE", list.len()) } else { "OBJECTIVE COMPLETE".into() };
        b.spawn((Node { align_items: AlignItems::Center, column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
            r.spawn((text(title, display(40.0), TEXT), TextShadow::default()));
            r.spawn((text(format!("+{xp} XP"), display(40.0), ACCENT), TextShadow::default()));
        });
        for d in &list {
            b.spawn(text(d.text.clone(), body(SIZE_BODY), TEXT));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_line_counts_xp_to_the_next_rank() {
        let (f, s) = rank_line(&Career { xp: 1_500, ..Default::default() });
        assert!((f - 1.0 / 3.0).abs() < 1e-4, "{f}");
        assert_eq!(s, "1500 XP \u{b7} 1000 TO CORPORAL");
        let (f, s) = rank_line(&Career { xp: 50_000, ..Default::default() });
        assert_eq!(f, 1.0);
        assert!(s.ends_with("TOP RANK"), "{s}");
    }
}
