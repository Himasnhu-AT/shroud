//! The CS-style round economy on top of `match_flow`: freeze time, money, the buy window,
//! what the player carries from round to round, and the squad's own wallets and buys. The rules
//! (prices, rewards, eco/force/full buy) are `brain::economy`; the buy menu UI is
//! `ui::buy_menu`.
//!
//! - Round start: [`RoundPhase`] holds the freeze countdown ([`FreezeLimit`], 0 in headless and
//!   test-driven runs) and then the buy window. During freeze the player can look and buy but
//!   not move or fire, and the enemy AI does not run.
//! - The buy window is open during freeze and for [`BUY_TIME_S`] after it while the player is
//!   within [`BUY_ZONE_M`] of the spawn.
//! - Round end: wallets are paid (win / loss bonus); the player keeps guns and armor by
//!   surviving ([`RoundLoadout`]), else starts the next round with the pistol only. Same for
//!   squad members.
//! - New match: everything resets (start money, pistol only, no armor).

use bevy::prelude::*;
use brain::economy::{self as econ, BuyMode, Gear, GrenadeBag, Wallet};
use brain::grenade::GrenadeKind;
use brain::weapons::{Slot, WeaponKind};

use crate::armed::Armed;
use crate::armor::Armor;
use crate::dev::{Dev, Prop};
use crate::enemy::{self, Enemy, EnemyCombat, EnemySet, GunState as EnemyGun, Mind, Role, Squad};
use crate::map::{ArenaRes, to_ground};
use crate::match_flow::{MatchFlow, ROUNDS_TO_WIN, begin_round, check_round_end};
use crate::player::{Player, PlayerIntent, PlayerSet};
use crate::state::{GameState, Headless, not_paused};
use crate::weapon::arsenal::{GunState, Phase, Weapon, draw_time};
use crate::weapon::{Arsenal, Damage, DamageCause, GunEvent, Health, WeaponSet};

/// Freeze time at round start in a normal run (s).
pub const FREEZE_S: f32 = 8.0;
/// Buying stays open this long after freeze time ends, inside the buy zone (s).
pub const BUY_TIME_S: f32 = 20.0;
/// Buy zone radius around the player spawn (m).
pub const BUY_ZONE_M: f32 = 8.0;
/// Kill reward for a frag or bomb kill (CS pays $300 for a grenade kill). `brain::economy` has
/// only per-gun rewards (`WeaponKind::kill_reward`).
pub const UTILITY_KILL_REWARD: u32 = 300;

/// Freeze time for the next rounds (s). Headless and test-driven runs default to 0 so scripts
/// and tests don't stall; `round.freeze` sets it over the control port.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct FreezeLimit(pub f32);

impl Default for FreezeLimit {
    fn default() -> Self {
        FreezeLimit(FREEZE_S)
    }
}

/// Where the current round is: freeze countdown, then the buy window.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct RoundPhase {
    /// Seconds of freeze time left (0 = live).
    pub freeze_left: f32,
    /// Seconds of the post-freeze buy window left.
    pub buy_left: f32,
    /// Round pay has been handed out (round over).
    paid: bool,
}

impl RoundPhase {
    pub fn frozen(&self) -> bool {
        self.freeze_left > 0.0
    }
    /// Practice: live at once, and the buy window never closes.
    pub fn practice() -> Self {
        RoundPhase { freeze_left: 0.0, buy_left: f32::INFINITY, paid: false }
    }
}

/// Run condition: the round is live (freeze time over).
pub fn round_live(phase: Res<RoundPhase>) -> bool {
    !phase.frozen()
}

/// The player's money.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerWallet(pub Wallet);

/// What the player starts the next round with (kept by surviving).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct RoundLoadout {
    pub primary: Option<WeaponKind>,
    pub secondary: WeaponKind,
    pub armor: Option<Armor>,
    /// Grenades not thrown (kept like guns).
    pub grenades: GrenadeBag,
}

impl Default for RoundLoadout {
    fn default() -> Self {
        RoundLoadout { primary: None, secondary: WeaponKind::Pistol, armor: None, grenades: GrenadeBag::default() }
    }
}

/// The grenades the player carries this round (`weapon::throwing` equips and throws them).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerGrenades(pub GrenadeBag);

impl RoundLoadout {
    /// The arsenal for a round start: full ammo, the primary drawn if there is one.
    pub fn arsenal(&self) -> Arsenal {
        let p = self.primary.map(GunState::full);
        Arsenal { slots: [p, Some(GunState::full(self.secondary))], active: if p.is_some() { 0 } else { 1 }, last: if p.is_some() { 1 } else { 0 } }
    }
}

/// The squad's money and what each member carries over (by spawn slot).
#[derive(Resource, Debug, Clone, Default)]
pub struct SquadEconomy {
    pub wallets: Vec<Wallet>,
    /// Primary a member survived with (bought guns only; `None` = pistol).
    pub kept: Vec<Option<WeaponKind>>,
    pub kept_armor: Vec<Option<Armor>>,
    /// Grenades a member survived with (not thrown), by slot.
    pub kept_grenades: Vec<GrenadeBag>,
    /// What each member carries this round (bought + kept, minus throws), by slot.
    pub bags: Vec<GrenadeBag>,
    /// This round's buy.
    pub mode: Option<BuyMode>,
    /// The enemy that last hit the player this round (slot, what hit): paid if the player dies.
    last_hitter: Option<(usize, DamageCause)>,
}

