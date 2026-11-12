//! Bodycam camera style (Video → "Camera style: Bodycam"; `video.camera_style` on the control
//! port). Classic, the default, is untouched by everything here.
//!
//! - Free aim ([`free_aim`]): the mouse moves the gun; the camera follows rigidly (no smoothing,
//!   the same frame) once the gun leaves a 6°×4° box (it collapses to 0 on ADS), with a dutch
//!   roll from yaw speed and strafing (±7°) and a small vest shock per shot. The chest mount is
//!   pulled back towards the eyes by a sphere cast so it never ends up in or past a wall. The engine
//!   still writes the CS-style camera each frame; this overrides its rotation afterwards.
//! - Handling: the viewmodel's 6-DOF spring rig, wall compression and true-barrel shots live in
//!   `weapon::viewmodel` / `weapon::fire` and read [`BodycamAim`].
//! - Optics (`lens`): the engine's body camera (`engine::look::BodycamLook`) in the map's look at
//!   the wide FOV: fisheye, metering exposure, sensor curve, noise, smear, rolling shutter.
//! - HUD (`hud`): [REC] stamp, no crosshair and no health/ammo numbers (R on a full mag shows a
//!   short ammo readout: the "press check").

use avian3d::prelude::*;
use bevy::prelude::*;
use engine::fps::{FpsLook, FpsMotor, FpsViewKick};
use engine::gfx::MainCamera;

use crate::player::{Player, PlayerSet};
use crate::settings::{CameraStyle, ShroudSettings};
use crate::state::{GameState, has_visuals, not_paused};
use crate::weapon::{GunEvent, Weapon, WeaponSet, ease};

pub mod hud;
pub mod lens;

/// Vertical FOV (Bevy's `PerspectiveProjection::fov`, like the FOV setting): the engine's
/// recommended render FOV for its body camera. 100° vertical at 16:9 is a ~130° horizontal
/// action-cam lens; the fisheye keeps the corners and magnifies the centre.
pub const BODYCAM_FOV_DEG: f32 = engine::look::BodycamLook::RECOMMENDED_FOV_DEG;
/// Free-aim box half extents (yaw, pitch): the gun moves this far on screen before the camera
/// turns. Small, so turning reads as immediate (the owner's playtest: 18° × 12° felt late).
pub const DEADZONE: Vec2 = Vec2::new(6.0f32.to_radians(), 4.0f32.to_radians());
pub const MAX_ROLL: f32 = 7.0f32.to_radians();
/// Chest mount (spec §2.1 `spine_02`): the camera this far below and right of the eyes.
pub const CHEST_DROP: f32 = 0.22;
pub const CHEST_RIGHT: f32 = 0.08;
/// Chest mount: the vest rolls more with turns and strafes than a head does.
pub const CHEST_ROLL_GAIN: f32 = 1.35;

/// Camera offset from the eyes (body space: +X right, +Y up) for a mount. `gait` is the step
/// phase (radians), `pace` 0..~1.2 how fast he moves, `t` seconds (breathing). The head mount
/// is the eye itself; the chest bobs side to side with each step and rises with each breath.
pub fn mount_offset(mount: crate::settings::BodycamMount, gait: f32, pace: f32, t: f32) -> Vec3 {
    match mount {
        crate::settings::BodycamMount::Head => Vec3::ZERO,
        crate::settings::BodycamMount::Chest => {
            let step = Vec3::new(gait.sin() * 0.014, -gait.sin().abs() * 0.010, 0.0) * pace;
            let breath = Vec3::Y * (t * 1.6).sin() * 0.004;
            Vec3::new(CHEST_RIGHT, -CHEST_DROP, 0.0) + step + breath
        }
    }
}

pub fn is_bodycam(s: &ShroudSettings) -> bool {
    s.camera_style == CameraStyle::Bodycam
}

/// Run condition: bodycam mode is on.
pub fn bodycam_on(s: Res<ShroudSettings>) -> bool {
    is_bodycam(&s)
}

/// The camera's base vertical FOV for the current camera style (before ADS zoom).
pub fn base_fov_deg(core: &engine::settings::CoreSettings, s: &ShroudSettings) -> f32 {
    if is_bodycam(s) { BODYCAM_FOV_DEG } else { core.fov_deg }
}

