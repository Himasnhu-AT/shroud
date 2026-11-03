//! Control-port commands for enemies, the AI brain and the match (test tooling only).

use bevy::prelude::*;
use brain::arena::Arena;
use brain::bandit::CTX_LABELS;
use brain::difficulty::Difficulty;
use brain::evolve::{EvolveConfig, MatchReport};
use brain::grenade::GrenadeKind;
use serde_json::{Value, json};

use engine::devtools::{AddControlCommand, params};
use crate::enemy::grenade::{Flashed, Grenade, ThrowGrenade};
use crate::armed::Armed;
use crate::enemy::{Enemy, EnemyCombat, EnemyFreeze, GunState, Mind, Role, Squad};
use brain::weapons::WeaponKind;

use crate::map::{ArenaRes, to_ground};
use crate::match_flow::MatchFlow;
use crate::save::Brain;
use crate::telemetry::Telemetry;
use crate::weapon::Health;

pub fn register(app: &mut App) {
    let none = params(&[]);
    app.add_control_command("enemy.list", "List enemies", none.clone(), enemy_list)
        .add_control_command("enemy.freeze", "Freeze enemies", params(&[("on", "boolean", "freeze (default true)")]), enemy_freeze)
        .add_control_command("enemy.kill", "Kill enemies", params(&[("id", "integer", "slot; all if omitted")]), enemy_kill)
        .add_control_command("match.state", "Match/round state", none.clone(), match_state)
        .add_control_command("ai.brain", "AI brain summary", none.clone(), ai_brain)
        .add_control_command("ai.evolve", "Evolve now from this match's data", none.clone(), ai_evolve)
        .add_control_command("ai.reset", "Reset the AI", none.clone(), ai_reset)
        .add_control_command("ai.squad", "Squad state: play, roles, tokens, re-peek spots, grenades, bandit scores", none.clone(), ai_squad)
        .add_control_command(
            "ai.doctrine",
            "Adaptive doctrine: tactic, reason, archetypes, counters, the player's style profile, switches this round, who jiggles / wallbangs / rushes, how many flashes",
            none.clone(),
            ai_doctrine,
        )
        .add_control_command(
            "ai.throw",
            "Make the squad throw a grenade at the player as soon as someone can (or, with `at`, throw one now from `from` to that point)",
            params(&[
                ("kind", "string", "frag | flash (default frag)"),
                ("at", "array", "[x, z] landing point: throw immediately (test hook)"),
                ("from", "array", "[x, y, z] release point for `at` (default 6 m short of it, 1.6 m up)"),
            ]),
            ai_throw,
        )
        .add_control_command(
            "enemy.loadout",
            "Hand enemies a primary now (test hook): `kind` for all (or `id`), or `kinds` by slot",
            params(&[
                ("kind", "string", "pistol | smg | rifle | shotgun | marksman"),
                ("id", "integer", "slot (default: every enemy)"),
                ("kinds", "array", "one kind per slot, e.g. [\"rifle\", \"smg\", \"shotgun\", \"marksman\"]"),
            ]),
            enemy_loadout,
        )
        .add_control_command(
            "match.time_limit",
            "Round time limit in seconds (omit or null: no limit). Headless runs never time out.",
            params(&[("seconds", "number", "limit; omit for none")]),
            match_time_limit,
        )
        .add_control_command(
            "ai.play",
            "Force the squad's play (test hook; omit `name` to hand it back to the bandit)",
            params(&[("name", "string", "trade | crossfire | pinflank | bait | overwatch | collapse | sweep | search | encircle | free")]),
            ai_play,
        )
        .add_control_command(
            "enemy.place",
            "Move an enemy (test hook)",
            params(&[("id", "integer", "slot"), ("x", "number", "world x"), ("z", "number", "world z"), ("face", "array", "[x, z] to look at")]),
            enemy_place,
        )
        .add_control_command(
            "enemy.lean",
            "Force every enemy's lean (test hook; omit `side` to give control back to the AI)",
            params(&[("side", "number", "-1 left .. +1 right")]),
            enemy_lean,
        )
        .add_control_command(
            "ai.difficulty",
            "Set the difficulty for the next round",
            params(&[("level", "string", "recruit | regular | veteran | elite")]),
            ai_difficulty,
        )
        .add_control_command(
            "sim.run",
            "Headless trainer: evolve N times vs the default ghost on a copy of the brain",
            params(&[("generations", "integer", "default 5"), ("seed", "integer", "default 1")]),
            sim_run,
        );
}

