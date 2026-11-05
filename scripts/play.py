#!/usr/bin/env python3
"""Launch Shroud (windowed by default) under memgate with the control port, run a script of
commands, save screenshots, quit. For tests and visual checks; never grabs the mouse.

  python3 -I scripts/play.py steps.json [--headless | --onscreen] [--out DIR] [--seed N] [--no-build] [--exe BIN] [--extra ARG ...] [--menu]

--exe BIN runs that binary instead of target/debug/shroud (implies --no-build): build, copy
and check your binary first when sibling worktrees share the target dir and may overwrite it.

Builds this checkout first (via memgate) and runs a private copy of the binary.

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
    # Build *this* checkout and snapshot the binary right away: the shared target dir means
    # another worktree's build can overwrite target/debug/shroud at any moment.
    src = args[args.index("--exe") + 1] if "--exe" in args else "/Users/himanshu/codes/game/engine/target/debug/shroud"
    if "--no-build" not in args and "--exe" not in args:
        subprocess.run(["memgate", "--", "cargo", "build", "-q", "-p", "game"], cwd=root, check=True)
    exe = os.path.join(out, "shroud")
    import shutil
    shutil.copy2(src, exe)
    log = open(os.path.join(out, "game.log"), "w")
    cmd = ["memgate", "--", exe, "--control", "0", "--seed", seed, "--headless" if headless else "--play"]
    if "--menu" in args:  # boot into the main menu like a normal launch (start with game.start)
        cmd.remove("--play")
    cmd.append("--mute")  # never make noise (the user may be in a meeting)
    if not headless and "--onscreen" not in args:
        cmd.append("--offscreen")
    # --extra ARG (repeatable) passes ARG to the game, e.g. --extra --metrics-window
    cmd += [args[i + 1] for i, a in enumerate(args) if a == "--extra" and i + 1 < len(args)]
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
        m = re.search(r"control: port=(\d+) key=(.+)$", line.rstrip())
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
