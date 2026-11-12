//! The player's weapon light: the engine's `WeaponLight::tactical()` (a shadowed spot plus a dim
//! unshadowed spill, engine docs/GAME_GUIDE.md 3.4), switched with **T**.
//!
//! It hangs off the camera like the engine recommends, but rides the gun: every frame it moves to
//! just ahead of and under the muzzle of the gun in hand and points along the barrel
//! ([`MuzzleCam`]). In Bodycam the barrel moves inside the free-aim box, so the beam's hot spot
//! is where the shots go, and the light sits ~20-30 cm off the lens, so every prop throws a thin
//! hard shadow the camera can see. Ahead of the muzzle, it doesn't light the gun.
//!
//! It is one layer of the sun's shadow texture: `map::look::apply_light_budget` never culls it
//! and gives the map lamps one shadow fewer while it is on, so the number of shadowed spots in
//! view stays steady. It only shines while a round is live (not in menus).

use bevy::prelude::*;
use engine::gfx::MainCamera;
use engine::look::WeaponLight;

use super::viewmodel::MuzzleCam;
use super::WeaponSet;
use crate::state::{GameState, has_visuals, not_paused};

/// Whether the player switched the light on (kept across rounds).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct LightSwitch {
    pub on: bool,
}

/// The player's weapon light (a child of the main camera).
#[derive(Component)]
pub struct PlayerLight;

/// Where the light sits relative to the muzzle: a little ahead (so it doesn't light the gun) and
/// under the barrel, on the rail (camera space, metres).
const AHEAD: f32 = 0.06;
const BELOW: f32 = 0.035;

pub(super) fn build(app: &mut App) {
    app.init_resource::<LightSwitch>()
        .add_systems(Update, (spawn_light, toggle.run_if(in_state(GameState::Playing).and_then(not_paused)).after(WeaponSet)).run_if(has_visuals))
        // After the viewmodel moved the gun (Update) and after the engine re-aimed the light on a
        // change (its `sync_weapon_lights`, Update), before transforms propagate.
        .add_systems(PostUpdate, follow_gun.before(bevy::transform::TransformSystems::Propagate).run_if(has_visuals));
    #[cfg(feature = "dev")]
    {
        use engine::devtools::{AddControlCommand, params};
        app.add_control_command(
            "player.light",
            "The player's weapon light (T): switch it on/off, or omit `on` to read it",
            params(&[("on", "boolean", "switch (omit: read)")]),
            |world: &mut World, args| {
                if let Some(on) = args.get("on").and_then(|v| v.as_bool()) {
                    world.resource_mut::<LightSwitch>().on = on;
                }
                let on = world.resource::<LightSwitch>().on;
                let lit = world.query_filtered::<&WeaponLight, With<PlayerLight>>().iter(world).any(|l| l.on);
                Ok(serde_json::json!({ "on": on, "lit": lit }))
            },
        );
    }
}

fn spawn_light(mut commands: Commands, cams: Query<Entity, With<MainCamera>>, lights: Query<(), With<PlayerLight>>) {
    if !lights.is_empty() {
        return;
    }
    let Ok(cam) = cams.single() else { return };
    commands.spawn((Name::new("Weapon light"), PlayerLight, WeaponLight { on: false, ..WeaponLight::tactical() }, ChildOf(cam)));
}

fn toggle(keys: Option<Res<ButtonInput<KeyCode>>>, buy: Option<Res<crate::economy::BuyMenu>>, mut switch: ResMut<LightSwitch>) {
    let pressed = keys.is_some_and(|k| k.just_pressed(KeyCode::KeyT));
    if pressed && !buy.is_some_and(|b| b.open) {
        switch.on = !switch.on;
    }
}

/// The light's place for a muzzle `m` and barrel direction `barrel` (camera space).
pub fn rail_transform(m: Vec3, barrel: Vec3) -> Transform {
    let b = barrel.normalize_or(Vec3::NEG_Z);
    // "Under the barrel": down in the gun's frame, approximated by camera down made square to it.
    let down = (Vec3::NEG_Y - b * b.dot(Vec3::NEG_Y)).normalize_or(Vec3::NEG_Y);
    Transform::from_translation(m + b * AHEAD + down * BELOW).looking_to(b, Vec3::Y)
}

fn follow_gun(
    state: Option<Res<State<GameState>>>,
    switch: Res<LightSwitch>,
    muzzle: Res<MuzzleCam>,
    mut lights: Query<(&mut WeaponLight, &mut Transform), With<PlayerLight>>,
) {
    let want = switch.on && state.is_some_and(|s| *s.get() == GameState::Playing);
    for (mut light, mut tf) in &mut lights {
        if light.on != want {
            light.on = want;
        }
        // The engine aims the beam at the line of sight (`converge`) whenever the light changes;
        // here it follows the barrel instead, every frame.
        let t = rail_transform(muzzle.0, muzzle.1);
        if *tf != t {
            *tf = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_beam_runs_along_the_barrel_just_ahead_of_the_muzzle() {
        let m = Vec3::new(0.2, -0.2, -0.6);
        let barrel = Vec3::new(0.1, 0.0, -1.0).normalize();
        let t = rail_transform(m, barrel);
        assert!(t.forward().dot(barrel) > 0.9999);
        let d = t.translation - m;
        // Ahead of the muzzle along the barrel, and below it.
        assert!((d.dot(barrel) - AHEAD).abs() < 1e-4);
        assert!(d.y < -0.03);
    }
}
