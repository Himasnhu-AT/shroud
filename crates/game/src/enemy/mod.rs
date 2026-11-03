//! Enemies: spawn the evolved squad, feed each `brain::mind::EnemyMind` real
//! perception (raycasts), and apply its actions (movement, aim, hitscan fire).

pub mod animation;
pub mod debug;
pub mod grenade;
pub mod gun;
pub mod skin;

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use brain::difficulty::{Difficulty, DifficultyParams};
use brain::evolve::EnemyReport;
use brain::limits::Limits;
use brain::math::V2;
use brain::posture;
use brain::mind::{Action, AllyInfo, EnemyMind, MindCtx, Observation};
use brain::squad::{MemberView, SquadCoordinator, UtilityBudget, callsign};
use brain::utility::Goal;
use brain::perception::{self, Heard, RELOAD_RANGE, Sound};
use brain::sim::{CROUCH_SPEED, SPRINT, VISION_HALF_ANGLE, VISION_RANGE, WALK};
pub use brain::loadout::{GunState, Role};

use crate::armed::Armed;
use crate::assets::{AssetsDir, ENEMY_MODEL};
use crate::map::{ArenaRes, to_ground, to_world};
use crate::player::{Player, collide_and_slide};
use engine::fps::FpsMotor;
use crate::save::Brain;
use crate::settings::ShroudSettings;
use crate::state::{GameState, Headless, not_paused};
use crate::weapon::{Damage, Health, Hitbox, Rifle, ShotFired, WeaponSet};
use grenade::ThrowGrenade;
use gun::{EnemyFx, GunLink};

pub const ENEMY_HP: f32 = 100.0;
const EYE: f32 = 1.6;
const CROUCH_EYE: f32 = 1.0;
const HEAD_Y: f32 = 1.62;
const CROUCH_HEAD_Y: f32 = 1.0;
pub(crate) const ENGAGE_MEMORY: f32 = 1.5;
/// After first spotting the player, an enemy glints and holds fire this long (fairness telegraph).
const TELEGRAPH_S: f32 = 0.25;
/// Pre-fire aims at chest height of a standing player at the predicted spot.
const PREFIRE_AIM_Y: f32 = 1.15;
/// Lean roll of the model (radians at full lean).
const LEAN_ROLL: f32 = 0.3;
/// Enemy tracers that hit the player stop this far short, so nothing draws at the camera.
const TRACER_CLEARANCE: f32 = 1.5;
/// Near-misses stop a bit further out: a streak right past the face reads as noise, not a shot.
const TRACER_MISS_CLEARANCE: f32 = 2.5;

pub struct EnemyPlugin;

impl Plugin for EnemyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Squad>()
            .add_message::<Callout>()
            .init_resource::<EnemyFreeze>()
            .init_resource::<LeanOverride>()
            // The AI and enemy movement run on the fixed physics tick (Godot's
            // `_physics_process`, Unreal's async physics tick): their raycasts and capsule
            // slides no longer scale with frame rate, and hitboxes move before the physics
            // step, so the collider tree bullets test is never a tick stale. Rendering eases
            // between ticks (`TransformInterpolation` on the enemy and its model).
            // Paused (Esc menu): the AI doesn't advance.
            .configure_sets(FixedUpdate, EnemySet.run_if(in_state(GameState::Playing).and_then(not_paused)))
            .init_resource::<Inbox>()
            .init_resource::<AiTimings>()
            .add_systems(
                FixedUpdate,
                (collect_inbox, start_ai_clock, think_and_act, stop_ai_clock, sync_pose).chain().in_set(EnemySet),
            )
            .add_systems(Update, enemy_deaths.after(WeaponSet).run_if(in_state(GameState::Playing)))
            .add_systems(Update, despawn_corpses.run_if(not_paused))
            // Round over: score the bandit's last engagement window and keep what it learned.
            .add_systems(OnExit(GameState::Playing), finish_round)
            .add_plugins((debug::EnemyDebugPlugin, animation::EnemyAnimationPlugin, gun::EnemyGunPlugin, grenade::GrenadePlugin, skin::EnemySkinPlugin));
    }
}

/// A squad callout (bark) for the subtitles and the radio click. ASCII only.
#[derive(Message, Clone, Debug)]
pub struct Callout {
    pub speaker: &'static str,
    pub text: String,
    /// The speaker's loadout role ("Marksman"), if it is an enemy.
    pub role: Option<&'static str>,
}

/// Enemy AI and movement (`FixedUpdate`, before the physics step).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnemySet;

