//! The player's grenades: frag, flashbang, smoke (bought in the buy menu, carried in
//! [`PlayerGrenades`], kept by surviving like guns).
//!
//! - **4 / G** equips a grenade (pressed again: the next kind carried); 1 / 2 / Q / wheel go back
//!   to a gun.
//! - **Left mouse** pulls the pin; releasing throws overhand (far). **Right mouse** throws an
//!   underhand lob; both together a medium toss. The pin takes [`PIN_S`]: an early release
//!   throws as soon as it is out.
//! - The throw is a real arc from the hand (`enemy::grenade` flies, bounces and detonates it:
//!   frag damage with cover, flash whiteout and enemy blindness, smoke clouds that block the
//!   AI's sight).
//! - While a grenade is in hand the gun is put away (no firing, aiming or reloading) and a simple
//!   first-person hand holds the grenade.
//!
//! Control port: `grenade.equip`, `grenade.throw` (see `commands`).

use avian3d::prelude::*;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use brain::grenade::GrenadeKind;
use engine::fps::FpsMotor;
use engine::gfx::MainCamera;
use engine::input::CursorCaptured;

use super::arsenal::{Phase, Weapon, WeaponInput, draw_time};
use super::viewmodel::ViewRoot;
use super::{WeaponSet, approach, ease};
use crate::economy::{BuyMenu, PlayerGrenades, RoundLoadout, RoundPhase};
use crate::enemy::grenade::ThrowGrenade;
use crate::player::{Player, PlayerIntent, PlayerSet};
use crate::state::{GameState, has_visuals};
use crate::ui::hud::HideWhenPaused;
use crate::ui::theme::*;

/// Pulling the pin / winding up before the grenade can leave the hand (s).
pub const PIN_S: f32 = 0.3;
/// Follow-through after a throw before the next grenade (or the gun) comes up (s).
pub const RECOVER_S: f32 = 0.55;
/// Raising a grenade into view (s).
pub const EQUIP_S: f32 = 0.3;

/// How a grenade leaves the hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrowStyle {
    /// Left mouse: overhand, far.
    Over,
    /// Both buttons: a medium toss.
    Mid,
    /// Right mouse: underhand lob, short.
    Under,
}

impl ThrowStyle {
    /// Release speed (m/s) and how far above the view direction it is aimed (radians).
    pub fn speed_lift(self) -> (f32, f32) {
        match self {
            ThrowStyle::Over => (18.0, 0.3),
            ThrowStyle::Mid => (12.0, 0.3),
            ThrowStyle::Under => (7.0, 0.35),
        }
    }

    pub fn parse(s: &str) -> Option<ThrowStyle> {
        match s.trim().to_ascii_lowercase().as_str() {
            "over" | "overhand" | "far" => Some(ThrowStyle::Over),
            "mid" | "medium" => Some(ThrowStyle::Mid),
            "under" | "underhand" | "lob" | "short" => Some(ThrowStyle::Under),
            _ => None,
        }
    }
}

/// Launch velocity for a throw along the camera's `fwd` (unit), carrying some of the player's
/// own velocity.
pub fn throw_velocity(style: ThrowStyle, fwd: Vec3, player_vel: Vec3) -> Vec3 {
    let (speed, lift) = style.speed_lift();
    let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
    let dir = Quat::from_axis_angle(right, lift) * fwd;
    dir.normalize_or(fwd) * speed + player_vel * 0.6
}

/// The grenade in the player's hand and its throw.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct GrenadeHand {
    /// The grenade out (the gun is put away); `None` = a gun in hand.
    pub equipped: Option<GrenadeKind>,
    /// Seconds since it came up.
    pub raise: f32,
    /// Pin pulled: the throw style and wind-up progress 0..1.
    pub cocked: Option<(ThrowStyle, f32)>,
    /// The buttons were let go: throw as soon as the wind-up completes.
    release: bool,
    /// Follow-through left after a throw (s).
    pub recover: f32,
    /// Grenades thrown this round (telemetry / tests).
    pub thrown: u32,
    buttons_was: (bool, bool),
}

impl GrenadeHand {
    pub fn out(&self) -> bool {
        self.equipped.is_some()
    }
}

