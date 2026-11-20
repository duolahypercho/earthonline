"""Build 21 Beaver Street. Run with Blender --background --factory-startup --python this_file.
Optional arguments after --: --blend PATH and --parts DIR (editable source and standalone GLBs outside the checkout).
Model coordinates below are across the front, height, and depth toward the yard.
"""

import argparse
import hashlib
import json
import math
import random
import sys
from pathlib import Path

import bmesh
import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
import beaver_lighting

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "public/assets/buildings/beaver-terrace"
RISE = 10 / 3
rng = random.Random(21)
boxes, ramps = [], []
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
room = "Ground"
batches = {}
materials = {}
# Component insertion points in the builder's across / height / depth coordinates.
pivots = {}


def part(name, x, y, z):
    pivots[name] = Vector((x, -z, y))
    return name


def finish(name, color, rough=0.62, metal=0, texture=None):
    key = f"{name}@{color}"
    mat = bpy.data.materials.new(key)
    rgb = [int(color[i : i + 2], 16) / 255 for i in (0, 2, 4)]
    linear = [((c + 0.055) / 1.055) ** 2.4 if c > 0.04045 else c / 12.92 for c in rgb]
    mat.diffuse_color = (*linear, 1)
    mat.use_nodes = True
    tree = mat.node_tree
    bs = tree.nodes.get("Principled BSDF")
    bs.inputs["Base Color"].default_value = (*linear, 1)
    bs.inputs["Roughness"].default_value = rough
    bs.inputs["Metallic"].default_value = metal
    if texture:
        noise = tree.nodes.new("ShaderNodeTexNoise")
        noise.inputs["Scale"].default_value = texture
        noise.inputs["Detail"].default_value = 3
        bump = tree.nodes.new("ShaderNodeBump")
        bump.inputs["Strength"].default_value = 0.18
        bump.inputs["Distance"].default_value = 0.006
        tree.links.new(noise.outputs["Fac"], bump.inputs["Height"])
        tree.links.new(bump.outputs["Normal"], bs.inputs["Normal"])
    if name == "oiled-oak":
        coords = tree.nodes.new("ShaderNodeTexCoord")
        stretch = tree.nodes.new("ShaderNodeVectorMath")
        stretch.operation = "MULTIPLY"
        stretch.inputs[1].default_value = (2, 38, 22)
        tree.links.new(coords.outputs["Object"], stretch.inputs[0])
        grain = tree.nodes.new("ShaderNodeTexNoise")
        grain.inputs["Scale"].default_value = 1.8
        grain.inputs["Detail"].default_value = 3.0
        grain.inputs["Distortion"].default_value = 0.32
        tree.links.new(stretch.outputs["Vector"], grain.inputs["Vector"])
        colors = tree.nodes.new("ShaderNodeValToRGB")
        for stop, amount in zip(colors.color_ramp.elements, (0.50, 1.21)):
            stop.color = (*[min(1, c * amount) for c in linear], 1)
        tree.links.new(grain.outputs["Fac"], colors.inputs["Fac"])
        tree.links.new(colors.outputs["Color"], bs.inputs["Base Color"])
    materials[key] = mat
    return key


sage = finish("painted-clapboard", "68785e", texture=16)
plaster = finish("lime-plaster", "e3dbc6", texture=80)
ivory = finish("painted-white", "ece1c8")
oak = finish("oiled-oak", "ae906f", texture=9)
boards = [finish("oiled-oak", c, texture=12) for c in ("ba9a79", "b49575", "ba9e82", "b6987a")]
walnut = finish("oiled-oak", "785338", texture=14)
rust = finish("canvas-plain", "b66d4d", texture=105)
linen = finish("canvas-plain", "e6dcc3", texture=110)
blue = finish("canvas-plain", "708998", texture=100)
teal = finish("stoneware-glaze", "527775", 0.27)
brass = finish("brushed-brass", "b99a5e", 0.29, 0.72)
iron = finish("cast-iron", "393c39", 0.38, 0.5)
stone = finish("cast-stone", "b9b2a7", texture=64)
roof = finish("membrane-roof", "777874")
clay = finish("terracotta", "ac6c4f")
green = finish("leaf", "516e3c")
light_leaf = finish("new-leaf", "7d965b")
pottery = finish("porcelain", "e4dfce", 0.22)
glass = finish("glass-door", "ffffff", 0.08)
materials[glass].node_tree.nodes.get("Principled BSDF").inputs["Transmission Weight"].default_value = 1.0
paper = finish("book-paper", "d9c8a4")
book_colors = [teal, rust, walnut, blue, sage, brass]


def emit(mat, points, faces, smooth=False):
    """Batch by room and finish so the editable scene and GLB have few draws."""
    vertices, polygons, flags = batches.setdefault((room, mat), ([], [], []))
    offset = len(vertices)
    vertices.extend((x, -z, y) for x, y, z in points)
    for f in faces:
        patch = None
        if not smooth and len(f) == 4:
            patch = beaver_lighting.subdivide_quad(points, f, 0.36)
        if patch is None:
            polygons.append(tuple(offset + i for i in f))
            flags.append(smooth)
        else:
            coords, quads = patch
            start = len(vertices)
            vertices.extend((x, -z, y) for x, y, z in coords)
            polygons.extend(tuple(start + i for i in q) for q in quads)
            flags.extend([False] * len(quads))


cube_faces = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (3, 7, 6, 2), (0, 4, 7, 3), (1, 2, 6, 5)]


def box(mat, a, b, solid=False, bevel=0):
    x, y, z = a
    u, v, w = b
    if solid:
        boxes.append([a, b])
    points = [(x, y, z), (u, y, z), (u, v, z), (x, v, z), (x, y, w), (u, y, w), (u, v, w), (x, v, w)]
    if not bevel:
        emit(mat, points, cube_faces)
        return
    bm = bmesh.new()
    verts = [bm.verts.new(p) for p in points]
    for f in cube_faces:
        bm.faces.new([verts[i] for i in f])
    bmesh.ops.bevel(
        bm,
        geom=list(bm.edges),
        offset=min(bevel, (u - x) * 0.43, (v - y) * 0.43, (w - z) * 0.43),
        segments=3,
        affect="EDGES",
        profile=0.5,
    )
    bm.verts.index_update()
    emit(mat, [tuple(v.co) for v in bm.verts], [[v.index for v in f.verts] for f in bm.faces])
    bm.free()


def turned(mat, x, y, z, profile, n=16, closed=False):
    points = [
        (x + r * math.cos(i * math.tau / n), y + h, z + r * math.sin(i * math.tau / n))
        for r, h in profile
        for i in range(n)
    ]
    faces = [
        (j * n + i, (j + 1) * n + i, (j + 1) * n + (i + 1) % n, j * n + (i + 1) % n)
        for j in range(len(profile) - 1)
        for i in range(n)
    ]
    emit(mat, points, faces, True)
    if closed:
        # Flat end faces keep the rim normals independent of the foot and inner well.
        emit(mat, points[:n], [tuple(range(n))])
        emit(mat, points[-n:], [tuple(reversed(range(n)))])


def enamel_chip(centre, along, up, width, height, wear_random):
    """Thin, irregular rings expose the primer and oxidised steel at a contact point."""
    c, u, v = map(Vector, (centre, along, up))
    outline = []
    for j in range(13):
        a = j * math.tau / 13
        r = wear_random.uniform(0.70, 1.24)
        outline.append(u * (width * r * math.cos(a)) + v * (height * r * math.sin(a)))
    rings = [[c + p * k for p in outline] for k in (1, 0.75, 0.39)]
    for i, mat in enumerate((worn_primer, exposed_steel)):
        points = rings[i] + rings[i + 1]
        emit(mat, points, [(j, (j + 1) % 13, (j + 1) % 13 + 13, j + 13) for j in range(13)])
    emit(oxide, rings[-1], [tuple(range(13))])


