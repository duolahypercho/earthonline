"""Fit individual fired-clay treads and risers to the furnished Beaver entrance."""

import argparse
import gzip
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bake_beaver_porch_light import surfaces as scene_surfaces
from beaver_blender_scene import terrain_sampler
from beaver_entry import SETBACK
from beaver_lighting import contact_transfer
from beaver_street_scene import load_surface

NAMES = ("Stoop clay · ochre kiln", "Stoop clay · umber kiln")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def surfaces(folder):
    record = json.loads((folder / "surfaces.json").read_text())
    for f in record["files"]:
        if digest(folder / f["file"]) != f["sha256"]:
            raise ValueError(f["file"])
    mats = []
    for idx, name in enumerate(NAMES):
        mat = bpy.data.materials.new(name)
        mat.use_nodes = True
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        bs = nodes.get("Principled BSDF")
        for role in ("color", "normal", "orm"):
            path = folder / f"kiln-{idx + 1}-{role}.png"
            im = bpy.data.images.load(str(path.resolve()))
            im.colorspace_settings.name = "sRGB" if role == "color" else "Non-Color"
            im.pack()
            tex = nodes.new("ShaderNodeTexImage")
            tex.image = im
            if role == "color":
                links.new(tex.outputs["Color"], bs.inputs["Base Color"])
            elif role == "normal":
                node = nodes.new("ShaderNodeNormalMap")
                links.new(tex.outputs["Color"], node.inputs["Color"])
                links.new(node.outputs["Normal"], bs.inputs["Normal"])
            else:
                node = nodes.new("ShaderNodeSeparateColor")
                links.new(tex.outputs["Color"], node.inputs[0])
                links.new(node.outputs["Green"], bs.inputs["Roughness"])
        mats.append(mat)
    return mats


def tile(course, column, face, landing, matric, material, collection):
    """One mesh, including the turned front edge and the separately bedded riser."""
    rng = np.random.default_rng(20461 + 541 * (course + 1) + 29 * column)
    x0 = 5.15 + 0.32 * column + rng.uniform(0.0020, 0.0037)
    x1 = 5.15 + 0.32 * (1 + column) - rng.uniform(0.0020, 0.0037)
    front = 0 if landing else -0.33 * (course + 1)
    rear = SETBACK if landing else front + 0.33
    base = 0 if landing else -0.145 * course
    height = base + rng.uniform(0.0074, 0.0092)
    angle = rng.uniform(0, math.tau)
    nick = rng.uniform(0.08, 0.92)
    depth = (
        rng.uniform(0.0006, 0.0025)
        if (course + 2 * column) % 4
        else rng.uniform(0.003, 0.005)
    )
    xs = np.unique(np.r_[np.linspace(0, 1, 9), [nick - 0.031, nick, nick + 0.035]])
    points, faces, smooth, uvs = [], [], [], []

    def vert(x, y, z, u, v):
        # Parcel x/up/depth to assembled Blender metres.
        p = matric @ Vector((x, y, z, 1))
        points.append((p.x, -p.z, p.y))
        row, col = divmod(face % 4, 2)
        uvs.append(
            ((256 * col + 4.5 + 247 * u) / 512, 1 - (256 * row + 4.5 + 247 * v) / 512)
        )
        return len(points) - 1

    def quad(a, b, c, d, rounded=True):
        faces.append((a, b, c, d))
        smooth.append(rounded)

    profile = [(rear - 0.003, 0, 0)]
    for t in (0.27, 0.54, 0.78, 0.98):
        profile.append((rear - 0.003 - t * (rear - front - 0.008), 0, 0.86 * t))
    if landing:
        profile += [(front + 0.003, -0.002, 0.91), (front + 0.003, -0.017, 0.98)]
    else:
        # The rolled front projects 22 mm in assembled metres. The bottom
        # return leaves a narrow shadow joint above the separate upright.
        profile += [
            (front - 0.007, -0.0006, 0.88),
            (front - 0.0135, -0.003, 0.90),
            (front - 0.0175, -0.008, 0.915),
            (front - 0.018, -0.015, 0.938),
            (front - 0.015, -0.021, 0.96),
            (front - 0.008, -0.025, 0.97),
            (front + 0.003, -0.026, 0.985),
        ]
    rings = []
    for z, dy, v in profile:
        ring = []
        for u in xs:
            w = math.sin(math.pi * u)
            crown = 0.0012 * w * math.sin(math.pi * min(1, v / 0.86))
            twist = 0.00055 * math.sin(4.3 * u + angle) * (0.4 + 0.6 * v)
            tip = max(0, min(1, (v - 0.80) / 0.10))
            bite = depth * max(0, 1 - abs(u - nick) / 0.034) * tip
            zc = z + bite + 0.0011 * math.sin(7 * u + angle) * tip
            ring.append(
                vert(
                    x0 + u * (x1 - x0),
                    height + dy + crown + twist - bite * 0.56,
                    zc,
                    u,
                    v,
                )
            )
        if rings:
            for i in range(len(xs) - 1):
                quad(rings[-1][i], rings[-1][i + 1], ring[i + 1], ring[i])
        rings.append(ring)
    # Close the bedding sides where the edge is visible from the private walk.
    for col in (0, -1):
        top = [r[col] for r in rings]
        bottom = [
            vert(x0 if col == 0 else x1, base - 0.020, z, xs[col], min(1, v + 0.02))
            for z, _, v in profile
        ]
        for i in range(len(top) - 1):
            loop = (top[i], top[i + 1], bottom[i + 1], bottom[i])
            faces.append(loop if col == 0 else tuple(reversed(loop)))
            smooth.append(False)
    if not landing:
        # A small draft and eased upper edge give the upright its own break in gloss.
        prev = None
        for dy, offset, v in [
            (-0.026, -0.001, 0.05),
            (-0.029, -0.003, 0.07),
            (-0.085, -0.003, 0.37),
            (-0.140, -0.002, 0.70),
        ]:
            r = [vert(x0 + u * (x1 - x0), base + dy, front + offset, u, v) for u in xs]
            if prev:
                for i in range(len(xs) - 1):
                    quad(prev[i], prev[i + 1], r[i + 1], r[i], False)
            prev = r
    name = (
        f"Landing tile {column + 1:02d}"
        if landing
        else f"Tread {course + 1:02d} · tile {column + 1:02d}"
    )
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(points, [], faces)
    mesh.update()
    uv = mesh.uv_layers.new(name="Fired clay")
    mesh.materials.append(material)
    for poly, sm in zip(mesh.polygons, smooth):
        poly.use_smooth = sm
        for i in poly.loop_indices:
            uv.data[i].uv = uvs[mesh.loops[i].vertex_index]
    ob = bpy.data.objects.new(name, mesh)
    collection.objects.link(ob)
    ob["Tile face"] = face + 1
    return ob


