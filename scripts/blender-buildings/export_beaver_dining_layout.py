"""Publish the fitted Blender room and independently placeable parts in metres.

Open the fitted Blender scene and pass -- OUTPUT. The combined GLB uses the
same Blender site origin as the house. Each individual GLB uses a support pivot.
"""

import argparse
import hashlib
import json
import struct
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Vector

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
from beaver_dining_placement import site_and_chairs
from beaver_portable_end_grain import bake_end_grain
from beaver_portable_oak import prepare_oak


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def portable_materials_and_lights(path, tints):
    """Keep the authored fixture lights for viewers that support punctual lights.

    The building cooker reads geometry and the house places its lights separately.
    A fallback viewer can render every fixture mesh with its ordinary material.
    """
    data = path.read_bytes()
    magic, version, total, size, chunk = struct.unpack_from("<4sIII4s", data)
    if (magic, version, total, chunk) != (b"glTF", 2, len(data), b"JSON"):
        raise ValueError(f"Invalid GLB header in {path.name}")
    doc = json.loads(data[20 : 20 + size])
    required = doc.get("extensionsRequired", [])
    if "KHR_lights_punctual" in required:
        required.remove("KHR_lights_punctual")
        if not required:
            del doc["extensionsRequired"]
    for mat in doc.get("materials", []):
        if mat["name"] in tints:
            pbr = mat["pbrMetallicRoughness"]
            if "baseColorTexture" not in pbr:
                raise ValueError(f"Missing photograph for {mat['name']}")
            pbr["baseColorFactor"] = tints[mat["name"]]
    header = json.dumps(doc, separators=(",", ":"), ensure_ascii=False).encode()
    header += b" " * (-len(header) % 4)
    rest = data[20 + size :]
    prefix = struct.pack(
        "<4sIII4s", magic, version, 20 + len(header) + len(rest), len(header), chunk
    )
    path.write_bytes(prefix + header + rest)


def ensure_surface_coordinates(mesh, world):
    """Provide metre-scaled face UVs for surfaces authored with 3D coordinates."""
    if mesh.uv_layers:
        return
    uv = mesh.uv_layers.new(name="Surface metres")
    normals = world.to_3x3().inverted().transposed()
    positions = [world @ v.co for v in mesh.vertices]
    planes = ((1, 2), (0, 2), (0, 1))
    for face in mesh.polygons:
        normal = normals @ face.normal
        u, v = planes[max(range(3), key=lambda i: abs(normal[i]))]
        for loop in face.loop_indices:
            point = positions[mesh.loops[loop].vertex_index]
            uv.data[loop].uv = (point[u], point[v])


def collect(scene, components=(), collections=()):
    objects = set()
    for name in components:
        objects.update(scene.objects[name].children_recursive)
    for name in collections:
        objects.update(bpy.data.collections[name].all_objects)
    return sorted(
        (o for o in objects if o.type in ("MESH", "LIGHT") and not o.hide_render),
        key=lambda o: o.name,
    )


