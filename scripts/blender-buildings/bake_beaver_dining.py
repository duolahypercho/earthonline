"""Measure diffuse light across the furnished ground-floor boards at 21 Beaver.

Run in Blender on the finished, packed house. Pass an external directory for
measurements and optionally --install to cook the game's two linear pages.
"""

import argparse
import gzip
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from beaver_blender_scene import site_basis
from beaver_street_scene import load_surface
from cook_beaver_dining import DEST, cook

parser = argparse.ArgumentParser()
parser.add_argument("output", type=Path)
parser.add_argument("--install", action="store_true")
parser.add_argument(
    "--reference-key",
    nargs=3,
    type=float,
    metavar=("R", "G", "B"),
    required=True,
    help="Game clear-noon linear key RGB from a registered capture",
)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
if not all(math.isfinite(v) and v > 0 for v in args.reference_key):
    parser.error("Expected three positive finite components of the reference key.")
out = args.output.resolve()
if out.is_relative_to(ROOT):
    parser.error("Keep the raw light measurements outside the checkout.")
out.mkdir(parents=True, exist_ok=True)
scene = bpy.context.scene
scene.frame_set(3)
scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.seed = 73
scene.cycles.use_adaptive_sampling = False
scene.cycles.use_denoising = False
scene.cycles.diffuse_bounces = 4
scene.cycles.max_bounces = 10
with gzip.open(ROOT / "public/data/sf/sf-city.json.gz", "rt") as f:
    house = next(
        h for h in json.load(f)["detailBuildings"] if h["addr"] == "21 Beaver Street"
    )
ring = [Vector(p) for p in zip(house["points"][::2], house["points"][1::2])]
site = site_basis(ring, load_surface(ROOT, house["centroid"])["plan_scale"])[3]
# The actual boards share a datum with the feet of the portable dining ensemble.
chair = bpy.data.objects["dining-chair"]
feet = min(
    (o.matrix_world @ v.co).z
    for o in chair.children
    if o.type == "MESH"
    for v in o.data.vertices
)
height = feet + 0.002
quad = [(0, 0), (0, 13), (7, 13), (7, 0)]
mesh = bpy.data.meshes.new("Ground-floor light receiver")
mesh.from_pydata([site @ Vector((x, -z, height)) for x, z in quad], [], [(0, 1, 2, 3)])
mesh.update()
obj = bpy.data.objects.new(mesh.name, mesh)
scene.collection.objects.link(obj)
# The original oak and all furniture still reflect, emit and occlude light.
for ray in ("diffuse", "glossy", "transmission", "volume_scatter", "shadow"):
    setattr(obj, "visible_" + ray, False)
uv = mesh.uv_layers.new(name="Parcel floor coordinates")
for i, (x, z) in enumerate(quad):
    uv.data[i].uv = (x / 7, z / 13)
mat = bpy.data.materials.new("Unit diffuse floor receiver")
mat.use_nodes = True
nodes = mat.node_tree.nodes
bsdf = nodes.new("ShaderNodeBsdfDiffuse")
bsdf.inputs["Color"].default_value = (1, 1, 1, 1)
mat.node_tree.links.new(bsdf.outputs[0], nodes.get("Material Output").inputs["Surface"])
mesh.materials.append(mat)
node = nodes.new("ShaderNodeTexImage")
nodes.active = node
bpy.ops.object.select_all(action="DESELECT")
obj.select_set(True)
bpy.context.view_layer.objects.active = obj

day_sky = scene.world
unit = bpy.data.worlds.new("Uniform calibration sky")
unit.use_nodes = True
bg = unit.node_tree.nodes.get("Background")
bg.inputs["Color"].default_value = (1, 1, 1, 1)
pendant = next(
    o
    for o in scene.objects
    if o.type == "LIGHT" and o.name.startswith("Dining pendant")
)
source_key = bpy.data.objects["Morning sun"].data
key = np.asarray(source_key.color) * source_key.energy
for light in scene.objects:
    if light.type == "LIGHT":
        light.hide_render = True
