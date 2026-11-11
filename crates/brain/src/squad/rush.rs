//! Rush (`Play::Rush`): every member that can get there in time commits to his **last known
//! spot** (the shared board or a heard reload: never his true position) at once.
//!
//! - **Who**: living members within `RUSH_REACH_S` of sprinting (by path) of the spot; at least
//!   two, or no rush.
//! - **Angles**: members further than `RUSH_STAGE_FROM_M` first sprint to distinct approach
//!   sectors around the spot (the encirclement's staging cells, out of his sight, spread from
//!   the members already close). A fast rush (a heard reload: the window is short) skips the
//!   staging and spreads the routes with `encircle::diverse_vias` instead.
//! - **Timing**: at "go" each member's remaining path is measured; the near ones wait so that
//!   everyone arrives together (a fast rush caps the wait at `RUSH_FAST_LAG_S`). With a flash
//!   left in the budget, the member with the longest wait throws it first and the run is timed
//!   to arrive just after it pops.
//! - **Commitment**: the orders are sticky pushes (`Role::Rusher`): sprinting, never crouching,
//!   no fall-back to cover or posts when hit (`mind`), shooting on the move. Trades come from
//!   arriving together.
//! - **Triggers** (`rush_trigger`, `RUSH_COOLDOWN_S` between rushes): a heard mag-out (when
//!   the reload counter is strong or the squad is rush-minded), his
//!   health low (what the squad saw), numbers (3+ alive against a hurt player), and the
//!   bandit's Rush arm (`bandit::ARMS`), which also learns when rushing this player pays off;
//!   a rush arm that keeps losing switches the soft triggers off. The squad's rush gene
//!   (`Traits::rush`) moves the thresholds. Retake rushes (fuse running low) live in `bomb.rs`.
//!
//! Technique only: aim error, reaction time and damage are untouched.

use super::*;

/// Planning pace (m/s): a sprint with some slack for corners.
pub const RUSH_PACE: f32 = crate::sim::SPRINT * 0.9;
/// Members more than this many seconds of sprinting away stay out of the rush.
pub const RUSH_REACH_S: f32 = 9.0;
/// Seconds between the end of one rush and the next trigger.
pub const RUSH_COOLDOWN_S: f32 = 18.0;
/// After the planned arrival they keep pushing this long (s), then the usual plays resume.
pub const RUSH_PUSH_S: f32 = 5.0;
/// Staging ends after the slowest stager's sprint plus this (s)...
pub const RUSH_STAGE_SLACK_S: f32 = 1.5;
/// ...and never later than this (s, times the map scale).
pub const RUSH_STAGE_MAX_S: f32 = 9.0;
/// A member further than this (m, by path) stages on its own approach sector first.
pub const RUSH_STAGE_FROM_M: f32 = 18.0;
/// Fast rush (heard reload): the near ones wait at most this long (s) for the far ones.
pub const RUSH_FAST_LAG_S: f32 = 1.0;
/// The last known spot must be this fresh (s) for a health / numbers / learned rush.
pub const RUSH_INFO_S: f32 = 4.0;
/// Members this close (m) to the spot have arrived.
pub const RUSH_ARRIVED_M: f32 = 3.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RushReason {
    /// The squad heard his magazine come out.
    Reload,
    /// He is badly hurt (from what the squad saw).
    LowHp,
    /// Three or more of them against a hurt player.
    Numbers,
    /// The bandit's Rush arm (it pays off against this player).
    Learned,
    /// Bomb planted, fuse running low: everyone hits the site at once.
    Retake,
    /// Tests / control port.
    Forced,
}

impl RushReason {
    pub const ALL: [RushReason; 6] = [RushReason::Reload, RushReason::LowHp, RushReason::Numbers, RushReason::Learned, RushReason::Retake, RushReason::Forced];

