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
//! - **Adaptive doctrine** (`doctrine`): a rolling profile of the player's style picks a
//!   counter-tactic (jiggle-peek + flash + wallbang, off-angle traps, reload rushes, lean +
//!   stagger), each used as hard as the squad's doctrine genes say.

use crate::arena::{Arena, BlockKind, GRID_HALF};
use crate::bandit::{self, ArmScore, CTX_DIM, LiveCtx, TacticsLearner};
use crate::doctrine::{self, Counters, DOCTRINE_GENES, Doctrine, PlayerStyleProfile, StyleTick, TacticChange};
use crate::genome::Traits;
use crate::grenade::{self, GrenadeKind};
use crate::encircle::{self, Sector};
use crate::math::{BrainRng, V2, angle_diff, rng_from_seed};
use crate::mind::{Blackboard, RADIO_PERIOD_S};
use crate::search::{self, Searcher, Spot};
use crate::perception::{Alert, Heard, Sound, heard_sigma};
use crate::rooms::{self, Doorway};
use crate::weapons::WeaponKind;
use crate::player_model::{CoverState, HEAT_W, PlayerModel, SECTORS, classify_cover, heat_idx, sector_of};
use crate::utility::Goal;
use rand::Rng;
use std::cell::RefCell;
use std::f32::consts::{PI, TAU};

mod bomb;
pub use bomb::{Duty, SiteState, save_spot, site_dist as bomb_site_dist};

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

/// Within this range (m) of the threat, tactical routes pay `CLOSE_COST` per metre (fading out).
pub const CLOSE_M: f32 = 7.0;
pub const CLOSE_COST: f32 = 4.0;

/// Extra cost per metre `d` m from the threat: steep next to him, zero from `CLOSE_M` out.
pub fn close_cost(d: f32) -> f32 {
    let k = (1.0 - d / CLOSE_M).max(0.0);
    CLOSE_COST * k * k
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
        // Never route through his feet: a path that brushes past him ends at the standoff,
        // parked in front of his gun instead of round the far side.
        let v = v + close_cost(d);
        cache.borrow_mut()[i] = v;
        v
    }
}

// ---------------------------------------------------------------------------------------------
// Belief grid.

pub const BELIEF_W: usize = 24; // 4 m cells over 2 * GRID_HALF
const BELIEF_CELL: f32 = 2.0 * GRID_HALF / BELIEF_W as f32;

/// Decaying grid belief over the player's position (sum = 1). The particle filter is P2.
#[derive(Clone, Debug)]
pub struct Belief {
    pub p: Vec<f32>,
    pub walkable: Vec<bool>,
}

pub fn cell_center(i: usize) -> V2 {
    V2::new(-GRID_HALF + ((i % BELIEF_W) as f32 + 0.5) * BELIEF_CELL, -GRID_HALF + ((i / BELIEF_W) as f32 + 0.5) * BELIEF_CELL)
}

pub fn cell_of(p: V2) -> usize {
    let c = |v: f32| (((v + GRID_HALF) / BELIEF_CELL).floor().clamp(0.0, (BELIEF_W - 1) as f32)) as usize;
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
    /// Player unknown: split up over the belief's search spots (≤ 2 check the hot spot).
    Search,
    /// Player known: the spotter keeps shooting, the rest take his exits from distinct sides.
    Encircle,
    /// Bomb defusal, before a plant, player unknown: anchors hold the sites, a rotator holds
    /// mid; they rotate on info (`bomb`).
    Setup,
    /// Bomb defusal, bomb planted: converge on the site, one defuses, the rest cover (or save).
    Retake,
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
            Play::Search => "search",
            Play::Encircle => "encircle",
            Play::Setup => "site setup",
            Play::Retake => "retake",
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
    /// Searching a spot while the player is unknown.
    Searcher,
    /// Taking one of the player's exits (encirclement).
    Cutoff,
    /// Holds and pre-aims the angle the player keeps re-peeking.
    Anchor,
    /// Throwing utility (winding up a frag or flashbang).
    Breacher,
    /// Doctrine vs a reloader: rushing him on the mag-out sound.
    Rusher,
    /// Doctrine vs an angle holder: jiggle-peeking his angle to bait shots.
    Jiggler,
    /// Doctrine vs an angle holder: a suppressive burst through thin cover at his spot.
    Suppressor,
    /// Doctrine vs an entry fragger: holding an off-angle inside a door he will run through.
    Trapper,
    /// Doctrine vs an angle holder: shooting out the lamp that lights him, from out of his sight.
    Lights,
    /// Bomb defusal: holding an angle on a bomb site before the plant.
    SiteHold,
    /// Bomb defusal: between the sites (mid), first to rotate.
    Rotator,
    /// Bomb defusal: retaking the site after the plant (clearing an angle on the bomb).
    Retaker,
    /// Bomb defusal: going to (or on) the bomb to defuse it.
    Defuser,
    /// Bomb defusal: no time to defuse, getting out of the blast.
    Saving,
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
            Role::Searcher => "searcher",
            Role::Cutoff => "cutoff",
            Role::Anchor => "anchor",
            Role::Breacher => "breacher",
            Role::Rusher => "rusher",
            Role::Jiggler => "jiggler",
            Role::Suppressor => "suppressor",
            Role::Trapper => "trapper",
            Role::Lights => "lights",
            Role::SiteHold => "site hold",
            Role::Rotator => "rotator",
            Role::Retaker => "retaker",
            Role::Defuser => "defuser",
            Role::Saving => "saving",
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
    /// Go through this waypoint first (a route that differs from the teammates'), then `dest`.
    pub via: Option<V2>,
}

/// A jiggle-peek: from `hide` (out of his sight), step `out` (~0.25 m, toward the peek side)
/// and straight back, to bait a shot from the angle he holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Jiggle {
    pub hide: V2,
    pub out: V2,
}

/// What the doctrine asks of one member this tick (`SquadCoordinator::member_doctrine`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MemberDoctrine {
    /// Squad-wide counter strengths (0 = off).
    pub counters: Counters,
    /// This member jiggle-peeks (its order is `Role::Jiggler`).
    pub jiggle: Option<Jiggle>,
    /// This member shoots out a lamp (its order is `Role::Lights`).
    pub light: Option<LightShot>,
}

/// Shooting out a lamp that lights an angle holder: `lamp` is the id (`Arena::decor.lamps`
/// index), `pos`/`height` the fixture, `spot` where the shooter stands (sees the fixture, not him).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightShot {
    pub lamp: usize,
    pub pos: V2,
    pub height: f32,
    pub spot: V2,
}

/// Lamps shot out per round, at most (the ones lighting him, nearest first).
pub const MAX_LIGHTS: usize = 2;
/// Seconds the squad works on one lamp (walk to a spot and shoot) before giving up on it.
pub const LIGHT_TRY_S: f32 = 15.0;

