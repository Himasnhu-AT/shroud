//! The player model: a decayed profile of how the human plays (docs/PLAYER_MODEL.md).

use crate::arena::{ARENA_HALF, Arena};
use crate::math::V2;
use serde::{Deserialize, Serialize};

pub const HEAT_W: usize = 32; // 2 m cells over 64 m
pub const ROUTE_W: usize = 8; // 8 m cells
pub const RANGE_BINS: [f32; 6] = [5.0, 10.0, 15.0, 20.0, 30.0, f32::INFINITY];
pub const RANGE_LABELS: [&str; 6] = ["0–5 m", "5–10 m", "10–15 m", "15–20 m", "20–30 m", "30+ m"];

pub fn range_bin(d: f32) -> usize {
    RANGE_BINS.iter().position(|&b| d < b).unwrap_or(5)
}

pub fn heat_idx(p: V2) -> usize {
    let c = |v: f32| (((v + ARENA_HALF) / (2.0 * ARENA_HALF)) * HEAT_W as f32).floor().clamp(0.0, (HEAT_W - 1) as f32) as usize;
    c(p.y) * HEAT_W + c(p.x)
}

pub fn heat_center(i: usize) -> V2 {
    let cell = 2.0 * ARENA_HALF / HEAT_W as f32;
    V2::new(-ARENA_HALF + ((i % HEAT_W) as f32 + 0.5) * cell, -ARENA_HALF + ((i / HEAT_W) as f32 + 0.5) * cell)
}

pub fn route_idx(p: V2) -> usize {
    let c = |v: f32| (((v + ARENA_HALF) / (2.0 * ARENA_HALF)) * ROUTE_W as f32).floor().clamp(0.0, (ROUTE_W - 1) as f32) as usize;
    c(p.y) * ROUTE_W + c(p.x)
}

/// Raw counts for one match, filled by the game (or the sim) at ~10 Hz.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchTelemetry {
    pub heat: Vec<f32>,
    pub routes: Vec<f32>,
    pub death_spots: Vec<f32>,
    pub kill_spots: Vec<f32>,
    pub shots: [f32; 6],
    pub hits: [f32; 6],
    pub kills: [f32; 6],
    pub strafe_sum: f32,
    pub strafe_samples: f32,
    pub reaction_sum: f32,
    pub reaction_samples: f32,
    pub still_samples: f32,
    pub crouch_engaged: f32,
    pub engaged_samples: f32,
    pub samples: f32,
    pub headshots: f32,
    last_route: Option<usize>,
}

impl Default for MatchTelemetry {
    fn default() -> Self {
        MatchTelemetry {
            heat: vec![0.0; HEAT_W * HEAT_W],
            routes: vec![0.0; ROUTE_W.pow(4)],
            death_spots: vec![0.0; HEAT_W * HEAT_W],
            kill_spots: vec![0.0; HEAT_W * HEAT_W],
            shots: [0.0; 6],
            hits: [0.0; 6],
            kills: [0.0; 6],
            strafe_sum: 0.0,
            strafe_samples: 0.0,
            reaction_sum: 0.0,
            reaction_samples: 0.0,
            still_samples: 0.0,
            crouch_engaged: 0.0,
            engaged_samples: 0.0,
            samples: 0.0,
            headshots: 0.0,
            last_route: None,
        }
    }
}

