//! Bodycam optics: the engine's body camera (`engine::look::BodycamLook`, engine docs/LOOK.md
//! "Body camera") switched on in the map's [`Look`]. The engine renders it: a true fisheye at
//! the wide FOV ([`super::BODYCAM_FOV_DEG`], also used by the ADS zoom in `weapon::arsenal`),
//! a metering exposure that lags, overshoots and clips (it replaces Bevy's auto exposure on the
//! camera), crushed blacks and hard-clipped highlights, sensor noise that grows in the dark,
//! sharpening halos, motion smear, compression blocks, rolling shutter, edge chromatic
//! aberration, a vignette and halation (bloom from what clips, where the tier has bloom).
//! Its pipelines are warmed at render startup and are not in any material's pipeline key, so
//! switching Classic ↔ Bodycam mid-play rebuilds nothing.
//!
//! UI is drawn after the pass (the [REC] stamp stays straight). Anything placed on screen from a
//! world point goes through the lens with `engine::look::world_to_screen` (the hit marker,
//! `ui::hud`).

use engine::look::{BodycamLook, Look, ToneMap};

/// The chest camera Tells uses: the engine's `chest_cam` preset, tuned against the reference
/// footage (daylight from a doorway blows out to white, interiors fall to near black, a slow
/// swing of exposure when walking between them).
pub fn chest_cam() -> BodycamLook {
    let mut b = BodycamLook::chest_cam();
    // A shade less magnification in the centre than the full fisheye: the reference camera
    // bends the frame edges hard but keeps the middle of a room readable at range.
    b.fisheye.strength = 0.85;
    // Metered a touch over mid-grey: sunlit concrete clips hot (the reference look) while shade
    // keeps detail. The sensor's gain still runs out at EV 3, so dark rooms stay dark and noisy.
    b.exposure.compensation = 0.3;
    b.saturation = 0.8;
    b
}

/// A map's look as the bodycam sees it: the engine body camera on, its metering exposure in
/// charge (no histogram auto exposure), AgX. The look's own lens effects (vignette, CA,
/// distortion, grain) are off: the bodycam pass does them.
pub fn bodycam_look(mut look: Look) -> Look {
    look.name = format!("{} (bodycam)", look.name).into();
    look.lens.bodycam = chest_cam();
    look.exposure.auto = false;
    look.tonemapping = ToneMap::Agx;
    look.lens.vignette.intensity = 0.0;
    look.lens.chromatic_aberration = 0.0;
    look.lens.distortion = 0.0;
    look.lens.film_grain = 0.0;
    look
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodycam_look_turns_on_the_engine_camera() {
        let base = Look::default();
        let b = bodycam_look(base.clone());
        assert_ne!(b.name, base.name);
        assert!(b.lens.bodycam.enabled && b.lens.bodycam.exposure.enabled);
        // The engine meter is the only exposure control on the camera.
        assert!(!b.exposure.auto);
        assert_eq!(b.lens.film_grain, 0.0);
        assert!(!base.lens.bodycam.enabled);
    }
}
