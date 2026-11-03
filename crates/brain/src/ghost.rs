//! The trainer ghost: the sim's stand-in for the human (docs/research/adaptive-squad-ai.md §4.1,
//! "Ghost v2"), plus its multi-parameter calibration to real match outcomes.
//!
//! `run_round` builds a [`GhostView`] each tick and calls [`Ghost::act`]; the ghost answers with a
//! [`GhostAction`] (velocity, optional shot). The sim resolves shots geometrically and reports
//! back through [`Ghost::on_shot`]. The ghost only uses what a human could know: enemies it can
//! see, shots it can hear, and the player model.

use crate::arena::Arena;
use crate::limits::Limits;
use crate::mind::EnemyMind;
use crate::math::{BrainRng, V2, angle_diff, gauss, rng_from_seed};
use crate::player_model::{HEAT_W, MatchTelemetry, PlayerModel, ROUTE_W, heat_center, range_bin, route_idx, sector_of};
use crate::sim::{Body, CROUCH_SPEED, DT, HP, WALK};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

/// Behaviour parameters of Ghost v2. Stored in `PlayerModel::ghost`; the calibrated subset is
/// [`CALIB_PARAMS`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GhostParams {
    /// Base aim spread (1σ, degrees) once settled and standing still.
    pub aim_sigma_deg: f32,
    /// Multiplies the model's reaction time.
    pub reaction_scale: f32,
    /// Max crosshair turn rate (rad/s): large flicks take time.
    pub turn_rate: f32,
    /// Time to settle after acquiring a target; spread is up to 2x wider until then.
    pub flick_s: f32,
    /// Extra spread at full walk speed (sigma *= 1 + move_spread * speed / WALK).
    pub move_spread: f32,
    /// Extra spread per rad/s of target angular velocity.
    pub track_spread: f32,
    /// Share of non-engaged time the crosshair sits on a pre-aim angle rather than the walk direction.
    pub preaim: f32,
    /// Re-peek probability per decision while holding cover; also P(push | just got a kill).
    pub aggression: f32,
    /// HP fraction below which the ghost falls back to cover.
    pub retreat_hp: f32,
    /// Peek-side preference added to the model's strafe bias (-1 left … +1 right).
    pub peek_side: f32,
}

impl Default for GhostParams {
    fn default() -> Self {
        GhostParams {
            aim_sigma_deg: 4.0,
            reaction_scale: 1.0,
            turn_rate: 9.0,
            flick_s: 0.15,
            move_spread: 1.0,
            track_spread: 0.5,
            preaim: 0.8,
            aggression: 0.5,
            retreat_hp: 0.35,
            peek_side: 0.0,
        }
    }
}

/// Calibrated parameters: (name, lo, hi). The optimiser works in [0,1] per parameter.
pub const CALIB_PARAMS: [(&str, f32, f32); 6] = [
    ("aim_sigma_deg", 1.0, 10.0),
    ("reaction_scale", 0.5, 2.5),
    ("turn_rate", 3.0, 16.0),
    ("preaim", 0.0, 1.0),
    ("aggression", 0.0, 1.0),
    ("retreat_hp", 0.0, 0.7),
];

impl GhostParams {
    fn slot(&mut self, i: usize) -> &mut f32 {
        match i {
            0 => &mut self.aim_sigma_deg,
            1 => &mut self.reaction_scale,
            2 => &mut self.turn_rate,
            3 => &mut self.preaim,
            4 => &mut self.aggression,
            _ => &mut self.retreat_hp,
        }
    }
    /// Calibrated subset, normalised to [0,1].
    pub fn to_unit(&self) -> [f32; 6] {
        let mut p = *self;
        std::array::from_fn(|i| {
            let (_, lo, hi) = CALIB_PARAMS[i];
            ((*p.slot(i) - lo) / (hi - lo)).clamp(0.0, 1.0)
        })
    }
    pub fn with_unit(mut self, u: &[f32; 6]) -> Self {
        for (i, &x) in u.iter().enumerate() {
            let (_, lo, hi) = CALIB_PARAMS[i];
            *self.slot(i) = lo + x.clamp(0.0, 1.0) * (hi - lo);
        }
        self
    }
}

/// Which ghost to run. `V1` is the original coin-flip ghost, kept as a baseline for tests and benches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GhostMode {
    V1,
    #[default]
    V2,
}

/// What the ghost may know about one enemy this tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct EnemyView {
    pub pos: V2,
    pub vel: V2,
    pub alive: bool,
    pub crouched: bool,
    /// In the ghost's view (LOS and view cone), computed by the sim.
    pub visible: bool,
    /// Fired this tick (audible within `loud` m).
    pub fired: bool,
    /// How far (m) that shot carries for the ghost (by the enemy's gun).
    pub loud: f32,
}

pub struct GhostView<'a> {
    pub arena: &'a Arena,
    pub model: &'a PlayerModel,
    pub enemies: &'a [EnemyView],
}

