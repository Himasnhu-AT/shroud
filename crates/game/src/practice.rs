//! Practice ("Free play"): the chosen map with the squad standing as statues at their spawns and
//! the bomb sites. They are the real enemies (MetaHuman, gear, hitboxes, blood, death fall) with
//! the AI switched off: they don't move, shoot, perceive or react. Main menu → Practice.
//!
//! - No round timer, no freeze time, no round end: [`Practice`] turns off `match_flow`'s
//!   round start/end, the round clock's limit and the economy's round start.
//! - No economy limits: money is kept at [`PRACTICE_MONEY`], buying works anywhere at any time
//!   (`economy::can_buy`), reserve ammo refills, and the kit starts with one of each grenade.
//! - [`RESET_KEY`] (H) stands every target up again (also in the pause menu).
//! - The player can still die (his own frag, the bomb): he respawns at the spawn after
//!   [`RESPAWN_S`] with the kit he had.
//!
//! **Nothing here is recorded.** The squad must not learn from practice and it is not a match:
//! with [`Practice`] on, these never run or return early — `match_flow` (match/round start and
//! end, so no `PendingReport`, no `Evolving`), `enemy::EnemySet` (no AI, no squad doctrine) and
//! `enemy::finish_round` (no tactics into the brain), `telemetry` (no player-model samples),
//! `career` (no objectives, no XP), `economy::start_round`/`squad_buy`. Trace events other than
//! `practice.*` are dropped (`dev::Dev::trace`), so the crash/perf reports get one
//! `practice.start` and no match events.

use bevy::prelude::*;
use brain::arena::Arena;
use brain::economy::Wallet;
use brain::grenade::GrenadeKind;
use brain::math::V2;
use engine::ui::prelude::MenuEvent;
use engine::ui::screens::ScreenRequest;

use crate::dev::Prop;
use crate::economy::{BuyMenu, PlayerWallet, RoundLoadout, RoundPhase};
use crate::player::Player;
use crate::settings::ShroudSettings;
use crate::state::{AppState, GameState, Paused};
use crate::weapon::{Arsenal, Health, Weapon};

/// Stands the targets up again.
pub const RESET_KEY: KeyCode = KeyCode::KeyH;
/// [`RESET_KEY`] as shown in the HUD and the controls list.
pub const RESET_KEY_LABEL: &str = "H";
/// Money in practice, topped up after every buy (the most a wallet holds).
pub const PRACTICE_MONEY: u32 = brain::economy::MAX_MONEY;
/// Seconds before a dead player respawns.
pub const RESPAWN_S: f32 = 2.5;
/// A bomb-site statue isn't placed this close (m) to another statue.
const MIN_SPACING_M: f32 = 3.0;

/// Practice is on (set from the menu; off again on the main menu and for a match).
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Practice {
    pub on: bool,
    /// The next `Playing` entry starts a session: a fresh kit and the `practice.start` event.
    starting: bool,
}

impl Practice {
    /// Start practice on the next `Playing` entry.
    pub fn begin(&mut self) {
        *self = Practice { on: true, starting: true };
    }
    pub fn end(&mut self) {
        *self = Practice::default();
    }
}

/// Run condition: practice is on.
pub fn in_practice(p: Option<Res<Practice>>) -> bool {
    p.is_some_and(|p| p.on)
}

/// Run condition: a real match (practice off). Everything that records or teaches runs only here.
pub fn in_match(p: Option<Res<Practice>>) -> bool {
    !in_practice(p)
}

/// Stand the targets up again (the reset key, the pause menu, `practice.reset`).
#[derive(Message, Clone, Copy, Debug, Default)]
pub struct ResetTargets;

/// Where the statues stand and which way they look: every enemy spawn, then each bomb site's
/// centre (when walkable and not crowding another statue), all facing the player's spawn.
pub fn target_spots(arena: &Arena) -> Vec<(V2, V2)> {
    let mut spots: Vec<V2> = arena.enemy_spawns.clone();
    for site in &arena.sites {
        let c = site.center();
        if arena.nav.walkable_at(c) && spots.iter().all(|s| s.dist(c) >= MIN_SPACING_M) {
            spots.push(c);
        }
    }
    spots
        .into_iter()
        .map(|p| {
            let to = arena.player_spawn - p;
            (p, if to.len() > 0.01 { to.norm() } else { V2::new(0.0, 1.0) })
        })
        .collect()
}

/// The kit a practice session starts with: pistol, no armor, one of each grenade.
pub fn starting_kit() -> RoundLoadout {
    let mut k = RoundLoadout::default();
    for g in GrenadeKind::ALL {
        k.grenades.add(g);
    }
    k
}

pub struct PracticePlugin;

