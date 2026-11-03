//! Grenades in 3D (the squad's and the player's): an arced projectile with ground/wall bounces
//! (avian ray casts), a fuse, frag damage with falloff and a line-of-sight check, a flashbang
//! that whites out the player's screen depending on whether he was looking at it, and smoke.
//! The squad coordinator decides enemy throws (`brain::squad::ThrowOrder`) and calls them out
//! ("Frag out!", "Flashbang!"); the player throws from `weapon::throwing`.
//!
//! A player's frag also hurts enemies (and himself); his flashbang blinds enemies that see it
//! pop (`EnemyCombat::blind`, which the mind gets as `Observation::blind`); smoke spawns a
//! [`SmokeVolume`] that blocks enemy sight (`brain::grenade::SmokeCloud`, mirrored into
//! `ArenaRes`'s `smokes` so the AI's 2D line-of-sight checks see it too).

use avian3d::prelude::*;
use bevy::prelude::*;
use brain::grenade::{self as g, GrenadeKind};

use super::{EnemyCombat, EnemySet, Squad};
use crate::map::to_ground;
use crate::map::ArenaRes;
use crate::player::Player;
use engine::fps::FpsMotor;
use crate::state::{GameState, has_visuals, not_paused};
use crate::weapon::{Damage, WeaponSet};

const GRAVITY: f32 = 9.81;
const RADIUS: f32 = 0.07;
const RESTITUTION: f32 = 0.38;
/// Horizontal speed kept per ground bounce.
const GROUND_FRICTION: f32 = 0.6;

/// Throw a grenade (written by the enemy AI when the coordinator releases a throw).
#[derive(Message, Clone, Copy, Debug)]
pub struct ThrowGrenade {
    pub thrower: Entity,
    pub kind: GrenadeKind,
    pub from: Vec3,
    pub target: Vec3,
    /// Launch velocity (the player's throws); `None` = a lob that lands on `target`.
    pub vel: Option<Vec3>,
}

/// A grenade in flight or on the ground.
#[derive(Component, Debug)]
pub struct Grenade {
    pub kind: GrenadeKind,
    pub thrower: Entity,
    pub vel: Vec3,
    pub fuse: f32,
    resting: bool,
}

/// The player's flashbang whiteout: peak intensity 0..1, seconds since, total duration.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Flashed {
    pub peak: f32,
    pub t: f32,
    pub dur: f32,
}

impl Flashed {
    /// Current overlay alpha: full white for ~40% of the duration, then fades out.
    pub fn alpha(&self) -> f32 {
        if self.dur <= 0.0 || self.t >= self.dur {
            return 0.0;
        }
        let f = self.t / self.dur;
        let hold = 0.4;
        let a = if f < hold { 1.0 } else { 1.0 - (f - hold) / (1.0 - hold) };
        (self.peak * a).clamp(0.0, 1.0)
    }
}

#[derive(Component)]
struct FlashOverlay;

/// A smoke cloud on the map (blocks enemy sight, see the module doc).
#[derive(Component, Debug, Clone, Copy)]
pub struct SmokeVolume {
    pub cloud: g::SmokeCloud,
    /// Ground point in 3D (cloud centre at floor height).
    pub at: Vec3,
}

/// One billboard puff of a smoke cloud: its offset in the cloud (at full size) and size.
#[derive(Component)]
struct SmokePuff {
    offset: Vec3,
    size: f32,
    spin: f32,
}

/// Grey screen while the camera is inside a smoke cloud.
#[derive(Component)]
struct SmokeOverlay;

