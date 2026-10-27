//! Shot effects. Particles are the engine's pooled VFX (`engine::vfx`): muzzle smoke per gun,
//! surface impacts (concrete dust, plaster, metal sparks, wood splinters, suit sparks) and
//! ejected shell casings (lit mesh particles that bounce off the level). Tracers and bullet
//! holes are pooled entities on one shared mesh + material per kind, so they batch and never
//! churn: the oldest hole is moved, not respawned.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use engine::gfx::MainCamera;
use engine::vfx::{
    Blend, Burst, Collision, Curve, EffectDef, EffectLight, EmitterDef, Flipbook, Gradient, MeshShape, RenderDef, RenderMode, Shape, Velocity, Vfx, VfxLibrary,
    VfxTexture,
};

use super::viewmodel::{MuzzleCam, fit};
use super::{GunEvent, Impact, ShotFired, Surface, Tracer, WeaponKind, WeaponSet};
use crate::gfx::FxAssets;
use crate::player::Player;
use crate::state::{GameState, has_visuals};

const TRACER_SPEED: f32 = 350.0;
const TRACER_POOL: usize = 32;
/// Bullet holes kept in the world (oldest reused first).
pub const MAX_DECALS: usize = 96;

pub(super) fn build(app: &mut App) {
    app.init_resource::<Decals>()
        .init_resource::<Tracers>()
        .add_systems(Startup, register_effects)
        .add_systems(PostStartup, init_weapon_fx.run_if(has_visuals))
        .add_systems(OnEnter(GameState::Playing), clear_decals.run_if(has_visuals))
        .add_systems(Update, (shot_fx, impact_fx, update_tracers).chain().after(WeaponSet).run_if(has_visuals));
}

/// Effect names registered in the VFX library.
pub fn impact_effect(s: Surface) -> &'static str {
    match s {
        Surface::Concrete => "impact_concrete",
        Surface::Plaster => "impact_plaster",
        Surface::Metal => "impact_metal",
        Surface::Wood => "impact_wood",
        Surface::Suit => "impact_suit",
    }
}

fn muzzle_effect(k: WeaponKind) -> &'static str {
    match k {
        WeaponKind::Shotgun => "muzzle_shotgun",
        WeaponKind::Marksman => "muzzle_marksman",
        WeaponKind::Pistol | WeaponKind::Smg => "muzzle_light",
        WeaponKind::Rifle => "muzzle_rifle",
    }
}

fn casing_effect(k: WeaponKind) -> &'static str {
    match k {
        WeaponKind::Shotgun => "casing_shell",
        WeaponKind::Marksman => "casing_large",
        _ => "casing",
    }
}

fn smoke_book() -> Option<Flipbook> {
    Some(Flipbook { random_start: true, ..Default::default() })
}

fn hot() -> Gradient {
    Gradient::srgb(&[(0.0, [1.0, 0.95, 0.8], 1.0), (0.3, [1.0, 0.65, 0.2], 1.0), (1.0, [0.6, 0.12, 0.02], 0.0)])
}

const G: Vec3 = Vec3::new(0.0, -9.81, 0.0);

/// Muzzle smoke + sparks along -Z (the flash itself is the viewmodel's sprite).
fn muzzle_def(name: &str, smoke: u32, size: f32, sparks: u32, light: f32) -> EffectDef {
    let fwd = Vec3::NEG_Z;
    let mut def = EffectDef { cull_distance: 80.0, ..EffectDef::new(name) }.once(0.02);
    if light > 0.0 {
        def.light = Some(EffectLight { color: [1.0, 0.7, 0.35], intensity: light, range: 6.0, duration: 0.05, over_time: Curve::linear(1.0, 0.0), ..Default::default() });
    }
    def.with(EmitterDef {
        name: "smoke".into(),
        capacity: 16,
        bursts: vec![Burst::at(0.0, smoke)],
        lifetime: (0.5, 1.1),
        velocity: Velocity::Cone { dir: fwd, angle: 0.35, speed: (0.6, 1.8) },
        gravity: Vec3::new(0.0, 0.35, 0.0),
        drag: 2.2,
        size: (size * 0.7, size),
        size_over_life: Curve::linear(0.7, 3.2),
        color_over_life: Gradient::srgb(&[(0.0, [0.7, 0.7, 0.7], 0.0), (0.1, [0.65, 0.65, 0.66], 0.28), (1.0, [0.6, 0.6, 0.62], 0.0)]),
        rotation: (0.0, std::f32::consts::TAU),
        spin: (-0.6, 0.6),
        render: RenderDef { texture: VfxTexture::Smoke, blend: Blend::Alpha, lit: 0.6, flipbook: smoke_book(), ..Default::default() },
        ..Default::default()
    })
    .with(EmitterDef {
        name: "sparks".into(),
        capacity: 24,
        bursts: vec![Burst::at(0.0, sparks)],
        lifetime: (0.06, 0.18),
        velocity: Velocity::Cone { dir: fwd, angle: 0.4, speed: (8.0, 20.0) },
        gravity: G,
        size: (0.01, 0.018),
        color_over_life: hot(),
        render: RenderDef { mode: RenderMode::Stretched { factor: 0.02 }, texture: VfxTexture::Spark, intensity: 10.0, ..Default::default() },
        ..Default::default()
    })
}

