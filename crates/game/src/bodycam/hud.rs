//! Bodycam HUD (spec §4.3, adapted to the CS loop): a top-right camera stamp (blinking red
//! [REC] dot, local timestamp with UTC offset, device id), no crosshair, no health panel and no
//! ammo panel. Money, round timer, score, kill feed, freeze time and the buy menu stay: the
//! match loop needs them.
//!
//! Ammo check ("press check"): R with a full magazine (which reloads nothing) plays the inspect
//! turn and shows the ammo panel for [`PRESS_CHECK_S`]; it also shows while reloading.

use bevy::prelude::*;

use super::{bodycam_on, is_bodycam};
use crate::player::{Player, PlayerIntent};
use crate::settings::ShroudSettings;
use crate::state::{GameState, has_visuals};
use crate::ui::hud::KillFeed;
use crate::ui::theme::*;
use crate::weapon::{Weapon, WeaponSet};

/// How long a press check shows the ammo readout (s).
pub const PRESS_CHECK_S: f32 = 2.0;
/// Shown on the stamp (a fictional device; the spec's real body-camera brand is not used).
pub const DEVICE_ID: &str = "SHROUD BODY 4 // X84-9921";

/// The health panel (hidden in bodycam).
#[derive(Component)]
pub struct HealthPanel;
/// The gun / ammo panel (hidden in bodycam except during a press check).
#[derive(Component)]
pub struct AmmoPanel;

#[derive(Component)]
struct Stamp;
#[derive(Component)]
struct RecDot;
#[derive(Component)]
struct StampClock;

/// Seconds left on the ammo readout.
#[derive(Resource, Default)]
pub struct PressCheck(pub f32);

pub(super) fn build(app: &mut App) {
    app.init_resource::<PressCheck>()
        .add_systems(OnEnter(GameState::Playing), spawn_stamp.run_if(has_visuals).after(crate::match_flow::begin_round))
        .add_systems(
            Update,
            (press_check.run_if(bodycam_on).after(WeaponSet), strip_hud, update_stamp)
                .chain()
                .run_if(in_state(GameState::Playing).and_then(has_visuals)),
        );
}

fn spawn_stamp(mut commands: Commands) {
    let mono = label;
    commands
        .spawn((
            Name::new("Bodycam stamp"),
            Stamp,
            DespawnOnExit(GameState::Playing),
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                right: px(36),
                top: px(22),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: px(2),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|s| {
            s.spawn((Node { column_gap: px(8), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE)).with_children(|r| {
                r.spawn((
                    RecDot,
                    Node { width: px(12), height: px(12), border_radius: BorderRadius::MAX, ..default() },
                    BackgroundColor(Color::srgb(0.95, 0.12, 0.1)),
                    Pickable::IGNORE,
                ));
                r.spawn((Text::new("REC"), mono(20.0), TextColor(Color::srgb(0.95, 0.2, 0.16)), TextShadow { offset: Vec2::splat(1.0), ..default() }));
                r.spawn((StampClock, Text::new(""), mono(20.0), TextColor(Color::srgba(1.0, 1.0, 1.0, 0.92)), TextShadow { offset: Vec2::splat(1.0), ..default() }));
            });
            s.spawn((Text::new(DEVICE_ID), mono(16.0), TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)), TextShadow { offset: Vec2::splat(1.0), ..default() }));
        });
}

/// R on a full magazine (nothing to reload): inspect turn + ammo readout.
fn press_check(time: Res<Time>, intent: Res<PlayerIntent>, mut was: Local<bool>, mut check: ResMut<PressCheck>, mut q: Query<&mut Weapon, With<Player>>) {
    let pressed = intent.reload && !*was;
    *was = intent.reload;
    check.0 = (check.0 - time.delta_secs()).max(0.0);
    let Ok(mut w) = q.single_mut() else { return };
    if pressed && w.ammo >= w.mag && w.reloading.is_none() && w.ready() {
        check.0 = PRESS_CHECK_S;
        if w.inspect <= 0.0 {
            w.inspect = crate::weapon::arsenal::INSPECT_S * 0.6;
        }
    }
}

