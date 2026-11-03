//! F3 debug overlay: each enemy's path, destination, predicted player spot, and aim, plus the
//! squad's adaptive doctrine (tactic, why, archetypes) in a panel and its counters in the world
//! (jiggle spot, wallbang line, rushers, off-angle traps).

use bevy::prelude::*;
use brain::utility::Goal;

use super::{Enemy, EnemyCombat, Mind, Squad};
use crate::map::to_world;
use crate::state::has_visuals;
use crate::ui::theme::{ACCENT, PANEL, TEXT, body, text};

/// Enemy debug view on/off (F3, or `debug.overlay` on the control port).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugOverlay(pub bool);

pub struct EnemyDebugPlugin;

impl Plugin for EnemyDebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugOverlay>().add_systems(
            Update,
            (toggle, draw.run_if(|d: Res<DebugOverlay>| d.0), draw_doctrine.run_if(|d: Res<DebugOverlay>| d.0), doctrine_panel).chain().run_if(has_visuals),
        );
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut d: ResMut<DebugOverlay>) {
    if keys.just_pressed(KeyCode::F3) {
        d.0 = !d.0;
    }
}

pub fn goal_color(g: Goal) -> Color {
    match g {
        Goal::Attack => Color::srgb(1.0, 0.2, 0.2),
        Goal::Push => Color::srgb(1.0, 0.55, 0.1),
        Goal::Flank => Color::srgb(0.9, 0.2, 0.9),
        Goal::TakeCover => Color::srgb(0.2, 0.6, 1.0),
        Goal::Retreat => Color::srgb(0.5, 0.5, 0.5),
        Goal::Ambush => Color::srgb(0.2, 1.0, 0.4),
        Goal::Hunt => Color::srgb(1.0, 1.0, 0.3),
        Goal::Hold => Color::srgb(0.3, 0.9, 0.9),
    }
}

/// The doctrine panel (top left, F3 only).
#[derive(Component)]
struct DoctrinePanel;

fn doctrine_panel(mut commands: Commands, overlay: Res<DebugOverlay>, squad: Res<Squad>, panel: Query<Entity, With<DoctrinePanel>>, kids: Query<&Children>, mut texts: Query<&mut Text>) {
    let line = squad.coord.as_ref().map(|c| {
        let d = &c.doctrine;
        let arch = if d.active.is_empty() { "none".to_string() } else { d.active.labels().join(", ") };
        let p = &c.style;
        format!(
            "DOCTRINE  {}\n{}\narchetypes: {arch}\nspeed {:.1} m/s  hold {:.1} s  ads {:.0}%  hs {:.0}% ({} hits)  early reloads {}  entries {}",
            d.tactic.name(),
            if d.reason.is_empty() { "-" } else { &d.reason },
            p.avg_move_speed,
            p.stationary_angle_hold_secs,
            p.ads_usage_ratio * 100.0,
            p.headshot_rate * 100.0,
            p.hits,
            p.premature_reload_count,
            p.rapid_entries,
        )
    });
    match (overlay.0, panel.single()) {
        (true, Ok(e)) => {
            for k in kids.iter_descendants(e) {
                if let Ok(mut t) = texts.get_mut(k) {
                    let want = line.clone().unwrap_or_default();
                    if t.0 != want {
                        t.0 = want;
                    }
                }
            }
        }
        (true, Err(_)) => {
            commands
                .spawn((
                    Name::new("Doctrine panel"),
                    DoctrinePanel,
                    Node { position_type: PositionType::Absolute, left: px(16), top: px(16), padding: UiRect::all(px(8)), ..default() },
                    BackgroundColor(PANEL),
                    Pickable::IGNORE,
                ))
                .with_child(text(line.unwrap_or_default(), body(15.0), TEXT));
        }
        (false, Ok(e)) => commands.entity(e).despawn(),
        (false, Err(_)) => {}
    }
}

/// The doctrine's counters in the world: jiggle spot (and step), wallbang line, rush lines,
/// off-angle traps with the door they watch.
fn draw_doctrine(mut gizmos: Gizmos, squad: Res<Squad>, q: Query<(&Enemy, &Transform)>) {
    let Some(c) = squad.coord.as_ref() else { return };
    let pos = |slot: usize| q.iter().find(|(e, _)| e.slot == slot).map(|(_, tf)| tf.translation);
    let orange = ACCENT;
    if let Some((_, j)) = c.jiggler {
        let hide = to_world(j.hide, 0.05);
        gizmos.circle(Isometry3d::new(hide, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), 0.4, orange);
        gizmos.arrow(hide + Vec3::Y * 0.2, to_world(j.hide + j.out * 3.0, 0.25), orange);
    }
    if let Some((i, at, _)) = c.wallbang
        && let Some(p) = pos(i)
    {
        gizmos.line(p + Vec3::Y * 1.4, to_world(at, 1.1), Color::srgb(1.0, 0.1, 0.1));
    }
    for &(i, at, _) in &c.rushers {
        if let Some(p) = pos(i) {
            gizmos.arrow(p + Vec3::Y * 0.3, to_world(at, 0.3), Color::srgb(1.0, 0.85, 0.1));
        }
    }
    for &(_, spot, door) in &c.traps {
        let s = to_world(spot, 0.05);
        gizmos.circle(Isometry3d::new(s, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), 0.5, Color::srgb(0.2, 0.9, 1.0));
        gizmos.line(s + Vec3::Y * 0.2, to_world(door, 0.2), Color::srgb(0.2, 0.9, 1.0));
    }
}

fn draw(mut gizmos: Gizmos, q: Query<(&Transform, &Mind, &EnemyCombat), With<Enemy>>) {
    for (tf, mind, c) in &q {
        let d = &mind.0.debug;
        let color = d.goal.map(goal_color).unwrap_or(Color::WHITE);
        let head = tf.translation + Vec3::Y * 2.2;
        gizmos.sphere(Isometry3d::from_translation(head), 0.15, color);
        gizmos.line(tf.translation + Vec3::Y * 1.5, tf.translation + Vec3::Y * 1.5 + *tf.forward() * 3.0, color);
        let mut prev = tf.translation + Vec3::Y * 0.1;
        for wp in &d.path {
            let p = to_world(*wp, 0.1);
            gizmos.line(prev, p, Color::srgb(1.0, 0.9, 0.2));
            prev = p;
        }
        if let Some(dest) = d.dest {
            gizmos.sphere(Isometry3d::from_translation(to_world(dest, 0.3)), 0.3, color);
        }
        if let Some(p) = d.predicted {
            gizmos.sphere(Isometry3d::from_translation(to_world(p, 1.0)), 0.6, Color::srgb(1.0, 0.2, 1.0));
        }
        if c.sees_player {
            gizmos.sphere(Isometry3d::from_translation(head + Vec3::Y * 0.4), 0.08, Color::srgb(1.0, 0.0, 0.0));
        }
    }
}
