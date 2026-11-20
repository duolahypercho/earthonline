"""Street-facing fittings and a wall-trained bougainvillea, in local house metres."""

import math
import random
from mathutils import Vector


def rainwater(emit, tube, box, copper, strap, ceramic):
    # A folded gutter feeds an actual offset downpipe on the stair-tower return.
    profile = [
        (-0.22, 0.04),
        (-0.25, -0.055),
        (-0.22, -0.085),
        (-0.13, -0.085),
        (-0.107, -0.052),
        (-0.11, 0.035),
    ]
    pts = [(x, y + 10.01, z - 0.02) for x in (5.25, 6.94) for z, y in profile]
    n = len(profile)
    emit(copper, pts, [(i, i + 1, n + i + 1, n + i) for i in range(n - 1)])
    x = 6.89
    pipe = [
        (x, 9.97, -0.21),
        (x, 9.75, -0.21),
        (x, 9.65, -0.145),
        (x, 9.51, -0.13),
        (x, 3.44, -0.13),
        (6.98, 3.28, -0.13),
        (7.03, 3.18, -0.11),
        (7.035, 3.03, -0.10),
        (7.035, 0.14, -0.10),
        (7.02, 0.045, -0.19),
        (6.99, -0.035, -0.31),
    ]
    tube(copper, pipe, 0.042, (1, 0, 0), 20)
    for h in (0.5, 2.50, 4.18, 6.24, 8.40):
        xx, z = (7.035, -0.10) if h < 3.0 else (x, -0.13)
        tube(strap, [(xx, h - 0.011, z), (xx, h + 0.011, z)], 0.047, (1, 0, 0), 20)
        box(strap, (xx - 0.056, h - 0.014, z + 0.008), (xx + 0.056, h + 0.014, 0.005), bevel=0.003)
    # A glazed address tile sits beside the latch, on the front of the jamb.
    box(ceramic, (4.964, 1.39, -0.873), (5.147, 1.56, -0.849), bevel=0.006)
    two = [
        (0.023, 0.103),
        (0.026, 0.120),
        (0.040, 0.129),
        (0.055, 0.129),
        (0.067, 0.119),
        (0.068, 0.108),
        (0.062, 0.096),
        (0.026, 0.052),
        (0.023, 0.046),
        (0.072, 0.046),
    ]
    one = [(0.097, 0.111), (0.115, 0.129), (0.117, 0.048)]
    for line in (two, one, [(0.097, 0.046), (0.137, 0.046)]):
        tube(strap, [(5.14 - x, 1.398 + y, -0.876) for x, y in line], 0.004, (0, 0, 1), 8)


def window_joinery(box, emit, wood, putty, stone):
    for level in (0, 1):
        y = level * (10 / 3)
        # Splayed sill with a drip groove, horns clear of the plaster on either end.
        outline = [
            (-0.032, 0.87),
            (-0.185, 0.842),
            (-0.193, 0.808),
            (-0.158, 0.803),
            (-0.154, 0.817),
            (-0.144, 0.816),
            (-0.139, 0.801),
            (-0.036, 0.800),
        ]
        n = len(outline)
        verts = [(x, y + h, z) for x in (0.54, 4.80) for z, h in outline]
        emit(
            stone,
            verts,
            [(i, (i + 1) % n, n + (i + 1) % n, n + i) for i in range(n)]
            + [tuple(reversed(range(n))), tuple(range(n, n * 2))],
        )
        # This secondary frame gives each lower casement its own closing stile
        # and putty bevel. The fixed transoms have narrower glazing bars.
        for left, right in ((0.766, 1.994), (2.069, 3.314), (3.390, 4.548)):
            for x in (left, right - 0.028):
                box(wood, (x, y + 0.926, -0.092), (x + 0.028, y + 1.814, -0.044), bevel=0.003)
            for h in (0.918, 1.785):
                box(wood, (left, y + h, -0.093), (right, y + h + 0.03, -0.044), bevel=0.003)
            for x in (left + 0.032, right - 0.039):
                box(putty, (x, y + 0.954, -0.080), (x + 0.006, y + 1.782, -0.052), bevel=0.002)
            mid = (left + right) / 2
            box(wood, (mid - 0.012, y + 1.897, -0.054), (mid + 0.012, y + 2.515, -0.025), bevel=0.003)


