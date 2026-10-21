//! Reusable menu widgets and keyboard/mouse focus.
//!
//! - [`Focusable`] entities form one vertical list (by `order`); Up/Down moves focus, the mouse
//!   focuses whatever it hovers. Enter activates, Left/Right adjusts, Esc is [`NavInput::Back`].
//! - Buttons carry a [`UiAction`]; activating one writes a [`UiActivated`] message that
//!   `ui::screens` handles.
//! - Field rows ([`Field`]) edit [`Settings`] directly: toggle, segmented choice / cards, slider
//!   (drag, click, arrow keys). Their visuals refresh from `Settings` every frame.
//! - Colours come from [`Look`]: idle / hot (hovered or focused) / on (selected).

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use brain::arena::MapId;
use brain::difficulty::Difficulty;

use super::theme::*;
use crate::settings::{GraphicsQuality, Settings, WindowModeSetting};
use crate::state::has_visuals;

// ---- input ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavInput {
    Up,
    Down,
    Left,
    Right,
    Accept,
    Back,
    NextTab,
    PrevTab,
}

impl NavInput {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "up" => Self::Up,
            "down" => Self::Down,
            "left" => Self::Left,
            "right" => Self::Right,
            "accept" | "enter" => Self::Accept,
            "back" | "esc" => Self::Back,
            "next_tab" => Self::NextTab,
            "prev_tab" => Self::PrevTab,
            _ => return None,
        })
    }
}

/// Navigation inputs for this frame (keyboard, plus anything the control port queues).
#[derive(Resource, Default)]
pub struct NavQueue(pub Vec<NavInput>);

/// What a button does. Handled in `ui::screens`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    Play,
    Settings,
    Quit,
    StartMatch,
    Resume,
    Restart,
    MainMenu,
    Back,
    Tab(usize),
    /// Reset the open settings tab to defaults.
    ResetTab,
    NextMatch,
    ResetAi,
}

#[derive(Message, Clone, Copy, Debug)]
pub struct UiActivated(pub UiAction);

/// Part of the focus list. Lower `order` is higher on screen.
#[derive(Component, Clone, Copy, Debug)]
pub struct Focusable {
    pub order: u32,
}

/// The focused entity (one of the [`Focusable`]s).
#[derive(Resource, Default, Debug)]
pub struct Focus(pub Option<Entity>);

#[derive(Component, Clone, Copy)]
pub struct ActionButton(pub UiAction);

/// Colours for idle / hot (hovered or focused) / on (selected) states.
/// `bg`/`border` apply to the entity itself; `text` to text in its subtree (not crossing
/// another `Look`).
#[derive(Component, Clone, Copy)]
pub struct Look {
    pub bg: [Color; 3],
    pub border: [Color; 3],
    pub text: [Color; 3],
}

impl Look {
    pub const fn new(bg: [Color; 3], border: [Color; 3], text: [Color; 3]) -> Self {
        Self { bg, border, text }
    }
}

/// A child whose background follows the nearest `Look` owner's state (e.g. a focus bar).
#[derive(Component, Clone, Copy)]
pub struct LookBg(pub [Color; 3]);

/// Text inside a `Look` subtree that keeps its own colour.
#[derive(Component)]
pub struct KeepColor;

// ---- fields ----

/// A setting the menus can edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Difficulty,
    Map,
    ShowCallouts,
    CrosshairColor,
    CrosshairSize,
    Sensitivity,
    InvertY,
    Quality,
    Fov,
    WindowMode,
    Vsync,
    ShowFps,
    Master,
    Sfx,
    Ambience,
}

pub enum FieldKind {
    Toggle,
    Choice(Vec<&'static str>),
    Slider { min: f32, max: f32, step: f32 },
}

/// Crosshair colour presets (name, sRGB).
pub const CROSSHAIR_COLORS: [(&str, [f32; 3]); 6] = [
    ("Green", [0.4, 1.0, 0.6]),
    ("Cyan", [0.35, 0.9, 1.0]),
    ("White", [1.0, 1.0, 1.0]),
    ("Yellow", [1.0, 0.92, 0.3]),
    ("Magenta", [1.0, 0.4, 0.95]),
    ("Red", [1.0, 0.3, 0.3]),
];

const QUALITIES: [GraphicsQuality; 4] = [GraphicsQuality::Low, GraphicsQuality::Medium, GraphicsQuality::High, GraphicsQuality::Ultra];
const WINDOW_MODES: [WindowModeSetting; 3] = [WindowModeSetting::Windowed, WindowModeSetting::Borderless, WindowModeSetting::Fullscreen];

fn index_of<T: PartialEq>(all: &[T], v: &T) -> f32 {
    all.iter().position(|x| x == v).unwrap_or(0) as f32
}

fn pick<T: Copy>(all: &[T], v: f32) -> T {
    all[(v.round().max(0.0) as usize).min(all.len() - 1)]
}

impl Field {
    pub const ALL: [Field; 15] = [
        Field::Difficulty,
        Field::Map,
        Field::ShowCallouts,
        Field::CrosshairColor,
        Field::CrosshairSize,
        Field::Sensitivity,
        Field::InvertY,
        Field::Quality,
        Field::Fov,
        Field::WindowMode,
        Field::Vsync,
        Field::ShowFps,
        Field::Master,
        Field::Sfx,
        Field::Ambience,
    ];

