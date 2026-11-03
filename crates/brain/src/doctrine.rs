//! Adaptive doctrine (bodycam spec §5): a rolling profile of how the player is fighting *right
//! now* ([`PlayerStyleProfile`]), the archetypes it shows ([`Archetype`], several at once, with
//! hysteresis) and the squad tactic that counters the strongest one ([`AiTactic`]).
//!
//! Counters are technique only: where to stand, how to peek, when to flash, wallbang or rush.
//! Aim error, reaction time and damage stay fixed by difficulty (`limits.rs`). How hard each
//! counter is used is evolvable (`TraitGenome::doctrine`, one gene per archetype).
//!
//! The squad (`squad::SquadCoordinator`) owns one profile and one doctrine per round; the game
//! and the trainer feed the profile every tick. A summary (share of play time each archetype
//! was active) goes into the player model, so a habit carries over to the next match as a prior.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::arena::Arena;
use crate::math::{V2, angle_diff};
use crate::player_model::MatchTelemetry;
use crate::rooms::{self, Doorway};

/// Rolling window (s) for movement, angle holds, ADS share and room entries.
pub const WINDOW_S: f32 = 20.0;
/// Longer window (s) for sparse habits: early reloads and headshots.
pub const HABIT_WINDOW_S: f32 = 60.0;
/// Time constant (s) of the speed EMA.
pub const SPEED_TAU_S: f32 = 1.0;
/// Slower than this (m/s) counts as standing still.
pub const STILL_SPEED: f32 = 0.6;
/// An angle hold survives aim adjustments within this cone (rad, ~20 deg) and 1 m of drift.
pub const HOLD_CONE: f32 = 0.35;
pub const HOLD_DRIFT_M: f32 = 1.0;
/// Crossing a doorway at least this fast (m/s) is a rapid room entry.
pub const ENTRY_SPEED: f32 = 3.5;
/// A reload with more than this share of the magazine left is premature.
pub const PREMATURE_MAG: f32 = 0.5;

/// Archetype thresholds (spec §5.2) and the levels they switch off at (hysteresis).
pub const CAMPER_SPEED: f32 = 1.2;
pub const CAMPER_HOLD_S: f32 = 4.0;
pub const CAMPER_OFF_SPEED: f32 = 2.0;
pub const RUSHER_SPEED: f32 = 4.2;
pub const RUSHER_OFF_SPEED: f32 = 2.5;
pub const RELOADER_COUNT: u32 = 2;
pub const PRECISION_RATE: f32 = 0.45;
pub const PRECISION_OFF_RATE: f32 = 0.35;
pub const PRECISION_MIN_HITS: u32 = 4;
/// The primary tactic is kept at least this long (s), and a rival must score this much higher.
pub const TACTIC_DWELL_S: f32 = 1.5;
pub const TACTIC_SWITCH: f32 = 1.3;
/// A habit from earlier matches (share of play time at least `PRIOR_SHARE`) starts the round
/// active and lapses after `PRIOR_S` unless the live profile confirms it.
pub const PRIOR_SHARE: f32 = 0.35;
pub const PRIOR_S: f32 = 25.0;

/// One gene per archetype (`TraitGenome::doctrine`): how hard the squad uses that counter.
pub const DOCTRINE_GENES: usize = 4;

/// What the game (or the trainer) knows about the player this tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct StyleTick {
    pub dt: f32,
    pub pos: V2,
    /// Ground-plane velocity (m/s).
    pub vel: V2,
    /// Ground-plane look angle (rad, `V2::angle` convention).
    pub yaw: f32,
}

/// The player's style over the last ~20 s (spec §5.1).
#[derive(Clone, Debug, Default)]
pub struct PlayerStyleProfile {
    /// EMA of horizontal speed (m/s).
    pub avg_move_speed: f32,
    /// Seconds standing still watching one angle (> 4 s: angle holder).
    pub stationary_angle_hold_secs: f32,
    /// Share of shots fired aiming down sights (window).
    pub ads_usage_ratio: f32,
    /// Headshots / hits over the habit window.
    pub headshot_rate: f32,
    /// Hits behind `headshot_rate`.
    pub hits: u32,
    /// Reloads with more than half a magazine left (habit window).
    pub premature_reload_count: u32,
    /// Doorways crossed at a run (window).
    pub rapid_entries: u32,
    /// Seconds of profile time.
    pub clock: f32,
    /// The angle being held: (where he stands, look angle), while still.
    pub hold: Option<(V2, f32)>,
    shots: VecDeque<(f32, bool)>,
    hit_log: VecDeque<(f32, bool)>,
    reloads: VecDeque<(f32, ())>,
    entries: VecDeque<(f32, ())>,
    doors: Option<Vec<Doorway>>,
    last_pos: Option<V2>,
}

