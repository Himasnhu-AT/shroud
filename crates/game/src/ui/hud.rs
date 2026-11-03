//! In-game HUD: crosshair + hit marker, health bar, round score + timer + enemy pips at the top,
//! kill feed, and round start / round over banners. The weapon panel, dynamic crosshair spread,
//! scope and pickup prompt are `ui::weapon_hud`.

use bevy::prelude::*;

use super::theme::*;
use crate::enemy::{Enemy, EnemyCombat, Squad};
use crate::match_flow::{MAX_ROUNDS, MatchFlow, ROUNDS_TO_WIN};
use crate::player::Player;
use crate::settings::ShroudSettings;
use crate::state::{GameState, has_visuals};
use crate::weapon::{Health, HitMarker, WeaponSet};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudInfo>()
            .init_resource::<RoundClock>()
            .add_systems(
                OnEnter(GameState::Playing),
                (reset_clock, (spawn_hud, round_start_banner).chain().run_if(has_visuals))
                    .after(crate::match_flow::begin_round)
                    .after(crate::enemy::spawn_squad),
            )
            .add_systems(
                Update,
                // (The kill feed lines are `ui::killfeed`'s, spawned into the `KillFeed` node.)
                (tick_clock, (update_hud, update_pips, round_over_banner).run_if(has_visuals))
                    .after(WeaponSet)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(Update, (style_crosshairs, animate_fx, hide_when_paused).run_if(has_visuals));
    }
}

/// Match info shown on the HUD; written by match flow / evolution code.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct HudInfo {
    pub generation: u32,
    pub enemies_left: u32,
    /// (player rounds won, squad rounds won).
    pub round_score: (u32, u32),
}

impl Default for HudInfo {
    fn default() -> Self {
        Self { generation: 1, enemies_left: 0, round_score: (0, 0) }
    }
}

/// Seconds since the round started (stops when the round is decided).
#[derive(Resource, Default)]
pub struct RoundClock(pub f32);

fn reset_clock(mut c: ResMut<RoundClock>) {
    c.0 = 0.0;
}

fn tick_clock(time: Res<Time>, flow: Res<MatchFlow>, phase: Res<crate::economy::RoundPhase>, mut c: ResMut<RoundClock>) {
    // Freeze time doesn't count against the round clock.
    if flow.round_over.is_none() && !phase.frozen() {
        c.0 += time.delta_secs();
    }
}

#[derive(Component)]
struct HpText;
#[derive(Component)]
struct HpFill;
#[derive(Component)]
struct HpGhost;
#[derive(Component)]
struct ScoreText(bool);
#[derive(Component)]
struct RoundText;
#[derive(Component)]
struct TimerText;
#[derive(Component)]
struct GenText;
#[derive(Component)]
struct Pips;
#[derive(Component)]
struct Pip(usize);
#[derive(Component)]
struct HitMarkerUi;
/// Top-right anchor for the kill feed (`ui::killfeed` fills it).
#[derive(Component)]
pub struct KillFeed;
#[derive(Component)]
struct RoundStartBanner;

/// A crosshair (HUD or the settings preview), styled from [`ShroudSettings`].
#[derive(Component)]
pub struct CrosshairUi;
#[derive(Component)]
pub struct CrosshairBar;

/// Fade/scale animation for banners and kill-feed lines.
#[derive(Component)]
struct Fx {
    t: f32,
    /// Total life in seconds; `None` = stays until the state ends.
    life: Option<f32>,
    fade_in: f32,
    fade_out: f32,
    from_scale: f32,
}

/// Base alpha of a node under an [`Fx`] (captured on its first frame).
#[derive(Component)]
struct FadeBase {
    text: Option<f32>,
    bg: Option<f32>,
}

const BAR_W: f32 = 300.0;

fn hud_root() -> impl Bundle {
    (DespawnOnExit(GameState::Playing), HideWhenPaused, fullscreen(), Pickable::IGNORE)
}

/// HUD layers hidden while the pause menu (or settings over it) is open.
#[derive(Component)]
pub struct HideWhenPaused;