def bent_tube(mat, centres, radius, axis, sides=10):
    """Sweep a round section through the supplied centreline in its bend plane."""
    pts = [Vector(p) for p in centres]
    axis = Vector(axis).normalized()
    points = []
    for i, centre in enumerate(pts):
        tangent = (pts[min(i + 1, len(pts) - 1)] - pts[max(i - 1, 0)]).normalized()
        across = tangent.cross(axis).normalized()
        for j in range(sides):
            a = j * math.tau / sides
            points.append(centre + radius * (axis * math.cos(a) + across * math.sin(a)))
    faces = [
        (i * sides + j, i * sides + (j + 1) % sides, (i + 1) * sides + (j + 1) % sides, (i + 1) * sides + j)
        for i in range(len(pts) - 1)
        for j in range(sides)
    ]
    emit(mat, points, faces, True)


def cushion(mat, centre, radii):
    """A softly crowned superellipsoid with subtly gathered upholstery."""
    x, y, z = centre

    def power(t, e):
        return math.copysign(abs(t) ** e, t)

    pts = []
    for j in range(13):
        t = (j / 12 - 0.5) * math.pi
        for i in range(28):
            a = i * math.tau / 28
            lat = power(math.cos(t), 0.42)
            pts.append(
                (
                    x + radii[0] * lat * power(math.cos(a), 0.40),
                    y + radii[1] * power(math.sin(t), 0.52),
                    z + radii[2] * lat * power(math.sin(a), 0.40),
                )
            )
    faces = [
        (j * 28 + i, (j + 1) * 28 + i, (j + 1) * 28 + (i + 1) % 28, j * 28 + (i + 1) % 28)
        for j in range(12)
        for i in range(28)
    ]
    emit(mat, pts, faces, True)


def floor(x0, x1, z0, z1, y):
    box(plaster, (x0, y - 0.16, z0), (x1, y - 0.025, z1), True)
    x = x0
    row = 0
    while x < x1 - 0.005:
        u = min(x1, x + 0.18)
        z = z0
        while z < z1 - 0.005:
            w = min(z1, z + (1.15 + (row % 3) * 0.55 if z == z0 else rng.uniform(2.4, 3.8)))
            box(rng.choice(boards), (x, y - 0.025, z), (u - 0.002, y, w - 0.002))
            z = w
        x = u
        row += 1


def table(x, y, z, w, d, h):
    box(oak, (x, y + h - 0.085, z), (x + w, y + h, z + d), True, 0.026)
    for a in [x + 0.09, x + w - 0.09]:
        for b in [z + 0.09, z + d - 0.09]:
            turned(
                walnut, a, y, b, [(0.035, 0), (0.043, 0.09), (0.035, h * 0.3), (0.055, h * 0.75), (0.058, h - 0.08)], 10
            )
    boxes.append([[x + 0.04, y, z + 0.04], [x + w - 0.04, y + h, z + d - 0.04]])


def chair(x, y, z):
    table(x, y, z, 0.55, 0.53, 0.46)
    for a in [x + 0.05, x + 0.5]:
        box(walnut, (a, y + 0.43, z + 0.47), (a + 0.035, y + 0.91, z + 0.52))
    box(oak, (x, y + 0.77, z + 0.46), (x + 0.56, y + 0.95, z + 0.525), False, 0.025)


def plant(x, y, z, size=1):
    turned(
        clay,
        x,
        y,
        z,
        [
            (0.12 * size, 0),
            (0.18 * size, 0.30 * size),
            (0.195 * size, 0.31 * size),
            (0.195 * size, 0.35 * size),
            (0.14 * size, 0.35 * size),
        ],
    )
    for i in range(19):
        a = i * 2.4
        r = size * (0.13 + i * 0.009)
        h = size * (1.15 - i * 0.024)
        dx, dz = math.cos(a), math.sin(a)
        mid = (x + 0.54 * r * dx, y + h - 0.10, z + 0.54 * r * dz)
        tip = (x + r * dx, y + h, z + r * dz)
        span = size * 0.064
        pts = [
            (x, y + 0.32 * size, z),
            (mid[0] + dz * span, mid[1], mid[2] - dx * span),
            tip,
            (mid[0] - dz * span, mid[1], mid[2] + dx * span),
            (mid[0], mid[1] + span * 0.5, mid[2]),
        ]
        emit(
            green if i % 3 else light_leaf,
            pts,
            [(0, 1, 4), (1, 2, 4), (2, 3, 4), (3, 0, 4), (4, 1, 0), (4, 2, 1), (4, 3, 2), (4, 0, 3)],
        )


def books(x, y, z, n):
    for i in range(n):
        dx = rng.uniform(0.05, 0.11)
        h = rng.uniform(0.23, 0.35)
        mat = rng.choice(book_colors)
        box(paper, (x, y, z), (x + dx, y + h, z + 0.19))
        box(mat, (x - 0.003, y - 0.003, z - 0.008), (x + dx + 0.003, y + h + 0.004, z + 0.019))
        box(brass, (x + 0.01, y + h * 0.7, z - 0.010), (x + dx - 0.01, y + h * 0.7 + 0.007, z - 0.008))
        x += dx + 0.01


# Walls, sash windows and weatherboards. Ground door opens beside the stair.
for level in range(3):
    y = level * RISE
    room = part(("ground-storey", "bedroom-storey", "workroom-storey")[level], 0, y, 0)
    if not level:
        floor(0, 7, 0, 13, y)
    else:
        floor(0, 5.1, 0, 13, y)
        floor(5.1, 7, 0, 3, y)
        floor(5.1, 7, 9, 13, y)
    for a, b in [(0, 0.14), (6.86, 7)]:
        box(plaster, (a, y, 0), (b, y + RISE, 13), True)
        box(ivory, (a - 0.012, y + 0.01, 0.13), (b + 0.018, y + 0.16, 12.85))
    for z in [0, 12.85]:
        panels = [
            (0, 0.7, 0, RISE),
            (0.7, 4.6, 0, 0.86),
            (0.7, 4.6, 2.55, RISE),
            (4.6, 5.2, 0, RISE),
            (6.7, 7, 0, RISE),
            (5.2, 6.7, 2.4 if level == 0 and z == 0 else 0, RISE),
        ]
        for x, u, h, v in panels:
            box(plaster, (x, y + h, z), (u, y + v, z + 0.15), True)
            for j in range(math.ceil((v - h) / 0.15)):
                low = y + h + j * 0.15
                outside = z - 0.028 if z == 0 else z + 0.151
                box(sage, (x, low, outside), (u, min(y + v, low + 0.146), outside + 0.022))
        for x in [0.69, 2, 3.32, 4.56]:
            box(ivory, (x, y + 0.83, z - 0.065), (x + 0.065, y + 2.57, z + 0.18), True, 0.009)
        for h in [0.85, 1.83, 2.53]:
            box(ivory, (0.63, y + h, z - 0.079), (4.69, y + h + 0.061, z + 0.23), False, 0.009)
        # The thin glass uses the engine's transparent door glass material.
        box(glass, (0.73, y + 0.91, z + 0.06), (4.55, y + 2.52, z + 0.067), True)
    for yoff, depth in [(RISE - 0.20, 0.08), (RISE - 0.12, 0.15), (RISE - 0.065, 0.19)]:
        box(ivory, (-0.04, y + yoff, -depth), (7.04, y + yoff + 0.055, 0.15), False, 0.009)
    if level == 2:
        for x in [i * 0.40 for i in range(18)]:
            box(ivory, (x, y + RISE - 0.31, -0.12), (x + 0.09, y + RISE - 0.17, 0.05), False, 0.012)
    if level < 2:
        room = part(f"stair-flight-{level + 1}", 5.10, y, 3.0)
        ramps.append([[5.1, y, 3], [5.1, y + RISE, 9], [6.85, y + RISE, 9], [6.85, y, 3]])
        for i in range(20):
            z = 3 + 0.3 * i
            h = y + (i + 1) * RISE / 20
            box(oak, (5.1, h - 0.21, z), (6.85, h, z + 0.31), False, 0.009)
            turned(
                ivory,
                5.17,
                h,
                z + 0.13,
                [(0.033, 0), (0.032, 0.12), (0.021, 0.20), (0.027, 0.65), (0.033, 0.72), (0.033, 0.88)],
                8,
            )
        # Long sloping walnut handrail, continuous over the nosings.
        pts = [
            (x, yy + offset, z)
            for x, yy, z in [
                (5.115, y + 0.99, 3),
                (5.23, y + 0.99, 3),
                (5.23, y + RISE + 0.99, 9),
                (5.115, y + RISE + 0.99, 9),
            ]
            for offset in [0, 0.07]
        ]
        emit(walnut, pts, [(0, 2, 4, 6), (1, 7, 5, 3), (0, 1, 3, 2), (2, 3, 5, 4), (4, 5, 7, 6), (6, 7, 1, 0)])
        for i in range(20):
            h = y + (i + 1) * RISE / 20
            z = 3 + 0.3 * i
            boxes.append([[5.11, h, z], [5.23, h + 0.96, z + 0.30]])
    if level:
        room = part(f"landing-balustrade-{level}", 5.05, y, 3.0)
        for i in range(23):
            turned(
                ivory,
                5.05,
                y,
                3.13 + 0.26 * i,
                [(0.029, 0), (0.026, 0.09), (0.02, 0.23), (0.025, 0.74), (0.029, 0.93)],
                8,
            )
        box(walnut, (4.99, y + 0.91, 3.03), (5.11, y + 1.0, 8.97), True, 0.018)