fn prune<T>(q: &mut VecDeque<(f32, T)>, now: f32, window: f32) {
    while q.front().is_some_and(|e| now - e.0 > window) {
        q.pop_front();
    }
}

impl PlayerStyleProfile {
    /// Advance one tick. `arena` (optional) enables room-entry detection.
    pub fn tick(&mut self, arena: Option<&Arena>, s: StyleTick) {
        if s.dt <= 0.0 {
            return;
        }
        self.clock += s.dt;
        let speed = s.vel.len();
        let a = 1.0 - (-s.dt / SPEED_TAU_S).exp();
        self.avg_move_speed += (speed - self.avg_move_speed) * a;
        // Angle hold: standing still, the look staying within a small cone.
        if speed < STILL_SPEED {
            match self.hold {
                Some((p, y)) if angle_diff(s.yaw, y).abs() < HOLD_CONE && p.dist(s.pos) < HOLD_DRIFT_M => self.stationary_angle_hold_secs += s.dt,
                _ => {
                    self.hold = Some((s.pos, s.yaw));
                    self.stationary_angle_hold_secs = 0.0;
                }
            }
        } else {
            self.hold = None;
            self.stationary_angle_hold_secs = 0.0;
        }
        // Rapid room entries: crossing a doorway at a run.
        if let Some(arena) = arena {
            let doors = self.doors.get_or_insert_with(|| rooms::doorways(arena));
            if let Some(last) = self.last_pos
                && speed >= ENTRY_SPEED
                && doors.iter().any(|d| d.pos.dist(s.pos) < d.width * 0.5 + 1.0 && d.side(last) != d.side(s.pos))
                && self.entries.back().is_none_or(|e| self.clock - e.0 > 1.0)
            {
                self.entries.push_back((self.clock, ()));
            }
        }
        self.last_pos = Some(s.pos);
        self.refresh();
    }

    /// He fired a shot (once per trigger pull).
    pub fn shot(&mut self, ads: bool) {
        self.shots.push_back((self.clock, ads));
        self.refresh();
    }

    /// One of his shots hit an enemy.
    pub fn hit(&mut self, headshot: bool) {
        self.hit_log.push_back((self.clock, headshot));
        self.refresh();
    }

