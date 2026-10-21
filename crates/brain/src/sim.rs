//! Fast 2D top-down trainer world (docs/EVOLUTION.md). Same arena, nav, LOS and
//! `EnemyMind` as the game; the human is replaced by a `Ghost` built from the player model.

use crate::arena::Arena;
use crate::limits::Limits;
use crate::math::{V2, angle_diff, rng_from_seed};
use crate::mind::{AllyInfo, EnemyMind, MindCtx, Observation};
use crate::squad::{MemberView, SquadCoordinator, UtilityBudget};
use crate::utility::Goal;
use crate::perception::{self, Sound, GUNSHOT_RANGE, RELOAD_RANGE};
use crate::ghost::{EnemyView, Ghost, GhostMode, GhostStats, GhostView};
use crate::grenade::{self, GrenadeKind, SimGrenade};
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
/// Legacy single hearing radius (gunshots now use `perception::GUNSHOT_RANGE` with occlusion).
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
pub fn run_round_with(arena: &Arena, model: &PlayerModel, limits: Limits, minds: Vec<EnemyMind>, seed: u64, mode: GhostMode) -> RoundResult {
    run_round_squad(arena, model, limits, minds, seed, mode, true)
}

/// `run_round_with`, with the squad coordinator's plays on or off (off: minds act alone).
pub fn run_round_squad(arena: &Arena, model: &PlayerModel, limits: Limits, minds: Vec<EnemyMind>, seed: u64, mode: GhostMode, coordinated: bool) -> RoundResult {
    run_round_opts(arena, model, limits, minds, seed, mode, coordinated, UtilityBudget::STANDARD)
}

