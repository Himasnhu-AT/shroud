//! Decal textures, generated once at start-up (no files): bullet holes per surface and a blood
//! spatter. Each is a 128² sRGB RGBA image on a unit quad; the alpha is the coverage.
//!
//! - Concrete / plaster: a jagged dark hole inside a chipped crater of paler, freshly exposed
//!   material, hairline cracks running out of it and a faint dusty halo.
//! - Metal: a small punched hole, a bright ring of bare steel where the paint was knocked off
//!   and a few paint chips round it.
//! - Wood: a dark hole with pale splinters torn out along the grain.
//! - Blood: an irregular spatter, darker where it is thick, with droplets thrown one way.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const SIZE: u32 = 128;

/// An image from `f(u, v)` over [-1, 1]², returning linear-ish sRGB colour and alpha.
fn image(f: impl Fn(f32, f32) -> [f32; 4]) -> Image {
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = (x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
            for c in f(u, v) {
                data.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
    Image::new(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

/// Smooth value noise in [0, 1].
fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let a = hash(xi, yi, seed) + (hash(xi + 1, yi, seed) - hash(xi, yi, seed)) * sx;
    let b = hash(xi, yi + 1, seed) + (hash(xi + 1, yi + 1, seed) - hash(xi, yi + 1, seed)) * sx;
    a + (b - a) * sy
}

/// Three octaves of [`noise`], in [0, 1].
fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    (noise(x, y, seed) * 0.57 + noise(x * 2.1, y * 2.1, seed + 1) * 0.29 + noise(x * 4.3, y * 4.3, seed + 2) * 0.14).clamp(0.0, 1.0)
}

/// Noise around a circle (periodic in the angle): `a` in radians.
fn ring_noise(a: f32, freq: f32, seed: u32) -> f32 {
    fbm(a.cos() * freq + 7.0, a.sin() * freq + 7.0, seed)
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub fn concrete_hole() -> Image {
    image(|u, v| {
        let (r, a) = ((u * u + v * v).sqrt(), v.atan2(u));
        let hole = 0.1 + 0.05 * ring_noise(a, 2.5, 1);
        let crater = 0.3 + 0.22 * ring_noise(a, 1.8, 2);
        // Hairline cracks: a few thin rays that wander.
        let crack = (a * 5.0 + 1.6 * fbm(u * 3.0, v * 3.0, 3) * 6.0).sin().abs().powf(60.0) * smooth(0.95, 0.35, r) * smooth(crater * 0.8, crater, r);
        let grain = fbm(u * 9.0, v * 9.0, 4);
        if r < hole {
            let c = 0.02 + 0.03 * grain;
            return [c, c, c, 1.0];
        }
        let inner = 1.0 - smooth(crater - 0.06, crater + 0.02, r);
        // Fresh chipped concrete: paler than the wall, with shadowed pits toward the hole.
        let fresh = mix([0.38, 0.37, 0.35], [0.62, 0.6, 0.57], smooth(hole, crater, r) * (0.6 + 0.4 * grain));
        let halo = (1.0 - r).max(0.0).powf(2.0) * 0.28 * grain;
        let col = mix([0.09, 0.085, 0.08], fresh, inner);
        let alpha = (inner * 0.9).max(halo).max(crack * 0.7);
        let col = mix(col, [0.05, 0.05, 0.05], crack * (1.0 - inner));
        [col[0], col[1], col[2], alpha]
    })
}

pub fn metal_hole() -> Image {
    image(|u, v| {
        let (r, a) = ((u * u + v * v).sqrt(), v.atan2(u));
        let hole = 0.09 + 0.02 * ring_noise(a, 3.0, 11);
        let bare = 0.17 + 0.05 * ring_noise(a, 2.0, 12);
        let chips = smooth(0.62, 0.7, fbm(u * 5.0, v * 5.0, 13)) * smooth(0.6, 0.25, r);
        if r < hole {
            return [0.01, 0.01, 0.01, 1.0];
        }
        if r < bare {
            // Bare steel, polished where the bullet dragged, a dark rim at the lip.
            let t = smooth(hole, bare, r);
            let c = 0.35 + 0.35 * t * (0.7 + 0.3 * fbm(u * 12.0, v * 12.0, 14));
            return [c, c, c * 1.02, 1.0];
        }
        let scorch = (1.0 - r).max(0.0).powf(3.0) * 0.4;
        let c = mix([0.06, 0.055, 0.05], [0.5, 0.5, 0.5], chips);
        [c[0], c[1], c[2], chips.max(scorch)]
    })
}

pub fn wood_hole() -> Image {
    image(|u, v| {
        let (r, a) = ((u * u + v * v).sqrt(), v.atan2(u));
        let hole = 0.1 + 0.04 * ring_noise(a, 2.5, 21);
        // Splinters run along the grain (u): an ellipse stretched by streaky noise.
        let streak = fbm(u * 2.0, v * 26.0, 22);
        let e = ((u / 0.75).powi(2) + (v / 0.22).powi(2)).sqrt();
        let torn = smooth(1.0, 0.55, e + (0.5 - streak) * 0.5);
        if r < hole {
            return [0.03, 0.02, 0.015, 1.0];
        }
        let fibre = 0.75 + 0.25 * fbm(u * 3.0, v * 40.0, 23);
        let col = mix([0.5, 0.36, 0.22], [0.82, 0.66, 0.46], fibre * smooth(hole, 0.6, r));
        let soot = (1.0 - r).max(0.0).powf(3.0) * 0.3;
        [col[0], col[1], col[2], (torn * 0.95).max(soot)]
    })
}

pub fn blood_spatter() -> Image {
    image(|u, v| {
        let (r, a) = ((u * u + v * v).sqrt(), v.atan2(u));
        // Main splash with a lobed edge, thrown toward +u.
        let edge = 0.32 + 0.16 * ring_noise(a, 2.2, 31) + 0.12 * a.cos().max(0.0);
        let body = smooth(edge, edge - 0.05, r);
        // Droplets: a scatter of dots, denser toward +u.
        let mut drop: f32 = 0.0;
        for k in 0..18 {
            let t = hash(k, 0, 32);
            let ang = (hash(k, 1, 32) - 0.5) * 2.2;
            let d = 0.4 + 0.55 * t;
            let (cx, cy) = (ang.cos() * d, ang.sin() * d);
            let rad = 0.012 + 0.04 * hash(k, 2, 32) * (1.0 - t * 0.6);
            // Elongated along the throw.
            let (dx, dy) = (u - cx, v - cy);
            let along = dx * ang.cos() + dy * ang.sin();
            let across = -dx * ang.sin() + dy * ang.cos();
            let q = ((along / (rad * 1.8)).powi(2) + (across / rad).powi(2)).sqrt();
            drop = drop.max(smooth(1.0, 0.7, q));
        }
        let thick = fbm(u * 4.0, v * 4.0, 33);
        let cover = body.max(drop);
        // Darker and browner where it pooled, brighter red at thin edges.
        let col = mix([0.32, 0.02, 0.015], [0.14, 0.01, 0.008], (body * (0.4 + 0.6 * thick)).min(1.0));
        [col[0], col[1], col[2], cover * 0.93]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(img: &Image, x: u32, y: u32) -> u8 {
        img.data.as_ref().unwrap()[((y * SIZE + x) * 4 + 3) as usize]
    }

    #[test]
    fn holes_are_opaque_in_the_middle_and_clear_at_the_corners() {
        for img in [concrete_hole(), metal_hole(), wood_hole()] {
            assert_eq!(alpha_at(&img, SIZE / 2, SIZE / 2), 255);
            assert!(alpha_at(&img, 0, 0) < 8);
        }
        let b = blood_spatter();
        assert!(alpha_at(&b, SIZE / 2, SIZE / 2) > 200);
        assert!(alpha_at(&b, 0, 0) < 30);
    }
}
