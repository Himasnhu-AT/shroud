//! The player's career (rank ladder) on top of [`brain::career`], which holds the rules.
//!
//! - [`CareerSave`]: `<data dir>/career.json` (XP, matches, wins, objectives done). Missing →
//!   fresh career; corrupt → moved aside to `career.json.corrupt`, fresh career.
//! - [`MatchCareer`]: this match's 3–6 objectives (rolled when a match starts, from the rank,
//!   the map's site letters and a per-match seed) and the [`MatchLog`] filled from real game
//!   events: the player's gun kills (bomb and frag kills don't count), plants, detonations,
//!   damage taken, death, holding a primary, round results.
//! - A finished match is credited on entering `Evolving` and saved; the [`Award`] waits for the
//!   debrief. Abandoned matches (restart / main menu) pay nothing: a new match rerolls.
//! - [`ObjectiveDone`] fires when an objective completes mid-match (HUD toast, trace).
//!
//! HUD tracker and toast: `ui::career_hud`. Debrief: `ui::debrief`. Menu card: `menu`.
//! Control port (dev): `career.state`, `career.set_xp`, `career.reroll`.

use std::path::PathBuf;

use bevy::prelude::*;
use brain::career::{Award, Career, KillLog, MatchLog, Objective, roll_objectives};
use engine::cli::GameSeed;
use engine::paths::DataDir;

use crate::bomb::{Bomb, BombEvent};
use crate::dev::{Dev, Prop};
use crate::enemy::Squad;
use crate::map::ArenaRes;
use crate::match_flow::{MatchFlow, ROUNDS_TO_WIN};
use crate::player::Player;
use crate::state::GameState;
use crate::ui::hud::RoundClock;
use crate::ui::killfeed::{KillEvent, detect_kills};
use crate::weapon::{Arsenal, Damage, Health, WeaponSet};

/// The saved career.
#[derive(Resource)]
pub struct CareerSave {
    pub career: Career,
    pub path: PathBuf,
}

impl CareerSave {
    pub fn load(dir: &DataDir) -> Self {
        let path = dir.file("career.json");
        let career: Career = engine::paths::load_json(&path);
        info!("career: {} ({} XP) from {}", career.rank().name(), career.xp, path.display());
        CareerSave { career, path }
    }

    pub fn save(&self) {
        if let Err(e) = engine::paths::save_json(&self.path, &self.career) {
            warn!("career: could not save {}: {e}", self.path.display());
        }
    }
}

/// This match's objectives and log, and the last credited match's award.
#[derive(Resource, Default, Debug)]
pub struct MatchCareer {
    /// `MatchFlow::match_no` these objectives belong to.
    pub match_no: u32,
    pub objectives: Vec<Objective>,
    pub log: MatchLog,
    /// Completion already announced, per objective.
    pub announced: Vec<bool>,
    /// The match has been credited (it ended).
    pub credited: bool,
    /// The last credited match: what it paid, and the XP before it.
    pub award: Option<Award>,
    pub xp_before: u32,
    /// The bomb's defuser on the previous frame (the kill can clear it the same frame).
    last_defuser: Option<usize>,
    /// The round was already over on the previous frame: kills and damage from then on (the
    /// round-over pause) are not logged.
    round_closed: bool,
}

impl MatchCareer {
    /// (done, progress, target) per objective.
    pub fn status(&self) -> Vec<(bool, u32, u32)> {
        self.objectives
            .iter()
            .map(|o| {
                let (p, t) = o.goal.progress(&self.log);
                (p >= t, p.min(t), t)
            })
            .collect()
    }
}

/// An objective completed mid-match.
#[derive(Message, Clone, Debug)]
pub struct ObjectiveDone {
    pub text: String,
    pub xp: u32,
}

pub struct CareerPlugin;

