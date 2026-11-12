//! Drives a headless `shroud` through its control port.
#![cfg(feature = "dev")]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ashlar_control::Client;
use serde_json::{Value, json};

/// Kills the game if the test fails before `game.quit`.
struct Game(Child);

impl Drop for Game {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn launch() -> (Game, u16, PathBuf) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_shroud"))
        .args(["--headless", "--control", "0", "--seed", "1"])
        .env_remove("SHROUD_TRACE_ENDPOINT")
        .env_remove("SHROUD_TRACE_KEY")
        // Own data dir: never read or write the player's settings.json / brain.json.
        .env("SHROUD_DATA_DIR", std::env::temp_dir().join(format!("shroud-test-control-{}", std::process::id())))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn shroud");
    let stdout = child.stdout.take().expect("stdout");
    let game = Game(child);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.starts_with("control:") {
                let _ = tx.send(line);
            }
        }
    });
    let line = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("no `control:` line on stdout within 60 s");
    let port = field(&line, "port=").parse().expect("port");
    let key = PathBuf::from(field(&line, "key="));
    (game, port, key)
}

fn field<'a>(line: &'a str, name: &str) -> &'a str {
    let rest = &line[line.find(name).unwrap_or_else(|| panic!("`{name}` in {line}")) + name.len()..];
    // The key path is last and may contain spaces ("Application Support").
    if name == "key=" { rest } else { rest.split_whitespace().next().unwrap_or("") }
}

fn exec(c: &mut Client, command: &str, params: Value) -> Value {
    c.call("engine.execute", json!({"command": command, "params": params}))
        .unwrap_or_else(|e| panic!("{command}: {e}"))
}

fn pos(state: &Value) -> [f64; 3] {
    let p = &state["player"]["pos"];
    [0, 1, 2].map(|i| p[i].as_f64().expect("pos"))
}

