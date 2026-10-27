//! Records the player's habits during a match for the AI's player model
//! (not crash/perf telemetry: that's `ops::trace`). Also writes the L0 engagement log:
//! one `DuelRecord` per duel, a 10 Hz trajectory and per-shot records.

use bevy::prelude::*;
use brain::math::V2;
use brain::player_model::{DuelOutcome, DuelRecord, MatchTelemetry, ShotRecord, TrajSample, classify_cover, range_bin};

use crate::enemy::{Enemy, EnemyCombat, EnemySet};
use crate::map::{ArenaRes, to_ground};
use crate::player::{Player, PlayerIntent};
use engine::fps::{FpsLook, FpsMotor};
use crate::state::GameState;
use crate::weapon::{Damage, Health, Rifle, ShotFired, WeaponSet};

/// A duel ends after this long without line of sight either way.
const DUEL_GRACE: f32 = 1.0;
/// Half-angle of the player's view cone used for "the player sees the enemy".
const PLAYER_VIEW_HALF: f32 = 0.9;

#[derive(Resource, Default)]
pub struct Telemetry(pub MatchTelemetry);

struct OpenDuel {
    rec: DuelRecord,
    start: f32,
    last_contact: f32,
}

#[derive(Resource, Default)]
struct Sampler {
    timer: f32,
    /// Seconds since the match started.
    clock: f32,
    engaged: bool,
    engaged_at: Option<f32>,
    duel: Option<OpenDuel>,
    /// (end time, start position, cover block, peek side, outcome) of the last closed duel.
    last_duel: Option<(f32, V2, Option<u16>, brain::player_model::PeekSide, DuelOutcome)>,
    dead_enemies: usize,
}

pub struct TelemetryPlugin;

impl Plugin for TelemetryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Telemetry>()
            .init_resource::<Sampler>()
            .add_systems(OnEnter(GameState::Playing), new_round.after(crate::match_flow::begin_round))
            .add_systems(OnExit(GameState::Playing), close_open_duel)
            .add_systems(Update, record.after(EnemySet).after(WeaponSet).run_if(in_state(GameState::Playing)));
    }
}

fn new_round(mut t: ResMut<Telemetry>, mut s: ResMut<Sampler>) {
    t.0.rounds += 1;
    let clock = if t.0.rounds == 1 { 0.0 } else { s.clock };
    let last_duel = if t.0.rounds == 1 { None } else { s.last_duel };
    *s = Sampler { clock, last_duel, ..default() };
}

fn finish(t: &mut Telemetry, s: &mut Sampler, player_dead: bool) {
    let Some(mut d) = s.duel.take() else { return };
    d.rec.duration = d.last_contact - d.start;
    d.rec.outcome = if player_dead {
        DuelOutcome::Loss
    } else if d.rec.kills > 0 {
        DuelOutcome::Win
    } else {
        DuelOutcome::Disengage
    };
    s.last_duel = Some((s.clock, V2::new(d.rec.pos[0], d.rec.pos[1]), d.rec.cover_block, d.rec.peek, d.rec.outcome));
    info!(
        "duel: {:?} in {:?} peek {:?} cover {:?} range {:.0} m vs {} ({:.1} s)",
        d.rec.outcome, d.rec.zone, d.rec.peek, d.rec.cover, d.rec.range, d.rec.enemies_visible, d.rec.duration
    );
    t.0.push_duel(d.rec);
}

fn close_open_duel(mut t: ResMut<Telemetry>, mut s: ResMut<Sampler>, players: Query<&Health, With<Player>>) {
    let dead = players.single().is_ok_and(|h| h.is_dead());
    finish(&mut t, &mut s, dead);
}

