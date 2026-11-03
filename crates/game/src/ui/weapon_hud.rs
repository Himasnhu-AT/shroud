//! Weapon HUD: the panel bottom right (gun class + name, a silhouette icon, mag / reserve, fire
//! mode, slots, mag pips or reload progress), a low-ammo / reload warning under the crosshair,
//! the crosshair opening with the real spread cone (hidden aimed and sprinting), the Longbow's
//! scope overlay and the "hold E" pickup prompt. Text is only rewritten when its value changes.

use bevy::prelude::*;
use bevy::ui::{RadialGradient, RadialGradientShape, UiPosition};
use brain::weapons::FireMode;
use engine::gfx::MainCamera;

use super::hud::{CrosshairBar, CrosshairUi, HideWhenPaused, hud_panel_gradient};
use super::theme::*;
use crate::player::Player;
use crate::settings::ShroudSettings;
use crate::state::{GameState, has_visuals};
use crate::weapon::arsenal::{Optic, ReloadKind, Weapon};
use crate::weapon::pickups::PickupFocus;
use crate::weapon::{Arsenal, WeaponKind, WeaponSet, class_label};

pub struct WeaponHudPlugin;

impl Plugin for WeaponHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), spawn.run_if(has_visuals).after(crate::match_flow::begin_round)).add_systems(
            Update,
            (update_panel, crosshair_spread, scope_overlay, pickup_prompt).after(WeaponSet).run_if(in_state(GameState::Playing).and_then(has_visuals)),
        );
    }
}

#[derive(Component)]
struct GunName;
#[derive(Component)]
struct GunClass;
#[derive(Component)]
struct MagText;
#[derive(Component)]
struct ReserveText;
#[derive(Component)]
struct ModeText;
#[derive(Component)]
struct SlotText(usize);
#[derive(Component)]
struct Icon;
#[derive(Component)]
struct Pips;
#[derive(Component)]
struct Pip(u32);
#[derive(Component)]
struct ReloadBar;
#[derive(Component)]
struct ReloadFill;
#[derive(Component)]
struct Warning;
#[derive(Component)]
struct Scope;
#[derive(Component)]
struct ReflexSight;
#[derive(Component)]
struct Prompt;
#[derive(Component)]
struct PromptText;
#[derive(Component)]
struct PromptFill;

fn root() -> impl Bundle {
    (Name::new("Weapon HUD"), DespawnOnExit(GameState::Playing), HideWhenPaused, fullscreen(), Pickable::IGNORE)
}

fn mode_label(k: WeaponKind) -> &'static str {
    match (k.spec().mode, k) {
        (FireMode::Auto, _) => "AUTO",
        (FireMode::Semi, _) => "SEMI",
        (FireMode::Single, WeaponKind::Shotgun) => "PUMP",
        (FireMode::Single, _) => "BOLT",
    }
}

