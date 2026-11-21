"""A flared stair foot, scrolled walnut rail, and a bound runner fitted to the treads."""

import math

import bpy
import numpy as np
from beaver_dining_parts import image_map, lathe, material, mesh
from mathutils import Vector


def runner_material():
    wool = material("Olive and madder wool stair runner", (119, 98, 67), 0.91)
    wool.node_tree.nodes["Principled BSDF"].inputs["Sheen Weight"].default_value = 0.34
    y, x = np.mgrid[0:768, 0:768]
    u = x / 768
    v = y / 768
    edge = np.minimum(u, 1 - u)
    color = np.full((768, 768, 3), (0.43, 0.35, 0.246))
    color[edge < 0.16] = (0.34, 0.285, 0.224)
    for pos, tint, width in [
        (0.014, (0.64, 0.52, 0.33), 0.005),
        (0.035, (0.38, 0.27, 0.19), 0.009),
        (0.056, (0.64, 0.54, 0.36), 0.004),
        (0.133, (0.68, 0.58, 0.40), 0.005),
        (0.153, (0.40, 0.26, 0.19), 0.004),
        (0.178, (0.63, 0.51, 0.34), 0.006),
    ]:
        color[abs(edge - pos) < width] = tint

    def diamonds(px, py, dx, dy):
        return abs((px + 0.5) % 1 - 0.5) / dx + abs((py + 0.5) % 1 - 0.5) / dy

    medallion = diamonds(u * 5.5, v * 3, 0.38, 0.39)
    offset = diamonds(u * 5.5 + 0.5, v * 3 + 0.5, 0.23, 0.26)
    inside = edge > 0.18
    color[inside & (medallion < 1)] = (0.50, 0.385, 0.265)
    color[inside & (abs(medallion - 0.84) < 0.08)] = (0.62, 0.52, 0.36)
    color[inside & (medallion < 0.38)] = (0.32, 0.33, 0.26)
    color[inside & (offset < 0.68)] = (0.59, 0.53, 0.375)
    petals = diamonds((edge - 0.097) * 12, v * 13, 0.34, 0.42)
    color[(edge > 0.056) & (edge < 0.134) & (petals < 1)] = (0.59, 0.45, 0.29)
    warp = np.cos(2 * math.pi * (u * 187 + 0.08 * np.sin(v * 21)))
    weft = np.sin(2 * math.pi * (v * 154 + 0.13 * np.sin(u * 50)))
    rng = np.random.default_rng(1933)
    grain = 0.97 + 0.045 * warp + 0.045 * weft + 0.025 * rng.standard_normal(u.shape)
    image_map(
        wool, "Stair runner - woven borders and diamonds", color * grain[:, :, None]
    )
    return wool


