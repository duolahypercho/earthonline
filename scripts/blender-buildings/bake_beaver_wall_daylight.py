"""Measure exterior diffuse daylight on the two returns, street, yard and portal in Cycles.

Run on the registered site-daylight house with an external output directory.
The receiver is hidden in the editable file. The original plaster, neighbor,
doors, foliage and glazing remain in all secondary rays.
"""

import argparse
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).resolve().parent))
from survey_beaver_front_daylight import HOURS, PHOTO, ROOT, SITE_SCENE, SUN, digest

SIZE = 64
C = 5.955
LABELS = (
    "Narrow side passage",
    "Opposite side passage",
    "Street wall",
    "Rear wall",
    "Round stair bay",
    "Entry arch face",
    "Entry east reveal",
    "Entry west reveal",
    "Street gable",
    "Upper casement backing",
    "Arcade moulding face",
    "First arch intrados",
    "Middle arch intrados",
    "Third arch intrados",
    "Arcade sill underside",
    "Porch arch intrados",
)


def plaster_objects(scene):
    return [
        ob
        for ob in scene.objects
        if ob.type == "MESH"
        and not ob.hide_render
        and any(
            name in ob.name
            for name in (
                " - lime-plaster@",
                " - painted-stucco@",
                " - float-finish-stucco@",
                " - painted-white@",
            )
        )
    ]


def arcade_surfaces(scene, parcel):
    verts, faces = [], []
    for ob in plaster_objects(scene):
        if not ob.name.startswith(("upper-arched-loggia -", "workroom-storey -")):
            continue
        start = len(verts)
        for v in ob.data.vertices:
            x, y, z = ob.matrix_world @ v.co
            verts.append((parcel @ np.array((x, z, -y, 1.0)))[:3])
        ob.data.calc_loop_triangles()
        for tri in ob.data.loop_triangles:
            indices = [start + j for j in tri.vertices]
            centre = np.mean([verts[j] for j in indices], axis=0)
            if (
                0.4 < centre[0] < 4.85
                and 7.18 < centre[1] < 9.75
                and -0.55 < centre[2] < -0.12
            ):
                faces.append(indices)
    return BVHTree.FromPolygons(verts, faces, all_triangles=True)