/// What the AI cost this frame (metrics in dev builds): the whole think step and the squad
/// coordinator's tick inside it.
#[derive(Resource, Default, Debug)]
pub struct AiTimings {
    /// The AI ran this frame (not paused, a round is live).
    pub ran: bool,
    pub think_ms: f32,
    pub squad_ms: f32,
    started: Option<std::time::Instant>,
}

fn start_ai_clock(mut t: ResMut<AiTimings>) {
    t.started = Some(std::time::Instant::now());
    t.squad_ms = 0.0;
}

fn stop_ai_clock(mut t: ResMut<AiTimings>) {
    if let Some(s) = t.started.take() {
        t.think_ms = s.elapsed().as_secs_f32() * 1000.0;
        t.ran = true;
    }
}

/// Test hook: frozen enemies still perceive but don't move or shoot.
#[derive(Resource, Default)]
pub struct EnemyFreeze(pub bool);

/// Test hook: force every enemy's lean (-1 left .. +1 right) to check the pose visually.
#[derive(Resource, Default)]
pub struct LeanOverride(pub Option<f32>);

#[derive(Component)]
pub struct Enemy {
    pub slot: usize,
}

#[derive(Component)]
pub struct Mind(pub EnemyMind);

#[derive(Component)]
pub struct EnemyCombat {
    /// Primary + sidearm: magazines, reload, cycle and switch timers (`brain::loadout`).
    /// `gun.held` is the gun in hand, `gun.primary` the loadout; `gun.reloading()`.
    pub gun: GunState,
    /// What the loadout makes this enemy (rifleman, rusher, breacher, marksman).
    pub role: Role,
    /// Trigger pulls and pulls that hit the player (scoreboard accuracy).
    pub shots: u32,
    pub shots_hit: u32,
    pub hit_t: f32,
    pub last_seen: f32,
    pub last_known: V2,
    pub heard: Option<V2>,
    pub sees_player: bool,
    pub crouched: bool,
    pub vel: V2,
    pub dead: bool,
    /// Seconds since this enemy last fired.
    pub last_shot: f32,
    /// Hits taken so far, and whether the latest was a headshot (drives hit reactions).
    pub hits: u32,
    pub hit_head: bool,
    /// Seconds since the current engagement began (the player was spotted after being lost).
    pub engage_t: f32,
    /// World point the gun should point at while engaged.
    pub aim_at: Option<Vec3>,
    pub report: EnemyReport,
    /// Lean out from cover: -1 left … +1 right (local), smoothed.
    pub lean: f32,
    /// Was pre-firing last frame (for the telegraph glint).
    pub prefiring: bool,
}

impl EnemyCombat {
    /// Fighting the player: saw them within the last [`ENGAGE_MEMORY`] seconds (metrics).
    #[cfg(feature = "dev")]
    pub fn engaged(&self) -> bool {
        !self.dead && self.last_seen < ENGAGE_MEMORY
    }
}

/// The glTF model under an enemy root (leans with the body).
#[derive(Component)]
pub struct ModelRoot;

#[derive(Component)]
pub struct BodyPart {
    pub head: bool,
}

/// Round bookkeeping: reports of enemies that died this round.
#[derive(Resource, Default)]
pub struct Squad {
    pub dead_reports: Vec<EnemyReport>,
    /// Squad layer: radio-delayed blackboard, belief grid, plays and callouts (`brain::squad`).
    pub coord: Option<SquadCoordinator>,
    /// Number of enemies spawned this round (coordinator slots).
    pub size: usize,
    /// This round's difficulty and what it set.
    pub difficulty: Difficulty,
    pub params: Option<DifficultyParams>,
    pub limits: Option<Limits>,
}

struct Visuals {
    body: Handle<Mesh>,
    head: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
}

