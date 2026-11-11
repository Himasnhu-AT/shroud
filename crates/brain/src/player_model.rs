//! The player model: a decayed profile of how the human plays (docs/PLAYER_MODEL.md).

use crate::arena::{Arena, Block, BlockKind, GRID_HALF};
use crate::math::V2;
use serde::{Deserialize, Serialize};

pub const HEAT_W: usize = 48; // 2 m cells over 2 * GRID_HALF = 96 m
pub const ROUTE_W: usize = 12; // 8 m cells
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
    let c = |v: f32| (((v + GRID_HALF) / (2.0 * GRID_HALF)) * HEAT_W as f32).floor().clamp(0.0, (HEAT_W - 1) as f32) as usize;
    c(p.y) * HEAT_W + c(p.x)
}

pub fn heat_center(i: usize) -> V2 {
    let cell = 2.0 * GRID_HALF / HEAT_W as f32;
    V2::new(-GRID_HALF + ((i % HEAT_W) as f32 + 0.5) * cell, -GRID_HALF + ((i / HEAT_W) as f32 + 0.5) * cell)
}

pub fn route_idx(p: V2) -> usize {
    let c = |v: f32| (((v + GRID_HALF) / (2.0 * GRID_HALF)) * ROUTE_W as f32).floor().clamp(0.0, (ROUTE_W - 1) as f32) as usize;
    c(p.y) * ROUTE_W + c(p.x)
}

// ---------------------------------------------------------------------------
// L0: the engagement log (docs/research/adaptive-squad-ai.md, section 4.1).

/// Max duel records, trajectory samples, shot records and reaction samples kept per match.
/// Guns in `WeaponKind::ALL` (per-gun arrays; older saves with fewer load padded).
pub const WEAPON_COUNT: usize = crate::weapons::WeaponKind::ALL.len();
pub const DUEL_CAP: usize = 600;
pub const TRAJ_CAP: usize = 9000; // 15 min at 10 Hz
pub const SHOT_CAP: usize = 3000;
pub const REACTION_CAP: usize = 500;
/// Prior strength (pseudo-duels) for hierarchical shrinkage.
pub const KAPPA: f32 = 6.0;
/// Forgetting: pseudo-counts are multiplied by this after each match.
pub const FORGET: f32 = 0.7;
/// Coarse range bins used for situation keys.
pub const RANGE3_LABELS: [&str; 3] = ["under 10 m", "10 to 20 m", "over 20 m"];

pub fn range3(d: f32) -> u8 {
    if d < 10.0 {
        0
    } else if d < 20.0 {
        1
    } else {
        2
    }
}

/// Enemies in sight, clamped to 1..=3 (3 means 3 or more).
pub fn odds_bin(visible: u8) -> u8 {
    visible.clamp(1, 3)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CoverState {
    #[default]
    Exposed,
    LowCover,
    /// Within 1.5 m of a tall block (peeking from its edge).
    TallEdge,
}

/// Which side of the nearest tall block the player peeks from, as the player sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeekSide {
    #[default]
    None,
    Left,
    Right,
}

