//! Ballistics for the player's guns, all from [`brain::weapons::WeaponSpec`]: fire modes
//! (auto / semi / pump / bolt), pellets, the spread cone, deterministic recoil patterns,
//! damage falloff, headshots, penetration through thin cover, and reloads (tactical, empty,
//! shell by shell). Surfaces come from the arena blocks (`BlockKind` + `Skin`), so effects and
//! penetration need nothing from the map code.

use avian3d::prelude::*;
use bevy::prelude::*;
use brain::arena::{Arena, Block, BlockKind, Skin};
use engine::fps::{FpsMotor, FpsViewKick};
use engine::gfx::MainCamera;

use super::arsenal::{ReloadKind, Weapon};
use super::viewmodel::MuzzleCam;
use super::{Damage, FireMode, GunEvent, Health, HitMarker, Hitbox, Impact, ShotFired, Tracer, WeaponKind, WeaponSet};
use crate::map::ArenaRes;
use crate::player::{Player, PlayerIntent};

/// Most surfaces one bullet passes through.
const MAX_PENETRATIONS: usize = 2;
/// Recoil spring back to centre (1/s): slow while a string is being fired, fast after.
const RECOVER_FIRING: f32 = 2.2;
const RECOVER_IDLE: f32 = 8.5;
/// Highest the view climbs from recoil.
const MAX_CLIMB: f32 = 0.22;
/// A pause this long starts a new recoil string.
const STRING_RESET_S: f32 = 0.32;
/// A semi-auto pull during cooldown fires when the gun is ready, if within this long.
const TRIGGER_BUFFER_S: f32 = 0.14;

// Reload stage marks (bit flags on `Weapon::reload_marks`).
const MARK_OUT: u8 = 1;
const MARK_IN: u8 = 2;
const MARK_CHAMBER: u8 = 4;

pub(super) fn build(app: &mut App) {
    app.add_systems(Update, fire_and_reload.in_set(WeaponSet));
}

/// What a bullet hit, for effects, sound and penetration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Surface {
    #[default]
    Concrete,
    Plaster,
    Metal,
    Wood,
    /// Armoured suits: sparks and a dull thud, no blood.
    Suit,
}

impl Surface {
    pub fn of_block(b: &Block) -> Surface {
        match b.skin {
            Skin::Crate | Skin::Desk => Surface::Wood,
            Skin::Container | Skin::Barrel | Skin::Cabinet | Skin::Siding | Skin::Car | Skin::Shelf | Skin::Dumpster => {
                Surface::Metal
            }
            Skin::Drywall | Skin::Plaster | Skin::Stucco => Surface::Plaster,
            Skin::Auto => match b.kind {
                BlockKind::Tall => Surface::Metal,
                _ => Surface::Concrete,
            },
            _ => Surface::Concrete,
        }
    }
}

/// Bullets go through this block (if the gun can penetrate and it is thin enough there).
pub fn penetrable(b: &Block) -> bool {
    b.kind != BlockKind::Wall && (b.kind == BlockKind::Low || matches!(b.skin, Skin::Crate | Skin::Desk | Skin::Drywall | Skin::Cabinet | Skin::Shelf | Skin::Car))
}

/// Thickest cover (metres along the bullet) a gun with this `penetration` gets through.
pub fn max_thickness(penetration: f32) -> f32 {
    if penetration <= 0.0 { 0.0 } else { 0.6 + 3.0 * penetration }
}

/// Fraction of damage left after passing `thickness` metres of cover.
pub fn penetration_mult(penetration: f32, thickness: f32) -> f32 {
    let max = max_thickness(penetration);
    if max <= 0.0 || thickness > max {
        return 0.0;
    }
    penetration * (1.0 - 0.5 * thickness / max)
}

/// The arena block containing world point `p` (blocks stand on y = 0).
pub fn block_at(arena: &Arena, p: Vec3) -> Option<&Block> {
    const EPS: f32 = 0.03;
    arena.blocks.iter().find(|b| {
        (p.x - b.center.x).abs() <= b.half.x + EPS && (p.z - b.center.y).abs() <= b.half.y + EPS && p.y <= b.height + EPS && p.y >= -EPS
    })
}

