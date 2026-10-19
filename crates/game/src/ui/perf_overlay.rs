//! Perf overlay (F2, `--perf`, or `perf.overlay`): a compact panel fed by the perf monitor's
//! snapshots. Text is rebuilt at most 4 times a second and only while visible.

use std::fmt::Write;
use std::time::{Duration, Instant};

use bevy::prelude::*;

use crate::ops::perf::{PerfCost, PerfOverlay, PerfShared, PerfSnapshot};
use crate::state::has_visuals;

const REFRESH: Duration = Duration::from_millis(250);

#[derive(Component)]
struct PerfPanel;

#[derive(Component)]
struct PerfText;

pub struct PerfOverlayPlugin;

impl Plugin for PerfOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn.run_if(has_visuals))
            .add_systems(Update, update.run_if(has_visuals).run_if(resource_exists::<PerfShared>));
    }
}

fn spawn(mut commands: Commands) {
    commands.spawn((
        PerfPanel,
        Node {
            position_type: PositionType::Absolute,
            left: px(20),
            top: px(56),
            padding: UiRect::axes(px(12), px(8)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.78)),
        GlobalZIndex(50),
        Visibility::Hidden,
        children![(
            PerfText,
            Text::new("perf: waiting for first sample"),
            TextFont::from_font_size(15.0),
            TextColor(Color::srgb(0.85, 0.95, 0.85)),
        )],
    ));
}

fn update(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut on: ResMut<PerfOverlay>,
    shared: Res<PerfShared>,
    mut cost: ResMut<PerfCost>,
    mut panel: Query<&mut Visibility, With<PerfPanel>>,
    mut text: Query<&mut Text, With<PerfText>>,
    mut last: Local<Option<Instant>>,
) {
    let t0 = Instant::now();
    if keys.is_some_and(|k| k.just_pressed(KeyCode::F2)) {
        on.0 = !on.0;
    }
    let want = if on.0 { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut panel {
        if *v != want {
            *v = want;
        }
    }
    if on.0 && last.is_none_or(|l| l.elapsed() >= REFRESH) {
        *last = Some(Instant::now());
        if let Some(s) = shared.latest()
            && let Ok(mut t) = text.single_mut()
        {
            t.0 = render(&s, cost.avg_us);
        }
    }
    cost.add(t0.elapsed());
}

fn opt(v: Option<f32>, unit: &str, prec: usize) -> String {
    v.map_or_else(|| "n/a".into(), |v| format!("{v:.prec$}{unit}"))
}

fn render(s: &PerfSnapshot, overhead_us: f32) -> String {
    let mut o = String::with_capacity(900);
    let f = &s.frame_ms;
    let _ = writeln!(o, "PERF  [F2]  {:.0} fps{}", s.fps, if s.stalled { "  (stalled)" } else { "" });
    let _ = writeln!(o, "frame  p50 {:.1}  p95 {:.1}  p99 {:.1}  max {:.1} ms", f.p50, f.p95, f.p99, f.max);
    let _ = writeln!(o, "ram    rss {}  footprint {}", opt(s.rss_mb, " MB", 0), opt(s.footprint_mb, " MB", 0));
    let _ = writeln!(o, "cpu    process {}  ({} cores)", opt(s.cpu_pct, "%", 0), s.cores);
    for t in s.threads.iter().take(6) {
        let _ = writeln!(o, "         {:<26} {:>5.1}%", truncate(&t.name, 26), t.cpu_pct);
    }
    let g = &s.gpu;
    let _ = writeln!(
        o,
        "gpu    process {}  device {}  frame {}",
        opt(g.process_util_pct, "%", 0),
        opt(g.device_util_pct, "%", 0),
        opt(g.frame_ms, " ms/frame", 2)
    );
    let _ = writeln!(
        o,
        "       mem {} (process)  {} (device)",
        opt(g.process_alloc_mb, " MB", 0),
        opt(g.device_in_use_mb, " MB", 0)
    );
    for p in g.passes.iter().take(3) {
        let _ = writeln!(o, "         {:<26} {:>5.2} ms", truncate(&p.name, 26), p.ms);
    }
    let m = &s.game;
    let _ = writeln!(o, "game   {}  {} entities", m.state, m.entities);
    let _ = writeln!(
        o,
        "       ai {:.2} ms (max {:.2})  physics {:.2}  weapons {:.2}",
        m.ai_ms, m.ai_max_ms, m.physics_ms, m.weapons_ms
    );
    let _ = write!(o, "monitor {overhead_us:.0} us/frame   hitches {}", s.hitches);
    o
}

fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}