    /// Name used by the control port (`ui.setting`).
    pub fn key(self) -> &'static str {
        match self {
            Field::Difficulty => "difficulty",
            Field::Map => "map",
            Field::ShowCallouts => "show_callouts",
            Field::CrosshairColor => "crosshair_color",
            Field::CrosshairSize => "crosshair_size",
            Field::Sensitivity => "sensitivity",
            Field::InvertY => "invert_y",
            Field::Quality => "quality",
            Field::Fov => "fov",
            Field::WindowMode => "window_mode",
            Field::Vsync => "vsync",
            Field::ShowFps => "show_fps",
            Field::Master => "master_volume",
            Field::Sfx => "sfx_volume",
            Field::Ambience => "ambience_volume",
        }
    }

    pub fn from_key(k: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == k)
    }

    pub fn label(self) -> &'static str {
        match self {
            Field::Difficulty => "Difficulty",
            Field::Map => "Map",
            Field::ShowCallouts => "Squad callouts",
            Field::CrosshairColor => "Crosshair colour",
            Field::CrosshairSize => "Crosshair size",
            Field::Sensitivity => "Mouse sensitivity",
            Field::InvertY => "Invert look",
            Field::Quality => "Graphics quality",
            Field::Fov => "Field of view",
            Field::WindowMode => "Display mode",
            Field::Vsync => "V-Sync",
            Field::ShowFps => "FPS counter",
            Field::Master => "Master volume",
            Field::Sfx => "Effects",
            Field::Ambience => "Ambience",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Field::Difficulty => "Applies from the next round.",
            Field::Map => "",
            Field::ShowCallouts => "Show enemy radio chatter as subtitles.",
            Field::CrosshairColor => "",
            Field::CrosshairSize => "",
            Field::Sensitivity => "",
            Field::InvertY => "Mouse up looks down.",
            Field::Quality => "Ambient occlusion, anti-aliasing, shadows, bloom.",
            Field::Fov => "Horizontal-ish; wider shows more, smaller targets.",
            Field::WindowMode => "",
            Field::Vsync => "Off for lowest latency; may tear.",
            Field::ShowFps => "Small counter in the top-right corner.",
            Field::Master => "",
            Field::Sfx => "Gunfire, footsteps, hits.",
            Field::Ambience => "Background bed.",
        }
    }

    pub fn kind(self) -> FieldKind {
        match self {
            Field::Difficulty => FieldKind::Choice(Difficulty::ALL.iter().map(|d| d.label()).collect()),
            Field::Map => FieldKind::Choice(MapId::ALL.iter().map(|m| m.label()).collect()),
            Field::ShowCallouts | Field::InvertY | Field::Vsync | Field::ShowFps => FieldKind::Toggle,
            Field::CrosshairColor => FieldKind::Choice(CROSSHAIR_COLORS.iter().map(|c| c.0).collect()),
            Field::CrosshairSize => FieldKind::Slider { min: 0.5, max: 2.0, step: 0.1 },
            Field::Sensitivity => FieldKind::Slider { min: 0.1, max: 4.0, step: 0.05 },
            Field::Quality => FieldKind::Choice(vec!["Low", "Medium", "High", "Ultra"]),
            Field::Fov => FieldKind::Slider { min: 60.0, max: 110.0, step: 1.0 },
            Field::WindowMode => FieldKind::Choice(vec!["Windowed", "Borderless", "Fullscreen"]),
            Field::Master | Field::Sfx | Field::Ambience => FieldKind::Slider { min: 0.0, max: 1.0, step: 0.05 },
        }
    }

    /// Current value: index for choices, 0/1 for toggles, the number for sliders.
    pub fn get(self, s: &Settings) -> f32 {
        let b = |v: bool| if v { 1.0 } else { 0.0 };
        match self {
            Field::Difficulty => index_of(&Difficulty::ALL, &s.difficulty),
            Field::Map => index_of(&MapId::ALL, &s.map),
            Field::ShowCallouts => b(s.show_callouts),
            Field::CrosshairColor => {
                let c = Vec3::from(s.crosshair_color);
                CROSSHAIR_COLORS
                    .iter()
                    .enumerate()
                    .min_by(|a, b| Vec3::from(a.1.1).distance(c).total_cmp(&Vec3::from(b.1.1).distance(c)))
                    .map_or(0.0, |(i, _)| i as f32)
            }
            Field::CrosshairSize => s.crosshair_size,
            Field::Sensitivity => s.sensitivity,
            Field::InvertY => b(s.invert_y),
            Field::Quality => index_of(&QUALITIES, &s.quality),
            Field::Fov => s.fov_deg,
            Field::WindowMode => index_of(&WINDOW_MODES, &s.window_mode),
            Field::Vsync => b(s.vsync),
            Field::ShowFps => b(s.show_fps),
            Field::Master => s.master_volume,
            Field::Sfx => s.sfx_volume,
            Field::Ambience => s.ambience_volume,
        }
    }

    /// Set from a value in the same units as [`Field::get`] (clamped / snapped).
    pub fn set(self, s: &mut Settings, v: f32) {
        let v = match self.kind() {
            FieldKind::Slider { min, max, step } => (((v - min) / step).round() * step + min).clamp(min, max),
            FieldKind::Toggle => v.round().clamp(0.0, 1.0),
            FieldKind::Choice(o) => v.round().clamp(0.0, (o.len() - 1) as f32),
        };
        let on = v > 0.5;
        match self {
            Field::Difficulty => s.difficulty = pick(&Difficulty::ALL, v),
            Field::Map => s.map = pick(&MapId::ALL, v),
            Field::ShowCallouts => s.show_callouts = on,
            Field::CrosshairColor => s.crosshair_color = pick(&CROSSHAIR_COLORS, v).1,
            Field::CrosshairSize => s.crosshair_size = v,
            Field::Sensitivity => s.sensitivity = v,
            Field::InvertY => s.invert_y = on,
            Field::Quality => s.quality = pick(&QUALITIES, v),
            Field::Fov => s.fov_deg = v,
            Field::WindowMode => s.window_mode = pick(&WINDOW_MODES, v),
            Field::Vsync => s.vsync = on,
            Field::ShowFps => s.show_fps = on,
            Field::Master => s.master_volume = v,
            Field::Sfx => s.sfx_volume = v,
            Field::Ambience => s.ambience_volume = v,
        }
    }

    pub fn format(self, v: f32) -> String {
        match self {
            Field::CrosshairSize | Field::Sensitivity => format!("{v:.2}"),
            Field::Fov => format!("{v:.0}\u{b0}"),
            Field::Master | Field::Sfx | Field::Ambience => format!("{:.0}%", v * 100.0),
            _ => format!("{v}"),
        }
    }

    /// Left/Right adjustment (`dir` = -1 or +1). Choices wrap only when `wrap`.
    pub fn step(self, s: &mut Settings, dir: f32) {
        let v = self.get(s);
        match self.kind() {
            FieldKind::Slider { step, .. } => self.set(s, v + step * dir),
            FieldKind::Toggle => self.set(s, 1.0 - v),
            FieldKind::Choice(o) => self.set(s, (v + dir).clamp(0.0, (o.len() - 1) as f32)),
        }
    }
}

