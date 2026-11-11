//! Match = first to 3 rounds (at most 5) vs the evolved squad. Rounds restart by re-entering
//! `Playing` (freeze time, money and the buy menu are `economy`); after the match the report
//! goes to `Evolving`.

use bevy::prelude::*;
use brain::evolve::{EnemyReport, MatchReport};

use crate::enemy::{self, Enemy, EnemyCombat, Squad};
use crate::dev::{Dev, Prop};
use crate::map::to_ground;
use crate::player::Player;
use crate::save::Brain;
use crate::state::GameState;
use crate::telemetry::Telemetry;
use crate::ui::hud::{HudInfo, RoundClock};
use crate::weapon::{Health, WeaponSet};

pub use brain::economy::{MAX_ROUNDS, ROUNDS_TO_WIN};
/// Round time limit (s). Time up: the side with more of its health left wins — the player's
/// HP share against the squad's total remaining HP share; a tie goes to the squad (they held).
pub const ROUND_TIME_S: f32 = 150.0;

/// The round time limit in force (s); `None` = no limit. Headless runs (tests, tools) have none,
/// so long scripted rounds aren't cut short; `match.time_limit` sets it over the control port.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct RoundLimit(pub Option<f32>);

impl Default for RoundLimit {
    fn default() -> Self {
        RoundLimit(Some(ROUND_TIME_S))
    }
}
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
    /// The last finished round of this match: Some(player won). None before the first.
    pub last_round: Option<bool>,
    /// The current (or just finished) round was decided by the clock.
    pub timed_out: bool,
    started: bool,
}

impl MatchFlow {
    pub fn seed(&self) -> u64 {
        (self.match_no as u64) << 8 | self.round as u64
    }
    /// Drop the current match (pause menu restart / quit): the next round starts a new match.
    pub fn abandon(&mut self) {
        self.started = false;
        self.round_over = None;
    }
    /// The round in play ended with a result (the round-over banner). False when it is left
    /// without one (`abandon`): nothing from it may train the squad.
    pub fn round_decided(&self) -> bool {
        self.round_over.is_some()
    }
    fn needs_new_match(&self) -> bool {
        !self.started || self.player_rounds >= ROUNDS_TO_WIN || self.enemy_rounds >= ROUNDS_TO_WIN
    }
}

/// The finished match, waiting for `Evolving`.
#[derive(Resource, Default)]
pub struct PendingReport(pub Option<MatchReport>);

pub struct MatchFlowPlugin;

impl Plugin for MatchFlowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchFlow>()
            .init_resource::<RoundLimit>()
            .init_resource::<PendingReport>()
            .add_plugins((crate::armor::ArmorPlugin, crate::economy::EconomyPlugin))

            .add_systems(OnEnter(GameState::Playing), (begin_round, enemy::spawn_squad).chain())
            .add_systems(
                Update,
                (check_round_end.after(WeaponSet), tick_round_over, update_hud).chain().run_if(in_state(GameState::Playing)),
            );
    }
}

pub(crate) fn begin_round(mut flow: ResMut<MatchFlow>, mut telemetry: ResMut<Telemetry>, brain: Res<Brain>, dev: Dev) {
    if flow.needs_new_match() {
        flow.match_no += 1;
        flow.round = 0;
        flow.player_rounds = 0;
        flow.enemy_rounds = 0;
        flow.enemies.clear();
        flow.last_round = None;
        flow.started = true;
        telemetry.0 = Default::default();
        dev.trace("match.start", &[("generation", Prop::Int(brain.state.generation as i64 + 1))]);
    }
    flow.round += 1;
    flow.round_over = None;
    flow.timed_out = false;
    info!("match {} round {}", flow.match_no, flow.round);
}

fn merge(into: &mut Vec<EnemyReport>, r: &EnemyReport) {
    match into.iter_mut().find(|e| e.trait_id == r.trait_id && e.neat_id == r.neat_id) {
        Some(e) => e.absorb(r),
        None => into.push(r.clone()),
    }
}

pub(crate) fn check_round_end(
    mut flow: ResMut<MatchFlow>,
    squad: Res<Squad>,
    clock: Res<RoundClock>,
    limit: Res<RoundLimit>,
    headless: Res<crate::state::Headless>,
    players: Query<(&Health, &Transform), With<Player>>,
    enemies: Query<(&EnemyCombat, &Health), With<Enemy>>,
    mut telemetry: ResMut<Telemetry>,
    dev: Dev,
) {
    if flow.round_over.is_some() {
        return;
    }
    let Ok((php, ptf)) = players.single() else { return };
    let alive = enemies.iter().filter(|(c, _)| !c.dead).count();
    let player_dead = php.is_dead();
    let time_up = !headless.0 && limit.0.is_some_and(|l| clock.0 >= l);
    if !player_dead && alive > 0 && !time_up {
        return;
    }
    let player_won = if !player_dead && alive > 0 {
        // Time up: compare what each side has left.
        let squad_hp: f32 = enemies.iter().filter(|(c, _)| !c.dead).map(|(_, h)| h.hp.max(0.0)).sum();
        let squad_max = (squad.size.max(alive) as f32 * crate::enemy::ENEMY_HP).max(1.0);
        flow.timed_out = true;
        php.hp / php.max.max(1.0) > squad_hp / squad_max
    } else {
        !player_dead
    };
    flow.last_round = Some(player_won);
    if player_won {
        flow.player_rounds += 1;
    } else {
        flow.enemy_rounds += 1;
        if player_dead {
            telemetry.0.death(to_ground(ptf.translation));
        }
    }
    for r in squad.dead_reports.iter().chain(enemies.iter().filter(|(c, _)| !c.dead).map(|(c, _)| &c.report)) {
        merge(&mut flow.enemies, r);
    }
    flow.round_over = Some((ROUND_END_PAUSE, player_won));
    if !player_won {
        dev.trace("player.died", &[("round", Prop::Int(flow.round as i64))]);
    }
    dev.trace(
        "round.end",
        &[("round", Prop::Int(flow.round as i64)), ("player_won", Prop::Bool(player_won)), ("timed_out", Prop::Bool(flow.timed_out))],
    );
    // The round-over banner is drawn by `ui::hud`.
}

fn tick_round_over(
    time: Res<Time>,
    mut flow: ResMut<MatchFlow>,
    telemetry: Res<Telemetry>,
    mut pending: ResMut<PendingReport>,
    mut next: ResMut<NextState<GameState>>,
    dev: Dev,
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
        dev.trace(
            "match.end",
            &[("player_won", Prop::Bool(player_won)), ("player_rounds", Prop::Int(flow.player_rounds as i64)), ("enemy_rounds", Prop::Int(flow.enemy_rounds as i64))],
        );
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