fn enemy_list(world: &mut World, _: Value) -> Result<Value, String> {
    let tactic = world.resource::<Squad>().coord.as_ref().map(|c| c.doctrine.tactic.name());
    let out: Vec<Value> = world
        .query_filtered::<(&Enemy, &Transform, &Health, &Mind, &EnemyCombat), With<Enemy>>()
        .iter(world)
        .filter(|(.., c)| !c.dead)
        .map(|(e, tf, hp, m, c)| {
            let p = to_ground(tf.translation);
            json!({
                "id": e.slot,
                "pos": [p.x, p.y],
                "hp": hp.hp,
                "goal": m.0.goal().label(),
                "role": m.0.order().map(|o| o.role.label()),
                "lean": c.lean,
                "post": m.0.post_phase().map(|p| p.label()),
                "isolated": m.0.isolated(),
                "alert": m.0.alert().label(),
                "sees_player": c.sees_player,
                "crouched": c.crouched,
                "trait_id": m.0.trait_id,
                "neat_id": m.0.neat_id,
                "damage_dealt": c.report.damage_dealt,
                "loadout": c.role.label(),
                "weapon": c.gun.held.label(),
                "primary": c.gun.primary.label(),
                "ammo": c.gun.ammo(),
                "reloading": c.gun.reloading(),
                "shots": c.shots,
                "shots_hit": c.shots_hit,
                "clearing": m.0.debug.clearing,
                "doctrine": tactic,
                "jiggling": m.0.jiggling(),
                "traits": {
                    "aggression": m.0.traits.aggression,
                    "preferred_range": m.0.traits.preferred_range,
                    "flank_bias": m.0.traits.flank_bias,
                    "cover_affinity": m.0.traits.cover_affinity,
                    "reaction_ms": m.0.traits.reaction_time * 1000.0,
                    "aim_error_deg": m.0.traits.aim_error_deg,
                },
            })
        })
        .collect();
    Ok(json!(out))
}

fn weapon_kind(s: &str) -> Result<WeaponKind, String> {
    let s = s.to_lowercase();
    WeaponKind::ALL
        .into_iter()
        .find(|k| format!("{k:?}").to_lowercase() == s || k.label().to_lowercase() == s)
        .ok_or_else(|| format!("unknown weapon {s:?} (pistol, smg, rifle, shotgun, marksman)"))
}

fn enemy_loadout(world: &mut World, p: Value) -> Result<Value, String> {
    let by_slot: Option<Vec<WeaponKind>> = match p.get("kinds").and_then(Value::as_array) {
        Some(a) => Some(a.iter().map(|v| weapon_kind(v.as_str().unwrap_or(""))).collect::<Result<_, _>>()?),
        None => None,
    };
    let one = match p.get("kind").and_then(Value::as_str) {
        Some(s) => Some(weapon_kind(s)?),
        None => None,
    };
    if by_slot.is_none() && one.is_none() {
        return Err("give `kind` or `kinds`".into());
    }
    let id = p.get("id").and_then(Value::as_u64).map(|v| v as usize);
    let mut out = Vec::new();
    for (e, mut mind, mut c, mut armed) in world.query::<(&Enemy, &mut Mind, &mut EnemyCombat, &mut Armed)>().iter_mut(world) {
        let kind = match (&by_slot, one) {
            (Some(v), _) => v.get(e.slot).copied(),
            (None, Some(k)) if id.is_none_or(|i| i == e.slot) => Some(k),
            _ => None,
        };
        let Some(kind) = kind else { continue };
        mind.0.set_weapon(kind);
        c.gun = GunState::new(kind);
        c.role = Role::of(kind);
        armed.0 = kind;
        out.push(json!({ "id": e.slot, "weapon": kind.label(), "role": c.role.label() }));
    }
    Ok(json!(out))
}

