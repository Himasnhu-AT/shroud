//! Match = best of 3 rounds vs the evolved squad. Rounds restart by re-entering
//! `Playing`; after the match the report goes to `Evolving`.

use bevy::prelude::*;
use brain::evolve::{EnemyReport, MatchReport};

use ashlar_trace::Prop;

use crate::enemy::{self, Enemy, EnemyCombat, Squad};
use crate::ops::trace::{GameEvent, GameTracer, trace_event};
use crate::map::to_ground;
use crate::player::Player;
use crate::save::Brain;
use crate::state::GameState;
use crate::telemetry::Telemetry;
use crate::ui::hud::HudInfo;
use crate::weapon::{Health, WeaponSet};

pub const ROUNDS_TO_WIN: u32 = 2;
const ROUND_END_PAUSE: f32 = 2.5;

#[derive(Resource, Debug, Default)]
pub struct MatchFlow {
    pub match_no: u32,
    pub round: u32,
    pub player_rounds: u32,
    pub enemy_rounds: u32,
    pub enemies: Vec<EnemyReport>,
    /// Some((seconds left, player won)) while the round-over banner shows.
    pub round_over: Option<(f32, bool)>,
    pub last_result: Option<(bool, u32, u32)>,
    started: bool,
}

impl MatchFlow {
    pub fn seed(&self) -> u64 {
        (self.match_no as u64) << 8 | self.round as u64
    }
    fn needs_new_match(&self) -> bool {
        !self.started || self.player_rounds >= ROUNDS_TO_WIN || self.enemy_rounds >= ROUNDS_TO_WIN
    }
}

/// The finished match, waiting for `Evolving`.
#[derive(Resource, Default)]
pub struct PendingReport(pub Option<MatchReport>);

#[derive(Component)]
struct RoundBanner;

pub struct MatchFlowPlugin;

impl Plugin for MatchFlowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchFlow>()
            .init_resource::<PendingReport>()
            .add_systems(OnEnter(GameState::Playing), (begin_round, enemy::spawn_squad).chain())
            .add_systems(
                Update,
                (check_round_end.after(WeaponSet), tick_round_over, update_hud).chain().run_if(in_state(GameState::Playing)),
            );
    }
}

fn begin_round(mut flow: ResMut<MatchFlow>, mut telemetry: ResMut<Telemetry>, brain: Res<Brain>, tracer: Option<Res<GameTracer>>) {
    if flow.needs_new_match() {
        flow.match_no += 1;
        flow.round = 0;
        flow.player_rounds = 0;
        flow.enemy_rounds = 0;
        flow.enemies.clear();
        flow.started = true;
        telemetry.0 = Default::default();
        if let Some(t) = &tracer {
            trace_event(t, GameEvent::MatchStart, &[("generation", Prop::Int(brain.state.generation as i64 + 1))]);
        }
    }
    flow.round += 1;
    flow.round_over = None;
    info!("match {} round {}", flow.match_no, flow.round);
}

fn merge(into: &mut Vec<EnemyReport>, r: &EnemyReport) {
    match into.iter_mut().find(|e| e.trait_id == r.trait_id && e.neat_id == r.neat_id) {
        Some(e) => {
            e.damage_dealt += r.damage_dealt;
            e.kills += r.kills;
            e.time_alive += r.time_alive;
            e.time_engaged += r.time_engaged;
        }
        None => into.push(r.clone()),
    }
}

fn check_round_end(
    mut commands: Commands,
    mut flow: ResMut<MatchFlow>,
    squad: Res<Squad>,
    players: Query<(&Health, &Transform), With<Player>>,
    enemies: Query<&EnemyCombat, With<Enemy>>,
    mut telemetry: ResMut<Telemetry>,
    visuals: Option<Res<crate::state::Headless>>,
    tracer: Option<Res<GameTracer>>,
) {
    if flow.round_over.is_some() {
        return;
    }
    let Ok((php, ptf)) = players.single() else { return };
    let alive = enemies.iter().filter(|c| !c.dead).count();
    let player_dead = php.is_dead();
    if !player_dead && alive > 0 {
        return;
    }
    let player_won = !player_dead;
    if player_won {
        flow.player_rounds += 1;
    } else {
        flow.enemy_rounds += 1;
        telemetry.0.death(to_ground(ptf.translation));
    }
    for r in squad.dead_reports.iter().chain(enemies.iter().filter(|c| !c.dead).map(|c| &c.report)) {
        merge(&mut flow.enemies, r);
    }
    flow.round_over = Some((ROUND_END_PAUSE, player_won));
    if let Some(t) = &tracer {
        if !player_won {
            trace_event(t, GameEvent::PlayerDied, &[("round", Prop::Int(flow.round as i64))]);
        }
        trace_event(t, GameEvent::RoundEnd, &[("round", Prop::Int(flow.round as i64)), ("player_won", Prop::Bool(player_won))]);
    }
    if visuals.is_some_and(|h| !h.0) {
        let text = format!(
            "{}\n{} : {}",
            if player_won { "ROUND WON" } else { "ROUND LOST" },
            flow.player_rounds,
            flow.enemy_rounds
        );
        commands.spawn((
            RoundBanner,
            DespawnOnExit(GameState::Playing),
            Node { width: percent(100), height: percent(100), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
            children![(
                Text::new(text),
                TextFont::from_font_size(52.0),
                TextLayout::justify(Justify::Center),
                TextColor(if player_won { Color::srgb(0.5, 1.0, 0.6) } else { Color::srgb(1.0, 0.4, 0.35) }),
            )],
        ));
    }
}

fn tick_round_over(
    time: Res<Time>,
    mut flow: ResMut<MatchFlow>,
    telemetry: Res<Telemetry>,
    mut pending: ResMut<PendingReport>,
    mut next: ResMut<NextState<GameState>>,
    tracer: Option<Res<GameTracer>>,
) {
    let Some((left, _)) = flow.round_over.as_mut() else { return };
    *left -= time.delta_secs();
    if *left > 0.0 {
        return;
    }
    if flow.player_rounds >= ROUNDS_TO_WIN || flow.enemy_rounds >= ROUNDS_TO_WIN {
        let player_won = flow.player_rounds >= ROUNDS_TO_WIN;
        flow.last_result = Some((player_won, flow.player_rounds, flow.enemy_rounds));
        pending.0 = Some(MatchReport {
            telemetry: telemetry.0.clone(),
            enemies: flow.enemies.clone(),
            player_won,
            player_rounds: flow.player_rounds,
            enemy_rounds: flow.enemy_rounds,
        });
        if let Some(t) = &tracer {
            trace_event(t, GameEvent::MatchEnd, &[("player_won", Prop::Bool(player_won)), ("player_rounds", Prop::Int(flow.player_rounds as i64)), ("enemy_rounds", Prop::Int(flow.enemy_rounds as i64))]);
        }
        next.set(GameState::Evolving);
    } else {
        // Re-entering Playing resets player, rifle and HUD, and respawns the squad.
        next.set(GameState::Playing);
    }
}

fn update_hud(
    flow: Res<MatchFlow>,
    brain: Res<Brain>,
    enemies: Query<&EnemyCombat, With<Enemy>>,
    hud: Option<ResMut<HudInfo>>,
) {
    let Some(mut hud) = hud else { return };
    let info = HudInfo {
        generation: brain.state.generation + 1,
        enemies_left: enemies.iter().filter(|c| !c.dead).count() as u32,
        round_score: (flow.player_rounds, flow.enemy_rounds),
    };
    if *hud != info {
        *hud = info;
    }
}