/// Step out this far (m) on a jiggle-peek.
pub const JIGGLE_M: f32 = 0.25;
/// A mag-out is heard this far (m) in the open: the clack carries further than the reload rustle.
pub const MAG_OUT_RANGE: f32 = 14.0;
/// A reload rush lasts this long (s) or until the rusher is this close (m).
pub const RUSH_S: f32 = 3.0;
pub const RUSH_DONE_M: f32 = 3.0;
/// A wallbang burst lasts this long (s); the next one waits `WALLBANG_GAP_S` (shorter with a
/// stronger counter).
pub const WALLBANG_BURST_S: f32 = 0.7;
pub const WALLBANG_GAP_S: f32 = 4.5;
/// Wallbang only through cover that lets at least this share of the damage through.
pub const WALLBANG_MIN: f32 = 0.15;

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
/// The encirclement hands over to the bandit's plays after this long at the latest (s).
pub const ENCIRCLE_S: f32 = 14.0;
/// During a bandit play, he counts as out of the ring this far (m) from where it was set.
pub const RELOCATED_M: f32 = 8.0;
/// After the synchronised peek the bandit's plays take over after this long (s).
pub const GO_S: f32 = 4.0;
/// Staging ends (everyone peeks) after this long at the latest (s), times the map scale...
pub const STAGE_S: f32 = 7.0;
/// ...or, when the cutoffs have further to go, their longest route at this pace (m/s: they
/// sprint unseen, walk the last bit) plus `STAGE_SLACK_S`, up to `STAGE_MAX_MULT` times that.
pub const STAGE_PACE: f32 = 4.5;
pub const STAGE_SLACK_S: f32 = 2.0;
pub const STAGE_MAX_MULT: f32 = 2.5;
/// A mover counts as in position this close (m) to its sector goal.
pub const ARRIVED_M: f32 = 2.5;
/// The sectors are re-planned when he has moved this far (m).
pub const ENCIRCLE_REPLAN_M: f32 = 3.0;
/// Encirclement assignments are kept at least this long (s) unless he moved.
pub const ENCIRCLE_COMMIT_S: f32 = 2.5;
/// Longest stagger (s) before a member reacts to a new play.
pub const MAX_LAG_S: f32 = 0.5;
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

    // --- Split search and encirclement.
    rng: BrainRng,
    /// Current search spots (debug / control port).
    pub search_spots: Vec<Spot>,
    /// Spots looked into: (position, time).
    searched: Vec<(V2, f32)>,
    /// Per member: (search target, time assigned).
    search_target: Vec<Option<(V2, f32)>>,
    /// Members checking the hot spot (last seen / heard).
    pub investigators: Vec<usize>,
    /// Encirclement: approach sectors around the player and each member's sector.
    pub sectors: Vec<Sector>,
    pub sector_of: Vec<Option<usize>>,
    /// Per member: the via point of its route (cleared once passed).
    pub vias: Vec<Option<V2>>,
    encircle_at: V2,
    encircle_t: f32,
    /// How long staging may take this time (s): the movers' longest route, bounded
    /// (`STAGE_S`..`STAGE_MAX_MULT` x `STAGE_S`, both times the map scale).
    stage_s: f32,
    /// When the staged cutoffs were told to peek together.
    pub go_t: Option<f32>,
    /// Members that were shooting when the encirclement was planned.
    pub shooters: Vec<usize>,
    /// Staggered reactions: a member's new order lands after this many seconds.
    order_lag: Vec<f32>,

    // --- Room clearing (stack and breach).
    doors: Option<Vec<Doorway>>,
    /// A stack-and-breach on the room he holds: Trade's entry, a pin-and-flank flanker, a
    /// collapse, or the search's investigators going through his door.
    pub breach: Option<Breach>,
    /// A grenade wanted through the breach door (lands here).
    breach_throw: Option<V2>,
    /// Search breaches: the (entry, hot spot) the current breach was planned for.
    breach_for: Option<(usize, V2)>,
    /// Map size vs the 64 m maps (`search::map_scale`): big maps give staging and the
    /// encirclement proportionally longer, since everyone has further to walk.
    map_scale: f32,

    // --- Adaptive doctrine (`doctrine`).
    /// The player's style this round (fed by the caller: `observe_player`, `player_shot`...).
    pub style: PlayerStyleProfile,
    pub doctrine: Doctrine,
    doctrine_ready: bool,
    /// A mag-out heard: (where, when) and who heard it.
    mag_out: Option<(V2, f32)>,
    mag_listeners: Vec<usize>,
    /// Reload rushers: (member, target, until).
    pub rushers: Vec<(usize, V2, f32)>,
    /// The jiggle-peeker and its peek, re-picked every few seconds.
    pub jiggler: Option<(usize, Jiggle)>,
    jiggle_t: f32,
    /// Wallbang: (member, spot, until) and the time the next one may start.
    pub wallbang: Option<(usize, V2, f32)>,
    wallbang_next: f32,
    /// Shooting out the light over him: the lamp and when to give up on it, the member on it
    /// and its shot; lamps already tried this round; when to look for a shooter again.
    pub light_target: Option<(usize, f32)>,
    pub light_shot: Option<(usize, LightShot)>,
    lights_tried: Vec<usize>,
    lights_next: f32,
    /// Off-angle traps: (member, spot, door it watches).
    pub traps: Vec<(usize, V2, V2)>,
    traps_t: f32,
    /// The planted bomb, if any (the game updates it every frame; `crate::objective`).
    pub bomb: Option<crate::objective::PlantedBomb>,
    /// The member assigned to defuse it.
    pub defuser: Option<usize>,
    /// Bomb defusal mode (on once the game calls `set_bomb`): the squad plays the sites.
    pub objective: bool,
    /// Objective state (pre-plant setup, rotations, retake, save): `bomb.rs`.
    pub site: bomb::SiteState,
}

/// Stack and breach: the entry and its trader stack beside the door frame, utility goes in,
/// then the entry pushes through the door and pies the corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Breach {
    pub door: Doorway,
    /// The side they stack on (the entry's position when it was planned).
    pub from: V2,
    /// A point just inside the room.
    pub inside: V2,
    pub t: f32,
    /// Play time at which the entry goes in (set once stacked).
    pub go_at: Option<f32>,
    called: bool,
}

/// Breach a door when he is within this far (m) past it.
pub const BREACH_REACH: f32 = 14.0;
/// Longest the stack waits for the trader (s).
pub const BREACH_STACK_S: f32 = 6.0;