fn match_time_limit(world: &mut World, p: Value) -> Result<Value, String> {
    let secs = p.get("seconds").and_then(Value::as_f64).map(|s| s as f32);
    world.resource_mut::<crate::match_flow::RoundLimit>().0 = secs;
    Ok(json!({ "seconds": secs }))
}

fn ai_play(world: &mut World, p: Value) -> Result<Value, String> {

    use brain::squad::Play;
    let want = p.get("name").and_then(Value::as_str).map(|s| s.to_lowercase().replace([' ', '-', '_'], ""));
    let all = [Play::Free, Play::Overwatch, Play::Crossfire, Play::Trade, Play::PinFlank, Play::Collapse, Play::Bait, Play::Sweep, Play::Search, Play::Encircle];
    let play = match &want {
        None => None,
        Some(w) => Some(
            all.into_iter()
                .find(|p| p.label().replace([' ', '-'], "") == *w || format!("{p:?}").to_lowercase() == *w)
                .ok_or_else(|| format!("unknown play {w:?}"))?,
        ),
    };
    let mut sq = world.resource_mut::<Squad>();
    let Some(c) = sq.coord.as_mut() else { return Err("no squad".into()) };
    c.force_play = play;
    Ok(json!({ "play": play.map(|p| p.label()) }))
}

fn enemy_place(world: &mut World, p: Value) -> Result<Value, String> {

    let id = p.get("id").and_then(Value::as_u64).ok_or("need `id`")? as usize;
    let x = p.get("x").and_then(Value::as_f64).ok_or("need `x`")? as f32;
    let z = p.get("z").and_then(Value::as_f64).ok_or("need `z`")? as f32;
    let face = p.get("face").and_then(Value::as_array).and_then(|a| Some(Vec3::new(a.first()?.as_f64()? as f32, 0.0, a.get(1)?.as_f64()? as f32)));
    for (e, mut tf) in world.query::<(&Enemy, &mut Transform)>().iter_mut(world) {
        if e.slot == id {
            tf.translation = Vec3::new(x, 0.0, z);
            if let Some(f) = face.filter(|f| f.distance(tf.translation) > 0.1) {
                tf.look_at(f, Vec3::Y);
            }
            return Ok(json!({ "id": id, "pos": [x, z] }));
        }
    }
    Err(format!("no enemy {id}"))
}

fn enemy_freeze(world: &mut World, p: Value) -> Result<Value, String> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    world.resource_mut::<EnemyFreeze>().0 = on;
    Ok(json!({ "frozen": on }))
}

fn enemy_kill(world: &mut World, p: Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_u64).map(|v| v as usize);
    let mut killed = 0;
    for (e, mut hp) in world.query::<(&Enemy, &mut Health)>().iter_mut(world) {
        if id.is_none_or(|i| i == e.slot) && hp.hp > 0.0 {
            hp.hp = 0.0;
            killed += 1;
        }
    }
    Ok(json!({ "killed": killed }))
}

fn match_state(world: &mut World, _: Value) -> Result<Value, String> {
    let f = world.resource::<MatchFlow>();
    Ok(json!({
        "match": f.match_no,
        "round": f.round,
        "player_rounds": f.player_rounds,
        "enemy_rounds": f.enemy_rounds,
        "round_over": f.round_over.map(|(_, won)| json!({ "player_won": won })),
        "last_result": f.last_result.map(|(w, p, e)| json!({ "player_won": w, "player_rounds": p, "enemy_rounds": e })),
    }))
}

