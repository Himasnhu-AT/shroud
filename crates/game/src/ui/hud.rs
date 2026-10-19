//! In-game HUD: crosshair, hit marker, HP, ammo, generation, enemies left, round score.

use bevy::prelude::*;

use crate::player::Player;
use crate::state::{GameState, has_visuals};
use crate::weapon::{Health, HitMarker, Rifle};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudInfo>()
            .add_systems(OnEnter(GameState::Playing), spawn_hud.run_if(has_visuals))
            .add_systems(
                Update,
                update_hud.run_if(in_state(GameState::Playing).and_then(has_visuals)),
            );
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
        Self {
            generation: 1,
            enemies_left: 0,
            round_score: (0, 0),
        }
    }
}

#[derive(Component)]
struct HpText;
#[derive(Component)]
struct AmmoText;
#[derive(Component)]
struct InfoText;
#[derive(Component)]
struct GenText;
#[derive(Component)]
struct HitMarkerUi;

const HUD_COLOR: Color = Color::srgba(0.95, 0.97, 1.0, 0.9);

fn label(size: f32) -> (TextFont, TextColor) {
    (TextFont::from_font_size(size), TextColor(HUD_COLOR))
}

fn crosshair_bar(w: f32, h: f32, left: f32, top: f32) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            width: px(w),
            height: px(h),
            left: px(left),
            top: px(top),
            ..default()
        },
        BackgroundColor(HUD_COLOR),
    )
}

fn spawn_hud(mut commands: Commands) {
    let root = commands
        .spawn((
            DespawnOnExit(GameState::Playing),
            Node {
                width: percent(100),
                height: percent(100),
                position_type: PositionType::Absolute,
                ..default()
            },
        ))
        .id();

    // Crosshair: 4 bars around a 20×20 box centered on screen.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: px(20),
            height: px(20),
            left: percent(50),
            top: percent(50),
            margin: UiRect::new(px(-10), px(0), px(-10), px(0)),
            ..default()
        },
        ChildOf(root),
        children![
            crosshair_bar(2.0, 6.0, 9.0, 0.0),
            crosshair_bar(2.0, 6.0, 9.0, 14.0),
            crosshair_bar(6.0, 2.0, 0.0, 9.0),
            crosshair_bar(6.0, 2.0, 14.0, 9.0),
        ],
    ));

    // Hit marker: an "X" over the crosshair, hidden until a hit.
    commands.spawn((
        HitMarkerUi,
        Text::new("X"),
        TextFont::from_font_size(30.0),
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            margin: UiRect::new(px(-9), px(0), px(-19), px(0)),
            ..default()
        },
        Visibility::Hidden,
        ChildOf(root),
    ));

    let corner = |left: Option<f32>, right: Option<f32>, top: Option<f32>, bottom: Option<f32>| Node {
        position_type: PositionType::Absolute,
        left: left.map_or(Val::Auto, px),
        right: right.map_or(Val::Auto, px),
        top: top.map_or(Val::Auto, px),
        bottom: bottom.map_or(Val::Auto, px),
        ..default()
    };

    commands.spawn((
        GenText,
        Text::new("Generation 1"),
        label(22.0),
        corner(Some(20.0), None, Some(16.0), None),
        ChildOf(root),
    ));
    commands.spawn((
        InfoText,
        Text::new(""),
        label(22.0),
        corner(None, Some(20.0), Some(16.0), None),
        ChildOf(root),
    ));
    commands.spawn((
        HpText,
        Text::new("HP 100"),
        label(32.0),
        corner(Some(24.0), None, None, Some(20.0)),
        ChildOf(root),
    ));
    commands.spawn((
        AmmoText,
        Text::new("30 / 30"),
        label(32.0),
        corner(None, Some(24.0), None, Some(20.0)),
        ChildOf(root),
    ));
}

fn update_hud(
    info: Res<HudInfo>,
    marker: Res<HitMarker>,
    player: Query<(&Health, Option<&Rifle>), With<Player>>,
    mut texts: ParamSet<(
        Query<&mut Text, With<HpText>>,
        Query<&mut Text, With<AmmoText>>,
        Query<&mut Text, With<InfoText>>,
        Query<&mut Text, With<GenText>>,
    )>,
    mut hit: Query<(&mut Visibility, &mut TextColor), With<HitMarkerUi>>,
) {
    let set = |t: &mut Text, s: String| {
        if t.0 != s {
            t.0 = s;
        }
    };
    if let Ok((hp, rifle)) = player.single() {
        if let Ok(mut t) = texts.p0().single_mut() {
            set(&mut t, format!("HP {}", hp.hp.ceil() as i32));
        }
        if let (Some(r), Ok(mut t)) = (rifle, texts.p1().single_mut()) {
            let s = if r.reloading.is_some() {
                format!("Reloading… / {}", r.mag)
            } else {
                format!("{} / {}", r.ammo, r.mag)
            };
            set(&mut t, s);
        }
    }
    if let Ok(mut t) = texts.p2().single_mut() {
        set(
            &mut t,
            format!(
                "Enemies {}   Score {} – {}",
                info.enemies_left, info.round_score.0, info.round_score.1
            ),
        );
    }
    if let Ok(mut t) = texts.p3().single_mut() {
        set(&mut t, format!("Generation {}", info.generation));
    }
    if let Ok((mut vis, mut color)) = hit.single_mut() {
        let show = marker.timer > 0.0;
        let want = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        color.0 = if marker.kill {
            Color::srgb(1.0, 0.2, 0.2)
        } else if marker.headshot {
            Color::srgb(1.0, 0.8, 0.2)
        } else {
            Color::WHITE
        };
    }
}
