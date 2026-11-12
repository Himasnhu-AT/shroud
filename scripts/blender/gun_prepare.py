"""Prepare a downloaded gun model for the game (headless Blender).

    Blender -b --factory-startup --python scripts/blender/gun_prepare.py -- \
        <in.glb|gltf|fbx> <out.glb> --kind rifle [--length 0.9] [--muzzle +x|-x|+y|-y] \
        [--keep NAME,...] [--drop NAME,...] [--max-texture 2048]

Input is a file the engine's importer (`assets.py import`) already unpacked into
`assets/library/...` (never a raw download). Output is a game-ready GLB in the game's gun frame:
metres, +Y up, barrel toward -Z, centred left-right, rest pose baked, textures capped at
`--max-texture` (8K Fab textures become 2K). It then prints a `guns.json` entry with estimated
anchors (muzzle, sight line, firing hand, support hand, centre). Check them with the ADS shots in
`scripts/arms-look.json` and adjust.

`--muzzle` names the Blender axis the barrel points along (default: the long horizontal axis's
thinner end). `--keep` / `--drop` filter mesh objects by name substring (packs often ship spare
magazines, bullets, display stands).
"""
import json
import math
import sys

import bpy
from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1:]
SRC, OUT = argv[0], argv[1]


def opt(name, default=None):
    if name in argv:
        return argv[argv.index(name) + 1]
    return default


KIND = opt("--kind", "rifle")
LENGTH = float(opt("--length", "0") or 0)
MUZZLE = opt("--muzzle")
KEEP = [s for s in (opt("--keep", "") or "").split(",") if s]
DROP = [s for s in (opt("--drop", "") or "").split(",") if s]
MAX_TEX = int(opt("--max-texture", "2048"))

bpy.ops.wm.read_factory_settings(use_empty=True)
if SRC.lower().endswith(".fbx"):
    bpy.ops.import_scene.fbx(filepath=SRC)
else:
    bpy.ops.import_scene.gltf(filepath=SRC)

# Scene meshes only (the glTF importer keeps a bone-shape icosphere outside the scene).
meshes = [o for o in bpy.context.scene.objects if o.type == "MESH" and not o.name.startswith("Icosphere")]
if KEEP:
    meshes = [o for o in meshes if any(k.lower() in o.name.lower() for k in KEEP)]
meshes = [o for o in meshes if not any(d.lower() in o.name.lower() for d in DROP)]
if not meshes:
    sys.exit("gun_prepare: no meshes left")

# Bake modifiers (rigs at rest) and every parent transform into single-user mesh data.
for o in meshes:
    if o.data.users > 1:
        o.data = o.data.copy()
    with bpy.context.temp_override(object=o, active_object=o, selected_objects=[o]):
        for m in list(o.modifiers):
            try:
                bpy.ops.object.modifier_apply(modifier=m.name)
            except RuntimeError:
                o.modifiers.remove(m)
    mw = o.matrix_world.copy()
    o.parent = None
    o.data.transform(mw)
    o.matrix_world.identity()
    o.vertex_groups.clear()
for o in list(bpy.data.objects):
    if o not in meshes:
        bpy.data.objects.remove(o, do_unlink=True)


def all_verts():
    return [v.co.copy() for o in meshes for v in o.data.vertices]


pts = all_verts()
lo = Vector([min(p[i] for p in pts) for i in range(3)])
hi = Vector([max(p[i] for p in pts) for i in range(3)])
size = hi - lo
# Blender is Z-up: the barrel lies along X or Y.
long_axis = 0 if size.x >= size.y else 1


def end_area(sign):
    """Cross-section (thickness x height) of the last 6 % of the gun toward `sign`."""
    L = size[long_axis]
    cut = hi[long_axis] - 0.06 * L if sign > 0 else lo[long_axis] + 0.06 * L
    sel = [p for p in pts if (p[long_axis] >= cut if sign > 0 else p[long_axis] <= cut)]
    if not sel:
        return 1e9
    other = 1 - long_axis
    return (max(p[other] for p in sel) - min(p[other] for p in sel)) * (max(p.z for p in sel) - min(p.z for p in sel))


if MUZZLE:
    axis = {"x": 0, "y": 1}[MUZZLE[1]]
    sign = 1.0 if MUZZLE[0] == "+" else -1.0
else:
    axis = long_axis
    sign = 1.0 if end_area(1) < end_area(-1) else -1.0
# Turn the barrel onto Blender +Y (glTF -Z) about Z.
dirv = Vector((sign, 0, 0)) if axis == 0 else Vector((0, sign, 0))
ang = math.atan2(dirv.x, dirv.y)  # angle from +Y to dirv
rot = Matrix.Rotation(ang, 4, "Z")
scale = 1.0
L0 = size[axis]
if LENGTH > 0:
    scale = LENGTH / L0
