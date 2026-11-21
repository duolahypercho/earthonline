"""Measure the real dining shade, tile wall and work surface in the fitted Cycles scene."""

import argparse
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bake_beaver_fitted_finishes import PHOTO, vertices
from bake_beaver_floor_gloss import ROOT, digest, survey
from bake_beaver_kitchen_board import board_uv
from beaver_floor_gloss import SOURCES

WIDTH, HEIGHT = 640, 64


def recessed_joints(scene, material, graph):
    """Place each white receiver on visible mortar, behind the tile's glazed arris."""
    tile = scene.objects["Hand-set running bond tile"]
    # Each un-bevelled source tile is a separate eight-vertex solid.
    solids = np.array(
        [list(tile.matrix_world @ v.co) for v in tile.data.vertices]
    ).reshape(-1, 8, 3)
    lo, hi = solids.min(axis=1), solids.max(axis=1)
    wall_low, wall_high = lo.min(0), hi.max(0)
    bed = vertices(scene.objects["Lime grout bed"], graph)[:, 0].max() + 0.00012
    rows = np.unique(lo[:, 2])
    tops = np.array([hi[lo[:, 2] == z, 2].max() for z in rows])
    horizontal = (rows[1:] + tops[:-1]) * 0.5
    verts, faces, coords = [], [], []
    for orientation in range(2):
        for j in range(32):
            v = np.clip((j - 1) / 29, 0, 1)
            z = wall_low[2] + v * (wall_high[2] - wall_low[2])
            band = int(
                np.clip(np.searchsorted(rows, z, side="right") - 1, 0, len(rows) - 1)
            )
            same = lo[:, 2] == rows[band]
            along = np.argsort(lo[same, 1])
            joins = (lo[same, 1][along][1:] + hi[same, 1][along][:-1]) * 0.5
            for i in range(64):
                u = np.clip((i - 1) / 61, 0, 1)
                y = wall_low[1] + u * (wall_high[1] - wall_low[1])
                cy, cz = (
                    (y, horizontal[np.abs(horizontal - z).argmin()])
                    if orientation == 0
                    else (joins[np.abs(joins - y).argmin()], z)
                )
                base = len(verts)
                for du, dv in ((0, 0), (1, 0), (1, 1), (0, 1)):
                    verts.append(
                        (bed, cy + (du - 0.5) * 0.00025, cz + (dv - 0.5) * 0.00025)
                    )
                    coords.append(
                        ((576 + i + du) / WIDTH, (32 * orientation + j + dv) / HEIGHT)
                    )
                faces.append(tuple(range(base, base + 4)))
    mesh = bpy.data.meshes.new("Exposed kitchen mortar")
    mesh.from_pydata(verts, [], faces)
    mesh.materials.append(material)
    uv = mesh.uv_layers.new(name="Fixture light")
    for loop in mesh.loops:
        uv.data[loop.index].uv = coords[loop.vertex_index]
    obj = bpy.data.objects.new(mesh.name, mesh)
    scene.collection.objects.link(obj)
    return obj