/// Surface at a hit point (`n` = hit normal); the floor, roofs and lintels are concrete.
pub fn surface_at(arena: &Arena, p: Vec3, n: Vec3) -> (Surface, Option<&Block>) {
    match block_at(arena, p - n * 0.02) {
        Some(b) => (Surface::of_block(b), Some(b)),
        None => (Surface::Concrete, None),
    }
}

/// Distance from `p` (inside or on the block) to where a ray along `dir` leaves it.
pub fn exit_distance(b: &Block, p: Vec3, dir: Vec3) -> f32 {
    let min = Vec3::new(b.center.x - b.half.x, 0.0, b.center.y - b.half.y);
    let max = Vec3::new(b.center.x + b.half.x, b.height, b.center.y + b.half.y);
    let mut t = f32::INFINITY;
    for i in 0..3 {
        let d = dir[i];
        if d.abs() < 1e-6 {
            continue;
        }
        let bound = if d > 0.0 { max[i] } else { min[i] };
        t = t.min(((bound - p[i]) / d).max(0.0));
    }
    if t.is_finite() { t } else { 0.0 }
}

/// Recoil for shot `i` of a string: (pitch up, yaw) in radians. Deterministic per gun: a
/// climb that steepens over the first shots and a horizontal wander you can learn.
pub fn recoil_pattern(kind: WeaponKind, i: u32) -> (f32, f32) {
    let s = kind.spec();
    let n = i as f32;
    let climb = 1.0 + (n.min(8.0)) * 0.07;
    let (phase, drift) = match kind {
        WeaponKind::Pistol => (0.4, 0.0),
        WeaponKind::Smg => (1.3, 0.35),
        WeaponKind::Rifle => (2.1, -0.45),
        WeaponKind::Shotgun => (0.9, 0.0),
        WeaponKind::Marksman => (3.0, 0.0),
        WeaponKind::HeavyPistol => (0.7, 0.0),
        // The attacker rifle pulls hard right, then swings left late in a spray.
        WeaponKind::BattleRifle => (0.3, 0.6),
        WeaponKind::Sniper => (2.6, 0.0),
    };
    let wander = (n * 0.55 + phase).sin() * 0.7 + (n * 1.37 + phase * 2.0).sin() * 0.3;
    // Late in a long string the gun leans to one side.
    let lean = drift * ((n - 6.0).max(0.0) / 10.0).min(1.0);
    (s.kick_pitch * climb, s.kick_yaw * (wander + lean))
}

/// A direction inside a cone of half-angle `half` around `fwd`; `u, v` in 0..1.
pub fn cone_dir(fwd: Vec3, right: Vec3, up: Vec3, half: f32, u: f32, v: f32) -> Vec3 {
    let r = half * u.sqrt();
    let a = v * std::f32::consts::TAU;
    (fwd + right * (r * a.cos()).tan() + up * (r * a.sin()).tan()).normalize_or(fwd)
}

/// Pellet `i` of `n` for a shotgun: one near the centre, the rest in a jittered ring.
fn pellet_offset(i: u32, n: u32) -> (f32, f32) {
    let j = || rand::random_range(-1.0f32..1.0);
    if i == 0 {
        return (0.15 * j().abs(), rand::random_range(0.0..1.0));
    }
    let ring = n.saturating_sub(1).max(1) as f32;
    let r = (0.55 + 0.35 * j().abs()).min(1.0);
    let a = (i as f32 - 1.0) / ring + 0.04 * j();
    (r * r, a.rem_euclid(1.0))
}

