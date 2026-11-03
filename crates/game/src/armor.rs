//! Body armor bought in the buy menu (`economy`): a Kevlar vest ([`ARMOR_FULL`] points), with or
//! without a helmet. It lives on whoever wears it (the player, or squad members who bought it) as
//! the [`Armor`] component. `weapon::apply_damage` splits each hit between health and the vest
//! (`brain::economy::armor_split`: the vest takes half, 60% with a helmet; headshots skip the vest
//! unless there is a helmet). The player keeps armor by surviving a round; dying loses it.

use bevy::prelude::*;
pub use brain::economy::ARMOR_FULL;

use crate::player::Player;
use crate::state::{GameState, has_visuals};
use crate::ui::hud::HideWhenPaused;
use crate::ui::theme::*;
use crate::weapon::WeaponSet;

#[derive(Component, Debug, Default, Clone, Copy, PartialEq)]
pub struct Armor {
    pub points: f32,
    pub helmet: bool,
}

impl Armor {
    pub fn vest(helmet: bool) -> Self {
        Armor { points: ARMOR_FULL, helmet }
    }
}

pub struct ArmorPlugin;

impl Plugin for ArmorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), spawn_ui.run_if(has_visuals))
            .add_systems(Update, update_ui.after(WeaponSet).run_if(has_visuals).run_if(in_state(GameState::Playing)));
    }
}

#[derive(Component)]
struct ArmorText;
#[derive(Component)]
struct ArmorFill;
#[derive(Component)]
struct ArmorBox;
#[derive(Component)]
struct HelmetTag;

fn spawn_ui(mut commands: Commands) {
    commands
        .spawn((
            Name::new("Armor"),
            ArmorBox,
            HideWhenPaused,
            DespawnOnExit(GameState::Playing),
            Node {
                position_type: PositionType::Absolute,
                left: px(58),
                bottom: px(142),
                align_items: AlignItems::Center,
                column_gap: px(GAP_S),
                padding: UiRect::axes(px(10), px(4)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.6)),
            Pickable::IGNORE,
        ))
        .with_children(|b| {
            b.spawn(text("ARMOR", label(SIZE_TINY), TEXT_MUTED));
            b.spawn((Node { width: px(120), height: px(6), ..default() }, BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.1)), Pickable::IGNORE))
                .with_child((ArmorFill, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(INTEL), Pickable::IGNORE));
            b.spawn((ArmorText, text("0", label(SIZE_LABEL), INTEL)));
            b.spawn((HelmetTag, text("+ HELMET", label(SIZE_TINY), INTEL.with_alpha(0.8)), Node { display: Display::None, ..default() }));
        });
}

fn update_ui(
    player: Query<Option<&Armor>, With<Player>>,
    mut texts: Query<&mut Text, With<ArmorText>>,
    mut nodes: ParamSet<(Query<&mut Node, With<ArmorFill>>, Query<&mut Node, With<ArmorBox>>, Query<&mut Node, With<HelmetTag>>)>,
    added: Query<(), Added<ArmorText>>,
    mut shown: Local<Option<(u32, bool)>>,
) {
    let a = player.single().ok().flatten().copied().unwrap_or_default();
    let v = (a.points.max(0.0).ceil() as u32, a.helmet && a.points > 0.0);
    // Text and bar only change with the rounded value; nodes spawned this round get it once.
    if *shown == Some(v) && added.is_empty() {
        return;
    }
    *shown = Some(v);
    for mut t in &mut texts {
        t.0 = v.0.to_string();
    }
    for mut n in &mut nodes.p0() {
        n.width = percent(100.0 * v.0 as f32 / ARMOR_FULL);
    }
    let d = if v.0 == 0 { Display::None } else { Display::Flex };
    for mut n in &mut nodes.p1() {
        n.display = d;
    }
    let h = if v.1 { Display::Flex } else { Display::None };
    for mut n in &mut nodes.p2() {
        n.display = h;
    }
}
