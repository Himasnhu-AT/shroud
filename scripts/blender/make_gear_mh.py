"""Enemy operator kit, skinned to the MetaHuman skeleton (headless Blender).

    Blender -b --factory-startup --python scripts/blender/make_gear_mh.py -- <enemy_mh.glb> <out.glb>

Reads the local MetaHuman enemy (rest pose) and writes a GLB of clothing and gear that deform
with the same bones. The game binds each mesh to the enemy's own joints by name
(`enemy/skin.rs`), so nothing here is animated or rendered from the MetaHuman file itself.

Clothing follows the body: a combat shirt, cargo trousers, boots and gloves are the body's own
surface regions pushed out by the cloth's thickness plus folds (rings that bunch at the knees,
elbows, waist and above the boots), so they keep the body's skin weights. Hard gear is modelled
here: a plate carrier (curved plates, cummerbund, shoulder straps, MOLLE rows, magazine and admin
pouches), a high-cut helmet (rails, NVG shroud, Velcro, chin strap), a balaclava, a belt, knee
pads and per-squad packs. Hard gear takes the weights of the nearest body (or head) vertex,
limited to the bones it should follow.

Each output mesh is `<part>.<material>` (one material each); the game picks the squad's
material by the suffix and the squad's pack by the part name. UVs: clothing keeps the body's
UVs; hard gear is box-projected at 1 UV unit per metre. Coordinates follow the import: +Y up,
+Z forward, +X the character's left. Output contains our geometry with weights derived from the
MetaHuman body, so it is generated locally next to it and never committed.
"""
import math
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector, noise
from mathutils.kdtree import KDTree

argv = sys.argv[sys.argv.index("--") + 1:]
SRC, OUT = argv[0], argv[1]

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=SRC, bone_heuristic="BLENDER")
ARM = next(o for o in bpy.data.objects if o.type == "ARMATURE")
ARM.animation_data_clear()
ARM.data.pose_position = "REST"
bpy.context.view_layer.update()
BODY = bpy.data.objects["body_lod1_mesh"]
HEADS = [o for o in bpy.data.objects if o.type == "MESH" and o.name.startswith("head_lod")]
MW = BODY.matrix_world.copy()
MWI = MW.inverted()
AW = ARM.matrix_world


def bone(name):
    return AW @ ARM.data.bones[name].head_local


def is_facial(n):
    return n.startswith("FACIAL")


# --- body data (world space) -------------------------------------------------------------
def mesh_data(obj):
    m = obj.matrix_world
    names = {g.index: g.name for g in obj.vertex_groups}
    pos = [m @ v.co for v in obj.data.vertices]
    nrm = [(m.to_3x3() @ v.normal).normalized() for v in obj.data.vertices]
    wts = [{names[g.group]: g.weight for g in v.groups if g.weight > 1e-4} for v in obj.data.vertices]
    return pos, nrm, wts


BPOS, BNRM, BWTS = mesh_data(BODY)
BDOM = [max(w, key=w.get) if w else "" for w in BWTS]
BTREE = KDTree(len(BPOS))
for i, p in enumerate(BPOS):
    BTREE.insert(p, i)
BTREE.balance()
# Each body vertex's representative among coincident duplicates (UV seams).
WELD = list(range(len(BPOS)))
for _i, _p in enumerate(BPOS):
    if WELD[_i] == _i:
        for _co, _j, _d in BTREE.find_range(_p, 1e-5):
            if _j > _i:
                WELD[_j] = _i

HPOS, HNRM, HWTS = [], [], []
for h in HEADS:
    p, n, w = mesh_data(h)
    HPOS += p
    HNRM += n
    HWTS += w
HTREE = KDTree(len(HPOS))
for i, p in enumerate(HPOS):
    HTREE.insert(p, i)
HTREE.balance()

PELVIS, SPINE5, NECK, HEAD = bone("pelvis"), bone("spine_05"), bone("neck_01"), bone("head")
EYE_L = sum((bpy.data.objects["eyeLeft_lod1_mesh"].matrix_world @ v.co for v in bpy.data.objects["eyeLeft_lod1_mesh"].data.vertices), Vector()) / 386
EYE_R = sum((bpy.data.objects["eyeRight_lod1_mesh"].matrix_world @ v.co for v in bpy.data.objects["eyeRight_lod1_mesh"].data.vertices), Vector()) / 386
ANKLE_Y = bone("foot_l").y
KNEE = {s: bone(f"calf_{s}") for s in "lr"}
ELBOW = {s: bone(f"lowerarm_{s}") for s in "lr"}
WRIST = {s: bone(f"hand_{s}") for s in "lr"}


# --- output objects ------------------------------------------------------------------------
MATS = {}


def material(name):
    if name not in MATS:
        m = bpy.data.materials.new("Gear_" + name)
        MATS[name] = m
    return MATS[name]


OUTS = []


def emit(name, mat, verts, faces, weights, uvs=None, smooth=True):
    """A mesh object from world-space `verts`, per-vertex `weights` ({bone: w}), parented to the
    armature like the body (same object space, so its inverse binds match the body's)."""
    me = bpy.data.meshes.new(f"{name}.{mat}")
    me.from_pydata([MWI @ v for v in verts], [], faces)
    me.materials.append(material(mat))
    uvl = me.uv_layers.new(name="UVMap")
    if uvs is not None:
        for poly in me.polygons:
            for li in poly.loop_indices:
                uvl.data[li].uv = uvs[me.loops[li].vertex_index]
    else:
        box_uv(me, verts)
    for p in me.polygons:
        p.use_smooth = smooth
    ob = bpy.data.objects.new(f"{name}.{mat}", me)
    bpy.context.scene.collection.objects.link(ob)
    ob.parent = ARM
    ob.matrix_world = MW
    groups = {}
    for i, w in enumerate(weights):
        for b, x in w.items():
            g = groups.get(b) or ob.vertex_groups.new(name=b)
            groups[b] = g
            g.add([i], x, "REPLACE")
    mod = ob.modifiers.new("Armature", "ARMATURE")
    mod.object = ARM
    OUTS.append(ob)
    return ob


def box_uv(me, world):
    """Box projection, 1 UV unit per metre, each face on the axis plane its normal faces most."""
    uvl = me.uv_layers[0]
    for poly in me.polygons:
        n = MW.to_3x3() @ poly.normal
        a = Vector((abs(n.x), abs(n.y), abs(n.z)))
        for li in poly.loop_indices:
            p = world[me.loops[li].vertex_index]
            if a.x >= a.y and a.x >= a.z:
                uv = (p.z, p.y)
            elif a.y >= a.z:
                uv = (p.x, p.z)
            else:
                uv = (p.x, p.y)
            uvl.data[li].uv = uv


def norm_weights(w, allow=None, remap=None):
    out = {}
    for b, x in w.items():
        if remap:
            b = remap(b)
        if allow and not allow(b):
            continue
        out[b] = out.get(b, 0.0) + x
    s = sum(out.values())
    if s <= 0:
        return None
    # glTF keeps four influences: the strongest four, renormalised.
    top = sorted(out.items(), key=lambda kv: -kv[1])[:4]
    s = sum(x for _, x in top)
    return {b: x / s for b, x in top}


