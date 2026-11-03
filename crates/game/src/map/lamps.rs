//! Breakable lights.
//!
//! Every map lamp (`brain::arena::Decor::lamps`; its index is the lamp id the AI uses) can be
//! shot out by anyone's bullet: a [`BulletPath`] (made from every `ShotFired`, player or enemy)
//! that crosses the fixture's box ([`brain::arena::Lamp::fixture`]) kills its light, swaps the
//! glowing panel or bulb for dark glass, and throws sparks, a pop and a few shards. Lamps stay
//! out for the round and come back at the next round start.
//!
//! Gameplay lives in `ArenaRes` (`Arena::dark_lamps`), so it works headless too; the doctrine
//! reads it (`Arena::lamps_lighting`). Broken lights drop out of the nearest-K shadow budget
//! (`look::apply_light_budget` skips [`LampOut`]), so the next lamp in line gets the shadow.

use bevy::prelude::*;

use super::glass::{BulletPath, ShardBurst, segment_hits_box};
use super::{ArenaRes, MapSet};
use crate::state::GameState;
use crate::weapon::{Impact, Surface};

/// On a map lamp's light (spot or point): its id and its lit intensity.
#[derive(Component, Clone, Copy, Debug)]
pub struct LampLight {
    pub id: usize,
    pub power: f32,
}

/// On a map lamp's glowing part: its id and the lit / shot-out materials.
#[derive(Component, Clone, Debug)]
pub struct LampGlow {
    pub id: usize,
    pub lit: Handle<StandardMaterial>,
    pub dead: Handle<StandardMaterial>,
}

/// On a lamp's light and glow while it is shot out.
#[derive(Component, Clone, Copy, Debug)]
pub struct LampOut;

/// Lamp `id` was just shot out.
#[derive(Message, Clone, Copy, Debug)]
pub struct LampBroken {
    pub id: usize,
}

pub struct LampsPlugin;

impl Plugin for LampsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LampBroken>()
            .add_systems(Update, (break_lamps, show_broken).chain())
            .add_systems(OnEnter(GameState::Playing), restore_lamps.in_set(MapSet).after(super::sync_map));
        #[cfg(feature = "dev")]
        {
            use engine::devtools::{AddControlCommand, params};
            app.add_control_command("lamp.info", "Map lamps: count, broken ids, and the ones lighting the player", params(&[]), |world: &mut World, _| {
                let player = world.query_filtered::<&Transform, With<crate::player::Player>>().iter(world).next().map(|t| t.translation);
                let a = &world.resource::<ArenaRes>().0;
                let lighting = player.map(|p| a.lamps_lighting(brain::math::V2::new(p.x, p.z))).unwrap_or_default();
                let lamps: Vec<_> = a
                    .decor
                    .lamps
                    .iter()
                    .map(|l| {
                        let (at, h) = l.fixture();
                        [at.x, h, at.y]
                    })
                    .collect();
                Ok(serde_json::json!({ "lamps": lamps.len(), "broken": a.dark_lamps, "lighting_player": lighting, "fixtures": lamps }))
            })
            .add_control_command("lamp.break", "Shoot out lamp `id`", params(&[("id", "int", "lamp id (lamp.info)")]), |world: &mut World, args| {
                let id = args.get("id").and_then(|v| v.as_u64()).ok_or("id required")? as usize;
                let a = &world.resource::<ArenaRes>().0;
                let l = a.decor.lamps.get(id).copied().ok_or("no such lamp")?;
                let (at, h) = l.fixture();
                let at = Vec3::new(at.x, h, at.y);
                if a.lamp_lit(id) {
                    world.resource_mut::<ArenaRes>().0.dark_lamps.push(id);
                    world.write_message(LampBroken { id });
                }
                Ok(serde_json::json!({ "id": id, "at": [at.x, at.y, at.z] }))
            });
        }
    }
}

/// The fixture box of lamp `l` in world space (centre, half extents).
pub fn fixture_box(l: &brain::arena::Lamp) -> (Vec3, Vec3) {
    let (at, h) = l.fixture();
    let [x, y, z] = l.fixture_half();
    (Vec3::new(at.x, h, at.y), Vec3::new(x, y, z))
}

