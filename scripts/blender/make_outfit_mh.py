"""Fit the Tactical Soldier Outfit (Fab, RMDavid) to the enemy's MetaHuman (headless Blender).

    Blender -b --factory-startup --python scripts/blender/make_outfit_mh.py -- \
        ENEMY_MH.glb OUTFIT.glb HELMET.glb TEXTURES_DIR OUT.glb

ENEMY_MH.glb is `models/metahuman/enemy_mh.glb`; OUTFIT / HELMET are the engine importer's GLBs
of `Soldier_Outfit_v1.fbx` / `Soldier_Helmet_Outfit_v1.fbx` (assets/library/characters/
tactical-soldier-outfit/); TEXTURES_DIR is the unpacked `soldier_outfit_textures.zip`. Run by
`scripts/make-outfit-mh.sh`, which also rebinds the result (`gear_rebind.py`).

The outfit is rigged to a taller MetaHuman (head joint 1.62 m against the enemy's 1.43 m). Each
vertex is moved from the outfit's rest skeleton onto the enemy's with its own skin weights:
v' = sum_b w_b * E_b * S * O_b^-1 * v, where O_b / E_b are the bone's rest world matrices in the
two skeletons and S scales by the height ratio about the bone, so every piece follows its bones to
the enemy's proportions and pose. Vertices that end up inside the body are pushed just outside
it. Weights stay the outfit's own (same UE bone names); bones the enemy lacks fall back to their
nearest parent it has. Each texture set gets its own maps (colour and normal capped at 2K,
occlusion at 1K, JPEG in the GLB; roughness is constant: the download has no roughness map);
the helmet uses its packed AO / roughness / metal map. The slung sniper rifle is dropped. Pieces
are joined per texture set, capped at CAP triangles each, and named `<part>.own` (enemy/skin.rs
keeps their own material).
"""
import os
import re
import sys

import bpy
from mathutils import Matrix
from mathutils.bvhtree import BVHTree
from mathutils.kdtree import KDTree

argv = sys.argv[sys.argv.index("--") + 1:]
ENEMY, OUTFIT, HELMET, TEX, OUT = argv
PUSH = 0.004  # metres outside the body skin
NEAR = 0.06  # only vertices this close to the body are pushed
CAP = 10000  # triangles per joined piece (about 55 k per soldier)

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=ENEMY, bone_heuristic="BLENDER")
ARM = next(o for o in bpy.data.objects if o.type == "ARMATURE")
ARM.data.pose_position = "REST"
BODY = bpy.data.objects["body_lod1_mesh"]
enemy_objs = set(bpy.data.objects)

bpy.ops.import_scene.gltf(filepath=OUTFIT, bone_heuristic="BLENDER")
bpy.ops.import_scene.gltf(filepath=HELMET, bone_heuristic="BLENDER")
new = [o for o in bpy.data.objects if o not in enemy_objs]
for a in (o for o in new if o.type == "ARMATURE"):
    a.data.pose_position = "REST"
bpy.context.view_layer.update()


def rest_world(arm, name):
    """A bone's rest frame in world space, location and rotation only (the outfit's armature
    object carries the FBX unit scale; sizes are matched separately by the height ratio)."""
    loc, rot, _ = (arm.matrix_world @ arm.data.bones[name].matrix_local).decompose()
    return Matrix.LocRotScale(loc, rot, None)


ENEMY_BONES = set(ARM.data.bones.keys())
H_ENEMY = rest_world(ARM, "head").to_translation() - rest_world(ARM, "ball_l" if "ball_l" in ENEMY_BONES else "foot_l").to_translation()


def scale_of(oarm):
    ho = rest_world(oarm, "head").to_translation() - rest_world(oarm, "ball_l" if "ball_l" in oarm.data.bones else "foot_l").to_translation()
    return H_ENEMY.length / ho.length


def enemy_bone(oarm, name):
    """The bone of the enemy a vertex group of the outfit maps to (itself or nearest parent)."""
    b = oarm.data.bones.get(name)
    while b is not None and b.name not in ENEMY_BONES:
        b = b.parent
    return b.name if b else "pelvis"


# Body surface for the push-out (world space, rest pose).
bdeps = BODY.evaluated_get(bpy.context.evaluated_depsgraph_get())
bme = bdeps.to_mesh()
BPOS = [BODY.matrix_world @ v.co for v in bme.vertices]
BNRM = [(BODY.matrix_world.to_3x3() @ v.normal).normalized() for v in bme.vertices]
BVH = BVHTree.FromPolygons(BPOS, [list(p.vertices) for p in bme.polygons])
bdeps.to_mesh_clear()


