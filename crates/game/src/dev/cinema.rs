//! Trailer / footage tooling (dev only): fixed-step frame capture, a cinematic camera, HUD
//! toggle and smooth player aim, all on the control port.
//!
//! | command | params | notes |
//! |---|---|---|
//! | `capture.frames` | `{dir, fps?=60, seconds? \| frames?}` | from the next frame on, game time advances exactly 1/fps per frame and every frame is saved as `dir/f_00000.png` (offscreen render target). Replies at once; poll `capture.status`. |
//! | `capture.status` | — | `{active, frame, total, saved, dir}`; `frame` = frames rendered so far (game time = frame / fps) |
//! | `capture.stop` | — | ends a capture early |
//! | `camera.path` | `{from:{pos,look}, to?:{pos,look}, seconds?, ease?, fov?}` | flies the main camera (eased) and holds the end pose; viewmodel hidden. `look` = world point. |
//! | `camera.release` | — | camera back on the player |
//! | `hud.visible` | `{on}` | hides / shows every root UI node |
//! | `player.aim` | `{at:[x,y,z], seconds?}` | eases the player's look to a point |
//! | `player.track` | `{on, enemy?, rate?}` | keeps easing the look onto an enemy's chest (nearest alive, or slot `enemy`) |
//!
//! Capture holds the quality governor and idle frame skip off so every frame renders at the
//! same quality. PNGs are encoded on the IO pool; the main thread waits if more than a few
//! frames are queued.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::window::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use engine::devtools::offscreen::OffscreenTarget;
use engine::devtools::{AddControlCommand, params};
use engine::fps::FpsLook;
use engine::gfx::MainCamera;
use engine::render_budget::{QualityGovernor, RenderBudget};
use serde_json::{Value, json};

use crate::enemy::{Enemy, EnemyCombat};
use crate::player::Player;
use crate::weapon::viewmodel::ViewRoot;

/// Frames that may wait for PNG encoding before the main thread blocks.
const MAX_QUEUED: u32 = 6;
const MAX_PITCH: f32 = 1.54;

// ---- capture ----

#[derive(Resource, Default)]
struct Capture {
    job: Option<Job>,
    /// The last job's numbers (kept after it ends so `capture.status` can report it).
    last: Option<Value>,
}

struct Job {
    dir: PathBuf,
    fps: f64,
    total: u32,
    /// Frames handed to the renderer so far.
    next: u32,
    armed: bool,
    saved: Arc<AtomicU32>,
    failed: Arc<AtomicU32>,
    governor_was: Option<bool>,
    idle_skip_was: Option<bool>,
}

impl Job {
    fn status(&self, active: bool) -> Value {
        json!({
            "active": active,
            "frame": self.next,
            "total": self.total,
            "fps": self.fps,
            "saved": self.saved.load(Ordering::Relaxed),
            "failed": self.failed.load(Ordering::Relaxed),
            "dir": self.dir.display().to_string(),
        })
    }
}

fn capture_frames(world: &mut World, p: Value) -> Result<Value, String> {
    if world.get_resource::<OffscreenTarget>().is_none() {
        return Err("capture.frames needs --offscreen".into());
    }
    if world.resource::<Capture>().job.is_some() {
        return Err("a capture is already running (capture.stop)".into());
    }
    let dir = PathBuf::from(p.get("dir").and_then(Value::as_str).ok_or("`dir` (string) is required")?);
    let fps = p.get("fps").and_then(Value::as_f64).unwrap_or(60.0).clamp(1.0, 240.0);
    let total = match (p.get("frames").and_then(Value::as_u64), p.get("seconds").and_then(Value::as_f64)) {
        (Some(n), _) => n as u32,
        (None, Some(s)) => (s * fps).round() as u32,
        _ => return Err("`seconds` or `frames` is required".into()),
    }
    .clamp(1, 100_000);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let governor_was = world.get_resource_mut::<QualityGovernor>().map(|mut g| std::mem::replace(&mut g.enabled, false));
    let idle_skip_was = world.get_resource_mut::<RenderBudget>().map(|mut b| std::mem::replace(&mut b.idle_skip, false));
    world.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / fps)));
    let job = Job {
        dir,
        fps,
        total,
        next: 0,
        armed: false,
        saved: default(),
        failed: default(),
        governor_was,
        idle_skip_was,
    };
    let out = job.status(true);
    world.resource_mut::<Capture>().job = Some(job);
    Ok(out)
}

