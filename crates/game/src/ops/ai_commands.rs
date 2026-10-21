//! Control-port commands for enemies, the AI brain and the match (test tooling only).

use bevy::prelude::*;
use brain::arena::Arena;
use brain::bandit::CTX_LABELS;
use brain::difficulty::Difficulty;
use brain::evolve::{EvolveConfig, MatchReport};
use brain::grenade::GrenadeKind;
use serde_json::{Value, json};

use super::control::{ControlAppExt, params};
use crate::enemy::grenade::{Flashed, Grenade, ThrowGrenade};
use crate::enemy::{Enemy, EnemyCombat, EnemyFreeze, Mind, Squad};
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
            "enemy.lean",
            "Force every enemy's lean (test hook; omit `side` to give control back to the AI)",
            params(&[("side", "number", "-1 left .. +1 right")]),
            enemy_lean,
        )
        .add_control_command(
            "ai.difficulty",
            "Set the difficulty for the next round (in memory; not saved)",
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
                "alert": m.0.alert().label(),
                "sees_player": c.sees_player,
                "crouched": c.crouched,
                "trait_id": m.0.trait_id,
                "neat_id": m.0.neat_id,
                "damage_dealt": c.report.damage_dealt,
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
    let tracer = world.remove_resource::<crate::ops::trace::GameTracer>();
    world.resource_mut::<Brain>().reset(tracer.as_ref());
    if let Some(t) = tracer {
        world.insert_resource(t);
    }
    Ok(json!({ "generation": 0 }))
}

fn sim_run(world: &mut World, p: Value) -> Result<Value, String> {
    let n = p.get("generations").and_then(Value::as_u64).unwrap_or(5).min(50) as usize;
    let seed = p.get("seed").and_then(Value::as_u64).unwrap_or(1);
    let mut state = brain::evolve::BrainState::new(seed);
    let _ = world; // runs on a fresh brain so the player's brain is untouched
    let arena = Arena::foundry();
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
        "alert": c.board.alert.label(),
        "board_age": c.board.age,
        "roles": c.orders.iter().map(|o| o.map(|o| o.role.label())).collect::<Vec<_>>(),
        "prefire": c.orders.iter().map(|o| o.and_then(|o| o.prefire).map(|p| [p.x, p.y])).collect::<Vec<_>>(),
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
    match world.get_resource_mut::<crate::settings::Settings>() {
        Some(mut s) => s.difficulty = d,
        None => world.insert_resource(crate::settings::Settings { difficulty: d, ..Default::default() }),
    }
    let p = d.params();
    Ok(json!({ "difficulty": d.label(), "squad_size": p.squad_size, "utility": p.utility, "coordination": p.coordination, "max_attackers": p.max_attackers, "applies": "next round" }))
}