/// Shared smoke visuals: the puff quad and its soft round texture material.
#[derive(Resource, Clone)]
struct SmokeAssets {
    quad: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Blast {
    ttl: f32,
    max: f32,
    size: f32,
}

/// Blinking warning light on a live frag.
#[derive(Component)]
struct FuseLight;

pub struct GrenadePlugin;

impl Plugin for GrenadePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ThrowGrenade>()
            .init_resource::<Flashed>()
            .add_systems(Startup, (spawn_overlay, init_smoke_assets).run_if(has_visuals))
            .add_systems(OnExit(GameState::Playing), clear)
            .add_systems(
                Update,
                (spawn_grenades, fly, detonate, tick_smokes)
                    .chain()
                    .after(EnemySet)
                    .before(WeaponSet)
                    .run_if(in_state(GameState::Playing).and_then(not_paused)),
            )
            .add_systems(Update, (tick_flash, tick_enemy_blind, blink, tick_blasts.run_if(has_visuals)).run_if(not_paused))
            .add_systems(Update, (update_overlay, animate_smoke).run_if(has_visuals));
    }
}

/// Launch velocity that lands at `to` after a flight time that grows with distance (a lob).
pub fn launch_velocity(from: Vec3, to: Vec3) -> Vec3 {
    let d = to - from;
    let horiz = d.xz().length();
    let t = (horiz / 11.0).clamp(0.55, 1.5);
    Vec3::new(d.x / t, (d.y + 0.5 * GRAVITY * t * t) / t, d.z / t)
}

fn spawn_grenades(
    mut commands: Commands,
    mut msgs: MessageReader<ThrowGrenade>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut assets: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>, Handle<StandardMaterial>, Handle<StandardMaterial>)>>,
) {
    let (mut meshes, mut materials) = (meshes, materials);
    for t in msgs.read() {
        let mut e = commands.spawn((
            Name::new(format!("{} grenade", t.kind.label())),
            Grenade { kind: t.kind, thrower: t.thrower, vel: t.vel.unwrap_or_else(|| launch_velocity(t.from, t.target)), fuse: t.kind.fuse(), resting: false },
            Transform::from_translation(t.from),
            Visibility::default(),
            DespawnOnExit(GameState::Playing),
        ));
        if let (Some(m), Some(s)) = (meshes.as_deref_mut(), materials.as_deref_mut()) {
            let (mesh, frag_mat, flash_mat, smoke_mat) = assets
                .get_or_insert_with(|| {
                    (
                        m.add(Sphere::new(RADIUS * 1.3)),
                        s.add(StandardMaterial { base_color: Color::srgb(0.18, 0.24, 0.12), perceptual_roughness: 0.7, ..default() }),
                        s.add(StandardMaterial { base_color: Color::srgb(0.75, 0.75, 0.7), metallic: 0.6, perceptual_roughness: 0.35, ..default() }),
                        s.add(StandardMaterial { base_color: Color::srgb(0.32, 0.36, 0.38), metallic: 0.3, perceptual_roughness: 0.5, ..default() }),
                    )
                })
                .clone();
            let mat = match t.kind {
                GrenadeKind::Frag => frag_mat,
                GrenadeKind::Flash => flash_mat,
                GrenadeKind::Smoke => smoke_mat,
            };
            e.insert((Mesh3d(mesh), MeshMaterial3d(mat), engine::render_budget::Actor));
            if t.kind == GrenadeKind::Frag {
                e.with_child((FuseLight, PointLight { color: Color::srgb(1.0, 0.15, 0.1), intensity: 0.0, range: 3.0, ..default() }, Transform::default()));
            }
        }
    }
}

