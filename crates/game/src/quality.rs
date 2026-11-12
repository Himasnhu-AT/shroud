//! When the engine's quality governor may act in Tells (`engine::render_budget::QualityGovernor`).
//!
//! The governor lowers quality when the GPU can't hold 60 fps. Mid-round that showed as the
//! owner's playtest bugs: render scale dropped at 55-67 fps (a blurrier picture, a retargeted
//! camera, TAA history thrown away, a 0.3-0.45 s stall and a few frames of an empty map) right
//! after spawning and when he started to move, with a toast over the action. So:
//!
//! - **Never during a live round.** The governor only decides in a window of freeze time
//!   ([`DECIDE_AFTER_S`] in, so spawn-time shader warm-up isn't what it measures), and at most
//!   one step per round ([`RoundQuality`]). During live play (and while paused) its ladder is cut
//!   at the level it applied and step-ups are off: it holds whatever it chose. Memory
//!   emergencies still act (they would otherwise end in a crash).
//! - **Cheap levers first.** Shadow resolution and the lamp-shadow budget (`Game(n)`) go before
//!   any render-scale step, and the scale never goes below 75 %.
//! - **No toast in a round** (`ui::perf_toast`): changes are logged; the perf overlay shows them.
//!
//! Dev: `--governor` turns the governor on in test-driven runs (control port, offscreen), where the
//! engine starts it off, so play.py can measure it.

use bevy::prelude::*;
use engine::render_budget::{GovernorState, GovernorStep, QualityGovernor};

use crate::economy::RoundPhase;
use crate::state::{GameState, Paused};

/// Seconds into freeze time before the governor may judge (pipelines warm, exposure settled).
pub const DECIDE_AFTER_S: f32 = 3.0;
/// The engine default for "enough headroom to step back up" (share of the frame budget).
const UP_AT: f32 = 0.70;

/// Governor changes counted at the start of this round (one step per round).
#[derive(Resource, Default)]
pub struct RoundQuality {
    changes_at_start: Option<u32>,
    freeze_total: f32,
}

pub struct QualityPlugin;

impl Plugin for QualityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RoundQuality>()
            .add_systems(Startup, configure)
            .add_systems(OnEnter(GameState::Playing), |mut r: ResMut<RoundQuality>| *r = RoundQuality::default())
            .add_systems(Update, hold_in_rounds);
    }
}

/// The steps Tells allows, mildest first: nothing that rebuilds pipelines, resolution last.
pub fn steps() -> Vec<GovernorStep> {
    use GovernorStep::*;
    vec![ShadowMapHalf, Game(1), Game(2), Scale(0.85), Scale(0.75)]
}

fn configure(governor: Option<ResMut<QualityGovernor>>) {
    let Some(mut g) = governor else { return };
    g.steps = steps();
    #[cfg(feature = "dev")]
    if engine::cli::dev_flag("governor") {
        g.enabled = true;
    }
}

/// May the governor decide now? Only in freeze time, after the warm-up, before its one step.
pub fn may_decide(state: Option<GameState>, paused: bool, freeze_left: f32, freeze_elapsed: f32, stepped: bool) -> bool {
    match state {
        // Menus, debrief, evolving: nothing to disturb.
        Some(s) if s != GameState::Playing => !paused,
        None => !paused,
        Some(_) => !paused && freeze_left > 0.0 && freeze_elapsed >= DECIDE_AFTER_S && !stepped,
    }
}

fn hold_in_rounds(
    state: Option<Res<State<GameState>>>,
    paused: Option<Res<Paused>>,
    phase: Option<Res<RoundPhase>>,
    gov_state: Option<Res<GovernorState>>,
    mut round: ResMut<RoundQuality>,
    governor: Option<ResMut<QualityGovernor>>,
) {
    let Some(mut g) = governor else { return };
    let state = state.map(|s| *s.get());
    let freeze_left = phase.map_or(0.0, |p| p.freeze_left);
    let (changes, level) = gov_state.as_ref().map_or((0, 0), |s| (s.changes, s.level));
    let start = *round.changes_at_start.get_or_insert(changes);
    round.freeze_total = round.freeze_total.max(freeze_left);
    let open = may_decide(state, paused.is_some_and(|p| p.0), freeze_left, round.freeze_total - freeze_left, changes > start);
    // Closed: the ladder ends at the level already applied (nothing left to step down to, the
    // applied steps stay as they are) and nothing counts as headroom (no step back up). Toggling
    // `enabled` instead would reset it to full quality, itself a mid-round change.
    let all = steps();
    let (want_steps, up_at) = if open { (all, UP_AT) } else { (all[..level.min(all.len())].to_vec(), 0.0) };
    if g.steps != want_steps {
        info!("quality: governor {} (level {level})", if open { "may step now (freeze time)" } else { "holds (live round)" });
        g.steps = want_steps;
    }
    if g.up_at != up_at {
        g.up_at = up_at;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_governor_only_decides_in_freeze_time_once_per_round() {
        let p = Some(GameState::Playing);
        // Live round: never.
        assert!(!may_decide(p, false, 0.0, 9.0, false));
        // Freeze time, still warming up: no; after: yes; after a step this round: no.
        assert!(!may_decide(p, false, 6.0, 2.0, false));
        assert!(may_decide(p, false, 4.0, 4.0, false));
        assert!(!may_decide(p, false, 4.0, 4.0, true));
        // Paused: no. Outside rounds: yes.
        assert!(!may_decide(p, true, 4.0, 4.0, false));
        assert!(may_decide(Some(GameState::Debrief), false, 0.0, 0.0, false));
        // Resolution is the last resort and never below 75 %.
        let s = steps();
        let first_scale = s.iter().position(|x| matches!(x, GovernorStep::Scale(_))).unwrap();
        assert!(s[..first_scale].iter().all(|x| !x.costly()));
        assert!(s.iter().all(|x| !matches!(x, GovernorStep::Scale(f) if *f < 0.75)));
    }
}