fn ai_brain(world: &mut World, _: Value) -> Result<Value, String> {
    let arena = current_arena(world);
    let b = &world.resource::<Brain>().state;
    Ok(json!({
        "generation": b.generation,
        "squad": b.squad.iter().map(|s| json!({ "trait_id": s.trait_id, "neat_id": s.neat_id, "genes": b.traits.genomes[s.trait_id].genes })).collect::<Vec<_>>(),
        "neat_species": b.neat.species_count(),
        "player_model": {
            "matches_observed": b.player_model.matches_observed,
            "confidence": b.player_model.confidence(),
            "strafe_bias": b.player_model.strafe_bias,
            "reaction_ms": b.player_model.reaction_s * 1000.0,
            "camping": b.player_model.camping,
            "accuracy_by_range": b.player_model.accuracy,
        },
        "insights": b.player_model.insights(&arena, None, b.history.last().map(|h| &h.stats), b.squad_reaction_ms()),
        "history": b.history.iter().map(|h| json!({ "generation": h.generation, "player_won": h.player_won, "sim_win_rate": h.sim_win_rate })).collect::<Vec<_>>(),
    }))
}

fn ai_evolve(world: &mut World, _: Value) -> Result<Value, String> {
    let telemetry = world.resource::<Telemetry>().0.clone();
    let enemies = world.resource::<MatchFlow>().enemies.clone();
    let report = MatchReport { telemetry, enemies, ..Default::default() };
    let arena = current_arena(world);
    let mut brain = world.resource_mut::<Brain>();
    let s = brain.state.evolve(&report, &arena, &EvolveConfig::default(), |_| {});
    brain.save();
    Ok(json!({
        "generation": s.generation,
        "sim_win_rate": s.sim_win_rate,
        "neat_generations": s.neat_generations,
        "ms": s.elapsed_ms as u64,
        "deltas": s.deltas.iter().map(|d| json!({ "trait": d.label, "before": d.before, "after": d.after })).collect::<Vec<_>>(),
        "insights": s.insights,
    }))
}

fn ai_reset(world: &mut World, _: Value) -> Result<Value, String> {
    crate::save::reset_brain(world);
    Ok(json!({ "generation": 0 }))
}

fn sim_run(world: &mut World, p: Value) -> Result<Value, String> {
    let n = p.get("generations").and_then(Value::as_u64).unwrap_or(5).min(50) as usize;
    let seed = p.get("seed").and_then(Value::as_u64).unwrap_or(1);
    let mut state = brain::evolve::BrainState::new(seed);
    // Runs on a fresh brain so the player's brain is untouched, on the current map.
    let arena = Arena::build(world.resource::<crate::map::ArenaRes>().0.id);
    let curve: Vec<Value> = (0..n)
        .map(|_| {
            let s = state.evolve(&MatchReport::default(), &arena, &EvolveConfig::default(), |_| {});
            json!({ "generation": s.generation, "sim_win_rate": s.sim_win_rate, "ms": s.elapsed_ms as u64 })
        })
        .collect();
    Ok(json!(curve))
}

/// The map being played (falls back to Foundry outside a match).
fn current_arena(world: &World) -> Arena {
    world.get_resource::<ArenaRes>().map_or_else(Arena::foundry, |a| Arena::build(a.0.id))
}