/// Ballistic integration with bounces: ray-cast each step's motion; reflect off what it hits.
fn fly(time: Res<Time>, spatial: SpatialQuery, mut q: Query<(&mut Grenade, &mut Transform)>, players: Query<Entity, With<Player>>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    // Bounce off the world, not off the player's capsule (it would deflect into his face).
    let filter = SpatialQueryFilter::from_excluded_entities(players.iter());
    for (mut gr, mut tf) in &mut q {
        gr.fuse -= dt;
        if gr.resting {
            continue;
        }
        gr.vel.y -= GRAVITY * dt;
        let mut pos = tf.translation;
        let mut left = dt;
        // Up to two bounces per frame.
        for _ in 0..3 {
            let step = gr.vel * left;
            let len = step.length();
            let Ok(dir) = Dir3::new(step) else { break };
            match spatial.cast_ray(pos, dir, len + RADIUS, true, &filter) {
                Some(hit) => {
                    let travel = (hit.distance - RADIUS).max(0.0);
                    pos += dir * travel;
                    left *= 1.0 - (travel / len.max(1e-6)).min(1.0);
                    let n = hit.normal.normalize_or(Vec3::Y);
                    // Reflect: the normal part bounces back (restitution), the tangential part
                    // slides on with friction (more on the floor than off a wall).
                    let vn = gr.vel.dot(n) * n;
                    let vt = gr.vel - vn;
                    let fric = if n.y > 0.7 { GROUND_FRICTION } else { 0.8 };
                    gr.vel = vt * fric - vn * RESTITUTION;
                    pos += n * 0.005;
                    if left <= 1e-4 {
                        break;
                    }
                }
                None => {
                    pos += step;
                    break;
                }
            }
        }
        // Safety floor (the arena floor top is y = 0).
        if pos.y < RADIUS {
            pos.y = RADIUS;
            if gr.vel.y < 0.0 {
                gr.vel.y = -gr.vel.y * RESTITUTION;
                gr.vel.x *= GROUND_FRICTION;
                gr.vel.z *= GROUND_FRICTION;
            }
        }
        if pos.y <= RADIUS + 0.02 && gr.vel.length() < 0.4 {
            gr.resting = true;
            gr.vel = Vec3::ZERO;
            pos.y = RADIUS;
        }
        tf.translation = pos;
    }
}

/// Does a ray from `from` reach `player` (nothing solid in between)?
fn reaches(spatial: &SpatialQuery, from: Vec3, to: Vec3, player: Entity, exclude: &[Entity]) -> bool {
    let d = to - from;
    let Ok(dir) = Dir3::new(d) else { return true };
    let filter = SpatialQueryFilter::from_excluded_entities(exclude.iter().copied());
    spatial.cast_ray(from, dir, d.length() + 0.3, true, &filter).is_none_or(|h| h.entity == player || h.distance >= d.length() - 0.2)
}

/// Does a ray from `from` reach `to` past everything solid except `exclude`d bodies?
fn clear_line(spatial: &SpatialQuery, from: Vec3, to: Vec3, exclude: &[Entity]) -> bool {
    let d = to - from;
    let Ok(dir) = Dir3::new(d) else { return true };
    let filter = SpatialQueryFilter::from_excluded_entities(exclude.iter().copied());
    spatial.cast_ray(from, dir, d.length(), true, &filter).is_none_or(|h| h.distance >= d.length() - 0.2)
}