/// Side-view silhouette from blocks: (x, y, w, h) in a 96×34 box.
fn icon_parts(k: WeaponKind) -> &'static [(f32, f32, f32, f32)] {
    match k {
        WeaponKind::Pistol => &[(30.0, 8.0, 40.0, 10.0), (58.0, 16.0, 10.0, 16.0), (26.0, 10.0, 6.0, 4.0)],
        WeaponKind::Smg => &[(18.0, 8.0, 56.0, 12.0), (8.0, 11.0, 12.0, 4.0), (40.0, 20.0, 8.0, 14.0), (62.0, 20.0, 9.0, 12.0), (74.0, 10.0, 16.0, 6.0)],
        WeaponKind::Rifle => &[(2.0, 12.0, 30.0, 4.0), (30.0, 8.0, 42.0, 12.0), (44.0, 20.0, 8.0, 12.0), (60.0, 20.0, 8.0, 12.0), (72.0, 9.0, 22.0, 9.0), (38.0, 3.0, 22.0, 5.0)],
        WeaponKind::Shotgun => &[(2.0, 9.0, 52.0, 6.0), (10.0, 16.0, 30.0, 6.0), (52.0, 8.0, 22.0, 12.0), (62.0, 20.0, 8.0, 12.0), (72.0, 10.0, 22.0, 10.0)],
        WeaponKind::Marksman => &[(0.0, 13.0, 36.0, 3.0), (34.0, 9.0, 34.0, 11.0), (40.0, 1.0, 26.0, 6.0), (50.0, 20.0, 7.0, 11.0), (66.0, 20.0, 8.0, 12.0), (74.0, 9.0, 22.0, 10.0)],
        WeaponKind::HeavyPistol => &[(26.0, 7.0, 46.0, 12.0), (60.0, 17.0, 11.0, 16.0), (22.0, 9.0, 6.0, 5.0), (34.0, 4.0, 6.0, 3.0)],
        WeaponKind::BattleRifle => &[(0.0, 12.0, 28.0, 4.0), (26.0, 8.0, 44.0, 12.0), (40.0, 20.0, 9.0, 13.0), (58.0, 20.0, 8.0, 12.0), (70.0, 10.0, 24.0, 10.0), (30.0, 5.0, 12.0, 3.0)],
        WeaponKind::Sniper => &[(0.0, 12.0, 40.0, 3.0), (38.0, 9.0, 32.0, 10.0), (36.0, 0.0, 32.0, 7.0), (52.0, 19.0, 7.0, 11.0), (64.0, 19.0, 8.0, 13.0), (72.0, 8.0, 24.0, 13.0), (44.0, 6.0, 4.0, 3.0)],
    }
}

