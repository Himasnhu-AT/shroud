//! The player's career: XP, ranks, and the 3–6 objectives handed out each match.
//!
//! - Every finished match pays XP: a win, each round won, and each objective completed
//!   ([`match_xp`]). An abandoned match pays nothing.
//! - XP only goes up; enough of it promotes you through [`Rank::ALL`]. Elite is the top.
//! - Each match rolls fresh objectives ([`roll_objectives`]): 3 at Recruit up to 6 at the top
//!   ranks, with targets that grow with rank.
//! - The game records what happens in a [`MatchLog`]; [`Goal::progress`] reads it, so the HUD can
//!   show live progress and the debrief the final tally.
//!
//! Pure rules, no Bevy. The game owns the save file (`Career` is serde).

use crate::weapons::WeaponKind;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

// ---- Ranks ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Rank {
    Recruit,
    Private,
    Corporal,
    Sergeant,
    StaffSergeant,
    Lieutenant,
    Captain,
    Elite,
}

impl Rank {
    pub const ALL: [Rank; 8] =
        [Rank::Recruit, Rank::Private, Rank::Corporal, Rank::Sergeant, Rank::StaffSergeant, Rank::Lieutenant, Rank::Captain, Rank::Elite];

    /// Total XP needed to hold this rank.
    pub fn xp_floor(self) -> u32 {
        match self {
            Rank::Recruit => 0,
            Rank::Private => 1_000,
            Rank::Corporal => 2_500,
            Rank::Sergeant => 4_500,
            Rank::StaffSergeant => 7_000,
            Rank::Lieutenant => 10_000,
            Rank::Captain => 14_000,
            Rank::Elite => 19_000,
        }
    }

    pub fn for_xp(xp: u32) -> Rank {
        Rank::ALL.into_iter().rev().find(|r| xp >= r.xp_floor()).unwrap_or(Rank::Recruit)
    }

    pub fn next(self) -> Option<Rank> {
        Rank::ALL.get(self.index() + 1).copied()
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Rank::Recruit => "Recruit",
            Rank::Private => "Private",
            Rank::Corporal => "Corporal",
            Rank::Sergeant => "Sergeant",
            Rank::StaffSergeant => "Staff Sergeant",
            Rank::Lieutenant => "Lieutenant",
            Rank::Captain => "Captain",
            Rank::Elite => "Elite",
        }
    }

    /// Objectives per match at this rank: 3, 3, 4, 4, 5, 5, 6, 6.
    pub fn objective_count(self) -> usize {
        3 + self.index() / 2
    }

    /// 0 (easy targets) ..= 3 (hard targets).
    pub fn tier(self) -> u32 {
        (self.index() / 2) as u32
    }
}

// ---- XP ------------------------------------------------------------------------------------

pub const XP_MATCH_WIN: u32 = 500;
pub const XP_MATCH_PLAYED: u32 = 100;
pub const XP_ROUND_WIN: u32 = 50;

/// One line on the debrief's XP breakdown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XpLine {
    pub label: String,
    pub xp: u32,
}

/// What a finished match pays: played/won, rounds won, then each completed objective.
pub fn match_xp(won: bool, rounds_won: u32, objectives: &[Objective], log: &MatchLog) -> Vec<XpLine> {
    let mut lines = vec![if won {
        XpLine { label: "Match won".into(), xp: XP_MATCH_WIN }
    } else {
        XpLine { label: "Match played".into(), xp: XP_MATCH_PLAYED }
    }];
    if rounds_won > 0 {
        lines.push(XpLine { label: format!("Rounds won \u{d7}{rounds_won}"), xp: rounds_won * XP_ROUND_WIN });
    }
    for o in objectives.iter().filter(|o| o.done(log)) {
        lines.push(XpLine { label: o.goal.describe(), xp: o.xp });
    }
    lines
}

// ---- Career (saved) ------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Career {
    pub xp: u32,
    #[serde(default)]
    pub matches: u32,
    #[serde(default)]
    pub wins: u32,
    #[serde(default)]
    pub objectives_done: u32,
}

/// The result of crediting a match.
#[derive(Clone, Debug, PartialEq)]
pub struct Award {
    pub lines: Vec<XpLine>,
    pub total: u32,
    pub rank_before: Rank,
    pub rank_after: Rank,
}

impl Award {
    pub fn promoted(&self) -> bool {
        self.rank_after > self.rank_before
    }
}

impl Career {
    pub fn rank(&self) -> Rank {
        Rank::for_xp(self.xp)
    }