def bougainvillea(emit, bark, foliage, bracts, flower):
    rng = random.Random(2179)

    # Branches are trained along two wires. Leaves grow in alternating pairs
    # away from the thick wood, bracts in little groups at the terminal shoots.
    def point(t, controls):
        a, b, c, d = controls
        return a * (1 - t) ** 3 + 3 * b * t * (1 - t) ** 2 + 3 * c * t * t * (1 - t) + d * t**3

    def branch(coords, r0, r1):
        ctrl = [Vector(v) for v in coords]
        path = [point(i / 12, ctrl) for i in range(13)]
        verts = []
        for i, p in enumerate(path):
            tangent = (path[min(12, i + 1)] - path[max(0, i - 1)]).normalized()
            u = tangent.cross(Vector((0, 0, 1))).normalized()
            v = tangent.cross(u).normalized()
            rad = r0 + (r1 - r0) * i / 12
            for j in range(7):
                a = math.tau * j / 7
                verts.append(tuple(p + rad * (math.cos(a) * u + math.sin(a) * v)))
        emit(
            bark,
            verts,
            [
                (i * 7 + j, i * 7 + (j + 1) % 7, (i + 1) * 7 + (j + 1) % 7, (i + 1) * 7 + j)
                for i in range(12)
                for j in range(7)
            ],
            True,
        )
        return ctrl

    def blade(p, offset, width, mat):
        axis = Vector(offset)
        perp = Vector((-axis.y, axis.x, 0)).normalized() * width
        c = p + axis * 0.52
        verts = [
            p,
            p + axis * 0.22 - perp * 0.67,
            c - perp,
            p + axis * 0.80 - perp * 0.62,
            p + axis,
            p + axis * 0.80 + perp * 0.52,
            c + perp,
            p + axis * 0.22 + perp * 0.56,
            c + Vector((0, 0, -width * 0.52)),
        ]
        emit(mat, [tuple(q) for q in verts], [(8, i, (i + 1) % 8) for i in range(8)], True)

    scaffold = [
        [(0.38, -0.02, -0.27), (0.61, 0.91, -0.22), (0.09, 1.67, -0.16), (0.34, 2.73, -0.19)],
        [(0.34, 2.73, -0.19), (0.49, 3.65, -0.27), (1.48, 3.76, -0.19), (2.45, 3.55, -0.25)],
        [(0.34, 2.73, -0.19), (-0.02, 3.26, -0.29), (0.26, 4.21, -0.19), (0.50, 4.72, -0.36)],
        [(0.34, 2.73, -0.19), (0.87, 2.99, -0.18), (1.65, 3.20, -0.14), (2.28, 2.98, -0.19)],
        [(0.50, 4.72, -0.36), (0.27, 5.08, -0.34), (0.39, 5.51, -0.39), (0.71, 5.76, -0.33)],
        [(2.45, 3.55, -0.25), (2.82, 3.49, -0.32), (3.36, 3.75, -0.22), (3.75, 3.30, -0.33)],
    ]
    for idx, c in enumerate(scaffold):
        trunk = branch(c, 0.039 if idx == 0 else 0.018, 0.014 if idx == 0 else 0.007)
        for ti in range(3 if idx == 0 else 13):
            t = rng.uniform(0.35, 0.98) if idx == 0 else (ti + rng.random()) / 13
            root = point(t, trunk)
            reach = rng.uniform(0.22, 0.69)
            d = Vector(
                (
                    rng.uniform(-0.8, 0.8) * reach,
                    rng.uniform(-0.65, 1.05) * reach,
                    -rng.uniform(0.08, 0.30),
                )
            )
            # Keep the bulk between the window head and balcony, with a few
            # ends hanging in front of the glass and open air between sprays.
            tip = root + d
            twig = branch(
                [
                    root,
                    root + d * 0.33 + Vector((0, 0.10, 0)),
                    root + d * 0.72 + Vector((0, 0.07, 0)),
                    tip,
                ],
                0.006,
                0.0014,
            )
            for j in range(rng.randint(10, 16)):
                u = (j + rng.random()) / 16
                center = point(u, twig)
                a = rng.uniform(-0.45, 0.48) + (0 if j % 2 else math.pi)
                length = rng.uniform(0.070, 0.14)
                direction = Vector(
                    (math.cos(a) * length, rng.uniform(-0.055, 0.064), -rng.uniform(0.021, 0.077))
                )
                blade(center, direction, rng.uniform(0.023, 0.041), rng.choice(foliage))
                if u > 0.44 and rng.random() < 0.52:
                    for k in range(3):
                        a = math.tau * k / 3 + rng.uniform(-0.3, 0.3)
                        blade(
                            center,
                            Vector((math.cos(a) * 0.047, math.sin(a) * 0.047, -0.026)),
                            0.019,
                            rng.choice(bracts),
                        )
                    if j % 4 == 0:
                        blade(center, Vector((0.002, 0.011, -0.014)), 0.004, flower)