fn end_capture(world: &mut World) -> Option<Value> {
    let job = world.resource_mut::<Capture>().job.take()?;
    world.insert_resource(TimeUpdateStrategy::Automatic);
    if let (Some(was), Some(mut g)) = (job.governor_was, world.get_resource_mut::<QualityGovernor>()) {
        g.enabled = was;
    }
    if let (Some(was), Some(mut b)) = (job.idle_skip_was, world.get_resource_mut::<RenderBudget>()) {
        b.idle_skip = was;
    }
    // Keep counting saves that finish after the job ended.
    world.resource_mut::<Capture>().last = Some(job.status(false));
    world.insert_resource(LastSaved(job.saved.clone(), job.failed.clone()));
    Some(job.status(false))
}

#[derive(Resource)]
struct LastSaved(Arc<AtomicU32>, Arc<AtomicU32>);

fn capture_status(world: &mut World, _: Value) -> Result<Value, String> {
    if let Some(job) = &world.resource::<Capture>().job {
        return Ok(job.status(true));
    }
    let mut last = world.resource::<Capture>().last.clone().unwrap_or(json!({"active": false}));
    if let Some(s) = world.get_resource::<LastSaved>() {
        last["saved"] = json!(s.0.load(Ordering::Relaxed));
        last["failed"] = json!(s.1.load(Ordering::Relaxed));
    }
    Ok(last)
}

fn capture_stop(world: &mut World, _: Value) -> Result<Value, String> {
    end_capture(world).ok_or_else(|| "no capture running".to_string())
}

/// `Last`: hand this frame to the screenshot path (the render world reads it back after it
/// renders this frame), then the next frame. Arms one frame late so every captured frame had
/// the fixed time step.
fn capture_frame(world: &mut World) {
    let Some(target) = world.get_resource::<OffscreenTarget>().map(|t| t.0.clone()) else { return };
    let (path, saved, failed) = {
        let mut cap = world.resource_mut::<Capture>();
        let Some(job) = cap.job.as_mut() else { return };
        if !job.armed {
            job.armed = true;
            return;
        }
        if job.next >= job.total {
            drop(cap);
            end_capture(world);
            return;
        }
        // Back-pressure: don't let the PNG encoder fall far behind (8 MB per frame).
        let t0 = std::time::Instant::now();
        while job.next - job.saved.load(Ordering::Relaxed) - job.failed.load(Ordering::Relaxed) > MAX_QUEUED
            && t0.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        let path = job.dir.join(format!("f_{:05}.png", job.next));
        job.next += 1;
        (path, job.saved.clone(), job.failed.clone())
    };
    world.spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
        let img = shot.image.clone();
        let (path, saved, failed) = (path.clone(), saved.clone(), failed.clone());
        bevy::tasks::IoTaskPool::get()
            .spawn(async move {
                let ok = img.try_into_dynamic().ok().map(|d| d.to_rgb8().save(&path));
                match ok {
                    Some(Ok(())) => saved.fetch_add(1, Ordering::Relaxed),
                    _ => failed.fetch_add(1, Ordering::Relaxed),
                };
            })
            .detach();
    });
}

// ---- cinematic camera ----

#[derive(Clone, Copy)]
struct Pose {
    pos: Vec3,
    look: Vec3,
}

#[derive(Clone, Copy, PartialEq)]
enum Ease {
    Linear,
    InOut,
    Out,
    In,
}

impl Ease {
    fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            // Smootherstep: zero velocity and acceleration at both ends.
            Ease::InOut => t * t * t * (t * (t * 6.0 - 15.0) + 10.0),
            Ease::Out => 1.0 - (1.0 - t).powi(3),
            Ease::In => t * t * t,
        }
    }
}

#[derive(Resource, Default)]
struct CineCam(Option<Cine>);