    /// (XP into this rank, XP this rank spans); None at Elite.
    pub fn progress(&self) -> Option<(u32, u32)> {
        let r = self.rank();
        r.next().map(|n| (self.xp - r.xp_floor(), n.xp_floor() - r.xp_floor()))
    }

    /// Credit a finished match.
    pub fn credit(&mut self, won: bool, rounds_won: u32, objectives: &[Objective], log: &MatchLog) -> Award {
        let rank_before = self.rank();
        let lines = match_xp(won, rounds_won, objectives, log);
        let total = lines.iter().map(|l| l.xp).sum();
        self.xp = self.xp.saturating_add(total);
        self.matches += 1;
        self.wins += won as u32;
        self.objectives_done += objectives.iter().filter(|o| o.done(log)).count() as u32;
        Award { lines, total, rank_before, rank_after: self.rank() }
    }
}

// ---- What happened in a match --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KillLog {
    pub weapon: WeaponKind,
    pub headshot: bool,
    /// The victim was defusing the bomb.
    pub victim_defusing: bool,
}

/// One round, filled in as it plays. The last round in [`MatchLog::rounds`] may still be live.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoundLog {
    /// None while the round is live.
    pub won: Option<bool>,
    /// Site letter and seconds after the round went live (freeze time excluded).
    pub planted: Option<(char, f32)>,
    pub exploded: bool,
    pub kills: Vec<KillLog>,
    pub damage_taken: f32,
    pub died: bool,
    /// The player held a primary at any point this round.
    pub had_primary: bool,
    /// Squad members alive at round start (for the ace).
    pub squad_size: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchLog {
    pub rounds: Vec<RoundLog>,
}

impl MatchLog {
    /// Start a new round (call at round start).
    pub fn begin_round(&mut self, squad_size: u32) {
        self.rounds.push(RoundLog { squad_size, ..Default::default() });
    }
    /// The live (last) round, if any.
    pub fn current(&mut self) -> Option<&mut RoundLog> {
        self.rounds.last_mut()
    }
    fn kills(&self) -> impl Iterator<Item = &KillLog> {
        self.rounds.iter().flat_map(|r| r.kills.iter())
    }
    fn won_rounds(&self) -> impl Iterator<Item = &RoundLog> {
        self.rounds.iter().filter(|r| r.won == Some(true))
    }
}

// ---- Objectives ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Goal {
    /// Plant the bomb on this site.
    PlantOn(char),
    /// Plant the bomb N times.
    Plants(u32),
    /// Have the bomb go off N times.
    Detonations(u32),
    /// N kills in the match.
    Kills(u32),
    /// N headshot kills.
    Headshots(u32),
    /// N kills with this gun.
    KillsWith(WeaponKind, u32),
    /// N kills in a single round.
    MultiKill(u32),
    /// Kill the whole squad in one round.
    Ace,
    /// Kill a defender while they defuse.
    KillDefuser,
    /// Plant within this many seconds of the round going live.
    FastPlant(u32),
    /// Win a round without taking damage.
    FlawlessRound,
    /// Win a round without a primary.
    PistolWin,
    /// Win N rounds in a row.
    Streak(u32),
}

impl Goal {
    /// Player-facing text.
    pub fn describe(&self) -> String {
        match *self {
            Goal::PlantOn(s) => format!("Plant the bomb on site {s}"),
            Goal::Plants(n) => format!("Plant the bomb {n} times"),
            Goal::Detonations(1) => "Let the bomb explode".into(),
            Goal::Detonations(n) => format!("Let the bomb explode {n} times"),
            Goal::Kills(n) => format!("Get {n} kills"),
            Goal::Headshots(n) => format!("Get {n} headshot kills"),
            Goal::KillsWith(w, n) => format!("Get {n} kills with the {}", weapon_name(w)),
            Goal::MultiKill(n) => format!("Get {n} kills in one round"),
            Goal::Ace => "Ace: kill the whole squad in one round".into(),
            Goal::KillDefuser => "Kill a defender while they defuse".into(),
            Goal::FastPlant(s) => format!("Plant within {s} s of the round starting"),
            Goal::FlawlessRound => "Win a round without taking damage".into(),
            Goal::PistolWin => "Win a round without a primary weapon".into(),
            Goal::Streak(n) => format!("Win {n} rounds in a row"),
        }
    }