impl PeekSide {
    pub fn word(self) -> &'static str {
        match self {
            PeekSide::None => "",
            PeekSide::Left => "left",
            PeekSide::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DuelOutcome {
    Win,
    Loss,
    #[default]
    Disengage,
}

fn block_dist(b: &Block, p: V2) -> f32 {
    let dx = ((p.x - b.center.x).abs() - b.half.x).max(0.0);
    let dy = ((p.y - b.center.y).abs() - b.half.y).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// Cover state, peek side (relative to the nearest tall block within 4 m) and that block's index.
pub fn classify_cover(arena: &Arena, player: V2, enemy: V2) -> (CoverState, PeekSide, Option<u16>) {
    let nearest = |kind: BlockKind| {
        arena
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.kind == kind)
            .map(|(i, b)| (i, block_dist(b, player)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    };
    let tall = nearest(BlockKind::Tall);
    let low = nearest(BlockKind::Low);
    let cover = match (tall, low) {
        (Some((_, d)), _) if d <= 1.5 => CoverState::TallEdge,
        (_, Some((_, d))) if d <= 1.5 => CoverState::LowCover,
        _ => CoverState::Exposed,
    };
    match tall {
        Some((i, d)) if d <= 4.0 => {
            let to_block = arena.blocks[i].center - player;
            let right = V2::new(-to_block.y, to_block.x);
            let side = if (enemy - player).dot(right) > 0.0 { PeekSide::Right } else { PeekSide::Left };
            (cover, side, Some(i as u16))
        }
        _ => (cover, PeekSide::None, None),
    }
}

/// One duel: a contiguous period in which the player and at least one enemy had line of sight.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DuelRecord {
    /// Seconds since the match started, and how long the duel lasted.
    pub t: f32,
    pub duration: f32,
    /// 1-based round number.
    pub round: u32,
    /// The player had an enemy in view before any enemy saw the player.
    pub player_saw_first: bool,
    /// Zone name at duel start ("" if none).
    pub zone: String,
    pub pos: [f32; 2],
    pub cover: CoverState,
    pub peek: PeekSide,
    /// Index into `Arena::blocks` of the tall block `peek` refers to.
    pub cover_block: Option<u16>,
    /// Distance to the nearest enemy in contact at duel start (m).
    pub range: f32,
    /// Enemies with line of sight (either way) at duel start.
    pub enemies_visible: u8,
    /// Player HP fraction and magazine ammo at duel start.
    pub hp: f32,
    pub ammo: u32,
    pub moving: bool,
    pub crouched: bool,
    /// Aim-down-sights (the game has none yet; always false).
    pub ads: bool,
    /// Angle (rad) between the crosshair and the first enemy when contact began.
    pub preaim_err: f32,
    /// Seconds from duel start to the player's first shot.
    pub first_shot_s: Option<f32>,
    pub dmg_dealt: f32,
    pub dmg_taken: f32,
    pub kills: u8,
    pub outcome: DuelOutcome,
    /// Started within 5 s of the previous duel's end from the same angle (same cover side, or within 2.5 m).
    pub repeek: bool,
    /// Started within 8 s of the previous duel's end.
    pub follow_up: bool,
    /// Follow-up after a won duel.
    pub after_kill: bool,
}

impl DuelRecord {
    pub fn key(&self) -> SitKey {
        SitKey { zone: self.zone.clone(), peek: self.peek, range: range3(self.range), odds: odds_bin(self.enemies_visible) }
    }
    pub fn won(&self) -> bool {
        self.outcome == DuelOutcome::Win
    }
    /// Disengages don't count toward win rates.
    pub fn decided(&self) -> bool {
        self.outcome != DuelOutcome::Disengage
    }
}

/// 10 Hz trajectory sample.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct TrajSample {
    pub t: f32,
    pub x: f32,
    pub z: f32,
    pub vx: f32,
    pub vz: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub hp: f32,
    pub fire: bool,
    pub crouch: bool,
}

/// One player shot, for a per-shot hit model.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ShotRecord {
    pub t: f32,
    /// Distance to the targeted enemy (m).
    pub range: f32,
    /// Target's angular velocity relative to the player (rad/s, ground plane).
    pub ang_vel: f32,
    pub moving: bool,
    pub crouched: bool,
    /// Angle (rad) between the shot and the target's chest at fire time.
    pub aim_err: f32,
    pub hit: bool,
    pub headshot: bool,
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
    /// L0 engagement log: one record per duel (capped at `DUEL_CAP`).
    #[serde(default)]
    pub duels: Vec<DuelRecord>,
    /// L0 10 Hz trajectory (capped at `TRAJ_CAP`).
    #[serde(default)]
    pub trajectory: Vec<TrajSample>,
    /// L0 per-shot records (capped at `SHOT_CAP`).
    #[serde(default)]
    pub shot_log: Vec<ShotRecord>,
    /// Individual reaction times (s), for the log-normal reaction model.
    #[serde(default)]
    pub reactions: Vec<f32>,
    /// Shots fired per gun (`WeaponKind::ALL` order).
    #[serde(default, deserialize_with = "crate::pad::zeros")]
    pub weapon_shots: [f32; WEAPON_COUNT],
    /// Rounds played this match.
    #[serde(default)]
    pub rounds: u32,
    /// Seconds each archetype (`doctrine::Archetype` order) was active this match, and the
    /// seconds the squad's doctrine observed him.
    #[serde(default, deserialize_with = "crate::pad::zeros")]
    pub style_time: [f32; crate::doctrine::DOCTRINE_GENES],
    #[serde(default)]
    pub style_play_s: f32,
    /// Bombs planted per site (`Arena::sites` order: A, B).
    #[serde(default)]
    pub plant_sites: [f32; 2],
    /// Squad rushes started this match per `squad::RushReason` (`RushReason::ALL` order), and
    /// rounds the squad won while rushing.
    #[serde(default, deserialize_with = "crate::pad::zeros")]
    pub rushes: [f32; 6],
    #[serde(default)]
    pub rush_wins: f32,
    #[serde(default)]
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
            duels: Vec::new(),
            trajectory: Vec::new(),
            shot_log: Vec::new(),
            reactions: Vec::new(),
            weapon_shots: [0.0; WEAPON_COUNT],
            rounds: 0,
            style_time: [0.0; crate::doctrine::DOCTRINE_GENES],
            style_play_s: 0.0,
            plant_sites: [0.0; 2],
            rushes: [0.0; 6],
            rush_wins: 0.0,
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
    /// The player fired gun `kind` (once per trigger pull).
    pub fn shot_with(&mut self, kind: crate::weapons::WeaponKind) {
        self.weapon_shots[crate::loadout::kind_index(kind)] += 1.0;
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
    /// The player planted the bomb on site `site` (0 = A, 1 = B).
    pub fn plant(&mut self, site: usize) {
        if let Some(n) = self.plant_sites.get_mut(site) {
            *n += 1.0;
        }
    }
    pub fn death(&mut self, pos: V2) {
        self.death_spots[heat_idx(pos)] += 1.0;
    }
    pub fn reaction(&mut self, seconds: f32) {
        self.reaction_sum += seconds;
        self.reaction_samples += 1.0;
        if self.reactions.len() < REACTION_CAP {
            self.reactions.push(seconds);
        }
    }
    pub fn push_duel(&mut self, d: DuelRecord) {
        if self.duels.len() < DUEL_CAP {
            self.duels.push(d);
        }
    }
    pub fn push_traj(&mut self, s: TrajSample) {
        if self.trajectory.len() < TRAJ_CAP {
            self.trajectory.push(s);
        }
    }
    pub fn push_shot(&mut self, s: ShotRecord) {
        if self.shot_log.len() < SHOT_CAP {
            self.shot_log.push(s);
        }
    }
}

// ---------------------------------------------------------------------------
// L1: situation posteriors with hierarchical shrinkage and forgetting.

/// Situation key: zone, peek side, coarse range bin (`range3`), odds (`odds_bin`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SitKey {
    pub zone: String,
    pub peek: PeekSide,
    pub range: u8,
    pub odds: u8,
}

/// Decayed duel-win pseudo-counts for one situation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SitCell {
    pub key: SitKey,
    pub wins: f32,
    pub n: f32,
}