room = part("terrace-roof", 0, 3 * RISE, 0)
box(plaster, (0.14, 3 * RISE - 0.10, 0.15), (6.86, 3 * RISE - 0.02, 12.85))
box(roof, (-0.05, 3 * RISE, -0.10), (7.05, 3 * RISE + 0.15, 13.04), True)
for x in [0, 6.85]:
    box(ivory, (x, 10.15, 0), (x + 0.15, 10.37, 13.03))
for z in [0, 12.85]:
    box(ivory, (0, 10.15, z), (7, 10.37, z + 0.15))
room = part("foundation", 0, -2.4, 0)
box(stone, (0, -2.4, 0), (7, -0.16, 13), True)
room = part("front-door", 6.635, 0, 0.15)
box(sage, (6.58, 0, 0.15), (6.69, 2.35, 1.56), True, 0.012)
for h in [0.2, 1.12]:
    box(ivory, (6.55, h, 0.31), (6.58, h + 0.83, 1.39), False, 0.02)
    box(sage, (6.52, h + 0.045, 0.36), (6.55, h + 0.78, 1.34), False, 0.014)
turned(brass, 6.47, 1.02, 1.36, [(0.05, 0), (0.05, 0.065)])
room = part("entrance-casing", 5.95, 0, 0)
for x in [5.12, 6.68]:
    box(ivory, (x, 0, -0.07), (x + 0.09, 2.51, 0.19), False, 0.008)
box(ivory, (5.02, 2.43, -0.14), (6.86, 2.56, 0.17), False, 0.015)

room = "Ground"
room = part("upholstered-sofa", 1.04, 0, 2.70)
for x in [0.70, 1.34]:
    for z in [1.62, 3.77]:
        turned(walnut, x, 0, z, [(0.042, 0), (0.053, 0.095), (0.052, 0.11)], 12, closed=True)
box(rust, (0.60, 0.10, 1.51), (1.46, 0.42, 3.89), True, 0.07)
cushion(rust, (0.63, 0.66, 2.70), (0.20, 0.42, 1.18))
for z in [1.91, 2.71, 3.49]:
    cushion(rust, (1.05, 0.49, z), (0.44, 0.145, 0.40))
for z in [1.52, 3.89]:
    cushion(rust, (1.04, 0.57, z), (0.51, 0.27, 0.14))
for z, m in [(1.99, linen), (3.32, blue)]:
    cushion(m, (0.81, 0.79, z), (0.16, 0.26, 0.28))
room = part("sitting-room-rug", 2.465, 0.012, 2.725)
box(rust, (0.37, 0.012, 1.18), (4.56, 0.027, 4.27))
for inset in [0.07, 0.14, 0.22]:
    for x in [0.37 + inset, 4.53 - inset]:
        box(linen, (x, 0.028, 1.18 + inset), (x + 0.023, 0.030, 4.27 - inset))
    for z in [1.18 + inset, 4.24 - inset]:
        box(linen, (0.37 + inset, 0.028, z), (4.56 - inset, 0.030, z + 0.023))
for i in range(12):
    for j in range(7):
        x = 0.72 + i * 0.31
        z = 1.55 + j * 0.37
        emit(
            blue if (i + j) % 2 else linen,
            [(x, 0.032, z - 0.078), (x - 0.05, 0.032, z), (x, 0.032, z + 0.078), (x + 0.05, 0.032, z)],
            [(0, 1, 2, 3), (3, 2, 1, 0)],
        )
room = part("coffee-table", 2.505, 0, 2.70)
table(2.05, 0, 2.0, 0.91, 1.4, 0.44)
room = part("clothbound-book", 2.43, 0.44, 2.54)
box(blue, (2.21, 0.44, 2.25), (2.65, 0.49, 2.83), False, 0.01)
room = part("oak-dining-table", 2.375, 0, 7.575)
table(1.7, 0, 6.7, 1.35, 1.75, 0.79)
room = part("dining-chair", 2.375, 0, 6.345)
chair(2.1, 0, 6.08)
room = part("dining-chair-rear", 2.375, 0, 8.905)
chair(2.1, 0, 8.64)
room = part("base-cabinets", 0.6, 0, 11.53)
box(sage, (0.16, 0, 10.36), (1.02, 0.91, 12.68), True)
# Continuous counter with an inset sink cut through its rear section.
for x, u, z, w in [
    (0.14, 1.07, 10.36, 11.09),
    (0.14, 1.07, 11.92, 12.72),
    (0.14, 0.32, 11.09, 11.92),
    (0.88, 1.07, 11.09, 11.92),
]:
    box(pottery, (x, 0.91, z), (u, 1.0, w), True, 0.012)
for z in [10.36, 11.47]:
    box(ivory, (1.02, 0.1, z), (1.06, 0.81, z + 1.03), False, 0.012)
    box(sage, (1.06, 0.17, z + 0.06), (1.08, 0.74, z + 0.97), False, 0.01)
    box(brass, (1.08, 0.69, z + 0.38), (1.14, 0.725, z + 0.65), False, 0.008)
chassis = finish("appliance-steel", "999e9a", 0.36, 0.55)
room = part("oven-range", 0.62, 0, 9.77)
box(iron, (0.23, 0.035, 9.20), (1.025, 0.17, 10.35), True, 0.019)
box(chassis, (0.19, 0.15, 9.18), (1.058, 0.93, 10.35), True, 0.015)
box(pottery, (0.17, 0.92, 9.165), (1.072, 1.0, 10.358), True, 0.014)
box(iron, (0.24, 1.0, 9.36), (0.98, 1.035, 10.38), False, 0.022)
for x in [0.41, 0.80]:
    for z in [9.62, 10.09]:
        turned(iron, x, 1.036, z, [(0.126, 0), (0.127, 0.025), (0.08, 0.028)], 20)
room = part("sink-basin", 0.6, 1.0, 11.50)
box(iron, (0.32, 0.912, 11.09), (0.88, 0.934, 11.92), False, 0.012)
for xx in [0.32, 0.86]:
    box(iron, (xx, 0.93, 11.09), (xx + 0.02, 1.0, 11.92))
for zz in [11.09, 11.90]:
    box(iron, (0.32, 0.93, zz), (0.88, 1.0, zz + 0.02))
