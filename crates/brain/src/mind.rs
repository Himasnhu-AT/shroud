//! `EnemyMind`: joins traits, utility tactics, NEAT micro and human-like aiming.
//! The game and the 2D trainer both call `think(&Observation, &MindCtx) -> Action`.

use crate::arena::Arena;
use crate::genome::{TraitGenome, Traits};
use crate::limits::Limits;
use crate::math::{BrainRng, V2, angle_diff, gauss, rng_from_seed};
use crate::neat;
use crate::player_model::PlayerModel;
use crate::squad::{self, Order};
use crate::utility::{self, Goal, UtilityInput};

pub const NEAT_INPUTS: usize = 14;
pub const NEAT_OUTPUTS: usize = 4;
/// How fast enemies can turn their aim (rad/s): a quick flick by the player breaks their aim.
pub const MAX_TURN_RATE: f32 = std::f32::consts::TAU;
pub const ENGAGE_MEMORY_S: f32 = 1.5;
/// After a hit, aim spread is up to `1 + FLINCH_SPREAD` times wider, easing back over `FLINCH_S`.
pub const FLINCH_S: f32 = 0.25;
pub const FLINCH_SPREAD: f32 = 1.5;

#[derive(Clone, Copy, Debug, Default)]
pub struct AllyInfo {
    pub pos: V2,
    pub engaged: bool,
    pub flanking: bool,
}

/// How often the squad radios sightings to each other.
pub const RADIO_PERIOD_S: f32 = 0.35;

/// Squad blackboard: the freshest player sighting any squad member has radioed in.
/// Updated every `RADIO_PERIOD_S`, so shared info is slightly late (fair, human-like).
#[derive(Clone, Copy, Debug)]
pub struct Blackboard {
    pub pos: V2,
    /// Seconds since that sighting.
    pub age: f32,
    timer: f32,
}

impl Default for Blackboard {
    fn default() -> Self {
        Blackboard { pos: V2::ZERO, age: 999.0, timer: 0.0 }
    }
}

