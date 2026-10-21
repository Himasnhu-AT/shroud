//! Enemy grenades in 3D: an arced projectile with ground/wall bounces (avian ray casts), a fuse,
//! frag damage with falloff and a line-of-sight check, and a flashbang that whites out the
//! player's screen depending on whether he was looking at it. The squad coordinator decides
//! the throws (`brain::squad::ThrowOrder`) and calls them out ("Frag out!", "Flashbang!").

use avian3d::prelude::*;
use bevy::prelude::*;
use brain::grenade::{self as g, GrenadeKind};

use super::{EnemyCombat, EnemySet, Squad};
use crate::map::to_ground;
use crate::player::{Player, PlayerMotor};
use crate::state::{GameState, has_visuals, not_paused};
use crate::weapon::{Damage, WeaponSet};

const GRAVITY: f32 = 9.81;
const RADIUS: f32 = 0.07;
const RESTITUTION: f32 = 0.38;
/// Horizontal speed kept per ground bounce.
const GROUND_FRICTION: f32 = 0.6;

/// Throw a grenade (written by the enemy AI when the coordinator releases a throw).
#[derive(Message, Clone, Copy, Debug)]
pub struct ThrowGrenade {
    pub thrower: Entity,
    pub kind: GrenadeKind,
    pub from: Vec3,
    pub target: Vec3,
}

/// A grenade in flight or on the ground.
#[derive(Component, Debug)]
pub struct Grenade {
    pub kind: GrenadeKind,
    pub thrower: Entity,
    pub vel: Vec3,
    pub fuse: f32,
    resting: bool,
}

/// The player's flashbang whiteout: peak intensity 0..1, seconds since, total duration.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Flashed {
    pub peak: f32,
    pub t: f32,
    pub dur: f32,
}

impl Flashed {
    /// Current overlay alpha: full white for ~40% of the duration, then fades out.
    pub fn alpha(&self) -> f32 {
        if self.dur <= 0.0 || self.t >= self.dur {
            return 0.0;
        }
        let f = self.t / self.dur;
        let hold = 0.4;
        let a = if f < hold { 1.0 } else { 1.0 - (f - hold) / (1.0 - hold) };
        (self.peak * a).clamp(0.0, 1.0)
    }
}

#[derive(Component)]
struct FlashOverlay;

#[derive(Component)]
struct Blast {
    ttl: f32,
    max: f32,
    size: f32,
}

/// Blinking warning light on a live frag.
#[derive(Component)]
struct FuseLight;

pub struct GrenadePlugin;

impl Plugin for GrenadePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ThrowGrenade>()
            .init_resource::<Flashed>()
            .add_systems(Startup, spawn_overlay.run_if(has_visuals))
            .add_systems(OnExit(GameState::Playing), clear)
            .add_systems(
                Update,
                (spawn_grenades, fly, detonate).chain().after(EnemySet).before(WeaponSet).run_if(in_state(GameState::Playing).and_then(not_paused)),
            )
            .add_systems(Update, (tick_flash, blink, tick_blasts.run_if(has_visuals)).run_if(not_paused))
            .add_systems(Update, update_overlay.run_if(has_visuals));
    }
}

/// Launch velocity that lands at `to` after a flight time that grows with distance (a lob).
pub fn launch_velocity(from: Vec3, to: Vec3) -> Vec3 {
    let d = to - from;
    let horiz = d.xz().length();
    let t = (horiz / 11.0).clamp(0.55, 1.5);
    Vec3::new(d.x / t, (d.y + 0.5 * GRAVITY * t * t) / t, d.z / t)
}