#[test]
fn control_port_drives_the_game() {
    let (mut game, port, key_file) = launch();
    let mut c = Client::connect(port, key_file.parent().expect("key dir")).expect("connect");
    c.set_timeout(Duration::from_secs(30));

    assert_eq!(c.call("app.ping", json!({})).expect("ping"), json!("pong"));

    let cmds = c.call("engine.commands", json!({})).expect("commands");
    let ids: Vec<&str> = cmds.as_array().expect("list").iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"player.move"), "commands: {ids:?}");

    exec(&mut c, "game.start", json!({}));
    // Let the state change and physics settle.
    std::thread::sleep(Duration::from_millis(300));
    let before = pos(&exec(&mut c, "game.state", json!({})));
    let moved = exec(&mut c, "player.move", json!({"forward": 1, "ms": 500}));
    let after = pos(&exec(&mut c, "game.state", json!({})));
    let d = ((after[0] - before[0]).powi(2) + (after[2] - before[2]).powi(2)).sqrt();
    assert!(d > 0.5, "player barely moved: {before:?} -> {after:?} (move replied {moved})");

    let fired = exec(&mut c, "player.fire", json!({"shots": 3}));
    assert_eq!(fired["shots"], json!(3), "{fired}");
    assert!(fired["hits"].is_u64() && fired["damage"].is_number(), "{fired}");

    exec(&mut c, "game.quit", json!({}));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = game.0.try_wait().expect("wait") {
            assert!(status.success(), "exit status {status}");
            break;
        }
        assert!(Instant::now() < deadline, "game did not exit after game.quit");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn arsenal_over_the_control_port() {
    let (mut game, port, key_file) = launch();
    let mut c = Client::connect(port, key_file.parent().expect("key dir")).expect("connect");
    c.set_timeout(Duration::from_secs(30));
    exec(&mut c, "game.start", json!({}));
    std::thread::sleep(Duration::from_millis(300));
    exec(&mut c, "enemy.freeze", json!({"on": true}));

    // Round 1 of a match: pistol only, $800, no freeze time in test runs.
    let s = exec(&mut c, "weapon.state", json!({}));
    assert_eq!(s["kind"], json!("Pistol"), "{s}");
    assert_eq!(s["slots"][1]["kind"], json!("Pistol"), "{s}");
    let e = exec(&mut c, "econ.state", json!({}));
    assert_eq!(e["money"], json!(800), "{e}");
    assert_eq!(e["freeze_left"], json!(0.0), "{e}");

    // Buy menu rules: too poor for a rifle; with money it's bought, drawn and paid for.
    let poor = c.call("engine.execute", json!({"command": "buy", "params": {"item": "rifle"}}));
    assert!(poor.is_err(), "bought a rifle with $800: {poor:?}");
    exec(&mut c, "econ.set_money", json!({"money": 5000}));
    let bought = exec(&mut c, "buy", json!({"item": "rifle"}));
    assert_eq!(bought["money"], json!(5000 - 2900), "{bought}");
    let bought = exec(&mut c, "buy", json!({"item": "helmet"}));
    assert_eq!(bought["money"], json!(5000 - 2900 - 1000), "{bought}");
    let s = exec(&mut c, "weapon.state", json!({}));
    assert_eq!(s["kind"], json!("Rifle"), "{s}");
    let e = exec(&mut c, "econ.state", json!({}));
    assert_eq!(e["armor"]["helmet"], json!(true), "{e}");
    let owned = c.call("engine.execute", json!({"command": "buy", "params": {"item": "rifle"}}));
    assert!(owned.is_err(), "bought the rifle twice: {owned:?}");

    // Pump shotgun: one trigger pull per shot, one shell gone per shot.
    exec(&mut c, "weapon.give", json!({"kind": "shotgun"}));
    std::thread::sleep(Duration::from_millis(900));
    let fired = exec(&mut c, "player.fire", json!({"shots": 2}));
    assert_eq!(fired["shots"], json!(2), "{fired}");
    let s = exec(&mut c, "weapon.state", json!({}));
    assert_eq!(s["kind"], json!("Shotgun"), "{s}");
    assert_eq!(s["ammo"], json!(4), "{s}");

    // ADS tightens the cone.
    let hip = s["spread"].as_f64().expect("spread");
    exec(&mut c, "weapon.ads", json!({"on": true}));
    std::thread::sleep(Duration::from_millis(600));
    let aimed = exec(&mut c, "weapon.state", json!({}));
    assert!(aimed["spread"].as_f64().expect("spread") < hip, "{aimed} vs hip {hip}");
    exec(&mut c, "weapon.ads", json!({"on": false}));

    // Sidearm; the shotgun keeps its shells in its slot.
    exec(&mut c, "weapon.switch", json!({"slot": 2}));
    std::thread::sleep(Duration::from_millis(900));
    let s = exec(&mut c, "weapon.state", json!({}));
    assert_eq!(s["kind"], json!("Pistol"), "{s}");
    assert_eq!(s["slots"][0]["ammo"], json!(4), "{s}");
    let fired = exec(&mut c, "player.fire", json!({"shots": 3}));
    assert_eq!(fired["shots"], json!(3), "{fired}");

    // Walk-over ammo: a dropped pistol tops up the reserve.
    let before = exec(&mut c, "weapon.state", json!({}))["reserve"].as_u64().expect("reserve");
    let p = pos(&exec(&mut c, "game.state", json!({})));
    exec(&mut c, "weapon.drop_pickup", json!({"kind": "pistol", "x": p[0], "z": p[2]}));
    std::thread::sleep(Duration::from_millis(300));
    let after = exec(&mut c, "weapon.state", json!({}))["reserve"].as_u64().expect("reserve");
    assert!(after > before, "reserve {before} -> {after}");

    exec(&mut c, "game.quit", json!({}));
    let deadline = Instant::now() + Duration::from_secs(15);
    while game.0.try_wait().expect("wait").is_none() {
        assert!(Instant::now() < deadline, "game did not exit after game.quit");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Polls `command` until `done` holds (or panics after `secs`).
fn wait_for(c: &mut Client, command: &str, secs: u64, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let v = exec(c, command, json!({}));
        if done(&v) {
            return v;
        }
        assert!(Instant::now() < deadline, "{command}: still {v}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn bomb_hold_plant_then_explosion_wins_and_defuse_loses() {
    // Arrange
    let (_game, port, key_file) = launch();
    let mut c = Client::connect(port, key_file.parent().expect("key dir")).expect("connect");
    c.set_timeout(Duration::from_secs(30));
    exec(&mut c, "game.start", json!({}));
    std::thread::sleep(Duration::from_millis(300));
    exec(&mut c, "enemy.freeze", json!({"on": true}));
    exec(&mut c, "player.god", json!({"on": true}));

    // Round 1: the player carries the bomb; holding E in site A plants it.
    let s = exec(&mut c, "bomb.state", json!({}));
    assert_eq!(s["phase"], json!("carried"), "{s}");
    assert_eq!(s["sites"].as_array().map(Vec::len), Some(2), "{s}");
    exec(&mut c, "bomb.plant", json!({"site": "A", "plant": false}));
    let s = wait_for(&mut c, "bomb.state", 5, |s| s["can_plant"] == json!(true));
    assert_eq!(s["in_site"], json!("A"), "{s}");
    exec(&mut c, "weapon.interact", json!({"on": true}));
    let s = wait_for(&mut c, "bomb.state", 8, |s| s["phase"] == json!("planted"));
    exec(&mut c, "weapon.interact", json!({"on": false}));
    assert_eq!(s["site"], json!("A"), "{s}");
    assert!(s["fuse_left"].as_f64().expect("fuse") > 30.0, "{s}");

    // Round 2: an instant plant with a short fuse goes off; the player wins the round.
    exec(&mut c, "game.start", json!({}));
    wait_for(&mut c, "bomb.state", 5, |s| s["phase"] == json!("carried"));
    exec(&mut c, "bomb.plant", json!({"site": "B", "fuse": 1.5}));
    wait_for(&mut c, "bomb.state", 6, |s| s["phase"] == json!("exploded"));
    let m = wait_for(&mut c, "match.state", 3, |m| !m["round_over"].is_null());
    assert_eq!(m["round_over"]["player_won"], json!(true), "{m}");
    assert_eq!(m["end"], json!("exploded"), "{m}");

    // Round 3: a defender on the bomb defuses it; the squad wins.
    let round = m["round"].as_u64().expect("round");
    wait_for(&mut c, "match.state", 6, |m| m["round"].as_u64() == Some(round + 1) && m["round_over"].is_null());
    exec(&mut c, "bomb.plant", json!({"site": "A", "fuse": 30}));
    wait_for(&mut c, "bomb.state", 5, |s| s["phase"] == json!("planted"));
    exec(&mut c, "bomb.defuse", json!({}));
    let s = wait_for(&mut c, "bomb.state", 5, |s| s["defuse"].as_f64().unwrap_or(0.0) > 0.05);
    assert!(!s["defuser"].is_null(), "{s}");
    wait_for(&mut c, "bomb.state", 12, |s| s["phase"] == json!("defused"));
    let m = wait_for(&mut c, "match.state", 3, |m| !m["round_over"].is_null());
    assert_eq!(m["round_over"]["player_won"], json!(false), "{m}");
    assert_eq!(m["end"], json!("defused"), "{m}");
    exec(&mut c, "game.quit", json!({}));
}

#[test]
fn career_logs_plants_and_credits_a_finished_match_with_promotion() {
    // Arrange: 900 XP, 100 short of Private; frozen squad, god mode.
    let (_game, port, key_file) = launch();
    let mut c = Client::connect(port, key_file.parent().expect("key dir")).expect("connect");
    c.set_timeout(Duration::from_secs(30));
    exec(&mut c, "game.start", json!({}));
    std::thread::sleep(Duration::from_millis(300));
    exec(&mut c, "enemy.freeze", json!({"on": true}));
    exec(&mut c, "player.god", json!({"on": true}));
    let set = exec(&mut c, "career.set_xp", json!({"xp": 900}));
    assert_eq!(set["rank"], json!("Cadet"), "{set}");
    let s = exec(&mut c, "career.state", json!({}));
    let n = s["objectives"].as_array().map_or(0, Vec::len);
    assert_eq!(n, 3, "a Cadet gets 3 objectives: {s}");
    assert_eq!(s["rounds_logged"], json!(1), "{s}");

    // Act: win three rounds by letting the bomb go off on A.
    for round in 1..=3u64 {
        wait_for(&mut c, "match.state", 8, |m| m["round"].as_u64() == Some(round) && m["round_over"].is_null());
        wait_for(&mut c, "bomb.state", 5, |s| s["phase"] == json!("carried"));
        exec(&mut c, "bomb.plant", json!({"site": "A", "fuse": 1.0}));
        let m = wait_for(&mut c, "match.state", 8, |m| !m["round_over"].is_null());
        assert_eq!(m["round_over"]["player_won"], json!(true), "{m}");
        if round == 1 {
            // Assert: the log saw the plant, the explosion and the win; bomb kills are not his.
            let s = wait_for(&mut c, "career.state", 3, |s| s["rounds"][0]["won"] == json!(true));
            let r = &s["rounds"][0];
            assert_eq!(r["planted"]["site"], json!("A"), "{s}");
            assert_eq!(r["exploded"], json!(true), "{s}");
            assert_eq!(r["kills"].as_array().map(Vec::len), Some(0), "{s}");
            assert_eq!(r["had_primary"], json!(false), "{s}");
            assert_eq!(r["died"], json!(false), "{s}");
        }
    }

    // Assert: the finished match is credited once, saved, and promotes to Private.
    let s = wait_for(&mut c, "career.state", 10, |s| s["credited"] == json!(true));
    let award = &s["award"];
    assert!(award["total"].as_u64().expect("total") >= 500 + 3 * 50, "{s}");
    assert_eq!(award["promoted"], json!(true), "{s}");
    assert_eq!(award["rank_after"], json!("Private"), "{s}");
    assert_eq!(s["rank"], json!("Private"), "{s}");
    assert_eq!((s["matches"].as_u64(), s["wins"].as_u64()), (Some(1), Some(1)), "{s}");
    let xp = s["xp"].as_u64().expect("xp");

    // An abandoned match pays nothing: start one, leave it, start again.
    exec(&mut c, "game.menu", json!({}));
    std::thread::sleep(Duration::from_millis(300));
    exec(&mut c, "game.start", json!({}));
    let s = wait_for(&mut c, "career.state", 10, |s| s["credited"] == json!(false) && s["rounds_logged"] == json!(1));
    assert_eq!(s["xp"].as_u64(), Some(xp), "{s}");
    assert_eq!(s["matches"].as_u64(), Some(1), "{s}");
    exec(&mut c, "game.quit", json!({}));
}

#[test]
fn practice_statues_take_hits_reset_and_nothing_is_recorded() {
    // Arrange: a fresh data dir; note what the squad knows and what the career holds.
    let (_game, port, key_file) = launch();
    let mut c = Client::connect(port, key_file.parent().expect("key dir")).expect("connect");
    c.set_timeout(Duration::from_secs(30));
    // Headless boots into a match round: let it start (and log its round) first.
    wait_for(&mut c, "game.state", 10, |s| s["state"] == json!("Playing"));
    wait_for(&mut c, "career.state", 10, |s| s["rounds_logged"] == json!(1));
    let brain_before = exec(&mut c, "ai.brain", json!({}));
    let career_before = exec(&mut c, "career.state", json!({}));

    // Act: practice on Foundry.
    exec(&mut c, "practice.start", json!({"map": "foundry"}));
    let p = wait_for(&mut c, "practice.state", 10, |p| p["on"] == json!(true) && p["targets"].as_array().is_some_and(|t| !t.is_empty()));
    let n = p["targets"].as_array().map_or(0, Vec::len);
    assert!(n >= 6, "every spawn gets a statue: {p}");
    let recorded = p["recorded"].clone();
    // Play is held behind the practice loading screen (shaders, then the governor's look), which
    // takes longer on a loaded machine: wait until it lets go before moving or shooting.
    wait_for(&mut c, "loading.status", 120, |s| s["holds"] == json!(false));
    std::thread::sleep(Duration::from_millis(300));

    // Statues don't move: same spots a second later.
    std::thread::sleep(Duration::from_millis(1000));
    let later = exec(&mut c, "practice.state", json!({}));
    let (a, b) = (p["targets"].as_array().expect("targets"), later["targets"].as_array().expect("targets"));
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        let d = |i: usize| (a["pos"][i].as_f64().unwrap_or(0.0) - b["pos"][i].as_f64().unwrap_or(9.0)).abs();
        assert!(d(0) < 0.01 && d(1) < 0.01, "a statue moved: {a} -> {b}");
        assert_eq!(b["hp"], json!(100.0), "a statue took damage on its own: {b}");
    }

    // Buying works anywhere, any time, and the money comes back.
    exec(&mut c, "player.teleport", json!({"x": 0, "z": 0}));
    let bought = exec(&mut c, "buy", json!({"item": "rifle"}));
    assert!(bought["money"].is_u64(), "{bought}");
    let e = wait_for(&mut c, "econ.state", 3, |e| e["money"] == json!(16000));
    assert_eq!(e["freeze_left"], json!(0.0), "{e}");

    // Shoot the first statue from 5 m.
    let t = &p["targets"][0]["pos"];
    let (x, z) = (t[0].as_f64().expect("x"), t[1].as_f64().expect("z"));
    exec(&mut c, "player.teleport", json!({"x": x + 5.0, "z": z}));
    exec(&mut c, "player.look", json!({"at": [x, 1.3, z]}));
    std::thread::sleep(Duration::from_millis(900));
    let fired = exec(&mut c, "player.fire", json!({"shots": 6}));
    assert!(fired["hits"].as_u64().unwrap_or(0) > 0, "no hits on a statue at 5 m: {fired}");
    // A reload takes nothing from the reserve: it refills.
    let w0 = exec(&mut c, "weapon.state", json!({}));
    exec(&mut c, "player.reload", json!({}));
    let w1 = wait_for(&mut c, "weapon.state", 6, |w| w["ammo"].as_u64() > w0["ammo"].as_u64() && w["reloading"] != json!(true));
    assert_eq!(w1["reserve"], w0["reserve"], "reserve after a reload: {w0} -> {w1}");

    // Kill them all: no round end, no match, no evolution.
    exec(&mut c, "enemy.kill", json!({}));
    std::thread::sleep(Duration::from_millis(1500));
    let s = exec(&mut c, "game.state", json!({}));
    assert_eq!(s["state"], json!("Playing"), "{s}");
    let m = exec(&mut c, "match.state", json!({}));
    assert!(m["round_over"].is_null() && m["last_result"].is_null(), "{m}");
    assert_eq!((m["player_rounds"].as_u64(), m["enemy_rounds"].as_u64()), (Some(0), Some(0)), "{m}");

    // Shots, kills and the "round" left no trace in the match records.
    let p1 = exec(&mut c, "practice.state", json!({}));
    assert_eq!(p1["recorded"], recorded, "practice was recorded");

    // Reset stands every target up again.
    exec(&mut c, "practice.reset", json!({}));
    let p2 = wait_for(&mut c, "practice.state", 5, |p| {
        p["targets"].as_array().is_some_and(|t| t.len() == n && t.iter().all(|t| t["dead"] == json!(false) && t["hp"] == json!(100.0)))
    });
    assert_eq!(p2["targets"].as_array().map(Vec::len), Some(n));

    // Leave: practice is off on the menu.
    exec(&mut c, "game.menu", json!({}));
    wait_for(&mut c, "practice.state", 5, |p| p["on"] == json!(false));

    // Assert: the squad learned nothing and the career is untouched.
    let brain_after = exec(&mut c, "ai.brain", json!({}));
    assert_eq!(brain_after["generation"], brain_before["generation"]);
    assert_eq!(brain_after["player_model"], brain_before["player_model"]);
    assert_eq!(brain_after["history"], brain_before["history"]);
    let career_after = exec(&mut c, "career.state", json!({}));
    for k in ["xp", "matches", "wins", "rounds_logged", "credited"] {
        assert_eq!(career_after[k], career_before[k], "career.{k} changed in practice");
    }
    exec(&mut c, "game.quit", json!({}));
}