def receiver(scene, parcel):
    # The first four rectangles are on the exterior face of the plaster.
    # The remaining charts follow the stair bay, portal, upper arcade and gable.
    # Offsets of 12 mm keep the neutral receivers clear of the finish.
    points, faces, tex = [], [], []
    inv = np.linalg.inv(parcel)

    def point(col, u, v):
        depth = 13 * u
        height = -0.16 + 11.16 * v
        if col == 0:
            return [-0.012, height, depth]
        if col == 1:
            return [7.012, height, depth]
        if col == 2:
            return [7 * u, height, -0.068]
        if col == 3:
            return [7 * u, height, 13.060]
        if col == 4:
            angle = (u - 0.5) * math.pi
            return [
                C + 1.012 * math.sin(angle),
                2.98 + 7.20 * v,
                0.04 - 0.912 * math.cos(angle),
            ]
        if col == 5:
            return [4.95 + 2.01 * u, 2.92 * v, -0.862]
        if col in (6, 7):
            return [5.172 if col == 6 else 6.738, 2.92 * v, -0.85 + 1.27 * u]
        if col == 8:
            return [-0.15 + 7.30 * u, 9.96 + 1.04 * v, -0.442]
        if col == 9:
            return [0.52 + 4.24 * u, 7.32 + 2.25 * v, 0.396]
        if col == 10:
            return [0.52 + 4.24 * u, 7.24 + 2.40 * v, -0.252]
        if 11 <= col <= 13:
            angle = u * math.pi
            spring = 20 / 3 + 0.72 + (1.77 - 0.86) * 2.06 / 1.69
            rise = 0.72 * 2.06 / 1.69
            return [
                1.315 + 1.31 * (col - 11) - 0.585 * math.cos(angle),
                spring + (rise - 0.012) * math.sin(angle),
                -0.235 + 0.291 * v,
            ]
        if col == 14:
            return [0.55 + 4.15 * u, 7.247, -0.445 + 0.545 * v]
        angle = u * math.pi
        return [
            5.955 - 0.785 * math.cos(angle),
            1.92 + 0.708 * math.sin(angle),
            -0.85 + 1.27 * v,
        ]

    # Project each cell of the moulding charts onto the actual front or bottom.
    # A plane behind the projecting sill would be trapped inside its masonry.
    arcade = arcade_surfaces(scene, parcel)
    for col in range(len(LABELS)):
        if col in (10, 14):
            for y in range(SIZE):
                for x in range(SIZE):
                    u, v = (x + 0.5) / SIZE, (y + 0.5) / SIZE
                    at = point(col, u, v)
                    if col == 10:
                        hit = arcade.ray_cast(
                            Vector((at[0], at[1], -0.7)), Vector((0, 0, 1)), 0.6
                        )[0]
                        depth = hit.z - 0.012 if hit is not None else at[2]
                    else:
                        hit = arcade.ray_cast(
                            Vector((at[0], 7.18, at[2])), Vector((0, 1, 0)), 0.3
                        )[0]
                        depth = hit.y - 0.012 if hit is not None else at[1]
                    cell = []
                    corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)]
                    if col == 10:
                        corners.reverse()
                    for ix, iy in corners:
                        a, b = ix / SIZE, iy / SIZE
                        pos = point(col, a, b)
                        pos[2 if col == 10 else 1] = depth
                        px, py, pz, _ = inv @ np.array([*pos, 1])
                        cell.append(len(points))
                        points.append((px, -pz, py))
                        tex.append(((col + a) / len(LABELS), b))
                    faces.append(cell)
            continue
        segments = 48 if col in (4, 11, 12, 13, 15) else 1
        for i in range(segments):
            uv = [
                (i / segments, 0),
                ((i + 1) / segments, 0),
                ((i + 1) / segments, 1),
                (i / segments, 1),
            ]
            # In parcel XYZ, x is across, y up, z toward the back.
            first = len(points)
            for u, v in uv:
                x, y, z, _ = inv @ np.array([*point(col, u, v), 1])
                points.append((x, -z, y))
                tex.append(((col + u) / len(LABELS), v))
            ids = list(range(first, first + 4))
            # Check winding in Blender space against the target world outward normal.
            if col in (0, 1, 6, 7):
                aim = (-1 if col in (0, 7) else 1, 0, 0)
            elif col == 4:
                angle = ((i + 0.5) / segments - 0.5) * math.pi
                aim = (math.sin(angle), 0, -math.cos(angle))
            elif col in (11, 12, 13, 15):
                angle = ((i + 0.5) / segments) * math.pi
                aim = (math.cos(angle), -math.sin(angle), 0)
            elif col == 14:
                aim = (0, -1, 0)
            else:
                aim = (0, 0, 1 if col == 3 else -1)
            # World normal uses the transpose of parcel-from-source.
            ax, ay, az = parcel[:3, :3].T @ np.array(aim)
            face_n = (Vector(points[first + 1]) - Vector(points[first])).cross(
                Vector(points[first + 2]) - Vector(points[first])
            )
            if face_n.dot(Vector((ax, -az, ay))) < 0:
                ids.reverse()
            faces.append(ids)
    mesh = bpy.data.meshes.new("Exterior plaster receiving surfaces")
    mesh.from_pydata(points, [], faces)
    mesh.update()
    obj = bpy.data.objects.new("Exterior daylight · neutral wall receivers", mesh)
    scene.collection.objects.link(obj)
    for ray in ("diffuse", "glossy", "transmission", "volume_scatter", "shadow"):
        setattr(obj, "visible_" + ray, False)
    uv = mesh.uv_layers.new(name="Physical wall charts")
    for loop in mesh.loops:
        uv.data[loop.index].uv = tex[loop.vertex_index]
    mat = bpy.data.materials.new("White diffuse · wall irradiance")
    mat.use_nodes = True
    nodes = mat.node_tree.nodes
    shader = nodes.new("ShaderNodeBsdfDiffuse")
    shader.inputs["Color"].default_value = (1, 1, 1, 1)
    mat.node_tree.links.new(
        shader.outputs[0], nodes["Material Output"].inputs["Surface"]
    )
    mesh.materials.append(mat)
    tex_node = nodes.new("ShaderNodeTexImage")
    nodes.active = tex_node
    obj["Charts"] = " | ".join(LABELS)
    obj.hide_render = True
    return obj, tex_node