fn spawn(mut commands: Commands) {
    let root = commands.spawn(root()).id();

    // Bottom right: the gun panel.
    commands
        .spawn((
            crate::bodycam::hud::AmmoPanel,
            Node {
                position_type: PositionType::Absolute,
                right: px(40),
                bottom: px(40),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: px(6),
                padding: UiRect::new(px(22), px(18), px(10), px(14)),
                ..default()
            },
            hud_panel_gradient(false),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_children(|a| {
            a.spawn((Node { column_gap: px(GAP_M), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE)).with_children(|r| {
                r.spawn((Icon, Node { width: px(96), height: px(34), ..default() }, Pickable::IGNORE));
                r.spawn((Node { flex_direction: FlexDirection::Column, align_items: AlignItems::FlexEnd, ..default() }, Pickable::IGNORE)).with_children(|c| {
                    c.spawn((GunClass, text("RIFLE", label(SIZE_TINY), TEXT_MUTED)));
                    c.spawn((GunName, text("KESTREL", label(SIZE_LABEL + 2.0), TEXT)));
                });
            });
            a.spawn((Node { align_items: AlignItems::Baseline, column_gap: px(GAP_S), ..default() }, Pickable::IGNORE)).with_children(|r| {
                r.spawn((ModeText, text("AUTO", label(SIZE_TINY), ACCENT), Node { margin: UiRect::right(px(6)), ..default() }));
                r.spawn((MagText, text("30", display(64.0), TEXT), Node { margin: UiRect::bottom(px(-10)), ..default() }));
                r.spawn((ReserveText, text("/ 90", display(26.0), TEXT_MUTED)));
            });
            a.spawn((Pips, Node { column_gap: px(2), height: px(10), ..default() }, Pickable::IGNORE));
            a.spawn((
                ReloadBar,
                Node { width: px(180), height: px(10), display: Display::None, ..default() },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.1)),
                Pickable::IGNORE,
            ))
            .with_child((ReloadFill, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(ACCENT), Pickable::IGNORE));
            a.spawn((Node { column_gap: px(GAP_M), ..default() }, Pickable::IGNORE)).with_children(|r| {
                for i in 0..2 {
                    r.spawn((SlotText(i), text("", label(SIZE_TINY), TEXT_DIM)));
                }
            });
        });

    // Under the crosshair: low ammo / reload warning, then the pickup prompt.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                top: percent(56),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(10),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_children(|c| {
            c.spawn((
                Warning,
                text("", label(SIZE_LABEL), ACCENT),
                Node { padding: UiRect::axes(px(10), px(3)), ..default() },
                BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.55)),
                Visibility::Hidden,
            ));
            c.spawn((
                Prompt,
                Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, row_gap: px(4), padding: UiRect::axes(px(12), px(6)), ..default() },
                BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.6)),
                Visibility::Hidden,
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                p.spawn((PromptText, text("", label(SIZE_LABEL), TEXT)));
                p.spawn((Node { width: px(160), height: px(4), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.12)), Pickable::IGNORE))
                    .with_child((PromptFill, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(ACCENT), Pickable::IGNORE));
            });
        });

    // Scope: black outside a circle, thin reticle inside.
    let dark = Color::srgba(0.0, 0.0, 0.0, 1.0);
    let line = Color::srgba(0.02, 0.02, 0.02, 0.9);
    let mask = || {
        BackgroundGradient::from(RadialGradient::new(
            UiPosition::CENTER,
            RadialGradientShape::ClosestSide,
            vec![
                ColorStop::new(Color::NONE, percent(0)),
                ColorStop::new(Color::NONE, percent(82)),
                ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.6), percent(89)),
                ColorStop::new(dark, percent(92)),
                ColorStop::new(dark, percent(100)),
            ],
        ))
    };
    let full = || Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() };
    let overlay = || Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() };
    // Overlays sit under their siblings (ZIndex -1) so the ammo panel and the warnings stay
    // readable while aimed.
    commands
        .spawn((Scope, overlay(), mask(), ZIndex(-1), Visibility::Hidden, Pickable::IGNORE, ChildOf(root)))
        .with_children(|s| {
            s.spawn((full(), mask(), Pickable::IGNORE));
            s.spawn((full(), mask(), Pickable::IGNORE));
            let bar = |w: Val, h: Val| (Node { position_type: PositionType::Absolute, width: w, height: h, ..default() }, BackgroundColor(line), Pickable::IGNORE);
            // Full-width and full-height hairlines with thick outer posts.
            s.spawn(bar(percent(100), px(1)));
            s.spawn(bar(px(1), percent(100)));
            s.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: px(4), ..default() }, Pickable::IGNORE)).with_children(|r| {
                r.spawn((Node { width: percent(38), height: percent(100), ..default() }, BackgroundColor(line)));
                r.spawn(Node { width: percent(24), ..default() });
                r.spawn((Node { width: percent(38), height: percent(100), ..default() }, BackgroundColor(line)));
            });
            s.spawn((
                Node { position_type: PositionType::Absolute, height: percent(100), width: px(4), flex_direction: FlexDirection::Column, ..default() },
                Pickable::IGNORE,
            ))
            .with_children(|r| {
                r.spawn(Node { height: percent(62), ..default() });
                r.spawn((Node { height: percent(38), width: percent(100), ..default() }, BackgroundColor(line)));
            });
            // Centre dot and range ticks.
            s.spawn((Node { position_type: PositionType::Absolute, width: px(4), height: px(4), ..default() }, BackgroundColor(Color::srgb(0.9, 0.15, 0.1)), Pickable::IGNORE));
            for i in 1..=4 {
                s.spawn((
                    Node { position_type: PositionType::Absolute, width: px(14 - 2 * i), height: px(2), top: percent(50.0 + i as f32 * 3.0), ..default() },
                    BackgroundColor(line),
                    Pickable::IGNORE,
                ));
            }
        });

    // Kestrel optic: a wide eyepiece (dark housing, the world still readable at the rim), a
    // red dot in a thin ring, short stadia either side.
    let red = Color::srgba(1.0, 0.16, 0.1, 0.95);
    let housing = BackgroundGradient::from(RadialGradient::new(
        UiPosition::CENTER,
        RadialGradientShape::ClosestSide,
        vec![
            ColorStop::new(Color::NONE, percent(0)),
            ColorStop::new(Color::NONE, percent(84)),
            ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.35), percent(88)),
            ColorStop::new(Color::srgba(0.02, 0.02, 0.025, 0.97), percent(92)),
            ColorStop::new(Color::srgba(0.02, 0.02, 0.025, 0.93), percent(100)),
        ],
    ));
    commands
        .spawn((ReflexSight, overlay(), housing, ZIndex(-1), Visibility::Hidden, Pickable::IGNORE, ChildOf(root)))
        .with_children(|s| {
            s.spawn((
                Node { position_type: PositionType::Absolute, width: px(5), height: px(5), border_radius: BorderRadius::MAX, ..default() },
                BackgroundColor(red),
                Pickable::IGNORE,
            ));
            s.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: px(66),
                    height: px(66),
                    border: UiRect::all(px(2)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BorderColor::all(red.with_alpha(0.8)),
                Pickable::IGNORE,
            ));
            // Stadia: a gap, then a short bar each side (row), and one below (column).
            s.spawn((Node { position_type: PositionType::Absolute, column_gap: px(120), ..default() }, Pickable::IGNORE)).with_children(|r| {
                for _ in 0..2 {
                    r.spawn((Node { width: px(36), height: px(2), ..default() }, BackgroundColor(red.with_alpha(0.7)), Pickable::IGNORE));
                }
            });
            s.spawn((Node { position_type: PositionType::Absolute, top: percent(50), margin: UiRect::top(px(60)), width: px(2), height: px(30), ..default() }, BackgroundColor(red.with_alpha(0.7)), Pickable::IGNORE));
        });
}

