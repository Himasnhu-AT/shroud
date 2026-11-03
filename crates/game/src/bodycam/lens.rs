//! Bodycam optics (spec §4): the sensor pass ([`BodycamLens`], a Bevy `FullscreenMaterial`
//! scheduled after tonemapping and FXAA) on the main camera, and the look changes that go with
//! it (auto exposure with slow dark / fast bright adaptation, AgX, stronger bloom; the engine
//! look's own lens effects are off because the pass does them). The wide FOV is
//! [`super::base_fov_deg`] (used by the ADS zoom in `weapon::arsenal`).
//!
//! Bloom stays on the quality tiers that have it (High, Ultra): adding it on Medium would change
//! every material's pipeline key, a rebuild the guide forbids outside loads.

use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::BoxedSystem;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;
use engine::gfx::MainCamera;
use engine::look::Look;

use super::is_bodycam;
use crate::settings::ShroudSettings;
use crate::state::has_visuals;

/// Per-camera bodycam sensor settings (spec §4.1 numbers; `time` animates the grain).
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Debug, PartialEq)]
#[extract_app(bevy::render::RenderApp)]
pub struct BodycamLens {
    pub fisheye_strength: f32,
    pub chromatic_aberration: f32,
    pub vignette_intensity: f32,
    pub sensor_grain_strength: f32,
    pub sharpness_strength: f32,
    pub time: f32,
    pub _pad0: f32,
    pub _pad1: f32,
}

impl Default for BodycamLens {
    fn default() -> Self {
        BodycamLens {
            fisheye_strength: 0.32,
            chromatic_aberration: 0.018,
            vignette_intensity: 0.72,
            sensor_grain_strength: 0.045,
            sharpness_strength: 0.65,
            time: 0.0,
            _pad0: 0.0,
            _pad1: 0.0,
        }
    }
}

impl FullscreenMaterial for BodycamLens {
    fn fragment_shader() -> ShaderRef {
        "embedded://shroud/bodycam/bodycam_post.wesl".into()
    }

    /// After tonemapping and FXAA: the sensor sees the finished picture (grain is not smoothed
    /// away, the vignette is not tonemapped).
    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system
            .in_set(bevy::core_pipeline::Core3dSystems::PostProcess)
            .after(bevy::core_pipeline::tonemapping::tonemapping)
            .after(bevy::anti_alias::fxaa::fxaa)
    }
}

pub(super) fn build(app: &mut App) {
    if app.get_sub_app(bevy::render::RenderApp).is_none() {
        return;
    }
    bevy::asset::embedded_asset!(app, "bodycam_post.wesl");
    app.add_plugins(FullscreenMaterialPlugin::<BodycamLens>::default())
        .add_systems(Update, (sync_lens, animate_lens.run_if(any_with_component::<BodycamLens>)).chain().run_if(has_visuals));
}

/// The sensor pass on the main camera exactly while bodycam is on.
fn sync_lens(mut commands: Commands, settings: Res<ShroudSettings>, cams: Query<(Entity, Has<BodycamLens>), (With<MainCamera>, With<Camera3d>)>) {
    let want = is_bodycam(&settings);
    for (e, has) in &cams {
        if want && !has {
            commands.entity(e).insert(BodycamLens::default());
        } else if !want && has {
            commands.entity(e).remove::<BodycamLens>();
        }
    }
}

fn animate_lens(time: Res<Time<Real>>, mut q: Query<&mut BodycamLens>) {
    let t = time.elapsed_secs() % 1000.0;
    for mut l in &mut q {
        l.time = t;
    }
}

/// A map's look as the bodycam sees it: metered auto exposure (shadows adapt slowly, highlights
/// blow out and recover fast), AgX, spec bloom, no engine lens effects (the pass does them).
pub fn bodycam_look(mut look: Look) -> Look {
    look.name = format!("{} (bodycam)", look.name).into();
    look.exposure.auto = true;
    look.exposure.auto_min = -3.0;
    look.exposure.auto_max = 3.0;
    // EV per second: bright → dark is slow, dark → bright is fast.
    look.exposure.speed_darken = 1.2;
    look.exposure.speed_brighten = 3.5;
    // Meter a little under mid-grey: open daylight keeps its contrast, dark rooms still lift.
    look.exposure.compensation = -1.7;
    look.tonemapping = engine::look::ToneMap::Agx;
    look.bloom.intensity = 0.28;
    look.bloom.low_frequency_boost = 0.6;
    look.lens.vignette.intensity = 0.0;
    look.lens.chromatic_aberration = 0.0;
    look.lens.distortion = 0.0;
    look.lens.film_grain = 0.0;
    look
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodycam_look_is_a_distinct_auto_exposed_look() {
        let base = Look::default();
        let b = bodycam_look(base.clone());
        assert_ne!(b.name, base.name);
        assert!(b.exposure.auto && b.exposure.speed_darken < b.exposure.speed_brighten);
        assert_eq!(b.lens.film_grain, 0.0);
        assert_eq!(std::mem::size_of::<BodycamLens>() % 16, 0);
    }
}
