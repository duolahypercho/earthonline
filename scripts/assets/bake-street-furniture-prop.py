# Bake a generated prop into the street-furniture contract.
#
#   blender -b -P scripts/assets/bake-street-furniture-prop.py -- \
#       <in.glb> <out.glb> <height-metres> [--report out.json]
#
# GENERATE AT NATIVE DENSITY. DECIMATE HERE, NEVER AT THE GENERATOR.
#
# `face_limit` on a Tripo text/image task is applied BEFORE the mesh is handed
# back, so a low limit does not give you a low-poly version of the asset - it
# gives you the only copy, permanently missing everything the cap removed.
# Measured on one hydrant prompt, same text, two runs:
#
#   face_limit  1500 ->  1460 tris  - shatters: torn faces, holes in the body
#   face_limit 30000 -> 28488 tris  - clean, with bolt heads, flange rivets and
#                                     dome ribs present as real GEOMETRY
#
# The bolts and rivets are in the generator's own concept image; capping at
# 1500 threw away 95% of the mesh and all of them, and no later step can get
# them back. Ask for 30000 (or omit the limit), keep that master off-repo under
# artifacts/, and let THIS script produce whatever the runtime budget allows.
#
# Also measured, and the reason a runtime cap is not just "the master, smaller":
# collapse decimation tears this topology badly. 4000 still shows holes in the
# dome; ~10000 is the first clean rung. A generated mesh carries no intent in
# its triangles, so there is nothing for the collapse to preserve - whereas the
# authored lathe in `street-furniture.js` reads correctly at 904. Do not expect
# a generated master to decimate into a near-tier prop; use it as reference, or
# bake its detail to a normal map onto authored geometry.
#
# WHY VERTEX COLOUR AND NOT THE TEXTURE SET.
#
# Every kind in `street-furniture.js` is drawn from merged geometry with
# VERTEX COLOURS and no texture at all - see `PALETTE` and `assemble()`. The
# pass shares a small set of untextured materials across all kinds, and the
# renderer's `applyEnvironmentGrading` owns their wet/dry response by
# `userData.envClass`. Handing one kind three JPEGs would add texture bindings
# and samplers to a pipeline this repo has already blacked out once by
# exceeding Metal's per-pipeline sampler cap, and it would take that kind out
# of the grading path the other seventeen are in.
#
# So the generated albedo is sampled per vertex and written as a colour
# attribute, and the textures are discarded. At this vertex density that keeps
# the two-tone body/cap break and the rust streaks; what it loses is normal-map
# relief, which at a 0.78 m prop is not what the eye reads anyway.
#
# The mesh is scaled to the kind's declared `height` and its base is put on
# y = 0 with the footprint centred on x/z, because the pass places an item by
# its ground point and applies its own yaw.
import bpy
import sys
import json
import math
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
if len(argv) < 3:
    raise SystemExit("usage: <in.glb> <out.glb> <height-metres> [--report out.json]")
src, dst, target_height = argv[0], argv[1], float(argv[2])
report_path = argv[argv.index("--report") + 1] if "--report" in argv else None
# Optional re-hue. The generated albedo carries the wear; the catalogue's
# PALETTE carries the city's established livery, and the two need not agree -
# the first hydrant baked to a mid grey against a palette that says
# hydrantBody #b23a2c. Tinting keeps the baked RELATIVE variation (rust
# streaks, the body/cap luminance break) and moves its hue onto the palette,
# rather than choosing between a realistic prop and a coherent street.
tint_hex = argv[argv.index("--tint") + 1].lstrip("#") if "--tint" in argv else None
tint_rgb = None
if tint_hex:
    srgb8 = [int(tint_hex[i:i + 2], 16) / 255.0 for i in (0, 2, 4)]
    tint_rgb = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in srgb8]

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)

meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
if not meshes:
    raise SystemExit("no mesh in the source glb")

# One object: the pass merges per kind anyway, and a multi-object prop would
# become several InstancedMesh buckets.
bpy.ops.object.select_all(action="DESELECT")
for o in meshes:
    o.select_set(True)
bpy.context.view_layer.objects.active = meshes[0]
if len(meshes) > 1:
    bpy.ops.object.join()
