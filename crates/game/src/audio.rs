//! Sound: gunshots (player flat, enemies spatial), hit/headshot/kill feedback, hurt, low-HP
//! heartbeat, footsteps (player and enemies), round stingers and an ambient bed. The player's
//! guns (shots, reloads, foley) are `weapon::sound`.
//!
//! Everything is driven by existing messages and state; nothing here touches gameplay. Loading,
//! volumes (master x effects / ambience, live), muting (`--headless`, `--mute`, test runs unless
//! `--sound`) and `--volume` are the engine's (`engine::audio`). Missing files are skipped.

use std::collections::HashMap;

use bevy::prelude::*;
use engine::audio::{AudioCategory, SoundLibrary, Volumes, play_loop};
use engine::fps::FpsMotor;
use rand::Rng;

use crate::assets::AssetsDir;
use crate::enemy::{Enemy, EnemyCombat};
use crate::match_flow::MatchFlow;
use crate::player::Player;
use crate::state::GameState;
use crate::enemy::gun::EnemyFx;
use crate::weapon::{Damage, Dummy, Health, Impact, ShotFired, WeaponSet};

/// Shroud's sounds live in the engine's library.
pub(crate) type Sfx = SoundLibrary;

/// The ambient file is mixed hot: 0.35 at the default ambience volume (0.6).
const AMBIENT_GAIN: f32 = 0.35 / 0.6;

/// Spatial one-shot for other modules (enemy grenades). Silent if the group has no files.
pub(crate) fn play_at(commands: &mut Commands, sfx: &Sfx, master: &Volumes, name: &str, volume: f32, at: Vec3) {
    play(commands, sfx, master, name, volume, Some(at));
}

/// Play `name` (random variant, slight pitch/volume jitter) at the effects volume.
/// `at = Some(pos)` makes it a spatial emitter.
fn play(commands: &mut Commands, sfx: &Sfx, master: &Volumes, name: &str, volume: f32, at: Option<Vec3>) {
    engine::audio::play(commands, sfx, master, name, AudioCategory::Sfx, volume, at);
}

/// (group name, number of variations). Files are `audio/<name>.ogg` or `audio/<name>_<i>.ogg`.
const SOUNDS: &[(&str, usize)] = &[
    ("rifle_shot", 1),
    ("enemy_shot", 1),
    ("impact", 3),
    ("ricochet", 1),
    // A supersonic round passing close (`scripts/make-combat-sounds.py`).
    ("bullet_crack", 3),
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
    // Optional: plays on squad callouts if `audio/radio_click.ogg` exists.
    ("radio_click", 1),
    // Optional: enemy grenades (`enemy/grenade.rs`; frags fall back to `impact`).
    ("frag_explode", 1),
    ("flashbang", 1),
    // The bomb (`bomb.rs`): keypad while planting, beeps, the defuse kit, the blast.
    ("bomb_plant", 1),
    ("bomb_beep", 1),
    ("bomb_defuse", 1),
    ("bomb_explode", 1),
];

