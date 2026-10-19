//! Records the player's habits during a match for the AI's player model
//! (not crash/perf telemetry: that's `ops::trace`).

use bevy::prelude::*;
use brain::math::V2;
use brain::player_model::{MatchTelemetry, range_bin};

use crate::enemy::{Enemy, EnemyCombat, EnemySet};
use crate::map::to_ground;
use crate::player::{Player, PlayerMotor};
use crate::state::GameState;
use crate::weapon::{Damage, ShotFired, WeaponSet};

#[derive(Resource, Default)]
pub struct Telemetry(pub MatchTelemetry);

#[derive(Resource, Default)]
struct Sampler {
    timer: f32,
    clock: f32,
    engaged: bool,
    engaged_at: Option<f32>,
}

pub struct TelemetryPlugin;

impl Plugin for TelemetryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Telemetry>()
            .init_resource::<Sampler>()
            .add_systems(Update, record.after(EnemySet).after(WeaponSet).run_if(in_state(GameState::Playing)));
    }
}

fn record(
    time: Res<Time>,
    mut t: ResMut<Telemetry>,
    mut s: ResMut<Sampler>,
    players: Query<(Entity, &Transform, &PlayerMotor), With<Player>>,
    enemies: Query<(Entity, &Transform, &EnemyCombat), With<Enemy>>,
    mut shots: MessageReader<ShotFired>,
    mut damage: MessageReader<Damage>,
) {
    let Ok((player, ptf, motor)) = players.single() else { return };
    let dt = time.delta_secs();
    s.clock += dt;
    let pos = to_ground(ptf.translation);
    let engaged = enemies.iter().any(|(_, _, c)| c.sees_player && !c.dead);
    if engaged && !s.engaged {
        s.engaged_at = Some(s.clock);
    }
    s.engaged = engaged;

    for shot in shots.read().filter(|sh| sh.by == player) {
        if let Some(at) = s.engaged_at.take() {
            let r = s.clock - at;
            if r < 2.0 {
                t.0.reaction(r);
            }
        }
        // The enemy this shot was aimed at: closest to the ray, within ~6°.
        let target = enemies
            .iter()
            .filter_map(|(_, etf, _)| {
                let to = etf.translation + Vec3::Y * 1.0 - shot.origin;
                let cos = to.normalize_or_zero().dot(shot.dir.normalize_or_zero());
                (cos > 0.994).then(|| to.length())
            })
            .min_by(f32::total_cmp);
        if let Some(d) = target {
            t.0.shots[range_bin(d)] += 1.0;
        }
    }
    for d in damage.read().filter(|d| d.source == player) {
        if let Ok((_, etf, _)) = enemies.get(d.target) {
            let dist = to_ground(etf.translation).dist(pos);
            t.0.hits[range_bin(dist)] += 1.0;
            if d.headshot {
                t.0.headshots += 1.0;
            }
        }
    }

    s.timer -= dt;
    if s.timer <= 0.0 {
        s.timer = 0.1;
        let vel = Vec2::new(motor.velocity.x, motor.velocity.z);
        let speed = vel.length();
        let right = ptf.right();
        let lateral = if speed > 0.5 { vel.dot(Vec2::new(right.x, right.z)) / speed } else { 0.0 };
        t.0.sample(pos, speed, motor.crouched, engaged, lateral);
        let fwd = ptf.forward();
        t.0.look(V2::new(fwd.x, fwd.z).angle());
    }
}
