//! Fast 2D top-down trainer world (docs/EVOLUTION.md). Same arena, nav, LOS and
//! `EnemyMind` as the game; the human is replaced by a `Ghost` built from the player model.

use crate::arena::Arena;
use crate::limits::Limits;
use crate::math::{V2, angle_diff, rng_from_seed};
use crate::mind::{AllyInfo, Blackboard, EnemyMind, MindCtx, Observation};
use crate::utility::Goal;
use crate::ghost::{EnemyView, Ghost, GhostMode, GhostStats, GhostView};
use crate::player_model::PlayerModel;
use rand::Rng;

pub const DT: f32 = 0.05;
pub const ROUND_TIME: f32 = 60.0;
pub const HP: f32 = 100.0;
pub const PLAYER_DAMAGE: f32 = 25.0;
pub const FIRE_INTERVAL: f32 = 0.1;
pub const MAG: u32 = 30;
pub const RELOAD_S: f32 = 2.0;
pub const WALK: f32 = 4.0;
pub const SPRINT: f32 = 6.0;
pub const CROUCH_SPEED: f32 = 2.0;
pub const BODY_RADIUS: f32 = 0.4;
pub const VISION_RANGE: f32 = 55.0;
pub const VISION_HALF_ANGLE: f32 = 1.05; // ~120° cone
pub const HEARING_RANGE: f32 = 30.0;
/// Window for assist and trade credit (s).
pub const CREDIT_WINDOW: f32 = 3.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct EnemyStats {
    pub damage_dealt: f32,
    pub kills: u32,
    pub time_alive: f32,
    pub time_engaged: f32,
    pub damage_taken: f32,
    /// Hit the player within `CREDIT_WINDOW` s before a teammate killed them.
    pub assists: u32,
    /// Damage dealt within `CREDIT_WINDOW` s after a teammate died (trading for them).
    pub trade_damage: f32,
}

impl EnemyStats {
    /// Individual shaping reward rho_i (docs/EVOLUTION.md, paper section 4.3). Selection is on
    /// squad win probability; this breaks ties and feeds the team-spirit blend.
    pub fn fitness(&self) -> f32 {
        let idle_penalty = if self.time_engaged <= 0.0 { 20.0 } else { 0.0 };
        (self.damage_dealt + 40.0 * self.kills as f32 + 25.0 * self.assists as f32 + 0.5 * self.trade_damage + 0.5 * self.time_alive
            + 0.3 * self.time_engaged
            - idle_penalty)
            .max(0.0)
    }
}

#[derive(Clone, Debug)]
pub struct RoundResult {
    pub player_won: bool,
    pub duration: f32,
    pub enemies: Vec<EnemyStats>,
    /// The ghost's own numbers (for calibration against real telemetry).
    pub ghost: GhostStats,
}

pub(crate) struct Body {
    pub(crate) pos: V2,
    pub(crate) vel: V2,
    pub(crate) hp: f32,
    pub(crate) crouched: bool,
    pub(crate) ammo: u32,
    pub(crate) reload_t: f32,
    fire_cd: f32,
    pub(crate) hit_t: f32,
}

impl Body {
    pub(crate) fn new(pos: V2) -> Self {
        Body { pos, vel: V2::ZERO, hp: HP, crouched: false, ammo: MAG, reload_t: 0.0, fire_cd: 0.0, hit_t: 99.0 }
    }
    pub(crate) fn alive(&self) -> bool {
        self.hp > 0.0
    }
    pub(crate) fn height(&self) -> f32 {
        if self.crouched { 1.1 } else { 1.8 }
    }
    /// Advance weapon timers; returns true if a shot can be fired now (and consumes it).
    pub(crate) fn try_fire(&mut self, want: bool) -> bool {
        self.fire_cd -= DT;
        if self.reload_t > 0.0 {
            self.reload_t -= DT;
            if self.reload_t <= 0.0 {
                self.ammo = MAG;
            }
            return false;
        }
        if want && self.fire_cd <= 0.0 && self.ammo > 0 {
            self.fire_cd = FIRE_INTERVAL;
            self.ammo -= 1;
            if self.ammo == 0 {
                self.reload_t = RELOAD_S;
            }
            return true;
        }
        false
    }
}