/// Despawn any old enemies and spawn the brain's current squad. Called on each round start.
pub fn spawn_squad(
    mut commands: Commands,
    old: Query<Entity, With<Enemy>>,
    arena: Res<ArenaRes>,
    brain: Res<Brain>,
    headless: Res<Headless>,
    assets_dir: Res<AssetsDir>,
    asset_server: Option<Res<AssetServer>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut squad: ResMut<Squad>,
    flow: Res<crate::match_flow::MatchFlow>,
    settings: Option<Res<ShroudSettings>>,
    telemetry: Option<Res<crate::telemetry::Telemetry>>,
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    squad.dead_reports.clear();
    squad.coord = None;
    let model = match (&asset_server, headless.0) {
        (Some(server), false) if assets_dir.has(ENEMY_MODEL) => Some(server.load::<bevy::world_serialization::WorldAsset>(
            bevy::gltf::GltfAssetLabel::Scene(0).from_asset(ENEMY_MODEL),
        )),
        _ => None,
    };
    let visuals = match (headless.0, meshes, materials) {
        (false, Some(mut m), Some(mut s)) => Some(Visuals {
            body: m.add(Capsule3d::new(0.35, 0.75)),
            head: m.add(Sphere::new(0.2)),
            mat: s.add(StandardMaterial { base_color: Color::srgb(0.8, 0.22, 0.2), perceptual_roughness: 0.6, ..default() }),
        }),
        _ => None,
    };

    let seed = flow.seed();
    // Difficulty (headless tests have no settings: Regular).
    let difficulty = settings.map_or(Difficulty::Regular, |s| s.difficulty);
    let params = difficulty.params();
    let limits = difficulty.limits(brain.state.generation);
    let n = params.squad_size.min(arena.0.enemy_spawns.len());
    // More enemies than evolved squad slots: slots are reused with their own seeds.
    // Guns from the evolved weapon genes, within this difficulty's caps (`brain::loadout`).
    let minds = brain.state.squad_minds_on(seed, n, &limits, difficulty);
    squad.size = minds.len();
    squad.difficulty = difficulty;
    squad.params = Some(params);
    squad.limits = Some(limits);
    let mut coord = SquadCoordinator::new(&arena.0, squad.size);
    coord.enabled = params.coordination;
    coord.max_tokens = params.max_attackers;
    coord.utility = if params.utility {
        let (frags, flashes) = difficulty.grenades();
        UtilityBudget { frags, flashes }
    } else {
        UtilityBudget::NONE
    };
    coord.tactics = brain.state.tactics.clone();
    // Re-peeks from earlier rounds of this match: the anchor watches those corners from the start.
    if let Some(t) = &telemetry {
        let spots: Vec<(V2, u32)> = t.0.duels.iter().filter(|d| d.repeek).map(|d| (V2::new(d.pos[0], d.pos[1]), 1)).collect();
        coord.seed_peeks(&spots);
    }
    info!("squad: {} x{} ({} coordination, {:?} utility)", difficulty.label(), squad.size, if params.coordination { "with" } else { "no" }, coord.utility);
    squad.coord = Some(coord);
    for (slot, (mind, spawn)) in minds.into_iter().zip(arena.0.enemy_spawns.iter()).enumerate() {
        let report = EnemyReport { trait_id: mind.trait_id, neat_id: mind.neat_id, ..default() };
        let kind = mind.weapon();
        info!("  {} (slot {slot}): {} with the {}", callsign(slot), mind.role().label(), kind.label());
        let root = commands
            .spawn((
                Name::new(format!("Enemy {slot}")),
                Enemy { slot },
                engine::render_budget::Actor,
                TransformInterpolation,
                Armed(kind),
                EnemyCombat {
                    gun: GunState::new(kind),
                    role: mind.role(),
                    shots: 0,
                    shots_hit: 0,
                    hit_t: 99.0,
                    last_seen: 99.0,
                    last_known: arena.0.player_spawn,
                    heard: None,
                    sees_player: false,
                    crouched: false,
                    vel: V2::ZERO,
                    dead: false,
                    last_shot: 99.0,
                    hits: 0,
                    hit_head: false,
                    engage_t: 99.0,
                    aim_at: None,
                    report,
                    lean: 0.0,
                    prefiring: false,
                },
                Mind(mind),
                Health::new(ENEMY_HP),
                RigidBody::Kinematic,
                Transform::from_translation(to_world(*spawn, 0.0)).looking_to(Vec3::Z, Vec3::Y),
                Visibility::default(),
            ))
            .id();
        let mut body = commands.spawn((
            BodyPart { head: false },
            Hitbox { owner: root, head: false },
            Collider::capsule(0.35, 0.75),
            Transform::from_xyz(0.0, 0.725, 0.0),
            ChildOf(root),
        ));
        if let (Some(v), None) = (&visuals, &model) {
            body.insert((Mesh3d(v.body.clone()), MeshMaterial3d(v.mat.clone())));
        }
        let mut head = commands.spawn((
            BodyPart { head: true },
            Hitbox { owner: root, head: true },
            Collider::sphere(0.2),
            Transform::from_xyz(0.0, HEAD_Y, 0.0),
            ChildOf(root),
        ));
        if let (Some(v), None) = (&visuals, &model) {
            head.insert((Mesh3d(v.head.clone()), MeshMaterial3d(v.mat.clone())));
        }
        if let Some(scene) = &model {
            commands.spawn((
                ModelRoot,
                // Its lean roll changes on the fixed tick: ease it too.
                TransformInterpolation,
                bevy::world_serialization::WorldAssetRoot(scene.clone()),
                Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                ChildOf(root),
            ));
        }
    }
}

