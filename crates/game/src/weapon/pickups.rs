//! Guns on the ground. Enemies drop the gun they carry ([`Armed`] on their root, Rifle if
//! none) when they die. Walking over a gun of a kind you carry takes its ammo; holding E on
//! another kind swaps it for your primary (yours is left on the floor). Pickups are
//! [`Actor`]s, capped, and cleared when a round ends.

use bevy::prelude::*;
use engine::render_budget::Actor;

use super::arsenal::{GunState, Phase, Weapon, WeaponInput};
use super::{Armed, Arsenal, GunEvent, Health, WeaponKind, WeaponSet};
use crate::assets::AssetsDir;
use crate::enemy::Enemy;
use crate::player::Player;
use crate::state::GameState;

/// Most guns on the ground at once (oldest goes first).
pub const MAX_PICKUPS: usize = 12;
/// Seconds a dropped gun stays.
pub const PICKUP_TTL: f32 = 90.0;
/// Hold E this long to swap.
pub const SWAP_HOLD_S: f32 = 0.45;
/// Reach for a swap (metres from the eye, roughly in view).
pub const SWAP_RANGE: f32 = 2.4;
/// Walk-over radius for ammo.
pub const AMMO_RANGE: f32 = 1.3;

pub(super) fn build(app: &mut App) {
    app.init_resource::<PickupFocus>().add_systems(PostStartup, init_pickup_assets.run_if(crate::state::has_visuals)).add_systems(
        Update,
        (drop_enemy_guns, ammo_pickup, swap_pickup, animate_pickups).chain().after(WeaponSet).run_if(in_state(GameState::Playing)),
    );
}

/// A gun lying on the ground.
#[derive(Component, Clone, Copy, Debug)]
pub struct Pickup {
    pub gun: GunState,
    pub age: f32,
    /// Resting height of its centre.
    pub y: f32,
    pub spin: f32,
}

/// Enemy that has already dropped its gun.
#[derive(Component)]
struct Dropped;

/// The pickup the player could swap to right now (for the HUD prompt).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct PickupFocus {
    pub target: Option<(Entity, WeaponKind)>,
    /// 0..1 hold progress.
    pub hold: f32,
}

/// Shared pickup visuals: one scene handle per gun, one primitive mesh + material.
#[derive(Resource)]
pub struct PickupAssets {
    scenes: [Option<Handle<bevy::world_serialization::WorldAsset>>; 5],
    mesh: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
}

fn init_pickup_assets(
    mut commands: Commands,
    server: Res<AssetServer>,
    dir: Res<AssetsDir>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let scenes = WeaponKind::ALL.map(|k| {
        let path = super::viewmodel::fit(k).path;
        dir.has(path).then(|| server.load(bevy::gltf::GltfAssetLabel::Scene(0).from_asset(path)))
    });
    commands.insert_resource(PickupAssets {
        scenes,
        mesh: meshes.add(Cuboid::new(0.08, 0.12, 0.6)),
        mat: mats.add(StandardMaterial { base_color: Color::srgb(0.16, 0.16, 0.17), metallic: 0.6, perceptual_roughness: 0.45, ..default() }),
    });
}

/// Spawn a pickup (model if present, else a primitive) at `pos` on the ground.
pub fn spawn_pickup(commands: &mut Commands, assets: Option<&PickupAssets>, gun: GunState, pos: Vec3) -> Entity {
    let y = pos.y.max(0.0) + 0.08;
    let yaw = rand::random_range(0.0..std::f32::consts::TAU);
    let e = commands
        .spawn((
            Name::new(format!("Pickup {:?}", gun.kind)),
            Pickup { gun, age: 0.0, y, spin: yaw },
            Actor,
            Transform::from_translation(Vec3::new(pos.x, y, pos.z)).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::default(),
            DespawnOnExit(GameState::Playing),
        ))
        .id();
    let Some(assets) = assets else { return e };
    let f = super::viewmodel::fit(gun.kind);
    let idx = WeaponKind::ALL.iter().position(|k| *k == gun.kind).unwrap_or(0);
    match &assets.scenes[idx] {
        // Lying on its side, centred on the pickup.
        Some(scene) => {
            commands.spawn((
                bevy::world_serialization::WorldAssetRoot(scene.clone()),
                {
                    let rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_y(f.yaw);
                    let scale = f.scale * 1.15;
                    Transform { translation: -(rot * (f.center * scale)), rotation: rot, scale: Vec3::splat(scale) }
                },
                ChildOf(e),
            ));
        }
        None => {
            commands.spawn((Mesh3d(assets.mesh.clone()), MeshMaterial3d(assets.mat.clone()), Transform::default(), ChildOf(e)));
        }
    }
    e
}

fn drop_enemy_guns(
    mut commands: Commands,
    assets: Option<Res<PickupAssets>>,
    enemies: Query<(Entity, &Transform, &Health, Option<&Armed>), (With<Enemy>, Without<Dropped>)>,
    pickups: Query<(Entity, &Pickup)>,
) {
    let mut count = pickups.iter().count();
    for (e, tf, hp, armed) in &enemies {
        if !hp.is_dead() {
            continue;
        }
        commands.entity(e).try_insert(Dropped);
        let kind = armed.map_or(WeaponKind::Rifle, |a| a.0);
        let spec = kind.spec();
        let gun = GunState { kind, ammo: spec.mag, reserve: spec.reserve / 3 };
        if count >= MAX_PICKUPS
            && let Some((old, _)) = pickups.iter().max_by(|a, b| a.1.age.total_cmp(&b.1.age))
        {
            commands.entity(old).try_despawn();
            count -= 1;
        }
        let at = tf.translation + tf.right() * 0.4;
        spawn_pickup(&mut commands, assets.as_deref(), gun, Vec3::new(at.x, 0.0, at.z));
        count += 1;
    }
}

