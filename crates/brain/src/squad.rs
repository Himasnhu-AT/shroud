//! Squad layer (docs/research/adaptive-squad-ai.md §4.2): a coordinator that both the
//! trainer (`sim::run_round`) and the game tick once per frame, before the minds think.
//!
//! - **Blackboard v2**: the radio-delayed sighting (`Blackboard`), a decaying belief grid over
//!   the player's position (sightings, heard shots, negative information), claims on positions,
//!   and the current play.
//! - **Plays with role slots**: bounding overwatch, crossfire, trade, pin-and-flank, collapse, bait.
//!   The coordinator assigns roles; minds treat orders as high-priority goals and keep their micro.
//! - **Position evaluation** (Killzone-style): cover points scored by Σ w_k·f_k, with evolvable
//!   weights (`Traits::pef`). The same terms are A* link costs for tactical paths.
//! - **Learned play selection**: a LinUCB contextual bandit (`bandit`) picks the tactical play
//!   from player-habit features; the hand weights are its prior. Rewarded per engagement window.
//! - **Anchors**: a member holds and pre-aims the corner the player keeps re-peeking, and fires a
//!   short pre-fire burst when he is due to reappear there.
//! - **Breachers**: frags flush a player camping cover, flashbangs go in before a flank or entry.
//! - **Callouts**: short ASCII text events for the UI.

use crate::arena::{ARENA_HALF, Arena, BlockKind};
use crate::bandit::{self, ArmScore, CTX_DIM, LiveCtx, TacticsLearner};
use crate::genome::Traits;
use crate::grenade::{self, GrenadeKind};
use crate::math::{V2, angle_diff};
use crate::mind::{Blackboard, RADIO_PERIOD_S};
use crate::perception::{Alert, Heard, Sound, heard_sigma};
use crate::player_model::{CoverState, HEAT_W, PlayerModel, SECTORS, classify_cover, heat_idx, sector_of};
use crate::utility::Goal;
use std::cell::RefCell;
use std::f32::consts::{PI, TAU};

// ---------------------------------------------------------------------------------------------
// Position evaluation functions.

pub const PEF_TERMS: usize = 8;
/// Indices of the PEF terms (also gene order in `TraitGenome::pef`).
pub mod term {
    pub const LINE_OF_FIRE: usize = 0;
    pub const PARTIAL_COVER: usize = 1;
    pub const PROXIMITY: usize = 2;
    pub const PREFERRED_RANGE: usize = 3;
    pub const PLAYER_WEAK_RANGE: usize = 4;
    pub const SPREAD: usize = 5;
    pub const OFF_PRE_AIM: usize = 6;
    pub const DEATH_HEAT: usize = 7;
}
pub const PEF_LABELS: [&str; PEF_TERMS] =
    ["Line of fire", "Partial cover", "Proximity", "Preferred range", "His weak range", "Spread", "Off his pre-aim", "His death spots"];

/// What a position is scored against.
pub struct PefCtx<'a> {
    pub arena: &'a Arena,
    pub model: &'a PlayerModel,
    /// Belief-weighted player position.
    pub target: V2,
    pub me: V2,
    pub pref_range: f32,
    /// Positions already claimed by teammates.
    pub mates: &'a [V2],
}

/// Share of the player's look time spent facing the sector that `p` lies in, seen from `from`.
fn pre_aim_share(model: &PlayerModel, from: V2, p: V2) -> f32 {
    model.facing[sector_of((p - from).angle())]
}

/// Share of the player's deaths within ~3 m of `p` (0..1).
fn death_heat(model: &PlayerModel, p: V2) -> f32 {
    let total: f32 = model.death_spots.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let i = heat_idx(p) as i64;
    let (cx, cz) = (i % HEAT_W as i64, i / HEAT_W as i64);
    let mut s = 0.0;
    for dz in -1..=1 {
        for dx in -1..=1 {
            let (x, z) = (cx + dx, cz + dz);
            if x >= 0 && z >= 0 && x < HEAT_W as i64 && z < HEAT_W as i64 {
                s += model.death_spots[(z * HEAT_W as i64 + x) as usize];
            }
        }
    }
    (3.0 * s / total).min(1.0)
}

/// The eight terms f_k(p) of §4.2.3, each oriented so higher is better for the squad.
pub fn pef_features(p: V2, c: &PefCtx) -> [f32; PEF_TERMS] {
    let d = p.dist(c.target);
    let lof = c.arena.line_of_sight(p, false, c.target, false);
    let hidden_crouched = !c.arena.line_of_sight(p, true, c.target, false);
    let bearing = (p - c.target).angle();
    let sep = c.mates.iter().map(|m| angle_diff(bearing, (*m - c.target).angle()).abs()).fold(PI, f32::min) / PI;
    let mut f = [0.0; PEF_TERMS];
    f[term::LINE_OF_FIRE] = lof as u8 as f32;
    // Best: can shoot standing, hidden crouched. Fully hidden is still some cover.
    f[term::PARTIAL_COVER] = if lof && hidden_crouched { 1.0 } else if !lof { 0.3 } else { 0.0 };
    f[term::PROXIMITY] = 1.0 - (p.dist(c.me) / 30.0).min(1.0);
    f[term::PREFERRED_RANGE] = 1.0 - ((d - c.pref_range).abs() / 15.0).min(1.0);
    f[term::PLAYER_WEAK_RANGE] = 1.0 - c.model.accuracy_at(d).clamp(0.0, 1.0);
    f[term::SPREAD] = sep;
    f[term::OFF_PRE_AIM] = 1.0 - (pre_aim_share(c.model, c.target, p) * 4.0).min(1.0);
    f[term::DEATH_HEAT] = death_heat(c.model, p);
    f
}

pub fn pef_score(p: V2, c: &PefCtx, w: &[f32; PEF_TERMS]) -> f32 {
    pef_features(p, c).iter().zip(w).map(|(f, w)| f * w).sum()
}

/// Candidate positions: cover points at fighting range of the target plus a ring around it.
fn candidates(arena: &Arena, target: V2, ring: f32) -> Vec<V2> {
    let max_d = (ring + 4.0).clamp(10.0, 24.0);
    let mut out: Vec<V2> = arena.cover.iter().map(|c| c.pos).filter(|p| (4.0..max_d).contains(&p.dist(target))).collect();
    for k in 0..16 {
        let p = target + V2::from_angle(k as f32 * TAU / 16.0) * ring.max(6.0);
        if arena.in_bounds(p) && arena.nav.walkable_at(p) {
            out.push(p);
        }
    }
    out
}

/// Best-scoring position, with a hard bearing separation (radians, around `target`) from
/// `mates` when possible. Falls back to the unconstrained best.
pub fn best_position(c: &PefCtx, w: &[f32; PEF_TERMS], min_sep: f32) -> Option<V2> {
    let mut best: Option<(f32, V2)> = None;
    let mut best_sep: Option<(f32, V2)> = None;
    let mut best_any: Option<(f32, V2)> = None;
    for p in candidates(c.arena, c.target, c.pref_range) {
        if c.mates.iter().any(|m| m.dist(p) < 2.5) {
            continue;
        }
        let s = pef_score(p, c, w);
        let bearing = (p - c.target).angle();
        // Prefer firing positions that see the target (standing) and keep the separation.
        let sep_ok = c.mates.iter().all(|m| angle_diff(bearing, (*m - c.target).angle()).abs() > min_sep);
        let lof = c.arena.line_of_sight(p, false, c.target, false);
        if sep_ok && lof && best.is_none_or(|(b, _)| s > b) {
            best = Some((s, p));
        }
        if sep_ok && best_sep.is_none_or(|(b, _)| s > b) {
            best_sep = Some((s, p));
        }
        if best_any.is_none_or(|(b, _)| s > b) {
            best_any = Some((s, p));
        }
    }
    best.or(best_sep).or(best_any).map(|b| b.1)
}

