//! Learned play selection: a disjoint LinUCB contextual bandit (Li et al. 2010) over the squad's
//! tactical plays. Pure Rust, tiny dense matrices (d = `CTX_DIM`).
//!
//! - Context `x`: player habits from `PlayerModel` plus live match state (`context`).
//! - One ridge-regression model per arm: `A = I + Σ x xᵀ`, `b = Σ r x`, `θ = A⁻¹ b`.
//! - Score: `θ·x + α √(xᵀ A⁻¹ x)` (optimism under uncertainty).
//! - Reward per engagement window (`squad::SquadCoordinator`): damage dealt, the player forced
//!   back, minus enemies lost without a trade.
//! - Forgetting between matches (`forget`) keeps it tracking a player who changes style.
//!
//! Persisted in `BrainState::tactics` (serde, defaults for old saves).

use crate::arena::Arena;
use crate::player_model::{Contrast, PlayerModel, heat_center};
use crate::squad::Play;
use serde::{Deserialize, Serialize};

/// Context features, in order (all roughly 0..1).
pub const CTX_DIM: usize = 10;
pub const CTX_LABELS: [&str; CTX_DIM] =
    ["bias", "rush", "zone focus", "re-peek rate", "strafe bias", "his range", "squad alive", "squad hp", "his hp", "his accuracy"];

/// Plays the bandit chooses between (when the player is known and the squad can coordinate).
/// Arms are only ever appended: a save with fewer arms loads and gets fresh ones for the rest
/// (`TacticsLearner::fix`; Rush came fifth).
pub const ARMS: [Play; 5] = [Play::Crossfire, Play::PinFlank, Play::Trade, Play::Bait, Play::Rush];

/// Exploration strength.
pub const ALPHA: f32 = 0.35;
/// Per-match forgetting of the arm models (toward the ridge prior).
pub const FORGET: f32 = 0.85;

pub fn arm_of(p: Play) -> Option<usize> {
    ARMS.iter().position(|&a| a == p)
}

/// Ridge-regression state for one arm (row-major `CTX_DIM`² matrix).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Arm {
    pub a: Vec<f32>,
    pub b: Vec<f32>,
    /// Decayed number of rewards behind this arm.
    pub n: f32,
    /// Decayed reward sum (for reports).
    pub reward_sum: f32,
}

impl Default for Arm {
    fn default() -> Self {
        let mut a = vec![0.0; CTX_DIM * CTX_DIM];
        for i in 0..CTX_DIM {
            a[i * CTX_DIM + i] = 1.0;
        }
        Arm { a, b: vec![0.0; CTX_DIM], n: 0.0, reward_sum: 0.0 }
    }
}

impl Arm {
    fn valid(&self) -> bool {
        self.a.len() == CTX_DIM * CTX_DIM && self.b.len() == CTX_DIM && self.a.iter().chain(&self.b).all(|v| v.is_finite())
    }

    /// (θ·x, √(xᵀ A⁻¹ x)).
    pub fn estimate(&self, x: &[f32; CTX_DIM]) -> (f32, f32) {
        let Some(inv) = invert(&self.a) else { return (0.0, 1.0) };
        let mut ainv_x = [0.0f64; CTX_DIM];
        for (i, v) in ainv_x.iter_mut().enumerate() {
            *v = (0..CTX_DIM).map(|j| inv[i * CTX_DIM + j] * x[j] as f64).sum();
        }
        let mean: f64 = (0..CTX_DIM).map(|i| ainv_x[i] * self.b[i] as f64).sum();
        let var: f64 = (0..CTX_DIM).map(|i| ainv_x[i] * x[i] as f64).sum();
        (mean as f32, var.max(0.0).sqrt() as f32)
    }

    pub fn update(&mut self, x: &[f32; CTX_DIM], r: f32) {
        if !self.valid() {
            *self = Arm::default();
        }
        for i in 0..CTX_DIM {
            for j in 0..CTX_DIM {
                self.a[i * CTX_DIM + j] += x[i] * x[j];
            }
            self.b[i] += r * x[i];
        }
        self.n += 1.0;
        self.reward_sum += r;
    }

