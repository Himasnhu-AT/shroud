#!/usr/bin/env python3
"""Launch Shroud (windowed by default) with the control port, run a script of
commands, save screenshots, quit. For tests and visual checks; never grabs the mouse.

  python3 -I scripts/play.py steps.json [--headless | --onscreen] [--out DIR] [--seed N] [--no-build] [--exe BIN] [--extra ARG ...] [--menu]

--exe BIN runs that binary instead of target/debug/shroud (implies --no-build): build, copy
and check your binary first when sibling worktrees share the target dir and may overwrite it.

Builds this checkout first and runs a private copy of the binary.

Default is --offscreen: hidden window, camera renders to an image, so screenshots work even
when another app is fullscreen. --onscreen shows the window instead.

steps.json: a list of steps, each one of
  {"sleep": 2.5}
  {"cmd": "player.look", "params": {"at": [0, 1.2, -27]}}
  {"shot": "name"}            -> DIR/name.png (windowed only)
  {"capture": "name", "seconds": 6, "fps": 60}
                              -> starts capture.frames into DIR/name/ (fixed time step; footage)
  {"wait_frame": 120}         -> waits until the running capture has rendered that many frames
                                 (game time = frames / fps): schedule actions inside a capture
  {"wait_capture": true}      -> waits for the capture to end and every PNG to be written, then
                                 encodes DIR/name.mp4 (H.264, yuv420p) with ffmpeg and deletes the PNGs
                                 (add "keep": true to keep them)
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
        subprocess.run(["cargo", "build", "-q", "-p", "game"], cwd=root, check=True)
    exe = os.path.join(out, "shroud")
    import shutil
    shutil.copy2(src, exe)
    log = open(os.path.join(out, "game.log"), "w")
    cmd = [exe, "--control", "0", "--seed", seed, "--headless" if headless else "--play"]
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
    deadline = time.time() + 900  # first build can be slow
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
        def call(command, params=None):
            nonlocal n
            n += 1
            f.write(json.dumps({"id": n, "key": key, "method": "engine.execute", "params": {"command": command, "params": params or {}}}) + "\n"); f.flush()
            return json.loads(f.readline())
        cap = None
        for st in steps:
            if "sleep" in st:
                time.sleep(st["sleep"]); continue
            if "capture" in st:
                cap = (st["capture"], st.get("fps", 60), st.get("keep", False))
                r = call("capture.frames", {"dir": os.path.join(out, cap[0]), "fps": cap[1], "seconds": st.get("seconds", 5)})
                print(json.dumps(r)[:500], flush=True); continue
            if "wait_frame" in st:
                while (call("capture.status").get("result") or {}).get("frame", 1 << 30) < st["wait_frame"]:
                    time.sleep(0.05)
                continue
            if "wait_capture" in st:
                while True:
                    r = call("capture.status").get("result") or {}
                    if not r.get("active") and r.get("saved", 0) + r.get("failed", 0) >= r.get("total", 0):
                        break
                    time.sleep(0.2)
                print(json.dumps(r), flush=True)
                if cap:
                    d = os.path.join(out, cap[0])
                    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-framerate", str(cap[1]), "-i", os.path.join(d, "f_%05d.png"),
                                    "-c:v", "libx264", "-preset", "slow", "-crf", "16", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
                                    os.path.join(out, cap[0] + ".mp4")], check=False)
                    if not cap[2]:
                        shutil.rmtree(d, ignore_errors=True)
                    print(f"encoded {cap[0]}.mp4", flush=True)
                continue
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
