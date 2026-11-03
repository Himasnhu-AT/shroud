//! Match economy (CS-style): money per side, round rewards, the buy menu's price rules, body
//! armor, and how the enemy squad decides what to buy. Pure logic shared by the game (player
//! wallet, buy menu, squad wallets) and its tests; no engine types.
//!
//! - A match is first to [`ROUNDS_TO_WIN`] rounds, so at most [`MAX_ROUNDS`].
//! - Everyone starts a match with [`START_MONEY`]. A round win pays [`WIN_REWARD`]; a loss pays
//!   [`loss_bonus`] (grows with the losing streak, capped). Kills pay the gun's
//!   `WeaponKind::kill_reward`. Money never passes [`MAX_MONEY`].
//! - Guns cost `WeaponKind::price`; armor costs [`KEVLAR_PRICE`] / [`KEVLAR_HELMET_PRICE`].
//! - The squad buys with the same rules ([`team_mode`], [`member_buy`]). *Which* gun a member
//!   buys among those it can afford follows its evolved loadout genes (technique), so the economy
//!   decides how much they can spend, never how well they fight.

use serde::{Deserialize, Serialize};

use crate::grenade::GrenadeKind;
use crate::weapons::{Slot, WeaponKind};

/// Rounds needed to win a match.
pub const ROUNDS_TO_WIN: u32 = 3;
/// Longest possible match (one side reaches [`ROUNDS_TO_WIN`] by then).
pub const MAX_ROUNDS: u32 = ROUNDS_TO_WIN * 2 - 1;

pub const START_MONEY: u32 = 800;
pub const MAX_MONEY: u32 = 16_000;
pub const WIN_REWARD: u32 = 3_250;
/// First loss in a row.
pub const LOSS_BASE: u32 = 1_400;
/// Added per further loss in a row.
pub const LOSS_STEP: u32 = 500;
pub const LOSS_MAX: u32 = 3_400;

pub const KEVLAR_PRICE: u32 = 650;
pub const KEVLAR_HELMET_PRICE: u32 = 1_000;
/// Helmet alone, when the vest is already full.
pub const HELMET_UPGRADE_PRICE: u32 = 350;
/// Armor points of a fresh vest.
pub const ARMOR_FULL: f32 = 100.0;
/// Share of a hit the vest takes (the rest goes to health).
pub const ABSORB: f32 = 0.5;
/// Share with a helmet on (it also covers headshots; without one a headshot skips the vest).
pub const ABSORB_HELMET: f32 = 0.6;

/// Loss bonus after `streak` losses in a row (0 = the last round was won).
pub fn loss_bonus(streak: u32) -> u32 {
    if streak == 0 { 0 } else { (LOSS_BASE + LOSS_STEP * (streak - 1)).min(LOSS_MAX) }
}

/// One side's (or one enemy's) money.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    pub money: u32,
    /// Rounds lost in a row.
    pub loss_streak: u32,
}

impl Default for Wallet {
    fn default() -> Self {
        Wallet { money: START_MONEY, loss_streak: 0 }
    }
}

impl Wallet {
    /// Add money up to the cap; returns what was actually added.
    pub fn earn(&mut self, amount: u32) -> u32 {
        let before = self.money;
        self.money = (self.money + amount).min(MAX_MONEY);
        self.money - before
    }

    /// End-of-round pay; returns what was added.
    pub fn round_end(&mut self, won: bool) -> u32 {
        if won {
            self.loss_streak = 0;
            self.earn(WIN_REWARD)
        } else {
            self.loss_streak += 1;
            self.earn(loss_bonus(self.loss_streak))
        }
    }

    /// Kill reward for a kill with `kind`; returns what was added.
    pub fn kill(&mut self, kind: WeaponKind) -> u32 {
        self.earn(kind.kill_reward())
    }

    pub fn can_afford(&self, cost: u32) -> bool {
        self.money >= cost
    }

    /// Pay `cost` if there is enough; false (and nothing spent) otherwise.
    pub fn spend(&mut self, cost: u32) -> bool {
        if self.money < cost {
            return false;
        }
        self.money -= cost;
        true
    }
}

/// Buy-menu columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuyCategory {
    Pistols,
    SmgsShotguns,
    RiflesSnipers,
    Gear,
    Grenades,
}