/// Free-aim state for this frame (bodycam only; `active == false` in Classic).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct BodycamAim {
    pub active: bool,
    /// Where the gun points: absolute (yaw, pitch), mouse look + recoil kick.
    pub weapon: Vec2,
    /// Where the chest camera looks: absolute (yaw, pitch).
    pub camera: Vec2,
    /// Dutch roll (radians, about the view axis).
    pub roll: f32,
    /// Vest shock spring: (pitch, roll) value and velocity.
    shock: Vec2,
    shock_vel: Vec2,
    last_weapon_yaw: Option<f32>,
    yaw_rate: f32,
    /// The gun's rotation in camera space (applied to the viewmodel root).
    pub weapon_in_camera: Quat,
    /// Step phase for the chest mount's sway (radians), and the camera's offset from the eyes
    /// this frame (body space; zero on the head mount).
    gait: f32,
    pub mount_offset: Vec3,
}

impl BodycamAim {
    /// Gun minus camera (yaw, pitch).
    pub fn delta(&self) -> Vec2 {
        self.weapon - self.camera
    }
}

pub struct BodycamPlugin;

impl Plugin for BodycamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BodycamAim>().add_systems(
            Update,
            free_aim.after(PlayerSet::Move).before(WeaponSet).run_if(in_state(GameState::Playing).and_then(not_paused).and_then(has_visuals)),
        );
        hud::build(app);
        #[cfg(feature = "dev")]
        dev(app);
    }
}

/// `video.camera_style {style: "classic" | "bodycam"}` (no `style`: report it). The rest of the
/// state is in `video.bodycam`.
#[cfg(feature = "dev")]
fn dev(app: &mut App) {
    use engine::devtools::{AddControlCommand, params};
    use serde_json::{Value, json};
    app.add_control_command(
        "video.camera_style",
        "Camera style: classic (CS-style centre aim, full HUD) or bodycam (chest-cam optics, free aim)",
        params(&[("style", "string", "classic | bodycam (omit to read)")]),
        |world: &mut World, p: Value| {
            if let Some(s) = p.get("style").and_then(Value::as_str) {
                let style = match s.to_ascii_lowercase().as_str() {
                    "classic" => CameraStyle::Classic,
                    "bodycam" => CameraStyle::Bodycam,
                    _ => return Err(format!("unknown style `{s}` (classic | bodycam)")),
                };
                let mut st = world.resource_mut::<ShroudSettings>();
                if st.camera_style != style {
                    st.camera_style = style;
                }
            }
            Ok(json!({"style": format!("{:?}", world.resource::<ShroudSettings>().camera_style).to_ascii_lowercase()}))
        },
    )
    .add_control_command(
        "video.bodycam",
        "Bodycam free-aim and handling state; optionally drive the player for `ms` without blocking (for screenshots mid-turn / mid-burst)",
        params(&[
            ("turn", "number", "turn at this many degrees per second (+ = left)"),
            ("strafe", "number", "-1..1 strafe input"),
            ("fire", "boolean", "hold the trigger"),
            ("ms", "integer", "how long to drive (default 1000)"),
        ]),
        |world: &mut World, p: Value| {
            let turn = p.get("turn").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let strafe = p.get("strafe").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let fire = p.get("fire").and_then(Value::as_bool).unwrap_or(false);
            if turn != 0.0 || strafe != 0.0 || fire {
                let ms = p.get("ms").and_then(Value::as_u64).unwrap_or(1000) as f32;
                *world.resource_mut::<Drive>() = Drive { turn: turn.to_radians(), strafe: strafe.clamp(-1.0, 1.0), fire, left: ms / 1000.0 };
            }
            let a = *world.resource::<BodycamAim>();
        let m = *world.resource::<crate::weapon::viewmodel::MuzzleCam>();
        Ok(json!({
            "active": a.active,
            "weapon_deg": [a.weapon.x.to_degrees(), a.weapon.y.to_degrees()],
            "camera_deg": [a.camera.x.to_degrees(), a.camera.y.to_degrees()],
            "delta_deg": [a.delta().x.to_degrees(), a.delta().y.to_degrees()],
            "roll_deg": a.roll.to_degrees(),
            "mount_offset": [a.mount_offset.x, a.mount_offset.y, a.mount_offset.z],
            "muzzle_cam": [m.0.x, m.0.y, m.0.z],
            "barrel_cam": [m.1.x, m.1.y, m.1.z],
            "press_check_s": world.resource::<hud::PressCheck>().0,
            }))
        },
    )
    .init_resource::<Drive>()
    .add_systems(Update, drive.after(PlayerSet::Input).before(PlayerSet::Move).run_if(in_state(GameState::Playing)));
}