/// Walk over a gun you carry: take its rounds (up to twice the starting reserve).
fn ammo_pickup(
    mut commands: Commands,
    mut player: Query<(&Transform, &mut Weapon, &mut Arsenal), With<Player>>,
    mut pickups: Query<(Entity, &Transform, &mut Pickup)>,
    mut events: MessageWriter<GunEvent>,
) {
    let Ok((ptf, mut w, mut a)) = player.single_mut() else { return };
    let feet = ptf.translation;
    for (e, tf, mut p) in &mut pickups {
        if tf.translation.xz().distance(feet.xz()) > AMMO_RANGE {
            continue;
        }
        let kind = p.gun.kind;
        let cap = kind.spec().reserve * 2;
        let rounds = p.gun.ammo + p.gun.reserve;
        let taken = if w.kind == kind {
            let t = rounds.min(cap.saturating_sub(w.reserve));
            w.reserve += t;
            t
        } else if let Some(Some(g)) = a.slots.iter_mut().find(|g| g.is_some_and(|g| g.kind == kind)) {
            let t = rounds.min(cap.saturating_sub(g.reserve));
            g.reserve += t;
            t
        } else {
            continue;
        };
        if taken == 0 {
            continue;
        }
        events.write(GunEvent::Pickup { kind });
        let left = rounds - taken;
        if left == 0 {
            commands.entity(e).despawn();
        } else {
            p.gun.ammo = left.min(p.gun.ammo);
            p.gun.reserve = left - p.gun.ammo;
        }
    }
}

/// Hold E on a gun of another kind to swap it for your primary.
fn swap_pickup(
    mut commands: Commands,
    time: Res<Time>,
    input: Res<WeaponInput>,
    assets: Option<Res<PickupAssets>>,
    mut player: Query<(&Transform, &mut Weapon, &mut Arsenal, &mut Armed), With<Player>>,
    cams: Query<&Transform, (With<engine::gfx::MainCamera>, Without<Player>, Without<Pickup>)>,
    pickups: Query<(Entity, &Transform, &Pickup)>,
    mut focus: ResMut<PickupFocus>,
    mut events: MessageWriter<GunEvent>,
) {
    let Ok((ptf, mut w, mut a, mut armed)) = player.single_mut() else { return };
    let cam = cams.single().map(|c| ptf.mul_transform(*c)).unwrap_or(*ptf);
    let eye = cam.translation;
    let fwd = *cam.forward();
    let carried = |k: WeaponKind| a.slots.iter().flatten().any(|g| g.kind == k);
    // Best candidate: close and near the middle of the view.
    let best = pickups
        .iter()
        .filter(|(_, _, p)| !carried(p.gun.kind) && p.gun.kind != WeaponKind::Pistol)
        .filter_map(|(e, tf, p)| {
            let to = tf.translation - eye;
            let d = to.length();
            let facing = to.normalize_or_zero().dot(fwd);
            (d <= SWAP_RANGE && (facing > 0.55 || tf.translation.xz().distance(ptf.translation.xz()) < 0.9)).then_some((e, p.gun, d - facing))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let target = best.map(|(e, g, _)| (e, g.kind));
    let mut hold = if target == focus.target && input.interact { focus.hold + time.delta_secs() / SWAP_HOLD_S } else { 0.0 };
    if let (Some((e, gun, _)), true) = (best, hold >= 1.0) {
        hold = 0.0;
        // Leave the current primary where the new one was.
        let old = if a.active == 0 { Some(w.state()) } else { a.slots[0] };
        if let (Some(old), Ok((_, tf, _))) = (old, pickups.get(e)) {
            spawn_pickup(&mut commands, assets.as_deref(), old, tf.translation.with_y(0.0));
        }
        commands.entity(e).despawn();
        a.slots[0] = Some(gun);
        if a.active != 0 {
            a.last = a.active;
            a.active = 0;
        }
        *w = Weapon::new(gun);
        w.phase = Phase::Drawing { left: super::arsenal::draw_time(gun.kind), total: super::arsenal::draw_time(gun.kind) };
        if armed.0 != gun.kind {
            armed.0 = gun.kind;
        }
        events.write(GunEvent::Pickup { kind: gun.kind });
        events.write(GunEvent::Draw { kind: gun.kind });
    }
    let new = PickupFocus { target: if hold >= 1.0 { None } else { target }, hold: hold.min(1.0) };
    if *focus != new {
        *focus = new;
    }
}

/// Slow turn and a small glint bob so guns read as pickups.
fn animate_pickups(time: Res<Time>, mut q: Query<(Entity, &mut Pickup, &mut Transform)>, mut commands: Commands) {
    let dt = time.delta_secs();
    for (e, mut p, mut tf) in &mut q {
        p.age += dt;
        if p.age > PICKUP_TTL {
            commands.entity(e).despawn();
            continue;
        }
        p.spin += dt * 0.6;
        let y = p.y + 0.05 + (p.age * 2.0).sin() * 0.03;
        let rot = Quat::from_rotation_y(p.spin);
        tf.translation.y = y;
        tf.rotation = rot;
    }
}