turned(brass, 0.60, 0.934, 11.49, [(0.042, 0), (0.043, 0.004), (0.018, 0.005)], 16)
room = part("brass-faucet", 0.465, 1.0, 11.965)
turned(
    brass,
    0.465,
    1.0,
    11.965,
    [(0.064, 0), (0.067, 0.01), (0.052, 0.022), (0.038, 0.03), (0.035, 0.19)],
    32,
    closed=True,
)
# Constant-radius gooseneck bends around a semicircle in the vertical plane.
points = []
for i in range(27):
    a = math.pi * i / 26
    for j in range(14):
        q = math.tau * j / 14
        radius = 0.112 + 0.018 * math.cos(q)
        points.append((0.465 + 0.018 * math.sin(q), 1.189 + radius * math.sin(a), 11.853 + radius * math.cos(a)))
emit(
    brass,
    points,
    [
        (i * 14 + j, i * 14 + (j + 1) % 14, (i + 1) * 14 + (j + 1) % 14, (i + 1) * 14 + j)
        for i in range(26)
        for j in range(14)
    ],
    True,
)
turned(brass, 0.465, 1.158, 11.741, [(0.024, 0), (0.024, 0.022), (0.018, 0.031)], 24)
turned(iron, 0.465, 1.157, 11.741, [(0.001, 0), (0.019, 0)], 24)
box(brass, (0.49, 1.10, 11.95), (0.55, 1.122, 11.977), False, 0.009)
box(brass, (0.535, 1.104, 11.952), (0.555, 1.191, 11.974), False, 0.009)
room = "Ground"
for y in [1.49, 1.92]:
    box(oak, (0.15, y, 10.42), (0.50, y + 0.064, 11.90))
    for i in range(5 if y < 1.8 else 2):
        turned(
            pottery,
            0.33,
            y + 0.065,
            (10.57 if y < 1.8 else 11.57) + i * 0.27,
            [(0.063, 0), (0.084, 0.14), (0.085, 0.17), (0.072, 0.17), (0.065, 0.05)],
        )
for k in range(4):
    y = 0.92 + 0.36 * k
    box(oak, (0.15, y, 4.28), (0.52, y + 0.06, 6.06))
    books(0.18, y + 0.065, 4.34 + k * 0.32, 3)
plant(4.03, 0.01, 0.58)
plant(0.56, 1.0, 12.32, 0.60)
room = part("dining-pendant", 2.37, RISE - 0.16, 7.51)
turned(brass, 2.37, 1.99, 7.51, [(0.38, 0), (0.35, 0.075), (0.22, 0.19), (0.075, 0.29), (0.065, 0.33)], 32)
turned(iron, 2.37, 2.30, 7.51, [(0.010, 0), (0.010, RISE - 2.45)], 8)
room = part("painted-fruit-bowl", 2.39, 0.79, 7.49)
turned(teal, 2.39, 0.79, 7.49, [(0.08, 0), (0.15, 0.06), (0.23, 0.16), (0.20, 0.16), (0.08, 0.04)], 24, closed=True)
apple = finish("apple-skin", "a84036", 0.36, texture=110)
pear = finish("pear-skin", "b1aa54", 0.49, texture=95)
for i, (dx, dz, h) in enumerate([(-0.075, -0.035, 0.108), (0.076, -0.026, 0.108), (0.014, 0.069, 0.138)]):
    room = part(("red-apple", "red-apple-small", "orchard-pear")[i], 2.39 + dx, 0.79 + h - 0.06, 7.49 + dz)
    x = 2.39 + dx
    z = 7.49 + dz
    bottom = 0.79 + h - 0.06
    if i == 2:
        profile = [
            (0.018, 0),
            (0.055, 0.017),
            (0.064, 0.052),
            (0.056, 0.087),
            (0.031, 0.12),
            (0.023, 0.147),
            (0.012, 0.153),
        ]
    else:
        profile = [
            (0.02, 0),
            (0.051, 0.011),
            (0.068, 0.041),
            (0.072, 0.068),
            (0.065, 0.092),
            (0.045, 0.11),
            (0.026, 0.115),
            (0.012, 0.105),
        ]
        if i == 1:
            profile = [(r * 0.91, y * 0.94) for r, y in profile]
    turned(apple if i != 2 else pear, x, bottom, z, profile, 32, closed=True)
    top = bottom + profile[-1][1]
    turned(walnut, x, top, z, [(0.005, 0), (0.004, 0.032)], 9, closed=True)
    if i == 2:
        leaf = [
            (x, top + 0.015, z),
            (x + 0.021, top + 0.033, z - 0.012),
            (x + 0.052, top + 0.05, z + 0.005),
            (x + 0.024, top + 0.038, z + 0.013),
        ]
        emit(green, leaf, [(0, 1, 2, 3), (3, 2, 1, 0)])

room = "Bedroom floor"
y = RISE
# Partition heads meet the underside of the slab above.
ceiling = y + RISE - 0.16
for a, b in [
    ((0.14, y, 8.45), (3.50, ceiling, 8.59)),
    ((4.85, y, 8.45), (5.10, ceiling, 8.59)),
    ((3.50, y + 2.35, 8.45), (4.85, ceiling, 8.59)),
    ((0.14, y, 4.15), (2.72, ceiling, 4.29)),
    ((2.60, y, 0.15), (2.74, ceiling, 2.72)),
]:
    box(plaster, a, b, True)
room = part("double-bed", 1.825, y, 11.35)
for x in [0.98, 2.67]:
    for z in [10.28, 12.34]:
        box(walnut, (x - 0.045, y, z - 0.045), (x + 0.045, y + 0.10, z + 0.045), False, 0.007)
box(walnut, (0.85, y + 0.08, 10.15), (2.80, y + 0.43, 12.48), True, 0.055)
box(oak, (0.80, y + 0.08, 12.40), (2.85, y + 1.22, 12.54), True, 0.04)
cushion(linen, (1.825, y + 0.51, 11.30), (0.97, 0.19, 1.18))
boxes.append([[0.86, y + 0.40, 10.16], [2.79, y + 0.67, 12.46]])
# Quilt drapes over the bed edge; small sinusoidal folds catch raking sunlight.
nx, nz = 35, 39
pts = []
for j in range(nz):
    t = j / (nz - 1)
    for i in range(nx):
        s = i / (nx - 1)
        edge = max(0, abs(s - 0.5) - 0.40) * 10
        h = (
            0.73
            - 0.26 * edge**1.5
            - 0.12 * max(0, 0.11 - t) * 9
            + 0.022 * math.sin(39 * s + 2 * math.sin(8 * t))
            + 0.012 * math.cos(23 * t + 4 * s)
        )
        pts.append((0.80 + 2.06 * s, y + h, 10.06 + 1.85 * t + 0.026 * math.sin(17 * s) * max(t, 0.2)))
emit(
    blue,
    pts,
    [
        (j * nx + i, (j + 1) * nx + i, (j + 1) * nx + i + 1, j * nx + i + 1)
        for j in range(nz - 1)
        for i in range(nx - 1)
    ],
    True,
)
for x in [1.36, 2.31]:
    cushion(linen, (x, y + 0.75, 12.10), (0.42, 0.145, 0.255))
room = part("bedside-table", 3.43, y, 12.09)
table(3.10, y, 11.75, 0.66, 0.68, 0.62)
room = part("bedside-lamp", 3.43, y + 0.62, 12.07)
turned(brass, 3.43, y + 0.62, 12.07, [(0.13, 0), (0.12, 0.04), (0.04, 0.065), (0.032, 0.31), (0.07, 0.37)])
turned(linen, 3.43, y + 0.95, 12.07, [(0.195, 0), (0.145, 0.26), (0.115, 0.265)], 24)
room = part("potted-plant", 0.57, y + 0.01, 9.28)
plant(0.57, y + 0.01, 9.28, 0.85)
room = part("bathtub", 0.78, y + 0.04, 2.19)
box(pottery, (0.29, y + 0.04, 1.16), (1.27, y + 0.20, 3.23), True, 0.06)
for x in [0.29, 1.12]:
    box(pottery, (x, y + 0.15, 1.16), (x + 0.15, y + 0.58, 3.23), True, 0.055)
