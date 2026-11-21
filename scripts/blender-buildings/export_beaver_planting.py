"""Export the reviewed Blender garden as metric meshes with surveyed ground anchors."""

import gzip
import hashlib
import json
from pathlib import Path

import bpy
from mathutils import Vector

root = Path(__file__).resolve().parents[2]
out = root / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/courtyard-planting.glb"
site = bpy.data.collections["Site · parcel and surrounding buildings"]
objects = sorted(
    (o for o in site.objects if o.name.startswith(("Boundary grass ·", "Courtyard citrus ·"))),
    key=lambda o: o.name,
)
with gzip.open(root / "public/data/sf/sf-city.json.gz", "rt") as stream:
    house = next(b for b in json.load(stream)["detailBuildings"] if b["addr"] == "21 Beaver Street")
ring = [Vector(p) for p in zip(house["points"][::2], house["points"][1::2])]
across = (ring[2] - ring[1]) * 1.25 / 7
back = (ring[0] - ring[1]) * 1.25 / 13
axis = across.normalized()
side = Vector((-axis.y, axis.x))


def plan(anchor):
    depth = -anchor.y / side.dot(back)
    return ((anchor.x - axis.dot(back) * depth) / across.length, depth)


def channel(x):
    value = 12.92 * x if x <= .0031308 else 1.055 * x**(1 / 2.4) - .055
    return round(255 * value)


# Eight sides at the trunk base are shared by all the branches and leaf materials.
trunk = next(o for o in objects if o.name.endswith("Citrus · bark"))
base = [trunk.data.vertices[face.vertices[0]].co for face in list(trunk.data.polygons)[:8]]
tree_root = sum(base, Vector()) / len(base)
bpy.ops.object.select_all(action="DESELECT")
for obj in objects:
    mesh = obj.data
    grassy = obj.name.startswith("Boundary grass")
    family = "courtyard-grass" if grassy else "citrus-bark" if obj == trunk else "citrus-leaf"
    mat = mesh.materials[0]
    mat.name = family + "@" + "".join(f"{channel(c):02x}" for c in mat.diffuse_color[:3])
    # The source grass already has photographic site UVs. The portable mesh
    # reserves TEXCOORD_0 for a single surveyed root shared by the whole blade.
    for old_uv in list(mesh.uv_layers):
        mesh.uv_layers.remove(old_uv)
    uv = mesh.uv_layers.new(name="Ground anchor")
    for face in mesh.polygons:
        ids = face.vertices
        anchor = ((mesh.vertices[ids[0]].co + mesh.vertices[ids[1]].co) * .5
                  if grassy else tree_root)
        x, z = plan(anchor)
        # MeshBatch creates independent vertices per face. Store only the metric
        # offsets in POSITION; the game resamples each root against its installed grade.
        for loop in face.loop_indices:
            vi = mesh.loops[loop].vertex_index
            mesh.vertices[vi].co -= anchor
            uv.data[loop].uv = (x, 1 - z)
    mesh.update()
    obj.select_set(True)

if len(objects) != 9:
    raise ValueError("Expected four grass, four leaf and one bark material")
bpy.ops.export_scene.gltf(filepath=str(out), export_format="GLB", use_selection=True,
                          export_texcoords=True, export_normals=True, export_animations=False)
data = out.read_bytes()
manifest = {
    "source": "Original modeled grass and citrus from the editable 21 Beaver Street scene",
    "build": "scripts/blender-buildings/export_beaver_planting.py",
    "scene_builder": "scripts/blender-buildings/beaver_blender_scene.py",
    "footprint": "public/data/sf/sf-city.json.gz",
    "attribution": "Earth Online contributors",
    "license": "MIT",
    "url": "/" + out.relative_to(root / "public").as_posix(),
    "bytes": len(data),
    "sha256": hashlib.sha256(data).hexdigest(),
    "encoding": "POSITION in metres relative to the root, TEXCOORD_0.xy is root XZ in the seven-by-thirteen house plan",
}
out.with_suffix(".provenance.json").write_text(json.dumps(manifest, indent=2) + "\n")
