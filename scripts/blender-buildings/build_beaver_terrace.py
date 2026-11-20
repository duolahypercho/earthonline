"""Build 21 Beaver Street. Run with Blender --background --factory-startup --python this_file.
Optional arguments after --: --blend PATH (editable source outside the checkout).
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
from mathutils import Vector

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
    if name == "wood-siding":
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
plaster = finish("scanned-painted-plaster", "efe6d6", texture=80)
ivory = finish("painted-white", "ece1c8")
oak = finish("wood-siding", "b5895c", texture=9)
boards = [finish("wood-siding", c, texture=12) for c in ("c69b72", "bb9167", "c49e78", "c19974")]
walnut = finish("wood-siding", "785338", texture=14)
rust = finish("canvas-plain", "b66d4d", texture=105)
linen = finish("canvas-plain", "e6dcc3", texture=110)
blue = finish("canvas-plain", "708998", texture=100)
teal = finish("glazed-tile", "527775", 0.27)
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
    polygons.extend(tuple(offset + i for i in f) for f in faces)
    flags.extend([smooth] * len(faces))


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


def turned(mat, x, y, z, profile, n=16):
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
    room = ("Ground", "Bedroom floor", "Workroom")[level]
    y = level * RISE
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

room = "Exterior"
box(plaster, (0.14, 3 * RISE - 0.10, 0.15), (6.86, 3 * RISE - 0.02, 12.85))
box(roof, (-0.05, 3 * RISE, -0.10), (7.05, 3 * RISE + 0.15, 13.04), True)
for x in [0, 6.85]:
    box(ivory, (x, 10.15, 0), (x + 0.15, 10.37, 13.03))
for z in [0, 12.85]:
    box(ivory, (0, 10.15, z), (7, 10.37, z + 0.15))
box(stone, (0, -2.4, 0), (7, -0.16, 13), True)
box(sage, (6.58, 0, 0.15), (6.69, 2.35, 1.56), True, 0.012)
for h in [0.2, 1.12]:
    box(ivory, (6.55, h, 0.31), (6.58, h + 0.83, 1.39), False, 0.02)
    box(sage, (6.52, h + 0.045, 0.36), (6.55, h + 0.78, 1.34), False, 0.014)
turned(brass, 6.47, 1.02, 1.36, [(0.05, 0), (0.05, 0.065)])
for x in [5.12, 6.68]:
    box(ivory, (x, 0, -0.07), (x + 0.09, 2.51, 0.19), False, 0.008)
box(ivory, (5.02, 2.43, -0.14), (6.86, 2.56, 0.17), False, 0.015)

room = "Ground"
box(rust, (0.60, 0.10, 1.51), (1.46, 0.42, 3.89), True, 0.07)
cushion(rust, (0.63, 0.66, 2.70), (0.20, 0.42, 1.18))
for z in [1.91, 2.71, 3.49]:
    cushion(rust, (1.05, 0.49, z), (0.44, 0.145, 0.40))
for z in [1.52, 3.89]:
    cushion(rust, (1.04, 0.57, z), (0.51, 0.27, 0.14))
for z, m in [(1.99, linen), (3.32, blue)]:
    cushion(m, (0.81, 0.79, z), (0.16, 0.26, 0.28))
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
table(2.05, 0, 2.0, 0.91, 1.4, 0.44)
box(blue, (2.21, 0.44, 2.25), (2.65, 0.49, 2.83), False, 0.01)
table(1.7, 0, 6.7, 1.35, 1.75, 0.79)
chair(2.1, 0, 6.08)
chair(2.1, 0, 8.64)
box(sage, (0.16, 0, 9.18), (1.02, 0.91, 12.68), True)
# Continuous counter with an inset sink cut through its rear section.
for x, u, z, w in [
    (0.14, 1.07, 9.13, 11.09),
    (0.14, 1.07, 11.92, 12.72),
    (0.14, 0.32, 11.09, 11.92),
    (0.88, 1.07, 11.09, 11.92),
]:
    box(pottery, (x, 0.91, z), (u, 1.0, w), True, 0.012)
for z in [9.25, 10.36, 11.47]:
    box(ivory, (1.02, 0.1, z), (1.06, 0.81, z + 1.03), False, 0.012)
    box(sage, (1.06, 0.17, z + 0.06), (1.08, 0.74, z + 0.97), False, 0.01)
    box(brass, (1.08, 0.69, z + 0.38), (1.14, 0.725, z + 0.65), False, 0.008)
box(iron, (0.24, 1.0, 9.36), (0.98, 1.035, 10.38), False, 0.022)
for x in [0.41, 0.80]:
    for z in [9.62, 10.09]:
        turned(iron, x, 1.036, z, [(0.126, 0), (0.127, 0.025), (0.08, 0.028)], 20)
box(iron, (0.32, 0.912, 11.09), (0.88, 0.934, 11.92), False, 0.012)
for xx in [0.32, 0.86]:
    box(iron, (xx, 0.93, 11.09), (xx + 0.02, 1.0, 11.92))
for zz in [11.09, 11.90]:
    box(iron, (0.32, 0.93, zz), (0.88, 1.0, zz + 0.02))
turned(brass, 0.60, 0.934, 11.49, [(0.042, 0), (0.043, 0.004), (0.018, 0.005)], 16)
box(brass, (0.44, 1.01, 11.94), (0.49, 1.38, 11.99), False, 0.015)
box(brass, (0.44, 1.34, 11.74), (0.49, 1.39, 11.98), False, 0.015)
for y in [1.49, 1.92]:
    box(oak, (0.15, y, 10.42), (0.50, y + 0.064, 11.90))
    for i in range(5):
        turned(
            pottery,
            0.33,
            y + 0.065,
            10.57 + i * 0.27,
            [(0.063, 0), (0.084, 0.14), (0.085, 0.17), (0.072, 0.17), (0.065, 0.05)],
        )
for k in range(4):
    y = 0.92 + 0.36 * k
    box(oak, (0.15, y, 4.28), (0.52, y + 0.06, 6.06))
    books(0.18, y + 0.065, 4.34 + k * 0.32, 3)
plant(4.03, 0.01, 0.58)
plant(0.56, 1.0, 12.32, 0.60)
turned(brass, 2.37, 1.99, 7.51, [(0.38, 0), (0.35, 0.075), (0.22, 0.19), (0.075, 0.29), (0.065, 0.33)], 32)
turned(iron, 2.37, 2.30, 7.51, [(0.010, 0), (0.010, RISE - 2.45)], 8)
turned(teal, 2.39, 0.79, 7.49, [(0.08, 0), (0.15, 0.06), (0.23, 0.16), (0.20, 0.16), (0.08, 0.04)], 24)
apple = finish("apple-skin", "a84036", 0.36, texture=110)
pear = finish("pear-skin", "b1aa54", 0.49, texture=95)
for i, (dx, dz, h) in enumerate([(-0.075, -0.035, 0.108), (0.076, -0.026, 0.108), (0.014, 0.069, 0.138)]):
    x = 2.39 + dx
    z = 7.49 + dz
    turned(
        apple if i != 2 else pear,
        x,
        0.79 + h - 0.06,
        z,
        [(0.02, 0), (0.063, 0.022), (0.071, 0.064), (0.059, 0.097), (0.026, 0.115), (0.012, 0.107)],
        20,
    )
    turned(walnut, x, 0.79 + h + 0.044, z, [(0.006, 0), (0.004, 0.032)], 7)

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
table(3.10, y, 11.75, 0.66, 0.68, 0.62)
turned(brass, 3.43, y + 0.62, 12.07, [(0.13, 0), (0.12, 0.04), (0.04, 0.065), (0.032, 0.31), (0.07, 0.37)])
turned(linen, 3.43, y + 0.95, 12.07, [(0.195, 0), (0.145, 0.26), (0.115, 0.265)], 24)
plant(0.57, y + 0.01, 9.28, 0.85)
box(pottery, (0.29, y + 0.04, 1.16), (1.27, y + 0.20, 3.23), True, 0.06)
for x in [0.29, 1.12]:
    box(pottery, (x, y + 0.15, 1.16), (x + 0.15, y + 0.58, 3.23), True, 0.055)
for z in [1.16, 3.09]:
    box(pottery, (0.30, y + 0.15, z), (1.26, y + 0.58, z + 0.14), True, 0.05)
box(sage, (1.56, y, 0.20), (2.46, y + 0.85, 0.84), True, 0.02)
turned(pottery, 2.02, y + 0.85, 0.55, [(0.12, 0), (0.25, 0.10), (0.25, 0.15), (0.21, 0.15), (0.06, 0.04)])
room = "Workroom"
y = RISE * 2
table(1.16, y, 0.58, 2.26, 1.04, 0.80)
chair(1.90, y, 1.81)
books(1.32, y + 0.8, 0.65, 5)
for k in range(6):
    h = y + 0.21 + 0.37 * k
    box(oak, (0.26, h, 11.85), (3.03, h + 0.06, 12.24))
    books(0.40, h + 0.06, 11.88, 12)
    books(1.75, h + 0.06, 11.88, 11)
for x in [0.20, 3.03]:
    box(walnut, (x, y, 11.80), (x + 0.07, y + 2.39, 12.25), True)
plant(4.14, y + 0.01, 0.72)
chair(2.82, y, 6.57)

room = "Kitchen fittings"
# Enamel refrigerator: individual doors, dark compression seals, brushed handles and grille.
enamel = finish("ivory-enamel", "e8e5da", 0.28, 0.17)
box(enamel, (1.24, 0.025, 11.84), (2.13, 2.06, 12.72), True, 0.055)
box(iron, (1.27, 0.15, 11.80), (2.10, 2.02, 11.86), False, 0.020)
for low, high in [(0.18, 0.69), (0.715, 2.025)]:
    box(enamel, (1.25, low, 11.72), (2.12, high, 11.84), True, 0.038)
    h = high - 0.18 if high < 1 else low + 0.14
    box(brass, (1.36, h, 11.64), (1.40, h + 0.17, 11.72), False, 0.016)
for i in range(8):
    box(iron, (1.36 + i * 0.083, 0.067, 11.832), (1.41 + i * 0.083, 0.108, 11.844))
# Oven glazing, trim, selector knobs and folded towel over the handle.
box(iron, (1.055, 0.18, 9.31), (1.11, 0.83, 10.32), False, 0.022)
box(blue, (1.11, 0.25, 9.42), (1.125, 0.62, 10.21), False, 0.018)
box(brass, (1.13, 0.68, 9.41), (1.19, 0.72, 10.22), False, 0.012)
box(linen, (1.15, 0.43, 9.64), (1.196, 0.74, 9.96), False, 0.014)
for i in range(4):
    cushion(iron, (1.136, 0.779, 9.46 + 0.20 * i), (0.023, 0.042, 0.042))
# Stacked ceramic dishes, a mixing bowl with an actual hollow profile, and mugs with loop handles.
for i in range(4):
    turned(
        pottery,
        0.60,
        1.00 + i * 0.020,
        10.71,
        [(0.078, 0), (0.147, 0.018), (0.15, 0.03), (0.13, 0.033), (0.06, 0.018)],
        24,
    )
turned(
    teal,
    0.71,
    1.0,
    12.03,
    [(0.065, 0), (0.135, 0.05), (0.173, 0.15), (0.168, 0.17), (0.15, 0.17), (0.115, 0.08), (0.04, 0.025)],
    28,
)
for xx, zz in [(2.18, 7.03), (2.89, 8.13)]:
    turned(pottery, xx, 0.79, zz, [(0.061, 0), (0.079, 0.025), (0.082, 0.147), (0.071, 0.15), (0.066, 0.036)], 24)
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
# Hand-laid ceramic splashback with offset joints, extractor and a lidded cooking pot.
tile_shades = [finish("glazed-tile", c, 0.24) for c in ("dce2d6", "e9e8da", "d8dfd1")]
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
box(iron, (0.16, 1.82, 9.31), (0.82, 1.94, 10.34), False, 0.026)
box(enamel, (0.17, 1.94, 9.47), (0.53, RISE - 0.16, 10.19), False, 0.014)
for side in [9.42, 10.02]:
    box(pottery, (0.72, 1.816, side), (0.77, 1.824, side + 0.17))
turned(teal, 0.79, 1.065, 9.65, [(0.115, 0), (0.15, 0.02), (0.151, 0.155), (0.155, 0.165), (0.145, 0.171)], 32)
turned(iron, 0.79, 1.228, 9.65, [(0.156, 0), (0.151, 0.009), (0.08, 0.038), (0.014, 0.039)], 32)
turned(walnut, 0.79, 1.263, 9.65, [(0.027, 0), (0.025, 0.028), (0.035, 0.031)], 16)
for z in [9.45, 9.77]:
    box(iron, (0.75, 1.17, z), (0.83, 1.195, z + 0.08), False, 0.009)
box(oak, (0.20, 1.00, 10.58), (0.25, 1.38, 11.01), False, 0.023)
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

for (room_name, mat_name), (points, faces, smooth) in batches.items():
    mesh = bpy.data.meshes.new(f"{room_name} - {mat_name}")
    mesh.from_pydata(points, [], faces)
    mesh.materials.append(materials[mat_name])
    for face, shade in zip(mesh.polygons, smooth):
        face.use_smooth = shade
    mesh.update()
    obj = bpy.data.objects.new(mesh.name, mesh)
    bpy.context.collection.objects.link(obj)

parser = argparse.ArgumentParser()
parser.add_argument("--blend", type=Path)
parser.add_argument("--render", type=Path, help="Optional kitchen photograph rendered from the editable scene")
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else [])
if args.blend:
    if args.blend.resolve().is_relative_to(ROOT):
        raise ValueError("The editable Blender scene belongs in the off-repository asset store.")
    args.blend.parent.mkdir(parents=True, exist_ok=True)

OUT.mkdir(parents=True, exist_ok=True)
glb = OUT / "terrace.glb"
bpy.ops.export_scene.gltf(
    filepath=str(glb),
    export_format="GLB",
    export_texcoords=False,
    export_yup=True,
    export_animations=False,
    export_extras=False,
)
raw = glb.read_bytes()
manifest = {
    "source": "Original Blender model generated by scripts/blender-buildings/build_beaver_terrace.py",
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
