"""Sewn upholstery, timber joinery and a bound wool rug in house-local metres."""

import math
import random

from mathutils import Vector


def rounded_outline(a, b, radius, sides=10):
    """A sampled rounded rectangle with continuous straight sides for modeling sewn panels."""
    points = []
    corners = [
        (a - radius, b - radius, 0),
        (-a + radius, b - radius, 90),
        (-a + radius, -b + radius, 180),
        (a - radius, -b + radius, 270),
    ]
    for index, (cx, cy, start) in enumerate(corners):
        for j in range(sides + 1):
            angle = math.radians(start + 90 * j / sides)
            points.append((cx + radius * math.cos(angle), cy + radius * math.sin(angle)))
        nx, ny, deg = corners[(index + 1) % 4]
        end = (nx + radius * math.cos(math.radians(deg)), ny + radius * math.sin(math.radians(deg)))
        last = points[-1]
        for j in range(1, 1 + int(math.dist(last, end) / 0.060)):
            fraction = j / (1 + int(math.dist(last, end) / 0.060))
            points.append(
                (last[0] + (end[0] - last[0]) * fraction, last[1] + (end[1] - last[1]) * fraction)
            )
    return points


def cord(emit, material, path, radius):
    """Closed welt with its own silhouette at the edge of a sewn panel."""
    vertices = []
    count = len(path)
    rings = 6
    for i, p in enumerate(path):
        t = (path[(i + 1) % count] - path[i - 1]).normalized()
        seed = Vector((0, 1, 0)) if abs(t.y) < 0.82 else Vector((1, 0, 0))
        u = t.cross(seed).normalized()
        v = t.cross(u)
        for j in range(rings):
            a = math.tau * j / rings
            vertices.append(tuple(p + radius * (math.cos(a) * u + math.sin(a) * v)))
    emit(
        material,
        vertices,
        [
            (
                i * rings + j,
                i * rings + (j + 1) % rings,
                ((i + 1) % count) * rings + (j + 1) % rings,
                ((i + 1) % count) * rings + j,
            )
            for i in range(count)
            for j in range(rings)
        ],
        True,
    )


def pad(emit, material, piping, centre, axis_u, axis_v, face_axis, a, b, half, seed, loose=False):
    """Two crowned fabric panels joined around a boxed or knife-edge gusset."""
    centre, u_axis, v_axis, n_axis = map(Vector, (centre, axis_u, axis_v, face_axis))
    outline = rounded_outline(a, b, min(a, b) * (0.42 if loose else 0.23))
    crown = 0.081 if loose else 0.033
    phase = seed * 1.461
    rng = random.Random(230 + seed)
    tucks = [
        (rng.uniform(-0.86, 0.86), rng.uniform(0.06, 0.13), rng.uniform(0.30, 0.88)) for _ in range(5)
    ]

    def point(x, y, h):
        return centre + x * u_axis + y * v_axis + h * n_axis

    def face(x, y, sign):
        u, v = x / a, y / b
        field = max(0, (1 - u * u) * (1 - v * v))
        slope = 0.011 * math.sin(phase + u * 2 + v * 1.3)
        if loose:
            slope += 0.028 * math.sin(3.6 * u + 2.2 * v + phase) * field**0.6
            slope -= 0.024 * math.exp(-(((u + 0.50 - 0.32 * v) / 0.14) ** 2)) * field
        # A compressed patch and shallow fan-shaped tucks approach the boxed seam.
        dent = 0.040 * math.exp(
            -(((u - 0.24 * math.sin(phase)) / 0.54) ** 2) - ((v + 0.27) / 0.55) ** 2
        )
        folds = 0.0
        for centre_line, width, reach in tucks:
            d = 1 - abs(v)
            centre_u = centre_line + 0.26 * d * math.sin(phase + 3 * centre_line)
            q = (u - centre_u) / width
            folds += (
                (0.012 if loose else 0.008) * (1 - 2 * q * q) * math.exp(-q * q - (d / reach) ** 2 * 7)
            )
        return (
            sign * (half + crown * field**0.68) + slope + (folds - dent * field if sign > 0 else 0.0)
        )

    n = len(outline)
    verts = []
    faces = []
    # A continuous rounded skirt. The lower edge of loose pillows pinches closed.
    for step in range(7):
        t = step / 6
        for x, y in outline:
            swell = (0.002 if loose else 0.008) * math.sin(math.pi * t)
            f = face(x, y, -1) * (1 - t) + face(x, y, 1) * t
            verts.append(tuple(point(x * (1 + swell / a), y * (1 + swell / b), f)))
    for j in range(6):
        faces.extend(
            (j * n + i, j * n + (i + 1) % n, (j + 1) * n + (i + 1) % n, (j + 1) * n + i)
            for i in range(n)
        )
    for sign in (-1, 1):
        previous = 0 if sign < 0 else n * 6
        for ring in range(1, 15):
            t = 1 - ring / 15
            offset = len(verts)
            for x, y in outline:
                verts.append(tuple(point(x * t, y * t, face(x * t, y * t, sign))))
            for i in range(n):
                f = (previous + i, previous + (i + 1) % n, offset + (i + 1) % n, offset + i)
                faces.append(f if sign > 0 else f[::-1])
            previous = offset
        c = len(verts)
        verts.append(tuple(point(0, 0, face(0, 0, sign))))
        for i in range(n):
            tri = (previous + i, previous + (i + 1) % n, c)
            faces.append(tri if sign > 0 else tri[::-1])
    if u_axis.cross(v_axis).dot(n_axis) < 0:
        faces = [f[::-1] for f in faces]
    emit(material, verts, faces, True)
    if piping:
        for side in (1,) if loose else (-1, 1):
            rim = [point(x, y, face(x, y, side)) for x, y in outline]
            cord(emit, piping, rim, 0.0028 if loose else 0.0036)