for material in bpy.data.materials:
    if not material.node_tree:
        continue
    for shader in material.node_tree.nodes:
        strength = {"BSDF_PRINCIPLED": "Emission Strength", "EMISSION": "Strength"}.get(
            shader.type
        )
        if strength:
            slot = shader.inputs[strength]
            for link in list(slot.links):
                material.node_tree.links.remove(link)
            slot.default_value = 0
sun = bpy.data.objects.new(
    "Unit sun for indirect daylight", bpy.data.lights.new("Unit sun", "SUN")
)
sun.data.energy = 1
sun.data.angle = math.radians(0.53)
scene.collection.objects.link(sun)
size = 512
checks = {}


def bake(label, resolution, samples, passes=("DIRECT", "INDIRECT")):
    scene.cycles.samples = samples
    image = bpy.data.images.new(
        label, resolution, resolution, float_buffer=True, alpha=False
    )
    image.colorspace_settings.name = "Non-Color"
    node.image = image
    nodes.active = node
    bpy.ops.object.bake(
        type="DIFFUSE",
        pass_filter=set(passes),
        target="IMAGE_TEXTURES",
        margin=0,
        use_clear=True,
    )
    data = np.empty(resolution * resolution * 4, dtype=np.float32)
    image.pixels.foreach_get(data)
    values = data.reshape(resolution, resolution, 4)[:, :, :3].copy()
    bpy.data.images.remove(image)
    if not np.isfinite(values).all() or values.max() < 0.001:
        raise ValueError(f"No finite light field for {label}")
    np.save(out / (label + ".npy"), values)
    checks[label] = {
        "resolution": resolution,
        "samples": samples,
        "maximum": float(values.max()),
        "mean": values.mean((0, 1)).tolist(),
    }
    print(label, checks[label], flush=True)


scene.world = unit
bg.inputs["Strength"].default_value = 0
sun.hide_render = True
pendant.hide_render = False
bake("pendant", size, 1536)
bake("pendant-direct", size, 512, ("DIRECT",))
pendant.hide_render = True
bg.inputs["Strength"].default_value = 1
bake("uniform-sky", size, 512)
scene.world = day_sky
bake("clear-sky", size, 512)
scene.world = unit
bg.inputs["Strength"].default_value = 0
sun.hide_render = False
for side, label in [(1, "front"), (-1, "rear")]:
    for angle in [25, 57]:
        alt = math.radians(angle)
        direction = Vector((0, side * math.cos(alt), math.sin(alt)))
        sun.rotation_euler = (-direction).to_track_quat("-Z", "Y").to_euler()
        bake(f"{label}-{angle}", size // 2, 384, ("INDIRECT",))

source = Path(bpy.data.filepath)
lamp_local = site.inverted() @ pendant.matrix_world.translation
meta = {
    "source": "Furnished 21 Beaver Street Blender scene, fixed lamp, windows, architecture and furniture.",
    "attribution": "Earth Online",
    "license": "Project-authored lighting data",
    "source_scene": {
        "file": source.name,
        "sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
    },
    "build": "scripts/blender-buildings/bake_beaver_dining.py --install",
    "house_sha256": json.loads((DEST / "terrace.provenance.json").read_text())[
        "sha256"
    ],
    "parcel_extent": [7, 13],
    "receiver_height_m": height,
    "lamp_power_w": pendant.data.energy,
    "lamp_radius_m": pendant.data.shadow_soft_size,
    "lamp_parcel_xyz": [lamp_local.x, lamp_local.z, -lamp_local.y],
    "lamp_linear_rgb": list(pendant.data.color),
    "source_key_rgb": key.tolist(),
    "reference_game_key_rgb": args.reference_key,
    "diffuse_bounces": scene.cycles.diffuse_bounces,
    "seed": scene.cycles.seed,
    "measurements": checks,
}
(out / "source.json").write_text(json.dumps(meta, indent=2) + "\n")
cook(out, DEST if args.install else out, meta)