def arch_bases(scene, group, inverse):
    # Both pier shells previously ended at the sill elevation above the sloping
    # garden. Continue the same vertical faces to the surveyed terrain.
    repo = Path(__file__).resolve().parents[2]
    with gzip.open(repo / "public/data/sf/sf-city.json.gz", "rt") as f:
        city = json.load(f)
    home = next(
        b for b in city["detailBuildings"] if b.get("addr") == "21 Beaver Street"
    )
    ring = [Vector(p) for p in zip(home["points"][::2], home["points"][1::2])]
    sample = terrain_sampler(repo)
    height_scale = load_surface(repo, home["centroid"])["height_scale"]
    datum = max(sample(p) for p in ring) * height_scale + 0.22
    wall = next(
        o
        for o in scene.objects
        if o.name.startswith("arched-entry-and-lantern - painted-stucco@")
    )
    basis = wall.matrix_world
    back = basis.inverted()
    parcel = inverse.inverted()

    def grade(x, z):
        surveyed = (
            ring[1] + (ring[2] - ring[1]) * (x / 7) + (ring[0] - ring[1]) * (z / 13)
        )
        return min(-0.005, float(sample(surveyed)) * height_scale - datum - 0.06)

    bases = []
    for lo, hi in ((4.95, 5.16), (6.75, 6.96)):
        sides = [(lo, -0.85), (hi, -0.85), (hi, SETBACK + 0.075), (lo, SETBACK + 0.075)]
        bases.append({"plan": sides, "ground": [grade(x, z) for x, z in sides]})
    # Lower the original bottom bevel and underside together. Extruding a
    # second box leaves the original rounded edge exposed as a dark joint.
    moved = 0
    for vertex in wall.data.vertices:
        co = basis @ vertex.co
        x, y, z, _ = parcel @ Vector((co.x, co.z, -co.y, 1))
        if (
            y <= 0.025
            and any(
                v["plan"][0][0] - 0.001 <= x <= v["plan"][1][0] + 0.001 for v in bases
            )
            and -0.851 <= z <= SETBACK + 0.076
        ):
            co.z += grade(x, z)
            vertex.co = back @ co
            moved += 1
    if moved != 312:
        raise ValueError(f"Unexpected pier base vertices: {moved}")
    wall.data.update()
    return bases


