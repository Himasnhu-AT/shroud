//! The player's plants reach the model only through finished-match telemetry, and the trainer's
//! defuse follows the game's rule (the coordinator's current defuser).

use brain::math::V2;
use brain::objective::{DEFUSE_S, PlantedBomb};
use brain::player_model::{MatchTelemetry, PlayerModel};
use brain::sim::{BombEnd, DT, bomb_step};

#[test]
fn match_plants_fold_into_the_model_on_ingest() {
    // Arrange: two plants on B in a short match (too few samples for the rest of the model).
    let mut m = PlayerModel::default();
    let mut t = MatchTelemetry::default();
    t.plant(1);
    t.plant(1);
    // Act
    m.ingest(&t);
    // Assert
    assert!(m.plant_share(1) > 0.7, "B share {}", m.plant_share(1));
    assert_eq!(m.matches_observed, 0, "the rest of a too-short match is still ignored");
}

#[test]
fn ingesting_plants_matches_recording_them_one_by_one() {
    // Arrange
    let mut a = PlayerModel::default();
    a.record_plant(0);
    let mut b = a.clone();
    // Act
    a.record_plant(1);
    b.ingest_plants(&[0.0, 1.0]);
    // Assert
    for s in 0..2 {
        assert!((a.plants[s] - b.plants[s]).abs() < 1e-6, "site {s}: {:?} vs {:?}", a.plants, b.plants);
    }
}

#[test]
fn old_telemetry_without_plants_loads() {
    // Arrange: a save from before `plant_sites`.
    let mut v = serde_json::to_value(MatchTelemetry::default()).unwrap();
    v.as_object_mut().unwrap().remove("plant_sites");
    // Act
    let t: MatchTelemetry = serde_json::from_value(v).expect("loads");
    // Assert
    assert_eq!(t.plant_sites, [0.0, 0.0]);
}

fn bomb(fuse: f32) -> PlantedBomb {
    PlantedBomb { pos: V2::ZERO, site: 0, fuse_left: fuse, defuse_progress: 0.0 }
}

#[test]
fn defuse_counts_from_the_first_tick_and_resets_on_a_new_defuser() {
    // Arrange
    let mut b = bomb(30.0);
    let mut defusing = None;
    // Act + assert: member 2 starts this tick and is credited this tick.
    assert_eq!(bomb_step(&mut b, &mut defusing, Some(2)), None);
    assert!((b.defuse_progress - DT / DEFUSE_S).abs() < 1e-6);
    // Another member takes over: from zero, this tick counted.
    bomb_step(&mut b, &mut defusing, Some(1));
    assert!((b.defuse_progress - DT / DEFUSE_S).abs() < 1e-6);
    assert_eq!(defusing, Some(1));
    // Nobody on it: reset.
    bomb_step(&mut b, &mut defusing, None);
    assert_eq!(b.defuse_progress, 0.0);
}

#[test]
fn a_defuse_finishing_on_the_last_tick_beats_the_fuse() {
    // Arrange: one tick of defuse left and the fuse runs out the same tick.
    let mut b = bomb(0.0);
    b.defuse_progress = 1.0 - DT / DEFUSE_S * 0.5;
    let mut defusing = Some(0);
    // Act + assert (the game checks the defuse before the explosion too).
    assert_eq!(bomb_step(&mut b, &mut defusing, Some(0)), Some(BombEnd::Defused));
    let mut b = bomb(0.0);
    assert_eq!(bomb_step(&mut b, &mut None, None), Some(BombEnd::Exploded));
}