fn spawn_grenades(
    mut commands: Commands,
    mut msgs: MessageReader<ThrowGrenade>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut assets: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>, Handle<StandardMaterial>)>>,
) {
    let (mut meshes, mut materials) = (meshes, materials);
    for t in msgs.read() {
        let mut e = commands.spawn((
            Name::new(format!("{} grenade", t.kind.label())),
            Grenade { kind: t.kind, thrower: t.thrower, vel: launch_velocity(t.from, t.target), fuse: t.kind.fuse(), resting: false },
            Transform::from_translation(t.from),
            Visibility::default(),
            DespawnOnExit(GameState::Playing),
        ));
        if let (Some(m), Some(s)) = (meshes.as_deref_mut(), materials.as_deref_mut()) {
            let (mesh, frag_mat, flash_mat) = assets
                .get_or_insert_with(|| {
                    (
                        m.add(Sphere::new(RADIUS * 1.3)),
                        s.add(StandardMaterial { base_color: Color::srgb(0.18, 0.24, 0.12), perceptual_roughness: 0.7, ..default() }),
                        s.add(StandardMaterial { base_color: Color::srgb(0.75, 0.75, 0.7), metallic: 0.6, perceptual_roughness: 0.35, ..default() }),
                    )
                })
                .clone();
            let mat = if t.kind == GrenadeKind::Frag { frag_mat } else { flash_mat };
            e.insert((Mesh3d(mesh), MeshMaterial3d(mat)));
            if t.kind == GrenadeKind::Frag {
                e.with_child((FuseLight, PointLight { color: Color::srgb(1.0, 0.15, 0.1), intensity: 0.0, range: 3.0, ..default() }, Transform::default()));
            }
        }
    }
}

/// Ballistic integration with bounces: ray-cast each step's motion; reflect off what it hits.
fn fly(time: Res<Time>, spatial: SpatialQuery, mut q: Query<(&mut Grenade, &mut Transform)>, players: Query<Entity, With<Player>>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    // Bounce off the world, not off the player's capsule (it would deflect into his face).
    let filter = SpatialQueryFilter::from_excluded_entities(players.iter());
    for (mut gr, mut tf) in &mut q {
        gr.fuse -= dt;
        if gr.resting {
            continue;
        }
        gr.vel.y -= GRAVITY * dt;
        let mut pos = tf.translation;
        let mut left = dt;
        // Up to two bounces per frame.
        for _ in 0..3 {
            let step = gr.vel * left;
            let len = step.length();
            let Ok(dir) = Dir3::new(step) else { break };
            match spatial.cast_ray(pos, dir, len + RADIUS, true, &filter) {
                Some(hit) => {
                    let travel = (hit.distance - RADIUS).max(0.0);
                    pos += dir * travel;
                    left *= 1.0 - (travel / len.max(1e-6)).min(1.0);
                    let n = hit.normal.normalize_or(Vec3::Y);
                    // Reflect: the normal part bounces back (restitution), the tangential part
                    // slides on with friction (more on the floor than off a wall).
                    let vn = gr.vel.dot(n) * n;
                    let vt = gr.vel - vn;
                    let fric = if n.y > 0.7 { GROUND_FRICTION } else { 0.8 };
                    gr.vel = vt * fric - vn * RESTITUTION;
                    pos += n * 0.005;
                    if left <= 1e-4 {
                        break;
                    }
                }
                None => {
                    pos += step;
                    break;
                }
            }
        }
        // Safety floor (the arena floor top is y = 0).
        if pos.y < RADIUS {
            pos.y = RADIUS;
            if gr.vel.y < 0.0 {
                gr.vel.y = -gr.vel.y * RESTITUTION;
                gr.vel.x *= GROUND_FRICTION;
                gr.vel.z *= GROUND_FRICTION;
            }
        }
        if pos.y <= RADIUS + 0.02 && gr.vel.length() < 0.4 {
            gr.resting = true;
            gr.vel = Vec3::ZERO;
            pos.y = RADIUS;
        }
        tf.translation = pos;
    }
}

/// Does a ray from `from` reach `player` (nothing solid in between)?
fn reaches(spatial: &SpatialQuery, from: Vec3, to: Vec3, player: Entity, exclude: &[Entity]) -> bool {
    let d = to - from;
    let Ok(dir) = Dir3::new(d) else { return true };
    let filter = SpatialQueryFilter::from_excluded_entities(exclude.iter().copied());
    spatial.cast_ray(from, dir, d.length() + 0.3, true, &filter).is_none_or(|h| h.entity == player || h.distance >= d.length() - 0.2)
}