for z in [1.16, 3.09]:
    box(pottery, (0.30, y + 0.15, z), (1.26, y + 0.58, z + 0.14), True, 0.05)
room = part("washstand", 2.01, y, 0.52)
box(sage, (1.56, y, 0.20), (2.46, y + 0.85, 0.84), True, 0.02)
turned(pottery, 2.02, y + 0.85, 0.55, [(0.12, 0), (0.25, 0.10), (0.25, 0.15), (0.21, 0.15), (0.06, 0.04)])
room = "Workroom"
y = RISE * 2
room = part("workroom-desk", 2.29, y, 1.1)
table(1.16, y, 0.58, 2.26, 1.04, 0.80)
room = part("desk-chair", 2.175, y, 2.075)
chair(1.90, y, 1.81)
room = part("desk-books", 1.32, y + 0.8, 0.65)
books(1.32, y + 0.8, 0.65, 5)
room = part("stocked-bookcase", 1.65, y, 12.00)
for k in range(6):
    h = y + 0.21 + 0.37 * k
    box(oak, (0.26, h, 11.85), (3.03, h + 0.06, 12.24))
    books(0.40, h + 0.06, 11.88, 12)
    books(1.75, h + 0.06, 11.88, 11)
for x in [0.20, 3.03]:
    box(walnut, (x, y, 11.80), (x + 0.07, y + 2.39, 12.25), True)
room = part("workroom-plant", 4.14, y + 0.01, 0.72)
plant(4.14, y + 0.01, 0.72)
room = part("workroom-chair", 3.095, y, 6.835)
chair(2.82, y, 6.57)

wear_random = random.Random(1939)
nickel = finish("satin-nickel", "b8b3a4", 0.32, 0.78)
seasoned_iron = finish("seasoned-iron", "474742", 0.72, 0.40)
smoked_enamel = finish("smoked-enamel", "2d3431", 0.19, 0.20)
worn_primer = finish("enamel-primer", "b1a08a", 0.71)
exposed_steel = finish("exposed-steel", "69594b", 0.60, 0.37)
oxide = finish("iron-oxide", "8c5232", 0.88, texture=110)
room = part("enamel-refrigerator", 1.685, 0.025, 12.22)
# Enamel refrigerator: individual doors, dark compression seals, brushed handles and grille.
enamel = finish("ivory-enamel", "e8e5da", 0.28, 0.17)
box(enamel, (1.24, 0.025, 11.84), (2.13, 2.06, 12.72), True, 0.055)
box(iron, (1.27, 0.15, 11.80), (2.10, 2.02, 11.86), False, 0.020)
for low, high in [(0.18, 0.69), (0.715, 2.025)]:
    box(enamel, (1.25, low, 11.72), (2.12, high, 11.84), True, 0.038)
# Project the two pulls far enough for fingers to clear the enamel. Their
# fixing plates are the places where the paint is most likely to fail.
front_u, front_v = (-1, 0, 0), (0, 1, 0)
for lo, hi in ((0.525, 0.667), (0.899, 1.072)):
    for h in (lo, hi):
        box(nickel, (1.347, h - 0.022, 11.709), (1.419, h + 0.022, 11.723), False, 0.008)
        for x in (1.358, 1.408):
            box(exposed_steel, (x - 0.005, h - 0.004, 11.705), (x + 0.005, h + 0.004, 11.710), False, 0.003)
            box(seasoned_iron, (x - 0.0035, h - 0.0008, 11.704), (x + 0.0035, h + 0.0008, 11.706))
        enamel_chip((1.375, h - 0.025, 11.718), front_u, front_v, 0.020, 0.007, wear_random)
    bent_tube(
        nickel,
        [
            (1.383, lo, 11.709),
            (1.383, lo + 0.014, 11.680),
            (1.383, lo + 0.034, 11.654),
            (1.383, lo + 0.050, 11.647),
            (1.383, hi - 0.044, 11.647),
            (1.383, hi - 0.022, 11.652),
            (1.383, hi - 0.007, 11.675),
            (1.383, hi, 11.709),
        ],
        0.013,
        (1, 0, 0),
        12,
    )
for x, y, rx, ry in [
    (1.308, 0.226, 0.024, 0.006),
    (2.061, 0.228, 0.010, 0.005),
    (1.309, 0.762, 0.021, 0.006),
    (2.061, 0.758, 0.014, 0.004),
    (1.519, 0.645, 0.010, 0.003),
]:
    enamel_chip((x, y, 11.718), front_u, front_v, rx, ry, wear_random)
    for _ in range(4):
        enamel_chip(
            (x + wear_random.uniform(-rx * 1.15, rx * 1.15), y + wear_random.uniform(0.005, 0.010), 11.718),
            front_u,
            front_v,
            0.002,
            0.0012,
            wear_random,
        )
box(nickel, (1.311, 0.038, 11.823), (2.063, 0.143, 11.843), False, 0.009)
for row in range(2):
    for i in range(24):
        x = 1.35 + i * 0.028
        y = 0.074 + row * 0.026
        box(seasoned_iron, (x, y, 11.819), (x + 0.017, y + 0.005, 11.825))
room = "oven-range"
# Oven glazing, trim, selector knobs and folded towel over the handle.
box(iron, (1.055, 0.18, 9.31), (1.11, 0.83, 10.32), False, 0.022)
box(smoked_enamel, (1.11, 0.25, 9.42), (1.125, 0.62, 10.21), False, 0.018)
box(brass, (1.13, 0.68, 9.41), (1.19, 0.72, 10.22), False, 0.012)
box(linen, (1.15, 0.43, 9.64), (1.196, 0.74, 9.96), False, 0.014)
for i in range(4):
    cushion(iron, (1.136, 0.779, 9.46 + 0.20 * i), (0.023, 0.042, 0.042))
for z in (9.34, 10.282):
    for h in (0.221, 0.798):
        box(nickel, (1.110, h - 0.006, z - 0.006), (1.115, h + 0.006, z + 0.006), False, 0.003)
        box(seasoned_iron, (1.1145, h - 0.001, z - 0.004), (1.116, h + 0.001, z + 0.004))
        enamel_chip((1.111, h - 0.008, z + 0.006), (0, 0, -1), (0, 1, 0), 0.009, 0.004, wear_random)
for cz in (9.62, 10.09):
    for cx in (0.41, 0.80):
        for arm in range(4):
            a = math.pi / 4 + arm * math.pi / 2
            path = [
                (cx + r * math.cos(a), h, cz + r * math.sin(a))
                for r, h in ((0.181, 1.034), (0.176, 1.048), (0.165, 1.057), (0.143, 1.058), (0.11, 1.058))
            ]
            bent_tube(seasoned_iron, path, 0.008, (-math.sin(a), 0, math.cos(a)), 8)
for z in (9.235, 10.289):
    for _ in range(9):
        enamel_chip(
            (1.044 + wear_random.uniform(-0.004, 0.004), 1.0008, z + wear_random.uniform(-0.035, 0.035)),
            (0, 0, 1),
            (1, 0, 0),
            wear_random.uniform(0.002, 0.009),
            wear_random.uniform(0.001, 0.003),
            wear_random,
        )
for i in range(4):
    z = 9.46 + i * 0.20
    box(pottery, (1.154, 0.789, z - 0.0015), (1.161, 0.809, z + 0.0015), False, 0.001)
    for tick in range(9):
        a = math.radians(-130 + 32.5 * tick)
        h = 0.779 + 0.052 * math.cos(a)
        u = z + 0.052 * math.sin(a)
        box(pottery, (1.110, h - 0.0025, u - 0.001), (1.112, h + 0.0025, u + 0.001))
