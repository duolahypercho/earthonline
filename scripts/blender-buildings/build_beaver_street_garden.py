"""Install the street-side dry garden in a fully furnished, editable Blender house."""

import argparse
import json
import math
import random
import sys
from pathlib import Path

import bmesh
import bpy
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from beaver_dining_parts import material, mesh
from beaver_front_plants import agave, sage, tussock
from beaver_lighting import contact_transfer
from build_beaver_window_linen import digest

GROUP = "Front garden · agaves and deergrass"
PARENT = (
    HERE.parents[1]
    / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"
)
ROSETTES = [
    (3.04, -1.35, 0.84),
    (1.00, -0.81, 0.69),
    (4.39, -0.72, 0.43),
    (1.52, -1.70, 0.36),
    (3.86, -1.94, 0.31),
    (4.55, -1.54, 0.24),
    (2.03, -1.89, 0.18),
]
GRASSES = [
    (0.49, -0.93, 0.46),
    (0.58, -1.52, 0.33),
    (1.51, -0.47, 0.51),
    (1.96, -0.98, 0.54),
    (2.37, -1.72, 0.34),
    (2.76, -0.48, 0.57),
    (3.7, -0.65, 0.55),
    (4.35, -1.26, 0.44),
    (3.37, -1.94, 0.23),
    (1.86, -1.67, 0.29),
    (4.70, -0.44, 0.36),
]


def make_material(directory, spec):
    mat = material(spec["name"], (255, 255, 255), 1)
    tree = mat.node_tree
    bs = tree.nodes["Principled BSDF"]
    for role, rec in spec["maps"].items():
        path = directory / rec["file"]
        if digest(path) != rec["sha256"]:
            raise ValueError(path)
        img = bpy.data.images.load(str(path), check_existing=False)
        img.colorspace_settings.name = "sRGB" if role == "pigment" else "Non-Color"
        img.pack()
        tex = tree.nodes.new("ShaderNodeTexImage")
        tex.image = img
        if role == "pigment":
            tree.links.new(tex.outputs["Color"], bs.inputs["Base Color"])
        elif role == "normal":
            n = tree.nodes.new("ShaderNodeNormalMap")
            n.inputs["Strength"].default_value = (
                0.65 if spec["name"].startswith("Agave") else 0.82
            )
            tree.links.new(tex.outputs["Color"], n.inputs["Color"])
            tree.links.new(n.outputs[0], bs.inputs["Normal"])
        else:
            separate = tree.nodes.new("ShaderNodeSeparateColor")
            tree.links.new(tex.outputs["Color"], separate.inputs[0])
            tree.links.new(separate.outputs["Green"], bs.inputs["Roughness"])
            tree.links.new(separate.outputs["Blue"], bs.inputs["Metallic"])
    return mat


