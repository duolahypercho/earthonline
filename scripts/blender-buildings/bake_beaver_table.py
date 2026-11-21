"""Bake the installed dining light just above the oak and around its place settings."""

import argparse
import gzip
import hashlib
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.kdtree import KDTree

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from beaver_blender_scene import site_basis
from beaver_street_scene import load_surface
from check_interior_containment import accessor, read_glb
from cook_beaver_dining import DEST
from cook_beaver_mission_roof import png

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output-dir", type=Path, help="External verification bake")
args = parser.parse_args(
    sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
)
out = args.output_dir.resolve() if args.output_dir else DEST
if args.output_dir and out.is_relative_to(ROOT):
    parser.error("Keep verification bakes outside the checkout.")
out.mkdir(parents=True, exist_ok=True)

scene = bpy.context.scene
scene.frame_set(3)
scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = 2048
scene.cycles.seed = 73
scene.cycles.use_denoising = False
scene.cycles.use_adaptive_sampling = False
scene.cycles.diffuse_bounces = 4
scene.cycles.max_bounces = 10
with gzip.open(ROOT / "public/data/sf/sf-city.json.gz", "rt") as stream:
    building = next(
        h
        for h in json.load(stream)["detailBuildings"]
        if h["addr"] == "21 Beaver Street"
    )
ring = [Vector(p) for p in zip(building["points"][::2], building["points"][1::2])]
site = site_basis(ring, load_surface(ROOT, building["centroid"])["plan_scale"])[3]
inverse = site.inverted()


def bounds(objects):
    points = [
        inverse @ (obj.matrix_world @ v.co)
        for obj in objects
        for v in obj.data.vertices
    ]
    a = np.array([(p.x, p.z, -p.y) for p in points])
    return a.min(axis=0), a.max(axis=0)


boards = [
    o for o in scene.objects["oak-dining-table"].children if "dining-board" in o.name
]
lo, hi = bounds(boards)
top = hi[1]
# Probe both the tabletop and the room air up to the tallest piece of fruit.
settings = (
    "painted-fruit-bowl",
    "red-apple",
    "red-apple-small",
    "orchard-pear",
    "striped-table-runner",
    "open-recipe-book",
    "stoneware-mug",
    "stoneware-mug-rear",
)
_, cap = bounds(
    [o for n in settings for o in scene.objects[n].children if o.type == "MESH"]
)
lo[1] = top + 0.004
hi[1] = cap[1] + 0.012
floor = json.loads((DEST / "dining-floor-light.provenance.json").read_text())
source = Path(bpy.data.filepath)
digest = hashlib.sha256(source.read_bytes()).hexdigest()
if digest != floor["source_scene"]["sha256"]:
    raise ValueError(
        "Use the furnished Blender scene that supplied the pendant calibration."
    )
scale = floor["fixture_units_per_source_unit"]
# Bind the meter for each place-setting vertex to its original shipping
# primitive. Native cooking groups the triangle indices by material and keeps
# the vertices of each primitive contiguous. The house hash pins that contract.
house_path = DEST / "terrace.glb"
if hashlib.sha256(house_path.read_bytes()).hexdigest() != floor["house_sha256"]:
    raise ValueError("The installed house differs from the source light survey.")
