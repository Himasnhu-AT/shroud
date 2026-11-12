#!/usr/bin/env python3
"""Per-pass GPU time on Foundry with Xcode's Metal System Trace (engine docs/GAME_GUIDE.md 8:
Apple GPUs have no timestamps inside passes, so perf.stats can't split the frame).

  python3 -I scripts/gpu-trace.py --exe BIN --quality High|Ultra [--onscreen] [--secs 3] [--out DIR] [--set JSON]

Same view as perf-ab.py (A site, squad in view), vsync off. Records a trace, exports the GPU
intervals and prints GPU ms per frame for each command buffer / encoder label, plus the total.
--set '[["look.feature", {"name": "ssao", "on": false}]]' applies commands before recording.
"""
import json, os, re, shutil, socket, subprocess, sys, time, collections
import xml.etree.ElementTree as ET

args = sys.argv[1:]
def opt(name, default=None):
    return args[args.index(name) + 1] if name in args else default
exe_src = opt("--exe"); quality = opt("--quality", "High"); onscreen = "--onscreen" in args
secs = float(opt("--secs", "3")); extra = json.loads(opt("--set", "[]"))
out = opt("--out", f".tmp/gpu-trace-{quality.lower()}-{'on' if onscreen else 'off'}")
shutil.rmtree(out, ignore_errors=True)
os.makedirs(os.path.join(out, "data"))
json.dump({"core": {"quality": quality, "vsync": False, "window_mode": "Borderless" if onscreen else "Windowed"}, "game": {"map": "Foundry", "rev": 2}}, open(os.path.join(out, "data", "settings.json"), "w"))
exe = os.path.join(out, "shroud"); shutil.copy2(exe_src, exe)
proc = subprocess.Popen([exe, "--control", "0", "--seed", "1", "--play", "--mute"] + ([] if onscreen else ["--offscreen"]), stdout=subprocess.PIPE,
                        stderr=open(os.path.join(out, "game.log"), "w"), text=True, env=dict(os.environ, SHROUD_DATA_DIR=os.path.join(out, "data")))
while True:
    line = proc.stdout.readline()
    if not line: sys.exit("game exited early")
    m = re.search(r"control: port=(\d+) key=(.+)$", line.rstrip())
    if m: port, key = int(m.group(1)), open(m.group(2)).read().strip(); break
f = socket.create_connection(("127.0.0.1", port)).makefile("rw"); n = 0
def call(c, p=None):
    global n; n += 1
    f.write(json.dumps({"id": n, "key": key, "method": "engine.execute", "params": {"command": c, "params": p or {}}}) + "\n"); f.flush()
    return json.loads(f.readline())
for _ in range(600):
    st = call("game.state").get("result") or {}; ld = call("loading.status").get("result") or {}
    if st.get("state") == "Playing" and not ld.get("holds", False) and (st.get("frame") or 0) > 30: break
    time.sleep(0.5)
for c, p in [("render.governor", {"enabled": False}), ("match.time_limit", {}), ("enemy.freeze", {"on": True}), ("player.god", {"on": True}),
             ("player.teleport", {"x": -18, "z": -18}), ("player.look", {"at": [-34, 1.6, -34]})]:
    call(c, p)
for i, (x, z) in enumerate([(-24, -24), (-26, -22), (-22, -26), (-28, -27), (-25, -29), (-30, -24)]):
    call("enemy.place", {"id": i, "x": x, "z": z, "face": [-18, -18]})
for c, p in extra:
    print(json.dumps(call(c, p))[:200])
time.sleep(8)
frames0 = (call("render.stats").get("result") or {}).get("frames_rendered", 0)
trace = os.path.join(out, "gpu.trace")
subprocess.run(["xcrun", "xctrace", "record", "--template", "Metal System Trace", "--attach", str(proc.pid), "--time-limit", f"{secs}s", "--output", trace],
               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
frames1 = (call("render.stats").get("result") or {}).get("frames_rendered", 0)
ps = call("perf.stats").get("result") or {}
call("game.quit")
try: proc.wait(timeout=15)
except subprocess.TimeoutExpired: proc.kill()
gi = os.path.join(out, "gi.xml")
with open(gi, "w") as fh:
    subprocess.run(["xcrun", "xctrace", "export", "--input", trace, "--xpath", '/trace-toc/run[@number="1"]/data/table[@schema="metal-gpu-intervals"]'], stdout=fh, stderr=subprocess.DEVNULL)
shutil.rmtree(trace, ignore_errors=True)  # ~150 MB
r = subprocess.run([sys.executable, "-I", os.path.join(os.path.dirname(os.path.abspath(__file__)), "gpu-intervals.py"), gi, str(proc.pid)], capture_output=True, text=True).stdout
os.remove(gi)
mon = ps.get("perf", {}).get("monitor", {})
summary = {"quality": quality, "onscreen": onscreen, "set": extra, "fps": round(mon.get("fps", {}).get("now") or 0, 1), "frame_p50": ps.get("frame_ms", {}).get("p50"), "os_gpu_ms": mon.get("gpu", {}).get("frame_ms", {}).get("now")}
print(json.dumps(summary))
print(r)
