//! Bomb defusal for the defenders (`crate::objective`), CS-style.
//!
//! - **Setup** (`Play::Setup`, before the plant while he is unknown): anchors split between A
//!   and B, holding the attacker entries into each site (where the path from his spawn, or
//!   from the other site, reaches it) from a spot in or by the site with a line on the entry,
//!   preferring cover, off his usual pre-aim and on his death spots. The site he plants on more
//!   often (`PlayerModel::plant_share`) and spends time near gets the extra anchor. From four
//!   alive, one is a rotator holding mid between the sites (and checking fresh noises).
//! - **Rotations**: info (a sighting or a sound) near a site sends the rotator and all but one
//!   of the other site's anchors there at once ("Rotating A"). The last anchor stays until
//!   the info is confirmed (he has been at that site for `ROTATE_CONFIRM_S`) — a fake doesn't
//!   empty the other site. In a fight before the plant the play is the usual one (encircle,
//!   bandit plays) but that anchor keeps its site out of it.
//! - **Retake** (`Play::Retake`, bomb planted): everyone converges on the bomb through the
//!   encirclement's approach sectors around it (staging cells out of its sight, distinct
//!   routes from `encircle::diverse_vias`), then clears the angle on it from the peek cell.
//!   One defuser (nearest by path, not in a fight, healthy; re-picked if he dies) goes to the
//!   bomb and stays on it. He waits at a staging cell while the player is known to be watching
//!   the bomb, unless the fuse is running short (a stick: less than travel + `DEFUSE_S` +
//!   `STICK_S` left). If travel + `DEFUSE_S` can't beat the fuse, everyone saves: out of the
//!   blast (`BLAST_MAX_M`).

use super::*;
use crate::objective::{BLAST_MAX_M, BombSite, DEFUSE_REACH, DEFUSE_S};

/// Info this close (m) to a site's rectangle means he is going there.
pub const ROTATE_NEAR_M: f32 = 10.0;
/// ...and the other site must be at least this much further away (m).
pub const ROTATE_GAP_M: f32 = 8.0;
/// He has been at the site this long (s) before the last anchor of the other site leaves.
pub const ROTATE_CONFIRM_S: f32 = 3.0;
/// A rotation with no new info for this long (s) is dropped: everyone goes back to setup.
pub const ROTATE_STALE_S: f32 = 25.0;
/// Pace (m/s) used to plan the defuser's travel (it sprints unseen, walks the last bit).
pub const RETAKE_PACE: f32 = 4.5;
/// Safety margin (s) on the fuse for starting a defuse off the bomb.
pub const DEFUSE_MARGIN_S: f32 = 1.0;
/// With less than travel + `DEFUSE_S` + this left (s), defuse even with him watching.
pub const STICK_S: f32 = 4.0;
/// He is "watching the bomb" if seen within this long (s) with a line on it.
pub const WATCH_S: f32 = 3.0;
/// Save spots are at least this far past `BLAST_MAX_M` (m).
pub const SAVE_MARGIN_M: f32 = 3.0;
/// An anchor counts as still on its site within this far (m) of its spot.
const ON_SPOT_M: f32 = 15.0;

/// A defender's pre-plant job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duty {
    /// Holds site `n` (index into `Arena::sites`).
    Anchor(usize),
    /// Holds mid; first to rotate.
    Rotator,
}