/// What enemies heard/felt since last frame (a system can't read and write the same message type).
#[derive(Resource, Default)]
struct Inbox {
    /// (muzzle, direction) of each player shot.
    /// (muzzle, direction, heard within m) of each player shot.
    player_shots: Vec<(Vec3, Vec3, f32)>,
    hits: Vec<(Entity, bool)>,
}

fn collect_inbox(
    mut inbox: ResMut<Inbox>,
    players: Query<Entity, With<Player>>,
    mut shots: MessageReader<ShotFired>,
    mut damage: MessageReader<Damage>,
) {
    let player = players.single().ok();
    inbox.player_shots = shots.read().filter(|s| Some(s.by) == player).map(|s| (s.origin, s.dir, s.loudness_m)).collect();
    inbox.hits = damage.read().filter(|d| Some(d.source) == player).map(|d| (d.target, d.headshot)).collect();
}

fn v2_dir3(d: V2) -> Vec3 {
    Vec3::new(d.x, 0.0, d.y)
}

/// Line of sight from `eye` to the player: true if a ray reaches the player's collider.
fn ray_reaches(spatial: &SpatialQuery, filter: &SpatialQueryFilter, eye: Vec3, target: Vec3, player: Entity) -> bool {
    let d = target - eye;
    let Ok(dir) = Dir3::new(d) else { return true };
    spatial.cast_ray(eye, dir, d.length() + 0.5, true, filter).is_some_and(|h| h.entity == player)
}

/// Distance along a unit ray at which it enters a sphere, if it does (in front of the origin).
fn ray_sphere_enter(origin: Vec3, dir: Vec3, center: Vec3, r: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let disc = b * b - (oc.length_squared() - r * r);
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0).then_some(t)
}