/// Control-port requests (one-shot).
#[derive(Resource, Debug, Clone, Default)]
pub struct GrenadeOverride {
    /// `Some(Some(k))`: equip `k`; `Some(None)`: back to the gun.
    pub equip: Option<Option<GrenadeKind>>,
    /// Throw the equipped grenade with this style (pin and release in one go).
    pub throw: Option<ThrowStyle>,
}

pub(super) fn build(app: &mut App) {
    let input = grenade_input.after(PlayerSet::Input).before(PlayerSet::Move);
    #[cfg(feature = "dev")]
    let input = input.after(crate::dev::ControlDrive);
    app.init_resource::<GrenadeHand>()
        .init_resource::<GrenadeOverride>()
        .add_systems(OnEnter(GameState::Playing), (reset_round.after(crate::economy::start_round), spawn_hud.run_if(has_visuals)))
        .add_systems(Update, input.run_if(in_state(GameState::Playing)))
        .add_systems(Update, (animate_hand, update_hud).after(WeaponSet).run_if(has_visuals));
    #[cfg(feature = "dev")]
    commands::register(app);
}

/// Round start: the grenades kept from last round (or none), nothing in hand.
fn reset_round(loadout: Res<RoundLoadout>, mut bag: ResMut<PlayerGrenades>, mut hand: ResMut<GrenadeHand>) {
    bag.0 = loadout.grenades;
    *hand = GrenadeHand::default();
}

fn equip(hand: &mut GrenadeHand, kind: GrenadeKind, w: Option<&mut Weapon>) {
    *hand = GrenadeHand { equipped: Some(kind), thrown: hand.thrown, buttons_was: (true, true), ..default() };
    if let Some(w) = w {
        w.reloading = None;
        w.inspect = 0.0;
        w.ads_wanted = false;
        w.ads = 0.0;
    }
}

fn unequip(hand: &mut GrenadeHand, w: Option<&mut Weapon>) {
    *hand = GrenadeHand { thrown: hand.thrown, ..default() };
    if let Some(w) = w {
        let t = draw_time(w.kind);
        w.phase = Phase::Drawing { left: t, total: t };
    }
}

