#!/usr/bin/env python3
"""Same-binary GPU A/B on Foundry: one quality preset, a fixed view with the squad (MetaHumans)
placed in it, then each feature switched off in turn (engine docs/GAME_GUIDE.md section 8).

  python3 -I scripts/perf-ab.py --exe BIN --quality High|Ultra [--onscreen] [--out DIR] [--secs 6]

Offscreen renders 1920x1080. --onscreen opens a borderless fullscreen window on the current
display (on a 14" MacBook Pro: 3024x1964). Prints one line per config: GPU ms/frame (this
process, from the OS), fps, frame p50/p95 (ms), and the deltas against the baseline.
Check `pgrep -fl "studio gen"` (or any other GPU job) first and say so with the numbers.
"""
import json, os, re, shutil, socket, statistics, subprocess, sys, time

args = sys.argv[1:]
def opt(name, default=None):
    return args[args.index(name) + 1] if name in args else default

exe_src = opt("--exe")
quality = opt("--quality", "High")
onscreen = "--onscreen" in args
secs = float(opt("--secs", "6"))
out = opt("--out", f".tmp/perf-ab-{quality.lower()}-{'on' if onscreen else 'off'}")
os.makedirs(os.path.join(out, "data"), exist_ok=True)
QI = {"Low": 0, "Medium": 1, "High": 2, "Ultra": 3}[quality]
with open(os.path.join(out, "data", "settings.json"), "w") as f:
    json.dump({"core": {"quality": quality, "vsync": "--vsync" in args, "window_mode": "Borderless" if onscreen else "Windowed"}, "game": {"map": "Foundry", "rev": 2}}, f)
exe = os.path.join(out, "shroud")
shutil.copy2(exe_src, exe)
cmd = [exe, "--control", "0", "--seed", "1", "--play", "--mute"] + ([] if onscreen else ["--offscreen"])
proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=open(os.path.join(out, "game.log"), "w"), text=True, env=dict(os.environ, SHROUD_DATA_DIR=os.path.join(out, "data")))
while True:
    line = proc.stdout.readline()
    if not line:
        sys.exit("game exited early")
    m = re.search(r"control: port=(\d+) key=(.+)$", line.rstrip())
    if m:
        port, key = int(m.group(1)), open(m.group(2)).read().strip()
        break
f = socket.create_connection(("127.0.0.1", port)).makefile("rw")
n = 0
def call(c, p=None):
    global n
    n += 1
    f.write(json.dumps({"id": n, "key": key, "method": "engine.execute", "params": {"command": c, "params": p or {}}}) + "\n")
    f.flush()
    return json.loads(f.readline())

for _ in range(600):
    st = call("game.state").get("result") or {}
    ld = call("loading.status").get("result") or {}
    if st.get("state") == "Playing" and not ld.get("holds", False) and (st.get("frame") or 0) > 30:
        break
    time.sleep(0.5)
call("render.governor", {"enabled": False})
call("match.time_limit")  # no round time limit: the view must not reset mid-measurement
call("enemy.freeze", {"on": True})
call("player.god", {"on": True})
# A-site interior looking out over the squad (Foundry), squad placed in view.
call("player.teleport", {"x": -18, "z": -18})
call("player.look", {"at": [-34, 1.6, -34]})
for i, (x, z) in enumerate([(-24, -24), (-26, -22), (-22, -26), (-28, -27), (-25, -29), (-30, -24)]):
    call("enemy.place", {"id": i, "x": x, "z": z, "face": [-18, -18]})
time.sleep(4)

def measure(label):
    # Toggled view features rebuild pipelines (a stall): let that pass first.
    time.sleep(6.0)
    g, fps, p50, p95 = [], [], [], []
    t_end = time.time() + secs
    while time.time() < t_end:
        time.sleep(1.0)
        r = call("perf.stats").get("result") or {}
        mon = r.get("perf", {}).get("monitor", {})
        g.append(mon.get("gpu", {}).get("frame_ms", {}).get("now") or 0)
        fps.append(mon.get("fps", {}).get("now") or 0)
        p50.append(r.get("frame_ms", {}).get("p50") or 0)
        p95.append(r.get("frame_ms", {}).get("p95") or 0)
    row = {"config": label, "gpu_ms": round(statistics.median(g), 2), "fps": round(statistics.median(fps), 1), "p50": round(statistics.median(p50), 2), "p95": round(statistics.median(p95), 2)}
    print(json.dumps(row), flush=True)
    return row

# Each variant is measured between two baselines (the machine drifts: clocks, other jobs);
# its delta is against their mean.
variants = []
features = ["ssao", "contact_shadows", "bloom", "taa", "shadows", "fog"] + (["volumetric", "ssr"] if quality == "Ultra" else [])
for feat in features:
    variants.append((f"-{feat}", [("look.feature", {"name": feat, "on": False})], [("look.feature", {"name": feat, "on": True})]))
if quality == "Ultra":
    variants.append(("-overscan (1.25->1.0)", [("look.set", {"fields": {"lens": {"bodycam": {"fisheye": {"overscan": 1.0}}}}})], [("look.set", {"fields": {"lens": {"bodycam": {"fisheye": {"overscan": 1.25}}}}})]))
variants.append(("probes frozen (refresh 0)", [("look.probe", {"refresh": 0})], [("look.probe", {"refresh": -1})]))
variants.append(("-bodycam pass", [("look.set", {"fields": {"lens": {"bodycam": {"enabled": False}}}})], [("look.set", {"fields": {"lens": {"bodycam": {"enabled": True}}}})]))
for sc in [0.85, 0.75, 0.5]:
    variants.append((f"render scale {sc}", [("render.governor", {"scale": sc})], [("render.governor", {"scale": 1.0})]))
away = [("enemy.place", {"id": i, "x": 30, "z": 30 + i}) for i in range(6)]
back = [("enemy.place", {"id": i, "x": x, "z": z, "face": [-18, -18]}) for i, (x, z) in enumerate([(-24, -24), (-26, -22), (-22, -26), (-28, -27), (-25, -29), (-30, -24)])]
variants.append(("-6 MetaHumans in view", away, back))
only = opt("--only")
if only:
    variants = [v for v in variants if any(o in v[0] for o in only.split(","))]

base0 = measure(f"{quality} baseline")
call("capture.screenshot", {"path": os.path.join(out, "view.png")})
rows, prev = [], base0
for label, on, off in variants:
    for c, p in on:
        r = call(c, p)
        if not r.get("ok"):
            print(json.dumps({"config": label, "error": r.get("error")}))
    v = measure(label)
    for c, p in off:
        call(c, p)
    b = measure("baseline")
    ref = {k: (prev[k] + b[k]) / 2 for k in ("gpu_ms", "p50", "p95", "fps")}
    rows.append((label, v, ref))
    prev = b
print("summary", quality, "onscreen" if onscreen else "offscreen 1920x1080", "| baseline", json.dumps(base0))
for label, v, ref in rows:
    print(f"  {label:<24} p50 {v['p50']:6.2f} ({v['p50'] - ref['p50']:+.2f})  gpu {v['gpu_ms']:6.2f} ({v['gpu_ms'] - ref['gpu_ms']:+.2f})  p95 {v['p95']:6.2f}  fps {v['fps']:6.1f}")
call("render.stats")
call("game.quit")
try:
    proc.wait(timeout=15)
except subprocess.TimeoutExpired:
    proc.kill()