impl BuyCategory {
    pub const ALL: [BuyCategory; 5] =
        [BuyCategory::Pistols, BuyCategory::SmgsShotguns, BuyCategory::RiflesSnipers, BuyCategory::Gear, BuyCategory::Grenades];

    pub fn label(self) -> &'static str {
        match self {
            BuyCategory::Pistols => "Pistols",
            BuyCategory::SmgsShotguns => "SMGs & Shotguns",
            BuyCategory::RiflesSnipers => "Rifles & Snipers",
            BuyCategory::Gear => "Gear",
            BuyCategory::Grenades => "Grenades",
        }
    }
}

/// Which column a gun goes in, from its spec (so new guns sort themselves): sidearms are
/// pistols; pellet guns are shotguns; scoped guns are snipers; otherwise cheap primaries are
/// SMGs and the rest rifles.
pub fn category(kind: WeaponKind) -> BuyCategory {
    let s = kind.spec();
    if s.slot == Slot::Secondary {
        BuyCategory::Pistols
    } else if s.pellets > 1 {
        BuyCategory::SmgsShotguns
    } else if s.ads_zoom < 0.5 {
        BuyCategory::RiflesSnipers
    } else if kind.price() < 1_700 {
        BuyCategory::SmgsShotguns
    } else {
        BuyCategory::RiflesSnipers
    }
}

/// Guns in a buy column, cheapest first.
pub fn guns_in(cat: BuyCategory) -> Vec<WeaponKind> {
    let mut v: Vec<WeaponKind> = WeaponKind::ALL.iter().copied().filter(|&k| category(k) == cat).collect();
    v.sort_by_key(|k| (k.price(), *k));
    v
}

/// Armor items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gear {
    Kevlar,
    KevlarHelmet,
}

impl Gear {
    pub const ALL: [Gear; 2] = [Gear::Kevlar, Gear::KevlarHelmet];

    pub fn label(self) -> &'static str {
        match self {
            Gear::Kevlar => "Kevlar vest",
            Gear::KevlarHelmet => "Kevlar + helmet",
        }
    }

    /// What it costs someone wearing `armor` points (and maybe a helmet); `None` = nothing to
    /// gain (already owned).
    pub fn price(self, armor: f32, helmet: bool) -> Option<u32> {
        let full = armor >= ARMOR_FULL;
        match self {
            Gear::Kevlar if full => None,
            Gear::Kevlar => Some(KEVLAR_PRICE),
            Gear::KevlarHelmet if full && helmet => None,
            Gear::KevlarHelmet if full => Some(HELMET_UPGRADE_PRICE),
            Gear::KevlarHelmet => Some(KEVLAR_HELMET_PRICE),
        }
    }
}

pub const FRAG_PRICE: u32 = 300;
pub const FLASH_PRICE: u32 = 200;
pub const SMOKE_PRICE: u32 = 300;

/// What a grenade costs.
pub fn grenade_price(kind: GrenadeKind) -> u32 {
    match kind {
        GrenadeKind::Frag => FRAG_PRICE,
        GrenadeKind::Flash => FLASH_PRICE,
        GrenadeKind::Smoke => SMOKE_PRICE,
    }
}

/// How many of a grenade one person may carry.
pub fn grenade_max(kind: GrenadeKind) -> u8 {
    match kind {
        GrenadeKind::Flash => 2,
        GrenadeKind::Frag | GrenadeKind::Smoke => 1,
    }
}

/// The grenades someone carries (bought, kept by surviving like guns).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrenadeBag {
    pub frag: u8,
    pub flash: u8,
    pub smoke: u8,
}

impl GrenadeBag {
    pub fn count(&self, kind: GrenadeKind) -> u8 {
        match kind {
            GrenadeKind::Frag => self.frag,
            GrenadeKind::Flash => self.flash,
            GrenadeKind::Smoke => self.smoke,
        }
    }

    fn slot(&mut self, kind: GrenadeKind) -> &mut u8 {
        match kind {
            GrenadeKind::Frag => &mut self.frag,
            GrenadeKind::Flash => &mut self.flash,
            GrenadeKind::Smoke => &mut self.smoke,
        }
    }

    pub fn total(&self) -> u8 {
        self.frag + self.flash + self.smoke
    }

    /// Room for one more of `kind`?
    pub fn can_add(&self, kind: GrenadeKind) -> bool {
        self.count(kind) < grenade_max(kind)
    }

