//! Rendering look: sky, sun, post-processing, PBR texture loading with
//! world-scale UVs, procedural FX textures. Nothing here runs headless.

use bevy::anti_alias::fxaa::Fxaa;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{
    Atmosphere, AtmosphereEnvironmentMapLight, CascadeShadowConfigBuilder, DirectionalLightShadowMap, light_consts::lux,
};
use bevy::mesh::VertexAttributeValues;
use bevy::pbr::{
    AtmosphereSettings, DistanceFog, FogFalloff, ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel,
};
use bevy::platform::collections::HashSet;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::camera::{MipBias, TemporalJitter};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::assets::AssetsDir;
use crate::settings::GraphicsQuality;
use crate::state::has_visuals;

/// Haze colour for distance fog (matches the low-sun sky near the horizon).
pub const FOG_COLOR: Color = Color::srgb(0.58, 0.62, 0.70);

pub struct GfxPlugin;

impl Plugin for GfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingMips>()
            .add_systems(Startup, (spawn_sky_and_sun, init_fx_assets).run_if(has_visuals))
            .add_systems(Update, (generate_mips, face_camera, apply_camera_quality).run_if(has_visuals));
    }
}

/// Render components for the first-person camera (never used headless). The quality-dependent
/// parts (AA, SSAO, bloom, shadow resolution) are added by [`apply_camera_quality`], which
/// also re-applies them whenever `Settings::quality` changes.
pub fn camera_components(quality: GraphicsQuality) -> impl Bundle {
    (
        Msaa::Off,
        WantQuality(quality),
        Tonemapping::AgX,
        Exposure { ev100: 13.0 },
        AtmosphereSettings::default(),
        AtmosphereEnvironmentMapLight {
            intensity: 1.0,
            size: UVec2::splat(128),
            ..default()
        },
        DistanceFog {
            color: FOG_COLOR,
            directional_light_color: Color::srgba(1.0, 0.85, 0.65, 0.35),
            directional_light_exponent: 24.0,
            falloff: FogFalloff::Linear {
                start: 30.0,
                end: 180.0,
            },
        },
    )
}

/// Quality preset currently applied to a camera's render features.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WantQuality(pub GraphicsQuality);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppliedQuality(GraphicsQuality);

/// Low: FXAA, no SSAO/bloom, 1k shadows. Medium: TAA. High: TAA + SSAO + bloom.
/// Ultra: TAA + best SSAO + 4k shadows.
pub fn apply_camera_quality(
    mut commands: Commands,
    settings: Res<crate::settings::Settings>,
    mut cams: Query<(Entity, &mut WantQuality, Option<&AppliedQuality>)>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
) {
    let q = settings.quality;
    for (e, mut want, applied) in &mut cams {
        if want.0 != q {
            want.0 = q;
        }
        if applied.is_some_and(|a| a.0 == q) {
            continue;
        }
        let mut ec = commands.entity(e);
        ec.insert(AppliedQuality(q));
        let bloom = Bloom {
            intensity: 0.12,
            ..Bloom::NATURAL
        };
        match q {
            GraphicsQuality::Low => {
                ec.remove::<(TemporalAntiAliasing, TemporalJitter, MipBias, ScreenSpaceAmbientOcclusion, Bloom)>()
                    .insert(Fxaa::default());
            }
            GraphicsQuality::Medium => {
                ec.remove::<(Fxaa, ScreenSpaceAmbientOcclusion, Bloom)>()
                    .insert(TemporalAntiAliasing::default());
            }
            GraphicsQuality::High | GraphicsQuality::Ultra => {
                let ssao = ScreenSpaceAmbientOcclusion {
                    quality_level: if q == GraphicsQuality::Ultra {
                        ScreenSpaceAmbientOcclusionQualityLevel::Ultra
                    } else {
                        ScreenSpaceAmbientOcclusionQualityLevel::High
                    },
                    ..default()
                };
                ec.remove::<Fxaa>().insert((TemporalAntiAliasing::default(), ssao, bloom));
            }
        }
        let size = match q {
            GraphicsQuality::Low => 1024,
            GraphicsQuality::Medium | GraphicsQuality::High => 2048,
            GraphicsQuality::Ultra => 4096,
        };
        if shadow_map.size != size {
            shadow_map.size = size;
        }
    }
}

fn spawn_sky_and_sun(mut commands: Commands, media: Option<ResMut<Assets<ScatteringMedium>>>) {
    if let Some(mut media) = media {
        commands.spawn((
            Name::new("Atmosphere"),
            Atmosphere::earth(media.add(ScatteringMedium::earth(256, 256))),
        ));
    } else {
        commands.insert_resource(ClearColor(FOG_COLOR));
    }
    // Low afternoon sun: long readable shadows, warm key vs. cool sky fill.
    // Each map re-aims and re-tints it (`map::look::apply_look`).
    commands.spawn((
        Name::new("Sun"),
        Sun,
        DirectionalLight {
            illuminance: lux::RAW_SUNLIGHT,
            shadow_maps_enabled: true,
            color: Color::srgb(1.0, 0.93, 0.84),
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: 3,
            first_cascade_far_bound: 10.0,
            maximum_distance: 90.0,
            ..default()
        }
        .build(),
        Transform::from_xyz(-30.0, 22.0, 26.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.7, 0.78, 0.95),
        brightness: 800.0,
        ..default()
    });
}

/// Marker on the one directional light.
#[derive(Component)]
pub struct Sun;

// ---------------------------------------------------------------- textures

/// Images that still need a CPU-built mip chain (JPGs ship without mips).
#[derive(Resource, Default)]
pub struct PendingMips(HashSet<AssetId<Image>>);