/// A row that edits a [`Field`] (focusable).
#[derive(Component, Clone, Copy)]
pub struct FieldRow(pub Field);

/// One option of a choice field (segment or card); clicking selects it.
#[derive(Component, Clone, Copy)]
pub struct Segment {
    pub field: Field,
    pub index: usize,
}

/// Toggle switch (clickable).
#[derive(Component, Clone, Copy)]
pub struct ToggleSwitch(pub Field);
#[derive(Component)]
struct ToggleKnob;

/// Slider track (click/drag), its fill and knob, and the value label.
#[derive(Component, Clone, Copy)]
pub struct SliderTrack(pub Field);
#[derive(Component)]
struct SliderFill;
#[derive(Component)]
struct SliderKnob;
#[derive(Component, Clone, Copy)]
pub struct FieldValue(pub Field);

/// The slider being dragged with the mouse.
#[derive(Resource, Default)]
struct Dragging(Option<Entity>);

// ---- looks ----

const CLEAR: Color = Color::NONE;

pub fn nav_look() -> Look {
    Look::new(
        [CLEAR, with_alpha(ACCENT, 0.10), CLEAR],
        [CLEAR; 3],
        [TEXT_MUTED, TEXT, TEXT],
    )
}

pub fn primary_look() -> Look {
    Look::new(
        [with_alpha(ACCENT, 0.88), ACCENT, ACCENT],
        [CLEAR, Color::srgb(1.0, 0.85, 0.6), CLEAR],
        [BG, BG, BG],
    )
}

pub fn secondary_look() -> Look {
    Look::new(
        [Color::srgba(1.0, 1.0, 1.0, 0.04), Color::srgba(1.0, 1.0, 1.0, 0.10), RAISED],
        [LINE_STRONG, with_alpha(ACCENT, 0.8), LINE_STRONG],
        [TEXT_MUTED, TEXT, TEXT],
    )
}

fn row_look() -> Look {
    Look::new(
        [Color::srgba(0.06, 0.075, 0.1, 0.72), Color::srgba(0.13, 0.15, 0.19, 0.92), CLEAR],
        [CLEAR, with_alpha(ACCENT, 0.35), CLEAR],
        [TEXT, TEXT, TEXT],
    )
}