/// Tactical A* extra cost per metre at `p` (use with `NavGrid::find_path_cost`): the PEF terms
/// as penalties for moving through the threat's line of fire, his pre-aim and his strong range.
pub fn tactical_cost<'a>(arena: &'a Arena, model: &'a PlayerModel, threat: V2, w: &'a [f32; PEF_TERMS]) -> impl Fn(V2) -> f32 + 'a {
    let cache = RefCell::new(vec![f32::NAN; HEAT_W * HEAT_W]);
    move |p: V2| {
        let i = heat_idx(p);
        let cached = cache.borrow()[i];
        if !cached.is_nan() {
            return cached;
        }
        let d = p.dist(threat);
        let exposed = d < 45.0 && arena.line_of_sight(threat, false, p, false);
        let v = if exposed {
            0.6 * w[term::LINE_OF_FIRE]
                + 0.6 * w[term::PLAYER_WEAK_RANGE] * model.accuracy_at(d).clamp(0.0, 1.0)
                + 1.2 * w[term::OFF_PRE_AIM] * pre_aim_share(model, threat, p)
        } else {
            0.0
        };
        cache.borrow_mut()[i] = v;
        v
    }
}

// ---------------------------------------------------------------------------------------------
// Belief grid.

pub const BELIEF_W: usize = 16;
const BELIEF_CELL: f32 = 2.0 * ARENA_HALF / BELIEF_W as f32;

/// Decaying grid belief over the player's position (sum = 1). The particle filter is P2.
#[derive(Clone, Debug)]
pub struct Belief {
    pub p: Vec<f32>,
    walkable: Vec<bool>,
}

fn cell_center(i: usize) -> V2 {
    V2::new(-ARENA_HALF + ((i % BELIEF_W) as f32 + 0.5) * BELIEF_CELL, -ARENA_HALF + ((i / BELIEF_W) as f32 + 0.5) * BELIEF_CELL)
}

fn cell_of(p: V2) -> usize {
    let c = |v: f32| (((v + ARENA_HALF) / BELIEF_CELL).floor().clamp(0.0, (BELIEF_W - 1) as f32)) as usize;
    c(p.y) * BELIEF_W + c(p.x)
}

impl Belief {
    pub fn new(arena: &Arena, prior: V2) -> Self {
        let walkable: Vec<bool> = (0..BELIEF_W * BELIEF_W)
            .map(|i| {
                let c = cell_center(i);
                // A cell counts if any of a few samples inside it is walkable.
                [V2::ZERO, V2::new(1.0, 1.0), V2::new(-1.0, 1.0), V2::new(1.0, -1.0), V2::new(-1.0, -1.0)]
                    .iter()
                    .any(|o| arena.nav.walkable_at(c + *o))
            })
            .collect();
        let mut b = Belief { p: vec![0.0; BELIEF_W * BELIEF_W], walkable };
        b.gaussian(prior, 6.0, 1.0);
        b
    }

    fn normalize(&mut self) {
        for (p, w) in self.p.iter_mut().zip(&self.walkable) {
            if !w {
                *p = 0.0;
            }
        }
        let s: f32 = self.p.iter().sum();
        if s > 1e-9 {
            self.p.iter_mut().for_each(|p| *p /= s);
        } else {
            let n = self.walkable.iter().filter(|w| **w).count().max(1) as f32;
            for (p, w) in self.p.iter_mut().zip(&self.walkable) {
                *p = if *w { 1.0 / n } else { 0.0 };
            }
        }
    }

    /// Blend in a Gaussian bump at `at` with mass `mix` (1 = replace).
    fn gaussian(&mut self, at: V2, sigma: f32, mix: f32) {
        let mut bump: Vec<f32> = (0..self.p.len()).map(|i| (-cell_center(i).dist(at).powi(2) / (2.0 * sigma * sigma)).exp()).collect();
        let s: f32 = bump.iter().sum::<f32>().max(1e-9);
        bump.iter_mut().for_each(|b| *b /= s);
        for (p, b) in self.p.iter_mut().zip(bump) {
            *p = (1.0 - mix) * *p + mix * b;
        }
        self.normalize();
    }

    /// The player moves: spread probability to neighbouring cells.
    pub fn diffuse(&mut self, secs: f32) {
        let a = (secs * 4.0 / BELIEF_CELL * 0.5).min(0.8);
        let w = BELIEF_W as i64;
        let old = self.p.clone();
        for i in 0..old.len() {
            let (x, z) = ((i as i64) % w, (i as i64) / w);
            let (mut s, mut n) = (0.0, 0.0);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx >= 0 && nz >= 0 && nx < w && nz < w && self.walkable[(nz * w + nx) as usize] {
                    s += old[(nz * w + nx) as usize];
                    n += 1.0;
                }
            }
            if n > 0.0 {
                self.p[i] = (1.0 - a) * old[i] + a * s / n;
            }
        }
        self.normalize();
    }

    /// Pull a little mass toward where this player habitually spends time (`PlayerModel::heat`).
    pub fn habit_prior(&mut self, model: &PlayerModel, mix: f32) {
        let mut prior = vec![0.0; self.p.len()];
        for (i, h) in model.heat.iter().enumerate() {
            prior[cell_of(crate::player_model::heat_center(i))] += *h;
        }
        let s: f32 = prior.iter().sum();
        if s <= 0.0 {
            return;
        }
        for (p, q) in self.p.iter_mut().zip(prior) {
            *p = (1.0 - mix) * *p + mix * q / s;
        }
        self.normalize();
    }

    pub fn sighting(&mut self, at: V2) {
        self.gaussian(at, 1.5, 1.0);
    }

    pub fn heard(&mut self, at: V2) {
        self.gaussian(at, 3.0, 0.8);
    }

    /// A heard sound with its own uncertainty (quiet sounds are hard to place).
    pub fn heard_with(&mut self, at: V2, sigma: f32) {
        self.gaussian(at, sigma, 0.8);
    }

    /// Negative information: a member looking from `eye` along `look` saw nobody.
    pub fn clear_seen(&mut self, arena: &Arena, eye: V2, look: V2, range: f32, half_angle: f32) {
        for i in 0..self.p.len() {
            if self.p[i] <= 0.0 {
                continue;
            }
            let c = cell_center(i);
            let d = c - eye;
            let dist = d.len();
            if dist > range || (dist > 3.0 && angle_diff(d.angle(), look.angle()).abs() > half_angle) {
                continue;
            }
            // Only clear a cell if even a crouched player there would have been seen.
            if arena.line_of_sight(eye, false, c, true) {
                self.p[i] *= 0.15;
            }
        }
        self.normalize();
    }

    pub fn mean(&self) -> V2 {
        let mut m = V2::ZERO;
        for (i, p) in self.p.iter().enumerate() {
            m += cell_center(i) * *p;
        }
        m
    }

    pub fn peak(&self) -> V2 {
        let i = (0..self.p.len()).max_by(|&a, &b| self.p[a].total_cmp(&self.p[b])).unwrap_or(0);
        cell_center(i)
    }

    pub fn prob_at(&self, p: V2) -> f32 {
        self.p[cell_of(p)]
    }
}

// ---------------------------------------------------------------------------------------------
// Plays, roles, orders, callouts.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Play {
    /// No coordination: everyone uses their own utility AI.
    Free,
    Overwatch,
    Crossfire,
    Trade,
    PinFlank,
    Collapse,
    Bait,
    /// Player lost and far away: converge on the belief peak along different routes.
    Sweep,
}