/// The buy menu is open (cursor freed, look and weapon keys off). `ui::buy_menu` draws it.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct BuyMenu {
    pub open: bool,
}

/// Something to buy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuyItem {
    Gun(WeaponKind),
    Gear(Gear),
    Grenade(GrenadeKind),
}

/// Buy-menu name of a grenade.
pub fn grenade_label(kind: GrenadeKind) -> &'static str {
    match kind {
        GrenadeKind::Frag => "Frag grenade",
        GrenadeKind::Flash => "Flashbang",
        GrenadeKind::Smoke => "Smoke grenade",
    }
}

impl BuyItem {
    pub fn label(self) -> &'static str {
        match self {
            BuyItem::Gun(k) => k.label(),
            BuyItem::Gear(g) => g.label(),
            BuyItem::Grenade(k) => grenade_label(k),
        }
    }

    pub fn category(self) -> econ::BuyCategory {
        match self {
            BuyItem::Gun(k) => econ::category(k),
            BuyItem::Gear(_) => econ::BuyCategory::Gear,
            BuyItem::Grenade(_) => econ::BuyCategory::Grenades,
        }
    }
}

/// Every buy-menu item in column order: guns from `WeaponKind::ALL` (so new guns appear by
/// themselves), then gear, then grenades.
pub fn catalog() -> Vec<BuyItem> {
    let mut v = Vec::new();
    for cat in econ::BuyCategory::ALL {
        if cat == econ::BuyCategory::Gear {
            v.extend(Gear::ALL.iter().map(|g| BuyItem::Gear(*g)));
        } else if cat == econ::BuyCategory::Grenades {
            v.extend(GrenadeKind::ALL.iter().map(|k| BuyItem::Grenade(*k)));
        } else {
            v.extend(econ::guns_in(cat).into_iter().map(BuyItem::Gun));
        }
    }
    v
}

/// What an item costs the player now: `Err` says why it can't be bought ("owned", "full").
pub fn quote(item: BuyItem, carried: &[WeaponKind], armor: Option<Armor>, bag: GrenadeBag) -> Result<u32, &'static str> {
    match item {
        BuyItem::Grenade(k) => bag.quote(k),
        BuyItem::Gun(k) if carried.contains(&k) => Err("owned"),
        BuyItem::Gun(k) => Ok(k.price()),
        BuyItem::Gear(g) => {
            let a = armor.unwrap_or_default();
            g.price(a.points, a.helmet && a.points > 0.0).ok_or("owned")
        }
    }
}

pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn build(&self, app: &mut App) {
        let hold = hold_player.after(PlayerSet::Input).before(PlayerSet::Move);
        // After the control port's player overrides, so scripted moves are held too.
        #[cfg(feature = "dev")]
        let hold = hold.after(crate::dev::ControlDrive);
        app.init_resource::<FreezeLimit>()
            .init_resource::<RoundPhase>()
            .init_resource::<PlayerWallet>()
            .init_resource::<RoundLoadout>()
            .init_resource::<PlayerGrenades>()
            .init_resource::<SquadEconomy>()
            .init_resource::<BuyMenu>()
            .add_systems(Startup, test_runs_skip_freeze)
            // Practice has no rounds and no squad buy (`practice::start` sets money and kit).
            .add_systems(
                OnEnter(GameState::Playing),
                (start_round.after(begin_round), squad_buy.after(enemy::spawn_squad).after(start_round)).run_if(crate::practice::in_match),
            )
            .add_systems(OnExit(GameState::Playing), close_menu)
            // The AI holds during freeze time.
            .configure_sets(FixedUpdate, EnemySet.run_if(round_live))
            .add_systems(
                Update,
                (
                    tick_phase.run_if(not_paused),
                    hold,
                    (track_kills, pay_round.after(check_round_end)).chain().after(WeaponSet),
                    track_squad_throws,
                )
                    .run_if(in_state(GameState::Playing)),
            );
        #[cfg(feature = "dev")]
        commands::register(app);
    }
}

fn test_runs_skip_freeze(headless: Res<Headless>, mut limit: ResMut<FreezeLimit>) {
    if headless.0 || engine::cli::test_driven() {
        limit.0 = 0.0;
    }
}

/// Round start: a new match resets money and loadout; then the round's freeze time begins.
/// Runs before `weapon::arsenal::reset_loadout`, which hands out [`RoundLoadout`].
pub(crate) fn start_round(
    mut commands: Commands,
    flow: Res<MatchFlow>,
    limit: Res<FreezeLimit>,
    mut phase: ResMut<RoundPhase>,
    mut wallet: ResMut<PlayerWallet>,
    mut loadout: ResMut<RoundLoadout>,
    mut squad: ResMut<SquadEconomy>,
    mut menu: ResMut<BuyMenu>,
    players: Query<Entity, With<Player>>,
) {
    if flow.round <= 1 {
        wallet.0 = Wallet::default();
        *loadout = RoundLoadout::default();
        *squad = SquadEconomy::default();
    }
    squad.last_hitter = None;
    *phase = RoundPhase { freeze_left: limit.0.max(0.0), buy_left: BUY_TIME_S, paid: false };
    menu.open = false;
    for e in &players {
        match loadout.armor {
            Some(a) => commands.entity(e).insert(a),
            None => commands.entity(e).remove::<Armor>(),
        };
    }
}

fn close_menu(mut menu: ResMut<BuyMenu>) {
    menu.open = false;
}