/// Frag damage to a body whose capsule centre is `center` (head `head`) from a blast at `at`:
/// full through a clear line to the centre, half if only the head is exposed.
fn frag_exposure(spatial: &SpatialQuery, at: Vec3, center: Vec3, head: Vec3, exclude: &[Entity]) -> f32 {
    let d = to_ground(at).dist(to_ground(center)).max((at.y - center.y).abs() * 0.5);
    let base = g::frag_damage(d);
    if base <= 0.0 {
        0.0
    } else if clear_line(spatial, at, center, exclude) {
        base
    } else if clear_line(spatial, at, head, exclude) {
        base * 0.5
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
fn detonate(
    mut commands: Commands,
    spatial: SpatialQuery,
    q: Query<(Entity, &Grenade, &Transform)>,
    players: Query<(Entity, &Transform, &FpsMotor), With<Player>>,
    mut enemies: Query<(Entity, &Transform, &mut EnemyCombat), Without<Grenade>>,
    hitboxes: Query<Entity, With<super::BodyPart>>,
    mut squad: ResMut<Squad>,
    mut flashed: ResMut<Flashed>,
    mut damage: MessageWriter<Damage>,
    sfx: Option<Res<crate::audio::Sfx>>,
    master: Option<Res<engine::audio::Volumes>>,
    smoke_assets: Option<Res<SmokeAssets>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let mut visuals = meshes.zip(materials);
    let player = players.single().ok();
    let damage_mult = squad.params.map_or(1.0, |p| p.damage_mult);
    // Bodies don't shield each other from a blast (keeps the check about walls and cover).
    let mut shields: Vec<Entity> = hitboxes.iter().collect();
    shields.extend(player.map(|p| p.0));
    for (e, gr, tf) in &q {
        // Smoke waits until it has stopped rolling (or a few seconds past its fuse).
        if gr.fuse > 0.0 || (gr.kind == GrenadeKind::Smoke && !gr.resting && gr.fuse > -3.0) {
            continue;
        }
        commands.entity(e).despawn();
        let at = tf.translation + Vec3::Y * 0.15;
        let by_player = player.is_some_and(|p| p.0 == gr.thrower);
        match gr.kind {
            GrenadeKind::Frag => {
                if let Some((pe, ptf, motor)) = player {
                    let center = ptf.translation;
                    let head = center + Vec3::Y * if motor.crouched { 0.25 } else { 0.55 };
                    // The squad's difficulty scales its own frags, not the player's.
                    let amount = frag_exposure(&spatial, at, center, head, &shields) * if by_player { 1.0 } else { damage_mult };
                    if amount > 0.5 {
                        damage.write(Damage { target: pe, amount, source: gr.thrower, headshot: false });
                        if !by_player {
                            if let Some(c) = squad.coord.as_mut() {
                                c.player_damaged(amount);
                            }
                            if let Ok((_, _, mut c)) = enemies.get_mut(gr.thrower) {
                                c.report.damage_dealt += amount;
                            }
                        }
                    }
                }
                // The player's frag hurts the squad (theirs keep clear of each other).
                if by_player {
                    for (ee, etf, c) in &enemies {
                        if c.dead {
                            continue;
                        }
                        let center = etf.translation + Vec3::Y * 0.95;
                        let head = etf.translation + Vec3::Y * if c.crouched { 1.15 } else { 1.6 };
                        let amount = frag_exposure(&spatial, at, center, head, &shields);
                        if amount > 0.5 {
                            damage.write(Damage { target: ee, amount, source: gr.thrower, headshot: false });
                        }
                    }
                }
                if let (Some(sfx), Some(master)) = (&sfx, &master) {
                    let name = if sfx.has("frag_explode") { "frag_explode" } else { "impact" };
                    crate::audio::play_at(&mut commands, sfx, master, name, 1.0, at);
                }
                if let Some((m, s)) = visuals.as_mut() {
                    spawn_blast(&mut commands, &mut *m, &mut *s, at, LinearRgba::rgb(3.2, 1.1, 0.2), 2.0, 0.35, Color::srgb(1.0, 0.6, 0.25), 900_000.0);
                }
            }
            GrenadeKind::Flash => {
                if let Some((pe, ptf, _)) = player {
                    let eye = ptf.translation + Vec3::Y * 0.6;
                    let d = at.distance(eye);
                    if d < g::FLASH_RANGE && reaches(&spatial, at, eye, pe, &shields) {
                        let facing = ptf.forward().dot((at - eye).normalize_or_zero());
                        let i = g::flash_intensity(facing, d);
                        let dur = g::flash_duration(i);
                        // A stronger flash replaces a fading one.
                        if dur > 0.0 && i >= flashed.alpha() {
                            *flashed = Flashed { peak: i, t: 0.0, dur };
                        }
                    }
                }
                // The player's flashbang blinds enemies that see it pop.
                if by_player {
                    for (_, etf, mut c) in &mut enemies {
                        if c.dead {
                            continue;
                        }
                        let eye = etf.translation + Vec3::Y * if c.crouched { 1.15 } else { 1.6 };
                        let d = at.distance(eye);
                        if d >= g::FLASH_RANGE || !clear_line(&spatial, at, eye, &shields) {
                            continue;
                        }
                        let facing = etf.forward().dot((at - eye).normalize_or_zero());
                        let dur = g::flash_duration(g::flash_intensity(facing, d));
                        if dur > c.blind {
                            c.blind = dur;
                        }
                    }
                }
                if let (Some(sfx), Some(master)) = (&sfx, &master)
                    && sfx.has("flashbang")
                {
                    crate::audio::play_at(&mut commands, sfx, master, "flashbang", 1.0, at);
                }
                if let Some((m, s)) = visuals.as_mut() {
                    spawn_blast(&mut commands, &mut *m, &mut *s, at, LinearRgba::rgb(20.0, 20.0, 22.0), 1.2, 0.18, Color::WHITE, 2_000_000.0);
                }
            }
            GrenadeKind::Smoke => {
                let ground = Vec3::new(at.x, 0.0, at.z);
                let mut v = commands.spawn((
                    Name::new("Smoke cloud"),
                    SmokeVolume { cloud: g::SmokeCloud::new(to_ground(at)), at: ground },
                    Transform::from_translation(ground),
                    Visibility::default(),
                    DespawnOnExit(GameState::Playing),
                ));
                if let Some(sa) = &smoke_assets {
                    v.with_children(|c| spawn_puffs(c, sa, e.to_bits()));
                }
                if let (Some(sfx), Some(master)) = (&sfx, &master) {
                    let name = if sfx.has("smoke_pop") { "smoke_pop" } else { "impact" };
                    crate::audio::play_at(&mut commands, sfx, master, name, 0.6, at);
                }
            }
        }
    }
}

/// Puffs of one cloud: a low ring that hugs the floor, a fuller middle and a few on top, so it
/// reads as a dome ~3 m tall. Deterministic jitter from `seed`.
fn spawn_puffs(c: &mut ChildSpawnerCommands, sa: &SmokeAssets, seed: u64) {
    let mut h = seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut rnd = move || {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        (h % 10_000) as f32 / 10_000.0
    };
    let r = g::SMOKE_RADIUS;
    let rings: [(usize, f32, f32, f32); 4] = [(8, 0.8, 0.9, 2.7), (8, 0.55, 1.8, 3.0), (6, 0.32, 2.6, 2.8), (3, 0.12, 3.2, 2.4)];
    for (n, rad, y, size) in rings {
        for k in 0..n {
            let a = k as f32 / n as f32 * std::f32::consts::TAU + rnd() * 0.6;
            let rr = r * rad * (0.85 + 0.3 * rnd());
            let offset = Vec3::new(a.cos() * rr, y + rnd() * 0.3, a.sin() * rr);
            c.spawn((
                SmokePuff { offset, size: size * (0.85 + 0.3 * rnd()), spin: rnd() * std::f32::consts::TAU },
                Mesh3d(sa.quad.clone()),
                MeshMaterial3d(sa.mat.clone()),
                Transform::from_translation(offset * 0.1).with_scale(Vec3::splat(0.1)),
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
                engine::render_budget::Actor,
            ));
        }
    }
}

/// Age smoke clouds, drop spent ones, and mirror them into the arena (AI line of sight).
fn tick_smokes(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut SmokeVolume)>, mut arena: ResMut<ArenaRes>) {
    let dt = time.delta_secs();
    let mut live = Vec::new();
    for (e, mut s) in &mut q {
        s.cloud.age += dt;
        if s.cloud.done() {
            commands.entity(e).despawn();
        } else {
            live.push(s.cloud);
        }
    }
    if arena.0.smokes != live {
        arena.0.smokes = live;
    }
}

fn tick_enemy_blind(time: Res<Time>, mut q: Query<&mut EnemyCombat>) {
    let dt = time.delta_secs();
    for mut c in &mut q {
        if c.blind > 0.0 {
            c.blind = (c.blind - dt).max(0.0);
        }
    }
}

fn init_smoke_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    // Soft round puff with a little lumpy edge so overlapping quads don't read as discs.
    let size = 64u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let r = (u * u + v * v).sqrt();
            let ang = v.atan2(u);
            let lump = 0.12 * ((ang * 3.0).sin() * 0.6 + (ang * 7.0 + 1.3).sin() * 0.4);
            let a = (1.0 - (r / (0.95 + lump)).powf(2.0)).clamp(0.0, 1.0).powf(0.9);
            // Slightly lighter on top.
            let shade = 0.85 + 0.15 * (1.0 - (v + 1.0) * 0.5);
            let c = (shade * 255.0) as u8;
            data.extend_from_slice(&[c, c, c, (a * 255.0) as u8]);
        }
    }
    let img = Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    let tex = images.add(img);
    commands.insert_resource(SmokeAssets {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        mat: mats.add(StandardMaterial {
            base_color: Color::srgba(0.74, 0.75, 0.76, 0.96),
            base_color_texture: Some(tex),
            alpha_mode: AlphaMode::Blend,
            // Lit like the scene, so it isn't glowing white in shade.
            perceptual_roughness: 1.0,
            reflectance: 0.0,
            double_sided: true,
            cull_mode: None,
            ..default()
        }),
    });
    commands.spawn((
        Name::new("Smoke overlay"),
        SmokeOverlay,
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::srgba(0.62, 0.63, 0.64, 0.0)),
        GlobalZIndex(899),
        bevy::picking::Pickable::IGNORE,
    ));
}