def sofa(emit, box, turned, finish, boxes):
    seat = finish("canvas-plain", "b29d7e", 0.94)
    back = finish("canvas-plain", "af997d", 0.95)
    arm = finish("canvas-plain", "aa9478", 0.94)
    welt = finish("canvas-plain", "a79175", 0.93)
    shadow = finish("canvas-plain", "57483c", 0.96)
    wood = finish("furniture-oak", "80634b", 0.52)
    accent = finish("canvas-plain", "727a69", 0.96)
    sand = finish("canvas-plain", "c8bba4", 0.94)
    for x in (0.68, 1.37):
        for z in (1.61, 3.78):
            turned(wood, x, 0.008, z, [(0.026, 0), (0.030, 0.022), (0.041, 0.13)], 14, closed=True)
    box(wood, (0.64, 0.120, 1.535), (1.42, 0.168, 3.865), bevel=0.009)
    box(arm, (0.57, 0.163, 1.50), (1.47, 0.356, 3.90), bevel=0.038)
    box(shadow, (0.63, 0.334, 1.58), (1.435, 0.387, 3.83), bevel=0.021)
    for j, z in enumerate((1.986, 2.70, 3.411)):
        pad(
            emit,
            seat,
            welt,
            (1.075, 0.463 + 0.006 * math.sin(j * 2), z),
            (1, 0, 0),
            (0, 0, 1),
            (0, 1, 0),
            0.414,
            0.360,
            0.086,
            j + 1,
        )
        tilt = 0.14 + 0.024 * math.cos(j * 3)
        pad(
            emit,
            back,
            welt,
            (0.725 + 0.008 * j, 0.764, z + 0.007 * math.cos(j)),
            (0, 0, 1),
            (-math.sin(tilt), math.cos(tilt), 0),
            (math.cos(tilt), math.sin(tilt), 0),
            0.361,
            0.247,
            0.072,
            j + 5,
        )
    for j, z in enumerate((1.535, 3.865)):
        pad(
            emit,
            arm,
            welt,
            (1.015, 0.501, z),
            (1, 0, 0),
            (0, 1, 0),
            (0, 0, 1),
            0.478,
            0.205,
            0.079,
            j + 10,
        )

    def pillow(mat, mid, tilt, roll, seed):
        axis = Vector((math.cos(tilt), math.sin(tilt), 0))
        up = Vector((-math.sin(tilt), math.cos(tilt), 0))
        long = Vector((0, 0, 1))
        u = long * math.cos(roll) + up * math.sin(roll)
        v = up * math.cos(roll) - long * math.sin(roll)
        pad(emit, mat, None, mid, u, v, axis, 0.22, 0.204, 0.005, seed, True)

    pillow(sand, (0.838, 0.710, 1.876), 0.32, -0.20, 18)
    pillow(accent, (0.843, 0.719, 3.393), 0.44, 0.28, 19)
    boxes.append([[0.57, 0.12, 1.47], [1.48, 0.67, 3.96]])