/// `run_round_squad` with a grenade budget (`UtilityBudget::NONE`: no grenades).
#[allow(clippy::too_many_arguments)]
pub fn run_round_opts(
    arena: &Arena,
    model: &PlayerModel,
    limits: Limits,
    mut minds: Vec<EnemyMind>,
    seed: u64,
    mode: GhostMode,
    coordinated: bool,
    utility: UtilityBudget,
) -> RoundResult {
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
    // --- squad layer (squad.rs) ---
    let mut squad = SquadCoordinator::new(arena, n);
    squad.enabled = coordinated;
    squad.utility = utility;
    // --- end squad layer ---
    let mut last_hit: Vec<f32> = vec![f32::NEG_INFINITY; n]; // assist/trade bookkeeping
    let mut last_ally_death = f32::NEG_INFINITY;
    // Utility: grenades in flight, and how long the ghost stays flashed.
    let mut grenades: Vec<SimGrenade> = Vec::new();
    let mut blind_t = 0.0f32;

    while t < ROUND_TIME && ghost.body.alive() && enemies.iter().any(Body::alive) {
        t += DT;
        // --- Enemies think and act.
        let snapshot: Vec<(V2, bool, bool, bool)> = (0..n)
            .map(|i| (enemies[i].pos, enemies[i].alive(), last_seen[i] < 1.5, minds[i].goal() == Goal::Flank))
            .collect();
        let mut ghost_spotted = false;
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
        squad.player_hp = ghost.body.hp / HP;
        let orders = squad.tick(DT, &views, arena, model).to_vec();
        squad.drain_callouts();
        for th in squad.take_throws() {
            if enemies[th.thrower].alive() {
                grenades.push(SimGrenade::new(arena, th.kind, th.thrower, enemies[th.thrower].pos, th.target));
            }
        }
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
            ghost_spotted |= sees;
            if sees {
                last_seen[i] = 0.0;
                last_known[i] = ghost.body.pos;
            } else {
                last_seen[i] += DT;
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
                player_hurt: ghost.body.hit_t < 0.8,
                squad_alert: board.alert,
                token_denied: !squad.tokens.get(i).copied().unwrap_or(true),
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
            // Pre-fire at a spot hits only if he really is there (in the open).
            let can_hit = sees || act.prefire.is_some() && arena.line_of_sight(e.pos, e.crouched, ghost.body.pos, ghost.body.crouched);
            if enemy_fired[i] && can_hit && ray_hits(e.pos, act.shot_dir, act.shot_pitch_err, ghost.body.pos, ghost.body.height()) {
                let dmg = limits.damage_per_hit;
                squad.player_damaged(dmg);
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
        // --- Grenades: fly, wait out the fuse, then hurt or blind the ghost.
        let gp = ghost.body.pos;
        let mut flee: Option<V2> = None;
        grenades.retain_mut(|g| {
            g.fuse -= DT;
            let at = g.pos();
            if g.fuse > 0.0 {
                // A landed frag (heard clinking, called out) makes him run from it after a
                // reaction delay, like a player would.
                let d = at.dist(gp);
                let reacting = g.kind.fuse() - g.fuse > 0.45 * g.kind.fuse() + model.reaction_s;
                if g.kind == GrenadeKind::Frag && reacting && d < grenade::FRAG_RADIUS + 1.0 {
                    flee = Some(at);
                }
                return true;
            }
            match g.kind {
                GrenadeKind::Frag => {
                    let dmg = grenade::frag_damage_at(arena, at, gp, ghost.body.crouched);
                    if dmg > 0.0 && ghost.body.alive() {
                        ghost.body.hp -= dmg;
                        ghost.body.hit_t = 0.0;
                        stats[g.thrower].damage_dealt += dmg;
                        squad.player_damaged(dmg);
                        if !ghost.body.alive() {
                            stats[g.thrower].kills += 1;
                        }
                    }
                }
                GrenadeKind::Flash => {
                    if arena.line_of_sight(at, false, gp, ghost.body.crouched) {
                        let facing = V2::from_angle(ghost.facing).dot((at - gp).norm());
                        blind_t = blind_t.max(grenade::flash_duration(grenade::flash_intensity(facing, at.dist(gp))));
                    }
                }
            }
            false
        });
        if !ghost.body.alive() {
            break;
        }
        blind_t = (blind_t - DT).max(0.0);

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
                    visible: e.alive() && blind_t <= 0.0 && visible(arena, &ghost.body, look, e, true),
                    fired: enemy_fired[i],
                }
            })
            .collect();
        let view = GhostView { arena, model, enemies: &views };
        let mut act = ghost.act(&view);
        if blind_t > 0.0 {
            act.shot = None;
        }
        if let Some(at) = flee {
            // Like a player: dash to the nearest cover outside the blast, else straight away.
            let away = (gp - at).norm();
            let spot = arena
                .cover
                .iter()
                .map(|c| c.pos)
                .filter(|c| c.dist(at) > grenade::FRAG_RADIUS + 0.5 && c.dist(gp) < 9.0 && (*c - gp).dot(away) > -0.2 && arena.nav.clear_line(gp, *c))
                .min_by(|a, b| a.dist(gp).total_cmp(&b.dist(gp)));
            act.vel = spot.map_or(away, |c| (c - gp).norm()) * SPRINT;
        }
        ghost.body.vel = act.vel;
        // --- Stimuli the ghost makes this tick: gunshot, footsteps, reload (perception.rs).
        let gp = ghost.body.pos;
        let mut sounds = Vec::with_capacity(3);
        if act.shot.is_some() {
            sounds.push((Sound::Gunshot, gp, GUNSHOT_RANGE));
        }
        let steps = perception::footstep_range(act.vel.len(), ghost.body.crouched);
        if steps > 0.0 {
            sounds.push((Sound::Footstep, gp, steps));
        }
        if ghost.body.reload_t > 0.0 {
            sounds.push((Sound::Reload, gp, RELOAD_RANGE));
        }
        for i in (0..n).filter(|&i| enemies[i].alive()) {
            let near_miss = act.shot.as_ref().is_some_and(|s| perception::near_miss(arena, gp, s.dir, enemies[i].pos));
            let h = if near_miss { Some(perception::Heard { sound: Sound::Gunshot, pos: gp, loud: 1.0 }) } else { perception::listen(arena, &sounds, enemies[i].pos) };
            if let Some(h) = h {
                heard[i] = Some(h.pos);
                squad.hear(i, &h, arena);
            }
        }
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
                // Damage is a sense: everyone learns where the shot came from, now.
                squad.reveal(j, from, arena);
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