    /// Add one if there is room.
    pub fn add(&mut self, kind: GrenadeKind) -> bool {
        let ok = self.can_add(kind);
        if ok {
            *self.slot(kind) += 1;
        }
        ok
    }

    /// Use one up (thrown); false if there was none.
    pub fn take(&mut self, kind: GrenadeKind) -> bool {
        let n = self.slot(kind);
        if *n == 0 {
            return false;
        }
        *n -= 1;
        true
    }

    /// The next grenade carried after `current` in frag → flash → smoke order (wrapping), for
    /// the cycle key; the first carried one when `current` is `None`.
    pub fn next_after(&self, current: Option<GrenadeKind>) -> Option<GrenadeKind> {
        let all = GrenadeKind::ALL;
        let start = current.and_then(|c| all.iter().position(|&k| k == c)).map_or(0, |i| i + 1);
        (0..all.len()).map(|k| all[(start + k) % all.len()]).find(|&k| self.count(k) > 0)
    }

    /// What buying `kind` costs: `Err` when the bag is full of it.
    pub fn quote(&self, kind: GrenadeKind) -> Result<u32, &'static str> {
        if self.can_add(kind) { Ok(grenade_price(kind)) } else { Err("full") }
    }
}

/// Grenades a squad member buys with what is left after its gun and armor: a flashbang first
/// (cheapest, sets up pushes), then a frag, on force and full buys only. Returns the bag and
/// its cost.
pub fn member_grenades(mode: BuyMode, left: u32, kept: GrenadeBag) -> (GrenadeBag, u32) {
    let mut bag = kept;
    let mut cost = 0;
    if mode == BuyMode::Eco {
        return (bag, 0);
    }
    for kind in [GrenadeKind::Flash, GrenadeKind::Frag] {
        let p = grenade_price(kind);
        if left >= cost + p && bag.add(kind) {
            cost += p;
        }
    }
    (bag, cost)
}

/// A squad member with at least this much buys its grenades before its gun (a flash and a frag
/// still leave a force-buy SMG); below it, grenades come out of what the gun and armor leave.
pub const UTILITY_FIRST_AT: u32 = 1_800;

pub fn utility_first(money: u32) -> bool {
    money >= UTILITY_FIRST_AT
}

/// Split a hit of `amount` between health and a vest of `points`: returns
/// (health damage, armor points used). Headshots skip the vest unless there is a helmet.
pub fn armor_split(amount: f32, points: f32, headshot: bool, helmet: bool) -> (f32, f32) {
    if points <= 0.0 || amount <= 0.0 || (headshot && !helmet) {
        return (amount.max(0.0), 0.0);
    }
    let share = if helmet { ABSORB_HELMET } else { ABSORB };
    let used = (amount * share).min(points);
    (amount - used, used)
}

/// How the squad spends this round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuyMode {
    /// Save: keep what survived, buy nothing.
    Eco,
    /// Buy the best each member can afford right now.
    Force,
    /// Everyone can afford a real gun and armor.
    Full,
}

impl BuyMode {
    pub fn label(self) -> &'static str {
        match self {
            BuyMode::Eco => "eco",
            BuyMode::Force => "force buy",
            BuyMode::Full => "full buy",
        }
    }
}

/// Average money under this: save (unless the round must be won or it's the pistol round).
pub const ECO_BELOW: u32 = 2_000;
/// Average money at or above this: full buy (a rifle plus a vest and helmet).
pub const FULL_BUY_AT: u32 = 3_900;

/// The squad's buy for the round. `pistol_round`: the first round of the match (everyone has
/// the start money; spend it). `must_win`: losing this round loses the match.
pub fn team_mode(wallets: &[Wallet], pistol_round: bool, must_win: bool) -> BuyMode {
    if wallets.is_empty() {
        return BuyMode::Eco;
    }
    let avg = wallets.iter().map(|w| w.money as u64).sum::<u64>() / wallets.len() as u64;
    let avg = avg as u32;
    if avg >= FULL_BUY_AT {
        BuyMode::Full
    } else if pistol_round || must_win || avg >= ECO_BELOW {
        BuyMode::Force
    } else {
        BuyMode::Eco
    }
}

/// What one squad member buys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MemberBuy {
    /// Primary carried this round (`None` = sidearm only).
    pub primary: Option<WeaponKind>,
    pub armor: bool,
    pub helmet: bool,
    /// Money spent.
    pub cost: u32,
}