/// The adaptive doctrine: tactic, why, archetypes, counter strengths, the live style profile
/// and this round's switches.
fn doctrine_json(c: &brain::squad::SquadCoordinator) -> Value {
    let d = &c.doctrine;
    let k = d.counters();
    let p = &c.style;
    json!({
        "tactic": d.tactic.name(),
        "reason": d.reason,
        "since": d.since,
        "archetypes": d.active.labels(),
        "counters": { "jiggle": k.jiggle, "off_angle": k.off_angle, "reload_rush": k.reload_rush, "stagger": k.stagger },
        "profile": {
            "avg_move_speed": p.avg_move_speed,
            "stationary_angle_hold_secs": p.stationary_angle_hold_secs,
            "ads_usage_ratio": p.ads_usage_ratio,
            "headshot_rate": p.headshot_rate,
            "hits": p.hits,
            "premature_reload_count": p.premature_reload_count,
            "rapid_entries": p.rapid_entries,
        },
        "jiggler": c.jiggler.map(|(i, j)| json!({ "id": i, "hide": [j.hide.x, j.hide.y] })),
        "wallbang": c.wallbang.map(|(i, at, _)| json!({ "id": i, "at": [at.x, at.y] })),
        "rushers": c.rushers.iter().map(|r| r.0).collect::<Vec<_>>(),
        "traps": c.traps.iter().map(|t| json!({ "id": t.0, "spot": [t.1.x, t.1.y], "door": [t.2.x, t.2.y] })).collect::<Vec<_>>(),
        "history": d.history.iter().map(|h| json!({ "t": h.t, "from": h.from.name(), "to": h.to.name(), "reason": h.reason })).collect::<Vec<_>>(),
    })
}

fn ai_doctrine(world: &mut World, _: Value) -> Result<Value, String> {
    let jiggling: Vec<usize> = world.query::<(&Enemy, &Mind)>().iter(world).filter(|(_, m)| m.0.jiggling()).map(|(e, _)| e.slot).collect();
    let sq = world.resource::<Squad>();
    let Some(c) = sq.coord.as_ref() else { return Ok(json!({ "coord": null })) };
    let mut v = doctrine_json(c);
    v["play"] = json!(c.play.label());
    v["roles"] = json!(c.orders.iter().map(|o| o.map(|o| o.role.label())).collect::<Vec<_>>());
    v["flashes_used"] = json!(c.flashes_used);
    v["frags_used"] = json!(c.frags_used);
    v["utility"] = json!({ "frags": c.utility.frags, "flashes": c.utility.flashes });
    v["jiggling"] = json!(jiggling);
    v["difficulty"] = json!(sq.difficulty.label());
    Ok(v)
}

fn ai_squad(world: &mut World, _: Value) -> Result<Value, String> {
    let grenades: Vec<Value> = world
        .query::<(&Grenade, &Transform)>()
        .iter(world)
        .map(|(g, tf)| json!({ "kind": g.kind.label(), "pos": [tf.translation.x, tf.translation.y, tf.translation.z], "fuse": g.fuse }))
        .collect();
    let flashed = *world.resource::<Flashed>();
    let tactics = world.resource::<Brain>().state.tactics.clone();
    let sq = world.resource::<Squad>();
    let Some(c) = sq.coord.as_ref() else { return Ok(json!({ "coord": null })) };
    Ok(json!({
        "difficulty": sq.difficulty.label(),
        "size": sq.size,
        "coordination": c.enabled,
        "play": c.play.label(),
        "doctrine": doctrine_json(c),
        "breach": c.breach.map(|b| json!({ "door": [b.door.pos.x, b.door.pos.y], "inside": [b.inside.x, b.inside.y], "t": b.t, "go_at": b.go_at })),
        "alert": c.board.alert.label(),
        "board_age": c.board.age,
        "roles": c.orders.iter().map(|o| o.map(|o| o.role.label())).collect::<Vec<_>>(),
        "prefire": c.orders.iter().map(|o| o.and_then(|o| o.prefire).map(|p| [p.x, p.y])).collect::<Vec<_>>(),
        "dests": c.orders.iter().map(|o| o.and_then(|o| o.dest).map(|p| [p.x, p.y])).collect::<Vec<_>>(),
        "vias": c.orders.iter().map(|o| o.and_then(|o| o.via).map(|p| [p.x, p.y])).collect::<Vec<_>>(),
        "search": {
            "spots": c.search_spots.iter().map(|s| json!({ "pos": [s.pos.x, s.pos.y], "mass": s.mass })).collect::<Vec<_>>(),
            "investigators": c.investigators,
            "hot": c.hot_spot().map(|p| [p.x, p.y]),
        },
        "encircle": {
            "shooters": c.shooters,
            "go": c.go_t.is_some(),
            "sectors": c.sectors.iter().map(|s| json!({ "bearing_deg": s.bearing.to_degrees(), "goal": [s.goal.x, s.goal.y], "peek": [s.peek.x, s.peek.y] })).collect::<Vec<_>>(),
            "sector_of": c.sector_of,
        },
        "tokens": c.tokens,
        "max_tokens": c.max_tokens,
        "player_still_s": c.still_t,
        "unseen_s": c.unseen_t,
        "peeks": c.peeks.iter().map(|p| json!({ "pos": [p.pos.x, p.pos.y], "count": p.count, "delay": p.delay })).collect::<Vec<_>>(),
        "utility": { "frags": c.utility.frags, "flashes": c.utility.flashes, "frags_used": c.frags_used, "flashes_used": c.flashes_used },
        "grenades": grenades,
        "flashed": { "alpha": flashed.alpha(), "t": flashed.t, "dur": flashed.dur },
        "bandit": {
            "context": CTX_LABELS.iter().zip(c.last_ctx).map(|(l, v)| json!({ "feature": l, "value": v })).collect::<Vec<_>>(),
            "scores": c.last_scores.iter().map(|s| json!({ "play": s.play.label(), "mean": s.mean, "bonus": s.bonus, "prior": s.prior, "total": s.total, "n": s.n })).collect::<Vec<_>>(),
            "episodes_round": c.tactics.episodes,
            "episodes_saved": tactics.episodes,
        },
    }))
}