/// Does a ray from `from` along `dir` (with vertical error) hit a body at `to`?
fn ray_hits(from: V2, dir: V2, pitch_err: f32, to: V2, target_h: f32) -> bool {
    let d = to - from;
    let along = d.dot(dir);
    if along <= 0.0 || along > VISION_RANGE {
        return false;
    }
    let lateral = d.cross(dir).abs();
    let vertical_ok = pitch_err.abs() < (target_h * 0.5 / along).atan();
    lateral < BODY_RADIUS && vertical_ok
}

fn visible(arena: &Arena, from: &Body, look: V2, to: &Body, use_cone: bool) -> bool {
    let d = to.pos - from.pos;
    let dist = d.len();
    if dist > VISION_RANGE {
        return false;
    }
    if use_cone && dist > 5.0 && from.hit_t > 1.0 && angle_diff(d.angle(), look.angle()).abs() > VISION_HALF_ANGLE {
        return false;
    }
    arena.line_of_sight(from.pos, from.crouched, to.pos, to.crouched)
}

/// Run one round against Ghost v2. `minds` are consumed (one per enemy spawn).
pub fn run_round(arena: &Arena, model: &PlayerModel, limits: Limits, minds: Vec<EnemyMind>, seed: u64) -> RoundResult {
    run_round_with(arena, model, limits, minds, seed, GhostMode::V2)
}