/// Puffs face the camera, grow with the cloud, drift and turn slowly; the camera inside a
/// cloud greys the screen.
fn animate_smoke(
    time: Res<Time>,
    cams: Query<&GlobalTransform, With<engine::gfx::MainCamera>>,
    clouds: Query<(&SmokeVolume, &Children)>,
    mut puffs: Query<(&SmokePuff, &mut Transform)>,
    mut overlay: Query<&mut BackgroundColor, With<SmokeOverlay>>,
    paused: Res<crate::state::Paused>,
) {
    let Ok(cam) = cams.single() else { return };
    let eye = cam.translation();
    let t = time.elapsed_secs();
    let mut inside: f32 = 0.0;
    for (s, children) in &clouds {
        let grow = s.cloud.radius() / g::SMOKE_RADIUS;
        let d = to_ground(eye).dist(s.cloud.at);
        let r = s.cloud.radius();
        if r > 0.3 && eye.y < 3.2 * grow + 0.5 {
            inside = inside.max(((r * 0.95 - d) / (r * 0.4)).clamp(0.0, 1.0));
        }
        for c in children.iter() {
            let Ok((p, mut tf)) = puffs.get_mut(c) else { continue };
            let drift = Vec3::new((t * 0.21 + p.spin).sin(), (t * 0.17 + p.spin * 2.0).sin() * 0.3, (t * 0.19 + p.spin).cos()) * 0.25;
            let local = p.offset * (0.3 + 0.7 * grow) + drift;
            let to_cam = (eye - (s.at + local)).normalize_or(Vec3::Z);
            // Face the camera (the quad's front is +Z), slowly turning about the view axis.
            let rot = Transform::IDENTITY.looking_to(-to_cam, Vec3::Y).rotation * Quat::from_rotation_z(p.spin + t * 0.05);
            tf.translation = local;
            tf.rotation = rot;
            tf.scale = Vec3::splat(p.size * grow.max(0.05));
        }
    }
    let a = if paused.0 { 0.0 } else { inside * 0.97 };
    for mut bg in &mut overlay {
        if (bg.0.alpha() - a).abs() > 1e-3 {
            bg.0.set_alpha(a);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_blast(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
    color: LinearRgba,
    size: f32,
    ttl: f32,
    light: Color,
    intensity: f32,
) {
    commands
        .spawn((
            Blast { ttl, max: ttl, size },
            Mesh3d(meshes.add(Sphere::new(1.0))),
            MeshMaterial3d(materials.add(StandardMaterial { base_color: color.into(), unlit: true, alpha_mode: AlphaMode::Add, ..default() })),
            Transform::from_translation(at).with_scale(Vec3::splat(0.1)),
            bevy::light::NotShadowCaster,
            engine::render_budget::Actor,
            DespawnOnExit(GameState::Playing),
        ))
        .with_child(PointLight { color: light, intensity, range: 14.0, ..default() });
}

fn tick_blasts(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Blast, &mut Transform)>) {
    for (e, mut b, mut tf) in &mut q {
        b.ttl -= time.delta_secs();
        if b.ttl <= 0.0 {
            commands.entity(e).despawn();
        } else {
            let f = 1.0 - b.ttl / b.max;
            tf.scale = Vec3::splat(b.size * (0.3 + 0.7 * f.sqrt()) * (1.0 - 0.6 * f));
        }
    }
}

/// The frag's warning light blinks faster as the fuse runs down.
fn blink(grenades: Query<(&Grenade, &Children)>, mut lights: Query<&mut PointLight, With<FuseLight>>) {
    for (gr, children) in &grenades {
        let rate = 3.0 + 9.0 * (1.0 - gr.fuse / gr.kind.fuse()).clamp(0.0, 1.0);
        let on = (gr.fuse * rate).fract() < 0.5;
        for c in children.iter() {
            if let Ok(mut l) = lights.get_mut(c) {
                l.intensity = if on { 40_000.0 } else { 0.0 };
            }
        }
    }
}

fn tick_flash(time: Res<Time>, mut f: ResMut<Flashed>) {
    if f.dur > 0.0 {
        f.t += time.delta_secs();
        if f.t >= f.dur {
            *f = Flashed::default();
        }
    }
}

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        Name::new("Flashbang overlay"),
        FlashOverlay,
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::srgba(1.0, 1.0, 0.97, 0.0)),
        GlobalZIndex(900),
        bevy::picking::Pickable::IGNORE,
    ));
}