/// A controllable choice compared within matched strata (section 6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Contrast {
    /// arm a = re-peeked the same angle within 5 s, b = repositioned first.
    Repeek,
    /// arm a = peeked left of this tall block, b = right.
    Side(u16),
    /// arm a = crouched at duel start, b = standing.
    Crouch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContrastCell {
    pub contrast: Contrast,
    pub arm_a: bool,
    /// `stratum()`: range3 * 3 + odds - 1.
    pub stratum: u8,
    pub wins: f32,
    pub n: f32,
}

/// Log-normal reaction time: Normal-Gamma posterior on ln(seconds) from decayed sufficient stats.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct ReactionPost {
    pub n: f32,
    pub sum: f32,
    pub sum_sq: f32,
}

impl ReactionPost {
    /// Pooled prior: median 330 ms, log-sd about 0.35, worth 4 observations.
    const MU0: f32 = -1.109; // ln 0.33
    const K0: f32 = 4.0;
    const A0: f32 = 3.0;
    const B0: f32 = 3.0 * 0.35 * 0.35;

    pub fn add(&mut self, seconds: f32) {
        if seconds > 0.05 {
            let x = seconds.ln();
            self.n += 1.0;
            self.sum += x;
            self.sum_sq += x * x;
        }
    }

    /// Posterior mean of ln(rt) and the variance of that mean.
    pub fn posterior(&self) -> (f32, f32) {
        let kn = Self::K0 + self.n;
        let mu = (Self::K0 * Self::MU0 + self.sum) / kn;
        let an = Self::A0 + self.n / 2.0;
        let (ss, xbar) = if self.n > 0.0 {
            ((self.sum_sq - self.sum * self.sum / self.n).max(0.0), self.sum / self.n)
        } else {
            (0.0, Self::MU0)
        };
        let bn = Self::B0 + 0.5 * ss + Self::K0 * self.n * (xbar - Self::MU0).powi(2) / (2.0 * kn);
        (mu, bn / (an * kn))
    }