impl Play {
    pub fn label(self) -> &'static str {
        match self {
            Play::Free => "free",
            Play::Overwatch => "bounding overwatch",
            Play::Crossfire => "crossfire",
            Play::Trade => "trade",
            Play::PinFlank => "pin and flank",
            Play::Collapse => "collapse",
            Play::Bait => "bait",
            Play::Sweep => "sweep",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Bounding overwatch: hold the angle while the other team moves.
    Overwatch,
    /// Bounding overwatch: move to the next cover.
    Bound,
    Crossfire,
    Entry,
    Trader,
    Pin,
    Flanker,
    Collapser,
    Bait,
    Support,
    /// Holds and pre-aims the angle the player keeps re-peeking.
    Anchor,
    /// Throwing utility (winding up a frag or flashbang).
    Breacher,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Overwatch => "overwatch",
            Role::Bound => "bound",
            Role::Crossfire => "crossfire",
            Role::Entry => "entry",
            Role::Trader => "trader",
            Role::Pin => "pin",
            Role::Flanker => "flanker",
            Role::Collapser => "collapser",
            Role::Bait => "bait",
            Role::Support => "support",
            Role::Anchor => "anchor",
            Role::Breacher => "breacher",
        }
    }
}

/// A high-priority goal from the coordinator.
#[derive(Clone, Copy, Debug)]
pub struct Order {
    pub role: Role,
    pub goal: Goal,
    /// Where to go (None: the mind picks, or stays put for `Goal::Hold`).
    pub dest: Option<V2>,
    /// Angle to watch when not engaged.
    pub watch: Option<V2>,
    /// Path with tactical A* costs around this threat.
    pub threat: Option<V2>,
    /// Keep following the order while in contact (flanker, trader); others fight first.
    pub sticky: bool,
    /// Anchor only: fire a short burst at this spot now (he is due to re-peek it).
    pub prefire: Option<V2>,
}

/// The coordinator wants member `thrower` to release a grenade at `target` now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrowOrder {
    pub thrower: usize,
    pub kind: GrenadeKind,
    pub target: V2,
}

/// Grenades the squad may throw this round (difficulty).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UtilityBudget {
    pub frags: u32,
    pub flashes: u32,
}

impl UtilityBudget {
    pub const NONE: UtilityBudget = UtilityBudget { frags: 0, flashes: 0 };
    /// Default (the trainer and tests): two of each.
    pub const STANDARD: UtilityBudget = UtilityBudget { frags: 2, flashes: 2 };
}

/// A spot the player has re-peeked: lost there and seen again there within a few seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeekSpot {
    pub pos: V2,
    /// Re-peeks seen here.
    pub count: u32,
    /// Typical seconds from ducking out of sight to re-appearing.
    pub delay: f32,
    called: bool,
}

/// One bandit decision being scored (an engagement window).
#[derive(Clone, Copy, Debug)]
struct Episode {
    arm: Play,
    x: [f32; CTX_DIM],
    t: f32,
    dmg: f32,
    losses: u32,
    traded: u32,
    killed: bool,
    player_start: Option<V2>,
    centroid: V2,
}

/// A loss that a teammate can still trade within this long (s).
const TRADE_S: f32 = 3.0;
/// An engagement window ends (and is rewarded) after this long even if the play continues.
pub const EPISODE_S: f32 = 9.0;
/// Re-peek: seen again within this long (s) and this close (m) to where he vanished.
pub const REPEEK_S: f32 = 5.0;
pub const REPEEK_M: f32 = 3.0;
/// Pre-fire burst length (s of trigger time, about 3-4 rounds).
pub const PREFIRE_BURST_S: f32 = 0.32;
/// The player counts as camping after this long within `CAMP_M` of one spot.
pub const CAMP_S: f32 = 2.8;
pub const CAMP_M: f32 = 2.5;

#[derive(Clone, Debug, PartialEq)]
pub struct Callout {
    pub speaker: usize,
    pub text: String,
}

pub const CALLSIGNS: [&str; 6] = ["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot"];
pub fn callsign(slot: usize) -> &'static str {
    CALLSIGNS[slot % CALLSIGNS.len()]
}

/// One member as the coordinator sees it this tick (perception from the previous tick).
#[derive(Clone, Copy, Debug)]
pub struct MemberView {
    pub pos: V2,
    pub alive: bool,
    pub hp: f32,
    pub last_known: V2,
    pub last_seen: f32,
    pub heard: Option<V2>,
    pub look: V2,
    pub traits: Traits,
    /// A bark the mind wants to say (see `EnemyMind::take_bark`).
    pub bark: Option<&'static str>,
}

impl MemberView {
    /// A slot whose enemy is gone (dead and despawned).
    pub fn gone() -> Self {
        MemberView {
            pos: V2::ZERO,
            alive: false,
            hp: 0.0,
            last_known: V2::ZERO,
            last_seen: 99.0,
            heard: None,
            look: V2::ZERO,
            traits: Traits::default(),
            bark: None,
        }
    }
}

const REPLAN_S: f32 = 1.0;
const SWEEP_DIST: f32 = 18.0;
const MIN_PLAY_S: f32 = 3.0;
const KNOWN_S: f32 = 3.0;
/// A member's own sighting this fresh counts as "contact" for the callout.
const FRESH_CONTACT_S: f32 = 0.2;
const BELIEF_RANGE: f32 = 40.0;
const BELIEF_HALF_ANGLE: f32 = 1.05;
const BARK_GAP_S: f32 = 0.5;
const BARK_REPEAT_S: f32 = 5.0;
const MAX_QUEUED: usize = 16;

pub struct SquadCoordinator {
    /// Radio-delayed freshest sighting (shared with the minds through `merge`).
    pub board: Blackboard,
    pub belief: Belief,
    pub play: Play,
    pub play_t: f32,
    /// Claimed positions per member.
    pub claims: Vec<Option<V2>>,
    pub orders: Vec<Option<Order>>,
    /// Off: no plays (minds act alone); the belief and barks still run.
    pub enabled: bool,
    /// Tests: always run this play.
    pub force_play: Option<Play>,
    radio_t: f32,
    replan_t: f32,
    radio_ticks: u32,
    heard_t: f32,
    heard_at: Option<V2>,
    prev_heard: Vec<Option<V2>>,
    prev_alive: Vec<bool>,
    was_known: bool,
    bound_team: usize,
    bound_t: f32,
    /// Sticky role slots for the current play (entry/flanker/bait, then trader).
    lead: Option<usize>,
    second: Option<usize>,
    /// Breadcrumbs of the lead's movement (the trader follows them).
    trail: Vec<V2>,
    callouts: Vec<Callout>,
    bark_gap: f32,
    recent: Vec<(String, f32)>,
    time: f32,
    /// Attack tokens (DOOM / Amalur style): who may actively shoot and push right now.
    pub tokens: Vec<bool>,
    token_t: Vec<f32>,
    /// Cap on simultaneous attackers (difficulty), on top of `token_budget`.
    pub max_tokens: usize,

    // --- Learned play selection.
    /// The contextual bandit. Callers copy it in from `BrainState::tactics` and back out after
    /// the round (`end_round`), so it learns across rounds and matches.
    pub tactics: TacticsLearner,
    /// Update the bandit from rewards (off: choose only).
    pub learn: bool,
    /// The player's health 0..1 (set by the caller each tick; part of the context).
    pub player_hp: f32,
    /// Last bandit scores (debug / control port) and the context they used.
    pub last_scores: Vec<ArmScore>,
    pub last_ctx: [f32; CTX_DIM],
    zone_focus: Option<f32>,
    episode: Option<Episode>,
    /// Times of recent losses not yet traded.
    untraded: Vec<f32>,
    /// Camping detector over radio-delayed sightings.
    still_at: V2,
    pub still_t: f32,
    still_samples: f32,
    known_samples: f32,

    // --- Re-peek tracking and anchors.
    pub peeks: Vec<PeekSpot>,
    seen_prev: bool,
    last_fresh: V2,
    /// (position, time) where the player was last lost from sight.
    vanish: Option<(V2, f32)>,
    /// Seconds since anyone had a fresh sighting.
    pub unseen_t: f32,
    repeeks: f32,
    reappears: f32,
    anchor: Option<usize>,

