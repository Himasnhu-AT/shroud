//! Insights v2 (docs/research/adaptive-squad-ai.md, section 6): credible situation
//! weaknesses, matched contrasts, gated trends and the "what they learned" card.
//! All text is ASCII (the UI font has no dashes or arrows).

use crate::arena::Arena;
use crate::evolve::EnemyReport;
use crate::player_model::{Contrast, KAPPA, MatchTelemetry, PeekSide, PlayerModel, SitKey, heat_center, odds_bin, range3};
use crate::utility::Goal;
use serde::{Deserialize, Serialize};

/// Report a weakness only if P(win rate < overall) is at least this.
pub const CREDIBLE: f32 = 0.9;
/// ... and only with at least this many (decayed) duels behind it.
pub const MIN_DUELS: f32 = 6.0;
/// Contrasts compare two arms, so they get a stricter two-sided gate.
const CONTRAST_CREDIBLE: f32 = 0.95;

// ---------------------------------------------------------------------------
// Small numerics (no deps).

/// Beta(a, b) CDF at x, by midpoint integration on a 1000-point grid.
pub fn beta_cdf(a: f32, b: f32, x: f32) -> f32 {
    let (cdf, grid) = beta_grid(a, b);
    let i = ((x.clamp(0.0, 1.0) * grid as f32) as usize).min(grid);
    if i == 0 { 0.0 } else { cdf[i - 1] }
}

/// Beta(a, b) quantile.
pub fn beta_quantile(a: f32, b: f32, q: f32) -> f32 {
    let (cdf, grid) = beta_grid(a, b);
    let i = cdf.iter().position(|&c| c >= q).unwrap_or(grid - 1);
    (i as f32 + 0.5) / grid as f32
}

fn beta_grid(a: f32, b: f32) -> (Vec<f32>, usize) {
    const N: usize = 1000;
    let (a, b) = (a.max(1e-3) as f64, b.max(1e-3) as f64);
    let logs: Vec<f64> = (0..N)
        .map(|i| {
            let p = (i as f64 + 0.5) / N as f64;
            (a - 1.0) * p.ln() + (b - 1.0) * (1.0 - p).ln()
        })
        .collect();
    let m = logs.iter().copied().fold(f64::MIN, f64::max);
    let dens: Vec<f64> = logs.iter().map(|l| (l - m).exp()).collect();
    let total: f64 = dens.iter().sum();
    let mut acc = 0.0;
    (
        dens.iter()
            .map(|d| {
                acc += d / total;
                acc as f32
            })
            .collect(),
        N,
    )
}

/// Standard normal CDF (Abramowitz-Stegun 7.1.26 erf).
pub fn normal_cdf(z: f32) -> f32 {
    let x = (z as f64).abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * (-x * x).exp();
    let p = 0.5 * (1.0 + y);
    (if z >= 0.0 { p } else { 1.0 - p }) as f32
}

fn pct(p: f32) -> String {
    format!("{:.0}%", (p * 100.0).clamp(0.0, 100.0))
}

// ---------------------------------------------------------------------------
// Situation groups.

/// A set of situations: any dimension left `None` is pooled over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SitGroup {
    pub zone: Option<String>,
    pub peek: Option<PeekSide>,
    pub range: Option<u8>,
    pub odds: Option<u8>,
}

impl SitGroup {
    pub fn matches(&self, k: &SitKey) -> bool {
        self.zone.as_ref().is_none_or(|z| *z == k.zone)
            && self.peek.is_none_or(|p| p == k.peek)
            && self.range.is_none_or(|r| r == k.range)
            && self.odds.is_none_or(|o| o == k.odds)
    }

    fn of(k: &SitKey, mask: [bool; 4]) -> Self {
        SitGroup {
            zone: mask[0].then(|| k.zone.clone()),
            peek: mask[1].then_some(k.peek),
            range: mask[2].then_some(k.range),
            odds: mask[3].then_some(k.odds),
        }
    }