/// Entry pick weight by gun: shotguns and SMGs go first; marksmen never.
fn entry_weight(k: WeaponKind) -> f32 {
    match k {
        WeaponKind::Shotgun => 1.0,
        WeaponKind::Smg => 0.8,
        WeaponKind::Rifle => 0.5,
        WeaponKind::BattleRifle => 0.5,
        WeaponKind::Pistol | WeaponKind::HeavyPistol => 0.4,
        WeaponKind::Marksman | WeaponKind::Sniper => -1.0,
    }
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
            rng: rng_from_seed(0x5ea2c4 ^ n as u64),
            search_spots: Vec::new(),
            searched: Vec::new(),
            search_target: vec![None; n],
            investigators: Vec::new(),
            sectors: Vec::new(),
            sector_of: vec![None; n],
            vias: vec![None; n],
            encircle_at: V2::ZERO,
            encircle_t: -99.0,
            stage_s: STAGE_S,
            go_t: None,
            shooters: Vec::new(),
            order_lag: vec![0.0; n],
            doors: None,
            breach: None,
            breach_throw: None,
            breach_for: None,
            map_scale: search::map_scale(arena),
            style: PlayerStyleProfile::default(),
            doctrine: Doctrine::default(),
            doctrine_ready: false,
            mag_out: None,
            mag_listeners: Vec::new(),
            rushers: Vec::new(),
            jiggler: None,
            jiggle_t: 0.0,
            wallbang: None,
            wallbang_next: 0.0,
            light_target: None,
            light_shot: None,
            lights_tried: Vec::new(),
            lights_next: 0.0,
            traps: Vec::new(),
            traps_t: -99.0,
            bomb: None,
            defuser: None,
            objective: false,
            site: bomb::SiteState::default(),
        }
    }

    // ---------------------------------------------------------------------------------------
    // Bomb objective (CONTRACT with the game: `set_bomb` each frame, `wants_defuse` per member).

    /// The game reports the planted bomb (None before a plant / after it's resolved).
    /// Calling it at all switches the squad to bomb defusal (`objective`).
    pub fn set_bomb(&mut self, bomb: Option<crate::objective::PlantedBomb>) {
        self.objective = true;
        if bomb.is_none() {
            self.defuser = None;
        }
        self.bomb = bomb;
    }

    /// Member `i` is trying to defuse right now (the game ticks the defuse while this holds and
    /// it stands within `objective::DEFUSE_REACH` of the bomb).
    pub fn wants_defuse(&self, i: usize) -> bool {
        self.bomb.is_some() && self.defuser == Some(i) && !self.site.saving
    }

    // ---------------------------------------------------------------------------------------
    // Adaptive doctrine: inputs from the caller.

    /// Feed the player's movement and look this tick (game: `enemy::think_and_act`; trainer:
    /// the ghost). Call before `tick`.
    pub fn observe_player(&mut self, arena: &Arena, t: StyleTick) {
        self.style.tick(Some(arena), t);
    }

    /// He fired (once per trigger pull); `ads`: aiming down sights.
    pub fn player_shot(&mut self, ads: bool) {
        self.style.shot(ads);
    }

    /// One of his shots hit a member.
    pub fn player_hit(&mut self, headshot: bool) {
        self.style.hit(headshot);
    }

    /// He started a reload with `mag_frac` of his magazine left.
    pub fn player_reload(&mut self, mag_frac: f32) {
        self.style.reload(mag_frac);
    }

    /// Member `who` heard his magazine come out at `pos` (`MAG_OUT_RANGE`, see
    /// `perception::hear`). With the reload counter on, the nearest one or two rush him.
    pub fn heard_mag_out(&mut self, who: usize, pos: V2) {
        if self.mag_out.is_none_or(|(_, t)| self.time - t > 0.5) {
            self.mag_out = Some((pos, self.time));
            self.mag_listeners.clear();
        }
        if !self.mag_listeners.contains(&who) {
            self.mag_listeners.push(who);
        }
    }

    /// Doctrine switches since the last call (log them: `ai.tactic`).
    pub fn drain_tactic_changes(&mut self) -> Vec<TacticChange> {
        self.doctrine.drain_changes()
    }

    /// What the doctrine wants from member `i` (call after `tick`, hand it to the mind).
    pub fn member_doctrine(&self, i: usize) -> MemberDoctrine {
        if !self.enabled {
            return MemberDoctrine::default();
        }
        // The lights go out before anyone peeks: the jiggler waits at its hide spot meanwhile.
        let jiggle = self.jiggler.filter(|j| j.0 == i && self.light_shot.is_none()).map(|j| j.1);
        MemberDoctrine { counters: self.doctrine.counters(), jiggle, light: self.light_shot.filter(|l| l.0 == i).map(|l| l.1) }
    }

    /// The squad's doctrine genes: the mean over living members.
    fn doctrine_genes(members: &[MemberView]) -> [f32; DOCTRINE_GENES] {
        let alive: Vec<&MemberView> = members.iter().filter(|m| m.alive).collect();
        let mut g = [0.5; DOCTRINE_GENES];
        if !alive.is_empty() {
            for (k, x) in g.iter_mut().enumerate() {
                *x = alive.iter().map(|m| m.traits.doctrine[k].clamp(0.0, 1.0)).sum::<f32>() / alive.len() as f32;
            }
        }
        g
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

    /// Round over: score the open engagement window. `squad_won`: the squad won the round (with
    /// bomb rules the player can die and still win it, so this is the round result, not his death).
    pub fn end_round(&mut self, squad_won: bool) {
        if let Some(e) = self.episode.as_mut() {
            e.killed |= squad_won;
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
        self.search_target.resize(n, None);
        self.sector_of.resize(n, None);
        self.vias.resize(n, None);
        self.order_lag.resize(n, 0.0);
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

        // Adaptive doctrine: archetypes from the style profile, a counter-tactic per tick.
        if !self.doctrine_ready {
            self.doctrine = Doctrine::with_prior(&model.style);
            self.doctrine_ready = true;
        }
        let genes = Self::doctrine_genes(members);
        if let Some(change) = self.doctrine.update(&self.style, dt, genes)
            && self.enabled
            && let Some(s) = spotter
        {
            self.bark(s, change.to.callout().to_string(), true);
        }

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
        // Bomb planted: the objective comes first, even for a lone survivor or an
        // uncoordinated squad (someone has to defuse it).
        if self.objective && self.bomb.is_some() && alive > 0 && self.force_play.is_none() {
            return Play::Retake;
        }
        if !self.enabled || alive == 0 {
            return Play::Free;
        }
        let known = self.board.age < KNOWN_S;
        // Before the plant, with him unknown: hold the sites instead of hunting him.
        if self.objective && !known && arena.sites.len() >= 2 && self.force_play.is_none() {
            return Play::Setup;
        }
        if alive == 1 && self.force_play.is_none() {
            // A lone survivor still searches with the squad's belief.
            return if known { Play::Free } else { Play::Search };
        }
        if let Some(p) = self.force_play {
            return p;
        }
        // Unknown (unaware, or lost him): split up over the search spots; at most two check
        // where he was last seen or heard. Never one swarm toward one point.
        if !known {
            return Play::Search;
        }
        // A fresh contact is encircled first: the spotter shoots, the rest cut off his exits.
        // Once they are in position (or it took too long) the bandit picks the play.
        let fresh = !matches!(self.play, Play::Crossfire | Play::Trade | Play::PinFlank | Play::Bait | Play::Encircle);
        // He broke out of the ring (moved well away from where it was set): encircle again.
        let relocated = self.play != Play::Encircle && self.play_t > MIN_PLAY_S && self.board.pos.dist(self.encircle_at) > RELOCATED_M;
        let fresh = fresh || relocated;
        if fresh || (self.play == Play::Encircle && !self.encircled()) {
            return Play::Encircle;
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
        // Bomb defusal: track rotations; in a fight before the plant one anchor keeps the
        // other site (he could be faking), so the play runs without it.
        let held = self.objective_update(play, members, arena, model);
        let masked: Vec<MemberView>;
        let members = match held {
            Some(k) => {
                masked = members.iter().enumerate().map(|(i, m)| if i == k { MemberView { alive: false, ..*m } } else { *m }).collect();
                &masked[..]
            }
            None => members,
        };
        // Engagement windows: score the bandit's choice when the play changes or the window ends.
        let expired = self.episode.is_some_and(|e| self.time - e.t > EPISODE_S);
        if play != self.play || expired {
            self.close_episode(Some(members));
        }
        let before = self.orders.clone();
        self.plan_play(play, members, arena, model, full);
        // A member still "reacting" keeps its previous order a little longer.
        for (i, lag) in self.order_lag.iter_mut().enumerate() {
            if *lag > 0.0 {
                *lag -= RADIO_PERIOD_S;
                if let (Some(o), Some(b)) = (self.orders.get_mut(i), before.get(i)) {
                    *o = *b;
                }
            }
        }
        if let Some(k) = held {
            self.orders[k] = self.site_hold_order(k);
        }
        if matches!(self.play, Play::Setup | Play::Retake) {
            // The objective owns the orders (no anchors, grenades or doctrine roles on top).
            self.anchor = None;
            self.pending_throw = None;
            return;
        }
        if self.play == Play::Free {
            self.anchor = None;
            self.pending_throw = None;
            // A lone survivor still uses the doctrine (a reload rush, an off-angle).
            if self.enabled {
                self.plan_doctrine(members, arena);
            }
            return;
        }
        self.plan_anchor(members, arena);
        self.plan_utility(members, arena);
        self.plan_doctrine(members, arena);
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
        self.orders[i] = Some(Order { role: Role::Anchor, goal: Goal::Hold, dest: None, watch: Some(spot.pos), threat: None, sticky: true, via: None, prefire });
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
                Some(Order { role: Role::Breacher, goal: Goal::Hold, dest: None, watch: Some(t.target), threat: None, sticky: true, via: None, prefire: None });
            let left = left - RADIO_PERIOD_S;
            if left > 0.0 {
                self.pending_throw = Some((t, left));
                return;
            }
            self.pending_throw = None;
            self.breach_throw = None;
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
                GrenadeKind::Smoke => "Smoke out!".to_string(),
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
            // Stacked on a door: a flashbang (or a frag) goes through it first.
            None if self.breach_throw.is_some() => {
                if self.flashes_used < self.utility.flashes {
                    GrenadeKind::Flash
                } else if self.frags_used < self.utility.frags {
                    GrenadeKind::Frag
                } else {
                    self.breach_throw = None;
                    return;
                }
            }
            None if !known => return,
            None => {
                let camping = self.still_t >= CAMP_S;
                let hugging = self.still_t >= 2.0 && classify_cover(arena, target, Self::centroid(members)).0 != CoverState::Exposed;
                // Doctrine vs an angle holder: blind the angle first, then the frag.
                let blind_first = self.doctrine.counters().jiggle > 0.0 && self.flash_wanted > 0.0 && self.flashes_used < self.utility.flashes;
                if blind_first {
                    GrenadeKind::Flash
                } else if self.frags_used < self.utility.frags && (camping || hugging) {
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
            // A flash lands a step in front of him, on the thrower's side, so he sees it pop;
            // a breach grenade lands just inside the door.
            let aim = match self.breach_throw {
                Some(p) => p,
                None if frag => target,
                None => target + (m.pos - target).norm() * 1.5,
            };
            let d = m.pos.dist(aim);
            if !(grenade::THROW_MIN..grenade::THROW_MAX).contains(&d) || grenade::landing(arena, m.pos, aim).dist(aim) > 2.0 {
                continue;
            }
            // Never frag a spot a teammate stands in.
            if frag && members.iter().any(|o| o.alive && o.pos.dist(aim) < grenade::FRAG_RADIUS + 1.0) {
                continue;
            }
            let will = if frag { m.traits.aggression } else { m.traits.team_cohesion };
            // A cutoff still running to its sector keeps running: stopping for the wind-up
            // leaves that side of the ring open (someone already in place throws instead).
            let en_route = self.orders.get(i).copied().flatten().is_some_and(|o| o.role == Role::Cutoff && o.goal == Goal::Flank);
            let score = 0.6 * will + 0.4 * (1.0 - d / grenade::THROW_MAX) - if en_route { 0.35 } else { 0.0 };
            if best.is_none_or(|(b, _, _)| score > b) {
                best = Some((score, i, aim));
            }
        }
        // Willingness is evolvable (aggression for frags, cohesion for flashes).
        let Some((_, i, aim)) = best.filter(|b| forced.is_some() || b.0 >= 0.3) else { return };
        self.force_throw = None;
        self.pending_throw = Some((ThrowOrder { thrower: i, kind, target: aim }, grenade::WINDUP_S));
        self.orders[i] = Some(Order { role: Role::Breacher, goal: Goal::Hold, dest: None, watch: Some(aim), threat: None, sticky: true, via: None, prefire: None });
    }

    /// Doctrine counters as orders (after the play, the anchor and utility): reload rushes,
    /// jiggle-peeks, flashes and wallbangs on an angle holder, off-angle traps for a rusher.
    fn plan_doctrine(&mut self, members: &[MemberView], arena: &Arena) {
        let c = self.doctrine.counters();
        let now = self.time;
        let thrower = self.pending_throw.map(|(t, _)| t.thrower);
        // Members other jobs already claim.
        let busy_base: Vec<usize> = [self.lead, self.second, self.anchor, thrower].into_iter().flatten().collect();

        // --- Reload punish: the nearest one or two who heard the mag-out sprint at him.
        self.rushers.retain(|&(i, at, until)| now < until && members.get(i).is_some_and(|m| m.alive && m.pos.dist(at) > RUSH_DONE_M));
        if let Some((at, t)) = self.mag_out.take()
            && c.reload_rush > 0.0
            && now - t < 1.0
        {
            let mut who: Vec<usize> = self.mag_listeners.iter().copied().filter(|&i| members.get(i).is_some_and(|m| m.alive) && !busy_base.contains(&i)).collect();
            who.sort_by(|&a, &b| members[a].pos.dist(at).total_cmp(&members[b].pos.dist(at)).then(a.cmp(&b)));
            let alive = members.iter().filter(|m| m.alive).count();
            let n = if c.reload_rush >= 0.6 && alive >= 3 { 2 } else { 1 };
            for &i in who.iter().take(n) {
                if !self.rushers.iter().any(|r| r.0 == i) {
                    self.rushers.push((i, at, now + RUSH_S));
                }
            }
            if let Some(&i) = who.first() {
                self.bark(i, "Mag out - rushing him!".into(), true);
            }
        }
        self.mag_listeners.clear();
        for &(i, at, _) in &self.rushers {
            self.orders[i] = Some(Order { role: Role::Rusher, goal: Goal::Push, dest: Some(at), watch: Some(at), threat: None, sticky: true, via: None, prefire: None });
        }
        let mut busy = busy_base.clone();
        busy.extend(self.rushers.iter().map(|r| r.0));

        // --- Angle holder: flash the angle, jiggle-peek it, wallbang him through thin cover.
        let known = self.board.age < 3.0;
        let target = self.board.pos;
        if c.jiggle > 0.0 && known {
            // Flash sooner the stronger the counter (the utility budget and cooldown still rule).
            if self.still_t >= CAMP_S * (1.3 - 0.8 * c.jiggle) && self.flashes_used < self.utility.flashes {
                self.flash_wanted = self.flash_wanted.max(2.0);
            }
            // Shoot out the lamps lighting him (where there are lamps), before anyone peeks: one
            // member walks to a spot that sees the fixture but not him and taps it out.
            // A lamp is out, or its time is up: next one (each lamp gets one try per round).
            if let Some((lamp, until)) = self.light_target
                && (now >= until || !arena.lamp_lit(lamp))
            {
                self.light_target = None;
                self.light_shot = None;
            }
            // The shooter got pulled into the fight or another job: someone else takes the lamp.
            if let Some((i, _)) = self.light_shot
                && (busy.contains(&i) || members.get(i).is_none_or(|m| !m.alive || m.last_seen < 0.3))
            {
                self.light_shot = None;
            }
            if self.light_target.is_none() && self.lights_tried.len() < MAX_LIGHTS
                && let Some(lamp) = arena.lamps_lighting(target).into_iter().find(|l| !self.lights_tried.contains(l))
            {
                self.lights_tried.push(lamp);
                self.light_target = Some((lamp, now + LIGHT_TRY_S));
            }
            if let Some((lamp, _)) = self.light_target
                && self.light_shot.is_none()
                && now >= self.lights_next
            {
                self.lights_next = now + 1.0;
                let (pos, height) = arena.decor.lamps[lamp].fixture();
                let pick = (0..members.len())
                    .filter(|&i| members[i].alive && !busy.contains(&i) && members[i].last_seen > 0.5 && self.jiggler.is_none_or(|j| j.0 != i))
                    .filter_map(|i| doctrine::light_spot(arena, members[i].pos, pos, height, target).map(|(walk, spot)| (walk, i, spot)))
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                if let Some((_, i, spot)) = pick {
                    self.light_shot = Some((i, LightShot { lamp, pos, height, spot }));
                    self.bark(i, "Shooting out his light".into(), false);
                }
            }
            if let Some((i, s)) = self.light_shot {
                busy.push(i);
                self.orders[i] =
                    Some(Order { role: Role::Lights, goal: Goal::Hold, dest: Some(s.spot), watch: Some(s.pos), threat: Some(target), sticky: true, via: None, prefire: None });
            }
            self.jiggle_t -= RADIO_PERIOD_S;
            let keep = self.jiggler.filter(|(i, j)| {
                members.get(*i).is_some_and(|m| m.alive && m.pos.dist(j.hide) < 12.0) && !busy.contains(i) && self.jiggle_t > 0.0
            });
            self.jiggler = keep.or_else(|| {
                self.jiggle_t = 3.0 + 3.0 * (1.0 - c.jiggle);
                let mut best: Option<(f32, usize, Jiggle)> = None;
                for (i, m) in members.iter().enumerate() {
                    if !m.alive || busy.contains(&i) || !(5.0..32.0).contains(&m.pos.dist(target)) {
                        continue;
                    }
                    let Some((walk, j)) = doctrine::jiggle_spot(arena, m.pos, target) else { continue };
                    let s = -walk;
                    if best.as_ref().is_none_or(|b| s > b.0) {
                        best = Some((s, i, j));
                    }
                }
                best.map(|(_, i, j)| (i, j))
            });
            if let Some((i, j)) = self.jiggler {
                busy.push(i);
                self.orders[i] =
                    Some(Order { role: Role::Jiggler, goal: Goal::Hold, dest: Some(j.hide), watch: Some(target), threat: Some(target), sticky: true, via: None, prefire: None });
            }
            // Wallbang: someone with a gun that goes through what's between them and him.
            self.wallbang = self.wallbang.filter(|w| now < w.2 && members.get(w.0).is_some_and(|m| m.alive));
            if self.wallbang.is_none() && now >= self.wallbang_next && self.still_t >= 1.0 {
                let pick = (0..members.len())
                    .filter(|&i| members[i].alive && !busy.contains(&i) && members[i].last_seen > 0.5)
                    .filter_map(|i| {
                        let m = &members[i];
                        let d = m.pos.dist(target);
                        let pen = m.traits.weapon.spec().penetration;
                        if !(4.0..30.0).contains(&d) || pen <= 0.0 || arena.line_of_sight(m.pos, false, target, false) {
                            return None;
                        }
                        let keep = arena.penetration(m.pos, target, pen);
                        (keep >= WALLBANG_MIN).then_some((keep - d / 100.0, i))
                    })
                    .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
                if let Some((_, i)) = pick {
                    self.wallbang = Some((i, target, now + WALLBANG_BURST_S));
                    self.wallbang_next = now + WALLBANG_GAP_S * (1.4 - 0.8 * c.jiggle);
                    self.bark(i, "Spraying through the wall".into(), false);
                }
            }
            if let Some((i, at, _)) = self.wallbang {
                busy.push(i);
                self.orders[i] = Some(Order { role: Role::Suppressor, goal: Goal::Hold, dest: None, watch: Some(at), threat: None, sticky: true, via: None, prefire: Some(at) });
            }
        } else {
            self.jiggler = None;
            self.wallbang = None;
            self.light_target = None;
            self.light_shot = None;
        }

        // --- Entry fragger: hold off-angles inside the doors he will run through.
        if c.off_angle > 0.0 && self.board.age > 1.5 {
            if now - self.traps_t > 3.0 {
                self.traps_t = now;
                let from = self.predicted(arena);
                let doors = self.doors.get_or_insert_with(|| rooms::doorways(arena)).clone();
                let want = if c.off_angle >= 0.6 { 2 } else { 1 };
                let mut traps: Vec<(usize, V2, V2)> = Vec::new();
                let mut cands: Vec<usize> = (0..members.len()).filter(|&i| members[i].alive && !busy.contains(&i) && members[i].last_seen > 2.0).collect();
                cands.sort_by(|&a, &b| members[a].pos.dist(from).total_cmp(&members[b].pos.dist(from)).then(a.cmp(&b)));
                for i in cands {
                    if traps.len() >= want {
                        break;
                    }
                    let m = members[i].pos;
                    // A door between him and this member, close to the member.
                    let door = doors
                        .iter()
                        .filter(|d| d.pos.dist(m) < 14.0 && d.pos.dist(from) < 30.0 && d.side(m) != d.side(from) && !traps.iter().any(|t| t.2 == d.pos))
                        .min_by(|a, b| a.pos.dist(m).total_cmp(&b.pos.dist(m)));
                    if let Some(d) = door
                        && let Some(spot) = doctrine::off_angle_spot(arena, d, from, m)
                    {
                        traps.push((i, spot, d.pos));
                    }
                }
                self.traps = traps;
            }
            self.traps.retain(|t| members.get(t.0).is_some_and(|m| m.alive && m.last_seen > 0.5) && !busy.contains(&t.0));
            for &(i, spot, door) in &self.traps {
                self.orders[i] = Some(Order { role: Role::Trapper, goal: Goal::Hold, dest: Some(spot), watch: Some(door), threat: None, sticky: false, via: None, prefire: None });
            }
        } else {
            self.traps.clear();
        }
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
            self.breach = None;
            self.breach_throw = None;
            self.breach_for = None;
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
        if changed {
            // Staggered reactions: nobody but the spotter turns on a dime with the rest.
            for (i, m) in members.iter().enumerate() {
                let lag = if m.last_seen < FRESH_CONTACT_S || self.force_play.is_some() { 0.0 } else { self.rng.random::<f32>() * MAX_LAG_S };
                if let Some(l) = self.order_lag.get_mut(i) {
                    *l = lag;
                }
            }
        }
        match play {
            Play::Search => return self.plan_search(members, arena, full, changed),
            Play::Encircle => return self.plan_encircle(members, arena, full, changed),
            Play::Setup => return self.plan_setup(members, arena, changed),
            Play::Retake => return self.plan_retake(members, arena, changed),
            _ => {}
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
        let hold = |dest: Option<V2>, role: Role| Order { role, goal: Goal::Hold, dest, watch: Some(target), threat: Some(target), sticky: false, via: None, prefire: None };

        match play {
            Play::Free | Play::Search | Play::Encircle | Play::Setup | Play::Retake => {}
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
                // The entry: a close-range gun among the three nearest (never a marksman).
                let entry = *self.lead.get_or_insert_with(|| pick_entry(&by_dist, members));
                if self.second.is_none() {
                    self.second = pick_trader(&by_dist, members, entry);
                }
                let second = self.second;
                let ep = members[entry].pos;
                let away = (ep - target).norm();
                // He holds a room: stack on the door, utility in, then breach (`Breach`).
                let stacking = self.plan_breach(entry, second, members, arena, target, changed);
                let via = self.breach.filter(|_| !stacking).map(|b| b.inside);
                if !stacking {
                    self.orders[entry] = Some(Order { role: Role::Entry, goal: Goal::Push, dest: None, watch: Some(target), threat: None, sticky: via.is_some(), via, prefire: None });
                }
                if let Some(tr) = second.filter(|_| !stacking) {
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
                    self.orders[tr] = Some(Order { role: Role::Trader, goal: Goal::Hold, dest: Some(follow), watch: Some(target), threat: None, sticky: true, via: None, prefire: None });
                }
                let rest: Vec<usize> = by_dist.iter().copied().filter(|&i| i != entry && Some(i) != second).collect();
                if full || changed {
                    self.assign_positions(&rest, members, arena, model, target, Some(ep));
                }
                for &i in &rest {
                    self.orders[i] = Some(hold(self.claims[i], Role::Support));
                }
                if changed && self.breach.is_none() {
                    self.bark(entry, "Going in - trade me".into(), true);
                    self.flash_wanted = 4.0;
                }
            }
            Play::PinFlank => {
                let flanker = *self.lead.get_or_insert_with(|| {
                    // Flank bias, and the gun: SMGs flank, marksmen stay on their sightlines.
                    let w = |i: usize| {
                        members[i].traits.flank_bias
                            + match members[i].traits.weapon {
                                WeaponKind::Smg => 0.4,
                                WeaponKind::Shotgun => 0.2,
                                WeaponKind::Marksman | WeaponKind::Sniper => -1.0,
                                _ => 0.0,
                            }
                    };
                    *alive.iter().max_by(|&&a, &&b| w(a).total_cmp(&w(b)).then(b.cmp(&a))).unwrap_or(&alive[0])
                });
                // He holds a room: the flanker takes a door the pins aren't watching and breaches
                // it (stack, utility, in) instead of walking a flank into the walls.
                let stacking = self.plan_breach(flanker, None, members, arena, target, changed);
                if let Some(b) = self.breach {
                    if !stacking {
                        self.orders[flanker] = Some(Order {
                            role: Role::Flanker,
                            goal: Goal::Push,
                            dest: None,
                            watch: Some(target),
                            threat: None,
                            sticky: true,
                            via: Some(b.inside),
                            prefire: None,
                        });
                    }
                } else {
                    if full || changed || self.claims[flanker].is_none() {
                        let (sector, dest) = least_watched_flank(arena, model, target, members[flanker].traits.preferred_range);
                        self.claims[flanker] = Some(dest);
                        if changed {
                            self.bark(flanker, format!("Flanking {}", SECTORS[sector]), true);
                            self.flash_wanted = 4.0;
                        }
                    }
                    self.orders[flanker] =
                        Some(Order { role: Role::Flanker, goal: Goal::Flank, dest: self.claims[flanker], watch: Some(target), threat: Some(target), sticky: true, via: None, prefire: None });
                }
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
                // He is in a room: the entry and its trader breach it first (stack, utility,
                // in); the rest hold their angles until the entry is through, then collapse.
                let entry = *self.lead.get_or_insert_with(|| pick_entry(&by_dist, members));
                if self.second.is_none() {
                    self.second = pick_trader(&by_dist, members, entry);
                }
                let trader = self.second;
                let stacking = self.plan_breach(entry, trader, members, arena, target, changed);
                if let Some(b) = self.breach {
                    let rest: Vec<usize> = by_dist.iter().copied().filter(|&i| i != entry && Some(i) != trader).collect();
                    if stacking {
                        if full || changed {
                            self.assign_positions(&rest, members, arena, model, target, Some(b.door.pos));
                        }
                        for &i in &rest {
                            self.orders[i] = Some(hold(self.claims[i], Role::Support));
                        }
                        return;
                    }
                    self.orders[entry] =
                        Some(Order { role: Role::Entry, goal: Goal::Push, dest: None, watch: Some(target), threat: None, sticky: true, via: Some(b.inside), prefire: None });
                    if let Some(tr) = trader {
                        self.orders[tr] =
                            Some(Order { role: Role::Trader, goal: Goal::Push, dest: None, watch: Some(target), threat: None, sticky: true, via: Some(b.inside), prefire: None });
                    }
                }
                // Converge along different bearings so they come from different routes.
                let mut by_bearing = alive.clone();
                if self.breach.is_some() {
                    by_bearing.retain(|&i| i != entry && Some(i) != trader);
                }
                by_bearing.sort_by(|&a, &b| (members[a].pos - target).angle().total_cmp(&(members[b].pos - target).angle()));
                let k = by_bearing.len() as f32;
                for (r, &i) in by_bearing.iter().enumerate() {
                    let ang = (members[i].pos - target).angle() + (r as f32 - (k - 1.0) * 0.5) * 0.35;
                    let dest = arena.nav.nearest_walkable(target + V2::from_angle(ang) * 5.0);
                    self.orders[i] = Some(Order { role: Role::Collapser, goal: Goal::Hunt, dest: Some(dest), watch: Some(target), threat: Some(target), sticky: false, via: None, prefire: None });
                }
                if changed && self.breach.is_none() {
                    let z = arena.zone_of(target).map_or("there", |z| z.name);
                    let text = if play == Play::Collapse { format!("Shots at {z} - collapse") } else { format!("Checking {z}") };
                    self.bark(by_dist[0], text, play == Play::Collapse);
                }
            }
            Play::Bait => {
                let bait = *self
                    .lead
                    // The healthiest member draws him out; a marksman stays on its sightline.
                    .get_or_insert_with(|| {
                        let hp = |i: usize| members[i].hp - if members[i].traits.weapon.long_gun() { 2.0 } else { 0.0 };
                        *alive.iter().max_by(|&&a, &&b| hp(a).total_cmp(&hp(b)).then(b.cmp(&a))).unwrap_or(&alive[0])
                    });
                let r = model.weakest_range().clamp(6.0, 28.0);
                let dest = arena.nav.nearest_walkable(target + (members[bait].pos - target).norm() * r);
                self.orders[bait] = Some(Order { role: Role::Bait, goal: Goal::Push, dest: Some(dest), watch: Some(target), threat: None, sticky: true, via: None, prefire: None });
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
                        self.orders[i] = Some(Order { role: Role::Bound, goal: Goal::Hold, dest: self.claims[i], watch: Some(target), threat: Some(target), sticky: false, via: None, prefire: None });
                    } else {
                        self.claims[i] = None;
                        self.orders[i] = Some(hold(None, Role::Overwatch));
                    }
                }
            }
        }
    }

    /// Seed the coordinator's own choices (spot sampling, staggers); deterministic per seed.
    pub fn seed(&mut self, seed: u64) {
        self.rng = rng_from_seed(seed ^ 0x5ea2_c4d1);
    }

    /// Where he was last seen or heard, if recent enough to check (freshest of the two).
    pub fn hot_spot(&self) -> Option<V2> {
        let heard = self.heard_at.filter(|_| self.heard_t < 8.0).map(|h| (h, self.heard_t));
        let seen = (self.board.age < 15.0).then_some((self.board.pos, self.board.age));
        let hot = match (heard, seen) {
            (Some(h), Some(s)) => Some(if h.1 < s.1 { h.0 } else { s.0 }),
            (a, b) => a.or(b).map(|x| x.0),
        };
        hot.filter(|h| !self.searched.iter().any(|(p, t)| self.time - t < search::SEARCHED_S && p.dist(*h) < search::SEARCHED_R))
    }

    /// The synchronised peek has had its moment (or the encirclement has run its time).
    fn encircled(&self) -> bool {
        self.play_t > (ENCIRCLE_S * self.map_scale).max(self.stage_s + GO_S) || self.go_t.is_some_and(|g| self.time - g > GO_S)
    }

    /// Every mover is at its staging cell.
    fn staged(&self, members: &[MemberView]) -> bool {
        let mut movers = 0;
        for (i, m) in members.iter().enumerate().filter(|(_, m)| m.alive) {
            if let Some(s) = self.sector_of.get(i).copied().flatten().and_then(|s| self.sectors.get(s)) {
                movers += 1;
                if m.pos.dist(s.goal) > ARRIVED_M {
                    return false;
                }
            }
        }
        movers > 0
    }

    /// Clear via points that a member has reached or passed.
    fn update_vias(&mut self, members: &[MemberView]) {
        for (i, v) in self.vias.iter_mut().enumerate() {
            let (Some(p), Some(m), Some(o)) = (*v, members.get(i), self.orders.get(i).copied().flatten()) else {
                *v = None;
                continue;
            };
            let past = o.dest.is_some_and(|d| m.pos.dist(d) < p.dist(d));
            if !m.alive || m.pos.dist(p) < 3.0 || past {
                *v = None;
            }
        }
    }

    /// Search: split up over the belief's spots; at most two check the hot spot.
    fn plan_search(&mut self, members: &[MemberView], arena: &Arena, full: bool, changed: bool) {
        let alive: Vec<usize> = (0..members.len()).filter(|&i| members[i].alive).collect();
        let now = self.time;
        if changed {
            self.search_target.iter_mut().for_each(|t| *t = None);
            self.investigators.clear();
            self.vias.iter_mut().for_each(|v| *v = None);
            self.sector_of.iter_mut().for_each(|s| *s = None);
            if (self.was_known || self.board.alert == Alert::Combat)
                && let Some(&s) = alive.first()
            {
                self.bark(s, "Lost him - spreading out".into(), false);
            }
        }
        // Negative information: spots (and the hot spot) someone has looked into are done.
        self.searched.retain(|(_, t)| now - t < search::SEARCHED_S);
        let mut looked: Vec<V2> = Vec::new();
        let hot_now = self.hot_spot();
        for sp in self.search_spots.iter().map(|s| s.pos).chain(hot_now) {
            let seen = alive.iter().any(|&i| {
                let m = &members[i];
                m.pos.dist(sp) < 3.0 || ((full || changed) && search::sees_spot(arena, m.pos, m.look, sp))
            });
            if seen {
                looked.push(sp);
            }
        }
        for p in looked {
            self.searched.push((p, now));
        }
        self.update_vias(members);
        let hot = self.hot_spot();
        let dropped = self.search_target.iter().enumerate().any(|(i, t)| {
            t.is_some_and(|(p, at)| {
                now - at > search::COMMIT_MAX_S
                    || self.searched.iter().any(|(q, _)| q.dist(p) < search::SEARCHED_R)
                    || !members.get(i).is_some_and(|m| m.alive)
            })
        });
        let lost_hot = !self.investigators.is_empty() && hot.is_none();
        if !(full || changed || dropped || lost_hot) {
            self.search_breach(members, arena);
            return;
        }
        let eyes: Vec<V2> = alive.iter().map(|&i| members[i].pos).collect();
        let mut spots = search::spots(arena, &self.belief, hot, &eyes, &self.searched, now);
        if spots.is_empty() {
            // Everything searched recently: start over from the belief.
            spots = search::spots(arena, &self.belief, hot, &eyes, &[], now);
        }
        self.search_spots = spots;
        let fields: Vec<Vec<f32>> = (0..members.len())
            .map(|i| if members[i].alive { arena.nav.distance_field(&[members[i].pos], f32::INFINITY) } else { Vec::new() })
            .collect();
        // Investigators: the closest (by path) one or two; they keep the job while it lasts.
        self.investigators.retain(|&i| members.get(i).is_some_and(|m| m.alive));
        let want = match (hot, alive.len()) {
            (None, _) => 0,
            (Some(_), 0..=3) => 1,
            _ => 2,
        };
        if let Some(h) = hot {
            let mut by_d: Vec<usize> = alive.iter().copied().filter(|i| !self.investigators.contains(i)).collect();
            by_d.sort_by(|&a, &b| arena.nav.field_at(&fields[a], h).total_cmp(&arena.nav.field_at(&fields[b], h)).then(a.cmp(&b)));
            for i in by_d {
                if self.investigators.len() >= want {
                    break;
                }
                self.investigators.push(i);
            }
        }
        self.investigators.truncate(want);
        let searchers: Vec<usize> = alive.iter().copied().filter(|i| !self.investigators.contains(i)).collect();
        let picks = {
            let who: Vec<Searcher> = searchers
                .iter()
                .map(|&i| Searcher { slot: i, pos: members[i].pos, field: &fields[i], current: self.search_target[i].map(|t| t.0) })
                .collect();
            let reserved: Vec<V2> = hot.into_iter().collect();
            search::assign(arena, &self.search_spots, &who, &reserved, &mut self.rng)
        };
        // Investigators come in along different routes.
        if let Some(h) = hot {
            let routes: Vec<(V2, V2)> = self.investigators.iter().map(|&i| (members[i].pos, h)).collect();
            let guns = (self.board.alert == Alert::Combat).then(|| Self::centroid(members));
            let vias = encircle::diverse_vias(arena, h, guns, &routes);
            for (k, i) in self.investigators.clone().into_iter().enumerate() {
                if changed || self.search_target[i].is_none_or(|t| t.0.dist(h) > 2.0) {
                    self.vias[i] = vias[k];
                    self.search_target[i] = Some((h, now));
                    let text = if self.heard_t < self.board.age { "Checking that noise" } else { "Checking his last position" };
                    self.bark(i, text.into(), false);
                }
            }
        }
        for (k, &i) in searchers.iter().enumerate() {
            let old = self.search_target[i].map(|t| t.0);
            match picks[k] {
                Some(p) if old.is_none_or(|o| o.dist(p) > 1.0) => {
                    self.search_target[i] = Some((p, now));
                    self.vias[i] = None;
                    if let Some(z) = arena.zone_of(p) {
                        self.bark(i, format!("Checking {}", z.name), false);
                    }
                }
                Some(_) => {}
                None => self.search_target[i] = None,
            }
        }
        let combat = self.board.alert == Alert::Combat;
        for &i in &alive {
            let Some((dest, _)) = self.search_target[i] else {
                self.orders[i] = None;
                continue;
            };
            let inv = self.investigators.contains(&i);
            self.orders[i] = Some(Order {
                role: if inv { Role::Collapser } else { Role::Searcher },
                goal: Goal::Hunt,
                dest: Some(dest),
                watch: if inv { Some(dest) } else { None },
                // In a fight, route around his likely lines of fire.
                threat: if combat { hot } else { None },
                sticky: true,
                via: self.vias[i],
                prefire: None,
            });
        }
        self.search_breach(members, arena);
    }

    /// Search: he went to ground in a room (the hot spot is past a door): the investigators
    /// stack on that door and breach it instead of strolling in one at a time.
    fn search_breach(&mut self, members: &[MemberView], arena: &Arena) {
        let hot = self.hot_spot();
        let (Some(h), Some(&entry)) = (hot, self.investigators.first()) else {
            self.breach = None;
            self.breach_for = None;
            return;
        };
        let trader = self.investigators.get(1).copied();
        // Plan once per (entry, hot spot); a new spot or a new entry re-plans.
        let plan = self.breach_for.is_none_or(|(e, p)| e != entry || p.dist(h) > 2.0);
        if plan {
            self.breach_for = Some((entry, h));
        }
        let stacking = self.plan_breach(entry, trader, members, arena, h, plan);
        // The entry never throws the breach grenade (the utility pick skips the lead).
        self.lead = self.breach.map(|_| entry);
        let Some(b) = self.breach.filter(|_| !stacking) else { return };
        for i in std::iter::once(entry).chain(trader) {
            self.orders[i] = Some(Order {
                role: Role::Collapser,
                goal: Goal::Push,
                dest: Some(h),
                watch: Some(h),
                threat: None,
                sticky: true,
                via: (b.door.side(members[i].pos) == b.door.side(b.from)).then_some(b.inside),
                prefire: None,
            });
        }
    }

    /// Encircle: the spotter keeps shooting; everyone else takes a distinct approach sector
    /// (his exits) along a route the others are not using.
    fn plan_encircle(&mut self, members: &[MemberView], arena: &Arena, full: bool, changed: bool) {
        let alive: Vec<usize> = (0..members.len()).filter(|&i| members[i].alive).collect();
        let p = arena.nav.nearest_walkable(self.board.pos);
        let now = self.time;
        let closest = |alive: &[usize]| alive.iter().copied().min_by(|&a, &b| members[a].pos.dist(p).total_cmp(&members[b].pos.dist(p)));
        if changed {
            self.sector_of.iter_mut().for_each(|s| *s = None);
            self.vias.iter_mut().for_each(|v| *v = None);
            self.search_target.iter_mut().for_each(|t| *t = None);
            self.investigators.clear();
            self.go_t = None;
            // The shooters: whoever sees him (at most two), else the closest.
            let mut contact: Vec<usize> = alive.iter().copied().filter(|&i| members[i].last_seen < 1.0).collect();
            contact.sort_by(|&a, &b| members[a].last_seen.total_cmp(&members[b].last_seen).then(members[a].pos.dist(p).total_cmp(&members[b].pos.dist(p))));
            // Big maps need more cutoffs to close the far side: a second shooter only from 5.
            let two_from = if self.map_scale > 1.2 { 5 } else { 4 };
            contact.truncate(if alive.len() >= two_from { 2 } else { 1 });
            self.shooters = contact;
        }
        self.shooters.retain(|&i| members.get(i).is_some_and(|m| m.alive));
        if self.shooters.is_empty()
            && let Some(c) = closest(&alive)
        {
            self.shooters.push(c);
        }
        self.update_vias(members);
        let moved = p.dist(self.encircle_at) > ENCIRCLE_REPLAN_M;
        let due = changed || (full && (moved || now - self.encircle_t > ENCIRCLE_COMMIT_S));
        if due {
            let old: Vec<Option<f32>> = self.sector_of.iter().map(|s| s.and_then(|s| self.sectors.get(s)).map(|s| s.bearing)).collect();
            if changed || moved || self.sectors.is_empty() {
                self.sectors = encircle::sectors(arena, p);
                self.encircle_at = p;
            }
            let movers: Vec<usize> = alive.iter().copied().filter(|i| !self.shooters.contains(i)).collect();
            let dist: Vec<Vec<f32>> = movers
                .iter()
                .map(|&i| {
                    let f = arena.nav.distance_field(&[members[i].pos], f32::INFINITY);
                    self.sectors.iter().map(|s| arena.nav.field_at(&f, s.goal)).collect()
                })
                .collect();
            let fixed: Vec<f32> = self.shooters.iter().map(|&i| (members[i].pos - p).angle()).collect();
            // Previous sectors, mapped onto the current set by nearest bearing.
            let prev: Vec<Option<usize>> = movers
                .iter()
                .map(|&i| {
                    old.get(i).copied().flatten().and_then(|b| {
                        (0..self.sectors.len()).min_by(|&x, &y| angle_diff(self.sectors[x].bearing, b).abs().total_cmp(&angle_diff(self.sectors[y].bearing, b).abs()))
                    })
                })
                .collect();
            let mut taken: Vec<usize> = prev.iter().flatten().copied().collect();
            taken.sort_unstable();
            let unique = taken.windows(2).all(|w| w[0] != w[1]);
            let keep = (!changed && unique && !taken.is_empty()).then_some(prev.as_slice());
            let pick = encircle::assign(&self.sectors, &dist, &fixed, keep, &mut self.rng);
            // Big maps: the far sectors are a long walk round the block. Give the cutoffs time
            // to get there before the synchronised peek, or the ring closes from one side only.
            if changed || moved {
                let longest = pick.iter().enumerate().filter_map(|(k, s)| s.map(|s| dist[k][s])).filter(|d| d.is_finite()).fold(0.0f32, f32::max);
                let lo = STAGE_S * self.map_scale;
                self.stage_s = (longest / STAGE_PACE + STAGE_SLACK_S).clamp(lo, lo * STAGE_MAX_MULT);
            }
            self.sector_of.iter_mut().for_each(|s| *s = None);
            // Routes, nearest first (they commit their corridors first).
            let mut order: Vec<usize> = (0..movers.len()).filter(|&k| pick[k].is_some()).collect();
            let d_of = |k: usize| pick[k].map_or(f32::INFINITY, |s| dist[k][s]);
            order.sort_by(|&a, &b| d_of(a).total_cmp(&d_of(b)));
            let routes: Vec<(V2, V2)> = order.iter().filter_map(|&k| pick[k].map(|s| (members[movers[k]].pos, self.sectors[s].goal))).collect();
            let guns: Vec<V2> = self.shooters.iter().map(|&i| members[i].pos).collect();
            let guns = (!guns.is_empty()).then(|| guns.iter().fold(V2::ZERO, |a, q| a + *q) * (1.0 / guns.len() as f32));
            let vias = encircle::diverse_vias(arena, p, guns, &routes);
            for (r, &k) in order.iter().enumerate() {
                let i = movers[k];
                let Some(s) = pick[k] else { continue };
                let bearing = self.sectors[s].bearing;
                let new_sector = old.get(i).copied().flatten().is_none_or(|b| angle_diff(b, bearing).abs() > 0.3);
                self.sector_of[i] = Some(s);
                if new_sector || changed {
                    self.vias[i] = vias[r];
                    let dir = SECTORS[sector_of(bearing)];
                    let text = if r == 0 { format!("Flanking {dir}") } else { format!("Cutting him off - {dir}") };
                    self.bark(i, text, false);
                }
            }
            self.encircle_t = now;
        }
        // Staged: everyone is a step from his line of fire (or it took too long). Then all
        // peek at once from their sides, so he can't take them one at a time.
        if self.go_t.is_none() && (self.staged(members) || self.play_t > self.stage_s) {
            self.go_t = Some(now);
            if let Some(&s) = self.shooters.first() {
                self.bark(s, "All set - go go go".into(), true);
            }
        }
        let go = self.go_t.is_some();
        for &i in &alive {
            if self.shooters.contains(&i) {
                // The spotter fights on its own judgement (shoot, strafe, peek).
                self.orders[i] = None;
                continue;
            }
            let Some(s) = self.sector_of[i].and_then(|s| self.sectors.get(s)).copied() else {
                self.orders[i] = None;
                continue;
            };
            let dest = if go { s.peek } else { s.goal };
            let arrived = members[i].pos.dist(dest) < ARRIVED_M;
            self.orders[i] = Some(Order {
                role: Role::Cutoff,
                goal: if arrived { Goal::Hold } else if go { Goal::Push } else { Goal::Flank },
                dest: Some(dest),
                watch: Some(p),
                threat: if go { None } else { Some(p) },
                sticky: !arrived,
                via: if arrived || go { None } else { self.vias[i] },
                prefire: None,
            });
        }
    }

    /// Stack and breach (Trade's entry, a flanker, a collapse): when `plan`, find the door
    /// between `entry` and his room (`rooms::breach_door`); then `entry` (and `trader`) stack
    /// beside its frame, utility goes in if there is any, and they go. Returns whether they are
    /// still stacking (their orders are set); once through, the caller pushes the entry with
    /// `via = breach.inside`. No door between them: `self.breach` is None and nothing is set.
    fn plan_breach(&mut self, entry: usize, trader: Option<usize>, members: &[MemberView], arena: &Arena, target: V2, plan: bool) -> bool {
        let ep = members[entry].pos;
        if plan {
            let doors = self.doors.get_or_insert_with(|| rooms::doorways(arena));
            self.breach = rooms::breach_door(arena, doors, ep, target, BREACH_REACH)
                .map(|door| Breach { door, from: ep, inside: door.inside(ep, 2.5), t: 0.0, go_at: None, called: false });
            self.breach_throw = None;
        }
        let stacking = self.breach.is_some_and(|b| b.go_at.is_none_or(|g| self.play_t < g));
        if let Some(mut b) = self.breach {
            b.t += RADIO_PERIOD_S;
            let stack_e = b.door.stack(arena, b.from, 1.6, 0);
            let stack_t = b.door.stack(arena, b.from, 3.0, 0);
            if b.go_at.is_none() {
                let tr_ready = trader.is_none_or(|tr| members[tr].pos.dist(stack_t) < 2.5);
                if (ep.dist(stack_e) < 1.8 && tr_ready && b.t > 1.0) || b.t > BREACH_STACK_S {
                    let have = self.utility.flashes > self.flashes_used || self.utility.frags > self.frags_used;
                    let zone = arena.zone_of(b.inside).map_or("the room", |z| z.name);
                    if have {
                        self.breach_throw = Some(b.inside);
                        b.go_at = Some(self.play_t + grenade::WINDUP_S + grenade::FLASH_FUSE_S + 0.3);
                        self.bark(entry, format!("Stacked on {zone} door - utility in"), true);
                    } else {
                        b.go_at = Some(self.play_t + 0.3);
                        self.bark(entry, format!("Stacked on {zone} door - breaching"), true);
                    }
                }
            } else if !stacking && !b.called {
                b.called = true;
                self.bark(entry, "Breach! Go go go".into(), true);
            }
            self.breach = Some(b);
            if stacking {
                let watch = Some(b.door.pos);
                self.orders[entry] = Some(Order { role: Role::Entry, goal: Goal::Hold, dest: Some(stack_e), watch, threat: Some(target), sticky: true, via: None, prefire: None });
                if let Some(tr) = trader {
                    self.orders[tr] = Some(Order { role: Role::Trader, goal: Goal::Hold, dest: Some(stack_t), watch, threat: Some(target), sticky: true, via: None, prefire: None });
                }
            }
        }
        stacking
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

/// The entry for a breach or a push: a close-range gun among the three nearest (`by_dist`),
/// never a marksman.
fn pick_entry(by_dist: &[usize], members: &[MemberView]) -> usize {
    by_dist
        .iter()
        .take(3)
        .enumerate()
        .map(|(r, &i)| (entry_weight(members[i].traits.weapon) - 0.25 * r as f32, i))
        .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
        .map_or(by_dist[0], |x| x.1)
}

/// The entry's trader: the nearest other member, a marksman only if nobody else is left.
fn pick_trader(by_dist: &[usize], members: &[MemberView], entry: usize) -> Option<usize> {
    by_dist
        .iter()
        .copied()
        .find(|&i| i != entry && !members[i].traits.weapon.long_gun())
        .or_else(|| by_dist.iter().copied().find(|&i| i != entry))
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