#[derive(Clone, Copy, Debug)]
pub struct Shot {
    pub dir: V2,
    pub pitch_err: f32,
    /// Enemy the ghost aimed at.
    pub target: usize,
    /// V1 only: pre-rolled hit/miss (no geometry).
    pub scripted_hit: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GhostAction {
    pub vel: V2,
    pub shot: Option<Shot>,
}

/// Per-round ghost statistics, comparable with `MatchTelemetry`.
#[derive(Clone, Copy, Debug, Default)]
pub struct GhostStats {
    pub shots: [f32; 6],
    pub hits: [f32; 6],
    pub kills: u32,
    /// Seconds in which at least one enemy saw the ghost (the game's "engaged").
    pub engaged_s: f32,
    /// First-sight-to-first-shot times (sum, count).
    pub reaction_sum: f32,
    pub reaction_n: f32,
    /// Hits that landed on a head (rate from `PlayerModel::headshot_rate`).
    pub headshots: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tactic {
    Roam,
    Hold,
    Peek,
    Retreat,
    Push,
}

/// Scripted stand-in for the human, parameterised by the player model and `GhostParams`.
pub struct Ghost {
    pub(crate) body: Body,
    pub(crate) facing: f32,
    pub stats: GhostStats,
    mode: GhostMode,
    p: GhostParams,
    rng: BrainRng,
    reaction: f32,
    pref_range: f32,
    // Navigation.
    dest: Option<V2>,
    path: Vec<V2>,
    still_t: f32,
    // Aim.
    target: Option<usize>,
    target_seen: f32,
    settle_t: f32,
    fired_at_target: bool,
    last_bearing: f32,
    // Memory: last place a threat was seen or heard.
    threat: Option<V2>,
    threat_age: f32,
    // Pre-aim.
    preaim_yaw: f32,
    preaim_t: f32,
    preaim_from: V2,
    use_preaim: bool,
    // Tactics.
    tactic: Tactic,
    tactic_t: f32,
    cover: Option<V2>,
    peek_spot: Option<V2>,
    decide_t: f32,
    strafe_dir: f32,
    strafe_t: f32,
    strafing: bool,
}

const MIDS: [f32; 6] = [3.0, 7.5, 12.5, 17.5, 25.0, 32.0];

impl Ghost {
    pub fn new(pos: V2, model: &PlayerModel, seed: u64, mode: GhostMode) -> Self {
        Self::with_params(pos, model, model.ghost, seed, mode)
    }

    pub fn with_params(pos: V2, model: &PlayerModel, p: GhostParams, seed: u64, mode: GhostMode) -> Self {
        let mut rng = rng_from_seed(seed ^ 0x9E37_79B9);
        let jitter = 0.8 + 0.4 * rng.random::<f32>();
        let reaction = match mode {
            GhostMode::V1 => model.reaction_s / model.ghost_skill.max(0.1) * jitter,
            GhostMode::V2 => model.reaction_s * p.reaction_scale * jitter,
        };
        let b = (0..6).max_by(|&a, &b| model.kill_share[a].total_cmp(&model.kill_share[b])).unwrap_or(2);
        Ghost {
            body: Body::with_gun(pos, model.favourite_weapon()),
            facing: -std::f32::consts::FRAC_PI_2,
            stats: GhostStats::default(),
            mode,
            p,
            rng,
            reaction,
            pref_range: MIDS[b],
            dest: None,
            path: Vec::new(),
            still_t: 0.0,
            target: None,
            target_seen: 0.0,
            settle_t: 0.0,
            fired_at_target: false,
            last_bearing: 0.0,
            threat: None,
            threat_age: 99.0,
            preaim_yaw: -std::f32::consts::FRAC_PI_2,
            preaim_t: 0.0,
            preaim_from: V2::new(999.0, 999.0),
            use_preaim: true,
            tactic: Tactic::Roam,
            tactic_t: 0.0,
            cover: None,
            peek_spot: None,
            decide_t: 0.0,
            strafe_dir: 1.0,
            strafe_t: 0.0,
            strafing: false,
        }
    }

    /// Decide this tick. Advances the ghost's own weapon timers and facing.
    pub fn act(&mut self, v: &GhostView) -> GhostAction {
        self.body.hit_t += DT;
        match self.mode {
            GhostMode::V1 => self.act_v1(v),
            GhostMode::V2 => self.act_v2(v),
        }
    }

    /// Shot feedback from the sim.
    pub fn on_shot(&mut self, dist: f32, hit: bool, killed: bool, v: &GhostView) {
        let b = range_bin(dist);
        self.stats.shots[b] += 1.0;
        if hit {
            self.stats.hits[b] += 1.0;
        }
        if killed {
            self.stats.kills += 1;
            if self.mode == GhostMode::V2 {
                self.after_kill(v);
            }
        }
    }

