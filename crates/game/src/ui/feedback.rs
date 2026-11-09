//! Damage feedback: red edge vignette, damage-direction indicators, a pulsing low-HP vignette
//! and camera shake. Driven by [`Damage`] messages aimed at the player and the player's [`Health`].

use bevy::prelude::*;

use crate::audio::LOW_HP_FRAC;
use crate::player::{Player, PlayerSet};
use engine::gfx::MainCamera;
use crate::state::{GameState, has_visuals, not_paused};
use crate::weapon::{Damage, Health};

const INDICATORS: usize = 6;
const INDICATOR_LIFE: f32 = 1.6;
const FLASH_DECAY: f32 = 1.5;

pub struct FeedbackPlugin;

impl Plugin for FeedbackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Feedback>()
            .add_systems(Startup, spawn_feedback_ui.run_if(has_visuals))
            .add_systems(OnEnter(GameState::Playing), reset_feedback)
            .add_systems(
                Update,
                (collect_damage, update_vignette.run_if(has_visuals), update_indicators.run_if(has_visuals))
                    .chain()
                    .after(crate::weapon::WeaponSet)
                    .run_if(in_state(GameState::Playing)),
            )
            // After the player system rewrites the camera transform each frame, so the shake is
            // purely additive and vanishes as soon as trauma decays.
            .add_systems(
                Update,
                camera_shake
                    .after(PlayerSet::Move)
                    .run_if(in_state(GameState::Playing).and_then(has_visuals).and_then(not_paused)),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct Feedback {
    /// 0..1 red flash from recent hits, decays.
    flash: f32,
    /// 0..1 camera shake trauma, decays; amplitude goes with trauma squared.
    trauma: f32,
    /// World positions of recent attackers: (position, seconds left).
    slots: [Option<(Vec3, f32)>; INDICATORS],
    next_slot: usize,
}

impl Feedback {
    /// Add camera shake (0..1 trauma, e.g. a nearby explosion).
    pub(crate) fn shake(&mut self, trauma: f32) {
        self.trauma = (self.trauma + trauma).clamp(0.0, 1.0);
    }
}

#[derive(Component)]
struct VignetteEdge {
    /// Gradient direction in radians so the opaque end sits at the screen edge.
    angle: f32,
}

#[derive(Component)]
struct Indicator(usize);

#[derive(Component)]
struct IndicatorBar;

const RED: (f32, f32, f32) = (0.75, 0.02, 0.02);

fn red(a: f32) -> Color {
    Color::srgba(RED.0, RED.1, RED.2, a.clamp(0.0, 1.0))
}

fn edge_gradient(angle: f32, alpha: f32) -> BackgroundGradient {
    BackgroundGradient::from(LinearGradient::new(
        angle,
        vec![ColorStop::new(red(0.0), percent(0)), ColorStop::new(red(alpha), percent(100))],
    ))
}

fn spawn_feedback_ui(mut commands: Commands) {
    use std::f32::consts::{FRAC_PI_2, PI};
    let root = commands
        .spawn((
            Name::new("FeedbackRoot"),
            Node {
                width: percent(100),
                height: percent(100),
                position_type: PositionType::Absolute,
                ..default()
            },
            GlobalZIndex(5),
            Pickable::IGNORE,
        ))
        .id();

    // (angle, left, top, width, height): opaque side faces the screen edge.
    let edges = [
        (0.0, 0.0, 0.0, 100.0, 22.0),                  // top
        (PI, 0.0, 78.0, 100.0, 22.0),                  // bottom
        (-FRAC_PI_2, 0.0, 0.0, 16.0, 100.0),           // left
        (FRAC_PI_2, 84.0, 0.0, 16.0, 100.0),           // right
    ];
    for (angle, l, t, w, h) in edges {
        commands.spawn((
            VignetteEdge { angle },
            Node {
                position_type: PositionType::Absolute,
                left: percent(l),
                top: percent(t),
                width: percent(w),
                height: percent(h),
                ..default()
            },
            edge_gradient(angle, 0.0),
            Pickable::IGNORE,
            ChildOf(root),
        ));
    }

    // Direction indicators: a zero-size pivot at screen centre, rotated; the bar sits above it.
    for i in 0..INDICATORS {
        commands
            .spawn((
                Indicator(i),
                Node {
                    position_type: PositionType::Absolute,
                    left: percent(50),
                    top: percent(50),
                    width: px(0),
                    height: px(0),
                    display: Display::None,
                    ..default()
                },
                UiTransform::IDENTITY,
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .with_child((
                IndicatorBar,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(-60),
                    top: px(-210),
                    width: px(120),
                    height: px(26),
                    border_radius: BorderRadius::all(px(13)),
                    ..default()
                },
                BackgroundColor(red(0.0)),
                Pickable::IGNORE,
            ));
    }
}

fn reset_feedback(mut fb: ResMut<Feedback>) {
    *fb = Feedback::default();
}

fn collect_damage(
    time: Res<Time>,
    mut fb: ResMut<Feedback>,
    mut dmg: MessageReader<Damage>,
    player: Query<Entity, With<Player>>,
    sources: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    fb.flash = (fb.flash - FLASH_DECAY * dt).max(0.0);
    fb.trauma = (fb.trauma - 1.8 * dt).max(0.0);
    for s in fb.slots.iter_mut().flatten() {
        s.1 -= dt;
    }
    let Ok(player) = player.single() else { return };
    for d in dmg.read() {
        if d.target != player {
            continue;
        }
        fb.flash = (fb.flash + 0.35 + d.amount / 50.0).min(1.0);
        fb.trauma = (fb.trauma + 0.25 + d.amount / 60.0).min(1.0);
        if d.source != player
            && let Ok(src) = sources.get(d.source)
        {
            let i = fb.next_slot;
            fb.slots[i] = Some((src.translation(), INDICATOR_LIFE));
            fb.next_slot = (i + 1) % INDICATORS;
        }
    }
    for s in fb.slots.iter_mut() {
        if s.is_some_and(|(_, t)| t <= 0.0) {
            *s = None;
        }
    }
}

fn update_vignette(
    time: Res<Time>,
    fb: Res<Feedback>,
    player: Query<&Health, With<Player>>,
    mut edges: Query<(&VignetteEdge, &mut BackgroundGradient)>,
) {
    let mut alpha = fb.flash * 0.85;
    if let Ok(h) = player.single() {
        let frac = h.hp / h.max;
        if !h.is_dead() && frac < LOW_HP_FRAC {
            let severity = 1.0 - frac / LOW_HP_FRAC;
            // Pulse at roughly the heartbeat rate (see audio::heartbeat).
            let period = 1.0 - 0.45 * severity;
            let pulse = 0.5 + 0.5 * (time.elapsed_secs() * std::f32::consts::TAU / period).sin();
            alpha = alpha.max((0.25 + 0.4 * severity) * (0.55 + 0.45 * pulse));
        }
    }
    for (e, mut g) in &mut edges {
        *g = edge_gradient(e.angle, alpha);
    }
}

fn update_indicators(
    fb: Res<Feedback>,
    player: Query<&GlobalTransform, With<Player>>,
    mut pivots: Query<(&Indicator, &mut Node, &mut UiTransform, &Children)>,
    mut bars: Query<&mut BackgroundColor, With<IndicatorBar>>,
) {
    let Ok(ptf) = player.single() else { return };
    let ppos = ptf.translation();
    let fwd = ptf.forward().as_vec3();
    let fwd = Vec2::new(fwd.x, fwd.z).normalize_or_zero();
    let right = Vec2::new(-fwd.y, fwd.x);
    for (Indicator(i), mut node, mut tf, children) in &mut pivots {
        let slot = fb.slots[*i];
        node.display = if slot.is_some() { Display::Flex } else { Display::None };
        let Some((pos, left)) = slot else { continue };
        let d = Vec2::new(pos.x - ppos.x, pos.z - ppos.z);
        // 0 = straight ahead (top of screen), positive = clockwise (towards the right).
        let angle = d.dot(right).atan2(d.dot(fwd));
        tf.rotation = Rot2::radians(angle);
        let a = (left / INDICATOR_LIFE).clamp(0.0, 1.0).powf(0.7) * 0.9;
        for c in children.iter() {
            if let Ok(mut bg) = bars.get_mut(c) {
                bg.0 = red(a);
            }
        }
    }
}

fn camera_shake(
    time: Res<Time>,
    fb: Res<Feedback>,
    mut cams: Query<&mut Transform, With<MainCamera>>,
) {
    if fb.trauma <= 0.0 {
        return;
    }
    let t = time.elapsed_secs() * 38.0;
    let amp = fb.trauma * fb.trauma;
    let n = |k: f32| (t + k).sin() * 0.6 + (t * 1.7 + k * 2.3).sin() * 0.4;
    for mut tf in &mut cams {
        tf.rotation *= Quat::from_euler(EulerRot::YXZ, n(1.0) * 0.03 * amp, n(5.0) * 0.04 * amp, n(9.0) * 0.05 * amp);
        tf.translation += Vec3::new(n(13.0), n(17.0), 0.0) * 0.02 * amp;
    }
}
