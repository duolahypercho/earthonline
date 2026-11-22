"""Light-filtering linen, sewn cushions, and a folded throw for the rear window."""

import math

import numpy as np
from beaver_dining_parts import image_map, material, mesh


def weave(name, tone, checks=False):
    cloth = material(name, tone, 0.85)
    tree = cloth.node_tree
    tree.nodes["Principled BSDF"].inputs["Sheen Weight"].default_value = 0.30
    yy, xx = np.mgrid[0:512, 0:512]
    u = xx / 512
    v = yy / 512
    warp = np.sin((xx * 93 / 512 + 0.0012 * np.sin(yy / 24)) * 2 * math.pi)
    weft = np.sin((yy * 110 / 512 + 0.010 * np.sin(xx / 37)) * 2 * math.pi)
    intensity = 0.98 + 0.036 * warp + 0.030 * weft + 0.012 * np.cos(u * 31 + v * 17)
    if checks:
        intensity *= np.where((xx % 89 < 8) | (yy % 133 < 7), 0.57, 1)
        intensity *= np.where((xx % 89 < 14) & (xx % 89 > 10), 0.70, 1)
    image_map(
        cloth, name + " - warp and weft", intensity[:, :, None] * np.array(tone) / 255
    )
    return cloth


def cushion(group, name, tone, width, height, rotation, position):
    fabric = weave(name + " - dyed linen", tone)
    n = 32
    coords = []
    uv = []
    faces = []
    for side in (-1, 1):
        for row in range(n + 1):
            v = row / n * 2 - 1
            for col in range(n + 1):
                u = col / n * 2 - 1
                fullness = max(0, (1 - u * u) * (1 - v * v))
                puff = 0.110 * fullness**0.63
                fold = (
                    0.011
                    * math.sin(23 * (v + 0.44 * u))
                    * math.exp(-(((abs(u) - 0.74) / 0.22) ** 2))
                )
                fold += (
                    0.012
                    * math.sin(28 * (u - 0.28 * v) + 2)
                    * math.exp(-(((abs(v) - 0.82) / 0.16) ** 2))
                )
                # Draw corner ears out and pinch the fabric just inside the seam.
                x = width * 0.5 * u * (0.93 + 0.07 * abs(v) ** 4) + 0.011 * math.sin(
                    2.8 * v + 0.5
                ) * (1 - u * u)
                z = (
                    height * 0.5 * v * (0.91 + 0.09 * abs(u) ** 3)
                    + 0.018 * (u + 0.3) * v
                )
                y = (
                    side * (0.003 + puff + fold * min(1, fullness * 10))
                    + 0.021 * v
                    - 0.023 * (1 - v * v) * u
                )
                coords.append((x, y, z))
                uv.append(((u + 1) * 0.5, (v + 1) * 0.5))
                if row and col:
                    i = len(coords) - 1
                    face = (i - n - 2, i - n - 1, i, i - 1)
                    faces.append(face if side < 0 else face[::-1])
    stride = (n + 1) ** 2
    ring = (
        list(range(n + 1))
        + [r * (n + 1) + n for r in range(1, n + 1)]
        + list(range(stride - 2, stride - n - 2, -1))
        + [r * (n + 1) for r in range(n - 1, 0, -1)]
    )
    for a, b in zip(ring, ring[1:] + ring[:1]):
        faces.append((a, a + stride, b + stride, b))
    obj = mesh(group, name, coords, faces, [fabric], uv=uv)
    obj.location = position
    obj.rotation_euler = rotation
    return obj