def tapered_leg(emit, mat, top, foot, top_radius, foot_radius):
    top, foot = Vector(top), Vector(foot)
    axis = (top - foot).normalized()
    u = axis.cross(Vector((0, 0, 1))).normalized()
    v = axis.cross(u)
    vertices = []
    n = 16
    for t, radius in [
        (0, foot_radius * 0.90),
        (0.02, foot_radius),
        (0.79, top_radius * 0.96),
        (1, top_radius),
    ]:
        c = foot.lerp(top, t)
        vertices.extend(
            tuple(c + radius * (math.cos(j * math.tau / n) * u + math.sin(j * math.tau / n) * v))
            for j in range(n)
        )
    emit(
        mat,
        vertices,
        [
            (i * n + j, i * n + (j + 1) % n, (i + 1) * n + (j + 1) % n, (i + 1) * n + j)
            for i in range(3)
            for j in range(n)
        ],
        True,
    )


def coffee_table(emit, box, finish, boxes):
    timber = finish("furniture-oak", "a18061", 0.49)
    edge = finish("furniture-oak", "8c6a4c", 0.51)
    for x in (2.136, 2.873):
        for z in (2.095, 3.295):
            tapered_leg(
                emit,
                edge,
                (x, 0.402, z),
                (x + (0.046 if x > 2.5 else -0.046), 0.014, z + (0.044 if z > 2.7 else -0.044)),
                0.027,
                0.015,
            )
    for x in (2.136, 2.873):
        box(edge, (x - 0.014, 0.339, 2.093), (x + 0.014, 0.410, 3.298), bevel=0.004)
    for z in (2.095, 3.295):
        box(edge, (2.132, 0.344, z - 0.014), (2.878, 0.411, z + 0.014), bevel=0.004)
    outline = rounded_outline(0.46, 0.70, 0.098)
    vv = []
    n = len(outline)
    for h, scale in [(0.399, 0.97), (0.409, 0.993), (0.427, 1), (0.439, 0.99)]:
        vv.extend((2.505 + x * scale, h, 2.70 + z * scale) for x, z in outline)
    # Separate the flat top from the radiused edge so its grain catches even light.
    emit(
        timber,
        vv,
        [
            (j * n + i, (j + 1) * n + i, (j + 1) * n + (i + 1) % n, j * n + (i + 1) % n)
            for i in range(n)
            for j in range(3)
        ],
        True,
    )
    # Resolve shadows beneath the cup foot and the book on the tabletop.
    nx, nz = 42, 64
    top = []
    for j in range(nz + 1):
        z = 0.70 * (2 * j / nz - 1)
        arc = max(0.0, abs(z) - (0.70 - 0.098))
        half_width = 0.46 - 0.098 + math.sqrt(max(0.0, 0.098**2 - arc**2))
        for i in range(nx + 1):
            x = half_width * (2 * i / nx - 1)
            top.append((2.505 + 0.99 * x, 0.439, 2.70 + 0.99 * z))
    stride = nx + 1
    emit(
        timber,
        top,
        [
            (k, k + stride, k + stride + 1, k + 1)
            for j in range(nz)
            for i in range(nx)
            for k in (j * stride + i,)
        ],
        True,
    )
    boxes.append([[2.05, 0, 2.0], [2.96, 0.44, 3.4]])


def book(box, finish):
    cloth = finish("canvas-plain", "516259", 0.91)
    paper = finish("book-leaves", "d6c8a9", 0.88)
    for y in (0.441, 0.472):
        box(cloth, (2.27, y, 2.32), (2.504, y + 0.0032, 2.635), bevel=0.0015)
    box(paper, (2.274, 0.445, 2.325), (2.498, 0.471, 2.631), bevel=0.002)
    box(cloth, (2.267, 0.443, 2.32), (2.276, 0.475, 2.635), bevel=0.003)
    ribbon = finish("cotton-ribbon", "83654c", 0.96)
    box(ribbon, (2.473, 0.452, 2.624), (2.481, 0.453, 2.649), bevel=0.0003)