struct Cine {
    from: Pose,
    to: Pose,
    seconds: f32,
    t: f32,
    ease: Ease,
    fov: Option<(f32, f32)>,
    /// The camera's FOV before the path (restored on release).
    fov_was: Option<f32>,
}

fn v3(v: &Value) -> Option<Vec3> {
    let a = v.as_array()?;
    let c: Vec<f32> = a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect();
    match c[..] {
        [x, y, z] => Some(Vec3::new(x, y, z)),
        _ => None,
    }
}

fn pose(v: Option<&Value>, name: &str) -> Result<Pose, String> {
    let v = v.ok_or_else(|| format!("`{name}` is required"))?;
    let pos = v.get("pos").and_then(v3).ok_or_else(|| format!("`{name}.pos` must be [x, y, z]"))?;
    let look = v.get("look").and_then(v3).ok_or_else(|| format!("`{name}.look` must be [x, y, z]"))?;
    if pos.distance_squared(look) < 1e-4 {
        return Err(format!("`{name}.look` is `{name}.pos`"));
    }
    Ok(Pose { pos, look })
}

fn camera_path(world: &mut World, p: Value) -> Result<Value, String> {
    let from = pose(p.get("from"), "from")?;
    let to = if p.get("to").is_some() { pose(p.get("to"), "to")? } else { from };
    let seconds = p.get("seconds").and_then(Value::as_f64).unwrap_or(4.0).max(0.01) as f32;
    let ease = match p.get("ease").and_then(Value::as_str).unwrap_or("inout") {
        "linear" => Ease::Linear,
        "out" => Ease::Out,
        "in" => Ease::In,
        _ => Ease::InOut,
    };
    let fov = match p.get("fov") {
        Some(Value::Array(a)) => {
            let f: Vec<f32> = a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect();
            f.first().map(|&a0| (a0, *f.get(1).unwrap_or(&a0)))
        }
        Some(v) => v.as_f64().map(|f| (f as f32, f as f32)),
        None => None,
    };
    let fov_was = world.resource::<CineCam>().0.as_ref().and_then(|c| c.fov_was).or_else(|| {
        world.query_filtered::<&Projection, With<MainCamera>>().iter(world).next().and_then(|p| match p {
            Projection::Perspective(pp) => Some(pp.fov),
            _ => None,
        })
    });
    world.resource_mut::<CineCam>().0 = Some(Cine { from, to, seconds, t: 0.0, ease, fov, fov_was });
    Ok(json!({"seconds": seconds}))
}

fn camera_release(world: &mut World, _: Value) -> Result<Value, String> {
    let Some(c) = world.resource_mut::<CineCam>().0.take() else { return Ok(json!({"was_active": false})) };
    if let Some(fov) = c.fov_was {
        for mut p in world.query_filtered::<&mut Projection, With<MainCamera>>().iter_mut(world) {
            if let Projection::Perspective(pp) = &mut *p {
                pp.fov = fov;
            }
        }
    }
    // The controller rewrites the camera's local pose every frame it runs; between rounds put
    // it back at the eye ourselves.
    let eye_y = world
        .query_filtered::<&engine::fps::FpsMotor, With<Player>>()
        .iter(world)
        .next()
        .map(|m| m.eye_offset);
    for mut tf in world.query_filtered::<&mut Transform, With<MainCamera>>().iter_mut(world) {
        if let Some(y) = eye_y {
            *tf = Transform::from_xyz(0.0, y, 0.0);
        }
    }
    for mut v in world.query_filtered::<&mut Visibility, With<ViewRoot>>().iter_mut(world) {
        *v = Visibility::Inherited;
    }
    Ok(json!({"was_active": true}))
}

