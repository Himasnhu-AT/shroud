//! Drives a headless `shroud` through its control port.
#![cfg(feature = "control")]

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