#[derive(Default, PartialEq, Clone)]
struct Shown {
    kind: Option<WeaponKind>,
    ammo: u32,
    reserve: u32,
    slots: [Option<WeaponKind>; 2],
    active: usize,
    reloading: bool,
}

fn update_panel(
    mut commands: Commands,
    time: Res<Time>,
    player: Query<(&Weapon, &Arsenal), With<Player>>,
    mut texts: ParamSet<(
        Query<&mut Text, With<GunName>>,
        Query<&mut Text, With<GunClass>>,
        Query<(&mut Text, &mut TextColor), With<MagText>>,
        Query<&mut Text, With<ReserveText>>,
        Query<&mut Text, With<ModeText>>,
        Query<(&mut Text, &mut TextColor, &SlotText)>,
        Query<(&mut Text, &mut TextColor, &mut Visibility), With<Warning>>,
    )>,
    icons: Query<(Entity, Option<&Children>), With<Icon>>,
    pips_root: Query<(Entity, Option<&Children>), With<Pips>>,
    mut pips: Query<(&Pip, &mut BackgroundColor)>,
    mut nodes: ParamSet<(Query<&mut Node, With<Pips>>, Query<&mut Node, With<ReloadBar>>, Query<&mut Node, With<ReloadFill>>)>,
    fresh: Query<(), Added<MagText>>,
    mut shown: Local<Shown>,
) {
    let Ok((w, a)) = player.single() else { return };
    if !fresh.is_empty() {
        // A new HUD (new round): write everything.
        *shown = Shown::default();
    }
    let now = Shown {
        kind: Some(w.kind),
        ammo: w.ammo,
        reserve: w.reserve,
        slots: [a.slots[0].map(|g| g.kind), a.slots[1].map(|g| g.kind)],
        active: a.active,
        reloading: w.reloading.is_some(),
    };
    let kind_changed = shown.kind != now.kind;
    if kind_changed {
        if let Ok(mut t) = texts.p0().single_mut() {
            t.0 = w.kind.label().to_uppercase();
        }
        if let Ok(mut t) = texts.p1().single_mut() {
            t.0 = class_label(w.kind).into();
        }
        if let Ok(mut t) = texts.p4().single_mut() {
            t.0 = mode_label(w.kind).into();
        }
        // Rebuild the icon and the pips for the new gun.
        if let Ok((icon, kids)) = icons.single() {
            for k in kids.into_iter().flatten() {
                commands.entity(*k).despawn();
            }
            for &(x, y, wd, h) in icon_parts(w.kind) {
                commands.spawn((
                    Node { position_type: PositionType::Absolute, left: px(x), top: px(y), width: px(wd), height: px(h), ..default() },
                    BackgroundColor(TEXT.with_alpha(0.85)),
                    Pickable::IGNORE,
                    ChildOf(icon),
                ));
            }
        }
        if let Ok((root, kids)) = pips_root.single() {
            for k in kids.into_iter().flatten() {
                commands.entity(*k).despawn();
            }
            let wide = if w.mag <= 8 { 14.0 } else if w.mag <= 16 { 8.0 } else { 4.0 };
            for i in 0..w.mag {
                commands.spawn((Pip(i), Node { width: px(wide), height: px(10), ..default() }, BackgroundColor(TEXT), Pickable::IGNORE, ChildOf(root)));
            }
        }
    }
    let low = w.ammo * 4 <= w.mag;
    if (kind_changed || shown.ammo != now.ammo)
        && let Ok((mut t, mut c)) = texts.p2().single_mut()
    {
        t.0 = w.ammo.to_string();
        c.0 = if w.ammo == 0 { ENEMY } else if low { ACCENT } else { TEXT };
    }
    for (pip, mut bg) in &mut pips {
        let c = if pip.0 < w.ammo { TEXT } else { Color::srgba(1.0, 1.0, 1.0, 0.14) };
        if bg.0 != c {
            bg.0 = c;
        }
    }
    if (kind_changed || shown.reserve != now.reserve)
        && let Ok(mut t) = texts.p3().single_mut()
    {
        t.0 = format!("/ {}", w.reserve);
    }
    if kind_changed || shown.slots != now.slots || shown.active != now.active {
        for (mut t, mut c, s) in &mut texts.p5() {
            let label = a.slots[s.0].map_or(String::new(), |g| format!("{}  {}", s.0 + 1, g.kind.label().to_uppercase()));
            if t.0 != label {
                t.0 = label;
            }
            let col = if s.0 == a.active { TEXT } else { TEXT_DIM };
            if c.0 != col {
                c.0 = col;
            }
        }
    }

    // Reload bar replaces the pips while reloading (shotgun: fills shell by shell).
    let progress = w.reload_progress().map(|p| if w.reload_kind == ReloadKind::Shells { (w.ammo as f32 + p) / w.mag as f32 } else { p });
    let (pips_d, bar_d) = if progress.is_some() { (Display::None, Display::Flex) } else { (Display::Flex, Display::None) };
    if let Ok(mut n) = nodes.p0().single_mut()
        && n.display != pips_d
    {
        n.display = pips_d;
    }
    if let Ok(mut n) = nodes.p1().single_mut()
        && n.display != bar_d
    {
        n.display = bar_d;
    }
    if let (Some(p), Ok(mut n)) = (progress, nodes.p2().single_mut()) {
        let want = percent(p.clamp(0.0, 1.0) * 100.0);
        if n.width != want {
            n.width = want;
        }
    }

    // Warning under the crosshair.
    let msg = if w.reloading.is_some() {
        "RELOADING"
    } else if w.ammo == 0 && w.reserve == 0 {
        "NO AMMO"
    } else if w.ammo == 0 {
        "RELOAD  [R]"
    } else if low {
        "LOW AMMO"
    } else {
        ""
    };
    if let Ok((mut t, mut c, mut v)) = texts.p6().single_mut() {
        if t.0 != msg {
            t.0 = msg.into();
        }
        let want = if msg.is_empty() { Visibility::Hidden } else { Visibility::Inherited };
        if *v != want {
            *v = want;
        }
        if !msg.is_empty() {
            let pulse = 0.6 + 0.4 * (time.elapsed_secs() * 6.0).sin().abs();
            let base = if w.ammo == 0 && w.reloading.is_none() { ENEMY } else { ACCENT };
            c.0 = base.with_alpha(if w.reloading.is_some() { 0.8 } else { pulse });
        }
    }
    *shown = now;
}

