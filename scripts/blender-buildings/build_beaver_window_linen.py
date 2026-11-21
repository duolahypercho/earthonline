"""Fit the folded Roman blind, lift tapes and hardware in the editable Beaver house."""

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import itertools

from beaver_dining_parts import material, mesh

ROOT = HERE.parents[1]
PHOTO = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina"
GROUP = "Made-to-measure rear Roman blind"
CLOTH = "Five pleats in unbleached flax"
LEFT_X, RIGHT_X = 6.465, 1.385
RAIL_Y = -16.405
LIFTS = (0.049, 0.270, 0.551, 0.792, 0.955)
END_TAPES = (LIFTS[0], LIFTS[-1])
TAU = math.tau


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def flax_material(folder):
    recipe = json.loads((PHOTO / "linen.provenance.json").read_text())
    cloth = material("Sun-filtering plainweave flax", (204, 174, 128), 0.84)
    tree = cloth.node_tree
    bs = tree.nodes["Principled BSDF"]
    coord = tree.nodes.new("ShaderNodeUVMap")
    coord.uv_map = "Linen metres"
    images = {}
    for rec in recipe["inputs"]:
        path = folder / rec["url"].split("/")[-1]
        if digest(path) != rec["sha256"]:
            raise ValueError(f"Linen photograph changed: {path.name}")
        img = bpy.data.images.load(str(path), check_existing=False)
        img.colorspace_settings.name = (
            "sRGB" if rec["role"] == "albedo" else "Non-Color"
        )
        img.pack()
        tex = tree.nodes.new("ShaderNodeTexImage")
        tex.image = img
        tree.links.new(coord.outputs["UV"], tex.inputs["Vector"])
        images[rec["role"]] = tex.outputs["Color"]
    dye = tree.nodes.new("ShaderNodeMixRGB")
    dye.blend_type = "MULTIPLY"
    dye.inputs[0].default_value = 1.0
    dye.inputs[2].default_value = (0.92, 0.70, 0.43, 1)
    tree.links.new(images["albedo"], dye.inputs[1])
    tree.links.new(dye.outputs[0], bs.inputs["Base Color"])
    normal = tree.nodes.new("ShaderNodeNormalMap")
    normal.uv_map = coord.uv_map
    normal.inputs["Strength"].default_value = 0.32
    tree.links.new(images["normal"], normal.inputs["Color"])
    tree.links.new(normal.outputs[0], bs.inputs["Normal"])
    sep = tree.nodes.new("ShaderNodeSeparateColor")
    tree.links.new(images["arm"], sep.inputs[0])
    tree.links.new(sep.outputs["Green"], bs.inputs["Roughness"])
    bs.inputs["Sheen Weight"].default_value = 0.22
    transmit = tree.nodes.new("ShaderNodeBsdfTranslucent")
    density = tree.nodes.new("ShaderNodeMixRGB")
    density.blend_type = "MULTIPLY"
    density.inputs[0].default_value = 0.33
    density.inputs[1].default_value = (0.86, 0.48, 0.185, 1)
    tree.links.new(images["albedo"], density.inputs[2])
    tree.links.new(density.outputs[0], transmit.inputs["Color"])
    tree.links.new(normal.outputs[0], transmit.inputs["Normal"])
    mix = tree.nodes.new("ShaderNodeMixShader")
    mix.inputs[0].default_value = 0.46
    tree.links.new(bs.outputs[0], mix.inputs[1])
    tree.links.new(transmit.outputs[0], mix.inputs[2])
    tree.links.new(mix.outputs[0], tree.nodes["Material Output"].inputs[0])
    return cloth, recipe


# Depth and drop along the folds, from a rear stitch line out around the nose
# and back to the next row of lift rings. The return hangs below that row.
PROFILE = (
    (0.0, 0.0),
    (0.18, 0.18),
    (0.48, 0.44),
    (0.82, 0.79),
    (1.0, 1.12),
    (0.93, 1.215),
    (0.64, 1.13),
    (0.29, 0.94),
    (0.085, 0.83),
    (0.0, 1.0),
)