    /// Plain-words description, e.g. "peeking left around cover in the center tower, at close range".
    pub fn phrase(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let zone = self.zone.as_deref().filter(|z| !z.is_empty());
        match (self.peek, zone) {
            (Some(PeekSide::None), Some(z)) => parts.push(format!("away from tall cover in {z}")),
            (Some(p), Some(z)) => parts.push(format!("peeking {} around cover in {z}", p.word())),
            (Some(PeekSide::None), None) => parts.push("away from tall cover".into()),
            (Some(p), None) => parts.push(format!("peeking {} around cover", p.word())),
            (None, Some(z)) => parts.push(format!("in {z}")),
            (None, None) => {}
        }
        if let Some(r) = self.range {
            parts.push(["at close range (under 10 m)", "at mid range (10 to 20 m)", "at long range (over 20 m)"][r.min(2) as usize].into());
        }
        if let Some(o) = self.odds {
            parts.push(["one on one", "against 2 enemies at once", "against 3 or more at once"][(o.clamp(1, 3) - 1) as usize].into());
        }
        parts.join(", ")
    }
}

/// Dimension masks (zone, peek, range, odds) considered for situation insights. Kept small:
/// every extra candidate is another chance of a false pattern.
const MASKS: [[bool; 4]; 5] = [
    [true, true, false, false],
    [true, false, false, false],
    [false, true, false, false],
    [false, false, true, false],
    [false, false, false, true],
];