    fn nearest_visible(&self, v: &GhostView) -> Option<(f32, usize)> {
        let mut best: Option<(f32, usize)> = None;
        for (i, e) in v.enemies.iter().enumerate() {
            if e.alive && e.visible {
                let d = e.pos.dist(self.body.pos);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, i));
                }
            }
        }
        best
    }

    // ------------------------------------------------------------------ V1 (baseline)

    fn act_v1(&mut self, v: &GhostView) -> GhostAction {
        let model = v.model;
        let mut out = GhostAction::default();
        match self.nearest_visible(v) {
            Some((dist, i)) => {
                if self.target != Some(i) {
                    self.target = Some(i);
                    self.target_seen = 0.0;
                }
                self.target_seen += DT;
                let to = (v.enemies[i].pos - self.body.pos).norm();
                self.facing = to.angle();
                self.strafe_t -= DT;
                if self.strafe_t <= 0.0 {
                    self.strafe_t = 0.4 + 0.6 * self.rng.random::<f32>();
                    let p_right = 0.5 + 0.5 * model.strafe_bias;
                    self.strafe_dir = if self.rng.random::<f32>() < p_right { -1.0 } else { 1.0 };
                }
                self.body.crouched =
                    self.rng.random::<f32>() < model.crouch_rate * 0.1 || (self.body.crouched && self.rng.random::<f32>() < 0.9);
                let range_push = ((dist - self.pref_range) / 10.0).clamp(-1.0, 1.0);
                let mv = (to.perp() * self.strafe_dir + to * range_push).clamp_len(1.0);
                let speed = if self.body.crouched { CROUCH_SPEED } else { WALK };
                out.vel = mv * speed;
                self.path.clear();
                self.dest = None;
                let want_fire = self.target_seen >= self.reaction;
                if self.body.try_fire(want_fire) {
                    let acc = (model.accuracy[range_bin(dist)] * model.ghost_skill).min(0.9);
                    let hit = self.rng.random::<f32>() < acc;
                    out.shot = Some(Shot { dir: to, pitch_err: 0.0, target: i, scripted_hit: Some(hit) });
                }
            }
            None => {
                self.target = None;
                self.body.crouched = false;
                self.body.try_fire(false);
                if self.still_t > 0.0 {
                    self.still_t -= DT;
                    self.facing += 0.6 * DT;
                } else {
                    if self.path.is_empty() {
                        if self.dest.is_some() && self.rng.random::<f32>() < model.camping {
                            self.still_t = 2.0 + 4.0 * self.rng.random::<f32>();
                        }
                        let d = self.pick_dest(model, v.arena);
                        self.dest = Some(d);
                        self.path = v.arena.nav.find_path(self.body.pos, d).unwrap_or_default();
                    }
                    let dir = self.follow_path();
                    out.vel = dir * WALK;
                    if dir.len() > 0.0 {
                        self.facing = dir.angle();
                    }
                }
            }
        }
        out
    }

    fn pick_dest(&mut self, model: &PlayerModel, arena: &Arena) -> V2 {
        // Half the time follow a learned route transition, otherwise sample the heatmap.
        let from = route_idx(self.body.pos);
        let w = ROUTE_W * ROUTE_W;
        let row = &model.routes[from * w..(from + 1) * w];
        let row_sum: f32 = row.iter().sum();
        if row_sum > 0.0 && self.rng.random::<f32>() < 0.5 {
            let mut r = self.rng.random::<f32>() * row_sum;
            for (i, &p) in row.iter().enumerate() {
                r -= p;
                if r <= 0.0 {
                    let cell = 2.0 * crate::arena::GRID_HALF / ROUTE_W as f32;
                    let c = V2::new(-crate::arena::GRID_HALF + ((i % ROUTE_W) as f32 + 0.5) * cell, -crate::arena::GRID_HALF + ((i / ROUTE_W) as f32 + 0.5) * cell);
                    return arena.nav.nearest_walkable(c);
                }
            }
        }
        let total: f32 = model.heat.iter().sum();
        let mut r = self.rng.random::<f32>() * total;
        for i in 0..HEAT_W * HEAT_W {
            r -= model.heat[i];
            if r <= 0.0 {
                return arena.nav.nearest_walkable(heat_center(i));
            }
        }
        arena.nav.nearest_walkable(V2::ZERO)
    }

    /// Unit direction along the current path (zero when there's none).
    fn follow_path(&mut self) -> V2 {
        while let Some(&wp) = self.path.first() {
            if wp.dist(self.body.pos) < 0.6 {
                self.path.remove(0);
            } else {
                return (wp - self.body.pos).norm();
            }
        }
        V2::ZERO
    }

    fn go_to(&mut self, arena: &Arena, d: V2) {
        self.dest = Some(d);
        self.path = arena.nav.find_path(self.body.pos, d).unwrap_or_default();
    }

    // ------------------------------------------------------------------ V2

    fn act_v2(&mut self, v: &GhostView) -> GhostAction {
        let a = v.arena;
        let model = v.model;
        let pos = self.body.pos;
        self.tactic_t += DT;
        self.decide_t -= DT;

        // 1. Memory: what it sees or hears.
        let seen = self.nearest_visible(v);
        self.threat_age += DT;
        if let Some((_, i)) = seen {
            self.threat = Some(v.enemies[i].pos);
            self.threat_age = 0.0;
        } else if let Some(e) = v.enemies.iter().filter(|e| e.alive && e.fired && e.pos.dist(pos) < e.loud).min_by(|x, y| {
            x.pos.dist(pos).total_cmp(&y.pos.dist(pos))
        }) {
            self.threat = Some(e.pos);
            self.threat_age = 0.0;
        }
        let threat_recent = self.threat.filter(|_| self.threat_age < 6.0);

        // 2. Target and reaction.
        let target = seen.map(|(_, i)| i);
        if target != self.target {
            // A fresh target restarts reaction; a quick switch between visible enemies is faster.
            self.target_seen = if self.target.is_some() && target.is_some() { self.reaction * 0.4 } else { 0.0 };
            self.fired_at_target = false;
            self.target = target;
            self.settle_t = 0.0;
            if let Some(i) = target {
                self.last_bearing = (v.enemies[i].pos - pos).angle();
            }
        }

        // 3. Low HP / ammo housekeeping.
        let low_hp = self.body.hp < self.p.retreat_hp * HP;
        // Fall back only while exposed to the threat (once hidden, it holds there).
        if low_hp
            && self.tactic != Tactic::Retreat
            && let Some(t) = self.threat
            && a.line_of_sight(pos, false, t, false)
            && let Some(c) = find_cover(a, pos, t, 18.0)
        {
            self.tactic = Tactic::Retreat;
            self.tactic_t = 0.0;
            self.cover = Some(c);
            self.go_to(a, c);
        }
        let mut reload = false;
        if target.is_none() && !self.body.reloading() && self.body.gun.ammo() < self.body.gun.spec().mag / 3 {
            let covered = self.threat.is_none_or(|t| !a.line_of_sight(pos, false, t, false));
            if covered || self.threat_age > 3.0 {
                reload = true;
            } else if self.tactic != Tactic::Retreat
                && let Some(c) = find_cover(a, pos, self.threat.unwrap_or(pos), 10.0)
            {
                self.tactic = Tactic::Retreat;
                self.tactic_t = 0.0;
                self.cover = Some(c);
                self.go_to(a, c);
            }
        }
        if reload {
            self.body.gun.start_reload();
        }

        // 4. Movement.
        let mut vel = V2::ZERO;
        if let Some(i) = target {
            let e = v.enemies[i];
            let dist = e.pos.dist(pos);
            let to = (e.pos - pos).norm();
            match self.tactic {
                Tactic::Retreat => vel = self.follow_path() * WALK,
                Tactic::Hold | Tactic::Peek => {
                    // Holding an angle: stand and shoot, crouch per the model.
                    self.body.crouched = self.rng.random::<f32>() < model.crouch_rate * 0.1 || (self.body.crouched && self.rng.random::<f32>() < 0.95);
                    if self.tactic == Tactic::Peek && self.tactic_t > 1.2 {
                        // Peeked long enough: duck back.
                        self.tactic = Tactic::Hold;
                        self.tactic_t = 0.0;
                    }
                    if self.tactic == Tactic::Hold
                        && let Some(c) = self.cover
                        && c.dist(pos) > 0.5
                        && self.tactic_t > 1.0
                    {
                        vel = (c - pos).norm() * WALK;
                    }
                }
                _ => {
                    // Open ground: counter-strafe (move, stop to shoot), keep the preferred range.
                    self.strafe_t -= DT;
                    if self.strafe_t <= 0.0 {
                        self.strafing = !self.strafing;
                        self.strafe_t = if self.strafing { 0.3 + 0.3 * self.rng.random::<f32>() } else { 0.25 + 0.25 * self.rng.random::<f32>() };
                        let p_right = 0.5 + 0.5 * (model.strafe_bias + self.p.peek_side).clamp(-1.0, 1.0);
                        self.strafe_dir = if self.rng.random::<f32>() < p_right { -1.0 } else { 1.0 };
                    }
                    self.body.crouched = self.rng.random::<f32>() < model.crouch_rate * 0.1 || (self.body.crouched && self.rng.random::<f32>() < 0.9);
                    let range_push = ((dist - self.pref_range) / 10.0).clamp(-1.0, 1.0);
                    let mv = if self.strafing { to.perp() * self.strafe_dir + to * range_push } else { to * range_push * 0.3 };
                    let speed = if self.body.crouched { CROUCH_SPEED } else { WALK };
                    vel = mv.clamp_len(1.0) * speed;
                    self.path.clear();
                    self.dest = None;
                }
            }
        } else {
            self.body.crouched = false;
            vel = self.move_unengaged(v, threat_recent);
        }
        self.body.vel = vel;

        // 5. Crosshair: turn-rate limited toward target, last threat, or a pre-aim angle.
        let desired = if let Some(i) = target {
            (v.enemies[i].pos - pos).angle()
        } else if let Some(t) = threat_recent {
            (t - pos).angle()
        } else {
            self.update_preaim(v);
            if !self.use_preaim && vel.len() > 0.1 { vel.angle() } else { self.preaim_yaw }
        };
        let turn = angle_diff(desired, self.facing).clamp(-self.p.turn_rate * DT, self.p.turn_rate * DT);
        self.facing = (self.facing + turn).rem_euclid(TAU);

        // 6. Trigger and spread.
        let mut out = GhostAction { vel, shot: None };
        let mut want_fire = false;
        let mut sigma = 0.0;
        if let Some(i) = target {
            let e = v.enemies[i];
            let bearing = (e.pos - pos).angle();
            let ang_vel = (angle_diff(bearing, self.last_bearing) / DT).abs();
            self.last_bearing = bearing;
            self.target_seen += DT;
            let err = angle_diff(bearing, self.facing).abs();
            if err < 0.05 {
                self.settle_t += DT;
            } else {
                self.settle_t = 0.0;
            }
            let settle = 1.0 + (1.0 - (self.settle_t / self.p.flick_s.max(0.01)).min(1.0));
            let moving = 1.0 + self.p.move_spread * vel.len() / WALK;
            let tracking = 1.0 + self.p.track_spread * ang_vel;
            let crouch = if self.body.crouched { 0.85 } else { 1.0 };
            sigma = self.p.aim_sigma_deg.to_radians() * settle * moving * tracking * crouch;
            want_fire = self.target_seen >= self.reaction && err < 0.08;
        }
        if self.body.try_fire(want_fire)
            && let Some(i) = target
        {
            let dir = V2::from_angle(self.facing + gauss(&mut self.rng) * sigma);
            out.shot = Some(Shot { dir, pitch_err: gauss(&mut self.rng) * sigma * 0.6, target: i, scripted_hit: None });
            if !self.fired_at_target {
                self.fired_at_target = true;
                self.stats.reaction_sum += self.target_seen;
                self.stats.reaction_n += 1.0;
            }
        }
        out
    }

    fn move_unengaged(&mut self, v: &GhostView, threat: Option<V2>) -> V2 {
        let a = v.arena;
        let pos = self.body.pos;
        match self.tactic {
            Tactic::Retreat | Tactic::Push => {
                let dir = self.follow_path();
                if dir.len() > 0.0 {
                    return dir * WALK;
                }
                // Arrived: hold here.
                self.tactic = Tactic::Hold;
                self.tactic_t = 0.0;
                self.cover = Some(pos);
                V2::ZERO
            }
            Tactic::Peek => {
                match self.peek_spot {
                    Some(s) if self.tactic_t < 1.2 && s.dist(pos) > 0.3 => (s - pos).norm() * WALK,
                    Some(_) if self.tactic_t < 1.2 => V2::ZERO,
                    _ => {
                        // Nothing there: back into cover.
                        self.tactic = Tactic::Hold;
                        self.tactic_t = 0.0;
                        V2::ZERO
                    }
                }
            }
            Tactic::Hold => {
                if let Some(c) = self.cover
                    && c.dist(pos) > 0.4
                {
                    return (c - pos).norm() * WALK;
                }
                match threat {
                    Some(t) if self.decide_t <= 0.0 => {
                        // Re-peek the angle at the modelled rate, otherwise keep holding.
                        self.decide_t = 1.0 + self.rng.random::<f32>();
                        if self.rng.random::<f32>() < self.p.aggression {
                            let side = (v.model.strafe_bias + self.p.peek_side).clamp(-1.0, 1.0);
                            self.peek_spot = peek_spot(a, pos, t, side, &mut self.rng);
                            if self.peek_spot.is_some() {
                                self.tactic = Tactic::Peek;
                                self.tactic_t = 0.0;
                            }
                        }
                        V2::ZERO
                    }
                    Some(_) => V2::ZERO,
                    None => {
                        if self.tactic_t > self.still_t {
                            self.tactic = Tactic::Roam;
                            self.path.clear();
                        }
                        V2::ZERO
                    }
                }
            }
            Tactic::Roam => {
                if let Some(t) = threat {
                    // Contact: get to cover facing it.
                    if let Some(c) = find_cover(a, pos, t, 8.0) {
                        self.tactic = Tactic::Retreat;
                        self.tactic_t = 0.0;
                        self.cover = Some(c);
                        self.go_to(a, c);
                        return self.follow_path() * WALK;
                    }
                }
                if self.path.is_empty() {
                    if self.dest.is_some() && self.rng.random::<f32>() < v.model.camping {
                        // Hold a nearby cover spot for a while, pre-aiming.
                        self.still_t = 2.0 + 4.0 * self.rng.random::<f32>();
                        self.tactic = Tactic::Hold;
                        self.tactic_t = 0.0;
                        self.cover = nearest_cover(a, pos, 6.0);
                        self.dest = None;
                        return V2::ZERO;
                    }
                    let d = self.pick_dest(v.model, a);
                    self.go_to(a, d);
                }
                self.follow_path() * WALK
            }
        }
    }

    /// Conditional decision after a kill: push the next remembered enemy, or reset to cover.
    fn after_kill(&mut self, v: &GhostView) {
        let pos = self.body.pos;
        if self.rng.random::<f32>() < self.p.aggression {
            let next = v
                .enemies
                .iter()
                .filter(|e| e.alive && (e.visible || (e.fired && e.pos.dist(pos) < e.loud)))
                .map(|e| e.pos)
                .next()
                .or(self.threat);
            if let Some(t) = next {
                self.tactic = Tactic::Push;
                self.tactic_t = 0.0;
                self.go_to(v.arena, v.arena.nav.nearest_walkable(t));
            }
        } else if let Some(c) = nearest_cover(v.arena, pos, 8.0) {
            self.tactic = Tactic::Retreat;
            self.tactic_t = 0.0;
            self.cover = Some(c);
            self.go_to(v.arena, c);
        }
    }

    /// Score look directions by where enemies are likely to appear: the squad's approach lanes
    /// (from their spawns), the player's own look habits, open sightlines, and remembered threats.
    fn update_preaim(&mut self, v: &GhostView) {
        let pos = self.body.pos;
        self.preaim_t -= DT;
        if self.preaim_t > 0.0 && pos.dist(self.preaim_from) < 1.5 {
            return;
        }
        self.preaim_t = 0.5;
        self.preaim_from = pos;
        self.use_preaim = self.rng.random::<f32>() < self.p.preaim;
        let a = v.arena;
        let n = a.enemy_spawns.len().max(1) as f32;
        let spawn = a.enemy_spawns.iter().fold(V2::ZERO, |s, &p| s + p) * (1.0 / n);
        let to_spawn = (spawn - pos).norm();
        let mut best = (f32::MIN, self.preaim_yaw);
        for k in 0..24 {
            let ang = k as f32 * TAU / 24.0;
            let dir = V2::from_angle(ang);
            let mut sight = 0.0;
            let mut d = 3.0;
            while d <= 36.0 && a.line_of_sight(pos, false, pos + dir * d, false) {
                sight = d;
                d += 3.0;
            }
            let open = (sight / 24.0).min(1.0);
            let lane = dir.dot(to_spawn).max(0.0);
            let habit = v.model.facing[sector_of(ang)] * 4.0;
            let memory = self.threat.map_or(0.0, |t| dir.dot((t - pos).norm()).max(0.0) * 2.0 / (1.0 + self.threat_age / 10.0));
            let score = open * (0.3 + lane + habit + memory) + 0.1 * self.rng.random::<f32>();
            if score > best.0 {
                best = (score, ang);
            }
        }
        self.preaim_yaw = best.1;
    }
}

