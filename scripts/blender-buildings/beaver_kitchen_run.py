"""Extend the preparation surface with three 500 mm cabinets at the dining end."""

import math
import random

import bpy
from beaver_dining_parts import lathe, material, mesh
from mathutils import Vector


def box(group, name, low, high, mat, bevel=0):
    x, y, z = low
    u, v, w = high
    verts = [
        (x, y, z),
        (u, y, z),
        (u, v, z),
        (x, v, z),
        (x, y, w),
        (u, y, w),
        (u, v, w),
        (x, v, w),
    ]
    faces = [
        (3, 2, 1, 0),
        (4, 5, 6, 7),
        (0, 1, 5, 4),
        (1, 2, 6, 5),
        (2, 3, 7, 6),
        (3, 0, 4, 7),
    ]
    obj = mesh(group, name, verts, faces, [mat], smooth=False)
    if bevel:
        b = obj.modifiers.new("Eased joinery edges", "BEVEL")
        b.width = bevel
        b.segments = 3
        normal = obj.modifiers.new("Face normals", "WEIGHTED_NORMAL")
        normal.keep_sharp = True
    return obj


def extend_kitchen(scene, group):
    shift = 1.50
    cabinet = scene.objects["base-cabinets"]
    # The sink cabinet and the existing near prep cabinet are disconnected meshes
    # batched by finish. Their gap is occupied by the freestanding range.
    for ob in cabinet.children:
        m = ob.matrix_world.copy()
        undo = m.inverted()
        for poly in ob.data.polygons:
            ys = [(m @ ob.data.vertices[i].co).y for i in poly.vertices]
            if min(ys) < -13.78 < max(ys):
                raise ValueError("The prep cabinet now crosses the range opening")
        for v in ob.data.vertices:
            p = m @ v.co
            if p.y > -13.78:
                p.y += shift
                v.co = undo @ p
        ob.data.update()
    for key, d in [
        ("oven-range", shift),
        ("range-hood", shift),
        ("enamel-stockpot", shift),
        ("oak-sideboard", 0.722),
    ]:
        ob = scene.objects[key]
        ob.location.y += d
    bpy.context.view_layer.update()

    def finish(prefix):
        return next(o.data.materials[0] for o in cabinet.children if prefix in o.name)

    frame = finish("cabinet-enamel@6c")
    front = finish("cabinet-enamel@8c")
    counter = finish("porcelain")
    brass = finish("brushed-brass")
    wood = finish("furniture-oak@99")
    y0 = -14.363

    def part(label, a, b, mat, r=0.003):
        return box(group, label, a, b, mat, r)

    part(
        "Three-bay kitchen plinth",
        (0.578, y0, 0.012),
        (1.135, y0 + shift, 0.104),
        frame,
    )
    part(
        "Painted kitchen carcass", (0.566, y0, 0.099), (1.207, y0 + shift, 0.876), frame
    )
    part(
        "Honed worktop with drip edge",
        (0.551, y0 - 0.002, 0.875),
        (1.239, y0 + shift + 0.001, 0.920),
        counter,
        0.007,
    )
    for i in range(3):
        y = y0 + 0.5 * i
        part(
            f"Drawer front {i + 1}",
            (1.202, y + 0.008, 0.696),
            (1.224, y + 0.492, 0.860),
            front,
            0.004,
        )
        part(
            f"Recessed panel {i + 1}",
            (1.207, y + 0.015, 0.124),
            (1.218, y + 0.485, 0.684),
            front,
            0.004,
        )
        for t in [0.015, 0.433]:
            part(
                f"Door stile {i + 1}",
                (1.214, y + t, 0.121),
                (1.230, y + t + 0.052, 0.682),
                frame,
                0.003,
            )
        for z in [0.121, 0.625]:
            part(
                f"Door rail {i + 1}",
                (1.214, y + 0.065, z),
                (1.230, y + 0.435, z + 0.057),
                frame,
                0.003,
            )
        # Turned cabinet hardware sits proud of the drawer face.
        profile = [
            (0.001, 0, 0),
            (0.018, 0, 0),
            (0.019, 0.003, 0),
            (0.011, 0.006, 0),
            (0.008, 0.015, 0),
            (0.017, 0.022, 0),
            (0.019, 0.030, 0),
            (0.016, 0.035, 0),
            (0.001, 0.036, 0),
        ]
        for tag, yy, zz in [("drawer", y + 0.25, 0.776), ("door", y + 0.429, 0.608)]:
            knob = lathe(
                group, f"Patinated brass {tag} pull {i + 1}", profile, [brass], 24
            )
            knob.rotation_euler.y = math.pi / 2
            knob.location = (1.224, yy, zz)
    tiles = scene.objects["kitchen-tiles"]
    for o in tiles.children:
        o.hide_render = True
        o.hide_viewport = True
    mortar = material("Warm kitchen tile grout", (180, 172, 153), 0.84)
    glazes = [
        material(f"Warm white glazed kitchen tile {i + 1}", c, 0.24 + 0.025 * i)
        for i, c in enumerate(
            [(218, 212, 191), (227, 220, 201), (219, 212, 193), (222, 216, 196)]
        )
    ]
    y1 = -16.42
    end = -11.76
    part("Lime grout bed", (0.549, y1, 0.921), (0.567, end, 1.599), mortar, 0.001)
    rng = random.Random(744)
    # Batch separately bevelled tile solids to keep one editable mesh for the run.
    vertices = []
    faces = []
    ids = []
    for row in range(7):
        y = y1 - (0.105 if row % 2 else 0)
        while y < end:
            lo = max(y, y1) + 0.0012
            hi = min(y + 0.21, end) - 0.0012
            if hi - lo > 0.02:
                x = 0.572 + rng.uniform(-0.0008, 0.0008)
                z = 0.923 + row * 0.0967
                k = len(vertices)
                for xx in (x - 0.006, x):
                    vertices.extend(
                        [
                            (xx, lo, z),
                            (xx, hi, z),
                            (xx, hi, z + 0.094),
                            (xx, lo, z + 0.094),
                        ]
                    )
                tint = rng.randrange(4)
                for face in [
                    (0, 3, 2, 1),
                    (4, 5, 6, 7),
                    (0, 1, 5, 4),
                    (1, 2, 6, 5),
                    (2, 3, 7, 6),
                    (3, 0, 4, 7),
                ]:
                    faces.append(tuple(k + i for i in face))
                    ids.append(tint)
            y += 0.21
    field = mesh(
        group, "Hand-set running bond tile", vertices, faces, glazes, ids, smooth=False
    )
    edge = field.modifiers.new("Glazed tile arris", "BEVEL")
    edge.width = 0.0018
    edge.segments = 3
    field.modifiers.new("Tile normals", "WEIGHTED_NORMAL")
    # Each spot is anchored to the existing luminous disk under the hood.
    source = next(
        o for o in scene.objects["range-hood"].children if "opal-work-light" in o.name
    )
    points = [source.matrix_world @ v.co for v in source.data.vertices]
    mid = (max(p.y for p in points) + min(p.y for p in points)) * 0.5
    spots = []
    for i, half in enumerate(
        ([p for p in points if p.y < mid], [p for p in points if p.y >= mid])
    ):
        xyz = sum(half, Vector()) / len(half)
        light = bpy.data.lights.new(f"Range hood task light {i + 1}", "SPOT")
        light.energy = 2.1
        light.color = (1, 0.75, 0.49)
        light.spot_size = math.radians(73)
        light.spot_blend = 0.73
        light.shadow_soft_size = 0.038
        lamp = bpy.data.objects.new(light.name, light)
        group.objects.link(lamp)
        lamp.location = xyz + Vector((0, 0, -0.012))
        lamp.rotation_euler = (
            (Vector((0.568, xyz.y, 1.33)) - lamp.location)
            .to_track_quat("-Z", "Y")
            .to_euler()
        )
        spots.append(lamp)
    # The cooking board stands on the countertop beside the range.
    board = scene.objects["chopping-board"]
    board.location.y += 3.04
    crock = material("Ivory utensil crock", (193, 176, 141), 0.37)
    vessel = lathe(
        group,
        "Utensil crock",
        [
            (0.001, 0, 0),
            (0.075, 0, 0),
            (0.086, 0.012, 0),
            (0.091, 0.143, 0),
            (0.088, 0.177, 0),
            (0.084, 0.183, 0),
            (0.079, 0.184, 0),
            (0.076, 0.176, 0),
            (0.073, 0.021, 0),
            (0.001, 0.018, 0),
        ],
        [crock],
        56,
    )
    vessel.location = (0.813, -13.09, 0.92)
    for i in range(5):
        angle = i * 0.99 + 0.15
        dx = 0.029 * math.cos(angle)
        dy = 0.038 * math.sin(angle)
        axis = Vector((dx * 3, dy * 3, 0.34 + (0.025 if i % 2 else -0.008)))
        start = vessel.location + Vector((dx, dy, 0.034))
        radius = 0.0065 + i * 0.00042
        handle = lathe(
            group,
            f"Beech spoon {i + 1}",
            [
                (0.001, 0, 0),
                (radius, 0, 0),
                (radius, axis.length * 0.75, 0),
                (0.013, axis.length * 0.80, 0),
                (0.023, axis.length * 0.91, 0),
                (0.021, axis.length * 0.975, 0),
                (0.011, axis.length, 0),
                (0.001, axis.length + 0.005, 0),
            ],
            [wood],
            28,
        )
        handle.scale = (1, 0.43, 1)
        handle.location = start
        handle.rotation_euler = axis.to_track_quat("Z", "Y").to_euler()
    return spots