    /// (progress, target). Done when progress >= target.
    pub fn progress(&self, log: &MatchLog) -> (u32, u32) {
        let count = |n: usize| n as u32;
        match *self {
            Goal::PlantOn(s) => (count(log.rounds.iter().filter(|r| r.planted.is_some_and(|p| p.0 == s)).count()).min(1), 1),
            Goal::Plants(n) => (count(log.rounds.iter().filter(|r| r.planted.is_some()).count()), n),
            Goal::Detonations(n) => (count(log.rounds.iter().filter(|r| r.exploded).count()), n),
            Goal::Kills(n) => (count(log.kills().count()), n),
            Goal::Headshots(n) => (count(log.kills().filter(|k| k.headshot).count()), n),
            Goal::KillsWith(w, n) => (count(log.kills().filter(|k| k.weapon == w).count()), n),
            Goal::MultiKill(n) => (log.rounds.iter().map(|r| count(r.kills.len())).max().unwrap_or(0), n),
            Goal::Ace => {
                let ace = log.rounds.iter().any(|r| r.squad_size > 0 && count(r.kills.len()) >= r.squad_size);
                (ace as u32, 1)
            }
            Goal::KillDefuser => ((log.kills().any(|k| k.victim_defusing)) as u32, 1),
            Goal::FastPlant(s) => ((log.rounds.iter().any(|r| r.planted.is_some_and(|p| p.1 <= s as f32))) as u32, 1),
            Goal::FlawlessRound => ((log.won_rounds().any(|r| r.damage_taken <= 0.0)) as u32, 1),
            Goal::PistolWin => ((log.won_rounds().any(|r| !r.had_primary)) as u32, 1),
            Goal::Streak(n) => {
                let (mut best, mut run) = (0, 0);
                for r in &log.rounds {
                    match r.won {
                        Some(true) => run += 1,
                        Some(false) => run = 0,
                        None => {}
                    }
                    best = best.max(run);
                }
                (best, n)
            }
        }
    }
}

fn weapon_name(w: WeaponKind) -> &'static str {
    match w {
        WeaponKind::Pistol => "Vector-9",
        WeaponKind::Smg => "Hornet",
        WeaponKind::Rifle => "Kestrel",
        WeaponKind::Shotgun => "Breacher",
        WeaponKind::Marksman => "Longbow",
        WeaponKind::HeavyPistol => "Warden",
        WeaponKind::BattleRifle => "Shrike",
        WeaponKind::Sniper => "Osprey",
    }
}

/// A goal with its XP reward.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    pub goal: Goal,
    pub xp: u32,
}

impl Objective {
    pub fn done(&self, log: &MatchLog) -> bool {
        let (p, t) = self.goal.progress(log);
        p >= t
    }
}