fn think_and_act(
    time: Res<Time>,
    spatial: SpatialQuery,
    arena: Res<ArenaRes>,
    brain: Res<Brain>,
    players: Query<(Entity, &Transform, &FpsMotor, &Health, Option<&Rifle>), With<Player>>,
    mut squad: ResMut<Squad>,
    mut enemies: Query<
        (Entity, &mut Transform, &mut Mind, &mut EnemyCombat, &Children, &Health, Option<&GunLink>),
        (With<Enemy>, Without<Player>),
    >,
    globals: Query<&GlobalTransform>,
    inbox: Res<Inbox>,
    freeze: Res<EnemyFreeze>,
    lean_override: Res<LeanOverride>,
    mut out: AiOut,
    slots: Query<&Enemy>,
    mut hurt: Local<(f32, f32)>,
    mut timings: ResMut<AiTimings>,
    mut scratch: Local<AiScratch>,
) {
    let AiOut { damage: damage_out, shots: shots_out, fx, callouts, throws: throws_out } = &mut out;
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let Ok((player, ptf, motor, php, rifle)) = players.single() else { return };
    let AiScratch { sounds, heard_events, reveals, snapshot, allies_buf, rng } = &mut *scratch;
    let rng = rng.get_or_insert_with(|| brain::math::rng_from_seed(0x5407));
    let player_reloading = rifle.is_some_and(|r| r.reloading.is_some());
    let p_center = ptf.translation;
    let p_ground = to_ground(p_center);
    // Aim inside the capsule (its very top grazes): ~1.45 m standing, ~0.95 m crouched.
    let p_head = p_center + Vec3::Y * if motor.crouched { 0.25 } else { 0.55 };
    let p_fwd = ptf.forward();
    let p_facing = V2::new(p_fwd.x, p_fwd.z).angle();
    let p_vel = V2::new(motor.velocity.x, motor.velocity.z);
    // The player just took damage (a window to push): (last hp, seconds since hurt).
    hurt.1 += dt;
    if php.hp < hurt.0 {
        hurt.1 = 0.0;
    }
    hurt.0 = php.hp;
    let player_hurt = hurt.1 < 0.8;

    let player_shots = &inbox.player_shots;
    let hits = &inbox.hits;
    // Stimuli the player makes this frame (perception.rs): gunshots, footsteps, reload.
    sounds.clear();
    // Each shot carries as far as its gun is loud (`ShotFired::loudness_m`, scaled like the sim).
    sounds.extend(player_shots.iter().map(|(o, _, l)| (Sound::Gunshot, to_ground(*o), perception::gunshot_range_m(*l))));
    let steps = if motor.grounded { perception::footstep_range(p_vel.len(), motor.crouched) } else { 0.0 };
    if steps > 0.0 {
        sounds.push((Sound::Footstep, p_ground, steps));
    }
    if player_reloading {
        sounds.push((Sound::Reload, p_ground, RELOAD_RANGE));
    }
    heard_events.clear();
    reveals.clear();
    snapshot.clear();
    snapshot.extend(
        enemies
            .iter()
            .filter(|e| !e.3.dead)
            .map(|(e, tf, m, c, _, _, _)| (e, to_ground(tf.translation), c.last_seen < ENGAGE_MEMORY, m.0.goal() == Goal::Flank)),
    );
    // --- Squad layer: one coordinator tick before the minds think (perception from last frame).
    let n = squad.size;
    let mut views: Vec<Option<MemberView>> = vec![None; n];
    // Where each slot's thrower would release a grenade from (its hand).
    let mut hands: Vec<Option<(Entity, Vec3)>> = vec![None; n];
    for (e, tf, mut mind, mut c, _, hp, _) in &mut enemies {
        let Ok(slot) = slots.get(e).map(|s| s.slot) else { continue };
        if slot < n && !c.dead {
            hands[slot] = Some((e, tf.translation + Vec3::Y * 1.65 + tf.forward() * 0.35 + tf.right() * 0.2));
        }
        if !c.dead {
            let pos = to_ground(tf.translation);
            // Damage is a sense: being hit reveals where the shooter stands, to everyone.
            if hits.iter().any(|h| h.0 == e) {
                reveals.push(slot);
            }
            let near_miss = player_shots.iter().any(|(o, d, _)| perception::near_miss(&arena.0, to_ground(*o), V2::new(d.x, d.z).norm(), pos));
            let h = if near_miss { Some(Heard { sound: Sound::Gunshot, pos: p_ground, loud: 1.0 }) } else { perception::listen(&arena.0, sounds, pos) };
            if let Some(h) = h {
                c.heard = Some(h.pos);
                heard_events.push((slot, h));
            }
        }
        if slot < n {
            views[slot] = Some(MemberView {
                pos: to_ground(tf.translation),
                alive: !c.dead,
                hp: hp.hp / hp.max,
                last_known: c.last_known,
                last_seen: c.last_seen,
                heard: c.heard,
                look: V2::new(tf.forward().x, tf.forward().z),
                traits: mind.0.traits,
                bark: if c.dead { None } else { mind.0.take_bark() },
            });
        }
    }
    let limits = squad.limits.unwrap_or_else(|| brain.state.limits());
    let Some(coord) = squad.coord.as_mut() else { return };
    coord.player_hp = php.hp / php.max;
    let views: Vec<MemberView> = views.into_iter().map(|v| v.unwrap_or_else(MemberView::gone)).collect();
    for &slot in reveals.iter() {
        coord.reveal(slot, p_ground, &arena.0);
    }
    for (slot, h) in heard_events.iter() {
        coord.hear(*slot, h, &arena.0);
    }
    let squad_t = std::time::Instant::now();
    let orders = coord.tick(dt, &views, &arena.0, &brain.state.player_model).to_vec();
    timings.squad_ms = squad_t.elapsed().as_secs_f32() * 1000.0;
    // Always drain (a frozen squad's queued callouts would play stale later); speak if live.
    let calls = coord.drain_callouts();
    if !freeze.0 {
        for c in calls {
            let role = views.get(c.speaker).filter(|v| v.alive).map(|v| Role::of(v.traits.weapon).label());
            callouts.write(Callout { speaker: callsign(c.speaker), text: c.text, role });
        }
    }
    // Breachers releasing grenades this frame.
    for t in coord.take_throws() {
        if let Some(Some((thrower, from))) = hands.get(t.thrower)
            && !freeze.0
        {
            throws_out.write(ThrowGrenade { thrower: *thrower, kind: t.kind, from: *from, target: Vec3::new(t.target.x, 0.1, t.target.y) });
        }
    }
    let board = coord.board;
    let tokens = coord.tokens.clone();
    let cx = MindCtx { arena: &arena.0, model: &brain.state.player_model, limits };
    let mut player_hp_left = php.hp;
    let mut dealt = 0.0;
    // SHROUD_LOG_AI=1: log every enemy's decision about once per second.
    let log_ai = std::env::var_os("SHROUD_LOG_AI").is_some() && (time.elapsed_secs() % 1.0) < dt;

    for (e, mut tf, mut mind, mut c, children, hp, gun_link) in &mut enemies {
        if c.dead {
            continue;
        }
        let pos = to_ground(tf.translation);
        // Leaning out past a corner moves the eye (and the hitboxes, in `sync_pose`) sideways.
        let eye = tf.translation + Vec3::Y * if c.crouched { CROUCH_EYE } else { EYE } + tf.right() * (c.lean * posture::LEAN_M);
        let mut excluded: Vec<Entity> = children.iter().collect();
        excluded.push(e);
        let filter = SpatialQueryFilter::from_excluded_entities(excluded);

        // --- Perception.
        c.hit_t += dt;
        if let Some(&(_, head)) = hits.iter().rev().find(|h| h.0 == e) {
            c.hit_t = 0.0;
            c.hits += hits.iter().filter(|h| h.0 == e).count() as u32;
            c.hit_head = head;
        }
        let look = V2::new(tf.forward().x, tf.forward().z);
        let to_p = p_ground - pos;
        let dist = to_p.len();
        let in_cone = dist < 5.0 || c.hit_t < 1.0 || brain::math::angle_diff(to_p.angle(), look.angle()).abs() < VISION_HALF_ANGLE;
        let sees = dist < VISION_RANGE
            && in_cone
            && (ray_reaches(&spatial, &filter, eye, p_head, player) || ray_reaches(&spatial, &filter, eye, p_center, player));
        c.sees_player = sees;
        c.engage_t += dt;
        if sees && c.last_seen >= ENGAGE_MEMORY {
            // New engagement: telegraph before the first shot.
            c.engage_t = 0.0;
            if !freeze.0 {
                fx.write(EnemyFx::Glint { shooter: e, at: gun::muzzle_point(&tf, c.crouched, gun_link, &globals) });
            }
        }
        c.aim_at = (c.last_seen < ENGAGE_MEMORY || sees).then(|| {
            let target = if sees { p_ground } else { c.last_known };
            Vec3::new(target.x, p_center.y + 0.2, target.y)
        });
        if sees {
            c.last_seen = 0.0;
            c.last_known = p_ground;
        } else {
            c.last_seen += dt;
        }
        c.report.time_alive += dt;
        c.report.goal_time[brain::evolve::goal_index(mind.0.goal())] += dt;
        if c.last_seen < ENGAGE_MEMORY {
            c.report.time_engaged += dt;
        }

        let shared = board.merge(c.last_known, c.last_seen);
        let obs = Observation {
            dt,
            pos,
            hp: hp.hp / hp.max,
            ammo_frac: c.gun.ammo_frac(),
            crouched: c.crouched,
            player_visible: sees,
            player_pos: shared.0,
            player_vel: if sees { p_vel } else { V2::ZERO },
            player_hp: php.hp / php.max,
            player_facing: p_facing,
            time_since_seen: shared.1,
            heard_shot: c.heard,
            allies: {
                // Reused buffer: handed to the observation and taken back after `think`.
                let mut v = std::mem::take(allies_buf);
                v.clear();
                v.extend(snapshot.iter().filter(|a| a.0 != e).map(|a| AllyInfo { pos: a.1, engaged: a.2, flanking: a.3 }));
                v
            },
            taking_fire: c.hit_t < 0.6,
            player_reloading,
            player_hurt,
            squad_alert: board.alert,
            token_denied: slots.get(e).is_ok_and(|en| !tokens.get(en.slot).copied().unwrap_or(true)),
            weapon: c.gun.held,
            primary_ammo: c.gun.primary_frac(),
            busy_s: c.gun.busy_s(),
        };
        if let Ok(en) = slots.get(e) {
            mind.0.set_order(orders.get(en.slot).copied().flatten());
        }
        let act: Action = if freeze.0 { Action::default() } else { mind.0.think(&obs, &cx) };
        *allies_buf = obs.allies;
        // Lean out of tall cover toward where the squad thinks he is (or the angle it holds).
        let lean_want = if let Some(l) = lean_override.0 {
            l
        } else if act.lean != 0.0 {
            // The mind is peeking out of a post: lean the way it steps.
            act.lean.clamp(-1.0, 1.0)
        } else if !freeze.0 && act.move_dir.len() < 0.35 && matches!(mind.0.goal(), Goal::Hold | Goal::Attack | Goal::Ambush) {
            let target = mind.0.order().and_then(|o| o.watch).filter(|_| !sees && c.last_seen > ENGAGE_MEMORY).unwrap_or(shared.0);
            let side = posture::lean_side(&arena.0, pos, target);
            let world = (target - pos).norm().perp() * side;
            world.dot(act.look_dir.norm().perp()).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        c.lean += (lean_want - c.lean) * (1.0 - (-8.0 * dt).exp());
        // Pre-fire: the gun points at the spot; a glint telegraphs the burst.
        if let Some(pf) = act.prefire {
            c.aim_at = Some(Vec3::new(pf.x, PREFIRE_AIM_Y, pf.y));
            if !c.prefiring {
                fx.write(EnemyFx::Glint { shooter: e, at: gun::muzzle_point(&tf, c.crouched, gun_link, &globals) });
            }
        }
        c.prefiring = act.prefire.is_some();
        if log_ai {
            info!(
                "ai {e}: pos ({:.1},{:.1}) goal {:?} sees {} hp {:.0} path {} dest {:?} move ({:.2},{:.2}) fire {} seen_t {:.2} aim_off {:.3} gate {:.2} ammo {} reload {:.1} react {:.2}",
                pos.x, pos.y, mind.0.goal(), sees, hp.hp, mind.0.debug.path.len(), mind.0.debug.dest, act.move_dir.x, act.move_dir.y, act.fire,
                mind.0.debug.seen_time, mind.0.debug.aim_off, mind.0.debug.micro[2], c.gun.ammo(), c.gun.reload_t, mind.0.traits.reaction_time
            );
        }

        // --- Movement (horizontal collide-and-slide; the floor is flat).
        c.crouched = act.crouch;
        let speed = if c.crouched { CROUCH_SPEED } else if act.sprint { SPRINT } else { WALK };
        // Hard standoff with the true player position (the mind also keeps it from its belief).
        c.vel = if freeze.0 { V2::ZERO } else { brain::mind::standoff(act.move_dir, pos, p_ground) * speed };
        let shape = Collider::capsule(0.35, 0.5);
        let center = tf.translation + Vec3::Y * 0.95;
        let slid = collide_and_slide(&spatial, &shape, center, v2_dir3(c.vel) * dt, &filter);
        tf.translation = Vec3::new(slid.position.x, 0.0, slid.position.z);
        if !freeze.0 && act.look_dir.len() > 0.1 {
            tf.look_to(v2_dir3(act.look_dir), Vec3::Y);
        }

        // --- Weapon (primary + sidearm, `brain::loadout::GunState`).
        c.last_shot += dt;
        if let Some(k) = act.weapon {
            c.gun.switch_to(k);
        }
        if act.reload {
            c.gun.start_reload();
        }
        let kind = c.gun.held;
        let trigger = act.fire && c.engage_t >= TELEGRAPH_S;
        if !c.gun.tick(dt, trigger) {
            continue;
        }
        c.last_shot = 0.0;
        c.shots += 1;
        let mut muzzle = gun::muzzle_point(&tf, c.crouched, gun_link, &globals);
        if gun_link.is_none() {
            // No animated hand to follow the lean: shift the fallback muzzle with it.
            muzzle += tf.right() * (c.lean * posture::LEAN_M);
        }
        // Aimed fire pitches at the player; pre-fire at chest height on the spot.
        let aim_pt = act.prefire.map_or(p_center, |pf| Vec3::new(pf.x, PREFIRE_AIM_Y, pf.y));
        let horiz = (aim_pt - muzzle).xz().length().max(0.1);
        let pitch = (aim_pt.y - muzzle.y).atan2(horiz) + act.shot_pitch_err;
        let dir = Vec3::new(act.shot_dir.x * pitch.cos(), pitch.sin(), act.shot_dir.y * pitch.cos());
        if Dir3::new(dir).is_err() {
            continue;
        }
        shots_out.write(ShotFired::new(muzzle, dir, e, kind));
        // Every pellet is its own ray inside the gun's cone; one tracer per shot (three for a
        // shotgun blast), damage summed into one hit per trigger pull.
        let spec = kind.spec();
        let range = spec.max_range.min(brain::sim::VISION_RANGE);
        let mut amount = 0.0;
        let cam = p_center + Vec3::Y * 0.6;
        for k in 0..spec.pellets {
            let d = if act.spread > 0.0 {
                let yaw = act.shot_dir.angle() + brain::math::gauss(rng) * act.spread;
                let p = pitch + brain::math::gauss(rng) * act.spread * 0.6;
                Vec3::new(yaw.cos() * p.cos(), p.sin(), yaw.sin() * p.cos())
            } else {
                dir
            };
            let Ok(d3) = Dir3::new(d) else { continue };
            let hit = spatial.cast_ray(muzzle, d3, range, true, &filter);
            let on_player = hit.is_some_and(|h| h.entity == player);
            if on_player {
                amount += brain::loadout::enemy_damage(kind, muzzle.distance(p_center), &limits);
            }
            if k < 3 {
                // Tracer: stop where the bullet stops, but never near the player's eye.
                let mut len = hit.map_or(60.0f32.min(range), |h| h.distance);
                let clearance = if on_player { TRACER_CLEARANCE } else { TRACER_MISS_CLEARANCE };
                if let Some(t) = ray_sphere_enter(muzzle, d, cam, clearance) {
                    len = len.min(t);
                }
                let to = (muzzle.distance(cam) > clearance).then_some(muzzle + d * len);
                fx.write(EnemyFx::Shot { shooter: e, from: muzzle, to });
            }
        }
        if amount > 0.0 && player_hp_left > 0.0 {
            c.shots_hit += 1;
            damage_out.write(Damage { target: player, amount, source: e, headshot: false });
            c.report.damage_dealt += amount;
            player_hp_left -= amount;
            dealt += amount;
            if player_hp_left <= 0.0 {
                c.report.kills += 1;
                c.report.kills_by_goal[brain::evolve::goal_index(mind.0.goal())] += 1;
            }
        }
    }
    if dealt > 0.0
        && let Some(coord) = squad.coord.as_mut()
    {
        coord.player_damaged(dealt);
    }
}

/// Per-frame buffers of the AI system, reused so a frame allocates nothing (perf rule).
#[derive(Default)]
struct AiScratch {
    sounds: Vec<(Sound, V2, f32)>,
    heard_events: Vec<(usize, Heard)>,
    reveals: Vec<usize>,
    snapshot: Vec<(Entity, V2, bool, bool)>,
    allies_buf: Vec<AllyInfo>,
    /// Pellet spread.
    rng: Option<brain::math::BrainRng>,
}

/// Message writers the AI system needs (bundled to stay under the system parameter limit).
#[derive(SystemParam)]
struct AiOut<'w> {
    damage: MessageWriter<'w, Damage>,
    shots: MessageWriter<'w, ShotFired>,
    fx: MessageWriter<'w, EnemyFx>,
    callouts: MessageWriter<'w, Callout>,
    throws: MessageWriter<'w, ThrowGrenade>,
}