/// Impact effects emit along +Y (the play transform turns +Y onto the hit normal).
fn dust_def(name: &str, rgb: [f32; 3], count: u32, size: f32, debris: Option<([f32; 3], u32, MeshShape)>, sparks: u32) -> EffectDef {
    let mut def = EffectDef { cull_distance: 90.0, ..EffectDef::new(name) }.once(0.02).with(EmitterDef {
        name: "dust".into(),
        capacity: 16,
        bursts: vec![Burst::at(0.0, count)],
        lifetime: (0.5, 1.2),
        shape: Shape::Sphere { radius: 0.03, surface: false },
        velocity: Velocity::Cone { dir: Vec3::Y, angle: 0.55, speed: (0.6, 2.4) },
        gravity: Vec3::new(0.0, -0.4, 0.0),
        drag: 3.0,
        size: (size * 0.6, size),
        size_over_life: Curve::linear(0.5, 2.6),
        color_over_life: Gradient::srgb(&[(0.0, rgb, 0.75), (1.0, rgb, 0.0)]),
        rotation: (0.0, std::f32::consts::TAU),
        spin: (-1.0, 1.0),
        render: RenderDef { texture: VfxTexture::Smoke, blend: Blend::Alpha, lit: 0.9, flipbook: smoke_book(), ..Default::default() },
        ..Default::default()
    });
    if let Some((c, n, shape)) = debris {
        def = def.with(EmitterDef {
            name: "debris".into(),
            capacity: 12,
            bursts: vec![Burst::at(0.0, n)],
            lifetime: (0.5, 0.9),
            velocity: Velocity::Cone { dir: Vec3::Y, angle: 0.8, speed: (1.5, 4.5) },
            gravity: G,
            size: (0.005, 0.012),
            color_over_life: Gradient::srgb(&[(0.0, c, 1.0), (0.85, c, 1.0), (1.0, c, 0.0)]),
            rotation: (0.0, std::f32::consts::TAU),
            spin: (-12.0, 12.0),
            render: RenderDef { mode: RenderMode::Mesh { shape }, lit: 1.0, ..Default::default() },
            collision: Collision::Plane { height: 0.0, restitution: 0.3, friction: 0.5, die: false },
            ..Default::default()
        });
    }
    if sparks > 0 {
        def = def.with(EmitterDef {
            name: "sparks".into(),
            capacity: 16,
            bursts: vec![Burst::at(0.0, sparks)],
            lifetime: (0.12, 0.35),
            velocity: Velocity::Cone { dir: Vec3::Y, angle: 0.9, speed: (3.0, 9.0) },
            gravity: G,
            drag: 0.5,
            size: (0.012, 0.022),
            color_over_life: hot(),
            render: RenderDef { mode: RenderMode::Stretched { factor: 0.03 }, texture: VfxTexture::Spark, intensity: 9.0, ..Default::default() },
            ..Default::default()
        });
    }
    def
}

/// Spent cases: lit brass (or red hull) bits thrown right/up (effect space: +X right, -Z
/// forward), bouncing off level colliders.
fn casing_def(name: &str, rgb: [f32; 3], size: f32, life: f32) -> EffectDef {
    EffectDef { cull_distance: 40.0, ..EffectDef::new(name) }.once(0.02).with(EmitterDef {
        name: "case".into(),
        capacity: 2,
        bursts: vec![Burst::at(0.0, 1)],
        lifetime: (life, life * 1.2),
        velocity: Velocity::Fixed { v: Vec3::new(2.4, 2.0, 0.4), jitter: Vec3::new(0.5, 0.6, 0.4) },
        gravity: G,
        drag: 0.2,
        size: (size, size * 1.1),
        color_over_life: Gradient::srgb(&[(0.0, rgb, 1.0), (0.9, rgb, 1.0), (1.0, rgb, 0.0)]),
        rotation: (0.0, std::f32::consts::TAU),
        spin: (-18.0, 18.0),
        render: RenderDef { mode: RenderMode::Mesh { shape: MeshShape::Cube }, lit: 1.0, ..Default::default() },
        collision: Collision::Physics { restitution: 0.35, friction: 0.4, die: false },
        ..Default::default()
    })
}