#[allow(clippy::too_many_arguments)]
fn record(
    time: Res<Time>,
    arena: Res<ArenaRes>,
    intent: Res<PlayerIntent>,
    mut t: ResMut<Telemetry>,
    mut s: ResMut<Sampler>,
    players: Query<(Entity, &Transform, &FpsMotor, &FpsLook, &Health, &Rifle), With<Player>>,
    enemies: Query<(Entity, &Transform, &EnemyCombat), With<Enemy>>,
    mut shots: MessageReader<ShotFired>,
    mut damage: MessageReader<Damage>,
) {
    let Ok((player, ptf, motor, look, php, rifle)) = players.single() else { return };
    let s = &mut *s;
    let dt = time.delta_secs();
    s.clock += dt;
    let pos = to_ground(ptf.translation);
    let fwd = ptf.forward();
    let p_vel = V2::new(motor.velocity.x, motor.velocity.z);
    let speed = p_vel.len();
    let enemy_seen = enemies.iter().any(|(_, _, c)| c.sees_player && !c.dead);
    if enemy_seen && !s.engaged {
        s.engaged_at = Some(s.clock);
    }
    s.engaged = enemy_seen;

    // --- Contact: enemies that see the player, or that the player can see.
    let eye = ptf.translation + Vec3::Y * 0.6;
    let mut contacts: Vec<(V2, f32, bool)> = Vec::new(); // (pos, crosshair angle, player sees it)
    for (_, etf, c) in enemies.iter().filter(|e| !e.2.dead) {
        let ep = to_ground(etf.translation);
        let to = (etf.translation + Vec3::Y * 1.0 - eye).normalize_or_zero();
        let ang = fwd.dot(to).clamp(-1.0, 1.0).acos();
        let player_sees = ang < PLAYER_VIEW_HALF && arena.0.line_of_sight(pos, motor.crouched, ep, c.crouched);
        if player_sees || c.sees_player {
            contacts.push((ep, ang, player_sees));
        }
    }
    let dead_now = enemies.iter().filter(|e| e.2.dead).count();
    let new_kills = dead_now.saturating_sub(s.dead_enemies);
    s.dead_enemies = dead_now;

    let damages: Vec<Damage> = damage.read().cloned().collect();
    let player_shots: Vec<ShotFired> = shots.read().filter(|sh| sh.by == player).cloned().collect();

    // --- Duel bookkeeping.
    if let Some(d) = s.duel.as_mut() {
        d.rec.kills = d.rec.kills.saturating_add(new_kills as u8);
    }
    if !contacts.is_empty() {
        if s.duel.is_none() {
            let (ep, ang, _) = *contacts.iter().min_by(|a, b| a.0.dist(pos).total_cmp(&b.0.dist(pos))).unwrap();
            let (cover, peek, cover_block) = classify_cover(&arena.0, pos, ep);
            let (follow_up, repeek, after_kill) = match s.last_duel {
                Some((end, lpos, lblock, lpeek, outcome)) if s.clock - end <= 8.0 => {
                    let same_angle = (lblock.is_some() && lblock == cover_block && lpeek == peek) || lpos.dist(pos) < 2.5;
                    (true, s.clock - end <= 5.0 && same_angle, outcome == DuelOutcome::Win)
                }
                _ => (false, false, false),
            };
            let rec = DuelRecord {
                t: s.clock,
                round: t.0.rounds,
                player_saw_first: contacts.iter().any(|c| c.2) && !enemy_seen,
                zone: arena.0.zone_of(pos).map_or(String::new(), |z| z.name.to_string()),
                pos: [pos.x, pos.y],
                cover,
                peek,
                cover_block,
                range: ep.dist(pos),
                enemies_visible: contacts.len().min(255) as u8,
                hp: php.hp / php.max,
                ammo: rifle.ammo,
                moving: speed > 0.5,
                crouched: motor.crouched,
                preaim_err: ang,
                follow_up,
                repeek,
                after_kill,
                ..default()
            };
            s.duel = Some(OpenDuel { rec, start: s.clock, last_contact: s.clock });
        }
        if let Some(d) = s.duel.as_mut() {
            d.last_contact = s.clock;
        }
    }
    let clock = s.clock;
    if let Some(d) = s.duel.as_mut() {
        if d.rec.first_shot_s.is_none() && !player_shots.is_empty() {
            d.rec.first_shot_s = Some(clock - d.start);
        }
        for dm in &damages {
            if dm.source == player {
                d.rec.dmg_dealt += dm.amount;
            } else if dm.target == player {
                d.rec.dmg_taken += dm.amount;
            }
        }
    }
    let alive = enemies.iter().filter(|e| !e.2.dead).count();
    if php.is_dead() || alive == 0 || s.duel.as_ref().is_some_and(|d| clock - d.last_contact > DUEL_GRACE) {
        finish(&mut t, s, php.is_dead());
    }

    // --- Shots: reaction, per-range counts, per-shot records.
    for shot in &player_shots {
        t.0.shot_with(shot.kind);
        if let Some(at) = s.engaged_at.take() {
            let r = s.clock - at;
            if r < 2.0 {
                t.0.reaction(r);
            }
        }
        let dir = shot.dir.normalize_or_zero();
        // The targeted enemy: smallest angle off the shot, within ~20 degrees.
        let target = enemies
            .iter()
            .filter(|e| !e.2.dead)
            .map(|(e, etf, c)| {
                let to = etf.translation + Vec3::Y * 1.0 - shot.origin;
                (e, etf, c, dir.dot(to.normalize_or_zero()).clamp(-1.0, 1.0).acos(), to.length())
            })
            .min_by(|a, b| a.3.total_cmp(&b.3));
        let Some((e, etf, c, err, dist)) = target.filter(|t| t.3 < 0.35) else { continue };
        if err < 0.11 {
            t.0.shots[range_bin(dist)] += 1.0;
        }
        let rel = to_ground(etf.translation) - pos;
        let rel_v = c.vel - p_vel;
        let hit = damages.iter().find(|d| d.source == player && d.target == e);
        t.0.push_shot(ShotRecord {
            t: s.clock,
            range: dist,
            ang_vel: rel.cross(rel_v).abs() / rel.len_sq().max(0.25),
            moving: speed > 0.5,
            crouched: motor.crouched,
            aim_err: err,
            hit: hit.is_some(),
            headshot: hit.is_some_and(|d| d.headshot),
        });
    }
    for d in damages.iter().filter(|d| d.source == player) {
        if let Ok((_, etf, _)) = enemies.get(d.target) {
            let dist = to_ground(etf.translation).dist(pos);
            t.0.hits[range_bin(dist)] += 1.0;
            if d.headshot {
                t.0.headshots += 1.0;
            }
        }
    }

    // --- 10 Hz samples.
    s.timer -= dt;
    if s.timer <= 0.0 {
        s.timer = 0.1;
        let right = ptf.right();
        let lateral = if speed > 0.5 { p_vel.dot(V2::new(right.x, right.z)) / speed } else { 0.0 };
        t.0.sample(pos, speed, motor.crouched, enemy_seen, lateral);
        t.0.look(V2::new(fwd.x, fwd.z).angle());
        t.0.push_traj(TrajSample {
            t: s.clock,
            x: pos.x,
            z: pos.y,
            vx: p_vel.x,
            vz: p_vel.y,
            yaw: look.yaw,
            pitch: look.pitch,
            hp: php.hp / php.max,
            fire: intent.fire,
            crouch: motor.crouched,
        });
    }
}