/// Open the crosshair to the spread cone; hide it aimed, sprinting or switching.
fn crosshair_spread(
    settings: Res<ShroudSettings>,
    windows: Query<&Window>,
    ui_scale: Option<Res<UiScale>>,
    player: Query<&Weapon, With<Player>>,
    cams: Query<&Projection, With<MainCamera>>,
    mut xs: Query<(Entity, &Children, &mut Visibility), With<CrosshairUi>>,
    parents: Query<&ChildOf>,
    hud: Query<(), With<HideWhenPaused>>,
    mut bars: Query<&mut UiTransform, With<CrosshairBar>>,
) {
    let Ok(w) = player.single() else { return };
    let h = windows.iter().next().map_or(1080.0, |w| w.height());
    let fov = cams.iter().next().and_then(|p| if let Projection::Perspective(pp) = p { Some(pp.fov) } else { None }).unwrap_or(1.4);
    let scale = ui_scale.map_or(1.0, |s| s.0) * settings.crosshair_size.max(0.1);
    // Pixels from the centre to the cone edge, beyond the bars' resting 5 px gap.
    let gap = (w.spread.tan() / (fov * 0.5).tan() * h * 0.5 / scale - 5.0).clamp(0.0, 120.0);
    // Bodycam: no crosshair (the gun is the aim).
    let hidden = w.ads > 0.3 || w.sprint > 0.5 || !w.ready() || crate::bodycam::is_bodycam(&settings);
    for (e, kids, mut vis) in &mut xs {
        // Only the HUD's crosshair (not the settings preview).
        if !parents.iter_ancestors(e).any(|a| hud.contains(a)) {
            continue;
        }
        let want = if hidden { Visibility::Hidden } else { Visibility::Inherited };
        if *vis != want {
            *vis = want;
        }
        // Children: top, bottom, left, right, centre dot.
        for (i, k) in kids.iter().enumerate() {
            let off = match i {
                0 => Vec2::new(0.0, -gap),
                1 => Vec2::new(0.0, gap),
                2 => Vec2::new(-gap, 0.0),
                3 => Vec2::new(gap, 0.0),
                _ => Vec2::ZERO,
            };
            if let Ok(mut t) = bars.get_mut(k) {
                let want = Val2::px(off.x, off.y);
                if t.translation != want {
                    t.translation = want;
                }
            }
        }
    }
}