/// Closest cover spot within `max_d` hidden (standing) from `threat`.
fn find_cover(a: &Arena, from: V2, threat: V2, max_d: f32) -> Option<V2> {
    a.cover
        .iter()
        .map(|c| c.pos)
        .filter(|&c| c.dist(from) < max_d && !a.line_of_sight(c, false, threat, false))
        .min_by(|x, y| x.dist(from).total_cmp(&y.dist(from)))
}

fn nearest_cover(a: &Arena, from: V2, max_d: f32) -> Option<V2> {
    a.cover.iter().map(|c| c.pos).filter(|c| c.dist(from) < max_d).min_by(|x, y| x.dist(from).total_cmp(&y.dist(from)))
}

/// A spot just beside cover that sees `threat`, on the preferred side first.
fn peek_spot(a: &Arena, from: V2, threat: V2, side: f32, rng: &mut BrainRng) -> Option<V2> {
    let to = (threat - from).norm();
    let first = if rng.random::<f32>() < 0.5 + 0.5 * side { -1.0 } else { 1.0 };
    for s in [first, -first] {
        for off in [1.2, 2.0, 2.8] {
            let p = from + to.perp() * (s * off) + to * 0.3;
            if a.nav.walkable_at(p) && a.line_of_sight(p, false, threat, false) {
                return Some(p);
            }
        }
    }
    None
}

