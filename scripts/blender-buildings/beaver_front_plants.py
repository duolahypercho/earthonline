"""Metric rosettes, folded grass blades and seed stalks for the street-side bed."""

import math
import random
from itertools import pairwise

from beaver_dining_parts import mesh
from mathutils import Vector


class Plant:
    def __init__(self):
        self.p = []
        self.uv = []
        self.faces = []

    def vertex(self, p, uv):
        index = len(self.p)
        self.p.append(tuple(p))
        self.uv.append(uv)
        return index

    def rows(self, rows):
        for a, b in pairwise(rows):
            for i in range(len(a) - 1):
                self.faces.append((a[i], b[i], b[i + 1], a[i + 1]))

    def object(self, collection, name, material, root):
        ob = mesh(collection, name, self.p, self.faces, [material], uv=self.uv)
        ob.location = root
        ob.asset_mark()
        ob.asset_data.author = "Earth Online"
        return ob


def leaf_uv(tile, x, t):
    return ((128 * tile + 3 + 122 * x) / 512, 1 - (376 - 371 * t) / 512)


def straw_uv(tile, x, t):
    return ((64 * tile + 3 + 58 * x) / 512, (4 + 119 * t) / 512)


def agave(collection, name, root, size, seed, mat):
    rng = random.Random(seed)
    shape = Plant()
    count = round(29 + 14 * size)
    for j in range(count):
        add_leaf(shape, rng, size, j, count)
    ob = shape.object(collection, name, mat, root)
    ob["leaf_count"] = count
    return ob


def add_leaf(shape, rng, size, j, count):
    age = j / (count - 1)
    dry = j < 3
    a = j * 2.39996 + rng.uniform(-0.21, 0.21)
    radial = Vector((math.cos(a), math.sin(a), 0))
    transverse = Vector((-radial.y, radial.x, 0))
    up = Vector((0, 0, 1))
    reach = size * (1.04 - 0.94 * age**1.35) * rng.uniform(0.90, 1.10)
    rise = size * (0.18 + 0.82 * age**0.86) * rng.uniform(0.91, 1.08)
    width = size * (0.143 - 0.085 * age) * rng.uniform(0.92, 1.18)
    if dry:
        reach *= 0.70
        width *= 0.54
    sweep = size * rng.uniform(-0.12, 0.12)
    twist = rng.uniform(-0.48, 0.48)
    variant = rng.randrange(4)
    dry_band = rng.choice((3, 4, 5))

    def blade(t, q, underside=False):
        bell = math.sin(math.pi * t) ** 0.72
        w = width * (0.14 * (1 - t) + bell * (0.87 + 0.26 * t)) * (1 - 0.75 * t**8)
        center = radial * (size * 0.012 + reach * t**1.28) + transverse * sweep * t**2
        arch = (0.22 + 0.68 * (1 - age)) * math.sin(math.pi * t) / math.pi
        z = rise * (t + arch) + size * 0.022
        if dry:
            z = size * (0.023 + 0.19 * math.sin(math.pi * t * 0.90)) + 0.024 * math.sin(
                t * 8 + a
            )
        ridge = 0.023 * size * bell
        z += (abs(q) ** 1.55 * 0.047 * size - ridge) * bell + q * twist * w * t
        if underside:
            z -= size * (0.003 + 0.022 * (1 - abs(q) ** 1.4)) * bell
        return center + transverse * (w * q) + up * z

    columns = (-1.0, -0.52, 0.0, 0.52, 1.0)
    # Eight cross-section points close each succulent blade, including its fleshy back.
    rows = []
    for t in (0.0, 0.08, 0.17, 0.26, 0.36, 0.47, 0.58, 0.69, 0.79, 0.87, 0.93, 0.97):
        ring = []
        sections = [(q, False) for q in columns] + [
            (0.52, True),
            (0.0, True),
            (-0.52, True),
        ]
        for q, back in sections:
            uv = (
                straw_uv(dry_band, (q + 1) / 2, t)
                if dry
                else leaf_uv(variant, (q + 1) / 2, t)
            )
            ring.append(shape.vertex(blade(t, q, back), uv))
        rows.append(ring + [ring[0]])
    shape.rows(rows)
    tip = shape.vertex(
        blade(1, 0), straw_uv(dry_band, 0.5, 1) if dry else leaf_uv(variant, 0.5, 1)
    )
    for l, r in pairwise(rows[-1]):
        shape.faces.append((r, l, tip))
    if not dry and age < 0.90:
        # The marginal spines hook toward the root; terminal spines are in the blade profile.
        for edge in (-1, 1):
            m = rng.randint(5, 9)
            for k in range(m):
                t = 0.16 + 0.70 * (k + rng.uniform(0.15, 0.75)) / m
                length = size * rng.uniform(0.006, 0.013)
                p = blade(t, edge)
                pts = [
                    blade(t - 0.014, edge),
                    blade(t + 0.012, edge),
                    p
                    + transverse * edge * length
                    - radial * length * 0.56
                    + up * length * 0.25,
                ]
                ids = [
                    shape.vertex(pt, straw_uv(7, 0.25 + 0.25 * i, 0.7))
                    for i, pt in enumerate(pts)
                ]
                shape.faces.append(tuple(ids if edge < 0 else ids[::-1]))