/// The squad's objective state.
#[derive(Clone, Debug, Default)]
pub struct SiteState {
    /// Attacker entries into each site (computed once per arena).
    pub chokes: Vec<Vec<V2>>,
    /// Mid between the sites (rotator spot) and what it watches.
    pub mid: Option<(V2, V2)>,
    /// Per member: its pre-plant duty and (spot, angle it watches).
    pub duty: Vec<Option<Duty>>,
    pub hold: Vec<Option<(V2, V2)>>,
    /// Alive members the duties were planned for (re-planned when this changes).
    planned_for: Vec<usize>,
    /// Rotating to this site: (site, when called, last fresh info, confirmed).
    pub rotate: Option<(usize, f32, f32, bool)>,
    /// Members already sent to the rotation site.
    pub rotated: Vec<usize>,
    /// When he was first placed at the rotation site in a row (confirmation clock).
    at_site_t: Option<f32>,
    // --- Retake.
    /// The fuse can't be beaten: everyone is saving.
    pub saving: bool,
    pub save_spot: Vec<Option<V2>>,
    /// Approach sectors around the bomb, and who has reached its staging cell.
    pub sectors: Vec<Sector>,
    pub staged: Vec<bool>,
    retakers: Vec<usize>,
    /// The defuser is going for the bomb (false: waiting for the angle to be cleared).
    pub going: bool,
    defuse_called: bool,
    watch_called: bool,
}

/// Distance (m) from `p` to the site's rectangle (0 inside).
pub fn site_dist(s: &BombSite, p: V2) -> f32 {
    let dx = (s.min.x - p.x).max(p.x - s.max.x).max(0.0);
    let dz = (s.min.y - p.y).max(p.y - s.max.y).max(0.0);
    (dx * dx + dz * dz).sqrt()
}

/// Where attackers walk into site `s`: the first point of the path from his spawn (and from
/// each other site) within 5 m of the site's rectangle, deduplicated.
pub fn site_chokes(arena: &Arena, s: usize) -> Vec<V2> {
    let site = arena.sites[s];
    let c = arena.nav.nearest_walkable(site.center());
    let mut froms = vec![arena.player_spawn];
    froms.extend(arena.sites.iter().enumerate().filter(|(k, _)| *k != s).map(|(_, o)| o.center()));
    let mut out: Vec<V2> = Vec::new();
    for f in froms {
        let Some(path) = arena.nav.find_path(arena.nav.nearest_walkable(f), c) else { continue };
        let Some(ch) = path.iter().copied().find(|p| site_dist(&site, *p) < 5.0) else { continue };
        if !out.iter().any(|o| o.dist(ch) < 6.0) {
            out.push(ch);
        }
    }
    if out.is_empty() {
        out.push(arena.nav.nearest_walkable(site.center() + (arena.player_spawn - site.center()).norm() * 6.0));
    }
    out
}

/// Best spot to hold `choke` from in or by `site`: a line on the entry at `range`-ish,
/// cover, inside the site, off his pre-aim, on his death spots, away from `taken`.
pub fn hold_spot(arena: &Arena, model: &PlayerModel, site: &BombSite, choke: V2, taken: &[V2], range: f32) -> V2 {
    let pad = 4.0;
    let mut cands: Vec<(V2, bool)> = arena.cover.iter().filter(|c| site_dist(site, c.pos) < pad).map(|c| (c.pos, true)).collect();
    let mut x = site.min.x - pad;
    while x <= site.max.x + pad {
        let mut z = site.min.y - pad;
        while z <= site.max.y + pad {
            let p = V2::new(x, z);
            if arena.nav.walkable_at(p) {
                cands.push((p, false));
            }
            z += 2.0;
        }
        x += 2.0;
    }
    let range = range.clamp(8.0, 20.0);
    let mut best: Option<(f32, V2)> = None;
    for (p, cover) in cands {
        let d = p.dist(choke);
        if !(5.0..=32.0).contains(&d) || !arena.nav.walkable_at(p) || !arena.line_of_sight(p, false, choke, false) {
            continue;
        }
        let mut s = -(d - range).abs() / 10.0;
        s -= site_dist(site, p) * 0.15;
        if cover {
            s += 0.4;
        }
        if !arena.line_of_sight(p, true, choke, false) {
            s += 0.4; // shoot standing, hide crouched
        }
        s -= 2.0 * pre_aim_share(model, choke, p);
        s += 0.5 * death_heat(model, p);
        for t in taken {
            if t.dist(p) < 5.0 {
                s -= 1.5 * (1.0 - t.dist(p) / 5.0) + 0.5;
            }
        }
        if best.is_none_or(|(b, _)| s > b) {
            best = Some((s, p));
        }
    }
    best.map_or_else(|| arena.nav.nearest_walkable(site.center()), |b| b.1)
}

