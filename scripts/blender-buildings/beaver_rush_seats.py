"""Wound rush seat on four rails. Six chairs share the same metre-scale mesh."""

import math
import random

import bpy
import numpy as np
from beaver_dining_parts import image_map, material, mesh
from mathutils import Matrix, Vector


def woven_seat(collection):
    palette = (
        (161, 126, 73),
        (182, 149, 98),
        (169, 138, 87),
        (184, 151, 94),
        (157, 127, 82),
    )
    mats = [
        material(f"Twisted chair rush {i + 1}", color, 0.67)
        for i, color in enumerate(palette)
    ]
    h, w = 64, 256
    v, u = np.mgrid[0:h, 0:w]
    phase = 2 * math.pi * (u / w * 6 + v / h)
    ridges = 0.098 * np.sin(phase) + 0.038 * np.sin(phase * 3 + 0.4)
    normal = np.stack(
        [0.5 + ridges, 0.5 + 0.044 * np.sin(phase), 0.985 + 0 * ridges], axis=-1
    )
    image_map(mats[0], "Rush - twisted leaf fibres", normal, "Normal")
    normal_image = next(
        n.image for n in mats[0].node_tree.nodes if n.type == "TEX_IMAGE"
    )
    for mat in mats:
        tree = mat.node_tree
        bs = tree.nodes.get("Principled BSDF")
        bs.inputs["Sheen Weight"].default_value = 0.17
        if mat != mats[0]:
            tex = tree.nodes.new("ShaderNodeTexImage")
            tex.image = normal_image
            n = tree.nodes.new("ShaderNodeNormalMap")
            tree.links.new(tex.outputs["Color"], n.inputs["Color"])
            tree.links.new(n.outputs["Normal"], bs.inputs["Normal"])
    points = []
    faces = []
    uv = []
    ids = []
    rng = random.Random(3250)
    sides = 7
    for edge in range(4):
        along = np.array([(1, 0), (0, 1), (-1, 0), (0, -1)][edge], float)
        outward = np.array([(0, -1), (1, 0), (0, 1), (-1, 0)][edge], float)
        half_width, depth = (0.224, 0.206) if edge % 2 == 0 else (0.206, 0.224)
        strands = round(half_width * 2 / 0.0055)
        for i in range(strands):
            frac = (i + 0.5) / strands
            stationary = (frac * 2 - 1) * half_width
            end = max(0.002, (1 - abs(2 * frac - 1)) * depth)
            radius = half_width / strands * rng.uniform(0.96, 1.08)
            hue = rng.choices(range(5), weights=[2, 3, 4, 1, 2])[0]
            sag = 0.012 * (math.cos(stationary / half_width * math.pi / 2)) ** 2
            base = len(points)
            # The outside bend wraps the timber rail. The inner end dives
            # beneath the adjacent field at the diagonal join.
            route = [
                (depth - 0.010, -0.019),
                (depth + 0.005, -0.025),
                (depth + 0.015, -0.019),
                (depth + 0.020, -0.009),
                (depth + 0.020, -0.001),
                (depth + 0.016, 0.007),
                (depth + 0.009, 0.012),
                (depth, 0.014),
            ]
            for t in (0.12, 0.32, 0.55, 0.76, 0.94, 1):
                route.append(
                    (
                        depth - end * t,
                        0.013 - sag * t + 0.002 * math.sin(math.pi * t) - 0.004 * t**9,
                    )
                )
            travel = 0.0
            for j, (offset, z) in enumerate(route):
                if j:
                    travel += math.dist(route[j - 1], route[j])
                first = np.array(route[max(j - 1, 0)])
                last = np.array(route[min(j + 1, len(route) - 1)])
                d = last - first
                d /= max(np.linalg.norm(d), 0.0001)
                lateral = outward * (-d[1])
                for k in range(sides + 1):
                    theta = math.tau * k / sides
                    xy = (
                        along * (stationary + radius * math.cos(theta))
                        + outward * offset
                    )
                    xy += lateral * radius * 0.80 * math.sin(theta)
                    points.append(
                        (xy[0], xy[1], z + radius * 0.80 * d[0] * math.sin(theta))
                    )
                    uv.append((travel / 0.070, k / sides))
                    if j and k:
                        n = base + j * (sides + 1) + k
                        faces.append((n - sides - 2, n - 1, n, n - sides - 1))
                        ids.append(hue)
    return mesh(collection, "Four-way wrapped rush seat", points, faces, mats, ids, uv)


def fit_seats(scene, collection):
    source = woven_seat(collection)
    prototype = source.data
    bpy.data.objects.remove(source, do_unlink=True)
    used = set()
    new = []
    for chair in sorted(
        (
            o
            for o in scene.objects
            if o.type == "EMPTY" and o.name.startswith("dining-chair")
        ),
        key=lambda o: o.name,
    ):
        rush = [o for o in chair.children if "rush-seat@" in o.name]
        body = [o for o in chair.children if "furniture-oak@" in o.name]
        points = np.array(
            [o.matrix_world @ v.co for o in rush for v in o.data.vertices]
        )
        high = np.array(
            [
                p
                for o in body
                for v in o.data.vertices
                if (p := o.matrix_world @ v.co).z > 0.83
            ]
        )
        center = (points.min(0) + points.max(0)) * 0.5
        back = Vector((*(high.mean(0) - center)[:2], 0)).normalized()
        right = back.cross(Vector((0, 0, 1))).normalized()
        obj = bpy.data.objects.new(chair.name + " - hand-wrapped rush", prototype)
        collection.objects.link(obj)
        placement = Matrix((right, back, Vector((0, 0, 1)))).transposed().to_4x4()
        placement.translation = (center[0], center[1], 0.439)
        obj.parent = chair
        obj.matrix_parent_inverse = chair.matrix_world.inverted()
        obj.matrix_basis = placement
        for o in rush:
            o.hide_render = True
            o.hide_viewport = True
        for ob in body:
            mat = ob.data.materials[0]
            if mat in used:
                continue
            used.add(mat)
            tree = mat.node_tree
            bs = tree.nodes["Principled BSDF"]
            base = bs.inputs["Base Color"]
            if base.links:
                src = base.links[0].from_socket
                tint = tree.nodes.new("ShaderNodeVectorMath")
                tint.operation = "MULTIPLY"
                tint.inputs[1].default_value = (1.52, 1.19, 0.90)
                tree.links.new(src, tint.inputs[0])
                tree.links.new(tint.outputs[0], base)
            bs.inputs["Coat Weight"].default_value = 0.22
            bs.inputs["Coat Roughness"].default_value = 0.33
        new.append(obj)
    return new
