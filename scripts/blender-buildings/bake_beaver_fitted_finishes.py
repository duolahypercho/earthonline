"""Measure incident room radiance and diffuse transfer on the fitted bowl and table in Cycles."""

import argparse
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bake_beaver_floor_gloss import ROOT, digest, survey
from beaver_floor_gloss import SOURCES

PHOTO = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"


def vertices(obj, graph):
    evaluated = obj.evaluated_get(graph)
    return np.array(
        [tuple(evaluated.matrix_world @ v.co) for v in evaluated.data.vertices]
    )


def exposed_linen(scene, cloth, graph, mat):
    # Both sides of the solidified, folded sheet reuse its weave UVs. Find
    # the exposed fibre at each chart sample, including where folds overlap.
    # A receiver buried in the back of the hem would record a black hole.
    mesh = cloth.data
    mesh.calc_loop_triangles()
    uv = mesh.uv_layers.active.data
    triangles = list(mesh.loop_triangles)
    uv_points = [(uv[l].uv.x, uv[l].uv.y, 0.0) for t in triangles for l in t.loops]
    uv_tree = BVHTree.FromPolygons(
        uv_points,
        [(3 * i, 3 * i + 1, 3 * i + 2) for i in range(len(triangles))],
        all_triangles=True,
    )
    evaluated = cloth.evaluated_get(graph)
    exposed = evaluated.data
    exposed.calc_loop_triangles()
    points = [evaluated.matrix_world @ v.co for v in exposed.vertices]
    surface = BVHTree.FromPolygons(
        points, [t.vertices[:] for t in exposed.loop_triangles], all_triangles=True
    )
    sheet = [cloth.matrix_world @ v.co for v in mesh.vertices]
    high = max(v.z for v in points) + 0.05
    verts = []
    faces = []
    down = Vector((0, 0, -1))
    for y in range(64):
        for x in range(64):
            u = max(0.0, min(1.0, ((x + 0.5) * 2 - 1.5) / 125))
            v = max(0.0, min(1.0, ((y + 0.5) * 2 - 1.5) / 125))
            _, _, i, _ = uv_tree.ray_cast(Vector((u, v, 1)), down)
            if i is None:
                _, _, i, _ = uv_tree.find_nearest(Vector((u, v, 0)))
            tri = triangles[i]
            aa, bb, cc = [np.asarray(uv[l].uv) for l in tri.loops]
            w1, w2 = np.linalg.solve(
                np.stack([bb - aa, cc - aa], axis=1), np.array([u, v]) - aa
            )
            a, b, c = [sheet[j] for j in tri.vertices]
            at = a + (b - a) * w1 + (c - a) * w2
            p, n, _, _ = surface.ray_cast(Vector((at.x, at.y, high)), down)
            if p is None:
                p, n, _, _ = surface.find_nearest(at)
            if n.z < 0:
                n = -n
            origin = p + n * 0.0007
            tangent = n.orthogonal().normalized() * 0.00008
            side = n.cross(tangent)
            start = len(verts)
            verts.extend(
                [
                    origin - tangent - side,
                    origin + tangent - side,
                    origin + tangent + side,
                    origin - tangent + side,
                ]
            )
            faces.append(tuple(range(start, start + 4)))
    result = bpy.data.meshes.new("Exposed flax receivers")
    result.from_pydata(verts, [], faces)
    result.update()
    result.materials.append(mat)
    chart = result.uv_layers.new(name="Finish light")
    for i, poly in enumerate(result.polygons):
        x = i % 64
        y = i // 64
        for l, (a, b) in zip(poly.loop_indices, ((0, 0), (1, 0), (1, 1), (0, 1))):
            chart.data[l].uv = ((256 + 2 * (x + a)) / 384, 2 * (y + b) / 128)
    obj = bpy.data.objects.new(result.name, result)
    scene.collection.objects.link(obj)
    return obj


