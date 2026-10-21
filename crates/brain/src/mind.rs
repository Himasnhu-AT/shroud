//! `EnemyMind`: joins traits, utility tactics, NEAT micro and human-like aiming.
//! The game and the 2D trainer both call `think(&Observation, &MindCtx) -> Action`.

use crate::arena::Arena;
use crate::genome::{TraitGenome, Traits};
use crate::limits::Limits;
use crate::math::{BrainRng, V2, angle_diff, gauss, rng_from_seed};
use crate::neat;
use crate::perception::{Alert, COMBAT_CALM_S, Heard, SUSPICIOUS_CALM_S, Sound};
use crate::player_model::PlayerModel;
use crate::posture;
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

/// Squad blackboard: the freshest player sighting any squad member has radioed in, plus the
/// squad's alert level. Out of combat, stale chatter is shared every `RADIO_PERIOD_S` (fair,
/// human-like); a fresh sighting, being shot, or anything in combat relays at once.
#[derive(Clone, Copy, Debug)]
pub struct Blackboard {
    pub pos: V2,
    /// Seconds since that sighting.
    pub age: f32,
    pub alert: Alert,
    /// Seconds since the last stimulus of any kind (drives calming down).
    pub calm: f32,
    timer: f32,
}

impl Default for Blackboard {
    fn default() -> Self {
        Blackboard { pos: V2::ZERO, age: 999.0, alert: Alert::Unaware, calm: 999.0, timer: 0.0 }
    }
}

/// A sighting this fresh (s) is "seeing him now": the squad goes to combat at once.
pub const FRESH_SIGHTING_S: f32 = 0.15;
/// Age given to a heard gunshot / footstep on the board: less certain than a sighting.
pub const HEARD_SHOT_AGE: f32 = 0.8;
pub const HEARD_STEP_AGE: f32 = 2.0;

impl Blackboard {
    /// Advance time and take the freshest of the members' (pos, seconds since seen).
    pub fn tick(&mut self, dt: f32, sightings: impl Iterator<Item = (V2, f32)>) {
        self.age += dt;
        self.calm += dt;
        self.timer -= dt;
        let radio = self.timer <= 0.0;
        if radio {
            self.timer = RADIO_PERIOD_S;
        }
        for (pos, age) in sightings {
            let fresh = age < FRESH_SIGHTING_S;
            if fresh {
                self.alert = Alert::Combat;
                self.calm = 0.0;
            }
            if (fresh || radio || self.alert == Alert::Combat) && age < self.age {
                self.pos = pos;
                self.age = age;
            }
        }
        if self.alert == Alert::Combat && self.calm > COMBAT_CALM_S && self.age > COMBAT_CALM_S {
            self.alert = Alert::Suspicious;
            self.calm = 0.0;
        } else if self.alert == Alert::Suspicious && self.calm > SUSPICIOUS_CALM_S {
            self.alert = Alert::Unaware;
        }
    }

    /// Damage is a sense: an enemy was hit, so everyone knows where the shooter stood. No delay.
    pub fn reveal(&mut self, shooter: V2) {
        self.pos = shooter;
        self.age = 0.0;
        self.alert = Alert::Combat;
        self.calm = 0.0;
    }