/// One member's buy. `kept`: the primary it survived last round with (free). `ranked`: the
/// primaries it may carry, best first by its loadout genes (the first is the gun the squad's
/// gene draft gave it). Deterministic.
pub fn member_buy(mode: BuyMode, money: u32, kept: Option<WeaponKind>, kept_armor: bool, ranked: &[WeaponKind]) -> MemberBuy {
    let mut b = MemberBuy { primary: kept, armor: kept_armor, ..Default::default() };
    if mode == BuyMode::Eco {
        return b;
    }
    let mut left = money;
    if b.primary.is_none() {
        // Full buy: keep enough for a vest when possible; force: anything affordable.
        let reserve = if mode == BuyMode::Full { KEVLAR_PRICE } else { 0 };
        let pick = ranked.iter().copied().find(|k| k.price() + reserve <= left).or_else(|| ranked.iter().copied().find(|k| k.price() <= left));
        if let Some(k) = pick {
            left -= k.price();
            b.cost += k.price();
            b.primary = Some(k);
        }
    }
    if !b.armor {
        if left >= KEVLAR_HELMET_PRICE {
            b.armor = true;
            b.helmet = true;
            b.cost += KEVLAR_HELMET_PRICE;
        } else if left >= KEVLAR_PRICE {
            b.armor = true;
            b.cost += KEVLAR_PRICE;
        }
    }
    b
}