    /// Shrink toward the prior (A → γA + (1-γ)I, b → γb).
    fn forget(&mut self, g: f32) {
        if !self.valid() {
            *self = Arm::default();
            return;
        }
        for i in 0..CTX_DIM {
            for j in 0..CTX_DIM {
                let id = if i == j { 1.0 } else { 0.0 };
                let v = &mut self.a[i * CTX_DIM + j];
                *v = g * *v + (1.0 - g) * id;
            }
        }
        self.b.iter_mut().for_each(|v| *v *= g);
        self.n *= g;
        self.reward_sum *= g;
    }
}

/// Gauss-Jordan inverse with partial pivoting (f64 for stability). `None` if singular.
fn invert(a: &[f32]) -> Option<Vec<f64>> {
    let n = CTX_DIM;
    let mut m: Vec<f64> = a.iter().map(|&v| v as f64).collect();
    let mut inv = vec![0.0f64; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    for c in 0..n {
        let p = (c..n).max_by(|&x, &y| m[x * n + c].abs().total_cmp(&m[y * n + c].abs()))?;
        if m[p * n + c].abs() < 1e-12 {
            return None;
        }
        if p != c {
            for k in 0..n {
                m.swap(p * n + k, c * n + k);
                inv.swap(p * n + k, c * n + k);
            }
        }
        let d = m[c * n + c];
        for k in 0..n {
            m[c * n + k] /= d;
            inv[c * n + k] /= d;
        }
        for r in 0..n {
            if r != c {
                let f = m[r * n + c];
                if f != 0.0 {
                    for k in 0..n {
                        m[r * n + k] -= f * m[c * n + k];
                        inv[r * n + k] -= f * inv[c * n + k];
                    }
                }
            }
        }
    }
    Some(inv)
}

/// Disjoint LinUCB over `ARMS`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TacticsLearner {
    pub arms: Vec<Arm>,
    /// Rewarded engagement windows in this brain's lifetime.
    pub episodes: u64,
}

impl Default for TacticsLearner {
    fn default() -> Self {
        TacticsLearner { arms: vec![Arm::default(); ARMS.len()], episodes: 0 }
    }
}

/// One arm's score breakdown (for the control port / debug overlay).
#[derive(Clone, Copy, Debug)]
pub struct ArmScore {
    pub play: Play,
    pub mean: f32,
    pub bonus: f32,
    pub prior: f32,
    pub total: f32,
    pub n: f32,
}

/// Weight of the hand-tuned rule score; decays as an arm gathers data.
pub const PRIOR_W: f32 = 1.0;
pub const PRIOR_N: f32 = 6.0;

impl TacticsLearner {
    fn fix(&mut self) {
        self.arms.resize(ARMS.len(), Arm::default());
    }

    /// Score the available arms. `prior[k]`: the hand rule's score for `ARMS[k]` (None = arm
    /// unavailable). Returns scores for available arms.
    pub fn scores(&self, x: &[f32; CTX_DIM], prior: &[Option<f32>; ARMS.len()]) -> Vec<ArmScore> {
        let mut out = Vec::new();
        for (k, p) in prior.iter().enumerate() {
            let Some(p) = *p else { continue };
            let arm = self.arms.get(k).cloned().unwrap_or_default();
            let (mean, sd) = arm.estimate(x);
            let bonus = ALPHA * sd;
            let prior = PRIOR_W * p / (1.0 + arm.n / PRIOR_N);
            out.push(ArmScore { play: ARMS[k], mean, bonus, prior, total: mean + bonus + prior, n: arm.n });
        }
        out
    }

    /// The best available arm (ties: lowest index), or None if none is available.
    pub fn choose(&self, x: &[f32; CTX_DIM], prior: &[Option<f32>; ARMS.len()]) -> Option<Play> {
        self.scores(x, prior).into_iter().fold(None, |best: Option<ArmScore>, s| match best {
            Some(b) if b.total >= s.total => Some(b),
            _ => Some(s),
        })
        .map(|s| s.play)
    }

    pub fn update(&mut self, play: Play, x: &[f32; CTX_DIM], reward: f32) {
        self.fix();
        if let Some(k) = arm_of(play) {
            self.arms[k].update(x, reward.clamp(-1.0, 1.0));
            self.episodes += 1;
        }
    }

    /// Between matches: forget a little so the squad keeps adapting.
    pub fn forget(&mut self) {
        self.fix();
        self.arms.iter_mut().for_each(|a| a.forget(FORGET));
    }
}