def light_receiver(scene, bowl, boards, graph, parcel):
    # The original meshes still shade and reflect into the receivers. A small
    # normal offset prevents coplanar rays from striking the emitting face.
    mat = bpy.data.materials.new("Unit white finish receiver")
    mat.use_nodes = True
    tree = mat.node_tree
    bs = tree.nodes.new("ShaderNodeBsdfDiffuse")
    bs.inputs[0].default_value = (1, 1, 1, 1)
    tree.links.new(bs.outputs[0], tree.nodes["Material Output"].inputs["Surface"])
    texture = tree.nodes.new("ShaderNodeTexImage")
    tree.nodes.active = texture

    def chart_receiver(obj, mesh, x):
        target = bpy.data.objects.new(obj.name + " light receiver", mesh)
        scene.collection.objects.link(target)
        target.matrix_world = obj.matrix_world
        normals = [v.normal.copy() for v in mesh.vertices]
        for v, normal in zip(mesh.vertices, normals):
            v.co += normal * 0.0005
        uv = mesh.uv_layers.active
        uv.name = "Finish light"
        for loop in uv.data:
            u, v = loop.uv
            loop.uv = ((x + 1.5 + 125 * u) / 384, (1.5 + 125 * v) / 128)
        mesh.materials.clear()
        mesh.materials.append(mat)
        for face in mesh.polygons:
            face.material_index = 0
        return target

    receiver = chart_receiver(
        bowl, bpy.data.meshes.new_from_object(bowl.evaluated_get(graph)), 0
    )
    cloth = scene.objects["Folded striped kitchen linen"]
    # The source sheet has one non-overlapping UV chart across all three folds.
    # Its solidify modifier duplicates these UVs on the interior of the hem.
    linen = exposed_linen(scene, cloth, graph, mat)
    peaks = np.concatenate([vertices(o, graph) for o in boards])
    upper = float(peaks[:, 2].max())
    plan = (
        np.stack([peaks[:, 0], peaks[:, 2], -peaks[:, 1], np.ones(len(peaks))], axis=1)
        @ parcel.T
    )
    lo, hi = plan[:, [0, 2]].min(0), plan[:, [0, 2]].max(0)
    inverse = np.linalg.inv(parcel)

    def source(x, z, height):
        x, y, z, _ = inverse @ [x, height, z, 1]
        return Vector((x, -z, y))

    corners = [(0, 1), (1, 1), (1, 0), (0, 0)]
    plane = bpy.data.meshes.new("Tabletop light chart")
    plane.from_pydata(
        [source(*(lo + np.array(i) * (hi - lo)), upper + 0.0005) for i in corners],
        [],
        [(0, 1, 2, 3)],
    )
    plane.materials.append(mat)
    chart = plane.uv_layers.new(name="Finish light")
    for i, (u, v) in enumerate(corners):
        chart.data[i].uv = ((129.5 + 125 * u) / 384, (1.5 + 125 * (1 - v)) / 128)
    top = bpy.data.objects.new(plane.name, plane)
    scene.collection.objects.link(top)
    bpy.ops.object.select_all(action="DESELECT")
    for o in (top, linen, receiver):
        o.select_set(True)
    bpy.context.view_layer.objects.active = receiver
    bpy.ops.object.join()
    for kind in (
        "camera",
        "diffuse",
        "glossy",
        "transmission",
        "volume_scatter",
        "shadow",
    ):
        setattr(receiver, "visible_" + kind, False)
    return (
        receiver,
        texture,
        [*lo, *hi],
        [
            source(*(lo + np.array(i) * (hi - lo)), upper + 0.018)
            for i in ((0.2, 0.2), (0.8, 0.2), (0.2, 0.8), (0.8, 0.8))
        ],
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("portable", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--panoramas",
        type=Path,
        help="Reuse verified radiance from the identical geometry and probe positions",
    )
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    out = args.output.resolve()
    if out.is_relative_to(ROOT):
        parser.error("Keep raw measurements outside the repository")
    portable = json.loads(
        (args.portable / "portable-dining.provenance.json").read_text()
    )
    calibration = json.loads(
        (PHOTO / "fitted-dining/dining-floor-light.provenance.json").read_text()
    )
    if (
        digest(Path(bpy.data.filepath)) != portable["source_scene_sha256"]
        or portable["source_scene_sha256"] != calibration["source_scene"]["sha256"]
    ):
        raise ValueError(
            "The portable furniture and floor lighting must match the open scene"
        )
    out.mkdir(parents=True, exist_ok=True)
    scene = bpy.data.scenes["Dining · site daylight"]
    bpy.context.window.scene = scene
    scene.frame_set(3)
    # A separate unit sun measures only its indirect return. The live sun and
    # shadow maps provide direct sunlight at the actual game time.
    sky = next(n for n in scene.world.node_tree.nodes if n.type == "TEX_SKY")
    sky.sun_disc = False
    graph = bpy.context.evaluated_depsgraph_get()
    bowl = scene.objects["Hand-thrown mixing bowl"]
    rail = scene.objects["Continuous moulded walnut rail and volute"]
    end = scene.objects["Carved centre of the walnut volute"]
    boards = [
        o
        for o in scene.objects
        if o.type == "MESH"
        and o.active_material
        and o.active_material.name.startswith("Dining tabletop · plank")
    ]
    if len(boards) != 4:
        raise ValueError("Expected the four fitted dining boards")
    parcel = np.array(portable["placement"]["parcel_from_assembled_gltf"])
    rail_vertices = vertices(rail, graph)
    low = vertices(end, graph).mean(0)
    rail_points = [Vector(low)]
    for fraction in (0.50, 0.94):
        z = low[2] + fraction * (rail_vertices[:, 2].max() - low[2])
        rail_points.append(
            Vector(rail_vertices[np.abs(rail_vertices[:, 2] - z) < 0.07].mean(0))
        )
    if not rail_points[0].z < rail_points[1].z < rail_points[2].z:
        raise ValueError("Rail probes must follow increasing physical height")
    bowl_points = vertices(bowl, graph)
    midpoint = (bowl_points.min(0) + bowl_points.max(0)) * 0.5
    receiver, texture, bounds, table_points = light_receiver(
        scene, bowl, boards, graph, parcel
    )
    points = [*rail_points, Vector(midpoint), *table_points]
    mesh_groups = [(rail, end)] * 3 + [(bowl,)] + [tuple(boards)] * 4
    original = {o: o.visible_camera for group in mesh_groups for o in group}

    def visibility(index=None):
        hidden = mesh_groups[index] if index is not None else ()
        for o, value in original.items():
            o.visible_camera = value and o not in hidden

    transfer = np.empty((8, 128, 384, 3), np.float32)
    incident = np.empty((7, 8, 64, 128, 3), np.float32)

    def bake_transfer(row, passes):
        receiver.hide_render = False
        receiver.visible_camera = True
        visibility()
        bpy.context.view_layer.objects.active = receiver
        receiver.select_set(True)
        bpy.context.view_layer.update()
        scene.cycles.samples = 1536
        image = bpy.data.images.new(
            "Diffuse transfer", 384, 128, alpha=False, float_buffer=True
        )
        image.colorspace_settings.name = "Non-Color"
        texture.image = image
        bpy.ops.object.bake(
            type="DIFFUSE",
            pass_filter=passes,
            target="IMAGE_TEXTURES",
            margin=1,
            use_clear=True,
        )
        values = np.empty(384 * 128 * 4, np.float32)
        image.pixels.foreach_get(values)
        bpy.data.images.remove(image)
        # PNG row zero is the top of the Blender UV chart.
        field = values.reshape(128, 384, 4)[::-1, :, :3].copy()
        if row in (0, 7):
            field *= calibration["fixture_units_per_source_unit"]
        elif row == 2:
            field /= np.asarray(calibration["source_key_rgb"])
        if not np.isfinite(field).all() or field.max() <= 0:
            raise ValueError(
                f"No surface light from {(SOURCES + ('pendant-direct',))[row]}"
            )
        transfer[row] = field
        print(
            "Measured surface",
            (SOURCES + ("pendant-direct",))[row],
            np.round(field.mean(axis=(0, 1)), 4),
            flush=True,
        )
        receiver.hide_render = True
        scene.cycles.samples = 64

    def diffuse(row):
        bake_transfer(row, {"INDIRECT"} if row >= 3 else {"DIRECT", "INDIRECT"})
        if row == 0:
            others = [
                o
                for o in scene.objects
                if o.type == "LIGHT"
                and o.data.type in ("POINT", "SPOT", "AREA")
                and not o.hide_render
                and o.name != "Dining pendant · opal globe"
            ]
            for o in others:
                o.hide_render = True
            bake_transfer(7, {"DIRECT"})
            for o in others:
                o.hide_render = False

    def store(row, index, field):
        incident[row, index] = field

    # Keep primary-ray panoramas and UV bakes in separate Cycles sessions:
    # the panorama session caches the hidden receiver in its acceleration structure.
    names = [[o.name for o in group] for group in mesh_groups]
    scene_name = scene.name
    packed = bpy.data.filepath
    survey(scene, None, calibration, store, points=[], source_ready=diffuse)
    if args.panoramas:
        prior = json.loads(args.panoramas.with_suffix(".provenance.json").read_text())
        if (
            digest(args.panoramas) != prior["sha256"]
            or prior["source_scene"] != calibration["source_scene"]
            or prior["geometry_sha256"] != portable["parts"]["fitted-room"]["sha256"]
            or prior["sources"] != list(SOURCES)
            or not np.allclose(prior["probe_blender_xyz"], points, rtol=0, atol=1e-6)
        ):
            raise ValueError("Panoramas belong to a different arrangement")
        with np.load(args.panoramas, allow_pickle=False) as previous:
            incident[:] = previous["incident"]
    else:
        bpy.ops.wm.open_mainfile(filepath=packed)
        scene = bpy.data.scenes[scene_name]
        bpy.context.window.scene = scene
        scene.frame_set(3)
        next(
            n for n in scene.world.node_tree.nodes if n.type == "TEX_SKY"
        ).sun_disc = False
        mesh_groups = [[scene.objects[n] for n in group] for group in names]
        original = {o: o.visible_camera for group in mesh_groups for o in group}
        survey(scene, None, calibration, store, points=points, probe_ready=visibility)
    raw = out / "fitted-finish-survey.npz"
    np.savez_compressed(raw, incident=incident, transfer=transfer)
    info = {
        "file": raw.name,
        "sha256": digest(raw),
        "source_scene": calibration["source_scene"],
        "house_sha256": calibration["house_sha256"],
        "geometry_sha256": portable["parts"]["fitted-room"]["sha256"],
        "probe_blender_xyz": [list(p) for p in points],
        "table_parcel_bounds": bounds,
        "sources": SOURCES,
        "samples": {"panorama": 64, "diffuse": 1536},
        "build": "scripts/blender-buildings/bake_beaver_fitted_finishes.py",
        "license": "Project-authored lighting data",
        "attribution": "Earth Online",
    }
    raw.with_suffix(".provenance.json").write_text(json.dumps(info, indent=2) + "\n")


if __name__ == "__main__":
    main()
