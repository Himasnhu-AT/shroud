//! The built-in control commands (`game.*`, `time.*`, `player.*`, `capture.*`, `debug.*`,
//! `perf.*`). Enemy, AI and match-flow modules register their own.
//!
//! Player commands go through the same path as a human: `player.move` and `player.fire` write
//! [`PlayerIntent`] (with [`ExternalControl`] on) between `PlayerSet::Input` and `PlayerSet::Move`.

use std::collections::VecDeque;
use std::time::Duration;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use serde_json::{Value, json};

use super::control::{ControlAppExt, PendingReplies, ReplyId, params};
use super::{DebugOverlay, GameSeed, data_dir};
use crate::player::{ExternalControl, Player, PlayerCamera, PlayerIntent, PlayerLook, PlayerMotor, PlayerSet};
use crate::state::{GameState, Headless};
use crate::weapon::{Damage, Health, RELOAD_TIME, Rifle, ShotFired, WeaponSet};

const MAX_PITCH: f32 = 1.54;
/// `player.fire` gives up after this long (e.g. no ammo and reloading forever).
const FIRE_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest `player.move` (the control port times a reply out after 60 s).
const MAX_MOVE_MS: u64 = 50_000;

/// Ordering for the systems that drive the player.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ControlDrive;

#[derive(Resource, Default)]
struct MoveJob(Option<Move>);

struct Move {
    reply: ReplyId,
    axis: Vec2,
    sprint: bool,
    crouch: Option<bool>,
    left: Duration,
}

#[derive(Resource, Default)]
struct FireJob(Option<Fire>);

struct Fire {
    reply: ReplyId,
    shots: u32,
    fired: u32,
    hits: u32,
    damage: f32,
    elapsed: Duration,
}

#[derive(Resource, Default)]
struct StepJob(Option<Step>);

struct Step {
    reply: ReplyId,
    left: u32,
    started: u32,
}

/// `player.crouch` holds crouch until turned off.
#[derive(Resource, Default)]
struct HeldCrouch(bool);

/// `player.god`: the player's health is refilled every frame.
#[derive(Resource, Default)]
pub struct GodMode(pub bool);

/// Frames left before `game.quit` exits (so the reply reaches the client first).
#[derive(Resource, Default)]
struct QuitIn(Option<u32>);

/// Recent real frame times, for `perf.stats`.
#[derive(Resource, Default)]
pub struct FrameStats {
    times: VecDeque<f32>,
}

impl FrameStats {
    const LEN: usize = 600;

