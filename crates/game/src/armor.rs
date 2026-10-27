//! Body armor: every round starts with [`BASE_ARMOR`], plus [`COMEBACK_ARMOR`] after losing
//! the previous round of the match. Armor soaks [`ABSORB`] of every non-lethal hit: a system
//! after damage sees the player's health drop and refunds that share from the armor (so
//! weapon code needs no change). A lethal hit is never undone — every other system has already
//! seen the death — so armor stretches a fight rather than cheating death.

use bevy::prelude::*;

use crate::match_flow::{MatchFlow, begin_round, check_round_end};
use crate::player::Player;
use crate::state::{GameState, has_visuals};
use crate::ui::hud::HideWhenPaused;
use crate::ui::theme::*;
use crate::weapon::{Health, WeaponSet};

pub const BASE_ARMOR: f32 = 25.0;
pub const COMEBACK_ARMOR: f32 = 25.0;
pub const MAX_ARMOR: f32 = 50.0;
/// Share of each hit the armor takes.
pub const ABSORB: f32 = 0.5;

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct Armor {
    pub points: f32,
    /// The player's health after last frame's refund (to see new damage).
    last_hp: Option<f32>,
}

pub struct ArmorPlugin;

impl Plugin for ArmorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Armor>()
            .add_systems(OnEnter(GameState::Playing), (grant.after(begin_round), spawn_ui.run_if(has_visuals)))
            .add_systems(Update, absorb.after(WeaponSet).before(check_round_end).run_if(in_state(GameState::Playing)))
            .add_systems(Update, update_ui.after(absorb).run_if(has_visuals).run_if(in_state(GameState::Playing)));
    }
}

/// Round start: base armor, more after a lost round.
fn grant(flow: Res<MatchFlow>, mut armor: ResMut<Armor>) {
    let comeback = if flow.last_round == Some(false) { COMEBACK_ARMOR } else { 0.0 };
    *armor = Armor { points: (BASE_ARMOR + comeback).min(MAX_ARMOR), last_hp: None };
}

fn absorb(mut armor: ResMut<Armor>, mut players: Query<&mut Health, With<Player>>) {
    let Ok(mut hp) = players.single_mut() else { return };
    let Some(last) = armor.last_hp else {
        armor.last_hp = Some(hp.hp);
        return;
    };
    let mut now = hp.hp;
    if now < last && !hp.is_dead() && armor.points > 0.0 {
        let refund = ((last - now) * ABSORB).min(armor.points);
        now = (now + refund).min(hp.max);
        hp.hp = now;
        armor.points -= refund;
    }
    if armor.last_hp != Some(now) {
        armor.last_hp = Some(now);
    }
}

#[derive(Component)]
struct ArmorText;
#[derive(Component)]
struct ArmorFill;
#[derive(Component)]
struct ArmorBox;

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
        });
}

fn update_ui(
    armor: Res<Armor>,
    mut texts: Query<&mut Text, With<ArmorText>>,
    mut fills: Query<&mut Node, (With<ArmorFill>, Without<ArmorBox>)>,
    mut boxes: Query<&mut Node, With<ArmorBox>>,
    added: Query<(), Added<ArmorText>>,
    mut shown: Local<Option<u32>>,
) {
    let v = armor.points.max(0.0).ceil() as u32;
    // Text and bar only change with the rounded value; nodes spawned this round get it once.
    let fresh_ui = !added.is_empty();
    if *shown == Some(v) && !fresh_ui {
        return;
    }
    *shown = Some(v);
    for mut t in &mut texts {
        t.0 = v.to_string();
    }
    for mut n in &mut fills {
        n.width = percent(100.0 * v as f32 / MAX_ARMOR);
    }
    let d = if v == 0 { Display::None } else { Display::Flex };
    for mut n in &mut boxes {
        if n.display != d {
            n.display = d;
        }
    }
}
