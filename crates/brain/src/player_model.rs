//! The player model: a decayed profile of how the human plays (docs/PLAYER_MODEL.md).

use crate::arena::{ARENA_HALF, Arena};
use crate::math::V2;
use serde::{Deserialize, Serialize};

pub const HEAT_W: usize = 32; // 2 m cells over 64 m
pub const ROUTE_W: usize = 8; // 8 m cells
pub const RANGE_BINS: [f32; 6] = [5.0, 10.0, 15.0, 20.0, 30.0, f32::INFINITY];
pub const RANGE_LABELS: [&str; 6] = ["0-5 m", "5-10 m", "10-15 m", "15-20 m", "20-30 m", "30+ m"];
/// Compass names for 8 look sectors (x = east, z/y = south).
pub const SECTORS: [&str; 8] = ["east", "south-east", "south", "south-west", "west", "north-west", "north", "north-east"];

pub fn sector_of(angle: f32) -> usize {
    (((angle / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8)) as usize
}

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
    /// Time share per look direction (8 compass sectors).
    #[serde(default)]
    pub facing: [f32; 8],
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
            facing: [0.0; 8],
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
    /// Periodic sample of where the player is looking (ground-plane angle).
    pub fn look(&mut self, facing: f32) {
        self.facing[sector_of(facing)] += 1.0;
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
    /// Look-direction share per compass sector.
    #[serde(default)]
    pub facing: [f32; 8],
    /// EMA of shots per range bin (sample size behind `accuracy`).
    #[serde(default)]
    pub shots: [f32; 6],
}

/// Headline numbers for one match (kept in history for trends).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MatchStats {
    pub accuracy: f32,
    pub shots: f32,
    pub kills: f32,
    pub deaths: f32,
    pub reaction_ms: f32,
    pub camping: f32,
}

impl MatchStats {
    pub fn from_telemetry(t: &MatchTelemetry) -> Self {
        let shots: f32 = t.shots.iter().sum();
        let hits: f32 = t.hits.iter().sum();
        MatchStats {
            accuracy: if shots > 0.0 { hits / shots } else { 0.0 },
            shots,
            kills: t.kill_spots.iter().sum(),
            deaths: t.death_spots.iter().sum(),
            reaction_ms: if t.reaction_samples > 0.0 { 1000.0 * t.reaction_sum / t.reaction_samples } else { 0.0 },
            camping: if t.samples > 0.0 { t.still_samples / t.samples } else { 0.0 },
        }
    }
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
            facing: [0.125; 8],
            shots: [0.0; 6],
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
        let looks: f32 = t.facing.iter().sum();
        if looks > 0.0 {
            for (m, x) in self.facing.iter_mut().zip(t.facing) {
                *m += (x / looks - *m) * a;
            }
        }
        for b in 0..6 {
            self.shots[b] += (t.shots[b] - self.shots[b]) * a;
        }
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

    /// Accuracy at a range bin, smoothed toward the overall rate so a handful of shots can't dominate.
    pub fn smoothed_accuracy(&self, bin: usize) -> f32 {
        let prior = 8.0;
        (self.accuracy[bin] * self.shots[bin] + self.overall_accuracy * prior) / (self.shots[bin] + prior)
    }

