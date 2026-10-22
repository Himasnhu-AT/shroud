//! `EnemyMind`: joins traits, utility tactics, NEAT micro and human-like aiming.
//! The game and the 2D trainer both call `think(&Observation, &MindCtx) -> Action`.

use crate::arena::Arena;
use crate::genome::{TraitGenome, Traits};
use crate::limits::Limits;
use crate::math::{BrainRng, V2, angle_diff, gauss, rng_from_seed};
use crate::neat;
use crate::perception::{Alert, COMBAT_CALM_S, Heard, SUSPICIOUS_CALM_S, Sound};
use crate::player_model::PlayerModel;
use crate::posts;
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
    /// Lean out of cover: -1 left .. +1 right of `look_dir` (0 = upright; the game may still
    /// lean on its own at tall corners).
    pub lean: f32,
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
    /// Peek-shoot-hide phase, if running the post loop.
    pub post: Option<PostPhase>,
    /// No ally close enough to help.
    pub isolated: bool,
}

/// Peek-shoot-hide loop at cover posts (`posts.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostPhase {
    /// Moving to the post.
    Move,
    /// Crouched behind it (reloading if needed).
    Hide,
    /// Stepped / stood out to fire a burst.
    Peek,
}

impl PostPhase {
    pub fn label(self) -> &'static str {
        match self {
            PostPhase::Move => "to-post",
            PostPhase::Hide => "hide",
            PostPhase::Peek => "peek",
        }
    }
}

impl PostState {
    /// Where to stand now.
    fn target(&self) -> V2 {
        if self.phase == PostPhase::Peek { self.post.peek } else { self.post.pos }
    }
}

#[derive(Clone, Copy, Debug)]
struct PostState {
    post: posts::Post,
    phase: PostPhase,
    /// Seconds left in the phase (Hide / Peek); time spent in Move.
    t: f32,
    /// Trigger time spent this peek, and the burst wanted.
    fired: f32,
    burst: f32,
    /// Hid after a peek: the next step is a new post.
    peeked: bool,
    /// Only here to reload; leave once the magazine is full.
    reload_only: bool,
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
    /// Peek-shoot-hide state, recent peek spots (pos, seconds ago), retry timer.
    post: Option<PostState>,
    recent_peeks: Vec<(V2, f32)>,
    post_retry: f32,
    isolated: bool,
    alone_barked: bool,
    /// This enemy's own radio delay (s) and its belief of where the player is: radio updates
    /// reach it `radio_lag` late, so the squad never turns in unison.
    radio_lag: f32,
    belief: Option<V2>,
    radio_wait: f32,
    /// Crouch decisions in a fight hold for a moment (no robotic crouch-spam).
    crouch_hold: f32,
    crouch_prev: bool,
    /// Burst length jitter for the current burst.
    burst_jitter: f32,
    /// Reaction time needed for the current sighting (latched when it starts).
    react_need: f32,
    pub debug: MindDebug,
}

