//! Shared look for every screen: palette, type scale, fonts and small bundle helpers.
//!
//! Fonts (Barlow / Barlow Condensed, OFL) load from `assets/fonts/` into fixed handles; if a
//! file is missing its handle gets Bevy's built-in font instead, so text always renders.
//! Layout is authored for a 1080 px tall viewport and scaled with [`UiScale`].

use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::text::Font;

use crate::state::has_visuals;

// ---- palette ----

/// Near-black blue used behind menus.
pub const BG: Color = Color::srgb(0.035, 0.043, 0.06);
/// Panels and cards.
pub const PANEL: Color = Color::srgba(0.07, 0.085, 0.11, 0.88);
/// Raised surface (hovered card, slider track).
pub const RAISED: Color = Color::srgba(0.13, 0.15, 0.19, 0.95);
/// Hairlines and idle borders.
pub const LINE: Color = Color::srgba(1.0, 1.0, 1.0, 0.09);
pub const LINE_STRONG: Color = Color::srgba(1.0, 1.0, 1.0, 0.18);

pub const TEXT: Color = Color::srgb(0.93, 0.945, 0.965);
pub const TEXT_MUTED: Color = Color::srgb(0.58, 0.62, 0.69);
pub const TEXT_DIM: Color = Color::srgb(0.38, 0.42, 0.49);

/// Selection / focus / primary action.
pub const ACCENT: Color = Color::srgb(1.0, 0.66, 0.22);
/// Player side (health, round wins).
pub const FRIENDLY: Color = Color::srgb(0.38, 0.9, 0.72);
/// Enemy side (squad, losses, damage).
pub const ENEMY: Color = Color::srgb(1.0, 0.36, 0.3);
/// Secondary info (the enemy brain, intel).
pub const INTEL: Color = Color::srgb(0.5, 0.78, 1.0);

pub fn with_alpha(c: Color, a: f32) -> Color {
    c.with_alpha(a)
}

// ---- type scale (px at the 1080 reference height) ----

pub const SIZE_HERO: f32 = 132.0;
pub const SIZE_H1: f32 = 64.0;
pub const SIZE_H2: f32 = 40.0;
pub const SIZE_NAV: f32 = 34.0;
pub const SIZE_LABEL: f32 = 18.0;
pub const SIZE_BODY: f32 = 20.0;
pub const SIZE_SMALL: f32 = 16.0;
pub const SIZE_TINY: f32 = 14.0;

// ---- spacing ----

pub const GAP_XS: f32 = 4.0;
pub const GAP_S: f32 = 8.0;
pub const GAP_M: f32 = 16.0;
pub const GAP_L: f32 = 28.0;
pub const GAP_XL: f32 = 48.0;
/// Distance from the screen edge for menus.
pub const EDGE: f32 = 112.0;
pub const RADIUS: f32 = 4.0;

// ---- fonts ----

/// Barlow Medium: body copy.
pub const FONT_BODY: Handle<Font> = uuid_handle!("5c2a3a90-6a77-4a8e-9f3e-2d1b8c4b0a01");
/// Barlow SemiBold: emphasised body, values.
pub const FONT_BODY_BOLD: Handle<Font> = uuid_handle!("5c2a3a90-6a77-4a8e-9f3e-2d1b8c4b0a02");
/// Barlow Condensed SemiBold: labels, nav, buttons (usually upper case).
pub const FONT_LABEL: Handle<Font> = uuid_handle!("5c2a3a90-6a77-4a8e-9f3e-2d1b8c4b0a03");
/// Barlow Condensed Bold: titles and big numbers.
pub const FONT_DISPLAY: Handle<Font> = uuid_handle!("5c2a3a90-6a77-4a8e-9f3e-2d1b8c4b0a04");

const FONT_FILES: [(&Handle<Font>, &str); 4] = [
    (&FONT_BODY, "fonts/Barlow-Medium.ttf"),
    (&FONT_BODY_BOLD, "fonts/Barlow-SemiBold.ttf"),
    (&FONT_LABEL, "fonts/BarlowCondensed-SemiBold.ttf"),
    (&FONT_DISPLAY, "fonts/BarlowCondensed-Bold.ttf"),
];

fn font(handle: &Handle<Font>, size: f32) -> TextFont {
    TextFont::from_font_size(size).with_font(handle.clone())
}

