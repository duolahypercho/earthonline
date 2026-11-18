#!/usr/bin/env python3
"""Fail when any interior vertex leaves a building's inset footprint.

    python3 scripts/blender-buildings/check_interior_containment.py [--strict] sf-building-262552522 [...]
    python3 scripts/blender-buildings/check_interior_containment.py --dir <build output dir> <id>

Reads public/assets/buildings/<id>/{interior,fine-s*}.glb (or the GLBs in
--dir, a build's output before install) and the footprint
polygon of artifacts/buildings/<id>/spec.json (both in the footprint frame,
glTF +Y up). A vertex FAILs when it is outside the polygon: it pokes through
the facade. A vertex inside the polygon but closer than INSET to an edge is
buried in the wall; it is reported as WARN, and fails too with --strict
(curtains and rods in a window reveal sit there today).
"""
import glob
import json
import math
import os
import struct
import sys

import numpy as np

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
WALL_T = 0.34
INSET = WALL_T - 0.1  # the inner wall face, less a little slack for trim and curtains
COMPONENTS = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
WIDTH = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}


def read_glb(path):
    with open(path, 'rb') as f:
        data = f.read()
    magic, _, _ = struct.unpack_from('<III', data, 0)
    assert magic == 0x46546C67, f'{path} is not a GLB'
    off, gltf, blob = 12, None, b''
    while off < len(data):
        n, kind = struct.unpack_from('<II', data, off)
        chunk = data[off + 8: off + 8 + n]
        if kind == 0x4E4F534A:
            gltf = json.loads(chunk)
        elif kind == 0x004E4942:
            blob = chunk
        off += 8 + n
    return gltf, blob


def accessor(gltf, blob, i):
    acc = gltf['accessors'][i]
    view = gltf['bufferViews'][acc['bufferView']]
    dtype, w = COMPONENTS[acc['componentType']], WIDTH[acc['type']]
    start = view.get('byteOffset', 0) + acc.get('byteOffset', 0)
    stride = view.get('byteStride') or np.dtype(dtype).itemsize * w
    rows = np.frombuffer(blob, np.uint8, count=stride * (acc['count'] - 1) + np.dtype(dtype).itemsize * w, offset=start)
    out = np.lib.stride_tricks.as_strided(rows, (acc['count'], np.dtype(dtype).itemsize * w), (stride, 1))
    return np.frombuffer(out.copy().tobytes(), dtype).reshape(acc['count'], w).astype(np.float64)


def local_matrix(node):
    if 'matrix' in node:
        return np.array(node['matrix'], dtype=np.float64).reshape(4, 4).T
    t = node.get('translation', [0, 0, 0])
    x, y, z, w = node.get('rotation', [0, 0, 0, 1])
    s = node.get('scale', [1, 1, 1])
    r = np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])
    m = np.eye(4)
    m[:3, :3] = r * np.array(s)
    m[:3, 3] = t
    return m


def world_vertices(path):
    """Yield (node name, Nx3 frame-space positions) for every mesh node."""
    gltf, blob = read_glb(path)
    nodes = gltf.get('nodes', [])
    scene = gltf['scenes'][gltf.get('scene', 0)]
    stack = [(i, np.eye(4)) for i in scene['nodes']]
    while stack:
        i, parent = stack.pop()
        node = nodes[i]
        m = parent @ local_matrix(node)
        if 'mesh' in node:
            mesh = gltf['meshes'][node['mesh']]
            for prim in mesh['primitives']:
                p = accessor(gltf, blob, prim['attributes']['POSITION'])
                p = np.einsum('ij,nj->ni', m[:3, :3], p) + m[:3, 3]
                yield node.get('name') or mesh.get('name', '?'), prim.get('material'), gltf.get('materials', []), p
        stack += [(c, m) for c in node.get('children', [])]


def inset_distance(poly, pts):
    """Signed distance of each point to the polygon boundary, positive inside."""
    x, z = pts[:, 0], pts[:, 2]
    inside = np.zeros(len(pts), bool)
    dist = np.full(len(pts), np.inf)
    for (ax, az), (bx, bz) in zip(poly, poly[1:] + poly[:1]):
        crosses = (az > z) != (bz > z)
        xi = ax + (z - az) * (bx - ax) / ((bz - az) or 1e-12)
        inside ^= crosses & (x < xi)
        ex, ez = bx - ax, bz - az
        t = np.clip(((x - ax) * ex + (z - az) * ez) / (ex * ex + ez * ez), 0, 1)
        dist = np.minimum(dist, np.hypot(x - (ax + t * ex), z - (az + t * ez)))
    return np.where(inside, dist, -dist)


def check(building_id, strict=False, glb_dir=None):
    spec = json.load(open(os.path.join(ROOT, 'artifacts/buildings', building_id, 'spec.json')))
    poly = [(p['x'], p['z']) for p in spec['polygon']]
    glb_dir = glb_dir or os.path.join(ROOT, 'public/assets/buildings', building_id)
    files = sorted(glob.glob(os.path.join(glb_dir, 'fine-s*.glb')))
    files.append(os.path.join(glb_dir, 'interior.glb'))
    failures = 0
    for path in files:
        if not os.path.exists(path):
            continue
        for name, mat, mats, pts in world_vertices(path):
            d = inset_distance(poly, pts)
            bad = d < INSET
            if not bad.any():
                continue
            outside = bool((d < 0).any())
            level = 'FAIL' if outside or strict else 'WARN'
            failures += level == 'FAIL'
            worst = int(np.argmin(d))
            label = mats[mat]['name'] if mat is not None and mat < len(mats) else '?'
            where = 'outside the footprint' if outside else f'inside the {INSET:.2f} m wall inset'
            print(f'{level} {building_id} {os.path.basename(path)} {name} [{label}]: {int(bad.sum())} vertices '
                  f'{where}, worst {d[worst]:.3f} m at ({pts[worst, 0]:.3f}, {pts[worst, 1]:.3f}, {pts[worst, 2]:.3f})')
    print(f'{"FAIL" if failures else "ok"} {building_id}: {failures} interior primitive(s) fail containment')
    return failures


if __name__ == '__main__':
    args = sys.argv[1:]
    strict = '--strict' in args
    glb_dir = None
    if '--dir' in args:
        glb_dir = args[args.index('--dir') + 1]
        args.remove(glb_dir)
    ids = [a for a in args if a not in ('--strict', '--dir')]
    if not ids or (glb_dir and len(ids) != 1):
        sys.exit(__doc__)
    sys.exit(1 if sum(check(i, strict, glb_dir) for i in ids) else 0)