fn set_vis(v: &mut Visibility, want: Visibility) {
    if *v != want {
        *v = want;
    }
}

/// Bodycam hides the health and ammo panels (ammo comes back for a press check or a reload) and
/// moves the kill feed below the stamp.
fn strip_hud(
    settings: Res<ShroudSettings>,
    check: Res<PressCheck>,
    player: Query<&Weapon, With<Player>>,
    mut health: Query<&mut Visibility, (With<HealthPanel>, Without<AmmoPanel>)>,
    mut ammo: Query<&mut Visibility, (With<AmmoPanel>, Without<HealthPanel>)>,
    mut feed: Query<&mut Node, With<KillFeed>>,
) {
    let body = is_bodycam(&settings);
    let reloading = player.single().is_ok_and(|w| w.reloading.is_some());
    let hide = |on: bool| if on { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut health {
        set_vis(&mut v, hide(body));
    }
    for mut v in &mut ammo {
        set_vis(&mut v, hide(body && check.0 <= 0.0 && !reloading));
    }
    let top = px(if body { 84 } else { 56 });
    for mut n in &mut feed {
        if n.top != top {
            n.top = top;
        }
    }
}

fn update_stamp(
    time: Res<Time<Real>>,
    settings: Res<ShroudSettings>,
    paused: Res<crate::state::Paused>,
    mut stamp: Query<&mut Visibility, (With<Stamp>, Without<RecDot>)>,
    mut dot: Query<&mut Visibility, (With<RecDot>, Without<Stamp>)>,
    mut clock: Query<&mut Text, With<StampClock>>,
    mut last: Local<i64>,
) {
    let body = is_bodycam(&settings);
    for mut v in &mut stamp {
        set_vis(&mut v, if body && !paused.0 { Visibility::Inherited } else { Visibility::Hidden });
    }
    if !body {
        return;
    }
    // 1 Hz blink: on for 0.6 s.
    let on = time.elapsed_secs().fract() < 0.6;
    for mut v in &mut dot {
        set_vis(&mut v, if on { Visibility::Inherited } else { Visibility::Hidden });
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    if now != *last || clock.iter().any(|t| t.0.is_empty()) {
        *last = now;
        let s = local_stamp(now);
        for mut t in &mut clock {
            if t.0 != s {
                t.0 = s.clone();
            }
        }
    }
}

/// `YYYY-MM-DD HH:MM:SS +hhmm` in local time (UTC where the offset is unknown).
pub fn local_stamp(unix: i64) -> String {
    let offset = utc_offset_secs(unix);
    format_stamp(unix + offset, offset)
}

#[cfg(unix)]
fn utc_offset_secs(unix: i64) -> i64 {
    let t = unix as libc::time_t;
    // SAFETY: `localtime_r` writes only into `tm`, which outlives the call.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() { 0 } else { tm.tm_gmtoff as i64 }
    }
}

#[cfg(not(unix))]
fn utc_offset_secs(_unix: i64) -> i64 {
    0
}

/// Formats `local` (unix seconds shifted by `offset`) as `YYYY-MM-DD HH:MM:SS +hhmm`.
pub fn format_stamp(local: i64, offset: i64) -> String {
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let sign = if offset < 0 { '-' } else { '+' };
    let o = offset.abs() / 60;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} {sign}{:02}{:02}", secs / 3600, secs / 60 % 60, secs % 60, o / 60, o % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_format() {
        // 2026-11-03 08:52:05 UTC, shown at +05:30.
        let unix = 1_793_695_925;
        assert_eq!(format_stamp(unix, 0), "2026-11-03 08:52:05 +0000");
        assert_eq!(format_stamp(unix + 19_800, 19_800), "2026-11-03 14:22:05 +0530");
        assert_eq!(format_stamp(0, -18_000 + 0), "1970-01-01 00:00:00 -0500");
    }
}