impl Blackboard {
    /// Advance time; on a radio tick take the freshest of the members' (pos, seconds since seen).
    pub fn tick(&mut self, dt: f32, sightings: impl Iterator<Item = (V2, f32)>) {
        self.age += dt;
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.timer = RADIO_PERIOD_S;
            for (pos, age) in sightings {
                if age < self.age {
                    self.pos = pos;
                    self.age = age;
                }
            }
        }
    }

    /// Merge with one member's own memory: returns (last known pos, seconds since seen).
    pub fn merge(&self, own_pos: V2, own_age: f32) -> (V2, f32) {
        if self.age < own_age { (self.pos, self.age) } else { (own_pos, own_age) }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Observation {
    pub dt: f32,
    pub pos: V2,
    pub hp: f32,
    pub ammo_frac: f32,
    pub crouched: bool,
    pub player_visible: bool,
    /// Last known position if not visible.
    pub player_pos: V2,
    pub player_vel: V2,
    pub player_hp: f32,
    /// Player look direction (radians on the ground plane).
    pub player_facing: f32,
    pub time_since_seen: f32,
    pub heard_shot: Option<V2>,
    pub allies: Vec<AllyInfo>,
    pub taking_fire: bool,
    /// The player is reloading (a window to push).
    pub player_reloading: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Action {
    /// World-space move direction, length ≤ 1.
    pub move_dir: V2,
    pub sprint: bool,
    pub crouch: bool,
    /// Where the enemy is looking (unit vector).
    pub look_dir: V2,
    /// Wants to pull the trigger this tick (the weapon applies fire rate and ammo).
    pub fire: bool,
    /// Direction a bullet fired now would take, aim error included.
    pub shot_dir: V2,
    /// Vertical aim error for the shot (radians, + is high).
    pub shot_pitch_err: f32,
    pub reload: bool,
}

pub struct MindCtx<'a> {
    pub arena: &'a Arena,
    pub model: &'a PlayerModel,
    pub limits: Limits,
}

/// What the mind is doing, for the debug overlay and the control port.
#[derive(Clone, Debug, Default)]
pub struct MindDebug {
    pub goal: Option<Goal>,
    pub dest: Option<V2>,
    pub path: Vec<V2>,
    pub predicted: Option<V2>,
    pub micro: [f32; NEAT_OUTPUTS],
    pub seen_time: f32,
    pub aim_off: f32,
}

pub struct EnemyMind {
    pub traits: Traits,
    pub trait_id: usize,
    pub neat_id: usize,
    net: neat::Network,
    rng: BrainRng,
    goal: Goal,
    think_timer: f32,
    dest: Option<V2>,
    path: Vec<V2>,
    seen_time: f32,
    aim_yaw: f32,
    on_target_time: f32,
    burst_timer: f32,
    burst_pause: f32,
    flank_side: f32,
    /// Flinch: aim is disturbed briefly after being hit (set on the rising edge of `taking_fire`).
    was_hit: bool,
    flinch_t: f32,
    inputs: [f32; NEAT_INPUTS],
    /// Current squad order (from `squad::SquadCoordinator`), treated as a high-priority goal.
    order: Option<Order>,
    /// A callout the mind wants to make (taken by the coordinator).
    bark: Option<&'static str>,
    was_reloading: bool,
    pub debug: MindDebug,
}

impl EnemyMind {
    pub fn new(tg: &TraitGenome, trait_id: usize, ng: &neat::Genome, neat_id: usize, limits: &Limits, seed: u64) -> Self {
        let mut rng = rng_from_seed(seed);
        let flank_side = if rand::Rng::random::<bool>(&mut rng) { 1.0 } else { -1.0 };
        EnemyMind {
            traits: tg.decode(limits),
            trait_id,
            neat_id,
            net: ng.build(),
            rng,
            goal: Goal::Hunt,
            think_timer: 0.0,
            dest: None,
            path: Vec::new(),
            seen_time: 0.0,
            aim_yaw: std::f32::consts::FRAC_PI_2,
            on_target_time: 0.0,
            burst_timer: 0.0,
            burst_pause: 0.0,
            flank_side,
            was_hit: false,
            flinch_t: 0.0,
            inputs: [0.0; NEAT_INPUTS],
            order: None,
            bark: None,
            was_reloading: false,
            debug: MindDebug::default(),
        }
    }

    pub fn goal(&self) -> Goal {
        self.goal
    }

    pub fn order(&self) -> Option<Order> {
        self.order
    }

    /// Set (or clear) the squad's order. Call before `think`.
    pub fn set_order(&mut self, order: Option<Order>) {
        self.order = order;
    }

    /// A pending callout ("Reloading", "Falling back", ...), if any.
    pub fn take_bark(&mut self) -> Option<&'static str> {
        self.bark.take()
    }

    /// The order applies to the current goal.
    fn following(&self) -> Option<Order> {
        self.order.filter(|o| o.goal == self.goal)
    }

    pub fn think(&mut self, o: &Observation, cx: &MindCtx) -> Action {
        let t = self.traits;
        let to_player = o.player_pos - o.pos;
        let dist = to_player.len();
        let dir_p = to_player.norm();

        // Perception timers.
        if o.player_visible {
            self.seen_time += o.dt;
        } else {
            // A brief occlusion doesn't make them start their reaction over.
            self.seen_time = (self.seen_time - 2.0 * o.dt).max(0.0);
        }
        let engaged = o.player_visible || o.time_since_seen < ENGAGE_MEMORY_S;
        let aiming_at_me = V2::from_angle(o.player_facing).dot(-dir_p);

        // 1. Tactics (~4 Hz, or immediately when hit).
        self.think_timer -= o.dt;
        if self.think_timer <= 0.0 || o.taking_fire && self.goal != Goal::TakeCover && self.goal != Goal::Retreat {
            self.think_timer = 0.25;
            let ui = UtilityInput {
                visible: o.player_visible,
                dist,
                hp: o.hp,
                player_hp: o.player_hp,
                taking_fire: o.taking_fire,
                time_since_seen: o.time_since_seen,
                allies_engaged: o.allies.iter().filter(|a| a.engaged).count(),
                flankers: o.allies.iter().filter(|a| a.flanking).count(),
                model_confidence: cx.model.confidence(),
                player_aiming_at_me: aiming_at_me,
                player_reloading: o.player_reloading,
            };
            let new_goal = utility::choose(self.goal, &ui, &t, self.order.map(|o| o.goal));
            if new_goal != self.goal {
                match new_goal {
                    Goal::Retreat => self.bark = Some("Falling back"),
                    Goal::TakeCover if o.taking_fire => self.bark = Some("Taking fire"),
                    _ => {}
                }
            }
            let order = self.order.filter(|od| od.goal == new_goal);
            let new_dest = match order {
                Some(Order { dest: Some(d), .. }) => Some(cx.arena.nav.nearest_walkable(d)),
                Some(Order { goal: Goal::Hold, dest: None, .. }) => None,
                _ => self.destination(new_goal, o, cx),
            };
            let moved = match (new_dest, self.dest) {
                (Some(a), Some(b)) => a.dist(b) > 2.0,
                (a, b) => a.is_some() != b.is_some(),
            };
            if new_goal != self.goal || moved || self.path.is_empty() {
                self.path = match (new_dest, order.and_then(|od| od.threat)) {
                    (Some(d), Some(threat)) => {
                        let cost = squad::tactical_cost(cx.arena, cx.model, threat, &t.pef);
                        cx.arena.nav.find_path_cost(o.pos, d, Some(&cost)).unwrap_or_default()
                    }
                    (Some(d), None) => cx.arena.nav.find_path(o.pos, d).unwrap_or_default(),
                    (None, _) => Vec::new(),
                };
            }
            self.goal = new_goal;
            self.dest = new_dest;
        }

        // 2. Path following.
        while let Some(&wp) = self.path.first() {
            if wp.dist(o.pos) < 0.6 {
                self.path.remove(0);
            } else {
                break;
            }
        }
        let path_dir = self.path.first().map(|&wp| (wp - o.pos).norm()).unwrap_or(V2::ZERO);

        // 3. NEAT micro in the engagement frame (forward = toward player).
        let mut micro = [0.0; NEAT_OUTPUTS];
        if engaged {
            let (cover_d, cover_side) = self.nearest_cover(o.pos, dir_p, cx);
            let lateral = dir_p.perp();
            self.inputs = [
                (dist / 30.0).min(1.5),
                ((dist - t.preferred_range) / 20.0).clamp(-1.5, 1.5),
                o.player_visible as u8 as f32,
                (o.time_since_seen / 3.0).min(1.0),
                o.hp,
                o.ammo_frac,
                o.player_hp,
                o.player_vel.dot(lateral) / 6.0,
                o.player_vel.dot(dir_p) / 6.0,
                aiming_at_me,
                o.taking_fire as u8 as f32,
                (cover_d / 10.0).min(1.5),
                cover_side,
                cx.model.accuracy_at(dist),
            ];
            let out = self.net.activate(&self.inputs);
            micro.copy_from_slice(&out[..NEAT_OUTPUTS]);
        }
        self.debug.micro = micro;
        let w_micro = match self.goal {
            Goal::Attack => 1.0,
            Goal::Push | Goal::Flank => 0.4,
            Goal::Hold if self.path.is_empty() => 0.3,
            _ => 0.0,
        };
        let micro_move = if engaged { dir_p * micro[0] + dir_p.perp() * micro[1] } else { V2::ZERO };
        let mut move_dir = path_dir + micro_move * w_micro;

        // Spacing from allies.
        let spacing = 2.5 + 2.0 * (1.0 - t.team_cohesion);
        for a in &o.allies {
            let d = o.pos - a.pos;
            let l = d.len();
            if l > 0.01 && l < spacing {
                move_dir += d.norm() * ((spacing - l) / spacing);
            }
        }
        let move_dir = move_dir.clamp_len(1.0);

        // 4. Aim: turn toward the player (with a small lead) at a capped rate.
        let target_dir = if o.player_visible || o.time_since_seen < ENGAGE_MEMORY_S {
            (o.player_pos + o.player_vel * 0.08 - o.pos).norm()
        } else if let Some(w) = self.following().and_then(|od| od.watch) {
            // Hold the angle the squad expects him from.
            (w - o.pos).norm()
        } else if move_dir.len() > 0.1 {
            move_dir.norm()
        } else {
            V2::from_angle(self.aim_yaw)
        };
        let want = target_dir.angle();
        let diff = angle_diff(want, self.aim_yaw);
        let max_step = MAX_TURN_RATE * o.dt;
        self.aim_yaw += diff.clamp(-max_step, max_step);
        let aim_off = angle_diff(want, self.aim_yaw).abs();
        if o.player_visible && aim_off < 0.05 {
            self.on_target_time += o.dt;
        } else {
            self.on_target_time = 0.0;
        }

        // 5. Trigger: reaction delay, aim settled, NEAT fire gate, bursts.
        // The net can veto a shot only when it's strongly against it (e.g. to stay hidden).
        let fire_gate = if engaged { micro[2] > -0.6 } else { true };
        let ready = o.player_visible && self.seen_time >= t.reaction_time && aim_off < 0.07 && fire_gate && o.ammo_frac > 0.0;
        self.debug.seen_time = self.seen_time;
        self.debug.aim_off = aim_off;
        let mut fire = false;
        if self.burst_pause > 0.0 {
            self.burst_pause -= o.dt;
        } else if ready {
            fire = true;
            self.burst_timer += o.dt;
            if self.burst_timer >= t.burst_length as f32 * 0.1 {
                self.burst_timer = 0.0;
                self.burst_pause = 0.3 + 0.2 * self.rng_f();
            }
        } else {
            self.burst_timer = 0.0;
        }

        // Spread: base error, worse while moving, better once settled on target.
        let settle = 1.0 + 0.6 * (1.0 - (self.on_target_time / 0.6).min(1.0));
        let moving = 1.0 + move_dir.len() * 0.35;
        if o.taking_fire && !self.was_hit {
            self.flinch_t = FLINCH_S;
        }
        self.was_hit = o.taking_fire;
        self.flinch_t = (self.flinch_t - o.dt).max(0.0);
        let flinch = 1.0 + FLINCH_SPREAD * self.flinch_t / FLINCH_S;
        let sigma = t.aim_error_deg.to_radians() * settle * moving * flinch;
        let shot_dir = V2::from_angle(self.aim_yaw + gauss(&mut self.rng) * sigma);
        let shot_pitch_err = gauss(&mut self.rng) * sigma * 0.6;

        let crouch = match self.goal {
            Goal::TakeCover | Goal::Retreat => self.path.is_empty(),
            Goal::Attack => micro[3] > 0.3 && !fire,
            _ => false,
        };
        let far = self.dest.is_some_and(|d| d.dist(o.pos) > 2.0);
        let sprint = (matches!(self.goal, Goal::Flank | Goal::Retreat | Goal::Push) || self.goal == Goal::Hold && far) && !o.player_visible;
        let reload = o.ammo_frac <= 0.0 || (!engaged && o.ammo_frac < 0.4);
        if reload && !self.was_reloading {
            self.bark = Some("Reloading");
        }
        self.was_reloading = reload;

        self.debug.goal = Some(self.goal);
        self.debug.dest = self.dest;
        self.debug.path.clone_from(&self.path);

        Action {
            move_dir,
            sprint,
            crouch,
            look_dir: V2::from_angle(self.aim_yaw),
            fire,
            shot_dir,
            shot_pitch_err,
            reload,
        }
    }

    fn rng_f(&mut self) -> f32 {
        rand::Rng::random(&mut self.rng)
    }

    fn nearest_cover(&self, pos: V2, dir_p: V2, cx: &MindCtx) -> (f32, f32) {
        let mut best = (f32::MAX, 0.0);
        for c in &cx.arena.cover {
            let d = c.pos.dist(pos);
            if d < best.0 {
                best = (d, (c.pos - pos).dot(dir_p.perp()).signum());
            }
        }
        best
    }

    fn destination(&mut self, goal: Goal, o: &Observation, cx: &MindCtx) -> Option<V2> {
        let t = &self.traits;
        let a = cx.arena;
        let player = o.player_pos;
        self.debug.predicted = None;
        let dest = match goal {
            Goal::Attack => None,
            Goal::Push => {
                // Fight at the player's weakest range if we trust the model, else our own.
                let trust = t.prediction_trust * cx.model.confidence();
                let range = t.preferred_range + (cx.model.weakest_range() - t.preferred_range) * trust;
                Some(player + (o.pos - player).norm() * range.min(o.pos.dist(player) - 2.0).max(4.0))
            }
            Goal::Flank => {
                // Crossfire: pick the angle around the player farthest (in bearing) from where
                // teammates already are, preferring the player's back, not too far to walk.
                let r = t.preferred_range.max(10.0) * 0.8;
                let ally_bearings: Vec<f32> = o.allies.iter().map(|al| (al.pos - player).angle()).collect();
                let facing = V2::from_angle(o.player_facing);
                let mut best: Option<(f32, V2)> = None;
                for k in 0..16 {
                    let ang = k as f32 * std::f32::consts::TAU / 16.0;
                    let dir = V2::from_angle(ang);
                    let p = player + dir * r;
                    if !a.nav.walkable_at(p) {
                        continue;
                    }
                    let sep = ally_bearings
                        .iter()
                        .map(|b| angle_diff(ang, *b).abs())
                        .fold(std::f32::consts::PI, f32::min)
                        / std::f32::consts::PI;
                    let behind = -dir.dot(facing); // +1 directly behind the player
                    let travel = p.dist(o.pos) / 40.0;
                    let side = 0.05 * self.flank_side * dir.cross(facing).signum();
                    let score = sep + 0.4 * behind - 0.6 * travel + side;
                    if best.is_none_or(|(s, _)| score > s) {
                        best = Some((score, p));
                    }
                }
                best.map(|b| b.1)
            }
            Goal::TakeCover => self.best_cover(o.pos, player, 15.0, false, cx),
            Goal::Retreat => self.best_cover(o.pos, player, 25.0, true, cx),
            Goal::Ambush => {
                let predicted = cx.model.predict(player, a);
                self.debug.predicted = Some(predicted);
                let mut best: Option<(f32, V2)> = None;
                for k in 0..16 {
                    let p = predicted + V2::from_angle(k as f32 * std::f32::consts::TAU / 16.0) * t.preferred_range;
                    if !a.nav.walkable_at(p) || !a.line_of_sight(p, false, predicted, false) {
                        continue;
                    }
                    let score = p.dist(o.pos);
                    if best.is_none_or(|(s, _)| score < s) {
                        best = Some((score, p));
                    }
                }
                best.map(|b| b.1).or(Some(predicted))
            }
            Goal::Hunt => Some(o.heard_shot.unwrap_or(player)),
            Goal::Hold => None,
        };
        dest.map(|d| a.nav.nearest_walkable(d))
    }

    /// A cover point hidden from the player (crouched). `far`: prefer far from the player.
    fn best_cover(&self, me: V2, player: V2, radius: f32, far: bool, cx: &MindCtx) -> Option<V2> {
        let mut best: Option<(f32, V2)> = None;
        for c in &cx.arena.cover {
            let d_me = c.pos.dist(me);
            let d_pl = c.pos.dist(player);
            if d_me > radius || d_pl < 5.0 || cx.arena.line_of_sight(player, false, c.pos, true) {
                continue;
            }
            let score = if far { d_me * 0.5 - d_pl } else { d_me - 0.2 * d_pl.min(20.0) };
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, c.pos));
            }
        }
        best.map(|b| b.1)
    }
}