def rug(emit, box, finish):
    wool = finish("woven-carpet", "ffffff", 0.96)
    hem = finish("flax-linen", "a79a7c", 0.98)
    # Slight irregularity in the binding, each tassel rooted in the selvedge.
    x0, x1, z0, z1 = 0.65, 4.29, 1.41, 4.01
    nx, nz = 108, 78
    pts = []
    for j in range(nz + 1):
        v = j / nz
        for i in range(nx + 1):
            u = i / nx
            fluting = 0.0016 * math.cos(v * 73 + u * 11) + 0.0013 * math.sin(u * 97 + v * 7)
            x = (
                x0
                + (x1 - x0) * u
                + 0.005 * math.sin(v * 13 + 0.5) * (1 - u) ** 8
                + 0.006 * math.sin(v * 15) * u**8
            )
            z = (
                z0
                + (z1 - z0) * v
                + 0.005 * math.sin(u * 16) * (1 - v) ** 8
                + 0.005 * math.sin(u * 19) * v**8
            )
            pts.append((x, 0.016 + fluting, z))
    stride = nx + 1
    emit(
        wool,
        pts,
        [
            (k, k + stride, k + stride + 1, k + 1)
            for j in range(nz)
            for i in range(nx)
            for k in (j * stride + i,)
        ],
        True,
    )
    rng = random.Random(445)
    vs = []
    fs = []
    for side in (-1, 1):
        for i in range(245):
            z = z0 + 0.008 + (z1 - z0 - 0.016) * i / 244
            row = round(i / 244 * nz)
            x, _, _ = pts[row * stride + (0 if side < 0 else nx)]
            root = Vector((x - side * 0.002, 0.014, z))
            end = root + Vector((side * rng.uniform(0.020, 0.052), -0.006, rng.uniform(-0.007, 0.007)))
            k = len(vs)
            for t, half in ((0, 0.0035), (0.40, 0.0027), (1, 0.0018)):
                m = root.lerp(end, t)
                vs.extend([tuple(m + Vector((0, 0, -half))), tuple(m + Vector((0, 0, half)))])
            fs.extend([(k, k + 1, k + 3, k + 2), (k + 2, k + 3, k + 5, k + 4)])
    emit(hem, vs, fs, True)


def chair(emit, box, finish, boxes, x, y, z):
    grain = finish("furniture-oak", "a18061", 0.49)
    end = finish("furniture-oak", "8c6a4c", 0.51)
    for dx in (0.077, 0.470):
        for dz in (0.079, 0.460):
            tapered_leg(
                emit,
                end,
                (x + dx, y + 0.436, z + dz),
                (
                    x + dx + (0.033 if dx > 0.27 else -0.033),
                    y + 0.006,
                    z + dz + (0.035 if dz > 0.27 else -0.035),
                ),
                0.020,
                0.012,
            )
    outline = rounded_outline(0.249, 0.229, 0.061)
    vs = []
    fs = []
    n = len(outline)
    for j in range(10):
        f = (10 - j) / 10
        for u, v in outline:
            bowl = 0.020 * (1 - f * f) * (0.85 + 0.15 * v / 0.229)
            vs.append((x + 0.274 + u * f, y + 0.454 - bowl, z + 0.262 + v * f))
        if j:
            for i in range(n):
                fs.append(((j - 1) * n + i, j * n + i, j * n + (i + 1) % n, (j - 1) * n + (i + 1) % n))
    center = len(vs)
    vs.append((x + 0.274, y + 0.434, z + 0.262))
    fs.extend((9 * n + i, center, 9 * n + (i + 1) % n) for i in range(n))
    bottom = len(vs)
    vs.extend((x + 0.274 + u * 0.99, y + 0.433, z + 0.262 + v * 0.99) for u, v in outline)
    fs.extend((i, (i + 1) % n, bottom + (i + 1) % n, bottom + i) for i in range(n))
    emit(grain, vs, fs, True)
    for dx in (0.072, 0.475):
        tapered_leg(
            emit, end, (x + dx, y + 0.891, z + 0.55), (x + dx, y + 0.432, z + 0.441), 0.018, 0.022
        )
    # The thin bentwood crest wraps around the sitter rather than forming a straight rail.
    vs = []
    fs = []
    for i in range(33):
        u = (i - 16) / 16
        for j in range(4):
            v = j / 3
            back = 0.527 + 0.047 * (1 - u * u) + 0.016 * v
            h = 0.748 + 0.026 * u * u + 0.096 * v
            vs.extend(
                [
                    (x + 0.274 + 0.250 * u, y + h, z + back),
                    (x + 0.274 + 0.250 * u, y + h, z + back + 0.014),
                ]
            )
    for i in range(32):
        for j in range(3):
            k = i * 8 + j * 2
            fs.extend([(k, k + 2, k + 10, k + 8), (k + 1, k + 9, k + 11, k + 3)])
        k = i * 8
        fs.extend([(k, k + 8, k + 9, k + 1), (k + 6, k + 7, k + 15, k + 14)])
    emit(grain, vs, fs, True)
    boxes.append([[x + 0.035, y, z + 0.045], [x + 0.512, y + 0.45, z + 0.496]])