impl Plugin for CareerPlugin {
    fn build(&self, app: &mut App) {
        let save = CareerSave::load(app.world().resource::<DataDir>());
        app.insert_resource(save)
            .init_resource::<MatchCareer>()
            .add_message::<ObjectiveDone>()
            .add_systems(
                OnEnter(GameState::Playing),
                start_round.after(crate::match_flow::begin_round).after(crate::enemy::spawn_squad),
            )
            .add_systems(OnEnter(GameState::Evolving), credit_match)
            .add_systems(
                Update,
                track
                    .after(WeaponSet)
                    .after(detect_kills)
                    .after(crate::match_flow::check_round_end)
                    .run_if(in_state(GameState::Playing)),
            );
        #[cfg(feature = "dev")]
        commands::register(app);
    }
}

/// Seed for a match's objectives: varies per match and per career, fixed for a given `--seed`.
fn objective_seed(game_seed: u64, matches_played: u32, match_no: u32) -> u64 {
    game_seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((matches_played as u64) << 20) ^ match_no as u64
}

/// New match → roll objectives and clear the log; every round → a new round in the log.
pub(crate) fn start_round(
    flow: Res<MatchFlow>,
    squad: Res<Squad>,
    arena: Res<ArenaRes>,
    seed: Res<GameSeed>,
    save: Res<CareerSave>,
    mut mc: ResMut<MatchCareer>,
    dev: Dev,
) {
    if mc.match_no != flow.match_no || mc.objectives.is_empty() {
        let sites: Vec<char> = arena.0.sites.iter().map(|s| s.letter).collect();
        let s = objective_seed(seed.0, save.career.matches, flow.match_no);
        reroll(&mut mc, &save.career, &sites, s);
        mc.match_no = flow.match_no;
        dev.trace("career.objectives", &[("count", Prop::Int(mc.objectives.len() as i64)), ("rank", Prop::Int(save.career.rank().index() as i64))]);
    }
    while (mc.log.rounds.len() as u32) < flow.round {
        mc.log.begin_round(squad.size as u32);
    }
    mc.last_defuser = None;
    mc.round_closed = false;
}

/// Fresh objectives (and log) for the current match.
pub(crate) fn reroll(mc: &mut MatchCareer, career: &Career, sites: &[char], seed: u64) {
    mc.objectives = roll_objectives(career.rank(), sites, seed);
    mc.announced = vec![false; mc.objectives.len()];
    mc.log = MatchLog::default();
    mc.credited = false;
}

/// Fill the live round from this frame's events and announce completed objectives.
fn track(
    mut mc: ResMut<MatchCareer>,
    flow: Res<MatchFlow>,
    bomb: Res<Bomb>,
    clock: Res<RoundClock>,
    mut kills: MessageReader<KillEvent>,
    mut bombs: MessageReader<BombEvent>,
    mut damage: MessageReader<Damage>,
    player: Query<(Entity, &Health, Option<&Arsenal>), With<Player>>,
    mut done_out: MessageWriter<ObjectiveDone>,
    dev: Dev,
) {
    let player = player.single().ok();
    let defuser = bomb.defuser;
    // Only mark the resource changed when the log really changes (the HUD redraws on it).
    let m = mc.bypass_change_detection();
    let last_defuser = std::mem::replace(&mut m.last_defuser, defuser);
    // Over before this frame? The frame the round ends still counts: the killing blow (his or
    // the squad's) lands on it.
    let closed = std::mem::replace(&mut m.round_closed, flow.round_over.is_some());
    let Some(round) = m.log.rounds.last_mut() else {
        kills.clear();
        bombs.clear();
        damage.clear();
        return;
    };
    let mut changed = false;
    for k in kills.read() {
        if closed || !(k.by_player && k.gun && !k.on_player) {
            continue;
        }
        let Some(weapon) = k.weapon else { continue };
        let victim_defusing = k.victim_slot.is_some_and(|v| defuser == Some(v) || last_defuser == Some(v));
        round.kills.push(KillLog { weapon, headshot: k.headshot, victim_defusing });
        changed = true;
    }
    for e in bombs.read() {
        match *e {
            BombEvent::Planted { site, .. } if round.planted.is_none() => {
                round.planted = Some((site, clock.0));
                changed = true;
            }
            BombEvent::Exploded { .. } if !round.exploded => {
                round.exploded = true;
                changed = true;
            }
            _ => {}
        }
    }
    if let Some((pe, hp, arsenal)) = player {
        // Read before deciding: damage on the frame the round ends (a hit or the killing blow)
        // counts, so that round is not flawless.
        let taken: f32 = damage.read().filter(|d| d.target == pe).map(|d| d.amount).sum();
        if taken > 0.0 && !closed {
            round.damage_taken += taken;
            changed = true;
        }
        if hp.is_dead() && !round.died {
            round.died = true;
            changed = true;
        }
        if !round.had_primary && arsenal.is_some_and(|a| a.slots[0].is_some()) {
            round.had_primary = true;
            changed = true;
        }
    } else {
        damage.clear();
    }
    if flow.round_over.is_some() && round.won.is_none() {
        round.won = flow.last_round;
        changed = true;
    }
    if !changed {
        return;
    }
    for (i, o) in m.objectives.iter().enumerate() {
        if !m.announced.get(i).copied().unwrap_or(true) && o.done(&m.log) {
            m.announced[i] = true;
            done_out.write(ObjectiveDone { text: o.goal.describe(), xp: o.xp });
            dev.trace("career.objective", &[("index", Prop::Int(i as i64)), ("xp", Prop::Int(o.xp as i64))]);
            info!("objective complete: {} (+{} XP)", o.goal.describe(), o.xp);
        }
    }
    mc.set_changed();
}