    // --- Utility (grenades).
    pub utility: UtilityBudget,
    pub frags_used: u32,
    pub flashes_used: u32,
    /// Per member: (has thrown a frag, has thrown a flash) this round.
    thrown: Vec<(bool, bool)>,
    throw_cd: f32,
    /// A throw being wound up: the order and seconds until release.
    pending_throw: Option<(ThrowOrder, f32)>,
    throws: Vec<ThrowOrder>,
    /// A flashbang is wanted before a push (set when a flank or entry starts).
    flash_wanted: f32,
    /// Tests / control port: throw this as soon as anyone can.
    pub force_throw: Option<GrenadeKind>,
}

/// At most this many members hold attack tokens at once (fewer in small squads).
pub const MAX_TOKENS: usize = 3;

/// Token budget for `alive` members: everyone up to 2, then 2 of 3, then 3.
pub fn token_budget(alive: usize) -> usize {
    match alive {
        0..=2 => alive,
        3 => 2,
        _ => MAX_TOKENS,
    }
}

impl SquadCoordinator {
    pub fn new(arena: &Arena, n: usize) -> Self {
        SquadCoordinator {
            board: Blackboard::default(),
            belief: Belief::new(arena, arena.player_spawn),
            play: Play::Free,
            play_t: 0.0,
            claims: vec![None; n],
            orders: vec![None; n],
            enabled: true,
            force_play: None,
            radio_t: 0.0,
            replan_t: 0.0,
            radio_ticks: 0,
            heard_t: 99.0,
            heard_at: None,
            prev_heard: vec![None; n],
            prev_alive: vec![true; n],
            was_known: false,
            bound_team: 0,
            bound_t: 0.0,
            lead: None,
            second: None,
            trail: Vec::new(),
            callouts: Vec::new(),
            bark_gap: 0.0,
            recent: Vec::new(),
            time: 0.0,
            tokens: vec![true; n],
            token_t: vec![0.0; n],
            max_tokens: MAX_TOKENS,
            tactics: TacticsLearner::default(),
            learn: true,
            player_hp: 1.0,
            last_scores: Vec::new(),
            last_ctx: [0.0; CTX_DIM],
            zone_focus: None,
            episode: None,
            untraded: Vec::new(),
            still_at: V2::ZERO,
            still_t: 0.0,
            still_samples: 0.0,
            known_samples: 0.0,
            peeks: Vec::new(),
            seen_prev: false,
            last_fresh: V2::ZERO,
            vanish: None,
            unseen_t: 99.0,
            repeeks: 0.0,
            reappears: 0.0,
            anchor: None,
            utility: UtilityBudget::STANDARD,
            frags_used: 0,
            flashes_used: 0,
            thrown: vec![(false, false); n],
            throw_cd: 3.0,
            pending_throw: None,
            throws: Vec::new(),
            flash_wanted: 0.0,
            force_throw: None,
        }
    }

    /// Seed re-peek spots seen earlier in the match (e.g. from the duel log): (position, count).
    pub fn seed_peeks(&mut self, spots: &[(V2, u32)]) {
        for &(p, c) in spots {
            self.add_peek(p, c, 1.5, false);
        }
    }

    fn add_peek(&mut self, p: V2, count: u32, delay: f32, live: bool) {
        match self.peeks.iter_mut().find(|s| s.pos.dist(p) < REPEEK_M) {
            Some(s) => {
                s.pos = s.pos.lerp(p, 0.3);
                s.count += count;
                s.delay = if live { 0.6 * s.delay + 0.4 * delay } else { s.delay };
            }
            None => self.peeks.push(PeekSpot { pos: p, count, delay, called: false }),
        }
    }

    /// The player took `amount` damage from the squad (bullets or grenades): bandit reward.
    pub fn player_damaged(&mut self, amount: f32) {
        if let Some(e) = self.episode.as_mut() {
            e.dmg += amount;
        }
        // A hit soon after a loss trades it.
        let now = self.time;
        if let Some(k) = self.untraded.iter().position(|&t| now - t <= TRADE_S) {
            self.untraded.remove(k);
            if let Some(e) = self.episode.as_mut() {
                e.traded += 1;
            }
        }
    }

    /// Round over: score the open engagement window (`player_dead`: the squad won it).
    pub fn end_round(&mut self, player_dead: bool) {
        if let Some(e) = self.episode.as_mut() {
            e.killed |= player_dead;
        }
        self.close_episode(None);
    }

    /// Grenades to release now (the caller spawns them at the thrower).
    pub fn take_throws(&mut self) -> Vec<ThrowOrder> {
        std::mem::take(&mut self.throws)
    }

    /// The re-peek spot the player just ducked away from, if he is due back there.
    pub fn hot_peek(&self) -> Option<PeekSpot> {
        let (v, _) = self.vanish?;
        if !(0.25..6.0).contains(&self.unseen_t) {
            return None;
        }
        self.peeks.iter().copied().filter(|s| s.pos.dist(v) < REPEEK_M + 0.5).max_by_key(|s| s.count)
    }

    /// Context for the bandit from the model and the live match.
    pub fn context(&mut self, members: &[MemberView], arena: &Arena, model: &PlayerModel) -> [f32; CTX_DIM] {
        let zf = *self.zone_focus.get_or_insert_with(|| bandit::zone_focus(model, arena));
        let alive: Vec<&MemberView> = members.iter().filter(|m| m.alive).collect();
        let live = LiveCtx {
            live_still: (self.known_samples >= 5.0).then(|| self.still_samples / self.known_samples),
            repeeks: self.repeeks,
            reappears: self.reappears,
            alive_ratio: alive.len() as f32 / members.len().max(1) as f32,
            squad_hp: alive.iter().map(|m| m.hp).sum::<f32>() / alive.len().max(1) as f32,
            player_hp: self.player_hp,
        };
        bandit::context(model, zf, &live)
    }

    fn centroid(members: &[MemberView]) -> V2 {
        let alive: Vec<V2> = members.iter().filter(|m| m.alive).map(|m| m.pos).collect();
        let n = alive.len().max(1) as f32;
        alive.iter().fold(V2::ZERO, |a, p| a + *p) * (1.0 / n)
    }

    /// Reward the open episode and update the bandit. `members`: for the forced-back check.
    fn close_episode(&mut self, members: Option<&[MemberView]>) {
        let Some(e) = self.episode.take() else { return };
        let dur = self.time - e.t;
        if dur < 1.0 && e.dmg <= 0.0 && e.losses == 0 && !e.killed {
            return; // too short to say anything
        }
        let untraded = e.losses.saturating_sub(e.traded) as f32;
        // Forced back: he ended farther from where the squad stood, without the squad losing anyone.
        let known_now = (self.board.age < KNOWN_S).then_some(self.board.pos);
        let forced = match (e.player_start, known_now, members) {
            (Some(a), Some(b), Some(_)) if e.losses == 0 => b.dist(e.centroid) - a.dist(e.centroid) > 3.0,
            _ => false,
        };
        let r = (e.dmg / 50.0).min(1.0) + if e.killed { 0.6 } else { 0.0 } + if forced { 0.25 } else { 0.0 } - 0.5 * untraded - 0.2 * e.traded as f32;
        if self.learn {
            self.tactics.update(e.arm, &e.x, r);
        }
    }

    /// Damage is a sense: member `victim` was hit, so the shooter's position (the player's
    /// position at shot time) goes on the board for everyone at once, and the squad fights.
    pub fn reveal(&mut self, victim: usize, shooter: V2, arena: &Arena) {
        let fresh = self.board.alert == Alert::Combat && self.board.age < 1.0;
        self.board.reveal(shooter);
        self.belief.sighting(shooter);
        if !fresh {
            self.bark(victim, format!("Taking fire - {}", where_text(arena, shooter)), true);
        }
    }

    /// Member `who` heard `h`: relay it (board + belief with the sound's uncertainty).
    pub fn hear(&mut self, who: usize, h: &Heard, arena: &Arena) {
        let before = self.board.alert;
        self.board.hear(h);
        self.belief.heard_with(h.pos, heard_sigma(h.loud));
        self.heard_at = Some(h.pos);
        self.heard_t = 0.0;
        if before != Alert::Combat {
            let text = match h.sound {
                Sound::Gunshot => format!("Shots - {}", where_text(arena, h.pos)),
                Sound::Footstep => "Footsteps - checking it".to_string(),
                Sound::Reload => "Heard a reload".to_string(),
            };
            self.bark(who, text, h.sound == Sound::Gunshot);
        }
    }