def body_weights(p, allow, k=3):
    """Weights at world point `p` from the nearest body vertices (inverse-distance blend)."""
    acc = {}
    for co, i, d in BTREE.find_n(p, k):
        f = 1.0 / max(d, 1e-3)
        for b, x in BWTS[i].items():
            acc[b] = acc.get(b, 0.0) + x * f
    w = norm_weights(acc, allow)
    if w is None:
        # Nothing allowed nearby: the strongest allowed bone of a wider search.
        for co, i, d in BTREE.find_n(p, 64):
            w = norm_weights(BWTS[i], allow)
            if w:
                break
    return w or {"spine_05": 1.0}


def head_weights(p):
    acc = {}
    for co, i, d in HTREE.find_n(p, 3):
        f = 1.0 / max(d, 1e-3)
        for b, x in HWTS[i].items():
            acc[b] = acc.get(b, 0.0) + x * f
    # Facial joints hang off the head and never animate here: fold them into it.
    return norm_weights(acc, None, lambda b: "head" if is_facial(b) else b) or {"head": 1.0}


# --- clothing from body regions --------------------------------------------------------------
FINGERS = ("thumb", "index", "middle", "ring", "pinky")


def region(dom, p):
    """Which garment a body vertex belongs to (None: bare / covered by other gear)."""
    if dom.startswith(("hand_", "wrist_")) or dom.startswith(FINGERS):
        return "gloves"
    side = "l" if p.x > 0 else "r"
    if dom.startswith("lowerarm") and (p - WRIST[side]).length < 0.075:
        return "gloves"
    if dom.startswith(("foot_", "ball_", "ankle_")) or "toe_" in dom:
        return "feet"
    if dom.startswith(("calf", "thigh", "pelvis")) or dom == "root":
        # The trouser legs reach down inside the boot shafts (they overlap by 5 cm), so the
        # hem never shows a gap.
        if p.y < ANKLE_Y + 0.10:
            return "boots"
        if p.y < ANKLE_Y + 0.16:
            return ("boots", "trousers")
        return "trousers"
    if dom.startswith(("spine", "clavicle", "upperarm", "lowerarm", "neck")):
        return "shirt"
    return None


def ring_fold(p, axis_a, axis_b, wave, amp, seed):
    """Compression folds: rings round a limb from `axis_a` to `axis_b`, wobbling round it."""
    ax = axis_b - axis_a
    L = ax.length
    t = (p - axis_a).dot(ax) / (L * L)
    radial = (p - axis_a - ax * t)
    ang = math.atan2(radial.x + radial.y * 0.3, radial.z)
    ph = t * L / wave * math.tau + 1.7 * noise.noise(Vector((ang * 0.8, t * 3.0, seed)))
    return amp * (0.5 + 0.5 * math.sin(ph)) * (0.6 + 0.4 * noise.noise(Vector((ang * 2.0, t * 6.0, seed + 3.0))))


def bump_near(p, c, r):
    d = (p - c).length
    return max(0.0, 1.0 - d / r) ** 2


def garment_offset(kind, i):
    """(thickness, folds) in metres for body vertex i in garment `kind`: the thickness builds the
    smoothed cloth surface, the folds are added on top along its normals."""
    p, dom = BPOS[i], BDOM[i]
    side = "l" if p.x > 0 else "r"
    drape = 0.002 * noise.noise(p * 9.0)
    if kind == "gloves":
        cuff = bump_near(p, WRIST[side] + (bone(f"lowerarm_{side}") - WRIST[side]).normalized() * 0.04, 0.05)
        return 0.0022 + 0.005 * cuff, 0.0
    if kind == "boots":
        # Leather over the foot, a padded collar at the top of the shaft, room in the toe box.
        top = max(0.0, 1.0 - abs(p.y - (ANKLE_Y + 0.15)) / 0.03)
        toe = 0.006 if dom.startswith("ball_") or "toe_" in dom else 0.0
        return 0.01 + 0.004 * top + toe, 0.0
    if kind == "trousers":
        thigh, calf = bone(f"thigh_{side}"), KNEE[side]
        foot = bone(f"foot_{side}")
        if p.y > calf.y + 0.02 and (dom.startswith("thigh") or dom.startswith("pelvis")):
            f = ring_fold(p, thigh, calf, 0.08, 0.007, 1.0 if side == "l" else 2.0)
        else:
            f = ring_fold(p, calf, foot, 0.05, 0.008, 3.0 if side == "l" else 4.0)
        knee = bump_near(p, KNEE[side] + Vector((0, 0.02, 0.0)), 0.12)
        blouse = max(0.0, 1.0 - (p.y - (ANKLE_Y + 0.15)) / 0.12)  # bunching over the boot tops
        seat = 0.008 if p.z < PELVIS.z - 0.04 and p.y > PELVIS.y - 0.15 else 0.0
        crotch = 0.012 * bump_near(p, PELVIS + Vector((0, -0.1, 0.04)), 0.12)
        waist = 0.004 * max(0.0, 1.0 - abs(p.y - (PELVIS.y + 0.06)) / 0.05)
        base = 0.017 + 0.006 * knee + 0.012 * blouse + seat + waist + crotch
        # Taper to sit inside the boot collar.
        if p.y < ANKLE_Y + 0.19:
            base = min(base, 0.004 + 0.003 * max(0.0, (p.y - ANKLE_Y - 0.1) / 0.09))
            f = 0.0
        return base, f * (1.0 + blouse) + drape
    # shirt
    if dom.startswith(("upperarm", "lowerarm")):
        ua, la, hd = bone(f"upperarm_{side}"), ELBOW[side], WRIST[side]
        if (p - ua).length < (p - la).length:
            f = ring_fold(p, ua, la, 0.06, 0.006, 5.0 if side == "l" else 6.0)
        else:
            f = ring_fold(p, la, hd, 0.045, 0.007, 7.0 if side == "l" else 8.0)
        elbow = bump_near(p, ELBOW[side], 0.08)
        return 0.012 + 0.006 * elbow, f + drape
    tuck = max(0.0, 1.0 - (p.y - PELVIS.y) / 0.08)
    return 0.011 - 0.003 * tuck, 0.004 * noise.noise(p * 6.0) * (1.0 - tuck) + drape * 2.0


BODY_BVH = None


def body_bvh():
    global BODY_BVH
    if BODY_BVH is None:
        from mathutils.bvhtree import BVHTree
        BODY_BVH = BVHTree.FromPolygons(BPOS, [list(p.vertices) for p in BODY.data.polygons])
    return BODY_BVH


def relax(pos, faces, iters, pinned):
    """Laplacian smoothing (pinned vertices stay): irons the body's anatomy out of the cloth."""
    nbr = [set() for _ in pos]
    for f in faces:
        for k, a in enumerate(f):
            b = f[(k + 1) % len(f)]
            nbr[a].add(b)
            nbr[b].add(a)
    pos = [p.copy() for p in pos]
    for _ in range(iters):
        new = []
        for i, p in enumerate(pos):
            if i in pinned or not nbr[i]:
                new.append(p)
                continue
            avg = sum((pos[j] for j in nbr[i]), Vector()) / len(nbr[i])
            new.append(p.lerp(avg, 0.5))
        pos = new
    return pos


def boundary(faces):
    """Vertices on open edges (used by one face only)."""
    edges = {}
    for f in faces:
        for k, a in enumerate(f):
            e = tuple(sorted((a, f[(k + 1) % len(f)])))
            edges[e] = edges.get(e, 0) + 1
    return {v for e, c in edges.items() if c == 1 for v in e}


