//! F3 debug overlay: each enemy's path, destination, predicted player spot, and aim.

use bevy::prelude::*;
use brain::utility::Goal;

use super::{Enemy, EnemyCombat, Mind};
use crate::map::to_world;
use crate::state::has_visuals;

/// Enemy debug view on/off (F3, or `debug.overlay` on the control port).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugOverlay(pub bool);

pub struct EnemyDebugPlugin;

impl Plugin for EnemyDebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugOverlay>().add_systems(
            Update,
            (toggle, draw.run_if(|d: Res<DebugOverlay>| d.0)).chain().run_if(has_visuals),
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