fn hide_when_paused(paused: Res<crate::state::Paused>, mut q: Query<&mut Visibility, With<HideWhenPaused>>) {
    let want = if paused.0 { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

pub fn spawn_crosshair(p: &mut ChildSpawnerCommands, settings: &ShroudSettings) {
    let bar = |w: f32, h: f32, left: f32, top: f32| {
        (
            CrosshairBar,
            Node { position_type: PositionType::Absolute, width: px(w), height: px(h), left: px(left), top: px(top), ..default() },
            BackgroundColor(Color::WHITE),
            Outline::new(px(1), px(0), Color::srgba(0.0, 0.0, 0.0, 0.55)),
            Pickable::IGNORE,
        )
    };
    // 24×24 box centred on its parent; 2 px bars, 5 px gap from centre.
    p.spawn((
        CrosshairUi,
        Node { width: px(24), height: px(24), ..default() },
        UiTransform::from_scale(Vec2::splat(settings.crosshair_size)),
        Pickable::IGNORE,
    ))
    .with_children(|c| {
        c.spawn(bar(2.0, 7.0, 11.0, 0.0));
        c.spawn(bar(2.0, 7.0, 11.0, 17.0));
        c.spawn(bar(7.0, 2.0, 0.0, 11.0));
        c.spawn(bar(7.0, 2.0, 17.0, 11.0));
        c.spawn(bar(2.0, 2.0, 11.0, 11.0));
    });
}

fn style_crosshairs(
    settings: Res<ShroudSettings>,
    added: Query<(), Added<CrosshairUi>>,
    mut xs: Query<(&mut UiTransform, &Children), With<CrosshairUi>>,
    mut bars: Query<&mut BackgroundColor, With<CrosshairBar>>,
) {
    if !settings.is_changed() && added.is_empty() {
        return;
    }
    let [r, g, b] = settings.crosshair_color;
    for (mut tf, kids) in &mut xs {
        tf.scale = Vec2::splat(settings.crosshair_size);
        for k in kids.iter() {
            if let Ok(mut bg) = bars.get_mut(k) {
                bg.0 = Color::srgb(r, g, b);
            }
        }
    }
}

fn spawn_hud(mut commands: Commands, settings: Res<ShroudSettings>, squad: Res<Squad>) {
    let root = commands.spawn((Name::new("Hud"), hud_root())).id();

    // Crosshair + hit marker, centred.
    commands
        .spawn((
            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_children(|c| {
            spawn_crosshair(c, &settings);
            c.spawn((
                HitMarkerUi,
                Node { position_type: PositionType::Absolute, width: px(36), height: px(36), ..default() },
                UiTransform::IDENTITY,
                Visibility::Hidden,
                Pickable::IGNORE,
            ))
            .with_children(|m| {
                // Four diagonal ticks around the centre.
                for (x, y, deg) in [(6.0, 6.0, 45.0), (22.0, 6.0, -45.0), (6.0, 22.0, -45.0), (22.0, 22.0, 45.0)] {
                    m.spawn((
                        Node { position_type: PositionType::Absolute, left: px(x - 1.0), top: px(y - 5.0), width: px(3), height: px(11), ..default() },
                        UiTransform::from_rotation(Rot2::degrees(deg)),
                        BackgroundColor(Color::WHITE),
                        Pickable::IGNORE,
                    ));
                }
            });
        });

    // Top centre: score, round, timer, enemy pips.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(26),
                left: px(0),
                right: px(0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(10),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_children(|top| {
            top.spawn((Node { align_items: AlignItems::Stretch, ..default() }, Pickable::IGNORE)).with_children(|row| {
                score_box(row, true);
                row.spawn((
                    Node {
                        width: px(180),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        padding: UiRect::vertical(px(6)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.72)),
                    Pickable::IGNORE,
                ))
                .with_children(|mid| {
                    mid.spawn((RoundText, text("ROUND 1", label(SIZE_TINY + 1.0), TEXT_MUTED)));
                    mid.spawn((TimerText, text("0:00", display(34.0), TEXT)));
                });
                score_box(row, false);
            });
            top.spawn((Pips, Node { column_gap: px(6), ..default() }, Pickable::IGNORE)).with_children(|p| {
                for i in 0..squad.size.max(1) {
                    p.spawn((
                        Pip(i),
                        Node { width: px(14), height: px(14), border: UiRect::all(px(2)), border_radius: BorderRadius::all(px(2)), ..default() },
                        UiTransform::from_rotation(Rot2::degrees(45.0)),
                        BackgroundColor(ENEMY),
                        BorderColor::all(ENEMY),
                        Pickable::IGNORE,
                    ));
                }
            });
        });

    // Top-left: squad generation + difficulty.
    commands.spawn((
        GenText,
        text("", label(SIZE_LABEL), TEXT.with_alpha(0.85)),
        Node { position_type: PositionType::Absolute, left: px(40), top: px(30), padding: UiRect::axes(px(10), px(4)), ..default() },
        BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.55)),
        ChildOf(root),
    ));

    // Top-right: kill feed (below the optional FPS counter).
    commands.spawn((
        KillFeed,
        Node {
            position_type: PositionType::Absolute,
            right: px(40),
            top: px(56),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexEnd,
            row_gap: px(6),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(root),
    ));

    // Bottom-left: health.
    commands
        .spawn((
            crate::bodycam::hud::HealthPanel,
            Node {
                position_type: PositionType::Absolute,
                left: px(40),
                bottom: px(40),
                align_items: AlignItems::FlexEnd,
                column_gap: px(GAP_M),
                padding: UiRect::new(px(18), px(22), px(10), px(14)),
                ..default()
            },
            hud_panel_gradient(true),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_children(|h| {
            h.spawn((HpText, text("100", display(64.0), TEXT), Node { margin: UiRect::bottom(px(-10)), ..default() }));
            h.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(8), padding: UiRect::bottom(px(4)), ..default() }, Pickable::IGNORE))
                .with_children(|c| {
                    c.spawn(text("HEALTH", label(SIZE_TINY + 1.0), TEXT_MUTED));
                    c.spawn((
                        Node { width: px(BAR_W), height: px(10), ..default() },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.10)),
                        Pickable::IGNORE,
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            HpGhost,
                            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
                            BackgroundColor(Color::srgba(1.0, 0.85, 0.8, 0.55)),
                            Pickable::IGNORE,
                        ));
                        bar.spawn((
                            HpFill,
                            Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
                            BackgroundColor(FRIENDLY),
                            Pickable::IGNORE,
                        ));
                        // Quarter ticks.
                        for q in 1..4 {
                            bar.spawn((
                                Node { position_type: PositionType::Absolute, left: percent(q as f32 * 25.0), width: px(2), height: percent(100), ..default() },
                                BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.9)),
                                Pickable::IGNORE,
                            ));
                        }
                    });
                });
        });

}