obj = bpy.context.view_layer.objects.active
bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
mesh = obj.data

# ---------------------------------------------------------------- vertex bake
# Sample the base-colour image at each loop's UV. Nearest-texel is deliberate:
# a prop this size has more vertices than the texture has meaningful features,
# and bilinear would only blur the rust breaks that carry the read.
image = None
for slot in obj.material_slots:
    mat = slot.material
    if not mat or not mat.use_nodes:
        continue
    bsdf = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if not bsdf:
        continue
    link = next((l for l in mat.node_tree.links
                 if l.to_node == bsdf and l.to_socket.name == "Base Color"), None)
    node = link.from_node if link else None
    while node is not None and node.type != "TEX_IMAGE":
        nxt = next((l.from_node for l in mat.node_tree.links if l.to_node == node), None)
        node = nxt
    if node is not None and node.type == "TEX_IMAGE" and node.image:
        image = node.image
        break

# THE SOURCE'S OWN COLOUR ATTRIBUTE HAS TO GO FIRST.
#
# Generated GLBs commonly carry an all-white COLOR_0. Adding a second attribute
# beside it exports as COLOR_1, and three.js reads COLOR_0 - so the bake is
# present in the file, ignored by the renderer, and the prop draws white.
# Measured on the first hydrant: COLOR_0 #ffffff, COLOR_1 #939291.
while mesh.color_attributes:
    mesh.color_attributes.remove(mesh.color_attributes[0])

sampled = 0
if image is not None and mesh.uv_layers.active is not None:
    w, h = image.size
    px = list(image.pixels)  # RGBA float, bottom-up rows
    uv = mesh.uv_layers.active.data
    attr = mesh.color_attributes.new(name="Col", type="BYTE_COLOR", domain="CORNER")
    raw = []
    for loop in mesh.loops:
        u, v = uv[loop.index].uv
        x = min(w - 1, max(0, int(u * w)))
        y = min(h - 1, max(0, int(v * h)))
        i = (y * w + x) * 4
        # glTF vertex colours are linear; Blender's image pixels already are.
        raw.append((px[i], px[i + 1], px[i + 2]))
        sampled += 1
    if tint_rgb and raw:
        # Normalise the bake to unit mean luminance, then multiply by the
        # palette colour. Relative variation survives; absolute hue does not.
        lum = [0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] for c in raw]
        mean = sum(lum) / len(lum) or 1.0
        raw = [tuple(min(1.0, tint_rgb[k] * (l / mean)) for k in range(3)) for l in lum]
    for loop in mesh.loops:
        c = raw[loop.index]
        attr.data[loop.index].color = (c[0], c[1], c[2], 1.0)
else:
    attr = mesh.color_attributes.new(name="Col", type="BYTE_COLOR", domain="CORNER")
    for loop in mesh.loops:
        attr.data[loop.index].color = (0.72, 0.72, 0.72, 1.0)

# The textures go. A vertex-coloured material is what the pass's shared
# material set expects, and leaving images in the file would ship megabytes the
# runtime never binds.
for slot in obj.material_slots:
    mat = slot.material
    if not mat or not mat.use_nodes:
        continue
    for n in [n for n in mat.node_tree.nodes if n.type == "TEX_IMAGE"]:
        mat.node_tree.nodes.remove(n)
    bsdf = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf:
        cattr = mat.node_tree.nodes.new("ShaderNodeVertexColor")
        cattr.layer_name = "Col"
        mat.node_tree.links.new(cattr.outputs["Color"], bsdf.inputs["Base Color"])

