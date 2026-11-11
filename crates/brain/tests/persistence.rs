//! Saved brains and trainer checkpoints: files that parse but would panic are rejected, and
//! non-finite floats never reach disk.

use brain::evolve::{BrainState, SAVE_VERSION, SquadSlot};
use brain::trainer::Trainer;
use std::path::PathBuf;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shroud-persist-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn names(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()
}

/// Writes `s` as JSON without going through `save` (which would sanitize it).
fn write_raw(path: &std::path::Path, s: &impl serde::Serialize) {
    std::fs::write(path, serde_json::to_vec(s).unwrap()).unwrap();
}

#[test]
fn fresh_brain_is_valid() {
    BrainState::new(1).validate().unwrap();
}

#[test]
fn out_of_range_squad_slot_is_rejected_and_moved_aside() {
    // Arrange
    let dir = temp_dir("slot");
    let path = dir.join("brain.json");
    let mut s = BrainState::new(5);
    s.squad[0] = SquadSlot { trait_id: 10_000, neat_id: 0 };
    write_raw(&path, &s);
    // Act
    let (loaded, note) = BrainState::load_checked(&path, 77);
    // Assert
    assert_eq!(loaded.seed, 77, "started fresh");
    assert!(note.unwrap().contains("squad slot"));
    assert!(!path.exists());
    assert!(names(&dir).iter().any(|n| n.starts_with("brain.json.corrupt-")), "{:?}", names(&dir));
    // The fresh brain is usable (this used to panic on every launch).
    let _ = loaded.squad_minds(1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_neat_refs_are_rejected() {
    let mut s = BrainState::new(6);
    s.neat.genomes[3].conns[0].out_node = 99_999;
    assert!(s.validate().unwrap_err().contains("missing node"));

    let mut s = BrainState::new(6);
    s.neat.genomes[0].nodes.truncate(3);
    assert!(s.validate().is_err());

    let mut s = BrainState::new(6);
    s.neat.genomes[0].num_inputs += 1;
    assert!(s.validate().is_err());
}

#[test]
fn bad_champion_slots_are_rejected() {
    let mut s = BrainState::new(8);
    s.champion = Some(brain::evolve::Champion {
        traits: vec![s.traits.genomes[0].clone()],
        neat: vec![s.neat.genomes[0].clone()],
        score: Default::default(),
        slots: Some(vec![SquadSlot { trait_id: 0, neat_id: 5_000 }]),
    });
    assert!(s.validate().unwrap_err().contains("champion slot"));
}

#[test]
fn overflowing_float_in_file_is_rejected() {
    // `1e39` parses as f32 infinity, or (when another crate in the build turns on a stricter
    // serde_json float mode) fails as out of range. Either way the file must be rejected.
    let dir = temp_dir("inf");
    let path = dir.join("brain.json");
    let s = BrainState::new(5);
    let mut j = serde_json::to_value(&s).unwrap();
    j["traits"]["genomes"][0]["genes"][0] = serde_json::json!(1e39);
    std::fs::write(&path, serde_json::to_vec(&j).unwrap()).unwrap();
    let (loaded, note) = BrainState::load_checked(&path, 12);
    assert_eq!(loaded.seed, 12);
    let note = note.expect("a rejected file leaves a note");
    assert!(note.contains("non-finite") || note.contains("out of range") || note.contains("invalid"), "{note}");
    assert!(!path.exists(), "the bad file is moved aside");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn older_save_version_is_kept_under_a_versioned_name() {
    let dir = temp_dir("ver");
    let path = dir.join("brain.json");
    let mut s = BrainState::new(5);
    s.version = SAVE_VERSION - 1;
    write_raw(&path, &s);
    std::fs::write(dir.join("brain.json.bak"), b"manual reset backup").unwrap();
    let (_, note) = BrainState::load_checked(&path, 1);
    assert!(note.is_some());
    assert!(dir.join(format!("brain.json.v{}.bak", SAVE_VERSION - 1)).exists());
    assert_eq!(std::fs::read(dir.join("brain.json.bak")).unwrap(), b"manual reset backup", "reset backup untouched");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_file_starts_fresh_without_a_note() {
    let dir = temp_dir("missing");
    let (s, note) = BrainState::load_checked(&dir.join("brain.json"), 3);
    assert_eq!(s.seed, 3);
    assert!(note.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn non_finite_floats_are_sanitized_before_saving() {
    // Arrange: NaN/inf in the places evolution writes.
    let dir = temp_dir("nan");
    let path = dir.join("brain.json");
    let mut s = BrainState::new(4);
    s.traits.genomes[2].fitness = f32::NAN;
    s.traits.genomes[1].genes[0] = f32::INFINITY;
    s.neat.genomes[0].conns[0].weight = f32::NAN;
    s.neat.genomes[1].fitness = f32::NEG_INFINITY;
    s.player_model.camping = f32::NAN;
    s.player_model.heat[0] = f32::NAN;
    // Act
    s.save(&path).unwrap();
    let (loaded, note) = BrainState::load_checked(&path, 999);
    // Assert: the brain survived (it used to be written as `null` and reset next launch).
    assert!(note.is_none(), "{note:?}");
    assert_eq!(loaded.seed, 4);
    assert_eq!(loaded.traits.genomes[2].fitness, 0.0);
    assert_eq!(loaded.neat.genomes[0].conns[0].weight, 0.0);
    assert_eq!(loaded.player_model.camping, 0.0);
    // No temp file left behind.
    assert_eq!(names(&dir), vec!["brain.json".to_string()]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn trainer_checkpoint_is_validated_and_sanitized() {
    let dir = temp_dir("trainer");
    let path = dir.join("trainer.json");
    // Missing: Ok(None).
    assert!(Trainer::load_checked(&path).unwrap().is_none());
    // NaN in the state: saved sanitized, loads back.
    let mut t = Trainer::new(BrainState::new(2));
    t.state.neat.genomes[4].fitness = f32::NAN;
    t.save(&path).unwrap();
    assert!(Trainer::load_checked(&path).unwrap().is_some());
    // Parses but would panic: rejected and moved aside.
    let mut bad = Trainer::new(BrainState::new(2));
    bad.state.squad[1].neat_id = 1_000;
    write_raw(&path, &bad);
    assert!(Trainer::load_checked(&path).is_err());
    assert!(Trainer::load(&path).is_none());
    assert!(names(&dir).iter().any(|n| n.starts_with("trainer.json.corrupt-")), "{:?}", names(&dir));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn trained_brain_still_validates() {
    // Evolution adds hidden nodes and connections; they must pass the load check.
    let a = brain::arena::Arena::yard();
    let cfg = brain::evolve::EvolveConfig {
        budget: std::time::Duration::from_millis(400),
        max_neat_generations: 6,
        squads: 4,
        rounds_per_genome: 1,
        rungs: 1,
        ..Default::default()
    };
    let mut s = BrainState::new(21);
    s.bootstrap(&a, &cfg);
    s.evolve(&Default::default(), &a, &cfg, |_| {});
    s.validate().unwrap();
    let dir = temp_dir("trained");
    let path = dir.join("brain.json");
    s.save(&path).unwrap();
    let (back, note) = BrainState::load_checked(&path, 0);
    assert!(note.is_none(), "{note:?}");
    assert_eq!(back.generation, s.generation);
    let _ = std::fs::remove_dir_all(dir);
}