/// Roll this match's objectives: [`Rank::objective_count`] distinct kinds, targets and XP
/// scaled by [`Rank::tier`]. `sites` are the map's site letters. Deterministic per `seed`.
pub fn roll_objectives(rank: Rank, sites: &[char], seed: u64) -> Vec<Objective> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let t = rank.tier();
    let site = if sites.is_empty() { 'A' } else { sites[rng.random_range(0..sites.len())] };
    let gun = [WeaponKind::Pistol, WeaponKind::Rifle, WeaponKind::BattleRifle, WeaponKind::Smg, WeaponKind::Shotgun, WeaponKind::Marksman]
        [rng.random_range(0..6)];
    // (goal, xp, minimum tier). Order is the pool; we shuffle and take the first N.
    let pool: Vec<(Goal, u32, u32)> = vec![
        (Goal::PlantOn(site), 150, 0),
        (Goal::Plants(2 + t), 150 + 50 * t, 0),
        (Goal::Detonations(1 + t / 2), 200 + 50 * t, 0),
        (Goal::Kills(5 + 3 * t), 150 + 50 * t, 0),
        (Goal::Headshots(2 + 2 * t), 200 + 50 * t, 0),
        (Goal::KillsWith(gun, 3 + t), 200 + 25 * t, 0),
        (Goal::MultiKill(2 + t.min(2)), 200 + 75 * t, 0),
        (Goal::PistolWin, 250, 0),
        (Goal::Streak(2 + t.min(1)), 200 + 50 * t, 1),
        (Goal::FastPlant(40 - 6 * t), 250, 1),
        (Goal::KillDefuser, 300, 1),
        (Goal::FlawlessRound, 300, 2),
        (Goal::Ace, 500, 2),
    ];
    let mut avail: Vec<_> = pool.into_iter().filter(|p| p.2 <= t).collect();
    // Fisher-Yates.
    for i in (1..avail.len()).rev() {
        let j = rng.random_range(0..=i);
        avail.swap(i, j);
    }
    // At most two plant goals so a match is not all bomb chores.
    let mut out = Vec::new();
    let mut bomb_goals = 0;
    for (goal, xp, _) in avail {
        let bomb = matches!(goal, Goal::PlantOn(_) | Goal::Plants(_) | Goal::FastPlant(_) | Goal::Detonations(_));
        if bomb && bomb_goals >= 2 {
            continue;
        }
        bomb_goals += bomb as u32;
        out.push(Objective { goal, xp });
        if out.len() == rank.objective_count() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kill(w: WeaponKind, hs: bool) -> KillLog {
        KillLog { weapon: w, headshot: hs, victim_defusing: false }
    }

    #[test]
    fn ranks_climb_with_xp() {
        assert_eq!(Rank::for_xp(0), Rank::Recruit);
        assert_eq!(Rank::for_xp(999), Rank::Recruit);
        assert_eq!(Rank::for_xp(1_000), Rank::Private);
        assert_eq!(Rank::for_xp(1_000_000), Rank::Elite);
        assert_eq!(Rank::Elite.next(), None);
        let c = Career { xp: 1_500, ..Default::default() };
        assert_eq!(c.progress(), Some((500, 1_500)));
    }

    #[test]
    fn objective_count_grows_with_rank() {
        for r in Rank::ALL {
            let o = roll_objectives(r, &['A', 'B'], 7);
            assert_eq!(o.len(), r.objective_count(), "{r:?}");
            assert!((3..=6).contains(&o.len()));
            // Distinct kinds.
            for (i, a) in o.iter().enumerate() {
                for b in &o[i + 1..] {
                    assert_ne!(std::mem::discriminant(&a.goal), std::mem::discriminant(&b.goal));
                }
            }
        }
        assert_eq!(roll_objectives(Rank::Sergeant, &['A', 'B'], 3), roll_objectives(Rank::Sergeant, &['A', 'B'], 3));
        // Never more than two bomb objectives, so every match has some fighting to do.
        for seed in 0..200 {
            let o = roll_objectives(Rank::Recruit, &['A', 'B'], seed);
            let bomb = o.iter().filter(|o| matches!(o.goal, Goal::PlantOn(_) | Goal::Plants(_) | Goal::FastPlant(_) | Goal::Detonations(_))).count();
            assert!(bomb <= 2, "seed {seed}: {o:?}");
        }
    }

    #[test]
    fn progress_reads_the_log() {
        let mut log = MatchLog::default();
        log.begin_round(5);
        {
            let r = log.current().unwrap();
            r.planted = Some(('B', 25.0));
            r.kills = vec![kill(WeaponKind::Rifle, true), kill(WeaponKind::Rifle, false), kill(WeaponKind::Pistol, true)];
            r.exploded = true;
            r.won = Some(true);
            r.had_primary = true;
        }
        log.begin_round(5);
        {
            let r = log.current().unwrap();
            r.won = Some(true);
            r.kills = vec![kill(WeaponKind::Pistol, false); 5];
        }
        assert_eq!(Goal::PlantOn('B').progress(&log), (1, 1));
        assert_eq!(Goal::PlantOn('A').progress(&log), (0, 1));
        assert_eq!(Goal::Kills(5).progress(&log), (8, 5));
        assert_eq!(Goal::Headshots(3).progress(&log), (2, 3));
        assert_eq!(Goal::KillsWith(WeaponKind::Rifle, 3).progress(&log), (2, 3));
        assert_eq!(Goal::MultiKill(3).progress(&log), (5, 3));
        assert_eq!(Goal::Ace.progress(&log), (1, 1));
        assert_eq!(Goal::FastPlant(30).progress(&log), (1, 1));
        assert_eq!(Goal::PistolWin.progress(&log), (1, 1));
        assert_eq!(Goal::FlawlessRound.progress(&log), (1, 1));
        assert_eq!(Goal::Streak(2).progress(&log), (2, 2));
        assert_eq!(Goal::Detonations(1).progress(&log), (1, 1));
    }

    #[test]
    fn credit_pays_and_promotes() {
        let mut c = Career { xp: 900, ..Default::default() };
        let mut log = MatchLog::default();
        log.begin_round(5);
        log.current().unwrap().planted = Some(('A', 50.0));
        let objs = [Objective { goal: Goal::PlantOn('A'), xp: 150 }, Objective { goal: Goal::Ace, xp: 500 }];
        let a = c.credit(true, 3, &objs, &log);
        assert_eq!(a.total, XP_MATCH_WIN + 3 * XP_ROUND_WIN + 150);
        assert!(a.promoted());
        assert_eq!(c.rank(), Rank::Private);
        assert_eq!((c.matches, c.wins, c.objectives_done), (1, 1, 1));
    }
}
