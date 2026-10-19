//! Control-port commands for enemies, the AI brain and the match (test tooling only).

use bevy::prelude::*;
use brain::arena::Arena;
use brain::evolve::{EvolveConfig, MatchReport};
use serde_json::{Value, json};

use super::control::{ControlAppExt, params};
use crate::enemy::{Enemy, EnemyCombat, EnemyFreeze, Mind};
use crate::map::to_ground;
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
    let b = &world.resource::<Brain>().state;
    let arena = Arena::foundry();
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
    let mut brain = world.resource_mut::<Brain>();
    let s = brain.state.evolve(&report, &Arena::foundry(), &EvolveConfig::default(), |_| {});
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