/// The aimed gun's optic overlay (Longbow scope or Kestrel optic), if any.
fn scope_overlay(
    player: Query<&Weapon, With<Player>>,
    mut scope: Query<&mut Visibility, (With<Scope>, Without<ReflexSight>)>,
    mut reflex: Query<&mut Visibility, (With<ReflexSight>, Without<Scope>)>,
) {
    let optic = player.single().ok().and_then(|w| w.sighted());
    let show = |on: bool, v: &mut Visibility| {
        let want = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    };
    for mut v in &mut scope {
        show(optic == Some(Optic::Scope), &mut v);
    }
    for mut v in &mut reflex {
        show(optic == Some(Optic::Reflex), &mut v);
    }
}

fn pickup_prompt(
    focus: Res<PickupFocus>,
    mut prompt: Query<&mut Visibility, With<Prompt>>,
    mut text: Query<&mut Text, With<PromptText>>,
    mut fill: Query<&mut Node, With<PromptFill>>,
) {
    if !focus.is_changed() {
        return;
    }
    let want = if focus.target.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut prompt {
        if *v != want {
            *v = want;
        }
    }
    if let (Some((_, k)), Ok(mut t)) = (focus.target, text.single_mut()) {
        let s = format!("HOLD  [E]  TAKE {}", k.label().to_uppercase());
        if t.0 != s {
            t.0 = s;
        }
    }
    if let Ok(mut n) = fill.single_mut() {
        n.width = percent(focus.hold * 100.0);
    }
}
