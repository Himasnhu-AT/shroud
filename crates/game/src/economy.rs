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
use brain::economy::{self as econ, BuyMode, Gear, Wallet};
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
use crate::weapon::{Arsenal, Damage, GunEvent, Health, WeaponSet};

/// Freeze time at round start in a normal run (s).
pub const FREEZE_S: f32 = 8.0;
/// Buying stays open this long after freeze time ends, inside the buy zone (s).
pub const BUY_TIME_S: f32 = 20.0;
/// Buy zone radius around the player spawn (m).
pub const BUY_ZONE_M: f32 = 8.0;

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
}

impl Default for RoundLoadout {
    fn default() -> Self {
        RoundLoadout { primary: None, secondary: WeaponKind::Pistol, armor: None }
    }
}

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
    /// This round's buy.
    pub mode: Option<BuyMode>,
    /// The enemy that last hit the player this round (slot, gun): paid if the player dies.
    last_hitter: Option<(usize, WeaponKind)>,
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
}

impl BuyItem {
    pub fn label(self) -> &'static str {
        match self {
            BuyItem::Gun(k) => k.label(),
            BuyItem::Gear(g) => g.label(),
        }
    }

    pub fn category(self) -> econ::BuyCategory {
        match self {
            BuyItem::Gun(k) => econ::category(k),
            BuyItem::Gear(_) => econ::BuyCategory::Gear,
        }
    }
}

/// Every buy-menu item in column order: guns from `WeaponKind::ALL` (so new guns appear by
/// themselves), then gear.
pub fn catalog() -> Vec<BuyItem> {
    let mut v = Vec::new();
    for cat in econ::BuyCategory::ALL {
        if cat == econ::BuyCategory::Gear {
            v.extend(Gear::ALL.iter().map(|g| BuyItem::Gear(*g)));
        } else {
            v.extend(econ::guns_in(cat).into_iter().map(BuyItem::Gun));
        }
    }
    v
}