/// The trigger, recoil, reloads and every bullet the player fires this frame.
pub fn fire_and_reload(
    time: Res<Time>,
    spatial: SpatialQuery,
    arena: Res<ArenaRes>,
    intent: Res<PlayerIntent>,
    muzzle: Res<MuzzleCam>,
    mut players: Query<(Entity, &Transform, &mut Weapon, &mut FpsViewKick, &FpsMotor), With<Player>>,
    cams: Query<&Transform, (With<MainCamera>, Without<Player>)>,
    hitboxes: Query<&Hitbox>,
    healths: Query<(), With<Health>>,
    mut marker: ResMut<HitMarker>,
    mut out: FireOut,
    mut hits: Local<Vec<(Entity, f32, bool)>>,
    mut last_breath: Local<Vec2>,
) {
    let dt = time.delta_secs();
    let Ok((player, ptf, mut w, mut kick, motor)) = players.single_mut() else { return };
    let spec = w.kind.spec();
    let kind = w.kind;

    // Timers.
    w.since_shot += dt;
    if w.since_shot > STRING_RESET_S && w.burst != 0 {
        w.burst = 0;
    }
    w.cooldown -= dt;
    if !intent.fire || w.cooldown < -0.05 {
        w.cooldown = w.cooldown.max(0.0);
    }
    w.queued = (w.queued - dt).max(0.0);
    if w.cycling > 0.0 {
        let before = w.cycling;
        w.cycling = (w.cycling - dt).max(0.0);
        // The pump / bolt is worked a beat after the shot.
        let at = (spec.cycle_s - 0.2).max(0.05);
        if before > at && w.cycling <= at {
            out.events.write(if kind == WeaponKind::Shotgun { GunEvent::Pump } else { GunEvent::Bolt });
        }
    }
    let pressed = intent.fire && !w.trigger_was;
    if w.trigger_was != intent.fire {
        w.trigger_was = intent.fire;
    }
    if pressed && spec.mode != FireMode::Auto {
        w.queued = TRIGGER_BUFFER_S;
    }

    // Recoil recovery.
    let rate = if w.since_shot < spec.shot_interval() * 1.6 { RECOVER_FIRING } else { RECOVER_IDLE };
    if kick.pitch != 0.0 || kick.yaw != 0.0 {
        let k = (-rate * dt).exp();
        kick.pitch *= k;
        kick.yaw *= k;
        if kick.pitch.abs() < 1e-5 && kick.yaw.abs() < 1e-5 {
            kick.pitch = 0.0;
            kick.yaw = 0.0;
        }
    }

    // Breathing: the scoped view drifts in a slow figure-eight (the aim moves with it).
    let t = time.elapsed_secs();
    let breath = if w.scoped() { Vec2::new((t * 0.7).sin() * 0.0011, (t * 1.4).sin() * 0.0014) } else { Vec2::ZERO };
    if breath != *last_breath {
        kick.yaw += breath.x - last_breath.x;
        kick.pitch += breath.y - last_breath.y;
        *last_breath = breath;
    }

    if !w.ready() {
        return;
    }

    // Reloading.
    if let Some(left) = w.reloading {
        // Shotgun: the trigger interrupts between shells.
        if w.reload_kind == ReloadKind::Shells && pressed && w.ammo > 0 {
            w.reloading = None;
            w.queued = TRIGGER_BUFFER_S;
        } else {
            let left = left - dt;
            let p = 1.0 - left / w.reload_total.max(1e-3);
            if w.reload_kind != ReloadKind::Shells {
                if p >= 0.15 && w.reload_marks & MARK_OUT == 0 {
                    w.reload_marks |= MARK_OUT;
                    out.events.write(GunEvent::MagOut { kind });
                }
                let in_at = if w.reload_kind == ReloadKind::Empty { 0.55 } else { 0.68 };
                if p >= in_at && w.reload_marks & MARK_IN == 0 {
                    w.reload_marks |= MARK_IN;
                    out.events.write(GunEvent::MagIn { kind });
                }
                if w.reload_kind == ReloadKind::Empty && p >= 0.84 && w.reload_marks & MARK_CHAMBER == 0 {
                    w.reload_marks |= MARK_CHAMBER;
                    out.events.write(GunEvent::Chamber { kind });
                }
            }
            if left > 0.0 {
                w.reloading = Some(left);
                return;
            }
            match w.reload_kind {
                ReloadKind::Shells => {
                    w.ammo += 1;
                    w.reserve -= 1;
                    out.events.write(GunEvent::ShellIn);
                    if w.reload_total >= spec.reload_empty_s - 1e-3 && spec.reload_empty_s > spec.reload_s {
                        out.events.write(GunEvent::Pump);
                    }
                    if w.ammo < w.mag && w.reserve > 0 {
                        w.reloading = Some(spec.reload_s);
                        w.reload_total = spec.reload_s;
                    } else {
                        w.reloading = None;
                    }
                }
                _ => {
                    let take = (w.mag - w.ammo).min(w.reserve);
                    w.ammo += take;
                    w.reserve -= take;
                    w.reloading = None;
                }
            }
            w.reload_marks = 0;
            if w.reloading.is_some() {
                return;
            }
        }
    }

    // Start a reload: R, or the trigger on an empty mag.
    let wants_reload = (intent.reload && w.ammo < w.mag) || (intent.fire && w.ammo == 0);
    if wants_reload && w.reloading.is_none() {
        if w.reserve > 0 {
            let empty = w.ammo == 0;
            let (rk, t) = match (spec.pellets > 1 || kind == WeaponKind::Shotgun, empty) {
                (true, true) => (ReloadKind::Shells, spec.reload_empty_s),
                (true, false) => (ReloadKind::Shells, spec.reload_s),
                (false, true) => (ReloadKind::Empty, spec.reload_empty_s),
                (false, false) => (ReloadKind::Tactical, spec.reload_s),
            };
            w.reload_kind = rk;
            w.reload_total = t;
            w.reloading = Some(t);
            w.reload_marks = 0;
            w.inspect = 0.0;
            return;
        } else if pressed && w.ammo == 0 {
            out.events.write(GunEvent::DryFire);
        }
    }

    // Fire.
    let want = match spec.mode {
        FireMode::Auto => intent.fire,
        _ => w.queued > 0.0,
    };
    if !want || w.cooldown > 0.0 || w.ammo == 0 || w.sprint > 0.35 {
        return;
    }
    let Ok(cam_local) = cams.single() else { return };
    // Compose from local transforms: GlobalTransform lags a frame behind movement.
    let cam = ptf.mul_transform(*cam_local);
    let origin = cam.translation;
    let (fwd, right, up) = (*cam.forward(), *cam.right(), *cam.up());

    w.ammo -= 1;
    w.cooldown += spec.shot_interval();
    w.queued = 0.0;
    w.since_shot = 0.0;
    w.inspect = 0.0;
    if spec.mode == FireMode::Single {
        w.cycling = spec.cycle_s;
    }
    let spread = w.spread;
    w.bloom = (w.bloom + spec.spread_per_shot).min(spec.spread_per_shot * 10.0 + 0.02);

    // Recoil: the pattern, softened aimed and crouched.
    let (kp, ky) = recoil_pattern(kind, w.burst);
    w.burst += 1;
    let soften = (1.0 - 0.3 * w.ads) * if motor.crouched { 0.85 } else { 1.0 };
    kick.pitch = (kick.pitch + kp * soften).min(MAX_CLIMB);
    kick.yaw += ky * soften;

    out.shots.write(ShotFired::new(origin, fwd, player, kind));
    out.events.write(GunEvent::Shot { kind });
    let muzzle_world = cam.transform_point(muzzle.0);

    hits.clear();
    for i in 0..spec.pellets.max(1) {
        let (u, v) = if spec.pellets > 1 { pellet_offset(i, spec.pellets) } else { (rand::random_range(0.0..1.0), rand::random_range(0.0..1.0)) };
        let dir = cone_dir(fwd, right, up, spread, u, v);
        let Ok(dir3) = Dir3::new(dir) else { continue };
        let mut from = origin;
        // The shooter, then each piece of cover already passed through.
        let mut excluded: [Entity; 1 + MAX_PENETRATIONS] = [player; 1 + MAX_PENETRATIONS];
        let mut travelled = 0.0;
        let mut mult = 1.0;
        let mut end = origin + dir * spec.max_range;
        for pass in 0..=MAX_PENETRATIONS {
            let filter = SpatialQueryFilter::from_excluded_entities(excluded[..=pass].iter().copied());
            let Some(hit) = spatial.cast_ray(from, dir3, spec.max_range - travelled, true, &filter) else {
                end = from + dir * (spec.max_range - travelled);
                break;
            };
            let p = from + dir * hit.distance;
            travelled += hit.distance;
            end = p;
            let target = match hitboxes.get(hit.entity) {
                Ok(hb) => Some((hb.owner, hb.head)),
                Err(_) if healths.contains(hit.entity) => Some((hit.entity, false)),
                Err(_) => None,
            };
            if let Some((target, head)) = target {
                let amount = spec.damage_at(travelled) * mult * if head { spec.headshot_mult } else { 1.0 };
                match hits.iter_mut().find(|h| h.0 == target) {
                    Some(h) => {
                        h.1 += amount;
                        h.2 |= head;
                    }
                    None => hits.push((target, amount, head)),
                }
                out.impacts.write(Impact { pos: p, normal: hit.normal, flesh: true, surface: Surface::Suit, exit: false });
                break;
            }
            let (surface, block) = surface_at(&arena.0, p, hit.normal);
            out.impacts.write(Impact { pos: p, normal: hit.normal, flesh: false, surface, exit: false });
            // Through thin cover?
            let Some(b) = block.filter(|b| pass < MAX_PENETRATIONS && spec.penetration > 0.0 && penetrable(b)) else { break };
            let thick = exit_distance(b, p + dir * 0.01, dir) + 0.01;
            let keep = penetration_mult(spec.penetration, thick);
            if keep <= 0.05 {
                break;
            }
            mult *= keep;
            from = p + dir * (thick + 0.02);
            travelled += thick + 0.02;
            out.impacts.write(Impact { pos: from, normal: dir, flesh: false, surface, exit: true });
            excluded[pass + 1] = hit.entity;
        }
        // Every bullet of a rifle; a few pellets of a shotgun.
        if i < 3 {
            out.tracers.write(Tracer { from: muzzle_world, to: end });
        }
    }
    for &(target, amount, headshot) in hits.iter() {
        out.damage.write(Damage { target, amount, source: player, headshot });
        let head = headshot || (marker.timer > 0.0 && marker.headshot);
        *marker = HitMarker { timer: super::HIT_MARKER_S, headshot: head, kill: false };
    }
}