def fit_surfaces(group, root, curb, soil, plan):
    previous = []
    for ob in root.children:
        family = ob.active_material.name.split("@")[0]
        previous.append(ob.name)
        ob.hide_render = ob.hide_viewport = True
        if family not in ("float-finish-stucco", "decomposed-granite"):
            continue
        ground = family == "decomposed-granite"
        data = ob.data.copy()
        data.transform(ob.matrix_world)
        if ground:
            surface = bmesh.new()
            surface.from_mesh(data)
            bmesh.ops.subdivide_edges(
                surface, edges=list(surface.edges), cuts=2, use_grid_fill=True
            )
            surface.to_mesh(data)
            surface.free()
            data.update()

        data.materials.clear()
        data.materials.append(soil if ground else curb)
        new = bpy.data.objects.new(
            "Mineral soil in the front bed"
            if ground
            else "Lime-rendered retaining wall",
            data,
        )
        group.objects.link(new)
        for old in list(data.uv_layers):
            data.uv_layers.remove(old)
        layer = data.uv_layers.new(name="Surface metres")
        for poly in data.polygons:
            along = Vector((-poly.normal.y, poly.normal.x, 0))
            for index in poly.loop_indices:
                pt = data.vertices[data.loops[index].vertex_index].co
                x, _, z = plan @ Vector((pt.x, pt.z, -pt.y))
                crown = 0.126 - 0.093 * x + 0.078 * z
                if ground:
                    uv = (pt.x / 0.91, -pt.y / 0.91)
                else:
                    # One continuous projection along the street face; short returns turn at its ends.
                    u = pt.x if abs(along.x) > 0.4 else pt.y
                    uv = (u / 1.53, 1 - max(0, crown - pt.z) / 0.67)
                layer.data[index].uv = uv
    # The public path and the planter collision use the surveyed wall. Model the
    # narrow bullnose above that wall and put a small mortar joint between lengths.
    border = [
        (
            0.14 + i * 4.77 / 31,
            -1.68 - 0.79 * max(0, math.sin(math.pi * i / 31)) ** 0.82,
        )
        for i in range(32)
    ]
    border += [(4.91, 0.04), (0.14, 0.04)]
    assembled = plan.inverted()
    pts = []
    uv = []
    faces = []
    length = 0.0
    for (x, z), (xx, zz) in zip(border, border[1:] + border[:1]):
        run = math.hypot(xx - x, zz - z)
        normal = Vector((-(zz - z) / run, (xx - x) / run))
        start = len(pts)
        profile = [
            (-0.004, 0.042),
            (-0.009, 0.056),
            (-0.006, 0.069),
            (0.009, 0.080),
            (0.063, 0.078),
            (0.082, 0.065),
            (0.083, 0.045),
        ]
        for t in (0.002 / run, 1 - 0.002 / run):
            for offset, rise in profile:
                px = x + (xx - x) * t + normal.x * offset
                pz = z + (zz - z) * t + normal.y * offset
                pp = assembled @ Vector((px, 0.07 - 0.093 * px + 0.078 * pz + rise, pz))
                pts.append((pp.x, -pp.z, pp.y))
                uv.append(((length + run * t) / 0.96, 0.985 - offset / 0.66))
        n = len(profile)
        for j in range(n - 1):
            faces.append((start + j, start + j + n, start + j + n + 1, start + j + 1))
        length += run
    mesh(group, "Bullnose of the retaining wall", pts, faces, [curb], uv=uv)
    return sorted(previous)


def mineral_mulch(coll, soil, location):
    rng = random.Random(210518)
    verts = []
    faces = []
    uv = []
    for i in range(2850):
        x = rng.uniform(0.23, 4.80)
        edge = -1.68 - 0.79 * max(0, math.sin(math.pi * (x - 0.14) / 4.77)) ** 0.82
        z = rng.uniform(edge + 0.14, -0.08)
        center = location(x, z)
        r = rng.uniform(0.007, 0.024) * (2.7 if i % 37 == 0 else 1)
        stretch = rng.uniform(0.65, 1.50)
        angle = rng.random() * math.tau
        count = rng.choice((4, 5, 6))
        start = len(verts)
        for band in (0, 1):
            for j in range(count):
                a = angle + j * math.tau / count
                rad = r * rng.uniform(0.77, 1.14) * (0.62 if band else 1)
                p = center + Vector(
                    (
                        math.cos(a) * rad * stretch,
                        math.sin(a) * rad,
                        r
                        * (
                            rng.uniform(0.20, 0.82)
                            if band
                            else rng.uniform(-0.11, 0.05)
                        ),
                    )
                )
                verts.append(p)
                uv.append((p.x / 0.16 + i * 0.017, p.y / 0.16))
        for j in range(count):
            k = (j + 1) % count
            faces.append((start + j, start + k, start + count + k, start + count + j))
        faces.append(tuple(start + count + j for j in range(count)))
    return mesh(
        coll, "Fractured granite mulch", verts, faces, [soil], uv=uv, smooth=False
    )