def receiver(scene, shade, wall, counter, graph):
    material = bpy.data.materials.new("White fixture light receiver")
    material.use_nodes = True
    tree = material.node_tree
    diffuse = tree.nodes.new("ShaderNodeBsdfDiffuse")
    diffuse.inputs["Color"].default_value = (1, 1, 1, 1)
    tree.links.new(diffuse.outputs[0], tree.nodes["Material Output"].inputs["Surface"])
    texture = tree.nodes.new("ShaderNodeTexImage")
    tree.nodes.active = texture
    mesh = bpy.data.meshes.new_from_object(shade.evaluated_get(graph))
    target = bpy.data.objects.new("Outside and inside of the spun shade", mesh)
    scene.collection.objects.link(target)
    target.matrix_world = shade.matrix_world
    normals = [v.normal.copy() for v in mesh.vertices]
    for vertex, normal in zip(mesh.vertices, normals):
        # The rolled rim is only millimetres thick.
        vertex.co += 0.0002 * normal
    mesh.materials.clear()
    mesh.materials.append(material)
    for face in mesh.polygons:
        face.material_index = 0
    uv = mesh.uv_layers.active
    uv.name = "Fixture light"
    for loop in uv.data:
        u, v = loop.uv
        loop.uv = ((1.5 + 125 * u) / WIDTH, (1.5 + 61 * v) / HEIGHT)

    points = vertices(wall, graph)
    lo, hi = points.min(axis=0), points.max(axis=0)
    edge = vertices(counter, graph)
    back, front = edge[:, 0].min(), edge[:, 0].max()
    top = edge[:, 2].max()

    # Separate planes have the real room and installed props around them, but
    # cast no light or shadows themselves. Both run the full length of the tiles.
    def plane(name, offset, width, point):
        corners = [(0, 0), (1, 0), (1, 1), (0, 1)]
        m = bpy.data.meshes.new(name)
        m.from_pydata([point(u, v) for u, v in corners], [], [(0, 1, 2, 3)])
        m.materials.append(material)
        layer = m.uv_layers.new(name="Fixture light")
        for i, (u, v) in enumerate(corners):
            layer.data[i].uv = (
                (offset + 1.5 + (width - 3) * u) / WIDTH,
                (1.5 + 61 * v) / HEIGHT,
            )
        obj = bpy.data.objects.new(name, m)
        scene.collection.objects.link(obj)
        return obj

    wall_plane = plane(
        "Light falling on the tiles",
        128,
        256,
        lambda u, v: (
            hi[0] + 0.0005,
            lo[1] + u * (hi[1] - lo[1]),
            lo[2] + v * (hi[2] - lo[2]),
        ),
    )
    # Increasing Y crossed with decreasing X faces the ceiling.
    work_plane = plane(
        "Light falling on the slab",
        384,
        128,
        lambda u, v: (
            front + v * (back - front),
            lo[1] + u * (hi[1] - lo[1]),
            top + 0.0007,
        ),
    )
    original = next(
        o for o in scene.objects["chopping-board"].children if o.type == "MESH"
    )
    wood = bpy.data.objects.new("Oak board illumination", original.data.copy())
    scene.collection.objects.link(wood)
    wood.matrix_world = original.matrix_world.copy()
    uv, _, _ = board_uv(wood.data)
    uv.name = "Fixture light"
    for loop in uv.data:
        u, v = loop.uv
        loop.uv = ((513.5 + 61 * u) / WIDTH, (1.5 + 61 * v) / HEIGHT)
    for v, n in zip(wood.data.vertices, [v.normal.copy() for v in wood.data.vertices]):
        v.co += 0.00045 * n
    wood.data.materials.clear()
    wood.data.materials.append(material)
    mortar = recessed_joints(scene, material, graph)
    bpy.ops.object.select_all(action="DESELECT")
    for obj in (target, wall_plane, work_plane, wood, mortar):
        obj.select_set(True)
    bpy.context.view_layer.objects.active = target
    bpy.ops.object.join()
    for kind in (
        "camera",
        "diffuse",
        "glossy",
        "transmission",
        "volume_scatter",
        "shadow",
    ):
        setattr(target, "visible_" + kind, False)
    return target, texture, lo, hi, back, front, top


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("portable", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    if args.output.resolve().is_relative_to(ROOT):
        parser.error("Keep raw measurements outside the repository")
    args.output.mkdir(parents=True, exist_ok=True)
    portable = json.loads(
        (args.portable / "portable-dining.provenance.json").read_text()
    )
    calibration = json.loads(
        (PHOTO / "fitted-dining/dining-floor-light.provenance.json").read_text()
    )
    packed = bpy.data.filepath
    if (
        digest(Path(packed)) != portable["source_scene_sha256"]
        or portable["source_scene_sha256"] != calibration["source_scene"]["sha256"]
    ):
        raise ValueError("The house, fixture calibration and portable scene must match")

    def open_scene():
        scene = bpy.data.scenes["Dining · site daylight"]
        bpy.context.window.scene = scene
        scene.frame_set(3)
        next(
            n for n in scene.world.node_tree.nodes if n.type == "TEX_SKY"
        ).sun_disc = False
        return scene

    scene = open_scene()
    graph = bpy.context.evaluated_depsgraph_get()
    shade = scene.objects["Spun enamel shade and rolled lip"]
    shade_pts = vertices(shade, graph)
    bottom, crown = shade_pts.min(0), shade_pts.max(0)
    center = (bottom + crown) / 2
    target, texture, lo, hi, back, front, top = receiver(
        scene,
        shade,
        scene.objects["Hand-set running bond tile"],
        scene.objects["Honed worktop with drip edge"],
        graph,
    )
    probes = [
        [center[0] + (crown[0] - center[0]) * 0.65, center[1], crown[2] + 0.025],
        [center[0] + (crown[0] - center[0]) * 0.75, center[1], bottom[2] - 0.008],
    ]
    # Three stations along the installed run; two rows distinguish the wall from
    # the outer counter edge. A helper slice below selects the closest row.
    for depth, height in (
        (hi[0] + 0.025, (lo[2] + hi[2]) * 0.5),
        (front + 0.035, top + 0.16),
    ):
        for u in (0.125, 0.5, 0.875):
            probes.append([depth, lo[1] + u * (hi[1] - lo[1]), height])

    transfer = np.empty((8, HEIGHT, WIDTH, 3), np.float32)

    def bake(row, passes):
        target.hide_render = False
        target.visible_camera = True
        target.select_set(True)
        bpy.context.view_layer.objects.active = target
        bpy.context.view_layer.update()
        scene.cycles.samples = 1024
        image = bpy.data.images.new(
            "Fixture irradiance", WIDTH, HEIGHT, alpha=False, float_buffer=True
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
        values = np.empty((HEIGHT, WIDTH, 4), np.float32)
        image.pixels.foreach_get(values.ravel())
        bpy.data.images.remove(image)
        field = values[::-1, :, :3].copy()
        if row in (0, 7):
            field *= calibration["fixture_units_per_source_unit"]
        elif row == 2:
            field /= np.asarray(calibration["source_key_rgb"])
        if not np.isfinite(field).all() or field.max() <= 0:
            raise ValueError(f"Empty kitchen lighting case {row}")
        transfer[row] = field
        print("Measured kitchen surfaces", row, field.max(axis=(0, 1)), flush=True)
        scene.cycles.samples = 64
        target.hide_render = True

    spots = [
        o
        for o in scene.objects
        if o.type == "LIGHT" and o.name.startswith("Range hood task light")
    ]
    if len(spots) != 2:
        raise ValueError("Expected the two lamps installed under the hood")

    def source(row):
        bake(row, {"INDIRECT"} if row >= 3 else {"DIRECT", "INDIRECT"})
        if row == 0:
            other = [
                o
                for o in scene.objects
                if o.type == "LIGHT" and not o.hide_render and o not in spots
            ]
            for obj in other:
                obj.hide_render = True
            bake(7, {"DIRECT"})
            for obj in other:
                obj.hide_render = False

    spot_positions = [list(o.matrix_world.translation) for o in spots]
    survey(scene, None, calibration, None, points=[], source_ready=source)
    bpy.ops.wm.open_mainfile(filepath=packed)
    scene = open_scene()
    incident = np.empty((7, 8, 64, 128, 3), np.float32)

    def store(row, index, rgb):
        incident[row, index] = rgb

    survey(scene, None, calibration, store, points=[Vector(p) for p in probes])

    # Project any parcel point onto the same metric axes as the two receivers.
    from_parcel = np.linalg.inv(
        np.array(portable["placement"]["parcel_from_assembled_gltf"])
    )
    along = -from_parcel[2].copy()
    along[3] -= lo[1]
    along /= hi[1] - lo[1]
    across = -from_parcel[0].copy()
    across[3] += front
    across /= front - back
    out = args.output / "kitchen-light-survey.npz"
    np.savez_compressed(out, transfer=transfer, incident=incident)
    notice = {
        "file": out.name,
        "sha256": digest(out),
        "source_scene": calibration["source_scene"],
        "house_sha256": calibration["house_sha256"],
        "geometry_sha256": portable["parts"]["fitted-room"]["sha256"],
        "sources": list(SOURCES),
        "probe_blender_xyz": probes,
        "spot_blender_xyz": spot_positions,
        "kitchen_frame": [
            along.tolist(),
            across.tolist(),
            [lo[2], hi[2], top, crown[2]],
        ],
        "samples": {"panorama": 64, "diffuse": 1024},
        "license": "Project-authored lighting data",
        "attribution": "Earth Online",
    }
    out.with_suffix(".provenance.json").write_text(json.dumps(notice, indent=2) + "\n")


if __name__ == "__main__":
    main()
