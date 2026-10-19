//! Enemies: spawn the evolved squad, feed each `brain::mind::EnemyMind` real
//! perception (raycasts), and apply its actions (movement, aim, hitscan fire).

pub mod animation;
pub mod debug;
pub mod gun;

use avian3d::prelude::*;
use bevy::prelude::*;
use brain::evolve::EnemyReport;
use brain::math::V2;
use brain::mind::{Action, AllyInfo, EnemyMind, MindCtx, Observation};
use brain::sim::{CROUCH_SPEED, FIRE_INTERVAL, HEARING_RANGE, MAG, RELOAD_S, SPRINT, VISION_HALF_ANGLE, VISION_RANGE, WALK};

use crate::assets::{AssetsDir, ENEMY_MODEL};
use crate::map::{ArenaRes, to_ground, to_world};
use crate::player::{Player, PlayerMotor, PlayerSet, collide_and_slide};
use crate::save::Brain;
use crate::state::{GameState, Headless};
use crate::weapon::{Damage, Health, Hitbox, ShotFired, WeaponSet};
use gun::{EnemyFx, GunLink};

pub const ENEMY_HP: f32 = 100.0;
const EYE: f32 = 1.6;
const CROUCH_EYE: f32 = 1.0;
const HEAD_Y: f32 = 1.62;
const CROUCH_HEAD_Y: f32 = 1.0;
const ENGAGE_MEMORY: f32 = 1.5;
/// After first spotting the player, an enemy glints and holds fire this long (fairness telegraph).
const TELEGRAPH_S: f32 = 0.25;
/// Enemy tracers that hit the player stop this far short, so nothing draws at the camera.
const TRACER_CLEARANCE: f32 = 1.5;

pub struct EnemyPlugin;

impl Plugin for EnemyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Squad>()
            .init_resource::<EnemyFreeze>()
            .configure_sets(
                Update,
                EnemySet.after(PlayerSet::Move).before(WeaponSet).run_if(in_state(GameState::Playing)),
            )
            .init_resource::<Inbox>()
            .add_systems(Update, (collect_inbox, think_and_act, sync_crouch).chain().in_set(EnemySet))
            .add_systems(Update, enemy_deaths.after(WeaponSet).run_if(in_state(GameState::Playing)))
            .add_systems(Update, despawn_corpses)
            .add_plugins((debug::EnemyDebugPlugin, animation::EnemyAnimationPlugin, gun::EnemyGunPlugin));
    }
}

/// Enemy AI runs here: after player movement, before weapons apply damage.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnemySet;

/// Test hook: frozen enemies still perceive but don't move or shoot.
#[derive(Resource, Default)]
pub struct EnemyFreeze(pub bool);

#[derive(Component)]
pub struct Enemy {
    pub slot: usize,
}

#[derive(Component)]
pub struct Mind(pub EnemyMind);

#[derive(Component)]
pub struct EnemyCombat {
    pub ammo: u32,
    pub reload: f32,
    pub cooldown: f32,
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
}

#[derive(Component)]
pub struct BodyPart {
    pub head: bool,
}