    /// Posterior median reaction time in ms, with an 80% interval.
    pub fn median_ms(&self) -> (f32, f32, f32) {
        let (mu, var) = self.posterior();
        let sd = var.sqrt();
        (1000.0 * mu.exp(), 1000.0 * (mu - 1.2816 * sd).exp(), 1000.0 * (mu + 1.2816 * sd).exp())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SituationModel {
    pub cells: Vec<SitCell>,
    pub contrasts: Vec<ContrastCell>,
    /// Decayed number of rounds behind the counts.
    pub rounds: f32,
    pub reaction: ReactionPost,
}

pub fn stratum(d: &DuelRecord) -> u8 {
    range3(d.range) * 3 + odds_bin(d.enemies_visible) - 1
}

impl SituationModel {
    /// Forget (pseudo-counts times `FORGET`), then add one match.
    pub fn ingest(&mut self, t: &MatchTelemetry) {
        for c in &mut self.cells {
            c.wins *= FORGET;
            c.n *= FORGET;
        }
        for c in &mut self.contrasts {
            c.wins *= FORGET;
            c.n *= FORGET;
        }
        self.cells.retain(|c| c.n >= 0.02);
        self.contrasts.retain(|c| c.n >= 0.02);
        self.rounds = self.rounds * FORGET + t.rounds as f32;
        let r = &mut self.reaction;
        r.n *= FORGET;
        r.sum *= FORGET;
        r.sum_sq *= FORGET;
        for &rt in &t.reactions {
            r.add(rt);
        }
        for d in t.duels.iter().filter(|d| d.decided()) {
            let w = if d.won() { 1.0 } else { 0.0 };
            let key = d.key();
            match self.cells.iter_mut().find(|c| c.key == key) {
                Some(c) => {
                    c.wins += w;
                    c.n += 1.0;
                }
                None => self.cells.push(SitCell { key, wins: w, n: 1.0 }),
            }
            let st = stratum(d);
            let mut add = |contrast: Contrast, arm_a: bool| match self
                .contrasts
                .iter_mut()
                .find(|c| c.contrast == contrast && c.arm_a == arm_a && c.stratum == st)
            {
                Some(c) => {
                    c.wins += w;
                    c.n += 1.0;
                }
                None => self.contrasts.push(ContrastCell { contrast, arm_a, stratum: st, wins: w, n: 1.0 }),
            };
            if d.follow_up {
                add(Contrast::Repeek, d.repeek);
            }
            if let (Some(b), PeekSide::Left | PeekSide::Right, CoverState::TallEdge) = (d.cover_block, d.peek, d.cover) {
                add(Contrast::Side(b), d.peek == PeekSide::Left);
            }
            add(Contrast::Crouch, d.crouched);
        }
    }

    /// (wins, n) summed over cells matching `f`.
    pub fn counts(&self, f: impl Fn(&SitKey) -> bool) -> (f32, f32) {
        self.cells.iter().filter(|c| f(&c.key)).fold((0.0, 0.0), |a, c| (a.0 + c.wins, a.1 + c.n))
    }

    /// Overall duel win rate (raw; 0.5 with no data) and its pseudo-count.
    pub fn overall(&self) -> (f32, f32) {
        let (w, n) = self.counts(|_| true);
        (if n > 0.0 { w / n } else { 0.5 }, n)
    }

    /// Beta posterior (alpha, beta) of the duel win rate in situation `key`, shrunk
    /// situation -> zone-agnostic situation -> global with strength `KAPPA` at each level.
    pub fn posterior(&self, key: &SitKey) -> (f32, f32) {
        let (wg, ng) = self.counts(|_| true);
        let p_global = (wg + 1.0) / (ng + 2.0);
        let (wp, np) = self.counts(|k| k.peek == key.peek && k.range == key.range && k.odds == key.odds);
        let p_parent = (wp + KAPPA * p_global) / (np + KAPPA);
        let (w, n) = self.counts(|k| k == key);
        (w + KAPPA * p_parent, (n - w) + KAPPA * (1.0 - p_parent))
    }

    /// Posterior mean duel win probability in a situation (for the ghost and the squad).
    pub fn win_prob(&self, key: &SitKey) -> f32 {
        let (a, b) = self.posterior(key);
        a / (a + b)
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
    /// Share of the player's hits that are headshots (EMA over matches). The trainer's ghost
    /// lands headshots at this rate, so a precise player trains squads that counter it.
    #[serde(default = "default_headshot_rate")]
    pub headshot_rate: f32,
    pub matches_observed: u32,
    /// Look-direction share per compass sector.
    #[serde(default)]
    pub facing: [f32; 8],
    /// EMA of shots per range bin (sample size behind `accuracy`).
    #[serde(default)]
    pub shots: [f32; 6],
    /// Ghost calibration: scales the trainer ghost's aim and reaction so sim outcomes
    /// match real ones (1.0 = the raw model).
    #[serde(default = "one")]
    pub ghost_skill: f32,
    /// L1 situation posteriors (duel wins per situation, contrasts, reaction time).
    #[serde(default)]
    pub situations: SituationModel,
    /// Ghost v2 behaviour parameters, calibrated to real outcomes after each match.
    #[serde(default)]
    pub ghost: crate::ghost::GhostParams,
    /// Share of shots per gun (`WeaponKind::ALL` order), EMA over matches. The trainer ghost
    /// carries the favourite.
    #[serde(default, deserialize_with = "crate::pad::zeros")]
    pub weapon_use: [f32; WEAPON_COUNT],
    /// Archetypes across matches (adaptive doctrine): the squad starts a round already
    /// countering a strong habit (`doctrine::Doctrine::with_prior`).
    #[serde(default)]
    pub style: crate::doctrine::StyleSummary,
    /// Bomb plants per site (index into `Arena::sites`: A, B), decayed: the squad stacks the
    /// site he favours (`record_plant`, `plant_share`).
    #[serde(default)]
    pub plants: [f32; 2],
}

fn one() -> f32 {
    1.0
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
            headshot_rate: crate::sim::GHOST_HEADSHOT,
            matches_observed: 0,
            facing: [0.125; 8],
            shots: [0.0; 6],
            ghost_skill: 1.0,
            situations: SituationModel::default(),
            ghost: crate::ghost::GhostParams::default(),
            weapon_use: [0.0; WEAPON_COUNT],
            style: crate::doctrine::StyleSummary::default(),
            plants: [0.0; 2],
        }
    }
}

fn default_headshot_rate() -> f32 {
    crate::sim::GHOST_HEADSHOT
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
    /// Resets any world grid whose size doesn't match the current `HEAT_W` / `ROUTE_W` (a
    /// save from before the grids grew to cover big maps). Everything else is kept.
    pub fn fit_grids(&mut self) {
        let fresh = PlayerModel::default();
        if self.heat.len() != fresh.heat.len() {
            self.heat = fresh.heat;
        }
        if self.routes.len() != fresh.routes.len() {
            self.routes = fresh.routes;
        }
        if self.death_spots.len() != fresh.death_spots.len() {
            self.death_spots = fresh.death_spots;
        }
        if self.kill_spots.len() != fresh.kill_spots.len() {
            self.kill_spots = fresh.kill_spots;
        }
    }

    /// How much enemies should trust this model (0 … 1).
    pub fn confidence(&self) -> f32 {
        1.0 - 0.6f32.powi(self.matches_observed as i32)
    }

    /// Blend one match into the model (EMA).
    pub fn ingest(&mut self, t: &MatchTelemetry) {
        // Plants count even in a short match (a plant is never noise). Only finished matches
        // get here, so an abandoned one teaches the squad nothing.
        self.ingest_plants(&t.plant_sites);
        if t.samples < 10.0 {
            return;
        }
        self.situations.ingest(t);
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
            if hits >= 5.0 {
                self.headshot_rate += ((t.headshots / hits).clamp(0.0, 1.0) - self.headshot_rate) * a;
            }
        }
        let gun_shots: f32 = t.weapon_shots.iter().sum();
        if gun_shots >= 5.0 {
            for (u, s) in self.weapon_use.iter_mut().zip(t.weapon_shots) {
                *u += (s / gun_shots - *u) * a;
            }
        }
        self.style.ingest(t, a);
        self.matches_observed += 1;
    }