fn register_effects(library: Option<ResMut<VfxLibrary>>) {
    let Some(mut lib) = library else { return };
    let defs = [
        muzzle_def("muzzle_light", 3, 0.09, 5, 0.0),
        muzzle_def("muzzle_rifle", 4, 0.12, 7, 0.0),
        muzzle_def("muzzle_shotgun", 8, 0.2, 14, 0.0),
        muzzle_def("muzzle_marksman", 7, 0.18, 10, 0.0),
        dust_def("impact_concrete", [0.62, 0.6, 0.56], 6, 0.22, Some(([0.45, 0.44, 0.42], 5, MeshShape::Shard)), 3),
        dust_def("impact_plaster", [0.88, 0.86, 0.82], 8, 0.26, Some(([0.9, 0.88, 0.85], 4, MeshShape::Cube)), 0),
        dust_def("impact_metal", [0.4, 0.4, 0.42], 2, 0.12, None, 12),
        dust_def("impact_wood", [0.55, 0.42, 0.28], 4, 0.18, Some(([0.62, 0.45, 0.26], 7, MeshShape::Shard)), 0),
        dust_def("impact_suit", [0.3, 0.32, 0.36], 2, 0.15, None, 9),
        casing_def("casing", [0.85, 0.62, 0.25], 0.0045, 1.4),
        casing_def("casing_large", [0.85, 0.62, 0.25], 0.006, 1.6),
        casing_def("casing_shell", [0.75, 0.12, 0.08], 0.009, 1.8),
    ];
    for d in defs {
        let name = d.name.clone();
        if let Err(e) = lib.register(d) {
            warn!("weapon vfx `{name}`: {e}");
        }
    }
}

/// Shared handles for holes and tracers.
#[derive(Resource)]
struct WeaponFx {
    quad: Handle<Mesh>,
    cube: Handle<Mesh>,
    tracer: Handle<StandardMaterial>,
    /// Bullet-hole material per surface (index = `hole_index`).
    holes: [Handle<StandardMaterial>; 3],
}

fn hole_index(s: Surface) -> usize {
    match s {
        Surface::Metal | Surface::Suit => 1,
        Surface::Wood => 2,
        Surface::Concrete | Surface::Plaster => 0,
    }
}

fn init_weapon_fx(mut commands: Commands, fx: Option<Res<FxAssets>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let Some(fx) = fx else { return };
    let base = mats.get(fx.decal.id()).cloned().unwrap_or_default();
    let metal = mats.add(StandardMaterial { base_color: Color::srgb(0.55, 0.5, 0.45), metallic: 0.8, perceptual_roughness: 0.35, ..base.clone() });
    let wood = mats.add(StandardMaterial { base_color: Color::srgb(0.5, 0.33, 0.2), ..base });
    commands.insert_resource(WeaponFx { quad: fx.quad.clone(), cube: fx.cube.clone(), tracer: fx.tracer.clone(), holes: [fx.decal.clone(), metal, wood] });
}

/// Bullet-hole entities, reused oldest first.
#[derive(Resource, Default)]
struct Decals {
    ents: Vec<Entity>,
    next: usize,
}

struct TracerSlot {
    e: Entity,
    from: Vec3,
    dir: Vec3,
    len: f32,
    age: f32,
    live: bool,
}

#[derive(Resource, Default)]
struct Tracers {
    slots: Vec<TracerSlot>,
    next: usize,
}

fn clear_decals(decals: Res<Decals>, mut vis: Query<&mut Visibility>) {
    for e in &decals.ents {
        if let Ok(mut v) = vis.get_mut(*e)
            && *v != Visibility::Hidden
        {
            *v = Visibility::Hidden;
        }
    }
}

/// Muzzle smoke at the gun, casings out of the ejection port (shotgun on the pump, marksman on
/// the bolt).
fn shot_fx(
    mut vfx: Vfx,
    mut shots: MessageReader<ShotFired>,
    mut events: MessageReader<GunEvent>,
    muzzle: Res<MuzzleCam>,
    player: Query<(Entity, &super::Weapon), With<Player>>,
    cams: Query<&GlobalTransform, With<MainCamera>>,
) {
    let (Ok((pe, w)), Ok(cam)) = (player.single(), cams.single()) else {
        shots.clear();
        events.clear();
        return;
    };
    let cam = cam.compute_transform();
    let f = fit(w.kind);
    // Ejection port: a bit behind the muzzle along the barrel, on the right.
    let port_cam = muzzle.0 * Vec3::new(1.0, 1.0, 0.0) + Vec3::new(0.02, 0.03, f.hip.z + f.muzzle.z * 0.25);
    let eject = |vfx: &mut Vfx, kind: WeaponKind| {
        let at = Transform::from_translation(cam.transform_point(port_cam)).with_rotation(cam.rotation);
        vfx.play(casing_effect(kind), at);
    };
    for s in shots.read() {
        if s.by != pe {
            continue;
        }
        let at = Transform::from_translation(cam.transform_point(muzzle.0)).with_rotation(cam.rotation);
        vfx.play(muzzle_effect(s.kind), at);
        if !matches!(s.kind, WeaponKind::Shotgun | WeaponKind::Marksman) {
            eject(&mut vfx, s.kind);
        }
    }
    for ev in events.read() {
        match ev {
            GunEvent::Pump => eject(&mut vfx, WeaponKind::Shotgun),
            GunEvent::Bolt => eject(&mut vfx, WeaponKind::Marksman),
            _ => {}
        }
    }
}