/// Round bookkeeping: reports of enemies that died this round.
#[derive(Resource, Default)]
pub struct Squad {
    pub dead_reports: Vec<EnemyReport>,
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
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    squad.dead_reports.clear();
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
    let minds = brain.state.squad_minds(seed);
    for (slot, (mind, spawn)) in minds.into_iter().zip(arena.0.enemy_spawns.iter()).enumerate() {
        let report = EnemyReport { trait_id: mind.trait_id, neat_id: mind.neat_id, ..default() };
        let root = commands
            .spawn((
                Name::new(format!("Enemy {slot}")),
                Enemy { slot },
                Mind(mind),
                EnemyCombat {
                    ammo: MAG,
                    reload: 0.0,
                    cooldown: 0.0,
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
                },
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
    player_shots: Vec<Vec3>,
    hits: Vec<(Entity, bool)>,
}

fn collect_inbox(
    mut inbox: ResMut<Inbox>,
    players: Query<Entity, With<Player>>,
    mut shots: MessageReader<ShotFired>,
    mut damage: MessageReader<Damage>,
) {
    let player = players.single().ok();
    inbox.player_shots = shots.read().filter(|s| Some(s.by) == player).map(|s| s.origin).collect();
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
    players: Query<(Entity, &Transform, &PlayerMotor, &Health), With<Player>>,
    mut enemies: Query<
        (Entity, &mut Transform, &mut Mind, &mut EnemyCombat, &Children, &Health, Option<&GunLink>),
        (With<Enemy>, Without<Player>),
    >,
    globals: Query<&GlobalTransform>,
    inbox: Res<Inbox>,
    freeze: Res<EnemyFreeze>,
    mut damage_out: MessageWriter<Damage>,
    mut shots_out: MessageWriter<ShotFired>,
    mut fx: MessageWriter<EnemyFx>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let Ok((player, ptf, motor, php)) = players.single() else { return };
    let p_center = ptf.translation;
    let p_ground = to_ground(p_center);
    // Aim inside the capsule (its very top grazes): ~1.45 m standing, ~0.95 m crouched.
    let p_head = p_center + Vec3::Y * if motor.crouched { 0.25 } else { 0.55 };
    let p_fwd = ptf.forward();
    let p_facing = V2::new(p_fwd.x, p_fwd.z).angle();
    let p_vel = V2::new(motor.velocity.x, motor.velocity.z);

    let player_shots = &inbox.player_shots;
    let hits = &inbox.hits;

    let snapshot: Vec<(Entity, V2, bool)> = enemies
        .iter()
        .filter(|e| !e.3.dead)
        .map(|(e, tf, _, c, _, _, _)| (e, to_ground(tf.translation), c.last_seen < ENGAGE_MEMORY))
        .collect();
    let limits = brain.state.limits();
    let cx = MindCtx { arena: &arena.0, model: &brain.state.player_model, limits };
    let mut player_hp_left = php.hp;
    // SHROUD_LOG_AI=1: log every enemy's decision about once per second.
    let log_ai = std::env::var_os("SHROUD_LOG_AI").is_some() && (time.elapsed_secs() % 1.0) < dt;

    for (e, mut tf, mut mind, mut c, children, hp, gun_link) in &mut enemies {
        if c.dead {
            continue;
        }
        let pos = to_ground(tf.translation);
        let eye = tf.translation + Vec3::Y * if c.crouched { CROUCH_EYE } else { EYE };
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
        if let Some(o) = player_shots.iter().find(|o| to_ground(**o).dist(pos) < HEARING_RANGE) {
            c.heard = Some(to_ground(*o));
        }
        c.report.time_alive += dt;
        if c.last_seen < ENGAGE_MEMORY {
            c.report.time_engaged += dt;
        }

        let obs = Observation {
            dt,
            pos,
            hp: hp.hp / hp.max,
            ammo_frac: if c.reload > 0.0 { 0.0 } else { c.ammo as f32 / MAG as f32 },
            crouched: c.crouched,
            player_visible: sees,
            player_pos: c.last_known,
            player_vel: if sees { p_vel } else { V2::ZERO },
            player_hp: php.hp / php.max,
            player_facing: p_facing,
            time_since_seen: c.last_seen,
            heard_shot: c.heard,
            allies: snapshot
                .iter()
                .filter(|a| a.0 != e)
                .map(|a| AllyInfo { pos: a.1, engaged: a.2, flanking: false })
                .collect(),
            taking_fire: c.hit_t < 0.6,
        };
        let act: Action = if freeze.0 { Action::default() } else { mind.0.think(&obs, &cx) };
        if log_ai {
            info!(
                "ai {e}: pos ({:.1},{:.1}) goal {:?} sees {} hp {:.0} path {} dest {:?} move ({:.2},{:.2}) fire {} seen_t {:.2} aim_off {:.3} gate {:.2} ammo {} reload {:.1} react {:.2}",
                pos.x, pos.y, mind.0.goal(), sees, hp.hp, mind.0.debug.path.len(), mind.0.debug.dest, act.move_dir.x, act.move_dir.y, act.fire,
                mind.0.debug.seen_time, mind.0.debug.aim_off, mind.0.debug.micro[2], c.ammo, c.reload, mind.0.traits.reaction_time
            );
        }

        // --- Movement (horizontal collide-and-slide; the floor is flat).
        c.crouched = act.crouch;
        let speed = if c.crouched { CROUCH_SPEED } else if act.sprint { SPRINT } else { WALK };
        c.vel = act.move_dir * speed;
        let shape = Collider::capsule(0.35, 0.5);
        let center = tf.translation + Vec3::Y * 0.95;
        let slid = collide_and_slide(&spatial, &shape, center, v2_dir3(c.vel) * dt, &filter);
        tf.translation = Vec3::new(slid.position.x, 0.0, slid.position.z);
        if !freeze.0 && act.look_dir.len() > 0.1 {
            tf.look_to(v2_dir3(act.look_dir), Vec3::Y);
        }

        // --- Weapon.
        c.cooldown -= dt;
        c.last_shot += dt;
        if c.reload > 0.0 {
            c.reload -= dt;
            if c.reload <= 0.0 {
                c.ammo = MAG;
            }
            continue;
        }
        if act.reload && c.ammo < MAG {
            c.reload = RELOAD_S;
            continue;
        }
        if !(act.fire && c.cooldown <= 0.0 && c.ammo > 0 && c.engage_t >= TELEGRAPH_S) {
            continue;
        }
        c.cooldown = FIRE_INTERVAL;
        c.last_shot = 0.0;
        c.ammo -= 1;
        if c.ammo == 0 {
            c.reload = RELOAD_S;
        }
        let muzzle = gun::muzzle_point(&tf, c.crouched, gun_link, &globals);
        let horiz = (p_center - muzzle).xz().length().max(0.1);
        let pitch = (p_center.y - muzzle.y).atan2(horiz) + act.shot_pitch_err;
        let dir = Vec3::new(act.shot_dir.x * pitch.cos(), pitch.sin(), act.shot_dir.y * pitch.cos());
        let Ok(dir3) = Dir3::new(dir) else { continue };
        shots_out.write(ShotFired { origin: muzzle, dir, by: e });
        let hit = spatial.cast_ray(muzzle, dir3, brain::sim::VISION_RANGE, true, &filter);
        // Tracer: stop where the bullet stops, but never within TRACER_CLEARANCE of the player's eye.
        let mut len = hit.map_or(60.0, |h| h.distance);
        let cam = p_center + Vec3::Y * 0.6;
        if let Some(t) = ray_sphere_enter(muzzle, dir, cam, TRACER_CLEARANCE) {
            len = len.min(t);
        }
        let to = (muzzle.distance(cam) > TRACER_CLEARANCE).then_some(muzzle + dir * len);
        fx.write(EnemyFx::Shot { shooter: e, from: muzzle, to });
        if hit.is_some_and(|h| h.entity == player) && player_hp_left > 0.0 {
            let amount = limits.damage_per_hit;
            damage_out.write(Damage { target: player, amount, source: e, headshot: false });
            c.report.damage_dealt += amount;
            player_hp_left -= amount;
            if player_hp_left <= 0.0 {
                c.report.kills += 1;
            }
        }
    }
}

/// Shrink the hitboxes while crouched so low cover hides the enemy.
fn sync_crouch(enemies: Query<(&EnemyCombat, &Children)>, mut parts: Query<(&BodyPart, &mut Transform, &mut Collider)>) {
    for (c, children) in &enemies {
        for child in children.iter() {
            if let Ok((part, mut tf, mut col)) = parts.get_mut(child) {
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