def vertex_normals(pos, faces):
    n = [Vector() for _ in pos]
    for f in faces:
        a, b, c = pos[f[0]], pos[f[1]], pos[f[2]]
        fn = (b - a).cross(c - a)
        for i in f:
            n[i] += fn
    return [v.normalized() if v.length > 1e-12 else Vector((0, 1, 0)) for v in n]


def build_garments():
    regs = {}
    for i, (p, d) in enumerate(zip(BPOS, BDOM)):
        r = region(d, p)
        for k in ((r,) if isinstance(r, str) else r or ()):
            regs.setdefault(k, set()).add(i)
    polys = BODY.data.polygons
    uv = BODY.data.uv_layers[0].data
    vuv = [None] * len(BPOS)
    for poly in polys:
        for li in poly.loop_indices:
            vuv[BODY.data.loops[li].vertex_index] = tuple(uv[li].uv)
    bvh = body_bvh()
    # Smoothing passes and the least clearance over the body, per garment.
    passes = {"gloves": 0, "boots": 14, "trousers": 18, "shirt": 12}
    clear = {"gloves": 0.0015, "boots": 0.012, "trousers": 0.01, "shirt": 0.007}
    feet = regs.pop("feet", set())
    for kind, verts in regs.items():
        faces = [list(p.vertices) for p in polys if sum(v in verts for v in p.vertices) >= len(p.vertices) - 1]
        # The body's halves are split along UV seams (duplicate vertices): weld them so the
        # cloth relaxes as one piece instead of opening along the seam.
        faces = [[WELD[v] for v in f] for f in faces]
        faces = [f for f in faces if len(set(f)) == len(f)]
        used = sorted({v for f in faces for v in f})
        idx = {v: k for k, v in enumerate(used)}
        lf = [[idx[v] for v in f] for f in faces]
        offs = [garment_offset(kind, v) for v in used]
        pos = [BPOS[v] + BNRM[v] * o[0] for v, o in zip(used, offs)]
        # Boundary vertices (cuffs, hems) stay put so the seams between garments keep meeting.
        pos = relax(pos, lf, passes[kind], boundary(lf))
        # Never inside the body: push out to the clearance along the nearest surface normal.
        for k, q in enumerate(pos):
            hit, n, fi, d = bvh.find_nearest(q)
            if hit is None:
                continue
            h = (q - hit).dot(n)
            c = min(clear[kind], offs[k][0])
            if h < c:
                pos[k] = q + n * (c - h)
        nrm = vertex_normals(pos, lf)
        pos = [q + n * o[1] for q, n, o in zip(pos, nrm, offs)]
        w = [norm_weights(BWTS[v]) or {"pelvis": 1.0} for v in used]
        mat = {"gloves": "leather", "boots": "leather", "trousers": "trousers", "shirt": "shirt"}[kind]
        if kind == "boots":
            boot_shafts(pos, lf)
            add_boot_feet(feet, pos)
            continue
        ob = emit(kind, mat, pos, lf, w, [vuv[v] for v in used])
        if kind == "gloves":
            # The hands are the body's densest part; a glove needs less.
            decimate(ob, 0.4)
        if kind == "trousers":
            add_cargo_pockets()


def boot_shafts(pos, faces):
    """Boot shafts cut level at the collar (a clean top edge, not the body's triangle zig-zag),
    with a padded collar roll."""
    top = ANKLE_Y + 0.15
    bm = bmesh.new()
    vs = [bm.verts.new(q) for q in pos]
    for f in faces:
        try:
            bm.faces.new([vs[i] for i in f])
        except ValueError:
            pass
    bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces), plane_co=(0, top, 0), plane_no=(0, 1, 0), clear_outer=True)
    part = Part()
    part.add(bm)
    for s in "lr":
        k, f = KNEE[s], bone(f"foot_{s}")
        axis = (k - f).normalized()
        t = (top - 0.008 - f.y) / max(axis.y, 1e-3)
        limb_ring(part, f + axis * t, axis, 0.012, 0.018, 0.005)
    part.finish("boots", "boot", lambda p: body_weights(p, lambda b: b.startswith(("calf", "foot", "ankle")) and "knee" not in b, k=4), smooth=True)


def add_cargo_pockets():
    """Bellowed cargo pockets on the outer thighs, with flaps."""
    bvh = body_bvh()
    part = Part()
    for s in "lr":
        sx = 1.0 if s == "l" else -1.0
        hip, knee = bone(f"thigh_{s}"), KNEE[s]
        c = hip.lerp(knee, 0.52)
        # Outer surface of the thigh at that height (ray from outside toward the bone).
        # From the bone outward (from outside, the hanging arm would be hit first).
        hit, n, fi, d = bvh.ray_cast(c, Vector((sx, 0.0, 0.0)), 0.3)
        if hit is None:
            continue
        axis = (hip - knee).normalized()
        out = Vector((sx, 0.0, 0.0))
        out = (out - axis * out.dot(axis)).normalized()
        fwd = axis.cross(out).normalized() * sx
        m = Matrix((fwd * sx, axis, out)).transposed()
        at = hit + out * (0.017 + 0.019)
        part.box(at, (0.15, 0.18, 0.038), rot=m, bevel=0.012, segs=2)
        part.box(at + axis * 0.085 + out * 0.008, (0.155, 0.045, 0.03), rot=m, bevel=0.008, segs=2)
    part.finish("trousers", "trousers_pocket", lambda p: body_weights(p, lambda b: b.startswith("thigh") and "twist" not in b and "Cor" not in b and "_lwr" not in b, k=4), smooth=False)


# --- hard-gear construction helpers (bmesh, world space) ---------------------------------
class Part:
    def __init__(self):
        self.bm = bmesh.new()

    def box(self, center, size, rot=Matrix.Identity(3), bevel=0.0, segs=2):
        tmp = bmesh.new()
        bmesh.ops.create_cube(tmp, size=1.0)
        bmesh.ops.scale(tmp, vec=Vector(size), verts=tmp.verts)
        if bevel > 0:
            bmesh.ops.bevel(tmp, geom=list(tmp.edges), offset=bevel, segments=segs, profile=0.5, affect="EDGES")
        bmesh.ops.transform(tmp, matrix=rot.to_4x4(), verts=tmp.verts)
        bmesh.ops.translate(tmp, vec=Vector(center), verts=tmp.verts)
        self.add(tmp)

    def cyl(self, center, r, h, rot=Matrix.Identity(3), segs=12):
        tmp = bmesh.new()
        bmesh.ops.create_cone(tmp, cap_ends=True, segments=segs, radius1=r, radius2=r, depth=h)
        # Cone axis is Z: stand it on Y, then rotate.
        bmesh.ops.rotate(tmp, verts=tmp.verts, matrix=Matrix.Rotation(-math.pi / 2, 3, "X"))
        bmesh.ops.transform(tmp, matrix=rot.to_4x4(), verts=tmp.verts)
        bmesh.ops.translate(tmp, vec=Vector(center), verts=tmp.verts)
        self.add(tmp)

    def add(self, other):
        me = bpy.data.meshes.new("tmp")
        other.to_mesh(me)
        other.free()
        self.bm.from_mesh(me)
        bpy.data.meshes.remove(me)

    def finish(self, name, mat, weights_fn, smooth=False):
        bm = self.bm
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        verts = [v.co.copy() for v in bm.verts]
        faces = [[v.index for v in f.verts] for f in bm.faces]
        bm.free()
        ob = emit(name, mat, verts, faces, [weights_fn(v) for v in verts], smooth=True)
        # Hard edges stay sharp, bevels and curves shade smooth.
        if not smooth:
            ob.data.set_sharp_from_angle(angle=math.radians(35))
        return ob