room = part("stacked-plates", 0.60, 1.0, 10.71)
# Stacked ceramic dishes, a mixing bowl with an actual hollow profile, and mugs with loop handles.
for i in range(4):
    turned(
        pottery,
        0.60,
        1.00 + i * 0.020,
        10.71,
        [(0.078, 0), (0.147, 0.018), (0.15, 0.03), (0.13, 0.033), (0.06, 0.018)],
        24,
        True,
    )
room = part("mixing-bowl", 0.71, 1.0, 12.03)
turned(
    teal,
    0.71,
    1.0,
    12.03,
    [(0.065, 0), (0.135, 0.05), (0.173, 0.15), (0.168, 0.17), (0.15, 0.17), (0.115, 0.08), (0.04, 0.025)],
    28,
    True,
)
for xx, zz in [(2.18, 7.03), (2.89, 8.13)]:
    room = part("stoneware-mug" if xx < 2.5 else "stoneware-mug-rear", xx, 0.79, zz)
    turned(
        pottery,
        xx,
        0.79,
        zz,
        [(0.061, 0), (0.079, 0.025), (0.082, 0.147), (0.071, 0.15), (0.066, 0.036)],
        24,
        closed=True,
    )
    pts = []
    for i in range(17):
        a = i * math.tau / 16
        for j in range(7):
            b = j * math.tau / 7
            pts.append(
                (
                    xx + 0.084 + (0.056 + 0.013 * math.cos(b)) * math.cos(a),
                    0.87 + (0.059 + 0.013 * math.cos(b)) * math.sin(a),
                    zz + 0.013 * math.sin(b),
                )
            )
    emit(
        pottery,
        pts,
        [
            (i * 7 + j, i * 7 + (j + 1) % 7, (i + 1) * 7 + (j + 1) % 7, (i + 1) * 7 + j)
            for i in range(16)
            for j in range(7)
        ],
        True,
    )
room = "Ground"
# Hand-laid ceramic splashback with offset joints, extractor and a lidded cooking pot.
tile_shades = [finish("stoneware-glaze", c, 0.24) for c in ("dce2d6", "e9e8da", "d8dfd1")]
for row in range(5):
    z = 9.16
    for j in range(15):
        if z >= 12.66:
            break
        w = min(12.66, z + (0.14 if row % 2 and j == 0 else 0.28))
        box(
            rng.choice(tile_shades),
            (0.145, 1.015 + row * 0.115, z),
            (0.164, 1.126 + row * 0.115, w - 0.004),
            False,
            0.003,
        )
        z = w
room = part("range-hood", 0.49, 1.82, 9.825)
box(iron, (0.16, 1.82, 9.31), (0.82, 1.94, 10.34), False, 0.026)
box(enamel, (0.17, 1.94, 9.47), (0.53, RISE - 0.16, 10.19), False, 0.014)
for side in [9.42, 10.02]:
    box(pottery, (0.72, 1.816, side), (0.77, 1.824, side + 0.17))
room = part("enamel-stockpot", 0.79, 1.065, 9.65)
turned(
    teal, 0.79, 1.065, 9.65, [(0.115, 0), (0.15, 0.02), (0.151, 0.155), (0.155, 0.165), (0.145, 0.171)], 32, closed=True
)
turned(iron, 0.79, 1.228, 9.65, [(0.156, 0), (0.151, 0.009), (0.08, 0.038), (0.014, 0.039)], 32, closed=True)
turned(walnut, 0.79, 1.263, 9.65, [(0.027, 0), (0.025, 0.028), (0.035, 0.031)], 16)
for z in [9.45, 9.77]:
    box(iron, (0.75, 1.17, z), (0.83, 1.195, z + 0.08), False, 0.009)
room = part("chopping-board", 0.225, 1.0, 10.795)
box(oak, (0.20, 1.00, 10.58), (0.25, 1.38, 11.01), False, 0.023)
room = "oven-range"
# A striped cotton towel, looped over the oven rail with tiny ripples.
pts = []
for j in range(11):
    t = j / 10
    for i in range(15):
        q = i / 14
        pts.append(
            (
                1.198 + 0.014 * math.cos(6 * math.pi * q) + 0.018 * t,
                0.725 - 0.27 * t + 0.008 * math.sin(8 * q) * t,
                9.64 + q * 0.32,
            )
        )
for j in range(10):
    for i in range(14):
        k = j * 15 + i
        emit(
            blue if j in (7, 9) else linen, [pts[k], pts[k + 15], pts[k + 16], pts[k + 1]], [(0, 1, 2, 3), (3, 2, 1, 0)]
        )
# Hanging pleated linen on both edges of the bedroom window.
room = "Bedroom floor"
for x in [0.58, 4.15]:
    pts = [
        (x + i * 0.018, RISE + 0.78 + j * 1.88, 12.65 + 0.045 * math.cos(i * 0.85)) for j in range(2) for i in range(30)
    ]
    emit(
        linen,
        pts,
        [(i, 30 + i, 31 + i, i + 1) for i in range(29)] + [(i + 1, 31 + i, 30 + i, i) for i in range(29)],
        True,
    )

# A waxed pine bench in the rear window reveal. The patterned seat leaves the sash accessible.
room = part("window-seat", 3.50, 0.07, 12.40)
paint = finish("milk-paint", "93a69b", 0.61)
chalk = finish("eggshell-paint", "e5d9bd", 0.54)
stripe = finish("woven-indigo", "4a6174", 0.9)
flax = finish("flax-linen", "d9c69d", 0.94, texture=150)
ink = finish("blockprint-ink", "87452f", 0.86)
matboard = finish("rag-paper", "dfd0ac", 0.94)
box(paint, (2.39, 0.07, 12.04), (4.61, 0.42, 12.76), True, 0.026)
for i in range(3):
    x = 2.43 + i * 0.72
    box(chalk, (x, 0.12, 12.017), (x + 0.66, 0.38, 12.042), False, 0.012)
    box(paint, (x + 0.035, 0.16, 12.009), (x + 0.625, 0.35, 12.019), False, 0.009)
cushion(flax, (3.5, 0.45, 12.39), (1.13, 0.083, 0.35))
for i in range(3):
    cushion(stripe if i == 1 else ink, (2.83 + i * 0.63, 0.66, 12.62), (0.24, 0.21, 0.085))
room = part("roman-shade", 2.65, 2.65, 12.65)
# Overlapping folds of a half-raised woven Roman shade have a visible lining and selvage.
for i in range(5):
    low = 2.11 + 0.088 * i
    pts = []
    for j in range(7):
        t = j / 6
        for k in range(49):
            u = k / 48
            scallop = math.sin(math.pi * u * 3) ** 2
            pts.append(
                (
                    0.63 + u * 4.04,
                    low + t * 0.145 - 0.018 * scallop * (1 - t),
                    12.66 - 0.052 * math.sin(math.pi * t) - 0.008 * scallop,
                )
            )
    emit(
        flax,
        pts,
        [(j * 49 + k, j * 49 + k + 1, (j + 1) * 49 + k + 1, (j + 1) * 49 + k) for j in range(6) for k in range(48)],
        True,
    )
    for x in [0.78, 4.46]:
        box(stripe, (x, low, 12.592), (x + 0.036, low + 0.089, 12.603))
for x in [4.65, 4.674]:
    box(chalk, (x, 1.57, 12.65), (x + 0.008, 2.65, 12.66))
box(walnut, (4.638, 1.54, 12.64), (4.693, 1.58, 12.67), False, 0.009)

room = "Ground"
# Tongue-and-groove panelling, a continuous chair rail and a stepped picture rail.
for i in range(51):
    x = 0.19 + i * 0.131
    box(paint, (x, 0.13, 12.78), (x + 0.127, 0.80, 12.846))