elif not 0.12 <= L0 <= 1.5:
    # Probably centimetres (or a 100x rig): pick the power of ten that gives a gun-sized length.
    for k in (0.01, 0.1, 10.0, 100.0, 0.001):
        if 0.12 <= L0 * k <= 1.5:
            scale = k
            break
    print(f"gun_prepare: length {L0:.3f} looks wrong; scaling by {scale}")
xf = Matrix.Scale(scale, 4) @ rot
for o in meshes:
    o.data.transform(xf)
pts = all_verts()
lo = Vector([min(p[i] for p in pts) for i in range(3)])
hi = Vector([max(p[i] for p in pts) for i in range(3)])
# Centre left-right; leave the rest (anchors are measured, not assumed).
shift = Matrix.Translation((-(lo.x + hi.x) * 0.5, 0, 0))
for o in meshes:
    o.data.transform(shift)
    o.data.update()

# Textures: cap the size.
for img in bpy.data.images:
    if img.size[0] > MAX_TEX or img.size[1] > MAX_TEX:
        k = MAX_TEX / max(img.size)
        img.scale(max(1, int(img.size[0] * k)), max(1, int(img.size[1] * k)))
        img.pack()

for o in meshes:
    o.select_set(True)
bpy.ops.export_scene.gltf(filepath=OUT, export_format="GLB", use_selection=True, export_animations=False, export_skins=False, export_materials="EXPORT", export_tangents=True, export_image_format="AUTO")

# --- anchor estimates, in glTF coordinates (x, y up, z = -Blender y; barrel toward -z) -------
g = [Vector((p.x, p.z, -p.y)) for p in all_verts()]
glo = Vector([min(p[i] for p in g) for i in range(3)])
ghi = Vector([max(p[i] for p in g) for i in range(3)])
length = ghi.z - glo.z
front = [p for p in g if p.z <= glo.z + 0.015]
# The bore is the barrel's centre, the top tube at the front (a pump's magazine tube or a
# launcher hangs below it): the 75th percentile of the front's heights, less a barrel radius.
ys = sorted(p.y for p in front)
bore_y = ys[int(0.75 * (len(ys) - 1))] - 0.006
muzzle = Vector((0.0, bore_y, glo.z))
# Lowest point per 1 cm slice along the gun: the magazine and the pistol grip hang below the bore.
bins = {}
for p in g:
    k = int((p.z - glo.z) / 0.01)
    bins[k] = min(bins.get(k, 1e9), p.y)
pistol = KIND in ("pistol", "heavypistol")
drop = {k: bore_y - y for k, y in bins.items()}
if pistol:
    # Grip: the rear half; the hand sits under the slide.
    grip_z = glo.z + 0.75 * length
    hand = Vector((0.0, bore_y - 0.05, grip_z))
    support = hand + Vector((-0.022, -0.015, -0.015))
else:
    # The rear-most slice that hangs well below the bore (> 7 cm) in the back 70 % (not the stock
    # toe: stocks are at the very back, their drop is measured from a higher line).
    cand = [k for k, d in drop.items() if d > 0.07 and 0.3 * length < k * 0.01 < 0.8 * length]
    grip_k = max(cand) if cand else int(0.62 * length / 0.01)
    grip_z = glo.z + grip_k * 0.01 - 0.02
    hand = Vector((0.0, bore_y - 0.045, grip_z))
    support = Vector((0.0, bore_y - 0.015, glo.z + 0.35 * length))
# Sight line: the top of whatever rides above the receiver near the hand, less the eye's
# offset into a sight (2 cm for an optic's axis, 0.5 cm for an iron post).
top = [p.y for p in g if hand.z - 0.25 < p.z < hand.z + 0.05]
sight = (max(top) if top else ghi.y) - (0.02 if (max(top) - bore_y) > 0.06 else 0.005)
centre = (glo + ghi) * 0.5
entry = {
    "kind": KIND,
    "high": "models/weapons/hd/" + OUT.replace("\\", "/").split("/")[-1],
    "low": "models/weapons/hd/" + OUT.replace("\\", "/").split("/")[-1].replace(".glb", ".lod1.glb"),
    "own_materials": True,
    "hand": [round(x, 4) for x in hand],
    "muzzle": [round(x, 4) for x in muzzle],
    "sight": round(sight, 4),
    "center": [round(x, 4) for x in centre],
    "support": [round(x, 4) for x in support],
}
tris = sum(sum(len(p.vertices) - 2 for p in o.data.polygons) for o in meshes)
print(f"gun_prepare: {len(meshes)} meshes, {tris} triangles, length {length:.3f} m, scale {scale:g}, muzzle {'+-'[sign < 0]}{'xy'[axis]}")
print("gun_prepare entry: " + json.dumps(entry))