// ====================================================================== Calibration

/// Real per-match statistics the ghost is calibrated against. `None` = not enough data.
#[derive(Clone, Copy, Debug, Default)]
pub struct RealStats {
    /// Engaged seconds per kill.
    pub ttk: Option<f32>,
    /// Mean round length (s).
    pub round_len: Option<f32>,
    pub accuracy: [Option<f32>; 6],
    /// Share of duels the player won.
    pub duel_win: Option<f32>,
    /// Mean first-sight-to-first-shot time (s).
    pub reaction: Option<f32>,
}

/// Telemetry sample period (the game samples at 10 Hz).
pub const SAMPLE_S: f32 = 0.1;

/// Real duel win rate from the engagement log (paper §4.1 L0): wins over decided duels
/// (disengages excluded); None with fewer than 3 decided duels.
pub fn duel_win_rate(t: &MatchTelemetry) -> Option<f32> {
    let decided = t.duels.iter().filter(|d| d.decided()).count();
    (decided >= 3).then(|| t.duels.iter().filter(|d| d.won()).count() as f32 / decided as f32)
}

impl RealStats {
    pub fn from_telemetry(t: &MatchTelemetry, rounds: u32) -> Self {
        let kills: f32 = t.kills.iter().sum();
        let engaged = t.engaged_samples * SAMPLE_S;
        RealStats {
            ttk: (kills >= 1.0 && engaged > 0.0).then(|| engaged / kills),
            round_len: (rounds > 0 && t.samples > 0.0).then(|| t.samples * SAMPLE_S / rounds as f32),
            accuracy: std::array::from_fn(|b| (t.shots[b] >= 5.0).then(|| t.hits[b] / t.shots[b])),
            duel_win: duel_win_rate(t),
            reaction: (t.reaction_samples >= 1.0).then(|| t.reaction_sum / t.reaction_samples),
        }
    }
}

