//! The bomb on the HUD (`crate::bomb`): a "BOMB" tag above the health panel while you carry
//! it (with the site letter when you stand in one), the "hold E to plant" prompt with its
//! progress bar, the pulsing red bomb that replaces the round timer after the plant, a subtle
//! "defusing" warning while a defender is on it, the "bomb planted" banner, and "you are down,
//! bomb ticking" when you die after planting. The round-over reason is `hud::round_over_banner`.

use bevy::prelude::*;

use super::hud::{BombTimerIcon, RoundStartBanner, TimerText, banner_root, hud_root};
use super::theme::*;
use crate::bomb::{Bomb, BombEvent, BombPhase};
use crate::map::ArenaRes;
use crate::match_flow::MatchFlow;
use crate::player::Player;
use crate::state::{GameState, has_visuals};
use crate::weapon::{Health, WeaponSet};

pub struct BombHudPlugin;

impl Plugin for BombHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), spawn.run_if(has_visuals).after(crate::match_flow::begin_round)).add_systems(
            Update,
            (update, planted_banner).after(WeaponSet).run_if(in_state(GameState::Playing).and_then(has_visuals)),
        );
    }
}

#[derive(Component)]
struct CarryTag;
#[derive(Component)]
struct CarryText;
#[derive(Component)]
struct PlantPrompt;
#[derive(Component)]
struct PlantText;
#[derive(Component)]
struct PlantFill;
#[derive(Component)]
struct DefuseWarning;
#[derive(Component)]
struct DownNotice;