impl MatchTelemetry {
    /// Periodic sample. `lateral` is the player's sideways speed sign while engaged (−1 left … +1 right).
    pub fn sample(&mut self, pos: V2, speed: f32, crouched: bool, engaged: bool, lateral: f32) {
        self.samples += 1.0;
        self.heat[heat_idx(pos)] += 1.0;
        let r = route_idx(pos);
        if let Some(prev) = self.last_route
            && prev != r
        {
            self.routes[prev * ROUTE_W * ROUTE_W + r] += 1.0;
        }
        self.last_route = Some(r);
        if speed < 0.5 {
            self.still_samples += 1.0;
        }
        if engaged {
            self.engaged_samples += 1.0;
            if crouched {
                self.crouch_engaged += 1.0;
            }
            if lateral.abs() > 0.2 {
                self.strafe_sum += lateral.signum();
                self.strafe_samples += 1.0;
            }
        }
    }
    pub fn shot(&mut self, dist: f32, hit: bool, headshot: bool) {
        let b = range_bin(dist);
        self.shots[b] += 1.0;
        if hit {
            self.hits[b] += 1.0;
        }
        if headshot {
            self.headshots += 1.0;
        }
    }
    pub fn kill(&mut self, player_pos: V2, dist: f32) {
        self.kills[range_bin(dist)] += 1.0;
        self.kill_spots[heat_idx(player_pos)] += 1.0;
    }
    pub fn death(&mut self, pos: V2) {
        self.death_spots[heat_idx(pos)] += 1.0;
    }
    pub fn reaction(&mut self, seconds: f32) {
        self.reaction_sum += seconds;
        self.reaction_samples += 1.0;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerModel {
    /// Normalized time share per heat cell (sums to ~1).
    pub heat: Vec<f32>,
    /// Row-normalized transition probabilities between route cells.
    pub routes: Vec<f32>,
    pub death_spots: Vec<f32>,
    pub kill_spots: Vec<f32>,
    pub accuracy: [f32; 6],
    pub kill_share: [f32; 6],
    pub strafe_bias: f32,
    pub reaction_s: f32,
    pub camping: f32,
    pub crouch_rate: f32,
    pub overall_accuracy: f32,
    pub matches_observed: u32,
}

impl Default for PlayerModel {
    /// A plausible "average player" used before any real match.
    fn default() -> Self {
        let n = HEAT_W * HEAT_W;
        PlayerModel {
            heat: vec![1.0 / n as f32; n],
            routes: vec![0.0; ROUTE_W.pow(4)],
            death_spots: vec![0.0; n],
            kill_spots: vec![0.0; n],
            accuracy: [0.45, 0.35, 0.28, 0.22, 0.15, 0.08],
            kill_share: [0.15, 0.25, 0.25, 0.2, 0.1, 0.05],
            strafe_bias: 0.0,
            reaction_s: 0.35,
            camping: 0.3,
            crouch_rate: 0.2,
            overall_accuracy: 0.28,
            matches_observed: 0,
        }
    }
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let s: f32 = v.iter().sum();
    if s > 0.0 { v.iter().map(|x| x / s).collect() } else { v.to_vec() }
}

fn blend(dst: &mut [f32], src: &[f32], a: f32) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d += (*s - *d) * a;
    }
}

impl PlayerModel {
    /// How much enemies should trust this model (0 … 1).
    pub fn confidence(&self) -> f32 {
        1.0 - 0.6f32.powi(self.matches_observed as i32)
    }

    /// Blend one match into the model (EMA).
    pub fn ingest(&mut self, t: &MatchTelemetry) {
        if t.samples < 10.0 {
            return;
        }
        let a = if self.matches_observed < 3 { 0.4 } else { 0.3 };
        // First real match replaces the generic prior more strongly.
        let a = if self.matches_observed == 0 { 0.7 } else { a };
        blend(&mut self.heat, &normalize(&t.heat), a);
        // Row-normalize routes.
        let w = ROUTE_W * ROUTE_W;
        let mut rows = t.routes.clone();
        for r in 0..w {
            let row = &mut rows[r * w..(r + 1) * w];
            let s: f32 = row.iter().sum();
            if s > 0.0 {
                row.iter_mut().for_each(|x| *x /= s);
                blend(&mut self.routes[r * w..(r + 1) * w], row, a);
            }
        }
        blend(&mut self.death_spots, &normalize(&t.death_spots), a);
        blend(&mut self.kill_spots, &normalize(&t.kill_spots), a);
        let total_kills: f32 = t.kills.iter().sum();
        for b in 0..6 {
            if t.shots[b] >= 5.0 {
                self.accuracy[b] += (t.hits[b] / t.shots[b] - self.accuracy[b]) * a;
            }
            if total_kills > 0.0 {
                self.kill_share[b] += (t.kills[b] / total_kills - self.kill_share[b]) * a;
            }
        }
        if t.strafe_samples >= 5.0 {
            self.strafe_bias += (t.strafe_sum / t.strafe_samples - self.strafe_bias) * a;
        }
        if t.reaction_samples >= 1.0 {
            self.reaction_s += (t.reaction_sum / t.reaction_samples - self.reaction_s) * a;
        }
        self.camping += (t.still_samples / t.samples - self.camping) * a;
        if t.engaged_samples >= 5.0 {
            self.crouch_rate += (t.crouch_engaged / t.engaged_samples - self.crouch_rate) * a;
        }
        let shots: f32 = t.shots.iter().sum();
        if shots >= 5.0 {
            let hits: f32 = t.hits.iter().sum();
            self.overall_accuracy += (hits / shots - self.overall_accuracy) * a;
        }
        self.matches_observed += 1;
    }