fn segment_look() -> Look {
    Look::new(
        [Color::srgba(1.0, 1.0, 1.0, 0.04), Color::srgba(1.0, 1.0, 1.0, 0.10), with_alpha(ACCENT, 0.92)],
        [LINE, LINE_STRONG, ACCENT],
        [TEXT_MUTED, TEXT, BG],
    )
}

fn card_look() -> Look {
    Look::new(
        [PANEL, Color::srgba(0.12, 0.14, 0.18, 0.95), Color::srgba(0.16, 0.13, 0.09, 0.96)],
        [LINE, LINE_STRONG, ACCENT],
        [TEXT_MUTED, TEXT, TEXT],
    )
}

fn tab_look() -> Look {
    Look::new([CLEAR, Color::srgba(1.0, 1.0, 1.0, 0.06), CLEAR], [CLEAR; 3], [TEXT_DIM, TEXT, TEXT])
}

// ---- spawning ----

/// Main-menu style nav item: accent bar + big condensed label.
pub fn nav_button(p: &mut ChildSpawnerCommands, label_text: &str, action: UiAction, order: u32) {
    p.spawn((
        Button,
        ActionButton(action),
        Focusable { order },
        nav_look(),
        Node {
            width: px(420),
            height: px(62),
            align_items: AlignItems::Center,
            padding: UiRect::left(px(26)),
            ..default()
        },
        BackgroundColor(CLEAR),
    ))
    .with_children(|b| {
        b.spawn((
            LookBg([CLEAR, ACCENT, CLEAR]),
            Node { position_type: PositionType::Absolute, left: px(0), top: px(12), bottom: px(12), width: px(4), ..default() },
            BackgroundColor(CLEAR),
            Pickable::IGNORE,
        ));
        b.spawn(text(label_text.to_uppercase(), label(SIZE_NAV), TEXT_MUTED));
    });
}

/// Filled (primary) or outlined button with a label.
pub fn action_button(p: &mut ChildSpawnerCommands, label_text: &str, action: UiAction, order: Option<u32>, primary: bool, width: f32) {
    let mut e = p.spawn((
        Button,
        ActionButton(action),
        if primary { primary_look() } else { secondary_look() },
        Node {
            width: px(width),
            height: px(if primary { 64.0 } else { 52.0 }),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(RADIUS)),
            ..default()
        },
        BackgroundColor(CLEAR),
        BorderColor::all(CLEAR),
        children![text(label_text.to_uppercase(), label(if primary { 28.0 } else { 22.0 }), TEXT)],
    ));
    if let Some(order) = order {
        e.insert(Focusable { order });
    }
}

/// Tab strip button.
pub fn tab_button(p: &mut ChildSpawnerCommands, label_text: &str, index: usize, active: bool) {
    p.spawn((
        Button,
        ActionButton(UiAction::Tab(index)),
        tab_look(),
        Node {
            padding: UiRect::axes(px(22), px(10)),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(8),
            ..default()
        },
        BackgroundColor(CLEAR),
    ))
    .with_children(|t| {
        t.spawn(text(label_text.to_uppercase(), label(26.0), if active { TEXT } else { TEXT_DIM }));
        t.spawn((
            Node { width: percent(100), height: px(3), ..default() },
            BackgroundColor(if active { ACCENT } else { CLEAR }),
            Pickable::IGNORE,
        ));
    })
    .insert(TabButton { active });
}

#[derive(Component)]
pub struct TabButton {
    pub active: bool,
}