def build(output, placement):
    src = Path(bpy.data.filepath)
    prior = json.loads((src.parent / "window-linen.provenance.json").read_text())
    shipped = PARENT / "fitted-dining/window-linen.provenance.json"
    expected = json.loads(shipped.read_text())["source_scene_sha256"]
    if digest(src) != expected or prior["sha256"] != expected:
        raise ValueError("Expected the fitted rear-window house")
    if output.is_relative_to(HERE.parents[1]):
        raise ValueError("Keep the packed house outside Git")
    recipe = json.loads((output / "surfaces/garden-surfaces.json").read_text())
    parcel = Matrix(
        json.loads((placement / "portable-dining.provenance.json").read_text())[
            "placement"
        ]["parcel_from_assembled_gltf"]
    ).to_3x3()
    assembled = parcel.inverted()

    def location(x, z):
        p = assembled @ Vector((x, 0.07 - 0.093 * x + 0.078 * z, z))
        return Vector((p.x, -p.z, p.y))

    scene = bpy.data.scenes["Dining · site daylight"]
    bpy.context.window.scene = scene
    scene.frame_set(3)
    bpy.context.view_layer.update()
    root = scene.objects["front-dry-garden"]
    group = bpy.data.collections.new(GROUP)
    for sc in bpy.data.scenes:
        if root.name in sc.objects:
            sc.collection.children.link(group)
    group.asset_mark()
    group.asset_data.author = "Earth Online"
    plant, curb, soil = [
        make_material(output / "surfaces", m) for m in recipe["materials"]
    ]
    replaced = fit_surfaces(group, root, curb, soil, parcel)
    for i, (x, z, size) in enumerate(ROSETTES):
        agave(
            group, f"Blue agave {i + 1:02d}", location(x, z), size, 5083 + i * 19, plant
        )
    for i, (x, z, size) in enumerate(
        (
            (2.22, -0.67, 0.57),
            (2.55, -1.97, 0.21),
            (3.47, -1.86, 0.20),
            (1.48, -1.89, 0.18),
            (4.18, -0.94, 0.23),
            (0.54, -1.40, 0.22),
        )
    ):
        sage(
            group,
            f"Gray-leaved garden shrub {i + 1:02d}",
            location(x, z),
            size,
            1227 + i * 39,
            plant,
        )
    for i, (x, z, size) in enumerate(GRASSES):
        tussock(
            group, f"Deergrass {i + 1:02d}", location(x, z), size, 4289 + i * 137, plant
        )
    mineral_mulch(group, soil, location)
    # Survey the complete bed. Stones, blades and stems interrupt skylight within
    # 45 cm of each surface, independently of the direction of the live sun.
    bpy.context.view_layer.update()
    vertices = []
    faces = []
    for o in group.objects:
        offset = len(vertices)
        vertices.extend(o.matrix_world @ v.co for v in o.data.vertices)
        faces.extend([offset + i for i in f.vertices] for f in o.data.polygons)
    tree = BVHTree.FromPolygons(vertices, faces)
    for o in group.objects:
        contact_transfer(o.data, tree, o.matrix_world)
        o.data.uv_layers.active_index = 0
        o["source_part"] = o.name
    scene = scene.copy()
    scene.name = "Front garden · street daylight"
    scene.use_fake_user = True
    scene.view_settings.exposure = 1.0
    bpy.context.window.scene = scene
    camera = bpy.data.objects.new(
        "Street garden at standing height", bpy.data.cameras.new("Private front bed")
    )
    scene.collection.objects.link(camera)
    camera.location = (6.85, 6.64, 0.98)
    camera.rotation_euler = (
        (Vector((3.75, 1.44, 0.05)) - camera.location)
        .to_track_quat("-Z", "Y")
        .to_euler()
    )
    camera.data.lens = 36
    for b in list(scene.timeline_markers):
        scene.timeline_markers.remove(b)
    scene.camera = camera
    for area in bpy.context.screen.areas:
        if area.type == "VIEW_3D":
            area.spaces.active.region_3d.view_perspective = "CAMERA"
    bpy.context.preferences.filepaths.save_version = 0
    scene.unit_settings.system = "METRIC"
    filepath = output / "21 Beaver - Street garden.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(filepath), compress=True)
    bpy.ops.object.select_all(action="DESELECT")
    for ob in group.objects:
        ob.select_set(True)
    glb = output / "Street garden.glb"
    bpy.ops.export_scene.gltf(
        filepath=str(glb),
        export_format="GLB",
        use_selection=True,
        export_normals=True,
        export_texcoords=True,
        export_extras=True,
        export_animations=False,
        export_yup=True,
    )
    record = {
        "file": filepath.name,
        "sha256": digest(filepath),
        "parent": src.name,
        "parent_sha256": expected,
        "portable": {"file": glb.name, "sha256": digest(glb)},
        "parts": sorted(o.name for o in group.objects),
        "replaced": replaced,
        "rosettes": ROSETTES,
        "grass_clumps": GRASSES,
        "parcel_from_assembled_gltf": [list(r) for r in parcel.to_4x4()],
        "sources": [
            "https://landscapeplants.oregonstate.edu/plants/agave-parryi",
            "https://ucanr.edu/site/mg-sonoma/muhlenbergia-rigens-deer-grass",
        ],
        "materials": recipe,
        "build": "scripts/blender-buildings/build_beaver_street_garden.py",
    }
    (output / "street-garden.provenance.json").write_text(
        json.dumps(record, indent=2) + "\n"
    )


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("output", type=Path)
    cli.add_argument("placement", type=Path)
    a = cli.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    build(a.output.resolve(), a.placement.resolve())
