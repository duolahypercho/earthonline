"""Measure daylight, practicals, and pigment on the actual recessed porch."""

import argparse
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bake_beaver_wall_daylight import plaster_objects
from beaver_entry import SETBACK
from survey_beaver_front_daylight import SUN, digest

SIZE = 128
CHARTS = ("back wall", "east reveal", "west reveal", "arch underside", "arch face")
HOURS = (8.5, 12, 17, 22)


def receiver(scene, parcel):
    inverse = np.linalg.inv(parcel)
    points, faces, tex = [], [], []

    def surface(chart, u, v):
        if chart == 0:
            return (5.16 + 1.59 * u, 2.7 * v, SETBACK - 0.052 - 0.008), (0, 0, -1)
        if chart in (1, 2):
            east = chart == 1
            return (5.169 if east else 6.741, 2.92 * v, -0.85 + 1.26 * u), (
                1 if east else -1,
                0,
                0,
            )
        if chart == 3:
            angle = math.pi * u
            return (
                5.955 - 0.786 * math.cos(angle),
                1.92 + 0.711 * math.sin(angle),
                -0.85 + 1.258 * v,
            ), (math.cos(angle), -math.sin(angle), 0)
        return (4.95 + 2.01 * u, 2.92 * v, -0.859), (0, 0, -1)

    for chart in range(len(CHARTS)):
        count = 64 if chart == 3 else 1
        for i in range(count):
            start = len(points)
            for u, v in (
                (i / count, 0),
                ((i + 1) / count, 0),
                ((i + 1) / count, 1),
                (i / count, 1),
            ):
                pos, _ = surface(chart, u, v)
                x, y, z, _ = inverse @ [*pos, 1]
                points.append(Vector((x, -z, y)))
                tex.append(((chart + u) / len(CHARTS), v))
            _, normal = surface(chart, (i + 0.5) / count, 0.5)
            x, y, z = parcel[:3, :3].T @ normal
            indices = list(range(start, start + 4))
            if (points[start + 1] - points[start]).cross(
                points[start + 2] - points[start]
            ).dot(Vector((x, -z, y))) < 0:
                indices.reverse()
            faces.append(indices)
    mesh = bpy.data.meshes.new("Porch radiance charts")
    mesh.from_pydata(points, [], faces)
    mesh.update()
    uv = mesh.uv_layers.new(name="Porch surface coordinates")
    for loop in mesh.loops:
        uv.data[loop.index].uv = tex[loop.vertex_index]
    ob = bpy.data.objects.new("Porch radiance receivers", mesh)
    scene.collection.objects.link(ob)
    for ray in ("diffuse", "glossy", "transmission", "shadow", "volume_scatter"):
        setattr(ob, "visible_" + ray, False)
    mat = bpy.data.materials.new("Neutral porch receiver")
    mat.use_nodes = True
    tree = mat.node_tree
    diffuse = tree.nodes.new("ShaderNodeBsdfDiffuse")
    diffuse.inputs["Color"].default_value = (1, 1, 1, 1)
    tree.links.new(diffuse.outputs[0], tree.nodes["Material Output"].inputs["Surface"])
    target = tree.nodes.new("ShaderNodeTexImage")
    mat.node_tree.nodes.active = target
    mesh.materials.append(mat)
    return ob, target


def light_conditions(scene, measure):
    world = scene.world
    unit = bpy.data.worlds.new("Porch calibration sky")
    unit.use_nodes = True
    bg = unit.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = (1, 1, 1, 1)
    bg.inputs["Strength"].default_value = 0
    scene.world = unit
    sun = scene.objects[SUN]
    sun.hide_render = True
    # Few light paths reach the back wall from this small enclosed source.
    # Resolve the broad indirect return separately from the hard hood shadow.
    scene.cycles.samples = 2048
    fixture_direct = measure("Porch fixtures direct", {"DIRECT"})
    scene.cycles.samples = 4096
    fixture_indirect = measure("Porch fixtures indirect", {"INDIRECT"})
    scene.cycles.samples = 384
    fixture = fixture_direct + fixture_indirect
    for obj in scene.objects:
        if obj.type == "LIGHT":
            obj.hide_render = True
    for mat in bpy.data.materials:
        if not mat.node_tree:
            continue
        for node in mat.node_tree.nodes:
            socket = {
                "BSDF_PRINCIPLED": "Emission Strength",
                "EMISSION": "Strength",
            }.get(node.type)
            if socket:
                inp = node.inputs[socket]
                for wire in list(inp.links):
                    mat.node_tree.links.remove(wire)
                inp.default_value = 0
    rows = []
    sky_strength = []
    for hour in HOURS:
        scene.world = world
        # The keyed world must belong to the scene when Blender evaluates the frame.
        scene.frame_set(int(2 * hour))
        bg_source = next(n for n in world.node_tree.nodes if n.type == "BACKGROUND")
        sky_strength.append(float(bg_source.inputs["Strength"].default_value))
        sun.hide_render = True
        sky = measure(f"Porch sky {hour}", {"DIRECT", "INDIRECT"})
        scene.world = unit
        sun.hide_render = False
        bounce = measure(f"Porch solar bounce {hour}", {"INDIRECT"})
        rows.append(sky[:, :, :3] + bounce[:, :, :3])
    sun.hide_render = True
    bg.inputs["Strength"].default_value = 1
    if sky_strength[-1] != 0 or min(sky_strength[:3]) <= 0:
        raise ValueError(
            f"Unexpected source sky at the four survey hours: {sky_strength}"
        )
    if np.max(rows[-1]) > 0.0001:
        raise ValueError("The unlit scene returned daylight after sunset")
    rows.append(measure("Unit overcast on porch", {"DIRECT", "INDIRECT"})[:, :, :3])
    rows.append(fixture[:, :, :3])
    return rows, sky_strength, fixture_direct, fixture_indirect