gltf, blob = read_glb(house_path)
verts, faces, texels, surfaces, cosines = [], [], [], [], []
material_indices = {}
pendant = next(
    o
    for o in scene.objects
    if o.type == "LIGHT" and o.name.startswith("Dining pendant")
)
stack = list(gltf["scenes"][gltf.get("scene", 0)]["nodes"])
while stack:
    node = gltf["nodes"][stack.pop()]
    stack.extend(node.get("children", []))
    name = node.get("name", "")
    if "mesh" not in node or name.startswith("shadow-proxy"):
        continue
    for prim in gltf["meshes"][node["mesh"]]["primitives"]:
        material = gltf["materials"][prim["material"]]["name"]
        count = gltf["accessors"][prim["indices"]]["count"]
        first = material_indices.get(material, 0)
        material_indices[material] = first + count
        if name.split(" - ")[0] not in settings:
            continue
        obj = scene.objects[name]
        positions = accessor(gltf, blob, prim["attributes"]["POSITION"])
        normals = accessor(gltf, blob, prim["attributes"]["NORMAL"])
        triangles = accessor(gltf, blob, prim["indices"]).astype(int).reshape(-1, 3)
        if len(np.unique(triangles)) != len(positions):
            raise ValueError(f"Unused shipping vertex in {name}")
        surfaces.append(
            dict(
                mesh=name,
                material=material,
                first_index=first,
                index_count=count,
                vertex_count=len(positions),
                atlas_start=len(cosines),
            )
        )
        # Survey the very same local mesh in Blender. Check the registration
        # before transferring a value to a shipping vertex with a split normal.
        tree = KDTree(len(obj.data.vertices))
        for v in obj.data.vertices:
            tree.insert(v.co, v.index)
        tree.balance()
        xyz = positions[:, [0, 2, 1]].copy()
        xyz[:, 1] *= -1
        triangle_edges = np.cross(
            xyz[triangles[:, 1]] - xyz[triangles[:, 0]],
            xyz[triangles[:, 2]] - xyz[triangles[:, 0]],
        )
        face_normal = np.zeros_like(xyz)
        for corner in triangles.T:
            np.add.at(face_normal, corner, triangle_edges)
        normal_matrix = obj.matrix_world.to_3x3().inverted().transposed()
        for coord, normal, geometric in zip(xyz, normals, face_normal):
            local = Vector(coord)
            if tree.find(local)[2] > 0.0002:
                raise ValueError(
                    f"Shipping vertex does not match the Blender mesh: {name}"
                )
            p = obj.matrix_world @ local
            n = (
                normal_matrix @ Vector((normal[0], -normal[2], normal[1]))
            ).normalized()
            ng = (normal_matrix @ Vector(geometric)).normalized()
            if ng.dot(n) < 0.1:
                ng = n
            row, col = divmod(len(cosines), 256)
            # Paper, enamel handles and printed leaves can be seen from either
            # side. Put the two meters on opposite sides of the actual surface.
            pair = []
            for side, row_offset in ((1, 0), (-1, 96)):
                centre = p + side * 0.0025 * ng
                shading_normal = side * n
                u = shading_normal.orthogonal().normalized() * 0.0002
                v = shading_normal.cross(u)
                start = len(verts)
                verts.extend(
                    (centre - u - v, centre + u - v, centre + u + v, centre - u + v)
                )
                faces.append((start, start + 1, start + 2, start + 3))
                texels.append((col, row + row_offset, 1, 1))
                light_direction = (
                    pendant.matrix_world.translation - centre
                ).normalized()
                pair.append(max(0.025, shading_normal.dot(light_direction)))
            cosines.append(pair)
if not surfaces or len(cosines) > 96 * 256:
    raise ValueError("The tabletop settings exceed their atlas allocation.")
quad = [(lo[0], lo[2]), (hi[0], lo[2]), (hi[0], hi[2]), (lo[0], hi[2])]
start = len(verts)
verts.extend(site @ Vector((x, -z, lo[1])) for x, z in quad)
# On this parcel the de-skewed X/Z basis may be reflected in Blender.
faces.append((start, start + 1, start + 2, start + 3))
if (verts[start + 1] - verts[start]).cross(verts[start + 2] - verts[start]).z < 0:
    faces[-1] = tuple(reversed(faces[-1]))
texels.append((0, 192, 256, 64))
mesh = bpy.data.meshes.new("Dining table light receivers")
mesh.from_pydata(verts, [], faces)
mesh.update()
obj = bpy.data.objects.new(mesh.name, mesh)
scene.collection.objects.link(obj)
uv = mesh.uv_layers.new(name="Table light")
for face, (x, y, w, h) in zip(mesh.polygons, texels):
    for loop in face.loop_indices:
        corner = mesh.loops[loop].vertex_index % 4
        du, dv = ((0, 0), (1, 0), (1, 1), (0, 1))[corner]
        uv.data[loop].uv = ((x + du * w) / 256, (y + dv * h) / 256)
for ray in ("diffuse", "glossy", "transmission", "volume_scatter", "shadow"):
    setattr(obj, "visible_" + ray, False)