def torso_front_z(y, x=0.0):
    """Body surface z at height y near x (front: max z, back: min z)."""
    best_f, best_b = -1.0, 1.0
    for co, i, d in BTREE.find_range(Vector((x, y, -0.07)), 0.2):
        if abs(co.y - y) < 0.02 and abs(co.x - x) < 0.035 and BDOM[i].startswith(("spine", "clavicle", "pelvis")):
            best_f = max(best_f, co.z)
            best_b = min(best_b, co.z)
    return best_f, best_b


def torso_half_width(y):
    w = 0.0
    for co, i, d in BTREE.find_range(Vector((0, y, -0.07)), 0.3):
        if abs(co.y - y) < 0.02 and BDOM[i].startswith(("spine", "pelvis")):
            w = max(w, abs(co.x))
    return w


TORSO = ("spine", "clavicle", "pelvis")


def torso_w(p):
    return body_weights(p, lambda b: b.startswith(("spine", "clavicle")) and "latissimus" not in b and "pec" not in b and "scap" not in b, k=4)


PLATES = {}


def build_carrier():
    """Plate carrier: curved front and back plates, cummerbund, shoulder straps, MOLLE rows on
    the plates and cummerbund, three rifle-mag pouches and an admin pouch; plus a belt."""
    cy = SPINE5.y - 0.1  # plate centre height (about the sternum's middle)
    zf, _ = torso_front_z(cy + 0.04)
    _, zb = torso_front_z(cy + 0.02)
    gear = Part()
    web = Part()
    hw = 0.135
    # Plates: a grid bent round the chest (curve in x, slight lean back at the top).
    def plate(front):
        tmp = bmesh.new()
        nx, ny = 10, 8
        w, h, t = 2 * hw, 0.30, 0.03
        z0 = zf + 0.022 if front else zb - 0.024
        sgn = 1.0 if front else -1.0
        grid = {}
        for j in range(ny + 1):
            for i in range(nx + 1):
                u, v = i / nx - 0.5, j / ny - 0.5
                # Swimmer-cut corners: narrow the top.
                cut = 0.045 * max(0.0, (v - 0.2) / 0.3) * (1 if abs(u) > 0.25 else 0)
                x = u * w * (1.0 - (cut / hw if abs(u) > 0.25 else 0))
                y = cy + v * h
                z = z0 - sgn * (x * x) * 1.4 - sgn * 0.03 * max(0.0, v) ** 1.5 * (1 if front else -0.3)
                for k, dz in enumerate((0.0, -sgn * t)):
                    grid[(i, j, k)] = tmp.verts.new((x, y, z + dz))
        for j in range(ny):
            for i in range(nx):
                q = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)]
                tmp.faces.new([grid[(a, b, 0)] for a, b in q] if front else [grid[(a, b, 0)] for a, b in reversed(q)])
                tmp.faces.new([grid[(a, b, 1)] for a, b in reversed(q)] if front else [grid[(a, b, 1)] for a, b in q])
        # Rim.
        ring = [(i, 0) for i in range(nx)] + [(nx, j) for j in range(ny)] + [(i, ny) for i in range(nx, 0, -1)] + [(0, j) for j in range(ny, 0, -1)]
        for a, b in zip(ring, ring[1:] + ring[:1]):
            f = [grid[(a[0], a[1], 0)], grid[(b[0], b[1], 0)], grid[(b[0], b[1], 1)], grid[(a[0], a[1], 1)]]
            tmp.faces.new(f if front else list(reversed(f)))
        gear.add(tmp)
        return z0, sgn

    zf0, _ = plate(True)
    zb0, _ = plate(False)
    PLATES.update(cy=cy, front=zf0, back=zb0)
    print(f"carrier: plates at y {cy:.3f}, front z {zf0:.3f} (chest {zf:.3f}), back z {zb0:.3f} (back {zb:.3f})")
    # MOLLE: rows of webbing across the lower front plate (behind the pouches) and the back.
    for z0, sgn in ((zf0, 1.0), (zb0, -1.0)):
        for r in range(4):
            y = cy - 0.12 + r * 0.032
            for c in range(6):
                x = -0.11 + c * 0.044
                z = z0 - sgn * (x * x) * 1.4 + sgn * 0.003
                web.box((x, y, z), (0.036, 0.022, 0.004), bevel=0.0008, segs=1)
    # Cummerbund: a band round each side from the front plate edge to the back plate edge.
    for s in (1.0, -1.0):
        tmp = bmesh.new()
        n = 9
        rows = []
        for j, dy in enumerate((-0.12, 0.05)):
            row = []
            for i in range(n + 1):
                t = i / n
                ang = math.pi * t  # front (0) round the side to the back (pi)
                y = cy - 0.06 + dy
                half = torso_half_width(y) + 0.022
                zc = (zf0 + zb0) * 0.5
                rz = (zf0 - zb0) * 0.5
                p = Vector((s * (hw - 0.01 + (half - hw + 0.01) * math.sin(ang)), y, zc + rz * math.cos(ang) * 0.92))
                row.append(p)
            rows.append(row)
        out = [[tmp.verts.new(p) for p in row] for row in rows]
        inn = [[tmp.verts.new(p - Vector((s * 0.012 * math.sin(math.pi * i / n), 0, 0.012 * math.cos(math.pi * i / n)))) for i, p in enumerate(row)] for row in rows]
        for i in range(n):
            f = [out[0][i], out[0][i + 1], out[1][i + 1], out[1][i]]
            tmp.faces.new(f if s > 0 else list(reversed(f)))
            f = [inn[0][i], inn[1][i], inn[1][i + 1], inn[0][i + 1]]
            tmp.faces.new(f if s > 0 else list(reversed(f)))
        # Top and bottom edges.
        for j in (0, 1):
            for i in range(n):
                f = [out[j][i], inn[j][i], inn[j][i + 1], out[j][i + 1]]
                tmp.faces.new(f if (s > 0) == (j == 0) else list(reversed(f)))
        gear.add(tmp)
        # MOLLE on the cummerbund.
        for i in range(2, n - 1, 2):
            for dy in (-0.09, -0.055, -0.02):
                t = i / n
                ang = math.pi * t
                y = cy - 0.06 + dy
                half = torso_half_width(y) + 0.026
                p = Vector((s * (hw - 0.01 + (half - hw + 0.01) * math.sin(ang)), y, (zf0 + zb0) * 0.5 + (zf0 - zb0) * 0.5 * math.cos(ang) * 0.92 + 0.003 * math.cos(ang)))
                yaw = math.atan2(s * math.sin(ang), math.cos(ang))
                web.box(p, (0.03, 0.02, 0.004), rot=Matrix.Rotation(yaw, 3, "Y"))
    # Shoulder straps: front plate top over the trapezius to the back plate top.
    for s in (1.0, -1.0):
        x = s * 0.085
        y0 = cy + 0.15
        top = max((co.y for co, i, d in BTREE.find_range(Vector((x, y0 + 0.08, -0.08)), 0.12)), default=y0 + 0.1)
        pts = []
        n = 8
        for i in range(n + 1):
            t = i / n
            ang = math.pi * t
            z = (zf0 - 0.01) * math.cos(ang) * 0.5 + (zf0 - 0.01) * 0.5 if t < 0.5 else (zb0 + 0.01) * (-math.cos(ang)) * 0.5 + (zb0 + 0.01) * 0.5
            z = (zf0 - 0.008) + (zb0 + 0.008 - (zf0 - 0.008)) * (0.5 - 0.5 * math.cos(ang))
            y = y0 + (top + 0.018 - y0) * math.sin(ang) ** 0.6
            pts.append(Vector((x + s * 0.01 * math.sin(ang), y, z)))
        for a, b in zip(pts, pts[1:]):
            d = b - a
            mid = (a + b) * 0.5
            rot = Vector((0, 0, 1)).rotation_difference(d.normalized()).to_matrix()
            gear.box(mid, (0.06, 0.012, d.length + 0.004), rot=rot, bevel=0.002, segs=1)
    gear.finish("carrier", "cordura", torso_w)
    web.finish("carrier", "webbing", torso_w)
    # Pouches: three rifle magazine pouches low on the front plate, an admin pouch above.
    pouch = Part()
    flap = Part()
    for i, x in enumerate((-0.085, 0.0, 0.085)):
        y = cy - 0.07
        z = zf0 - 1.4 * x * x + 0.032
        pouch.box((x, y, z), (0.072, 0.13, 0.04), bevel=0.008, segs=2)
        flap.box((x, y + 0.058, z + 0.006), (0.076, 0.03, 0.046), bevel=0.006, segs=2)
        # Bungee pull tab.
        flap.box((x, y + 0.045, z + 0.03), (0.016, 0.03, 0.004), bevel=0.0015, segs=1)
    pouch.box((0.0, cy + 0.085, zf0 + 0.022), (0.17, 0.08, 0.025), bevel=0.007, segs=2)
    flap.box((0.0, cy + 0.11, zf0 + 0.036), (0.15, 0.022, 0.006), bevel=0.002, segs=1)  # name tape Velcro
    # Hydration/utility pouch on the back.
    pouch.box((0.0, cy - 0.02, zb0 - 0.04), (0.17, 0.24, 0.05), bevel=0.012, segs=2)
    pouch.finish("pouches", "cordura", torso_w)
    flap.finish("pouches", "webbing", torso_w)
    # Belt with a buckle and two side pouches.
    belt = Part()
    metal = Part()
    by = PELVIS.y + 0.07
    n = 20
    ring_o, ring_i = [], []
    tmp = bmesh.new()
    for j, dy in enumerate((-0.022, 0.022)):
        ro, ri = [], []
        for i in range(n):
            a = math.tau * i / n
            dirv = Vector((math.sin(a), 0, math.cos(a)))
            # Body radius along this direction at belt height.
            r = 0.0
            for co, k, d in BTREE.find_range(Vector((0, by + dy, PELVIS.z)), 0.3):
                if abs(co.y - (by + dy)) < 0.015 and BDOM[k].startswith(("pelvis", "spine", "thigh")):
                    v = Vector((co.x, 0, co.z - PELVIS.z))
                    if v.length > 1e-4 and v.normalized().dot(dirv) > 0.97:
                        r = max(r, v.length)
            r = r or 0.15
            c = Vector((0, by + dy, PELVIS.z))
            ro.append(tmp.verts.new(c + dirv * (r + 0.022)))
            ri.append(tmp.verts.new(c + dirv * (r + 0.01)))
        ring_o.append(ro)
        ring_i.append(ri)
    for i in range(n):
        k = (i + 1) % n
        tmp.faces.new([ring_o[0][i], ring_o[0][k], ring_o[1][k], ring_o[1][i]])
        tmp.faces.new([ring_i[0][i], ring_i[1][i], ring_i[1][k], ring_i[0][k]])
        tmp.faces.new([ring_o[1][i], ring_o[1][k], ring_i[1][k], ring_i[1][i]])
        tmp.faces.new([ring_o[0][i], ring_i[0][i], ring_i[0][k], ring_o[0][k]])
    belt.add(tmp)
    bz = max(p.z for p in BPOS if abs(p.y - by) < 0.02 and abs(p.x) < 0.03) + 0.024
    metal.box((0.0, by, bz), (0.05, 0.036, 0.008), bevel=0.003, segs=1)
    for s in (1.0, -1.0):
        hx = torso_half_width(by) + 0.02
        belt.box((s * (hx - 0.02), by - 0.04, PELVIS.z - 0.02), (0.035, 0.09, 0.07), rot=Matrix.Rotation(s * 0.5, 3, "Y"), bevel=0.008, segs=2)
    hip_w = lambda p: body_weights(p, lambda b: b in ("pelvis", "spine_01", "spine_02"), k=4)
    belt.finish("belt", "webbing", hip_w)
    metal.finish("belt", "metal", hip_w)