def bake(out):
    src = Path(bpy.data.filepath)
    provenance = json.loads((src.parent / "porch-light.provenance.json").read_text())
    if digest(src) != provenance["sha256"] or out.resolve().is_relative_to(
        Path(__file__).resolve().parents[2]
    ):
        raise ValueError("Use the fitted opal source and an external survey directory")
    out.mkdir(exist_ok=True, parents=True)
    parcel = np.array(
        json.loads(
            (
                src.parent.parent / "window-optics/optics-source.provenance.json"
            ).read_text()
        )["parcel_from_assembled_gltf"]
    )
    scene = bpy.context.scene
    scene.frame_set(34)
    ob, target = receiver(scene, parcel)
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.context.view_layer.objects.active = ob
    scene.render.engine = "CYCLES"
    if sys.platform == "darwin":
        prefs = bpy.context.preferences.addons["cycles"].preferences
        prefs.compute_device_type = "METAL"
        devices = prefs.get_devices_for_type("METAL")
        for dev in devices:
            dev.use = dev.type == "METAL"
        scene.cycles.device = "GPU" if any(dev.use for dev in devices) else "CPU"
    scene.cycles.samples = 192
    scene.cycles.seed = 169
    scene.cycles.use_adaptive_sampling = False
    scene.render.use_persistent_data = True

    def measure(label, passes, project=False):
        img = bpy.data.images.new(
            label, SIZE * len(CHARTS), SIZE, float_buffer=True, alpha=True
        )
        img.colorspace_settings.name = "Non-Color"
        target.image = img
        bpy.ops.object.bake(
            type="DIFFUSE",
            pass_filter=passes,
            target="IMAGE_TEXTURES",
            margin=0,
            use_clear=True,
            use_selected_to_active=project,
            cage_extrusion=0.026,
            max_ray_distance=0.055,
        )
        values = np.empty((SIZE, SIZE * len(CHARTS), 4), np.float32)
        img.pixels.foreach_get(values.ravel())
        target.image = None
        bpy.data.images.remove(img)
        if not np.isfinite(values).all() or np.any(values < 0):
            raise ValueError(label)
        print(label, float(values[:, :, :3].max()), flush=True)
        return values

    originals = plaster_objects(scene)
    for surface in originals:
        surface.select_set(True)
    pigment = measure("Projected porch reflectance", {"COLOR"}, True)
    for surface in originals:
        surface.select_set(False)
    rows, sky_strength, fixture_direct, fixture_indirect = light_conditions(
        scene, measure
    )
    raw = out / "porch-surfaces.npz"
    np.savez_compressed(
        raw,
        radiance=np.array(rows),
        pigment=pigment,
        fixture_direct=fixture_direct[:, :, :3],
        fixture_indirect=fixture_indirect[:, :, :3],
    )
    (out / "porch-surfaces.provenance.json").write_text(
        json.dumps(
            {
                "source_sha256": provenance["sha256"],
                "raw_sha256": digest(raw),
                "charts": CHARTS,
                "hours": HOURS,
                "sky_strength": sky_strength,
                "resolution": SIZE,
                "samples": scene.cycles.samples,
                "fixture_direct_samples": 2048,
                "fixture_indirect_samples": 4096,
                "build": "scripts/blender-buildings/bake_beaver_porch_surfaces.py",
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    bake(parser.parse_args(sys.argv[sys.argv.index("--") + 1 :]).out)