def window_furnishings(scene, group):
    for ob in scene.objects["window-seat"].children:
        if "blockprint-ink" in ob.name or "woven-indigo" in ob.name:
            ob.hide_render = True
            ob.hide_viewport = True
    for i, (x, z, color, roll, w, h) in enumerate(
        [
            (5.55, 0.750, (159, 83, 49), 0.073, 0.55, 0.47),
            (4.86, 0.748, (72, 96, 96), -0.061, 0.54, 0.46),
            (4.18, 0.737, (186, 112, 70), 0.096, 0.53, 0.44),
        ]
    ):
        cushion(
            group,
            f"Sewn window cushion {i + 1}",
            color,
            w,
            h,
            (math.radians(13), roll, (-1) ** i * 0.061),
            (x, -16.255, z),
        )
    blanket = weave(
        "Window-seat throw - natural flax and charcoal", (193, 179, 148), True
    )
    points = []
    faces = []
    uv = []
    nx, ny = 37, 60
    for j in range(ny):
        t = j / (ny - 1)
        if t < 0.54:
            along = t / 0.54
            y = -16.13 + 0.337 * along
            z = 0.539 + 0.008 * math.sin(math.pi * along)
        elif t < 0.64:
            a = (t - 0.54) / 0.10 * math.pi / 2
            y = -15.793 + 0.040 * math.sin(a)
            z = 0.500 + 0.040 * math.cos(a)
        else:
            along = (t - 0.64) / 0.36
            y = -15.753 + 0.024 * along
            z = 0.50 - 0.293 * along
        for i in range(nx):
            u = i / (nx - 1)
            x = 3.77 + (u - 0.5) * 0.52 + 0.075 * t + 0.013 * math.sin(5 * t + u)
            ridge = 0.021 * math.sin(u * 8 * math.pi + t * 3) + 0.011 * math.sin(
                11 * u + t
            )
            drape = min(1, max(0, (t - 0.49) / 0.16))
            points.append(
                (
                    x,
                    y + ridge * drape,
                    z + ridge * (1 - drape) + 0.020 * math.sin(6 * u) * t**4,
                )
            )
            uv.append((u, t))
            if i and j:
                k = j * nx + i
                faces.append((k - nx - 1, k - nx, k, k - 1))
    throw = mesh(
        group, "Checked throw over the window seat", points, faces, [blanket], uv=uv
    )
    edge = throw.modifiers.new("Woven edge", "SOLIDIFY")
    edge.thickness = 0.002
    existing = scene.objects["roman-shade"]
    for ob in existing.children:
        if "flax-linen" in ob.name or "woven-indigo" in ob.name:
            ob.hide_render = True
            ob.hide_viewport = True
    flax = weave("Unbleached translucent Roman blind", (179, 149, 105))
    tree = flax.node_tree
    bs = tree.nodes["Principled BSDF"]
    transmit = tree.nodes.new("ShaderNodeBsdfTranslucent")
    transmit.inputs["Color"].default_value = (0.81, 0.62, 0.37, 1)
    mix = tree.nodes.new("ShaderNodeMixShader")
    mix.inputs[0].default_value = 0.32
    tree.links.new(bs.outputs[0], mix.inputs[1])
    tree.links.new(transmit.outputs[0], mix.inputs[2])
    tree.links.new(mix.outputs[0], tree.nodes["Material Output"].inputs[0])
    points = []
    faces = []
    uv = []
    nx, ny = 133, 65
    for j in range(ny):
        t = j / (ny - 1)
        for i in range(nx):
            u = i / (nx - 1)
            x = 1.185 + 5.286 * u
            # Each bay sags between the three lift tapes. The hem is shortest at a tape.
            sag = math.sin(2 * math.pi * (u - 0.025)) ** 2
            phase = t * 4.62 + 0.29 * sag * t + 0.051 * math.sin(u * 19 + t * 3)
            wave = math.sin(2 * math.pi * phase)
            y = (
                -16.392
                + 0.014 * t
                + 0.109 * (0.50 + 0.50 * wave) ** 1.6 * (0.63 + 0.37 * sag)
            )
            z = (
                2.647
                - 0.473 * t
                - 0.082 * sag * t**1.55
                + 0.019 * math.sin(12 * u + 2 * t) * t
            )
            points.append((x, y, z))
            uv.append((7 * u, t))
            if i and j:
                k = j * nx + i
                faces.append((k - nx - 1, k - nx, k, k - 1))
    blind = mesh(group, "Relaxed linen Roman blind", points, faces, [flax], uv=uv)
    return blind


def rear_plaster(scene):
    """Finish only the surveyed visible back wall of the opposite context house."""
    import bpy

    obj = scene.objects["Neighbouring footprints · Context · siding 0"]
    faces = []
    for poly in obj.data.polygons:
        p = obj.matrix_world @ poly.center
        if -54 < p.y < -48 and -19 < p.x < 14:
            faces.append(poly)
    if not faces:
        raise ValueError("The sampled rear neighbour has changed")
    mat = material("Sun-worn plaster on the opposite rear wall", (160, 146, 119), 0.85)
    tree = mat.node_tree
    bs = tree.nodes["Principled BSDF"]
    uv = tree.nodes.new("ShaderNodeUVMap")
    uv.uv_map = "Rear plaster courses"
    color_tex = tree.nodes.new("ShaderNodeTexImage")
    color_tex.image = bpy.data.images["worn_plaster_wall_diff_2k.jpg"]
    tree.links.new(uv.outputs[0], color_tex.inputs["Vector"])
    tint = tree.nodes.new("ShaderNodeMixRGB")
    tint.blend_type = "MULTIPLY"
    tint.inputs[0].default_value = 0.76
    tint.inputs[2].default_value = (0.78, 0.65, 0.48, 1)
    tree.links.new(color_tex.outputs["Color"], tint.inputs[1])
    tree.links.new(tint.outputs[0], bs.inputs["Base Color"])
    tex = tree.nodes.new("ShaderNodeTexImage")
    tex.image = bpy.data.images["worn_plaster_wall_nor_gl_2k.jpg"]
    normal = tree.nodes.new("ShaderNodeNormalMap")
    normal.uv_map = uv.uv_map
    normal.inputs["Strength"].default_value = 0.58
    tree.links.new(uv.outputs[0], tex.inputs["Vector"])
    tree.links.new(tex.outputs["Color"], normal.inputs["Color"])
    tree.links.new(normal.outputs[0], bs.inputs["Normal"])
    layer = obj.data.uv_layers.new(name=uv.uv_map)
    slot = len(obj.data.materials)
    obj.data.materials.append(mat)
    for poly in faces:
        poly.material_index = slot
        for i in poly.loop_indices:
            p = obj.matrix_world @ obj.data.vertices[obj.data.loops[i].vertex_index].co
            layer.data[i].uv = (p.x * 0.29, p.z * 0.29)