/// A settings row: label (+ hint) on the left, the control on the right.
pub fn field_row(p: &mut ChildSpawnerCommands, field: Field, order: u32) {
    p.spawn((
        Button,
        FieldRow(field),
        Focusable { order },
        row_look(),
        Node {
            width: percent(100),
            min_height: px(68),
            padding: UiRect::axes(px(22), px(10)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            column_gap: px(GAP_L),
            border: UiRect::left(px(3)),
            border_radius: BorderRadius::all(px(3)),
            ..default()
        },
        BackgroundColor(CLEAR),
        BorderColor::all(CLEAR),
    ))
    .with_children(|row| {
        row.spawn((Node { flex_direction: FlexDirection::Column, row_gap: px(2), flex_shrink: 1.0, ..default() }, Pickable::IGNORE))
            .with_children(|l| {
                l.spawn(text(field.label(), body_bold(SIZE_BODY), TEXT));
                let hint = field.hint();
                if !hint.is_empty() {
                    l.spawn((text(hint, body(SIZE_TINY), TEXT_MUTED), KeepColor));
                }
            });
        row.spawn((Node { width: px(460), flex_shrink: 0.0, justify_content: JustifyContent::FlexEnd, ..default() }, Pickable::IGNORE))
            .with_children(|c| match field.kind() {
                FieldKind::Toggle => spawn_toggle(c, field),
                FieldKind::Choice(options) => spawn_segments(c, field, &options),
                FieldKind::Slider { .. } => spawn_slider(c, field),
            });
    });
}

fn spawn_toggle(p: &mut ChildSpawnerCommands, field: Field) {
    p.spawn((Node { align_items: AlignItems::Center, column_gap: px(GAP_M), ..default() }, Pickable::IGNORE))
        .with_children(|r| {
            r.spawn((FieldValue(field), text("", label(20.0), TEXT_MUTED), KeepColor, Node { width: px(40), ..default() }));
            r.spawn((
                Button,
                ToggleSwitch(field),
                Node {
                    width: px(60),
                    height: px(30),
                    padding: UiRect::all(px(4)),
                    border_radius: BorderRadius::all(px(15)),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(RAISED),
            ))
            .with_child((
                ToggleKnob,
                Node { width: px(22), height: px(22), border_radius: BorderRadius::all(px(11)), ..default() },
                BackgroundColor(TEXT),
                Pickable::IGNORE,
            ));
        });
}

fn spawn_segments(p: &mut ChildSpawnerCommands, field: Field, options: &[&str]) {
    p.spawn((
        Node { width: percent(100), column_gap: px(4), ..default() },
        Pickable::IGNORE,
    ))
    .with_children(|r| {
        for (i, o) in options.iter().enumerate() {
            let mut seg = r.spawn((
                Button,
                Segment { field, index: i },
                segment_look(),
                Node {
                    flex_grow: 1.0,
                    flex_basis: px(0),
                    height: px(40),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
                BackgroundColor(CLEAR),
                BorderColor::all(CLEAR),
            ));
            if field == Field::CrosshairColor {
                let c = CROSSHAIR_COLORS[i].1;
                seg.with_child((
                    Node { width: px(20), height: px(20), border_radius: BorderRadius::all(px(10)), ..default() },
                    BackgroundColor(Color::srgb(c[0], c[1], c[2])),
                    Pickable::IGNORE,
                ));
            } else {
                seg.with_child(text(o.to_uppercase(), label(19.0), TEXT_MUTED));
            }
        }
    });
}

fn spawn_slider(p: &mut ChildSpawnerCommands, field: Field) {
    p.spawn((Node { align_items: AlignItems::Center, column_gap: px(GAP_M), ..default() }, Pickable::IGNORE))
        .with_children(|r| {
            r.spawn((
                Button,
                SliderTrack(field),
                RelativeCursorPosition::default(),
                // Tall invisible hit area around a thin track.
                Node { width: px(340), height: px(30), align_items: AlignItems::Center, ..default() },
            ))
            .with_children(|t| {
                t.spawn((
                    Node { width: percent(100), height: px(6), border_radius: BorderRadius::all(px(3)), ..default() },
                    BackgroundColor(RAISED),
                    Pickable::IGNORE,
                ))
                .with_child((
                    SliderFill,
                    Node { width: percent(50), height: percent(100), border_radius: BorderRadius::all(px(3)), ..default() },
                    BackgroundColor(ACCENT),
                    Pickable::IGNORE,
                ));
                t.spawn((
                    SliderKnob,
                    Node {
                        position_type: PositionType::Absolute,
                        left: percent(50),
                        width: px(18),
                        height: px(18),
                        margin: UiRect::left(px(-9)),
                        border_radius: BorderRadius::all(px(9)),
                        border: UiRect::all(px(2)),
                        ..default()
                    },
                    BackgroundColor(TEXT),
                    BorderColor::all(ACCENT),
                    Pickable::IGNORE,
                ));
            });
            r.spawn((
                FieldValue(field),
                text("", body_bold(SIZE_BODY), TEXT),
                KeepColor,
                TextLayout::justify(Justify::Right),
                Node { width: px(64), ..default() },
            ));
        });
}

/// Large selectable cards for a choice field (match setup): title + blurb.
pub fn card_row(p: &mut ChildSpawnerCommands, field: Field, order: u32, cards: &[(&str, &str)], card_w: f32) {
    p.spawn((
        Button,
        FieldRow(field),
        Focusable { order },
        Look::new([CLEAR; 3], [CLEAR; 3], [TEXT; 3]),
        Node { column_gap: px(GAP_M), ..default() },
        BackgroundColor(CLEAR),
    ))
    .with_children(|r| {
        for (i, (title, blurb)) in cards.iter().enumerate() {
            r.spawn((
                Button,
                Segment { field, index: i },
                card_look(),
                Node {
                    width: px(card_w),
                    min_height: px(132),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(GAP_S),
                    padding: UiRect::new(px(20), px(20), px(18), px(18)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(RADIUS)),
                    ..default()
                },
                BackgroundColor(CLEAR),
                BorderColor::all(CLEAR),
            ))
            .with_children(|c| {
                c.spawn((
                    LookBg([LINE, LINE_STRONG, ACCENT]),
                    Node { width: px(28), height: px(3), margin: UiRect::bottom(px(4)), ..default() },
                    BackgroundColor(LINE),
                    Pickable::IGNORE,
                ));
                c.spawn(text(title.to_uppercase(), display(32.0), TEXT_MUTED));
                c.spawn((text(*blurb, body(SIZE_SMALL), TEXT_MUTED), KeepColor));
            });
        }
    });
}

// ---- plugin ----

pub struct WidgetsPlugin;

impl Plugin for WidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NavQueue>()
            .init_resource::<Focus>()
            .init_resource::<Dragging>()
            .add_message::<UiActivated>()
            .add_message::<NavBack>()
            .add_systems(PreUpdate, keyboard_nav.run_if(has_visuals).after(bevy::input::InputSystems))
            .add_systems(
                Update,
                (mouse_focus, press_widgets, drag_sliders, (validate_focus, apply_nav).chain(), refresh_fields, paint)
                    .chain()
                    .in_set(WidgetSet)
                    .run_if(has_visuals),
            );
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct WidgetSet;

fn keyboard_nav(keys: Res<ButtonInput<KeyCode>>, mut q: ResMut<NavQueue>) {
    let map = [
        (KeyCode::ArrowUp, NavInput::Up),
        (KeyCode::KeyW, NavInput::Up),
        (KeyCode::ArrowDown, NavInput::Down),
        (KeyCode::KeyS, NavInput::Down),
        (KeyCode::ArrowLeft, NavInput::Left),
        (KeyCode::KeyA, NavInput::Left),
        (KeyCode::ArrowRight, NavInput::Right),
        (KeyCode::KeyD, NavInput::Right),
        (KeyCode::Enter, NavInput::Accept),
        (KeyCode::NumpadEnter, NavInput::Accept),
        (KeyCode::Space, NavInput::Accept),
        (KeyCode::Escape, NavInput::Back),
        (KeyCode::Backspace, NavInput::Back),
        (KeyCode::KeyE, NavInput::NextTab),
        (KeyCode::KeyQ, NavInput::PrevTab),
    ];
    for (k, n) in map {
        if keys.just_pressed(k) {
            q.0.push(n);
        }
    }
    if keys.just_pressed(KeyCode::Tab) {
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        q.0.push(if shift { NavInput::PrevTab } else { NavInput::NextTab });
    }
}

/// Nearest `Focusable` at or above `e`.
fn focus_owner(e: Entity, focusables: &Query<&Focusable>, parents: &Query<&ChildOf>) -> Option<Entity> {
    let mut cur = e;
    for _ in 0..6 {
        if focusables.contains(cur) {
            return Some(cur);
        }
        cur = parents.get(cur).ok()?.parent();
    }
    None
}

/// Hovering a widget focuses it (only when the mouse actually moved, so a resting cursor
/// doesn't fight the keyboard).
fn mouse_focus(
    motion: Res<AccumulatedMouseMotion>,
    hovered: Query<(Entity, &Interaction), Changed<Interaction>>,
    all: Query<(Entity, &Interaction)>,
    focusables: Query<&Focusable>,
    parents: Query<&ChildOf>,
    mut focus: ResMut<Focus>,
) {
    let moved = motion.delta != Vec2::ZERO;
    let pick = |e: Entity, i: &Interaction| (*i != Interaction::None).then(|| focus_owner(e, &focusables, &parents)).flatten();
    let mut target = hovered.iter().find_map(|(e, i)| pick(e, i));
    if target.is_none() && moved {
        target = all.iter().find_map(|(e, i)| pick(e, i));
    }
    if let Some(t) = target
        && focus.0 != Some(t)
    {
        focus.0 = Some(t);
    }
}

fn press_widgets(
    buttons: Query<(&Interaction, &ActionButton), Changed<Interaction>>,
    segments: Query<(&Interaction, &Segment), Changed<Interaction>>,
    toggles: Query<(&Interaction, &ToggleSwitch), Changed<Interaction>>,
    tracks: Query<(Entity, &Interaction), (With<SliderTrack>, Changed<Interaction>)>,
    mut settings: ResMut<Settings>,
    mut dragging: ResMut<Dragging>,
    mut out: MessageWriter<UiActivated>,
) {
    for (i, b) in &buttons {
        if *i == Interaction::Pressed {
            out.write(UiActivated(b.0));
        }
    }
    for (i, s) in &segments {
        if *i == Interaction::Pressed && s.field.get(&settings) != s.index as f32 {
            s.field.set(&mut settings, s.index as f32);
        }
    }
    for (i, t) in &toggles {
        if *i == Interaction::Pressed {
            t.0.step(&mut settings, 1.0);
        }
    }
    for (e, i) in &tracks {
        if *i == Interaction::Pressed {
            dragging.0 = Some(e);
        }
    }
}

fn drag_sliders(
    mouse: Res<ButtonInput<MouseButton>>,
    mut dragging: ResMut<Dragging>,
    tracks: Query<(&SliderTrack, &RelativeCursorPosition)>,
    mut settings: ResMut<Settings>,
) {
    let Some(e) = dragging.0 else { return };
    if !mouse.pressed(MouseButton::Left) {
        dragging.0 = None;
        return;
    }
    let Ok((track, pos)) = tracks.get(e) else {
        dragging.0 = None;
        return;
    };
    let Some(n) = pos.normalized else { return };
    if let FieldKind::Slider { min, max, .. } = track.0.kind() {
        // `normalized` is centred on the node: -0.5..0.5.
        let t = (n.x + 0.5).clamp(0.0, 1.0);
        let v = min + (max - min) * t;
        let before = track.0.get(&settings);
        let mut probe = settings.clone();
        track.0.set(&mut probe, v);
        if track.0.get(&probe) != before {
            track.0.set(&mut settings, v);
        }
    }
}

/// Keep focus on a live entity; default to the first item.
fn validate_focus(mut focus: ResMut<Focus>, q: Query<(Entity, &Focusable)>) {
    if focus.0.is_some_and(|e| q.contains(e)) {
        return;
    }
    let first = q.iter().min_by_key(|(_, f)| f.order).map(|(e, _)| e);
    if focus.0 != first {
        focus.0 = first;
    }
}

/// Move focus / adjust fields / activate buttons. Back and tab switching become actions.
fn apply_nav(
    mut queue: ResMut<NavQueue>,
    mut focus: ResMut<Focus>,
    focusables: Query<(Entity, &Focusable)>,
    buttons: Query<&ActionButton>,
    rows: Query<&FieldRow>,
    mut settings: ResMut<Settings>,
    mut out: MessageWriter<UiActivated>,
    mut back: MessageWriter<NavBack>,
) {
    let inputs: Vec<NavInput> = queue.0.drain(..).collect();
    for input in inputs {
        let cur = focus.0.and_then(|e| focusables.get(e).ok());
        match input {
            NavInput::Up | NavInput::Down => {
                let Some((_, f)) = cur else { continue };
                let next = if input == NavInput::Down {
                    focusables.iter().filter(|(_, o)| o.order > f.order).min_by_key(|(_, o)| o.order)
                } else {
                    focusables.iter().filter(|(_, o)| o.order < f.order).max_by_key(|(_, o)| o.order)
                };
                if let Some((e, _)) = next {
                    focus.0 = Some(e);
                }
            }
            NavInput::Left | NavInput::Right => {
                let dir = if input == NavInput::Right { 1.0 } else { -1.0 };
                // Toggles: Right = on, Left = off.
                if let Some(row) = cur.and_then(|(e, _)| rows.get(e).ok())
                    && (!matches!(row.0.kind(), FieldKind::Toggle) || (row.0.get(&settings) > 0.5) != (dir > 0.0))
                {
                    row.0.step(&mut settings, dir);
                }
            }
            NavInput::Accept => {
                let Some((e, _)) = cur else { continue };
                if let Ok(b) = buttons.get(e) {
                    out.write(UiActivated(b.0));
                } else if let Ok(row) = rows.get(e) {
                    match row.0.kind() {
                        FieldKind::Toggle => row.0.step(&mut settings, 1.0),
                        FieldKind::Choice(o) => {
                            let v = row.0.get(&settings) as usize;
                            row.0.set(&mut settings, ((v + 1) % o.len()) as f32);
                        }
                        FieldKind::Slider { .. } => {}
                    }
                }
            }
            NavInput::Back => {
                back.write(NavBack);
            }
            NavInput::NextTab | NavInput::PrevTab => {
                out.write(UiActivated(UiAction::Tab(if input == NavInput::NextTab { usize::MAX } else { usize::MAX - 1 })));
            }
        }
    }
}

/// Esc / Backspace (or `ui.nav back`). Handled by `ui::screens` depending on what is open.
#[derive(Message, Clone, Copy, Debug)]
pub struct NavBack;

/// Sync field widgets with `Settings` (cheap; runs every frame while widgets exist).
fn refresh_fields(
    settings: Res<Settings>,
    mut toggles: Query<(&ToggleSwitch, &mut Node, &mut BackgroundColor), Without<SliderKnob>>,
    mut knobs: Query<(&SliderTrack, &Children)>,
    mut fills: Query<&mut Node, (With<SliderFill>, Without<ToggleSwitch>, Without<SliderKnob>)>,
    mut knob_nodes: Query<&mut Node, (With<SliderKnob>, Without<ToggleSwitch>, Without<SliderFill>)>,
    children: Query<&Children>,
    mut values: Query<(&FieldValue, &mut Text)>,
) {
    for (t, mut node, mut bg) in &mut toggles {
        let on = t.0.get(&settings) > 0.5;
        let j = if on { JustifyContent::FlexEnd } else { JustifyContent::FlexStart };
        if node.justify_content != j {
            node.justify_content = j;
        }
        let c = if on { ACCENT } else { RAISED };
        if bg.0 != c {
            bg.0 = c;
        }
    }
    for (track, kids) in &mut knobs {
        let FieldKind::Slider { min, max, .. } = track.0.kind() else { continue };
        let t = ((track.0.get(&settings) - min) / (max - min)).clamp(0.0, 1.0) * 100.0;
        for k in kids.iter() {
            if let Ok(mut n) = knob_nodes.get_mut(k)
                && n.left != percent(t)
            {
                n.left = percent(t);
            }
            for g in children.iter_descendants(k) {
                if let Ok(mut n) = fills.get_mut(g)
                    && n.width != percent(t)
                {
                    n.width = percent(t);
                }
            }
        }
    }
    for (f, mut text) in &mut values {
        let v = f.0.get(&settings);
        let s = match f.0.kind() {
            FieldKind::Toggle => if v > 0.5 { "ON" } else { "OFF" }.to_string(),
            _ => f.0.format(v),
        };
        if text.0 != s {
            text.0 = s;
        }
    }
}

/// Apply [`Look`] colours from hover/focus/selection.
fn paint(
    focus: Res<Focus>,
    settings: Res<Settings>,
    looks: Query<(Entity, &Look, Option<&Interaction>, Option<&Segment>, Option<&TabButton>)>,
    rows: Query<&FieldRow>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    has_look: Query<(), With<Look>>,
    mut bgs: Query<&mut BackgroundColor>,
    mut borders: Query<&mut BorderColor>,
    mut texts: Query<&mut TextColor, Without<KeepColor>>,
    look_bgs: Query<&LookBg>,
) {
    for (e, look, interaction, segment, tab) in &looks {
        let hovered = interaction.is_some_and(|i| *i != Interaction::None);
        let mut hot = hovered || focus.0 == Some(e);
        let on = match (segment, tab) {
            (Some(s), _) => s.field.get(&settings) as usize == s.index,
            (_, Some(t)) => t.active,
            _ => false,
        };
        // The selected option of a focused row counts as hot (keyboard focus indicator).
        if let Some(s) = segment
            && on
        {
            hot |= ancestors(e, &parents).any(|a| focus.0 == Some(a) && rows.get(a).is_ok_and(|r| r.0 == s.field));
        }
        let idx = if on { 2 } else if hot { 1 } else { 0 };
        if let Ok(mut bg) = bgs.get_mut(e)
            && bg.0 != look.bg[idx]
        {
            bg.0 = look.bg[idx];
        }
        // Selected but not focused: a softer outline.
        let border = if on && !hot { look.border[2].with_alpha(look.border[2].alpha() * 0.5) } else { look.border[idx] };
        if let Ok(mut b) = borders.get_mut(e)
            && b.top != border
        {
            *b = BorderColor::all(border);
        }
        // Subtree (stop at nested Looks).
        let mut stack: Vec<Entity> = children.get(e).map(|c| c.iter().collect()).unwrap_or_default();
        while let Some(c) = stack.pop() {
            if has_look.contains(c) {
                continue;
            }
            if let Ok(mut t) = texts.get_mut(c)
                && t.0 != look.text[idx]
            {
                t.0 = look.text[idx];
            }
            if let Ok(lb) = look_bgs.get(c)
                && let Ok(mut bg) = bgs.get_mut(c)
            {
                let want = if on { lb.0[2] } else if hot { lb.0[1] } else { lb.0[0] };
                if bg.0 != want {
                    bg.0 = want;
                }
            }
            if let Ok(cc) = children.get(c) {
                stack.extend(cc.iter());
            }
        }
    }
}

fn ancestors<'a>(e: Entity, parents: &'a Query<&ChildOf>) -> impl Iterator<Item = Entity> + 'a {
    let mut cur = Some(e);
    std::iter::from_fn(move || {
        let p = parents.get(cur?).ok()?.parent();
        cur = Some(p);
        Some(p)
    })
}

/// Index helper for the tab strip.
pub fn wrap_tab(cur: usize, req: usize, n: usize) -> usize {
    match req {
        usize::MAX => (cur + 1) % n,
        r if r == usize::MAX - 1 => (cur + n - 1) % n,
        r => r.min(n - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_round_trip() {
        let mut s = Settings::default();
        for f in Field::ALL {
            let v = f.get(&s);
            f.set(&mut s, v);
            assert_eq!(f.get(&s), v, "{f:?}");
        }
        Field::Fov.set(&mut s, 500.0);
        assert_eq!(s.fov_deg, 110.0);
        Field::Master.step(&mut s, -1.0);
        assert!((s.master_volume - 0.75).abs() < 1e-4);
        Field::Quality.set(&mut s, 0.0);
        assert_eq!(s.quality, GraphicsQuality::Low);
        Field::CrosshairColor.set(&mut s, 3.0);
        assert_eq!(Field::CrosshairColor.get(&s), 3.0);
        assert_eq!(Field::from_key("vsync"), Some(Field::Vsync));
    }

    #[test]
    fn tabs_wrap() {
        assert_eq!(wrap_tab(3, usize::MAX, 4), 0);
        assert_eq!(wrap_tab(0, usize::MAX - 1, 4), 3);
        assert_eq!(wrap_tab(0, 2, 4), 2);
    }
}