/// `video.bodycam` drive job (dev only).
#[cfg(feature = "dev")]
#[derive(Resource, Default)]
struct Drive {
    turn: f32,
    strafe: f32,
    fire: bool,
    left: f32,
}

#[cfg(feature = "dev")]
fn drive(
    time: Res<Time>,
    mut job: ResMut<Drive>,
    mut intent: ResMut<crate::player::PlayerIntent>,
    mut players: Query<(&mut FpsLook, &mut engine::fps::FpsInput), With<Player>>,
) {
    if job.left <= 0.0 {
        return;
    }
    let dt = time.delta_secs();
    job.left -= dt;
    let done = job.left <= 0.0;
    for (mut look, mut input) in &mut players {
        look.yaw += job.turn * dt;
        input.move_axis.x = if done { 0.0 } else { job.strafe };
    }
    if job.fire {
        intent.fire = !done;
    }
}

/// Radius kept between the camera and any level surface: covers the near plane (0.03 m at a
/// 100° × 130° FOV is ±0.04 × ±0.06 m) with margin for the fisheye overscan.
pub const CAMERA_CLEARANCE: f32 = 0.12;

/// The mount offset shortened so the camera stays [`CAMERA_CLEARANCE`] clear of level
/// colliders: a sphere cast from the eyes (inside the body capsule, always clear) to where the
/// mount wants the camera. Turning into a corner, the chest camera slides back towards the head
/// instead of poking into (or through) the wall and showing the outside of the map.
fn clear_of_walls(spatial: &SpatialQuery, player: Entity, body: &Transform, eye: Vec3, offset: Vec3) -> Vec3 {
    if offset == Vec3::ZERO {
        return offset;
    }
    let from = body.transform_point(eye);
    let to = body.transform_point(eye + offset);
    let Ok((dir, len)) = Dir3::new_and_length(to - from) else { return offset };
    let filter = SpatialQueryFilter::from_excluded_entities([player]);
    let config = ShapeCastConfig::from_max_distance(len);
    match spatial.cast_shape(&Collider::sphere(CAMERA_CLEARANCE), from, Quat::IDENTITY, dir, &config, &filter) {
        Some(hit) => offset * (hit.distance / len).clamp(0.0, 1.0),
        None => offset,
    }
}

/// Where the camera wants to be: the gun's aim pulled back inside the (ADS-scaled) box.
pub fn follow_target(weapon: Vec2, camera: Vec2, ads: f32) -> Vec2 {
    let dz = DEADZONE * (1.0 - ads.clamp(0.0, 1.0));
    weapon - (weapon - camera).clamp(-dz, dz)
}

/// Spec §2.2 dutch angle: yaw speed (rad/s) and strafe speed (m/s, + = right), clamped ±7°.
pub fn dutch_roll(yaw_rate: f32, strafe: f32) -> f32 {
    (-yaw_rate * 0.08 - strafe * 0.025).clamp(-MAX_ROLL, MAX_ROLL)
}

