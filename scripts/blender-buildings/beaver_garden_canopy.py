"""A light, branching garden canopy within the rear garden, rooted on its grade."""

import math
import random

import bpy
from beaver_dining_parts import material, mesh
from mathutils import Vector


def garden_canopy(scene, collection):
    x, y = 0.93, -23.75
    dg = bpy.context.evaluated_depsgraph_get()
    hit, foot, _, _, ground, _ = scene.ray_cast(
        dg, Vector((x, y, 1.0)), Vector((0, 0, -1)), distance=9
    )
    if not hit or foot.z > 0.16:
        raise ValueError(f"No garden soil under the tree: {ground}")
    bark = material("Garden tree - grey-brown bark", (103, 93, 77), 0.87)
    leaves = [
        material(f"Garden leaf {i + 1}", v, 0.56)
        for i, v in enumerate(
            [(91, 111, 47), (102, 118, 56), (72, 95, 47), (124, 128, 62), (87, 104, 56)]
        )
    ]
    for mat in leaves:
        tree = mat.node_tree
        diffuse = tree.nodes["Principled BSDF"]
        translucent = tree.nodes.new("ShaderNodeBsdfTranslucent")
        translucent.inputs[0].default_value = (0.37, 0.43, 0.13, 1)
        m = tree.nodes.new("ShaderNodeMixShader")
        m.inputs[0].default_value = 0.31
        tree.links.new(diffuse.outputs[0], m.inputs[1])
        tree.links.new(translucent.outputs[0], m.inputs[2])
        tree.links.new(m.outputs[0], tree.nodes["Material Output"].inputs[0])
    rng = random.Random(6012)
    wood = []
    wf = []
    blades = []
    bf = []
    ids = []
    up = Vector((0, 0, 1))

    def twig(a, b, base, tip):
        d = (b - a).normalized()
        ux = d.cross(Vector((1, 0, 0))).normalized()
        uy = d.cross(ux).normalized()
        k = len(wood)
        n = 9 if base > 0.025 else 5
        for p, r in [(a, base), (b, tip)]:
            for i in range(n):
                v = math.tau * i / n
                wood.append(p + r * math.cos(v) * ux + r * math.sin(v) * uy)
        for i in range(n):
            wf.append((k + i, k + (i + 1) % n, k + n + (i + 1) % n, k + n + i))

    def leaf(a, ray, scale):
        d = ray.normalized()
        normal = (
            up + d * 0.21 + Vector((rng.uniform(-0.5, 0.5), rng.uniform(-0.6, 0.6), 0))
        ).normalized()
        side = d.cross(normal).normalized() * scale * rng.uniform(0.16, 0.24)
        mid = a + d * scale * 0.49
        end = a + d * scale - up * 0.011
        k = len(blades)
        blades.extend([a, mid - side, mid + normal * 0.003, mid + side, end])
        bf.extend(
            [
                (k, k + 2, k + 1),
                (k, k + 3, k + 2),
                (k + 1, k + 2, k + 4),
                (k + 2, k + 3, k + 4),
            ]
        )
        ids.extend([rng.choices(range(5), weights=[3, 1, 2, 1, 3])[0]] * 4)

    trunk = [
        foot + Vector(p)
        for p in [
            (0, 0, 0),
            (-0.06, 0.06, 0.75),
            (0.03, -0.04, 1.69),
            (0.24, 0.07, 2.64),
            (0.40, 0.24, 3.4),
            (0.49, 0.46, 4.08),
        ]
    ]
    for i in range(5):
        twig(trunk[i], trunk[i + 1], 0.087 - 0.011 * i, 0.077 - 0.011 * i)
    for j in range(19):
        base = trunk[min(5, 2 + j // 4)].lerp(
            trunk[min(5, 3 + j // 4)], rng.uniform(0.04, 0.89)
        )
        theta = j * 2.40 + 0.34
        outward = Vector((math.cos(theta), math.sin(theta), 0))
        side = outward.cross(up)
        elbow = base + outward * rng.uniform(0.56, 1.02) + up * rng.uniform(0.40, 0.93)
        tip = elbow + outward * rng.uniform(0.35, 0.82) + up * rng.uniform(0.45, 1.06)
        if j >= 12:
            base = trunk[1].lerp(trunk[2], rng.uniform(0.16, 0.82))
            elbow = (
                base + outward * rng.uniform(0.63, 1.21) + up * rng.uniform(0.29, 0.74)
            )
            tip = (
                elbow
                + outward * rng.uniform(0.65, 1.12)
                + up * rng.uniform(-0.15, 0.32)
            )
        twig(base, elbow, 0.044, 0.029)
        twig(elbow, tip, 0.029, 0.014)
        for fork in range(6):
            start = elbow.lerp(tip, (fork + rng.random()) / 6)
            spread = side * (-1 if fork % 2 else 1)
            branch = (
                spread * rng.uniform(0.33, 0.83)
                + outward * rng.uniform(0.12, 0.61)
                + up * rng.uniform(-0.18, 0.64)
            )
            far = start + branch
            twig(start, far, 0.013, 0.005)
            for shoot in range(5):
                root = start.lerp(far, (shoot + 0.5) / 5)
                direction = (
                    outward * rng.uniform(-0.20, 0.65)
                    + side * rng.uniform(-0.50, 0.50)
                    + up * rng.uniform(0.04, 0.44)
                )
                end = root + direction
                twig(root, end, 0.0045, 0.0015)
                for i in range(10):
                    anchor = root.lerp(end, (i + 0.33) / 10)
                    ray = Vector(
                        (
                            math.cos(theta + i * 2.1),
                            math.sin(theta + i * 2.1),
                            rng.uniform(-0.36, 0.45),
                        )
                    )
                    leaf(anchor, ray, rng.uniform(0.069, 0.134))
    trunk = mesh(
        collection, "Garden tree - scaffold and fine branches", wood, wf, [bark]
    )
    crown = mesh(collection, "Garden tree - folded leaves", blades, bf, leaves, ids)
    return trunk, crown