def build(out):
    src = Path(bpy.data.filepath)
    parent = json.loads((src.parent / "porch-light.provenance.json").read_text())
    if digest(src) != parent["sha256"] or out.resolve().is_relative_to(
        Path(__file__).resolve().parents[2]
    ):
        raise ValueError(
            "Open the registered opal porch scene and use a source-store output"
        )
    out.mkdir(parents=True, exist_ok=True)
    mats = surfaces(out / "surfaces")
    scene = bpy.context.scene
    if scene.objects["front-door"]["Open angle"] != 0:
        raise ValueError("Expected the closed entrance")
    registration = json.loads(
        (src.parent.parent / "window-optics/optics-source.provenance.json").read_text()
    )
    local = Matrix(registration["parcel_from_assembled_gltf"])
    inverse = local.inverted()
    replaced = [
        o
        for o in scene.objects
        if o.type == "MESH"
        and o.data.materials
        and o.data.materials[0].name.startswith("terracotta-paver@")
        and o.name.startswith(("Tiled entrance stair", "entrance-casing"))
    ]
    external = next(o for o in replaced if o.name.startswith("Tiled entrance stair"))
    levels = {
        round((external.matrix_world @ v.co).z, 3) for v in external.data.vertices
    }
    courses = 0
    while round(-0.145 * courses + 0.009, 3) in levels:
        courses += 1
    if courses < 1 or len(replaced) != 6:
        raise ValueError("Missing course or landing")
    for o in replaced:
        o.hide_render = True
        o.hide_viewport = True
    group = bpy.data.collections.new("Hand-set terracotta entrance")
    scene.collection.children.link(group)
    builds = []
    for course in range(-1, courses):
        coll = bpy.data.collections.new(
            "Sill landing" if course < 0 else f"Fired-clay course {course + 1:02d}"
        )
        group.children.link(coll)
        objects = []
        for col in range(5):
            face = ((course + 1) * 3 + col * 5 + (col // 3)) % 8
            objects.append(
                tile(course, col, face, course < 0, inverse, mats[face // 4], coll)
            )
        builds.append((course, objects))
    # The editable collection keeps individual units. For each portable course
    # group the export by material so its five tiles share GPU draws.
    bases = arch_bases(scene, group, inverse)
    bpy.context.view_layer.update()
    tree, _ = scene_surfaces(
        scene, np.array([6.0, -1.0, -2.0]), np.array([9.5, 3.4, 3.0])
    )
    for _, units in builds:
        for unit in units:
            contact_transfer(unit.data, tree, unit.matrix_world)
            unit.data.uv_layers.active_index = 0
    course_files = []
    bpy.ops.object.select_all(action="DESELECT")
    glbs = out / "courses"
    glbs.mkdir(exist_ok=True)
    for course, objects in builds:
        for ob in objects:
            ob.select_set(True)
        file = glbs / ("landing.glb" if course < 0 else f"course-{course + 1}.glb")
        bpy.ops.export_scene.gltf(
            filepath=str(file),
            use_selection=True,
            export_materials="EXPORT",
            export_texcoords=True,
            export_normals=True,
            export_animations=False,
            export_cameras=False,
            export_lights=False,
            export_extras=False,
        )
        for ob in objects:
            ob.select_set(False)
        course_files.append(
            {
                "file": str(file.relative_to(out)),
                "sha256": digest(file),
                "height": 0 if course < 0 else -0.145 * course,
                "depth": 0 if course < 0 else -0.33 * course,
            }
        )
    scene.frame_set(34)
    scene.camera = scene.objects["Entrance from the design reference"]
    scene.view_settings.exposure = 1
    scene.cycles.samples = 64
    scene.cycles.seed = 38
    scene.render.resolution_x = 1672
    scene.render.resolution_y = 941
    scene.render.resolution_percentage = 100
    bpy.context.preferences.filepaths.save_version = 0
    target = out / "21 Beaver Street - Hand-set terracotta.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(target.resolve()), compress=True)
    (out / "stoop.provenance.json").write_text(
        json.dumps(
            {
                "file": target.name,
                "sha256": digest(target),
                "parent_sha256": parent["sha256"],
                "house_sha256": registration["house_sha256"],
                "parcel_from_assembled_gltf": registration[
                    "parcel_from_assembled_gltf"
                ],
                "courses": course_files,
                "pier_bases": bases,
                "tiles": 5 * len(builds),
                "replaced": sorted(o.name for o in replaced),
                "build": "scripts/blender-buildings/build_beaver_stoop.py",
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("out", type=Path)
    build(cli.parse_args(sys.argv[sys.argv.index("--") + 1 :]).out)