    pub fn accuracy_at(&self, dist: f32) -> f32 {
        self.accuracy[range_bin(dist)]
    }

    /// Range bin where the player is weakest (lowest accuracy), ignoring the extremes.
    pub fn weakest_range(&self) -> f32 {
        let mids = [3.0, 7.5, 12.5, 17.5, 25.0, 32.0];
        let b = (0..5).min_by(|&a, &b| self.accuracy[a].total_cmp(&self.accuracy[b])).unwrap_or(3);
        mids[b]
    }

    /// Most likely place the player will be, given their last known position.
    /// Combines time-share heat with route transitions out of the current route cell.
    pub fn predict(&self, last_known: V2, arena: &Arena) -> V2 {
        let w = ROUTE_W * ROUTE_W;
        let from = route_idx(last_known);
        let row = &self.routes[from * w..(from + 1) * w];
        let mut best = (f32::MIN, last_known);
        for (i, &h) in self.heat.iter().enumerate() {
            let c = heat_center(i);
            if !arena.nav.walkable_at(c) {
                continue;
            }
            let route_p = row[route_idx(c)];
            let near = 1.0 / (1.0 + c.dist(last_known) / 12.0);
            let score = h * (0.3 + route_p * 2.0) * near;
            if score > best.0 {
                best = (score, c);
            }
        }
        best.1
    }

    /// Plain-language insights for the debrief, most notable first.
    pub fn insights(&self, arena: &Arena) -> Vec<String> {
        let mut out: Vec<(f32, String)> = Vec::new();
        if self.matches_observed == 0 {
            return vec!["They haven't seen you play yet.".into()];
        }
        // Favourite zone.
        let mut zone_share: Vec<(&str, f32)> = Vec::new();
        for (i, &h) in self.heat.iter().enumerate() {
            if let Some(z) = arena.zone_of(heat_center(i)) {
                match zone_share.iter_mut().find(|(n, _)| *n == z.name) {
                    Some(e) => e.1 += h,
                    None => zone_share.push((z.name, h)),
                }
            }
        }
        if let Some((z, s)) = zone_share.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
            out.push((*s * 2.0, format!("They noticed you hold {z} {:.0}% of the time.", s * 100.0)));
        }
        let best = (0..6).max_by(|&a, &b| self.accuracy[a].total_cmp(&self.accuracy[b])).unwrap_or(0);
        out.push((
            self.accuracy[best],
            format!(
                "You're deadliest at {} ({:.0}% accuracy). They'll avoid that range.",
                RANGE_LABELS[best],
                self.accuracy[best] * 100.0
            ),
        ));
        if self.strafe_bias.abs() > 0.2 {
            let side = if self.strafe_bias < 0.0 { "left" } else { "right" };
            let pct = 50.0 + self.strafe_bias.abs() * 50.0;
            out.push((self.strafe_bias.abs(), format!("You strafe {side} {pct:.0}% of the time when shooting.")));
        }
        out.push((
            (0.4 - self.reaction_s).abs(),
            format!("You react in ~{:.0} ms. Expect them to peek and pull back faster.", self.reaction_s * 1000.0),
        ));
        if self.camping > 0.4 {
            out.push((self.camping, format!("You camp a lot ({:.0}% of the time stationary). Expect more flanks.", self.camping * 100.0)));
        }
        out.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.into_iter().take(3).map(|(_, s)| s).collect()
    }
}