/// Keys, mouse and control-port requests → equip, pin, throw. Runs between the player's input
/// and movement, and takes the trigger and aim button from the gun while a grenade is out.
#[allow(clippy::too_many_arguments)]
fn grenade_input(
    time: Res<Time>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    captured: Option<Res<CursorCaptured>>,
    buy: Res<BuyMenu>,
    phase: Res<RoundPhase>,
    mut intent: ResMut<PlayerIntent>,
    mut input: ResMut<WeaponInput>,
    mut hand: ResMut<GrenadeHand>,
    mut ov: ResMut<GrenadeOverride>,
    mut bag: ResMut<PlayerGrenades>,
    mut players: Query<(Entity, &Transform, &FpsMotor, Option<&mut Weapon>), With<Player>>,
    cams: Query<&Transform, (With<MainCamera>, Without<Player>)>,
    spatial: SpatialQuery,
    mut throws: MessageWriter<ThrowGrenade>,
) {
    let Ok((pe, ptf, motor, mut weapon)) = players.single_mut() else { return };
    let dt = time.delta_secs();
    let live = captured.is_none_or(|c| c.0);
    // 4 / G: equip, or the next kind carried.
    let key = !buy.open && live && keys.as_ref().is_some_and(|k| k.just_pressed(KeyCode::Digit4) || k.just_pressed(KeyCode::KeyG));
    let mut want: Option<Option<GrenadeKind>> = ov.equip.take();
    if key {
        let next = bag.0.next_after(hand.equipped);
        if next.is_some() && next != hand.equipped {
            want = Some(next);
        }
    }
    let throw_now = ov.throw.take();
    match want {
        Some(Some(k)) if bag.0.count(k) > 0 && hand.equipped != Some(k) => equip(&mut hand, k, weapon.as_deref_mut()),
        Some(None) if hand.out() => unequip(&mut hand, weapon.as_deref_mut()),
        _ => {}
    }
    let Some(kind) = hand.equipped else { return };
    // A gun key puts the grenade away (a number key also picks that gun).
    if input.select.is_some() || input.cycle != 0 || input.last {
        input.cycle = 0;
        input.last = false;
        unequip(&mut hand, weapon.as_deref_mut());
        return;
    }
    // The gun is away: the trigger and aim button belong to the grenade.
    let (lmb, rmb) = (intent.fire, input.ads);
    intent.fire = false;
    intent.reload = false;
    input.ads = false;
    input.inspect = false;
    let (lmb_was, rmb_was) = hand.buttons_was;
    hand.buttons_was = (lmb, rmb);
    hand.raise += dt;

    if hand.recover > 0.0 {
        hand.recover -= dt;
        if hand.recover <= 0.0 {
            if bag.0.count(kind) > 0 {
                hand.raise = 0.0;
                hand.recover = 0.0;
            } else {
                match bag.0.next_after(Some(kind)) {
                    Some(k) => equip(&mut hand, k, None),
                    None => unequip(&mut hand, weapon.as_deref_mut()),
                }
            }
        }
        return;
    }
    if bag.0.count(kind) == 0 {
        unequip(&mut hand, weapon.as_deref_mut());
        return;
    }
    let frozen = phase.frozen();
    match hand.cocked {
        None => {
            if let Some(style) = throw_now.filter(|_| !frozen) {
                hand.cocked = Some((style, 0.0));
                hand.release = true;
            } else if !frozen && ((lmb && !lmb_was) || (rmb && !rmb_was)) {
                hand.cocked = Some((if lmb && rmb { ThrowStyle::Mid } else if lmb { ThrowStyle::Over } else { ThrowStyle::Under }, 0.0));
                hand.release = false;
            }
        }
        Some((style, t)) => {
            let style = match (lmb, rmb) {
                (true, true) => ThrowStyle::Mid,
                (true, false) if style != ThrowStyle::Mid => ThrowStyle::Over,
                (false, true) if style != ThrowStyle::Mid => ThrowStyle::Under,
                _ => style,
            };
            if !lmb && !rmb {
                hand.release = true;
            }
            let t = (t + dt / PIN_S).min(1.0);
            hand.cocked = Some((style, t));
            if hand.release && t >= 1.0 {
                // Release from the hand (a little right of and below the eye), unless that
                // point is inside a wall: then from the eye.
                let cam = cams.single().map(|c| ptf.mul_transform(*c)).unwrap_or(*ptf);
                let (eye, fwd) = (cam.translation, *cam.forward());
                let hand_pt = eye + fwd * 0.35 + *cam.right() * 0.15 - Vec3::Y * 0.12;
                let from = match Dir3::new(hand_pt - eye) {
                    Ok(dir) => {
                        let filter = SpatialQueryFilter::from_excluded_entities([pe]);
                        match spatial.cast_ray(eye, dir, eye.distance(hand_pt) + 0.08, true, &filter) {
                            Some(_) => eye,
                            None => hand_pt,
                        }
                    }
                    Err(_) => eye,
                };
                let vel = throw_velocity(style, fwd, motor.velocity);
                throws.write(ThrowGrenade { thrower: pe, kind, from, target: from + vel, vel: Some(vel) });
                bag.0.take(kind);
                hand.thrown += 1;
                hand.cocked = None;
                hand.release = false;
                hand.recover = RECOVER_S;
                info!("threw a {} ({style:?})", kind.label());
            }
        }
    }
}

// ---------------------------------------------------------------- first-person hand

/// The first-person grenade hand (a sibling of the gun pivot under the view root).
#[derive(Component)]
struct HandRoot;

/// One grenade model in the hand.
#[derive(Component)]
struct HandGrenade(GrenadeKind);