fn detonate(
    mut commands: Commands,
    spatial: SpatialQuery,
    q: Query<(Entity, &Grenade, &Transform)>,
    players: Query<(Entity, &Transform, &PlayerMotor), With<Player>>,
    mut enemies: Query<&mut EnemyCombat>,
    hitboxes: Query<Entity, With<super::BodyPart>>,
    mut squad: ResMut<Squad>,
    mut flashed: ResMut<Flashed>,
    mut damage: MessageWriter<Damage>,
    sfx: Option<Res<crate::audio::Sfx>>,
    master: Option<Res<crate::audio::Volumes>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let mut visuals = meshes.zip(materials);
    let player = players.single().ok();
    let damage_mult = squad.params.map_or(1.0, |p| p.damage_mult);
    // Enemy bodies don't shield the player from a blast (keeps the check about walls and cover).
    let shields: Vec<Entity> = hitboxes.iter().collect();
    for (e, gr, tf) in &q {
        if gr.fuse > 0.0 {
            continue;
        }
        commands.entity(e).despawn();
        let at = tf.translation + Vec3::Y * 0.15;
        match gr.kind {
            GrenadeKind::Frag => {
                if let Some((pe, ptf, motor)) = player {
                    let center = ptf.translation;
                    let head = center + Vec3::Y * if motor.crouched { 0.25 } else { 0.55 };
                    let d = to_ground(at).dist(to_ground(center)).max((at.y - center.y).abs() * 0.5);
                    let base = g::frag_damage(d);
                    let exposure = if base <= 0.0 {
                        0.0
                    } else if reaches(&spatial, at, center, pe, &shields) {
                        1.0
                    } else if reaches(&spatial, at, head, pe, &shields) {
                        0.5
                    } else {
                        0.0
                    };
                    let amount = base * exposure * damage_mult;
                    if amount > 0.5 {
                        damage.write(Damage { target: pe, amount, source: gr.thrower, headshot: false });
                        if let Some(c) = squad.coord.as_mut() {
                            c.player_damaged(amount);
                        }
                        if let Ok(mut c) = enemies.get_mut(gr.thrower) {
                            c.report.damage_dealt += amount;
                        }
                    }
                }
                if let (Some(sfx), Some(master)) = (&sfx, &master) {
                    let name = if sfx.has("frag_explode") { "frag_explode" } else { "impact" };
                    crate::audio::play_at(&mut commands, sfx, master, name, 1.0, at);
                }
                if let Some((m, s)) = visuals.as_mut() {
                    spawn_blast(&mut commands, &mut *m, &mut *s, at, LinearRgba::rgb(3.2, 1.1, 0.2), 2.0, 0.35, Color::srgb(1.0, 0.6, 0.25), 900_000.0);
                }
            }
            GrenadeKind::Flash => {
                if let Some((pe, ptf, _)) = player {
                    let eye = ptf.translation + Vec3::Y * 0.6;
                    let d = at.distance(eye);
                    if d < g::FLASH_RANGE && reaches(&spatial, at, eye, pe, &shields) {
                        let facing = ptf.forward().dot((at - eye).normalize_or_zero());
                        let i = g::flash_intensity(facing, d);
                        let dur = g::flash_duration(i);
                        // A stronger flash replaces a fading one.
                        if dur > 0.0 && i >= flashed.alpha() {
                            *flashed = Flashed { peak: i, t: 0.0, dur };
                        }
                    }
                }
                if let (Some(sfx), Some(master)) = (&sfx, &master)
                    && sfx.has("flashbang")
                {
                    crate::audio::play_at(&mut commands, sfx, master, "flashbang", 1.0, at);
                }
                if let Some((m, s)) = visuals.as_mut() {
                    spawn_blast(&mut commands, &mut *m, &mut *s, at, LinearRgba::rgb(20.0, 20.0, 22.0), 1.2, 0.18, Color::WHITE, 2_000_000.0);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_blast(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
    color: LinearRgba,
    size: f32,
    ttl: f32,
    light: Color,
    intensity: f32,
) {
    commands
        .spawn((
            Blast { ttl, max: ttl, size },
            Mesh3d(meshes.add(Sphere::new(1.0))),
            MeshMaterial3d(materials.add(StandardMaterial { base_color: color.into(), unlit: true, alpha_mode: AlphaMode::Add, ..default() })),
            Transform::from_translation(at).with_scale(Vec3::splat(0.1)),
            bevy::light::NotShadowCaster,
            DespawnOnExit(GameState::Playing),
        ))
        .with_child(PointLight { color: light, intensity, range: 14.0, ..default() });
}

fn tick_blasts(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Blast, &mut Transform)>) {
    for (e, mut b, mut tf) in &mut q {
        b.ttl -= time.delta_secs();
        if b.ttl <= 0.0 {
            commands.entity(e).despawn();
        } else {
            let f = 1.0 - b.ttl / b.max;
            tf.scale = Vec3::splat(b.size * (0.3 + 0.7 * f.sqrt()) * (1.0 - 0.6 * f));
        }
    }
}

/// The frag's warning light blinks faster as the fuse runs down.
fn blink(grenades: Query<(&Grenade, &Children)>, mut lights: Query<&mut PointLight, With<FuseLight>>) {
    for (gr, children) in &grenades {
        let rate = 3.0 + 9.0 * (1.0 - gr.fuse / gr.kind.fuse()).clamp(0.0, 1.0);
        let on = (gr.fuse * rate).fract() < 0.5;
        for c in children.iter() {
            if let Ok(mut l) = lights.get_mut(c) {
                l.intensity = if on { 40_000.0 } else { 0.0 };
            }
        }
    }
}

fn tick_flash(time: Res<Time>, mut f: ResMut<Flashed>) {
    if f.dur > 0.0 {
        f.t += time.delta_secs();
        if f.t >= f.dur {
            *f = Flashed::default();
        }
    }
}

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        Name::new("Flashbang overlay"),
        FlashOverlay,
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::srgba(1.0, 1.0, 0.97, 0.0)),
        GlobalZIndex(900),
        bevy::picking::Pickable::IGNORE,
    ));
}

