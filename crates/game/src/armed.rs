//! Which gun an actor (enemy or player root) holds. Shared by weapons (pickups, drops) and AI (loadouts, kill feed, telemetry).
use bevy::prelude::*;
pub use brain::weapons::WeaponKind;
/// The gun an actor's root currently carries as its primary / held weapon.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Armed(pub WeaponKind);