fn spawn(mut commands: Commands) {
    let root = commands.spawn((Name::new("Bomb HUD"), hud_root())).id();
    // Above the health panel: the bomb you carry.
    commands
        .spawn((
            CarryTag,
            Node {
                position_type: PositionType::Absolute,
                left: px(40),
                // Above the armor bar (`armor::spawn_ui`, bottom 142), which sits in the same
                // corner: in Bodycam (no health panel) the two overlapped.
                bottom: px(180),
                align_items: AlignItems::Center,
                column_gap: px(8),
                padding: UiRect::axes(px(10), px(4)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.55)),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_children(|t| {
            // A little charge: a block with a red dot.
            t.spawn((
                Node { width: px(18), height: px(11), border_radius: BorderRadius::all(px(2)), justify_content: JustifyContent::FlexEnd, padding: UiRect::all(px(2)), ..default() },
                BackgroundColor(ACCENT),
                Pickable::IGNORE,
            ))
            .with_child((Node { width: px(4), height: px(4), border_radius: BorderRadius::all(px(2)), ..default() }, BackgroundColor(ENEMY), Pickable::IGNORE));
            t.spawn((CarryText, text("BOMB", label(SIZE_LABEL), ACCENT)));
        });

    // Under the crosshair (below the pickup prompt): plant prompt and progress.
    commands
        .spawn((
            PlantPrompt,
            Node {
                position_type: PositionType::Absolute,
                top: percent(63),
                left: px(0),
                right: px(0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_children(|c| {
            c.spawn((
                Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, row_gap: px(5), padding: UiRect::axes(px(14), px(7)), ..default() },
                BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.6)),
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                p.spawn((PlantText, text("HOLD  [E]  TO PLANT", label(SIZE_LABEL), TEXT)));
                p.spawn((Node { width: px(220), height: px(5), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.12)), Pickable::IGNORE))
                    .with_child((PlantFill, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(ACCENT), Pickable::IGNORE));
            });
        });

    // Under the enemy pips: a defender is on the bomb.
    commands
        .spawn((
            Node { position_type: PositionType::Absolute, top: px(136), left: px(0), right: px(0), justify_content: JustifyContent::Center, ..default() },
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_child((
            DefuseWarning,
            text("DEFUSING", label(SIZE_LABEL), Color::WHITE),
            Node { padding: UiRect::axes(px(10), px(3)), ..default() },
            // White on a solid red badge: red text on the dark panel was unreadable.
            BackgroundColor(Color::srgba(0.72, 0.1, 0.1, 0.9)),
            Visibility::Hidden,
        ));

    // Dead after the plant.
    commands
        .spawn((
            DownNotice,
            Node {
                position_type: PositionType::Absolute,
                top: percent(38),
                left: px(0),
                right: px(0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(4),
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .with_children(|d| {
            d.spawn((text("YOU ARE DOWN", display(48.0), TEXT), TextShadow::default()));
            d.spawn((text("BOMB TICKING \u{b7} THE ROUND ENDS ON THE BOMB", label(SIZE_LABEL + 2.0), ENEMY), TextShadow::default()));
        });
}

fn show(v: &mut Visibility, on: bool) {
    let want = if on { Visibility::Inherited } else { Visibility::Hidden };
    if *v != want {
        *v = want;
    }
}

fn set_text(t: &mut Text, s: String) {
    if t.0 != s {
        t.0 = s;
    }
}

fn update(
    time: Res<Time>,
    bomb: Res<Bomb>,
    flow: Res<MatchFlow>,
    arena: Res<ArenaRes>,
    player: Query<&Health, With<Player>>,
    mut vis: ParamSet<(
        Query<&mut Visibility, With<CarryTag>>,
        Query<&mut Visibility, With<PlantPrompt>>,
        Query<(&mut Visibility, &mut TextColor), With<DefuseWarning>>,
        Query<&mut Visibility, With<DownNotice>>,
    )>,
    mut texts: ParamSet<(Query<&mut Text, With<CarryText>>, Query<&mut Text, With<PlantText>>)>,
    mut fill: Query<&mut Node, (With<PlantFill>, Without<BombTimerIcon>, Without<TimerText>)>,
    mut timer: Query<&mut Node, (With<TimerText>, Without<BombTimerIcon>, Without<PlantFill>)>,
    mut icon: Query<(&mut Node, &mut UiTransform), (With<BombTimerIcon>, Without<TimerText>, Without<PlantFill>)>,
) {
    let dead = player.single().is_ok_and(|h| h.is_dead());
    let carried = bomb.phase == BombPhase::Carried && !dead;
    let letter = |i: usize| arena.0.sites.get(i).map_or('?', |s| s.letter);
    for mut v in &mut vis.p0() {
        show(&mut v, carried);
    }
    if let Ok(mut t) = texts.p0().single_mut() {
        set_text(&mut t, match bomb.in_site {
            Some(i) => format!("BOMB  \u{b7}  SITE {}", letter(i)),
            None => "BOMB".into(),
        });
    }
    let prompt = bomb.can_plant && flow.round_over.is_none();
    for mut v in &mut vis.p1() {
        show(&mut v, prompt);
    }
    if prompt {
        if let Ok(mut t) = texts.p1().single_mut() {
            let site = letter(bomb.in_site.unwrap_or(0));
            set_text(&mut t, if bomb.plant > 0.0 { format!("PLANTING  \u{b7}  SITE {site}") } else { format!("HOLD  [E]  TO PLANT  \u{b7}  SITE {site}") });
        }
        if let Ok(mut n) = fill.single_mut() {
            n.width = percent(bomb.plant.clamp(0.0, 1.0) * 100.0);
        }
    }
    let planted = bomb.phase == BombPhase::Planted;
    // A defender on the bomb: a quiet red pulse under the pips.
    if let Ok((mut v, mut c)) = vis.p2().single_mut() {
        let on = planted && bomb.defuser.is_some();
        show(&mut v, on);
        if on {
            c.0 = Color::WHITE.with_alpha(0.7 + 0.3 * (time.elapsed_secs() * 6.0).sin().abs());
        }
    }
    for mut v in &mut vis.p3() {
        show(&mut v, planted && dead && flow.round_over.is_none());
    }
    // The round timer turns into the bomb once it's down (and stays until the round resets).
    let bomb_icon = bomb.phase != BombPhase::Carried;
    for mut n in &mut timer {
        let d = if bomb_icon { Display::None } else { Display::Flex };
        if n.display != d {
            n.display = d;
        }
    }
    for (mut n, mut tf) in &mut icon {
        let d = if bomb_icon { Display::Flex } else { Display::None };
        if n.display != d {
            n.display = d;
        }
        // Swell on each beep.
        let s = Vec2::splat(if planted { 1.0 + 0.12 * (bomb.led / 0.09).clamp(0.0, 1.0) } else { 1.0 });
        if tf.scale != s {
            tf.scale = s;
        }
    }
}

fn planted_banner(mut commands: Commands, mut events: MessageReader<BombEvent>) {
    for ev in events.read() {
        if let BombEvent::Planted { site, .. } = *ev {
            let root = banner_root(&mut commands, Some(2.4), 22.0);
            commands.entity(root).insert(RoundStartBanner).with_children(|b| {
                b.spawn(text(format!("SITE {site}"), label(SIZE_LABEL + 2.0), TEXT_MUTED));
                b.spawn((text("BOMB PLANTED", display(80.0), ENEMY), TextShadow::default()));
                b.spawn((Node { width: px(120), height: px(3), ..default() }, BackgroundColor(ENEMY), Pickable::IGNORE));
            });
        }
    }
}