impl Plugin for PracticePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Practice>()
            .add_message::<ResetTargets>()
            .add_systems(OnEnter(AppState::MainMenu), |mut p: ResMut<Practice>| {
                if p.on {
                    p.end();
                }
            })
            // Before the economy's round start (which practice skips): the arsenal and grenades
            // are handed out from `RoundLoadout` after it.
            .add_systems(OnEnter(GameState::Playing), start.run_if(in_practice).before(crate::economy::start_round).after(crate::map::MapSet))
            .add_systems(
                Update,
                (refill, reset_key, respawn, reset_targets.after(reset_key), menu_reset).run_if(in_state(GameState::Playing).and_then(in_practice)),
            );
        #[cfg(feature = "dev")]
        commands::register(app);
    }
}

/// Every `Playing` entry in practice: live at once (no freeze), money, the kit.
fn start(
    mut commands: Commands,
    mut practice: ResMut<Practice>,
    mut phase: ResMut<RoundPhase>,
    mut wallet: ResMut<PlayerWallet>,
    mut loadout: ResMut<RoundLoadout>,
    mut menu: ResMut<BuyMenu>,
    settings: Res<ShroudSettings>,
    players: Query<Entity, With<Player>>,
) {
    if practice.starting {
        practice.starting = false;
        *loadout = starting_kit();
        let map = brain::arena::MapId::ALL.iter().position(|m| *m == settings.map).unwrap_or(0) as i64;
        // Queued: `Dev` reads `Practice`, which this system writes.
        commands.queue(move |world: &mut World| crate::dev::trace(world, "practice.start", &[("map", Prop::Int(map))]));
        info!("practice: {} (targets don't fight back)", settings.map.label());
    }
    *phase = RoundPhase::practice();
    wallet.0 = Wallet { money: PRACTICE_MONEY, loss_streak: 0 };
    menu.open = false;
    for e in &players {
        match loadout.armor {
            Some(a) => commands.entity(e).insert(a),
            None => commands.entity(e).remove::<crate::armor::Armor>(),
        };
    }
}

/// Free refills: money back to [`PRACTICE_MONEY`] after a buy, reserve ammo back to full.
fn refill(mut wallet: ResMut<PlayerWallet>, mut guns: Query<(&mut Weapon, &mut Arsenal), With<Player>>) {
    if wallet.0.money < PRACTICE_MONEY {
        wallet.0.money = PRACTICE_MONEY;
    }
    for (mut w, mut a) in &mut guns {
        let full = w.kind.spec().reserve;
        if w.reserve < full {
            w.reserve = full;
        }
        if a.slots.iter().flatten().any(|g| g.reserve < g.kind.spec().reserve) {
            for g in a.slots.iter_mut().flatten() {
                g.reserve = g.reserve.max(g.kind.spec().reserve);
            }
        }
    }
}

fn reset_key(keys: Option<Res<ButtonInput<KeyCode>>>, paused: Res<Paused>, menu: Res<BuyMenu>, mut out: MessageWriter<ResetTargets>) {
    if !paused.0 && !menu.open && keys.is_some_and(|k| k.just_pressed(RESET_KEY)) {
        out.write(ResetTargets);
    }
}

/// The pause menu's "Reset targets": stand them up and resume.
fn menu_reset(mut events: MessageReader<MenuEvent>, mut out: MessageWriter<ResetTargets>, mut requests: MessageWriter<ScreenRequest>, mut pause: engine::pause::PauseCtl) {
    for MenuEvent(id) in events.read() {
        if *id == "practice_reset" {
            out.write(ResetTargets);
            requests.write(ScreenRequest::Replace(None));
            pause.set(false);
        }
    }
}

/// Respawn every statue where it stood (the squad spawn despawns the old ones and the corpses).
fn reset_targets(mut events: MessageReader<ResetTargets>, mut commands: Commands) {
    if events.read().count() > 0 {
        commands.run_system_cached(crate::enemy::spawn_squad);
        info!("practice: targets reset");
    }
}

/// Dead (his own frag, the bomb): back at the spawn after [`RESPAWN_S`], keeping the kit.
fn respawn(
    time: Res<Time>,
    players: Query<(&Health, &Arsenal, &Weapon, Option<&crate::armor::Armor>), With<Player>>,
    grenades: Res<crate::economy::PlayerGrenades>,
    mut loadout: ResMut<RoundLoadout>,
    mut next: ResMut<NextState<GameState>>,
    mut dead_for: Local<f32>,
) {
    let Ok((hp, a, w, armor)) = players.single() else { return };
    if !hp.is_dead() {
        *dead_for = 0.0;
        return;
    }
    *dead_for += time.delta_secs();
    if *dead_for < RESPAWN_S {
        return;
    }
    *dead_for = 0.0;
    let mut slots = a.slots;
    slots[a.active] = Some(w.state());
    *loadout = RoundLoadout {
        primary: slots[0].map(|g| g.kind),
        secondary: slots[1].map_or(brain::weapons::WeaponKind::Pistol, |g| g.kind),
        armor: armor.copied().filter(|a| a.points > 0.0),
        grenades: grenades.0,
    };
    // Re-entering Playing resets the player (and the targets) on the spawn.
    next.set(GameState::Playing);
}