    /// Re-deal attack tokens: prefer members in contact, current holders (hysteresis), close
    /// to the player and healthy. Out of combat everyone may shoot (tokens only ration a fight).
    fn assign_tokens(&mut self, dt: f32, members: &[MemberView]) {
        let n = members.len();
        self.tokens.resize(n, true);
        self.token_t.resize(n, 0.0);
        let alive: Vec<usize> = (0..n).filter(|&i| members[i].alive).collect();
        // Tokens are a fairness budget, so they apply with or without plays (`enabled`).
        if self.board.alert != Alert::Combat {
            self.tokens.iter_mut().for_each(|t| *t = true);
            return;
        }
        let target = self.board.pos;
        let score = |i: usize| {
            let m = &members[i];
            let contact = if m.last_seen < 1.0 { 2.0 } else { 0.0 };
            // Holders keep it for a while, then it rotates.
            let hold = if self.tokens[i] && self.token_t[i] < 6.0 { 1.0 } else { 0.0 };
            contact + hold + 1.0 / (1.0 + m.pos.dist(target) / 10.0) + 0.5 * m.hp
        };
        let mut ranked = alive.clone();
        ranked.sort_by(|&a, &b| score(b).total_cmp(&score(a)).then(a.cmp(&b)));
        let k = token_budget(alive.len()).min(self.max_tokens.max(1));
        let mut next = vec![false; n];
        for &i in ranked.iter().take(k) {
            next[i] = true;
        }
        for ((t, &now), &had) in self.token_t.iter_mut().zip(&next).zip(&self.tokens).take(n) {
            *t = if now && had { *t + dt } else { 0.0 };
        }
        self.tokens = next;
    }

    /// Callouts produced since the last call.
    pub fn drain_callouts(&mut self) -> Vec<Callout> {
        std::mem::take(&mut self.callouts)
    }

    /// Best guess at where the player is: a fresh radioed sighting, else the belief.
    pub fn predicted(&self, arena: &Arena) -> V2 {
        if self.board.age < 1.5 { self.board.pos } else { arena.nav.nearest_walkable(self.belief.mean()) }
    }

    fn bark(&mut self, speaker: usize, text: String, urgent: bool) {
        if !urgent && self.bark_gap > 0.0 {
            return;
        }
        if self.recent.iter().any(|(t, at)| *t == text && self.time - at < BARK_REPEAT_S) {
            return;
        }
        self.recent.retain(|(_, at)| self.time - at < BARK_REPEAT_S);
        self.recent.push((text.clone(), self.time));
        self.bark_gap = BARK_GAP_S;
        if self.callouts.len() >= MAX_QUEUED {
            self.callouts.remove(0);
        }
        self.callouts.push(Callout { speaker, text });
    }

    /// Advance one tick and (re)assign orders. Call before the minds think.
    pub fn tick(&mut self, dt: f32, members: &[MemberView], arena: &Arena, model: &PlayerModel) -> &[Option<Order>] {
        let n = members.len();
        self.claims.resize(n, None);
        self.orders.resize(n, None);
        self.prev_heard.resize(n, None);
        self.prev_alive.resize(n, true);
        self.thrown.resize(n, (false, false));
        self.time += dt;
        self.throw_cd -= dt;
        self.flash_wanted -= dt;
        self.play_t += dt;
        self.bound_t += dt;
        self.heard_t += dt;
        self.bark_gap -= dt;
        self.board.tick(dt, members.iter().filter(|m| m.alive).map(|m| (m.last_known, m.last_seen)));
        self.assign_tokens(dt, members);

        // Contact / lost callouts every tick, from the freshest *real* sighting (never the
        // belief peak or a stale prior), so the zone named is where he actually is.
        let spotter = (0..n).filter(|&i| members[i].alive).min_by(|&a, &b| members[a].last_seen.total_cmp(&members[b].last_seen));
        let known = self.board.age < KNOWN_S;
        if let Some(s) = spotter {
            let m = &members[s];
            if m.last_seen < FRESH_CONTACT_S && !self.was_known {
                self.bark(s, contact_text(arena, m.pos, m.last_known), true);
            } else if !known && self.was_known {
                self.bark(s, format!("Lost him - last seen {}", where_text(arena, self.board.pos)), false);
            }
        }
        self.was_known = known || spotter.is_some_and(|s| members[s].last_seen < FRESH_CONTACT_S);
        self.track_peeks(dt, members, spotter);

        // Barks every tick: man down, the minds' own.
        for i in 0..n {
            if self.prev_alive[i] && !members[i].alive {
                self.orders[i] = None;
                self.claims[i] = None;
                if let Some(e) = self.episode.as_mut() {
                    e.losses += 1;
                }
                self.untraded.push(self.time);
                if let Some(s) = nearest_alive(members, members[i].pos) {
                    self.bark(s, format!("Man down - {} is down", callsign(i)), true);
                }
            }
            self.prev_alive[i] = members[i].alive;
            if let (true, Some(b)) = (members[i].alive, members[i].bark) {
                self.bark(i, b.to_string(), false);
            }
        }

        // Radio tick: update the belief with the delay the minds share.
        self.radio_t -= dt;
        if self.radio_t > 0.0 {
            return &self.orders;
        }
        self.radio_t = RADIO_PERIOD_S;
        self.radio_ticks += 1;
        let now = self.time;
        self.untraded.retain(|&t| now - t <= TRADE_S);
        // Camping detector: radioed sightings that stay put.
        if self.board.age < 1.0 {
            self.known_samples += 1.0;
            if self.board.pos.dist(self.still_at) < CAMP_M {
                self.still_t += RADIO_PERIOD_S;
                self.still_samples += 1.0;
            } else {
                self.still_at = self.board.pos;
                self.still_t = 0.0;
            }
        } else if self.board.age > KNOWN_S {
            self.still_t = 0.0;
        }
        self.belief.diffuse(RADIO_PERIOD_S);
        if self.board.age > KNOWN_S {
            self.belief.habit_prior(model, 0.03);
        }
        let mut sighted = false;
        for (i, m) in members.iter().enumerate() {
            if !m.alive {
                continue;
            }
            if m.last_seen < RADIO_PERIOD_S {
                self.belief.sighting(m.last_known);
                sighted = true;
            }
            if m.heard.is_some() && m.heard != self.prev_heard[i] {
                let h = m.heard.unwrap_or_default();
                self.prev_heard[i] = m.heard;
                if self.heard_t > 1.0 || self.heard_at.is_some_and(|a| a.dist(h) > 4.0) {
                    if !sighted {
                        self.belief.heard(h);
                    }
                    self.heard_at = Some(h);
                    self.heard_t = 0.0;
                }
            }
        }
        if !sighted {
            // Negative information is only cheap enough every other radio tick.
            if self.radio_ticks.is_multiple_of(2) {
                for m in members.iter().filter(|m| m.alive) {
                    self.belief.clear_seen(arena, m.pos, m.look, BELIEF_RANGE, BELIEF_HALF_ANGLE);
                }
            }
        }

        self.replan_t -= RADIO_PERIOD_S;
        let full = self.replan_t <= 0.0;
        if full {
            self.replan_t = REPLAN_S;
        }
        self.plan(members, arena, model, full);
        &self.orders
    }

