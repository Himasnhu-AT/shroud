//! Serde helpers for per-gun arrays in saves. Guns are appended over time, so an array saved
//! with fewer entries than the current table (e.g. 4 weapon genes before the Shrike and Osprey)
//! must still load: missing entries get a fill value, extra entries are dropped.

use serde::{Deserialize, Deserializer};

fn padded<'de, D: Deserializer<'de>, const N: usize>(d: D, fill: f32) -> Result<[f32; N], D::Error> {
    let v: Vec<f32> = Vec::deserialize(d)?;
    let mut out = [fill; N];
    for (o, x) in out.iter_mut().zip(v) {
        *o = x;
    }
    Ok(out)
}

/// Missing entries are 0 (counters, shares).
pub fn zeros<'de, D: Deserializer<'de>, const N: usize>(d: D) -> Result<[f32; N], D::Error> {
    padded(d, 0.0)
}

/// Missing entries are -1, the "unset gene" marker (`genome::UNSET_LOADOUT`), seeded later.
pub fn unset<'de, D: Deserializer<'de>, const N: usize>(d: D) -> Result<[f32; N], D::Error> {
    padded(d, -1.0)
}