def build_packs():
    """Per-squad back kit, against the back plate's outer face (`PLATES`)."""
    cy, zb0 = PLATES["cy"], PLATES["back"]
    bp = Part()
    bp.box((0.0, cy - 0.04, zb0 - 0.003 - 0.07), (0.27, 0.36, 0.14), bevel=0.03, segs=3)
    bp.box((0.0, cy - 0.1, zb0 - 0.003 - 0.155), (0.21, 0.17, 0.04), bevel=0.012, segs=2)  # front pocket
    bp.box((0.0, cy + 0.15, zb0 - 0.003 - 0.065), (0.2, 0.03, 0.12), bevel=0.01, segs=2)  # lid
    bp.finish("pack_backpack", "cordura", torso_w)
    radio = Part()
    radio.box((-0.075, cy - 0.03, zb0 - 0.004 - 0.035), (0.085, 0.2, 0.065), bevel=0.01, segs=2)
    radio.cyl((-0.1, cy + 0.22, zb0 - 0.04), 0.005, 0.3, segs=6)
    radio.finish("pack_radio", "webbing", torso_w)
    none = Part()
    none.box((0.0, cy - 0.11, zb0 - 0.004 - 0.03), (0.2, 0.12, 0.06), bevel=0.012, segs=2)
    none.finish("pack_none", "cordura", torso_w)


def head_fit():
    """Centre and radii of the cranium (head vertices above the eyes)."""
    top = [p for p in HPOS if p.y > EYE_L.y + 0.01]
    lo = Vector((min(p.x for p in top), min(p.y for p in top), min(p.z for p in top)))
    hi = Vector((max(p.x for p in top), max(p.y for p in top), max(p.z for p in top)))
    c = (lo + hi) * 0.5
    return Vector((c.x, EYE_L.y + 0.02, c.z)), Vector(((hi.x - lo.x) * 0.5, hi.y - (EYE_L.y + 0.02), (hi.z - lo.z) * 0.5))