/// Round over: reward the open engagement window and keep the bandit for the next round/match.
fn finish_round(mut squad: ResMut<Squad>, mut brain: ResMut<Brain>, players: Query<&Health, With<Player>>) {
    let dead = players.single().is_ok_and(|h| h.is_dead());
    if let Some(c) = squad.coord.as_mut() {
        c.end_round(dead);
        brain.state.tactics = c.tactics.clone();
    }
}

/// Shrink the hitboxes while crouched so low cover hides the enemy; shift them (and roll the
/// model) sideways while leaning out of cover.
fn sync_pose(
    enemies: Query<(&EnemyCombat, &Children)>,
    mut parts: Query<(&BodyPart, &mut Transform, &mut Collider), Without<ModelRoot>>,
    mut models: Query<&mut Transform, With<ModelRoot>>,
) {
    for (c, children) in &enemies {
        let lean = if c.dead { 0.0 } else { c.lean };
        for child in children.iter() {
            if let Ok(mut tf) = models.get_mut(child) {
                // Roll about the feet: the head moves ~LEAN_M sideways at full lean.
                tf.rotation = Quat::from_rotation_z(-lean * LEAN_ROLL) * Quat::from_rotation_y(std::f32::consts::PI);
            }
            if let Ok((part, mut tf, mut col)) = parts.get_mut(child) {
                tf.translation.x = lean * posture::LEAN_M * if part.head { 1.0 } else { 0.45 };
                if part.head {
                    tf.translation.y = if c.crouched { CROUCH_HEAD_Y } else { HEAD_Y };
                } else {
                    let (half_len, y) = if c.crouched { (0.15, 0.5) } else { (0.375, 0.725) };
                    if (tf.translation.y - y).abs() > 1e-3 {
                        tf.translation.y = y;
                        *col = Collider::capsule(0.35, half_len * 2.0);
                    }
                }
            }
        }
    }
}

