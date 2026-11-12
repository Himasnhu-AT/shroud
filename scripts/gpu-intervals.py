#!/usr/bin/env python3
"""Sum an exported `metal-gpu-intervals` table (xctrace) per label for one process.
  python3 -I scripts/gpu-intervals.py gi.xml PID [FRAMES]
FRAMES defaults to how many times the tonemapping pass ran (once per frame)."""
import sys, collections, xml.etree.ElementTree as ET
path, pid = sys.argv[1], sys.argv[2]
frames = float(sys.argv[3]) if len(sys.argv) > 3 else 0.0
count = collections.Counter()
ids = {}
def val(e):
    if e is None: return None
    if "ref" in e.attrib: return ids.get(e.attrib["ref"])
    v = e.attrib.get("fmt", e.text)
    if "id" in e.attrib: ids[e.attrib["id"]] = (v, e)
    return (v, e)
by_label = collections.Counter(); by_depth0 = collections.Counter(); total = 0.0
for ev, row in ET.iterparse(path, events=("end",)):
    if row.tag != "row":
        continue
    cells = list(row)
    # resolve every cell (registers ids, including nested ones)
    resolved = []
    for c in cells:
        for sub in c.iter():
            if "id" in sub.attrib and sub is not c:
                ids[sub.attrib["id"]] = (sub.attrib.get("fmt", sub.text), sub)
        resolved.append(val(c))
    dur = int(resolved[1][1].text if resolved[1][1].text else 0) / 1e6
    depth = resolved[5][0] if resolved[5] else "?"
    label = resolved[6][0] if resolved[6] else ""
    proc = resolved[10][0] if resolved[10] else ""
    chan = resolved[2][0] if resolved[2] else ""
    if pid not in str(proc):
        row.clear(); continue
    if str(depth) == "0":
        total += dur; by_depth0[chan] += dur
    else:
        k = (chan, label.split(" (")[0].strip())
        by_label[k] += dur
        count[k] += 1
    row.clear()
frames = frames or max([c for (ch, l), c in count.items() if l.startswith("taa") or l.startswith("tonemapping") or l.startswith("bodycam_post")] or [1])
span = None
print(f"GPU busy per frame: {total / frames:.2f} ms ({frames:.0f} frames; frames = runs of the TAA / bodycam pass); by channel:", {k: round(v / frames, 2) for k, v in by_depth0.items()})
print("per label (labels are what the driver reports; unlabeled work is only in the total):")
for (chan, label), v in by_label.most_common(30):
    print(f"  {v / frames:7.3f} ms  x{count[(chan, label)]:<5} {chan:<10} {label[:90]}")