/// What an item costs the player now: `Err` says why it can't be bought ("owned").
pub fn quote(item: BuyItem, carried: &[WeaponKind], armor: Option<Armor>) -> Result<u32, &'static str> {
    match item {
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
            .init_resource::<SquadEconomy>()
            .init_resource::<BuyMenu>()
            .add_systems(Startup, test_runs_skip_freeze)
            .add_systems(OnEnter(GameState::Playing), (start_round.after(begin_round), squad_buy.after(enemy::spawn_squad).after(start_round)))
            .add_systems(OnExit(GameState::Playing), close_menu)
            // The AI holds during freeze time.
            .configure_sets(FixedUpdate, EnemySet.run_if(round_live))
            .add_systems(
                Update,
                (
                    tick_phase.run_if(not_paused),
                    hold,
                    (track_hits, pay_kills, pay_round.after(check_round_end)).chain().after(WeaponSet),
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

/// Freeze time: no moving, jumping or firing (looking and buying are fine).
fn hold_player(phase: Res<RoundPhase>, mut intent: ResMut<PlayerIntent>, mut inputs: Query<&mut engine::fps::FpsInput, With<Player>>) {
    if !phase.frozen() {
        return;
    }
    if intent.fire {
        intent.fire = false;
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
    if phase.frozen() {
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
    let price = quote(item, &carried, armor)?;
    if !world.resource_mut::<PlayerWallet>().0.spend(price) {
        return Err(format!("can't afford {} (${price})", item.label()));
    }
    let e = world.query_filtered::<Entity, With<Player>>().iter(world).next().ok_or("no player")?;
    match item {
        BuyItem::Gun(kind) => {
            let slot = if kind.spec().slot == Slot::Primary { 0 } else { 1 };
            let gun = GunState::full(kind);
            let mut ent = world.entity_mut(e);
            if let Some(mut a) = ent.get_mut::<Arsenal>() {
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
    }
    let left = world.resource::<PlayerWallet>().0.money;
    crate::dev::trace(world, "buy", &[("price", Prop::Int(price as i64)), ("money", Prop::Int(left as i64))]);
    info!("bought {} for ${price}, ${left} left", item.label());
    Ok(left)
}

/// Remember the enemy that last hit the player (it gets the kill reward if the player dies).
fn track_hits(
    mut damage: MessageReader<Damage>,
    players: Query<(), With<Player>>,
    enemies: Query<(&Enemy, &Armed)>,
    mut squad: ResMut<SquadEconomy>,
) {
    for d in damage.read() {
        if players.contains(d.target)
            && let Ok((e, armed)) = enemies.get(d.source)
        {
            squad.last_hitter = Some((e.slot, armed.0));
        }
    }
}

/// Kill rewards for the player: the gun in hand when an enemy goes down.
fn pay_kills(
    flow: Res<MatchFlow>,
    enemies: Query<(Entity, &EnemyCombat), With<Enemy>>,
    player: Query<&Armed, With<Player>>,
    mut wallet: ResMut<PlayerWallet>,
    mut paid: Local<(u32, u32, Vec<Entity>)>,
) {
    let key = (flow.match_no, flow.round);
    if (paid.0, paid.1) != key {
        *paid = (key.0, key.1, Vec::new());
    }
    let kind = player.single().map(|a| a.0).unwrap_or(WeaponKind::Pistol);
    for (e, c) in &enemies {
        if c.dead && !paid.2.contains(&e) {
            paid.2.push(e);
            wallet.0.kill(kind);
        }
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
    let gained = wallet.0.round_end(won);
    // Player: survive = keep the kit (ammo is refilled next round); die = pistol only.
    *loadout = match player.single() {
        Ok((hp, a, w, armor)) if !hp.is_dead() => {
            let mut slots = a.slots;
            slots[a.active] = Some(w.state());
            RoundLoadout {
                primary: slots[0].map(|g| g.kind),
                secondary: slots[1].map_or(WeaponKind::Pistol, |g| g.kind),
                armor: armor.copied().filter(|a| a.points > 0.0),
            }
        }
        _ => RoundLoadout::default(),
    };
    // Squad: everyone gets the round pay; survivors keep what they carry.
    let n = squad.wallets.len();
    for w in &mut squad.wallets {
        w.round_end(!won);
    }
    if !won && let Some((slot, kind)) = squad.last_hitter.take() && slot < n {
        squad.wallets[slot].kill(kind);
    }
    squad.kept = vec![None; n];
    squad.kept_armor = vec![None; n];
    for (e, c, armor) in &enemies {
        if e.slot < n && !c.dead {
            squad.kept[e.slot] = Some(c.gun.primary).filter(|k| k.spec().slot == Slot::Primary);
            squad.kept_armor[e.slot] = armor.copied().filter(|a| a.points > 0.0);
        }
    }
    dev.trace("econ.round", &[("money", Prop::Int(wallet.0.money as i64)), ("gained", Prop::Int(gained as i64))]);
}

/// After the squad spawns: each member buys with its own wallet; which gun it picks among those
/// it can afford comes from its evolved loadout genes (`brain::economy::member_buy`).
fn squad_buy(
    mut commands: Commands,
    flow: Res<MatchFlow>,
    squad: Res<Squad>,
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
    let must_win = flow.player_rounds + 1 >= ROUNDS_TO_WIN;
    let mode = econ::team_mode(&s.wallets, flow.round <= 1, must_win);
    s.mode = Some(mode);
    // Guns the difficulty lets this squad field (the gene draft's caps).
    let caps = brain::loadout::caps(squad.difficulty, n);
    let allowed: Vec<WeaponKind> =
        WeaponKind::PRIMARIES.iter().enumerate().filter(|(i, _)| caps.get(*i).is_none_or(|c| *c > 0)).map(|(_, k)| *k).collect();
    let mut spent = 0;
    let mut bought = Vec::new();
    let mut list: Vec<_> = enemies.iter_mut().collect();
    list.sort_by_key(|(_, e, ..)| e.slot);
    for (ent, e, mind, c, armed) in list {
        let slot = e.slot;
        let ranked = econ::rank_primaries(mind.0.weapon(), &mind.0.traits.loadout, &allowed);
        let b = econ::member_buy(mode, s.wallets[slot].money, s.kept[slot], s.kept_armor[slot].is_some(), &ranked);
        s.wallets[slot].spend(b.cost);
        spent += b.cost;
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
        bought.push(format!("{}{}", kind.label(), if armor.is_some() { "+armor" } else { "" }));
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
        // Column order.
        let cats: Vec<_> = c.iter().map(|i| i.category()).collect();
        let mut sorted = cats.clone();
        sorted.sort_by_key(|c| econ::BuyCategory::ALL.iter().position(|x| x == c));
        assert_eq!(cats, sorted);
    }

    #[test]
    fn quotes() {
        let carried = [WeaponKind::Pistol];
        assert_eq!(quote(BuyItem::Gun(WeaponKind::Pistol), &carried, None), Err("owned"));
        assert_eq!(quote(BuyItem::Gun(WeaponKind::Rifle), &carried, None), Ok(WeaponKind::Rifle.price()));
        assert_eq!(quote(BuyItem::Gear(Gear::Kevlar), &carried, Some(Armor::vest(false))), Err("owned"));
        assert_eq!(quote(BuyItem::Gear(Gear::KevlarHelmet), &carried, Some(Armor::vest(false))), Ok(econ::HELMET_UPGRADE_PRICE));
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
