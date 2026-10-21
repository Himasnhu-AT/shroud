//! Kinematic FPS controller.
//!
//! Two stages, ordered by [`PlayerSet`]:
//! 1. `PlayerSet::Input`: keyboard + mouse → [`PlayerIntent`] (skipped when
//!    [`ExternalControl`] is on, so a test/control port can write the intent).
//! 2. `PlayerSet::Move`: [`PlayerIntent`] → look, crouch, jump and
//!    collide-and-slide movement with Avian shape casts.

use avian3d::prelude::*;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

use crate::map::{ArenaRes, to_world};
use crate::settings::Settings;
use crate::state::{GameState, Paused, has_visuals};
use crate::weapon::Health;

pub const RADIUS: f32 = 0.35;
pub const STAND_HEIGHT: f32 = 1.8;
pub const CROUCH_HEIGHT: f32 = 1.2;
pub const STAND_EYE: f32 = 1.65;
/// Eye height while crouched (low cover is 1.1 m).
pub const CROUCH_EYE: f32 = 1.0;
pub const WALK_SPEED: f32 = 5.0;
pub const SPRINT_SPEED: f32 = 8.0;
pub const CROUCH_SPEED: f32 = 2.5;
pub const JUMP_SPEED: f32 = 6.5;
pub const GRAVITY: f32 = 18.0;
pub const PLAYER_MAX_HP: f32 = 100.0;
const SKIN: f32 = 0.02;
const MOUSE_SENSITIVITY: f32 = 0.0022;
const MAX_PITCH: f32 = 1.54;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerIntent>()
            .init_resource::<ExternalControl>()
            .configure_sets(
                Update,
                (PlayerSet::Input, PlayerSet::Move)
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(Startup, spawn_player)
            .add_systems(
                OnEnter(GameState::Playing),
                (reset_player.after(crate::map::MapSet), lock_cursor.run_if(has_visuals).run_if(not_test_driven)),
            )
            .add_systems(OnExit(GameState::Playing), (unlock_cursor.run_if(has_visuals), clear_intent))
            .add_systems(
                Update,
                (
                    keyboard_mouse_to_intent.in_set(PlayerSet::Input).run_if(has_visuals),
                    move_player.in_set(PlayerSet::Move),
                    // Outside PlayerSet: must also run while paused.
                    cursor_follow_pause
                        .before(PlayerSet::Input)
                        .run_if(in_state(GameState::Playing).and_then(has_visuals).and_then(not_test_driven)),
                ),
            );
    }
}

/// What the player wants to do this frame. Filled by keyboard/mouse (or an
/// external controller), consumed by the movement and weapon systems.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct PlayerIntent {
    /// x = strafe right, y = forward; length ≤ 1.
    pub move_axis: Vec2,
    /// Radians to turn this frame: x = yaw right, y = pitch up. Consumed (zeroed) by movement.
    pub look_delta: Vec2,
    pub sprint: bool,
    pub jump: bool,
    pub crouch: bool,
    /// Trigger held.
    pub fire: bool,
    pub reload: bool,
}

/// When `true`, keyboard/mouse no longer write [`PlayerIntent`].
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ExternalControl(pub bool);

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlayerSet {
    /// Devices → `PlayerIntent`.
    Input,
    /// `PlayerIntent` → movement/look. Weapon firing runs after this.
    Move,
}

#[derive(Component)]
pub struct Player;

/// The first-person camera (child of [`Player`]). Exists in headless too
/// (without render components) so its `GlobalTransform` is the aim ray.
#[derive(Component)]
pub struct PlayerCamera;

/// Look angles in radians. `yaw = 0` faces −Z (north).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PlayerLook {
    pub yaw: f32,
    pub pitch: f32,
}

/// Extra view rotation added on top of [`PlayerLook`] (weapon recoil).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ViewKick {
    pub pitch: f32,
    pub yaw: f32,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct PlayerMotor {
    pub velocity: Vec3,
    pub grounded: bool,
    pub crouched: bool,
    /// Current camera height above the capsule center (smoothed).
    eye_offset: f32,
}