/// Dark wash behind a bottom HUD block, fading towards the screen centre.
pub fn hud_panel_gradient(left: bool) -> BackgroundGradient {
    let (a, b) = (Color::srgba(0.02, 0.03, 0.05, 0.62), Color::srgba(0.02, 0.03, 0.05, 0.0));
    BackgroundGradient::from(LinearGradient::new(
        if left { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 },
        vec![ColorStop::new(a, percent(0)), ColorStop::new(a, percent(55)), ColorStop::new(b, percent(100))],
    ))
}

fn score_box(p: &mut ChildSpawnerCommands, player: bool) {
    let color = if player { FRIENDLY } else { ENEMY };
    p.spawn((
        Node {
            width: px(112),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::vertical(px(4)),
            border: UiRect::bottom(px(3)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.6)),
        BorderColor::all(color),
        Pickable::IGNORE,
    ))
    .with_children(|b| {
        b.spawn(text(if player { "YOU" } else { "SQUAD" }, label(SIZE_TINY), color.with_alpha(0.85)));
        b.spawn((ScoreText(player), text("0", display(40.0), TEXT)));
    });
}

fn fmt_clock(s: f32) -> String {
    let s = s.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

fn update_hud(
    time: Res<Time>,
    info: Res<HudInfo>,
    flow: Res<MatchFlow>,
    clock: Res<RoundClock>,
    phase: Res<crate::economy::RoundPhase>,
    settings: Res<ShroudSettings>,
    marker: Res<HitMarker>,
    player: Query<&Health, With<Player>>,
    mut texts: ParamSet<(
        Query<(&mut Text, &mut TextColor), With<HpText>>,
        Query<(&mut Text, &ScoreText)>,
        Query<&mut Text, With<RoundText>>,
        Query<(&mut Text, &mut TextColor), With<TimerText>>,
        Query<&mut Text, With<GenText>>,
    )>,
    mut bars: ParamSet<(
        Query<(&mut Node, &mut BackgroundColor), With<HpFill>>,
        Query<&mut Node, With<HpGhost>>,
    )>,
    mut hit: Query<(&mut Visibility, &mut UiTransform, &Children), With<HitMarkerUi>>,
    mut hit_bars: Query<&mut BackgroundColor, Without<HpFill>>,
    mut ghost: Local<f32>,
) {
    fn set(t: &mut Text, s: String) {
        if t.0 != s {
            t.0 = s;
        }
    }
    if let Ok(hp) = player.single() {
        let frac = (hp.hp / hp.max).clamp(0.0, 1.0);
        let low = frac < crate::audio::LOW_HP_FRAC;
        if let Ok((mut t, mut c)) = texts.p0().single_mut() {
            set(&mut t, format!("{}", hp.hp.max(0.0).ceil() as i32));
            c.0 = if low { ENEMY } else { TEXT };
        }
        // The ghost bar trails the real one, showing recent damage.
        *ghost = if frac >= *ghost { frac } else { (*ghost - time.delta_secs() * 0.6).max(frac) };
        if let Ok((mut n, mut bg)) = bars.p0().single_mut() {
            n.width = percent(frac * 100.0);
            bg.0 = if low { ENEMY } else { FRIENDLY };
        }
        if let Ok(mut n) = bars.p1().single_mut() {
            n.width = percent(*ghost * 100.0);
        }
    }
    for (mut t, s) in &mut texts.p1() {
        set(&mut t, if s.0 { info.round_score.0 } else { info.round_score.1 }.to_string());
    }
    if let Ok(mut t) = texts.p2().single_mut() {
        let mp = flow.player_rounds + 1 >= ROUNDS_TO_WIN || flow.enemy_rounds + 1 >= ROUNDS_TO_WIN;
        let round = format!("ROUND {}/{MAX_ROUNDS}", flow.round.max(1));
        set(
            &mut t,
            if phase.frozen() {
                "FREEZE TIME".to_string()
            } else if mp && flow.round > 1 {
                format!("{round} \u{b7} MATCH POINT")
            } else {
                round
            },
        );
    }
    if let Ok((mut t, mut c)) = texts.p3().single_mut() {
        // Freeze countdown, then the round time limit.
        let (s, col) = if phase.frozen() { (fmt_clock(phase.freeze_left.ceil()), ACCENT) } else { (fmt_clock(crate::match_flow::ROUND_TIME_S - clock.0), TEXT) };
        set(&mut t, s);
        if c.0 != col {
            c.0 = col;
        }
    }
    if let Ok(mut t) = texts.p4().single_mut() {
        set(&mut t, format!("SQUAD GEN {}  \u{b7}  {}", info.generation, settings.difficulty.label().to_uppercase()));
    }
    if let Ok((mut vis, mut tf, kids)) = hit.single_mut() {
        let show = marker.timer > 0.0;
        let want = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if show {
            let (color, scale, alpha) = hit_marker_look(&marker);
            let scale = Vec2::splat(scale);
            if tf.scale != scale {
                tf.scale = scale;
            }
            let color = color.with_alpha(alpha);
            for k in kids.iter() {
                if let Ok(mut bg) = hit_bars.get_mut(k)
                    && bg.0 != color
                {
                    bg.0 = color;
                }
            }
        }
    }
}

/// The hit marker's colour (white, headshot accent, kill red), scale and alpha: it punches
/// out on the hit, settles, and fades over the last 40% of its time.
fn hit_marker_look(m: &HitMarker) -> (Color, f32, f32) {
    let life = if m.kill { crate::weapon::KILL_MARKER_S } else { crate::weapon::HIT_MARKER_S };
    let k = (m.timer / life).clamp(0.0, 1.0);
    let color = if m.kill {
        ENEMY
    } else if m.headshot {
        ACCENT
    } else {
        Color::WHITE
    };
    let base = if m.kill { 1.35 } else { 1.0 };
    let pop = 1.0 + 0.4 * (k - 0.6).max(0.0) / 0.4;
    (color, base * pop, (k / 0.4).min(1.0))
}

fn update_pips(
    enemies: Query<(&Enemy, &EnemyCombat)>,
    mut pips: Query<(&Pip, &mut BackgroundColor, &mut BorderColor)>,
) {
    let dead: Vec<usize> = enemies.iter().filter(|(_, c)| c.dead).map(|(e, _)| e.slot).collect();
    for (pip, mut bg, mut border) in &mut pips {
        let is_dead = dead.contains(&pip.0);
        let (b, o) = if is_dead { (Color::NONE, Color::srgba(1.0, 1.0, 1.0, 0.25)) } else { (ENEMY, ENEMY) };
        if bg.0 != b {
            bg.0 = b;
            *border = BorderColor::all(o);
        }
    }
}

fn banner_root(c: &mut Commands, life: Option<f32>, top_pct: f32) -> Entity {
    let band = Color::srgba(0.02, 0.03, 0.05, 0.72);
    c.spawn((
        DespawnOnExit(GameState::Playing),
        HideWhenPaused,
        Fx { t: 0.0, life, fade_in: 0.28, fade_out: 0.5, from_scale: 1.12 },
        Node {
            position_type: PositionType::Absolute,
            top: percent(top_pct),
            left: px(0),
            right: px(0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(4),
            padding: UiRect::vertical(px(22)),
            ..default()
        },
        // A dark band, fading out to the sides, so the banner reads over a bright sky.
        BackgroundGradient::from(LinearGradient::new(
            std::f32::consts::FRAC_PI_2,
            vec![
                ColorStop::new(band.with_alpha(0.0), percent(0)),
                ColorStop::new(band, percent(30)),
                ColorStop::new(band, percent(70)),
                ColorStop::new(band.with_alpha(0.0), percent(100)),
            ],
        )),
        UiTransform::IDENTITY,
        GlobalZIndex(8),
        Pickable::IGNORE,
    ))
    .id()
}

fn round_start_banner(mut commands: Commands, flow: Res<MatchFlow>, settings: Res<ShroudSettings>) {
    let root = banner_root(&mut commands, Some(2.6), 22.0);
    commands.entity(root).insert(RoundStartBanner);
    let mp = flow.round > 1 && (flow.player_rounds + 1 >= ROUNDS_TO_WIN || flow.enemy_rounds + 1 >= ROUNDS_TO_WIN);
    let sub = if mp {
        "MATCH POINT".to_string()
    } else {
        format!("{}  \u{b7}  {}  \u{b7}  FIRST TO {ROUNDS_TO_WIN}", settings.map.label().to_uppercase(), settings.difficulty.label().to_uppercase())
    };
    commands.entity(root).with_children(|b| {
        b.spawn(text(sub, label(SIZE_LABEL + 2.0), if mp { ACCENT } else { TEXT_MUTED }));
        b.spawn((text(format!("ROUND {}", flow.round.max(1)), display(88.0), TEXT), TextShadow::default()));
        b.spawn((Node { width: px(120), height: px(3), ..default() }, BackgroundColor(ACCENT), Pickable::IGNORE));
    });
}

fn round_over_banner(
    mut commands: Commands,
    flow: Res<MatchFlow>,
    start_banners: Query<Entity, With<RoundStartBanner>>,
    mut shown: Local<bool>,
) {
    let Some((_, won)) = flow.round_over else {
        *shown = false;
        return;
    };
    if *shown {
        return;
    }
    *shown = true;
    for e in &start_banners {
        commands.entity(e).despawn();
    }
    let match_over = flow.player_rounds >= ROUNDS_TO_WIN || flow.enemy_rounds >= ROUNDS_TO_WIN;
    let color = if won { FRIENDLY } else { ENEMY };
    let title = match (match_over, won) {
        (true, true) => "VICTORY",
        (true, false) => "DEFEAT",
        (false, true) => "ROUND WON",
        (false, false) => "ROUND LOST",
    };
    let sub = if match_over { "The squad is studying what you did" } else { "Next round starting" };
    let root = banner_root(&mut commands, None, 22.0);
    commands.entity(root).with_children(|b| {
        b.spawn((text(title, display(104.0), color), TextShadow::default()));
        b.spawn((Node { align_items: AlignItems::Center, column_gap: px(GAP_L), ..default() }, Pickable::IGNORE)).with_children(|r| {
            r.spawn(text("YOU", label(SIZE_LABEL + 2.0), TEXT_MUTED));
            r.spawn(text(flow.player_rounds.to_string(), display(56.0), FRIENDLY));
            r.spawn((Node { width: px(24), height: px(3), ..default() }, BackgroundColor(TEXT_MUTED), Pickable::IGNORE));
            r.spawn(text(flow.enemy_rounds.to_string(), display(56.0), ENEMY));
            r.spawn(text("SQUAD", label(SIZE_LABEL + 2.0), TEXT_MUTED));
        });
        b.spawn(text(sub, body(SIZE_BODY), TEXT_MUTED));
    });
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn animate_fx(
    mut commands: Commands,
    time: Res<Time>,
    mut fx: Query<(Entity, &mut Fx, &mut UiTransform)>,
    children: Query<&Children>,
    mut nodes: Query<(Option<&FadeBase>, Option<&mut TextColor>, Option<&mut BackgroundColor>)>,
) {
    for (e, mut f, mut tf) in &mut fx {
        let first = f.t == 0.0;
        f.t += time.delta_secs();
        let a_in = if f.fade_in > 0.0 { ease_out(f.t / f.fade_in) } else { 1.0 };
        let a_out = match f.life {
            Some(l) if f.fade_out > 0.0 => ((l - f.t) / f.fade_out).clamp(0.0, 1.0),
            _ => 1.0,
        };
        if f.life.is_some_and(|l| f.t >= l) {
            commands.entity(e).despawn();
            continue;
        }
        let alpha = a_in * a_out;
        tf.scale = Vec2::splat(f.from_scale + (1.0 - f.from_scale) * a_in);
        for n in std::iter::once(e).chain(children.iter_descendants(e)) {
            let Ok((base, text, bg)) = nodes.get_mut(n) else { continue };
            match base {
                Some(b) => {
                    let (bt, bb) = (b.text, b.bg);
                    if let (Some(mut t), Some(a)) = (text, bt) {
                        t.0.set_alpha(a * alpha);
                    }
                    if let (Some(mut c), Some(a)) = (bg, bb) {
                        c.0.set_alpha(a * alpha);
                    }
                }
                None if first || f.t < 0.1 => {
                    let tb = text.map(|mut t| {
                        let a = t.0.alpha();
                        t.0.set_alpha(0.0);
                        a
                    });
                    let bb = bg.map(|mut c| {
                        let a = c.0.alpha();
                        c.0.set_alpha(0.0);
                        a
                    });
                    commands.entity(n).insert(FadeBase { text: tb, bg: bb });
                }
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ENEMY, HitMarker, hit_marker_look};

    #[test]
    fn hit_marker_pops_then_fades() {
        let hit = |t: f32, kill: bool| hit_marker_look(&HitMarker { timer: t, headshot: false, kill });
        let fresh = hit(crate::weapon::HIT_MARKER_S, false);
        let mid = hit(crate::weapon::HIT_MARKER_S * 0.5, false);
        let end = hit(crate::weapon::HIT_MARKER_S * 0.1, false);
        assert!(fresh.1 > 1.3 && (mid.1 - 1.0).abs() < 1e-5, "pops out then settles: {} {}", fresh.1, mid.1);
        assert!(fresh.2 == 1.0 && mid.2 == 1.0 && end.2 < 0.5, "fades at the end: {}", end.2);
        let kill = hit(crate::weapon::KILL_MARKER_S, true);
        assert!(kill.1 > fresh.1 && kill.0 == ENEMY, "kills are bigger and red");
    }

    #[test]
    fn clock_format() {
        assert_eq!(super::fmt_clock(0.0), "0:00");
        assert_eq!(super::fmt_clock(83.9), "1:23");
    }
}