/// `run_round` with a chosen ghost (`GhostMode::V1` is the legacy baseline).
pub fn run_round_with(arena: &Arena, model: &PlayerModel, limits: Limits, mut minds: Vec<EnemyMind>, seed: u64, mode: GhostMode) -> RoundResult {
    let n = minds.len();
    let mut enemies: Vec<Body> = arena.enemy_spawns.iter().take(n).map(|&p| Body::new(p)).collect();
    let mut looks: Vec<V2> = vec![V2::new(0.0, 1.0); n];
    let mut stats = vec![EnemyStats::default(); n];
    let mut last_seen: Vec<f32> = vec![99.0; n];
    let mut last_known: Vec<V2> = vec![arena.player_spawn; n];
    let mut heard: Vec<Option<V2>> = vec![None; n];
    let mut ghost = Ghost::new(arena.player_spawn, model, seed, mode);
    let mut enemy_fired = vec![false; n];
    let mut rng = rng_from_seed(seed);
    let cx = MindCtx { arena, model, limits };
    let mut t = 0.0;
    let mut ghost_shot = false;
    let mut board = Blackboard::default();
    let mut last_hit: Vec<f32> = vec![f32::NEG_INFINITY; n]; // assist/trade bookkeeping
    let mut last_ally_death = f32::NEG_INFINITY;

    while t < ROUND_TIME && ghost.body.alive() && enemies.iter().any(Body::alive) {
        t += DT;
        // --- Enemies think and act.
        let snapshot: Vec<(V2, bool, bool, bool)> = (0..n)
            .map(|i| (enemies[i].pos, enemies[i].alive(), last_seen[i] < 1.5, minds[i].goal() == Goal::Flank))
            .collect();
        let mut ghost_spotted = false;
        board.tick(DT, (0..n).filter(|&i| enemies[i].alive()).map(|i| (last_known[i], last_seen[i])));
        for i in 0..n {
            if !enemies[i].alive() {
                continue;
            }
            stats[i].time_alive += DT;
            let sees = visible(arena, &enemies[i], looks[i], &ghost.body, true);
            ghost_spotted |= sees;
            if sees {
                last_seen[i] = 0.0;
                last_known[i] = ghost.body.pos;
            } else {
                last_seen[i] += DT;
            }
            if ghost_shot && enemies[i].pos.dist(ghost.body.pos) < HEARING_RANGE {
                heard[i] = Some(ghost.body.pos);
            }
            if last_seen[i] < 1.5 {
                stats[i].time_engaged += DT;
            }
            let allies = (0..n)
                .filter(|&j| j != i && snapshot[j].1)
                .map(|j| AllyInfo { pos: snapshot[j].0, engaged: snapshot[j].2, flanking: snapshot[j].3 })
                .collect();
            let shared = board.merge(last_known[i], last_seen[i]);
            let obs = Observation {
                dt: DT,
                pos: enemies[i].pos,
                hp: enemies[i].hp / HP,
                ammo_frac: if enemies[i].reload_t > 0.0 { 0.0 } else { enemies[i].ammo as f32 / MAG as f32 },
                crouched: enemies[i].crouched,
                player_visible: sees,
                player_pos: shared.0,
                player_vel: if sees { ghost.body.vel } else { V2::ZERO },
                player_hp: ghost.body.hp / HP,
                player_facing: ghost.facing,
                time_since_seen: shared.1,
                heard_shot: heard[i],
                allies,
                taking_fire: enemies[i].hit_t < 0.6,
                player_reloading: ghost.body.reload_t > 0.0,
            };
            let act = minds[i].think(&obs, &cx);
            let e = &mut enemies[i];
            e.crouched = act.crouch;
            looks[i] = act.look_dir;
            let speed = if e.crouched { CROUCH_SPEED } else if act.sprint { SPRINT } else { WALK };
            e.vel = act.move_dir * speed;
            e.pos = arena.resolve_collision(e.pos + e.vel * DT, BODY_RADIUS);
            e.hit_t += DT;
            if act.reload && e.reload_t <= 0.0 && e.ammo < MAG {
                e.reload_t = RELOAD_S;
            }
            enemy_fired[i] = e.try_fire(act.fire);
            if enemy_fired[i] && sees && ray_hits(e.pos, act.shot_dir, act.shot_pitch_err, ghost.body.pos, ghost.body.height()) {
                let dmg = limits.damage_per_hit;
                ghost.body.hp -= dmg;
                ghost.body.hit_t = 0.0;
                stats[i].damage_dealt += dmg;
                last_hit[i] = t;
                if t - last_ally_death <= CREDIT_WINDOW {
                    stats[i].trade_damage += dmg;
                }
                if !ghost.body.alive() {
                    stats[i].kills += 1;
                    for j in (0..n).filter(|&j| j != i && t - last_hit[j] <= CREDIT_WINDOW) {
                        stats[j].assists += 1;
                    }
                }
            }
        }
        if ghost_spotted {
            ghost.stats.engaged_s += DT;
        }
        if !ghost.body.alive() {
            break;
        }

        // --- Ghost acts (ghost.rs); its shots resolve geometrically, like the enemies' do.
        let look = V2::from_angle(ghost.facing);
        let views: Vec<EnemyView> = (0..n)
            .map(|i| {
                let e = &enemies[i];
                EnemyView {
                    pos: e.pos,
                    vel: e.vel,
                    alive: e.alive(),
                    crouched: e.crouched,
                    visible: e.alive() && visible(arena, &ghost.body, look, e, true),
                    fired: enemy_fired[i],
                }
            })
            .collect();
        let view = GhostView { arena, model, enemies: &views };
        let act = ghost.act(&view);
        ghost.body.vel = act.vel;
        ghost_shot = act.shot.is_some();
        if let Some(shot) = act.shot {
            let from = ghost.body.pos;
            let hit = match shot.scripted_hit {
                Some(h) => h.then_some(shot.target),
                None => (0..n)
                    .filter(|&j| {
                        let e = &enemies[j];
                        e.alive()
                            && ray_hits(from, shot.dir, shot.pitch_err, e.pos, e.height())
                            && arena.line_of_sight(from, ghost.body.crouched, e.pos, e.crouched)
                    })
                    .min_by(|&x, &y| enemies[x].pos.dist(from).total_cmp(&enemies[y].pos.dist(from))),
            };
            let mut killed = false;
            if let Some(j) = hit {
                let e = &mut enemies[j];
                let dmg = if rng.random::<f32>() < 0.12 { PLAYER_DAMAGE * 2.0 } else { PLAYER_DAMAGE };
                e.hp -= dmg;
                e.hit_t = 0.0;
                stats[j].damage_taken += dmg;
                killed = !e.alive();
                if killed {
                    last_ally_death = t;
                }
            }
            let dist = enemies[shot.target].pos.dist(from);
            ghost.on_shot(dist, hit.is_some(), killed, &view);
        }
        ghost.body.pos = arena.resolve_collision(ghost.body.pos + ghost.body.vel * DT, BODY_RADIUS);
    }

    RoundResult { player_won: ghost.body.alive(), duration: t, enemies: stats, ghost: ghost.stats }
}
