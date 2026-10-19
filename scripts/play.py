#!/usr/bin/env python3
"""Launch Shroud (windowed by default) under memgate with the control port, run a script of
commands, save screenshots, quit. For tests and visual checks; never grabs the mouse.

  python3 -I scripts/play.py steps.json [--headless | --onscreen] [--out DIR] [--seed N]

Default is --offscreen: hidden window, camera renders to an image, so screenshots work even
when another app is fullscreen. --onscreen shows the window instead.

steps.json: a list of steps, each one of
  {"sleep": 2.5}
  {"cmd": "player.look", "params": {"at": [0, 1.2, -27]}}
  {"shot": "name"}            -> DIR/name.png (windowed only)
Every reply is printed as one JSON line.
"""
import json, os, re, socket, subprocess, sys, tempfile, time

def main():
    args = sys.argv[1:]
    steps = json.load(open(args[0]))
    headless = "--headless" in args
    out = args[args.index("--out") + 1] if "--out" in args else tempfile.mkdtemp(prefix="shroud-shots-")
    seed = args[args.index("--seed") + 1] if "--seed" in args else "1"
    os.makedirs(out, exist_ok=True)
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    exe = os.path.join("/Users/himanshu/codes/game/shroud/target/debug/shroud")
    log = open(os.path.join(out, "game.log"), "w")
    cmd = ["memgate", "--", exe, "--control", "0", "--seed", seed, "--headless" if headless else "--play"]
    if not headless and "--onscreen" not in args:
        cmd.append("--offscreen")
    env = dict(os.environ, SHROUD_DATA_DIR=os.environ.get("SHROUD_DATA_DIR", os.path.join(out, "data")))
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=log, text=True, cwd=root, env=env)
    port = key = None
    deadline = time.time() + 900  # memgate may queue us
    while time.time() < deadline:
        line = proc.stdout.readline()
        if not line:
            if proc.poll() is not None:
                sys.exit(f"game exited early; see {out}/game.log")
            continue
        m = re.search(r"control: port=(\d+) key=(\S+)", line)
        if m:
            port, key = int(m.group(1)), open(m.group(2)).read().strip()
            break
    s = socket.create_connection(("127.0.0.1", port))
    f = s.makefile("rw")
    n = 0
    try:
        # Wait until a round is live (first windowed frames can be slow while shaders compile;
        # commands sent before the round starts get undone by its reset).
        for _ in range(240):
            n += 1
            f.write(json.dumps({"id": n, "key": key, "method": "engine.execute", "params": {"command": "game.state"}}) + "\n"); f.flush()
            st = json.loads(f.readline()).get("result") or {}
            if st.get("state") == "Playing" and (st.get("frame") or 0) > 30:
                break
            time.sleep(0.5)
        time.sleep(1)
        for st in steps:
            if "sleep" in st:
                time.sleep(st["sleep"]); continue
            n += 1
            if "shot" in st:
                req = {"command": "capture.screenshot", "params": {"path": os.path.join(out, st["shot"] + ".png")}}
            else:
                req = {"command": st["cmd"], "params": st.get("params", {})}
            f.write(json.dumps({"id": n, "key": key, "method": "engine.execute", "params": req}) + "\n"); f.flush()
            print(f.readline().strip()[:2000], flush=True)
        f.write(json.dumps({"id": 0, "key": key, "method": "engine.execute", "params": {"command": "game.quit"}}) + "\n"); f.flush()
        f.readline()
    finally:
        try: proc.wait(timeout=15)
        except subprocess.TimeoutExpired: proc.kill()
    print(f"output: {out}")

main()
