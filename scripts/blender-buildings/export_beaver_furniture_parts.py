"""Export portable interior pieces and the hinged entry from the dressed Blender file."""

import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from beaver_entry import ACROSS_M, DEPTH_M

root = Path(__file__).resolve().parents[2]
surfaces = root / "public/assets/buildings/beaver-terrace/finishes/patina"
parser = argparse.ArgumentParser()
parser.add_argument("destination", type=Path)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
destination = args.destination.resolve()
if destination.is_relative_to(root):
    raise ValueError("Keep portable source models outside this checkout.")
destination.mkdir(parents=True, exist_ok=True)
parts = (
    "front-door",
    "upholstered-sofa",
    "woven-lap-throw",
    "sitting-room-rug",
    "coffee-table",
    "clothbound-book",
    "sitting-room-coffee-cup",
    "dining-chair",
    "dining-chair-rear",
    "dining-chair-west-front",
    "dining-chair-west-rear",
    "dining-chair-east-front",
    "dining-chair-east-rear",
    "oak-sideboard",
    "desk-chair",
    "workroom-chair",
)
lookup = {}
provenance = []
for name in ("painted-wood", "linen", "furniture/wool-carpet", "furniture/joinery"):
    manifest = json.loads((surfaces / (name + ".provenance.json")).read_text())
    provenance.append({k: manifest[k] for k in ("source", "license", "attribution", "sha256")})
    for family in manifest["families"]:
        lookup[family] = (name, manifest)
materials = {
    m
    for name in parts
    for o in bpy.data.collections[name].all_objects
    if o.type == "MESH"
    for m in o.data.materials
}
for mat in sorted(materials, key=lambda m: m.name):
    family = mat.name.partition("@")[0]
    if family not in lookup:
        continue
    group, spec = lookup[family]
    source = bpy.data.images[group + " diffuse"]
    pixels = np.empty(len(source.pixels), dtype=np.float32)
    source.pixels.foreach_get(pixels)
    gain = 1 + spec["paint_mix"] * (np.asarray(mat.diffuse_color[:3]) - 1)
    pixels.reshape(-1, 4)[:, :3] *= gain
    base = bpy.data.images.new(mat.name + " dyed albedo", *source.size, alpha=False)
    base.colorspace_settings.name = "sRGB"
    base.pixels.foreach_set(pixels)
    base.pack()
    tree = mat.node_tree
    tree.nodes.clear()
    shader = tree.nodes.new("ShaderNodeBsdfPrincipled")
    output = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(shader.outputs[0], output.inputs["Surface"])
    coords = tree.nodes.new("ShaderNodeUVMap")
    coords.uv_map = "Photographic finish"

    def texture(image):
        node = tree.nodes.new("ShaderNodeTexImage")
        node.image = image
        tree.links.new(coords.outputs[0], node.inputs["Vector"])
        return node.outputs["Color"]

    tree.links.new(texture(base), shader.inputs["Base Color"])
    channels = tree.nodes.new("ShaderNodeSeparateColor")
    tree.links.new(texture(bpy.data.images[group + " ORM"]), channels.inputs[0])
    tree.links.new(channels.outputs["Green"], shader.inputs["Roughness"])
    tree.links.new(channels.outputs["Blue"], shader.inputs["Metallic"])
    if spec["normal_strength"] > 0:
        normal = tree.nodes.new("ShaderNodeNormalMap")
        normal.uv_map = coords.uv_map
        normal.inputs["Strength"].default_value = spec["normal_strength"]
        tree.links.new(texture(bpy.data.images[group + " normal"]), normal.inputs["Color"])
        tree.links.new(normal.outputs["Normal"], shader.inputs["Normal"])

entries = []
for name in parts:
    handle = bpy.data.objects[name]
    original = handle.location.copy()
    original_scale = handle.scale.copy()
    path = destination / (name + ".glb")
    bpy.ops.object.select_all(action="DESELECT")
    for obj in bpy.data.collections[name].all_objects:
        obj.select_set(True)
    try:
        handle.location = (0, 0, 0)
        if name == "front-door":
            # The root stays on its hinge, with the installed size in metres.
            handle.scale = (ACROSS_M, DEPTH_M, 1.0)
        bpy.ops.export_scene.gltf(
            filepath=str(path),
            export_format="GLB",
            use_selection=True,
            export_texcoords=True,
            export_yup=True,
            export_animations=False,
            export_extras=False,
        )
    finally:
        handle.location = original
        handle.scale = original_scale
    raw = path.read_bytes()
    entries.append(
        dict(
            asset=path.name,
            bytes=len(raw),
            sha256=hashlib.sha256(raw).hexdigest(),
            pivot_in_house_m=[original.x, original.z, -original.y],
        )
    )
(destination / "components.json").write_text(
    json.dumps(
        dict(
            model_source="Original Earth Online Blender models, MIT",
            materials=provenance,
            components=entries,
        ),
        indent=2,
    )
    + "\n"
)