def tussock(collection, name, root, size, seed, mat):
    rng = random.Random(seed)
    shape = Plant()
    zero = Vector((0, 0, 1))
    for j in range(round(130 + 105 * size)):
        angle = rng.random() * math.tau
        side = Vector((-math.sin(angle), math.cos(angle), 0))
        axis = Vector((math.cos(angle), math.sin(angle), 0))
        height = size * rng.uniform(0.27, 1.04)
        spread = size * rng.uniform(0.25, 0.93)
        if j % 6 == 0:
            height *= 0.68
            spread *= 1.08
        twist = rng.uniform(-0.16, 0.16) * size
        flex = rng.uniform(0.53, 1.02)
        width = rng.uniform(0.0030, 0.0072) * (0.7 + 0.5 * size)
        radius = size * rng.uniform(0.012, 0.16)
        band = rng.choices((0, 1, 2, 3, 4), weights=(16, 17, 32, 24, 11))[0]
        rows = []
        for k in range(8):
            t = k / 7
            p = axis * (radius + spread * t**1.73) + side * twist * t**1.3
            p.z = height * (1.72 * t - flex * t**2.7)
            w = width * (1 - 0.98 * t)
            rows.append(
                [
                    shape.vertex(
                        p + side * (w * q) + zero * (0.59 * w * (1 - abs(q))),
                        straw_uv(band, (q + 1) / 2, t),
                    )
                    for q in (-1, 0, 1)
                ]
            )
        shape.rows(rows)
    for k in range(round(12 + 24 * size)):
        a = rng.random() * math.tau
        out = Vector((math.cos(a), math.sin(a), 0))
        side = Vector((-out.y, out.x, 0))
        base = out * rng.uniform(0, 0.07) * size
        lean = size * rng.uniform(0.32, 1.23)
        h = size * rng.uniform(0.83, 1.54)
        ear = rng.uniform(0.13, 0.26)
        rows = []
        for t, r in [
            (0, 0.0016),
            (0.42, 0.0014),
            (0.74 - ear * 0.3, 0.0011),
            (1 - ear, 0.0011),
            (1 - ear + 0.012, 0.0035),
            (1 - ear * 0.5, 0.0043),
            (0.988, 0.0029),
            (1, 0.0004),
        ]:
            p = base + out * (lean * t**1.75) + zero * (h * (1.10 * t - 0.15 * t * t))
            row = []
            for n in range(5):
                b = n * math.tau / 4
                row.append(
                    shape.vertex(
                        p + side * (r * math.cos(b)) + out * (r * math.sin(b)),
                        straw_uv(3 if t > 1 - ear else 2, n / 4, t),
                    )
                )
            rows.append(row)
        shape.rows(rows)
    return shape.object(collection, name, mat, root)


def sage(collection, name, root, size, seed, mat):
    """Branching gray-green filler behind the rosettes and along the gravel edge."""
    rng = random.Random(seed)
    shape = Plant()
    up = Vector((0, 0, 1))
    for i in range(28):
        azimuth = rng.uniform(0, math.tau)
        outward = Vector((math.cos(azimuth), math.sin(azimuth), 0))
        side = up.cross(outward)
        end = (outward * rng.uniform(0.28, 0.88) + up * rng.uniform(0.38, 1.12)) * size
        for j in range(3):
            t0 = j * 0.21
            start = end * t0
            d = end * (1 - t0) * rng.uniform(0.50, 1.0) + side * size * rng.uniform(
                -0.31, 0.31
            )
            old = start
            for k in range(1, 7):
                t = k / 6
                p = start + d * t + up * (size * 0.081 * math.sin(t * math.pi))
                r = (0.0034 - 0.0018 * t) * min(1, size * 2)
                line = [
                    shape.vertex(v, straw_uv(5, u, t))
                    for v, u in (
                        (old - side * r, 0.0),
                        (old + side * r, 1.0),
                        (p + side * r, 1.0),
                        (p - side * r, 0.0),
                    )
                ]
                shape.faces.append(tuple(line))
                for sign in (-1, 1):
                    leaf = (
                        side * (sign * rng.uniform(0.35, 0.85))
                        + up * rng.uniform(0.33, 0.91)
                        + outward * 0.40
                    ).normalized()
                    along = leaf * size * rng.uniform(0.076, 0.20)
                    across = (
                        up.cross(leaf).normalized() * size * rng.uniform(0.016, 0.033)
                    )
                    mid = p + along * 0.49
                    points = [
                        p,
                        mid + across,
                        mid + up * 0.003,
                        p + along,
                        mid - across,
                    ]
                    tile = (i + j + k) % 4
                    coords = [
                        (0.5, 0.21),
                        (0.05, 0.54),
                        (0.5, 0.55),
                        (0.5, 0.84),
                        (0.95, 0.54),
                    ]
                    ids = [
                        shape.vertex(v, leaf_uv(tile, *uv))
                        for v, uv in zip(points, coords)
                    ]
                    for f in ((0, 1, 2), (1, 3, 2), (2, 3, 4), (0, 2, 4)):
                        shape.faces.append(tuple(ids[n] for n in f))
                old = p
    return shape.object(collection, name, mat, root)