    /// He started a reload with `mag_frac` of the magazine left.
    pub fn reload(&mut self, mag_frac: f32) {
        if mag_frac > PREMATURE_MAG {
            self.reloads.push_back((self.clock, ()));
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        let now = self.clock;
        prune(&mut self.shots, now, WINDOW_S);
        prune(&mut self.entries, now, WINDOW_S);
        prune(&mut self.hit_log, now, HABIT_WINDOW_S);
        prune(&mut self.reloads, now, HABIT_WINDOW_S);
        let n = self.shots.len();
        self.ads_usage_ratio = if n > 0 { self.shots.iter().filter(|s| s.1).count() as f32 / n as f32 } else { 0.0 };
        self.hits = self.hit_log.len() as u32;
        self.headshot_rate = if self.hits > 0 { self.hit_log.iter().filter(|h| h.1).count() as f32 / self.hits as f32 } else { 0.0 };
        self.premature_reload_count = self.reloads.len() as u32;
        self.rapid_entries = self.entries.len() as u32;
    }

    /// Archetypes active given the ones active last time (`prev`): on at the spec's
    /// thresholds, off only well below them.
    pub fn detect(&self, prev: Archetypes) -> Archetypes {
        let mut out = Archetypes::NONE;
        let camper_on = self.avg_move_speed < CAMPER_SPEED && self.stationary_angle_hold_secs > CAMPER_HOLD_S;
        let camper_keep = prev.has(Archetype::AngleHolder) && self.avg_move_speed < CAMPER_OFF_SPEED;
        out.set(Archetype::AngleHolder, camper_on || camper_keep);
        let rusher_on = self.avg_move_speed > RUSHER_SPEED && self.rapid_entries >= 1;
        let rusher_keep = prev.has(Archetype::EntryFragger) && self.rapid_entries >= 1 && self.avg_move_speed > RUSHER_OFF_SPEED;
        out.set(Archetype::EntryFragger, rusher_on || rusher_keep);
        let reloader_on = self.premature_reload_count >= RELOADER_COUNT;
        let reloader_keep = prev.has(Archetype::CompulsiveReloader) && self.premature_reload_count >= 1;
        out.set(Archetype::CompulsiveReloader, reloader_on || reloader_keep);
        let precise_on = self.hits >= PRECISION_MIN_HITS && self.headshot_rate > PRECISION_RATE;
        let precise_keep = prev.has(Archetype::PrecisionAimer) && self.hits >= 2 && self.headshot_rate >= PRECISION_OFF_RATE;
        out.set(Archetype::PrecisionAimer, precise_on || precise_keep);
        out
    }

    /// How far past its threshold an archetype is (>= 1 at the threshold).
    pub fn severity(&self, a: Archetype) -> f32 {
        match a {
            Archetype::AngleHolder => (self.stationary_angle_hold_secs / CAMPER_HOLD_S).clamp(1.0, 3.0),
            Archetype::EntryFragger => (self.avg_move_speed / RUSHER_SPEED).clamp(0.6, 2.0) * (1.0 + 0.25 * self.rapid_entries.min(4) as f32),
            Archetype::CompulsiveReloader => (self.premature_reload_count as f32 / RELOADER_COUNT as f32).clamp(0.5, 3.0),
            Archetype::PrecisionAimer => (self.headshot_rate / PRECISION_RATE).clamp(0.7, 2.0),
        }
    }

    /// Why `a` is active, in words (logs, callouts, the F3 overlay).
    pub fn reason(&self, a: Archetype) -> String {
        match a {
            Archetype::AngleHolder => {
                format!("held one angle {:.1} s at {:.1} m/s", self.stationary_angle_hold_secs.max(CAMPER_HOLD_S), self.avg_move_speed)
            }
            Archetype::EntryFragger => format!("{} room entries at a run, {:.1} m/s", self.rapid_entries, self.avg_move_speed),
            Archetype::CompulsiveReloader => format!("{} reloads with over half a mag left", self.premature_reload_count),
            Archetype::PrecisionAimer => format!("{:.0}% headshots over {} hits", self.headshot_rate * 100.0, self.hits),
        }
    }
}

/// A player style the squad recognises (spec §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Archetype {
    /// Stands still watching one angle (AWP-style camper).
    AngleHolder = 0,
    /// W-key: runs through doors at speed.
    EntryFragger,
    /// Reloads with half a magazine left.
    CompulsiveReloader,
    /// Lands headshots.
    PrecisionAimer,
}

impl Archetype {
    pub const ALL: [Archetype; DOCTRINE_GENES] = [Archetype::AngleHolder, Archetype::EntryFragger, Archetype::CompulsiveReloader, Archetype::PrecisionAimer];

    pub fn label(self) -> &'static str {
        match self {
            Archetype::AngleHolder => "angle holder",
            Archetype::EntryFragger => "entry fragger",
            Archetype::CompulsiveReloader => "compulsive reloader",
            Archetype::PrecisionAimer => "precision aimer",
        }
    }

    /// The tactic that counters it.
    pub fn counter(self) -> AiTactic {
        match self {
            Archetype::AngleHolder => AiTactic::JigglePeekAndFlash,
            Archetype::EntryFragger => AiTactic::OffAngleTrap,
            Archetype::CompulsiveReloader => AiTactic::ReloadPunish,
            Archetype::PrecisionAimer => AiTactic::LeanPeekStagger,
        }
    }
}

/// A set of archetypes (bit per `Archetype`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Archetypes(pub u8);

impl Archetypes {
    pub const NONE: Archetypes = Archetypes(0);

    pub fn has(self, a: Archetype) -> bool {
        self.0 & (1 << a as u8) != 0
    }

    pub fn set(&mut self, a: Archetype, on: bool) {
        if on {
            self.0 |= 1 << a as u8;
        } else {
            self.0 &= !(1 << a as u8);
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Archetype> {
        Archetype::ALL.into_iter().filter(move |a| self.has(*a))
    }

    pub fn labels(self) -> Vec<&'static str> {
        self.iter().map(Archetype::label).collect()
    }
}

/// The squad's doctrine (spec §5.2 / §7 check 6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AiTactic {
    /// No counter: the bandit's plays as usual.
    #[default]
    Default,
    /// vs angle holders: jiggle-peek to bait shots, flash the angle, wallbang thin cover.
    JigglePeekAndFlash,
    /// vs entry fraggers: hold deep off-angles away from the door line, level-change on contact.
    OffAngleTrap,
    /// vs compulsive reloaders: rush on the mag-out sound.
    ReloadPunish,
    /// vs precision aimers: lean-peek from cover, stagger with random micro-strafes.
    LeanPeekStagger,
}