/// A finished match pays out (once); abandoned matches never get here.
fn credit_match(flow: Res<MatchFlow>, mut mc: ResMut<MatchCareer>, mut save: ResMut<CareerSave>, dev: Dev) {
    let finished = flow.player_rounds >= ROUNDS_TO_WIN || flow.enemy_rounds >= ROUNDS_TO_WIN;
    if mc.credited || !finished || mc.match_no != flow.match_no || mc.objectives.is_empty() {
        return;
    }
    let won = flow.player_rounds >= ROUNDS_TO_WIN;
    let xp_before = save.career.xp;
    let award = save.career.credit(won, flow.player_rounds, &mc.objectives, &mc.log);
    save.save();
    dev.trace("career.credited", &[("xp", Prop::Int(award.total as i64)), ("total_xp", Prop::Int(save.career.xp as i64))]);
    if award.promoted() {
        dev.trace("career.promoted", &[("rank", Prop::Int(award.rank_after.index() as i64))]);
        info!("promoted: {} -> {}", award.rank_before.name(), award.rank_after.name());
    }
    info!("career: +{} XP ({} total)", award.total, save.career.xp);
    mc.xp_before = xp_before;
    mc.award = Some(award);
    mc.credited = true;
}

// ---------------------------------------------------------------- control port

#[cfg(feature = "dev")]
mod commands {
    use bevy::prelude::*;
    use brain::career::Rank;
    use engine::devtools::{AddControlCommand, params};
    use serde_json::{Value, json};

    use super::{CareerSave, MatchCareer};
    use crate::map::ArenaRes;

    pub(super) fn register(app: &mut App) {
        app.add_control_command("career.state", "Rank, XP, this match's objectives with progress, the last award", params(&[]), state)
            .add_control_command("career.set_xp", "Set the career's XP (saved)", params(&[("xp", "integer", "total XP")]), set_xp)
            .add_control_command(
                "career.reroll",
                "Roll this match's objectives again (clears the match log)",
                params(&[("seed", "integer", "objective seed")]),
                reroll,
            );
    }