    /// The gun the player fires most (the rifle until there is data).
    pub fn favourite_weapon(&self) -> crate::weapons::WeaponKind {
        let total: f32 = self.weapon_use.iter().sum();
        if total < 0.2 {
            return crate::weapons::WeaponKind::Rifle;
        }
        let k = (0..WEAPON_COUNT).max_by(|&a, &b| self.weapon_use[a].total_cmp(&self.weapon_use[b]).then(b.cmp(&a))).unwrap_or(2);
        crate::weapons::WeaponKind::ALL[k]
    }

    /// Typical engagement distance (m): the shot-weighted mean over range bins (15 m with no data).
    pub fn engagement_range(&self) -> f32 {
        let mids = [3.0, 7.5, 12.5, 17.5, 25.0, 35.0];
        let n: f32 = self.shots.iter().sum();
        if n < 1.0 {
            return 15.0;
        }
        self.shots.iter().zip(mids).map(|(s, m)| s * m).sum::<f32>() / n
    }


    pub fn accuracy_at(&self, dist: f32) -> f32 {
        self.accuracy[range_bin(dist)]
    }

    /// A match's plants per site ([`MatchTelemetry::plant_sites`]), as if recorded one by one
    /// with [`Self::record_plant`]: everything before fades x0.85 per plant, then the match's
    /// plants count in full (their order within the match is not kept).
    pub fn ingest_plants(&mut self, counts: &[f32; 2]) {
        let n: f32 = counts.iter().map(|c| c.max(0.0)).sum();
        if n <= 0.0 {
            return;
        }
        let fade = 0.85f32.powf(n);
        for (p, c) in self.plants.iter_mut().zip(counts) {
            *p = *p * fade + c.max(0.0);
        }
    }