def dress_scans(materials, directory, scene):
    """Pack the same photographic colour, roughness and relief into the editable scene."""
    import bpy
    import json
    import numpy as np

    scene.view_layers[0].update()
    uv_name = "Photographic finish"
    for name in ("limewash", "aggregate", "fired-earth", "painted-wood", "linen"):
        spec = json.loads((directory / (name + ".provenance.json")).read_text())
        atlas = bpy.data.images.load(str(directory / (name + ".png")), check_existing=False)
        atlas.colorspace_settings.name = "Non-Color"
        w, h = atlas.size
        pixels = np.empty(w * h * 4, dtype=np.float32)
        atlas.pixels.foreach_get(pixels)
        pixels = pixels.reshape(3, w, w, 4)
        layers = []
        for i, kind in enumerate(("ORM", "normal", "diffuse")):
            img = bpy.data.images.new(name + " " + kind, w, w, alpha=False)
            img.colorspace_settings.name = "sRGB" if kind == "diffuse" else "Non-Color"
            values = pixels[i].copy()
            if kind == "diffuse":
                c = values[:, :, :3]
                values[:, :, :3] = np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)
            img.pixels.foreach_set(values.ravel())
            img.pack()
            layers.append(img)
        bpy.data.images.remove(atlas)
        for obj in scene.objects:
            if obj.type != "MESH" or not obj.data.materials:
                continue
            family = obj.data.materials[0].name.split("@")[0]
            if family not in spec["families"]:
                continue
            mesh = obj.data
            uv = mesh.uv_layers.get(uv_name) or mesh.uv_layers.new(name=uv_name)
            points = [obj.matrix_world @ vertex.co for vertex in mesh.vertices]
            metres = spec["metres_per_tile"]
            for poly in mesh.polygons:
                normal = poly.normal
                axis = max(range(3), key=lambda j: abs(normal[j]))
                pair = ((1, 2), (0, 2), (0, 1))[axis]
                for loop in poly.loop_indices:
                    p = points[mesh.loops[loop].vertex_index]
                    if name == "painted-wood":
                        coords = (p.z / 2.372, (p.x - 5.225) * 3.1 / 1.430)
                    else:
                        coords = (p[pair[0]] / metres, p[pair[1]] / metres)
                    uv.data[loop].uv = coords
        for key, mat in materials.items():
            if key.split("@")[0] not in spec["families"]:
                continue
            nodes = mat.node_tree.nodes
            link = mat.node_tree.links.new
            bs = nodes.get("Principled BSDF")
            uv = nodes.new("ShaderNodeUVMap")
            uv.uv_map = uv_name
            textures = []
            for img in layers:
                node = nodes.new("ShaderNodeTexImage")
                node.image = img
                link(uv.outputs[0], node.inputs[0])
                textures.append(node)
            mix = nodes.new("ShaderNodeMixRGB")
            mix.blend_type = "MULTIPLY"
            mix.inputs[0].default_value = spec["paint_mix"]
            mix.inputs[2].default_value = bs.inputs["Base Color"].default_value[:]
            link(textures[2].outputs[0], mix.inputs[1])
            link(mix.outputs[0], bs.inputs["Base Color"])
            parts = nodes.new("ShaderNodeSeparateColor")
            link(textures[0].outputs[0], parts.inputs[0])
            link(parts.outputs[1], bs.inputs["Roughness"])
            normal = nodes.new("ShaderNodeNormalMap")
            normal.uv_map = uv_name
            normal.inputs["Strength"].default_value = spec["normal_strength"]
            link(textures[1].outputs[0], normal.inputs["Color"])
            link(normal.outputs[0], bs.inputs["Normal"])