fn tick_phase(time: Res<Time>, flow: Res<MatchFlow>, mut phase: ResMut<RoundPhase>) {
    if flow.round_over.is_some() {
        return;
    }
    let dt = time.delta_secs();
    if phase.freeze_left > 0.0 {
        phase.freeze_left = (phase.freeze_left - dt).max(0.0);
    } else if phase.buy_left > 0.0 {
        phase.buy_left = (phase.buy_left - dt).max(0.0);
    }
}

/// Freeze time: no moving, jumping or firing (looking and buying are fine). Round over: no
/// firing (the kit the player keeps was settled when the round ended; grenades are held in
/// `weapon::throwing`).
fn hold_player(
    phase: Res<RoundPhase>,
    flow: Res<MatchFlow>,
    mut intent: ResMut<PlayerIntent>,
    mut inputs: Query<&mut engine::fps::FpsInput, With<Player>>,
) {
    let over = flow.round_over.is_some();
    if (phase.frozen() || over) && intent.fire {
        intent.fire = false;
    }
    if !phase.frozen() {
        return;
    }
    for mut i in &mut inputs {
        if i.move_axis != Vec2::ZERO || i.jump || i.sprint {
            i.move_axis = Vec2::ZERO;
            i.jump = false;
            i.sprint = false;
        }
    }
}

/// Can the player buy right now? Freeze time, or the buy window near the spawn; alive; round on.
pub fn can_buy(world: &mut World) -> Result<(), &'static str> {
    let phase = *world.resource::<RoundPhase>();
    let practice = world.get_resource::<crate::practice::Practice>().is_some_and(|p| p.on);
    if world.resource::<MatchFlow>().round_over.is_some() {
        return Err("round over");
    }
    let spawn = world.resource::<ArenaRes>().0.player_spawn;
    let (tf, hp) = world
        .query_filtered::<(&Transform, &Health), With<Player>>()
        .iter(world)
        .next()
        .map(|(t, h)| (*t, h.is_dead()))
        .ok_or("no player")?;
    if hp {
        return Err("dead");
    }
    // Practice: anywhere, any time.
    if phase.frozen() || practice {
        return Ok(());
    }
    if phase.buy_left <= 0.0 {
        return Err("buy time is over");
    }
    if to_ground(tf.translation).dist(spawn) > BUY_ZONE_M {
        return Err("outside the buy zone");
    }
    Ok(())
}

/// The player's guns (live state of the gun in hand included) and armor.
pub fn player_kit(world: &mut World) -> Option<(Vec<WeaponKind>, Option<Armor>)> {
    let (a, w, armor) = world.query_filtered::<(&Arsenal, &Weapon, Option<&Armor>), With<Player>>().iter(world).next()?;
    let mut kinds: Vec<WeaponKind> = a.slots.iter().flatten().map(|g| g.kind).collect();
    kinds.push(w.kind);
    Some((kinds, armor.copied()))
}

/// Buy `item` for the player: checks the window (unless `anywhere`), price and ownership,
/// spends the money and hands the item over. Returns the money left.
pub fn buy(world: &mut World, item: BuyItem, anywhere: bool) -> Result<u32, String> {
    if !anywhere {
        can_buy(world)?;
    }
    let (carried, armor) = player_kit(world).ok_or("no player")?;
    let bag = world.resource::<PlayerGrenades>().0;
    let price = quote(item, &carried, armor, bag)?;
    if !world.resource_mut::<PlayerWallet>().0.spend(price) {
        return Err(format!("can't afford {} (${price})", item.label()));
    }
    let e = world.query_filtered::<Entity, With<Player>>().iter(world).next().ok_or("no player")?;
    match item {
        BuyItem::Gun(kind) => {
            let slot = if kind.spec().slot == Slot::Primary { 0 } else { 1 };
            let gun = GunState::full(kind);
            let held = world.get::<Weapon>(e).map(|w| w.state());
            let mut ent = world.entity_mut(e);
            if let Some(mut a) = ent.get_mut::<Arsenal>() {
                // The gun in hand lives in `Weapon`: put its live state (magazine, reserve) back in
                // its slot before the new gun takes the hand, or it would come back full.
                if let Some(h) = held {
                    let active = a.active;
                    a.slots[active] = Some(h);
                }
                a.slots[slot] = Some(gun);
                if a.active != slot {
                    a.last = a.active;
                    a.active = slot;
                }
            }
            let mut w = Weapon::new(gun);
            w.phase = Phase::Drawing { left: draw_time(kind), total: draw_time(kind) };
            ent.insert((w, Armed(kind)));
            world.write_message(GunEvent::Draw { kind });
        }
        BuyItem::Gear(g) => {
            let helmet = g == Gear::KevlarHelmet || armor.is_some_and(|a| a.helmet && a.points > 0.0);
            world.entity_mut(e).insert(Armor::vest(helmet));
        }
        BuyItem::Grenade(k) => {
            world.resource_mut::<PlayerGrenades>().0.add(k);
        }
    }
    let left = world.resource::<PlayerWallet>().0.money;
    crate::dev::trace(world, "buy", &[("price", Prop::Int(price as i64)), ("money", Prop::Int(left as i64))]);
    info!("bought {} for ${price}, ${left} left", item.label());
    Ok(left)
}