pub fn body(size: f32) -> TextFont {
    font(&FONT_BODY, size)
}
pub fn body_bold(size: f32) -> TextFont {
    font(&FONT_BODY_BOLD, size)
}
pub fn label(size: f32) -> TextFont {
    font(&FONT_LABEL, size)
}
pub fn display(size: f32) -> TextFont {
    font(&FONT_DISPLAY, size)
}

/// A text node: `(Text, TextFont, TextColor)`.
pub fn text(s: impl Into<String>, font: TextFont, color: Color) -> impl Bundle {
    (Text::new(s), font, TextColor(color), Pickable::IGNORE)
}

/// Small upper-case section label ("DIFFICULTY").
pub fn eyebrow(s: &str, color: Color) -> impl Bundle {
    text(s.to_uppercase(), label(SIZE_LABEL), color)
}

/// Bordered panel column.
pub fn panel(padding: f32, gap: f32) -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(gap),
            padding: UiRect::all(px(padding)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(RADIUS)),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor::all(LINE),
    )
}

/// Full-screen absolute root for a screen or overlay.
pub fn fullscreen() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: percent(100),
        height: percent(100),
        ..default()
    }
}

/// Left-heavy darkening over the live 3D scene (menus).
pub fn menu_scrim(strength: f32) -> BackgroundGradient {
    BackgroundGradient::from(LinearGradient::new(
        std::f32::consts::FRAC_PI_2,
        vec![
            ColorStop::new(BG.with_alpha(0.97 * strength), percent(0)),
            ColorStop::new(BG.with_alpha(0.86 * strength), percent(38)),
            ColorStop::new(BG.with_alpha(0.45 * strength), percent(100)),
        ],
    ))
}

/// A thin horizontal rule.
pub fn rule(color: Color) -> impl Bundle {
    (Node { height: px(1), width: percent(100), ..default() }, BackgroundColor(color), Pickable::IGNORE)
}

/// A key cap ("ESC") followed by what it does, for footers.
pub fn key_hint(parent: &mut ChildSpawnerCommands, key: &str, action: &str) {
    parent
        .spawn((Node { column_gap: px(GAP_S), align_items: AlignItems::Center, ..default() }, Pickable::IGNORE))
        .with_children(|row| {
            row.spawn((
                Node {
                    padding: UiRect::axes(px(7), px(2)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(3)),
                    min_width: px(28),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BorderColor::all(LINE_STRONG),
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)),
                Pickable::IGNORE,
                children![text(key, label(SIZE_TINY), TEXT)],
            ));
            row.spawn(text(action, body(SIZE_SMALL), TEXT_MUTED));
        });
}

/// Footer row of key hints, pinned bottom-left.
pub fn spawn_footer(parent: &mut ChildSpawnerCommands, hints: &[(&str, &str)]) {
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(EDGE),
                bottom: px(44),
                column_gap: px(GAP_L),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            for (k, a) in hints {
                key_hint(row, k, a);
            }
        });
}

// ---- plugin ----

pub struct ThemePlugin;

impl Plugin for ThemePlugin {
    fn build(&self, app: &mut App) {
        let dir = crate::assets::assets_dir();
        if let Some(mut fonts) = app.world_mut().get_resource_mut::<Assets<Font>>() {
            for (handle, rel) in FONT_FILES {
                let bytes = std::fs::read(dir.join(rel)).unwrap_or_else(|_| {
                    warn!("theme: {rel} missing, using the built-in font");
                    bevy::text::DEFAULT_FONT_DATA.to_vec()
                });
                let _ = fonts.insert(handle.id(), Font::from_bytes(bytes));
            }
        }
        app.add_systems(Update, scale_ui.run_if(has_visuals));
    }
}

/// Reference height the layout is authored for.
pub const REF_HEIGHT: f32 = 1080.0;

/// Scale the UI with the viewport height (clamped so small windows stay readable).
fn scale_ui(cams: Query<&Camera, With<crate::player::PlayerCamera>>, mut scale: ResMut<UiScale>) {
    let Some(size) = cams.iter().find_map(|c| c.logical_viewport_size()) else { return };
    let s = (size.y / REF_HEIGHT).clamp(0.72, 2.0);
    if (scale.0 - s).abs() > 0.005 {
        scale.0 = s;
    }
}