fn free_aim(
    time: Res<Time>,
    settings: Res<ShroudSettings>,
    mut aim: ResMut<BodycamAim>,
    players: Query<(Entity, &FpsLook, Option<&FpsViewKick>, &FpsMotor, &Transform, Option<&Weapon>), With<Player>>,
    spatial: SpatialQuery,
    mut cams: Query<&mut Transform, (With<MainCamera>, Without<Player>)>,
    mut events: MessageReader<GunEvent>,
) {
    let shots: Vec<_> = events.read().filter_map(|e| if let GunEvent::Shot { kind } = e { Some(*kind) } else { None }).collect();
    if !is_bodycam(&settings) {
        if aim.active {
            *aim = BodycamAim::default();
        }
        return;
    }
    let Ok((player, look, kick, motor, body, weapon)) = players.single() else { return };
    let Ok(mut cam) = cams.single_mut() else { return };
    let aim = &mut *aim;
    let dt = time.delta_secs().clamp(1e-4, 0.05);
    let kick = kick.copied().unwrap_or_default();
    let target_weapon = Vec2::new(look.yaw + kick.yaw, look.pitch + kick.pitch);
    // First frame, round reset or a scripted `player.look` jump: start centred.
    if !aim.active || (target_weapon - aim.weapon).abs().max_element() > 1.0 {
        aim.camera = target_weapon;
        aim.roll = 0.0;
        aim.last_weapon_yaw = None;
        aim.yaw_rate = 0.0;
    }
    aim.active = true;
    aim.weapon = target_weapon;

    // Rigid follow: past the box edge the camera turns with the mouse in the same frame (any
    // smoothing here is input latency).
    let ads = weapon.map_or(0.0, |w| ease(w.ads));
    let camera = follow_target(aim.weapon, aim.camera, ads);
    aim.camera = camera;

    // Dutch roll from the gun's yaw speed (smoothed) and strafing.
    let rate = aim.last_weapon_yaw.map_or(0.0, |l| (aim.weapon.x - l) / dt);
    aim.last_weapon_yaw = Some(aim.weapon.x);
    aim.yaw_rate += (rate - aim.yaw_rate) * (1.0 - (-12.0 * dt).exp());
    let strafe = motor.velocity.dot(body.rotation * Vec3::X);
    let chest = settings.bodycam_mount == crate::settings::BodycamMount::Chest;
    let want_roll = dutch_roll(aim.yaw_rate, strafe) * (1.0 - 0.6 * ads) * if chest { CHEST_ROLL_GAIN } else { 1.0 };
    aim.roll += (want_roll - aim.roll) * (1.0 - (-6.0 * dt).exp());

    // Vest shock: each shot kicks the chest camera up and rolls it a little.
    for kind in shots {
        let s = crate::weapon::viewmodel::fit(kind).kick / 30.0;
        aim.shock_vel += Vec2::new(0.22 * s, rand::random_range(-0.25..0.25) * s);
    }
    let acc = -220.0 * aim.shock - 20.0 * aim.shock_vel;
    aim.shock_vel += acc * dt;
    aim.shock += aim.shock_vel * dt;

    // Camera relative to the body (which carries the mouse yaw).
    let cam_rot = Quat::from_rotation_y(camera.x - look.yaw) * Quat::from_rotation_x(camera.y + aim.shock.x) * Quat::from_rotation_z(aim.roll + aim.shock.y);
    let gun_rot = Quat::from_rotation_y(aim.weapon.x - look.yaw) * Quat::from_rotation_x(aim.weapon.y);
    aim.weapon_in_camera = cam_rot.inverse() * gun_rot;
    if cam.rotation != cam_rot {
        cam.rotation = cam_rot;
    }

    // Mount: the chest camera sits below and right of the eyes and sways with the steps. Only
    // the view moves: the body, hitboxes and every sight line (AI, LOS) stay at the head.
    let speed = Vec2::new(motor.velocity.x, motor.velocity.z).length();
    let pace = (speed / 4.5).min(1.2) * if motor.grounded { 1.0 } else { 0.0 };
    aim.gait = (aim.gait + dt * std::f32::consts::TAU * 0.95 * pace).rem_euclid(std::f32::consts::TAU * 2.0);
    let offset = mount_offset(settings.bodycam_mount, aim.gait, pace, time.elapsed_secs());
    let eye = Vec3::new(0.0, motor.eye_offset, 0.0);
    let offset = clear_of_walls(&spatial, player, body, eye, offset);
    if offset != Vec3::ZERO || aim.mount_offset != Vec3::ZERO {
        let want = eye + offset;
        if cam.translation != want {
            cam.translation = want;
        }
    }
    aim.mount_offset = offset;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_moves_stay_inside_the_box() {
        // 4° of yaw inside the 6° box: the camera does not move.
        let w = Vec2::new(4f32.to_radians(), 0.0);
        assert_eq!(follow_target(w, Vec2::ZERO, 0.0), Vec2::ZERO);
        // 30°: the camera is pulled to 24° (the box edge), at once.
        let t = follow_target(Vec2::new(30f32.to_radians(), 0.0), Vec2::ZERO, 0.0);
        assert!((t.x - 24f32.to_radians()).abs() < 1e-5);
    }

    #[test]
    fn ads_collapses_the_box() {
        let w = Vec2::new(0.1, -0.05);
        assert_eq!(follow_target(w, Vec2::ZERO, 1.0), w);
    }

    #[test]
    fn roll_is_clamped() {
        assert!((dutch_roll(100.0, 0.0) + MAX_ROLL).abs() < 1e-6);
        assert!((dutch_roll(0.0, -100.0) - MAX_ROLL).abs() < 1e-6);
        assert!(dutch_roll(0.0, 0.0).abs() < 1e-6);
    }
}