fn ai_throw(world: &mut World, p: Value) -> Result<Value, String> {
    let kind = match p.get("kind").and_then(Value::as_str).unwrap_or("frag") {
        "frag" => GrenadeKind::Frag,
        "flash" | "flashbang" => GrenadeKind::Flash,
        k => return Err(format!("unknown grenade kind {k:?} (frag | flash)")),
    };
    let num = |v: &Value, i: usize| v.get(i).and_then(Value::as_f64).map(|x| x as f32);
    if let Some(at) = p.get("at") {
        let (Some(x), Some(z)) = (num(at, 0), num(at, 1)) else { return Err("`at` must be [x, z]".into()) };
        let target = Vec3::new(x, 0.1, z);
        let from = p
            .get("from")
            .and_then(|f| Some(Vec3::new(num(f, 0)?, num(f, 1)?, num(f, 2)?)))
            .unwrap_or(target + Vec3::new(0.0, 1.5, -6.0));
        world.write_message(ThrowGrenade { thrower: Entity::PLACEHOLDER, kind, from, target });
        return Ok(json!({ "thrown": kind.label(), "from": [from.x, from.y, from.z], "at": [x, z] }));
    }
    let mut sq = world.resource_mut::<Squad>();
    let Some(c) = sq.coord.as_mut() else { return Err("no squad".into()) };
    c.force_throw = Some(kind);
    Ok(json!({ "queued": kind.label() }))
}

fn enemy_lean(world: &mut World, p: Value) -> Result<Value, String> {
    let side = p.get("side").and_then(Value::as_f64).map(|v| (v as f32).clamp(-1.0, 1.0));
    world.resource_mut::<crate::enemy::LeanOverride>().0 = side;
    Ok(json!({ "lean": side }))
}

fn ai_difficulty(world: &mut World, p: Value) -> Result<Value, String> {
    let want = p.get("level").and_then(Value::as_str).unwrap_or("regular").to_lowercase();
    let d = Difficulty::ALL.into_iter().find(|d| d.label().to_lowercase() == want).ok_or_else(|| format!("unknown difficulty {want:?}"))?;
    world.resource_mut::<crate::settings::ShroudSettings>().difficulty = d;
    let p = d.params();
    Ok(json!({ "difficulty": d.label(), "squad_size": p.squad_size, "utility": p.utility, "coordination": p.coordination, "max_attackers": p.max_attackers, "applies": "next round" }))
}
