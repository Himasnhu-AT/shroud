//! The player: the engine's first-person controller ([`FpsController`], kinematic
//! collide-and-slide, mouse look) tuned for Shroud, plus health, the trigger and round resets.
//!
//! Each frame `PlayerSet::Input` (the engine's `FpsSet::Input`) turns keyboard + mouse into the
//! controller's `FpsInput` and the trigger into [`PlayerIntent`]; the control port overrides
//! both between `Input` and `Move` (see `dev::commands`). `PlayerSet::Move` moves the player.

use avian3d::prelude::Collider;
use bevy::prelude::*;
use engine::fps::{FpsController, FpsInput, FpsLook, FpsMotor};
use engine::gfx::MainCamera;
use engine::input::CursorCaptured;
use engine::render_budget::Actor;
use engine::settings::CoreSettings;

pub use engine::fps::{FpsSet as PlayerSet, collide_and_slide};

use crate::map::{ArenaRes, to_world};
use crate::state::{GameState, Headless, has_visuals};
use crate::weapon::Health;

pub const PLAYER_MAX_HP: f32 = 100.0;

/// Shroud's movement: brisk sprint, high jump, a crouch low enough for 1.1 m cover.
pub fn controller() -> FpsController {
    FpsController {
        walk_speed: 5.0,
        sprint_speed: 8.0,
        crouch_speed: 2.5,
        height: 1.8,
        crouch_height: 1.2,
        radius: 0.35,
        jump_speed: 6.5,
        eye_height: 1.65,
        crouch_eye_height: 1.0,
        gravity: 18.0,
        ground_accel: 60.0,
        air_accel: 12.0,
        max_pitch: 1.54,
    }
}

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerIntent>()
            .init_resource::<ExternalControl>()
            // The engine runs the controller in all of `InGame`; Shroud moves only in rounds.
            .configure_sets(Update, (PlayerSet::Input, PlayerSet::Move).run_if(in_state(GameState::Playing)))
            .add_systems(Startup, spawn_player)
            .add_systems(OnEnter(GameState::Playing), reset_player.after(crate::map::MapSet))
            .add_systems(OnExit(GameState::Playing), clear_intent)
            .add_systems(
                Update,
                (trigger_input.in_set(PlayerSet::Input).run_if(has_visuals), stay_in_world.after(PlayerSet::Move).run_if(in_state(GameState::Playing))),
            );
    }
}

/// The trigger this frame (movement and look are the controller's `FpsInput`).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct PlayerIntent {
    /// Trigger held.
    pub fire: bool,
    pub reload: bool,
}

/// When `true`, the mouse/keyboard no longer write [`PlayerIntent`] (the control port does).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ExternalControl(pub bool);

#[derive(Component)]
pub struct Player;

/// Capsule centre for a player standing on the map's spawn.
fn spawn_position(arena: &ArenaRes) -> Vec3 {
    to_world(arena.0.player_spawn, controller().height * 0.5)
}

fn spawn_player(mut commands: Commands, arena: Res<ArenaRes>, headless: Res<Headless>, settings: Res<CoreSettings>) {
    let ctl = controller();
    let eye = Transform::from_xyz(0.0, ctl.eye_offset(false), 0.0);
    // Headless: no render components, but the transform is still the aim ray.
    let camera = if headless.0 {
        commands.spawn((MainCamera, eye)).id()
    } else {
        commands
            .spawn((
                MainCamera,
                Camera3d::default(),
                Projection::from(PerspectiveProjection { fov: settings.fov_deg.to_radians(), near: 0.03, ..default() }),
                crate::gfx::camera_look(),
                eye,
            ))
            .id()
    };
    commands
        .spawn(ctl.bundle(spawn_position(&arena)))
        .insert((Name::new("Player"), Player, Actor, Health::new(PLAYER_MAX_HP)))
        .add_child(camera);
}

fn reset_player(
    arena: Res<ArenaRes>,
    mut q: Query<(&FpsController, &mut Transform, &mut FpsLook, &mut FpsMotor, &mut FpsInput, &mut Health, &mut Collider), With<Player>>,
) {
    for (ctl, mut tf, mut look, mut motor, mut input, mut hp, mut col) in &mut q {
        tf.translation = spawn_position(&arena);
        *look = FpsLook::default();
        *motor = FpsMotor { eye_offset: ctl.eye_offset(false), ..default() };
        *input = FpsInput::default();
        hp.hp = hp.max;
        *col = ctl.collider(false);
    }
}

fn clear_intent(mut intent: ResMut<PlayerIntent>, mut inputs: Query<&mut FpsInput, With<Player>>) {
    *intent = PlayerIntent::default();
    for mut i in &mut inputs {
        *i = FpsInput::default();
    }
}

fn trigger_input(
    external: Res<ExternalControl>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    captured: Res<CursorCaptured>,
    mut intent: ResMut<PlayerIntent>,
) {
    if external.0 {
        return;
    }
    *intent = PlayerIntent { fire: captured.0 && mouse.pressed(MouseButton::Left), reload: keys.pressed(KeyCode::KeyR) };
}

/// Safety net: never fall out of the world.
fn stay_in_world(mut q: Query<(&FpsController, &mut Transform, &mut FpsMotor), With<Player>>) {
    for (ctl, mut tf, mut motor) in &mut q {
        if tf.translation.y < -5.0 {
            tf.translation.y = if motor.crouched { ctl.crouch_height } else { ctl.height } * 0.5;
            motor.velocity = Vec3::ZERO;
        }
    }
}