impl SitGroup {
    /// Every situation in `self` is also in `other`, and `self` is narrower.
    fn strictly_within(&self, other: &SitGroup) -> bool {
        fn dim<T: PartialEq>(mine: &Option<T>, theirs: &Option<T>) -> bool {
            theirs.as_ref().is_none_or(|t| mine.as_ref() == Some(t))
        }
        self != other && dim(&self.zone, &other.zone) && dim(&self.peek, &other.peek) && dim(&self.range, &other.range) && dim(&self.odds, &other.odds)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InsightKind {
    Situation(SitGroup),
    Contrast(Contrast),
    Trend,
    Fact,
}

#[derive(Clone, Debug)]
pub struct Insight {
    pub kind: InsightKind,
    pub text: String,
    /// Expected duels (about rounds) lost per round to this pattern; the ranking key.
    pub stake: f32,
    /// Posterior probability behind the claim.
    pub prob: f32,
}

struct Cand {
    g: SitGroup,
    w: f32,
    n: f32,
    a: f32,
    b: f32,
    prob: f32,
    stake: f32,
}

/// Prior strength for the reporting gate: `KAPPA`, raised by an empirical-Bayes estimate
/// when the (zone, peek) cells vary no more than binomial noise would. This is the
/// multiple-comparison guard: with dozens of candidate situations, a fixed weak prior
/// would report a chance pattern most matches.
pub fn gate_kappa(m: &PlayerModel) -> f32 {
    const MAX: f32 = 60.0;
    let mut keys: Vec<(&str, PeekSide)> = Vec::new();
    for c in &m.situations.cells {
        if !keys.contains(&(c.key.zone.as_str(), c.key.peek)) {
            keys.push((c.key.zone.as_str(), c.key.peek));
        }
    }
    let cells: Vec<(f32, f32)> = keys
        .iter()
        .map(|&(z, p)| m.situations.counts(|k| k.zone == z && k.peek == p))
        .filter(|c| c.1 >= 3.0)
        .collect();
    let n_all: f32 = cells.iter().map(|c| c.1).sum();
    if cells.len() < 2 || n_all <= 0.0 {
        return MAX;
    }
    let p = cells.iter().map(|c| c.0).sum::<f32>() / n_all;
    let pq = (p * (1.0 - p)).max(1e-3);
    let observed = cells.iter().map(|(w, n)| n * (w / n - p).powi(2)).sum::<f32>() / n_all;
    let noise = pq * cells.len() as f32 / n_all;
    let tau2 = observed - noise;
    if tau2 <= pq / (MAX + 1.0) { MAX } else { (pq / tau2 - 1.0).clamp(KAPPA, MAX) }
}

/// Posterior for group `g` with the duels of `excluded` groups removed.
fn evaluate(m: &PlayerModel, g: &SitGroup, excluded: &[&SitGroup], p0: f32, p_global: f32, kappa: f32) -> Option<Cand> {
    let s = &m.situations;
    let (w, n) = s.counts(|k| g.matches(k) && !excluded.iter().any(|a| a.matches(k)));
    if n < MIN_DUELS {
        return None;
    }
    let p_parent = if g.zone.is_some() {
        let parent = SitGroup { zone: None, ..g.clone() };
        let (wp, np) = s.counts(|k| parent.matches(k) && !excluded.iter().any(|a| a.matches(k)));
        (wp + kappa * p_global) / (np + kappa)
    } else {
        p_global
    };
    let (a, b) = (w + kappa * p_parent, (n - w) + kappa * (1.0 - p_parent));
    let prob = beta_cdf(a, b, p0);
    let stake = n / s.rounds.max(1.0) * (p0 - a / (a + b));
    Some(Cand { g: g.clone(), w, n, a, b, prob, stake })
}

/// Credible situation weaknesses, most rounds at stake first. Greedy with explaining-away:
/// a broad group ("the center tower") is replaced by a narrower credible one ("peeking left
/// at the center tower") when the rest of the broad group isn't credible on its own, and each
/// accepted group's duels are removed before testing the others.
pub fn situation_insights(m: &PlayerModel, max: usize) -> Vec<Insight> {
    let s = &m.situations;
    let (p0, n_all) = s.overall();
    if n_all < MIN_DUELS {
        return vec![];
    }
    let p_global = (p0 * n_all + 1.0) / (n_all + 2.0);
    let kappa = gate_kappa(m);
    let mut groups: Vec<SitGroup> = Vec::new();
    for c in &s.cells {
        for mask in MASKS {
            let g = SitGroup::of(&c.key, mask);
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
    }
    let mut accepted: Vec<SitGroup> = Vec::new();
    let mut out = Vec::new();
    while out.len() < max {
        let ex: Vec<&SitGroup> = accepted.iter().collect();
        let credible: Vec<Cand> = groups
            .iter()
            .filter(|g| !accepted.iter().any(|a| g.strictly_within(a) || *g == a))
            .filter_map(|g| evaluate(m, g, &ex, p0, p_global, kappa))
            .filter(|c| c.prob >= CREDIBLE && c.stake > 0.0)
            .collect();
        let Some(mut best) = credible.iter().max_by(|a, b| a.stake.total_cmp(&b.stake)) else { break };
        loop {
            let finer = credible
                .iter()
                .filter(|f| f.g.strictly_within(&best.g))
                .filter(|f| {
                    let mut ex2 = ex.clone();
                    ex2.push(&f.g);
                    evaluate(m, &best.g, &ex2, p0, p_global, kappa).is_none_or(|rest| rest.prob < CREDIBLE)
                })
                .max_by(|a, b| a.stake.total_cmp(&b.stake));
            match finer {
                Some(f) => best = f,
                None => break,
            }
        }
        let Cand { g, w, n, a, b, prob, stake } = best;
        let mean = a / (a + b);
        let (lo, hi) = (1.0 - beta_quantile(*a, *b, 0.9), 1.0 - beta_quantile(*a, *b, 0.1));
        let (wo, no) = (p0 * n_all - w, n_all - n);
        let elsewhere = if no >= 3.0 { format!(" Elsewhere you lose {}.", pct(1.0 - wo / no)) } else { String::new() };
        let text = format!(
            "You lose {} of duels {} (about {:.0} of {:.0}; likely between {} and {}).{elsewhere}",
            pct(1.0 - mean),
            g.phrase(),
            n - w,
            n,
            pct(lo),
            pct(hi)
        );
        out.push(Insight { kind: InsightKind::Situation(g.clone()), text, stake: *stake, prob: *prob });
        accepted.push(g.clone());
    }
    out
}

/// Matched contrasts (section 6.2): arms compared within range x odds strata.
pub fn contrast_insights(m: &PlayerModel, arena: &Arena) -> Vec<Insight> {
    let s = &m.situations;
    let rounds = s.rounds.max(1.0);
    let mut kinds: Vec<Contrast> = Vec::new();
    for c in &s.contrasts {
        if !kinds.contains(&c.contrast) {
            kinds.push(c.contrast);
        }
    }
    let mut out = Vec::new();
    for kind in kinds {
        let cell = |arm: bool, st: u8| {
            s.contrasts.iter().find(|c| c.contrast == kind && c.arm_a == arm && c.stratum == st).map_or((0.0, 0.0), |c| (c.wins, c.n))
        };
        let (mut big_w, mut ra, mut rb, mut var, mut na, mut nb) = (0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0);
        let mut strata = Vec::new();
        for st in 0..9u8 {
            let ((wa, a), (wb, b)) = (cell(true, st), cell(false, st));
            if a > 0.0 && b > 0.0 {
                strata.push((wa, a, wb, b));
                big_w += a + b;
            }
        }
        if big_w <= 0.0 {
            continue;
        }
        let k = gate_kappa(m) / 2.0;
        for (wa, a, wb, b) in strata {
            let ps = (wa + wb + 1.0) / (a + b + 2.0);
            let pa = (wa + k * ps) / (a + k);
            let pb = (wb + k * ps) / (b + k);
            let wt = (a + b) / big_w;
            ra += wt * pa;
            rb += wt * pb;
            var += wt * wt * (pa * (1.0 - pa) / (a + k) + pb * (1.0 - pb) / (b + k));
            na += a;
            nb += b;
        }
        let diff = ra - rb;
        let prob = normal_cdf(diff.abs() / var.sqrt().max(1e-4));
        if na < MIN_DUELS || nb < MIN_DUELS || prob < CONTRAST_CREDIBLE || diff.abs() < 0.1 {
            continue;
        }
        let worse_n = if diff < 0.0 { na } else { nb };
        let text = match kind {
            Contrast::Repeek => format!(
                "When you re-peek the same angle within 5 s, you win {} of duels. When you reposition first, you win {}.",
                pct(ra),
                pct(rb)
            ),
            Contrast::Crouch => format!("When you fight crouched you win {} of duels; standing, {}.", pct(ra), pct(rb)),
            Contrast::Side(b) => {
                let zone = arena.blocks.get(b as usize).and_then(|blk| arena.zone_of(blk.center)).map_or("that area", |z| z.name);
                format!("Peeking left of the cover in {zone}, you win {} of duels; peeking right of it, {}.", pct(ra), pct(rb))
            }
        };
        out.push(Insight { kind: InsightKind::Contrast(kind), text, stake: worse_n / rounds * diff.abs(), prob });
    }
    out
}

// ---------------------------------------------------------------------------
// Per-match summaries and trends (section 6.4).

/// Per-match sufficient statistics, kept in history so trends can be tested later.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchPosterior {
    pub duel_wins: f32,
    pub duels: f32,
    pub rt_n: f32,
    pub rt_mean_log: f32,
    pub rt_var_log: f32,
    pub hits: f32,
    pub shots: f32,
}

impl MatchPosterior {
    pub fn from_telemetry(t: &MatchTelemetry) -> Self {
        let decided: Vec<_> = t.duels.iter().filter(|d| d.decided()).collect();
        let logs: Vec<f32> = t.reactions.iter().filter(|&&r| r > 0.05).map(|r| r.ln()).collect();
        let n = logs.len() as f32;
        let mean = if n > 0.0 { logs.iter().sum::<f32>() / n } else { 0.0 };
        let var = if n > 1.0 { logs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / (n - 1.0) } else { 0.1 };
        MatchPosterior {
            duel_wins: decided.iter().filter(|d| d.won()).count() as f32,
            duels: decided.len() as f32,
            rt_n: n,
            rt_mean_log: mean,
            rt_var_log: var,
            hits: t.hits.iter().sum(),
            shots: t.shots.iter().sum(),
        }
    }
}

fn rate_change(w1: f32, n1: f32, w0: f32, n0: f32) -> (f32, f32, f32) {
    let (p1, p0) = ((w1 + 1.0) / (n1 + 2.0), (w0 + 1.0) / (n0 + 2.0));
    let sd = (p1 * (1.0 - p1) / (n1 + 2.0) + p0 * (1.0 - p0) / (n0 + 2.0)).sqrt().max(1e-4);
    (p0, p1, normal_cdf((p1 - p0).abs() / sd))
}

/// Last match vs the up-to-3 matches before it; reported only if P(change) >= 0.9.
pub fn trend_insights(history: &[MatchPosterior]) -> Vec<Insight> {
    let Some((last, before)) = history.split_last() else { return vec![] };
    let prev: Vec<&MatchPosterior> = before.iter().rev().take(3).collect();
    if prev.is_empty() {
        return vec![];
    }
    let span = prev.len() + 1;
    let sum = |f: fn(&MatchPosterior) -> f32| prev.iter().map(|p| f(p)).sum::<f32>();
    let mut out = Vec::new();
    let (w0, n0) = (sum(|p| p.duel_wins), sum(|p| p.duels));
    if last.duels >= MIN_DUELS && n0 >= MIN_DUELS {
        let (a, b, prob) = rate_change(last.duel_wins, last.duels, w0, n0);
        if prob >= CREDIBLE && (b - a).abs() >= 0.08 {
            out.push(Insight {
                kind: InsightKind::Trend,
                text: format!("Your duel win rate went from {} to {} over your last {span} matches.", pct(a), pct(b)),
                stake: 0.12,
                prob,
            });
        }
    }
    let (h0, s0) = (sum(|p| p.hits), sum(|p| p.shots));
    if last.shots >= 20.0 && s0 >= 20.0 {
        let (a, b, prob) = rate_change(last.hits, last.shots, h0, s0);
        if prob >= CREDIBLE && (b - a).abs() >= 0.04 {
            out.push(Insight {
                kind: InsightKind::Trend,
                text: format!("Your accuracy went from {} to {} over your last {span} matches.", pct(a), pct(b)),
                stake: 0.1,
                prob,
            });
        }
    }
    let rn0 = sum(|p| p.rt_n);
    if last.rt_n >= 4.0 && rn0 >= 4.0 {
        let m0 = prev.iter().map(|p| p.rt_mean_log * p.rt_n).sum::<f32>() / rn0;
        let v0 = prev.iter().map(|p| p.rt_var_log * p.rt_n).sum::<f32>() / rn0;
        let sd = (last.rt_var_log / last.rt_n + v0 / rn0).sqrt().max(1e-4);
        let prob = normal_cdf((last.rt_mean_log - m0).abs() / sd);
        let (a, b) = (1000.0 * m0.exp(), 1000.0 * last.rt_mean_log.exp());
        if prob >= CREDIBLE && (b / a - 1.0).abs() >= 0.08 {
            out.push(Insight {
                kind: InsightKind::Trend,
                text: format!("Your reaction time went from {a:.0} to {b:.0} ms over your last {span} matches."),
                stake: 0.1,
                prob,
            });
        }
    }
    out
}

/// All v2 insights, ranked by rounds at stake.
pub fn insights_v2(m: &PlayerModel, arena: &Arena, history: &[MatchPosterior]) -> Vec<Insight> {
    let mut v = situation_insights(m, 4);
    v.extend(contrast_insights(m, arena));
    v.extend(trend_insights(history));
    v.sort_by(|a, b| b.stake.total_cmp(&a.stake));
    v
}

// ---------------------------------------------------------------------------
// The "what they learned -> what they did -> did it work" card (section 6.5).

#[derive(Clone, Debug, Default)]
pub struct SquadCard {
    pub learned: String,
    pub did: String,
    pub worked: String,
    pub counter: String,
}

pub fn goal_doing(g: Goal) -> &'static str {
    match g {
        Goal::Attack => "attacking head-on",
        Goal::Push => "pushing you",
        Goal::Flank => "flanking",
        Goal::TakeCover => "in cover",
        Goal::Retreat => "falling back",
        Goal::Ambush => "holding ambushes",
        Goal::Hunt => "hunting for you",
    }
}

fn top_zone(arena: &Arena, heat: &[f32]) -> Option<(&'static str, f32)> {
    let total: f32 = heat.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mut z: Vec<(&'static str, f32)> = Vec::new();
    for (i, &v) in heat.iter().enumerate() {
        if let Some(zone) = arena.zone_of(heat_center(i)).filter(|_| v > 0.0) {
            match z.iter_mut().find(|(n, _)| *n == zone.name) {
                Some(e) => e.1 += v / total,
                None => z.push((zone.name, v / total)),
            }
        }
    }
    z.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))
}