fn impact_fx(
    mut commands: Commands,
    mut vfx: Vfx,
    wfx: Option<Res<WeaponFx>>,
    mut impacts: MessageReader<Impact>,
    mut tracer_msgs: MessageReader<Tracer>,
    mut decals: ResMut<Decals>,
    mut tracers: ResMut<Tracers>,
    mut holes: Query<(&mut Transform, &mut Visibility, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let Some(wfx) = wfx else {
        impacts.clear();
        tracer_msgs.clear();
        return;
    };
    for hit in impacts.read() {
        let n = hit.normal.try_normalize().unwrap_or(Vec3::Y);
        let rot = Quat::from_rotation_arc(Vec3::Y, n);
        vfx.play(impact_effect(hit.surface), Transform::from_translation(hit.pos + n * 0.01).with_rotation(rot));
        if hit.flesh {
            continue;
        }
        // Bullet hole: reuse the oldest once the pool is full.
        let up = if n.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
        let size = rand::random_range(0.11..0.17) * if hit.exit { 1.35 } else { 1.0 };
        let rotation = Transform::IDENTITY.looking_to(-n, up).rotation * Quat::from_rotation_z(rand::random_range(0.0..std::f32::consts::TAU));
        let tf = Transform { translation: hit.pos + n * 0.004, rotation, scale: Vec3::splat(size) };
        let mat = wfx.holes[hole_index(hit.surface)].clone();
        if decals.ents.len() < MAX_DECALS {
            let e = commands.spawn((Name::new("Bullet hole"), Mesh3d(wfx.quad.clone()), MeshMaterial3d(mat), tf, Visibility::Inherited, NotShadowCaster)).id();
            decals.ents.push(e);
        } else {
            let i = decals.next % MAX_DECALS;
            decals.next = (i + 1) % MAX_DECALS;
            if let Ok((mut t, mut v, mut m)) = holes.get_mut(decals.ents[i]) {
                *t = tf;
                if *v != Visibility::Inherited {
                    *v = Visibility::Inherited;
                }
                if m.0 != mat {
                    m.0 = mat;
                }
            }
        }
    }
    for t in tracer_msgs.read() {
        let len = t.from.distance(t.to);
        if len < 0.5 {
            continue;
        }
        let slot = TracerSlot { e: Entity::PLACEHOLDER, from: t.from, dir: (t.to - t.from) / len, len, age: 0.0, live: true };
        if tracers.slots.len() < TRACER_POOL {
            let e = commands
                .spawn((Name::new("Tracer"), Mesh3d(wfx.cube.clone()), MeshMaterial3d(wfx.tracer.clone()), Transform::from_translation(t.from).with_scale(Vec3::ZERO), NotShadowCaster))
                .id();
            tracers.slots.push(TracerSlot { e, ..slot });
        } else {
            let i = tracers.next % TRACER_POOL;
            tracers.next = (i + 1) % TRACER_POOL;
            let e = tracers.slots[i].e;
            tracers.slots[i] = TracerSlot { e, ..slot };
        }
    }
}

fn update_tracers(time: Res<Time>, mut tracers: ResMut<Tracers>, mut q: Query<&mut Transform>) {
    let dt = time.delta_secs();
    for s in tracers.slots.iter_mut().filter(|s| s.live) {
        s.age += dt;
        let Ok(mut tf) = q.get_mut(s.e) else { continue };
        let head = (s.age * TRACER_SPEED).min(s.len);
        let tail = (head - 6.0).max(0.0);
        let seg = head - tail;
        if s.age > s.len / TRACER_SPEED + 0.05 || seg <= 0.01 {
            s.live = false;
            tf.scale = Vec3::ZERO;
            continue;
        }
        *tf = Transform::from_translation(s.from + s.dir * (tail + seg * 0.5)).looking_to(s.dir, Vec3::Y).with_scale(Vec3::new(0.012, 0.012, seg));
    }
}