def top_z(u):
    return 2.760 - 0.088 * u + 0.005 * math.sin(TAU * u)


def point(u, course, phase):
    i = min(len(PROFILE) - 2, int(phase * (len(PROFILE) - 1)))
    f = phase * (len(PROFILE) - 1) - i
    (a, b), (c, d) = PROFILE[i : i + 2]
    depth, drop = a + f * (c - a), b + f * (d - b)
    row = course + drop
    # The intermediate rings gather the *back* at discrete stations. In front,
    # fabric between them hangs in broad, unequal catenary-like arcs.
    boundaries = (0.0, *LIFTS, 1.0)
    left, right = next((a, b) for a, b in itertools.pairwise(boundaries) if a <= u <= b)
    t = (u - left) / (right - left)
    bay = math.sin(math.pi * t) ** 1.35 * min(1.0, (right - left) / 0.16)
    drape = (0.017 + 0.053 * (right - left) / 0.28) * bay
    progress = min(row / 5.0, 1.0)
    ripple = 0.008 * math.sin(15 * u + course * 0.52) + 0.004 * math.sin(
        39 * u - course * 0.39
    )
    ripple += (
        0.006
        * math.sin(69 * u + 0.8 * math.sin(12 * u) + course * 0.74)
        * (1 - bay * 0.55)
    )
    depth_scale = 0.10 + 0.058 * bay + 0.013 * math.cos(10 * u + course * 0.67)
    y = (
        RAIL_Y
        + 0.008
        + depth * depth_scale
        + 0.008 * math.sin(15 * u + phase) * progress
    )
    z = (
        top_z(u)
        - 0.095
        - 0.099 * row
        - progress**0.66 * drape
        - ripple * min(1.0, row / 1.6)
    )
    return (LEFT_X + (RIGHT_X - LEFT_X) * u, y, z)


def surface(coll, name, u_values, rows, coords, mat, chart):
    points, faces, weave_uv, light_uv = [], [], [], []
    distances = np.zeros(len(u_values))
    previous = None
    for j, row in enumerate(rows):
        band = [coords(u, row) for u in u_values]
        if previous is not None:
            distances += np.linalg.norm(np.array(band) - previous, axis=1)
        previous = np.asarray(band)
        for i, (u, p) in enumerate(zip(u_values, band)):
            points.append(p)
            weave_uv.append(((LEFT_X - p[0]) / 0.50, distances[i] / 0.50))
            light_uv.append(chart(u, j / (len(rows) - 1)))
            if i and j:
                k = j * len(u_values) + i
                faces.append((k - len(u_values) - 1, k - 1, k, k - len(u_values)))
    ob = mesh(coll, name, points, faces, [mat], uv=weave_uv)
    ob.data.uv_layers.active.name = "Linen metres"
    uv = ob.data.uv_layers.new(name="Light across folds")
    loops = np.empty(len(ob.data.loops), np.int32)
    ob.data.loops.foreach_get("vertex_index", loops)
    uv.uv.foreach_set("vector", np.asarray(light_uv, np.float32)[loops].ravel())
    ob.data.uv_layers.active_index = 0
    return ob


def sweep(coll, name, path, radius, mat, sides=10):
    vertices, faces = [], []
    pts = [Vector(p) for p in path]
    for i, p in enumerate(pts):
        tangent = (pts[min(len(pts) - 1, i + 1)] - pts[max(0, i - 1)]).normalized()
        axis = tangent.cross(Vector((1, 0, 0)))
        if axis.length < 0.05:
            axis = tangent.cross(Vector((0, 0, 1)))
        axis.normalize()
        cross = tangent.cross(axis)
        for j in range(sides):
            angle = TAU * j / sides
            vertices.append(
                p + radius * (math.cos(angle) * axis + math.sin(angle) * cross)
            )
            if i:
                k = i * sides + j
                ne = i * sides + (j + 1) % sides
                faces.append((k - sides, ne - sides, ne, k))
    faces.extend(
        (
            tuple(reversed(range(sides))),
            tuple(range(len(vertices) - sides, len(vertices))),
        )
    )
    return mesh(coll, name, vertices, faces, [mat])


