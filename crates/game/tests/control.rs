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