def build_helmet():
    """High-cut ballistic helmet: a shell shrunk round the cranium, cut high over the ears, with
    a thickness rim, side rails, an NVG shroud, Velcro patches and a chin strap."""
    c, r = head_fit()
    r = Vector((r.x + 0.022, r.y + 0.024, r.z + 0.022))
    shell = Part()
    tmp = bmesh.new()
    nu, nv = 36, 14
    rows = []
    fwd = Vector((0, 0, 1))

    def cut_y(az):
        # az: 0 front, +-pi/2 sides, pi back. Brow at -0.0, ears cut high, nape low.
        front = math.cos(az)
        side = abs(math.sin(az))
        return 0.035 * max(0.0, front) ** 2 + 0.065 * side ** 3 - 0.03 * max(0.0, -front) - 0.005

    for j in range(nv + 1):
        row = []
        for i in range(nu):
            az = math.tau * i / nu
            yc = cut_y(az)
            # Latitude from the cut line up to the crown.
            t = j / nv
            lat = (math.asin(max(-1, min(1, yc / r.y))) + (math.pi / 2 - math.asin(max(-1, min(1, yc / r.y)))) * t)
            p = Vector((r.x * math.cos(lat) * math.sin(az), r.y * math.sin(lat), r.z * math.cos(lat) * math.cos(az)))
            row.append(p + c)
        rows.append(row)
    outer = [[tmp.verts.new(p) for p in row] for row in rows]
    inner = [[tmp.verts.new(c + (p - c) * 0.93) for p in row] for row in rows]
    for j in range(nv):
        for i in range(nu):
            k = (i + 1) % nu
            tmp.faces.new([outer[j][i], outer[j][k], outer[j + 1][k], outer[j + 1][i]])
            tmp.faces.new([inner[j][i], inner[j + 1][i], inner[j + 1][k], inner[j][k]])
    for i in range(nu):
        k = (i + 1) % nu
        tmp.faces.new([outer[0][i], inner[0][i], inner[0][k], outer[0][k]])
    bmesh.ops.remove_doubles(tmp, verts=tmp.verts, dist=1e-5)
    shell.add(tmp)
    shell.finish("helmet", "helmet", lambda p: {"head": 1.0}, smooth=True)
    def on_shell(az, lat, lift=0.0):
        """Point on the shell (az 0 = front, lat 0 = the centre's height) and its frame: the
        columns are (east tangent, north tangent, outward normal)."""
        ca, sa, cl, sl = math.cos(az), math.sin(az), math.cos(lat), math.sin(lat)
        p = c + Vector((r.x * cl * sa, r.y * sl, r.z * cl * ca))
        n = Vector((cl * sa / r.x, sl / r.y, cl * ca / r.z)).normalized()
        east = Vector((ca, 0.0, -sa)).normalized()
        north = n.cross(east).normalized()
        east = north.cross(n).normalized()
        return p + n * lift, Matrix((east, north, n)).transposed()

    hw = Part()
    # Side rails (ARC style) along the shell above the ear cut, and the shroud on the brow.
    for s in (1.0, -1.0):
        for k in range(5):
            p, m = on_shell(s * (math.pi / 2) + (k - 2) * 0.17, 0.5, 0.006)
            hw.box(p, (0.042, 0.02, 0.012), rot=m, bevel=0.002, segs=1)
    p, m = on_shell(0.0, 0.38, 0.008)
    hw.box(p, (0.06, 0.035, 0.016), rot=m, bevel=0.004, segs=1)
    p, m = on_shell(0.0, 0.3, 0.018)
    hw.box(p, (0.022, 0.03, 0.02), rot=m, bevel=0.003, segs=1)  # mount arm
    # Counterweight / battery pouch at the back.
    p, m = on_shell(math.pi, 0.45, 0.016)
    hw.box(p, (0.085, 0.05, 0.03), rot=m, bevel=0.008, segs=2)
    hw.finish("helmet", "webbing", lambda p: {"head": 1.0})
    # Velcro loop fields on the crown and the sides (the cover's fabric).
    vel = Part()
    for az, lat, size in ((0.0, 1.0, (0.06, 0.075, 0.005)), (math.pi / 2, 0.85, (0.06, 0.05, 0.005)), (-math.pi / 2, 0.85, (0.06, 0.05, 0.005)), (math.pi, 0.95, (0.08, 0.06, 0.005))):
        p, m = on_shell(az, lat, 0.002)
        vel.box(p, size, rot=m, bevel=0.0015, segs=1)
    vel.finish("helmet", "cordura", lambda p: {"head": 1.0})
    # Chin strap: from the shell's side rim, down the cheek over the balaclava, under the jaw.
    strap = Part()
    chin_y = EYE_L.y - 0.12
    for s in (1.0, -1.0):
        rim, _ = on_shell(s * math.pi / 2, math.asin(max(-1.0, min(1.0, cut_y(s * math.pi / 2) / r.y))), -0.004)
        ctrl = [rim, Vector((s * 0.07, EYE_L.y - 0.05, EYE_L.z - 0.06)), Vector((s * 0.045, chin_y, EYE_L.z - 0.03)), Vector((0.0, chin_y - 0.012, EYE_L.z - 0.035))]
        pts = []
        for k in range(10):
            t = k / 9
            # Catmull-Rom-ish: piecewise linear through the controls, then onto the surface.
            seg = min(int(t * 3), 2)
            u = t * 3 - seg
            q = ctrl[seg].lerp(ctrl[seg + 1], u)
            pts.append(surface(q, 0.008) if k > 0 else q)
        ribbon(strap, pts, 0.016, 0.003)
    strap.finish("helmet", "webbing", lambda p: head_weights(p) if p.y < EYE_L.y - 0.02 else {"head": 1.0}, smooth=True)


SURF = None


def surface_bvh():
    """BVH of the head and the body's neck/shoulders (world space) for ray projection."""
    global SURF
    if SURF is None:
        verts, polys = [], []
        for ob in HEADS + [BODY]:
            m = ob.matrix_world
            base = len(verts)
            verts += [m @ v.co for v in ob.data.vertices]
            for poly in ob.data.polygons:
                if ob is BODY and (m @ ob.data.vertices[poly.vertices[0]].co).y < NECK.y - 0.2:
                    continue
                polys.append([base + i for i in poly.vertices])
        from mathutils.bvhtree import BVHTree
        SURF = BVHTree.FromPolygons(verts, polys)
    return SURF


def surface(p, lift, axis_z=None):
    """`p` moved onto the outermost head/neck surface along the ray toward the head's vertical
    axis (from outside), then `lift` out along the surface normal."""
    bvh = surface_bvh()
    zc = HEAD.z + 0.02 if axis_z is None else axis_z
    tgt = Vector((0.0, p.y, zc))
    d = (tgt - p)
    if d.length < 1e-4:
        return p
    d.normalize()
    hit, n, i, dist = bvh.ray_cast(p - d * 0.3, d, 0.6)
    if hit is None:
        return p
    if n.dot(d) > 0:
        n = -n
    return hit + n * lift


def ribbon(part, pts, width, thick):
    """A strap of `width` x `thick` following `pts` (its flat side on the surface below)."""
    tmp = bmesh.new()
    rows = []
    for k, q in enumerate(pts):
        a = pts[max(k - 1, 0)]
        b = pts[min(k + 1, len(pts) - 1)]
        t = (b - a).normalized()
        # Outward: away from the head's axis.
        out = (q - Vector((0.0, q.y, HEAD.z + 0.02))).normalized()
        side = t.cross(out).normalized()
        out = side.cross(t).normalized()
        rows.append([tmp.verts.new(q + side * sx * width * 0.5 + out * ox * thick) for sx, ox in ((-1, 0), (1, 0), (1, 1), (-1, 1))])
    for k in range(len(rows) - 1):
        for e in range(4):
            f = (e + 1) % 4
            tmp.faces.new([rows[k][e], rows[k][f], rows[k + 1][f], rows[k + 1][e]])
    tmp.faces.new(list(reversed(rows[0])))
    tmp.faces.new(rows[-1])
    part.add(tmp)


