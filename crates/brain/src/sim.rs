//! Fast 2D top-down trainer world (docs/EVOLUTION.md). Same arena, nav, LOS and
//! `EnemyMind` as the game; the human is replaced by a `Ghost` built from the player model.

use crate::arena::Arena;
use crate::limits::Limits;
use crate::math::{BrainRng, V2, angle_diff, rng_from_seed};
use crate::mind::{AllyInfo, EnemyMind, MindCtx, Observation};
use crate::squad::{MemberView, SquadCoordinator};
use crate::utility::Goal;
use crate::player_model::{HEAT_W, PlayerModel, ROUTE_W, heat_center, range_bin, route_idx};
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

#[derive(Clone, Copy, Debug, Default)]
pub struct EnemyStats {
    pub damage_dealt: f32,
    pub kills: u32,
    pub time_alive: f32,
    pub time_engaged: f32,
    pub damage_taken: f32,
}

impl EnemyStats {
    /// Fitness from docs/EVOLUTION.md.
    pub fn fitness(&self) -> f32 {
        let idle_penalty = if self.time_engaged <= 0.0 { 20.0 } else { 0.0 };
        (self.damage_dealt + 40.0 * self.kills as f32 + 0.5 * self.time_alive + 0.3 * self.time_engaged - idle_penalty).max(0.0)
    }
}

#[derive(Clone, Debug)]
pub struct RoundResult {
    pub player_won: bool,
    pub duration: f32,
    pub enemies: Vec<EnemyStats>,
}

struct Body {
    pos: V2,
    vel: V2,
    hp: f32,
    crouched: bool,
    ammo: u32,
    reload_t: f32,
    fire_cd: f32,
    hit_t: f32,
}