/// Aggregated sim statistics over a batch of rounds.
#[derive(Clone, Copy, Debug, Default)]
pub struct SimStats {
    pub rounds: f32,
    pub player_wins: f32,
    pub duration: f32,
    pub engaged_s: f32,
    pub kills: f32,
    pub reaction_sum: f32,
    pub reaction_n: f32,
    pub shots: [f32; 6],
    pub hits: [f32; 6],
}

impl SimStats {
    pub fn add(&mut self, r: &crate::sim::RoundResult) {
        self.rounds += 1.0;
        self.player_wins += r.player_won as u8 as f32;
        self.duration += r.duration;
        self.engaged_s += r.ghost.engaged_s;
        self.kills += r.ghost.kills as f32;
        self.reaction_sum += r.ghost.reaction_sum;
        self.reaction_n += r.ghost.reaction_n;
        for b in 0..6 {
            self.shots[b] += r.ghost.shots[b];
            self.hits[b] += r.ghost.hits[b];
        }
    }
    pub fn merge(&mut self, o: &SimStats) {
        self.rounds += o.rounds;
        self.player_wins += o.player_wins;
        self.duration += o.duration;
        self.engaged_s += o.engaged_s;
        self.kills += o.kills;
        self.reaction_sum += o.reaction_sum;
        self.reaction_n += o.reaction_n;
        for b in 0..6 {
            self.shots[b] += o.shots[b];
            self.hits[b] += o.hits[b];
        }
    }
    pub fn win_rate(&self) -> f32 {
        self.player_wins / self.rounds.max(1.0)
    }
    /// The same statistics a real match yields (for planted-parameter tests).
    pub fn as_real(&self) -> RealStats {
        RealStats {
            ttk: (self.kills > 0.0).then(|| self.engaged_s / self.kills),
            round_len: (self.rounds > 0.0).then(|| self.duration / self.rounds),
            accuracy: std::array::from_fn(|b| (self.shots[b] >= 5.0).then(|| self.hits[b] / self.shots[b])),
            duel_win: None,
            reaction: (self.reaction_n > 0.0).then(|| self.reaction_sum / self.reaction_n),
        }
    }
}