/// Live match state the context needs (from the coordinator).
#[derive(Clone, Copy, Debug, Default)]
pub struct LiveCtx {
    /// Share of known time the player has stood still this round (None: no data yet).
    pub live_still: Option<f32>,
    /// Re-peeks seen this round, and re-appearances after being lost.
    pub repeeks: f32,
    pub reappears: f32,
    pub alive_ratio: f32,
    pub squad_hp: f32,
    pub player_hp: f32,
}

/// How concentrated the player's time is over the map's named zones (Herfindahl index, 0..1).
pub fn zone_focus(model: &PlayerModel, arena: &Arena) -> f32 {
    if arena.zones.is_empty() {
        return 0.0;
    }
    let mut share = vec![0.0f32; arena.zones.len()];
    for (i, h) in model.heat.iter().enumerate() {
        let c = heat_center(i);
        if let Some(z) = arena.zones.iter().position(|z| c.x >= z.min.x && c.x <= z.max.x && c.y >= z.min.y && c.y <= z.max.y) {
            share[z] += *h;
        }
    }
    let s: f32 = share.iter().sum();
    if s <= 0.0 {
        return 0.0;
    }
    let hhi: f32 = share.iter().map(|v| (v / s).powi(2)).sum();
    // Rescale so a uniform spread is 0 and one zone is 1.
    let k = arena.zones.len() as f32;
    if k <= 1.0 { 1.0 } else { ((hhi - 1.0 / k) / (1.0 - 1.0 / k)).clamp(0.0, 1.0) }
}

/// Re-peek rate from the player model's follow-up duels (prior, pseudo-counts).
pub fn model_repeek(model: &PlayerModel) -> (f32, f32) {
    let (mut a, mut n) = (0.0, 0.0);
    for c in model.situations.contrasts.iter().filter(|c| c.contrast == Contrast::Repeek) {
        n += c.n;
        if c.arm_a {
            a += c.n;
        }
    }
    (a, n)
}

/// His preferred fighting range (kill-share weighted), normalised by 30 m.
fn his_range(model: &PlayerModel) -> f32 {
    let mids = [3.0, 7.5, 12.5, 17.5, 25.0, 32.0];
    let s: f32 = model.kill_share.iter().sum();
    if s <= 0.0 {
        return 0.5;
    }
    (model.kill_share.iter().zip(mids).map(|(k, m)| k * m).sum::<f32>() / s / 30.0).clamp(0.0, 1.0)
}