/// What one dead enemy pays the player: by the cause of death. His gun kills pay that gun's
/// reward; his frags and the bomb he planted pay [`UTILITY_KILL_REWARD`]; kills by anyone else
/// (an enemy's own frag) pay nothing.
pub fn kill_reward(by_player: bool, cause: DamageCause) -> Option<u32> {
    match cause {
        DamageCause::Gun(k) if by_player => Some(k.kill_reward()),
        DamageCause::Grenade if by_player => Some(UTILITY_KILL_REWARD),
        DamageCause::Bomb => Some(UTILITY_KILL_REWARD),
        _ => None,
    }
}

/// Kill bookkeeping for one round.
#[derive(Default)]
struct KillBook {
    round: (u32, u32),
    /// Last hit on each enemy: (by the player, cause).
    last: Vec<(Entity, bool, DamageCause)>,
    paid: Vec<Entity>,
    /// The round was already over last frame (kills from then on pay nothing).
    closed: bool,
}

/// Kill rewards for the player, paid from what landed the kill (not the gun in hand), and the
/// enemy that last hit the player (it is paid at round end if he died). Kills that land after
/// the round ended (during the round-over pause) pay nothing.
fn track_kills(
    flow: Res<MatchFlow>,
    mut damage: MessageReader<Damage>,
    players: Query<(), With<Player>>,
    enemies: Query<(Entity, &Enemy, &Health)>,
    mut squad: ResMut<SquadEconomy>,
    mut wallet: ResMut<PlayerWallet>,
    mut book: Local<KillBook>,
) {
    let key = (flow.match_no, flow.round);
    if book.round != key {
        *book = KillBook { round: key, ..default() };
    }
    for d in damage.read() {
        if players.contains(d.target) {
            // The bomb (or anything not an enemy) clears it: nobody in the squad earned that kill.
            squad.last_hitter = enemies.get(d.source).ok().map(|(_, e, _)| (e.slot, d.cause));
        } else if enemies.contains(d.target) && !book.paid.contains(&d.target) {
            let by_player = players.contains(d.source);
            match book.last.iter_mut().find(|l| l.0 == d.target) {
                Some(l) => *l = (d.target, by_player, d.cause),
                None => book.last.push((d.target, by_player, d.cause)),
            }
        }
    }
    let closed = book.closed;
    book.closed = flow.round_over.is_some();
    for (e, _, hp) in &enemies {
        if !hp.is_dead() || book.paid.contains(&e) {
            continue;
        }
        book.paid.push(e);
        if closed {
            continue;
        }
        let Some(&(_, by_player, cause)) = book.last.iter().find(|l| l.0 == e) else { continue };
        if let Some(r) = kill_reward(by_player, cause) {
            wallet.0.earn(r);
        }
    }
}

/// The squad's grenades leave the members' bags as they are thrown (the thrower's, else any
/// member's: the coordinator picks throwers from the squad's pooled budget).
fn track_squad_throws(mut throws: MessageReader<crate::enemy::grenade::ThrowGrenade>, enemies: Query<&Enemy>, mut squad: ResMut<SquadEconomy>) {
    for t in throws.read() {
        let Ok(e) = enemies.get(t.thrower) else { continue };
        take_from_bags(&mut squad.bags, e.slot, t.kind);
    }
}

/// Take one `kind` from `slot`'s bag, or from the first other bag holding one.
fn take_from_bags(bags: &mut [GrenadeBag], slot: usize, kind: GrenadeKind) {
    if bags.get_mut(slot).is_some_and(|b| b.take(kind)) {
        return;
    }
    if let Some(b) = bags.iter_mut().find(|b| b.count(kind) > 0) {
        b.take(kind);
    }
}

/// Round over: pay both sides and decide what everyone carries into the next round.
fn pay_round(
    flow: Res<MatchFlow>,
    mut phase: ResMut<RoundPhase>,
    mut wallet: ResMut<PlayerWallet>,
    mut loadout: ResMut<RoundLoadout>,
    mut squad: ResMut<SquadEconomy>,
    mut menu: ResMut<BuyMenu>,
    grenades: Res<PlayerGrenades>,
    bomb: Res<crate::bomb::Bomb>,
    player: Query<(&Health, &Arsenal, &Weapon, Option<&Armor>), With<Player>>,
    enemies: Query<(&Enemy, &EnemyCombat, Option<&Armor>)>,
    dev: Dev,
) {
    let Some((_, won)) = flow.round_over else { return };
    if phase.paid {
        return;
    }
    phase.paid = true;
    menu.open = false;
    let gained = wallet.0.round_end(won) + wallet.0.earn(econ::planted_round_bonus(bomb.was_planted(), won));
    // Player: survive = keep the kit (ammo is refilled next round); die = pistol only.
    *loadout = match player.single() {
        Ok((hp, a, w, armor)) if !hp.is_dead() => {
            let mut slots = a.slots;
            slots[a.active] = Some(w.state());
            RoundLoadout {
                primary: slots[0].map(|g| g.kind),
                secondary: slots[1].map_or(WeaponKind::Pistol, |g| g.kind),
                armor: armor.copied().filter(|a| a.points > 0.0),
                grenades: grenades.0,
            }
        }
        _ => RoundLoadout::default(),
    };
    // Squad: everyone gets the round pay; survivors keep what they carry.
    let n = squad.wallets.len();
    for w in &mut squad.wallets {
        w.round_end(!won);
    }
    // The kill reward goes to whoever last hit him if he died, whatever the round result (after a
    // plant he can die and still win it).
    let player_dead = player.single().is_ok_and(|(hp, ..)| hp.is_dead());
    let hitter = squad.last_hitter.take();
    if player_dead && let Some((slot, cause)) = hitter && slot < n {
        squad.wallets[slot].earn(match cause {
            DamageCause::Gun(k) => k.kill_reward(),
            _ => UTILITY_KILL_REWARD,
        });
    }
    squad.kept = vec![None; n];
    squad.kept_armor = vec![None; n];
    let mut kept_grenades = vec![GrenadeBag::default(); n];
    for (e, c, armor) in &enemies {
        if e.slot < n && !c.dead {
            squad.kept[e.slot] = Some(c.gun.primary).filter(|k| k.spec().slot == Slot::Primary);
            squad.kept_armor[e.slot] = armor.copied().filter(|a| a.points > 0.0);
            kept_grenades[e.slot] = squad.bags.get(e.slot).copied().unwrap_or_default();
        }
    }
    squad.kept_grenades = kept_grenades;
    dev.trace("econ.round", &[("money", Prop::Int(wallet.0.money as i64)), ("gained", Prop::Int(gained as i64))]);
}