/// No living ally within this distance (m, ~4 s at a walk): the enemy is on its own.
pub const ISOLATION_M: f32 = 15.0;
/// The post loop runs while this enemy saw the player within this long (s) or is under fire.
pub const POST_CONTACT_S: f32 = 4.0;
/// Peek burst length range (s) and hide time after a peek (s).
pub const PEEK_BURST_S: (f32, f32) = (0.4, 0.9);
pub const HIDE_S: (f32, f32) = (1.2, 3.0);
/// Longest a peek waits for a shot (s) before ducking back.
pub const PEEK_MAX_S: f32 = 2.0;
/// Radio delay range (s), seeded per enemy.
pub const RADIO_LAG_S: (f32, f32) = (0.2, 0.6);
/// Members without an attack token shoot suppressively: spread this much wider.
pub const SUPPRESS_SPREAD: f32 = 1.6;
/// Reaction in a fight is the trait's reaction times this (never below the generation floor);
/// unalerted it is this much slower.
pub const COMBAT_REACTION: f32 = 0.6;
pub const UNALERTED_REACTION: f32 = 1.8;
/// First-shot accuracy: spread starts `1 + SETTLE_SPREAD` times wider and narrows over `SETTLE_S`.
pub const SETTLE_SPREAD: f32 = 0.6;
pub const SETTLE_S: f32 = 0.6;
/// Reload behind cover if there is cover this close (m).
pub const RELOAD_COVER_M: f32 = 4.0;

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
        let radio_lag = RADIO_LAG_S.0 + (RADIO_LAG_S.1 - RADIO_LAG_S.0) * rand::Rng::random::<f32>(&mut rng);
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
            post: None,
            recent_peeks: Vec::new(),
            post_retry: 0.0,
            isolated: false,
            alone_barked: false,
            radio_lag,
            belief: None,
            radio_wait: -1.0,
            crouch_hold: 0.0,
            crouch_prev: false,
            burst_jitter: 1.0,
            react_need: 0.0,
            debug: MindDebug::default(),
        }
    }

    /// Running the peek-shoot-hide loop (and its phase).
    pub fn post_phase(&self) -> Option<PostPhase> {
        self.post.map(|p| p.phase)
    }

    /// No ally close enough to help (as of the last `think`).
    pub fn isolated(&self) -> bool {
        self.isolated
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

    pub fn think(&mut self, raw: &Observation, cx: &MindCtx) -> Action {
        // Radio lag: what the squad says reaches this enemy `radio_lag` s late (its own eyes
        // are instant), so they don't all turn at once.
        let belief = *self.belief.get_or_insert(raw.player_pos);
        if raw.player_visible {
            self.belief = Some(raw.player_pos);
            self.radio_wait = -1.0;
        } else if raw.player_pos.dist(belief) > 1.5 {
            if self.radio_wait < 0.0 {
                self.radio_wait = self.radio_lag;
            }
            self.radio_wait -= raw.dt;
            if self.radio_wait <= 0.0 {
                self.belief = Some(raw.player_pos);
                self.radio_wait = -1.0;
            }
        } else {
            self.radio_wait = -1.0;
        }
        let lagged;
        let o = match self.belief {
            Some(b) if b != raw.player_pos => {
                lagged = Observation { player_pos: b, allies: raw.allies.clone(), ..*raw };
                &lagged
            }
            _ => raw,
        };
        let t = self.traits;
        let to_player = o.player_pos - o.pos;
        let dist = to_player.len();
        let dir_p = to_player.norm();

        // Perception timers. The reaction a sighting needs is set when it starts: quick if the
        // enemy was already fighting, slow if it was caught unaware (never below the floor).
        self.own_unseen = if o.player_visible { 0.0 } else { self.own_unseen + o.dt };
        if o.player_visible {
            if self.seen_time <= 0.0 {
                let base = t.reaction_time.max(cx.limits.min_reaction_s);
                self.react_need = match self.alert {
                    Alert::Combat => (base * COMBAT_REACTION).max(cx.limits.min_reaction_s),
                    Alert::Suspicious => base,
                    Alert::Unaware => base * UNALERTED_REACTION,
                };
            }
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

        // Isolation: nobody close enough to help (the last survivor included). Alone in a
        // fight it works cover posts: peek, fire a burst, hide, re-peek from somewhere else.
        // Members without a token do the same instead of standing in the open (one cycle).
        self.isolated = !o.allies.iter().any(|a| a.pos.dist(o.pos) < ISOLATION_M);
        let sticky_order = self.order.is_some_and(|od| od.sticky);
        let contact = self.own_unseen < POST_CONTACT_S || o.taking_fire;
        let window = o.player_reloading || o.player_hp < 0.35;
        let open_fight = matches!(self.goal, Goal::Attack | Goal::TakeCover) || self.post.is_some();
        let want_post = combat
            && contact
            && !sticky_order
            && !window
            && self.goal != Goal::Retreat
            && dist > posts::MIN_PLAYER_DIST
            && (self.isolated || o.token_denied && open_fight || self.goal == Goal::TakeCover);
        // Reload behind cover when it is close, not in the open.
        let hidden_here = !cx.arena.line_of_sight(o.player_pos, false, o.pos, true);
        let reload_spot = if engaged && o.ammo_frac < 0.2 && !hidden_here && !want_post {
            posts::reload_spot(cx.arena, o.pos, o.player_pos, RELOAD_COVER_M)
        } else {
            None
        };
        let had_post = self.post.is_some();
        self.update_post(o, cx, want_post, reload_spot);
        if !self.isolated {
            self.alone_barked = false;
        } else if !had_post && !self.alone_barked && let Some(st) = self.post.filter(|p| !p.reload_only) {
            self.alone_barked = true;
            let back = st.post.pos.dist(o.player_pos) > o.pos.dist(o.player_pos) + 2.0;
            self.bark = Some(if back { "Falling back" } else { "I'm alone - holding" });
        }
        let post_dest = self.post.map(|p| p.target());

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
            // Working a post: alone it overrides everything but falling back; with the squad
            // around it only replaces standing in the open.
            if let Some(st) = self.post {
                let keep = new_goal != Goal::Retreat && (self.isolated || st.reload_only || matches!(new_goal, Goal::Attack | Goal::TakeCover | Goal::Hold));
                if keep {
                    new_goal = Goal::TakeCover;
                } else {
                    self.post = None;
                }
            }
            let post_dest = self.post.and(post_dest);
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
            let new_dest = if post_dest.is_some() {
                post_dest
            } else {
                match order {
                    Some(Order { dest: Some(d), .. }) => Some(cx.arena.nav.nearest_walkable(d)),
                    Some(Order { goal: Goal::Hold, dest: None, .. }) => None,
                    _ => self.destination(new_goal, o, cx),
                }
            };
            let moved = match (new_dest, self.dest) {
                (Some(a), Some(b)) => a.dist(b) > 2.0,
                (a, b) => a.is_some() != b.is_some(),
            };
            if new_goal != self.goal || moved || self.path.is_empty() || repath {
                // (A post move keeps out of his sight lines too.)
                let post_threat = post_dest.map(|_| o.player_pos);
                self.path = match (new_dest, order.and_then(|od| od.threat).or(post_threat)) {
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
        let posting = self.post.map(|p| p.phase);
        let w_micro = match self.goal {
            _ if posting.is_some() => 0.0,
            Goal::Attack => 1.0,
            Goal::Push | Goal::Flank => 0.4,
            Goal::Hold if self.path.is_empty() => 0.3,
            _ => 0.0,
        };
        let micro_move = if engaged { dir_p * micro[0] + dir_p.perp() * micro[1] } else { V2::ZERO };
        let mut move_dir = path_dir + micro_move * w_micro;

        // Strafe while shooting: never stand still in the open trading shots. Strafes lean
        // toward the nearest cover (a step from safety) rather than a metronome A-D.
        if o.player_visible && posting.is_none() && matches!(self.goal, Goal::Attack | Goal::Push | Goal::Hold) && self.sidestep_t <= 0.0 {
            self.strafe_t -= o.dt;
            if self.strafe_t <= 0.0 {
                let (cover_d, cover_side) = self.nearest_cover(o.pos, dir_p, cx);
                let flip = -self.strafe_sign;
                self.strafe_sign = if cover_d < 6.0 && cover_side != 0.0 && flip != cover_side && self.rng_f() < 0.6 { cover_side } else { flip };
                self.strafe_t = 0.35 + 0.75 * self.rng_f();
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
        } else if o.player_visible || o.time_since_seen < ENGAGE_MEMORY_S || posting.is_some_and(|p| p != PostPhase::Move || self.own_unseen < 1.0) {
            // (At a post the gun stays on where he was: the peek comes out pre-aimed.)
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

        // 5. Trigger: shoot on sight once the reaction (fairness floor) has passed and the aim
        // is on him, in bursts. Out of combat the net may veto a shot to stay hidden. Attack
        // tokens no longer hold fire: members without one shoot too, just suppressively (wider
        // spread, below).
        let fire_gate = if engaged && !combat { micro[2] > -0.6 } else { true };
        let ready = o.player_visible && self.seen_time >= self.react_need && aim_off < 0.07 && fire_gate && o.ammo_frac > 0.0;
        self.debug.seen_time = self.seen_time;
        self.debug.aim_off = aim_off;
        let mut fire = false;
        if self.burst_pause > 0.0 {
            self.burst_pause -= o.dt;
        } else if ready {
            fire = true;
            self.burst_timer += o.dt;
            if self.burst_timer >= t.burst_length as f32 * 0.1 * self.burst_jitter {
                self.burst_timer = 0.0;
                self.burst_jitter = 0.7 + 0.6 * self.rng_f();
                self.burst_pause = if combat { 0.18 + 0.15 * self.rng_f() } else { 0.3 + 0.2 * self.rng_f() };
            }
        } else {
            self.burst_timer = 0.0;
        }
        if fire && let Some(st) = self.post.as_mut().filter(|p| p.phase == PostPhase::Peek) {
            st.fired += o.dt;
        }
        // One short burst per pre-fire window, only once the aim has arrived at the spot.
        let prefiring = prefire.is_some() && !fire && aim_off < 0.06 && o.ammo_frac > 0.0 && self.prefire_t < squad::PREFIRE_BURST_S;
        if prefiring {
            fire = true;
            self.prefire_t += o.dt;
        }

        // Spread: base error, worse while moving, wide on the first shots and narrowing as the
        // aim settles on him; suppressive (no token, not self-defence) fire is wider still.
        // (Pre-fire aims at a fixed spot, not a target to settle on: its own spread covers it.)
        let settle = if prefiring { 1.0 } else { 1.0 + SETTLE_SPREAD * (1.0 - (self.on_target_time / SETTLE_S).min(1.0)) };
        let suppress = if o.token_denied && !self.isolated && !o.taking_fire && dist >= 12.0 { SUPPRESS_SPREAD } else { 1.0 };
        let moving = 1.0 + move_dir.len() * 0.35;
        if o.taking_fire && !self.was_hit {
            self.flinch_t = FLINCH_S;
        }
        self.was_hit = o.taking_fire;
        self.flinch_t = (self.flinch_t - o.dt).max(0.0);
        let flinch = 1.0 + FLINCH_SPREAD * self.flinch_t / FLINCH_S;
        let sigma = t.aim_error_deg.to_radians() * settle * moving * flinch * suppress * if prefiring { PREFIRE_SPREAD } else { 1.0 };
        let shot_dir = V2::from_angle(self.aim_yaw + gauss(&mut self.rng) * sigma);
        let shot_pitch_err = gauss(&mut self.rng) * sigma * 0.6;

        // Behind low cover while holding or fighting: duck, pop up to look and shoot, duck to reload.
        let ducking = matches!(self.goal, Goal::Hold | Goal::Attack) && self.path.is_empty() && posture::low_cover_toward(cx.arena, o.pos, o.player_pos);
        if ducking {
            self.peek_t = (self.peek_t + o.dt) % PEEK_CYCLE_S;
        } else {
            self.peek_t = 0.0;
        }
        self.crouch_hold -= o.dt;
        let crouch = if let Some(phase) = posting {
            // Post loop: down behind the post, up to peek, upright on the move.
            phase == PostPhase::Hide && self.path.is_empty()
        } else if ducking {
            !fire && (o.ammo_frac <= 0.0 || (!o.player_visible && self.peek_t < PEEK_DOWN_S))
        } else {
            match self.goal {
                Goal::TakeCover | Goal::Retreat => self.path.is_empty(),
                Goal::Attack => {
                    // The net's crouch call holds for a beat: no bobbing every burst.
                    let want_c = micro[3] > 0.3;
                    if want_c != self.crouch_prev && self.crouch_hold <= 0.0 {
                        self.crouch_prev = want_c;
                        self.crouch_hold = 0.5 + 0.6 * self.rng_f();
                    }
                    self.crouch_prev
                }
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
            || posting == Some(PostPhase::Move) && far
            || (matches!(self.goal, Goal::Flank | Goal::Retreat | Goal::Push) || self.goal == Goal::Hold && far || combat && self.goal == Goal::Hunt)
                && (!o.player_visible || sticky && far);
        // Reload when empty, when calm, or early while hidden (a post, or cover he can't see).
        let hidden_now = !cx.arena.line_of_sight(o.player_pos, false, o.pos, true);
        let reload = o.ammo_frac <= 0.0
            || (!engaged && o.ammo_frac < 0.4)
            || posting == Some(PostPhase::Hide) && self.path.is_empty() && o.ammo_frac < 0.6
            || hidden_now && !o.player_visible && o.ammo_frac < 0.35;
        // Lean out from the post toward the peek side (the game shows it and moves the eye).
        let look = V2::from_angle(self.aim_yaw);
        let lean = match self.post {
            Some(st) if st.phase == PostPhase::Peek && st.post.peek.dist(st.post.pos) > 0.3 && o.pos.dist(st.post.peek) < 0.9 => {
                let side = (st.post.peek - st.post.pos).norm().dot(look.perp());
                if side.abs() > 0.3 { side.signum() } else { 0.0 }
            }
            _ => 0.0,
        };
        if reload && !self.was_reloading {
            self.bark = Some("Reloading");
        }
        self.was_reloading = reload;

        self.debug.goal = Some(self.goal);
        self.debug.post = posting;
        self.debug.isolated = self.isolated;
        self.debug.dest = self.dest;
        self.debug.path.clone_from(&self.path);

        Action {
            move_dir,
            sprint,
            crouch,
            look_dir: look,
            fire,
            shot_dir,
            shot_pitch_err,
            reload,
            prefire: prefire.filter(|_| fire),
            lean,
        }
    }

    /// Advance the peek-shoot-hide loop. `want`: fight from posts; `reload_spot`: cover close
    /// by to reload behind (used when not fighting from posts).
    fn update_post(&mut self, o: &Observation, cx: &MindCtx, want: bool, reload_spot: Option<V2>) {
        for r in &mut self.recent_peeks {
            r.1 += o.dt;
        }
        self.recent_peeks.retain(|r| r.1 < posts::RECENT_PEEK_S);
        self.post_retry -= o.dt;
        let reloading_here = self.post.is_some_and(|p| p.reload_only) && o.ammo_frac < 0.99;
        if want {
            if self.post.is_some_and(|p| p.reload_only) && !reloading_here {
                self.post = None;
            }
        } else if reloading_here {
            // Keep going: finish the reload behind the cover.
        } else if let Some(spot) = reload_spot {
            if self.post.is_none_or(|p| !p.reload_only) {
                let post = posts::Post { pos: spot, peek: spot, tall: false, score: 0.0 };
                self.post = Some(PostState { post, phase: PostPhase::Move, t: 0.0, fired: 0.0, burst: 0.0, peeked: false, reload_only: true });
                self.force_repath = true;
            }
        } else {
            if self.post.is_some_and(|p| !p.reload_only && p.peeked) {
                // Left the loop after a full cycle: give the squad's tactics a few seconds.
                self.post_retry = 2.0;
            }
            self.post = None;
            return;
        }
        if self.post.is_none() {
            if self.post_retry > 0.0 {
                return;
            }
            match posts::pick(cx.arena, o.pos, o.player_pos, &self.recent_peeks, None) {
                Some(post) => {
                    self.post = Some(PostState { post, phase: PostPhase::Move, t: 0.0, fired: 0.0, burst: 0.0, peeked: false, reload_only: false });
                    self.force_repath = true;
                }
                None => {
                    self.post_retry = 1.0;
                    return;
                }
            }
        }
        let Some(mut st) = self.post else { return };
        let before = st.target();
        let r = self.rng_f();
        match st.phase {
            PostPhase::Move => {
                st.t += o.dt;
                if o.pos.dist(st.post.pos) < 0.7 {
                    st.phase = PostPhase::Hide;
                    st.t = if st.reload_only { 0.3 } else { 0.3 + 0.5 * r };
                } else if st.t > 6.0 {
                    // Can't get there: try another.
                    self.post = None;
                    self.post_retry = 0.5;
                    return;
                }
            }
            PostPhase::Hide => {
                st.t -= o.dt;
                // Seen at the post: it's no longer cover. Fight back briefly, then move on.
                if o.player_visible && !st.reload_only {
                    st.peeked = true;
                    st.t = st.t.min(0.5);
                }
                let reloading = o.ammo_frac < 0.6;
                if st.t <= 0.0 && !reloading {
                    if st.reload_only {
                        self.post = None;
                        return;
                    }
                    if st.peeked && !self.isolated {
                        // Squad nearby: one cycle, then back to the squad's tactics.
                        self.post = None;
                        self.post_retry = 2.0;
                        return;
                    }
                    let next = if st.peeked { posts::pick(cx.arena, o.pos, o.player_pos, &self.recent_peeks, Some(st.post.pos)) } else { None };
                    match next {
                        Some(post) => {
                            st = PostState { post, phase: PostPhase::Move, t: 0.0, fired: 0.0, burst: 0.0, peeked: false, reload_only: false };
                        }
                        _ => {
                            // Peek (again) from here, from the cell seeing his latest position.
                            if let Some(peek) = posts::peek_cell(cx.arena, st.post.pos, o.player_pos) {
                                st.post.peek = peek;
                            }
                            st.phase = PostPhase::Peek;
                            st.t = PEEK_MAX_S;
                            st.fired = 0.0;
                            st.burst = PEEK_BURST_S.0 + (PEEK_BURST_S.1 - PEEK_BURST_S.0) * r;
                            self.recent_peeks.push((st.post.peek, 0.0));
                        }
                    }
                }
            }
            PostPhase::Peek => {
                st.t -= o.dt;
                if st.fired >= st.burst || st.t <= 0.0 || (o.taking_fire && st.fired > 0.15) || o.ammo_frac <= 0.0 {
                    st.phase = PostPhase::Hide;
                    st.t = HIDE_S.0 + (HIDE_S.1 - HIDE_S.0) * r;
                    st.peeked = true;
                }
            }
        }
        if st.target() != before {
            self.force_repath = true;
        }
        self.post = Some(st);
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