/// Above the HUD (z 8) and debrief (20); hidden while paused so the pause menu stays readable.
fn update_overlay(f: Res<Flashed>, paused: Res<crate::state::Paused>, mut q: Query<&mut BackgroundColor, With<FlashOverlay>>) {
    let a = if paused.0 { 0.0 } else { f.alpha() };
    for mut bg in &mut q {
        if (bg.0.alpha() - a).abs() > 1e-3 {
            bg.0.set_alpha(a);
        }
    }
}

fn clear(mut f: ResMut<Flashed>, mut arena: ResMut<ArenaRes>) {
    *f = Flashed::default();
    if !arena.0.smokes.is_empty() {
        arena.0.smokes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_lands_on_target() {
        let from = Vec3::new(0.0, 1.5, 0.0);
        let to = Vec3::new(12.0, 0.1, -5.0);
        let v = launch_velocity(from, to);
        // Integrate the ballistic path with a small step until it falls back to the target height.
        let (mut p, mut vel, dt) = (from, v, 1.0 / 240.0);
        for _ in 0..2000 {
            vel.y -= GRAVITY * dt;
            p += vel * dt;
            if vel.y < 0.0 && p.y <= to.y {
                break;
            }
        }
        assert!(p.xz().distance(to.xz()) < 0.25, "{p:?}");
    }

    #[test]
    fn flash_fades() {
        let f = Flashed { peak: 1.0, t: 0.0, dur: 2.0 };
        assert_eq!(f.alpha(), 1.0);
        assert!(Flashed { t: 1.5, ..f }.alpha() < 0.5);
        assert_eq!(Flashed { t: 2.0, ..f }.alpha(), 0.0);
    }
}