    /// He planted the bomb on site `site` (0 = A, 1 = B). Older plants fade (x0.85 each).
    pub fn record_plant(&mut self, site: usize) {
        for p in &mut self.plants {
            *p *= 0.85;
        }
        if let Some(p) = self.plants.get_mut(site) {
            *p += 1.0;
        }
    }

    /// Share of his plants on `site`, shrunk toward even (one pseudo-plant per site).
    pub fn plant_share(&self, site: usize) -> f32 {
        let total: f32 = self.plants.iter().sum();
        (self.plants.get(site).copied().unwrap_or(0.0) + 1.0) / (total + self.plants.len() as f32)
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

    /// Debrief insights (at most 4): credible v2 insights first (no trends without history),
    /// then descriptive habits. `this`: the match just played; `prev`: the match before it.
    pub fn insights(&self, arena: &Arena, this: Option<&MatchTelemetry>, prev: Option<&MatchStats>, squad_reaction_ms: f32) -> Vec<String> {
        self.insights_with_history(arena, this, prev, &[], squad_reaction_ms)
    }

    /// As `insights`, plus significance-gated trends over per-match `history` (oldest first).
    pub fn insights_with_history(
        &self,
        arena: &Arena,
        this: Option<&MatchTelemetry>,
        prev: Option<&MatchStats>,
        history: &[crate::insights::MatchPosterior],
        squad_reaction_ms: f32,
    ) -> Vec<String> {
        if self.matches_observed == 0 {
            return vec!["They haven't seen you play yet.".into()];
        }
        let mut out: Vec<String> = crate::insights::insights_v2(self, arena, history).into_iter().take(4).map(|i| i.text).collect();
        for s in self.legacy_insights(arena, this, if history.is_empty() { prev } else { None }, squad_reaction_ms) {
            if out.len() >= 4 {
                break;
            }
            out.push(s);
        }
        out
    }

    /// v1 descriptive insights (where you are, look, strafe), most informative first.
    fn legacy_insights(&self, arena: &Arena, this: Option<&MatchTelemetry>, prev: Option<&MatchStats>, squad_reaction_ms: f32) -> Vec<String> {
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
