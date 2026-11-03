//! Player utility against the AI: a flashbang blinds an enemy (no sight, no trigger) for its
//! duration, and smoke blocks the enemy's line of sight.
use brain::arena::Arena;
use brain::evolve::BrainState;
use brain::grenade::{self, SmokeCloud};
use brain::limits::Limits;
use brain::math::V2;
use brain::mind::{EnemyMind, MindCtx, Observation};
use brain::perception::Alert;
use brain::player_model::PlayerModel;
use brain::sim::DT;

fn mind(seed: u64) -> EnemyMind {
    BrainState::new(seed).squad_minds(seed).remove(0)
}

/// Two points on the foundry with a clear line between them (10 m apart).
const ME: V2 = V2 { x: -10.0, y: 18.0 };
const HIM: V2 = V2 { x: -10.0, y: 28.0 };

fn base() -> Observation {
    Observation {
        dt: DT,
        pos: ME,
        hp: 1.0,
        ammo_frac: 1.0,
        primary_ammo: 1.0,
        player_pos: HIM,
        player_hp: 1.0,
        squad_alert: Alert::Combat,
        time_since_seen: 0.0,
        player_visible: true,
        ..Default::default()
    }
}

#[test]
fn flash_blinds_the_enemy() {
    let a = Arena::foundry();
    assert!(a.line_of_sight(ME, false, HIM, false));
    let model = PlayerModel::default();
    let cx = MindCtx { arena: &a, model: &model, limits: Limits::for_generation(0) };
    for seed in 0..6u64 {
        // Blinded for a full flash: never fires, never pre-fires, even with him in plain view.
        let mut m = mind(seed);
        let blind = grenade::flash_duration(1.0);
        let mut left = blind;
        let mut fired = false;
        while left > 0.0 {
            let act = m.think(&Observation { blind: left, ..base() }, &cx);
            fired |= act.fire || act.prefire.is_some();
            left -= DT;
        }
        assert!(!fired, "seed {seed}: fired while flashed");
        // It clears: it can fight again.
        let fires = (0..120).any(|_| m.think(&base(), &cx).fire);
        assert!(fires, "seed {seed}: never fired after the flash wore off");
    }
}

#[test]
fn smoke_hides_the_player_from_the_arena_los() {
    let mut a = Arena::foundry();
    a.smokes.push(SmokeCloud { at: V2::new(-10.0, 23.0), age: 4.0 });
    assert!(!a.line_of_sight(ME, false, HIM, false));
    a.smokes[0].age = grenade::SMOKE_S;
    assert!(a.line_of_sight(ME, false, HIM, false), "a spent smoke stops blocking");
}