def set_of(mat_name):
    """Texture set key for a download material: '2' for Texture_set_2.001, 'helmet' for Textureset_1."""
    if mat_name.startswith("Textureset"):
        return "helmet"
    m = re.match(r"Texture_set_(\d+)", mat_name)
    return m.group(1) if m else None


def image(path, px):
    im = bpy.data.images.load(path, check_existing=True)
    if max(im.size) > px:
        im.scale(px, px * im.size[1] // im.size[0])
    return im


def material(key):
    m = bpy.data.materials.get("outfit_" + key)
    if m:
        return m
    m = bpy.data.materials.new("outfit_" + key)
    m.use_nodes = True
    nt = m.node_tree
    bsdf = nt.nodes["Principled BSDF"]

    def tex(path, px, colour):
        n = nt.nodes.new("ShaderNodeTexImage")
        n.image = image(path, px)
        if not colour:
            n.image.colorspace_settings.name = "Non-Color"
        return n

    def ao(src):
        grp = bpy.data.node_groups.get("glTF Material Output") or bpy.data.node_groups.new("glTF Material Output", "ShaderNodeTree")
        if not grp.interface.items_tree:
            grp.interface.new_socket("Occlusion", in_out="INPUT", socket_type="NodeSocketFloat")
        g = nt.nodes.new("ShaderNodeGroup")
        g.node_tree = grp
        nt.links.new(src, g.inputs[0])

    if key == "helmet":
        d = os.path.join(TEX, "Helmet_Texture", "2k", "V_1")
        nt.links.new(tex(os.path.join(d, "Basecolor_2k_01.png"), 2048, True).outputs["Color"], bsdf.inputs["Base Color"])
        orm = tex(os.path.join(d, "AoRoughnessMetallic_2k_01.png"), 1024, False)
        sep = nt.nodes.new("ShaderNodeSeparateColor")
        nt.links.new(orm.outputs["Color"], sep.inputs[0])
        nt.links.new(sep.outputs[1], bsdf.inputs["Roughness"])
        nt.links.new(sep.outputs[2], bsdf.inputs["Metallic"])
        ao(sep.outputs[0])
        nrm = os.path.join(d, "Normal_2k_01.png")
    else:
        d = os.path.join(TEX, "Soldier_Outfit_Textures")
        col = tex(os.path.join(d, f"Texture_Base_Color_Set_{key}.png"), 2048, True)
        nt.links.new(col.outputs["Color"], bsdf.inputs["Base Color"])
        op = os.path.join(d, f"Texture_Base_Color_Set_{key}_Opacity.png")
        if os.path.exists(op):
            nt.links.new(tex(op, 1024, False).outputs["Color"], bsdf.inputs["Alpha"])
            m.blend_method = "CLIP" if hasattr(m, "blend_method") else None
        bsdf.inputs["Roughness"].default_value = 0.78
        bsdf.inputs["Metallic"].default_value = 0.0
        ao(tex(os.path.join(d, f"Texture_AO_Set_{key}.png"), 1024, False).outputs["Color"])
        nrm = os.path.join(d, f"Texture_Normal_Set_{key}.png")
    nm = nt.nodes.new("ShaderNodeNormalMap")
    nt.links.new(tex(nrm, 2048, False).outputs["Color"], nm.inputs["Color"])
    nt.links.new(nm.outputs["Normal"], bsdf.inputs["Normal"])
    return m


def fit(ob):
    """New world-space vertex positions and per-vertex enemy weights for an outfit mesh."""
    oarm = next((md.object for md in ob.modifiers if md.type == "ARMATURE" and md.object), None)
    if oarm is None and ob.parent and ob.parent.type == "ARMATURE":
        oarm = ob.parent
    if oarm is None:
        # Loose piece (no skin, no bone parent): rides the pelvis of its file's armature.
        oarm = next(o for o in new if o.type == "ARMATURE")
    s = Matrix.Scale(scale_of(oarm), 4)
    names = {g.index: g.name for g in ob.vertex_groups}
    mats = {}

    def bone_map(name):
        if name not in mats:
            mats[name] = rest_world(ARM, enemy_bone(oarm, name)) @ s @ rest_world(oarm, name).inverted() if name in oarm.data.bones else None
        return mats[name]

    rigid = ob.parent_bone if ob.parent_type == "BONE" else None
    pos, wts = [], []
    for v in ob.data.vertices:
        w = ob.matrix_world @ v.co
        groups = [(names[g.group], g.weight) for g in v.groups if g.weight > 1e-4 and names.get(g.group) in oarm.data.bones]
        if not groups:
            groups = [(rigid or "pelvis", 1.0)]
        tot = sum(x for _, x in groups)
        acc = w * 0.0
        out = {}
        for n, x in groups:
            acc += (bone_map(n) @ w) * (x / tot)
            e = enemy_bone(oarm, n)
            out[e] = out.get(e, 0.0) + x / tot
        top = sorted(out.items(), key=lambda kv: -kv[1])[:4]
        t = sum(x for _, x in top)
        pos.append(acc)
        wts.append({b: x / t for b, x in top})
    return pos, wts


def push_out(pos):
    moved = 0
    for i, p in enumerate(pos):
        hit = BVH.find_nearest(p, NEAR)
        if hit[0] is None:
            continue
        loc, n, _, _ = hit
        d = (p - loc).dot(n)
        if d < PUSH:
            pos[i] = p + n * (PUSH - d)
            moved += 1
    return moved


groups = {}
for ob in [o for o in new if o.type == "MESH"]:
    mats = [s.material.name for s in ob.material_slots if s.material]
    key = next((set_of(m) for m in mats if set_of(m)), None)
    if key is None or any("Sniper" in m for m in mats) or "Sniper" in ob.name:
        continue
    pos, wts = fit(ob)
    if os.environ.get("OUTFIT_DEBUG"):
        lo = [min(p[k] for p in pos) for k in range(3)]
        hi = [max(p[k] for p in pos) for k in range(3)]
        src = [ob.matrix_world @ v.co for v in ob.data.vertices]
        slo = [min(p[k] for p in src) for k in range(3)]
        oarm = next((md.object.name for md in ob.modifiers if md.type == "ARMATURE" and md.object), None)
        print(f"outfit: dbg {ob.name} set {key} arm {oarm} parent {ob.parent.name if ob.parent else None}/{ob.parent_type}/{ob.parent_bone} src_lo {[round(x,2) for x in slo]} lo {[round(x,2) for x in lo]} hi {[round(x,2) for x in hi]}")
    moved = push_out(pos)
    me = ob.data.copy()
    for v, p in zip(me.vertices, pos):
        v.co = p
    nob = bpy.data.objects.new(f"{key}_{ob.name}", me)
    bpy.context.scene.collection.objects.link(nob)
    nob.vertex_groups.clear()
    for i, w in enumerate(wts):
        for b, x in w.items():
            g = nob.vertex_groups.get(b) or nob.vertex_groups.new(name=b)
            g.add([i], x, "REPLACE")
    me.materials.clear()
    me.materials.append(material(key))
    groups.setdefault(key, []).append(nob)
    if moved:
        print(f"outfit: {ob.name}: {moved} vertices pushed out of the body")

for o in new:
    bpy.data.objects.remove(o, do_unlink=True)

outs = []
for key, obs in sorted(groups.items()):
    bpy.ops.object.select_all(action="DESELECT")
    for o in obs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = obs[0]
    if len(obs) > 1:
        bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    ob.name = f"outfit_{key}.own"
    # Scan-grade source (~140 k triangles per soldier): cap each piece; the weights decimate with it.
    tris = sum(len(p.vertices) - 2 for p in ob.data.polygons)
    if tris > CAP:
        dm = ob.modifiers.new("decimate", "DECIMATE")
        dm.ratio = CAP / tris
        dm.use_collapse_triangulate = True
        bpy.ops.object.modifier_apply(modifier=dm.name)
    ob.parent = ARM
    ob.matrix_parent_inverse = ARM.matrix_world.inverted()
    md = ob.modifiers.new("Armature", "ARMATURE")
    md.object = ARM
    outs.append(ob)
    print(f"outfit: {ob.name}: {sum(len(p.vertices) - 2 for p in ob.data.polygons)} tris")

# Keep only the bones the outfit uses (and their parents), like make_gear_mh.
used = {g.name for o in outs for g in o.vertex_groups}
bpy.context.view_layer.objects.active = ARM
bpy.ops.object.mode_set(mode="EDIT")
keep = set()
for b in ARM.data.edit_bones:
    if b.name in used:
        p = b
        while p:
            keep.add(p.name)
            p = p.parent
for b in list(ARM.data.edit_bones):
    if b.name not in keep:
        ARM.data.edit_bones.remove(b)
bpy.ops.object.mode_set(mode="OBJECT")
bpy.ops.object.select_all(action="DESELECT")
for o in outs:
    o.select_set(True)
ARM.select_set(True)
if ARM.parent:
    ARM.parent.select_set(True)
bpy.ops.export_scene.gltf(filepath=OUT, export_format="GLB", use_selection=True, export_animations=False,
                          export_skins=True, export_tangents=True, export_materials="EXPORT",
                          export_image_format="JPEG", export_jpeg_quality=90,
                          export_texcoords=True, export_normals=True, export_apply=False)
print(f"outfit: {len(outs)} pieces, {len(keep)} joints -> {OUT}")