def build_balaclava():
    """A knit balaclava: the head's own surface (it carries the face's shape and the neck down
    to the collar) lifted by the knit's thickness and relaxed so the lips and nostrils soften,
    with one opening across the eyes. The head's parts are welded first so the knit has no
    seams."""
    eye_c = (EYE_L + EYE_R) * 0.5
    raw_p, raw_n, raw_w, raw_uv, raw_f = [], [], [], [], []
    for h in HEADS:
        m = h.matrix_world
        names = {g.index: g.name for g in h.vertex_groups}
        base = len(raw_p)
        uv = h.data.uv_layers[0].data
        vuv = {}
        for poly in h.data.polygons:
            for li in poly.loop_indices:
                vuv[h.data.loops[li].vertex_index] = tuple(uv[li].uv)
        for v in h.data.vertices:
            raw_p.append(m @ v.co)
            raw_n.append((m.to_3x3() @ v.normal).normalized())
            w = {names[g.group]: g.weight for g in v.groups if g.weight > 1e-4}
            raw_w.append(norm_weights(w, None, lambda b: "head" if is_facial(b) else b) or {"head": 1.0})
            raw_uv.append(vuv.get(v.index, (0.0, 0.0)))
        raw_f += [[base + i for i in poly.vertices] for poly in h.data.polygons]
    # Weld coincident vertices (the parts' shared borders); average their normals.
    tree = KDTree(len(raw_p))
    for i, q in enumerate(raw_p):
        tree.insert(q, i)
    tree.balance()
    rep = list(range(len(raw_p)))
    for i, q in enumerate(raw_p):
        if rep[i] != i:
            continue
        for co, j, d in tree.find_range(q, 2e-4):
            if j > i and rep[j] == j:
                rep[j] = i
    nsum = {}
    for i, r in enumerate(rep):
        nsum[r] = nsum.get(r, Vector()) + raw_n[i]
    # Keep: welded representatives outside the eye opening.
    def opening(q):
        return q.z > eye_c.z - 0.05 and abs(q.y - (eye_c.y + 0.002)) < 0.02 and abs(q.x) < 0.066
    keep = {}
    verts, wts, uvs, nrm = [], [], [], []
    for i, r in enumerate(rep):
        if r != i or opening(raw_p[i]):
            continue
        keep[i] = len(verts)
        verts.append(raw_p[i])
        nrm.append(nsum[i].normalized())
        wts.append(raw_w[i])
        uvs.append(raw_uv[i])
    faces = []
    for f in raw_f:
        g = [rep[i] for i in f]
        if all(i in keep for i in g) and len(set(g)) == len(g):
            faces.append([keep[i] for i in g])
    # The largest connected piece is the outer shell (drops the mouth's and nostrils' insides).
    parent = list(range(len(verts)))
    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a
    for f in faces:
        for a in f[1:]:
            ra, rb = find(f[0]), find(a)
            if ra != rb:
                parent[ra] = rb
    sizes = {}
    for f in faces:
        r = find(f[0])
        sizes[r] = sizes.get(r, 0) + 1
    main = max(sizes, key=sizes.get)
    faces = [f for f in faces if find(f[0]) == main]
    # Looser under the jaw: the throat folds forward when the head pitches down to aim.
    throat = lambda q: max(0.0, min(1.0, (EYE_L.y - 0.09 - q.y) / 0.06))
    verts = [q + n * (0.0045 + 0.008 * throat(q)) for q, n in zip(verts, nrm)]
    verts = relax(verts, faces, 3, boundary(faces))
    # Neck gaiter: the bottom edge runs on down 5 cm and out a little, under the shirt's collar
    # and the carrier, so no skin shows at the throat however the head turns.
    edge_use = {}
    for f in faces:
        for k, a in enumerate(f):
            b = f[(k + 1) % len(f)]
            edge_use.setdefault(tuple(sorted((a, b))), []).append((a, b))
    bottom = [ab[0] for e, ab in edge_use.items() if len(ab) == 1 and max(verts[e[0]].y, verts[e[1]].y) < NECK.y + 0.02]
    ext = {}
    for a, b in bottom:
        for v in (a, b):
            if v not in ext:
                q = verts[v]
                out = Vector((q.x, 0.0, q.z - NECK.z)).normalized() if abs(q.x) + abs(q.z - NECK.z) > 1e-4 else Vector()
                ext[v] = len(verts)
                verts.append(q + Vector((0.0, -0.065, 0.0)) + out * 0.01)
                wts.append(wts[v])
                uvs.append(uvs[v])
        faces.append([b, a, ext[a], ext[b]])
    ob = emit("balaclava", "balaclava", verts, faces, wts, uvs)
    decimate(ob, 0.3)


def decimate(ob, ratio):
    """Collapse-decimate a mesh (before skinning) to `ratio` of its triangles."""
    m = ob.modifiers.new("Dec", "DECIMATE")
    m.ratio = ratio
    m.use_collapse_triangulate = True
    with bpy.context.temp_override(object=ob, active_object=ob, selected_objects=[ob]):
        bpy.ops.object.modifier_move_to_index(modifier="Dec", index=0)
        bpy.ops.object.modifier_apply(modifier="Dec")


def body_or_head(p):
    co, i, dh = HTREE.find(p)
    co, j, db = BTREE.find(p)
    if dh <= db:
        return head_weights(p)
    return norm_weights(BWTS[j]) or {"neck_01": 1.0}


def limb_ring(part, centre, axis, lift, width, thick, segs=18):
    """A strap round a limb: rays from outside toward the limb's axis at `centre`, lifted."""
    bvh = body_bvh()
    axis = axis.normalized()
    u = axis.orthogonal().normalized()
    v = axis.cross(u).normalized()
    rings = []
    for dy in (-width * 0.5, width * 0.5):
        row_o, row_i = [], []
        for k in range(segs):
            a = math.tau * k / segs
            d = u * math.cos(a) + v * math.sin(a)
            c = centre + axis * dy
            # From the bone outward: the first surface is this limb's own.
            hit, n, fi, dist = bvh.ray_cast(c, d, 0.2)
            r = (hit - c).dot(d) if hit is not None else 0.06
            row_o.append(c + d * (r + lift + thick))
            row_i.append(c + d * (r + lift))
        rings.append((row_o, row_i))
    tmp = bmesh.new()
    O = [[tmp.verts.new(p) for p in rings[j][0]] for j in (0, 1)]
    I = [[tmp.verts.new(p) for p in rings[j][1]] for j in (0, 1)]
    for k in range(segs):
        m = (k + 1) % segs
        tmp.faces.new([O[0][k], O[0][m], O[1][m], O[1][k]])
        tmp.faces.new([I[0][k], I[1][k], I[1][m], I[0][m]])
        tmp.faces.new([O[1][k], O[1][m], I[1][m], I[1][k]])
        tmp.faces.new([O[0][k], I[0][k], I[0][m], O[0][m]])
    bmesh.ops.recalc_face_normals(tmp, faces=tmp.faces)
    part.add(tmp)