    /// A member heard something. Gunshots start a fight; steps and reloads make the squad
    /// suspicious. The heard spot goes on the board as an older (less certain) sighting.
    pub fn hear(&mut self, h: &Heard) {
        self.calm = 0.0;
        self.alert = self.alert.max(h.sound.alert());
        let age = if h.sound == Sound::Gunshot { HEARD_SHOT_AGE } else { HEARD_STEP_AGE };
        if age < self.age {
            self.pos = h.pos;
            self.age = age;
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
    /// The player just took damage (another window to push).
    pub player_hurt: bool,
    /// The squad's alert level (`Blackboard::alert`).
    pub squad_alert: Alert,
    /// The squad gave this member no attack token: reposition to a flank rather than trade
    /// shots (it still shoots in self-defence or up close).
    pub token_denied: bool,
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
    /// Pre-firing at this spot (the player is not visible): the shot's pitch should aim at
    /// chest height there, not at the player.
    pub prefire: Option<V2>,
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
    /// Seconds since this enemy itself last saw the player.
    own_unseen: f32,
    /// This enemy's awareness (its own senses, raised by the squad's).
    alert: Alert,
    /// Seconds in the current goal (commit time against oscillation).
    goal_t: f32,
    /// Last sound heard and seconds since (turn to face it, investigate).
    heard_prev: Option<V2>,
    heard_t: f32,
    /// Stuck detection: position at the start of the window, window timer, strikes.
    stuck_ref: V2,
    stuck_t: f32,
    stuck_n: u32,
    sidestep_t: f32,
    sidestep_dir: V2,
    force_repath: bool,
    /// Strafe while shooting (A-D rhythm).
    strafe_t: f32,
    strafe_sign: f32,
    /// Trigger time spent on the current pre-fire burst.
    prefire_t: f32,
    /// Crouch-peek rhythm behind low cover.
    peek_t: f32,
    pub debug: MindDebug,
}

/// Pre-fire spread is this much wider than aimed fire (shooting at a spot, not a target).
pub const PREFIRE_SPREAD: f32 = 1.6;
/// Crouch-peek cycle behind low cover: down for `PEEK_DOWN_S`, up for the rest of `PEEK_CYCLE_S`.
pub const PEEK_CYCLE_S: f32 = 2.4;
pub const PEEK_DOWN_S: f32 = 1.3;

/// Minimum time in a goal before switching (unless hit): no flip-flopping between goals.
pub const GOAL_COMMIT_S: f32 = 0.6;
/// Stuck: less than `STUCK_MIN_MOVE` m of progress in `STUCK_S` s while trying to move.
pub const STUCK_S: f32 = 0.7;
pub const STUCK_MIN_MOVE: f32 = 0.35;
/// How long a heard sound keeps an enemy looking at / investigating it.
pub const HEARD_MEMORY_S: f32 = 6.0;

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
            own_unseen: 99.0,
            alert: Alert::Unaware,
            goal_t: 0.0,
            heard_prev: None,
            heard_t: 99.0,
            stuck_ref: V2::ZERO,
            stuck_t: 0.0,
            stuck_n: 0,
            sidestep_t: 0.0,
            sidestep_dir: V2::ZERO,
            force_repath: false,
            strafe_t: 0.0,
            strafe_sign: flank_side,
            prefire_t: 0.0,
            peek_t: 0.0,
            debug: MindDebug::default(),
        }
    }

    pub fn goal(&self) -> Goal {
        self.goal
    }

    pub fn alert(&self) -> Alert {
        self.alert
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
        self.own_unseen = if o.player_visible { 0.0 } else { self.own_unseen + o.dt };
        if o.player_visible {
            self.seen_time += o.dt;
        } else {
            // A brief occlusion doesn't make them start their reaction over.
            self.seen_time = (self.seen_time - 2.0 * o.dt).max(0.0);
        }
        let engaged = o.player_visible || o.time_since_seen < ENGAGE_MEMORY_S;
        let aiming_at_me = V2::from_angle(o.player_facing).dot(-dir_p);

        // Alert: own senses (sight, pain, sound), raised to the squad's level.
        if o.heard_shot.is_some() && o.heard_shot != self.heard_prev {
            self.heard_prev = o.heard_shot;
            self.heard_t = 0.0;
        } else {
            self.heard_t += o.dt;
        }
        let heard_fresh = self.heard_t < HEARD_MEMORY_S;
        let own = if o.player_visible || o.taking_fire {
            Alert::Combat
        } else if heard_fresh {
            Alert::Suspicious
        } else {
            Alert::Unaware
        };
        self.alert = own.max(o.squad_alert);
        let combat = self.alert == Alert::Combat;
        self.goal_t += o.dt;

        // 1. Tactics (~4 Hz, or immediately when hit or stuck).
        self.think_timer -= o.dt;
        let urgent = o.taking_fire && self.goal != Goal::TakeCover && self.goal != Goal::Retreat;
        if self.think_timer <= 0.0 || urgent || self.force_repath {
            self.think_timer = 0.25;
            let repath = std::mem::take(&mut self.force_repath);
            let ui = UtilityInput {
                combat,
                token: !o.token_denied,
                player_hurt: o.player_hurt,
                in_contact: self.own_unseen < ENGAGE_MEMORY_S,
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
            let mut new_goal = utility::choose(self.goal, &ui, &t, self.order.map(|o| o.goal), self.order.is_some_and(|o| o.sticky));
            // Commit: hold a fresh goal briefly (being hit or ordered overrides).
            let ordered = self.order.is_some_and(|od| od.goal == new_goal);
            if self.goal_t < GOAL_COMMIT_S && !urgent && !ordered && new_goal != Goal::Retreat {
                new_goal = self.goal;
            }
            if new_goal != self.goal {
                self.goal_t = 0.0;
                match new_goal {
                    Goal::Retreat => self.bark = Some("Falling back"),
                    Goal::TakeCover if o.taking_fire => self.bark = Some("Taking fire"),
                    Goal::Push if o.player_reloading => self.bark = Some("He's reloading - pushing"),
                    Goal::Push => self.bark = Some("Pushing"),
                    Goal::Flank => self.bark = Some("Flanking"),
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
            if new_goal != self.goal || moved || self.path.is_empty() || repath {
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

        // Stuck detection: trying to move but barely progressing over a window → sidestep and
        // repath; after repeated strikes, detour via a nearby open spot.
        self.stuck_t += o.dt;
        if self.stuck_t >= STUCK_S {
            let trying = !self.path.is_empty();
            if trying && o.pos.dist(self.stuck_ref) < STUCK_MIN_MOVE {
                self.stuck_n += 1;
                let side = if self.stuck_n.is_multiple_of(2) { 1.0 } else { -1.0 };
                self.sidestep_dir = (path_dir.perp() * side - path_dir * 0.3).norm();
                self.sidestep_t = 0.4;
                self.force_repath = true;
                if self.stuck_n >= 3 {
                    // Pick a new waypoint: the most open nearby walkable spot toward the goal.
                    let goal_dir = self.dest.map_or(path_dir, |d| (d - o.pos).norm());
                    let mut best: Option<(f32, V2)> = None;
                    for k in 0..12 {
                        let dir = V2::from_angle(k as f32 * std::f32::consts::TAU / 12.0);
                        let p = o.pos + dir * 3.0;
                        if cx.arena.nav.walkable_at(p) && cx.arena.nav.clear_line(o.pos, p) {
                            let s = dir.dot(goal_dir) + self.rng_f() * 0.5;
                            if best.is_none_or(|(b, _)| s > b) {
                                best = Some((s, p));
                            }
                        }
                    }
                    if let Some((_, p)) = best {
                        self.path = vec![p];
                        self.force_repath = false;
                        self.think_timer = 0.6;
                    }
                    self.stuck_n = 0;
                }
            } else if o.pos.dist(self.stuck_ref) >= STUCK_MIN_MOVE {
                self.stuck_n = 0;
            }
            self.stuck_t = 0.0;
            self.stuck_ref = o.pos;
        }
        let path_dir = if self.sidestep_t > 0.0 {
            self.sidestep_t -= o.dt;
            self.sidestep_dir
        } else {
            self.path.first().map(|&wp| (wp - o.pos).norm()).unwrap_or(V2::ZERO)
        };

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

        // Strafe while shooting: never stand still in the open trading shots.
        if o.player_visible && matches!(self.goal, Goal::Attack | Goal::Push | Goal::Hold) && self.sidestep_t <= 0.0 {
            self.strafe_t -= o.dt;
            if self.strafe_t <= 0.0 {
                self.strafe_sign = -self.strafe_sign;
                self.strafe_t = 0.45 + 0.6 * self.rng_f();
            }
            let w = if self.path.is_empty() { 0.6 } else { 0.3 };
            move_dir += dir_p.perp() * self.strafe_sign * w;
        }

        // Context steering: keep the desired heading unless allies or walls make it dangerous.
        let spacing = 2.5 + 2.0 * (1.0 - t.team_cohesion);
        let move_dir = context_steer(move_dir.clamp_len(1.0), o.pos, &o.allies, spacing, cx.arena);
        // Standoff: never walk into the player (a body filling his camera is neither fair nor
        // readable); back off a little inside the minimum distance.
        let move_dir = if o.time_since_seen < 3.0 { standoff(move_dir, o.pos, o.player_pos) } else { move_dir };

        // 4. Aim: turn toward the player (with a small lead) at a capped rate; out of contact,
        // turn to face a fresh sound.
        // Anchor pre-fire: the squad says he is about to re-peek this exact spot.
        let prefire = self.following().and_then(|od| od.prefire).filter(|_| !o.player_visible);
        if prefire.is_none() {
            self.prefire_t = 0.0;
        }
        let target_dir = if let Some(pf) = prefire {
            (pf - o.pos).norm()
        } else if o.player_visible || o.time_since_seen < ENGAGE_MEMORY_S {
            (o.player_pos + o.player_vel * 0.08 - o.pos).norm()
        } else if let Some(h) = self.heard_prev.filter(|_| self.heard_t < 2.5 && !self.path.is_empty() || self.heard_t < 1.0) {
            (h - o.pos).norm()
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

        // 5. Trigger: reaction delay (fairness floor), aim settled, bursts. Out of combat the
        // net may veto a shot to stay hidden; in combat they shoot as soon as they can.
        // Attack tokens: members without one hold fire unless defending themselves.
        let fire_gate = if engaged && !combat { micro[2] > -0.6 } else { true };
        let may_shoot = !o.token_denied || o.taking_fire || dist < 12.0 || aiming_at_me > 0.97;
        let ready = o.player_visible && self.seen_time >= t.reaction_time && aim_off < 0.07 && fire_gate && may_shoot && o.ammo_frac > 0.0;
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
                self.burst_pause = if combat { 0.18 + 0.15 * self.rng_f() } else { 0.3 + 0.2 * self.rng_f() };
            }
        } else {
            self.burst_timer = 0.0;
        }
        // One short burst per pre-fire window, only once the aim has arrived at the spot.
        let prefiring = prefire.is_some() && !fire && aim_off < 0.06 && o.ammo_frac > 0.0 && self.prefire_t < squad::PREFIRE_BURST_S;
        if prefiring {
            fire = true;
            self.prefire_t += o.dt;
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
        let sigma = t.aim_error_deg.to_radians() * settle * moving * flinch * if prefiring { PREFIRE_SPREAD } else { 1.0 };
        let shot_dir = V2::from_angle(self.aim_yaw + gauss(&mut self.rng) * sigma);
        let shot_pitch_err = gauss(&mut self.rng) * sigma * 0.6;

        // Behind low cover while holding or fighting: duck, pop up to look and shoot, duck to reload.
        let ducking = matches!(self.goal, Goal::Hold | Goal::Attack) && self.path.is_empty() && posture::low_cover_toward(cx.arena, o.pos, o.player_pos);
        if ducking {
            self.peek_t = (self.peek_t + o.dt) % PEEK_CYCLE_S;
        } else {
            self.peek_t = 0.0;
        }
        let crouch = if ducking {
            !fire && (o.ammo_frac <= 0.0 || (!o.player_visible && self.peek_t < PEEK_DOWN_S))
        } else {
            match self.goal {
                Goal::TakeCover | Goal::Retreat => self.path.is_empty(),
                Goal::Attack => micro[3] > 0.3 && !fire,
                _ => false,
            }
        };
        // Sticky orders (the trader keeping up with the entry) sprint to close any gap.
        let gap = if self.following().is_some_and(|od| od.sticky) { 1.0 } else { 2.0 };
        let far = self.dest.is_some_and(|d| d.dist(o.pos) > gap);
        let sticky = self.following().is_some_and(|od| od.sticky);
        // Cover-to-cover: a push dashes between covers even in view; a combat hunt runs.
        let dash = self.goal == Goal::Push && far && !fire && self.following().is_none();
        let sprint = dash
            || (matches!(self.goal, Goal::Flank | Goal::Retreat | Goal::Push) || self.goal == Goal::Hold && far || combat && self.goal == Goal::Hunt)
                && (!o.player_visible || sticky && far);
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
            prefire: prefire.filter(|_| fire),
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
                // Cover-to-cover: bound to the next cover that gains ground, if there is one.
                let ranged = player + (o.pos - player).norm() * range.min(o.pos.dist(player) - 2.0).max(4.0);
                // (An ordered entry walks the direct line so its trader can follow.)
                let ordered = self.order.is_some_and(|od| od.goal == Goal::Push);
                Some(if ordered { ranged } else { advance_cover(o.pos, player, range, cx).unwrap_or(ranged) })
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
            // Investigate a fresh sound unless the squad knows something newer.
            Goal::Hunt => Some(match self.heard_prev {
                Some(h) if self.heard_t < HEARD_MEMORY_S && self.heard_t < o.time_since_seen => h,
                _ => player,
            }),
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

/// The next cover point on the way to `player`: hidden from him (crouched), within a short
/// dash, and at least 3 m closer to him, but no closer than ~`range`/2.
fn advance_cover(me: V2, player: V2, range: f32, cx: &MindCtx) -> Option<V2> {
    let d_now = me.dist(player);
    let mut best: Option<(f32, V2)> = None;
    for c in &cx.arena.cover {
        let (d_me, d_pl) = (c.pos.dist(me), c.pos.dist(player));
        if !(1.5..=12.0).contains(&d_me) || d_pl > d_now - 3.0 || d_pl < (range * 0.5).max(5.0) {
            continue;
        }
        if cx.arena.line_of_sight(player, false, c.pos, true) {
            continue;
        }
        let score = d_pl + 0.5 * d_me;
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, c.pos));
        }
    }
    best.map(|b| b.1)
}

/// Context steering (Fray, Game AI Pro 2): 16 headings scored by interest (the desired
/// direction) minus danger (allies too close in that direction, walls just ahead). Keeps the
/// desired vector untouched when its own heading is safe, so path following stays exact.
pub fn context_steer(desired: V2, pos: V2, allies: &[AllyInfo], spacing: f32, arena: &Arena) -> V2 {
    let danger_of = |d: V2| {
        let mut danger: f32 = 0.0;
        for a in allies {
            let to = a.pos - pos;
            let l = to.len();
            if l > 0.01 && l < spacing {
                danger = danger.max((1.0 - l / spacing) * d.dot(to * (1.0 / l)).max(0.0));
            }
        }
        if !arena.nav.walkable_at(pos + d * 0.7) && arena.nav.walkable_at(pos) {
            danger = 1.0;
        }
        danger
    };
    let mag = desired.len();
    if mag < 0.05 {
        // Standing: just keep some spacing.
        let mut push = V2::ZERO;
        for a in allies {
            let d = pos - a.pos;
            let l = d.len();
            if l > 0.01 && l < spacing {
                push += d.norm() * ((spacing - l) / spacing);
            }
        }
        return push.clamp_len(1.0);
    }
    let want = desired * (1.0 / mag);
    if danger_of(want) < 0.05 {
        return desired;
    }
    let mut best = (f32::MIN, want);
    for k in 0..16 {
        let d = V2::from_angle(k as f32 * std::f32::consts::TAU / 16.0);
        let s = d.dot(want) - 2.0 * danger_of(d);
        if s > best.0 {
            best = (s, d);
        }
    }
    best.1 * mag
}

/// Enemies keep at least this far (m) from the player.
pub const STANDOFF_M: f32 = 2.0;

/// Remove any approach toward `player` within `STANDOFF_M + 0.75` m, and back off inside
/// `STANDOFF_M`. Sideways motion (strafing, flanking around him) is kept.
pub fn standoff(move_dir: V2, pos: V2, player: V2) -> V2 {
    let to = player - pos;
    let d = to.len();
    if !(1e-3..=STANDOFF_M + 0.75).contains(&d) {
        return move_dir;
    }
    let toward = to * (1.0 / d);
    let mut m = move_dir;
    let inward = m.dot(toward);
    if inward > 0.0 {
        m = m - toward * inward;
    }
    if d < STANDOFF_M {
        m = m - toward * (0.6 * (1.0 - d / STANDOFF_M) + 0.2);
    }
    m.clamp_len(1.0)
}
