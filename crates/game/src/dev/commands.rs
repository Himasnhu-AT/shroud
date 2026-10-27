//! Shroud's own control commands: `game.state` (with the player), `game.start`, `game.menu`,
//! `player.*` and `debug.overlay`. The engine provides the generic ones (`engine.*`, `time.*`,
//! `screenshot`, `perf.*`, `metrics.*`, `ui.*`, `input.*`); enemy, AI and map commands live in
//! `ai_commands` and `map`.
//!
//! Player commands go through the same path as a human: `player.move` writes the controller's
//! `FpsInput` and `player.fire` the trigger ([`PlayerIntent`]) between `PlayerSet::Input` and
//! `PlayerSet::Move`, with [`ExternalControl`] on.

use std::time::Duration;

use bevy::diagnostic::FrameCount;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use engine::cli::GameSeed;
use engine::devtools::{AddControlCommand, PendingReplies, ReplyId, params};
use engine::fps::{FpsInput, FpsLook, FpsMotor};
use engine::gfx::MainCamera;
use engine::pause::PauseCtl;
use serde_json::{Value, json};

use crate::enemy::debug::DebugOverlay;
use crate::player::{ExternalControl, Player, PlayerIntent, PlayerSet};
use crate::state::{AppState, GameState};
use crate::weapon::{Damage, FireMode, Health, Rifle, ShotFired, WeaponSet};

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

/// `player.crouch` holds crouch until turned off.
#[derive(Resource, Default)]
struct HeldCrouch(bool);

/// `player.reload`: press R for one frame.
#[derive(Resource, Default)]
struct ReloadPress(bool);

/// `player.god`: the player's health is refilled every frame.
#[derive(Resource, Default)]
pub struct GodMode(pub bool);

pub(super) fn register(app: &mut App) {
    app.init_resource::<MoveJob>()
        .init_resource::<FireJob>()
        .init_resource::<HeldCrouch>()
        .init_resource::<GodMode>()
        .init_resource::<ReloadPress>()
        .configure_sets(Update, ControlDrive.after(PlayerSet::Input).before(PlayerSet::Move))
        .add_systems(Update, (drive_player.in_set(ControlDrive), (observe_fire, god_mode).after(WeaponSet)));

    let none = params(&[]);
    app.add_control_command("game.state", "Game state and the player", none.clone(), game_state)
        .add_control_command("game.start", "Start playing (restarts the round when already playing)", none.clone(), game_start)
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
            "Fire the gun in hand (semi-auto and pump/bolt guns get one trigger pull per shot)",
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
        .add_control_command(
            "debug.overlay",
            "Debug overlay",
            params(&[("on", "boolean", "default true")]),
            debug_overlay,
        )
        .add_control_command("game.menu", "Abandon the match and go to the main menu", none, game_menu);
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
    world.get_resource::<State<GameState>>().is_some_and(|s| *s.get() == GameState::Playing)
}

/// Old state names: `Menu` for the main menu, else the in-game sub-state.
fn state_name(world: &World) -> Option<String> {
    match world.get_resource::<State<AppState>>().map(|s| *s.get())? {
        AppState::InGame => world.get_resource::<State<GameState>>().map(|s| format!("{:?}", s.get())),
        AppState::MainMenu => Some("Menu".into()),
        s => Some(format!("{s:?}")),
    }
}

fn set_paused(world: &mut World, on: bool) {
    let mut st = SystemState::<PauseCtl>::new(world);
    if let Ok(mut ctl) = st.get_mut(world) {
        ctl.set(on);
    }
}

// ---- game ----