    /// Re-peek detection from the squad's fresh sightings: lost at a spot, seen again there soon.
    fn track_peeks(&mut self, dt: f32, members: &[MemberView], spotter: Option<usize>) {
        match spotter.filter(|&s| members[s].last_seen < FRESH_CONTACT_S) {
            Some(s) => {
                let at = members[s].last_known;
                if !self.seen_prev
                    && self.unseen_t > 0.5
                    && let Some((v, vt)) = self.vanish
                {
                    self.reappears += 1.0;
                    let gap = self.time - vt;
                    if gap <= REPEEK_S && at.dist(v) <= REPEEK_M {
                        self.repeeks += 1.0;
                        self.add_peek((v + at) * 0.5, 1, gap, true);
                    }
                }
                self.unseen_t = 0.0;
                self.last_fresh = at;
                self.seen_prev = true;
            }
            None => {
                if self.seen_prev {
                    self.vanish = Some((self.last_fresh, self.time));
                }
                self.seen_prev = false;
                self.unseen_t += dt;
            }
        }
    }

    fn choose_play(&mut self, members: &[MemberView], arena: &Arena, model: &PlayerModel) -> Play {
        let alive = members.iter().filter(|m| m.alive).count();
        if !self.enabled || alive == 0 {
            return Play::Free;
        }
        let known = self.board.age < KNOWN_S;
        if alive == 1 && self.force_play.is_none() {
            // A lone survivor still searches with the squad's belief.
            return if known { Play::Free } else { Play::Sweep };
        }
        if let Some(p) = self.force_play {
            return p;
        }
        // Unaware with nothing to go on: everyone hunts on their own (faster than a sweep).
        if !known && self.board.alert == Alert::Unaware && self.heard_t >= 2.0 {
            return Play::Free;
        }
        // In a fight, a lost player is hunted down fast (collapse), not bounded toward.
        if !known && (self.heard_t < 2.0 || self.board.alert == Alert::Combat) {
            return Play::Collapse;
        }
        if !known {
            // Bound with overwatch near where he probably is; sweep there quickly when far.
            let peak = self.belief.peak();
            let near = members.iter().filter(|m| m.alive).any(|m| m.pos.dist(peak) < SWEEP_DIST);
            return if near { Play::Overwatch } else { Play::Sweep };
        }
        if self.play_t < MIN_PLAY_S && matches!(self.play, Play::Crossfire | Play::Trade | Play::PinFlank | Play::Bait) {
            return self.play;
        }
        let k = alive as f32;
        let avg = |f: fn(&Traits) -> f32| members.iter().filter(|m| m.alive).map(|m| f(&m.traits)).sum::<f32>() / k;
        let (aggr, flank, coh, pred) = (avg(|t| t.aggression), avg(|t| t.flank_bias), avg(|t| t.team_cohesion), avg(|t| t.prediction_trust));
        // The hand weights are the bandit's prior (they fade as an arm gathers rewards).
        let mut prior = [Some(0.35 + 0.4 * coh), Some(0.2 + 0.6 * flank), Some(0.15 + 0.5 * aggr), (alive >= 3).then(|| 0.05 + 0.5 * pred * model.confidence())];
        if let Some(k) = bandit::arm_of(self.play) {
            prior[k] = prior[k].map(|p| p + 0.15);
        }
        let x = self.context(members, arena, model);
        self.last_scores = self.tactics.scores(&x, &prior);
        self.last_ctx = x;
        self.tactics.choose(&x, &prior).unwrap_or(Play::Crossfire)
    }

    fn plan(&mut self, members: &[MemberView], arena: &Arena, model: &PlayerModel, full: bool) {
        let play = self.choose_play(members, arena, model);
        // Engagement windows: score the bandit's choice when the play changes or the window ends.
        let expired = self.episode.is_some_and(|e| self.time - e.t > EPISODE_S);
        if play != self.play || expired {
            self.close_episode(Some(members));
        }
        self.plan_play(play, members, arena, model, full);
        if self.play == Play::Free {
            self.anchor = None;
            self.pending_throw = None;
            return;
        }
        self.plan_anchor(members, arena);
        self.plan_utility(members, arena);
        if self.episode.is_none() && self.force_play.is_none() && bandit::arm_of(self.play).is_some() {
            self.episode = Some(Episode {
                arm: self.play,
                x: self.last_ctx,
                t: self.time,
                dmg: 0.0,
                losses: 0,
                traded: 0,
                killed: false,
                player_start: (self.board.age < KNOWN_S).then_some(self.board.pos),
                centroid: Self::centroid(members),
            });
        }
    }

    /// An anchor holds the angle the player keeps re-peeking, and pre-fires when he is due.
    fn plan_anchor(&mut self, members: &[MemberView], arena: &Arena) {
        let Some(spot) = self.hot_peek() else {
            self.anchor = None;
            return;
        };
        let thrower = self.pending_throw.map(|(t, _)| t.thrower);
        let busy = |i: usize| Some(i) == self.lead || Some(i) == self.second || Some(i) == thrower;
        let sees = |i: usize| {
            let m = &members[i];
            m.alive && (4.0..40.0).contains(&m.pos.dist(spot.pos)) && arena.line_of_sight(m.pos, false, spot.pos, false)
        };
        let keep = self.anchor.filter(|&i| i < members.len() && sees(i) && !busy(i));
        let pick = keep.or_else(|| {
            (0..members.len())
                .filter(|&i| sees(i) && !busy(i))
                .min_by(|&a, &b| members[a].pos.dist(spot.pos).total_cmp(&members[b].pos.dist(spot.pos)).then(a.cmp(&b)))
        });
        self.anchor = pick;
        let Some(i) = pick else { return };
        // Pre-fire a little before his usual re-peek delay; only once he has done it twice.
        let w0 = (spot.delay * 0.75).max(0.5);
        let prefire = (spot.count >= 2 && (w0..w0 + 0.7).contains(&self.unseen_t)).then_some(spot.pos);
        self.orders[i] = Some(Order { role: Role::Anchor, goal: Goal::Hold, dest: None, watch: Some(spot.pos), threat: None, sticky: true, prefire });
        if spot.count >= 2
            && let Some(s) = self.peeks.iter_mut().find(|s| s.pos == spot.pos && !s.called)
        {
            s.called = true;
            let text = format!("He keeps re-peeking {} - holding it", spot_noun(arena, spot.pos));
            self.bark(i, text, true);
        }
    }