    fn state(world: &mut World, _: Value) -> Result<Value, String> {
        let c = world.resource::<CareerSave>().career.clone();
        let mc = world.resource::<MatchCareer>();
        let objectives: Vec<Value> = mc
            .objectives
            .iter()
            .zip(mc.status())
            .map(|(o, (done, p, t))| json!({"goal": format!("{:?}", o.goal), "text": o.goal.describe(), "xp": o.xp, "progress": p, "target": t, "done": done}))
            .collect();
        let rank = c.rank();
        Ok(json!({
            "rank": rank.name(),
            "rank_index": rank.index(),
            "xp": c.xp,
            "next": rank.next().map(|n| json!({"rank": n.name(), "xp": n.xp_floor()})),
            "progress": c.progress().map(|(a, b)| json!([a, b])),
            "matches": c.matches,
            "wins": c.wins,
            "objectives_done": c.objectives_done,
            "match": mc.match_no,
            "rounds_logged": mc.log.rounds.len(),
            "objectives": objectives,
            "rounds": mc.log.rounds.iter().map(|r| json!({
                "won": r.won,
                "planted": r.planted.map(|(site, secs)| json!({"site": site.to_string(), "secs": secs})),
                "exploded": r.exploded,
                "kills": r.kills.iter().map(|k| json!({"weapon": format!("{:?}", k.weapon), "headshot": k.headshot, "victim_defusing": k.victim_defusing})).collect::<Vec<_>>(),
                "damage_taken": r.damage_taken,
                "died": r.died,
                "had_primary": r.had_primary,
                "squad_size": r.squad_size,
            })).collect::<Vec<_>>(),
            "credited": mc.credited,
            "award": mc.award.as_ref().map(|a| json!({
                "total": a.total,
                "lines": a.lines.iter().map(|l| json!({"label": l.label, "xp": l.xp})).collect::<Vec<_>>(),
                "rank_before": a.rank_before.name(),
                "rank_after": a.rank_after.name(),
                "promoted": a.promoted(),
            })),
        }))
    }

    fn set_xp(world: &mut World, p: Value) -> Result<Value, String> {
        let xp = p.get("xp").and_then(Value::as_u64).ok_or("`xp` (integer) is required")?.min(u32::MAX as u64) as u32;
        let mut s = world.resource_mut::<CareerSave>();
        s.career.xp = xp;
        s.save();
        let rank: Rank = s.career.rank();
        Ok(json!({"xp": xp, "rank": rank.name()}))
    }

    fn reroll(world: &mut World, p: Value) -> Result<Value, String> {
        let seed = p.get("seed").and_then(Value::as_u64).ok_or("`seed` (integer) is required")?;
        let sites: Vec<char> = world.resource::<ArenaRes>().0.sites.iter().map(|s| s.letter).collect();
        let career = world.resource::<CareerSave>().career.clone();
        let rounds = world.resource::<crate::match_flow::MatchFlow>().round;
        let squad = world.resource::<crate::enemy::Squad>().size as u32;
        let mut mc = world.resource_mut::<MatchCareer>();
        super::reroll(&mut mc, &career, &sites, seed);
        for _ in 0..rounds {
            mc.log.begin_round(squad);
        }
        Ok(json!({"objectives": mc.objectives.iter().map(|o| o.goal.describe()).collect::<Vec<_>>()}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objective_seed_varies_per_match_and_career() {
        assert_ne!(objective_seed(1, 0, 1), objective_seed(1, 0, 2));
        assert_ne!(objective_seed(1, 0, 1), objective_seed(1, 1, 1));
        assert_eq!(objective_seed(7, 3, 2), objective_seed(7, 3, 2));
    }

    #[test]
    fn corrupt_career_file_loads_fresh_and_is_kept_aside() {
        // Arrange
        let dir = std::env::temp_dir().join(format!("shroud-career-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("career.json"), "{not json").unwrap();
        // Act
        let s = CareerSave::load(&DataDir(dir.clone()));
        // Assert
        assert_eq!(s.career, Career::default());
        assert!(dir.join("career.json.corrupt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn career_round_trips_through_the_save_file() {
        // Arrange
        let dir = std::env::temp_dir().join(format!("shroud-career-rt-{}", std::process::id()));
        let mut s = CareerSave::load(&DataDir(dir.clone()));
        s.career.xp = 4_600;
        s.career.matches = 3;
        // Act
        s.save();
        let back = CareerSave::load(&DataDir(dir.clone()));
        // Assert
        assert_eq!(back.career.xp, 4_600);
        assert_eq!(back.career.rank(), brain::career::Rank::Sergeant);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