    pub fn label(self) -> &'static str {
        match self {
            RushReason::Reload => "heard a reload",
            RushReason::LowHp => "he's low",
            RushReason::Numbers => "numbers advantage",
            RushReason::Learned => "learned: rushing pays vs him",
            RushReason::Retake => "fuse running low",
            RushReason::Forced => "forced",
        }
    }

    pub fn index(self) -> usize {
        RushReason::ALL.iter().position(|&r| r == self).unwrap_or(0)
    }

    /// The radio call when it starts.
    pub fn callout(self) -> &'static str {
        match self {
            RushReason::Reload => "He's reloading - go go go, rush him!",
            RushReason::LowHp => "He's weak - rush him, go go go!",
            RushReason::Numbers => "We've got the numbers - rush him!",
            RushReason::Learned | RushReason::Forced => "Go go go, rush him!",
            RushReason::Retake => "No time - rush the site, go go go!",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RushPhase {
    /// Far members sprint to their approach sectors; the rest wait.
    Stage,
    /// Everyone runs in, timed to arrive together.
    Go,
}

/// The rush in progress (`SquadCoordinator::rush`).
#[derive(Clone, Debug)]
pub struct Rush {
    pub reason: RushReason,
    /// His last known spot (updated only by fresh sightings of the squad).
    pub target: V2,
    pub phase: RushPhase,
    pub t0: f32,
    /// The members in it (slots).
    pub members: Vec<usize>,
    /// Per slot: the staging cell (sector goal), if it stages.
    pub stage: Vec<Option<V2>>,
    /// Per slot: when it starts its run (f32::INFINITY before "go").
    pub go_at: Vec<f32>,
    /// When the run is planned to arrive (after "go").
    pub arrive_t: f32,
    pub stage_until: f32,
    /// A fast rush (no staging, short waits).
    pub fast: bool,
    /// A flash went in first.
    pub flash: bool,
}

/// A rush started (for the log / trace / debrief).
#[derive(Clone, Debug)]
pub struct RushEvent {
    pub t: f32,
    pub reason: RushReason,
    pub target: V2,
    pub members: Vec<usize>,
}

/// Per-round rush counts (the game adds them to the match telemetry for the debrief).
#[derive(Clone, Copy, Debug, Default)]
pub struct RushStats {
    pub started: [u32; RushReason::ALL.len()],
    /// Rounds the squad won while a rush was on.
    pub won_in_rush: u32,
}

impl SquadCoordinator {
    /// The squad's rush strength from its genes: 0.3 + 0.7 x the mean rush gene.
    pub fn rush_strength(members: &[MemberView]) -> f32 {
        let alive: Vec<&MemberView> = members.iter().filter(|m| m.alive).collect();
        if alive.is_empty() {
            return 0.3;
        }
        0.3 + 0.7 * alive.iter().map(|m| m.traits.rush.clamp(0.0, 1.0)).sum::<f32>() / alive.len() as f32
    }

    /// Rush events since the last call (log them: `ai.tactic`).
    pub fn drain_rush_events(&mut self) -> Vec<RushEvent> {
        std::mem::take(&mut self.rush_events)
    }

    /// The Rush arm has been tried enough and keeps losing against this player.
    fn rush_learned_bad(&self) -> bool {
        let Some(k) = bandit::arm_of(Play::Rush) else { return false };
        self.tactics.arms.get(k).is_some_and(|a| a.n >= 3.0 && a.estimate(&self.last_ctx).0 < -0.25)
    }

    /// Should the squad rush now? (reason, the spot). Respects the cooldown.
    pub(super) fn rush_trigger(&self, members: &[MemberView]) -> Option<(RushReason, V2)> {
        if self.time < self.rush_next {
            return None;
        }
        let alive = members.iter().filter(|m| m.alive).count();
        if alive < 2 {
            return None;
        }
        // A heard mag-out: where they heard it, right now. The whole squad goes when the
        // reload counter is strong (he reloads early a lot) or the squad is rush-minded;
        // otherwise the doctrine sends its one or two rushers (`plan_doctrine`).
        let s = Self::rush_strength(members);
        if let Some((at, t)) = self.mag_out
            && self.time - t < 1.0
            && (self.doctrine.counters().reload_rush >= 0.6 || s >= 0.65)
        {
            return Some((RushReason::Reload, at));
        }
        if self.board.age > RUSH_INFO_S || self.rush_learned_bad() {
            return None;
        }
        let hp = self.player_hp;
        if hp > 0.0 && hp <= 0.25 + 0.2 * s {
            return Some((RushReason::LowHp, self.board.pos));
        }
        if alive >= 3 && hp <= 0.5 + 0.3 * s && self.board.age < 2.0 {
            return Some((RushReason::Numbers, self.board.pos));
        }
        None
    }

    /// Commit to a rush on `target`. False (and no rush) unless two members can get there.
    pub(super) fn start_rush(&mut self, reason: RushReason, target: V2, members: &[MemberView], arena: &Arena) -> bool {
        let n = members.len();
        let target = arena.nav.nearest_walkable(target);
        let field = arena.nav.distance_field(&[target], f32::INFINITY);
        let path_d = |i: usize| {
            let d = arena.nav.field_at(&field, members[i].pos);
            if d.is_finite() { d } else { members[i].pos.dist(target) * 1.5 }
        };
        let reach = RUSH_REACH_S * RUSH_PACE * self.map_scale.max(1.0);
        let mut who: Vec<usize> = (0..n).filter(|&i| members[i].alive && path_d(i) <= reach).collect();
        if who.len() < 2 {
            return false;
        }
        who.sort_by(|&a, &b| path_d(a).total_cmp(&path_d(b)).then(a.cmp(&b)));
        let fast = reason == RushReason::Reload;
        let mut stage = vec![None; n];
        self.vias.iter_mut().for_each(|v| *v = None);
        if fast {
            // No time to stage: spread the routes instead.
            let routes: Vec<(V2, V2)> = who.iter().map(|&i| (members[i].pos, target)).collect();
            let vias = encircle::diverse_vias(arena, target, None, &routes);
            for (k, &i) in who.iter().enumerate() {
                self.vias[i] = vias.get(k).copied().flatten().filter(|v| members[i].pos.dist(*v) + v.dist(target) < 1.4 * path_d(i) + 4.0);
            }
        } else {
            // Far members take distinct approach sectors, spread from the ones already close.
            let stagers: Vec<usize> = who.iter().copied().filter(|&i| path_d(i) > RUSH_STAGE_FROM_M).collect();
            if !stagers.is_empty() {
                let sectors = encircle::sectors(arena, target);
                let fixed: Vec<f32> = who.iter().filter(|i| !stagers.contains(i)).map(|&i| (members[i].pos - target).angle()).collect();
                let dist: Vec<Vec<f32>> = stagers
                    .iter()
                    .map(|&i| {
                        let f = arena.nav.distance_field(&[members[i].pos], f32::INFINITY);
                        sectors.iter().map(|s| arena.nav.field_at(&f, s.goal)).collect()
                    })
                    .collect();
                let pick = encircle::assign(&sectors, &dist, &fixed, None, &mut self.rng);
                for (k, &i) in stagers.iter().enumerate() {
                    stage[i] = pick[k].map(|s| sectors[s].goal);
                }
            }
        }
        let staging = stage.iter().any(Option::is_some);
        let slowest = who.iter().filter_map(|&i| stage[i].map(|s| members[i].pos.dist(s).max(path_d(i) - RUSH_STAGE_FROM_M))).fold(0.0f32, f32::max);
        let stage_until = self.time + (slowest / RUSH_PACE + RUSH_STAGE_SLACK_S).min(RUSH_STAGE_MAX_S * self.map_scale.max(1.0));
        self.rush = Some(Rush {
            reason,
            target,
            phase: RushPhase::Stage,
            t0: self.time,
            members: who.clone(),
            stage,
            go_at: vec![f32::INFINITY; n],
            arrive_t: f32::INFINITY,
            stage_until,
            fast,
            flash: false,
        });
        self.rush_stats.started[reason.index()] += 1;
        self.rush_events.push(RushEvent { t: self.time, reason, target, members: who.clone() });
        let speaker = who[0];
        if staging {
            self.bark(speaker, "Rush on my call - get in position".into(), true);
        }
        if !staging {
            self.rush_go(members, arena);
        }
        // (The "go" call is the reason's callout; staged rushes call it when they go.)
        true
    }

    /// Everyone is set: time the runs so they arrive together; a flash goes in first if any.
    fn rush_go(&mut self, members: &[MemberView], arena: &Arena) {
        let Some(mut r) = self.rush.take() else { return };
        let field = arena.nav.distance_field(&[r.target], f32::INFINITY);
        let d = |i: usize, via: Option<V2>| {
            let p = members[i].pos;
            let from = |q: V2| {
                let x = arena.nav.field_at(&field, q);
                if x.is_finite() { x } else { q.dist(r.target) * 1.5 }
            };
            match via {
                Some(v) => p.dist(v) * 1.2 + from(v),
                None => from(p),
            }
        };
        let runs: Vec<(usize, f32)> = r.members.iter().filter(|&&i| members[i].alive).map(|&i| (i, d(i, self.vias[i]) / RUSH_PACE)).collect();
        let longest = runs.iter().map(|x| x.1).fold(0.0f32, f32::max);
        // A flash pops just ahead of them: the member with the longest wait throws it.
        let mut lead_in = 0.0;
        if !r.fast && self.flashes_used < self.utility.flashes && self.pending_throw.is_none() {
            let pick = runs
                .iter()
                .filter(|(i, _)| !self.thrown[*i].1)
                .filter_map(|&(i, run)| {
                    let m = members[i].pos;
                    let aim = r.target + (m - r.target).norm() * 1.5;
                    let dd = m.dist(aim);
                    ((grenade::THROW_MIN..grenade::THROW_MAX).contains(&dd) && grenade::landing(arena, m, aim).dist(aim) <= 2.0).then_some((longest - run, i, aim))
                })
                .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
            if let Some((_, i, aim)) = pick {
                self.pending_throw = Some((ThrowOrder { thrower: i, kind: GrenadeKind::Flash, target: aim }, grenade::WINDUP_S));
                r.flash = true;
                // Arrive just after it pops (it flies about a second).
                let pop = grenade::WINDUP_S + GrenadeKind::Flash.fuse();
                lead_in = (pop - longest + 0.2).max(0.0);
            }
        }
        let now = self.time;
        let mut arrive = now;
        for &(i, run) in &runs {
            let mut wait = longest - run;
            if r.fast {
                wait = wait.min(RUSH_FAST_LAG_S);
            }
            let thrower = self.pending_throw.is_some_and(|(t, _)| t.thrower == i);
            let mut go = now + wait + lead_in;
            if thrower {
                go = go.max(now + grenade::WINDUP_S + 0.1);
            }
            r.go_at[i] = go;
            arrive = arrive.max(go + run);
        }
        r.arrive_t = arrive;
        r.phase = RushPhase::Go;
        let speaker = runs.first().map_or(r.members[0], |x| x.0);
        let reason = r.reason;
        self.rush = Some(r);
        self.bark(speaker, reason.callout().into(), true);
    }

    /// The rush's end: cooldown, back to the usual plays.
    pub(super) fn end_rush(&mut self) {
        if self.rush.take().is_some() {
            self.rush_next = self.time + RUSH_COOLDOWN_S;
            self.vias.iter_mut().for_each(|v| *v = None);
        }
    }

    pub(super) fn plan_rush(&mut self, members: &[MemberView], arena: &Arena) {
        if self.rush.is_none() {
            // Forced (tests): rush his last known spot.
            let at = self.board.pos;
            if self.force_play != Some(Play::Rush) || !self.start_rush(RushReason::Forced, at, members, arena) {
                self.orders.iter_mut().for_each(|o| *o = None);
                return;
            }
        }
        let now = self.time;
        // The squad saw him somewhere else just now: push there (still only what they saw).
        if self.board.age < 0.5
            && let Some(r) = self.rush.as_mut()
            && r.target.dist(self.board.pos) > 1.0
        {
            r.target = arena.nav.nearest_walkable(self.board.pos);
        }
        let Some(r) = self.rush.as_mut() else { return };
        r.members.retain(|&i| members.get(i).is_some_and(|m| m.alive));
        if r.members.is_empty() {
            self.end_rush();
            self.orders.iter_mut().for_each(|o| *o = None);
            return;
        }
        let target = r.target;
        if r.phase == RushPhase::Stage {
            let staged = r.members.iter().all(|&i| r.stage[i].is_none_or(|s| members[i].pos.dist(s) < ARRIVED_M));
            let contact = now - r.t0 > 0.7 && r.members.iter().any(|&i| members[i].last_seen < FRESH_CONTACT_S);
            if staged || contact || now >= r.stage_until {
                self.rush_go(members, arena);
            }
        }
        let Some(r) = self.rush.clone() else { return };
        // Over: the time is up, or everyone got there and nobody sees him.
        let arrived = r.members.iter().all(|&i| members[i].pos.dist(r.target) < RUSH_ARRIVED_M);
        let cold = r.members.iter().all(|&i| members[i].last_seen > 1.0);
        if r.phase == RushPhase::Go && (now > r.arrive_t + RUSH_PUSH_S || now > r.arrive_t && arrived && cold) {
            self.end_rush();
            return;
        }
        self.update_vias(members);
        let thrower = self.pending_throw.map(|(t, _)| t.thrower);
        for (i, m) in members.iter().enumerate() {
            if !m.alive {
                continue;
            }
            if !r.members.contains(&i) {
                // Too far to make it: close in behind them.
                let dest = arena.nav.nearest_walkable(target + (m.pos - target).norm() * 8.0);
                self.orders[i] = Some(Order { role: Role::Collapser, goal: Goal::Hunt, dest: Some(dest), watch: Some(target), threat: Some(target), sticky: false, via: None, prefire: None });
                continue;
            }
            if Some(i) == thrower {
                continue; // plan_utility holds it for the wind-up
            }
            let hold = |dest: Option<V2>| Order { role: Role::Rusher, goal: Goal::Hold, dest, watch: Some(target), threat: None, sticky: true, via: None, prefire: None };
            self.orders[i] = Some(match r.phase {
                RushPhase::Stage => match r.stage[i] {
                    Some(s) if m.pos.dist(s) >= ARRIVED_M => {
                        Order { role: Role::Rusher, goal: Goal::Flank, dest: Some(s), watch: Some(target), threat: Some(target), sticky: true, via: None, prefire: None }
                    }
                    Some(s) => hold(Some(s)),
                    None => hold(None),
                },
                RushPhase::Go if now < r.go_at[i] => hold(None),
                RushPhase::Go => Order { role: Role::Rusher, goal: Goal::Push, dest: Some(target), watch: Some(target), threat: None, sticky: true, via: self.vias[i], prefire: None },
            });
        }
    }
}

/// The debrief line about the squad's rushes this match, if they rushed at least twice.
pub fn debrief_line(t: &crate::player_model::MatchTelemetry) -> Option<String> {
    let total: f32 = t.rushes.iter().sum();
    if total < 2.0 {
        return None;
    }
    let (k, _) = t.rushes.iter().enumerate().filter(|(k, _)| RushReason::ALL[*k] != RushReason::Forced).max_by(|a, b| a.1.total_cmp(b.1))?;
    let when = match RushReason::ALL[k] {
        RushReason::Reload => "when they heard you reload",
        RushReason::LowHp => "when you were low",
        RushReason::Numbers => "when they had the numbers",
        RushReason::Learned => "because it kept working on you",
        RushReason::Retake => "to beat the fuse",
        RushReason::Forced => "",
    };
    let won = t.rush_wins.round() as u32;
    let tail = match won {
        0 => String::new(),
        1 => "; one of them won the round".to_string(),
        n => format!("; {n} of them won the round"),
    };
    Some(format!("They rushed you {} times, mostly {when}{tail}.", total.round() as u32))
}
