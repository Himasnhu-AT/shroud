//! Sound: gunshots (player flat, enemies spatial), hit/headshot/kill feedback, hurt, low-HP
//! heartbeat, footsteps (player and enemies), reload foley, round stingers and an ambient bed.
//!
//! Everything is driven by existing messages and state; nothing here touches gameplay.
//! Disabled in `--headless` and with `--mute`. Missing files are skipped silently.
//! Master volume: [`MasterVolume`] (`--volume 0.5` on the command line).

use std::collections::HashMap;

use bevy::audio::{PlaybackSettings, SpatialListener, Volume};
use bevy::prelude::*;
use rand::Rng;

use crate::assets::AssetsDir;
use crate::enemy::{Enemy, EnemyCombat};
use crate::match_flow::MatchFlow;
use crate::player::{Player, PlayerCamera, PlayerMotor};
use crate::state::{GameState, Headless};
use crate::weapon::{Damage, Dummy, Health, Rifle, ShotFired, Tracer, WeaponSet};

/// Linear master gain applied to every sound (0..=1).
#[derive(Resource, Clone, Copy, Debug)]
pub struct MasterVolume(pub f32);

impl Default for MasterVolume {
    fn default() -> Self {
        Self(0.8)
    }
}

/// Loaded sound groups, keyed by base name (`step` holds `step_0..`).
#[derive(Resource, Default)]
struct Sfx(HashMap<&'static str, Vec<Handle<AudioSource>>>);

/// (group name, number of variations). Files are `audio/<name>.ogg` or `audio/<name>_<i>.ogg`.
const SOUNDS: &[(&str, usize)] = &[
    ("rifle_shot", 1),
    ("enemy_shot", 1),
    ("impact", 3),
    ("ricochet", 1),
    ("hit_marker", 1),
    ("headshot", 1),
    ("kill_confirm", 1),
    ("player_hurt", 1),
    ("heartbeat", 1),
    ("step", 5),
    ("reload_start", 1),
    ("reload_end", 1),
    ("empty_click", 1),
    ("round_start", 1),
    ("round_won", 1),
    ("round_lost", 1),
    ("ambient", 1),
];

#[derive(Component)]
struct Ambient;

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let args: Vec<String> = std::env::args().collect();
        let headless = app.world().get_resource::<Headless>().is_some_and(|h| h.0);
        let mute = args.iter().any(|a| a == "--mute");
        let mut master = MasterVolume::default();
        if let Some(v) = args.iter().position(|a| a == "--volume").and_then(|i| args.get(i + 1))
            && let Ok(v) = v.parse::<f32>()
        {
            master.0 = v.clamp(0.0, 1.0);
        }
        app.insert_resource(master);
        if headless || mute {
            return;
        }
        app.init_resource::<Sfx>()
            .add_systems(Startup, load_sfx)
            .add_systems(OnEnter(GameState::Playing), (start_ambient, play_round_start))
            .add_systems(OnExit(GameState::Playing), stop_ambient)
            .add_systems(Update, attach_listener)
            .add_systems(
                Update,
                (
                    shot_sounds,
                    tracer_sounds,
                    damage_sounds,
                    reload_sounds,
                    player_footsteps,
                    enemy_footsteps,
                    heartbeat,
                    round_stingers,
                )
                    .after(WeaponSet)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn load_sfx(mut sfx: ResMut<Sfx>, server: Res<AssetServer>, dir: Res<AssetsDir>) {
    for &(name, n) in SOUNDS {
        let mut v = Vec::new();
        for i in 0..n {
            let rel = if n == 1 { format!("audio/{name}.ogg") } else { format!("audio/{name}_{i}.ogg") };
            if dir.has(&rel) {
                v.push(server.load(rel));
            }
        }
        sfx.0.insert(name, v);
    }
}

fn attach_listener(mut commands: Commands, cams: Query<Entity, (With<PlayerCamera>, Without<SpatialListener>)>) {
    for e in &cams {
        // Narrow ear gap: world units are metres, so the default 4.0 would over-pan.
        commands.entity(e).insert(SpatialListener::new(0.4));
    }
}

/// Play `name` with randomised pitch/volume. `at = Some(pos)` makes it a spatial emitter.
fn play(
    commands: &mut Commands,
    sfx: &Sfx,
    master: &MasterVolume,
    name: &str,
    volume: f32,
    at: Option<Vec3>,
) {
    let Some(list) = sfx.0.get(name).filter(|l| !l.is_empty()) else { return };
    let mut rng = rand::rng();
    let handle = list[rng.random_range(0..list.len())].clone();
    let speed = rng.random_range(0.93..1.07);
    let vol = volume * rng.random_range(0.88..1.0) * master.0;
    let mut settings = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(vol)).with_speed(speed);
    match at {
        Some(p) => {
            settings = settings.with_spatial(true);
            commands.spawn((AudioPlayer::new(handle), settings, Transform::from_translation(p)));
        }
        None => {
            commands.spawn((AudioPlayer::new(handle), settings));
        }
    }
}

fn start_ambient(
    mut commands: Commands,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    existing: Query<(), With<Ambient>>,
) {
    if !existing.is_empty() {
        return;
    }
    if let Some(h) = sfx.0.get("ambient").and_then(|l| l.first()) {
        commands.spawn((
            Ambient,
            AudioPlayer::new(h.clone()),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(0.35 * master.0)),
        ));
    }
}