/// Assign each of `from` a distinct hold (same count) minimising the total straight-line
/// distance: exhaustive for small squads, greedy beyond 6.
fn best_matching(from: &[V2], holds: &[(V2, V2)]) -> Vec<usize> {
    let n = from.len().min(holds.len());
    let mut idx: Vec<usize> = (0..holds.len()).collect();
    if n > 6 {
        return idx.into_iter().take(from.len()).collect();
    }
    let cost = |perm: &[usize]| (0..n).map(|k| from[k].dist(holds[perm[k]].0)).sum::<f32>();
    let mut best = (cost(&idx), idx.clone());
    // Heap's algorithm over all permutations.
    let mut c = vec![0usize; idx.len()];
    let mut i = 0;
    while i < idx.len() {
        if c[i] < i {
            if i % 2 == 0 { idx.swap(0, i) } else { idx.swap(c[i], i) }
            let k = cost(&idx);
            if k < best.0 - 1e-4 {
                best = (k, idx.clone());
            }
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
    best.1
}

/// How much to stack each site (sums to 1): his plant history and his time near it.
pub fn site_weights(arena: &Arena, model: &PlayerModel) -> [f32; 2] {
    let mut heat = [0.0f32; 2];
    for (i, h) in model.heat.iter().enumerate() {
        let c = crate::player_model::heat_center(i);
        for (s, site) in arena.sites.iter().take(2).enumerate() {
            if site_dist(site, c) < 6.0 {
                heat[s] += h;
            }
        }
    }
    let ht = heat[0] + heat[1];
    let w = |s: usize| model.plant_share(s) + if ht > 0.0 { 0.5 * heat[s] / ht } else { 0.25 };
    let (a, b) = (w(0), w(1));
    [a / (a + b), b / (a + b)]
}

impl SquadCoordinator {
    /// Objective bookkeeping each plan: rotation calls and confirmation. Returns the member to
    /// keep out of a contact play (an anchor holding the other site before the plant).
    pub(super) fn objective_update(&mut self, play: Play, members: &[MemberView], arena: &Arena, model: &PlayerModel) -> Option<usize> {
        if !self.objective || arena.sites.len() < 2 {
            return None;
        }
        let n = members.len();
        let st = &mut self.site;
        st.duty.resize(n, None);
        st.hold.resize(n, None);
        st.save_spot.resize(n, None);
        st.staged.resize(n, false);
        if st.chokes.is_empty() {
            st.chokes = (0..arena.sites.len()).map(|s| site_chokes(arena, s)).collect();
            let (a, b) = (arena.nav.nearest_walkable(arena.sites[0].center()), arena.nav.nearest_walkable(arena.sites[1].center()));
            let mid = arena.nav.find_path(a, b).and_then(|p| p.get(p.len() / 2).copied()).unwrap_or_else(|| arena.nav.nearest_walkable((a + b) * 0.5));
            st.mid = Some((mid, arena.player_spawn));
        }
        if self.bomb.is_some() {
            return None;
        }
        let alive: Vec<usize> = (0..n).filter(|&i| members[i].alive).collect();
        if alive != self.site.planned_for {
            self.plan_duties(members, arena, model);
        }
        self.track_rotation(members, arena, model);
        // In a fight before the plant: one anchor of the site he is not at keeps it.
        if !matches!(play, Play::Encircle | Play::Crossfire | Play::Trade | Play::PinFlank | Play::Bait | Play::Collapse | Play::Sweep | Play::Overwatch) {
            return None;
        }
        if alive.len() < 3 || self.site.rotate.is_some_and(|r| r.3) {
            return None;
        }
        let p = self.board.pos;
        let his = (0..2).min_by(|&a, &b| site_dist(&arena.sites[a], p).total_cmp(&site_dist(&arena.sites[b], p)))?;
        let other = 1 - his;
        alive
            .iter()
            .copied()
            .filter(|&i| self.site.duty[i] == Some(Duty::Anchor(other)) && members[i].last_seen > 1.0)
            .filter_map(|i| self.site.hold[i].map(|h| (i, members[i].pos.dist(h.0))))
            .filter(|(_, d)| *d < ON_SPOT_M)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0)
    }

    /// The order for a member holding its pre-plant spot.
    pub(super) fn site_hold_order(&self, i: usize) -> Option<Order> {
        let (spot, watch) = self.site.hold.get(i).copied().flatten()?;
        let role = if self.site.duty.get(i).copied().flatten() == Some(Duty::Rotator) { Role::Rotator } else { Role::SiteHold };
        Some(Order { role, goal: Goal::Hold, dest: Some(spot), watch: Some(watch), threat: None, sticky: true, via: None, prefire: None })
    }

    /// Split the alive members between the sites (and mid) and pick their spots.
    fn plan_duties(&mut self, members: &[MemberView], arena: &Arena, model: &PlayerModel) {
        let alive: Vec<usize> = (0..members.len()).filter(|&i| members[i].alive).collect();
        self.site.planned_for = alive.clone();
        self.site.duty.iter_mut().for_each(|d| *d = None);
        self.site.hold.iter_mut().for_each(|h| *h = None);
        self.site.rotated.clear();
        let n = alive.len();
        if n == 0 {
            return;
        }
        let w = site_weights(arena, model);
        let heavy = if w[1] > w[0] { 1 } else { 0 };
        // (anchors on A, anchors on B, rotators)
        let (ka, kb, kr) = match n {
            1 => (0, 0, 1),
            2 => (1, 1, 0),
            3 => {
                if heavy == 0 { (2, 1, 0) } else { (1, 2, 0) }
            }
            _ => {
                let k = n - 1;
                let on_heavy = ((k as f32 * w[heavy]).round() as usize).clamp(1, k - 1);
                if heavy == 0 { (on_heavy, k - on_heavy, 1) } else { (k - on_heavy, on_heavy, 1) }
            }
        };
        // Who goes where: by how much closer (path) each is to A than to B.
        let fa = arena.nav.distance_field(&[arena.sites[0].center()], f32::INFINITY);
        let fb = arena.nav.distance_field(&[arena.sites[1].center()], f32::INFINITY);
        let lean = |i: usize| {
            let (a, b) = (arena.nav.field_at(&fa, members[i].pos), arena.nav.field_at(&fb, members[i].pos));
            let (a, b) = (if a.is_finite() { a } else { 999.0 }, if b.is_finite() { b } else { 999.0 });
            a - b
        };
        let mut order = alive.clone();
        order.sort_by(|&a, &b| lean(a).total_cmp(&lean(b)).then(a.cmp(&b)));
        let (on_a, rest) = order.split_at(ka);
        let (mid, on_b) = rest.split_at(kr);
        debug_assert_eq!(on_b.len(), kb);
        for (s, who) in [(0usize, on_a), (1, on_b)] {
            let chokes = self.site.chokes[s].clone();
            let mut taken: Vec<V2> = Vec::new();
            let mut holds: Vec<(V2, V2)> = Vec::new();
            for (k, &i) in who.iter().enumerate() {
                let choke = chokes[k % chokes.len()];
                let spot = hold_spot(arena, model, &arena.sites[s], choke, &taken, members[i].traits.preferred_range);
                taken.push(spot);
                holds.push((spot, choke));
            }
            // Who takes which spot: the shortest total walk (nobody crosses a teammate).
            let pos: Vec<V2> = who.iter().map(|&i| members[i].pos).collect();
            for (k, h) in best_matching(&pos, &holds).into_iter().enumerate() {
                let i = who[k];
                self.site.duty[i] = Some(Duty::Anchor(s));
                self.site.hold[i] = Some(holds[h]);
            }
        }
        for &i in mid {
            self.site.duty[i] = Some(Duty::Rotator);
            self.site.hold[i] = self.site.mid;
        }
    }

    /// The freshest info on him (a sighting or a sound) and its age.
    fn fresh_info(&self) -> Option<(V2, f32)> {
        let heard = self.heard_at.filter(|_| self.heard_t < 8.0).map(|h| (h, self.heard_t));
        let seen = (self.board.age < 10.0).then_some((self.board.pos, self.board.age));
        match (heard, seen) {
            (Some(h), Some(s)) => Some(if h.1 < s.1 { h } else { s }),
            (a, b) => a.or(b),
        }
    }

    /// Rotation calls from info near a site, and their confirmation.
    fn track_rotation(&mut self, members: &[MemberView], arena: &Arena, model: &PlayerModel) {
        let now = self.time;
        let info = self.fresh_info();
        let toward = info.and_then(|(p, _)| {
            let (da, db) = (site_dist(&arena.sites[0], p), site_dist(&arena.sites[1], p));
            let (s, d, o) = if da < db { (0, da, db) } else { (1, db, da) };
            (d < ROTATE_NEAR_M && o > d + ROTATE_GAP_M).then_some(s)
        });
        match (toward, self.site.rotate) {
            (Some(s), Some(r)) if r.0 == s => {
                self.site.rotate = Some((s, r.1, now, r.3));
            }
            (Some(s), _) => {
                self.site.rotate = Some((s, now, now, false));
                self.site.rotated.clear();
                self.site.at_site_t = None;
                // The rotator and all but one of the other site's anchors go now.
                let alive = |i: usize| members.get(i).is_some_and(|m| m.alive);
                let mut go: Vec<usize> = (0..members.len()).filter(|&i| alive(i) && self.site.duty[i] == Some(Duty::Rotator)).collect();
                let mut others: Vec<usize> = (0..members.len()).filter(|&i| alive(i) && self.site.duty[i] == Some(Duty::Anchor(1 - s))).collect();
                // The one nearest its own spot stays.
                others.sort_by(|&a, &b| {
                    let d = |i: usize| self.site.hold[i].map_or(99.0, |h| members[i].pos.dist(h.0));
                    d(a).total_cmp(&d(b))
                });
                go.extend(others.iter().skip(1));
                self.send_rotation(s, &go, members, arena, model);
            }
            (None, Some(r)) if now - r.2 > ROTATE_STALE_S => {
                // Old info: back to the setup.
                self.site.rotate = None;
                self.site.rotated.clear();
                let alive: Vec<usize> = (0..members.len()).filter(|&i| members[i].alive).collect();
                self.site.planned_for = alive;
                self.site.planned_for.push(usize::MAX); // force a re-plan next time
            }
            _ => {}
        }
        // Confirmation: he has really been at that site for a while (seen, not a lone sound).
        if let Some((s, t0, last, false)) = self.site.rotate {
            let there = self.board.age < 1.5 && site_dist(&arena.sites[s], self.board.pos) < 4.0;
            if there {
                let since = *self.site.at_site_t.get_or_insert(now);
                if now - since >= ROTATE_CONFIRM_S {
                    self.site.rotate = Some((s, t0, last, true));
                    let rest: Vec<usize> = (0..members.len())
                        .filter(|&i| members[i].alive && self.site.duty[i] == Some(Duty::Anchor(1 - s)) && !self.site.rotated.contains(&i))
                        .collect();
                    self.send_rotation(s, &rest, members, arena, model);
                }
            } else {
                self.site.at_site_t = None;
            }
        }
    }

    fn send_rotation(&mut self, s: usize, who: &[usize], members: &[MemberView], arena: &Arena, model: &PlayerModel) {
        if who.is_empty() {
            return;
        }
        let site = arena.sites[s];
        let chokes = self.site.chokes[s].clone();
        let mut taken: Vec<V2> = (0..members.len())
            .filter(|&i| members[i].alive && (self.site.duty[i] == Some(Duty::Anchor(s)) || self.site.rotated.contains(&i)))
            .filter_map(|i| self.site.hold[i].map(|h| h.0))
            .collect();
        for (k, &i) in who.iter().enumerate() {
            let choke = chokes[(k + 1) % chokes.len()];
            let spot = hold_spot(arena, model, &site, choke, &taken, members[i].traits.preferred_range);
            taken.push(spot);
            self.site.hold[i] = Some((spot, choke));
            self.site.rotated.push(i);
        }
        let text = format!("Rotating {}", site.letter);
        self.bark(who[0], text, true);
    }

    /// Before the plant, he is unknown: hold the sites (and mid), rotate on info.
    pub(super) fn plan_setup(&mut self, members: &[MemberView], arena: &Arena, changed: bool) {
        if changed {
            self.vias.iter_mut().for_each(|v| *v = None);
            self.sector_of.iter_mut().for_each(|s| *s = None);
            self.investigators.clear();
        }
        let hot = self.fresh_info().filter(|(_, age)| *age < 6.0).map(|h| h.0);
        for i in 0..members.len() {
            if !members[i].alive || i >= self.site.duty.len() {
                self.orders[i] = None;
                continue;
            }
            let rotator = self.site.duty.get(i).copied().flatten() == Some(Duty::Rotator) && !self.site.rotated.contains(&i);
            // The rotator checks a fresh noise or sighting in mid (near a site it's a rotation).
            if rotator && self.site.rotate.is_none() && let Some(h) = hot {
                self.orders[i] = Some(Order {
                    role: Role::Rotator,
                    goal: Goal::Hunt,
                    dest: Some(arena.nav.nearest_walkable(h)),
                    watch: Some(h),
                    threat: Some(h),
                    sticky: false,
                    via: None,
                    prefire: None,
                });
                continue;
            }
            self.orders[i] = self.site_hold_order(i);
        }
    }

    // -------------------------------------------------------------------------------------
    // Retake.

    /// The bomb is down: converge, clear, defuse (or save).
    pub(super) fn plan_retake(&mut self, members: &[MemberView], arena: &Arena, changed: bool) {
        let Some(bomb) = self.bomb else { return };
        let n = members.len();
        self.site.save_spot.resize(n, None);
        self.site.staged.resize(n, false);
        let alive: Vec<usize> = (0..n).filter(|&i| members[i].alive).collect();
        if alive.is_empty() {
            self.defuser = None;
            return;
        }
        let bp = bomb.pos;
        let letter = arena.sites.get(bomb.site).map_or('?', |s| s.letter);
        if changed {
            self.site.saving = false;
            self.site.sectors = encircle::sectors(arena, bp);
            self.site.staged.iter_mut().for_each(|s| *s = false);
            self.site.retakers.clear();
            self.site.going = false;
            self.site.defuse_called = false;
            self.site.watch_called = false;
            self.site.save_spot.iter_mut().for_each(|s| *s = None);
            self.vias.iter_mut().for_each(|v| *v = None);
            self.sector_of.iter_mut().for_each(|s| *s = None);
            self.investigators.clear();
            if let Some(s) = nearest_alive(members, bp) {
                self.bark(s, format!("Bomb's down at {letter}"), true);
            }
        }
        let field = arena.nav.distance_field(&[bp], f32::INFINITY);
        let path_d = |i: usize| {
            let d = arena.nav.field_at(&field, members[i].pos);
            if d.is_finite() { d } else { members[i].pos.dist(bp) * 1.5 }
        };
        // The defuser: nearest by path, not in a fight, healthy. Kept until he dies.
        if self.defuser.is_some_and(|d| !members.get(d).is_some_and(|m| m.alive)) {
            self.defuser = None;
            self.site.going = false;
            self.site.defuse_called = false;
        }
        if self.defuser.is_none() && !self.site.saving {
            let cost = |i: usize| path_d(i) + if members[i].last_seen < 1.0 { 15.0 } else { 0.0 } + 20.0 * (1.0 - members[i].hp).max(0.0);
            self.defuser = alive.iter().copied().min_by(|&a, &b| cost(a).total_cmp(&cost(b)).then(a.cmp(&b)));
            if let Some(d) = self.defuser
                && !changed
            {
                self.bark(d, "I've got the defuse".into(), true);
            }
        }
        // Can the fuse be beaten?
        if let Some(d) = self.defuser
            && !self.site.saving
        {
            let on = members[d].pos.dist(bp) <= DEFUSE_REACH;
            let need = if on && bomb.defuse_progress > 0.0 {
                DEFUSE_S * (1.0 - bomb.defuse_progress.clamp(0.0, 1.0))
            } else {
                path_d(d) / RETAKE_PACE + DEFUSE_S + if on { 0.0 } else { DEFUSE_MARGIN_S }
            };
            if need > bomb.fuse_left {
                self.site.saving = true;
                self.defuser = None;
                self.bark(d, "No time, saving".into(), true);
            }
        }
        if self.site.saving {
            for &i in &alive {
                if self.site.save_spot[i].is_none() {
                    self.site.save_spot[i] = save_spot(arena, members[i].pos, bp, (self.board.age < 10.0).then_some(self.board.pos));
                }
                let dest = self.site.save_spot[i];
                self.orders[i] = Some(Order { role: Role::Saving, goal: Goal::Hold, dest, watch: Some(bp), threat: None, sticky: true, via: None, prefire: None });
            }
            return;
        }
        let def = self.defuser;
        let known = self.board.age < KNOWN_S;
        let player = known.then_some(self.board.pos);
        // Go, or wait for the angle to be cleared: he watches the bomb and there's still time.
        if let Some(d) = def {
            let watching = self.board.age < WATCH_S && arena.line_of_sight(self.board.pos, false, bp, true);
            let short = bomb.fuse_left < path_d(d) / RETAKE_PACE + DEFUSE_S + STICK_S;
            let go = !watching || short || self.site.going && members[d].pos.dist(bp) <= DEFUSE_REACH;
            if go && !self.site.going {
                self.site.going = true;
                if watching {
                    self.bark(d, "No time - sticking it".into(), true);
                }
            } else if !go && self.site.going {
                self.site.going = false;
            }
            if !go && !self.site.watch_called {
                self.site.watch_called = true;
                self.bark(d, format!("He's watching the bomb - clear {letter} first"), false);
            }
        }
        // Approach sectors around the bomb for everyone not going for it.
        let movers: Vec<usize> = alive.iter().copied().filter(|&i| !(Some(i) == def && self.site.going)).collect();
        if movers != self.site.retakers {
            let dist: Vec<Vec<f32>> = movers
                .iter()
                .map(|&i| {
                    let f = arena.nav.distance_field(&[members[i].pos], f32::INFINITY);
                    self.site.sectors.iter().map(|s| arena.nav.field_at(&f, s.goal)).collect()
                })
                .collect();
            let prev: Vec<Option<usize>> = movers.iter().map(|&i| self.sector_of.get(i).copied().flatten()).collect();
            let mut taken: Vec<usize> = prev.iter().flatten().copied().collect();
            taken.sort_unstable();
            let unique = taken.windows(2).all(|w| w[0] != w[1]);
            let keep = (unique && !taken.is_empty()).then_some(prev.as_slice());
            let pick = encircle::assign(&self.site.sectors, &dist, &[], keep, &mut self.rng);
            let routes: Vec<(V2, V2)> = movers.iter().zip(&pick).filter_map(|(&i, s)| s.map(|s| (members[i].pos, self.site.sectors[s].goal))).collect();
            let vias = encircle::diverse_vias(arena, bp, player, &routes);
            let mut r = 0;
            self.sector_of.iter_mut().for_each(|s| *s = None);
            for (k, &i) in movers.iter().enumerate() {
                self.sector_of[i] = pick[k];
                if pick[k].is_some() {
                    if !self.site.staged[i] {
                        self.vias[i] = vias.get(r).copied().flatten();
                    }
                    r += 1;
                }
            }
            if changed {
                if let Some(&first) = movers.iter().find(|&&i| Some(i) != def) {
                    self.bark(first, format!("Retaking {letter} - split up"), false);
                }
            }
            self.site.retakers = movers.clone();
        }
        self.update_vias(members);
        for &i in &alive {
            if Some(i) == def && self.site.going {
                let on = members[i].pos.dist(bp) <= DEFUSE_REACH;
                if on && !self.site.defuse_called {
                    self.site.defuse_called = true;
                    self.bark(i, "Defusing, cover me".into(), true);
                }
                self.orders[i] = Some(Order {
                    role: Role::Defuser,
                    goal: Goal::Hold,
                    dest: Some(bp),
                    watch: Some(player.unwrap_or(bp)),
                    threat: if on { None } else { player },
                    sticky: true,
                    via: if on { None } else { self.vias[i] },
                    prefire: None,
                });
                continue;
            }
            let Some(s) = self.sector_of[i].and_then(|s| self.site.sectors.get(s)).copied() else {
                // No sector (tiny site, few cells): straight to the bomb's edge.
                let dest = arena.nav.nearest_walkable(bp + (members[i].pos - bp).norm() * 6.0);
                let role = if Some(i) == def { Role::Defuser } else { Role::Retaker };
                self.orders[i] = Some(Order { role, goal: Goal::Hold, dest: Some(dest), watch: Some(player.unwrap_or(bp)), threat: player, sticky: false, via: None, prefire: None });
                continue;
            };
            if members[i].pos.dist(s.goal) < ARRIVED_M {
                self.site.staged[i] = true;
            }
            let role = if Some(i) == def { Role::Defuser } else { Role::Retaker };
            // The waiting defuser stays at its staging cell; the rest clear the angle on the bomb.
            let staged = self.site.staged[i];
            let watch = Some(player.unwrap_or(bp));
            self.orders[i] = Some(if staged && Some(i) == def {
                Order { role, goal: Goal::Hold, dest: Some(s.goal), watch, threat: None, sticky: false, via: None, prefire: None }
            } else if staged {
                Order { role, goal: Goal::Hold, dest: Some(s.peek), watch, threat: None, sticky: false, via: None, prefire: None }
            } else {
                Order { role, goal: Goal::Flank, dest: Some(s.goal), watch, threat: Some(player.unwrap_or(bp)), sticky: true, via: self.vias[i], prefire: None }
            });
        }
    }
}

/// Nearest (by path) spot out of the blast: at least `BLAST_MAX_M + SAVE_MARGIN_M` from the
/// bomb, preferring cells away from where he was last seen.
pub fn save_spot(arena: &Arena, from: V2, bomb: V2, player: Option<V2>) -> Option<V2> {
    let r = BLAST_MAX_M + SAVE_MARGIN_M;
    let field = arena.nav.distance_field(&[from], 120.0);
    let mut best: Option<(f32, V2)> = None;
    for (i, &d) in field.iter().enumerate() {
        if !d.is_finite() {
            continue;
        }
        let c = arena.nav.center(i);
        if c.dist(bomb) < r {
            continue;
        }
        let near_him = player.map_or(0.0, |p| (12.0 - c.dist(p)).max(0.0) * 2.0);
        let cost = d + near_him;
        if best.is_none_or(|(b, _)| cost < b) {
            best = Some((cost, c));
        }
    }
    best.map(|b| b.1).or_else(|| Some(arena.nav.nearest_walkable(bomb + (from - bomb).norm() * r)))
}