    /// Specific, actionable debrief insights, most informative first.
    /// `this`: the match just played; `prev`: the match before it; `squad_reaction_ms`: next squad's average.
    pub fn insights(&self, arena: &Arena, this: Option<&MatchTelemetry>, prev: Option<&MatchStats>, squad_reaction_ms: f32) -> Vec<String> {
        if self.matches_observed == 0 {
            return vec!["They haven't seen you play yet.".into()];
        }
        let mut out: Vec<(f32, String)> = Vec::new();
        let zone_counts = |grid: &[f32]| {
            let mut z: Vec<(&'static str, f32)> = Vec::new();
            for (i, &v) in grid.iter().enumerate() {
                if v <= 0.0 {
                    continue;
                }
                if let Some(zone) = arena.zone_of(heat_center(i)) {
                    match z.iter_mut().find(|(n, _)| *n == zone.name) {
                        Some(e) => e.1 += v,
                        None => z.push((zone.name, v)),
                    }
                }
            }
            z.sort_by(|a, b| b.1.total_cmp(&a.1));
            z
        };

        if let Some(t) = this {
            let stats = MatchStats::from_telemetry(t);
            // Where you die.
            if let Some((zone, n)) = zone_counts(&t.death_spots).first()
                && stats.deaths >= 2.0
                && *n >= 2.0
            {
                out.push((0.9, format!("{n:.0} of your {:.0} deaths were in {zone}. The next squad will hold angles on it.", stats.deaths)));
            }
            // Where you win fights.
            if let Some((zone, n)) = zone_counts(&t.kill_spots).first()
                && *n >= 2.0
            {
                out.push((0.6, format!("You won most fights from {zone} ({n:.0} kills). They'll stop walking into it head-on.")));
            }
            // Trends.
            if let Some(p) = prev {
                if p.shots >= 10.0 && stats.shots >= 10.0 && (stats.accuracy - p.accuracy).abs() >= 0.05 {
                    let dir = if stats.accuracy > p.accuracy { "up" } else { "down" };
                    out.push((
                        (stats.accuracy - p.accuracy).abs() * 4.0,
                        format!("Your accuracy went {dir} from {:.0}% to {:.0}% since last match.", p.accuracy * 100.0, stats.accuracy * 100.0),
                    ));
                }
                if p.reaction_ms > 0.0 && stats.reaction_ms > 0.0 && (stats.reaction_ms - p.reaction_ms).abs() >= 40.0 {
                    let word = if stats.reaction_ms < p.reaction_ms { "faster" } else { "slower" };
                    out.push((0.5, format!("You reacted {word} this match: {:.0} ms vs {:.0} ms.", stats.reaction_ms, p.reaction_ms)));
                }
            }
        }

        // Blind side.
        let (max_s, max_v) = self.facing.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, 0.0));
        let (min_s, min_v) = self.facing.iter().copied().enumerate().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, 0.0));
        if max_v - min_v > 0.15 {
            out.push((
                (max_v - min_v) * 2.0,
                format!(
                    "You looked {} {:.0}% of the time and {} only {:.0}%. Their flankers will come from the {}.",
                    SECTORS[max_s],
                    max_v * 100.0,
                    SECTORS[min_s],
                    min_v * 100.0,
                    SECTORS[min_s]
                ),
            ));
        }

        // Range strength vs weakness, only with enough shots behind both.
        let bins: Vec<usize> = (0..6).filter(|&b| self.shots[b] >= 6.0).collect();
        if bins.len() >= 2 {
            let best = *bins.iter().max_by(|&&a, &&b| self.smoothed_accuracy(a).total_cmp(&self.smoothed_accuracy(b))).unwrap();
            let worst = *bins.iter().min_by(|&&a, &&b| self.smoothed_accuracy(a).total_cmp(&self.smoothed_accuracy(b))).unwrap();
            let (ab, aw) = (self.smoothed_accuracy(best), self.smoothed_accuracy(worst));
            if ab - aw > 0.08 {
                out.push((
                    (ab - aw) * 3.0,
                    format!(
                        "You hit {:.0}% at {} but {:.0}% at {}. They'll try to fight you at {}.",
                        ab * 100.0,
                        RANGE_LABELS[best],
                        aw * 100.0,
                        RANGE_LABELS[worst],
                        RANGE_LABELS[worst]
                    ),
                ));
            }
        }

        if self.strafe_bias.abs() > 0.25 {
            let side = if self.strafe_bias < 0.0 { "left" } else { "right" };
            let pct = 50.0 + self.strafe_bias.abs() * 50.0;
            out.push((self.strafe_bias.abs(), format!("You strafe {side} {pct:.0}% of the time in fights. They'll lead their aim that way.")));
        }
        if self.camping > 0.45 {
            out.push((
                self.camping * 0.8,
                format!("You stayed still {:.0}% of the time. Expect flanks and pre-aimed angles on your spot.", self.camping * 100.0),
            ));
        }
        if squad_reaction_ms > 0.0 && self.reaction_s > 0.0 {
            let mine = self.reaction_s * 1000.0;
            let text = if mine + 60.0 < squad_reaction_ms {
                format!("You react in ~{mine:.0} ms; this squad averages {squad_reaction_ms:.0} ms, so they'll avoid open duels and peek from cover.")
            } else {
                format!("You react in ~{mine:.0} ms; this squad averages {squad_reaction_ms:.0} ms. They'll take open duels.")
            };
            out.push((0.35, text));
        }
        if let Some((zone, share)) = zone_counts(&self.heat).first() {
            out.push((share * 0.6, format!("You spend {:.0}% of your time in {zone}. They'll set ambushes around it.", share * 100.0)));
        }
        out.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.into_iter().take(4).map(|(_, s)| s).collect()
    }
}