    /// `(fps, p50, p95, p99)` in milliseconds over the last ~10 s.
    pub fn summary(&self) -> (f32, f32, f32, f32) {
        if self.times.is_empty() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let mut v: Vec<f32> = self.times.iter().copied().collect();
        v.sort_by(f32::total_cmp);
        let pct = |p: f32| v[((v.len() - 1) as f32 * p).round() as usize] * 1000.0;
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let fps = if mean > 0.0 { 1.0 / mean } else { 0.0 };
        (fps, pct(0.5), pct(0.95), pct(0.99))
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<MoveJob>()
        .init_resource::<FireJob>()
        .init_resource::<StepJob>()
        .init_resource::<HeldCrouch>()
        .init_resource::<GodMode>()
        .init_resource::<QuitIn>()
        .init_resource::<FrameStats>()
        .init_resource::<DebugOverlay>()
        .configure_sets(Update, ControlDrive.after(PlayerSet::Input).before(PlayerSet::Move))
        .add_systems(
            Update,
            (
                drive_player.in_set(ControlDrive),
                (observe_fire, god_mode).after(WeaponSet),
                quit_countdown,
            ),
        )
        .add_systems(Last, (step_frames, record_frame));

    let none = params(&[]);
    app.add_control_command("game.state", "Game state", none.clone(), game_state)
        .add_control_command("game.start", "Start playing", none.clone(), game_start)
        .add_control_command("game.pause", "Pause game time", none.clone(), game_pause)
        .add_control_command("game.resume", "Resume game time", none.clone(), game_resume)
        .add_control_command("game.quit", "Quit the game", none.clone(), game_quit)
        .add_control_command(
            "time.scale",
            "Set game speed",
            params(&[("factor", "number", "0..8, 1 = normal")]),
            time_scale,
        )
        .add_deferred_control_command(
            "time.step",
            "Run N frames while paused",
            params(&[("frames", "integer", "frames to run (default 1)")]),
            time_step,
        )
        .add_control_command(
            "player.teleport",
            "Teleport the player",
            params(&[
                ("x", "number", "world x"),
                ("z", "number", "world z"),
                ("y", "number", "capsule centre height (default: unchanged)"),
                ("yaw", "number", "radians, 0 = facing -Z"),
            ]),
            player_teleport,
        )
        .add_control_command(
            "player.look",
            "Aim the player",
            params(&[
                ("yaw", "number", "radians, 0 = facing -Z"),
                ("pitch", "number", "radians, + = up"),
                ("at", "array", "[x, y, z] world point to look at (instead of yaw/pitch)"),
            ]),
            player_look,
        )
        .add_deferred_control_command(
            "player.move",
            "Move the player for a while",
            params(&[
                ("forward", "number", "-1..1"),
                ("right", "number", "-1..1"),
                ("ms", "integer", "duration in game milliseconds (default 500)"),
                ("sprint", "boolean", "sprint"),
                ("crouch", "boolean", "crouch while moving"),
            ]),
            player_move,
        )
        .add_deferred_control_command(
            "player.fire",
            "Fire the rifle",
            params(&[("shots", "integer", "shots to fire (default 1)")]),
            player_fire,
        )
        .add_control_command("player.reload", "Reload", none.clone(), player_reload)
        .add_control_command(
            "player.crouch",
            "Hold or release crouch",
            params(&[("on", "boolean", "default true")]),
            player_crouch,
        )
        .add_control_command(
            "player.god",
            "God mode",
            params(&[("on", "boolean", "default true")]),
            player_god,
        )
        .add_deferred_control_command(
            "capture.screenshot",
            "Save a screenshot (windowed only)",
            params(&[("path", "string", "PNG path (default: data dir)")]),
            capture_screenshot,
        )
        .add_control_command(
            "debug.overlay",
            "Debug overlay",
            params(&[("on", "boolean", "default true")]),
            debug_overlay,
        )
        .add_control_command("perf.stats", "Frame-time statistics", none, perf_stats);
}

// ---- params ----

fn num(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64)
}

fn flag(p: &Value, k: &str) -> Option<bool> {
    p.get(k).and_then(Value::as_bool)
}

fn need(p: &Value, k: &str) -> Result<f64, String> {
    num(p, k).ok_or_else(|| format!("`{k}` (number) is required"))
}