/// `PostUpdate`, before transform propagation: place the camera (a child of the player) so its
/// world pose is on the path.
fn fly_camera(
    time: Res<Time>,
    mut cine: ResMut<CineCam>,
    players: Query<&Transform, (With<Player>, Without<MainCamera>)>,
    mut cams: Query<(&mut Transform, &mut Projection), (With<MainCamera>, Without<Player>)>,
    mut views: Query<&mut Visibility, With<ViewRoot>>,
) {
    let Some(c) = cine.0.as_mut() else { return };
    c.t = (c.t + time.delta_secs()).min(c.seconds);
    let k = c.ease.apply(c.t / c.seconds);
    let pos = c.from.pos.lerp(c.to.pos, k);
    let look = c.from.look.lerp(c.to.look, k);
    let world = Transform::from_translation(pos).looking_at(look, Vec3::Y);
    let Ok(player) = players.single() else { return };
    let local = Transform::from_matrix(player.to_matrix().inverse() * world.to_matrix());
    for (mut tf, mut proj) in &mut cams {
        *tf = local;
        if let (Some((a, b)), Projection::Perspective(pp)) = (c.fov, &mut *proj) {
            pp.fov = (a + (b - a) * k).to_radians();
        }
    }
    for mut v in &mut views {
        if *v != Visibility::Hidden {
            *v = Visibility::Hidden;
        }
    }
}

// ---- HUD ----

#[derive(Resource, Default)]
struct HudHidden(bool);

/// A root UI node's visibility before `hud.visible false`.
#[derive(Component)]
struct HudStash(Visibility);

fn hud_visible(world: &mut World, p: Value) -> Result<Value, String> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    world.resource_mut::<HudHidden>().0 = !on;
    Ok(json!({"on": on}))
}

fn apply_hud(
    mut commands: Commands,
    hidden: Res<HudHidden>,
    mut roots: Query<(Entity, &mut Visibility, Option<&HudStash>), (With<Node>, Without<ChildOf>)>,
) {
    for (e, mut vis, stash) in &mut roots {
        if hidden.0 {
            if stash.is_none() {
                commands.entity(e).try_insert(HudStash(*vis));
            }
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
        } else if let Some(s) = stash {
            *vis = s.0;
            commands.entity(e).try_remove::<HudStash>();
        }
    }
}

// ---- smooth aim ----

#[derive(Resource, Default)]
struct Aim {
    turn: Option<Turn>,
    track: Option<Track>,
}

struct Turn {
    from: Vec2,
    to: Vec2,
    seconds: f32,
    t: f32,
}

struct Track {
    enemy: Option<usize>,
    rate: f32,
}

fn eye(world: &mut World) -> Result<Vec3, String> {
    let body = world
        .query_filtered::<&Transform, With<Player>>()
        .iter(world)
        .next()
        .map(|t| t.translation)
        .ok_or("no player")?;
    let local = world
        .query_filtered::<&Transform, With<MainCamera>>()
        .iter(world)
        .next()
        .map_or(Vec3::Y * 0.8, |t| t.translation);
    Ok(body + local)
}

fn yaw_pitch(d: Vec3) -> Vec2 {
    Vec2::new((-d.x).atan2(-d.z), d.y.atan2(Vec2::new(d.x, d.z).length()))
}

fn player_aim(world: &mut World, p: Value) -> Result<Value, String> {
    let at = p.get("at").and_then(v3).ok_or("`at` must be [x, y, z]")?;
    let seconds = p.get("seconds").and_then(Value::as_f64).unwrap_or(0.6).max(0.01) as f32;
    let d = at - eye(world)?;
    if d.length_squared() < 1e-6 {
        return Err("`at` is the player's eye".into());
    }
    let look = world.query_filtered::<&FpsLook, With<Player>>().iter(world).next().ok_or("no FpsLook")?;
    let from = Vec2::new(look.yaw, look.pitch);
    let mut to = yaw_pitch(d);
    // Shortest way round.
    to.x = from.x + (to.x - from.x + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    world.resource_mut::<Aim>().turn = Some(Turn { from, to, seconds, t: 0.0 });
    Ok(json!({"yaw": to.x, "pitch": to.y}))
}

fn player_track(world: &mut World, p: Value) -> Result<Value, String> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    let mut aim = world.resource_mut::<Aim>();
    aim.track = on.then(|| Track {
        enemy: p.get("enemy").and_then(Value::as_u64).map(|n| n as usize),
        rate: p.get("rate").and_then(Value::as_f64).unwrap_or(6.0) as f32,
    });
    Ok(json!({"on": on}))
}

