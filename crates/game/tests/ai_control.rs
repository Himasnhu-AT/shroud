//! Squad AI through the control port (headless): difficulty applied at spawn, grenades thrown,
//! flashbang whiteout computed.
#![cfg(feature = "control")]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ashlar_control::Client;
use serde_json::{Value, json};

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
        .env("SHROUD_DATA_DIR", std::env::temp_dir().join(format!("shroud-test-ai-{}", std::process::id())))
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
    let line = rx.recv_timeout(Duration::from_secs(60)).expect("no `control:` line on stdout within 60 s");
    let field = |name: &str| {
        let rest = &line[line.find(name).expect(name) + name.len()..];
        if name == "key=" { rest.to_string() } else { rest.split_whitespace().next().unwrap_or("").to_string() }
    };
    let port = field("port=").parse().expect("port");
    let key = PathBuf::from(field("key="));
    (game, port, key)
}

fn exec(c: &mut Client, command: &str, params: Value) -> Value {
    c.call("engine.execute", json!({"command": command, "params": params})).unwrap_or_else(|e| panic!("{command}: {e}"))
}

/// Poll `ai.squad` until `f` holds (or time out).
fn wait_for(c: &mut Client, secs: f32, f: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs_f32(secs);
    loop {
        let s = exec(c, "ai.squad", json!({}));
        if f(&s) || Instant::now() > deadline {
            return s;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn difficulty_and_grenades() {
    let (_game, port, key_file) = launch();
    let mut c = Client::connect(port, key_file.parent().expect("key dir")).expect("connect");
    c.set_timeout(Duration::from_secs(30));

    exec(&mut c, "game.start", json!({}));
    std::thread::sleep(Duration::from_millis(300));
    // Elite from the next round: end this one.
    let d = exec(&mut c, "ai.difficulty", json!({"level": "elite"}));
    assert_eq!(d["squad_size"], json!(6), "{d}");
    exec(&mut c, "enemy.kill", json!({}));
    // Wait for the next round's squad to be alive (not just the setting to read Elite).
    let alive = |c: &mut Client| exec(c, "enemy.list", json!({})).as_array().map_or(0, |a| a.len());
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(&mut c) != 6 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let s = exec(&mut c, "ai.squad", json!({}));
    assert_eq!(s["difficulty"], json!("Elite"), "{s}");
    let n = alive(&mut c);
    assert!(n == 6 && s["size"].as_u64() == Some(n as u64), "elite squad: {n} enemies, {s}");
    assert_eq!(s["max_tokens"], json!(3));
    assert!(s["utility"]["frags"].as_u64() >= Some(1), "{s}");

    // Stand in front of the squad (god mode), let them see him, then force a frag.
    exec(&mut c, "player.god", json!({"on": true}));
    exec(&mut c, "player.teleport", json!({"x": 0, "z": -16}));
    exec(&mut c, "player.look", json!({"at": [0, 1.2, -28]}));
    std::thread::sleep(Duration::from_millis(600));
    exec(&mut c, "ai.throw", json!({"kind": "frag"}));
    let s = wait_for(&mut c, 4.0, |s| s["utility"]["frags_used"].as_u64() >= Some(1));
    assert!(s["utility"]["frags_used"].as_u64() >= Some(1), "frag thrown: {s}");
    // It flies, lands and goes off within its fuse.
    let s = wait_for(&mut c, 4.0, |s| s["grenades"].as_array().is_some_and(|g| g.is_empty()));
    assert!(s["grenades"].as_array().is_some_and(|g| g.is_empty()), "frag detonated: {s}");

    // A flashbang he is looking toward whites the screen out.
    exec(&mut c, "ai.throw", json!({"kind": "flash"}));
    let s = wait_for(&mut c, 12.0, |s| s["flashed"]["dur"].as_f64().is_some_and(|d| d > 0.0));
    assert!(s["flashed"]["dur"].as_f64().is_some_and(|d| d > 0.0), "flashed: {s}");
    assert!(s["bandit"]["context"].as_array().is_some_and(|a| a.len() == 10), "{s}");
}
