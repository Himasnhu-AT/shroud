//! Tiny 2D vector + RNG helpers. The brain works on the ground plane (x, z);
//! the game maps `V2 { x, y }` to world `(x, _, y)`.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Mul, Neg, Sub};

pub type BrainRng = ChaCha8Rng;

pub fn rng_from_seed(seed: u64) -> BrainRng {
    ChaCha8Rng::seed_from_u64(seed)
}

/// Standard normal sample (Box-Muller).
pub fn gauss(rng: &mut impl Rng) -> f32 {
    let u1: f32 = rng.random::<f32>().max(1e-7);
    let u2: f32 = rng.random();
    (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct V2 {
    pub x: f32,
    pub y: f32,
}

impl V2 {
    pub const ZERO: V2 = V2 { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self {
        V2 { x, y }
    }
    pub fn len(self) -> f32 {
        self.x.hypot(self.y)
    }
    pub fn len_sq(self) -> f32 {
        self.x * self.x + self.y * self.y
    }
    pub fn dist(self, o: V2) -> f32 {
        (self - o).len()
    }
    pub fn norm(self) -> V2 {
        let l = self.len();
        if l > 1e-6 { self * (1.0 / l) } else { V2::ZERO }
    }
    pub fn dot(self, o: V2) -> f32 {
        self.x * o.x + self.y * o.y
    }
    /// 2D cross product (z of the 3D cross).
    pub fn cross(self, o: V2) -> f32 {
        self.x * o.y - self.y * o.x
    }
    /// Rotate 90° counter-clockwise.
    pub fn perp(self) -> V2 {
        V2::new(-self.y, self.x)
    }
    pub fn clamp_len(self, max: f32) -> V2 {
        let l = self.len();
        if l > max { self * (max / l) } else { self }
    }
    pub fn lerp(self, o: V2, t: f32) -> V2 {
        self + (o - self) * t
    }
    pub fn angle(self) -> f32 {
        self.y.atan2(self.x)
    }
    pub fn from_angle(a: f32) -> V2 {
        V2::new(a.cos(), a.sin())
    }
}

impl Add for V2 {
    type Output = V2;
    fn add(self, o: V2) -> V2 {
        V2::new(self.x + o.x, self.y + o.y)
    }
}
impl AddAssign for V2 {
    fn add_assign(&mut self, o: V2) {
        self.x += o.x;
        self.y += o.y;
    }
}
impl Sub for V2 {
    type Output = V2;
    fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for V2 {
    type Output = V2;
    fn mul(self, s: f32) -> V2 {
        V2::new(self.x * s, self.y * s)
    }
}
impl Neg for V2 {
    type Output = V2;
    fn neg(self) -> V2 {
        V2::new(-self.x, -self.y)
    }
}

/// Smallest signed difference between two angles, in (-PI, PI].
pub fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = (a - b) % std::f32::consts::TAU;
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    } else if d <= -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}