def bind_tape(coll, u, mat):
    x = point(u, 0, 0)[0]
    bottom = point(u, 4, 1)[2] - 0.013
    top = top_z(u) - 0.085
    # Each narrow woven lift tape bears on the outside of the stack. A
    # separate rear line passes through the rings and returns to the head.
    path = []
    for i in range(41):
        t = i / 40
        y = (
            RAIL_Y
            + 0.121
            + 0.012 * math.sin(math.pi * t)
            + 0.007 * math.sin(t * TAU * 5)
        )
        path.append((x + 0.002 * math.sin(5 * t), y, top + (bottom - top) * t))
    path.extend(
        (
            (x, RAIL_Y + 0.115, bottom - 0.010),
            (x, RAIL_Y + 0.070, bottom - 0.013),
            (x, RAIL_Y + 0.017, bottom - 0.004),
            (x, RAIL_Y + 0.008, bottom + 0.017),
            (x, RAIL_Y + 0.007, top),
        )
    )
    offsets = [
        (-0.015, -0.0010),
        (-0.016, 0.0),
        (-0.015, 0.0010),
        (0.015, 0.0010),
        (0.016, 0.0),
        (0.015, -0.0010),
    ]
    pts = []
    faces = []
    for j, (px, py, pz) in enumerate(path):
        tangent = (
            Vector(path[min(j + 1, len(path) - 1)]) - Vector(path[max(0, j - 1)])
        ).normalized()
        forward = tangent.cross(Vector((1, 0, 0))).normalized()
        for i, (width, thick) in enumerate(offsets):
            pts.append(Vector((px + width, py, pz)) + thick * forward)
            if j:
                k = j * 6 + i
                n = j * 6 + (i + 1) % 6
                faces.append((k - 6, n - 6, n, k))
    mesh(coll, "Woven end lift tape", pts, faces, [mat])
    if u == END_TAPES[0]:
        end = 1.30
        sweep(
            coll,
            "Hanging braided raise cord",
            [
                (
                    x + 0.010 * math.sin(i * 0.24),
                    RAIL_Y + 0.087 + 0.005 * math.sin(i * 0.36),
                    bottom + (end - bottom) * i / 20,
                )
                for i in range(21)
            ],
            0.0052,
            mat,
        )