fn zone_share(arena: &Arena, heat: &[f32], name: &str) -> f32 {
    let total: f32 = heat.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    heat.iter().enumerate().filter(|(i, _)| arena.zone_of(heat_center(*i)).is_some_and(|z| z.name == name)).map(|(_, v)| v).sum::<f32>() / total
}

/// `before`: the model the squad was trained against; `top`: the best insight after this match.
#[allow(clippy::too_many_arguments)]
pub fn squad_card(
    before: &PlayerModel,
    arena: &Arena,
    t: &MatchTelemetry,
    enemies: &[EnemyReport],
    player_rounds: u32,
    enemy_rounds: u32,
    top: Option<&Insight>,
) -> SquadCard {
    let mut card = SquadCard::default();
    let mut expected_zone: Option<String> = None;
    if before.matches_observed == 0 {
        card.learned = "First match: they knew nothing about you yet.".into();
    } else if let Some(Insight { kind: InsightKind::Situation(g), .. }) = situation_insights(before, 1).first() {
        let here: Vec<_> = t.duels.iter().filter(|d| d.decided() && g.matches(&d.key())).collect();
        let won = here.iter().filter(|d| d.won()).count();
        card.learned = format!("They expected you to lose duels {}.", g.phrase());
        if !here.is_empty() {
            card.learned += &format!(" This match you won {won} of {} there.", here.len());
        }
        expected_zone = g.zone.clone();
    } else if let Some((zone, share)) = top_zone(arena, &before.heat) {
        let now = zone_share(arena, &t.heat, zone);
        card.learned = format!("They expected you in {zone} ({} of your time). This match: {}.", pct(share), pct(now));
        expected_zone = Some(zone.to_string());
    }

    let mut time = [0.0f32; 7];
    let mut kills = [0u32; 7];
    for e in enemies {
        for k in 0..7 {
            time[k] += e.goal_time[k];
            kills[k] += e.kills_by_goal[k];
        }
    }
    let total: f32 = time.iter().sum();
    let mut order: Vec<usize> = (0..7).collect();
    order.sort_by(|&a, &b| time[b].total_cmp(&time[a]));
    let flank = Goal::ALL.iter().position(|&g| g == Goal::Flank).unwrap_or(2);
    if total > 1.0 {
        card.did = format!(
            "They spent {} of their time {} and {} {}.",
            pct(time[order[0]] / total),
            goal_doing(Goal::ALL[order[0]]),
            pct(time[order[1]] / total),
            goal_doing(Goal::ALL[order[1]])
        );
        if order[0] != flank && order[1] != flank {
            card.did += &format!(" Flanking: {}.", pct(time[flank] / total));
        }
    } else {
        card.did = "No squad behaviour was recorded this match.".into();
    }

    let rounds = player_rounds + enemy_rounds;
    card.worked = format!("They won {enemy_rounds} of {rounds} rounds.");
    let k_total: u32 = kills.iter().sum();
    if let Some(best) = (0..7).filter(|&k| kills[k] > 0).max_by_key(|&k| kills[k]) {
        card.worked += &format!(" {} of their {k_total} kills came while {}.", kills[best], goal_doing(Goal::ALL[best]));
    }

    card.counter = match top.map(|i| &i.kind) {
        Some(InsightKind::Situation(g)) => match (g.peek, &g.zone) {
            (Some(PeekSide::Left), Some(z)) => format!("Counter: take {z} from the right side, or use another route."),
            (Some(PeekSide::Right), Some(z)) => format!("Counter: take {z} from the left side, or use another route."),
            (_, Some(z)) => format!("Counter: avoid {z} for a while, or enter it from a new angle."),
            _ if g.odds.is_some_and(|o| o >= 2) => "Counter: pull back and fight them one at a time.".into(),
            _ if g.range == Some(0) => "Counter: keep your distance; don't let them close in.".into(),
            _ if g.range.is_some() => "Counter: close the distance before you commit.".into(),
            _ => "Counter: change the habit above.".into(),
        },
        Some(InsightKind::Contrast(c)) => match c {
            Contrast::Repeek => "Counter: after a fight, move before you peek again.".into(),
            Contrast::Crouch => "Counter: switch stance; your other stance wins more.".into(),
            Contrast::Side(_) => "Counter: peek that cover from your stronger side.".into(),
        },
        _ => match expected_zone {
            Some(z) => format!("Counter: vary your route; they will keep holding {z}."),
            None => "Counter: mix up your routes and angles.".into(),
        },
    };
    card
}

/// Build a situation key from raw values (for the ghost/clone: look up `SituationModel::win_prob`).
pub fn duel_key(zone: &str, peek: PeekSide, range: f32, visible: u8) -> SitKey {
    SitKey { zone: zone.to_string(), peek, range: range3(range), odds: odds_bin(visible) }
}