def stair_details(scene, group):
    stair = scene.objects["stair-flight-1"]
    tread = next(o for o in stair.children if "ae906f" in o.name)
    trunk = next(o for o in stair.children if "785338" in o.name)
    stairs = np.array([list(tread.matrix_world @ v.co) for v in tread.data.vertices])
    sample = stairs[abs(stairs[:, 2] - (1 / 6 + 0.002)) < 0.0001]
    first_y = float(sample[:, 1].max())
    front_x = float(sample[:, 0].min())
    run = first_y - float(stairs[abs(stairs[:, 2] - (2 / 6 + 0.002)) < 0.0001, 1].max())

    def flare(y):
        t = np.clip((first_y - y) / (run * 7), 0, 1)
        return 0.405 * float((1 - t) ** 2)

    def edge(i):
        z = (i + 1) / 6
        front = stairs[abs(stairs[:, 2] - (z + 0.002)) < 0.0001]
        return float(front[:, 0].min()), float(front[:, 1].max()), z

    # Widen only the open edge of the starting treads. The wall edge stays put.
    for ob in stair.children:
        if ob == trunk:
            continue
        m = ob.matrix_world.copy()
        inv = m.inverted()
        for v in ob.data.vertices:
            p = m @ v.co
            weight = float(np.clip(1 - (p.x - front_x) / 2.22, 0, 1))
            p.x -= weight * flare(p.y)
            v.co = inv @ p
        ob.data.update()
    walnut = trunk.data.materials[0].copy()
    walnut.name = "Waxed walnut stair handrail"
    tree = walnut.node_tree
    bs = tree.nodes.get("Principled BSDF")
    bs.inputs["Coat Weight"].default_value = 0.24
    bs.inputs["Coat Roughness"].default_value = 0.25
    # Polished hand contact exposes the photographed lengthwise grain. Use
    # one board strip and keep its orientation around the rail's return.
    uv_node = tree.nodes.new("ShaderNodeUVMap")
    uv_node.uv_map = "UVMap"
    pigment = tree.nodes.new("ShaderNodeTexImage")
    pigment.image = bpy.data.images["Dining oak - Diffuse"]
    grain = tree.nodes.new("ShaderNodeVectorMath")
    grain.operation = "MULTIPLY"
    grain.inputs[1].default_value = (0.59, 0.39, 0.24)
    tree.links.new(uv_node.outputs[0], pigment.inputs["Vector"])
    tree.links.new(pigment.outputs["Color"], grain.inputs[0])
    tree.links.new(grain.outputs[0], bs.inputs["Base Color"])
    for link in list(bs.inputs["Roughness"].links):
        tree.links.remove(link)
    bs.inputs["Roughness"].default_value = 0.27
    bs.inputs["Coat Roughness"].default_value = 0.19
    trunk.hide_render = True
    trunk.hide_viewport = True
    # Follow the existing balusters to the top landing. Ease to level before
    # entering the horizontal volute above the turned starting newel.
    path = []
    levels = []
    for i in range(97):
        t = 1 - i / 96
        y = first_y - run * 20 * t
        x = 6.894 + 0.126 * t - flare(y)
        path.append(Vector((x, y, 1.016 + 3.333333 * t)))
    base = path[-1]
    for i in range(1, 12):
        t = i / 11
        path.append(
            Vector((base.x, base.y + 0.113 * t, base.z - 0.030 * (1 - (1 - t) ** 2)))
        )
    tangent = path[-1].copy()
    r = 0.1034
    for i in range(1, 90):
        angle = math.tau * 1.06 * i / 89
        radius = r * (1 - 0.82 * (i / 89) ** 1.72)
        path.append(
            Vector(
                (
                    tangent.x - r + radius * math.cos(angle),
                    tangent.y + radius * math.sin(angle),
                    tangent.z,
                )
            )
        )
    # Handrail crown, thumb recess and narrow bed moulding are one closed section.
    section = [
        (-0.024, -0.033),
        (0.024, -0.033),
        (0.027, -0.028),
        (0.026, -0.022),
        (0.021, -0.017),
        (0.024, -0.011),
        (0.034, -0.006),
        (0.039, 0.001),
        (0.037, 0.013),
        (0.031, 0.022),
        (0.017, 0.028),
        (0, 0.030),
        (-0.017, 0.028),
        (-0.031, 0.022),
        (-0.037, 0.013),
        (-0.039, 0.001),
        (-0.034, -0.006),
        (-0.024, -0.011),
        (-0.021, -0.017),
        (-0.026, -0.022),
        (-0.027, -0.028),
    ]
    vertices = []
    faces = []
    uv = []
    distance = 0.0
    for i, point in enumerate(path):
        along = (path[min(i + 1, len(path) - 1)] - path[max(0, i - 1)]).normalized()
        side = Vector((along.y, -along.x, 0)).normalized()
        up = side.cross(along).normalized()
        if i:
            distance += (point - path[i - 1]).length
        for j, (s, h) in enumerate(section):
            vertices.append(point + s * side + h * up)
            uv.append((distance * 0.62 + 0.17, (3.08 + 0.81 * j / len(section)) / 8))
            if i:
                a = (i - 1) * len(section) + j
                b = (i - 1) * len(section) + (j + 1) % len(section)
                faces.append((a, b, b + len(section), a + len(section)))
    faces.append(tuple(range(len(section) - 1, -1, -1)))
    faces.append(tuple(len(vertices) - len(section) + i for i in range(len(section))))
    rail = mesh(
        group,
        "Continuous moulded walnut rail and volute",
        vertices,
        faces,
        [walnut],
        uv=uv,
    )
    centre = lathe(
        group,
        "Carved centre of the walnut volute",
        [
            (0.001, -0.025, 0),
            (0.074, -0.025, 0),
            (0.091, -0.012, 0),
            (0.093, 0.002, 0),
            (0.089, 0.012, 0),
            (0.078, 0.023, 0),
            (0.061, 0.027, 0),
            (0.030, 0.026, 0),
            (0.019, 0.021, 0),
            (0.014, 0.017, 0),
            (0.001, 0.017, 0),
        ],
        [walnut],
        80,
    )
    cap_uv = centre.data.uv_layers.active
    for loop in centre.data.loops:
        p = centre.data.vertices[loop.vertex_index].co
        cap_uv.data[loop.index].uv = (
            0.38 + 0.62 * (p.x + 0.5 * p.y),
            (0.46 + p.y * 2.4 + 3) / 8,
        )
    centre.location = (tangent.x - r, tangent.y, tangent.z)
    ivory = next(o for o in stair.children if "painted-white" in o.name).data.materials[
        0
    ]
    finish = path[-1].z - 0.037
    post = lathe(
        group,
        "Turned starting newel",
        [
            (0.001, 0, 0),
            (0.059, 0, 0),
            (0.059, 0.26, 0),
            (0.054, 0.275, 0),
            (0.064, 0.29, 0),
            (0.063, 0.307, 0),
            (0.050, 0.324, 0),
            (0.041, 0.36, 0),
            (0.036, 0.47, 0),
            (0.047, 0.57, 0),
            (0.052, 0.60, 0),
            (0.044, 0.63, 0),
            (0.031, 0.68, 0),
            (0.030, 0.76, 0),
            (0.048, 0.80, 0),
            (0.056, 0.825, 0),
            (0.049, 0.85, 0),
            (0.054, 0.873, 0),
            (0.057, finish - 0.03, 0),
            (0.064, finish - 0.019, 0),
            (0.064, finish, 0),
            (0.001, finish, 0),
        ],
        [ivory],
        64,
    )
    post.location = (tangent.x - r, tangent.y, 0)
    # One length of carpet follows the rounded noses, treads, and risers.
    wool = runner_material()
    rods = material("Aged brass stair rods", (136, 110, 69), 0.40, 0.74)
    width = 0.89
    points = []
    faces = []
    uv = []
    rod_points = []
    rod_faces = []
    length = 0
    for i in range(20):
        left, y, z = edge(i)
        center = left + 0.70 - flare(y) * 0.70
        previous_z = i / 6
        profile = [
            (y + 0.006, previous_z + 0.006),
            (y + 0.006, z - 0.010),
            (y + 0.0045, z - 0.003),
            (y + 0.001, z + 0.003),
            (y - 0.007, z + 0.007),
            (y - 0.024, z + 0.008),
            (y - run + 0.007, z + 0.006),
        ]
        for yy, zz in profile:
            if levels:
                length += math.hypot(yy - levels[-1][0], zz - levels[-1][1])
            levels.append((yy, zz))
            row = len(points) // 5
            for k in range(5):
                u = k / 4
                points.append(
                    (
                        center + (u - 0.5) * width,
                        yy,
                        zz + 0.0014 * math.sin(math.pi * u),
                    )
                )
                uv.append((u, length / 0.70))
                if row and k:
                    p = row * 5 + k
                    faces.append((p - 6, p - 5, p, p - 1))
        for k, rad, ext in [
            (-1, 0.014, -0.027),
            (-1, 0.010, -0.019),
            (-1, 0.006, 0.000),
            (1, 0.006, 0),
            (1, 0.010, -0.019),
            (1, 0.014, -0.027),
        ]:
            x = center + k * (width / 2 - ext)
            row = len(rod_points) // 12
            for j in range(12):
                angle = math.tau * j / 12
                rod_points.append(
                    (
                        x,
                        y + 0.019 + rad * math.cos(angle),
                        previous_z + 0.024 + rad * math.sin(angle),
                    )
                )
                if row % 6:
                    a = (row - 1) * 12 + j
                    b = (row - 1) * 12 + (j + 1) % 12
                    rod_faces.append((a, b, b + 12, a + 12))
    carpet = mesh(
        group, "Bound wool runner over twenty nosings", points, faces, [wool], uv=uv
    )
    backing = carpet.modifiers.new("Felt backing", "SOLIDIFY")
    backing.thickness = 0.002
    rods_obj = mesh(group, "Brass rods and finials", rod_points, rod_faces, [rods])
    return [rail, centre, post, carpet, rods_obj]