def throw(emit, finish):
    """Settle a woven lap blanket over the occupied arm and adjacent seat in Blender."""
    import bpy

    scene = bpy.context.scene
    proxies = []
    for centre, size, radius in [
        ((1.015, -3.865, 0.504), (0.96, 0.17, 0.418), 0.045),
        ((1.075, -3.415, 0.475), (0.81, 0.71, 0.205), 0.048),
    ]:
        bpy.ops.mesh.primitive_cube_add(size=1, location=centre)
        obj = bpy.context.object
        obj.dimensions = size
        bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
        edge = obj.modifiers.new("Rounded upholstery", "BEVEL")
        edge.width = radius
        edge.segments = 5
        obj.modifiers.new("Woven throw collision", "COLLISION")
        obj.collision.thickness_outer = 0.008
        proxies.append(obj)
    nx, nz = 32, 54
    verts = []
    polys = []
    for j in range(nz + 1):
        t = j / nz
        for i in range(nx + 1):
            u = i / nx
            x = 0.78 + 0.61 * u + 0.052 * math.sin(t * 3.4 + 0.45 * u)
            z = 3.44 + 0.909 * t - 0.042 * u + 0.017 * math.sin(3.2 * u)
            gather = 0.064 * math.sin(3 * math.pi * u + 0.64 * math.sin(t * 4)) ** 2
            h = 0.745 + gather * (0.35 + 0.65 * abs(2 * t - 1)) + 0.023 * math.sin(10 * t + u)
            verts.append((x, -z, h))
    width = nx + 1
    for j in range(nz):
        for i in range(nx):
            k = j * width + i
            polys.append((k, k + width, k + width + 1, k + 1))
    mesh = bpy.data.meshes.new("Blanket warp and weft")
    mesh.from_pydata(verts, [], polys)
    cloth = bpy.data.objects.new("Lap blanket", mesh)
    scene.collection.objects.link(cloth)
    pins = cloth.vertex_groups.new(name="Tucked at the arm")
    pins.add([24 * width + i for i in (3, 4, 16, 17, 27)], 0.76, "REPLACE")
    modifier = cloth.modifiers.new("Settle the woven cloth", "CLOTH")
    sett = modifier.settings
    sett.quality = 7
    sett.mass = 0.30
    sett.tension_stiffness = 35
    sett.compression_stiffness = 30
    sett.shear_stiffness = 18
    sett.bending_stiffness = 0.29
    sett.vertex_group_mass = pins.name
    collision = modifier.collision_settings
    collision.distance_min = 0.008
    collision.use_self_collision = True
    collision.self_distance_min = 0.008
    first = scene.frame_current
    for i in range(1, 46):
        scene.frame_set(i)
    settled = cloth.evaluated_get(bpy.context.evaluated_depsgraph_get())
    points = [Vector((v.co.x, v.co.z, -v.co.y)) for v in settled.data.vertices]
    if not (0.064 < min(p.y for p in points) < 0.56 and max(p.y for p in points) < 1.1):
        raise ValueError("Blanket failed to settle on the sofa arm")
    for o in [cloth, *proxies]:
        data = o.data
        bpy.data.objects.remove(o, do_unlink=True)
        bpy.data.meshes.remove(data)
    scene.frame_set(first)
    yarn = finish("flax-linen", "a6a593", 0.96)
    band = finish("flax-linen", "777e73", 0.95)
    flax = finish("flax-linen", "c6bca5", 0.95)
    grouped = {yarn: [], band: [], flax: []}
    for j in range(nz):
        for i in range(nx):
            idx = j * nx + i
            material = band if j in (3, 4, 48, 49) else flax if j in (1, 6, 46, 51) else yarn
            grouped[material].append(polys[idx])
    for material, faces in grouped.items():
        used = sorted({i for f in faces for i in f})
        ids = {k: i for i, k in enumerate(used)}
        emit(
            material, [tuple(points[k]) for k in used], [tuple(ids[k] for k in f) for f in faces], True
        )
    rng = random.Random(918)
    fv = []
    ff = []
    for end, previous in ((0, 1), (nz, nz - 1)):
        for i in range(120):
            u = (i + 0.3) / 120 * nx
            a = min(nx - 1, int(u))
            t = u - a
            p = points[end * width + a].lerp(points[end * width + a + 1], t)
            q = points[previous * width + a].lerp(points[previous * width + a + 1], t)
            direction = (p - q).normalized()
            head = len(fv)
            right = Vector((0.0011, 0, 0))
            tip = (
                p
                + direction * rng.uniform(0.016, 0.029)
                + Vector((rng.uniform(-0.004, 0.004), -0.009, 0))
            )
            fv.extend(tuple(q) for q in (p - right, p + right, tip + right * 0.6, tip - right * 0.6))
            ff.append((head, head + 1, head + 2, head + 3))
    emit(flax, fv, ff, True)


