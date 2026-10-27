//! Shroud's look on top of the engine's rendering: camera exposure/tonemapping, distance fog,
//! atmosphere sky, sun, the quality presets, render-budget ranges and procedural FX textures.
//! (The engine applies AA/SSAO/bloom/shadow size per quality, FOV, PBR loading + mips, world-UV
//! cuboids and camera-facing billboards.) Nothing here runs headless.

use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use engine::gfx::QualityPreset;
use engine::render_budget::SceneryRanges;

pub use engine::gfx::{PendingMips, load_pbr, world_uv_cuboid};

use crate::state::has_visuals;

/// Where the fog is opaque: nothing is drawn beyond it.
pub const FOG_END: f32 = 180.0;

pub struct GfxPlugin;

impl Plugin for GfxPlugin {
    fn build(&self, app: &mut App) {
        // Medium keeps TAA but no bloom (the engine's default Medium has bloom).
        app.insert_resource(QualityPreset { bloom: [false, false, true, true], ..default() })
            // Arenas are at most ~91 m corner to corner: even on Low (x0.6) nothing inside one
            // fades, loses its shadow or goes flat; scenery stops at the fog.
            .insert_resource(SceneryRanges { shadow: 160.0, small_props: 160.0, far: 160.0, view: FOG_END, ..default() })
            .add_systems(Startup, (sky_and_sun, init_fx_assets).run_if(has_visuals));
    }
}

/// Shroud's camera look before the map's engine `Look` applies (never used headless). The
/// look (`map::look::look_of`, engine feature `look`) then owns exposure, tonemapping, fog,
/// sky and env light on the `MainCamera`; quality-dependent parts (AA, SSAO, bloom, shadow
/// resolution) and FOV come from the engine.
pub fn camera_look() -> impl Bundle {
    (Tonemapping::AgX, Exposure { ev100: 13.0 })
}

/// Atmosphere sky, low afternoon sun (each map re-aims and re-tints it in `map::look`), cool
/// ambient fill.
fn sky_and_sun(mut commands: Commands) {
    engine::gfx::sky_and_sun(&mut commands);
}

// ---------------------------------------------------------------- FX assets

/// Shared meshes/materials for shot effects.
#[derive(Resource, Clone)]
pub struct FxAssets {
    pub quad: Handle<Mesh>,
    pub cube: Handle<Mesh>,
    pub flash: Handle<StandardMaterial>,
    pub tracer: Handle<StandardMaterial>,
    pub decal: Handle<StandardMaterial>,
}

fn tex(size: u32, f: impl Fn(f32, f32) -> [f32; 4]) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            for c in f(u, v) {
                data.push((c.clamp(0.0, 1.0) * 255.0) as u8);
            }
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn init_fx_assets(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    // Muzzle flash: hot core + 6 spikes.
    let flash_tex = images.add(tex(128, |u, v| {
        let r = (u * u + v * v).sqrt();
        let a = v.atan2(u);
        let spikes = (a * 3.0).cos().abs().powf(12.0) * (1.0 - r).max(0.0);
        let core = (1.0 - r * 2.2).max(0.0).powf(1.5);
        let i = (core + spikes * 0.9).min(1.0);
        // Premultiplied for additive blending.
        [i, (0.85 + 0.15 * core) * i, (0.55 + 0.45 * core) * i, i]
    }));
    // Bullet hole: dark core, scorched soft ring.
    let decal_tex = images.add(tex(64, |u, v| {
        let r = (u * u + v * v).sqrt();
        let core: f32 = if r < 0.22 { 1.0 } else { 0.0 };
        let ring = (1.0 - r).max(0.0).powf(1.3) * 0.75;
        let a = core.max(ring);
        let c = 0.02 + 0.08 * (r - 0.2).max(0.0);
        [c, c * 0.95, c * 0.9, a]
    }));
    let glow = |c: LinearRgba, t: Option<Handle<Image>>| StandardMaterial {
        base_color: Color::BLACK,
        base_color_texture: t.clone(),
        emissive: c,
        emissive_texture: t,
        alpha_mode: AlphaMode::Add,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    let fx = FxAssets {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        flash: mats.add(StandardMaterial {
            base_color: Color::srgb(4.0, 2.6, 1.2),
            ..glow(LinearRgba::NONE, Some(flash_tex))
        }),
        tracer: mats.add(StandardMaterial {
            base_color: Color::srgb(6.0, 4.0, 1.6),
            ..glow(LinearRgba::NONE, None)
        }),
        decal: mats.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(decal_tex),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            depth_bias: 8.0,
            ..default()
        }),
    };
    commands.insert_resource(fx);
}
