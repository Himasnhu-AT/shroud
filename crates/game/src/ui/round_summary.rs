//! Round-end summary under the round-over banner: the round's MVP (you if you won the round,
//! else the squad member who did the most damage), your round line, and "time up" when the
//! clock decided it. Spawned once per round end; gone with the round.

use bevy::prelude::*;

use super::hud::HideWhenPaused;
use super::scoreboard::{Scoreboard, track};
use super::theme::*;
use crate::match_flow::MatchFlow;
use crate::state::{GameState, has_visuals};

pub struct RoundSummaryPlugin;

impl Plugin for RoundSummaryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            show_summary
                .after(track)
                .after(crate::match_flow::check_round_end)
                .run_if(has_visuals)
                .run_if(in_state(GameState::Playing)),
        );
    }
}

/// The round's MVP line: (name, detail, is the player).
pub fn mvp(sb: &Scoreboard, player_won: bool) -> (String, String, bool) {
    let p = sb.player_round;
    let plural = |n: u32| if n == 1 { "" } else { "s" };
    if player_won {
        return ("YOU".into(), format!("{} kill{} \u{b7} {:.0} damage", p.kills, plural(p.kills), p.damage), true);
    }
    let best = sb
        .enemies
        .iter()
        .max_by(|a, b| a.round.damage.total_cmp(&b.round.damage).then(a.round.kills.cmp(&b.round.kills)).then(b.slot.cmp(&a.slot)));
    match best {
        Some(r) => (
            r.callsign.to_uppercase(),
            format!(
                "{} \u{b7} {} \u{b7} {:.0} damage \u{b7} {} kill{}",
                r.role.label(),
                r.gun.label(),
                r.round.damage,
                r.round.kills,
                plural(r.round.kills)
            ),
            false,
        ),
        None => ("SQUAD".into(), String::new(), false),
    }
}

fn show_summary(mut commands: Commands, flow: Res<MatchFlow>, sb: Res<Scoreboard>, mut shown: Local<bool>) {
    let Some((_, won)) = flow.round_over else {
        *shown = false;
        return;
    };
    if *shown {
        return;
    }
    *shown = true;
    let (name, detail, is_player) = mvp(&sb, won);
    let p = sb.player_round;
    let yours = format!("Your round: {} kills \u{b7} {:.0} damage", p.kills, p.damage);
    commands
        .spawn((
            Name::new("Round summary"),
            DespawnOnExit(GameState::Playing),
            HideWhenPaused,
            Node {
                position_type: PositionType::Absolute,
                top: percent(56),
                left: px(0),
                right: px(0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            GlobalZIndex(8),
            Pickable::IGNORE,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: px(6),
                    padding: UiRect::axes(px(28), px(14)),
                    border: UiRect::top(px(3)),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(ACCENT),
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                if flow.timed_out {
                    p.spawn(text("TIME UP \u{b7} DECIDED ON HEALTH LEFT", label(SIZE_LABEL), ACCENT));
                }
                p.spawn((Node { align_items: AlignItems::Baseline, column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
                    r.spawn(text("MVP", label(SIZE_LABEL + 2.0), ACCENT));
                    r.spawn(text(name, display(40.0), if is_player { FRIENDLY } else { ENEMY }));
                });
                if !detail.is_empty() {
                    p.spawn(text(detail, body(SIZE_BODY - 2.0), TEXT));
                }
                if !is_player {
                    p.spawn(text(yours, body(SIZE_SMALL), TEXT_MUTED));
                }
            });
        });
}