impl AiTactic {
    pub const ALL: [AiTactic; 5] = [AiTactic::Default, AiTactic::JigglePeekAndFlash, AiTactic::OffAngleTrap, AiTactic::ReloadPunish, AiTactic::LeanPeekStagger];

    /// The Rust name (`ai.tactic` logs use it, like the spec's `AiTactic::JigglePeekAndFlash`).
    pub fn name(self) -> &'static str {
        match self {
            AiTactic::Default => "Default",
            AiTactic::JigglePeekAndFlash => "JigglePeekAndFlash",
            AiTactic::OffAngleTrap => "OffAngleTrap",
            AiTactic::ReloadPunish => "ReloadPunish",
            AiTactic::LeanPeekStagger => "LeanPeekStagger",
        }
    }

    /// Short words for the HUD overlay.
    pub fn label(self) -> &'static str {
        match self {
            AiTactic::Default => "default",
            AiTactic::JigglePeekAndFlash => "jiggle + flash",
            AiTactic::OffAngleTrap => "off-angle trap",
            AiTactic::ReloadPunish => "reload punish",
            AiTactic::LeanPeekStagger => "lean + stagger",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// The squad's callout when it switches to this tactic.
    pub fn callout(self) -> &'static str {
        match self {
            AiTactic::Default => "Back to standard",
            AiTactic::JigglePeekAndFlash => "He's holding that angle - jiggle it, flash him",
            AiTactic::OffAngleTrap => "He's running in hot - hold the corners",
            AiTactic::ReloadPunish => "He reloads early - push on the mag",
            AiTactic::LeanPeekStagger => "He's hitting heads - lean and keep moving",
        }
    }
}

/// How hard each counter is used right now: 0 = off, else `COUNTER_MIN..=1` (the squad's
/// gene for it). Several can be on at once (one per active archetype).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counters {
    /// vs angle holders: jiggle-peeks, flashes, wallbangs.
    pub jiggle: f32,
    /// vs entry fraggers: off-angle holds, crouch on contact.
    pub off_angle: f32,
    /// vs reloaders: rush on the mag-out.
    pub reload_rush: f32,
    /// vs precision aimers: lean-peeks from posts, micro-strafes.
    pub stagger: f32,
}

/// The weakest a counter is used once its archetype is active (gene 0).
pub const COUNTER_MIN: f32 = 0.25;

impl Counters {
    pub fn get(&self, a: Archetype) -> f32 {
        match a {
            Archetype::AngleHolder => self.jiggle,
            Archetype::EntryFragger => self.off_angle,
            Archetype::CompulsiveReloader => self.reload_rush,
            Archetype::PrecisionAimer => self.stagger,
        }
    }

    fn set(&mut self, a: Archetype, v: f32) {
        match a {
            Archetype::AngleHolder => self.jiggle = v,
            Archetype::EntryFragger => self.off_angle = v,
            Archetype::CompulsiveReloader => self.reload_rush = v,
            Archetype::PrecisionAimer => self.stagger = v,
        }
    }
}

/// Strength of a counter from its gene (0..1).
pub fn strength(gene: f32) -> f32 {
    COUNTER_MIN + (1.0 - COUNTER_MIN) * gene.clamp(0.0, 1.0)
}

/// A doctrine switch, for logs, traces and the debrief.
#[derive(Clone, Debug, PartialEq)]
pub struct TacticChange {
    /// Profile clock (s since the round started).
    pub t: f32,
    pub from: AiTactic,
    pub to: AiTactic,
    pub reason: String,
    pub active: Archetypes,
}

/// The squad's current doctrine and how it got there.
#[derive(Clone, Debug, Default)]
pub struct Doctrine {
    pub tactic: AiTactic,
    pub active: Archetypes,
    pub reason: String,
    /// Profile clock when `tactic` was adopted.
    pub since: f32,
    /// Switches not yet taken by `drain_changes` (logs, traces).
    pending: Vec<TacticChange>,
    /// Every switch this round (bounded).
    pub history: Vec<TacticChange>,
    /// Seconds each archetype was active this round, and seconds observed.
    pub active_time: [f32; DOCTRINE_GENES],
    pub play_time: f32,
    /// Habits from earlier matches: active until `prior_until` unless the profile confirms them.
    prior: Archetypes,
    prior_until: f32,
    /// Counter strengths (from the squad's genes) for the active archetypes.
    counters: Counters,
}