def run(out):
    out = out.resolve()
    if out.is_relative_to(ROOT):
        raise ValueError("Use an exterior source directory outside the repository")
    src = Path(bpy.data.filepath)
    reference = json.loads((PHOTO / "front-daylight.provenance.json").read_text())
    if digest(src) != reference["source_scene_sha256"]:
        raise ValueError("Expected the registered site-daylight house")
    info = json.loads((src.parent / "front-daylight.provenance.json").read_text())
    parent = json.loads(
        (src.parent.parent / "street-garden/street-garden.provenance.json").read_text()
    )
    if parent["sha256"] != info["parent_scene_sha256"]:
        raise ValueError("Parcel and solar study differ")
    out.mkdir(parents=True, exist_ok=True)
    scene = bpy.data.scenes[SITE_SCENE]
    bpy.context.window.scene = scene
    obj, target = receiver(scene, np.asarray(parent["parcel_from_assembled_gltf"]))
    scene.frame_set(34)
    bpy.context.preferences.filepaths.save_version = 0
    dst = out / "21 Beaver - Plaster and daylight.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(dst), compress=True)
    original_world = scene.world
    unit = bpy.data.worlds.new("Uniform wall survey sky")
    unit.use_nodes = True
    bg = unit.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = (1, 1, 1, 1)
    prefs = bpy.context.preferences.addons["cycles"].preferences
    if sys.platform == "darwin":
        prefs.compute_device_type = "METAL"
        for dev in prefs.get_devices_for_type("METAL"):
            dev.use = dev.type == "METAL"
        scene.cycles.device = "GPU"
    scene.cycles.samples = 256
    scene.cycles.seed = 671
    scene.cycles.use_adaptive_sampling = False
    scene.render.engine = "CYCLES"
    scene.render.use_persistent_data = True
    original_lights = [
        o for o in scene.objects if o.type == "LIGHT" and not o.hide_render
    ]
    sun = scene.objects[SUN]
    obj.hide_render = False
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj

    # Capture fixtures separately before muting emissive surfaces for the solar survey.
    def bake(label, passes):
        image = bpy.data.images.new(
            label, len(LABELS) * SIZE, SIZE, float_buffer=True, alpha=False
        )
        image.colorspace_settings.name = "Non-Color"
        target.image = image
        obj.active_material.node_tree.nodes.active = target
        bpy.ops.object.bake(
            type="DIFFUSE",
            pass_filter=passes,
            target="IMAGE_TEXTURES",
            margin=0,
            use_clear=True,
        )
        values = np.empty((SIZE, len(LABELS) * SIZE, 4), np.float32)
        image.pixels.foreach_get(values.ravel())
        bpy.data.images.remove(image)
        rgb = values[:, :, :3].copy()
        if not np.isfinite(rgb).all() or np.any(rgb < 0):
            raise ValueError(label)
        print(label, float(rgb.max()), flush=True)
        return rgb

    scene.world = unit
    bg.inputs["Strength"].default_value = 0
    sun.hide_render = True
    # Project the original diffuse pigment onto the same charts. Valid alpha
    # distinguishes a projected surface from a window opening or an empty cell.
    original_plaster = plaster_objects(scene)
    for ob in original_plaster:
        ob.select_set(True)
    pigment_size = SIZE * 2
    pigment_image = bpy.data.images.new(
        "Source plaster reflectance",
        len(LABELS) * pigment_size,
        pigment_size,
        float_buffer=True,
        alpha=True,
    )
    pigment_image.colorspace_settings.name = "Non-Color"
    target.image = pigment_image
    obj.active_material.node_tree.nodes.active = target
    bpy.ops.object.bake(
        type="DIFFUSE",
        pass_filter={"COLOR"},
        target="IMAGE_TEXTURES",
        margin=0,
        use_selected_to_active=True,
        cage_extrusion=0.035,
        max_ray_distance=0.10,
        use_clear=True,
    )
    pigment = np.empty((pigment_size, len(LABELS) * pigment_size, 4), np.float32)
    pigment_image.pixels.foreach_get(pigment.ravel())
    bpy.data.images.remove(pigment_image)
    for ob in original_plaster:
        ob.select_set(False)
    fixture = bake("Existing practical lights", {"DIRECT", "INDIRECT"})
    for material in bpy.data.materials:
        if not material.node_tree:
            continue
        for node in material.node_tree.nodes:
            socket = {
                "BSDF_PRINCIPLED": "Emission Strength",
                "EMISSION": "Strength",
            }.get(node.type)
            if socket:
                inp = node.inputs[socket]
                for link in list(inp.links):
                    material.node_tree.links.remove(link)
                inp.default_value = 0
    for ob in original_lights:
        ob.hide_render = True
    rows = []
    for hour in HOURS:
        scene.world = original_world
        scene.frame_set(2 * hour)
        sky_node = next(
            n for n in original_world.node_tree.nodes if n.type == "TEX_SKY"
        )
        print(
            "Sky direction",
            hour,
            sky_node.sun_rotation,
            sky_node.sun_elevation,
            flush=True,
        )
        sun.hide_render = True
        sky = bake(f"Sky · {hour:02}:00", {"DIRECT", "INDIRECT"})
        scene.world = unit
        sun.hide_render = False
        indirect = bake(f"Sun bounce · {hour:02}:00", {"INDIRECT"})
        rows.append(sky + indirect)
    sun.hide_render = True
    bg.inputs["Strength"].default_value = 1
    rows.append(bake("Overcast transfer", {"DIRECT", "INDIRECT"}))
    rows.append(fixture)
    raw = out / "plaster-irradiance.npz"
    np.savez_compressed(raw, radiance=np.array(rows, dtype=np.float32), pigment=pigment)
    proof = {
        "scene": dst.name,
        "scene_sha256": digest(dst),
        "parent_scene_sha256": digest(src),
        "radiance": raw.name,
        "radiance_sha256": digest(raw),
        "charts": LABELS,
        "hours": HOURS,
        "canonical_source_to_game": info["canonical_source_to_game"],
        "samples": scene.cycles.samples,
        "source_exposure_ev": scene.view_settings.exposure,
        "build": "scripts/blender-buildings/bake_beaver_wall_daylight.py",
    }
    (out / "plaster-irradiance.provenance.json").write_text(
        json.dumps(proof, indent=2) + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    run(parser.parse_args(sys.argv[sys.argv.index("--") + 1 :]).out)
