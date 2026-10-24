//! Shroud's palette as constants for the HUD and in-game overlays. Everything else (type scale,
//! spacing, embedded fonts, helpers, UI scaling) is the engine's UI kit, whose default
//! `Theme` is this same palette.

use bevy::prelude::*;
pub use engine::ui::theme::*;

/// Near-black blue used behind menus.
pub const BG: Color = Color::srgb(0.035, 0.043, 0.06);
/// Panels and cards.
pub const PANEL: Color = Color::srgba(0.07, 0.085, 0.11, 0.88);
/// Raised surface.
pub const RAISED: Color = Color::srgba(0.13, 0.15, 0.19, 0.95);
/// Hairlines and idle borders.
pub const LINE: Color = Color::srgba(1.0, 1.0, 1.0, 0.09);

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The constants and the engine theme must not drift apart.
    #[test]
    fn palette_matches_engine_theme() {
        let t = Theme::default();
        for (a, b) in [(BG, t.bg), (PANEL, t.panel), (LINE, t.line), (TEXT, t.text), (ACCENT, t.accent), (FRIENDLY, t.good), (ENEMY, t.bad), (INTEL, t.info)] {
            assert_eq!(a, b);
        }
    }
}