impl Default for PlayerMotor {
    fn default() -> Self {
        Self {
            velocity: Vec3::ZERO,
            grounded: false,
            crouched: false,
            eye_offset: eye_offset(false),
        }
    }
}

fn height(crouched: bool) -> f32 {
    if crouched { CROUCH_HEIGHT } else { STAND_HEIGHT }
}

fn eye_offset(crouched: bool) -> f32 {
    if crouched {
        CROUCH_EYE - CROUCH_HEIGHT * 0.5
    } else {
        STAND_EYE - STAND_HEIGHT * 0.5
    }
}

fn body_collider(crouched: bool) -> Collider {
    Collider::capsule(RADIUS, height(crouched) - 2.0 * RADIUS)
}

/// Capsule-center position for a player standing at ground point `p`.
fn spawn_position(arena: &ArenaRes) -> Vec3 {
    to_world(arena.0.player_spawn, STAND_HEIGHT * 0.5)
}

fn spawn_player(mut commands: Commands, arena: Res<ArenaRes>, headless: Res<crate::Headless>, settings: Res<Settings>) {
    let camera = if headless.0 {
        commands
            .spawn((PlayerCamera, Transform::from_xyz(0.0, eye_offset(false), 0.0)))
            .id()
    } else {
        commands
            .spawn((
                PlayerCamera,
                Camera3d::default(),
                Projection::from(PerspectiveProjection {
                    fov: settings.fov_deg.to_radians(),
                    near: 0.03,
                    ..default()
                }),
                crate::gfx::camera_components(settings.quality),
                Transform::from_xyz(0.0, eye_offset(false), 0.0),
            ))
            .id()
    };
    commands
        .spawn((
            Name::new("Player"),
            Player,
            PlayerLook::default(),
            ViewKick::default(),
            PlayerMotor::default(),
            Health::new(PLAYER_MAX_HP),
            RigidBody::Kinematic,
            body_collider(false),
            Transform::from_translation(spawn_position(&arena)),
            Visibility::default(),
        ))
        .add_child(camera);
}

fn reset_player(
    arena: Res<ArenaRes>,
    mut q: Query<(&mut Transform, &mut PlayerLook, &mut PlayerMotor, &mut Health, &mut Collider), With<Player>>,
) {
    for (mut tf, mut look, mut motor, mut hp, mut col) in &mut q {
        tf.translation = spawn_position(&arena);
        *look = PlayerLook::default();
        *motor = PlayerMotor::default();
        hp.hp = hp.max;
        *col = body_collider(false);
    }
}

fn clear_intent(mut intent: ResMut<PlayerIntent>) {
    *intent = PlayerIntent::default();
}