# ------------------------------------------------------------- triangle cap
#
# THE INSTALLED PROP MAY NOT EXCEED WHAT THE PROCEDURAL ONE RESERVED.
#
# `buildStreetFurniture`'s installedProps path throws on
# `actual > reserved`, where reserved is the triangle count of the catalogue
# geometry being replaced - 180 for the hydrant at lod0, 164 for the signal
# cabinet, 84 for the pay station. A generated mesh arrives an order of
# magnitude over that, so it is decimated here rather than at load.
#
# Decimation runs BEFORE the colour bake would be meaningless: the bake is on
# loops, and collapsing edges afterwards would average colours across the
# body/cap break. So this is placed after the bake and Blender carries the
# corner attribute through the collapse itself.
if "--max-tris" in argv:
    cap = int(argv[argv.index("--max-tris") + 1])
    current = sum(len(p.vertices) - 2 for p in mesh.polygons)
    if current > cap:
        mod = obj.modifiers.new(name="cap", type="DECIMATE")
        mod.decimate_type = "COLLAPSE"
        mod.ratio = max(0.01, cap / float(current))
        bpy.ops.object.modifier_apply(modifier=mod.name)
        mesh = obj.data
        # COLLAPSE overshoots slightly; one planar pass cleans the worst of it
        # without touching the silhouette.
        after = sum(len(p.vertices) - 2 for p in mesh.polygons)
        if after > cap:
            mod = obj.modifiers.new(name="cap2", type="DECIMATE")
            mod.decimate_type = "COLLAPSE"
            mod.ratio = max(0.01, cap / float(after))
            bpy.ops.object.modifier_apply(modifier=mod.name)
            mesh = obj.data

# ------------------------------------------------------------ metric + pivot
bb = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
lo = Vector((min(p.x for p in bb), min(p.y for p in bb), min(p.z for p in bb)))
hi = Vector((max(p.x for p in bb), max(p.y for p in bb), max(p.z for p in bb)))
# glTF is Y-up; Blender's importer puts that on +Z.
src_height = hi.z - lo.z
if not src_height > 1e-6:
    raise SystemExit("degenerate source height")
scale = target_height / src_height
obj.scale = (scale, scale, scale)
bpy.ops.object.transform_apply(scale=True)

bb = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
lo = Vector((min(p.x for p in bb), min(p.y for p in bb), min(p.z for p in bb)))
hi = Vector((max(p.x for p in bb), max(p.y for p in bb), max(p.z for p in bb)))
obj.location = (
    obj.location.x - (lo.x + hi.x) / 2.0,
    obj.location.y - (lo.y + hi.y) / 2.0,
    obj.location.z - lo.z,
)
bpy.ops.object.transform_apply(location=True)

bb = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
lo = Vector((min(p.x for p in bb), min(p.y for p in bb), min(p.z for p in bb)))
hi = Vector((max(p.x for p in bb), max(p.y for p in bb), max(p.z for p in bb)))

bpy.ops.object.select_all(action="DESELECT")
obj.select_set(True)
bpy.ops.export_scene.gltf(
    filepath=dst,
    export_format="GLB",
    use_selection=True,
    export_materials="EXPORT",
    export_image_format="NONE",
    export_normals=True,
    export_texcoords=False,
    export_yup=True,
    # EXACTLY ONE COLOUR ATTRIBUTE, IN SLOT 0.
    #
    # `export_all_vertex_colors` defaults to True and the material-driven
    # export runs as well, so the default settings emit COLOR_0 (a white one
    # the exporter synthesises) AND COLOR_1 (the bake). three.js reads COLOR_0,
    # so the prop draws white with the bake sitting unused in the file -
    # measured exactly that on the first pass. Naming the attribute and turning
    # the blanket export off puts the bake in slot 0 and emits nothing else.
    export_vertex_color="NAME",
    export_vertex_color_name="Col",
    export_all_vertex_colors=False,
    export_active_vertex_color_when_no_material=False,
)

tris = sum(len(p.vertices) - 2 for p in mesh.polygons)
report = {
    "source": src,
    "output": dst,
    "triangles": tris,
    "vertices": len(mesh.vertices),
    "loops_sampled": sampled,
    "source_height_m": round(src_height, 4),
    "scale_applied": round(scale, 6),
    "bbox_min": [round(lo.x, 4), round(lo.z, 4), round(-lo.y, 4)],
    "bbox_max": [round(hi.x, 4), round(hi.z, 4), round(-hi.y, 4)],
    "height_m": round(hi.z - lo.z, 4),
    "footprint_m": [round(hi.x - lo.x, 4), round(hi.y - lo.y, 4)],
    "vertex_colour": "baked from base-colour texture" if sampled else "flat fallback",
}
print("BAKE " + json.dumps(report))
if report_path:
    with open(report_path, "w") as f:
        json.dump(report, f, indent=2)