/// After the squad spawns: each member buys with its own wallet; which gun it picks among those
/// it can afford comes from its evolved loadout genes (`brain::economy::member_buy`).
fn squad_buy(
    mut commands: Commands,
    flow: Res<MatchFlow>,
    mut squad: ResMut<Squad>,
    mut econ_res: ResMut<SquadEconomy>,
    mut enemies: Query<(Entity, &Enemy, &mut Mind, &mut EnemyCombat, &mut Armed)>,
    dev: Dev,
) {
    let n = enemies.iter().map(|(_, e, ..)| e.slot + 1).max().unwrap_or(0);
    let s = &mut *econ_res;
    // Difficulty can change the squad size between rounds: newcomers get the team average.
    if s.wallets.len() < n {
        let avg = if s.wallets.is_empty() {
            Wallet::default()
        } else {
            let m = s.wallets.iter().map(|w| w.money).sum::<u32>() / s.wallets.len() as u32;
            Wallet { money: m, loss_streak: s.wallets[0].loss_streak }
        };
        s.wallets.resize(n, avg);
    }
    s.kept.resize(n, None);
    s.kept_armor.resize(n, None);
    s.kept_grenades.resize(n, GrenadeBag::default());
    s.bags = vec![GrenadeBag::default(); n];
    let must_win = flow.player_rounds + 1 >= ROUNDS_TO_WIN;
    let mode = econ::team_mode(&s.wallets, flow.round <= 1, must_win);
    s.mode = Some(mode);
    // Guns the difficulty lets this squad field (the gene draft's caps).
    let caps = brain::loadout::caps(squad.difficulty, n);
    let allowed: Vec<WeaponKind> =
        WeaponKind::PRIMARIES.iter().enumerate().filter(|(i, _)| caps.get(*i).is_none_or(|c| *c > 0)).map(|(_, k)| *k).collect();
    let mut spent = 0;
    let mut bought = Vec::new();
    // Grenades: only squads that use utility (Veteran, Elite) buy them; what they buy is what
    // the coordinator may throw this round (capped by the difficulty).
    let buys_utility = squad.params.is_some_and(|p| p.utility);
    let mut bag_total = GrenadeBag::default();
    let mut list: Vec<_> = enemies.iter_mut().collect();
    list.sort_by_key(|(_, e, ..)| e.slot);
    for (ent, e, mind, c, armed) in list {
        let slot = e.slot;
        let ranked = econ::rank_primaries(mind.0.weapon(), &mind.0.traits.loadout, &allowed);
        // Utility squads with money for a gun anyway buy their grenades first (CS-style force
        // buys keep a flash and a frag); poorer ones buy them with what is left.
        let first = buys_utility && econ::utility_first(s.wallets[slot].money);
        // Grenades it survived with (only utility squads ever have any).
        let mut bag = if buys_utility { s.kept_grenades[slot] } else { GrenadeBag::default() };
        if first {
            let (g, cost) = econ::member_grenades(mode, s.wallets[slot].money, bag);
            s.wallets[slot].spend(cost);
            spent += cost;
            bag = g;
        }
        let b = econ::member_buy(mode, s.wallets[slot].money, s.kept[slot], s.kept_armor[slot].is_some(), &ranked);
        s.wallets[slot].spend(b.cost);
        spent += b.cost;
        if buys_utility && !first {
            let (g, cost) = econ::member_grenades(mode, s.wallets[slot].money, bag);
            s.wallets[slot].spend(cost);
            spent += cost;
            bag = g;
        }
        bag_total.frag += bag.frag;
        bag_total.flash += bag.flash;
        s.bags[slot] = bag;
        let kind = b.primary.unwrap_or(WeaponKind::Pistol);
        apply_enemy_gun(kind, mind, c, armed);
        let armor = match (s.kept_armor[slot], b.armor) {
            (Some(a), _) => Some(a),
            (None, true) => Some(Armor::vest(b.helmet)),
            _ => None,
        };
        if let Some(a) = armor {
            commands.entity(ent).insert(a);
        }
        bought.push(format!(
            "{}{}{}",
            kind.label(),
            if armor.is_some() { "+armor" } else { "" },
            if bag.total() > 0 { format!("+{}F{}B", bag.frag, bag.flash) } else { String::new() }
        ));
    }
    let (frags, flashes) = squad.difficulty.grenades();
    if buys_utility && let Some(c) = squad.coord.as_mut() {
        c.utility = brain::squad::UtilityBudget { frags: frags.min(bag_total.frag as u32), flashes: flashes.min(bag_total.flash as u32) };
    }
    info!("squad {}: {} (spent ${spent})", mode.label(), bought.join(", "));
    dev.trace("econ.squad_buy", &[("spent", Prop::Int(spent as i64)), ("full", Prop::Bool(mode == BuyMode::Full)), ("eco", Prop::Bool(mode == BuyMode::Eco))]);
}