fn set_cursor(cursor: &mut CursorOptions, locked: bool) {
    cursor.visible = !locked;
    cursor.grab_mode = if locked {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
}

/// Runs driven by the control port / MCP never grab the user's mouse.
pub fn not_test_driven() -> bool {
    !crate::ops::has_flag("control") && !crate::ops::has_flag("mcp")
}

fn lock_cursor(mut cursors: Query<&mut CursorOptions>) {
    for mut c in &mut cursors {
        set_cursor(&mut c, true);
    }
}

fn unlock_cursor(mut cursors: Query<&mut CursorOptions>) {
    for mut c in &mut cursors {
        set_cursor(&mut c, false);
    }
}

/// The pause menu frees the cursor and resuming captures it again. A click re-captures it
/// if it was lost some other way (e.g. the window lost focus).
fn cursor_follow_pause(
    paused: Res<Paused>,
    mut cursors: Query<&mut CursorOptions>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    for mut c in &mut cursors {
        let locked = c.grab_mode != CursorGrabMode::None;
        if paused.0 {
            if locked {
                set_cursor(&mut c, false);
            }
        } else if !locked && (paused.is_changed() || mouse.just_pressed(MouseButton::Left)) {
            set_cursor(&mut c, true);
        }
    }
}

fn keyboard_mouse_to_intent(
    external: Res<ExternalControl>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    cursors: Query<&CursorOptions>,
    settings: Res<Settings>,
    mut intent: ResMut<PlayerIntent>,
) {
    if external.0 {
        return;
    }
    let captured = cursors
        .iter()
        .any(|c| c.grab_mode != CursorGrabMode::None);
    let axis = |neg: KeyCode, pos: KeyCode| {
        (keys.pressed(pos) as i32 - keys.pressed(neg) as i32) as f32
    };
    let mv = Vec2::new(axis(KeyCode::KeyA, KeyCode::KeyD), axis(KeyCode::KeyS, KeyCode::KeyW));
    *intent = PlayerIntent {
        move_axis: mv.clamp_length_max(1.0),
        look_delta: if captured {
            let invert = if settings.invert_y { -1.0 } else { 1.0 };
            Vec2::new(motion.delta.x, -motion.delta.y * invert) * MOUSE_SENSITIVITY * settings.sensitivity
        } else {
            Vec2::ZERO
        },
        sprint: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        jump: keys.pressed(KeyCode::Space),
        crouch: keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::KeyC]),
        fire: captured && mouse.pressed(MouseButton::Left),
        reload: keys.pressed(KeyCode::KeyR),
    };
}

/// Result of [`collide_and_slide`].
pub struct SlideResult {
    pub position: Vec3,
    pub hit_ground: bool,
    pub hit_ceiling: bool,
    pub hit_wall: bool,
}

/// Moves `shape` from `pos` by `disp`, sliding along anything it touches.
/// The whole displacement is swept, so nothing tunnels regardless of speed.
pub fn collide_and_slide(
    spatial: &SpatialQuery,
    shape: &Collider,
    pos: Vec3,
    disp: Vec3,
    filter: &SpatialQueryFilter,
) -> SlideResult {
    let mut out = SlideResult {
        position: pos,
        hit_ground: false,
        hit_ceiling: false,
        hit_wall: false,
    };
    let mut remaining = disp;
    let original_dir = disp.normalize_or_zero();
    for _ in 0..4 {
        let len = remaining.length();
        let Ok(dir) = Dir3::new(remaining) else { break };
        if len < 1e-5 {
            break;
        }
        let config = ShapeCastConfig {
            max_distance: len + SKIN,
            target_distance: 0.0,
            compute_contact_on_penetration: true,
            ignore_origin_penetration: false,
        };
        let Some(hit) = spatial.cast_shape(shape, out.position, Quat::IDENTITY, dir, &config, filter)
        else {
            out.position += remaining;
            break;
        };
        let n = hit.normal1.normalize_or_zero();
        let travel = (hit.distance - SKIN).clamp(0.0, len);
        out.position += *dir * travel;
        if n == Vec3::ZERO {
            break;
        }
        if n.y > 0.7 {
            out.hit_ground = true;
        } else if n.y < -0.7 {
            out.hit_ceiling = true;
        } else {
            out.hit_wall = true;
        }
        if hit.distance <= 0.0 {
            // Starting in contact/penetration: nudge out along the normal.
            out.position += n * SKIN;
        }
        remaining = *dir * (len - travel);
        let into = remaining.dot(n);
        if into < 0.0 {
            remaining -= n * into;
        }
        // Never slide back against the requested direction (prevents jitter in corners).
        if remaining.dot(original_dir) < 0.0 {
            break;
        }
    }
    out
}

