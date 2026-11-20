"""Bake diffuse window and fixture transfer into the terrace's two light coordinates."""

import math
from mathutils import Vector
from mathutils.bvhtree import BVHTree


def subdivide_quad(points, face, spacing):
    """Generate a bilinear patch with regular lighting samples on large flat faces."""
    a, b, c, d = [Vector(points[i]) for i in face]
    nu = max(1, math.ceil(max((b - a).length, (c - d).length) / spacing))
    nv = max(1, math.ceil(max((d - a).length, (c - b).length) / spacing))
    if nu * nv == 1:
        return None
    verts = [tuple(a.lerp(d, j / nv).lerp(b.lerp(c, j / nv), i / nu)) for j in range(nv + 1) for i in range(nu + 1)]
    quads = []
    for j in range(nv):
        for i in range(nu):
            k = j * (nu + 1) + i
            quads.append((k, k + 1, k + nu + 2, k + nu + 1))
    return verts, quads


def bake(objects, rise):
    vertices, faces = [], []
    for obj in objects:
        mesh = obj.data
        if mesh.materials[0].name.startswith("glass"):
            continue
        offset = len(vertices)
        vertices.extend(v.co.copy() for v in mesh.vertices)
        faces.extend([offset + v for v in f.vertices] for f in mesh.polygons)
    tree = BVHTree.FromPolygons(vertices, faces)
    # Stratified samples over each sash. The pendant samples sit inside its lower rim.
    windows = [
        [
            Vector((0.73 + (i + 0.5) * 3.82 / 7, -depth, level * rise + 0.94 + (j + 0.5) * 1.53 / 4))
            for depth in (-0.12, 13.14)
            for j in range(4)
            for i in range(7)
        ]
        for level in range(3)
    ]
    lamp = [Vector((2.37 + dx, -7.51 + dz, 1.975)) for dx, dz in [(-0.12, -0.09), (0.13, -0.09), (0, 0.13)]]
    bounce_cache = {}
    direct_cache = {}

    def visible_energy(p, n, sources, window):
        energy = 0.0
        for q in sources:
            delta = q - p
            d2 = delta.length_squared
            if d2 < 1e-6:
                continue
            direction = delta / math.sqrt(d2)
            cos_light = abs(direction.y) if window else max(direction.z, 0.0)
            projected = max(0.0, n.dot(direction)) * cos_light
            if projected > 0.002 and tree.ray_cast(p, direction, math.sqrt(d2) - 0.001)[0] is None:
                energy += projected / (d2 + (0.067 if window else 0.04))
        return energy

    def direct(p, n, level):
        # Finite areas and Lambert's cosine integral. Lamp intensity is calibrated
        # separately from daylight so its local pool is retained after sunset.
        day = visible_energy(p, n, windows[level], True) * (3.82 * 1.53 / 28) / math.pi
        lum = visible_energy(p, n, lamp, False) * 0.8 / len(lamp) if level == 0 else 0.0
        return day, lum

    def bounce(p, n, level):
        key = (level, *(round(v / 0.24) for v in p), *(round(v * 4) for v in n))
        if key in bounce_cache:
            return bounce_cache[key]
        tangent = n.cross(Vector((0, 0, 1)) if abs(n.z) < 0.85 else Vector((1, 0, 0))).normalized()
        bitangent = n.cross(tangent)
        day, lum = 0.0, 0.0
        for i in range(16):
            r = math.sqrt((i + 0.5) / 16)
            phi = i * 2.399963
            direction = tangent * (r * math.cos(phi)) + bitangent * (r * math.sin(phi)) + n * math.sqrt(1 - r * r)
            hit, hn, _, _ = tree.ray_cast(p, direction, 15)
            if hit is not None:
                if direction.dot(hn) > 0:
                    hn.negate()
                a, b = direct(hit + hn * 0.003, hn, level)
                day += a
                lum += b
        bounce_cache[key] = (day * 0.42 / 16, lum * 0.42 / 16)
        return bounce_cache[key]

    for obj in objects:
        mesh = obj.data
        uv = mesh.uv_layers.new(name="Diffuse light transfer")
        # Compute per corner to preserve the two different normals at a hard edge.
        for face in mesh.polygons:
            for idx in face.loop_indices:
                loop = mesh.loops[idx]
                p = mesh.vertices[loop.vertex_index].co
                n = mesh.corner_normals[idx].vector
                point = p + n * 0.004
                if not (0.06 < point.x < 6.94 and -12.96 < point.y < -0.04 and 0 <= point.z < 3 * rise):
                    uv.data[idx].uv = (0.0, 1.0)
                    continue
                level = min(2, int(point.z / rise))
                key = (*(round(v * 300) for v in p), *(round(v * 80) for v in n))
                if key not in direct_cache:
                    day, lum = direct(point, n, level)
                    bd, bl = bounce(point, n, level)
                    direct_cache[key] = (math.sqrt(min(1.0, day + bd)), 1.0 - math.sqrt(min(1.0, lum + bl)))
                # Blender's glTF exporter flips V.
                uv.data[idx].uv = direct_cache[key]