/// Above the HUD (z 8) and debrief (20); hidden while paused so the pause menu stays readable.
fn update_overlay(f: Res<Flashed>, paused: Res<crate::state::Paused>, mut q: Query<&mut BackgroundColor, With<FlashOverlay>>) {
    let a = if paused.0 { 0.0 } else { f.alpha() };
    for mut bg in &mut q {
        if (bg.0.alpha() - a).abs() > 1e-3 {
            bg.0.set_alpha(a);
        }
    }
}

fn clear(mut f: ResMut<Flashed>) {
    *f = Flashed::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_lands_on_target() {
        let from = Vec3::new(0.0, 1.5, 0.0);
        let to = Vec3::new(12.0, 0.1, -5.0);
        let v = launch_velocity(from, to);
        // Integrate the ballistic path with a small step until it falls back to the target height.
        let (mut p, mut vel, dt) = (from, v, 1.0 / 240.0);
        for _ in 0..2000 {
            vel.y -= GRAVITY * dt;
            p += vel * dt;
            if vel.y < 0.0 && p.y <= to.y {
                break;
            }
        }
        assert!(p.xz().distance(to.xz()) < 0.25, "{p:?}");
    }

    #[test]
    fn flash_fades() {
        let f = Flashed { peak: 1.0, t: 0.0, dur: 2.0 };
        assert_eq!(f.alpha(), 1.0);
        assert!(Flashed { t: 1.5, ..f }.alpha() < 0.5);
        assert_eq!(Flashed { t: 2.0, ..f }.alpha(), 0.0);
    }
}