/// A PBR texture set under `assets/textures/<dir>/` (`color`, `normal`,
/// `rough`, optional `ao`, all `.jpg`). `None` if the colour map is missing.
pub fn load_pbr(
    server: &AssetServer,
    dir: &AssetsDir,
    pending: &mut PendingMips,
    name: &str,
    tint: Color,
) -> Option<StandardMaterial> {
    let path = |m: &str| format!("textures/{name}/{m}.jpg");
    if !dir.has(&path("color")) {
        return None;
    }
    let mut load = |m: &str, srgb: bool| -> Option<Handle<Image>> {
        let p = path(m);
        if !dir.has(&p) {
            return None;
        }
        let h = server
            .load_builder()
            .with_settings(move |s: &mut ImageLoaderSettings| {
                s.is_srgb = srgb;
                s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    // 16x anisotropic + trilinear mips: no shimmer on floors at grazing angles.
                    anisotropy_clamp: 16,
                    ..ImageSamplerDescriptor::linear()
                });
            })
            .load(p);
        pending.0.insert(h.id());
        Some(h)
    };
    Some(StandardMaterial {
        base_color: tint,
        base_color_texture: load("color", true),
        normal_map_texture: load("normal", false),
        // Greyscale roughness map: G drives roughness; `metallic: 0` zeroes B.
        metallic_roughness_texture: load("rough", false),
        occlusion_texture: load("ao", false),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        ..default()
    })
}

fn generate_mips(
    mut events: MessageReader<AssetEvent<Image>>,
    mut pending: ResMut<PendingMips>,
    mut images: ResMut<Assets<Image>>,
) {
    for ev in events.read() {
        let AssetEvent::LoadedWithDependencies { id } = ev else { continue };
        if !pending.0.remove(id) {
            continue;
        }
        if let Some(mut img) = images.get_mut(*id) {
            build_mips(&mut img);
        }
    }
}

/// Box-filtered mip chain for single-layer RGBA8 images.
fn build_mips(img: &mut Image) {
    let fmt = img.texture_descriptor.format;
    if !matches!(fmt, TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb)
        || img.texture_descriptor.mip_level_count != 1
        || img.texture_descriptor.size.depth_or_array_layers != 1
    {
        return;
    }
    let Some(base) = img.data.take() else { return };
    let (mut w, mut h) = (
        img.texture_descriptor.size.width as usize,
        img.texture_descriptor.size.height as usize,
    );
    let mut out = base.clone();
    let mut prev = base;
    let mut levels = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut s = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        s += prev[(sy * w + sx) * 4 + c] as u32;
                    }
                    next[(y * nw + x) * 4 + c] = (s / 4) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    img.data = Some(out);
    img.texture_descriptor.mip_level_count = levels;
}

/// A cuboid whose UVs are planar projections in world metres divided by `tile`,
/// so textures keep their scale on any block size and line up across blocks.
pub fn world_uv_cuboid(size: Vec3, center: Vec3, tile: f32) -> Mesh {
    let mut mesh = Mesh::from(Cuboid::from_size(size));
    let pos: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(p)) => p.clone(),
        _ => return mesh,
    };
    let nrm: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(n)) => n.clone(),
        _ => return mesh,
    };
    let uvs: Vec<[f32; 2]> = pos
        .iter()
        .zip(&nrm)
        .map(|(p, n)| {
            let w = Vec3::from(*p) + center;
            let n = Vec3::from(*n);
            let uv = if n.y.abs() > 0.5 {
                Vec2::new(w.x, w.z)
            } else if n.x.abs() > 0.5 {
                Vec2::new(w.z * -n.x.signum(), -w.y)
            } else {
                Vec2::new(w.x * n.z.signum(), -w.y)
            };
            (uv / tile).to_array()
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.with_generated_tangents().unwrap_or_else(|_| Mesh::from(Cuboid::from_size(size)))
}

// ---------------------------------------------------------------- FX assets

/// Shared meshes/materials for shot effects.
#[derive(Resource, Clone)]
pub struct FxAssets {
    pub quad: Handle<Mesh>,
    pub cube: Handle<Mesh>,
    pub flash: Handle<StandardMaterial>,
    pub tracer: Handle<StandardMaterial>,
    pub spark: Handle<StandardMaterial>,
    pub decal: Handle<StandardMaterial>,
    /// Soft round sprite for dust/blood puffs (cloned per puff to fade).
    pub puff_tex: Handle<Image>,
}

/// Rotates the entity to face the active camera every frame.
#[derive(Component)]
pub struct FaceCamera;

fn face_camera(
    cams: Query<&GlobalTransform, With<Camera3d>>,
    mut q: Query<(&mut Transform, &GlobalTransform), (With<FaceCamera>, Without<Camera3d>)>,
) {
    let Some(cam) = cams.iter().next() else { return };
    let eye = cam.translation();
    for (mut tf, gtf) in &mut q {
        let to = eye - gtf.translation();
        if to.length_squared() > 1e-6 {
            let roll = tf.rotation.to_euler(EulerRot::YXZ).2;
            tf.look_to(-to, Vec3::Y);
            tf.rotate_local_z(roll);
        }
    }
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
    let puff_tex = images.add(tex(64, |u, v| {
        let r = (u * u + v * v).sqrt();
        let a = (1.0 - r).max(0.0).powf(1.6);
        [1.0, 1.0, 1.0, a]
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
        spark: mats.add(StandardMaterial {
            base_color: Color::srgb(9.0, 5.0, 1.5),
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
        puff_tex,
    };
    commands.insert_resource(fx);
}