#[cfg(feature = "dev")]
mod commands {
    use engine::devtools::{AddControlCommand, params};
    use serde_json::{Value, json};

    use super::*;
    use crate::enemy::{Enemy, EnemyCombat};

    pub(super) fn register(app: &mut App) {
        app.add_control_command(
            "practice.start",
            "Start practice (statue targets) on a map, from the menu or mid-game",
            params(&[("map", "string", "map name (default: the chosen map)")]),
            start_cmd,
        )
        .add_control_command("practice.reset", "Stand the practice targets up again", params(&[]), reset_cmd)
        .add_control_command("practice.state", "Practice on/off and the targets", params(&[]), state_cmd);
    }

    fn start_cmd(world: &mut World, p: Value) -> Result<Value, String> {
        if let Some(name) = p.get("map").and_then(Value::as_str) {
            let id = brain::arena::MapId::from_name(name).ok_or_else(|| format!("unknown map `{name}`"))?;
            world.resource_mut::<ShroudSettings>().map = id;
        }
        world.resource_mut::<Practice>().begin();
        world.resource_mut::<crate::match_flow::MatchFlow>().abandon();
        let in_game = world.get_resource::<State<AppState>>().is_some_and(|s| *s.get() == AppState::InGame);
        if in_game {
            world.resource_mut::<NextState<GameState>>().set(GameState::Playing);
        } else {
            world.resource_mut::<NextState<AppState>>().set(AppState::InGame);
        }
        Ok(json!({ "map": world.resource::<ShroudSettings>().map.label() }))
    }

    fn reset_cmd(world: &mut World, _: Value) -> Result<Value, String> {
        if !world.resource::<Practice>().on {
            return Err("practice is off".into());
        }
        world.write_message(ResetTargets);
        Ok(json!({}))
    }

    fn state_cmd(world: &mut World, _: Value) -> Result<Value, String> {
        let on = world.resource::<Practice>().on;
        let targets: Vec<Value> = world
            .query_filtered::<(&Transform, &EnemyCombat, &Health), With<Enemy>>()
            .iter(world)
            .map(|(tf, c, hp)| json!({ "pos": [tf.translation.x, tf.translation.z], "hp": hp.hp, "dead": c.dead, "gun": c.gun.held.label() }))
            .collect();
        // What a match would have recorded: must not move during practice.
        let t = &world.resource::<crate::telemetry::Telemetry>().0;
        let recorded = json!({
            "telemetry_rounds": t.rounds,
            "telemetry_shots": t.shots.iter().sum::<f32>(),
            "telemetry_duels": t.duels.len(),
            "match": world.resource::<crate::match_flow::MatchFlow>().match_no,
            "career_rounds": world.resource::<crate::career::MatchCareer>().log.rounds.len(),
        });
        Ok(json!({ "on": on, "targets": targets, "recorded": recorded }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain::arena::MapId;

    #[test]
    fn every_map_has_statue_spots_on_walkable_ground_facing_the_player() {
        for id in MapId::ALL {
            let a = Arena::build(id);
            let spots = target_spots(&a);
            assert!(spots.len() >= a.enemy_spawns.len(), "{id:?}: every spawn gets a statue");
            for (p, look) in &spots {
                assert!((look.len() - 1.0).abs() < 1e-3, "{id:?}: unit facing");
                assert!(look.dot((a.player_spawn - *p).norm()) > 0.99, "{id:?}: faces the player spawn");
            }
        }
    }

    #[test]
    fn practice_sends_no_match_trace_events() {
        use crate::dev::practice_drops;
        for e in ["match.start", "match.end", "round.end", "player.died", "buy", "bomb.planted", "career.objective", "lamp.broken"] {
            assert!(practice_drops(true, e), "{e} must not be sent in practice");
            assert!(!practice_drops(false, e), "{e} is sent in a match");
        }
        assert!(!practice_drops(true, "practice.start"));
    }

    #[test]
    fn begin_and_end() {
        let mut p = Practice::default();
        assert!(!p.on);
        p.begin();
        assert!(p.on && p.starting);
        p.end();
        assert_eq!(p, Practice::default());
    }

    #[test]
    fn the_practice_kit_has_every_grenade() {
        let k = starting_kit();
        for g in GrenadeKind::ALL {
            assert_eq!(k.grenades.count(g), 1);
        }
        assert_eq!(k.primary, None);
    }
}
