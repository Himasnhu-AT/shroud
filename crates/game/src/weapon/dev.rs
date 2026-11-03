//! Control-port commands for the arsenal (feature `dev`): `weapon.give`, `weapon.switch`,
//! `weapon.ads`, `weapon.inspect`, `weapon.interact`, `weapon.drop_pickup`, `weapon.state`.
//! `player.fire` (in `dev::commands`) fires whatever gun is in hand.

use bevy::prelude::*;
use engine::devtools::{AddControlCommand, params};
use serde_json::{Value, json};

use super::arsenal::{GunState, Phase, Weapon, WeaponOverride, draw_time};
use super::pickups::{PickupAssets, PickupFocus, spawn_pickup};
use super::{Armed, Arsenal, Slot, parse_kind};
use crate::player::Player;

pub(super) fn build(app: &mut App) {
    app.add_control_command(
        "weapon.give",
        "Give the player a gun (fills its slot with full ammo and draws it)",
        params(&[("kind", "string", "pistol | heavypistol | smg | rifle | battlerifle | shotgun | marksman | sniper (or a gun name: Warden, Shrike, Osprey ...)"), ("draw", "boolean", "switch to it (default true)")]),
        give,
    )
    .add_control_command("weapon.switch", "Switch to a slot (1 = primary, 2 = sidearm)", params(&[("slot", "integer", "1 or 2")]), switch)
    .add_control_command("weapon.ads", "Hold or release aim down sights", params(&[("on", "boolean", "default true")]), ads)
    .add_control_command("weapon.inspect", "Inspect the gun in hand", params(&[]), inspect)
    .add_control_command("weapon.interact", "Hold or release E (pickups)", params(&[("on", "boolean", "default true")]), interact)
    .add_control_command(
        "weapon.drop_pickup",
        "Put a gun on the ground",
        params(&[("kind", "string", "gun"), ("x", "number", "world x"), ("z", "number", "world z")]),
        drop_pickup,
    )
    .add_control_command("weapon.state", "The gun in hand and the arsenal", params(&[]), state);
}

fn player(world: &mut World) -> Result<Entity, String> {
    world.query_filtered::<Entity, With<Player>>().iter(world).next().ok_or_else(|| "no player".to_string())
}

fn kind_param(p: &Value) -> Result<super::WeaponKind, String> {
    let s = p.get("kind").and_then(Value::as_str).ok_or("`kind` (string) is required")?;
    parse_kind(s).ok_or_else(|| format!("unknown gun `{s}`"))
}

fn give(world: &mut World, p: Value) -> Result<Value, String> {
    let kind = kind_param(&p)?;
    let draw = p.get("draw").and_then(Value::as_bool).unwrap_or(true);
    let e = player(world)?;
    let slot = if kind.spec().slot == Slot::Primary { 0 } else { 1 };
    let gun = GunState::full(kind);
    let mut ent = world.entity_mut(e);
    let active = {
        let mut a = ent.get_mut::<Arsenal>().ok_or("no arsenal")?;
        a.slots[slot] = Some(gun);
        if draw && a.active != slot {
            a.last = a.active;
            a.active = slot;
        }
        a.active
    };
    if active == slot {
        let mut w = Weapon::new(gun);
        w.phase = Phase::Drawing { left: draw_time(kind), total: draw_time(kind) };
        ent.insert((w, Armed(kind)));
    }
    Ok(json!({"kind": format!("{kind:?}"), "slot": slot + 1, "active": active + 1}))
}

fn switch(world: &mut World, p: Value) -> Result<Value, String> {
    let slot = p.get("slot").and_then(Value::as_u64).ok_or("`slot` (1 or 2) is required")?;
    if !(1..=2).contains(&slot) {
        return Err("`slot` must be 1 or 2".into());
    }
    world.resource_mut::<WeaponOverride>().select = Some(slot as usize - 1);
    Ok(json!({"slot": slot}))
}

fn ads(world: &mut World, p: Value) -> Result<Value, String> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    world.resource_mut::<WeaponOverride>().ads = Some(on);
    Ok(json!({"ads": on}))
}

fn inspect(world: &mut World, _: Value) -> Result<Value, String> {
    world.resource_mut::<WeaponOverride>().inspect = true;
    Ok(json!({}))
}

fn interact(world: &mut World, p: Value) -> Result<Value, String> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    world.resource_mut::<WeaponOverride>().interact = Some(on);
    Ok(json!({"interact": on}))
}

fn drop_pickup(world: &mut World, p: Value) -> Result<Value, String> {
    let kind = kind_param(&p)?;
    let x = p.get("x").and_then(Value::as_f64).ok_or("`x` is required")? as f32;
    let z = p.get("z").and_then(Value::as_f64).ok_or("`z` is required")? as f32;
    let assets = world.remove_resource::<PickupAssets>();
    let e = {
        let mut commands = world.commands();
        spawn_pickup(&mut commands, assets.as_ref(), GunState::full(kind), Vec3::new(x, 0.0, z))
    };
    world.flush();
    if let Some(a) = assets {
        world.insert_resource(a);
    }
    Ok(json!({"entity": e.to_bits(), "kind": format!("{kind:?}")}))
}

fn state(world: &mut World, _: Value) -> Result<Value, String> {
    let e = player(world)?;
    let focus = *world.resource::<PickupFocus>();
    let ent = world.entity(e);
    let w = ent.get::<Weapon>().ok_or("no weapon")?;
    let a = ent.get::<Arsenal>().ok_or("no arsenal")?;
    let slots: Vec<Value> = a
        .slots
        .iter()
        .map(|g| g.map_or(Value::Null, |g| json!({"kind": format!("{:?}", g.kind), "ammo": g.ammo, "reserve": g.reserve})))
        .collect();
    Ok(json!({
        "kind": format!("{:?}", w.kind),
        "name": w.kind.label(),
        "ammo": w.ammo,
        "mag": w.mag,
        "reserve": w.reserve,
        "spread": w.spread,
        "spread_deg": w.spread.to_degrees(),
        "ads": w.ads,
        "scoped": w.scoped(),
        "optic": w.sighted().map(|o| format!("{o:?}")),
        "reloading": w.reloading.is_some(),
        "reload_kind": format!("{:?}", w.reload_kind),
        "phase": format!("{:?}", w.phase),
        "sprint": w.sprint,
        "active_slot": a.active + 1,
        "slots": slots,
        "pickup_focus": focus.target.map(|(_, k)| format!("{k:?}")),
    }))
}