def build_kneepads():
    pads = Part()
    straps = Part()
    bvh = body_bvh()
    for s in "lr":
        k = KNEE[s]
        foot = bone(f"foot_{s}")
        axis = (k - foot).normalized()
        hit, n, fi, d = bvh.ray_cast(k + Vector((0, 0.01, 0.0)), Vector((0, 0, 1)), 0.3)
        front = hit.z if hit is not None else k.z + 0.05
        p = Vector((k.x, k.y + 0.01, front + 0.017 + 0.02))
        pads.box(p, (0.09, 0.13, 0.03), rot=Matrix.Rotation(-0.06, 3, "X"), bevel=0.013, segs=3)
        for dy in (0.045, -0.045):
            limb_ring(straps, k + axis * dy, axis, 0.017, 0.018, 0.003)
    w = lambda p: body_weights(p, lambda b: b.startswith(("calf", "thigh")) and "twist" not in b and "Cor" not in b and "_lwr" not in b and "knee" not in b, k=4)
    pads.finish("kneepads", "pads", w, smooth=False)
    straps.finish("kneepads", "webbing", w, smooth=True)


def add_boot_feet(feet, shaft):
    """The boot below the ankle: the convex hull of each foot (toes and all) with the shaft's
    lowest ring, lifted by the leather's thickness, so no toe shows through; then the soles."""
    out = []
    for side in "lr":
        pts = [BPOS[i] for i in feet if (BPOS[i].x > 0) == (side == "l")]
        pts += [q for q in shaft if q.y < ANKLE_Y + 0.05 and (q.x > 0) == (side == "l")]
        bm = bmesh.new()
        for q in pts:
            bm.verts.new(q)
        res = bmesh.ops.convex_hull(bm, input=bm.verts)
        drop = {g for g in res.get("geom_interior", []) + res.get("geom_unused", []) if isinstance(g, bmesh.types.BMVert)}
        bmesh.ops.delete(bm, geom=list(drop), context="VERTS")
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.normal_update()
        # Smooth the hull's facets a little and round the toe.
        bmesh.ops.subdivide_edges(bm, edges=bm.edges, cuts=1, use_grid_fill=True)
        for _ in range(2):
            bmesh.ops.smooth_vert(bm, verts=bm.verts, factor=0.5, use_axis_x=True, use_axis_y=True, use_axis_z=True)
        bm.normal_update()
        for v in bm.verts:
            v.co = v.co + v.normal * 0.007
        bmesh.ops.triangulate(bm, faces=bm.faces)
        out.append(bm)
    part = Part()
    for bm in out:
        part.add(bm)
    part.finish("boots", "boot", lambda p: body_weights(p, lambda b: b.startswith(("foot", "ball", "calf_twist_02", "ankle")), k=4), smooth=True)
    add_soles(shaft + [BPOS[i] for i in feet])


def add_soles(boot_pos):
    """Lugged soles under the boots: the boot's footprint (its vertices near the ground) in the
    foot's heading, a little wider, from the floor up over the welt."""
    sole = Part()
    for s in "lr":
        f, b = bone(f"foot_{s}"), bone(f"ball_{s}")
        fwd = Vector((b.x - f.x, 0.0, b.z - f.z)).normalized()
        side = Vector((0.0, 1.0, 0.0)).cross(fwd).normalized()
        low = [q for q in boot_pos if q.y < 0.045 and (q.x > 0) == (s == "l")]
        if not low:
            continue
        along = [(q - f).dot(fwd) for q in low]
        across = [(q - f).dot(side) for q in low]
        a0, a1 = min(along) - 0.004, max(along) + 0.006
        c0, c1 = min(across) - 0.004, max(across) + 0.004
        centre = f + fwd * ((a0 + a1) * 0.5) + side * ((c0 + c1) * 0.5)
        centre.y = 0.008
        rot = Matrix((side, Vector((0, 1, 0)), fwd)).transposed()
        print(f"sole {s}: {a1 - a0:.3f} long, {c1 - c0:.3f} wide")
        sole.box(centre, (c1 - c0, 0.026, a1 - a0), rot=rot, bevel=0.008, segs=2)
        # Heel block.
        sole.box(centre - fwd * ((a1 - a0) * 0.32) + Vector((0, -0.004, 0)), (c1 - c0 - 0.006, 0.03, (a1 - a0) * 0.3), rot=rot, bevel=0.006, segs=1)
    sole.finish("boots", "sole", lambda p: body_weights(p, lambda b: b.startswith(("foot", "ball")), k=4), smooth=False)


def merge_by_material():
    """One mesh per material (`kit.<material>`), packs apart (one per squad look): fewer draws."""
    groups = {}
    for o in OUTS:
        part, mat = o.name.split(".")[:2]
        # Packs (one per squad look) and the helmet (replaceable by a pack helmet) stay apart.
        key = o.name.split(".")[0] if part.startswith("pack_") else (f"helmet.{mat}" if part == "helmet" else "kit." + mat)
        groups.setdefault(key, []).append(o)
    merged = []
    for key, objs in groups.items():
        if len(objs) > 1:
            with bpy.context.temp_override(active_object=objs[0], selected_editable_objects=objs, object=objs[0]):
                bpy.ops.object.join()
        o = objs[0]
        name = key if key.startswith(("kit.", "helmet.")) else o.name.split(".")[0] + "." + o.name.split(".")[1]
        o.name = name
        o.data.name = name
        merged.append(o)
    OUTS[:] = merged


def export():
    merge_by_material()
    # Joints: only the bones the gear uses (Bevy caps a skin at 256 joints).
    used = {g.name for o in OUTS for g in o.vertex_groups}
    bpy.ops.object.select_all(action="DESELECT")
    for o in list(bpy.data.objects):
        if o.type == "MESH" and o not in OUTS:
            bpy.data.objects.remove(o, do_unlink=True)
    bpy.context.view_layer.objects.active = ARM
    bpy.ops.object.mode_set(mode="EDIT")
    keep = set(used)
    # Keep every ancestor of a used bone so the hierarchy stays intact.
    for b in list(ARM.data.edit_bones):
        if b.name in used:
            p = b.parent
            while p:
                keep.add(p.name)
                p = p.parent
    for b in list(ARM.data.edit_bones):
        if b.name not in keep:
            ARM.data.edit_bones.remove(b)
    bpy.ops.object.mode_set(mode="OBJECT")
    for o in OUTS:
        o.select_set(True)
    ARM.select_set(True)
    root = ARM.parent
    if root:
        root.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=OUT,
        export_format="GLB",
        use_selection=True,
        export_animations=False,
        export_skins=True,
        export_tangents=True,
        export_materials="EXPORT",
        export_texcoords=True,
        export_normals=True,
        export_apply=False,
    )
    tris = sum(sum(len(p.vertices) - 2 for p in o.data.polygons) for o in OUTS)
    print(f"gear: {len(OUTS)} meshes, {tris} triangles, {len(keep)} joints -> {OUT}")
    for o in OUTS:
        print(f"  {o.name}: {len(o.data.vertices)} verts, {sum(len(p.vertices) - 2 for p in o.data.polygons)} tris")


build_garments()
build_carrier()
build_packs()
build_balaclava()
build_helmet()
build_kneepads()
export()