#[derive(Component)]
struct Ambient;

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        if app.world().resource::<Volumes>().muted {
            return;
        }
        app.add_systems(Startup, load_sfx)
            .add_systems(OnEnter(GameState::Playing), (start_ambient, play_round_start))
            .add_systems(OnExit(GameState::Playing), stop_ambient)
            .add_systems(
                Update,
                (
                    shot_sounds,
                    impact_sounds,
                    enemy_bullet_sounds,
                    damage_sounds,
                    player_footsteps,
                    enemy_footsteps,
                    heartbeat,
                    round_stingers,
                    radio_clicks,
                    bomb_sounds,
                )
                    .after(WeaponSet)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn load_sfx(mut sfx: ResMut<Sfx>, server: Res<AssetServer>, dir: Res<AssetsDir>) {
    for &(name, n) in SOUNDS {
        sfx.load_variants(&server, &dir, name, &format!("audio/{name}"), n);
    }
}

fn start_ambient(mut commands: Commands, sfx: Res<Sfx>, master: Res<Volumes>, existing: Query<(), With<Ambient>>) {
    if !existing.is_empty() {
        return;
    }
    if let Some(h) = sfx.0.get("ambient").and_then(|l| l.first())
        && let Some(e) = play_loop(&mut commands, &master, h.clone(), AudioCategory::Ambience, AMBIENT_GAIN)
    {
        commands.entity(e).insert(Ambient);
    }
}

fn stop_ambient(mut commands: Commands, q: Query<Entity, With<Ambient>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

fn play_round_start(mut commands: Commands, sfx: Res<Sfx>, master: Res<Volumes>) {
    play(&mut commands, &sfx, &master, "round_start", 0.6, None);
}

fn shot_sounds(
    mut commands: Commands,
    mut shots: MessageReader<ShotFired>,
    sfx: Res<Sfx>,
    master: Res<Volumes>,
    player: Query<Entity, With<Player>>,
) {
    let Ok(player) = player.single() else { return };
    for s in shots.read() {
        // The player's guns are voiced per gun (`weapon::sound`).
        if s.by != player {
            play(&mut commands, &sfx, &master, "enemy_shot", 1.0, Some(s.origin));
        }
    }
}

/// Most bullet impacts voiced per frame (a shotgun blast is one crack, not nine).
const MAX_IMPACTS_PER_FRAME: usize = 3;

/// The player's bullets: an impact (or the odd ricochet) where each one actually hits the world
/// (`Impact`, not where a tracer happens to end: misses into the sky are silent). Flesh hits
/// are voiced by the hit marker instead.
fn impact_sounds(mut commands: Commands, mut impacts: MessageReader<Impact>, sfx: Res<Sfx>, master: Res<Volumes>) {
    let mut rng = rand::rng();
    for i in impacts.read().filter(|i| !i.flesh && !i.exit).take(MAX_IMPACTS_PER_FRAME) {
        let name = if rng.random_bool(0.18) { "ricochet" } else { "impact" };
        play(&mut commands, &sfx, &master, name, 0.55, Some(i.pos));
    }
}

/// What an enemy bullet sounds like to the player, from its tracer (`EnemyFx::Shot`; enemies
/// never write `Tracer`). The tracer is cut 1.5 m short of the player's eye when it hit him and
/// 2.5 m short on a close miss.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BulletHeard {
    /// It hit him (the hurt sound covers it).
    Hit,
    /// A near miss: a crack / whiz past his head, at this point.
    Whiz(Vec3),
    /// It struck the world here.
    Impact(Vec3),
    /// Nothing worth a sound (out of earshot, or spent at max range).
    Nothing,
}

/// Classify an enemy tracer `from` -> `to` for a listener at `eye`.
pub(crate) fn bullet_heard(from: Vec3, to: Vec3, eye: Vec3) -> BulletHeard {
    let end_d = to.distance(eye);
    if end_d < 1.6 {
        return BulletHeard::Hit;
    }
    let seg = to - from;
    let len = seg.length();
    if len < 1e-3 {
        return BulletHeard::Nothing;
    }
    let dir = seg / len;
    // Closest approach of the bullet's line (it flies on past a clipped tracer end).
    let along = (eye - from).dot(dir);
    let closest = from + dir * along.max(0.0);
    if along > 1.5 && closest.distance(eye) < 2.6 {
        return BulletHeard::Whiz(closest);
    }
    // Tracers that ran their full length (59+ m) hit nothing.
    if len < 59.0 && end_d < 80.0 { BulletHeard::Impact(to) } else { BulletHeard::Nothing }
}

/// Enemy bullets: whiz past the player's head, impacts where they strike.
fn enemy_bullet_sounds(
    mut commands: Commands,
    mut fx: MessageReader<EnemyFx>,
    sfx: Res<Sfx>,
    master: Res<Volumes>,
    player: Query<&Transform, With<Player>>,
) {
    let Ok(ptf) = player.single() else { return };
    let eye = ptf.translation + Vec3::Y * 0.6;
    let mut impacts = 0;
    for ev in fx.read() {
        let EnemyFx::Shot { from, to: Some(to), .. } = *ev else { continue };
        match bullet_heard(from, to, eye) {
            BulletHeard::Whiz(at) => {
                let name = if sfx.has("bullet_crack") { "bullet_crack" } else { "ricochet" };
                play(&mut commands, &sfx, &master, name, 0.8, Some(at));
            }
            BulletHeard::Impact(at) if impacts < MAX_IMPACTS_PER_FRAME => {
                impacts += 1;
                play(&mut commands, &sfx, &master, "impact", 0.5, Some(at));
            }
            _ => {}
        }
    }
}

fn damage_sounds(
    mut commands: Commands,
    mut dmg: MessageReader<Damage>,
    sfx: Res<Sfx>,
    master: Res<Volumes>,
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

fn player_footsteps(
    mut commands: Commands,
    time: Res<Time>,
    sfx: Res<Sfx>,
    master: Res<Volumes>,
    player: Query<&FpsMotor, With<Player>>,
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
    master: Res<Volumes>,
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
    master: Res<Volumes>,
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
    master: Res<Volumes>,
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

/// A soft radio click under each squad callout (silent if the sound file is missing).
fn radio_clicks(mut commands: Commands, sfx: Res<Sfx>, master: Res<Volumes>, mut msgs: MessageReader<crate::enemy::Callout>) {
    if msgs.read().count() > 0 {
        play(&mut commands, &sfx, &master, "radio_click", 0.35, None);
    }
}

/// The bomb, heard where it is: beeps (louder as the fuse runs out), the defuse starting (the
/// player hears a defender on it, as in CS) and the blast. The keypad while planting is his own.
fn bomb_sounds(mut commands: Commands, sfx: Res<Sfx>, master: Res<Volumes>, mut events: MessageReader<crate::bomb::BombEvent>) {
    use crate::bomb::BombEvent as B;
    for ev in events.read() {
        match *ev {
            B::PlantStart => play(&mut commands, &sfx, &master, "bomb_plant", 0.7, None),
            B::Planted { at, .. } => play(&mut commands, &sfx, &master, "bomb_beep", 1.0, Some(at)),
            B::Beep { at, urgency } => play(&mut commands, &sfx, &master, "bomb_beep", 0.55 + 0.4 * urgency, Some(at)),
            B::DefuseStart { at } | B::Defused { at } => play(&mut commands, &sfx, &master, "bomb_defuse", 1.0, Some(at)),
            B::Exploded { at } => play(&mut commands, &sfx, &master, "bomb_explode", 1.0, Some(at)),
        }
    }
}

/// Groups that may legitimately have no files (silent until someone adds them).
#[cfg(test)]
pub(crate) const OPTIONAL_SOUNDS: &[&str] = &["radio_click", "frag_explode", "flashbang"];

/// The files `SoundLibrary::load_variants` looks for: `<name>.ogg`, or `<name>_<i>.ogg`.
#[cfg(test)]
pub(crate) fn sound_files(name: &str, n: usize) -> Vec<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/audio");
    if n <= 1 { vec![dir.join(format!("{name}.ogg"))] } else { (0..n).map(|i| dir.join(format!("{name}_{i}.ogg"))).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_required_sound_has_its_files() {
        // A missing or misnamed file is silent at runtime (`load_variants` skips it), so check here.
        for &(name, n) in SOUNDS {
            if OPTIONAL_SOUNDS.contains(&name) {
                continue;
            }
            for f in sound_files(name, n) {
                assert!(f.is_file(), "{name}: missing {}", f.display());
                assert!(std::fs::metadata(&f).unwrap().len() > 1000, "{name}: {} looks empty", f.display());
            }
        }
        for name in ["step", "impact", "ricochet", "enemy_shot", "hit_marker", "player_hurt"] {
            assert!(SOUNDS.iter().any(|s| s.0 == name), "{name} is triggered but never loaded");
        }
    }

    #[test]
    fn enemy_bullets_whiz_past_and_strike_the_world() {
        let eye = Vec3::new(0.0, 1.6, 0.0);
        // A hit: the tracer stops 1.5 m short of his eye.
        let from = Vec3::new(0.0, 1.6, -30.0);
        assert_eq!(bullet_heard(from, eye + Vec3::new(0.0, 0.0, -1.5), eye), BulletHeard::Hit);
        // A near miss clipped 2.5 m short: the bullet flies on past his head.
        let miss_to = Vec3::new(1.0, 1.6, -2.3);
        assert!(matches!(bullet_heard(Vec3::new(1.0, 1.6, -30.0), miss_to, eye), BulletHeard::Whiz(p) if p.distance(eye) < 1.1));
        // Into a wall 20 m to his side.
        let wall = Vec3::new(20.0, 1.0, -10.0);
        assert_eq!(bullet_heard(Vec3::new(20.0, 1.5, -30.0), wall, eye), BulletHeard::Impact(wall));
        // Full-length tracer into the sky: nothing to hear.
        assert_eq!(bullet_heard(Vec3::new(30.0, 1.5, -30.0), Vec3::new(30.0, 40.0, 20.0), eye), BulletHeard::Nothing);
    }
}