const HISTORY_CAP: usize = 32;

impl Doctrine {
    /// A doctrine that starts with the habits a summary from earlier matches shows.
    pub fn with_prior(summary: &StyleSummary) -> Self {
        let mut prior = Archetypes::NONE;
        if summary.matches > 0 {
            for a in Archetype::ALL {
                prior.set(a, summary.share[a as usize] >= PRIOR_SHARE);
            }
        }
        Doctrine { prior, prior_until: if prior.is_empty() { 0.0 } else { PRIOR_S }, ..Default::default() }
    }

    /// Update from the profile; `genes`: the squad's mean doctrine genes. Returns a switch.
    pub fn update(&mut self, p: &PlayerStyleProfile, dt: f32, genes: [f32; DOCTRINE_GENES]) -> Option<TacticChange> {
        let live = p.detect(self.active);
        let prior_live = p.clock < self.prior_until;
        let mut active = live;
        if prior_live {
            for a in self.prior.iter() {
                active.set(a, true);
            }
        }
        self.play_time += dt;
        for a in active.iter() {
            self.active_time[a as usize] += dt;
        }
        let mut counters = Counters::default();
        for a in active.iter() {
            counters.set(a, strength(genes[a as usize]));
        }
        self.counters = counters;
        self.active = active;
        // Primary tactic: the active archetype with the highest severity x strength.
        let score = |a: Archetype| {
            let sev = if live.has(a) { p.severity(a) } else { 0.8 };
            sev * counters.get(a)
        };
        let best = active.iter().max_by(|a, b| score(*a).total_cmp(&score(*b)));
        let want = best.map_or(AiTactic::Default, Archetype::counter);
        if want == self.tactic {
            return None;
        }
        let current = Archetype::ALL.into_iter().find(|a| a.counter() == self.tactic);
        let current_on = current.is_some_and(|a| active.has(a));
        if p.clock - self.since < TACTIC_DWELL_S && self.tactic != AiTactic::Default && current_on {
            return None;
        }
        if let (true, Some(cur), Some(b)) = (current_on, current, best)
            && score(b) < TACTIC_SWITCH * score(cur)
        {
            return None;
        }
        let reason = match best {
            Some(a) if live.has(a) => p.reason(a),
            Some(a) => format!("{} habit from earlier matches", a.label()),
            None => "no clear habit any more".to_string(),
        };
        let change = TacticChange { t: p.clock, from: self.tactic, to: want, reason: reason.clone(), active };
        self.tactic = want;
        self.since = p.clock;
        self.reason = reason;
        self.pending.push(change.clone());
        if self.history.len() < HISTORY_CAP {
            self.history.push(change.clone());
        }
        Some(change)
    }

    /// Counter strengths for every active archetype.
    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// Switches since the last call.
    pub fn drain_changes(&mut self) -> Vec<TacticChange> {
        std::mem::take(&mut self.pending)
    }
}

/// Across matches: the share of play time each archetype was active (EMA, `PlayerModel::style`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleSummary {
    pub share: [f32; DOCTRINE_GENES],
    pub matches: u32,
}

impl StyleSummary {
    /// Blend one match's `style_time` in (`a`: the model's learning rate).
    pub fn ingest(&mut self, t: &MatchTelemetry, a: f32) {
        if t.style_play_s < 10.0 {
            return;
        }
        let a = if self.matches == 0 { 1.0 } else { a };
        for (s, secs) in self.share.iter_mut().zip(t.style_time) {
            *s += ((secs / t.style_play_s).clamp(0.0, 1.0) - *s) * a;
        }
        self.matches += 1;
    }
}

/// The debrief line for the archetype the squad countered most this match, if any.
pub fn debrief_line(t: &MatchTelemetry) -> Option<String> {
    if t.style_play_s <= 0.0 {
        return None;
    }
    let (i, secs) = t.style_time.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1))?;
    if secs < 8.0 || secs / t.style_play_s < 0.08 {
        return None;
    }
    Some(
        match Archetype::ALL[i] {
            Archetype::AngleHolder => "They noticed you hold angles; they started jiggle-peeking and flashing.",
            Archetype::EntryFragger => "They noticed you rush rooms; they started holding off-angles and dropping to a crouch.",
            Archetype::CompulsiveReloader => "They noticed you reload early; they started rushing you on the mag-out.",
            Archetype::PrecisionAimer => "They noticed your headshots; they started lean-peeking and micro-strafing.",
        }
        .to_string(),
    )
}