mat = bpy.data.materials.new("White Lambertian meter")
mat.use_nodes = True
tree = mat.node_tree
white = tree.nodes.new("ShaderNodeBsdfDiffuse")
white.inputs["Color"].default_value = (1, 1, 1, 1)
tree.links.new(white.outputs[0], tree.nodes.get("Material Output").inputs[0])
mesh.materials.append(mat)
image = bpy.data.images.new(
    "Dining fixture readings", 256, 256, float_buffer=True, alpha=False
)
image.colorspace_settings.name = "Non-Color"
target = tree.nodes.new("ShaderNodeTexImage")
target.image = image
tree.nodes.active = target
bpy.ops.object.select_all(action="DESELECT")
obj.select_set(True)
bpy.context.view_layer.objects.active = obj
scene.world.node_tree.animation_data_clear()
scene.world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0
for lamp in scene.objects:
    if lamp.type == "LIGHT" and lamp.data.type == "SUN":
        lamp.hide_render = True


def measure(passes):
    bpy.ops.object.bake(
        type="DIFFUSE",
        pass_filter=passes,
        target="IMAGE_TEXTURES",
        margin=0,
        use_clear=True,
    )
    pixels = np.empty(256 * 256 * 4, dtype=np.float32)
    image.pixels.foreach_get(pixels)
    # Row zero is the minimum plan Z, also row zero in the native texture.
    field = pixels.reshape(256, 256, 4)[:, :, :3].copy() * scale
    if not np.isfinite(field).all() or np.median(field[192:]) <= 0.01:
        raise ValueError("The table did not receive the dining light.")
    return field


total = measure({"DIRECT", "INDIRECT"})
pendant = next(
    o
    for o in scene.objects
    if o.type == "LIGHT" and o.name.startswith("Dining pendant")
)
for lamp in scene.objects:
    if lamp.type == "LIGHT" and lamp != pendant:
        lamp.hide_render = True
# The direct-only pass isolates the point light for its finite-source highlight.
# The total above retains the opal, neighbouring fixtures and all bounced light.
for material in bpy.data.materials:
    if not material.node_tree:
        continue
    for node in material.node_tree.nodes:
        name = {"BSDF_PRINCIPLED": "Emission Strength", "EMISSION": "Strength"}.get(
            node.type
        )
        if name:
            slot = node.inputs[name]
            for link in list(slot.links):
                material.node_tree.links.remove(link)
            slot.default_value = 0
direct = measure({"DIRECT"})
# Store normal-independent beam strength on the curved place settings.
# The fragment shader evaluates the finite globe against the live view angle.
red = direct[:, :, 0].copy()
for side, offset in enumerate((0, 96 * 256)):
    red.flat[offset : offset + len(cosines)] /= np.asarray(cosines)[:, side]
values = np.dstack((total, red))
encoded = np.rint(255 * np.sqrt(values / (1 + values))).astype(np.uint8)
path = out / "tabletop-light.png"
png(path, encoded)
metadata = dict(
    source="Furnished 21 Beaver dining table, crockery, fruit and installed interior lamps.",
    license="Project-authored lighting data",
    attribution="Earth Online",
    url="/" + (DEST / path.name).relative_to(ROOT / "public").as_posix(),
    build="scripts/blender-buildings/bake_beaver_table.py",
    source_scene={"file": source.name, "sha256": digest},
    house_sha256=floor["house_sha256"],
    sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
    bytes=path.stat().st_size,
    minimum=lo.tolist(),
    maximum=hi.tolist(),
    tabletop_height_m=top,
    surfaces=surfaces,
    surface_vertices=len(cosines),
    reverse_face_row=96,
    board_map=[256, 64],
    atlas_origin=[256, 0],
    fixture_units_per_source_unit=scale,
    lamp_parcel_xyz=floor["lamp_parcel_xyz"],
    lamp_radius_m=pendant.data.shadow_soft_size,
    lamp_linear_rgb=list(pendant.data.color),
    samples=scene.cycles.samples,
    diffuse_bounces=scene.cycles.diffuse_bounces,
    transfer_encoding="sqrt(L/(1+L))",
    channels="Installed fixture RGB including bounce, direct pendant red in alpha.",
)
path.with_suffix(".provenance.json").write_text(json.dumps(metadata, indent=2) + "\n")
print("Baked table", lo, hi, path, flush=True)