box(chalk, (0.16, 0.79, 12.737), (6.85, 0.85, 12.85), False, 0.014)
for yy, depth, height in [(2.79, 0.085, 0.052), (3.02, 0.072, 0.056), (3.075, 0.111, 0.054), (3.125, 0.15, 0.043)]:
    box(chalk, (0.14, yy, 12.85 - depth), (6.86, yy + height, 12.85), False, 0.008)
    box(chalk, (0.14, yy, 0.15), (0.14 + depth, yy + height, 12.74), False, 0.008)
room = part("framed-coastal-print", 5.58, 1.765, 12.82)
# Original two-colour linocut of the coastal hills, mounted and framed on the back wall.
x0, x1, y0, y1 = 4.87, 6.29, 1.13, 2.40
box(walnut, (x0 - 0.043, y0 - 0.043, 12.755), (x1 + 0.043, y1 + 0.043, 12.82), False, 0.01)
box(matboard, (x0, y0, 12.744), (x1, y1, 12.76))
a, b = x0 + 0.12, x1 - 0.12
bottom, top = y0 + 0.13, y1 - 0.13
skyink = finish("print-ochre", "d3ad78", 0.93)
box(skyink, (a, bottom, 12.735), (b, top, 12.744))
for band, (mat, mid) in enumerate([(ink, 0.48), (paint, 0.32), (stripe, 0.19)]):
    pts = []
    for j in range(41):
        t = j / 40
        height = mid + 0.105 * math.sin(t * 6.2 + band * 1.81) + 0.034 * math.sin(t * 18.5 + band)
        for yy in [bottom, bottom + height]:
            pts.append((a + t * (b - a), yy, 12.731 - band * 0.003))
    emit(mat, pts, [(2 * j, 2 * j + 1, 2 * j + 3, 2 * j + 2) for j in range(40)])
pts = [
    (a + 0.74 + 0.098 * math.cos(j * math.tau / 40), top - 0.22 + 0.098 * math.sin(j * math.tau / 40), 12.728)
    for j in range(40)
]
emit(chalk, pts, [tuple(reversed(range(40)))])
# Jar of dry pasta, pantry tins and an irregular stack of cookbooks behind the cooktop.
for j, (zz, mat, h) in enumerate([(10.60, ink, 0.26), (10.91, flax, 0.31), (11.25, stripe, 0.20)]):
    room = part(f"pantry-canister-{j + 1}", 0.34, 2.0, zz)
    turned(mat, 0.34, 2.00, zz, [(0.082, 0), (0.091, 0.022), (0.091, h - 0.025), (0.082, h)], 24)
    turned(walnut, 0.34, 2.00 + h, zz, [(0.094, 0), (0.095, 0.018), (0.087, 0.025)], 24)

# Table linen bends over both short edges. The two woven borders follow the folds.
room = part("striped-table-runner", 2.40, 0.796, 7.575)


def runner_surface(u, v):
    z = 6.45 + v * 2.24
    end = max(6.70 - z, z - 8.45, 0.0)
    zz = min(8.45, max(6.70, z)) + (z - min(8.45, max(6.70, z))) * 0.17
    xx = 2.14 + u * 0.52 + 0.011 * math.sin(v * 10 + u * 2)
    return (xx, 0.796 - end * 0.95 + 0.0045 * math.sin(u * 24 + v * 7) + end * 0.028 * math.sin(u * 22), zz)


edges = [0, 0.022, 0.036, 0.048, 0.066, 0.080, 0.20, 0.36, 0.53, 0.69, 0.86, 0.92, 0.934, 0.952, 0.964, 0.978, 1.0]
for i, (a, b) in enumerate(zip(edges, edges[1:])):
    pts = [runner_surface(u, j / 52) for j in range(53) for u in (a, b)]
    col = stripe if i in [1, 3, 11, 13] else flax
    emit(col, pts, [(j * 2, j * 2 + 2, j * 2 + 3, j * 2 + 1) for j in range(52)], True)
for j in range(36):
    for v in [0, 1]:
        x, y, z = runner_surface(j / 35, v)
        box(flax, (x - 0.002, y - 0.032 - 0.01 * math.sin(j * 4), z - 0.002), (x + 0.002, y, z + 0.002))
room = "enamel-refrigerator"
# Hand-written list under a little round fridge magnet.
for x, y, w, h in [(1.70, 1.52, 0.18, 0.20), (1.90, 1.70, 0.13, 0.11)]:
    box(matboard, (x, y, 11.711), (x + w, y + h, 11.719))
    for j in range(4):
        box(
            stripe,
            (x + 0.02, y + 0.028 + j * 0.021, 11.708),
            (x + w * (0.58 + 0.15 * math.sin(j)), y + 0.032 + j * 0.021, 11.712),
        )
    cushion(ink, (x + w * 0.5, y + h - 0.006, 11.701), (0.017, 0.017, 0.012))

# Breakfast left on the table: an open, illustrated recipe book with bowed pages.
room = "Breakfast on the table"
coffee = finish("dark-roast", "39251c", 0.18)
for x, z in [(2.18, 7.03), (2.89, 8.13)]:
    room = "stoneware-mug" if x < 2.5 else "stoneware-mug-rear"
    turned(coffee, x, 0.907, z, [(0.001, 0), (0.068, 0), (0.069, 0.002)], 32)
    turned(pottery, x, 0.907, z, [(0.068, 0), (0.069, 0.002)], 32)
room = part("open-recipe-book", 2.576, 0.803, 6.92)
cover = finish("bookcloth", "475752", 0.84)
page = finish("uncoated-paper", "e6d9bb", 0.94)
x0, z0 = 2.576, 6.92


# The binding is open and rotated slightly toward the reader.
def page_point(x, y, z):
    a = -0.19
    return (x0 + x * math.cos(a) - z * math.sin(a), y, z0 + x * math.sin(a) + z * math.cos(a))


for side in [-1, 1]:
    pts = []

    def height(t):
        return 0.809 + 0.016 * math.sin(math.pi * t) + 0.003 * t

    for k in range(11):
        t = k / 10
        for z in [0, 0.302]:
            pts.append(page_point(side * (0.018 + 0.205 * t), height(t), z))
    quads = [(2 * j, 2 * j + 1, 2 * j + 3, 2 * j + 2) for j in range(10)]
    emit(page, pts, quads + [(d, c, b, a) for a, b, c, d in quads])
    for j in range(5):
        q = [
            page_point(side * x, 0.803 + j * 0.001, z)
            for x, z in [(0, -0.004), (0.231, -0.004), (0.231, 0.307), (0, 0.307)]
        ]
        emit(cover if j == 0 else page, q, [(0, 1, 2, 3), (3, 2, 1, 0)])
    for j in range(13):
        if side == 1 and 3 < j < 10:
            continue
        stop = 0.185 - 0.038 * (0.5 + 0.5 * math.sin(17 * j))
        pts = []
        for k in range(10):
            x = 0.038 + (stop - 0.038) * k / 9
            for dz in [0, 0.0027 if j else 0.006]:
                pts.append(page_point(side * x, height((x - 0.018) / 0.205) + 0.0006, 0.025 + j * 0.019 + dz))
        q = [(k * 2, k * 2 + 1, k * 2 + 3, k * 2 + 2) for k in range(9)]
        emit(ink if j == 0 else cover, pts, q + [(d, c, b, a) for a, b, c, d in q])
# A botanical illustration: two small citrus fruits and pointed leaves on the right-hand page.
for cx, cz, r, mat in [(0.11, 0.153, 0.034, ink), (0.149, 0.129, 0.027, ink), (0.090, 0.120, 0.015, paint)]:
    pts = []
    for k in range(24):
        a = k * math.tau / 24
        x = cx + r * math.cos(a)
        pts.append(page_point(x, height((x - 0.018) / 0.205) + 0.0008, cz + r * math.sin(a)))
    emit(mat, pts, [tuple(range(24)), tuple(reversed(range(24)))])
