// Bodycam sensor (spec §4.2), one fullscreen pass after tonemapping (and FXAA), so it works on
// the window and on offscreen render-to-image cameras alike. Bevy's FullscreenMaterial binds:
// 0 = scene, 1 = sampler, 2 = settings. The scene is tonemapped but still linear; sharpening and
// grain work on a gamma-2.2 copy so they read the same in shadows and highlights.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

struct BodycamLens {
    fisheye_strength: f32,
    chromatic_aberration: f32,
    vignette_intensity: f32,
    sensor_grain_strength: f32,
    sharpness_strength: f32,
    time: f32,
    _pad0: f32,
    _pad1: f32,
}
@group(0) @binding(2) var<uniform> settings: BodycamLens;

// Barrel distortion on an aspect-correct radius, normalised so the screen corners still sample
// the image corners: the centre is magnified, the edges squeezed, nothing is cut off.
fn distort_uv(uv: vec2<f32>, k: f32, aspect: f32) -> vec2<f32> {
    let c = (uv * 2.0 - 1.0) * vec2(aspect, 1.0) / sqrt(aspect * aspect + 1.0);
    let r2 = dot(c, c);
    let f = (1.0 + k * r2 + 0.5 * k * r2 * r2) / (1.0 + 1.5 * k);
    let d = c * f * sqrt(aspect * aspect + 1.0) / vec2(aspect, 1.0);
    return d * 0.5 + 0.5;
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3(0.0)), vec3(1.0 / 2.2));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3(0.0)), vec3(2.2));
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return to_gamma(textureSampleLevel(screen_texture, texture_sampler, uv, 0.0).rgb);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(screen_texture));
    let aspect = dims.x / dims.y;
    let uv = distort_uv(in.uv, settings.fisheye_strength, aspect);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }

    // Radial chromatic aberration: red out, blue in, growing with the square of the radius.
    // Two taps per fringe smear it instead of shifting it, so fine stripes don't turn to rainbows.
    let off = uv - vec2(0.5);
    let ca = off * length(off) * settings.chromatic_aberration;
    // Aspect-correct radius, 1 at the corners (vignette, edge softening).
    let dist = length(off * vec2(aspect, 1.0)) / (0.5 * length(vec2(aspect, 1.0)));
    let g = tap(uv);
    let r = (tap(uv + ca).r + tap(uv + ca * 0.5).r) * 0.5;
    let b = (tap(uv - ca).b + tap(uv - ca * 0.5).b) * 0.5;
    var color = vec3(r, g.g, b);

    // Cheap sensor unsharp mask (edge haloing) in the middle; towards the edges, where the
    // barrel squeezes the image (and would alias), the lens goes soft instead.
    let t = 1.0 / dims;
    let blur = (tap(uv + vec2(0.0, -t.y)) + tap(uv + vec2(0.0, t.y)) + tap(uv + vec2(t.x, 0.0)) + tap(uv + vec2(-t.x, 0.0))) * 0.25;
    let soft = smoothstep(0.45, 1.0, dist);
    color = color + (g - blur) * settings.sharpness_strength * (1.0 - soft);
    color = mix(color, (color + blur) * 0.5, soft * 0.8);

    // Cheap sensor tone curve: a little extra contrast (punchy mids, crushed blacks).
    color = mix(color, smoothstep(vec3(0.0), vec3(1.0), color), 0.3);

    // High-ISO grain, strongest in the shadows, a new pattern each sensor frame (30 Hz).
    let luma = dot(color, vec3(0.299, 0.587, 0.114));
    let shadow = clamp(1.0 - luma * 1.5, 0.15, 1.0);
    let frame = floor(settings.time * 30.0);
    let n = (hash12(in.uv * dims + vec2(frame * 17.0, frame * 41.0)) - 0.5) * 2.0;
    let chroma = vec3(hash12(in.uv * dims * 1.3 + frame), 0.5, hash12(in.uv * dims * 0.7 - frame)) - 0.5;
    color += vec3(n * settings.sensor_grain_strength * shadow) + chroma * settings.sensor_grain_strength * shadow * 0.35;

    // Heavy optical vignette (dist is 1 at the corners).
    // (smoothstep with edge0 > edge1 is undefined on Metal: flip it instead.)
    let vig = 1.0 - smoothstep(0.25, 1.0, dist * settings.vignette_intensity);
    color *= mix(0.12, 1.0, vig);

    return vec4(to_linear(color), 1.0);
}