/// A member's primaries best first: the drafted gun, then the rest of `allowed` by gene
/// strength (`prefs` lines up with `WeaponKind::PRIMARIES`), table order on ties.
pub fn rank_primaries(drafted: WeaponKind, prefs: &[f32], allowed: &[WeaponKind]) -> Vec<WeaponKind> {
    let gene = |k: WeaponKind| WeaponKind::PRIMARIES.iter().position(|&p| p == k).and_then(|i| prefs.get(i)).copied().unwrap_or(0.0);
    let mut rest: Vec<WeaponKind> = allowed.iter().copied().filter(|&k| k != drafted && k.spec().slot == Slot::Primary).collect();
    rest.sort_by(|a, b| gene(*b).total_cmp(&gene(*a)).then(a.cmp(b)));
    let mut out = Vec::with_capacity(rest.len() + 1);
    if drafted.spec().slot == Slot::Primary {
        out.push(drafted);
    }
    out.extend(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_length() {
        assert_eq!(ROUNDS_TO_WIN, 3);
        assert_eq!(MAX_ROUNDS, 5);
    }

    #[test]
    fn loss_bonus_grows_and_caps() {
        assert_eq!(loss_bonus(0), 0);
        assert_eq!(loss_bonus(1), 1_400);
        assert_eq!(loss_bonus(2), 1_900);
        assert_eq!(loss_bonus(4), 2_900);
        assert_eq!(loss_bonus(5), 3_400);
        assert_eq!(loss_bonus(9), 3_400);
    }

    #[test]
    fn wallet_rounds() {
        let mut w = Wallet::default();
        assert_eq!(w.money, 800);
        assert_eq!(w.round_end(false), 1_400);
        assert_eq!(w.round_end(false), 1_900);
        assert_eq!(w.money, 800 + 1_400 + 1_900);
        assert_eq!(w.round_end(true), 3_250);
        assert_eq!(w.loss_streak, 0);
        w.round_end(false);
        assert_eq!(w.money, 800 + 1_400 + 1_900 + 3_250 + 1_400, "a win resets the streak");
    }

    #[test]
    fn wallet_caps_and_spends() {
        let mut w = Wallet { money: 15_900, loss_streak: 0 };
        assert_eq!(w.kill(WeaponKind::Rifle), 100);
        assert_eq!(w.money, MAX_MONEY);
        assert!(w.spend(2_900));
        assert_eq!(w.money, 13_100);
        let mut poor = Wallet::default();
        assert!(!poor.spend(1_000));
        assert_eq!(poor.money, 800);
        assert!(poor.spend(650));
        assert_eq!(poor.money, 150);
    }

    #[test]
    fn kill_rewards_follow_the_gun() {
        let mut w = Wallet { money: 0, loss_streak: 0 };
        w.kill(WeaponKind::Smg);
        w.kill(WeaponKind::Marksman);
        assert_eq!(w.money, WeaponKind::Smg.kill_reward() + WeaponKind::Marksman.kill_reward());
    }

    #[test]
    fn every_gun_has_a_column() {
        for k in WeaponKind::ALL {
            let c = category(k);
            assert!(c != BuyCategory::Gear && c != BuyCategory::Grenades);
            assert!(guns_in(c).contains(&k));
        }
        assert_eq!(category(WeaponKind::Pistol), BuyCategory::Pistols);
        assert_eq!(category(WeaponKind::Smg), BuyCategory::SmgsShotguns);
        assert_eq!(category(WeaponKind::Shotgun), BuyCategory::SmgsShotguns);
        assert_eq!(category(WeaponKind::Rifle), BuyCategory::RiflesSnipers);
        assert_eq!(category(WeaponKind::Marksman), BuyCategory::RiflesSnipers);
        assert_eq!(category(WeaponKind::HeavyPistol), BuyCategory::Pistols);
        assert_eq!(category(WeaponKind::BattleRifle), BuyCategory::RiflesSnipers);
        assert_eq!(category(WeaponKind::Sniper), BuyCategory::RiflesSnipers);
    }

    #[test]
    fn gear_prices() {
        assert_eq!(Gear::Kevlar.price(0.0, false), Some(650));
        assert_eq!(Gear::KevlarHelmet.price(0.0, false), Some(1_000));
        assert_eq!(Gear::Kevlar.price(100.0, false), None);
        assert_eq!(Gear::KevlarHelmet.price(100.0, false), Some(350));
        assert_eq!(Gear::KevlarHelmet.price(100.0, true), None);
        assert_eq!(Gear::Kevlar.price(40.0, true), Some(650), "a damaged vest can be refilled");
    }

    #[test]
    fn grenade_limits_and_prices() {
        assert_eq!((grenade_price(GrenadeKind::Frag), grenade_price(GrenadeKind::Flash), grenade_price(GrenadeKind::Smoke)), (300, 200, 300));
        let mut b = GrenadeBag::default();
        assert!(b.add(GrenadeKind::Frag));
        assert!(!b.add(GrenadeKind::Frag), "one frag");
        assert_eq!(b.quote(GrenadeKind::Frag), Err("full"));
        assert!(b.add(GrenadeKind::Flash) && b.add(GrenadeKind::Flash));
        assert!(!b.add(GrenadeKind::Flash), "two flashbangs");
        assert!(b.add(GrenadeKind::Smoke));
        assert!(!b.add(GrenadeKind::Smoke), "one smoke");
        assert_eq!(b, GrenadeBag { frag: 1, flash: 2, smoke: 1 });
        assert_eq!(b.total(), 4);
        // Cycling: frag → flash → smoke → frag, skipping what isn't carried.
        assert_eq!(b.next_after(None), Some(GrenadeKind::Frag));
        assert_eq!(b.next_after(Some(GrenadeKind::Smoke)), Some(GrenadeKind::Frag));
        assert!(b.take(GrenadeKind::Frag));
        assert!(!b.take(GrenadeKind::Frag));
        assert_eq!(b.next_after(Some(GrenadeKind::Smoke)), Some(GrenadeKind::Flash));
        assert_eq!(b.quote(GrenadeKind::Frag), Ok(300));
        assert_eq!(GrenadeBag::default().next_after(None), None);
    }

    #[test]
    fn squad_members_buy_grenades_with_what_is_left() {
        let none = GrenadeBag::default();
        assert_eq!(member_grenades(BuyMode::Eco, 5_000, none), (none, 0));
        assert_eq!(member_grenades(BuyMode::Full, 600, none), (GrenadeBag { flash: 1, frag: 1, smoke: 0 }, 500));
        assert_eq!(member_grenades(BuyMode::Force, 250, none), (GrenadeBag { flash: 1, ..none }, 200));
        assert_eq!(member_grenades(BuyMode::Force, 100, none), (none, 0));
        assert!(utility_first(2_200) && !utility_first(800), "force rounds keep utility, pistol rounds don't");
        // A kept frag is not bought twice.
        let kept = GrenadeBag { frag: 1, ..none };
        assert_eq!(member_grenades(BuyMode::Full, 1_000, kept), (GrenadeBag { frag: 1, flash: 1, smoke: 0 }, 200));
    }

    #[test]
    fn armor_soaks_body_hits_and_helmets_cover_heads() {
        assert_eq!(armor_split(40.0, 100.0, false, false), (20.0, 20.0));
        assert_eq!(armor_split(40.0, 100.0, true, false), (40.0, 0.0), "no helmet: headshots skip the vest");
        let (hp, used) = armor_split(40.0, 100.0, true, true);
        assert!((hp - 16.0).abs() < 1e-4 && (used - 24.0).abs() < 1e-4);
        assert_eq!(armor_split(40.0, 5.0, false, false), (35.0, 5.0), "a worn vest gives what it has");
        assert_eq!(armor_split(40.0, 0.0, false, true), (40.0, 0.0));
    }

    #[test]
    fn squad_saves_when_poor_and_buys_when_rich() {
        let w = |m| Wallet { money: m, loss_streak: 1 };
        assert_eq!(team_mode(&[w(800); 4], true, false), BuyMode::Force, "pistol round spends");
        assert_eq!(team_mode(&[w(1_500); 4], false, false), BuyMode::Eco);
        assert_eq!(team_mode(&[w(1_500); 4], false, true), BuyMode::Force, "must-win rounds force");
        assert_eq!(team_mode(&[w(2_500); 4], false, false), BuyMode::Force);
        assert_eq!(team_mode(&[w(5_000); 4], false, false), BuyMode::Full);
        assert_eq!(team_mode(&[], false, false), BuyMode::Eco);
    }

    #[test]
    fn member_buys_from_its_genes_within_budget() {
        let ranked = [WeaponKind::Sniper, WeaponKind::Rifle, WeaponKind::Smg];
        // Rich: the drafted gun (its genes' choice) and armor.
        let b = member_buy(BuyMode::Full, 7_000, None, false, &ranked);
        assert_eq!(b.primary, Some(WeaponKind::Sniper));
        assert!(b.armor && b.helmet);
        assert_eq!(b.cost, 4_750 + 1_000);
        // Can't afford the sniper: next preference that fits, with a vest.
        let b = member_buy(BuyMode::Full, 4_000, None, false, &ranked);
        assert_eq!(b.primary, Some(WeaponKind::Rifle));
        assert_eq!((b.armor, b.helmet, b.cost), (true, true, 3_900));
        // Force with little money: the SMG.
        let b = member_buy(BuyMode::Force, 1_400, None, false, &ranked);
        assert_eq!(b.primary, Some(WeaponKind::Smg));
        assert!(!b.armor);
        // Eco: nothing bought, survivors keep what they had.
        let b = member_buy(BuyMode::Eco, 1_900, Some(WeaponKind::Rifle), true, &ranked);
        assert_eq!(b, MemberBuy { primary: Some(WeaponKind::Rifle), armor: true, helmet: false, cost: 0 });
        // Pistol round: a vest.
        let b = member_buy(BuyMode::Force, 800, None, false, &ranked);
        assert_eq!((b.primary, b.armor, b.cost), (None, true, 650));
        // Deterministic.
        assert_eq!(member_buy(BuyMode::Full, 4_000, None, false, &ranked), member_buy(BuyMode::Full, 4_000, None, false, &ranked));
    }

    #[test]
    fn ranking_puts_the_draft_first_then_genes() {
        let prefs = [0.9, 0.1, 0.5, 0.2, 0.0, 0.7]; // smg, rifle, shotgun, marksman, battle rifle, sniper
        let r = rank_primaries(WeaponKind::Rifle, &prefs, &WeaponKind::PRIMARIES);
        assert_eq!(r, vec![WeaponKind::Rifle, WeaponKind::Smg, WeaponKind::Sniper, WeaponKind::Shotgun, WeaponKind::Marksman, WeaponKind::BattleRifle]);
        // Four-gene preferences (old saves before migration): the new guns rank last.
        let r = rank_primaries(WeaponKind::Rifle, &prefs[..4], &WeaponKind::PRIMARIES);
        assert_eq!(r[4..], [WeaponKind::BattleRifle, WeaponKind::Sniper]);
        let r = rank_primaries(WeaponKind::Rifle, &prefs, &[WeaponKind::Rifle, WeaponKind::Smg]);
        assert_eq!(r, vec![WeaponKind::Rifle, WeaponKind::Smg]);
    }
}