/// The context vector (see `CTX_LABELS`). `zone_focus` is precomputed per round.
pub fn context(model: &PlayerModel, zone_focus: f32, live: &LiveCtx) -> [f32; CTX_DIM] {
    let still = match live.live_still {
        Some(s) => 0.5 * model.camping + 0.5 * s,
        None => model.camping,
    };
    let (ra, rn) = model_repeek(model);
    // Beta(1, 3) prior: players re-peek sometimes.
    let repeek = (ra + live.repeeks + 1.0) / (rn + live.reappears + 4.0);
    [
        1.0,
        (1.0 - still).clamp(0.0, 1.0),
        zone_focus,
        repeek.clamp(0.0, 1.0),
        model.strafe_bias.abs().min(1.0),
        his_range(model),
        live.alive_ratio.clamp(0.0, 1.0),
        live.squad_hp.clamp(0.0, 1.0),
        live.player_hp.clamp(0.0, 1.0),
        (model.overall_accuracy * 2.0).clamp(0.0, 1.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};

    #[test]
    fn inverse_of_identity_plus_rank_one() {
        let mut arm = Arm::default();
        let x = [1.0, 0.5, 0.0, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.3];
        arm.update(&x, 1.0);
        let inv = invert(&arm.a).unwrap();
        // A·A⁻¹ = I
        for i in 0..CTX_DIM {
            for j in 0..CTX_DIM {
                let v: f64 = (0..CTX_DIM).map(|k| arm.a[i * CTX_DIM + k] as f64 * inv[k * CTX_DIM + j]).sum();
                assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn converges_to_best_arm_per_context() {
        // Two player types: a rusher (x[1] high) for whom PinFlank pays, a camper for whom
        // Trade pays. Noisy rewards; the learner must pick the right arm for each.
        let mut l = TacticsLearner::default();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
        let prior = [Some(0.5), Some(0.4), Some(0.3), Some(0.2), None];
        let ctx = |rush: bool| {
            let mut x = [0.0; CTX_DIM];
            x[0] = 1.0;
            x[1] = if rush { 0.9 } else { 0.1 };
            x[6] = 1.0;
            x
        };
        let truth = |play: Play, rush: bool| match (play, rush) {
            (Play::PinFlank, true) => 0.6,
            (Play::Trade, false) => 0.6,
            _ => -0.2,
        };
        for _ in 0..600 {
            let rush = rng.random::<bool>();
            let x = ctx(rush);
            let p = l.choose(&x, &prior).unwrap();
            let r = truth(p, rush) + (rng.random::<f32>() - 0.5) * 0.6;
            l.update(p, &x, r);
        }
        assert_eq!(l.choose(&ctx(true), &prior), Some(Play::PinFlank));
        assert_eq!(l.choose(&ctx(false), &prior), Some(Play::Trade));
        // And it mostly exploits by now.
        let mut right = 0;
        for k in 0..100 {
            let rush = k % 2 == 0;
            let want = if rush { Play::PinFlank } else { Play::Trade };
            right += (l.choose(&ctx(rush), &prior) == Some(want)) as u32;
        }
        assert!(right >= 95, "{right}");
    }

    #[test]
    fn untrained_follows_the_prior() {
        let l = TacticsLearner::default();
        let x = context(&PlayerModel::default(), 0.2, &LiveCtx { alive_ratio: 1.0, squad_hp: 1.0, player_hp: 1.0, ..Default::default() });
        assert_eq!(l.choose(&x, &[Some(0.2), Some(0.9), Some(0.1), None, None]), Some(Play::PinFlank));
        assert_eq!(l.choose(&x, &[None, None, None, Some(0.0), None]), Some(Play::Bait));
        assert_eq!(l.choose(&x, &[None; ARMS.len()]), None);
    }

    #[test]
    fn serde_round_trip_and_old_saves() {
        let mut l = TacticsLearner::default();
        let x = [1.0, 0.3, 0.2, 0.1, 0.0, 0.5, 1.0, 1.0, 0.8, 0.4];
        l.update(Play::Crossfire, &x, 0.7);
        l.update(Play::Bait, &x, -0.4);
        let s = serde_json::to_string(&l).unwrap();
        let back: TacticsLearner = serde_json::from_str(&s).unwrap();
        assert_eq!(back, l);
        assert_eq!(back.choose(&x, &[Some(0.0); ARMS.len()]), l.choose(&x, &[Some(0.0); ARMS.len()]));
        // Missing / partial state loads as defaults and is repaired on use.
        let empty: TacticsLearner = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, TacticsLearner::default());
        let mut broken: TacticsLearner = serde_json::from_str(r#"{"arms":[{"a":[1.0],"b":[]}]}"#).unwrap();
        broken.update(Play::Trade, &x, 1.0);
        broken.forget();
        assert_eq!(broken.arms.len(), ARMS.len());
        assert!(broken.arms.iter().all(Arm::valid));
        // A save from before the Rush arm (four arms) keeps its four and learns the fifth.
        let mut four = TacticsLearner::default();
        four.arms.truncate(4);
        four.update(Play::Trade, &x, 0.5);
        let old = serde_json::to_string(&four).unwrap();
        let mut back: TacticsLearner = serde_json::from_str(&old).unwrap();
        assert_eq!(back.choose(&x, &[None, None, None, None, Some(0.1)]), Some(Play::Rush));
        back.update(Play::Rush, &x, 0.8);
        assert_eq!(back.arms.len(), ARMS.len());
        assert_eq!(back.arms[2], four.arms[2], "the old arms keep what they learned");
        assert!(back.arms[4].n > 0.0);
    }

    #[test]
    fn forgetting_shrinks_toward_prior() {
        let mut l = TacticsLearner::default();
        let x = [1.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5];
        for _ in 0..20 {
            l.update(Play::Crossfire, &x, 1.0);
        }
        let before = l.arms[0].estimate(&x);
        l.forget();
        let after = l.arms[0].estimate(&x);
        assert!(l.arms[0].n < 20.0);
        assert!(after.1 > before.1, "uncertainty grows after forgetting");
    }
}