fn stop_ambient(mut commands: Commands, q: Query<Entity, With<Ambient>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

fn play_round_start(mut commands: Commands, sfx: Res<Sfx>, master: Res<MasterVolume>) {
    play(&mut commands, &sfx, &master, "round_start", 0.6, None);
}

fn shot_sounds(
    mut commands: Commands,
    mut shots: MessageReader<ShotFired>,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    player: Query<Entity, With<Player>>,
) {
    let Ok(player) = player.single() else { return };
    for s in shots.read() {
        if s.by == player {
            play(&mut commands, &sfx, &master, "rifle_shot", 0.9, None);
        } else {
            play(&mut commands, &sfx, &master, "enemy_shot", 1.0, Some(s.origin));
        }
    }
}

/// Impacts where tracers end, plus a whiz/ricochet when an enemy bullet passes close to the player.
fn tracer_sounds(
    mut commands: Commands,
    mut tracers: MessageReader<Tracer>,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    player: Query<&Transform, With<Player>>,
) {
    let Ok(ptf) = player.single() else { return };
    let eye = ptf.translation + Vec3::Y * 0.6;
    let mut rng = rand::rng();
    for t in tracers.read() {
        let seg = t.to - t.from;
        let len2 = seg.length_squared().max(1e-4);
        let u = ((eye - t.from).dot(seg) / len2).clamp(0.0, 1.0);
        let closest = t.from + seg * u;
        let from_enemy = t.from.distance(eye) > 1.5;
        if from_enemy && closest.distance(eye) < 2.5 && u < 0.98 {
            play(&mut commands, &sfx, &master, "ricochet", 0.7, Some(closest));
        }
        if t.to.distance(eye) < 80.0 {
            if rng.random_bool(0.18) {
                play(&mut commands, &sfx, &master, "ricochet", 0.5, Some(t.to));
            } else {
                play(&mut commands, &sfx, &master, "impact", 0.55, Some(t.to));
            }
        }
    }
}

fn damage_sounds(
    mut commands: Commands,
    mut dmg: MessageReader<Damage>,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    player: Query<Entity, With<Player>>,
    healths: Query<(&Health, Has<Dummy>)>,
) {
    let Ok(player) = player.single() else { return };
    for d in dmg.read() {
        if d.target == player {
            let v = (0.5 + d.amount / 40.0).min(1.0);
            play(&mut commands, &sfx, &master, "player_hurt", v, None);
        } else if d.source == player {
            let killed = healths.get(d.target).is_ok_and(|(h, dummy)| h.is_dead() && !dummy);
            if killed {
                if d.headshot {
                    play(&mut commands, &sfx, &master, "headshot", 0.8, None);
                }
                play(&mut commands, &sfx, &master, "kill_confirm", 0.9, None);
            } else if d.headshot {
                play(&mut commands, &sfx, &master, "headshot", 0.9, None);
            } else {
                play(&mut commands, &sfx, &master, "hit_marker", 0.7, None);
            }
        }
    }
}

fn reload_sounds(
    mut commands: Commands,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    rifle: Query<&Rifle, With<Player>>,
    mut prev: Local<Option<bool>>,
    mut prev_ammo: Local<u32>,
) {
    let Ok(r) = rifle.single() else { return };
    let reloading = r.reloading.is_some();
    if let Some(was) = *prev {
        if !was && reloading {
            if *prev_ammo == 0 {
                play(&mut commands, &sfx, &master, "empty_click", 0.7, None);
            }
            play(&mut commands, &sfx, &master, "reload_start", 0.8, None);
        } else if was && !reloading {
            play(&mut commands, &sfx, &master, "reload_end", 0.9, None);
        }
    }
    *prev = Some(reloading);
    *prev_ammo = r.ammo;
}

fn player_footsteps(
    mut commands: Commands,
    time: Res<Time>,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    player: Query<&PlayerMotor, With<Player>>,
    mut dist: Local<f32>,
    mut was_grounded: Local<bool>,
) {
    let Ok(m) = player.single() else { return };
    let speed = Vec2::new(m.velocity.x, m.velocity.z).length();
    if m.grounded && !*was_grounded {
        play(&mut commands, &sfx, &master, "step", 0.9, None);
        *dist = 0.0;
    }
    *was_grounded = m.grounded;
    if !m.grounded || speed < 1.0 {
        // First step lands almost immediately after starting to move.
        *dist = 1.4;
        return;
    }
    *dist += speed * time.delta_secs();
    let stride = if speed > 6.5 { 2.6 } else { 2.0 };
    if *dist >= stride {
        *dist -= stride;
        let v = if m.crouched { 0.18 } else { (0.35 + speed / 14.0).min(0.8) };
        play(&mut commands, &sfx, &master, "step", v, None);
    }
}

fn enemy_footsteps(
    mut commands: Commands,
    time: Res<Time>,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    player: Query<&Transform, With<Player>>,
    enemies: Query<(Entity, &Transform, &EnemyCombat), With<Enemy>>,
    mut dist: Local<HashMap<Entity, f32>>,
) {
    let Ok(ptf) = player.single() else { return };
    dist.retain(|e, _| enemies.contains(*e));
    for (e, tf, c) in &enemies {
        let speed = (c.vel.x * c.vel.x + c.vel.y * c.vel.y).sqrt();
        if c.dead || speed < 1.0 || tf.translation.distance(ptf.translation) > 30.0 {
            continue;
        }
        let d = dist.entry(e).or_insert(0.0);
        *d += speed * time.delta_secs();
        if *d >= 2.1 {
            *d -= 2.1;
            let v = if c.crouched { 0.25 } else { 0.7 };
            let at = Vec3::new(tf.translation.x, 0.1, tf.translation.z);
            play(&mut commands, &sfx, &master, "step", v, Some(at));
        }
    }
}

/// Heartbeat below 35% HP; faster and louder as HP drops.
fn heartbeat(
    mut commands: Commands,
    time: Res<Time>,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    player: Query<&Health, With<Player>>,
    mut timer: Local<f32>,
) {
    let Ok(h) = player.single() else { return };
    let frac = h.hp / h.max;
    if h.is_dead() || frac >= LOW_HP_FRAC {
        *timer = 0.0;
        return;
    }
    *timer -= time.delta_secs();
    if *timer <= 0.0 {
        let severity = 1.0 - frac / LOW_HP_FRAC;
        *timer = 1.0 - 0.45 * severity;
        play(&mut commands, &sfx, &master, "heartbeat", 0.6 + 0.4 * severity, None);
    }
}

/// Fraction of max HP below which the low-HP effects (heartbeat, pulsing vignette) kick in.
pub const LOW_HP_FRAC: f32 = 0.35;

fn round_stingers(
    mut commands: Commands,
    sfx: Res<Sfx>,
    master: Res<MasterVolume>,
    flow: Res<MatchFlow>,
    mut prev: Local<bool>,
) {
    let over = flow.round_over;
    if let Some((_, won)) = over
        && !*prev
    {
        play(&mut commands, &sfx, &master, if won { "round_won" } else { "round_lost" }, 0.9, None);
    }
    *prev = over.is_some();
}
