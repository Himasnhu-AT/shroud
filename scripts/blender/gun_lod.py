"""Low-detail gun model (headless Blender): the high model's rest pose baked to a static mesh,
quadric-decimated to RATIO of its triangles, sharp edges kept, material names kept.

    Blender -b --factory-startup --python scripts/blender/gun_lod.py -- <high.glb> <low.glb> [RATIO]

Same frame as the high model (metres, barrel toward -Z), so the hands' anchors fit both.
"""
import math
import sys

import bpy

argv = sys.argv[sys.argv.index("--") + 1:]
SRC, OUT = argv[0], argv[1]
RATIO = float(argv[2]) if len(argv) > 2 else 0.35
# Textured packs: the low tier is seen from 4-6 m and beyond, so its maps shrink to this.
MAX_TEX = int(argv[3]) if len(argv) > 3 else 512

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=SRC)
# Scene meshes only (the glTF importer keeps a bone-shape icosphere outside the scene).
meshes = [o for o in bpy.context.scene.objects if o.type == "MESH" and not o.name.startswith("Icosphere")]
before = after = 0
for o in meshes:
    bpy.context.view_layer.objects.active = o
    before += sum(len(p.vertices) - 2 for p in o.data.polygons)
    # Bake the rig's rest pose and every parent transform into the vertices.
    with bpy.context.temp_override(object=o, active_object=o, selected_objects=[o]):
        for m in list(o.modifiers):
            bpy.ops.object.modifier_apply(modifier=m.name)
    mw = o.matrix_world.copy()
    o.parent = None
    o.data.transform(mw)
    o.matrix_world.identity()
    o.vertex_groups.clear()
    d = o.modifiers.new("Dec", "DECIMATE")
    d.ratio = RATIO
    d.use_collapse_triangulate = True
    with bpy.context.temp_override(object=o, active_object=o, selected_objects=[o]):
        bpy.ops.object.modifier_apply(modifier="Dec")
    o.data.set_sharp_from_angle(angle=math.radians(35))
    after += sum(len(p.vertices) - 2 for p in o.data.polygons)
for o in list(bpy.data.objects):
    if o.type != "MESH":
        bpy.data.objects.remove(o, do_unlink=True)
for img in bpy.data.images:
    if img.size[0] > MAX_TEX or img.size[1] > MAX_TEX:
        k = MAX_TEX / max(img.size)
        img.scale(max(1, int(img.size[0] * k)), max(1, int(img.size[1] * k)))
        img.pack()
for o in meshes:
    o.select_set(True)
bpy.ops.export_scene.gltf(filepath=OUT, export_format="GLB", use_selection=True, export_animations=False, export_skins=False, export_materials="EXPORT", export_normals=True, export_texcoords=True)
print(f"gun lod: {before} -> {after} triangles -> {OUT}")