fn apply_enemy_gun(kind: WeaponKind, mut mind: Mut<Mind>, mut c: Mut<EnemyCombat>, mut armed: Mut<Armed>) {
    if mind.0.weapon() != kind {
        mind.0.set_weapon(kind);
    }
    if c.gun.primary != kind {
        c.gun = EnemyGun::new(kind);
        c.role = Role::of(kind);
    }
    if armed.0 != kind {
        armed.0 = kind;
    }
}

#[cfg(feature = "dev")]
mod commands {
    use engine::devtools::{AddControlCommand, params};
    use serde_json::{Value, json};

    use super::*;

    pub(super) fn register(app: &mut App) {
        app.add_control_command("econ.state", "Money, loadout, freeze/buy window and the squad's wallets", params(&[]), state)
            .add_control_command(
                "econ.set_money",
                "Set the player's money (and optionally every squad member's)",
                params(&[("money", "integer", "player credits"), ("squad", "integer", "credits for each squad member")]),
                set_money,
            )
            .add_control_command(
                "buy",
                "Buy for the player (same rules as the buy menu)",
                params(&[
                    ("item", "string", "a gun (rifle, smg, Kestrel…), kevlar or helmet"),
                    ("anywhere", "boolean", "ignore the buy window (default false)"),
                ]),
                buy_cmd,
            )
            .add_control_command("buy.menu", "Open or close the buy menu", params(&[("open", "boolean", "default true")]), menu_cmd)
            .add_control_command(
                "round.freeze",
                "Set freeze time (s) for the next rounds, and restart it now",
                params(&[("seconds", "number", "0 = no freeze"), ("now", "boolean", "also apply to the current round (default true)")]),
                freeze_cmd,
            );
    }

    fn state(world: &mut World, _: Value) -> Result<Value, String> {
        let w = world.resource::<PlayerWallet>().0;
        let p = *world.resource::<RoundPhase>();
        let l = *world.resource::<RoundLoadout>();
        let s = world.resource::<SquadEconomy>().clone();
        let can = can_buy(world).err();
        let kit = player_kit(world);
        let g = world.resource::<PlayerGrenades>().0;
        Ok(json!({
            "money": w.money,
            "loss_streak": w.loss_streak,
            "freeze_left": p.freeze_left,
            "buy_left": p.buy_left,
            "can_buy": can.is_none(),
            "why_not": can,
            "menu_open": world.resource::<BuyMenu>().open,
            "carried": kit.as_ref().map(|(k, _)| k.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>()),
            "armor": kit.and_then(|(_, a)| a).map(|a| json!({"points": a.points, "helmet": a.helmet})),
            "grenades": {"frag": g.frag, "flash": g.flash, "smoke": g.smoke},
            "next_round": {"primary": l.primary.map(|k| format!("{k:?}")), "secondary": format!("{:?}", l.secondary), "armor": l.armor.is_some()},
            "squad": {
                "mode": s.mode.map(|m| m.label()),
                "money": s.wallets.iter().map(|w| w.money).collect::<Vec<_>>(),
                "kept": s.kept.iter().map(|k| k.map(|k| format!("{k:?}"))).collect::<Vec<_>>(),
            },
        }))
    }

    fn set_money(world: &mut World, p: Value) -> Result<Value, String> {
        let clamp = |v: u64| (v as u32).min(econ::MAX_MONEY);
        if let Some(m) = p.get("money").and_then(Value::as_u64) {
            world.resource_mut::<PlayerWallet>().0.money = clamp(m);
        }
        if let Some(m) = p.get("squad").and_then(Value::as_u64) {
            let mut s = world.resource_mut::<SquadEconomy>();
            if s.wallets.is_empty() {
                s.wallets = vec![Wallet::default(); 4];
            }
            for w in &mut s.wallets {
                w.money = clamp(m);
            }
        }
        state(world, Value::Null)
    }

    fn parse_item(s: &str) -> Option<BuyItem> {
        match s.trim().to_ascii_lowercase().as_str() {
            "kevlar" | "vest" | "armor" => Some(BuyItem::Gear(Gear::Kevlar)),
            "helmet" | "kevlar+helmet" | "kevlar_helmet" => Some(BuyItem::Gear(Gear::KevlarHelmet)),
            "frag" | "he" | "grenade" => Some(BuyItem::Grenade(GrenadeKind::Frag)),
            "flash" | "flashbang" => Some(BuyItem::Grenade(GrenadeKind::Flash)),
            "smoke" => Some(BuyItem::Grenade(GrenadeKind::Smoke)),
            other => crate::weapon::parse_kind(other).map(BuyItem::Gun),
        }
    }

    fn buy_cmd(world: &mut World, p: Value) -> Result<Value, String> {
        let s = p.get("item").and_then(Value::as_str).ok_or("`item` (string) is required")?;
        let item = parse_item(s).ok_or_else(|| format!("unknown item `{s}`"))?;
        let anywhere = p.get("anywhere").and_then(Value::as_bool).unwrap_or(false);
        let left = buy(world, item, anywhere)?;
        Ok(json!({"bought": item.label(), "money": left}))
    }

