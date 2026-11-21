"""Extract the furnished house's dining table as a self-contained Blender asset."""

import argparse
import hashlib
import json
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

import bpy
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("destination", type=Path)
out = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :]).destination.resolve()
if out.is_relative_to(ROOT):
    parser.error("Keep the editable Blender table outside the checkout.")
out.mkdir(parents=True, exist_ok=True)
source = Path(bpy.data.filepath)
scene = bpy.context.scene
scene.frame_set(3)
table = scene.objects["oak-dining-table"]
parts = [o for o in table.children_recursive if o.type == "MESH"]
points = [o.matrix_world @ v.co for o in parts for v in o.data.vertices]
low, high = [Vector([fn(p[i] for p in points) for i in range(3)]) for fn in (min, max)]
origin = Vector(((low.x + high.x) / 2, (low.y + high.y) / 2, low.z))
asset = bpy.data.collections.new("Dining table in waxed oak")
asset.asset_mark()
asset.asset_data.author = "Earth Online"
asset.asset_data.description = "Four individually finished boards, with apron and legs. The origin is on the floor between the feet."
pivot = bpy.data.objects.new("Centre of the four feet", None)
pivot.empty_display_size = 0.05
asset.objects.link(pivot)
for part in parts:
    obj = part.copy()
    obj.data = part.data.copy()
    obj.parent = pivot
    obj.matrix_parent_inverse = Matrix.Identity(4)
    obj.matrix_basis = Matrix.Translation(-origin) @ part.matrix_world
    asset.objects.link(obj)
source_digest = hashlib.sha256(source.read_bytes()).hexdigest()
sizes = list(high - low)
name = asset.name
with TemporaryDirectory() as temp:
    library = str(Path(temp) / "table.blend")
    bpy.data.libraries.write(library, {asset}, compress=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    with bpy.data.libraries.load(library) as (_, loaded):
        loaded.collections = [name]
    scene = bpy.context.scene
    scene.name = "Dining table"
    scene.collection.children.link(loaded.collections[0])
    scene.unit_settings.system = "METRIC"
    bpy.context.view_layer.update()
    images = [i for i in bpy.data.images if i.source == "FILE"]
    if not images or any(not im.packed_file for im in images):
        raise ValueError("Editable boards require their packed photographic maps")
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type == "VIEW_3D":
                space = area.spaces.active
                space.region_3d.view_rotation = Vector((-2.1, 2.9, -1.6)).to_track_quat(
                    "-Z", "Y"
                )
                space.region_3d.view_distance = 3.0
                space.region_3d.view_location = (0, 0, 0.42)
                space.shading.type = "MATERIAL"
                space.overlay.show_overlays = False
    bpy.context.preferences.filepaths.save_version = 0
    blend = out / "Waxed oak dining table.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(blend), compress=True)
finishes = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina/furniture"
credits = []
for path in (
    finishes / "joinery.provenance.json",
    finishes / "photo/floorboards.provenance.json",
):
    record = json.loads(path.read_text())
    credits.append(
        {
            key: record[key]
            for key in ("source", "attribution", "license", "license_url", "sha256")
            if key in record
        }
    )
    credits[-1]["provenance"] = path.relative_to(ROOT).as_posix()
manifest = dict(
    source="21 Beaver furnished house, original dining table and its packed photographic oak.",
    source_scene_sha256=source_digest,
    source_scene=source.name,
    attribution="Earth Online and the credited source photographers.",
    source_materials=credits,
    license="Original mesh and materials; photographic wood retains its source licence.",
    build="scripts/blender-buildings/export_beaver_table.py",
    file=blend.name,
    sha256=hashlib.sha256(blend.read_bytes()).hexdigest(),
    dimensions_m=sizes,
    mesh_parts=len(parts),
    packed_images=len(images),
    packed_image_sha256={
        im.name: hashlib.sha256(im.packed_file.data).hexdigest() for im in images
    },
)
(out / "table.provenance.json").write_text(json.dumps(manifest, indent=2) + "\n")