/// One replayed squad with its real result (player rounds won / played).
#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    pub player_rounds: f32,
    pub rounds: f32,
}

/// Calibration error: RMS of normalised residuals (0 = perfect). Terms: round-win rate per replayed
/// squad (weighted by rounds played), time-to-kill, reaction time, round length, accuracy by range,
/// duel win rate.
pub fn calibration_error(outcomes: &[Outcome], per_squad: &[SimStats], real: &RealStats) -> f32 {
    let mut terms: Vec<f32> = Vec::new();
    let n: f32 = outcomes.iter().map(|o| o.rounds).sum();
    if n > 0.0 {
        let mse = outcomes.iter().zip(per_squad).map(|(o, s)| o.rounds * ((s.win_rate() - o.player_rounds / o.rounds) / 0.15).powi(2)).sum::<f32>() / n;
        terms.push(mse);
    }
    let mut all = SimStats::default();
    per_squad.iter().for_each(|s| all.merge(s));
    let sim = all.as_real();
    if let Some(r) = real.ttk {
        // A ghost that never kills has an effectively infinite TTK.
        let s = sim.ttk.unwrap_or(all.engaged_s.max(r) * 4.0 + 60.0);
        terms.push(((s.max(0.05) / r.max(0.05)).ln() / 0.35).powi(2));
    }
    if let (Some(r), Some(s)) = (real.reaction, sim.reaction) {
        terms.push(((s.max(0.02) / r.max(0.02)).ln() / 0.25).powi(2));
    }
    if let (Some(r), Some(s)) = (real.round_len, sim.round_len) {
        terms.push(((s.max(0.5) / r.max(0.5)).ln() / 0.25).powi(2));
    }
    let acc: Vec<f32> = (0..6)
        .filter_map(|b| real.accuracy[b].map(|r| ((sim.accuracy[b].unwrap_or(0.0) - r) / 0.07).powi(2)))
        .collect();
    if !acc.is_empty() {
        terms.push(acc.iter().sum::<f32>() / acc.len() as f32);
    }
    // Duel term: needs the sim-side duel log too; wire here when the engagement log lands.
    if terms.is_empty() { 0.0 } else { (terms.iter().sum::<f32>() / terms.len() as f32).sqrt() }
}