fn move_player(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut intent: ResMut<PlayerIntent>,
    mut players: Query<
        (Entity, &mut Transform, &mut PlayerLook, &mut PlayerMotor, &ViewKick),
        (With<Player>, Without<PlayerCamera>),
    >,
    mut cams: Query<&mut Transform, With<PlayerCamera>>,
) {
    let dt = time.delta_secs().min(0.05);
    let Ok((entity, mut tf, mut look, mut motor, kick)) = players.single_mut() else {
        return;
    };
    // Collider is read by SpatialQuery, so it is replaced via commands.
    let mut collider = body_collider(motor.crouched);
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);

    // Look.
    look.yaw -= intent.look_delta.x;
    look.pitch = (look.pitch + intent.look_delta.y).clamp(-MAX_PITCH, MAX_PITCH);
    intent.look_delta = Vec2::ZERO;
    tf.rotation = Quat::from_rotation_y(look.yaw);

    // Crouch (feet stay planted; standing up needs headroom).
    if intent.crouch != motor.crouched {
        let dh = STAND_HEIGHT - CROUCH_HEIGHT;
        let can_change = intent.crouch || {
            let config = ShapeCastConfig::from_max_distance(dh);
            spatial
                .cast_shape(&collider, tf.translation, Quat::IDENTITY, Dir3::Y, &config, &filter)
                .is_none()
        };
        if can_change {
            let shift = if intent.crouch { -dh * 0.5 } else { dh * 0.5 };
            if motor.grounded {
                tf.translation.y += shift;
                // Keep the eye where it was; it then eases to the new height.
                motor.eye_offset -= shift;
            }
            motor.crouched = intent.crouch;
            collider = body_collider(motor.crouched);
            commands.entity(entity).insert(collider.clone());
        }
    }

    // Horizontal velocity.
    let speed = if motor.crouched {
        CROUCH_SPEED
    } else if intent.sprint && intent.move_axis.y > 0.1 {
        SPRINT_SPEED
    } else {
        WALK_SPEED
    };
    let fwd = tf.rotation * Vec3::NEG_Z;
    let right = tf.rotation * Vec3::X;
    let axis = intent.move_axis.clamp_length_max(1.0);
    let wish = (right * axis.x + fwd * axis.y) * speed;
    let accel = if motor.grounded { 60.0 } else { 12.0 };
    let horiz = Vec3::new(motor.velocity.x, 0.0, motor.velocity.z);
    let delta = wish - horiz;
    let step = delta.clamp_length_max(accel * dt);
    motor.velocity.x += step.x;
    motor.velocity.z += step.z;

    // Vertical velocity.
    if motor.grounded && intent.jump && !motor.crouched {
        motor.velocity.y = JUMP_SPEED;
        motor.grounded = false;
    }
    motor.velocity.y -= GRAVITY * dt;

    // Sweep horizontally, then vertically.
    let h = collide_and_slide(
        &spatial,
        &collider,
        tf.translation,
        Vec3::new(motor.velocity.x, 0.0, motor.velocity.z) * dt,
        &filter,
    );
    if h.hit_wall && dt > 0.0 {
        let moved = (h.position - tf.translation) / dt;
        motor.velocity.x = moved.x;
        motor.velocity.z = moved.z;
    }
    let v = collide_and_slide(&spatial, &collider, h.position, Vec3::Y * motor.velocity.y * dt, &filter);
    motor.grounded = v.hit_ground && motor.velocity.y <= 0.0;
    if motor.grounded || (v.hit_ceiling && motor.velocity.y > 0.0) {
        motor.velocity.y = 0.0;
    }
    let mut p = v.position;
    // Safety net: never fall out of the world.
    if p.y < -5.0 {
        p.y = height(motor.crouched) * 0.5;
        motor.velocity = Vec3::ZERO;
    }
    tf.translation = p;

    // Camera: ease eye height, apply pitch + recoil.
    let target = eye_offset(motor.crouched);
    motor.eye_offset += (target - motor.eye_offset) * (1.0 - (-14.0 * dt).exp());
    if let Ok(mut cam) = cams.single_mut() {
        cam.translation = Vec3::new(0.0, motor.eye_offset, 0.0);
        cam.rotation = Quat::from_rotation_y(kick.yaw) * Quat::from_rotation_x(look.pitch + kick.pitch);
    }
}