    fn menu_cmd(world: &mut World, p: Value) -> Result<Value, String> {
        let open = p.get("open").and_then(Value::as_bool).unwrap_or(true);
        if open {
            can_buy(world)?;
        }
        world.resource_mut::<BuyMenu>().open = open;
        Ok(json!({"open": open}))
    }

    fn freeze_cmd(world: &mut World, p: Value) -> Result<Value, String> {
        let secs = p.get("seconds").and_then(Value::as_f64).ok_or("`seconds` is required")?.max(0.0) as f32;
        world.resource_mut::<FreezeLimit>().0 = secs;
        if p.get("now").and_then(Value::as_bool).unwrap_or(true) {
            let mut ph = world.resource_mut::<RoundPhase>();
            ph.freeze_left = secs;
            ph.buy_left = BUY_TIME_S;
        }
        Ok(json!({"seconds": secs}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_every_gun_and_gear() {
        let c = catalog();
        for k in WeaponKind::ALL {
            assert!(c.contains(&BuyItem::Gun(k)), "{k:?}");
        }
        assert!(c.contains(&BuyItem::Gear(Gear::Kevlar)) && c.contains(&BuyItem::Gear(Gear::KevlarHelmet)));
        for k in GrenadeKind::ALL {
            assert!(c.contains(&BuyItem::Grenade(k)), "{k:?}");
        }
        // Column order.
        let cats: Vec<_> = c.iter().map(|i| i.category()).collect();
        let mut sorted = cats.clone();
        sorted.sort_by_key(|c| econ::BuyCategory::ALL.iter().position(|x| x == c));
        assert_eq!(cats, sorted);
    }

    #[test]
    fn quotes() {
        let carried = [WeaponKind::Pistol];
        let bag = GrenadeBag::default();
        assert_eq!(quote(BuyItem::Gun(WeaponKind::Pistol), &carried, None, bag), Err("owned"));
        assert_eq!(quote(BuyItem::Gun(WeaponKind::Rifle), &carried, None, bag), Ok(WeaponKind::Rifle.price()));
        assert_eq!(quote(BuyItem::Gear(Gear::Kevlar), &carried, Some(Armor::vest(false)), bag), Err("owned"));
        assert_eq!(quote(BuyItem::Gear(Gear::KevlarHelmet), &carried, Some(Armor::vest(false)), bag), Ok(econ::HELMET_UPGRADE_PRICE));
        assert_eq!(quote(BuyItem::Grenade(GrenadeKind::Smoke), &carried, None, bag), Ok(300));
        let one_flash = GrenadeBag { flash: 1, ..bag };
        assert_eq!(quote(BuyItem::Grenade(GrenadeKind::Flash), &carried, None, one_flash), Ok(200), "a second flashbang");
        assert_eq!(quote(BuyItem::Grenade(GrenadeKind::Flash), &carried, None, GrenadeBag { flash: 2, ..bag }), Err("full"));
    }

    use bevy::ecs::message::Messages;
    use bevy::ecs::system::RunSystemOnce;

    fn wallet_after_loss() -> u32 {
        let mut w = Wallet { money: 0, loss_streak: 0 };
        w.round_end(false);
        w.money
    }

    /// A world with what `pay_round` / `track_kills` read, and a player (`dead` or not).
    fn econ_world(dead: bool) -> (World, Entity) {
        let mut world = World::new();
        world.init_resource::<MatchFlow>();
        world.init_resource::<RoundPhase>();
        world.init_resource::<PlayerWallet>();
        world.init_resource::<RoundLoadout>();
        world.init_resource::<BuyMenu>();
        world.init_resource::<PlayerGrenades>();
        world.init_resource::<crate::bomb::Bomb>();
        world.init_resource::<Messages<Damage>>();
        world.insert_resource(SquadEconomy { wallets: vec![Wallet { money: 0, loss_streak: 0 }; 2], ..default() });
        let mut hp = Health::new(100.0);
        if dead {
            hp.hp = 0.0;
        }
        let p = world.spawn((Player, hp, RoundLoadout::default().arsenal(), Weapon::new(GunState::full(WeaponKind::Pistol)))).id();
        (world, p)
    }

    fn end_round(world: &mut World, player_won: bool) {
        world.resource_mut::<MatchFlow>().round_over = Some((2.5, player_won));
        world.run_system_once(pay_round).unwrap();
    }

    #[test]
    fn squad_kill_reward_is_paid_when_the_player_died_even_if_he_won() {
        // Arrange: he planted, died to member 1's SMG, and the bomb went off (he won).
        let (mut world, _) = econ_world(true);
        world.resource_mut::<SquadEconomy>().last_hitter = Some((1, DamageCause::Gun(WeaponKind::Smg)));
        // Act
        end_round(&mut world, true);
        // Assert
        let s = world.resource::<SquadEconomy>();
        assert_eq!(s.wallets[1].money, wallet_after_loss() + WeaponKind::Smg.kill_reward());
        assert_eq!(s.wallets[0].money, wallet_after_loss());
        assert_eq!(s.last_hitter, None);
    }

    #[test]
    fn squad_kill_reward_is_not_paid_when_the_player_survived_a_loss() {
        // Arrange: hit by member 0, then the clock ran out (squad won, he lived).
        let (mut world, _) = econ_world(false);
        world.resource_mut::<SquadEconomy>().last_hitter = Some((0, DamageCause::Gun(WeaponKind::Rifle)));
        // Act
        end_round(&mut world, false);
        // Assert
        let s = world.resource::<SquadEconomy>();
        let mut w = Wallet { money: 0, loss_streak: 0 };
        w.round_end(true);
        assert_eq!(s.wallets[0].money, w.money, "round pay only");
        assert_eq!(s.last_hitter, None);
    }

    #[test]
    fn surviving_members_keep_their_grenades() {
        // Arrange
        let mut bags = vec![GrenadeBag { frag: 1, flash: 1, smoke: 0 }, GrenadeBag { frag: 0, flash: 1, smoke: 0 }];
        // Act: member 1 throws a frag it doesn't carry (pooled budget), then a flash.
        take_from_bags(&mut bags, 1, GrenadeKind::Frag);
        take_from_bags(&mut bags, 1, GrenadeKind::Flash);
        // Assert
        assert_eq!(bags[0], GrenadeBag { frag: 0, flash: 1, smoke: 0 });
        assert_eq!(bags[1], GrenadeBag::default());
        // A kept bag is the start of the next buy.
        let (bag, cost) = econ::member_grenades(BuyMode::Eco, 5_000, bags[0]);
        assert_eq!((bag, cost), (bags[0], 0));
    }

    fn kill(world: &mut World, p: Entity, cause: DamageCause) -> Entity {
        let e = world.spawn((Enemy { slot: 0 }, Health { hp: 0.0, max: 100.0 })).id();
        world.write_message(Damage { target: e, amount: 100.0, source: p, headshot: false, cause });
        e
    }

    #[test]
    fn player_kills_pay_by_what_landed_them() {
        // Arrange: the pistol is in hand, but one kill was a shotgun's and one a frag.
        let (mut world, p) = econ_world(false);
        kill(&mut world, p, DamageCause::Gun(WeaponKind::Shotgun));
        kill(&mut world, p, DamageCause::Grenade);
        let track = world.register_system(track_kills);
        // Act
        world.run_system(track).unwrap();
        world.run_system(track).unwrap();
        // Assert: paid once each.
        assert_eq!(world.resource::<PlayerWallet>().0.money, Wallet::default().money + WeaponKind::Shotgun.kill_reward() + UTILITY_KILL_REWARD);
    }

    #[test]
    fn kills_during_the_round_over_pause_pay_nothing() {
        // Arrange: the round ended on an earlier frame.
        let (mut world, p) = econ_world(false);
        let track = world.register_system(track_kills);
        world.resource_mut::<MatchFlow>().round_over = Some((2.0, true));
        world.run_system(track).unwrap();
        // Act
        kill(&mut world, p, DamageCause::Gun(WeaponKind::Rifle));
        world.run_system(track).unwrap();
        // Assert
        assert_eq!(world.resource::<PlayerWallet>().0.money, Wallet::default().money);
    }

    #[test]
    fn the_round_winning_kill_still_pays() {
        // Arrange: the kill lands on the frame the round ends.
        let (mut world, p) = econ_world(false);
        let track = world.register_system(track_kills);
        world.run_system(track).unwrap();
        kill(&mut world, p, DamageCause::Gun(WeaponKind::Rifle));
        world.resource_mut::<MatchFlow>().round_over = Some((2.5, true));
        // Act
        world.run_system(track).unwrap();
        // Assert
        assert_eq!(world.resource::<PlayerWallet>().0.money, Wallet::default().money + WeaponKind::Rifle.kill_reward());
    }

    #[test]
    fn kill_rewards_by_cause() {
        assert_eq!(kill_reward(true, DamageCause::Gun(WeaponKind::Smg)), Some(WeaponKind::Smg.kill_reward()));
        assert_eq!(kill_reward(true, DamageCause::Grenade), Some(UTILITY_KILL_REWARD));
        assert_eq!(kill_reward(false, DamageCause::Bomb), Some(UTILITY_KILL_REWARD), "his bomb");
        assert_eq!(kill_reward(false, DamageCause::Grenade), None, "the squad's own frag");
        assert_eq!(kill_reward(false, DamageCause::Gun(WeaponKind::Rifle)), None);
    }

    #[test]
    fn buying_a_second_gun_keeps_the_first_guns_magazine() {
        // Arrange: pistol in hand with 3 rounds left.
        let mut world = World::new();
        world.insert_resource(PlayerWallet(Wallet { money: 10_000, loss_streak: 0 }));
        world.init_resource::<PlayerGrenades>();
        world.init_resource::<Messages<GunEvent>>();
        let mut w = Weapon::new(GunState::full(WeaponKind::Pistol));
        w.ammo = 3;
        let p = world.spawn((Player, RoundLoadout::default().arsenal(), w)).id();
        // Act
        buy(&mut world, BuyItem::Gun(WeaponKind::Rifle), true).unwrap();
        // Assert
        let a = world.get::<Arsenal>(p).unwrap();
        assert_eq!(a.active, 0);
        assert_eq!(a.slots[1].map(|g| g.ammo), Some(3), "the pistol keeps its magazine");
        assert_eq!(world.get::<Weapon>(p).unwrap().kind, WeaponKind::Rifle);
    }

    #[test]
    fn round_loadout_arsenal() {
        let a = RoundLoadout::default().arsenal();
        assert_eq!(a.slots[0], None);
        assert_eq!(a.active, 1);
        let a = RoundLoadout { primary: Some(WeaponKind::Rifle), ..default() }.arsenal();
        assert_eq!(a.active, 0);
        assert_eq!(a.slots[0].map(|g| g.kind), Some(WeaponKind::Rifle));
    }
}