#[derive(Clone, Debug)]
pub struct CalibConfig {
    pub iters: usize,
    pub lambda: usize,
    /// Seeded rounds per replayed squad per evaluation (common random numbers across candidates).
    pub rounds: usize,
    pub threads: usize,
    pub seed: u64,
    pub deadline: Option<std::time::Instant>,
    /// Pull toward the default parameters (mean squared unit distance x this), so parameters the
    /// data can't pin down stay plausible instead of drifting to their bounds.
    pub prior: f32,
}

impl Default for CalibConfig {
    fn default() -> Self {
        CalibConfig { iters: 14, lambda: 6, rounds: 8, threads: 4, seed: 1, deadline: None, prior: 0.5 }
    }
}

#[derive(Clone, Debug)]
pub struct CalibResult {
    pub params: GhostParams,
    pub error_before: f32,
    pub error_after: f32,
    /// Ghost player-win rate per replayed squad, before calibration.
    pub win_before: Vec<f32>,
    pub evaluations: usize,
}

/// Builds replayed squad `g` for a round seed, with the limits it was deployed under.
pub trait SquadFactory: Fn(usize, u64) -> (Vec<EnemyMind>, Limits) + Sync {}
impl<F: Fn(usize, u64) -> (Vec<EnemyMind>, Limits) + Sync> SquadFactory for F {}

/// Simulate `params` against every replayed squad.
pub fn simulate(arena: &Arena, model: &PlayerModel, params: GhostParams, squads: usize, rounds: usize, seed: u64, make_minds: &impl SquadFactory) -> Vec<SimStats> {
    let mut m = model.clone();
    m.ghost = params;
    (0..squads)
        .map(|g| {
            let mut s = SimStats::default();
            for k in 0..rounds {
                let rs = seed.wrapping_add((g * 1000 + k) as u64 * 7919);
                let (minds, limits) = make_minds(g, rs);
                s.add(&crate::sim::run_round(arena, &m, limits, minds, rs));
            }
            s
        })
        .collect()
}

/// (1+λ)-ES with 1/5-success step adaptation over the [`CALIB_PARAMS`] subset, starting from
/// `model.ghost`. Deterministic for a given config (unless `deadline` cuts it short).
pub fn calibrate(arena: &Arena, model: &PlayerModel, outcomes: &[Outcome], real: &RealStats, cfg: &CalibConfig, make_minds: &impl SquadFactory) -> CalibResult {
    let squads = outcomes.len().max(1);
    let eval = |p: GhostParams| {
        let s = simulate(arena, model, p, squads, cfg.rounds, cfg.seed, make_minds);
        (calibration_error(outcomes, &s, real), s)
    };
    let u_def = GhostParams::default().to_unit();
    let penalty = |p: &GhostParams| cfg.prior * p.to_unit().iter().zip(&u_def).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / 6.0;
    let mut best = model.ghost;
    let (mut best_err, s0) = eval(best);
    let error_before = best_err;
    let mut best_obj = best_err + penalty(&best);
    let win_before = s0.iter().map(SimStats::win_rate).collect();
    let mut rng = rng_from_seed(cfg.seed ^ 0xCA11_B8A7);
    let mut step = 0.18f32;
    let mut evaluations = 1;
    for _ in 0..cfg.iters {
        if cfg.deadline.is_some_and(|d| std::time::Instant::now() >= d) || best_err < 1e-3 {
            break;
        }
        let u = best.to_unit();
        let cands: Vec<GhostParams> = (0..cfg.lambda)
            .map(|_| {
                let x: [f32; 6] = std::array::from_fn(|i| u[i] + gauss(&mut rng) * step);
                best.with_unit(&x)
            })
            .collect();
        let errs = crate::evolve::run_parallel(&cands, cfg.threads, |&p| eval(p).0);
        let objs: Vec<f32> = errs.iter().zip(&cands).map(|(e, p)| e + penalty(p)).collect();
        evaluations += cands.len();
        let (k, &o) = objs.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1)).expect("lambda > 0");
        if o < best_obj {
            best_obj = o;
            best_err = errs[k];
            best = cands[k];
            step = (step * 1.3).min(0.4);
        } else {
            step = (step * 0.8).max(0.03);
        }
    }
    CalibResult { params: best, error_before, error_after: best_err, win_before, evaluations }
}
