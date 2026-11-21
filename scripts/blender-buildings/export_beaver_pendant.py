"""Export the installed opal pendant with its light, pivoted at the ceiling."""

import argparse
import hashlib
import json
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

import bpy
from mathutils import Matrix, Vector

root = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser()
parser.add_argument("destination", type=Path)
out = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :]).destination.resolve()
if out.is_relative_to(root):
    parser.error("Keep the portable fixture outside the checkout.")
out.mkdir(parents=True, exist_ok=True)
house = bpy.context.scene
house.frame_set(3)
source = bpy.data.objects["dining-pendant"]
lamp = next(
    o
    for o in house.objects
    if o.type == "LIGHT" and o.name.startswith("Dining pendant")
)
parts = [o for o in source.children if o.type == "MESH"]
points = [o.matrix_world @ v.co for o in parts for v in o.data.vertices]
mount = Vector(
    (
        lamp.matrix_world.translation.x,
        lamp.matrix_world.translation.y,
        max(p.z for p in points),
    )
)
offset = Matrix.Translation(-mount)
scene = bpy.data.scenes.new("Ceiling pendant")
scene.unit_settings.system = "METRIC"
fixture = bpy.data.collections.new("Opal dining pendant")
scene.collection.children.link(fixture)
fixture.asset_mark()
fixture.asset_data.description = "Enamelled shade, painted reflector, opal bulb and a 19 W practical. Place the origin at the ceiling mount."
fixture.asset_data.author = "Earth Online"
pivot = bpy.data.objects.new("Ceiling mounting point", None)
pivot.empty_display_size = 0.04
fixture.objects.link(pivot)
for original in [*parts, lamp]:
    obj = original.copy()
    obj.data = original.data.copy()
    obj.parent = pivot
    fixture.objects.link(obj)
    obj.matrix_parent_inverse = Matrix.Identity(4)
    obj.matrix_basis = offset @ original.matrix_world
    if obj.type == "LIGHT":
        obj.name = "Opal light source"
        obj["source_power_w"] = lamp.data.energy
        obj["emitting_radius_m"] = lamp.data.shadow_soft_size
bpy.context.window.scene = scene
bpy.context.view_layer.update()
blend = out / "Opal dining pendant.blend"
bpy.ops.object.select_all(action="SELECT")
bpy.context.view_layer.objects.active = pivot
glb = out / "opal-dining-pendant.glb"
bpy.ops.export_scene.gltf(
    filepath=str(glb),
    export_format="GLB",
    use_selection=True,
    export_lights=True,
    export_cameras=False,
    export_animations=False,
    export_extras=True,
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


metadata = {
    "source": "Original 21 Beaver Street dining pendant",
    "source_scene_sha256": digest(Path(bpy.data.filepath)),
    "license": "Project-authored mesh, materials and light",
    "attribution": "Earth Online",
    "build": "scripts/blender-buildings/export_beaver_pendant.py",
    "mesh_parts": len(parts),
    "lamp_power_w": lamp.data.energy,
    "emitting_radius_m": lamp.data.shadow_soft_size,
    "light_below_mount_m": mount.z - lamp.matrix_world.translation.z,
    "dimensions_m": [
        max(p[i] for p in points) - min(p[i] for p in points) for i in range(3)
    ],
}
# Start the standalone file with a normal workspace and only the editable prop.
with TemporaryDirectory() as temp:
    library = str(Path(temp) / "fixture.blend")
    bpy.data.libraries.write(library, {fixture}, compress=True)
    collection_name = fixture.name
    bpy.ops.wm.read_factory_settings(use_empty=True)
    with bpy.data.libraries.load(library) as (_, loaded):
        loaded.collections = [collection_name]
    scene = bpy.context.scene
    scene.name = "Ceiling pendant"
    scene.unit_settings.system = "METRIC"
    scene.collection.children.link(loaded.collections[0])
    for area in bpy.data.screens["Layout"].areas:
        if area.type == "VIEW_3D":
            space = area.spaces.active
            space.region_3d.view_location = (0, 0, -0.66)
            space.region_3d.view_distance = 2.1
            space.region_3d.view_rotation = Vector((-1, 2, 0.1)).to_track_quat("-Z", "Y")
            space.shading.color_type = "MATERIAL"
            space.overlay.show_overlays = False
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(blend), compress=True)
metadata["files"] = {
    p.name: {"bytes": p.stat().st_size, "sha256": digest(p)} for p in [blend, glb]
}
(out / "pendant.provenance.json").write_text(json.dumps(metadata, indent=2) + "\n")