/// Message writers for [`fire_and_reload`] (bundled to stay under the system parameter limit).
#[derive(bevy::ecs::system::SystemParam)]
pub struct FireOut<'w> {
    shots: MessageWriter<'w, ShotFired>,
    damage: MessageWriter<'w, Damage>,
    tracers: MessageWriter<'w, Tracer>,
    impacts: MessageWriter<'w, Impact>,
    events: MessageWriter<'w, GunEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain::math::V2;

    fn block(kind: BlockKind, skin: Skin) -> Block {
        Block { center: V2::new(0.0, 0.0), half: V2::new(1.0, 0.25), height: 1.1, kind, skin }
    }

    #[test]
    fn recoil_is_deterministic_and_climbs() {
        for k in WeaponKind::ALL {
            assert_eq!(recoil_pattern(k, 3), recoil_pattern(k, 3));
            assert!(recoil_pattern(k, 0).0 > 0.0);
            assert!(recoil_pattern(k, 6).0 >= recoil_pattern(k, 0).0);
        }
    }

    #[test]
    fn penetration_by_gun_and_cover() {
        let rifle = WeaponKind::Rifle.spec().penetration;
        let smg = WeaponKind::Smg.spec().penetration;
        assert!(penetration_mult(rifle, 0.5) > 0.0);
        assert_eq!(penetration_mult(smg, 0.5), 0.0);
        assert!(penetration_mult(rifle, 0.2) > penetration_mult(rifle, 1.0));
        assert_eq!(penetration_mult(rifle, 10.0), 0.0);
        assert!(penetrable(&block(BlockKind::Low, Skin::Auto)));
        assert!(penetrable(&block(BlockKind::Tall, Skin::Drywall)));
        assert!(!penetrable(&block(BlockKind::Tall, Skin::Concrete)));
        assert!(!penetrable(&block(BlockKind::Wall, Skin::Drywall)));
    }

    #[test]
    fn exit_distance_through_a_slab() {
        let b = block(BlockKind::Low, Skin::Crate);
        // Entering the 0.5 m deep face head-on along +Z.
        let d = exit_distance(&b, Vec3::new(0.0, 0.5, -0.25), Vec3::Z);
        assert!((d - 0.5).abs() < 1e-4, "{d}");
        assert_eq!(Surface::of_block(&b), Surface::Wood);
    }

    #[test]
    fn cone_dir_stays_inside_the_cone() {
        let half = 0.05;
        for i in 0..50 {
            let d = cone_dir(Vec3::NEG_Z, Vec3::X, Vec3::Y, half, (i as f32 / 50.0).fract(), (i as f32 * 0.37).fract());
            assert!(d.angle_between(Vec3::NEG_Z) <= half * 1.01);
        }
    }
}