    /// Breacher: frag a player camping cover; flashbang ahead of a flank or entry.
    fn plan_utility(&mut self, members: &[MemberView], arena: &Arena) {
        if let Some((t, left)) = self.pending_throw {
            if !members.get(t.thrower).is_some_and(|m| m.alive) {
                self.pending_throw = None;
                return;
            }
            self.orders[t.thrower] =
                Some(Order { role: Role::Breacher, goal: Goal::Hold, dest: None, watch: Some(t.target), threat: None, sticky: true, prefire: None });
            let left = left - RADIO_PERIOD_S;
            if left > 0.0 {
                self.pending_throw = Some((t, left));
                return;
            }
            self.pending_throw = None;
            self.throws.push(t);
            self.throw_cd = grenade::SQUAD_COOLDOWN_S;
            let text = match t.kind {
                GrenadeKind::Frag => {
                    self.frags_used += 1;
                    self.thrown[t.thrower].0 = true;
                    if self.still_t >= CAMP_S {
                        match arena.zone_of(t.target) {
                            Some(z) => format!("He's camping {} - frag out", z.name),
                            None => "He's camping - frag out".to_string(),
                        }
                    } else {
                        "Frag out!".to_string()
                    }
                }
                GrenadeKind::Flash => {
                    self.flashes_used += 1;
                    self.thrown[t.thrower].1 = true;
                    self.flash_wanted = 0.0;
                    "Flashbang!".to_string()
                }
            };
            self.bark(t.thrower, text, true);
            return;
        }
        let forced = self.force_throw;
        if self.throw_cd > 0.0 && forced.is_none() {
            return;
        }
        let known = self.board.age < 1.5;
        let target = if known { self.board.pos } else { self.predicted(arena) };
        let kind = match forced {
            Some(k) => k,
            None if !known => return,
            None => {
                let camping = self.still_t >= CAMP_S;
                let hugging = self.still_t >= 2.0 && classify_cover(arena, target, Self::centroid(members)).0 != CoverState::Exposed;
                if self.frags_used < self.utility.frags && (camping || hugging) {
                    GrenadeKind::Frag
                } else if self.flashes_used < self.utility.flashes && self.flash_wanted > 0.0 {
                    GrenadeKind::Flash
                } else {
                    return;
                }
            }
        };
        let frag = kind == GrenadeKind::Frag;
        let mut best: Option<(f32, usize, V2)> = None;
        for (i, m) in members.iter().enumerate() {
            let used = if frag { self.thrown[i].0 } else { self.thrown[i].1 };
            if !m.alive || (used && forced.is_none()) || Some(i) == self.lead || Some(i) == self.anchor {
                continue;
            }
            // A flash lands a step in front of him, on the thrower's side, so he sees it pop.
            let aim = if frag { target } else { target + (m.pos - target).norm() * 1.5 };
            let d = m.pos.dist(aim);
            if !(grenade::THROW_MIN..grenade::THROW_MAX).contains(&d) || grenade::landing(arena, m.pos, aim).dist(aim) > 2.0 {
                continue;
            }
            // Never frag a spot a teammate stands in.
            if frag && members.iter().any(|o| o.alive && o.pos.dist(aim) < grenade::FRAG_RADIUS + 1.0) {
                continue;
            }
            let will = if frag { m.traits.aggression } else { m.traits.team_cohesion };
            let score = 0.6 * will + 0.4 * (1.0 - d / grenade::THROW_MAX);
            if best.is_none_or(|(b, _, _)| score > b) {
                best = Some((score, i, aim));
            }
        }
        // Willingness is evolvable (aggression for frags, cohesion for flashes).
        let Some((_, i, aim)) = best.filter(|b| forced.is_some() || b.0 >= 0.3) else { return };
        self.force_throw = None;
        self.pending_throw = Some((ThrowOrder { thrower: i, kind, target: aim }, grenade::WINDUP_S));
        self.orders[i] = Some(Order { role: Role::Breacher, goal: Goal::Hold, dest: None, watch: Some(aim), threat: None, sticky: true, prefire: None });
    }

    fn plan_play(&mut self, play: Play, members: &[MemberView], arena: &Arena, model: &PlayerModel, full: bool) {
        let changed = play != self.play;
        if changed {
            self.play = play;
            self.play_t = 0.0;
            self.claims.iter_mut().for_each(|c| *c = None);
            self.bound_t = 0.0;
            self.lead = None;
            self.second = None;
            self.trail.clear();
        }
        let dead = |s: Option<usize>| s.is_none_or(|i| !members.get(i).is_some_and(|m| m.alive));
        if dead(self.lead) {
            self.lead = None;
        }
        if dead(self.second) || self.second == self.lead {
            self.second = None;
        }
        let alive: Vec<usize> = (0..members.len()).filter(|&i| members[i].alive).collect();
        if play == Play::Free {
            self.orders.iter_mut().for_each(|o| *o = None);
            return;
        }
        let target = match play {
            Play::Overwatch | Play::Sweep => arena.nav.nearest_walkable(self.belief.peak()),
            // The freshest of the last sound and the last sighting.
            Play::Collapse => self.heard_at.filter(|_| self.heard_t < self.board.age).unwrap_or(self.board.pos),
            _ => self.predicted(arena),
        };
        // Members closest to the target first.
        let mut by_dist = alive.clone();
        by_dist.sort_by(|&a, &b| members[a].pos.dist(target).total_cmp(&members[b].pos.dist(target)).then(a.cmp(&b)));
        let hold = |dest: Option<V2>, role: Role| Order { role, goal: Goal::Hold, dest, watch: Some(target), threat: Some(target), sticky: false, prefire: None };

        match play {
            Play::Free => {}
            Play::Crossfire => {
                if full || changed {
                    self.assign_positions(&by_dist, members, arena, model, target, None);
                }
                // The two closest are the crossfire pair (> 60 deg apart); the rest support.
                for (r, &i) in by_dist.iter().enumerate() {
                    self.orders[i] = Some(hold(self.claims[i], if r < 2 { Role::Crossfire } else { Role::Support }));
                }
                if changed {
                    self.bark(by_dist[0], "Spread out - crossfire".into(), true);
                }
            }
            Play::Trade => {
                let entry = *self.lead.get_or_insert(by_dist[0]);
                if self.second.is_none() {
                    self.second = by_dist.iter().copied().find(|&i| i != entry);
                }
                let second = self.second;
                let ep = members[entry].pos;
                let away = (ep - target).norm();
                self.orders[entry] = Some(Order { role: Role::Entry, goal: Goal::Push, dest: None, watch: Some(target), threat: None, sticky: false, prefire: None });
                if let Some(tr) = second {
                    if self.trail.last().is_none_or(|l| l.dist(ep) > 0.5) {
                        self.trail.push(ep);
                    }
                    // ~3 m back along the entry's own trail (the follower lags ~2-3 m while strafing); before it has one, 5 m behind it.
                    let follow = trail_point(&self.trail, 3.0).unwrap_or_else(|| {
                        [0.0f32, 0.5, -0.5, 1.0, -1.0]
                            .iter()
                            .map(|&da| ep + V2::from_angle(away.angle() + da) * 5.0)
                            .find(|p| arena.nav.walkable_at(*p) && arena.nav.clear_line(ep, *p))
                            .unwrap_or_else(|| arena.nav.nearest_walkable(ep + away * 5.0))
                    });
                    self.orders[tr] = Some(Order { role: Role::Trader, goal: Goal::Hold, dest: Some(follow), watch: Some(target), threat: None, sticky: true, prefire: None });
                }
                let rest: Vec<usize> = by_dist.iter().copied().filter(|&i| i != entry && Some(i) != second).collect();
                if full || changed {
                    self.assign_positions(&rest, members, arena, model, target, Some(ep));
                }
                for &i in &rest {
                    self.orders[i] = Some(hold(self.claims[i], Role::Support));
                }
                if changed {
                    self.bark(entry, "Going in - trade me".into(), true);
                    self.flash_wanted = 4.0;
                }
            }
            Play::PinFlank => {
                let flanker = *self.lead.get_or_insert_with(|| {
                    *alive
                        .iter()
                        .max_by(|&&a, &&b| members[a].traits.flank_bias.total_cmp(&members[b].traits.flank_bias).then(b.cmp(&a)))
                        .unwrap_or(&alive[0])
                });
                if full || changed || self.claims[flanker].is_none() {
                    let (sector, dest) = least_watched_flank(arena, model, target, members[flanker].traits.preferred_range);
                    self.claims[flanker] = Some(dest);
                    if changed {
                        self.bark(flanker, format!("Flanking {}", SECTORS[sector]), true);
                        self.flash_wanted = 4.0;
                    }
                }
                self.orders[flanker] =
                    Some(Order { role: Role::Flanker, goal: Goal::Flank, dest: self.claims[flanker], watch: Some(target), threat: Some(target), sticky: true, prefire: None });
                let pins: Vec<usize> = by_dist.iter().copied().filter(|&i| i != flanker).collect();
                if full || changed {
                    self.assign_positions(&pins, members, arena, model, target, None);
                }
                for &i in &pins {
                    self.orders[i] = Some(hold(self.claims[i], Role::Pin));
                }
                if changed && let Some(&p) = pins.first() {
                    self.bark(p, "Pinning him - go".into(), false);
                }
            }
            Play::Collapse | Play::Sweep => {
                // Converge along different bearings so they come from different routes.
                let mut by_bearing = alive.clone();
                by_bearing.sort_by(|&a, &b| (members[a].pos - target).angle().total_cmp(&(members[b].pos - target).angle()));
                let k = by_bearing.len() as f32;
                for (r, &i) in by_bearing.iter().enumerate() {
                    let ang = (members[i].pos - target).angle() + (r as f32 - (k - 1.0) * 0.5) * 0.35;
                    let dest = arena.nav.nearest_walkable(target + V2::from_angle(ang) * 5.0);
                    self.orders[i] = Some(Order { role: Role::Collapser, goal: Goal::Hunt, dest: Some(dest), watch: Some(target), threat: Some(target), sticky: false, prefire: None });
                }
                if changed {
                    let z = arena.zone_of(target).map_or("there", |z| z.name);
                    let text = if play == Play::Collapse { format!("Shots at {z} - collapse") } else { format!("Checking {z}") };
                    self.bark(by_dist[0], text, play == Play::Collapse);
                }
            }
            Play::Bait => {
                let bait = *self
                    .lead
                    .get_or_insert_with(|| *alive.iter().max_by(|&&a, &&b| members[a].hp.total_cmp(&members[b].hp).then(b.cmp(&a))).unwrap_or(&alive[0]));
                let r = model.weakest_range().clamp(6.0, 28.0);
                let dest = arena.nav.nearest_walkable(target + (members[bait].pos - target).norm() * r);
                self.orders[bait] = Some(Order { role: Role::Bait, goal: Goal::Push, dest: Some(dest), watch: Some(target), threat: None, sticky: true, prefire: None });
                let rest: Vec<usize> = by_dist.iter().copied().filter(|&i| i != bait).collect();
                if full || changed {
                    self.assign_positions(&rest, members, arena, model, target, Some(dest));
                }
                for &i in &rest {
                    self.orders[i] = Some(hold(self.claims[i], Role::Crossfire));
                }
                if changed {
                    self.bark(bait, "I'll draw him out".into(), true);
                }
            }
            Play::Overwatch => {
                // Two teams alternate: one holds the angle, the other bounds to the next cover.
                let movers: Vec<usize> = alive.iter().enumerate().filter(|(r, _)| r % 2 == self.bound_team).map(|x| *x.1).collect();
                let arrived = movers.iter().all(|&i| self.claims[i].is_some_and(|c| c.dist(members[i].pos) < 1.5));
                if self.bound_t > 6.0 || (arrived && self.bound_t > 1.0) {
                    self.bound_team ^= 1;
                    self.bound_t = 0.0;
                    self.claims.iter_mut().for_each(|c| *c = None);
                    return self.plan_play(play, members, arena, model, full);
                }
                let mut mates: Vec<V2> = Vec::new();
                for &i in &alive {
                    if movers.contains(&i) {
                        if self.claims[i].is_none() {
                            self.claims[i] = bound_point(arena, model, &members[i], target, &mates);
                            if self.bound_t < 0.5 {
                                self.bark(i, "Moving - cover me".into(), false);
                            }
                        }
                        if let Some(c) = self.claims[i] {
                            mates.push(c);
                        }
                        self.orders[i] = Some(Order { role: Role::Bound, goal: Goal::Hold, dest: self.claims[i], watch: Some(target), threat: Some(target), sticky: false, prefire: None });
                    } else {
                        self.claims[i] = None;
                        self.orders[i] = Some(hold(None, Role::Overwatch));
                    }
                }
            }
        }
    }

