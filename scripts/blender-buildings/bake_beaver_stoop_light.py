"""Measure the daylight and practical on each fitted clay tread, nose and riser."""

import argparse
import json
import sys
from pathlib import Path

import bpy
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bake_beaver_porch_light import digest
from bake_beaver_porch_surfaces import HOURS, light_conditions
from beaver_entry import SETBACK

WIDTH, STRIP = 128, 32


def bake(dest):
    src = Path(bpy.data.filepath)
    record = json.loads((src.parent / "stoop.provenance.json").read_text())
    if digest(src) != record["sha256"]:
        raise ValueError("Open the fitted terracotta house")
    count = len(record["courses"])
    if not 2 <= count <= 8:
        raise ValueError("Course count outside the light atlas")
    scene = bpy.context.scene
    inverse = np.linalg.inv(record["parcel_from_assembled_gltf"])
    points, faces, uv = [], [], []
    for course in range(count):
        row = course - 1
        y = 0 if course == 0 else -0.145 * row
        front = 0 if course == 0 else -0.33 * course
        rear = SETBACK if course == 0 else -0.33 * row
        # Keep the receiver a few millimetres outside the clay, including the rolled cap.
        profile = [
            (rear - 0.003, y + 0.016, 0),
            (front + 0.002, y + 0.016, 1 if course == 0 else 0.64),
        ]
        if course:
            profile.extend(
                [
                    (front - 0.013, y + 0.014, 0.68),
                    (front - 0.020, y + 0.006, 0.72),
                    (front - 0.023, y - 0.008, 0.75),
                    (front - 0.020, y - 0.022, 0.78),
                ]
            )

        def strip(zs, course):
            rings = []
            for z, h, v in zs:
                r = []
                for u in (0, 1):
                    x, yy, zz, _ = inverse @ [5.15 + 1.60 * u, h, z, 1]
                    r.append(len(points))
                    points.append((x, -zz, yy))
                    uv.append((u, (course + v) / count))
                if rings:
                    faces.append((rings[-1][0], rings[-1][1], r[1], r[0]))
                rings.append(r)

        strip(profile, course)
        if course:
            strip(
                [(front - 0.008, y - 0.022, 0.78), (front - 0.008, y - 0.144, 1)],
                course,
            )
    mesh = bpy.data.meshes.new("Terracotta irradiance receiver")
    mesh.from_pydata(points, [], faces)
    mesh.update()
    layer = mesh.uv_layers.new(name="Stoop light")
    for loop in mesh.loops:
        layer.data[loop.index].uv = uv[loop.vertex_index]
    ob = bpy.data.objects.new(mesh.name, mesh)
    scene.collection.objects.link(ob)
    ob.visible_shadow = False
    ob.visible_diffuse = False
    ob.visible_glossy = False
    mat = bpy.data.materials.new("Unit diffuse stair receiver")
    mat.use_nodes = True
    mesh.materials.append(mat)
    tree = mat.node_tree
    bs = tree.nodes.new("ShaderNodeBsdfDiffuse")
    bs.inputs["Color"].default_value = (1, 1, 1, 1)
    tree.links.new(bs.outputs[0], tree.nodes["Material Output"].inputs[0])
    target = tree.nodes.new("ShaderNodeTexImage")
    tree.nodes.active = target
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    scene.render.engine = "CYCLES"
    scene.cycles.seed = 169
    scene.cycles.use_adaptive_sampling = False
    scene.render.use_persistent_data = True
    if sys.platform == "darwin":
        prefs = bpy.context.preferences.addons["cycles"].preferences
        prefs.compute_device_type = "METAL"
        devices = prefs.get_devices_for_type("METAL")
        for device in devices:
            device.use = device.type == "METAL"
        scene.cycles.device = "GPU" if any(d.use for d in devices) else "CPU"

    def measure(label, passes):
        im = bpy.data.images.new(
            label, WIDTH, STRIP * count, float_buffer=True, alpha=True
        )
        im.colorspace_settings.name = "Non-Color"
        target.image = im
        bpy.ops.object.bake(
            type="DIFFUSE",
            pass_filter=passes,
            target="IMAGE_TEXTURES",
            margin=0,
            use_clear=True,
        )
        arr = np.empty((STRIP * count, WIDTH, 4), np.float32)
        im.pixels.foreach_get(arr.ravel())
        target.image = None
        bpy.data.images.remove(im)
        if not np.isfinite(arr).all() or arr.min() < 0:
            raise ValueError(label)
        print(label, arr[:, :, :3].max(), flush=True)
        return arr

    light, sky, _, _ = light_conditions(scene, measure)
    dest.mkdir(parents=True, exist_ok=True)
    data = dest / "irradiance.npy"
    np.save(data, np.stack(light))
    (dest / "irradiance.provenance.json").write_text(
        json.dumps(
            {
                "source_sha256": record["sha256"],
                "sha256": digest(data),
                "courses": count,
                "hours": HOURS,
                "sky_strength": sky,
                "build": "scripts/blender-buildings/bake_beaver_stoop_light.py",
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("out", type=Path)
    bake(p.parse_args(sys.argv[sys.argv.index("--") + 1 :]).out)
