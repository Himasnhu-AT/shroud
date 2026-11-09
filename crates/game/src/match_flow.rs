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
/// Round time limit (s, CS's 1:55). Time up before a plant: the squad (defenders) wins. After a
/// plant the clock no longer matters (`round_outcome`).
pub const ROUND_TIME_S: f32 = 115.0;

/// Why a round ended (bomb defusal rules, `brain::objective`; the player attacks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundEnd {
    /// The whole squad is dead.
    SquadWiped,
    /// The player died before planting.
    PlayerKilled,
    /// The clock ran out before a plant.
    TimeUp,
    /// The planted bomb went off.
    Exploded,
    /// The squad defused it.
    Defused,
}

impl RoundEnd {
    pub fn player_won(self) -> bool {
        matches!(self, RoundEnd::SquadWiped | RoundEnd::Exploded)
    }
    /// One line for the round-over banner.
    pub fn reason(self) -> &'static str {
        match self {
            RoundEnd::SquadWiped => "Squad eliminated \u{2014} you win",
            RoundEnd::PlayerKilled => "You were killed \u{2014} defenders win",
            RoundEnd::TimeUp => "Time ran out \u{2014} defenders win",
            RoundEnd::Exploded => "Bomb exploded \u{2014} you win",
            RoundEnd::Defused => "Bomb defused \u{2014} defenders win",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            RoundEnd::SquadWiped => "squad_wiped",
            RoundEnd::PlayerKilled => "player_killed",
            RoundEnd::TimeUp => "time_up",
            RoundEnd::Exploded => "exploded",
            RoundEnd::Defused => "defused",
        }
    }
}

/// What `round_outcome` looks at this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoundFacts {
    pub player_dead: bool,
    pub squad_alive: usize,
    pub time_up: bool,
    pub bomb: crate::bomb::BombPhase,
}

/// The round rules (`brain::objective`): before a plant the squad wiped wins it for the player,
/// his death or the clock for the squad; after a plant only the bomb (or the squad wiped) ends
/// it, whatever the clock and whether he is still alive. `None` = play on.
pub fn round_outcome(f: RoundFacts) -> Option<RoundEnd> {
    use crate::bomb::BombPhase;
    match f.bomb {
        BombPhase::Exploded => Some(RoundEnd::Exploded),
        BombPhase::Defused => Some(RoundEnd::Defused),
        BombPhase::Planted => (f.squad_alive == 0).then_some(RoundEnd::SquadWiped),
        BombPhase::Carried => {
            if f.player_dead {
                Some(RoundEnd::PlayerKilled)
            } else if f.squad_alive == 0 {
                Some(RoundEnd::SquadWiped)
            } else if f.time_up {
                Some(RoundEnd::TimeUp)
            } else {
                None
            }
        }
    }
}

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
    /// Why the current (just finished) round ended.
    pub end: Option<RoundEnd>,
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
    flow.end = None;
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
    bomb: Res<crate::bomb::Bomb>,
    players: Query<(&Health, &Transform), With<Player>>,
    enemies: Query<(&EnemyCombat, &Health), With<Enemy>>,
    mut telemetry: ResMut<Telemetry>,
    dev: Dev,
) {
    if flow.round_over.is_some() {
        return;
    }
    let Ok((php, ptf)) = players.single() else { return };
    let player_dead = php.is_dead();
    let facts = RoundFacts {
        player_dead,
        squad_alive: enemies.iter().filter(|(c, _)| !c.dead).count(),
        time_up: !headless.0 && limit.0.is_some_and(|l| clock.0 >= l),
        bomb: bomb.phase,
    };
    let Some(end) = round_outcome(facts) else { return };
    let player_won = end.player_won();
    flow.end = Some(end);
    flow.timed_out = end == RoundEnd::TimeUp;
    if player_dead {
        telemetry.0.death(to_ground(ptf.translation));
    }
    flow.last_round = Some(player_won);
    if player_won {
        flow.player_rounds += 1;
    } else {
        flow.enemy_rounds += 1;
    }
    for r in squad.dead_reports.iter().chain(enemies.iter().filter(|(c, _)| !c.dead).map(|(c, _)| &c.report)) {
        merge(&mut flow.enemies, r);
    }
    flow.round_over = Some((ROUND_END_PAUSE, player_won));
    if player_dead {
        dev.trace("player.died", &[("round", Prop::Int(flow.round as i64))]);
    }
    dev.trace(
        "round.end",
        &[("round", Prop::Int(flow.round as i64)), ("player_won", Prop::Bool(player_won)), ("timed_out", Prop::Bool(flow.timed_out)), ("bomb", Prop::Bool(bomb.was_planted()))],
    );
    info!("round {} over: {}", flow.round, end.label());
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::bomb::BombPhase;

    fn facts(player_dead: bool, squad_alive: usize, time_up: bool, bomb: BombPhase) -> RoundFacts {
        RoundFacts { player_dead, squad_alive, time_up, bomb }
    }

    #[test]
    fn before_the_plant() {
        assert_eq!(round_outcome(facts(false, 3, false, BombPhase::Carried)), None);
        assert_eq!(round_outcome(facts(false, 0, false, BombPhase::Carried)), Some(RoundEnd::SquadWiped));
        assert_eq!(round_outcome(facts(true, 2, false, BombPhase::Carried)), Some(RoundEnd::PlayerKilled));
        // The clock: defenders win, whatever the health left.
        assert_eq!(round_outcome(facts(false, 2, true, BombPhase::Carried)), Some(RoundEnd::TimeUp));
        // Trading the last enemy for your life is still a loss before the plant.
        assert_eq!(round_outcome(facts(true, 0, false, BombPhase::Carried)), Some(RoundEnd::PlayerKilled));
    }

    #[test]
    fn after_the_plant_only_the_bomb_or_a_wipe_ends_it() {
        // Dead player, clock out: the bomb is still ticking.
        assert_eq!(round_outcome(facts(true, 2, true, BombPhase::Planted)), None);
        assert_eq!(round_outcome(facts(false, 2, true, BombPhase::Planted)), None);
        assert_eq!(round_outcome(facts(true, 0, false, BombPhase::Planted)), Some(RoundEnd::SquadWiped));
        assert_eq!(round_outcome(facts(true, 2, false, BombPhase::Exploded)), Some(RoundEnd::Exploded));
        assert_eq!(round_outcome(facts(false, 1, false, BombPhase::Defused)), Some(RoundEnd::Defused));
    }

    #[test]
    fn winners() {
        assert!(RoundEnd::Exploded.player_won() && RoundEnd::SquadWiped.player_won());
        assert!(!RoundEnd::Defused.player_won() && !RoundEnd::TimeUp.player_won() && !RoundEnd::PlayerKilled.player_won());
    }
}