    /// Fill `who` (in priority order) with PEF-best positions, crossfire-separated (> 60°).
    fn assign_positions(&mut self, who: &[usize], members: &[MemberView], arena: &Arena, model: &PlayerModel, target: V2, avoid: Option<V2>) {
        let mut mates: Vec<V2> = avoid.into_iter().collect();
        for (k, &i) in who.iter().enumerate() {
            let m = &members[i];
            let c = PefCtx { arena, model, target, me: m.pos, pref_range: m.traits.preferred_range, mates: &mates };
            // The first pair must cross (> 60 deg); later members just spread out.
            let min_sep = if mates.len() < 2 && k < 2 { PI / 3.0 + 0.1 } else { 0.6 };
            let p = best_position(&c, &m.traits.pef, min_sep);
            self.claims[i] = p;
            if let Some(p) = p {
                mates.push(p);
            }
        }
    }
}

/// The point `back` metres back along a trail (latest point last), if the trail is that long.
fn trail_point(trail: &[V2], back: f32) -> Option<V2> {
    let mut left = back;
    for w in trail.windows(2).rev() {
        let seg = w[1].dist(w[0]);
        if seg >= left {
            return Some(w[1].lerp(w[0], left / seg.max(1e-6)));
        }
        left -= seg;
    }
    None
}

fn nearest_alive(members: &[MemberView], to: V2) -> Option<usize> {
    (0..members.len()).filter(|&i| members[i].alive).min_by(|&a, &b| members[a].pos.dist(to).total_cmp(&members[b].pos.dist(to)))
}

/// "that crate" / "that corner": the nearest block to a re-peek spot, in callout words.
fn spot_noun(arena: &Arena, p: V2) -> &'static str {
    let near = arena
        .blocks
        .iter()
        .filter(|b| b.kind != BlockKind::Wall)
        .map(|b| {
            let dx = ((p.x - b.center.x).abs() - b.half.x).max(0.0);
            let dz = ((p.y - b.center.y).abs() - b.half.y).max(0.0);
            (b.kind, dx.hypot(dz))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    match near {
        Some((BlockKind::Low, d)) if d < 2.5 => "that crate",
        Some((_, d)) if d < 2.5 => "that corner",
        _ => "that angle",
    }
}

/// "at the west lane" / "close" for a spot outside any zone.
fn where_text(arena: &Arena, p: V2) -> String {
    match arena.zone_of(p) {
        Some(z) => format!("at {}", z.name),
        None => "close".into(),
    }
}

/// "He's at the center tower, left side" (left/right as seen by the speaker).
fn contact_text(arena: &Arena, speaker: V2, target: V2) -> String {
    match arena.zone_of(target) {
        Some(z) => {
            let c = (z.min + z.max) * 0.5;
            let side = if (c - speaker).cross(target - c) > 0.0 { "right" } else { "left" };
            let name = z.name.strip_prefix("the ").unwrap_or(z.name);
            format!("He's at the {name}, {side} side")
        }
        None => "Contact".into(),
    }
}

/// Flank destination in the sector the player watches least (absolute, from `PlayerModel.facing`).
pub fn least_watched_flank(arena: &Arena, model: &PlayerModel, target: V2, pref_range: f32) -> (usize, V2) {
    let r = pref_range.clamp(8.0, 16.0);
    let mut sectors: Vec<usize> = (0..8).collect();
    sectors.sort_by(|&a, &b| model.facing[a].total_cmp(&model.facing[b]).then(a.cmp(&b)));
    for &s in &sectors {
        for scale in [1.0, 0.7, 1.3] {
            let p = target + V2::from_angle(s as f32 * PI / 4.0) * r * scale;
            if arena.in_bounds(p) && arena.nav.walkable_at(p) {
                return (s, p);
            }
        }
    }
    let s = sectors[0];
    (s, arena.nav.nearest_walkable(target + V2::from_angle(s as f32 * PI / 4.0) * r))
}

/// Next cover for a bounding move: closer to the target by >= 4 m, within ~14 m, PEF-scored.
fn bound_point(arena: &Arena, model: &PlayerModel, m: &MemberView, target: V2, mates: &[V2]) -> Option<V2> {
    let here = m.pos.dist(target);
    let c = PefCtx { arena, model, target, me: m.pos, pref_range: m.traits.preferred_range, mates };
    let mut best: Option<(f32, V2)> = None;
    for cp in &arena.cover {
        let p = cp.pos;
        let step = p.dist(m.pos);
        if !(3.0..=14.0).contains(&step) || p.dist(target) > here - 4.0 || mates.iter().any(|q| q.dist(p) < 3.0) {
            continue;
        }
        let s = pef_score(p, &c, &m.traits.pef);
        if best.is_none_or(|(b, _)| s > b) {
            best = Some((s, p));
        }
    }
    // Nothing closer: advance in the open toward the target.
    best.map(|b| b.1).or_else(|| (here > 6.0).then(|| arena.nav.nearest_walkable(m.pos + (target - m.pos).norm() * 8.0f32.min(here - 4.0))))
}