/// A dead enemy left lying for its death animation, then removed.
#[derive(Component)]
pub struct Corpse(f32);

fn despawn_corpses(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Corpse)>) {
    for (e, mut c) in &mut q {
        c.0 -= time.delta_secs();
        if c.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

fn enemy_deaths(
    mut commands: Commands,
    headless: Res<Headless>,
    parts: Query<(), With<BodyPart>>,
    mut enemies: Query<(Entity, &Transform, &Health, &mut EnemyCombat, &Children), With<Enemy>>,
    players: Query<&Transform, With<Player>>,
    mut squad: ResMut<Squad>,
    mut telemetry: ResMut<crate::telemetry::Telemetry>,
) {
    let p = players.single().map(|t| to_ground(t.translation)).ok();
    for (e, tf, hp, mut c, children) in &mut enemies {
        if hp.is_dead() && !c.dead {
            c.dead = true;
            c.vel = V2::ZERO;
            squad.dead_reports.push(c.report.clone());
            if let Some(p) = p {
                telemetry.0.kill(p, p.dist(to_ground(tf.translation)));
            }
            if headless.0 {
                commands.entity(e).despawn();
            } else {
                // Keep the body for the death animation; remove its hitboxes now.
                for child in children.iter().filter(|c| parts.contains(*c)) {
                    commands.entity(child).despawn();
                }
                commands.entity(e).insert(Corpse(3.0));
            }
        }
    }
}