fn game_state(world: &mut World, _: Value) -> Result<Value, String> {
    let state = state_name(world);
    let player = world
        .query_filtered::<(&Transform, Option<&Health>, Option<&Rifle>, &FpsLook, &FpsMotor), With<Player>>()
        .iter(world)
        .next()
        .map(|(tf, hp, rifle, look, motor)| {
            json!({
                "pos": v3(tf.translation),
                "hp": hp.map(|h| h.hp),
                "ammo": rifle.map(|r| r.ammo),
                "weapon": rifle.map(|r| format!("{:?}", r.kind)),
                "reserve": rifle.map(|r| r.reserve),
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
    let in_game = world.get_resource::<State<AppState>>().is_some_and(|s| *s.get() == AppState::InGame);
    if in_game {
        world.resource_mut::<NextState<GameState>>().set(GameState::Playing);
    } else {
        world.resource_mut::<NextState<AppState>>().set(AppState::InGame);
    }
    set_paused(world, false);
    Ok(json!({}))
}

fn game_menu(world: &mut World, _: Value) -> Result<Value, String> {
    world.resource_mut::<crate::match_flow::MatchFlow>().abandon();
    world.resource_mut::<NextState<AppState>>().set(AppState::MainMenu);
    set_paused(world, false);
    Ok(json!({}))
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
    if let Some(mut motor) = ent.get_mut::<FpsMotor>() {
        motor.velocity = Vec3::ZERO;
    }
    if let Some(yaw) = num(&p, "yaw")
        && let Some(mut look) = ent.get_mut::<FpsLook>()
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
            .query_filtered::<&Transform, With<MainCamera>>()
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
        let look = world.get::<FpsLook>(e).ok_or("no FpsLook")?;
        (
            num(&p, "yaw").map_or(look.yaw, |v| v as f32),
            num(&p, "pitch").map_or(look.pitch, |v| v as f32),
        )
    };
    let mut look = world.get_mut::<FpsLook>(e).ok_or("no FpsLook")?;
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

/// Between `PlayerSet::Input` and `PlayerSet::Move`: write the controller input and trigger for
/// running jobs and answer finished ones.
fn drive_player(
    time: Res<Time>,
    real: Res<Time<Real>>,
    state: Option<Res<State<GameState>>>,
    held: Res<HeldCrouch>,
    mut reload: ResMut<ReloadPress>,
    mut moves: ResMut<MoveJob>,
    mut fires: ResMut<FireJob>,
    mut intent: ResMut<PlayerIntent>,
    mut external: ResMut<ExternalControl>,
    mut pending: ResMut<PendingReplies>,
    mut players: Query<(&Transform, &mut FpsInput, Option<&Rifle>), With<Player>>,
    // (trigger pulse for non-auto guns, reload pressed last frame)
    mut pulse: Local<(bool, bool)>,
) {
    let is_playing = state.is_some_and(|s| *s.get() == GameState::Playing);
    let Ok((tf, mut input, gun)) = players.single_mut() else { return };
    // Auto guns hold the trigger; the others need it released between shots.
    let auto = gun.is_none_or(|g| g.kind.spec().mode == FireMode::Auto);
    let pos = v3(tf.translation);
    let driving = moves.0.is_some() || fires.0.is_some() || held.0;

    if let Some(m) = moves.0.as_mut() {
        if !is_playing {
            pending.reply(m.reply, Err("left Playing before the move finished".into()));
            moves.0 = None;
            input.move_axis = Vec2::ZERO;
        } else if m.left.is_zero() {
            pending.reply(m.reply, Ok(json!({"pos": pos})));
            moves.0 = None;
            input.move_axis = Vec2::ZERO;
            input.sprint = false;
            if !held.0 {
                input.crouch = false;
            }
        } else {
            input.move_axis = m.axis;
            input.sprint = m.sprint;
            input.jump = false;
            if let Some(c) = m.crouch {
                input.crouch = c;
            }
            m.left = m.left.saturating_sub(time.delta());
        }
    } else if driving {
        // Firing or holding crouch: the keyboard doesn't move the player meanwhile.
        input.move_axis = Vec2::ZERO;
        input.sprint = false;
        input.jump = false;
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
            pulse.0 = !pulse.0;
            intent.fire = auto || pulse.0;
        }
    }

    if held.0 {
        input.crouch = true;
    }
    // A one-frame press: clear our own press next frame (headless has no keyboard to do it).
    if std::mem::take(&mut pulse.1) {
        intent.reload = false;
    }
    if std::mem::take(&mut reload.0) {
        intent.reload = true;
        pulse.1 = true;
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
    let gun = world.get::<Rifle>(e).ok_or("no gun")?;
    let (ammo, can) = (gun.ammo, gun.reloading.is_none() && gun.ammo < gun.mag && gun.reserve > 0);
    if can {
        // One frame of R through the same path as the key.
        world.resource_mut::<ReloadPress>().0 = true;
    }
    Ok(json!({"ammo": ammo, "reloading": can}))
}

fn player_crouch(world: &mut World, p: Value) -> Result<Value, String> {
    let on = flag(&p, "on").unwrap_or(true);
    world.resource_mut::<HeldCrouch>().0 = on;
    let e = player_entity(world)?;
    if let Some(mut input) = world.get_mut::<FpsInput>(e) {
        input.crouch = on;
    }
    Ok(json!({}))
}

fn player_god(world: &mut World, p: Value) -> Result<Value, String> {
    world.resource_mut::<GodMode>().0 = flag(&p, "on").unwrap_or(true);
    Ok(json!({}))
}

// ---- debug ----

fn debug_overlay(world: &mut World, p: Value) -> Result<Value, String> {
    world.resource_mut::<DebugOverlay>().0 = flag(&p, "on").unwrap_or(true);
    Ok(json!({}))
}