/// With the control drive (after input, before the controller moves): ease the look.
fn drive_aim(
    time: Res<Time>,
    mut aim: ResMut<Aim>,
    mut players: Query<(&Transform, &mut FpsLook), With<Player>>,
    cams: Query<&Transform, (With<MainCamera>, Without<Player>)>,
    enemies: Query<(&Enemy, &Transform, &EnemyCombat), Without<Player>>,
) {
    let Ok((body, mut look)) = players.single_mut() else { return };
    let dt = time.delta_secs();
    if let Some(t) = aim.turn.as_mut() {
        t.t = (t.t + dt).min(t.seconds);
        let k = Ease::InOut.apply(t.t / t.seconds);
        let v = t.from.lerp(t.to, k);
        look.yaw = v.x;
        look.pitch = v.y.clamp(-MAX_PITCH, MAX_PITCH);
        if t.t >= t.seconds {
            aim.turn = None;
        }
        return;
    }
    let Some(track) = aim.track.as_ref() else { return };
    let eye = body.translation + cams.iter().next().map_or(Vec3::Y * 0.8, |t| t.translation);
    let target = enemies
        .iter()
        .filter(|(e, _, c)| !c.dead && track.enemy.is_none_or(|s| s == e.slot))
        // Prefer the ones in a fight with the player (they can see each other).
        .min_by(|a, b| {
            let score = |(_, tf, c): &(&Enemy, &Transform, &EnemyCombat)| tf.translation.distance(eye) - if c.sees_player { 1000.0 } else { 0.0 };
            score(a).total_cmp(&score(b))
        })
        .map(|(_, tf, _)| tf.translation + Vec3::Y * 1.3);
    let Some(target) = target else { return };
    let want = yaw_pitch(target - eye);
    let dyaw = (want.x - look.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let k = 1.0 - (-track.rate * dt).exp();
    look.yaw += dyaw * k;
    look.pitch = (look.pitch + (want.y - look.pitch) * k).clamp(-MAX_PITCH, MAX_PITCH);
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<Capture>()
        .init_resource::<CineCam>()
        .init_resource::<HudHidden>()
        .init_resource::<Aim>()
        .add_systems(Update, drive_aim.in_set(super::ControlDrive))
        .add_systems(
            PostUpdate,
            (fly_camera, apply_hud).before(bevy::transform::TransformSystems::Propagate).before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
        )
        .add_systems(Last, capture_frame);

    app.add_control_command(
        "capture.frames",
        "Render every frame to dir/f_NNNNN.png at a fixed time step (1/fps); replies at once, poll capture.status",
        params(&[
            ("dir", "string", "output directory"),
            ("fps", "number", "frames per game second (default 60)"),
            ("seconds", "number", "game seconds to capture"),
            ("frames", "integer", "frames to capture (instead of seconds)"),
        ]),
        capture_frames,
    )
    .add_control_command("capture.status", "The running (or last) frame capture", params(&[]), capture_status)
    .add_control_command("capture.stop", "End the frame capture now", params(&[]), capture_stop)
    .add_control_command(
        "camera.path",
        "Fly the main camera from one pose to another (eased), then hold; hides the viewmodel",
        params(&[
            ("from", "object", "{pos:[x,y,z], look:[x,y,z]}"),
            ("to", "object", "{pos, look} (default: from)"),
            ("seconds", "number", "duration in game seconds (default 4)"),
            ("ease", "string", "inout (default) | linear | in | out"),
            ("fov", "array", "vertical FOV degrees: n or [from, to]"),
        ]),
        camera_path,
    )
    .add_control_command("camera.release", "Give the camera back to the player", params(&[]), camera_release)
    .add_control_command("hud.visible", "Show or hide the HUD (every root UI node)", params(&[("on", "boolean", "default true")]), hud_visible)
    .add_control_command(
        "player.aim",
        "Ease the player's look onto a point",
        params(&[("at", "array", "[x, y, z]"), ("seconds", "number", "default 0.6")]),
        player_aim,
    )
    .add_control_command(
        "player.track",
        "Keep the player's look easing onto an enemy's chest",
        params(&[
            ("on", "boolean", "default true"),
            ("enemy", "integer", "squad slot (default: nearest alive, fighting ones first)"),
            ("rate", "number", "1/s (default 6)"),
        ]),
        player_track,
    );
}