def archive(out):
    path = Path(bpy.data.filepath)
    recipe = json.loads((path.parent / "fitted-dining.provenance.json").read_text())
    if digest(path) != recipe["sha256"]:
        raise ValueError("The fitted source does not match its adjacent provenance")
    scene = bpy.data.scenes[recipe["sunlight_scene"]]
    bpy.context.window.scene = scene
    bpy.context.view_layer.update()
    deps = bpy.context.evaluated_depsgraph_get()
    chairs = sorted(
        o.name
        for o in scene.objects
        if o.type == "EMPTY" and o.name.startswith("dining-chair")
    )

    def chair(name):
        return collect(scene, [name])

    kitchen = [
        "base-cabinets",
        "oven-range",
        "range-hood",
        "brass-faucet",
        "enamel-stockpot",
        "chopping-board",
        "kitchen-shelves",
        "kitchen-tiles",
        "pantry-canister-1",
        "pantry-canister-2",
        "pantry-canister-3",
        "mixing-bowl",
        "sink-basin",
        "stacked-plates",
        "kitchen-plant",
        "enamel-refrigerator",
        "oak-sideboard",
    ]
    selections = {
        "dining-table": collect(scene, ["oak-dining-table"]),
        "rush-chair": chair(chairs[0]),
        "mixing-bowl": collect(scene, collections=["Ash-glazed mixing bowl"]),
        "folded-linen": collect(scene, collections=["Folded kitchen linen"]),
        "enamel-pendant": collect(scene, collections=["Black enamel dining pendant"])
        + [
            next(
                o
                for o in scene.objects
                if o.name.startswith("Dining pendant ·") and o.type == "LIGHT"
            )
        ],
        "stair-and-runner": collect(
            scene, ["stair-flight-1"], ["Ground-floor stair joinery"]
        ),
        "kitchen": collect(scene, kitchen, ["Kitchen preparation run"]),
        "window-seat-and-blind": collect(
            scene, ["window-seat", "roman-shade"], ["Rear window textiles"]
        ),
        "garden-tree": collect(scene, collections=["Canopy in the rear garden"]),
        "ground-floor-oak": sorted(
            (
                o
                for o in scene.objects["ground-storey"].children
                if o.type == "MESH"
                and o.data.materials[0].name.startswith("Worn oiled floor")
            ),
            key=lambda o: o.name,
        ),
    }
    # Keep all six chairs exactly where Blender evaluated them.
    selections["fitted-room"] = sorted(
        {o for v in selections.values() for o in v}
        | {o for c in chairs for o in chair(c)},
        key=lambda o: o.name,
    )
    evaluated = {}
    shared = {}
    for original in selections["fitted-room"]:
        obj = original.evaluated_get(deps)
        if obj.type == "LIGHT":
            data = obj.data.copy()
        elif original.data in shared:
            data = shared[original.data]
        else:
            data = bpy.data.meshes.new_from_object(
                obj, preserve_all_data_layers=True, depsgraph=deps
            )
            # The six woven seats use the exact same UVs, materials and mesh.
            # Export their transforms as instances so glTF viewers can share it.
            if (
                original.data.name == "Four-way wrapped rush seat"
                and not original.modifiers
            ):
                shared[original.data] = data
            ensure_surface_coordinates(data, obj.matrix_world)
        evaluated[original] = (obj.matrix_world.copy(), data)
    store = bpy.data.scenes.new("Portable fitted dining assembly")
    bpy.context.window.scene = store
    for original in selections["dining-table"]:
        bake_end_grain(store, evaluated[original][1])
    materials = {
        mat
        for o, (_, data) in evaluated.items()
        if o.type == "MESH"
        for mat in data.materials
    }
    tints = prepare_oak(sorted(materials, key=lambda m: m.name))
    out.mkdir(exist_ok=True, parents=True)

    def emit(key, parts):
        if not parts:
            raise ValueError(f"No geometry found for {key}")
        points = []
        for o in parts:
            _m, data = evaluated[o]
            if o.type == "MESH":
                corners = np.array([o.matrix_world @ Vector(c) for c in o.bound_box])
                points.extend(corners)
        lo = np.min(points, axis=0)
        hi = np.max(points, axis=0)
        pivot = Vector(
            (*(0.5 * (lo + hi))[:2], hi[2] if key == "enamel-pendant" else lo[2])
        )
        if key == "fitted-room":
            pivot = Vector((0, 0, 0))
        if key == "mixing-bowl":
            pivot = scene.objects["Ash-glazed mixing bowl"].location.copy()
        if key == "folded-linen":
            pivot = scene.objects["Folded kitchen linen"].location.copy()
        subset = bpy.data.collections.new(key)
        store.collection.children.link(subset)
        offset = Matrix.Translation(-pivot)
        for original in parts:
            matrix, data = evaluated[original]
            ob = bpy.data.objects.new(original.name + " [export]", data)
            ob["source_part"] = original.name
            ob["assembly"] = original.parent.name if original.parent else original.name
            subset.objects.link(ob)
            ob.matrix_world = offset @ matrix
        bpy.context.view_layer.update()
        file = out / (key + ".glb")
        bpy.ops.export_scene.gltf(
            filepath=str(file),
            export_format="GLB",
            export_extras=True,
            export_lights=True,
            use_active_scene=True,
            export_normals=True,
            export_texcoords=True,
            export_materials="EXPORT",
        )
        portable_materials_and_lights(file, tints)
        for o in list(subset.objects):
            bpy.data.objects.remove(o, do_unlink=True)
        bpy.data.collections.remove(subset)
        return {
            "file": file.name,
            "bytes": file.stat().st_size,
            "sha256": digest(file),
            "blender_pivot_m": list(pivot),
            "gltf_pivot_m": [pivot.x, pivot.z, -pivot.y],
            "blender_bounds_m": [lo.tolist(), hi.tolist()],
            "objects": len(parts),
        }

    records = {key: emit(key, parts) for key, parts in selections.items()}
    placement = site_and_chairs(
        scene, ROOT, chairs, records["rush-chair"]["blender_pivot_m"]
    )
    stock = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina/furniture"
    credit = []
    for file in [
        stock / "photo/floorboards.provenance.json",
        stock / "joinery.provenance.json",
        stock / "wool-carpet.provenance.json",
        stock / "photo/washed-cotton.provenance.json",
        stock.parent / "painted-wood.provenance.json",
        stock.parent / "linen.provenance.json",
        path.parent / "oak/provenance.json",
    ]:
        doc = json.loads(file.read_text())
        credit.append(
            {
                k: doc[k]
                for k in (
                    "source",
                    "attribution",
                    "license",
                    "license_url",
                    "inputs",
                    "packed_maps",
                )
                if k in doc
            }
        )
    result = {
        "source_scene": path.name,
        "source_scene_sha256": digest(path),
        "design_image_sha256": recipe["design_image_sha256"],
        "source": "Fitted dining arrangement and measured human-scale furniture from 21 Beaver Street.",
        "attribution": "Earth Online and the credited material photographers.",
        "license": "Project-authored model and materials; third-party photographs retain their respective licences.",
        "credits": credit,
        "build": "scripts/blender-buildings/export_beaver_dining_layout.py",
        "axes": "glTF +Y is up. The room uses metres and the original Blender site origin. Translate each separate part by its gltf_pivot_m to reproduce its assembled position.",
        "materials": "GLB carries portable PBR channels. Procedural Cycles layers and photographic lighting remain editable in the Blender source.",
        "parts": records,
        "placement": placement,
    }
    (out / "portable-dining.provenance.json").write_text(
        json.dumps(result, indent=2) + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    out = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :]).output.resolve()
    if out.is_relative_to(ROOT):
        parser.error("Choose a destination in the external asset store")
    archive(out)