def build(out, inputs):
    src = Path(bpy.data.filepath)
    meta = json.loads((src.parent / "fitted-dining.provenance.json").read_text())
    if digest(src) != meta["sha256"] or out.resolve().is_relative_to(ROOT):
        raise ValueError(
            "Use the verified fitted house and an external output directory"
        )
    out.mkdir(parents=True, exist_ok=True)
    scene = bpy.data.scenes[meta["sunlight_scene"]]
    bpy.context.window.scene = scene
    scene.frame_set(3)
    old = scene.objects["Relaxed linen Roman blind"]
    old.hide_render = old.hide_viewport = True
    group = bpy.data.collections.new(GROUP)
    for sc in bpy.data.scenes:
        if old.name in sc.objects:
            sc.collection.children.link(group)
    flax, recipe = flax_material(inputs)
    tape = material("Dark woven cotton lift tape", (43, 37, 28), 0.86)
    brass = material("Rubbed brass lift rings", (129, 100, 56), 0.38, 0.78)
    twine = material("Unbleached lift cord", (146, 123, 91), 0.87)
    us = sorted(set([i / 144 for i in range(145)] + list(LIFTS)))
    steps = 5 * (len(PROFILE) - 1)

    def body(u, j):
        course = min(4, j // (len(PROFILE) - 1))
        phase = (j - course * (len(PROFILE) - 1)) / (len(PROFILE) - 1)
        return point(u, course, phase)

    surface(
        group,
        CLOTH,
        us,
        list(range(steps + 1)),
        body,
        flax,
        lambda u, v: (0.01 + 0.98 * u, 0.02 + 0.81 * v),
    )
    # Rolled, fabric-covered valance. Its underside shades the first pleat.
    head = [
        (0.004, 0.103),
        (0.005, 0.011),
        (0.020, 0),
        (0.067, -0.003),
        (0.092, 0.010),
        (0.097, 0.026),
        (0.099, 0.081),
        (0.094, 0.098),
        (0.080, 0.111),
        (0.046, 0.114),
        (0.009, 0.106),
    ]

    def header(u, j):
        d, h = head[j]
        return (
            LEFT_X + (RIGHT_X - LEFT_X) * u,
            RAIL_Y + d,
            top_z(u) - h + 0.003 * math.sin(29 * u),
        )

    surface(
        group,
        "Linen-covered headrail",
        us,
        list(range(len(head))),
        header,
        flax,
        lambda u, v: (0.01 + 0.98 * u, 0.86 + 0.12 * v),
    )
    for u in END_TAPES:
        bind_tape(group, u, tape)
    for u in LIFTS:
        x = point(u, 0, 0)[0]
        sweep(
            group,
            "Braided rear lift line",
            [
                (x, RAIL_Y - 0.010, top_z(u) - 0.065),
                (x, RAIL_Y - 0.010, point(u, 4, 1)[2]),
            ],
            0.0022,
            twine,
            8,
        )
        for row in range(5):
            z = point(u, row, 1)[2]
            path = [
                (
                    x + 0.006 * math.cos(TAU * j / 16),
                    RAIL_Y - 0.007 + 0.007 * math.sin(TAU * j / 16),
                    z,
                )
                for j in range(17)
            ]
            sweep(group, "Sewn brass guide ring", path, 0.00125, brass, 6)
    for u in (0.001, 0.499, 0.999):
        # Enclosed pockets and lap seams are a second ply, sewn onto the back.
        # Narrow hems use the same continuous cloth, offset toward the glass.
        def hem(v, j, u=u):
            x, y, z = body(u, j)
            return x + (v - 0.5) * (0.026 if u == 0.499 else 0.017), y - 0.0018, z

        surface(
            group,
            "Felled linen seam",
            list(np.linspace(0, 1, 3)),
            list(range(steps + 1)),
            hem,
            flax,
            lambda t, v, u=u: (
                0.01
                + 0.98
                * (
                    u
                    + (t - 0.5) * (0.026 if u == 0.499 else 0.017) / (RIGHT_X - LEFT_X)
                ),
                0.02 + 0.81 * v,
            ),
        )
    bpy.context.view_layer.update()
    filename = out / "21 Beaver - Rear window linen.blend"
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(filename), compress=True)
    rec = {
        "file": filename.name,
        "source": src.name,
        "source_sha256": meta["sha256"],
        "sha256": digest(filename),
        "design_image_sha256": meta["design_image_sha256"],
        "materials": recipe["inputs"],
        "attribution": recipe["attribution"],
        "license": "Project-authored geometry and textiles with CC0 linen photographs.",
        "source_url": recipe["source"],
        "head_width_m": LEFT_X - RIGHT_X,
        "lift_positions": list(LIFTS),
        "pleats": 5,
        "parts": sorted(o.name for o in group.objects),
        "build": "scripts/blender-buildings/build_beaver_window_linen.py",
    }
    (out / "window-linen.provenance.json").write_text(json.dumps(rec, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("linen_inputs", type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    build(args.output.resolve(), args.linen_inputs.resolve())