fn spawn_hand(commands: &mut Commands, root: Entity, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, fabric: Option<StandardMaterial>) {
    let mat = |materials: &mut Assets<StandardMaterial>, c: Color, metal: f32, rough: f32| {
        materials.add(StandardMaterial { base_color: c, metallic: metal, perceptual_roughness: rough, ..default() })
    };
    let glove = mat(materials, Color::srgb(0.06, 0.06, 0.065), 0.0, 0.75);
    let sleeve = match fabric {
        Some(m) => materials.add(m),
        None => mat(materials, Color::srgb(0.34, 0.29, 0.21), 0.0, 0.9),
    };
    let cuff = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.33, 0.3),
        emissive: LinearRgba::rgb(0.01, 0.05, 0.045),
        perceptual_roughness: 0.7,
        ..default()
    });
    let olive = mat(materials, Color::srgb(0.2, 0.25, 0.14), 0.1, 0.65);
    let steel = mat(materials, Color::srgb(0.5, 0.5, 0.48), 0.9, 0.35);
    let grey = mat(materials, Color::srgb(0.62, 0.63, 0.6), 0.6, 0.4);
    let smoke_body = mat(materials, Color::srgb(0.24, 0.27, 0.26), 0.3, 0.55);
    let band_white = mat(materials, Color::srgb(0.85, 0.85, 0.8), 0.0, 0.6);
    let band_green = mat(materials, Color::srgb(0.25, 0.55, 0.3), 0.0, 0.6);
    let sphere = meshes.add(Sphere::new(1.0).mesh().uv(20, 14));
    let tube = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(16).segments(1));
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let capsule = meshes.add(Capsule3d::new(1.0, 2.0).mesh().rings(2).latitudes(8).longitudes(10));
    let ring = meshes.add(Torus::new(0.007, 0.0095));

    let hand = commands
        .spawn((Name::new("Grenade hand"), HandRoot, Transform::from_xyz(0.17, -0.2, -0.38), Visibility::Hidden, ChildOf(root)))
        .id();
    let part = |commands: &mut Commands, parent: Entity, mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>, tf: Transform| {
        commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), tf, NotShadowCaster, ChildOf(parent)));
    };
    // The grenades (hand space: +Y up, -Z forward), one shown at a time.
    let models: [(GrenadeKind, Vec<(Handle<Mesh>, Handle<StandardMaterial>, Transform)>); 3] = [
        (
            GrenadeKind::Frag,
            vec![
                (sphere.clone(), olive.clone(), Transform::from_xyz(0.0, 0.03, 0.0).with_scale(Vec3::new(0.033, 0.039, 0.033))),
                (tube.clone(), steel.clone(), Transform::from_xyz(0.0, 0.074, 0.0).with_scale(Vec3::new(0.011, 0.016, 0.011))),
                // The spoon down the side, the pin ring on top.
                (cube.clone(), steel.clone(), Transform::from_xyz(0.03, 0.045, 0.0).with_rotation(Quat::from_rotation_z(-0.25)).with_scale(Vec3::new(0.004, 0.05, 0.012))),
                (ring.clone(), steel.clone(), Transform::from_xyz(-0.016, 0.08, 0.0).with_rotation(Quat::from_rotation_z(1.2))),
            ],
        ),
        (
            GrenadeKind::Flash,
            vec![
                (tube.clone(), grey.clone(), Transform::from_xyz(0.0, 0.035, 0.0).with_scale(Vec3::new(0.024, 0.1, 0.024))),
                (tube.clone(), band_white.clone(), Transform::from_xyz(0.0, 0.05, 0.0).with_scale(Vec3::new(0.0245, 0.012, 0.0245))),
                (tube.clone(), steel.clone(), Transform::from_xyz(0.0, 0.093, 0.0).with_scale(Vec3::new(0.012, 0.018, 0.012))),
                (cube.clone(), steel.clone(), Transform::from_xyz(0.026, 0.06, 0.0).with_scale(Vec3::new(0.004, 0.06, 0.012))),
                (ring.clone(), steel.clone(), Transform::from_xyz(-0.016, 0.1, 0.0).with_rotation(Quat::from_rotation_z(1.2))),
            ],
        ),
        (
            GrenadeKind::Smoke,
            vec![
                (tube.clone(), smoke_body.clone(), Transform::from_xyz(0.0, 0.04, 0.0).with_scale(Vec3::new(0.03, 0.11, 0.03))),
                (tube.clone(), band_green.clone(), Transform::from_xyz(0.0, 0.065, 0.0).with_scale(Vec3::new(0.0305, 0.015, 0.0305))),
                (tube.clone(), steel.clone(), Transform::from_xyz(0.0, 0.103, 0.0).with_scale(Vec3::new(0.012, 0.018, 0.012))),
                (cube.clone(), steel.clone(), Transform::from_xyz(0.032, 0.07, 0.0).with_scale(Vec3::new(0.004, 0.06, 0.012))),
                (ring.clone(), steel.clone(), Transform::from_xyz(-0.016, 0.11, 0.0).with_rotation(Quat::from_rotation_z(1.2))),
            ],
        ),
    ];
    for (kind, parts) in models {
        let g = commands.spawn((Name::new(format!("{} in hand", kind.label())), HandGrenade(kind), Transform::default(), Visibility::Hidden, ChildOf(hand))).id();
        for (m, s, tf) in parts {
            part(commands, g, &m, &s, tf);
        }
    }
    // A gloved fist round the grenade: palm behind, four fingers wrapped across the front,
    // thumb over the spoon; then the wrist, the team cuff and the sleeve running back and down.
    // Rounded segment a → b of radius r (the unit capsule is 4 tall: caps add r each end).
    let seg = |commands: &mut Commands, a: Vec3, b: Vec3, r: f32| {
        let d = b - a;
        let len = d.length().max(1e-4);
        let tf = Transform::from_translation((a + b) * 0.5)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, d / len))
            .with_scale(Vec3::new(r, (len + 2.0 * r) / 4.0, r));
        part(commands, hand, &capsule, &glove, tf);
    };
    // Palm behind the grenade (toward the camera, right), fingers curled across its front,
    // the thumb pinning the spoon.
    part(commands, hand, &sphere, &glove, Transform::from_xyz(0.02, 0.012, 0.03).with_scale(Vec3::new(0.026, 0.034, 0.02)));
    for i in 0..4 {
        let y = 0.036 - i as f32 * 0.017;
        let r = 0.0082 - i as f32 * 0.0005;
        let (a, b, tip) = (Vec3::new(0.034, y, 0.012), Vec3::new(0.02, y, -0.034), Vec3::new(-0.022, y, -0.03));
        seg(commands, a, b, r);
        seg(commands, b, tip, r * 0.92);
    }
    seg(commands, Vec3::new(0.012, 0.03, 0.03), Vec3::new(0.03, 0.06, 0.0), 0.0085);
    let wrist = Vec3::new(0.02, -0.015, 0.06);
    let elbow = Vec3::new(0.16, -0.32, 0.36);
    let d = elbow - wrist;
    let len = d.length();
    let rot = Quat::from_rotation_arc(Vec3::Y, d / len);
    part(commands, hand, &tube, &glove, Transform::from_translation(wrist + d / len * 0.015).with_rotation(rot).with_scale(Vec3::new(0.026, 0.05, 0.026)));
    part(commands, hand, &tube, &cuff, Transform::from_translation(wrist + d / len * 0.05).with_rotation(rot).with_scale(Vec3::new(0.031, 0.018, 0.031)));
    let s0 = 0.06;
    part(commands, hand, &tube, &sleeve, Transform::from_translation(wrist + d / len * (s0 + (len - s0) * 0.5)).with_rotation(rot).with_scale(Vec3::new(0.034, len - s0, 0.034)));
}