room = "painted-fruit-bowl"
# Curved bands of a hand-painted rim, plus small flecks where the glaze has worn.
turned(chalk, 2.39, 0.79, 7.49, [(0.219, 0.141), (0.228, 0.151)], 48)
turned(ink, 2.39, 0.79, 7.49, [(0.225, 0.148), (0.229, 0.152)], 48)
for k in range(21):
    a = k * math.tau / 21
    fleck = []
    for j in range(10):
        t = j * math.tau / 10
        h = 0.127 + 0.006 * math.sin(t)
        radius = 0.15 + 0.8 * (h - 0.06) + 0.001
        angle = a + 0.027 * math.cos(t)
        fleck.append((2.39 + radius * math.cos(angle), 0.79 + h, 7.49 + radius * math.sin(angle)))
    emit(chalk, fleck, [tuple(reversed(range(10)))])
room = "dining-pendant"
# A thin brass inner lip and the frosted globe beneath the dining shade.
turned(chalk, 2.37, 1.989, 7.51, [(0.379, 0), (0.367, 0.012), (0.21, 0.15)], 48)
bulb = finish("opal-lamp-glass", "f5dbac", 0.34)
bs = materials[bulb].node_tree.nodes.get("Principled BSDF")
bs.inputs["Emission Color"].default_value = (1, 0.77, 0.42, 1)
bs.inputs["Emission Strength"].default_value = 2.4
cushion(bulb, (2.37, 2.045, 7.51), (0.11, 0.087, 0.11))

component_meshes = {}
for (room_name, mat_name), (points, faces, smooth) in batches.items():
    mesh = bpy.data.meshes.new(f"{room_name} - {mat_name}")
    mesh.from_pydata(points, [], faces)
    mesh.materials.append(materials[mat_name])
    for face, shade in zip(mesh.polygons, smooth):
        face.use_smooth = shade
    mesh.update()
    obj = bpy.data.objects.new(mesh.name, mesh)
    bpy.context.collection.objects.link(obj)
    if room_name in pivots:
        component_meshes.setdefault(room_name, []).append(obj)

beaver_lighting.bake(list(bpy.context.scene.objects), RISE)

parser = argparse.ArgumentParser()
parser.add_argument("--parts", type=Path, help="Directory outside the checkout for reusable component GLBs")
parser.add_argument("--blend", type=Path)
parser.add_argument("--render", type=Path, help="Optional kitchen photograph rendered from the editable scene")
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else [])
if args.blend:
    if args.blend.resolve().is_relative_to(ROOT):
        raise ValueError("The editable Blender scene belongs in the off-repository asset store.")
    args.blend.parent.mkdir(parents=True, exist_ok=True)

if args.parts:
    if args.parts.resolve().is_relative_to(ROOT):
        raise ValueError("Standalone source components belong outside the checkout.")
    args.parts.mkdir(parents=True, exist_ok=True)

components = {}
for name, meshes in component_meshes.items():
    pivot = pivots[name]
    collection = bpy.data.collections.new(name)
    bpy.context.scene.collection.children.link(collection)
    collection.instance_offset = pivot
    collection.asset_mark()
    handle = bpy.data.objects.new(name, None)
    handle.location = pivot
    handle.empty_display_size = 0.12
    collection.objects.link(handle)
    for obj in meshes:
        bpy.context.collection.objects.unlink(obj)
        collection.objects.link(obj)
        obj.data.transform(Matrix.Translation(-pivot))
        obj.parent = handle
    components[name] = (handle, meshes)

OUT.mkdir(parents=True, exist_ok=True)
glb = OUT / "terrace.glb"
grain_links = []
for mat in materials.values():
    slot = mat.node_tree.nodes.get("Principled BSDF").inputs["Base Color"]
    if slot.is_linked:
        source = slot.links[0].from_socket
        mat.node_tree.links.remove(slot.links[0])
        grain_links.append((mat, source, slot))
bpy.ops.export_scene.gltf(
    filepath=str(glb),
    export_format="GLB",
    export_texcoords=True,
    export_yup=True,
    export_animations=False,
    export_extras=False,
)
manifest_source = "Original Blender model generated by scripts/blender-buildings/build_beaver_terrace.py"
if args.parts:
    entries = []
    for name, (root, meshes) in sorted(components.items()):
        path = args.parts / (name + ".glb")
        bpy.ops.object.select_all(action="DESELECT")
        for obj in [root, *meshes]:
            obj.select_set(True)
        placed = root.location.copy()
        root.location = (0, 0, 0)
        try:
            bpy.ops.export_scene.gltf(
                filepath=str(path),
                export_format="GLB",
                use_selection=True,
                export_texcoords=False,
                export_yup=True,
                export_animations=False,
                export_extras=False,
            )
        finally:
            root.location = placed
        content = path.read_bytes()
        entries.append(
            {
                "asset": path.name,
                "sha256": hashlib.sha256(content).hexdigest(),
                "bytes": len(content),
                "pivot_in_house_m": [placed.x, placed.z, -placed.y],
            }
        )
    (args.parts / "components.json").write_text(
        json.dumps(
            {
                "source": manifest_source,
                "license": "MIT",
                "attribution": "Earth Online contributors",
                "units": "metres, Y up",
                "components": entries,
            },
            indent=2,
        )
        + "\n"
    )
for mat, source, slot in grain_links:
    mat.node_tree.links.new(source, slot)
raw = glb.read_bytes()
manifest = {
    "source": manifest_source,
    "license": "MIT",
    "attribution": "Earth Online contributors. Siting from the recorded OpenStreetMap footprint in sf-city.json.gz.",
    "url": "/assets/buildings/beaver-terrace/terrace.glb",
    "sha256": hashlib.sha256(raw).hexdigest(),
    "bytes": len(raw),
    "boxes": boxes,
    "ramps": ramps,
}
(OUT / "terrace.provenance.json").write_text(json.dumps(manifest, indent=1) + "\n")


# These objects belong to the editable review scene. The portable GLB above contains the house alone.
scene = bpy.context.scene


def point_at(obj, target):
    obj.rotation_euler = (Vector(target) - obj.location).to_track_quat("-Z", "Y").to_euler()


def softbox(name, position, target, power, size, color):
    data = bpy.data.lights.new(name, "AREA")
    data.energy = power
    data.shape = "DISK"
    data.size = size
    data.color = color
    obj = bpy.data.objects.new(name, data)
    scene.collection.objects.link(obj)
    obj.location = position
    point_at(obj, target)


softbox("Rear window daylight", (2.5, -12.7, 2.22), (1.5, -7, 0.9), 270, 3.6, (0.86, 0.92, 1.0))
softbox("Light through the front sash", (2.5, -0.17, 2.15), (2.5, -8, 1.1), 480, 3.9, (1.0, 0.86, 0.70))
softbox("Dining pendant", (2.37, -7.51, 1.99), (2.37, -7.51, 0.78), 36, 0.43, (1, 0.78, 0.53))
softbox("Ceiling bounce", (4.4, -7.5, 2.93), (1.1, -10, 1.2), 115, 3.3, (1, 0.93, 0.83))
camera = bpy.data.cameras.new("Kitchen camera")
obj = bpy.data.objects.new(camera.name, camera)
scene.collection.objects.link(obj)
obj.location = (4.46, -5.47, 1.61)
point_at(obj, (1.18, -10.7, 1.27))
camera.lens = 26
scene.camera = obj
scene.world.color = (0.45, 0.56, 0.68)
scene.render.engine = "CYCLES"
scene.cycles.samples = 40
scene.cycles.use_denoising = True
scene.render.resolution_x = 1400
scene.render.resolution_y = 900
scene.render.resolution_percentage = 100
if args.blend:
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(args.blend.resolve()))
if args.render:
    args.render.parent.mkdir(parents=True, exist_ok=True)
    scene.render.filepath = str(args.render.resolve())
    bpy.ops.render.render(write_still=True)