def stair_rail(tube, box, turned, iron, fastener):
    """A bent handrail bolted to the top five fixed treads of the entrance."""
    x = 6.61
    for i in (0, 2, 4):
        z = -(i + 0.47) * 0.33
        y = -i * 0.145
        box(iron, (x - 0.061, y + 0.008, z - 0.063), (x + 0.061, y + 0.023, z + 0.063), bevel=0.009)
        tube(iron, [(x, y + 0.016, z), (x, y + 0.90, z)], 0.017, (1, 0, 0), 14)
        for dx in (-0.038, 0.038):
            turned(
                fastener, x + dx, y + 0.020, z + 0.032, [(0.009, 0), (0.009, 0.008)], 6, closed=True
            )
        tube(iron, [(x, y + 0.1, z), (x, y + 0.13, z)], 0.023, (1, 0, 0), 12)
    # The return curves into the end newel and stays behind the last nosing.
    a = 0.47 * 0.33
    path = [
        (x, 0.71, -a + 0.05),
        (x, 0.86, -a + 0.05),
        (x, 0.898, -a + 0.042),
        (x, 0.919, -a + 0.02),
        (x, 0.921, -a - 0.025),
        (x, 0.916, -a - 0.065),
        (x, 0.343, -a - 1.351),
        (x, 0.326, -a - 1.39),
        (x, 0.313, -a - 1.402),
        (x, 0.292, -a - 1.406),
        (x, 0.27, -a - 1.395),
        (x, 0.258, -a - 1.37),
        (x, 0.259, -a - 1.333),
    ]
    tube(iron, path, 0.026, (1, 0, 0), 18)


def settled_coverlet():
    """Drape quilted cloth on a temporary collision mattress, then sample its rest mesh."""
    import bpy
    from mathutils import Vector

    nx, nz = 42, 51
    scene = bpy.context.scene
    bpy.ops.mesh.primitive_cube_add(size=1, location=(1.825, -11.30, 0.48))
    bed = bpy.context.object
    bed.dimensions = (1.99, 2.36, 0.43)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    edge = bed.modifiers.new("Mattress corner radius", "BEVEL")
    edge.width = 0.125
    edge.segments = 5
    bed.modifiers.new("Mattress surface", "COLLISION")
    bed.collision.thickness_outer = 0.012
    points = []
    for j in range(nz + 1):
        t = j / nz
        for i in range(nx + 1):
            u = i / nx
            # The near edge is drawn off-centre and the far right corner is
            # turned back. This extra length can gather freely below the foot.
            x = 0.61 + 2.47 * u + 0.078 * (1 - t) * math.sin(2.9 * t + 2 * u)
            z = 9.80 + 2.18 * t + 0.15 * u - 0.19 * u * u * t + 0.035 * math.sin(5 * u + 1.8 * t)
            relief = 0.029 * math.sin(17 * u + 3.1 * t) * math.sin(math.pi * t)
            ridge = 0.10 * math.exp(-(((t - 0.23 - 0.22 * u) / 0.089) ** 2)) * (1 - 0.65 * u)
            points.append((x, -z, 0.745 + relief + ridge + 0.025 * math.sin(math.pi * u)))
    faces = [
        (j * (nx + 1) + i, j * (nx + 1) + i + nx + 1, j * (nx + 1) + i + nx + 2, j * (nx + 1) + i + 1)
        for j in range(nz)
        for i in range(nx)
    ]
    mesh = bpy.data.meshes.new("Linen drape grid")
    mesh.from_pydata(points, [], faces)
    cloth = bpy.data.objects.new("Coverlet drape", mesh)
    scene.collection.objects.link(cloth)
    pins = cloth.vertex_groups.new(name="Held near the headboard")
    pins.add([nz * (nx + 1) + i for i in range(10, 32)], 1.0, "REPLACE")
    mod = cloth.modifiers.new("Quilt settling", "CLOTH")
    settings = mod.settings
    settings.quality = 7
    settings.mass = 0.28
    settings.tension_stiffness = 32
    settings.compression_stiffness = 28
    settings.shear_stiffness = 18
    settings.bending_stiffness = 0.32
    settings.vertex_group_mass = pins.name
    col = mod.collision_settings
    col.distance_min = 0.01
    col.use_self_collision = True
    col.self_distance_min = 0.009
    start = scene.frame_current
    for f in range(1, 45):
        scene.frame_set(f)
    current = cloth.evaluated_get(bpy.context.evaluated_depsgraph_get())
    result = [Vector((v.co.x, v.co.z, -v.co.y)) for v in current.data.vertices]
    if min(v.y for v in result) < 0.09 or max(v.y for v in result) > 1.5:
        raise ValueError("Coverlet did not settle on the mattress")
    for obj in (cloth, bed):
        data = obj.data
        bpy.data.objects.remove(obj, do_unlink=True)
        bpy.data.meshes.remove(data)
    scene.frame_set(start)

    def sample(u, t):
        fx, fz = u * nx, t * nz
        i, j = min(nx - 1, int(fx)), min(nz - 1, int(fz))
        a, b = fx - i, fz - j
        k = j * (nx + 1) + i
        return (
            result[k].lerp(result[k + 1], a).lerp(result[k + nx + 1].lerp(result[k + nx + 2], a), b)
        )

    return sample