/// Off-angle trap (vs entry fraggers): a spot on `me`'s side of a doorway `door` that he will
/// come through from `from`, 3-8 m in, 35-80 deg off the door line, that sees the inside of the
/// door but can't be seen from outside it. Best: near cover, deep in the corner.
pub fn off_angle_spot(arena: &Arena, door: &Doorway, from: V2, me: V2) -> Option<V2> {
    let inward = door.normal * -door.side(from);
    if door.side(me) == door.side(from) {
        return None;
    }
    let outside = door.pos - inward * 2.5;
    let just_in = door.pos + inward * 1.0;
    let mut best: Option<(f32, V2)> = None;
    for k in 0..24 {
        let ang = k as f32 * std::f32::consts::TAU / 24.0;
        for r in [3.5f32, 5.0, 6.5, 8.0] {
            let p = door.pos + V2::from_angle(inward.angle() + ang) * r;
            let off = angle_diff((p - door.pos).angle(), inward.angle()).abs();
            if !(0.6..1.4).contains(&off) || !arena.nav.walkable_at(p) || (p - door.pos).dot(inward) < 1.5 {
                continue;
            }
            if arena.line_of_sight(outside, false, p, false) || !arena.line_of_sight(p, false, just_in, false) {
                continue;
            }
            let cover = arena.cover.iter().any(|c| c.pos.dist(p) < 1.5);
            let s = off + if cover { 0.4 } else { 0.0 } - 0.03 * p.dist(me) + 0.05 * r;
            if best.is_none_or(|(b, _)| s > b) {
                best = Some((s, p));
            }
        }
    }
    best.map(|b| b.1)
}

/// A jiggle-peek spot for a member at `me` vs a player holding an angle at `target`: a corner
/// within a short walk (<= `JIGGLE_WALK_M`) where it stands hidden from him, with a cell a step
/// aside that sees him. Returns (walk in m, the jiggle).
pub fn jiggle_spot(arena: &Arena, me: V2, target: V2) -> Option<(f32, crate::squad::Jiggle)> {
    let nav = &arena.nav;
    let field = nav.distance_field(&[me], JIGGLE_WALK_M);
    let (cx, cz) = nav.cell_of(me);
    let r = (JIGGLE_WALK_M * 0.7 / nav.cell).ceil() as i64;
    let mut best: Option<(f32, crate::squad::Jiggle)> = None;
    for dz in -r..=r {
        for dx in -r..=r {
            let (x, z) = (cx as i64 + dx, cz as i64 + dz);
            if x < 0 || z < 0 || x >= nav.w as i64 || z >= nav.w as i64 {
                continue;
            }
            let i = z as usize * nav.w + x as usize;
            let walk = field[i];
            if !walk.is_finite() || best.as_ref().is_some_and(|b| walk >= b.0) {
                continue;
            }
            let c = nav.center(i);
            if !(5.0..32.0).contains(&c.dist(target)) || arena.line_of_sight(target, false, c, false) {
                continue;
            }
            let side = (target - c).norm().perp();
            let peek = [0.6f32, 0.9, 1.2].into_iter().flat_map(|o| [side * o, side * -o]).map(|o| c + o).find(|p| {
                nav.walkable_at(*p) && nav.clear_line(c, *p) && arena.line_of_sight(*p, false, target, false)
            });
            if let Some(p) = peek {
                best = Some((walk, crate::squad::Jiggle { hide: c, out: (p - c).norm() * crate::squad::JIGGLE_M }));
            }
        }
    }
    best
}

/// Longest walk (m) to a jiggle-peek spot.
pub const JIGGLE_WALK_M: f32 = 14.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archetype_bits() {
        let mut s = Archetypes::NONE;
        s.set(Archetype::PrecisionAimer, true);
        s.set(Archetype::AngleHolder, true);
        assert!(s.has(Archetype::AngleHolder) && !s.has(Archetype::EntryFragger));
        assert_eq!(s.iter().count(), 2);
        s.set(Archetype::AngleHolder, false);
        assert_eq!(s.labels(), vec!["precision aimer"]);
    }
}