impl Body {
    fn new(pos: V2) -> Self {
        Body { pos, vel: V2::ZERO, hp: HP, crouched: false, ammo: MAG, reload_t: 0.0, fire_cd: 0.0, hit_t: 99.0 }
    }
    fn alive(&self) -> bool {
        self.hp > 0.0
    }
    fn height(&self) -> f32 {
        if self.crouched { 1.1 } else { 1.8 }
    }
    /// Advance weapon timers; returns true if a shot can be fired now (and consumes it).
    fn try_fire(&mut self, want: bool) -> bool {
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

/// Scripted stand-in for the human, parameterized by the player model.
pub struct Ghost {
    body: Body,
    facing: f32,
    dest: Option<V2>,
    path: Vec<V2>,
    still_t: f32,
    target: Option<usize>,
    target_seen: f32,
    strafe_dir: f32,
    strafe_t: f32,
    rng: BrainRng,
    reaction: f32,
    pref_range: f32,
}

impl Ghost {
    fn new(pos: V2, model: &PlayerModel, seed: u64) -> Self {
        let mut rng = rng_from_seed(seed ^ 0x9E37_79B9);
        let reaction = model.reaction_s / model.ghost_skill.max(0.1) * (0.8 + 0.4 * rng.random::<f32>());
        let mids = [3.0, 7.5, 12.5, 17.5, 25.0, 32.0];
        let b = (0..6).max_by(|&a, &b| model.kill_share[a].total_cmp(&model.kill_share[b])).unwrap_or(2);
        Ghost {
            body: Body::new(pos),
            facing: -std::f32::consts::FRAC_PI_2,
            dest: None,
            path: Vec::new(),
            still_t: 0.0,
            target: None,
            target_seen: 0.0,
            strafe_dir: 1.0,
            strafe_t: 0.0,
            rng,
            reaction,
            pref_range: mids[b],
        }
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
                    let cell = 64.0 / ROUTE_W as f32;
                    let c = V2::new(-32.0 + ((i % ROUTE_W) as f32 + 0.5) * cell, -32.0 + ((i / ROUTE_W) as f32 + 0.5) * cell);
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

/// Run one round. `minds` are consumed (one per enemy spawn).
pub fn run_round(arena: &Arena, model: &PlayerModel, limits: Limits, minds: Vec<EnemyMind>, seed: u64) -> RoundResult {
    run_round_with(arena, model, limits, minds, seed, true)
}

/// `run_round` with the squad coordinator's plays on or off (off: minds act alone).
pub fn run_round_with(arena: &Arena, model: &PlayerModel, limits: Limits, mut minds: Vec<EnemyMind>, seed: u64, coordinated: bool) -> RoundResult {
    let n = minds.len();
    let mut enemies: Vec<Body> = arena.enemy_spawns.iter().take(n).map(|&p| Body::new(p)).collect();
    let mut looks: Vec<V2> = vec![V2::new(0.0, 1.0); n];
    let mut stats = vec![EnemyStats::default(); n];
    let mut last_seen: Vec<f32> = vec![99.0; n];
    let mut last_known: Vec<V2> = vec![arena.player_spawn; n];
    let mut heard: Vec<Option<V2>> = vec![None; n];
    let mut ghost = Ghost::new(arena.player_spawn, model, seed);
    let mut rng = rng_from_seed(seed);
    let cx = MindCtx { arena, model, limits };
    let mut t = 0.0;
    let mut ghost_shot = false;
    // --- squad layer (squad.rs) ---
    let mut squad = SquadCoordinator::new(arena, n);
    squad.enabled = coordinated;
    // --- end squad layer ---

    while t < ROUND_TIME && ghost.body.alive() && enemies.iter().any(Body::alive) {
        t += DT;
        // --- Enemies think and act.
        let snapshot: Vec<(V2, bool, bool, bool)> = (0..n)
            .map(|i| (enemies[i].pos, enemies[i].alive(), last_seen[i] < 1.5, minds[i].goal() == Goal::Flank))
            .collect();
        // --- squad layer: one coordinator tick before the minds think ---
        let views: Vec<MemberView> = (0..n)
            .map(|i| MemberView {
                pos: enemies[i].pos,
                alive: enemies[i].alive(),
                hp: enemies[i].hp / HP,
                last_known: last_known[i],
                last_seen: last_seen[i],
                heard: heard[i],
                look: looks[i],
                traits: minds[i].traits,
                bark: minds[i].take_bark(),
            })
            .collect();
        let orders = squad.tick(DT, &views, arena, model).to_vec();
        squad.drain_callouts();
        for (m, o) in minds.iter_mut().zip(orders) {
            m.set_order(o);
        }
        let board = squad.board;
        // --- end squad layer ---
        for i in 0..n {
            if !enemies[i].alive() {
                continue;
            }
            stats[i].time_alive += DT;
            let sees = visible(arena, &enemies[i], looks[i], &ghost.body, true);
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
            if e.try_fire(act.fire) && sees && ray_hits(e.pos, act.shot_dir, act.shot_pitch_err, ghost.body.pos, ghost.body.height()) {
                let dmg = limits.damage_per_hit;
                ghost.body.hp -= dmg;
                ghost.body.hit_t = 0.0;
                stats[i].damage_dealt += dmg;
                if !ghost.body.alive() {
                    stats[i].kills += 1;
                }
            }
        }
        if !ghost.body.alive() {
            break;
        }

        // --- Ghost acts.
        ghost_shot = false;
        ghost.body.hit_t += DT;
        let look = V2::from_angle(ghost.facing);
        let mut best: Option<(f32, usize)> = None;
        for (i, e) in enemies.iter().enumerate() {
            if e.alive() && visible(arena, &ghost.body, look, e, true) {
                let d = e.pos.dist(ghost.body.pos);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, i));
                }
            }
        }
        match best {
            Some((dist, i)) => {
                if ghost.target != Some(i) {
                    ghost.target = Some(i);
                    ghost.target_seen = 0.0;
                }
                ghost.target_seen += DT;
                let to = (enemies[i].pos - ghost.body.pos).norm();
                ghost.facing = to.angle();
                // Strafe with the modeled bias, keep the modeled range.
                ghost.strafe_t -= DT;
                if ghost.strafe_t <= 0.0 {
                    ghost.strafe_t = 0.4 + 0.6 * ghost.rng.random::<f32>();
                    let p_right = 0.5 + 0.5 * model.strafe_bias;
                    ghost.strafe_dir = if ghost.rng.random::<f32>() < p_right { -1.0 } else { 1.0 };
                }
                ghost.body.crouched = ghost.rng.random::<f32>() < model.crouch_rate * 0.1 || (ghost.body.crouched && ghost.rng.random::<f32>() < 0.9);
                let range_push = ((dist - ghost.pref_range) / 10.0).clamp(-1.0, 1.0);
                let mv = (to.perp() * ghost.strafe_dir + to * range_push).clamp_len(1.0);
                let speed = if ghost.body.crouched { CROUCH_SPEED } else { WALK };
                ghost.body.vel = mv * speed;
                ghost.path.clear();
                ghost.dest = None;
                let want_fire = ghost.target_seen >= ghost.reaction;
                if ghost.body.try_fire(want_fire) {
                    ghost_shot = true;
                    let acc = (model.accuracy[range_bin(dist)] * model.ghost_skill).min(0.9);
                    if ghost.rng.random::<f32>() < acc {
                        let e = &mut enemies[i];
                        let dmg = if rng.random::<f32>() < 0.12 { PLAYER_DAMAGE * 2.0 } else { PLAYER_DAMAGE };
                        e.hp -= dmg;
                        e.hit_t = 0.0;
                        stats[i].damage_taken += dmg;
                    }
                }
            }
            None => {
                ghost.target = None;
                ghost.body.crouched = false;
                ghost.body.try_fire(false);
                if ghost.still_t > 0.0 {
                    ghost.still_t -= DT;
                    ghost.body.vel = V2::ZERO;
                    ghost.facing += 0.6 * DT; // look around
                } else {
                    if ghost.path.is_empty() {
                        if ghost.dest.is_some() && ghost.rng.random::<f32>() < model.camping {
                            ghost.still_t = 2.0 + 4.0 * ghost.rng.random::<f32>();
                        }
                        let d = ghost.pick_dest(model, arena);
                        ghost.dest = Some(d);
                        ghost.path = arena.nav.find_path(ghost.body.pos, d).unwrap_or_default();
                    }
                    if let Some(&wp) = ghost.path.first() {
                        if wp.dist(ghost.body.pos) < 0.6 {
                            ghost.path.remove(0);
                        }
                        let dir = (wp - ghost.body.pos).norm();
                        ghost.body.vel = dir * WALK;
                        if dir.len() > 0.0 {
                            ghost.facing = dir.angle();
                        }
                    }
                }
            }
        }
        ghost.body.pos = arena.resolve_collision(ghost.body.pos + ghost.body.vel * DT, BODY_RADIUS);
    }

    RoundResult { player_won: ghost.body.alive(), duration: t, enemies: stats }
}