fn v3(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

fn player_entity(world: &mut World) -> Result<Entity, String> {
    world
        .query_filtered::<Entity, With<Player>>()
        .iter(world)
        .next()
        .ok_or_else(|| "no player".to_string())
}

fn playing(world: &World) -> bool {
    world
        .get_resource::<State<GameState>>()
        .is_some_and(|s| *s.get() == GameState::Playing)
}

// ---- game / time ----

fn game_state(world: &mut World, _: Value) -> Result<Value, String> {
    let state = world
        .get_resource::<State<GameState>>()
        .map(|s| format!("{:?}", s.get()));
    let player = world
        .query_filtered::<(&Transform, Option<&Health>, Option<&Rifle>, &PlayerLook, &PlayerMotor), With<Player>>()
        .iter(world)
        .next()
        .map(|(tf, hp, rifle, look, motor)| {
            json!({
                "pos": v3(tf.translation),
                "hp": hp.map(|h| h.hp),
                "ammo": rifle.map(|r| r.ammo),
                "reloading": rifle.is_some_and(|r| r.reloading.is_some()),
                "yaw": look.yaw,
                "pitch": look.pitch,
                "crouched": motor.crouched,
                "grounded": motor.grounded,
            })
        });
    let time = world.resource::<Time<Virtual>>();
    Ok(json!({
        "state": state,
        "paused": time.is_paused(),
        "time_scale": time.relative_speed(),
        "frame": world.get_resource::<FrameCount>().map(|f| f.0),
        "seed": world.get_resource::<GameSeed>().map(|s| s.0),
        "player": player,
    }))
}

fn game_start(world: &mut World, _: Value) -> Result<Value, String> {
    world.resource_mut::<NextState<GameState>>().set(GameState::Playing);
    world.resource_mut::<Time<Virtual>>().unpause();
    Ok(json!({}))
}

fn game_pause(world: &mut World, _: Value) -> Result<Value, String> {
    world.resource_mut::<Time<Virtual>>().pause();
    Ok(json!({}))
}

fn game_resume(world: &mut World, _: Value) -> Result<Value, String> {
    world.resource_mut::<Time<Virtual>>().unpause();
    Ok(json!({}))
}

fn game_quit(world: &mut World, _: Value) -> Result<Value, String> {
    world.resource_mut::<QuitIn>().0 = Some(3);
    Ok(json!({}))
}

fn quit_countdown(mut quit: ResMut<QuitIn>, mut exit: MessageWriter<AppExit>) {
    if let Some(n) = quit.0 {
        if n == 0 {
            exit.write(AppExit::Success);
            quit.0 = None;
        } else {
            quit.0 = Some(n - 1);
        }
    }
}

fn time_scale(world: &mut World, p: Value) -> Result<Value, String> {
    let f = need(&p, "factor")?;
    if !(0.0..=8.0).contains(&f) {
        return Err("`factor` must be in 0..8".into());
    }
    world.resource_mut::<Time<Virtual>>().set_relative_speed_f64(f);
    Ok(json!({}))
}

fn time_step(world: &mut World, p: Value, reply: ReplyId) -> Result<(), String> {
    let frames = p.get("frames").and_then(Value::as_u64).unwrap_or(1).clamp(1, 100_000) as u32;
    if !world.resource::<Time<Virtual>>().is_paused() {
        return Err("time.step needs a paused game (game.pause first)".into());
    }
    if world.resource::<StepJob>().0.is_some() {
        return Err("a time.step is already running".into());
    }
    world.resource_mut::<Time<Virtual>>().unpause();
    let started = world.get_resource::<FrameCount>().map_or(0, |f| f.0);
    world.resource_mut::<StepJob>().0 = Some(Step { reply, left: frames, started });
    Ok(())
}

/// Last: count stepped frames (from the next frame on: this frame's virtual delta was 0) and
/// pause again when done.
fn step_frames(
    mut job: ResMut<StepJob>,
    frame: Res<FrameCount>,
    mut time: ResMut<Time<Virtual>>,
    mut pending: ResMut<PendingReplies>,
) {
    let Some(step) = job.0.as_mut() else { return };
    if frame.0 == step.started {
        return;
    }
    step.left -= 1;
    if step.left == 0 {
        time.pause();
        let reply = step.reply;
        job.0 = None;
        pending.reply(reply, Ok(json!({"frame": frame.0})));
    }
}

fn record_frame(time: Res<Time<Real>>, mut stats: ResMut<FrameStats>) {
    let dt = time.delta_secs();
    if dt > 0.0 {
        if stats.times.len() == FrameStats::LEN {
            stats.times.pop_front();
        }
        stats.times.push_back(dt);
    }
}

// ---- player ----

fn player_teleport(world: &mut World, p: Value) -> Result<Value, String> {
    let (x, z) = (need(&p, "x")? as f32, need(&p, "z")? as f32);
    let e = player_entity(world)?;
    let mut ent = world.entity_mut(e);
    if let Some(mut tf) = ent.get_mut::<Transform>() {
        tf.translation.x = x;
        tf.translation.z = z;
        if let Some(y) = num(&p, "y") {
            tf.translation.y = y as f32;
        }
    }
    if let Some(mut motor) = ent.get_mut::<PlayerMotor>() {
        motor.velocity = Vec3::ZERO;
    }
    if let Some(yaw) = num(&p, "yaw")
        && let Some(mut look) = ent.get_mut::<PlayerLook>()
    {
        look.yaw = yaw as f32;
    }
    Ok(json!({}))
}

fn player_look(world: &mut World, p: Value) -> Result<Value, String> {
    let e = player_entity(world)?;
    let (yaw, pitch) = if let Some(at) = p.get("at").and_then(Value::as_array) {
        let c: Vec<f32> = at.iter().filter_map(Value::as_f64).map(|v| v as f32).collect();
        let [x, y, z] = c[..] else {
            return Err("`at` must be [x, y, z]".into());
        };
        let body = world.get::<Transform>(e).map_or(Vec3::ZERO, |t| t.translation);
        let eye_local = world
            .query_filtered::<&Transform, With<PlayerCamera>>()
            .iter(world)
            .next()
            .map_or(Vec3::Y * 0.8, |t| t.translation);
        let d = Vec3::new(x, y, z) - (body + eye_local);
        if d.length_squared() < 1e-6 {
            return Err("`at` is the player's eye".into());
        }
        let yaw = (-d.x).atan2(-d.z);
        let pitch = d.y.atan2(Vec2::new(d.x, d.z).length());
        (yaw, pitch)
    } else {
        let look = world.get::<PlayerLook>(e).ok_or("no PlayerLook")?;
        (
            num(&p, "yaw").map_or(look.yaw, |v| v as f32),
            num(&p, "pitch").map_or(look.pitch, |v| v as f32),
        )
    };
    let mut look = world.get_mut::<PlayerLook>(e).ok_or("no PlayerLook")?;
    look.yaw = yaw;
    look.pitch = pitch.clamp(-MAX_PITCH, MAX_PITCH);
    Ok(json!({"yaw": look.yaw, "pitch": look.pitch}))
}

fn player_move(world: &mut World, p: Value, reply: ReplyId) -> Result<(), String> {
    if !playing(world) {
        return Err("not playing (game.start first)".into());
    }
    let axis = Vec2::new(
        num(&p, "right").unwrap_or(0.0) as f32,
        num(&p, "forward").unwrap_or(0.0) as f32,
    )
    .clamp_length_max(1.0);
    let ms = p.get("ms").and_then(Value::as_u64).unwrap_or(500).min(MAX_MOVE_MS);
    let job = Move {
        reply,
        axis,
        sprint: flag(&p, "sprint").unwrap_or(false),
        crouch: flag(&p, "crouch"),
        left: Duration::from_millis(ms),
    };
    if let Some(old) = world.resource_mut::<MoveJob>().0.replace(job) {
        world
            .resource_mut::<PendingReplies>()
            .reply(old.reply, Err("superseded by a newer player.move".into()));
    }
    Ok(())
}

fn player_fire(world: &mut World, p: Value, reply: ReplyId) -> Result<(), String> {
    if !playing(world) {
        return Err("not playing (game.start first)".into());
    }
    let shots = p.get("shots").and_then(Value::as_u64).unwrap_or(1).clamp(1, 1000) as u32;
    let job = Fire { reply, shots, fired: 0, hits: 0, damage: 0.0, elapsed: Duration::ZERO };
    if let Some(old) = world.resource_mut::<FireJob>().0.replace(job) {
        world
            .resource_mut::<PendingReplies>()
            .reply(old.reply, Err("superseded by a newer player.fire".into()));
    }
    Ok(())
}

/// Between `PlayerSet::Input` and `PlayerSet::Move`: write the intent for running jobs and
/// answer finished ones.
fn drive_player(
    time: Res<Time>,
    real: Res<Time<Real>>,
    state: Res<State<GameState>>,
    held: Res<HeldCrouch>,
    mut moves: ResMut<MoveJob>,
    mut fires: ResMut<FireJob>,
    mut intent: ResMut<PlayerIntent>,
    mut external: ResMut<ExternalControl>,
    mut pending: ResMut<PendingReplies>,
    players: Query<&Transform, With<Player>>,
) {
    let is_playing = *state.get() == GameState::Playing;
    let pos = players.single().map(|t| v3(t.translation)).unwrap_or(Value::Null);

    if let Some(m) = moves.0.as_mut() {
        if !is_playing {
            pending.reply(m.reply, Err("left Playing before the move finished".into()));
            moves.0 = None;
            intent.move_axis = Vec2::ZERO;
        } else if m.left.is_zero() {
            pending.reply(m.reply, Ok(json!({"pos": pos})));
            moves.0 = None;
            intent.move_axis = Vec2::ZERO;
            intent.sprint = false;
            if !held.0 {
                intent.crouch = false;
            }
        } else {
            intent.move_axis = m.axis;
            intent.sprint = m.sprint;
            if let Some(c) = m.crouch {
                intent.crouch = c;
            }
            m.left = m.left.saturating_sub(time.delta());
        }
    }

    if let Some(f) = fires.0.as_mut() {
        f.elapsed += real.delta();
        let done = f.fired >= f.shots || f.elapsed > FIRE_TIMEOUT || !is_playing;
        if done {
            let r = json!({"shots": f.fired, "hits": f.hits, "damage": f.damage});
            pending.reply(f.reply, Ok(r));
            fires.0 = None;
            intent.fire = false;
        } else {
            intent.fire = true;
        }
    }

    if held.0 {
        intent.crouch = true;
    }
    let driving = moves.0.is_some() || fires.0.is_some() || held.0;
    if external.0 != driving {
        external.0 = driving;
    }
}

/// After the weapon systems: count the player's shots, hits and damage for `player.fire`.
fn observe_fire(
    mut fires: ResMut<FireJob>,
    mut shots: MessageReader<ShotFired>,
    mut damage: MessageReader<Damage>,
    players: Query<Entity, With<Player>>,
) {
    let Ok(player) = players.single() else {
        shots.clear();
        damage.clear();
        return;
    };
    let Some(f) = fires.0.as_mut() else {
        shots.clear();
        damage.clear();
        return;
    };
    f.fired += shots.read().filter(|s| s.by == player).count() as u32;
    for d in damage.read().filter(|d| d.source == player) {
        f.hits += 1;
        f.damage += d.amount;
    }
}

fn god_mode(god: Res<GodMode>, mut players: Query<&mut Health, With<Player>>) {
    if god.0 {
        for mut h in &mut players {
            h.hp = h.max;
        }
    }
}

fn player_reload(world: &mut World, _: Value) -> Result<Value, String> {
    let e = player_entity(world)?;
    let mut rifle = world.get_mut::<Rifle>(e).ok_or("no rifle")?;
    if rifle.reloading.is_none() && rifle.ammo < rifle.mag {
        rifle.reloading = Some(RELOAD_TIME);
    }
    Ok(json!({"ammo": rifle.ammo, "reloading": rifle.reloading.is_some()}))
}

fn player_crouch(world: &mut World, p: Value) -> Result<Value, String> {
    let on = flag(&p, "on").unwrap_or(true);
    world.resource_mut::<HeldCrouch>().0 = on;
    world.resource_mut::<PlayerIntent>().crouch = on;
    Ok(json!({}))
}

fn player_god(world: &mut World, p: Value) -> Result<Value, String> {
    world.resource_mut::<GodMode>().0 = flag(&p, "on").unwrap_or(true);
    Ok(json!({}))
}

// ---- capture / debug / perf ----

fn capture_screenshot(world: &mut World, p: Value, reply: ReplyId) -> Result<(), String> {
    use bevy::render::view::window::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
    if world.resource::<Headless>().0 {
        return Err("capture.screenshot needs a window (not --headless)".into());
    }
    let path = match p.get("path").and_then(Value::as_str) {
        Some(s) => std::path::PathBuf::from(s),
        None => {
            let frame = world.get_resource::<FrameCount>().map_or(0, |f| f.0);
            data_dir().join("screenshots").join(format!("shot-{frame}.png"))
        }
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let shown = path.display().to_string();
    let shot = match world.get_resource::<super::offscreen::OffscreenTarget>() {
        Some(t) => Screenshot::image(t.0.clone()),
        None => Screenshot::primary_window(),
    };
    world
        .spawn(shot)
        .observe(save_to_disk(path))
        .observe(move |_: On<ScreenshotCaptured>, mut pending: ResMut<PendingReplies>| {
            pending.reply(reply, Ok(json!({"path": shown})));
        });
    Ok(())
}

fn debug_overlay(world: &mut World, p: Value) -> Result<Value, String> {
    world.resource_mut::<DebugOverlay>().0 = flag(&p, "on").unwrap_or(true);
    Ok(json!({}))
}

fn perf_stats(world: &mut World, _: Value) -> Result<Value, String> {
    let (fps, p50, p95, p99) = world.resource::<FrameStats>().summary();
    Ok(json!({
        "fps": fps,
        "frame_ms": {"p50": p50, "p95": p95, "p99": p99},
        "entities": world.entities().count_spawned(),
    }))
}
