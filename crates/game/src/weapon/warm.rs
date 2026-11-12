//! Shot-effect warm-up at round start (the loading screen's `WarmShots`, or the engine's
//! `WarmPipelines`).
//!
//! The engine's `WarmPipelines` draws every `StandardMaterial` once, but the first bullet impact
//! still froze the frame for ~1 s on this M5 Pro (fresh launch, dev build; 0.3-0.45 s in the
//! owner's release build): impact particles, bullet-hole decals and tracers have their own mesh
//! layouts, materials and blend states, compiled on first use. So when the warm-up runs, one
//! impact per surface (and a tracer) is played *inside the floor slab*, in front of the camera:
//! inside the view frustum (so every pipeline is queued and compiled) but behind the floor's
//! depth (so nothing shows). The impacts are `exit` impacts, which are silent (`audio`), and the
//! bullet holes go to the normal pool.

use bevy::prelude::*;
use engine::gfx::MainCamera;
use engine::render_budget::WarmPipelines;

use super::{Impact, Surface, Tracer};

pub(super) fn build(app: &mut App) {
    app.add_systems(
        Update,
        warm_shot_fx.run_if(on_message::<WarmPipelines>.or_else(on_message::<crate::loading::WarmShots>)).run_if(crate::state::has_visuals),
    );
}

/// Where the warm-up effects go: 2.5 m ahead of the camera on the ground plane, 0.4 m into the
/// floor slab (its top is y = 0, 1 m thick).
pub fn hidden_spot(cam: &GlobalTransform) -> Vec3 {
    let f = cam.forward().as_vec3();
    let flat = Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z);
    let p = cam.translation() + flat * 2.5;
    Vec3::new(p.x, -0.4, p.z)
}

fn warm_shot_fx(cams: Query<&GlobalTransform, With<MainCamera>>, mut impacts: MessageWriter<Impact>, mut tracers: MessageWriter<Tracer>) {
    let Some(cam) = cams.iter().next() else { return };
    let at = hidden_spot(cam);
    for surface in [Surface::Concrete, Surface::Plaster, Surface::Metal, Surface::Wood, Surface::Suit] {
        impacts.write(Impact { pos: at, normal: Vec3::Y, flesh: false, surface, exit: true });
    }
    impacts.write(Impact { pos: at, normal: Vec3::Y, flesh: true, surface: Surface::Suit, exit: true });
    tracers.write(Tracer { from: at + Vec3::X, to: at - Vec3::X });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_up_spot_is_ahead_and_inside_the_floor() {
        let cam = GlobalTransform::from(Transform::from_xyz(3.0, 1.65, 4.0).looking_to(Vec3::new(1.0, -0.3, 0.0), Vec3::Y));
        let p = hidden_spot(&cam);
        assert!((p.y + 0.4).abs() < 1e-6);
        assert!((p.x - 5.5).abs() < 1e-4 && (p.z - 4.0).abs() < 1e-4);
    }
}