def coffee_cup(emit, turned, finish):
    glaze = finish("stoneware-glaze", "85918a", 0.31)
    clay = finish("stoneware-foot", "a78c72", 0.77)
    brew = finish("brewed-coffee", "39251c", 0.15)
    x, y, z = 2.69, 0.440, 2.86
    turned(clay, x, y, z, [(0.029, 0), (0.032, 0.003), (0.032, 0.008)], 36, closed=True)
    turned(
        glaze,
        x,
        y,
        z,
        [
            (0.031, 0.006),
            (0.041, 0.011),
            (0.046, 0.032),
            (0.045, 0.082),
            (0.044, 0.090),
            (0.042, 0.092),
            (0.039, 0.09),
            (0.039, 0.079),
            (0.038, 0.028),
            (0.031, 0.015),
            (0.002, 0.014),
        ],
        44,
    )
    turned(brew, x, y, z, [(0.001, 0.069), (0.039, 0.069), (0.039, 0.070)], 44)
    points = []
    faces = []
    for i in range(29):
        t = i / 28 * math.pi
        c = Vector(
            (x + 0.038 + 0.034 * math.sin(t), y + 0.051 + 0.028 * math.cos(t), z + 0.008 * math.sin(t))
        )
        tip = Vector((0.034 * math.cos(t), -0.028 * math.sin(t), 0.008 * math.cos(t))).normalized()
        u = tip.cross(Vector((0, 0, 1))).normalized()
        v = tip.cross(u)
        for j in range(10):
            a = j / 10 * math.tau
            points.append(tuple(c + 0.0057 * (u * math.cos(a) + v * math.sin(a))))
    for i in range(28):
        for j in range(10):
            faces.append(
                (i * 10 + j, i * 10 + (j + 1) % 10, (i + 1) * 10 + (j + 1) % 10, (i + 1) * 10 + j)
            )
    emit(glaze, points, faces, True)


def sideboard(emit, box, turned, finish):
    """Freestanding 1.52 m dining-room cabinet with inset doors and a narrow plinth."""
    timber = finish("furniture-oak", "896c50", .55)
    endgrain = finish("furniture-oak", "745b45", .61)
    inset = finish("furniture-oak", "785e46", .58)
    brass = finish("satin-nickel", "a79169", .46, .50)
    for x in (.235, .525):
        for z in (6.91, 8.22):
            turned(endgrain, x, 0, z, [(.027, 0), (.035, .10), (.039, .14)], 12, closed=True)
    box(timber, (.169, .125, 6.81), (.588, .808, 8.32), True, .010)
    box(endgrain, (.157, .808, 6.805), (.604, .848, 8.325), False, .009)
    for i in range(3):
        z = 6.828 + .495 * i
        box(endgrain, (.586, .176, z), (.606, .780, z + .479), False, .004)
        box(inset, (.605, .216, z + .035), (.609, .741, z + .444), False, .003)
        for dz in (0, .086):
            box(brass, (.607, .635, z + .278 + dz), (.634, .650, z + .289 + dz), False, .004)
        box(brass, (.628, .633, z + .277), (.642, .652, z + .376), False, .005)
    crock = finish("stoneware-glaze", "9b9985", .34)
    turned(crock, .391, .848, 7.94, [(.061, 0), (.072, .012), (.122, .077), (.138, .102),
                                    (.133, .109), (.124, .105), (.113, .08), (.054, .016)], 40, closed=True)