/// Any bullet whose path crosses a lit fixture puts that lamp out (gameplay: `Arena::dark_lamps`).
fn break_lamps(mut paths: MessageReader<BulletPath>, mut arena: ResMut<ArenaRes>, mut out: MessageWriter<LampBroken>, mut impacts: MessageWriter<Impact>, dev: crate::dev::Dev) {
    if arena.0.decor.lamps.is_empty() {
        paths.clear();
        return;
    }
    for p in paths.read() {
        let mut hit: Option<(f32, usize, Vec3)> = None;
        for (id, l) in arena.0.decor.lamps.iter().enumerate() {
            if !arena.0.lamp_lit(id) {
                continue;
            }
            let (c, half) = fixture_box(l);
            // A little slack: a shot that stops on the ceiling right at the fixture still counts.
            let half = half + Vec3::splat(0.03);
            if !segment_hits_box(p.origin, p.dir, p.max_dist + 0.05, c, half) {
                continue;
            }
            let d = (c - p.origin).dot(p.dir).max(0.0);
            if hit.is_none_or(|h| d < h.0) {
                hit = Some((d, id, c));
            }
        }
        // One bullet, one lamp (the first it reaches).
        let Some((d, id, c)) = hit else { continue };
        arena.0.dark_lamps.push(id);
        let at = (p.origin + p.dir * d).lerp(c, 0.5);
        out.write(LampBroken { id });
        impacts.write(Impact { pos: at, normal: -p.dir, flesh: false, surface: Surface::Metal, exit: false });
        info!("lamp.broken {{ id: {id} }} at ({:.1}, {:.1}, {:.1})", c.x, c.y, c.z);
        dev.log("lamp.broken", || serde_json::json!({ "id": id, "at": [c.x, c.y, c.z] }));
        dev.trace("lamp.broken", &[("id", crate::dev::Prop::Int(id as i64))]);
    }
}

/// Lights off, glass dark, a few shards.
fn show_broken(
    mut commands: Commands,
    mut broken: MessageReader<LampBroken>,
    arena: Res<ArenaRes>,
    mut lights: Query<(Entity, &LampLight, Option<&mut PointLight>, Option<&mut SpotLight>), Without<LampOut>>,
    mut glows: Query<(Entity, &LampGlow, &mut MeshMaterial3d<StandardMaterial>), Without<LampOut>>,
    mut shards: MessageWriter<ShardBurst>,
) {
    for b in broken.read() {
        for (e, l, point, spot) in &mut lights {
            if l.id != b.id {
                continue;
            }
            if let Some(mut p) = point {
                p.intensity = 0.0;
                p.shadow_maps_enabled = false;
            }
            if let Some(mut s) = spot {
                s.intensity = 0.0;
                s.shadow_maps_enabled = false;
            }
            commands.entity(e).insert((LampOut, Visibility::Hidden));
        }
        for (e, g, mut m) in &mut glows {
            if g.id == b.id {
                m.0 = g.dead.clone();
                commands.entity(e).insert(LampOut);
            }
        }
        if let Some(l) = arena.0.decor.lamps.get(b.id) {
            let (c, half) = fixture_box(l);
            shards.write(ShardBurst { at: c, half, count: 8 });
        }
    }
}

/// Round start: every lamp lit again.
fn restore_lamps(
    mut commands: Commands,
    mut arena: ResMut<ArenaRes>,
    mut lights: Query<(Entity, &LampLight, Option<&mut PointLight>, Option<&mut SpotLight>), With<LampOut>>,
    mut glows: Query<(Entity, &LampGlow, &mut MeshMaterial3d<StandardMaterial>), With<LampOut>>,
) {
    if !arena.0.dark_lamps.is_empty() {
        arena.0.dark_lamps.clear();
    }
    for (e, l, point, spot) in &mut lights {
        if let Some(mut p) = point {
            p.intensity = l.power;
        }
        if let Some(mut s) = spot {
            s.intensity = l.power;
        }
        commands.entity(e).remove::<LampOut>().insert(Visibility::Inherited);
    }
    for (e, g, mut m) in &mut glows {
        m.0 = g.lit.clone();
        commands.entity(e).remove::<LampOut>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain::arena::{Arena, MapId};

    /// A shot straight up into an Office panel crosses its box; one beside it doesn't.
    #[test]
    fn shots_meet_the_fixture_box() {
        let a = Arena::build(MapId::Office);
        let (c, half) = fixture_box(&a.decor.lamps[0]);
        let from = Vec3::new(c.x + 3.0, 1.5, c.z);
        let dir = (c - from).normalize();
        // The ray stops on the ceiling just past the fixture.
        let to_ceiling = (brain::arena::OFFICE_CEILING - from.y) / dir.y;
        assert!(segment_hits_box(from, dir, to_ceiling, c, half));
        let wide = (c + Vec3::new(0.0, 0.0, 1.5) - from).normalize();
        assert!(!segment_hits_box(from, wide, 20.0, c, half));
    }
}