/// Shows the hand while a grenade is out: raise, wind-up (overhand: up and back; underhand:
/// down), the throw's forward snap, then empty until the next one comes up.
#[allow(clippy::too_many_arguments)]
fn animate_hand(
    mut commands: Commands,
    time: Res<Time>,
    hand: Res<GrenadeHand>,
    roots: Query<Entity, With<ViewRoot>>,
    mut hands: Query<(&mut Transform, &mut Visibility), (With<HandRoot>, Without<HandGrenade>)>,
    mut models: Query<(&HandGrenade, &mut Visibility), Without<HandRoot>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    fabric: (Option<Res<AssetServer>>, Option<Res<crate::assets::AssetsDir>>, Option<ResMut<crate::gfx::PendingMips>>),
    mut smooth: Local<(Vec3, Vec3)>,
) {
    let Ok((mut tf, mut vis)) = hands.single_mut() else {
        if let (Ok(root), Some(mut m), Some(mut s)) = (roots.single(), meshes, materials) {
            let sleeve = match fabric {
                (Some(server), Some(dir), Some(mut p)) => super::viewmodel::sleeve_fabric(&server, &dir, &mut p),
                _ => None,
            };
            spawn_hand(&mut commands, root, &mut m, &mut s, sleeve);
        }
        return;
    };
    let dt = time.delta_secs().clamp(1e-4, 0.05);
    let rest = Vec3::new(0.17, -0.2, -0.38);
    let Some(kind) = hand.equipped else {
        if *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
        return;
    };
    // Throw follow-through: snap forward and out of view in the first part, then empty.
    let thrown = hand.recover > 0.0;
    let snap = if thrown { 1.0 - (hand.recover / RECOVER_S) } else { 0.0 };
    let show_grenade = !thrown;
    let raise = ease(hand.raise / EQUIP_S);
    let (mut pos, mut euler) = (rest + Vec3::new(0.0, -0.25 * (1.0 - raise), 0.05 * (1.0 - raise)), Vec3::ZERO);
    if let Some((style, t)) = hand.cocked {
        let k = ease(t);
        let (dp, de) = match style {
            ThrowStyle::Over => (Vec3::new(0.05, 0.12, 0.14), Vec3::new(0.7, -0.2, 0.25)),
            ThrowStyle::Mid => (Vec3::new(0.02, 0.02, 0.1), Vec3::new(0.3, -0.1, 0.1)),
            ThrowStyle::Under => (Vec3::new(-0.04, -0.12, 0.06), Vec3::new(-0.5, 0.0, -0.1)),
        };
        pos += dp * k;
        euler += de * k;
    }
    if thrown {
        let f = (snap / 0.35).min(1.0);
        pos += Vec3::new(-0.08 * f, 0.02 * f - 0.4 * (snap - 0.35).max(0.0) * 2.0, -0.25 * f);
        euler += Vec3::new(-0.9 * f, 0.0, 0.0);
    }
    // A little lag so moves are smooth.
    smooth.0 = smooth.0.lerp(pos, approach(22.0, dt));
    smooth.1 = smooth.1.lerp(euler, approach(22.0, dt));
    let new = Transform::from_translation(smooth.0).with_rotation(Quat::from_euler(EulerRot::XYZ, smooth.1.x, smooth.1.y, smooth.1.z));
    if *tf != new {
        *tf = new;
    }
    if *vis != Visibility::Inherited {
        *vis = Visibility::Inherited;
    }
    for (g, mut v) in &mut models {
        let want = if g.0 == kind && show_grenade { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
}

// ---------------------------------------------------------------- HUD

#[derive(Component)]
struct GrenadeHudText(GrenadeKind);

#[derive(Component)]
struct GrenadeHudHint;

/// Above the gun panel (bottom right): FRAG 1 · FLASH 2 · SMOKE 1, the one in hand highlighted.
fn spawn_hud(mut commands: Commands) {
    commands
        .spawn((
            Name::new("Grenade HUD"),
            DespawnOnExit(GameState::Playing),
            HideWhenPaused,
            Node {
                position_type: PositionType::Absolute,
                right: px(40),
                bottom: px(214),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: px(2),
                padding: UiRect::axes(px(12), px(4)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.5)),
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            c.spawn((Node { column_gap: px(GAP_M), align_items: AlignItems::Baseline, ..default() }, Pickable::IGNORE)).with_children(|r| {
                r.spawn(text("4 / G", label(SIZE_TINY), TEXT_DIM));
                for k in GrenadeKind::ALL {
                    r.spawn((GrenadeHudText(k), text("", label(SIZE_TINY + 2.0), TEXT_DIM)));
                }
            });
            c.spawn((GrenadeHudHint, text("", label(SIZE_TINY), TEXT_MUTED)));
        });
}

fn update_hud(
    bag: Res<PlayerGrenades>,
    hand: Res<GrenadeHand>,
    mut texts: Query<(&GrenadeHudText, &mut Text, &mut TextColor), Without<GrenadeHudHint>>,
    mut hint: Query<&mut Text, With<GrenadeHudHint>>,
) {
    if !bag.is_changed() && !hand.is_changed() {
        return;
    }
    for (g, mut t, mut c) in &mut texts {
        let n = bag.0.count(g.0);
        let name = match g.0 {
            GrenadeKind::Frag => "FRAG",
            GrenadeKind::Flash => "FLASH",
            GrenadeKind::Smoke => "SMOKE",
        };
        let s = if n == 0 { String::new() } else { format!("{name} {n}") };
        if t.0 != s {
            t.0 = s;
        }
        let col = if hand.equipped == Some(g.0) { ACCENT } else { TEXT };
        if c.0 != col {
            c.0 = col;
        }
    }
    let s = if hand.out() { "LMB THROW  \u{b7}  RMB LOB" } else { "" };
    for mut t in &mut hint {
        if t.0 != s {
            t.0 = s.into();
        }
    }
}

#[cfg(feature = "dev")]
mod commands {
    use engine::devtools::{AddControlCommand, params};
    use serde_json::{Value, json};

    use super::*;

    pub(super) fn register(app: &mut App) {
        app.add_control_command(
            "grenade.equip",
            "Take a grenade out (or put it away)",
            params(&[("kind", "string", "frag, flash, smoke, or none for the gun")]),
            equip_cmd,
        )
        .add_control_command(
            "grenade.throw",
            "Throw a grenade the way the player does (pin, wind-up, release)",
            params(&[
                ("kind", "string", "frag, flash or smoke (default: the one in hand)"),
                ("style", "string", "over (default), mid or under"),
                ("give", "boolean", "add one to the bag first, free (default false)"),
            ]),
            throw_cmd,
        )
        .add_control_command("grenade.state", "Grenades carried and the one in hand", params(&[]), state);
    }

    fn kind_of(s: &str) -> Option<GrenadeKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "frag" | "he" => Some(GrenadeKind::Frag),
            "flash" | "flashbang" => Some(GrenadeKind::Flash),
            "smoke" => Some(GrenadeKind::Smoke),
            _ => None,
        }
    }

    fn state(world: &mut World, _: Value) -> Result<Value, String> {
        let b = world.resource::<PlayerGrenades>().0;
        let h = world.resource::<GrenadeHand>().clone();
        Ok(json!({
            "frag": b.frag, "flash": b.flash, "smoke": b.smoke,
            "equipped": h.equipped.map(|k| k.label()),
            "cocked": h.cocked.map(|(s, t)| json!({"style": format!("{s:?}"), "t": t})),
            "thrown": h.thrown,
        }))
    }

    fn equip_cmd(world: &mut World, p: Value) -> Result<Value, String> {
        let s = p.get("kind").and_then(Value::as_str).unwrap_or("frag");
        let k = if s == "none" { None } else { Some(kind_of(s).ok_or_else(|| format!("unknown grenade `{s}`"))?) };
        if let Some(k) = k
            && world.resource::<PlayerGrenades>().0.count(k) == 0
        {
            return Err(format!("no {} carried", k.label()));
        }
        world.resource_mut::<GrenadeOverride>().equip = Some(k);
        Ok(json!({"equip": k.map(|k| k.label())}))
    }

    fn throw_cmd(world: &mut World, p: Value) -> Result<Value, String> {
        let style = match p.get("style").and_then(Value::as_str) {
            Some(s) => ThrowStyle::parse(s).ok_or_else(|| format!("unknown style `{s}`"))?,
            None => ThrowStyle::Over,
        };
        let kind = match p.get("kind").and_then(Value::as_str) {
            Some(s) => Some(kind_of(s).ok_or_else(|| format!("unknown grenade `{s}`"))?),
            None => world.resource::<GrenadeHand>().equipped,
        }
        .ok_or("no grenade in hand; pass `kind`")?;
        if p.get("give").and_then(Value::as_bool).unwrap_or(false) {
            world.resource_mut::<PlayerGrenades>().0.add(kind);
        }
        if world.resource::<PlayerGrenades>().0.count(kind) == 0 {
            return Err(format!("no {} carried", kind.label()));
        }
        let mut ov = world.resource_mut::<GrenadeOverride>();
        ov.equip = Some(Some(kind));
        ov.throw = Some(style);
        Ok(json!({"kind": kind.label(), "style": format!("{style:?}")}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throws_go_where_you_look_and_overhand_goes_further() {
        let fwd = Vec3::NEG_Z;
        for s in [ThrowStyle::Over, ThrowStyle::Mid, ThrowStyle::Under] {
            let v = throw_velocity(s, fwd, Vec3::ZERO);
            assert!(v.z < 0.0 && v.y > 0.0, "{s:?}: forward and a little up ({v:?})");
            assert!(v.x.abs() < 1e-4);
        }
        let range = |s| {
            let v = throw_velocity(s, fwd, Vec3::ZERO);
            // Flat ground from 1.5 m: time to land, then horizontal distance.
            let t = (v.y + (v.y * v.y + 2.0 * 9.81 * 1.5).sqrt()) / 9.81;
            -v.z * t
        };
        assert!(range(ThrowStyle::Over) > 20.0, "{}", range(ThrowStyle::Over));
        assert!(range(ThrowStyle::Over) > range(ThrowStyle::Mid) && range(ThrowStyle::Mid) > range(ThrowStyle::Under));
        assert!(range(ThrowStyle::Under) < 10.0);
    }
}
