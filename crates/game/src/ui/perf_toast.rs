//! Tells the player when the engine's quality governor lowers or restores graphics.
//!
//! The governor (engine `render_budget::QualityGovernor`) steps quality down when the GPU or
//! memory can't keep up and back up when there is headroom. Each change sends a
//! `QualityChanged`; we turn it into a toast. GPU/headroom changes can come in bursts, so they
//! are rate-limited; memory changes always show (they never undo within a session).

use bevy::prelude::*;
use engine::render_budget::{GovernorReason, QualityChanged, QualityGovernor};
use engine::ui::theme::Theme;
use engine::ui::toast::Toast;

/// Minimum seconds between two load-driven toasts.
const MIN_GAP_S: f32 = 12.0;

pub struct PerfToastPlugin;

impl Plugin for PerfToastPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, announce.run_if(on_message::<QualityChanged>));
    }
}

/// Which changes get a toast now, given seconds since the last shown one.
fn should_show(reason: GovernorReason, since_last: f32) -> bool {
    match reason {
        GovernorReason::Memory => true,
        GovernorReason::Gpu | GovernorReason::Headroom | GovernorReason::Reset => since_last >= MIN_GAP_S,
    }
}

fn announce(
    mut changes: MessageReader<QualityChanged>,
    mut toasts: MessageWriter<Toast>,
    governor: Option<Res<QualityGovernor>>,
    theme: Option<Res<Theme>>,
    time: Res<Time<Real>>,
    mut last: Local<Option<f32>>,
) {
    let fps = governor.map(|g| g.target_fps).unwrap_or(60.0);
    let now = time.elapsed_secs();
    // Only the newest change of a burst matters: it describes the current state.
    let Some(ev) = changes.read().last() else {
        return;
    };
    debug!("perf toast: {:?} level {}/{}", ev.reason, ev.level, ev.levels);
    let since = last.map(|t| now - t).unwrap_or(f32::INFINITY);
    if !should_show(ev.reason, since) {
        return;
    }
    *last = Some(now);
    let theme = theme.map(|t| t.clone()).unwrap_or_default();
    let (title, color) = match ev.reason {
        GovernorReason::Gpu => ("Performance", theme.bad),
        GovernorReason::Memory => ("Low memory", theme.bad),
        GovernorReason::Headroom | GovernorReason::Reset => ("Performance", theme.good),
    };
    toasts.write(Toast::new(title).body(ev.text(fps)).color(color).secs(4.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_always_shows_and_load_changes_are_rate_limited() {
        assert!(should_show(GovernorReason::Memory, 0.0));
        assert!(!should_show(GovernorReason::Gpu, 3.0));
        assert!(should_show(GovernorReason::Gpu, MIN_GAP_S));
        assert!(should_show(GovernorReason::Headroom, f32::INFINITY));
    }
}
