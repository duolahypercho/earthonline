"""Headless Blender builder for one authored building.

    blender -b --factory-startup --python scripts/blender-buildings/build_building.py -- \
        --spec artifacts/buildings/<id>/spec.json --style artifacts/buildings/<id>/style.json \
        --out <dir> [--render <dir>]

Reads the spec written by make-spec.mjs (footprint frame, storeys, edge roles)
and the per-building style (what the builder changes after looking at the
street-level reference by eye), and writes:

    <out>/exterior.glb   shell, openings, glazing, trim, storefront, roof
    <out>/interior.glb   every storey: slabs, stair core, partitions, furniture, lights
                         (not written with --scope exterior: see exterior_scope_* below)
    <out>/physics.json   input for compileAuthoredBuildingPhysics (frame space)
    <out>/report.json    triangle / object counts per part

Everything is authored in the footprint frame: +X along the front edge, +Z out
of the front edge, y up, origin at the frame centre on the ground plane. Blender
is Z-up, so a frame point (x, y, z) is Blender (x, -z, y); the glTF exporter's
+Y-up conversion hands the frame coordinates back unchanged.

Exterior material names are the runtime's authored-kit names
(src/world/buildings/authored/kit/materials.js) so the game swaps in its own
world-anchored materials and the GLB carries no textures. Interior materials
are named `int-*` and ship as plain PBR colours.

No randomness outside a seeded RNG: the same spec + style gives the same bytes.
"""

import argparse
import json
import math
import os
import random
import sys

import bmesh
import bpy
from mathutils import Vector

# ---------------------------------------------------------------- arguments


def parse_args():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument('--spec', required=True)
    parser.add_argument('--style', default=None)
    parser.add_argument('--out', required=True)
    parser.add_argument('--render', default=None, help='write preview PNGs here (outside the repo)')
    parser.add_argument('--furniture-lib', default=None, help='prepared CC0 furniture GLBs (off-repo)')
    parser.add_argument('--library', default=None, help='library.json written by prepare_library.py')
    parser.add_argument('--textures', default=None, help='CC0 surface textures (off-repo)')
    # What to build. The city-wide default lives in scope.mjs (BUILDING_SCOPE),
    # which build.mjs always passes; 'full' here only serves direct Blender runs.
    parser.add_argument('--scope', default='full', choices=('full', 'exterior'),
                        help='exterior: the shell only, no interior, furniture or enterable floors')
    parser.add_argument('--seethrough-glass', default='',
                        help='comma list of see-through kit glass (exterior scope; from the kit, via scope.mjs)')
    parser.add_argument('--exterior-glass', default='glass-shell',
                        help='opaque kit pane that replaces see-through glass on an exterior-only build')
    return parser.parse_args(argv)


# Real furniture and surface textures, when the off-repo library is present.
# Without it every piece falls back to its box stand-in, so a clean clone still builds.
LIBRARY = {'dir': None, 'pieces': {}, 'textures': None, 'by_reference': True}


# ---------------------------------------------------------------- constants

WALL_T = 0.34          # exterior wall thickness
SLAB_T = 0.26          # floor slab thickness
PART_T = 0.12          # interior partition thickness
DOOR_W = 0.9
DOOR_H = 2.1
RISER = 0.185
TREAD = 0.27
FLIGHT_W = 1.1
STAIR_GAP = 0.2
LANDING_D = 1.2
CORRIDOR_W = 1.4
MIN_STOREY_M = 2.6     # below this a counted storey is a mezzanine

DEFAULT_STYLE = {
    'wall': 'brick-running-bond',
    'wallTint': None,
    'trim': 'cast-stone-trim',
    'frame': 'painted-white',
    'glass': 'glass-clear',
    'storefrontGlass': 'glass-low-iron',
    'storefrontFrame': 'painted-steel',
    'bulkhead': 'painted-steel-mid',
    'roof': 'asphalt-roof',
    'awning': 'awning-canvas',
    'bayM': 2.3,
    'windowW': 1.05,
    'windowSillM': 0.8,
    'windowHeadM': 2.35,
    'endPierM': 0.9,
    'cornice': True,
    'corniceProjectionM': 0.34,
    'beltCourse': True,
    'windowHoods': True,
    'storefront': {'bulkheadM': 0.55, 'transomM': 0.75, 'pierM': 0.45, 'awning': True},
    'ground': 'auto',     # restaurant | shop | lobby | auto (from amenity/shop/usage)
    'upper': 'auto',      # rooms | office | apartment | auto (from use code)
    'seed': 1,
}

# Viewport / fallback colours. Runtime replaces kit-named materials entirely.
KIT_PREVIEW = {
    'brick-running-bond': (0.42, 0.2, 0.14, 0.85, 0.0),
    'brick-common-bond': (0.45, 0.23, 0.16, 0.85, 0.0),
    'painted-stucco': (0.78, 0.74, 0.66, 0.8, 0.0),
    'float-finish-stucco': (0.8, 0.77, 0.7, 0.85, 0.0),
    'cast-stone': (0.66, 0.62, 0.55, 0.8, 0.0),
    'cast-stone-trim': (0.74, 0.7, 0.62, 0.75, 0.0),
    'limestone-ashlar': (0.7, 0.66, 0.58, 0.8, 0.0),
    'concrete-precast': (0.6, 0.6, 0.58, 0.85, 0.0),
    'painted-white': (0.86, 0.86, 0.83, 0.5, 0.0),
    'painted-steel': (0.08, 0.09, 0.1, 0.45, 0.6),
    'painted-steel-mid': (0.2, 0.21, 0.22, 0.5, 0.5),
    'painted-steel-light': (0.5, 0.5, 0.5, 0.5, 0.5),
    'asphalt-roof': (0.12, 0.12, 0.12, 0.95, 0.0),
    'awning-canvas': (0.35, 0.08, 0.07, 0.9, 0.0),
    'glass-clear': (0.1, 0.13, 0.15, 0.05, 0.0),
    'glass-low-iron': (0.12, 0.15, 0.16, 0.04, 0.0),
    'glass-door': (0.1, 0.12, 0.13, 0.05, 0.0),
    'glass-reflective': (0.12, 0.15, 0.18, 0.04, 0.8),
    'glass-storefront': (0.1, 0.12, 0.14, 0.05, 0.8),
    'glass-sky': (0.55, 0.62, 0.68, 0.08, 0.1),
    'glass-louvre': (0.6, 0.7, 0.78, 0.22, 0.05),
    'glass-shutter': (0.62, 0.64, 0.66, 0.38, 0.35),
    'signage': (0.1, 0.1, 0.1, 0.6, 0.0),
    'wood-siding': (0.55, 0.45, 0.33, 0.7, 0.0),
    'painted-clapboard': (0.91, 0.90, 0.87, 0.72, 0.0),
    'terracotta': (0.62, 0.35, 0.22, 0.75, 0.0),
}
INTERIOR = {
    'int-plaster': (0.86, 0.81, 0.72, 0.9, 0.0),
    'int-ceiling': (0.93, 0.91, 0.87, 0.95, 0.0),
    'int-wood-floor': (0.42, 0.28, 0.17, 0.55, 0.0),
    'int-concrete-floor': (0.5, 0.49, 0.47, 0.8, 0.0),
    'int-rug-border': (0.2, 0.14, 0.1, 1.0, 0.0),
    'int-curtain': (0.82, 0.74, 0.6, 1.0, 0.0),
    'int-duvet': (0.9, 0.88, 0.83, 1.0, 0.0),
    'int-blanket': (0.46, 0.2, 0.14, 1.0, 0.0),
    'int-baseboard': (0.93, 0.91, 0.86, 0.5, 0.0),
    'int-tile-floor': (0.36, 0.3, 0.25, 0.7, 0.0),
    'int-wood': (0.35, 0.22, 0.12, 0.5, 0.0),
    'int-wood-light': (0.62, 0.48, 0.32, 0.55, 0.0),
    'int-steel': (0.6, 0.61, 0.62, 0.3, 1.0),
    'int-dark-metal': (0.07, 0.07, 0.08, 0.4, 0.8),
    'int-fabric-bed': (0.78, 0.76, 0.72, 0.95, 0.0),
    'int-fabric-blue': (0.16, 0.22, 0.33, 0.95, 0.0),
    'int-fabric-red': (0.45, 0.12, 0.1, 0.95, 0.0),
    'int-fabric-green': (0.2, 0.3, 0.22, 0.95, 0.0),
    'int-leather': (0.25, 0.14, 0.08, 0.6, 0.0),
    'int-rug': (0.4, 0.25, 0.2, 1.0, 0.0),
    'int-ceramic': (0.92, 0.92, 0.9, 0.2, 0.0),
    'int-stair': (0.3, 0.2, 0.12, 0.6, 0.0),
    'int-door': (0.5, 0.36, 0.22, 0.6, 0.0),
    'int-light': (1.0, 0.8, 0.55, 0.5, 0.0),
    'int-screen': (0.02, 0.02, 0.03, 0.2, 0.0),
    'int-plant': (0.12, 0.3, 0.1, 0.9, 0.0),
    'int-bottle': (0.15, 0.3, 0.18, 0.1, 0.0),
}
EMISSIVE = {'int-light': 4.0}
# interior surface -> (CC0 texture set, metres per tile)
TEXTURED = {
    'int-wood-floor': ('herringbone_parquet', 1.4),
    'int-tile-floor': ('herringbone_parquet', 1.4),
    'int-concrete-floor': ('brushed_concrete', 2.5),
    'int-plaster': ('beige_wall_001', 2.2),
    'int-rug': ('poly_wool_herringbone', 0.7),
    'int-curtain': ('rough_linen', 0.6),
    'int-duvet': ('rough_linen', 0.8),
}

# ---------------------------------------------------------------- helpers


def B(x, y, z):
    """Frame (x right, y up, z out of the front) -> Blender (Z up)."""
    return Vector((x, -z, y))


def v2(p):
    return (p['x'], p['z'])


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1])


def add(a, b):
    return (a[0] + b[0], a[1] + b[1])


def mul(a, s):
    return (a[0] * s, a[1] * s)


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1]


def length(a):
    return math.hypot(a[0], a[1])


def norm(a):
    l = length(a) or 1e-9
    return (a[0] / l, a[1] / l)


def lerp2(a, b, t):
    return (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)


def line_intersect(p, d, q, e):
    den = d[0] * e[1] - d[1] * e[0]
    if abs(den) < 1e-9:
        return p
    t = ((q[0] - p[0]) * e[1] - (q[1] - p[1]) * e[0]) / den
    return (p[0] + d[0] * t, p[1] + d[1] * t)


def _clip_segment(a, b, r):
    """Liang-Barsky: the bbox (x0, x1, z0, z1) of segment a-b inside rect r, or None."""
    x0, x1, z0, z1 = r
    t0, t1 = 0.0, 1.0
    dx, dz = b[0] - a[0], b[1] - a[1]
    for p, q in ((-dx, a[0] - x0), (dx, x1 - a[0]), (-dz, a[1] - z0), (dz, z1 - a[1])):
        if abs(p) < 1e-12:
            if q < 0:
                return None
            continue
        t = q / p
        if p < 0:
            t0 = max(t0, t)
        else:
            t1 = min(t1, t)
        if t0 >= t1:
            return None
    xa, za = a[0] + dx * t0, a[1] + dz * t0
    xb, zb = a[0] + dx * t1, a[1] + dz * t1
    return (min(xa, xb), max(xa, xb), min(za, zb), max(za, zb))


class Materials:
    def __init__(self):
        self.cache = {}

    def get(self, name):
        if name in self.cache:
            return self.cache[name]
        mat = bpy.data.materials.new(name)
        mat.use_nodes = True
        bsdf = mat.node_tree.nodes.get('Principled BSDF')
        # `<kit name>@rrggbb` is a kit material with a tint multiplier; the
        # runtime parses the same suffix (blender/runtime.js).
        base, _, tint = name.partition('@')
        r, g, b, rough, metal = KIT_PREVIEW.get(base) or INTERIOR.get(base) or (0.6, 0.6, 0.6, 0.8, 0.0)
        if tint:
            tr, tg, tb = (int(tint[i:i + 2], 16) / 255 for i in (0, 2, 4))
            r, g, b = r * tr, g * tg, b * tb
        bsdf.inputs['Base Color'].default_value = (r, g, b, 1)
        bsdf.inputs['Roughness'].default_value = rough
        bsdf.inputs['Metallic'].default_value = metal
        if name.startswith('glass'):
            bsdf.inputs['Alpha'].default_value = 0.35
            try:
                mat.surface_render_method = 'BLENDED'
            except (AttributeError, TypeError):
                pass
        if name in EMISSIVE:
            bsdf.inputs['Emission Color'].default_value = (r, g, b, 1)
            bsdf.inputs['Emission Strength'].default_value = EMISSIVE[name]
        if name in TEXTURED and LIBRARY['textures']:
            self._texture(mat, bsdf, TEXTURED[name][0])
        mat.diffuse_color = (r, g, b, 1)
        self.cache[name] = mat
        return mat


    @staticmethod
    def _texture(mat, bsdf, set_name):
        folder = os.path.join(LIBRARY['textures'], set_name)
        if not os.path.isdir(folder):
            return
        files = {f.split('_')[-2] if f.count('_') > 1 else f: f for f in os.listdir(folder) if f.endswith('.jpg')}

        def find(tag):
            return next((os.path.join(folder, f) for f in os.listdir(folder) if f'_{tag}_' in f and f.endswith('.jpg')), None)

        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        diff, arm, nor = find('diff'), find('arm'), find('nor_gl')
        if diff:
            tex = nodes.new('ShaderNodeTexImage')
            tex.image = bpy.data.images.load(diff, check_existing=True)
            links.new(tex.outputs['Color'], bsdf.inputs['Base Color'])
        if arm:
            tex = nodes.new('ShaderNodeTexImage')
            tex.image = bpy.data.images.load(arm, check_existing=True)
            tex.image.colorspace_settings.name = 'Non-Color'
            sep = nodes.new('ShaderNodeSeparateColor')
            links.new(tex.outputs['Color'], sep.inputs['Color'])
            links.new(sep.outputs[1], bsdf.inputs['Roughness'])
            links.new(sep.outputs[2], bsdf.inputs['Metallic'])
        if nor:
            tex = nodes.new('ShaderNodeTexImage')
            tex.image = bpy.data.images.load(nor, check_existing=True)
            tex.image.colorspace_settings.name = 'Non-Color'
            nm = nodes.new('ShaderNodeNormalMap')
            nm.inputs['Strength'].default_value = 0.8
            links.new(tex.outputs['Color'], nm.inputs['Color'])
            links.new(nm.outputs['Normal'], bsdf.inputs['Normal'])


MATS = Materials()


class MeshBuilder:
    """Collects quads per material; winding is fixed per face against an outward hint."""

    def __init__(self, name):
        self.name = name
        self.verts = []
        self.faces = []
        self.face_mats = []
        self.mat_index = {}

    def _mi(self, mat):
        if mat not in self.mat_index:
            self.mat_index[mat] = len(self.mat_index)
        return self.mat_index[mat]

    def quad(self, pts, mat, outward=None):
        """pts: four Blender-space Vectors. outward: Blender-space Vector hint."""
        if outward is not None:
            n = (pts[1] - pts[0]).cross(pts[2] - pts[0])
            if n.dot(outward) < 0:
                pts = list(reversed(pts))
        base = len(self.verts)
        self.verts.extend(pts)
        self.faces.append(tuple(range(base, base + len(pts))))
        self.face_mats.append(self._mi(mat))

    def poly(self, pts, mat, outward):
        self.quad(pts, mat, outward)

    def box(self, center, size, mat, yaw=0.0, mats=None, skip=()):
        """Axis box in frame space. center (x, y, z), size (sx, sy, sz), yaw about frame +y.
        mats: optional per-face dict {'top','bottom','side'} overriding mat."""
        cx, cy, cz = center
        hx, hy, hz = size[0] / 2, size[1] / 2, size[2] / 2
        c, s = math.cos(yaw), math.sin(yaw)

        def P(x, y, z):
            # rotate (x, z) about y by yaw, same convention as physics.js rotateXZ
            rx = x * c + z * s
            rz = -x * s + z * c
            return B(cx + rx, cy + y, cz + rz)

        def dirv(x, y, z):
            return B(x * c + z * s, y, -x * s + z * c)

        m = mats or {}
        faces = {
            'top': ([P(-hx, hy, -hz), P(hx, hy, -hz), P(hx, hy, hz), P(-hx, hy, hz)], dirv(0, 1, 0)),
            'bottom': ([P(-hx, -hy, -hz), P(hx, -hy, -hz), P(hx, -hy, hz), P(-hx, -hy, hz)], dirv(0, -1, 0)),
            '+x': ([P(hx, -hy, -hz), P(hx, hy, -hz), P(hx, hy, hz), P(hx, -hy, hz)], dirv(1, 0, 0)),
            '-x': ([P(-hx, -hy, -hz), P(-hx, hy, -hz), P(-hx, hy, hz), P(-hx, -hy, hz)], dirv(-1, 0, 0)),
            '+z': ([P(-hx, -hy, hz), P(hx, -hy, hz), P(hx, hy, hz), P(-hx, hy, hz)], dirv(0, 0, 1)),
            '-z': ([P(-hx, -hy, -hz), P(hx, -hy, -hz), P(hx, hy, -hz), P(-hx, hy, -hz)], dirv(0, 0, -1)),
        }
        for key, (pts, out) in faces.items():
            if key in skip:
                continue
            fm = m.get(key) or (m.get('side') if key not in ('top', 'bottom') else None) or mat
            self.quad(pts, fm, out)

    def empty(self):
        return not self.faces

    @staticmethod
    def _uvs(mesh):
        # box-planar projection in metres, scaled by the material's tile size;
        # only textured interior surfaces read them (the exterior exports none)
        uv = mesh.uv_layers.new(name='UVMap')
        names = [m.name if m else '' for m in mesh.materials]
        for poly in mesh.polygons:
            n = poly.normal
            tile = TEXTURED.get(names[poly.material_index] if poly.material_index < len(names) else '', (None, 1.0))[1]
            ax = max(range(3), key=lambda i: abs(n[i]))
            for li in poly.loop_indices:
                co = mesh.vertices[mesh.loops[li].vertex_index].co
                u, v = ((co.y, co.z) if ax == 0 else (co.x, co.z) if ax == 1 else (co.x, co.y))
                uv.data[li].uv = (u / tile, v / tile)

    def build(self, collection, parent=None):
        if self.empty():
            return None
        mesh = bpy.data.meshes.new(self.name)
        mesh.from_pydata([tuple(v) for v in self.verts], [], self.faces)
        mats_by_index = sorted(self.mat_index.items(), key=lambda kv: kv[1])
        for name, _ in mats_by_index:
            mesh.materials.append(MATS.get(name))
        for poly, mi in zip(mesh.polygons, self.face_mats):
            poly.material_index = mi
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-4)
        bm.to_mesh(mesh)
        bm.free()
        self._uvs(mesh)
        mesh.update()
        obj = bpy.data.objects.new(self.name, mesh)
        collection.objects.link(obj)
        if parent is not None:
            obj.parent = parent
        return obj


# ---------------------------------------------------------------- wall grid


def wall_grid(mb, a_out, b_out, a_in, b_in, base, top, openings, mat_out, mat_in, mat_reveal,
              mat_top=None, cap_top=True, cap_ends=False, splits=(), xsplits=(), zone_mat=None):
    """A straight wall from a->b between two faces, with rectangular openings.

    a_out/b_out lie on the outer face, a_in/b_in on the inner face (mitred at
    corners). openings: list of (s0, s1, y0, y1) in metres along a_out->b_out
    and absolute heights. Emits only visible faces: outer and inner skins per
    solid cell, and reveal faces where a solid cell meets an opening.
    """
    L = length(sub(b_out, a_out))
    if L < 1e-3:
        return
    d = norm(sub(b_out, a_out))
    n_out = norm(sub(a_out, a_in)) if length(sub(a_out, a_in)) > 1e-6 else (-d[1], d[0])
    # outward normal perpendicular to the wall
    perp = (-d[1], d[0])
    if dot(perp, n_out) < 0:
        perp = (d[1], -d[0])
    thickness = abs(dot(sub(a_out, a_in), perp))
    xs = {0.0, L}
    ys = {base, top}
    clean = []
    for s0, s1, y0, y1 in openings:
        s0, s1 = max(0.0, s0), min(L, s1)
        y0, y1 = max(base, y0), min(top, y1)
        if s1 - s0 > 1e-3 and y1 - y0 > 1e-3:
            clean.append((s0, s1, y0, y1))
            xs.update((s0, s1))
            ys.update((y0, y1))
    ys.update(y for y in splits if base < y < top)
    xs.update(x for x in xsplits if 0 < x < L)
    xs = sorted(xs)
    ys = sorted(ys)
    out_mat = mat_out if callable(mat_out) else (lambda y0, y1: mat_out)
    rev_mat = mat_reveal if callable(mat_reveal) else (lambda y0, y1: mat_reveal)

    def is_open(i, j):
        if i < 0 or j < 0 or i >= len(xs) - 1 or j >= len(ys) - 1:
            return None  # outside
        cx = (xs[i] + xs[i + 1]) / 2
        cy = (ys[j] + ys[j + 1]) / 2
        return any(s0 <= cx <= s1 and y0 <= cy <= y1 for s0, s1, y0, y1 in clean)

    def outer(s):
        return add(a_out, mul(d, s))

    def inner(s):
        if s <= 1e-6:
            return a_in
        if s >= L - 1e-6:
            return b_in
        return sub(outer(s), mul(perp, thickness))

    out_v = B(perp[0], 0, perp[1])
    in_v = B(-perp[0], 0, -perp[1])
    along = B(d[0], 0, d[1])
    for i in range(len(xs) - 1):
        for j in range(len(ys) - 1):
            if is_open(i, j):
                continue
            s0, s1, y0, y1 = xs[i], xs[i + 1], ys[j], ys[j + 1]
            o0, o1, i0, i1 = outer(s0), outer(s1), inner(s0), inner(s1)
            face_mat = (zone_mat((s0 + s1) / 2, y0, y1) if zone_mat else None) or out_mat(y0, y1)
            mb.quad([B(o0[0], y0, o0[1]), B(o1[0], y0, o1[1]), B(o1[0], y1, o1[1]), B(o0[0], y1, o0[1])], face_mat, out_v)
            mb.quad([B(i0[0], y0, i0[1]), B(i1[0], y0, i1[1]), B(i1[0], y1, i1[1]), B(i0[0], y1, i0[1])], mat_in, in_v)
            # reveals toward openings (and caps at the top / ends when asked)
            left, right, below, above = is_open(i - 1, j), is_open(i + 1, j), is_open(i, j - 1), is_open(i, j + 1)
            if left or (left is None and cap_ends):
                mb.quad([B(o0[0], y0, o0[1]), B(i0[0], y0, i0[1]), B(i0[0], y1, i0[1]), B(o0[0], y1, o0[1])], rev_mat(y0, y1), -along)
            if right or (right is None and cap_ends):
                mb.quad([B(o1[0], y0, o1[1]), B(i1[0], y0, i1[1]), B(i1[0], y1, i1[1]), B(o1[0], y1, o1[1])], rev_mat(y0, y1), along)
            if below:
                mb.quad([B(o0[0], y0, o0[1]), B(o1[0], y0, o1[1]), B(i1[0], y0, i1[1]), B(i0[0], y0, i0[1])], rev_mat(y0, y1), B(0, -1, 0))
            if above or (above is None and cap_top):
                mb.quad([B(o0[0], y1, o0[1]), B(o1[0], y1, o1[1]), B(i1[0], y1, i1[1]), B(i0[0], y1, i0[1])],
                        (mat_top or rev_mat(y0, y1)) if above is None else rev_mat(y0, y1), B(0, 1, 0))


def partition(mb, a, b, base, top, doors, mat='int-plaster', t=PART_T):
    """Interior partition a->b (frame x/z tuples), centred, doors: list of s-centres along it."""
    L = length(sub(b, a))
    if L < 0.2:
        return []
    d = norm(sub(b, a))
    perp = (-d[1], d[0])
    ao, bo = add(a, mul(perp, t / 2)), add(b, mul(perp, t / 2))
    ai, bi = sub(a, mul(perp, t / 2)), sub(b, mul(perp, t / 2))
    openings = [(s - DOOR_W / 2, s + DOOR_W / 2, base, min(top - 0.05, base + DOOR_H)) for s in doors]
    wall_grid(mb, ao, bo, ai, bi, base, top, openings, mat, mat, 'int-wood-light', cap_top=True, cap_ends=True)
    # physics pieces: solid spans between door gaps
    pieces = []
    cuts = sorted([(max(0, s - DOOR_W / 2), min(L, s + DOOR_W / 2)) for s in doors])
    cursor = 0.0
    for c0, c1 in cuts + [(L, L)]:
        if c0 - cursor > 0.05:
            pieces.append((cursor, c0))
        cursor = max(cursor, c1)
    solids = []
    yaw = math.atan2(d[0], d[1]) - math.pi / 2  # box local +x along the wall
    for s0, s1 in pieces:
        mid = lerp2(a, b, (s0 + s1) / 2 / L)
        solids.append({'center': {'x': mid[0], 'y': (base + top) / 2, 'z': mid[1]},
                       'size': {'x': s1 - s0, 'y': top - base, 'z': t}, 'yaw': box_yaw(d)})
    return solids


def box_yaw(direction):
    """Yaw (physics.js convention) that turns local +x onto `direction` (frame x/z)."""
    # rotateXZ(1, 0, yaw) = (cos yaw, -sin yaw)
    return math.atan2(-direction[1], direction[0])


# ---------------------------------------------------------------- building


class Building:
    def __init__(self, spec, style):
        self.spec = spec
        self.style = style
        self.rng = random.Random(style.get('seed', 1))
        self.poly = [v2(p) for p in spec['polygon']]
        n = len(self.poly)
        cx = sum(p[0] for p in self.poly) / n
        cz = sum(p[1] for p in self.poly) / n
        self.centroid = (cx, cz)
        # `edgeRoles` in the style corrects a mis-detected role (a party wall
        # that is really an alley); the spec stays what the data says.
        roles = style.get('edgeRoles') or {}
        self.edges = [dict(e, role=roles[str(e['index'])]) if str(e['index']) in roles else e for e in spec['edges']]
        self.height = spec['heightM']
        self.storeys = spec['storeys']
        f = spec['floors']
        # The roll's storey count and the LiDAR height can disagree (a mezzanine
        # counted as a storey). The drawn roof may not rise above the LiDAR
        # height, and a storey needs MIN_STOREY_M; when the stack does not fit,
        # the height wins and the count drops.
        stack_top = f['groundM'] + (self.storeys - 1) * f['upperM'] if self.storeys > 1 else f['groundM']
        if stack_top > self.height - 0.05:
            self.storeys = max(1, min(self.storeys, int((self.height - 0.3) // MIN_STOREY_M)))
            usable = self.height - 0.3
            ground = min(4.2, usable * (0.55 if self.storeys == 2 else 1.0 / self.storeys + 0.1)) if self.storeys > 1 else usable
            upper = (usable - ground) / (self.storeys - 1) if self.storeys > 1 else f['upperM']
            f = {'groundM': round(ground, 3), 'upperM': round(upper, 3), 'roofM': 0.3}
            self.spec = dict(spec, floors=f, storeys=self.storeys)
            spec = self.spec
        self.levels = [0.0] + [f['groundM'] + k * f['upperM'] for k in range(self.storeys - 1)]
        self.roof_level = f['groundM'] + (self.storeys - 1) * f['upperM'] if self.storeys > 1 else f['groundM']
        self.inner = self._inset(WALL_T)
        xs = [p[0] for p in self.inner]
        zs = [p[1] for p in self.inner]
        self.ix0, self.ix1, self.iz0, self.iz1 = min(xs), max(xs), min(zs), max(zs)
        # conservative interior rectangle (inside the inset polygon)
        self.rect = self._inner_rect()
        # a concrete frame with recessed balconies on the front: rooms and
        # furniture stop behind the recess line (+z is out of the front), or
        # they stand in the balconies and read through the guard rails
        grid = style.get('frameGrid')
        if grid and grid.get('recessM', 0) > WALL_T:
            x0, x1, z0, z1 = self.rect
            self.rect = (x0, x1, z0, max(z0 + 4.0, z1 - (grid['recessM'] - WALL_T) - 0.1))
        self.physics = {'walls': [], 'solids': [], 'steps': [], 'floor': {'top': 0.0}}
        inset = self._floor_inset()
        if inset:
            self.physics['floor']['inset'] = inset
        self.window_slots = []
        self.lights = []
        self.coarse = {}
        self.soft = {}
        self.stats = {}
        self.exterior = bpy.data.collections.new('exterior')
        self.interior = bpy.data.collections.new('interior')
        bpy.context.scene.collection.children.link(self.exterior)
        bpy.context.scene.collection.children.link(self.interior)

    # -- geometry helpers

    def _floor_inset(self):
        """The physics floor is the frame rectangle (physics.js compileFloor).
        On an irregular footprint on a rotated frame its corners can leave the
        footprint's world AABB (+0.45 m); return the smallest inset (0.25 m
        steps) that keeps it inside, or 0 when none is needed."""
        fr = self.spec['frame']
        c, s = math.cos(fr['heading']), math.sin(fr['heading'])

        def rot(x, z):
            return (x * c + z * s, -x * s + z * c)
        pts = [rot(*p) for p in self.poly]
        x0, x1 = min(p[0] for p in pts) - 0.45, max(p[0] for p in pts) + 0.45
        z0, z1 = min(p[1] for p in pts) - 0.45, max(p[1] for p in pts) + 0.45
        for k in range(0, 80):
            inset = 0.25 * k
            hx, hz = fr['width'] / 2 - inset, fr['depth'] / 2 - inset
            if hx < 1 or hz < 1:
                return 0.0
            corners = [rot(sx * hx, sz * hz) for sx in (-1, 1) for sz in (-1, 1)]
            if all(x0 <= x <= x1 and z0 <= z <= z1 for x, z in corners):
                return inset
        return 0.0

    def inward(self, a, b):
        """Inward normal of the footprint edge a -> b (in ring order). From the
        ring's winding, not a centroid test: in a concave footprint (a lightwell
        notch, a jogged front) the centroid can sit outside an edge's interior side."""
        d = norm(sub(b, a))
        if not hasattr(self, '_ring_sign'):
            area = sum(p[0] * q[1] - q[0] * p[1] for p, q in zip(self.poly, self.poly[1:] + self.poly[:1]))
            self._ring_sign = 1 if area > 0 else -1
        return (-d[1] * self._ring_sign, d[0] * self._ring_sign)

    def _inset(self, t):
        n = len(self.poly)
        lines = []
        for i in range(n):
            a, b = self.poly[i], self.poly[(i + 1) % n]
            nin = self.inward(a, b)
            lines.append((add(a, mul(nin, t)), norm(sub(b, a))))
        out = []
        for i in range(n):
            p0, d0 = lines[i - 1]
            p1, d1 = lines[i]
            out.append(line_intersect(p0, d0, p1, d1))
        return out

    def _inner_rect(self):
        # Shrink the inset polygon AABB until its corners are inside the polygon.
        x0, x1, z0, z1 = self.ix0, self.ix1, self.iz0, self.iz1
        for _ in range(40):
            corners = [(x0, z0), (x1, z0), (x1, z1), (x0, z1)]
            if all(self._inside(c) for c in corners):
                break
            x0 += 0.05
            x1 -= 0.05
            z0 += 0.05
            z1 -= 0.05
        # Corners inside is not enough on a notched footprint (an L, a lightwell,
        # a jogged side): an inset edge can cross the rectangle between them and
        # rooms, stairs and furniture land outside the building. Then take the
        # largest rectangle on the inset polygon's own coordinates that no edge
        # enters (a convex footprint keeps the rectangle above unchanged).
        if not self._rect_contained((x0, x1, z0, z1)):
            xs = sorted({round(p[0], 3) for p in self.inner} | {x0, x1})
            zs = sorted({round(p[1], 3) for p in self.inner} | {z0, z1})
            spans_x = [(a, b) for i, a in enumerate(xs) for b in xs[i + 1:] if b - a >= 2.0]
            spans_z = [(a, b) for i, a in enumerate(zs) for b in zs[i + 1:] if b - a >= 2.0]
            cands = sorted(((sx[1] - sx[0]) * (sz[1] - sz[0]), sx + sz) for sx in spans_x for sz in spans_z)
            for _, r in reversed(cands):
                if self._rect_contained(r):
                    return r
        return (x0, x1, z0, z1)

    def _rect_contained(self, r):
        x0, x1, z0, z1 = r
        e = 0.02
        if not all(self._inside(c) for c in ((x0 + e, z0 + e), (x1 - e, z0 + e), (x1 - e, z1 - e), (x0 + e, z1 - e))):
            return False
        q = (x0 + e / 2, x1 - e / 2, z0 + e / 2, z1 - e / 2)
        return not any(_clip_segment(self.inner[i - 1], self.inner[i], q) for i in range(len(self.inner)))

    def _inside(self, p, pts=None):
        inside = False
        pts = pts or self.inner
        j = len(pts) - 1
        for i in range(len(pts)):
            xi, zi = pts[i]
            xj, zj = pts[j]
            if (zi > p[1]) != (zj > p[1]) and p[0] < (xj - xi) * (p[1] - zi) / (zj - zi + 1e-12) + xi:
                inside = not inside
            j = i
        return inside

    def storey_top(self, k):
        return self.levels[k + 1] - SLAB_T if k + 1 < self.storeys else self.roof_level - SLAB_T

    def storey_height(self, k):
        top = self.levels[k + 1] if k + 1 < self.storeys else self.roof_level
        return top - self.levels[k]

    # -- programme

    def ground_programme(self):
        g = self.style.get('ground', 'auto')
        if g != 'auto':
            return g
        s = self.spec
        if s.get('amenity') in ('restaurant', 'cafe', 'bar', 'fast_food', 'pub'):
            return 'restaurant'
        if s.get('shop') or s.get('facade') == 'shopfront' or s.get('usage') == 'commercial':
            return 'shop'
        return 'lobby'

    def upper_programme(self):
        u = self.style.get('upper', 'auto')
        if u != 'auto':
            return u
        use = (self.spec.get('use') or '').upper()
        if use in ('COMH',) or use.startswith('RES') or use in ('MRES', 'SRES'):
            return 'rooms'
        if use.startswith('COMO') or use.startswith('COM'):
            return 'office'
        return 'apartment'

    # -- stair core (left rear, against the first party wall if any)

    def plan_stair(self):
        x0, x1, z0, z1 = self.rect
        party = [e for e in self.edges if e['role'] == 'party']
        # choose the side: the party edge nearest x0 or x1
        side = -1
        if party:
            e = party[0]
            mx = (e['a']['x'] + e['b']['x']) / 2
            side = -1 if mx < 0 else 1
        # an explicit street door for the stairs puts the core on that side
        layout = (self.style.get('storefront') or {}).get('layout') or []
        stair_item = next((item for item in layout if item.get('stair')), None)
        front = next((e for e in self.edges if e['role'] == 'front'), None)
        if stair_item and front:
            a, b = v2(front['a']), v2(front['b'])
            fd = norm(sub(b, a))
            s_mid = self.f2s((stair_item['from'] + stair_item['to']) / 2, front['lengthM'])
            side = -1 if (a[0] + fd[0] * s_mid) < 0 else 1
        width = 2 * FLIGHT_W + STAIR_GAP
        rise_max = max(self.storey_height(k) for k in range(self.storeys))
        steps = math.ceil(rise_max / RISER)
        per_flight = math.ceil(steps / 2)
        run = per_flight * TREAD
        depth = LANDING_D + run
        sx0 = x0 if side < 0 else x1 - width
        self.stair = {
            'side': side,
            'x0': sx0,
            'x1': sx0 + width,
            'z0': z0,
            'z1': z0 + depth,
            'run': run,
            'perFlight': per_flight,
        }
        return self.stair

    # ---------------------------------------------------------- exterior

    def openings_for_edge(self, e):
        """Openings in metres along a->b for one edge, all storeys."""
        s = self.edge_style(e)
        L = e['lengthM']
        a, b = v2(e['a']), v2(e['b'])
        d = norm(sub(b, a))
        role = e['role']
        street = self.is_street(e)
        ops = []
        if role == 'party':
            return ops, [], []
        grid = s.get('frameGrid') if street else None
        # upper storeys: punched windows in regular bays, or at the positions
        # the style lists for the front (fractions along a->b, viewer's left
        # to right), plus a full-width opening behind every projecting bay
        bay = s['bayM']
        usable = L - 2 * s['endPierM']
        count = max(1, int(round(usable / bay))) if usable > s['windowW'] else 0
        centres = [s['endPierM'] + usable * (i + 0.5) / count for i in range(count)] if count else []
        bays = (s.get('bays') or []) if street else []
        if street and s.get('upperWindows'):
            centres = [self.f2s(f, L, e) for f in s['upperWindows']]
        spans = (s.get('upperWindowSpans') or []) if street else []
        if spans:
            centres = []
        if s.get('blankUpper'):  # a blank (e.g. mural) wall: no upper openings
            centres, spans, bays = [], [], []
        windows = []
        if grid:
            # a concrete frame replaces the punched windows on the storeys it covers
            fg_ops, fg_windows, first = self._frame_grid_openings(e, s, grid, L)
            ops += fg_ops
            windows += fg_windows
        for k in range(1, self.storeys):
            lvl = self.levels[k]
            if grid and k >= first:
                continue
            for f0, f1 in spans:
                sill = lvl + s.get('upperSillM', s['windowSillM'])
                head = lvl + min(s.get('upperHeadM', s['windowHeadM']), self.storey_height(k) - 0.2)
                s0, s1 = sorted((self.f2s(f0, L, e), self.f2s(f1, L, e)))
                o = (s0, s1, sill, head)
                ops.append(o)
                windows.append(('ribbon', o))
            for c in centres:
                o = (c - s['windowW'] / 2, c + s['windowW'] / 2, lvl + s['windowSillM'], lvl + min(s['windowHeadM'], self.storey_height(k) - 0.35))
                ops.append(o)
                windows.append(('punched', o))
            for bay_spec in bays:
                if k < bay_spec.get('fromStorey', 1) or k > bay_spec.get('toStorey', self.storeys):
                    continue
                c, w = self.f2s(bay_spec['at'], L, e), bay_spec['widthM']
                ops.append((c - w / 2 + 0.05, c + w / 2 - 0.05, lvl + 0.12, lvl + self.storey_height(k) - 0.3))
        doors = []
        g = self.spec['floors']['groundM']
        if street and s['storefront'].get('layout'):
            sf = s['storefront']
            head = g - sf.get('headDropM', 0.35)
            door_h = min(2.45, head - sf['transomM'] * 0.6)
            stair_door = None
            main = None
            front = role == 'front'
            for item in sf['layout']:
                s0, s1 = sorted((self.f2s(item['to'], L, e), self.f2s(item['from'], L, e)))
                kind = item['kind']
                item_head = min(head, item['headM']) if item.get('headM') else head
                ops.append((s0, s1, 0.0, item_head))
                if item.get('grille'):
                    windows.append(('grille', (s0, s1, 0.0, item_head if kind == 'window' else door_h)))
                if kind == 'window':
                    windows.append(('storefront', (s0, s1, 0.0, item_head)))
                    continue
                if kind in ('garage', 'glazed-garage', 'blank', 'gate'):
                    # closed infills: a roll-up shutter, a glazed sectional
                    # door, a boarded/painted panel, a slatted steel gate
                    windows.append((kind, (s0, s1, 0.0, item_head, item.get('material'), item.get('tint'), item.get('ribs'), item.get('recessM'), item.get('doorM'))))
                    continue
                span = (s0 + 0.04, s1 - 0.04, 0.0, door_h)
                if not front:
                    doors.append({'id': f'street-{e["index"]}-{len(doors)}', 'span': span})
                    windows.append(('storefront-door', (s0, s1, 0.0, item_head, span[0], span[1], item.get('leaf', 'wood-siding'), kind)))
                    continue
                door_id = 'stair-door' if item.get('stair') else f'door-{len(doors)}'
                doors.append({'id': door_id, 'span': span})
                windows.append(('storefront-door', (s0, s1, 0.0, head, span[0], span[1], item.get('leaf', 'wood-siding'), kind)))
                if item.get('stair'):
                    stair_door = span
                    self.stair_door_x = a[0] + d[0] * (s0 + s1) / 2
                elif main is None or kind == 'double-door':
                    main = (s0, s1)
                    self.main_door_x = a[0] + d[0] * (s0 + s1) / 2
            if front:
                self.front_edge = {'a': a, 'b': b, 'd': d, 'L': L, 'head': head, 'stairDoor': stair_door, 'mainBay': main}
        elif street and role != 'front':
            pass  # a street edge with no ground layout: upper windows only
        elif role == 'front':
            sf = s['storefront']
            head = g - sf['transomM'] * 0.2 - 0.35
            # stair door: nearest the stair side
            stair = self.stair
            stair_x = (stair['x0'] + stair['x1']) / 2
            # position along the edge for a frame x
            def s_of_x(x):
                # edge is roughly parallel to frame x
                return (x - a[0]) / d[0] if abs(d[0]) > 1e-6 else L / 2
            sd = s_of_x(self.rect[0] + 0.75) if stair['side'] < 0 else s_of_x(self.rect[1] - 0.75)
            sd = max(0.7, min(L - 0.7, sd))
            stair_door = (sd - 0.5, sd + 0.5, 0.0, 2.45)
            ops.append(stair_door)
            doors.append({'id': 'stair-door', 'span': stair_door})
            windows.append(('stair-door', stair_door))
            # storefront bays across the rest
            lo, hi = (min(sd - 0.5, sd + 0.5), max(sd - 0.5, sd + 0.5))
            free = [(0.35, lo - sf['pierM']), (hi + sf['pierM'], L - 0.35)]
            main_door_s = None
            want = self.spec.get('doorAlong', 0.5) * L
            bays = []
            for f0, f1 in free:
                w = f1 - f0
                if w < 1.2:
                    continue
                n = max(1, int(round(w / 3.0)))
                span = w / n
                for i in range(n):
                    b0 = f0 + i * span + (sf['pierM'] / 2 if i else 0)
                    b1 = f0 + (i + 1) * span - (sf['pierM'] / 2 if i < n - 1 else 0)
                    ops.append((b0, b1, 0.0, head))
                    bays.append((b0, b1))
            # the entrance goes in the bay nearest doorAlong
            door_bay = min(bays, key=lambda bb: abs((bb[0] + bb[1]) / 2 - want)) if bays else None
            for b0, b1 in bays:
                if (b0, b1) == door_bay and b1 - b0 > 1.6:
                    c = max(b0 + 0.6, min(b1 - 0.6, want))
                    span = (c - 0.55, c + 0.55, 0.0, min(2.4, head))
                    doors.append({'id': 'main-door', 'span': span})
                    self.main_door_x = a[0] + d[0] * c
                    windows.append(('storefront-door', (b0, b1, 0.0, head, span[0], span[1])))
                    main_door_s = (b0, b1)
                else:
                    windows.append(('storefront', (b0, b1, 0.0, head)))
            self.front_edge = {'a': a, 'b': b, 'd': d, 'L': L, 'head': head, 'stairDoor': stair_door, 'mainBay': main_door_s}
        elif role in ('rear', 'flank'):
            # ground: small high windows and a service door at the middle
            mid = L / 2
            door = (mid - 0.5, mid + 0.5, 0.0, 2.2)
            ops.append(door)
            doors.append({'id': f'service-{e["index"]}', 'span': door})
            windows.append(('service-door', door))
            for c in centres:
                if abs(c - mid) < 1.4:
                    continue
                o = (c - 0.5, c + 0.5, 2.0, 3.0)
                ops.append(o)
                windows.append(('high', o))
        cut = self._rotunda_cut(e)
        if cut:
            # the corner rotunda takes this end of the wall up to its head;
            # openings and doors inside that zone give way to it
            c0, c1, top = cut

            def clear(span):
                return span[1] <= c0 + 0.05 or span[0] >= c1 - 0.05 or span[2] >= top
            ops = [o for o in ops if clear(o)] + [(c0, c1, 0.0, top)]
            windows = [w for w in windows if clear(w[1])]
            doors = [dd for dd in doors if clear(dd['span'])]
        return ops, windows, doors

    # -- corner rotunda: `rotunda: {vertex, radiusM, storeys, mullionM, glass, frame}`

    def _rotunda_cut(self, e):
        spec = self.style.get('rotunda')
        if not spec:
            return None
        n = len(self.poly)
        v = int(spec['vertex']) % n
        r = spec.get('radiusM', 3.0)
        top = self.levels[min(self.storeys - 1, int(spec.get('storeys', 1)))] - 0.3 if self.storeys > 1 else self.height - 0.5
        L = e['lengthM']
        if e['index'] == (v - 1) % n:
            return (L - r, L, top)
        if e['index'] == v:
            return (0.0, r, top)
        return None

    def _rotunda(self):
        spec = self.style.get('rotunda')
        if not spec:
            return
        n = len(self.poly)
        v = int(spec['vertex']) % n
        r = spec.get('radiusM', 3.0)
        V = self.poly[v]
        u_in = norm(sub(self.poly[(v - 1) % n], V))
        u_out = norm(sub(self.poly[(v + 1) % n], V))
        O = add(V, add(mul(u_in, r), mul(u_out, r)))
        A, C = add(V, mul(u_in, r)), add(V, mul(u_out, r))
        a0 = math.atan2(A[1] - O[1], A[0] - O[0])
        a1 = math.atan2(C[1] - O[1], C[0] - O[0])
        # sweep the short way round, through the side that faces the corner
        da = (a1 - a0 + math.pi) % (2 * math.pi) - math.pi
        top = self._rotunda_cut({'index': v, 'lengthM': 1.0})[2]
        segs = max(6, int(spec.get('segments', 12)))
        pts = [add(O, (r * math.cos(a0 + da * i / segs), r * math.sin(a0 + da * i / segs))) for i in range(segs + 1)]
        glass = MeshBuilder('rotunda-glazing')
        frames = MeshBuilder('rotunda-frames')
        gmat, fmat = spec.get('glass', self.style['storefrontGlass']), spec.get('frame', self.style['storefrontFrame'])
        levels = [0.0] + [y for y in self.levels[1:] if y < top - 0.2] + [top]
        # `transomM`: intermediate transoms at this pitch inside each storey,
        # so the curtain wall reads as a grid of small panes, not tall strips
        pitch = spec.get('transomM', 0)
        if pitch and pitch > 0.3:
            extra = []
            for y0, y1 in zip(levels[:-1], levels[1:]):
                k = max(1, int(round((y1 - y0) / pitch)))
                extra += [y0 + (y1 - y0) * j / k for j in range(1, k)]
            levels = sorted(levels + extra)
        for i in range(segs):
            p, q = pts[i], pts[i + 1]
            mid = lerp2(p, q, 0.5)
            out = B(mid[0] - O[0], 0, mid[1] - O[1])
            glass.quad([B(p[0], 0.0, p[1]), B(q[0], 0.0, q[1]), B(q[0], top, q[1]), B(p[0], top, p[1])], gmat, out)
            yaw = box_yaw(norm(sub(q, p)))
            for y in levels:   # transoms at every floor line, sill and head
                frames.box((mid[0], y, mid[1]), (length(sub(q, p)) + 0.02, 0.08, 0.1), fmat, yaw, skip=('+x', '-x'))
        for p in pts:          # mullions
            frames.box((p[0], top / 2, p[1]), (0.07, top, 0.1), fmat, 0.0, skip=('top', 'bottom'))
        # `columns`: free-standing round-ish columns on the old corner line,
        # carrying the overhanging floors above the rotunda
        cols = int(spec.get('columns', 0))
        if cols:
            cmb = MeshBuilder('rotunda-columns')
            cm = self.tinted(spec.get('columnMaterial', 'concrete-precast'), spec.get('columnTint'))
            cd = spec.get('columnD', 0.6)
            for i in range(cols):
                t = 0.35 + 0.3 * i / max(1, cols - 1)
                # between the curve and the old corner, clear of both street walls
                ang = a0 + da * t
                edge_pt = add(O, (r * math.cos(ang), r * math.sin(ang)))
                c = lerp2(edge_pt, V, 0.4)
                if not self._inside_outer(c, cd / 2 + 0.05):
                    continue
                cmb.box((c[0], top / 2 + 0.2, c[1]), (cd, top + 0.4, cd), cm, 0.0, skip=('bottom',))
                cmb.box((c[0], top / 2 + 0.2, c[1]), (cd, top + 0.4, cd), cm, math.pi / 4, skip=('bottom',))
            cmb.build(self.exterior)
        glass.build(self.exterior)
        frames.build(self.exterior)

    def _inside_outer(self, p, margin=0.0):
        inside = False
        pts = self.poly
        j = len(pts) - 1
        for i in range(len(pts)):
            xi, zi = pts[i]
            xj, zj = pts[j]
            if (zi > p[1]) != (zj > p[1]) and p[0] < (xj - xi) * (p[1] - zi) / (zj - zi + 1e-12) + xi:
                inside = not inside
            j = i
        if not inside:
            return False
        return all(seg_distance(p, self.poly[i], self.poly[(i + 1) % len(self.poly)]) >= margin for i in range(len(self.poly)))

    def build_exterior(self):
        s = self.style
        mb = MeshBuilder('shell')
        trim = MeshBuilder('trim')
        glass = MeshBuilder('glazing')
        frames = MeshBuilder('frames')
        store = MeshBuilder('storefront')
        n = len(self.poly)
        for i, e in enumerate(self.edges):
            a_out, b_out = self.poly[i], self.poly[(i + 1) % n]
            a_in, b_in = self.inner[i], self.inner[(i + 1) % n]
            ops, windows, doors = self.openings_for_edge(e)
            es = self.edge_style(e)
            wall_mat = self.wall_material_at if e['role'] != 'party' else self.wall_material
            if es is not self.style and (es.get('wall') != s['wall'] or es.get('wallTint') != s.get('wallTint')):
                own = self.tinted(es['wall'], es.get('wallTint'))
                wall_mat = lambda y0, y1, own=own: own if not (self.band_top and (y0 + y1) / 2 < self.band_top) else self.wall_material_at(y0, y1)
            # `returnWallTint` (style): the wall tint on non-street, non-party
            # edges (court returns, rears), which the sky alone lights, so a
            # cool street tint turns saturated blue there
            if s.get('returnWallTint') and e['role'] != 'party' and not self.is_street(e):
                own = self.tinted(s['wall'], s['returnWallTint'])
                wall_mat = lambda y0, y1, own=own: own
            # `wallZones`: vertical colour zones along a street face, e.g. a
            # two-tone corner shop ([{from, to, material?, tint?, fromY?, toY?}])
            zones, xsplits = [], []
            if self.is_street(e) and es.get('wallZones'):
                for z in es['wallZones']:
                    z0, z1 = sorted((self.f2s(z['from'], e['lengthM'], e), self.f2s(z['to'], e['lengthM'], e)))
                    zones.append((z0, z1, z.get('fromY', -1.0), z.get('toY', 1e9),
                                  self.tinted(z.get('material', es['wall']), z.get('tint'))))
                    xsplits += [z0, z1]

            def zone_mat(sm, y0, y1, zones=zones):
                ym = (y0 + y1) / 2
                for z0, z1, zy0, zy1, m in zones:
                    if z0 <= sm <= z1 and zy0 <= ym <= zy1:
                        return m
                return None
            wall_grid(mb, a_out, b_out, a_in, b_in, 0.0, self.height, ops,
                      wall_mat, 'int-plaster', wall_mat, mat_top=s['trim'],
                      splits=[self.band_top] if self.band_top else (),
                      xsplits=xsplits, zone_mat=zone_mat if zones else None)
            self._dress_openings(e, a_out, b_out, windows, trim, glass, frames, store, es)
            if self.is_street(e) and es.get('mural'):
                self._mural(e, a_out, b_out, es['mural'])
            # physics: the wall with its DOORS open (glass blocks)
            nin = self.inward(a_out, b_out)
            pa = add(a_out, mul(nin, WALL_T / 2))
            pb = add(b_out, mul(nin, WALL_T / 2))
            self.physics['walls'].append({
                'id': f'wall-{i}',
                'a': {'x': pa[0], 'z': pa[1]}, 'b': {'x': pb[0], 'z': pb[1]},
                'height': self.height, 'thickness': WALL_T, 'base': 0,
                'openings': [{'id': dd['id'], 't0': dd['span'][0] / e['lengthM'], 't1': dd['span'][1] / e['lengthM'],
                              'sill': 0, 'head': dd['span'][3]} for dd in sorted(doors, key=lambda q: q['span'][0])],
            })
            self._edge_trim(e, a_out, b_out, trim)
            if self.is_street(e):
                self._facade_relief(e, a_out, b_out, es)
        self._rotunda()
        front = next((e for e in self.edges if e['role'] == 'front'), None)
        if front:
            fi = front['index']
            fa, fb = self.poly[fi], self.poly[(fi + 1) % n]
            self._bays(front, fa, fb)
            self._fire_escape(front, fa, fb)
            self._blade_sign(front, fa, fb)
        # roof
        roof = MeshBuilder('roof')
        top = self.roof_level
        pts = [B(p[0], top + 0.02, p[1]) for p in self.inner]
        roof.poly(pts, s['roof'], B(0, 1, 0))
        # `hipRoof: {riseM, material}`: a low hipped roof on a four-sided
        # footprint, rising from the wall head inside the LiDAR allowance
        hip = s.get('hipRoof')
        if hip and len(self.inner) == 4:
            rise = min(hip.get('riseM', 0.55), self.height + 0.58 - top)
            hm = MeshBuilder('hip-roof')
            ring = self.inner
            e0 = length(sub(ring[1], ring[0])) + length(sub(ring[3], ring[2]))
            e1 = length(sub(ring[2], ring[1])) + length(sub(ring[0], ring[3]))
            # ridge along the long axis, inset from each short end by half the short span
            if e0 < e1:
                ring = ring[1:] + ring[:1]
            a, b_, c_, d_ = ring
            m_ad, m_bc = lerp2(a, d_, 0.5), lerp2(b_, c_, 0.5)
            half = min(length(sub(a, d_)), length(sub(b_, c_))) / 2
            axis = norm(sub(m_bc, m_ad))
            r0, r1 = add(m_ad, mul(axis, half)), sub(m_bc, mul(axis, half))
            y0, y1 = top + 0.02, top + 0.02 + rise
            mat = hip.get('material', 'asphalt-roof')
            R0, R1 = B(r0[0], y1, r0[1]), B(r1[0], y1, r1[1])
            for p, q, apex in ((a, b_, (R0, R1)), (c_, d_, (R1, R0))):
                hm.quad([B(p[0], y0, p[1]), B(q[0], y0, q[1]), apex[1], apex[0]], mat, B(0, 1, 0))
            hm.quad([B(b_[0], y0, b_[1]), B(c_[0], y0, c_[1]), R1], mat, B(0, 1, 0))
            hm.quad([B(d_[0], y0, d_[1]), B(a[0], y0, a[1]), R0], mat, B(0, 1, 0))
            hm.build(self.exterior)
        # `rooftop: {units, seed, material}`: HVAC boxes and ducts on the roof
        # deck, kept inside the parapet and under the LiDAR height + 0.6 m
        rt = s.get('rooftop')
        if rt:
            rng = random.Random(rt.get('seed', 3))
            rmb = MeshBuilder('rooftop-units')
            x0, x1, z0, z1 = self.rect
            cap = self.height + 0.5 - top - 0.07   # the lid sits 0.06 above the box
            for _ in range(int(rt.get('units', 6))):
                w_, d_ = rng.uniform(1.0, 2.6), rng.uniform(0.9, 2.0)
                h_ = min(cap, rng.uniform(0.7, 1.15))
                if x1 - x0 < w_ + 3 or z1 - z0 < d_ + 3 or h_ < 0.3:
                    continue
                cx_, cz_ = rng.uniform(x0 + 1.5 + w_ / 2, x1 - 1.5 - w_ / 2), rng.uniform(z0 + 1.5 + d_ / 2, z1 - 1.5 - d_ / 2)
                if not self._inside((cx_, cz_)):
                    continue
                rmb.box((cx_, top + h_ / 2, cz_), (w_, h_, d_), rt.get('material', 'painted-steel-light'))
                rmb.box((cx_, top + h_ + 0.03, cz_), (w_ * 0.5, 0.06, d_ * 0.5), 'painted-steel-mid')
            rmb.build(self.exterior)
        # Shadow proxy: the footprint prism just inside the inner wall face, up
        # to the roof. The runtime draws it into the shadow map only (see
        # blender/runtime.js). Without it the sun passes straight through a
        # front and a rear window in line (glass and interiors cast nothing)
        # and a tall building throws a lattice of lit rectangles on the road.
        proxy = MeshBuilder('shadow-proxy')
        ring = self._inset(WALL_T + 0.05)
        proxy.poly([B(p[0], top, p[1]) for p in ring], 'int-shadow-proxy', B(0, 1, 0))
        for i in range(len(ring)):
            p, q = ring[i], ring[(i + 1) % len(ring)]
            nout = mul(self.inward(self.poly[i], self.poly[(i + 1) % len(self.poly)]), -1)
            proxy.quad([B(p[0], 0.05, p[1]), B(q[0], 0.05, q[1]), B(q[0], top, q[1]), B(p[0], top, p[1])],
                       'int-shadow-proxy', B(nout[0], 0, nout[1]))
        proxy.build(self.exterior)
        self.stats['exterior'] = {}
        for m in (mb, trim, glass, frames, store, roof):
            obj = m.build(self.exterior)
            if obj:
                self.stats['exterior'][m.name] = len(m.faces)

    # -- style positions: fractions from the viewer's LEFT, standing in the
    # street facing that edge. Which end of a -> b that is depends on the
    # footprint's winding (DataSF rings run either way) and on whether the
    # authoring frame is drawn mirrored (`frameMirrored`: the z-north frame
    # replayed under reflection.js, where the drawn viewer's right is the
    # authoring viewer's left).

    def left_to_right(self, e):
        """True when edge e runs a -> b from the street viewer's left to right."""
        a, b = v2(e['a']), v2(e['b'])
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        right = (nout[1], -nout[0])  # facing -nout with y up: right = (n.z, -n.x)
        if self.spec.get('frameMirrored'):
            right = mul(right, -1)
        return dot(d, right) > 0

    def f2s(self, f, L, e=None):
        e = e or next(x for x in self.edges if x['role'] == 'front')
        return f * L if self.left_to_right(e) else (1.0 - f) * L

    # -- per-edge style: `streetEdges` dresses more than one street face
    # (corners, jogged frontages). Keys are edge indices; values override the
    # building style for that edge only (spans, layout, frameGrid, ...).

    def edge_style(self, e):
        extra = (self.style.get('streetEdges') or {}).get(str(e['index']))
        if not extra:
            return self.style
        merged = dict(self.style)
        for k, v in extra.items():
            if isinstance(v, dict) and isinstance(merged.get(k), dict):
                merged[k] = {**merged[k], **v}
            else:
                merged[k] = v
        # a street edge never inherits the front-only projections
        for k in ('bays', 'fireEscape', 'bladeSign'):
            if k not in extra:
                merged.pop(k, None)
        if 'layout' not in (extra.get('storefront') or {}):
            merged['storefront'] = {**merged['storefront'], 'layout': []}
        if 'glassModel' in extra:
            apply_glass_model(merged)
        return merged

    def is_street(self, e):
        return e['role'] == 'front' or str(e['index']) in (self.style.get('streetEdges') or {})

    # -- materials with tints and the ground band

    def tinted(self, name, tint):
        return f'{name}@{tint.lstrip("#").lower()}' if tint else name

    # `glassBacking` (style or a streetEdges override; absent = unchanged):
    #   {material, tints: [...], depthM, kinds: [punched|ribbon|high|bay|grid],
    #    blindTint, blindMax}
    # An opaque room-tone plane `depthM` behind every pane of the listed window
    # kinds, one of `tints` per window (hashed from its position), plus an
    # optional drawn blind (0..blindMax of the pane height, from the head).
    # A window whose interior is not streamed in (the interior LOD is 90 m)
    # then reads as a lit room or a blind through the glass, not a dark hole.
    def _glass_backing(self, builder, s, kind, p, y0, y1, w, yaw, inward):
        gb = s.get('glassBacking')
        # an exterior-only build glazes every pane opaque (exterior_scope_glazing):
        # a room-tone plane behind it would never be seen
        if getattr(self, 'scope', 'full') == 'exterior':
            return
        if not gb or kind not in gb.get('kinds', ('punched', 'ribbon', 'bay')):
            return
        tints = gb.get('tints') or [None]
        hsh = int(abs(p[0] * 7.31 + p[1] * 3.17 + y0 * 5.03) * 97)
        mat = self.tinted(gb.get('material', 'painted-white'), tints[hsh % len(tints)])
        pane = ('top', 'bottom', '+x', '-x')
        q = add(p, mul(inward, gb.get('depthM', 0.3)))
        builder.box((q[0], (y0 + y1) / 2, q[1]), (w, y1 - y0, 0.02), mat, yaw, skip=pane)
        bmax = float(gb.get('blindMax', 0) or 0)
        if bmax > 0 and gb.get('blindTint'):
            drop = (y1 - y0) * bmax * ((hsh // 7) % 5) / 4
            if drop > 0.05:
                q = add(p, mul(inward, gb.get('depthM', 0.3) * 0.5))
                builder.box((q[0], y1 - drop / 2, q[1]), (w, drop, 0.02),
                            self.tinted(gb.get('material', 'painted-white'), gb['blindTint']), yaw, skip=pane)

    @property
    def wall_material(self):
        return self.tinted(self.style['wall'], self.style.get('wallTint'))

    @property
    def band_top(self):
        band = self.style.get('groundBand')
        if not band:
            return None
        return band.get('topM') or (self.levels[1] + 0.45 if self.storeys > 1 else None)

    def wall_material_at(self, y0, y1):
        band = self.style.get('groundBand')
        if band and self.band_top and (y0 + y1) / 2 < self.band_top:
            return self.tinted(band.get('material', self.style['wall']), band.get('tint'))
        return self.wall_material

    # -- projecting 3-sided bays (their own mesh: projectionExempt at runtime)

    def _bays(self, e, a, b):
        s = self.style
        specs = s.get('bays') or []
        if not specs:
            return
        L = e['lengthM']
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        mb = MeshBuilder('bay-windows')
        glass = MeshBuilder('bay-glazing')
        for spec in specs:
            c, w, p = self.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.6)
            mat = self.tinted(spec.get('material', 'painted-stucco'), spec.get('tint'))
            k0 = spec.get('fromStorey', 1)
            k1 = min(self.storeys, spec.get('toStorey', self.storeys - 1) + 1)
            y_bottom = self.levels[k0]
            y_top = self.levels[k1] - 0.3 if k1 < self.storeys else self.roof_level

            def at(sv, off):
                return add(add(a, mul(d, sv)), mul(nout, off))

            # `roundCorners: n` bends the two side faces into quarter arcs of n
            # facets (a bowed curtain-wall bay); absent, the classic 3-sided bay
            rc = int(spec.get('roundCorners', 0) or 0)
            if rc > 0:
                ring = [at(c - w / 2 + p - p * math.cos(math.pi / 2 * i / rc), p * math.sin(math.pi / 2 * i / rc)) for i in range(rc + 1)]
                ring += [at(c + w / 2 - p + p * math.cos(math.pi / 2 * (1 - i / rc)), p * math.sin(math.pi / 2 * (1 - i / rc))) for i in range(rc + 1)]
            else:
                ring = [at(c - w / 2, 0), at(c - w / 2 + p, p), at(c + w / 2 - p, p), at(c + w / 2, 0)]
            centre = at(c, p * 0.4)
            # `glazed`: every face is a full-height pane between the storey
            # plates, thin mullions at the facet joints and a sill rail (a
            # glass curtain-wall bay) instead of a punched window in a wall
            glazed = bool(spec.get('glazed'))
            gmat = spec.get('glass', s['glass'])
            fmat = self.tinted(spec.get('frame', s['frame']), spec.get('frameTint'))
            # corbel under the bay, plates at every storey line, cap on top
            self._prism(mb, ring, y_bottom - (0.2 if glazed else 0.45), y_bottom + 0.05, mat)
            for k in range(k0 + 1, k1):
                self._prism(mb, ring, self.levels[k] - 0.12, self.levels[k] + 0.12, mat)
            self._prism(mb, [ring[0]] + [add(q, mul(nout, 0.12)) for q in ring[1:-1]] + [ring[-1]], y_top - 0.05, y_top + 0.35, mat)
            for k in range(k0, k1):
                y0 = self.levels[k] + 0.05
                y1 = (self.levels[k + 1] if k + 1 < self.storeys else self.roof_level) - 0.12
                if glazed:
                    gy0 = y0 + 0.07
                    for fa, fb in zip(ring[:-1], ring[1:]):
                        fl = length(sub(fb, fa))
                        fyaw = box_yaw(norm(sub(fb, fa)))
                        gc = lerp2(fa, fb, 0.5)
                        glass.box((gc[0], (gy0 + y1) / 2, gc[1]), (fl, y1 - gy0, 0.02), gmat, fyaw)
                        self._glass_backing(glass, s, 'bay', gc, gy0, y1, fl, fyaw, norm(sub(centre, gc)))
                        # `paneM` (glazed bays): intermediate mullions so a wide
                        # facet reads as several lights, not one sheet
                        npane = int(round(fl / spec['paneM'])) if spec.get('paneM') else 1
                        fdir = norm(sub(fb, fa))
                        for i in range(1, npane):
                            q = add(fa, mul(fdir, fl * i / npane))
                            glass.box((q[0], (gy0 + y1) / 2, q[1]), (0.05, y1 - gy0, 0.06), fmat, fyaw, skip=('top', 'bottom'))
                        glass.box((gc[0], gy0 + 0.95, gc[1]), (fl, 0.05, 0.07), fmat, fyaw, skip=('+x', '-x'))
                    for q in ring:
                        glass.box((q[0], (gy0 + y1) / 2, q[1]), (0.07, y1 - gy0, 0.07), fmat, 0.0, skip=('top', 'bottom'))
                    continue
                for fa, fb in zip(ring[:-1], ring[1:]):
                    fl = length(sub(fb, fa))
                    fd = norm(sub(fb, fa))
                    fn = (-fd[1], fd[0])
                    if dot(sub(lerp2(fa, fb, 0.5), centre), fn) < 0:
                        fn = (fd[1], -fd[0])
                    ia, ib = sub(fa, mul(fn, 0.1)), sub(fb, mul(fn, 0.1))
                    ww = min(fl - 0.3, 1.05)
                    sill, head = y0 + 0.75, y1 - 0.35
                    wall_grid(mb, fa, fb, ia, ib, y0, y1, [((fl - ww) / 2, (fl + ww) / 2, sill, head)],
                              mat, 'int-plaster', mat, cap_top=False)
                    gc = add(lerp2(fa, fb, 0.5), mul(fn, -0.05))
                    fyaw = box_yaw(fd)
                    glass.box((gc[0], (sill + head) / 2, gc[1]), (ww, head - sill, 0.02), s['glass'], fyaw)
                    for yy in (sill + 0.03, head - 0.03, (sill + head) / 2 + 0.05):
                        glass.box((gc[0], yy, gc[1]), (ww, 0.06, 0.08), s['frame'], fyaw)
                    for side in (-1, 1):
                        q = add(gc, mul(fd, side * (ww / 2 - 0.03)))
                        glass.box((q[0], (sill + head) / 2, q[1]), (0.06, head - sill, 0.08), s['frame'], fyaw)
        mb.build(self.exterior)
        glass.build(self.exterior)

    def _prism(self, mb, ring, y0, y1, mat):
        cx = sum(p[0] for p in ring) / len(ring)
        cz = sum(p[1] for p in ring) / len(ring)
        mb.poly([B(p[0], y1, p[1]) for p in ring], mat, B(0, 1, 0))
        mb.poly([B(p[0], y0, p[1]) for p in ring], mat, B(0, -1, 0))
        for i in range(len(ring)):
            p, q = ring[i], ring[(i + 1) % len(ring)]
            mid = lerp2(p, q, 0.5)
            out = B(mid[0] - cx, 0, mid[1] - cz)
            mb.quad([B(p[0], y0, p[1]), B(q[0], y0, q[1]), B(q[0], y1, q[1]), B(p[0], y1, p[1])], mat, out)

    # -- steel fire escape on the front (visual; not climbable)

    def _fire_escape(self, e, a, b):
        spec = self.style.get('fireEscape')
        if not spec or self.storeys < 2:
            return
        L = e['lengthM']
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        yaw = box_yaw(d)
        c, w, depth = self.f2s(spec['at'], L, e), spec.get('widthM', 3.4), spec.get('depthM', 0.9)
        mat = spec.get('material', 'painted-steel')
        mb = MeshBuilder('fire-escape')

        def at(sv, off):
            return add(add(a, mul(d, sv)), mul(nout, off))

        k_last = min(self.storeys - 1, spec.get('toStorey', self.storeys - 1))
        for k in range(1, k_last + 1):
            y = self.levels[k] - 0.02
            p = at(c, depth / 2 + 0.02)
            mb.box((p[0], y, p[1]), (w, 0.05, depth), mat, yaw)
            # railing: top rail, mid rail, posts, front and both ends
            for yy in (y + 1.0, y + 0.5):
                q = at(c, depth)
                mb.box((q[0], yy, q[1]), (w, 0.04, 0.04), mat, yaw)
                for side in (-1, 1):
                    q2 = at(c + side * w / 2, depth / 2)
                    mb.box((q2[0], yy, q2[1]), (0.04, 0.04, depth), mat, yaw)
            for i in range(int(w / 0.25) + 1):
                q = at(c - w / 2 + i * w / int(w / 0.25), depth)
                mb.box((q[0], y + 0.5, q[1]), (0.02, 1.0, 0.02), mat, yaw)
            # stair to the next platform, climbing along the balcony
            if k + 1 <= k_last:
                rise = self.levels[k + 1] - self.levels[k]
                steps = 14
                run = (w - 0.9) / steps
                for i in range(steps):
                    q = at(c - w / 2 + 0.3 + (i + 0.5) * run, depth * 0.55)
                    mb.box((q[0], y + rise * (i + 1) / (steps + 1), q[1]), (run + 0.02, 0.03, 0.55), mat, yaw)
                for side_off in (0.28, 0.82):
                    for i in range(steps):
                        q = at(c - w / 2 + 0.3 + (i + 0.5) * run, depth * side_off)
                        mb.box((q[0], y + rise * (i + 0.5) / (steps + 1) + 0.9, q[1]), (run + 0.03, 0.035, 0.035), mat, yaw)
        mb.build(self.exterior)

    # -- blank vertical blade sign on a bracket

    def _blade_sign(self, e, a, b):
        # `bladeSign` (one) or `bladeSigns` (a list): blank vertical banners on brackets
        specs = list(self.style.get('bladeSigns') or []) + ([self.style['bladeSign']] if self.style.get('bladeSign') else [])
        if not specs or self.storeys < 2:
            return
        front_e, front_a, front_b = e, a, b
        mb = MeshBuilder('blade-sign')
        for spec in specs:
            # `edge`: hang this banner on another street face (default: the front)
            e, a, b = front_e, front_a, front_b
            if spec.get('edge') is not None:
                e = next(x for x in self.edges if x['index'] == int(spec['edge']))
                a, b = self.poly[e['index']], self.poly[(e['index'] + 1) % len(self.poly)]
            L = e['lengthM']
            d = norm(sub(b, a))
            nout = mul(self.inward(a, b), -1)
            yaw = box_yaw(d)
            y0 = self.levels[1] + spec.get('bottomM', 0.6)
            h = spec.get('heightM', 3.6)
            p = at_edge(a, d, nout, self.f2s(spec['at'], L, e), 0.62)
            mb.box((p[0], y0 + h / 2, p[1]), (0.12, h, 0.7), 'signage', yaw, mats={'+x': spec.get('face', 'painted-white'), '-x': spec.get('face', 'painted-white')})
            for yy in (y0 + 0.3, y0 + h - 0.3):
                q = at_edge(a, d, nout, self.f2s(spec['at'], L, e), 0.14)
                mb.box((q[0], yy, q[1]), (0.05, 0.05, 0.28), 'painted-steel', yaw)
        mb.build(self.exterior)

    def _dress_openings(self, e, a, b, windows, trim, glass, frames, store, es=None):
        s = es or self.style
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        yaw = box_yaw(d)
        depth_glass = WALL_T * 0.35  # glass plane set back from the outer face
        # `windowDepthM` (style; absent = unchanged): a deeper window reveal
        depth_glass = min(WALL_T - 0.03, s.get('windowDepthM', depth_glass))

        def at(sv, off):
            p = add(add(a, mul(d, sv)), mul(nout, off))
            return p

        for kind, spec_span in windows:
            s0, s1, y0, y1 = spec_span[:4]
            w = s1 - s0
            h = y1 - y0
            cs = (s0 + s1) / 2
            if kind in ('punched', 'ribbon', 'high'):
                self.window_slots.append({'a': at(s0, -WALL_T), 'b': at(s1, -WALL_T), 'nin': mul(nout, -1),
                                          'sill': y0, 'head': y1, 'kind': kind})
            if kind in ('punched', 'ribbon', 'high'):
                self._glass_backing(glass, s, kind, at(cs, -depth_glass), y0, y1, w, yaw, mul(nout, -1))
            if kind == 'punched':
                g = at(cs, -depth_glass)
                glass.box((g[0], (y0 + y1) / 2, g[1]), (w, h, 0.02), s['glass'], yaw)
                # frame: jambs, head, sill, meeting rail (double hung)
                ft = 0.07
                for sv, sw in ((s0 + ft / 2, ft), (s1 - ft / 2, ft)):
                    p = at(sv, -depth_glass + 0.03)
                    frames.box((p[0], (y0 + y1) / 2, p[1]), (sw, h, 0.1), s['frame'], yaw)
                for yy in (y0 + ft / 2, y1 - ft / 2, (y0 + y1) / 2 + 0.05):
                    p = at(cs, -depth_glass + 0.03)
                    frames.box((p[0], yy, p[1]), (w, ft, 0.1), s['frame'], yaw)
                # stone sill and hood
                p = at(cs, 0.03)
                trim.box((p[0], y0 - 0.05, p[1]), (w + 0.24, 0.1, 0.06 + WALL_T * 0.0 + 0.06), s['trim'], yaw)
                if s.get('windowHoods'):
                    p = at(cs, 0.04)
                    trim.box((p[0], y1 + 0.09, p[1]), (w + 0.3, 0.18, 0.08), s['trim'], yaw)
            elif kind == 'grille':
                # security grille flush with the outer face: bars + rails
                gm = s.get('grilleMaterial', 'painted-steel')
                bars = max(2, int(w / 0.12))
                for i in range(bars + 1):
                    p = at(s0 + 0.03 + (w - 0.06) * i / bars, 0.02)
                    store.box((p[0], (y0 + y1) / 2, p[1]), (0.02, h, 0.02), gm, yaw)
                for yy in (y0 + 0.08, (y0 + y1) / 2, y1 - 0.06):
                    p = at(cs, 0.02)
                    store.box((p[0], yy, p[1]), (w, 0.04, 0.03), gm, yaw)
            elif kind == 'ribbon':
                g = at(cs, -depth_glass)
                glass.box((g[0], (y0 + y1) / 2, g[1]), (w, h, 0.02), s['glass'], yaw)
                ft = 0.06
                sash_mat = s.get('sashMaterial', s['storefrontFrame'])
                n_lights = max(1, int(round(w / s.get('mullionM', 1.4))))
                for i in range(n_lights + 1):
                    p = at(s0 + ft / 2 + (w - ft) * i / n_lights, -depth_glass + 0.03)
                    frames.box((p[0], (y0 + y1) / 2, p[1]), (ft, h, 0.1), sash_mat, yaw)
                for yy in (y0 + ft / 2, y1 - ft / 2):
                    p = at(cs, -depth_glass + 0.03)
                    frames.box((p[0], yy, p[1]), (w, ft, 0.1), sash_mat, yaw)
                # steel-sash muntins: `sashRows` panes high, `sashCols` per light
                rows, cols = int(s.get('sashRows', 1)), int(s.get('sashCols', 1))
                mt = 0.035
                for r in range(1, rows):
                    p = at(cs, -depth_glass + 0.02)
                    frames.box((p[0], y0 + h * r / rows, p[1]), (w, mt, 0.05), sash_mat, yaw)
                if cols > 1:
                    lw = w / n_lights
                    for i in range(n_lights):
                        for c in range(1, cols):
                            p = at(s0 + lw * i + lw * c / cols, -depth_glass + 0.02)
                            frames.box((p[0], (y0 + y1) / 2, p[1]), (mt, h, 0.05), sash_mat, yaw)
                # an operable hopper band, drawn tilted open on some lights
                if s.get('sashHopper') and rows > 1:
                    lw = w / n_lights
                    for i in range(n_lights):
                        if (i + int(y0 * 7)) % 3:
                            continue
                        hy = y0 + h * (rows // 2) / rows
                        p = at(s0 + lw * (i + 0.5), -depth_glass + 0.09)
                        frames.box((p[0], hy + h / rows / 2, p[1]), (lw - 0.1, h / rows - 0.06, 0.03), s['glass'], yaw)
                p = at(cs, 0.03)
                trim.box((p[0], y0 - 0.04, p[1]), (w + 0.1, 0.08, 0.08), s['storefrontFrame'], yaw)
            elif kind == 'storefront-door':
                sf = s['storefront']
                bulk = sf['bulkheadM']
                transom = sf['transomM']
                d0, d1 = spec_span[4], spec_span[5]
                leaf_mat = spec_span[6] if len(spec_span) > 6 else s['storefrontFrame']
                double = len(spec_span) > 7 and spec_span[7] == 'double-door'
                gh1 = y1 - transom
                dh = min(2.45, gh1)
                ft = 0.06
                # display panes either side of the door
                for p0, p1 in ((s0, d0), (d1, s1)):
                    if p1 - p0 < 0.15:
                        continue
                    pc = (p0 + p1) / 2
                    g = at(pc, -0.12)
                    store.box((g[0], bulk / 2, g[1]), (p1 - p0, bulk, 0.08), s['bulkhead'], yaw)
                    glass.box((g[0], (bulk + gh1) / 2, g[1]), (p1 - p0, gh1 - bulk, 0.02), s['storefrontGlass'], yaw)
                    store.box((g[0], bulk, g[1]), (p1 - p0, ft, 0.09), s['storefrontFrame'], yaw)
                    for sv in (p0 + ft / 2, p1 - ft / 2):
                        p = at(sv, -0.12)
                        store.box((p[0], y1 / 2, p[1]), (ft, y1, 0.09), s['storefrontFrame'], yaw)
                # transom across the whole bay and a head over the door
                g = at(cs, -0.12)
                glass.box((g[0], (gh1 + y1) / 2, g[1]), (w, transom, 0.02), s['storefrontGlass'], yaw)
                store.box((g[0], gh1, g[1]), (w, ft, 0.09), s['storefrontFrame'], yaw)
                store.box((g[0], y1 - ft / 2, g[1]), (w, ft, 0.09), s['storefrontFrame'], yaw)
                self._transom_panes(store, at, s, sf, s0, s1, gh1, y1, yaw)
                gd = at((d0 + d1) / 2, -0.12)
                store.box((gd[0], dh + 0.03, gd[1]), (d1 - d0, 0.06, 0.1), s['storefrontFrame'], yaw)
                if dh + 0.06 < gh1:
                    glass.box((gd[0], (dh + 0.06 + gh1) / 2, gd[1]), (d1 - d0, gh1 - dh - 0.06, 0.02), s['storefrontGlass'], yaw)
                # leaves standing open inward against the jambs (a doorway the
                # player walks through; physics treats it as an open passage)
                leaves = [(d0 + 0.03, (d1 - d0) / 2 - 0.04), (d1 - 0.03, (d1 - d0) / 2 - 0.04)] if double else [(d0 + 0.03, d1 - d0 - 0.06)]
                for hinge_s, leaf_w in leaves:
                    hinge = at(hinge_s, -0.14)
                    c = add(hinge, mul(nout, -leaf_w / 2))
                    store.box((c[0], dh / 2, c[1]), (0.05, dh - 0.02, leaf_w), leaf_mat, yaw)
                    # glazed upper panel in the leaf
                    g2 = add(c, mul(d, 0.03 if hinge_s < (d0 + d1) / 2 else -0.03))
                    glass.box((g2[0], dh * 0.62, g2[1]), (0.02, dh * 0.5, leaf_w * 0.7), s['glass'], yaw)
            elif kind == 'storefront':
                sf = s['storefront']
                bulk = sf['bulkheadM']
                transom = sf['transomM']
                g = at(cs, -0.12)
                # bulkhead panel
                store.box((g[0], bulk / 2, g[1]), (w, bulk, 0.08), s['bulkhead'], yaw)
                # display glass and transom glass
                gh0, gh1 = bulk, y1 - transom
                glass.box((g[0], (gh0 + gh1) / 2, g[1]), (w, gh1 - gh0, 0.02), s['storefrontGlass'], yaw)
                glass.box((g[0], (gh1 + y1) / 2, g[1]), (w, transom, 0.02), s['storefrontGlass'], yaw)
                ft = 0.06
                for yy in (bulk, gh1, y1 - ft / 2):
                    store.box((g[0], yy, g[1]), (w, ft, 0.09), s['storefrontFrame'], yaw)
                mids = [cs] if not sf.get('paneM') else [s0 + w * i / max(1, round(w / sf['paneM'])) for i in range(1, max(1, round(w / sf['paneM'])))]
                for sv in [s0 + ft / 2, s1 - ft / 2] + mids:
                    p = at(sv, -0.12)
                    store.box((p[0], y1 / 2, p[1]), (ft, y1, 0.09), s['storefrontFrame'], yaw)
                self._transom_panes(store, at, s, sf, s0, s1, gh1, y1, yaw)
            elif kind in ('garage', 'glazed-garage', 'blank', 'gate'):
                self._closed_infill(kind, spec_span, at, s, store, glass, yaw)
            elif kind.startswith('fg-'):
                self._frame_grid_bay(kind, spec_span, at, s, store, glass, frames, yaw, nout)
            elif kind in ('stair-door', 'service-door'):
                p = at(cs, -WALL_T * 0.5)
                # door leaf (closed look is wrong for walk-in; ajar leaf inside the reveal)
                leaf_w = w - 0.08
                hinge = at(s0 + 0.04, -WALL_T * 0.5)
                # leaf swung fully inward against the jamb
                inward_dir = mul(nout, -1)
                c = add(hinge, mul(inward_dir, leaf_w / 2))
                store.box((c[0], (y0 + y1) / 2 - 0.02, c[1]), (0.05, y1 - y0 - 0.05, leaf_w), 'painted-steel-mid' if kind == 'service-door' else 'wood-siding', yaw)
                # frame head
                p2 = at(cs, -0.02)
                store.box((p2[0], y1 + 0.04, p2[1]), (w + 0.1, 0.08, 0.08), s['frame'], yaw)
                if kind == 'stair-door':
                    glass.box((p2[0], y1 + 0.25, p2[1]), (w - 0.1, 0.3, 0.02), s['glass'], yaw)
            elif kind == 'high':
                g = at(cs, -depth_glass)
                glass.box((g[0], (y0 + y1) / 2, g[1]), (w, h, 0.02), s['glass'], yaw)
                p = at(cs, 0.03)
                trim.box((p[0], y0 - 0.04, p[1]), (w + 0.16, 0.08, 0.1), s['trim'], yaw)

    # ---------------------------------------------------------- facade systems
    #
    # Style options for street faces (front and `streetEdges`); every one is
    # absent by default, so buildings that do not name them build unchanged.
    #   pilasters   {at: 'spans' | [fractions], widthM, projectionM, fromM, toM,
    #                material, tint, flutes, capital}
    #   spandrels   {material, tint}: framed panels between storey windows
    #   stripes     {fromStorey, offsetsM, heightM, material, tint}: banded precast
    #   coping      {heightM, projectionM, material, tint}: parapet cap
    #   frameGrid   exposed concrete frame: {columns | bayM, columnW, slabEdgeH,
    #                projectionM, fromStorey, pattern, storeyPatterns, shift,
    #                recessM, rail, mullionM, material, tint, frameMaterial,
    #                panelMaterial, panelTint, panelCoverage, panelFrame}
    #   storefront.layout kinds garage | glazed-garage | blank | gate (closed infills)
    #   storefront.transomPaneM / transomRows / paneM, sashRows / sashCols / sashMaterial

    def _projection_cap(self, d):
        # same world-AABB allowance rule as the cornice (see _edge_trim)
        h = self.spec['frame']['heading']
        dw = (d[0] * math.cos(h) + d[1] * math.sin(h), -d[0] * math.sin(h) + d[1] * math.cos(h))
        return 0.43 / (abs(dw[0]) + abs(dw[1])) - 0.03

    def grid_columns(self, grid, L, e):
        cw = grid.get('columnW', 0.4)
        if grid.get('columns'):
            cols = sorted(self.f2s(f, L, e) for f in grid['columns'])
        else:
            n = max(1, int(round(L / grid.get('bayM', 4.0))))
            cols = [L * i / n for i in range(n + 1)]
        return [min(max(c, cw / 2), L - cw / 2) for c in cols]

    def _frame_grid_openings(self, e, s, grid, L):
        """Bay openings of an exposed concrete frame: one opening per bay per
        storey between the columns and the slab edges, filled per `pattern`
        (glass | balcony | panel | screen | wall)."""
        cols = self.grid_columns(grid, L, e)
        cw = grid.get('columnW', 0.4)
        sh = grid.get('slabEdgeH', 0.35)
        first = max(1, int(grid.get('fromStorey', 1)))
        pattern = grid.get('pattern') or ['glass']
        rows = grid.get('storeyPatterns')
        ops, windows = [], []
        for k in range(first, self.storeys):
            y0 = self.levels[k] + sh / 2
            top_line = self.levels[k + 1] if k + 1 < self.storeys else self.roof_level
            y1 = top_line - sh / 2
            row = rows[(k - first) % len(rows)] if rows else pattern
            for i in range(len(cols) - 1):
                kind = row[(i + k * int(grid.get('shift', 0))) % len(row)]
                s0, s1 = cols[i] + cw / 2, cols[i + 1] - cw / 2
                if kind == 'wall' or s1 - s0 < 0.5:
                    continue
                o = (s0, s1, y0, y1)
                ops.append(o)
                windows.append(('fg-' + kind, o))
        return ops, windows, first

    def _frame_grid_bay(self, kind, span, at, s, store, glass, frames, yaw, nout):
        grid = s.get('frameGrid') or {}
        s0, s1, y0, y1 = span[:4]
        w, h, cs = s1 - s0, y1 - y0, (s0 + s1) / 2
        fmat = grid.get('frameMaterial', 'anodized-aluminium')
        cmat = self.tinted(grid.get('material', 'concrete-precast'), grid.get('tint'))
        # `glassRecessM` sets the glass bays back behind the frame too (with
        # reveals), so the frame reads as a deep grid rather than a flat one
        recess = grid.get('recessM', 1.2) if kind in ('fg-balcony', 'fg-panel') else grid.get('glassRecessM', WALL_T * 0.55)
        if kind == 'fg-loggia':
            recess = grid.get('loggiaRecessM', 0.3)
        # glass wall at the recess line: mullions and a door-height bar (the
        # frame's own reveals are the head and sill). Faces nobody can see
        # (ends of thin members) are skipped: a nine-storey frame has ~200 bays.
        vskip, hskip, pane = ('top', 'bottom'), ('+x', '-x'), ('top', 'bottom', '+x', '-x')
        g = at(cs, -recess)
        glass.box((g[0], (y0 + y1) / 2, g[1]), (w, h, 0.02), s['glass'], yaw, skip=pane)
        # backing kinds: 'grid' (glass bays), 'grid-balcony', 'grid-panel'
        self._glass_backing(glass, s, 'grid' if kind == 'fg-glass' else 'grid-' + kind[3:], g, y0, y1, w, yaw, mul(nout, -1))
        if kind == 'fg-loggia':
            fmat = grid.get('loggiaFrame', fmat)
            self._g1_loggia(grid, s0, s1, y0, y1, at, store, glass, yaw)
        n = max(1, int(round(w / grid.get('loggiaMullionM' if kind == 'fg-loggia' else 'mullionM', 1.3))))
        for i in range(n + 1):
            p = at(s0 + 0.03 + (w - 0.06) * i / n, -recess + 0.03)
            frames.box((p[0], (y0 + y1) / 2, p[1]), (grid.get('mullionW', 0.06), h, grid.get('mullionDepthM', 0.08)), fmat, yaw, skip=vskip)
        p = at(cs, -recess + 0.03)
        frames.box((p[0], y0 + min(2.2, h - 0.4), p[1]), (w, grid.get('mullionW', 0.06), grid.get('mullionDepthM', 0.08)), fmat, yaw, skip=hskip)
        self.window_slots.append({'a': at(s0, -recess), 'b': at(s1, -recess), 'nin': mul(nout, -1),
                                  'sill': y0, 'head': y1, 'kind': 'frame'})
        if recess > WALL_T:
            depth = recess - WALL_T
            mid = -(WALL_T + recess) / 2
            # the recess reveals take their own material (`balconyMaterial`,
            # `balconyTint`): a downward soffit is lit mostly by the ground
            # bounce, so it needs a different tint from the frame to read the same
            bmat = self.tinted(grid.get('balconyMaterial', grid.get('material', 'concrete-precast')),
                               grid.get('balconyTint', grid.get('tint')))
            for sv in (s0 + 0.05, s1 - 0.05):
                p = at(sv, mid)
                store.box((p[0], (y0 + y1) / 2, p[1]), (0.1, h, depth), bmat, yaw, skip=vskip)
            p = at(cs, mid)
            store.box((p[0], y1 - 0.05, p[1]), (w, 0.1, depth), bmat, yaw, skip=hskip + ('top',))   # soffit
            store.box((p[0], y0 + 0.05, p[1]), (w, 0.1, depth), bmat, yaw, skip=hskip + ('bottom',))   # balcony deck
            if kind == 'fg-balcony' and grid.get('balconyPlanters'):
                # a planter box with a shrub line on the deck, against the guard
                pl = at(s0 + w * (0.22 if (int(y0 * 10) % 2) else 0.7), -0.45)
                store.box((pl[0], y0 + 0.3, pl[1]), (min(1.1, w * 0.3), 0.4, 0.4), 'painted-steel-mid', yaw, skip=('bottom',))
                store.box((pl[0], y0 + 0.62, pl[1]), (min(1.0, w * 0.28), 0.28, 0.34), 'int-plant', yaw, skip=('bottom',))
            q = at(cs, -0.06)
            # `balconyLipM` (absent = unchanged): open balconies
            # get a deck slab projecting past the wall line, the guard at its lip
            lip = _g1_balcony_lip(self, grid, kind, s0, s1, y0, at, store, bmat, yaw) if kind == 'fg-balcony' else 0.0
            if lip > 0:
                q = at(cs, lip - 0.04)
            if kind == 'fg-glass' or (kind == 'fg-panel' and grid.get('panelRail') is False):
                pass  # a recessed window wall, or a screen bay whose leaves are the guard
            elif grid.get('rail') == 'solid':
                # An opaque dark steel guard plate with a cap,
                # the solid balcony front of a louvred residential frame
                rm = self.tinted(grid.get('railMaterial', 'painted-steel'), grid.get('railTint'))
                store.box((q[0], y0 + 0.53, q[1]), (w - 0.02, 1.02, 0.03), rm, yaw, skip=hskip + ('bottom',))
                store.box((q[0], y0 + 1.06, q[1]), (w, 0.05, 0.07), rm, yaw, skip=hskip + ('bottom',))
            elif grid.get('rail', 'glass') == 'glass':
                glass.box((q[0], y0 + 0.55, q[1]), (w - 0.1, 0.95, 0.02), grid.get('railGlass', s['glass']), yaw, skip=pane)
                store.box((q[0], y0 + 1.05, q[1]), (w, 0.05, 0.06), grid.get('railMaterial', fmat), yaw, skip=hskip)
                for sv in (s0 + w * 0.34, s1 - w * 0.34):   # two posts
                    pp = at(sv, lip - 0.04 if lip > 0 else -0.06)
                    store.box((pp[0], y0 + 0.53, pp[1]), (0.05, 1.05, 0.05), grid.get('railMaterial', fmat), yaw, skip=vskip)
            else:
                store.box((q[0], y0 + 1.05, q[1]), (w, 0.05, 0.05), fmat, yaw)
                nb = max(2, int(w / 0.12))
                for i in range(nb + 1):
                    p = at(s0 + 0.05 + (w - 0.1) * i / nb, -0.06)
                    store.box((p[0], y0 + 0.55, p[1]), (0.02, 1.0, 0.02), fmat, yaw)
        if kind in ('fg-panel', 'fg-screen'):
            # sliding screen leaves in front of the bay, parked to one side
            pm = self.tinted(grid.get('panelMaterial', 'perforated-metal'), grid.get('panelTint'))
            # `panelCoverageRange: [lo, hi]` slides each bay's leaves to its
            # own position (seeded per bay): some bays closed, some half open
            # onto the dark recess, instead of one fixed coverage everywhere
            rng_cov = grid.get('panelCoverageRange')
            cov = self.rng.uniform(*rng_cov) if rng_cov else grid.get('panelCoverage', 0.55)
            pw = w * cov
            left = self.rng.random() < 0.5
            ps0 = s0 + 0.05 if left else s1 - 0.05 - pw
            leaves = max(1, int(grid.get('panelLeaves', 2)))
            lw = pw / leaves
            louvres = int(grid.get('panelLouvres', 0))
            inset = grid.get('panelInsetM', 0.12)
            if self._g1_panel_leaf(grid, kind, s0, s1, y0, y1, at, store, frames, yaw, pm):
                return
            for j in range(leaves):
                sv = ps0 + lw * (j + 0.5)
                p = at(sv, -inset - 0.05 * j)
                store.box((p[0], (y0 + y1) / 2, p[1]), (lw - 0.02, h - 0.04, 0.03), pm, yaw, skip=pane)
                p2 = at(sv - lw / 2 + 0.03, -inset + 0.02 - 0.05 * j)
                frames.box((p2[0], (y0 + y1) / 2, p2[1]), (0.05, h - 0.04, 0.05), grid.get('panelFrame', 'painted-white'), yaw, skip=vskip)
                # louvre blades across the leaf: the face and the shadowed
                # underside only (tops and ends are never seen from the street)
                for n in range(louvres):
                    yy = y0 + 0.1 + (h - 0.2) * (n + 0.5) / louvres
                    q = at(sv, -inset + 0.05 - 0.05 * j)
                    store.box((q[0], yy, q[1]), (lw - 0.06, 0.035, 0.08), pm, yaw, skip=('top', '+x', '-x'))

    def _mural(self, e, a, b, spec):
        """A generic painted mural: seeded overlapping colour fields, bands and
        wedges in a palette, flush on the wall. Abstract by construction - it
        copies no artwork; it is there so a painted wall reads as painted."""
        L = e['lengthM']
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        s0, s1 = sorted((self.f2s(spec.get('from', 0.05), L, e), self.f2s(spec.get('to', 0.95), L, e)))
        y0 = spec.get('bottomM', 3.0)
        y1 = min(self.height - 0.4, spec.get('topM', self.height - 1.0))
        palette = spec.get('palette') or ['#e8d9b0', '#d2553a', '#2f6fa8', '#f0b531', '#3f8f5e', '#8b4a9c', '#f2efe6']
        rng = random.Random(spec.get('seed', 7))
        mb = MeshBuilder('mural')
        out = B(nout[0], 0, nout[1])

        def P(sv, y, layer):
            p = add(add(a, mul(d, sv)), mul(nout, 0.012 + 0.004 * layer))
            return B(p[0], y, p[1])

        # ground coat, then shapes back to front
        mb.quad([P(s0, y0, 0), P(s1, y0, 0), P(s1, y1, 0), P(s0, y1, 0)], f'painted-white@{palette[0].lstrip("#")}', out)
        W, H = s1 - s0, y1 - y0
        for layer in range(1, int(spec.get('shapes', 36)) + 1):
            col = rng.choice(palette[1:]).lstrip('#')
            mat = f'painted-white@{col}'
            kind = rng.random()
            cx, cy = s0 + rng.random() * W, y0 + rng.random() * H
            w, h = W * rng.uniform(0.08, 0.35), H * rng.uniform(0.08, 0.35)
            skew = W * rng.uniform(-0.12, 0.12)
            xa, xb = max(s0, cx - w / 2), min(s1, cx + w / 2)
            ya, yb = max(y0, cy - h / 2), min(y1, cy + h / 2)
            if xb - xa < 0.3 or yb - ya < 0.3:
                continue
            if kind < 0.45:      # skewed band
                sa, sb = max(s0, min(s1, xa + skew)), max(s0, min(s1, xb + skew))
                mb.quad([P(xa, ya, layer), P(xb, ya, layer), P(sb, yb, layer), P(sa, yb, layer)], mat, out)
            elif kind < 0.75:    # wedge
                mb.quad([P(xa, ya, layer), P(xb, ya, layer), P((xa + xb) / 2, yb, layer)], mat, out)
            else:                # block
                mb.quad([P(xa, ya, layer), P(xb, ya, layer), P(xb, yb, layer), P(xa, yb, layer)], mat, out)
        mb.build(self.exterior)

    def _transom_panes(self, store, at, s, sf, s0, s1, gh1, y1, yaw):
        tp = sf.get('transomPaneM')
        if not tp:
            return
        w = s1 - s0
        n = max(1, int(round(w / tp)))
        for i in range(1, n):
            p = at(s0 + w * i / n, -0.11)
            store.box((p[0], (gh1 + y1) / 2, p[1]), (0.04, y1 - gh1, 0.06), s['storefrontFrame'], yaw)
        rows = int(sf.get('transomRows', 1))
        for r in range(1, rows):
            p = at((s0 + s1) / 2, -0.11)
            store.box((p[0], gh1 + (y1 - gh1) * r / rows, p[1]), (w, 0.04, 0.06), s['storefrontFrame'], yaw)

    def _closed_infill(self, kind, span, at, s, store, glass, yaw):
        s0, s1, y0, y1, mat, tint = span[:6]
        w, h, cs = s1 - s0, y1 - y0, (s0 + s1) / 2
        back = -0.16
        # a layout item's own `ribs: "vertical"` overrides the edge's `garageRibs`
        if len(span) > 6 and span[6]:
            s = {**s, 'garageRibs': span[6]}
        if kind == 'garage':
            # `recessM` / `doorM` (absent = unchanged): the shutter set back in a dark garage mouth
            back, y1, h = _g1_garage_recess(self, span, store, glass, at, cs, s0, s1, y0, y1, back, yaw)
            # roll-up shutter: slat ribs on a plate, steel guides either side
            m = self.tinted(mat or 'painted-steel-light', tint)
            p = at(cs, back)
            store.box((p[0], (y0 + y1) / 2, p[1]), (w, h, 0.03), m, yaw)
            # `garageRibs: "vertical"`: a corrugated steel
            # gate, ribs running up the plate instead of roll-up slats
            n = 0 if _g1_vertical_ribs(s, store, at, cs, back, s0, w, y0, y1, h, m, yaw) else max(2, int(h / 0.1))
            for i in range(1, n):
                q = at(cs, back + 0.02)
                store.box((q[0], y0 + h * i / n, q[1]), (w - 0.02, 0.018, 0.015), m, yaw)
            for sv in (s0 + 0.04, s1 - 0.04):
                q = at(sv, back + 0.03)
                store.box((q[0], (y0 + y1) / 2, q[1]), (0.08, h, 0.06), 'painted-steel-mid', yaw)
        elif kind == 'glazed-garage':
            fm = self.tinted(mat or 'painted-white', tint)
            p = at(cs, back)
            glass.box((p[0], (y0 + y1) / 2, p[1]), (w, h, 0.02), 'glass-clear', yaw)
            cols = max(2, int(round(w / 0.75)))
            rows = max(3, int(round(h / 0.6)))
            for i in range(cols + 1):
                q = at(s0 + 0.04 + (w - 0.08) * i / cols, back + 0.03)
                store.box((q[0], (y0 + y1) / 2, q[1]), (0.07, h, 0.06), fm, yaw)
            for r in range(rows + 1):
                q = at(cs, back + 0.03)
                store.box((q[0], y0 + 0.035 + (h - 0.07) * r / rows, q[1]), (w, 0.07, 0.06), fm, yaw)
        elif kind == 'blank':
            m = self.tinted(mat or 'painted-steel-mid', tint)
            p = at(cs, back)
            store.box((p[0], (y0 + y1) / 2, p[1]), (w, h, 0.04), m, yaw)
        elif kind == 'gate':
            # full-height slatted steel gate in front of a dark recess
            m = self.tinted(mat or 'painted-steel', tint)
            p = at(cs, back - 0.25)
            store.box((p[0], (y0 + y1) / 2, p[1]), (w, h, 0.03), 'painted-steel', yaw)
            n = max(2, int(w / 0.11))
            for i in range(n + 1):
                q = at(s0 + 0.03 + (w - 0.06) * i / n, back)
                store.box((q[0], (y0 + y1) / 2, q[1]), (0.05, h, 0.04), m, yaw)
            for yy in (y0 + 0.1, y1 - 0.1):
                q = at(cs, back - 0.03)
                store.box((q[0], yy, q[1]), (w, 0.06, 0.03), m, yaw)

    def _facade_relief(self, e, a, b, s):
        """Pilasters, spandrel panels, stripes, coping and the concrete frame
        on one street face. Projections stay inside the contract allowance."""
        L = e['lengthM']
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        yaw = box_yaw(d)
        cap = self._projection_cap(d)
        mb = MeshBuilder(f'relief-{e["index"]}')

        def at(sv, off):
            return add(add(a, mul(d, sv)), mul(nout, off))

        pil = s.get('pilasters')
        if pil:
            pw = pil.get('widthM', 0.55)
            flutes = int(pil.get('flutes', 0))
            pp = max(0.02, min(pil.get('projectionM', 0.14), cap - (0.04 if flutes else 0.0)))
            mat = self.tinted(pil.get('material', s['wall']), pil.get('tint', s.get('wallTint')))
            where = pil.get('at', 'spans')
            if where == 'spans':
                spans = sorted(s.get('upperWindowSpans') or [])
                pos = [0.0, 1.0] + [(p1[1] + p2[0]) / 2 for p1, p2 in zip(spans, spans[1:])]
            else:
                pos = list(where)
            y0 = pil.get('fromM', 0.0)
            y1 = pil.get('toM') or self.height
            for f in pos:
                sv = min(max(self.f2s(f, L, e), pw / 2), L - pw / 2)
                p = at(sv, pp / 2)
                mb.box((p[0], (y0 + y1) / 2, p[1]), (pw, y1 - y0, pp + 0.01), mat, yaw)
                for j in range(flutes):
                    q = at(sv - pw / 2 + pw * (j + 0.5) / flutes, pp + 0.015)
                    mb.box((q[0], (y0 + y1) / 2, q[1]), (pw / flutes * 0.45, y1 - y0 - 0.3, 0.03), mat, yaw)
                if pil.get('capital'):
                    cp = min(pp + 0.08, cap)
                    q = at(sv, cp / 2)
                    mb.box((q[0], y1 - 0.3, q[1]), (pw + 0.16, 0.34, cp), mat, yaw)
        spd = s.get('spandrels')
        if spd:
            mat = self.tinted(spd.get('material', s['wall']), spd.get('tint', s.get('wallTint')))
            spans = s.get('upperWindowSpans') or []
            head = s.get('upperHeadM', s['windowHeadM'])
            sill = s.get('upperSillM', s['windowSillM'])
            g_head = self.spec['floors']['groundM'] - s['storefront'].get('headDropM', 0.35)
            for k in range(1, self.storeys):
                y_top = self.levels[k] + sill - 0.12
                y_bot = (self.levels[k - 1] + min(head, self.storey_height(k - 1) - 0.2) + 0.12) if k > 1 else g_head + 0.15
                if y_top - y_bot < 0.3:
                    continue
                for f0, f1 in spans:
                    s0, s1 = sorted((self.f2s(f0, L, e), self.f2s(f1, L, e)))
                    c, w = (s0 + s1) / 2, s1 - s0 - 0.2
                    for yy in (y_bot + 0.04, y_top - 0.04):
                        p = at(c, 0.02)
                        mb.box((p[0], yy, p[1]), (w, 0.08, 0.04), mat, yaw)
                    for sv in (c - w / 2 + 0.04, c + w / 2 - 0.04):
                        p = at(sv, 0.02)
                        mb.box((p[0], (y_bot + y_top) / 2, p[1]), (0.08, y_top - y_bot, 0.04), mat, yaw)
                    # `squares`: a row of raised square panels inside the frame
                    n = int(spd.get('squares', 0))
                    if n:
                        side = min(y_top - y_bot - 0.3, (w - 0.2) / n - 0.15)
                        if side > 0.15:
                            # `squaresTint`: a darker field inside a raised
                            # ring reads as a recessed panel from the street
                            field = self.tinted(spd.get('material', s['wall']), spd['squaresTint']) if spd.get('squaresTint') else None
                            for i in range(n):
                                sv = c - w / 2 + 0.1 + (w - 0.2) * (i + 0.5) / n
                                p = at(sv, 0.04)
                                if field:
                                    ring = 0.07
                                    for dy in (-(side - ring) / 2, (side - ring) / 2):
                                        mb.box((p[0], (y_bot + y_top) / 2 + dy, p[1]), (side, ring, 0.06), mat, yaw, skip=('-z',))
                                    for dsv in (-(side - ring) / 2, (side - ring) / 2):
                                        q = at(sv + dsv, 0.04)
                                        mb.box((q[0], (y_bot + y_top) / 2, q[1]), (ring, side - 2 * ring, 0.06), mat, yaw, skip=('-z',))
                                    q = at(sv, 0.015)
                                    mb.box((q[0], (y_bot + y_top) / 2, q[1]), (side - 2 * ring, side - 2 * ring, 0.01), field, yaw, skip=('-z',))
                                else:
                                    mb.box((p[0], (y_bot + y_top) / 2, p[1]), (side, side, 0.06), mat, yaw, skip=('-z',))
        awn = s.get('awningSpans')
        if awn:
            # canvas awnings over any storefront spans ({from, to, depthM?})
            g_head = self.spec['floors']['groundM'] - s['storefront'].get('headDropM', 0.35)
            am = MeshBuilder(f'awning-spans-{e["index"]}')
            for span in awn:
                s0, s1 = sorted((self.f2s(span['from'], L, e), self.f2s(span['to'], L, e)))
                depth = span.get('depthM', 1.1)
                y = g_head + 0.15
                for k in range(4):
                    t = (k + 0.5) / 4
                    q = at((s0 + s1) / 2, depth * t)
                    # `riseM` / `dropM` per span (defaults unchanged): a steeper fabric slope
                    am.box((q[0], y + span.get('riseM', 0.5) - span.get('dropM', 0.46) * t, q[1]), (s1 - s0, 0.03, depth / 4 + 0.02), s['awning'], yaw)
                q = at((s0 + s1) / 2, depth)
                am.box((q[0], y - 0.08, q[1]), (s1 - s0, 0.24, 0.03), s['awning'], yaw)
            am.build(self.exterior)
        stripes = s.get('stripes')
        if stripes:
            mat = self.tinted(stripes.get('material', s['wall']), stripes.get('tint'))
            hh = stripes.get('heightM', 0.45)
            for k in range(int(stripes.get('fromStorey', 1)), self.storeys):
                for off in stripes.get('offsetsM', [0.0]):
                    y = self.levels[k] + off
                    if y + hh > self.height:
                        continue
                    p = at(L / 2, 0.015)
                    mb.box((p[0], y + hh / 2, p[1]), (L, hh, 0.03), mat, yaw)
        grid = s.get('frameGrid')
        if grid:
            cmat = self.tinted(grid.get('material', 'concrete-precast'), grid.get('tint'))
            cw, sh = grid.get('columnW', 0.4), grid.get('slabEdgeH', 0.35)
            pr = max(0.02, min(grid.get('projectionM', 0.12), cap))
            for c in self.grid_columns(grid, L, e):
                p = at(c, pr / 2)
                mb.box((p[0], self.height / 2, p[1]), (cw, self.height, pr + 0.01), cmat, yaw)
            for k in range(max(1, int(grid.get('fromStorey', 1))), self.storeys + 1):
                y = self.levels[k] if k < self.storeys else self.roof_level
                p = at(L / 2, pr / 2)
                mb.box((p[0], y, p[1]), (L, sh, pr + 0.01), cmat, yaw)
        # `plaques`: small flat panels on the wall or glass - an address plate,
        # an accessibility plaque, a leasing sign ({at, y, w, h, face})
        for pq in s.get('plaques') or []:
            p = at(self.f2s(pq['at'], L, e), 0.03)
            mb.box((p[0], pq['y'], p[1]), (pq.get('w', 0.3), pq.get('h', 0.3), 0.02), pq.get('face', 'signage'), yaw, skip=('-z',))
        _g1_wall_lamps(self, s, L, e, at, mb, nout)
        # `roofBlocks`: a penthouse / stair block on the roof near this face,
        # capped to the LiDAR height allowance ({at, widthM, depthM, material, tint})
        for rb in s.get('roofBlocks') or []:
            rt = self.roof_level
            hgt = max(0.3, self.height + 0.5 - rt)
            dep = rb.get('depthM', 3.0)
            sc, bw, sb = self.f2s(rb['at'], L, e), rb.get('widthM', 4.0), rb.get('setbackM', 1.0)
            # `hipRiseM`: a hipped (pitched) cap on the block - a corner tower -
            # taken out of the same height allowance, so walls + cap stay under it
            hr = min(rb.get('hipRiseM', 0.0), hgt - 0.3)
            wall_h = hgt - hr if hr > 0.05 else hgt
            p = at(sc, -(sb + dep / 2))
            mb.box((p[0], rt + wall_h / 2, p[1]), (bw, wall_h, dep), self.tinted(rb.get('material', s['wall']), rb.get('tint', s.get('wallTint'))), yaw,
                   skip=('bottom', 'top') if hr > 0.05 else ('bottom',))
            if hr > 0.05:
                y0, y1 = rt + wall_h, rt + wall_h + hr
                ov = rb.get('eaveM', 0.2)
                a_, b_ = at(sc - bw / 2 - ov, -(sb - ov)), at(sc + bw / 2 + ov, -(sb - ov))
                c_, d_ = at(sc + bw / 2 + ov, -(sb + dep + ov)), at(sc - bw / 2 - ov, -(sb + dep + ov))
                half = min(bw, dep) / 2 + ov
                if bw >= dep:
                    r0, r1 = at(sc - bw / 2 - ov + half, -(sb + dep / 2)), at(sc + bw / 2 + ov - half, -(sb + dep / 2))
                    faces = ((a_, b_, r1, r0), (c_, d_, r0, r1), (b_, c_, r1), (d_, a_, r0))
                else:
                    r0, r1 = at(sc, -(sb - ov + half)), at(sc, -(sb + dep + ov - half))
                    faces = ((b_, c_, r1, r0), (d_, a_, r0, r1), (a_, b_, r0), (c_, d_, r1))
                hmat = rb.get('hipMaterial', 'asphalt-roof@9aa0a8')
                up = B(0, 1, 0)
                for f in faces:
                    ring = [B(q[0], y0, q[1]) for q in f[:2]] + [B(q[0], y1, q[1]) for q in f[2:]]
                    mb.quad(ring, hmat, up)
                    mb.quad(list(reversed(ring)), hmat, B(0, -1, 0))   # eave soffit side
        coping = s.get('coping')
        if coping:
            mat = self.tinted(coping.get('material', s['trim']), coping.get('tint'))
            cp = min(coping.get('projectionM', 0.06), cap)
            ch = coping.get('heightM', 0.2)
            p = at(L / 2, cp / 2)
            mb.box((p[0], self.height - ch / 2, p[1]), (L, ch, cp + 0.02), mat, yaw)
        mb.build(self.exterior)

    def _edge_trim(self, e, a, b, trim):
        s = self.style
        if e['role'] == 'party':
            return
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        L = e['lengthM']
        yaw = box_yaw(d)

        def at(sv, off):
            return add(add(a, mul(d, sv)), mul(nout, off))

        mid = at(L / 2, 0)
        g = self.spec['floors']['groundM']
        if s.get('beltCourse') and self.storeys > 1:
            p = at(L / 2, 0.06)
            trim.box((p[0], g - 0.1, p[1]), (L, 0.32, 0.12), s['trim'], yaw)
        if s.get('cornice') and e['role'] in ('front', 'flank'):
            # The contract measures projection against the WORLD-axis footprint
            # AABB, so on a rotated building a perpendicular projection p costs
            # p * (|cos| + |sin|) of the heading. Cap it to stay inside 0.45 m.
            h = self.spec['frame']['heading']
            dw = (d[0] * math.cos(h) + d[1] * math.sin(h), -d[0] * math.sin(h) + d[1] * math.cos(h))
            cap = 0.43 / (abs(dw[0]) + abs(dw[1])) - 0.03
            proj = min(s['corniceProjectionM'], cap)
            top = self.height
            # stepped cornice: frieze, bed mould, corona, cap
            for y, hgt, off in ((top - 0.95, 0.4, 0.05), (top - 0.55, 0.14, proj * 0.45), (top - 0.41, 0.26, proj), (top - 0.15, 0.08, proj * 0.6)):
                p = at(L / 2, off / 2)
                trim.box((p[0], y + hgt / 2, p[1]), (L, hgt, off + 0.02), s['trim'], yaw)
            # brackets under the corona
            count = max(2, int(L / 1.1))
            for i in range(count + 1):
                p = at(0.2 + (L - 0.4) * i / count, proj * 0.4)
                trim.box((p[0], top - 0.62, p[1]), (0.14, 0.34, proj * 0.8), s['trim'], yaw)
        fascia = s.get('fascia')
        if fascia and e['role'] == 'front':
            h = self.spec['frame']['heading']
            dw = (d[0] * math.cos(h) + d[1] * math.sin(h), -d[0] * math.sin(h) + d[1] * math.cos(h))
            proj = min(fascia.get('projectionM', 0.25), 0.43 / (abs(dw[0]) + abs(dw[1])) - 0.05)
            fh = fascia.get('heightM', 1.8)
            p = at(L / 2, proj / 2)
            fb = MeshBuilder('fascia')
            fb.box((p[0], self.height - fh / 2, p[1]), (L, fh, proj + 0.02), self.tinted(fascia.get('material', 'painted-steel-mid'), fascia.get('tint')), yaw)
            fb.build(self.exterior)
        if e['role'] == 'front' and s['storefront'].get('awning') and getattr(self, 'front_edge', None):
            fe = self.front_edge
            if fe['mainBay']:
                b0, b1 = fe['mainBay']
                w = b1 - b0 + 0.6
                depth = 1.2
                y = fe['head'] + 0.15
                p = at((b0 + b1) / 2, depth / 2)
                # sloped canvas: two boxes approximating the slope + valance
                awn = MeshBuilder('awning')
                for k in range(4):
                    t = (k + 0.5) / 4
                    q = at((b0 + b1) / 2, depth * t)
                    awn.box((q[0], y + 0.55 - 0.5 * t, q[1]), (w, 0.03, depth / 4 + 0.02), s['awning'], yaw)
                q = at((b0 + b1) / 2, depth)
                awn.box((q[0], y - 0.08, q[1]), (w, 0.22, 0.03), s['awning'], yaw)
                awn.build(self.exterior)
                # sign band above the storefront (blank, no business names)
                p = at(L / 2, 0.05)
                trim.box((p[0], fe['head'] + 0.45, p[1]), (L * 0.7, 0.45, 0.06), 'signage', yaw)

    # ---------------------------------------------------------- interior

    def build_interior(self):
        x0, x1, z0, z1 = self.rect
        st = self.stair
        slabs = MeshBuilder('slabs')
        stair_mb = MeshBuilder('stair')
        walls = MeshBuilder('partitions')
        self.furn = {}
        self.furn_objects = []
        # ground floor finish
        gpts = [B(p[0], 0.02, p[1]) for p in self.inner]
        ground_mat = {'restaurant': 'int-tile-floor', 'shop': 'int-wood-floor', 'vacant': 'int-concrete-floor'}.get(self.ground_programme(), 'int-wood-floor')
        slabs.poly(gpts, ground_mat, B(0, 1, 0))
        opening = (st['x0'], st['x1'], st['z0'], st['z1'])
        # upper slabs with the stair opening, plus the roof slab (no opening)
        for k in range(1, self.storeys + 1):
            top = self.levels[k] if k < self.storeys else self.roof_level
            hole = opening if k < self.storeys else None
            self._slab(slabs, top, hole, 'int-wood-floor' if k < self.storeys else 'int-plaster')
            if k < self.storeys:
                for piece in rect_minus((x0, x1, z0, z1), hole):
                    px0, px1, pz0, pz1 = piece
                    self.physics['solids'].append({
                        'id': f'slab-{k}-{len(self.physics["solids"])}', 'walkable': True,
                        'center': {'x': (px0 + px1) / 2, 'y': top - SLAB_T / 2, 'z': (pz0 + pz1) / 2},
                        'size': {'x': px1 - px0, 'y': SLAB_T, 'z': pz1 - pz0}})
        # stairs every storey
        for k in range(self.storeys - 1):
            self._stair_flight(stair_mb, k)
        # programmes
        self._ground(walls)
        for k in range(1, self.storeys):
            self._upper(walls, k)
        self.stats['interior'] = {}
        for m in (slabs, stair_mb, walls):
            obj = m.build(self.interior)
            if obj:
                self.stats['interior'][m.name] = len(m.faces)
        self._emit_furniture()

    def _slab(self, mb, top, hole, underside):
        """One floor slab: the inset footprint minus the stair opening. Each
        rectangle of the (bounding box - opening) split is clipped to the inset
        polygon, so an irregular footprint's slab follows its walls instead of
        filling the bounding box (a jagged 11-sided slab overhung by 1.2 m)."""
        bottom = top - SLAB_T
        top_mat = 'int-wood-floor' if underside != 'int-plaster' else 'int-plaster'
        pieces = rect_minus((self.ix0, self.ix1, self.iz0, self.iz1), hole) if hole else [(self.ix0, self.ix1, self.iz0, self.iz1)]
        for piece in pieces:
            ring = clip_ring_to_rect(self.inner, piece)
            if len(ring) < 3:
                continue
            mb.poly([B(p[0], top, p[1]) for p in ring], top_mat, B(0, 1, 0))
            mb.poly([B(p[0], bottom, p[1]) for p in ring], 'int-ceiling', B(0, -1, 0))
            cx = sum(p[0] for p in ring) / len(ring)
            cz = sum(p[1] for p in ring) / len(ring)
            for i in range(len(ring)):
                p, q = ring[i], ring[(i + 1) % len(ring)]
                if length(sub(q, p)) < 1e-4:
                    continue
                mid = lerp2(p, q, 0.5)
                mb.quad([B(p[0], bottom, p[1]), B(q[0], bottom, q[1]), B(q[0], top, q[1]), B(p[0], top, p[1])],
                        'int-plaster', B(mid[0] - cx, 0, mid[1] - cz))

    def _stair_flight(self, mb, k):
        st = self.stair
        base = self.levels[k]
        rise_total = self.storey_height(k)
        n = math.ceil(rise_total / RISER)
        riser = rise_total / n
        n1 = n // 2
        n2 = n - n1
        land_y = base + n1 * riser
        xa0, xa1 = st['x0'], st['x0'] + FLIGHT_W
        xb0, xb1 = st['x1'] - FLIGHT_W, st['x1']
        if st['side'] > 0:
            xa0, xa1, xb0, xb1 = xb0, xb1, xa0, xa1
        zl = st['z0'] + LANDING_D  # landing front edge
        # flight 1 climbs toward the rear (-z) and ends at the landing
        z_start1 = zl + n1 * TREAD
        for i in range(n1):
            zc = z_start1 - (i + 0.5) * TREAD
            y_top = base + (i + 1) * riser
            mb.box(((xa0 + xa1) / 2, (base + y_top) / 2 if i == 0 else y_top - riser / 2 - 0.06, zc),
                   (FLIGHT_W, riser if i == 0 else riser + 0.12, TREAD), 'int-stair')
        # landing
        mb.box(((st['x0'] + st['x1']) / 2, land_y - 0.1, (st['z0'] + zl) / 2), (st['x1'] - st['x0'], 0.2, LANDING_D), 'int-stair',
               mats={'top': 'int-stair', 'bottom': 'int-ceiling'})
        # flight 2 climbs toward the front (+z) from the landing to the next floor
        for i in range(n2):
            zc = zl + (i + 0.5) * TREAD
            y_top = land_y + (i + 1) * riser
            mb.box(((xb0 + xb1) / 2, y_top - riser / 2 - 0.06, zc), (FLIGHT_W, riser + 0.12, TREAD), 'int-stair')
        # stringers / soffits: a sloped-ish plate under each flight approximated by the treads' 0.12 skirt
        # handrails
        for (xa, xb, z_from, z_to, y_from, y_to) in (
                (xa0, xa1, z_start1, zl, base, land_y), (xb0, xb1, zl, zl + n2 * TREAD, land_y, land_y + n2 * riser)):
            xr = (xa + xb) / 2 + (FLIGHT_W / 2 - 0.05) * (1 if xb > xa and (xa == min(st['x0'], st['x1']) + 0) is False else 1)
            steps = 6
            for j in range(steps):
                t0, t1 = j / steps, (j + 1) / steps
                zz = z_from + (z_to - z_from) * (t0 + t1) / 2
                yy = y_from + (y_to - y_from) * (t0 + t1) / 2 + 0.9
                mb.box((xr, yy, zz), (0.05, 0.05, abs(z_to - z_from) / steps + 0.01), 'int-dark-metal')
        # guard along the open side of the upper opening
        top = self.levels[k + 1]
        gx = st['x1'] if st['side'] < 0 else st['x0']
        mb.box((gx, top + 0.5, (st['z0'] + st['z1']) / 2), (0.05, 1.0, st['z1'] - st['z0']), 'int-dark-metal')
        self.physics['solids'].append({'id': f'guard-{k + 1}', 'center': {'x': gx, 'y': top + 0.5, 'z': (st['z0'] + st['z1']) / 2},
                                       'size': {'x': 0.08, 'y': 1.0, 'z': st['z1'] - st['z0']}})
        # physics: two flights and the landing
        self.physics['steps'].append({'id': f'flight-{k}-a', 'from': {'x': (xa0 + xa1) / 2, 'z': z_start1}, 'to': {'x': (xa0 + xa1) / 2, 'z': zl},
                                      'width': FLIGHT_W, 'riseM': riser, 'runM': TREAD, 'count': n1, 'base': base})
        self.physics['solids'].append({'id': f'landing-{k}', 'walkable': True,
                                       'center': {'x': (st['x0'] + st['x1']) / 2, 'y': land_y - 0.1, 'z': (st['z0'] + zl) / 2},
                                       'size': {'x': st['x1'] - st['x0'], 'y': 0.2, 'z': LANDING_D}})
        self.physics['steps'].append({'id': f'flight-{k}-b', 'from': {'x': (xb0 + xb1) / 2, 'z': zl}, 'to': {'x': (xb0 + xb1) / 2, 'z': zl + 1},
                                      'width': FLIGHT_W, 'riseM': riser, 'runM': TREAD, 'count': n2, 'base': land_y})

    # -- furniture: real CC0 pieces (fine tier, per storey) + cheap boxes (coarse tier)
    #
    # Every placement belongs to a storey. The FINE tier (real models, soft
    # furnishings, warm lights) exports one GLB per storey and the runtime draws
    # only the storey the player is on; the COARSE tier (one box per large piece)
    # rides in interior.glb so every room still reads furnished through the
    # windows. See blender/runtime.js and the budget check in build.mjs.

    def storey_of(self, level):
        k = 0
        for i, lv in enumerate(self.levels):
            if level >= lv - 0.05:
                k = i
        return k

    def piece_size(self, kind, fit=None):
        rec = LIBRARY['pieces'].get(kind)
        size = list(rec['sizeM']) if rec else list(FALLBACK_SIZE.get(kind, (0.5, 0.5, 0.5)))
        if fit:
            size = [f if f else s for f, s in zip(fit, size)]
        return size

    def piece(self, kind, x, z, yaw=0.0, level=0.0, y=None, solid=True, fit=None, coarse=None):
        """Place one piece. y: absolute base height (on a table, hanging); fit: target size [x, h, z]
        (None entries keep the piece's own). coarse: material for its coarse box, or False for none."""
        base = level if y is None else y
        size = self.piece_size(kind, fit)
        k = self.storey_of(level)
        self.furn_objects.append({'kind': kind, 'x': x, 'z': z, 'yaw': yaw, 'y': base, 'fit': fit, 'storey': k})
        if solid:
            self.physics['solids'].append({
                'id': f'f-{kind}-{len(self.furn_objects)}',
                'center': {'x': x, 'y': base + size[1] / 2, 'z': z},
                'size': {'x': max(0.1, size[0]), 'y': max(0.1, size[1]), 'z': max(0.1, size[2])}, 'yaw': yaw})
        mat = COARSE_MATERIAL.get(kind) if coarse is None else coarse
        if mat:
            self.coarse_for(k).box((x, base + size[1] * 0.45, z), (size[0] * 0.95, size[1] * 0.9, size[2] * 0.95), mat, yaw)

    def coarse_for(self, k):
        if k not in self.coarse:
            self.coarse[k] = MeshBuilder(f'coarse-s{k}')
        return self.coarse[k]

    def soft_for(self, k):
        if k not in self.soft:
            self.soft[k] = MeshBuilder(f'soft-s{k}')
        return self.soft[k]

    def add_light(self, x, y, z, k, intensity=22.0, distance=9.0, priority=1):
        self.lights.append({'x': x, 'y': y, 'z': z, 'storey': k, 'intensity': intensity,
                            'distance': distance, 'color': [1.0, 0.74, 0.5], 'priority': priority})

    def _source_mesh(self, kind, cache):
        if kind in cache:
            return cache[kind]
        data = None
        if kind in LIBRARY['pieces']:
            before = set(bpy.data.objects)
            bpy.ops.import_scene.gltf(filepath=os.path.join(LIBRARY['dir'], LIBRARY['pieces'][kind]['file']))
            new = [o for o in bpy.data.objects if o not in before]
            mesh_obj = next((o for o in new if o.type == 'MESH'), None)
            data = mesh_obj.data if mesh_obj else None
            for o in new:
                for col in list(o.users_collection):
                    col.objects.unlink(o)
        elif FALLBACK.get(kind):
            mb = MeshBuilder(f'furn-{kind}')
            FURNITURE_BUILDERS[FALLBACK[kind]](mb)
            obj = mb.build(self.interior)
            data = obj.data
            self.interior.objects.unlink(obj)
        cache[kind] = data
        return data

    def _emit_furniture(self):
        cache = {}
        counts = {}
        self.library_triangles = {}
        for item in self.furn_objects:
            col = self.fine_collection(item['storey'])
            kind = item['kind']
            n = counts.get(kind, 0)
            if kind in LIBRARY['pieces'] and LIBRARY['by_reference']:
                # A reference, not a copy: an empty named lib:<kind>.<n> whose
                # transform the runtime instances the shared library piece at
                # (blender/runtime.js). The piece's triangles still count
                # against this storey's budget.
                counts[kind] = n + 1
                obj = bpy.data.objects.new(f'lib:{kind}.{n:03d}', None)
                obj.empty_display_size = 0.1
                obj.location = B(item['x'], item['y'], item['z'])
                obj.rotation_euler = (0, 0, item['yaw'])
                if item['fit']:
                    own = self.piece_size(kind)
                    fx, fh, fz = (f / o if f and o else 1.0 for f, o in zip(item['fit'], own))
                    obj.scale = (fx, fz, fh)
                col.objects.link(obj)
                self.library_triangles[col.name] = self.library_triangles.get(col.name, 0) + LIBRARY['pieces'][kind]['triangles']
                continue
            data = self._source_mesh(kind, cache)
            if data is None:
                continue
            counts[kind] = n + 1
            obj = bpy.data.objects.new(f'{item["kind"]}.{n:03d}', data)
            obj.location = B(item['x'], item['y'], item['z'])
            obj.rotation_euler = (0, 0, item['yaw'])
            if item['fit']:
                # a box stand-in has no library size: measure the mesh it is
                # (a default 0.5 m size here stretched a stand-in bed to 8 m)
                own = self.piece_size(item['kind']) if item['kind'] in LIBRARY['pieces'] else mesh_size(data)
                fx, fh, fz = (f / o if f and o else 1.0 for f, o in zip(item['fit'], own))
                obj.scale = (fx, fz, fh)
            col.objects.link(obj)
        for k, mb in self.soft.items():
            mb.build(self.fine_collection(k))
        for k, mb in self.coarse.items():
            mb.build(self.interior)
        self.stats['furniture'] = counts

    def fine_collection(self, k):
        name = f'fine-s{k}'
        if name not in bpy.data.collections:
            col = bpy.data.collections.new(name)
            bpy.context.scene.collection.children.link(col)
        return bpy.data.collections[name]

    # -- soft furnishings (procedural: nothing in the CC0 set covers them)

    def rug(self, x, z, w, d, yaw, level, k, inner='int-rug', border='int-rug-border'):
        mb = self.soft_for(k)
        mb.box((x, level + 0.006, z), (w, 0.012, d), border, yaw)
        mb.box((x, level + 0.009, z), (w - 0.16, 0.012, d - 0.16), inner, yaw)

    def bedding(self, x, z, yaw, level, k, width, length):
        mb = self.soft_for(k)
        c, s = math.cos(yaw), math.sin(yaw)

        def at(lx, lz):
            return (x + lx * c + lz * s, z - lx * s + lz * c)

        m = at(0, 0.02)
        mb.box((m[0], level + 0.44, m[1]), (width - 0.1, 0.22, length - 0.12), 'int-duvet', yaw)
        dv = at(0, length * 0.12)
        mb.box((dv[0], level + 0.575, dv[1]), (width + 0.04, 0.07, length * 0.72), 'int-duvet', yaw)
        bl = at(0, length * 0.36)
        mb.box((bl[0], level + 0.62, bl[1]), (width + 0.08, 0.035, 0.42), 'int-blanket', yaw)
        for side in (-0.26, 0.26):
            p = at(side * width, -length / 2 + 0.3)
            mb.box((p[0], level + 0.62, p[1]), (width * 0.42, 0.14, 0.36), 'int-duvet', yaw)

    def curtains(self, slot, level, k):
        mb = self.soft_for(k)
        a, b, nin = slot['a'], slot['b'], slot['nin']
        d = norm(sub(b, a))
        top = slot['head'] + 0.16
        bottom = level + 0.04
        off = 0.12
        # rod, running past the window unless that would cross a nearby corner
        ext = 0.25
        ra, rb = sub(add(a, mul(nin, off)), mul(d, ext)), add(add(b, mul(nin, off)), mul(d, ext))
        if not (self._inside(ra, self.poly) and self._inside(rb, self.poly)):
            ext = 0.0
            ra, rb = add(a, mul(nin, off)), add(b, mul(nin, off))
        mid = lerp2(ra, rb, 0.5)
        mb.box((mid[0], top + 0.03, mid[1]), (length(sub(rb, ra)), 0.03, 0.03), 'int-dark-metal', box_yaw(d))
        # two pleated panels drawn aside, floor length
        for start, sign in ((a, 1), (b, -1)):
            p0 = sub(add(start, mul(nin, off)), mul(d, sign * min(0.22, ext)))
            folds = 7
            pts = []
            for i in range(folds + 1):
                t = i / folds
                q = add(p0, mul(d, sign * 0.5 * t))
                q = add(q, mul(nin, 0.05 * (i % 2)))
                pts.append(q)
            for i in range(folds):
                q0, q1 = pts[i], pts[i + 1]
                mb.quad([B(q0[0], bottom, q0[1]), B(q1[0], bottom, q1[1]), B(q1[0], top, q1[1]), B(q0[0], top, q0[1])],
                        'int-curtain', B(nin[0], 0, nin[1]))

    def bedside_lamp(self, x, z, y, k):
        mb = self.soft_for(k)
        mb.box((x, y + 0.02, z), (0.14, 0.04, 0.14), 'int-ceramic')
        mb.box((x, y + 0.18, z), (0.03, 0.3, 0.03), 'int-dark-metal')
        mb.box((x, y + 0.38, z), (0.26, 0.18, 0.26), 'int-light')

    def book_rows(self, x, z, yaw, level, k, width=0.9, shelves=(0.45, 0.85, 1.25, 1.65)):
        mb = self.soft_for(k)
        rng = self.rng
        c, s = math.cos(yaw), math.sin(yaw)
        for sy in shelves:
            u = -width / 2 + 0.04
            while u < width / 2 - 0.1:
                bw = rng.uniform(0.025, 0.06)
                bh = rng.uniform(0.18, 0.28)
                mat = rng.choice(('int-fabric-red', 'int-fabric-blue', 'int-fabric-green', 'int-leather', 'int-duvet'))
                px, pz = x + u * c, z - u * s
                mb.box((px, level + sy + bh / 2, pz), (bw, bh, 0.2), mat, yaw)
                u += bw + rng.uniform(0.002, 0.012)
                if rng.random() < 0.12:
                    u += rng.uniform(0.1, 0.25)

    def wall_frame(self, x, z, facing, level, k, kind=None, height=1.5):
        kind = kind or self.rng.choice(('frame-a', 'frame-b', 'frame-c'))
        yaw = math.atan2(facing[0], facing[1])
        self.piece(kind, x + facing[0] * 0.02, z + facing[1] * 0.02, yaw, level, y=level + height - self.piece_size(kind)[1] / 2,
                   solid=False, coarse=False)

    def hang(self, kind, x, z, top, level, drop=None):
        h = self.piece_size(kind)[1]
        self.piece(kind, x, z, 0.0, level, y=top - (drop or h), solid=False, coarse=False)

    # -- ground programme

    def _ground(self, walls):
        x0, x1, z0, z1 = self.rect
        st = self.stair
        top = self.storey_top(0)
        prog = self.ground_programme()
        # partition along the lobby (from front wall to stair core front)
        wx = st['x0'] + 1.45 if st['side'] < 0 else st['x1'] - 1.45
        door_x = getattr(self, 'stair_door_x', None)
        if door_x is not None:
            # the lobby runs from the street door to the core: widen it to take the door
            wx = max(wx, door_x + 0.7) if st['side'] < 0 else min(wx, door_x - 0.7)
        self.physics['solids'] += partition(walls, (wx, z1), (wx, st['z1']), 0.0, top, [])
        self.physics['solids'] += partition(walls, (st['x0'], st['z1']), (st['x1'], st['z1']), 0.0, top,
                                            [0.7 if st['side'] < 0 else (st['x1'] - st['x0']) - 0.7])
        if st['side'] < 0 and wx > st['x1'] + 0.2:
            self.physics['solids'] += partition(walls, (st['x1'], st['z1']), (wx, st['z1']), 0.0, top, [])
        elif st['side'] > 0 and wx < st['x0'] - 0.2:
            self.physics['solids'] += partition(walls, (wx, st['z1']), (st['x0'], st['z1']), 0.0, top, [])
        # back-of-house at the rear, beside the stair core
        kx0 = st['x1'] + 0.1 if st['side'] < 0 else x0
        kx1 = x1 if st['side'] < 0 else st['x0'] - 0.1
        kz = z0 + min(6.0, (z1 - z0) * 0.3)
        self.physics['solids'] += partition(walls, (kx0, kz), (kx1, kz), 0.0, top, [(kx1 - kx0) * 0.3])
        room_x0 = wx + PART_T if st['side'] < 0 else x0
        room_x1 = x1 if st['side'] < 0 else wx - PART_T
        lobby_x0, lobby_x1 = (min(st['x0'], wx), max(st['x0'], wx))
        # lobby: runner, a sconce pair, one ceiling lamp
        lx = (lobby_x0 + lobby_x1) / 2
        self.rug(lx, (st['z1'] + z1) / 2, min(1.0, lobby_x1 - lobby_x0 - 0.4), max(1.0, z1 - st['z1'] - 1.0), 0.0, 0.0, 0)
        self.hang('ceiling-lamp', lx, (st['z1'] + z1) / 2, top, 0.0)
        side_wall = wx - 0.07 if st['side'] < 0 else wx + 0.07
        facing = (-1, 0) if st['side'] < 0 else (1, 0)
        for zz in frange(st['z1'] + 1.2, z1 - 1.2, 3.5):
            self.piece('wall-sconce', side_wall, zz, math.atan2(*facing), 0.0, y=1.75, solid=False, coarse=False)
        # back of house: a steel kitchen line behind a restaurant; a bare
        # store room behind anything else (a vacant unit has no kitchen)
        if prog == 'restaurant':
            for xc in frange(kx0 + 0.9, kx1 - 0.6, 1.5):
                self.piece('kitchen-counter', xc, z0 + 0.4, 0.0, 0.0)
            self.piece('range', (kx0 + kx1) / 2, kz - 0.5, math.pi, 0.0)
            self.piece('fridge', kx1 - 0.5, kz - 0.5, math.pi, 0.0)
        elif prog != 'vacant':
            self.piece('shelving', kx0 + 0.6, z0 + 0.4, 0.0, 0.0)
            self.piece('cardboard-box', kx0 + 1.6, z0 + 0.5, 0.3, 0.0)
        self.hang('ceiling-lamp', (kx0 + kx1) / 2, (z0 + kz) / 2, top, 0.0)
        if prog == 'restaurant':
            self._cafe(room_x0, room_x1, kz, z1, top, wx)
        elif prog == 'vacant':
            self._vacant(room_x0, room_x1, kz, z1, top)
        else:
            for xc in frange(room_x0 + 1.5, room_x1 - 1.2, 2.4):
                for zc in frange(kz + 1.8, z1 - 3.0, 3.2):
                    self.piece('shelf', xc, zc, math.pi / 2, 0.0)
            self.piece('shop-counter', room_x0 + 1.4, z1 - 2.0, 0.0, 0.0)
            self.add_light((room_x0 + room_x1) / 2, top - 0.4, (kz + z1) / 2, 0, priority=3)

    def _cafe(self, rx0, rx1, kz, z1, top, wx):
        st = self.stair
        right = st['side'] < 0  # bar on the side away from the lobby
        bar_x = rx1 - 0.95 if right else rx0 + 0.95
        wall_x = rx1 - 0.25 if right else rx0 + 0.25
        bz0 = kz + 1.2
        bz1 = min(z1 - 3.5, bz0 + 5.0)
        towards_room = -1 if right else 1
        # bar counter, back bar, stools with elbow room
        for zc in frange(bz0 + 0.6, bz1 - 0.3, 1.2):
            self.piece('bar-counter', bar_x, zc, math.pi / 2 if right else -math.pi / 2, 0.0)
        self.piece('back-bar', wall_x, (bz0 + bz1) / 2, math.pi / 2 if right else -math.pi / 2, 0.0)
        for i, zc in enumerate(frange(bz0 + 0.7, bz1 - 0.5, 0.95)):
            self.piece('bar-stool', bar_x + towards_room * 0.75, zc, math.atan2(-towards_room, 0), 0.0, solid=False, coarse=False)
        for j, zc in enumerate(frange(bz0 + 0.9, bz1 - 0.9, 1.6)):
            self.piece('wine-bottles', wall_x - towards_room * 0.02 + towards_room * 0.05, zc, math.pi / 2 if right else -math.pi / 2, 0.0,
                       y=1.25 + 0.4 * (j % 2), solid=False, coarse=False)
        for zc in frange(bz0 + 0.8, bz1 - 0.8, 1.9):
            self.hang('pendant', bar_x, zc, top, 0.0)
        # dining: a calm grid of two-tops, each with its own pendant
        tx0 = rx0 + 1.1 if right else bar_x + 1.9
        tx1 = bar_x - 1.9 if right else rx1 - 1.1
        door_x = getattr(self, 'main_door_x', None)
        tables = []
        for zc in frange(kz + 2.2, z1 - 2.3, 2.7):
            for xc in frange(tx0 + 0.4, tx1 - 0.4, 2.6):
                if door_x is not None and abs(xc - door_x) < 1.4 and zc > z1 - 4.5:
                    continue  # the entrance aisle stays clear
                tables.append((xc, zc))
        tables = tables[:9]
        for i, (xc, zc) in enumerate(tables):
            self.piece('cafe-table', xc, zc, 0.0, 0.0)
            self.piece('cafe-chair', xc - 0.62, zc, math.pi / 2, 0.0, solid=False, coarse=False)
            self.piece('cafe-chair', xc + 0.62, zc, -math.pi / 2, 0.0, solid=False, coarse=False)
            if i % 3 == 1:
                self.piece('cafe-chair', xc, zc + 0.62, math.pi, 0.0, solid=False, coarse=False)
            if i % 2 == 0:
                self.piece('plant-small', xc + 0.12, zc - 0.1, 0.0, 0.0, y=0.75, solid=False, coarse=False)
            self.hang('pendant', xc, zc, top, 0.0, drop=1.2)
        # greenery and pictures, not everywhere
        self.piece('plant-medium', rx0 + 0.5 if right else rx1 - 0.5, z1 - 0.6, 0.0, 0.0, solid=False, coarse=False)
        self.piece('plant-medium', rx0 + 0.5 if right else rx1 - 0.5, kz + 0.6, 0.0, 0.0, solid=False, coarse=False)
        art_x = rx0 + 0.02 if right else rx1 - 0.02
        facing = (1, 0) if right else (-1, 0)
        for zz in frange(kz + 2.4, z1 - 2.5, 3.2):
            self.wall_frame(art_x, zz, facing, 0.0, 0, height=1.65)
        self.piece('host-stand', (rx0 + 1.0) if right else (rx1 - 1.0), z1 - 1.3, 0.0, 0.0)
        # three warm lights over the dining room
        mid = (tx0 + tx1) / 2
        for zz in (kz + (z1 - kz) * 0.25, kz + (z1 - kz) * 0.62):
            self.add_light(mid, top - 0.5, zz, 0, intensity=28.0, distance=11.0, priority=5)
        self.add_light(bar_x, top - 0.5, (bz0 + bz1) / 2, 0, intensity=20.0, distance=8.0, priority=4)

    def _vacant(self, rx0, rx1, kz, z1, top):
        # between tenants: bare concrete, a few left-behind boxes, one work light
        spots = ((0.18, 0.3, 'cardboard-box', 0.2), (0.24, 0.34, 'cardboard-box', 1.1), (0.21, 0.4, 'crate', 0.4),
                 (0.72, 0.78, 'cardboard-box', 2.3))
        for fx, fz, kind, yaw in spots:
            self.piece(kind, rx0 + (rx1 - rx0) * fx, kz + (z1 - kz) * fz, yaw, 0.0)
        self.hang('ceiling-lamp', (rx0 + rx1) / 2, (kz + z1) / 2, top, 0.0)
        self.add_light((rx0 + rx1) / 2, top - 0.6, (kz + z1) / 2, 0, intensity=10.0, distance=9.0, priority=2)

    # -- upper programme: corridor + rooms, every floor furnished

    def _upper(self, walls, k):
        x0, x1, z0, z1 = self.rect
        st = self.stair
        base = self.levels[k]
        top = self.storey_top(k)
        prog = self.upper_programme()
        cxm = (x0 + x1) / 2
        if st['side'] < 0:
            cxm = max(cxm, st['x1'] + CORRIDOR_W / 2 + 0.1)
        else:
            cxm = min(cxm, st['x0'] - CORRIDOR_W / 2 - 0.1)
        cl, cr = cxm - CORRIDOR_W / 2, cxm + CORRIDOR_W / 2
        pz0, pz1 = st['z1'], st['z1'] + 1.4
        if prog == 'office':
            self._office(x0, x1, z0, z1, pz1, base, top, k)
            return
        # corridor: a runner, sconces, two ceiling lamps
        self.rug(cxm, (z0 + z1) / 2, CORRIDOR_W - 0.5, z1 - z0 - 1.0, 0.0, base, k)
        for zz in (z0 + (z1 - z0) * 0.3, z0 + (z1 - z0) * 0.7):
            self.hang('ceiling-lamp', cxm, zz, top, base)
        for zz in frange(z0 + 2.0, z1 - 1.5, 4.5):
            self.piece('wall-sconce', cl + 0.07, zz, math.pi / 2, base, y=base + 1.75, solid=False, coarse=False)
        self.add_light(cxm, top - 0.3, (z0 + z1) / 2, k, intensity=14.0, distance=10.0, priority=2)
        left = (x0, cl) if st['side'] < 0 else (cr, x1)
        right = (cr, x1) if st['side'] < 0 else (x0, cl)
        strips = [(left, pz1, z1, 'left'), (right, z0, z1, 'right')]
        best_room = None
        for (sx0, sx1), sz0, sz1, tag in strips:
            span = sz1 - sz0
            if span < 1.5 or sx1 - sx0 < 1.5:
                continue  # the stair core and landing leave no room on this side
            count = max(1, int(span // 4.4))  # fewer, roomier rooms
            depth = span / count
            wall_x = sx1 if (tag == 'left') == (st['side'] < 0) else sx0
            doors = []
            for i in range(count):
                rz0 = sz0 + i * depth
                rz1 = rz0 + depth
                doors.append(rz0 + 0.8)
                if i > 0:
                    self.physics['solids'] += partition(walls, (sx0, rz0), (sx1, rz0), base, top, [])
                window = self._has_window(sx0, sx1, rz0, rz1)
                self._room((sx0, sx1, rz0, rz1), base, top, k, wall_x, window)
                if window and best_room is None:
                    best_room = ((sx0 + sx1) / 2, (rz0 + rz1) / 2)
            self.physics['solids'] += partition(walls, (wall_x, sz0), (wall_x, sz1), base, top, [d - sz0 for d in doors])
        arrival_x = (st['x1'] - FLIGHT_W / 2) if st['side'] < 0 else (st['x0'] + FLIGHT_W / 2)
        self.physics['solids'] += partition(walls, (st['x0'], st['z1']), (st['x1'], st['z1']), base, top, [arrival_x - st['x0']])
        lx0, lx1 = left
        if pz1 < z1:
            self.physics['solids'] += partition(walls, (lx0, pz1), (lx1, pz1), base, top, [])
        if best_room:
            self.add_light(best_room[0], top - 0.4, best_room[1], k, intensity=16.0, distance=7.0, priority=1)

    def _has_window(self, rx0, rx1, rz0, rz1):
        for e in self.edges:
            if e['role'] == 'party':
                continue
            a, b = v2(e['a']), v2(e['b'])
            for px, pz in ((rx0, (rz0 + rz1) / 2), (rx1, (rz0 + rz1) / 2), ((rx0 + rx1) / 2, rz0), ((rx0 + rx1) / 2, rz1)):
                if seg_distance((px, pz), a, b) < WALL_T + 0.4:
                    return (px, pz)
        return None

    def _room(self, r, base, top, k, door_wall_x, window):
        """A hotel room: made bed against the back partition, nightstands and a lamp,
        a dresser opposite, an armchair corner by the window, a writing desk,
        a rug, a picture over the bed, curtains, one ceiling lamp. No more."""
        rx0, rx1, rz0, rz1 = r
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        w = rx1 - rx0
        toward_door = 1 if door_wall_x > cx else -1
        far_x = rx0 + 0.05 if toward_door > 0 else rx1 - 0.05
        # bed: headboard on the rz0 partition, centred in the room's far half.
        # A room too shallow for that (a strip in front of the stair core) takes
        # the bed side-on along the rz0 partition, head away from the door.
        # A room too narrow for the bed either way (a sliver strip) gets none.
        bed_w, bed_l = 1.5, 2.05
        fw, fl = bed_w + 0.1, bed_l + 0.05  # the frame's footprint
        m = 0.1  # clearance kept from every wall of the room
        if rz1 - rz0 >= bed_l + 0.06 + 0.9 and w >= fw + 2 * m:
            yaw, bsx, bsz = 0.0, fw, fl
        elif rz1 - rz0 >= fw + 0.06 + m and w >= fl + 2 * m:
            yaw, bsx, bsz = toward_door * math.pi / 2, fl, fw
        else:
            return
        bx = cx - toward_door * min(0.6, w * 0.1)
        bx = min(max(bx, rx0 + m + bsx / 2), rx1 - m - bsx / 2)
        bz = rz0 + 0.06 + (bed_w if yaw else bed_l) / 2
        if not yaw:
            stands = [(bx + side * (bed_w / 2 + 0.33), rz0 + 0.3) for side in (-1, 1)]
        else:
            stands = [(bx - toward_door * (bed_l / 2 + 0.33), rz0 + 0.3)]
        self.piece('bed-frame', bx, bz, yaw, base, fit=(fw, None, fl))
        self.bedding(bx, bz, yaw, base, k, bed_w, bed_l)
        stands = [s for s in stands if rx0 + m + 0.25 <= s[0] <= rx1 - m - 0.25]
        for nx, nz in stands:
            self.piece('nightstand', nx, nz, 0.0, base)
        if stands:
            self.bedside_lamp(stands[0][0], stands[0][1], base + 0.62, k)
        self.wall_frame(bx, rz0 + 0.07, (0, 1), base, k, height=1.75)
        # rug at the bed's open side, cut to the room less the clearance
        half = (bed_l if yaw else bed_w) / 2 + 0.4
        ux0, ux1 = max(rx0 + m, bx - half), min(rx1 - m, bx + half)
        uz0, uz1 = max(rz0 + m, bz + 0.55 - 0.85), min(rz1 - m, bz + 0.55 + 0.85)
        if yaw:
            uz0, uz1 = max(rz0 + m, bz + bsz / 2 - 0.5), min(rz1 - m, bz + bsz / 2 + 0.9)
        if ux1 - ux0 >= 0.8 and uz1 - uz0 >= 0.6:
            self.rug((ux0 + ux1) / 2, (uz0 + uz1) / 2, ux1 - ux0, uz1 - uz0, 0.0, base, k)
        # dresser on the opposite partition (past the bed's foot when the bed is
        # side-on), a small plant on it; left out when the room has no space for it
        dsize = self.piece_size('dresser') if 'dresser' in LIBRARY['pieces'] else FURNITURE_FOOTPRINT[FALLBACK['dresser']]
        dw, _, dd = dsize
        dx = bx if not yaw else bx + toward_door * (bsx / 2 + 0.3 + dw / 2)
        dz = rz1 - 0.34
        clear_bed = dz - dd / 2 >= bz + bsz / 2 + 0.2 or abs(dx - bx) >= (bsx + dw) / 2 + 0.2
        clear_door = abs(dx - door_wall_x) >= dw / 2 + 0.6
        if clear_bed and clear_door and rx0 + m <= dx - dw / 2 and dx + dw / 2 <= rx1 - m and dz - dd / 2 >= rz0 + m:
            self.piece('dresser', dx, dz, math.pi, base)
            if k % 2 == 1:
                self.piece('plant-small', dx + 0.35, dz, 0.0, base, y=base + 1.18, solid=False, coarse=False)
        # window corner: armchair + side table, writing desk along the window wall
        if window:
            wxw, wzw = window
            along_x = abs(wzw - rz0) < 0.6 or abs(wzw - rz1) < 0.6
            if not along_x:
                inward = 1 if wxw < cx else -1
                chair_z = rz1 - 0.9
                self.piece('lounge-chair', wxw + inward * 0.75, chair_z, math.atan2(inward, -0.6), base, coarse='int-fabric-green')
                self.piece('side-table', wxw + inward * 0.45, chair_z - 0.75, 0.0, base)
                self.piece('writing-desk', wxw + inward * 0.35, (rz0 + rz1) / 2 + 0.2, math.atan2(-inward, 0) + math.pi, base)
                self.piece('cafe-chair', wxw + inward * 0.95, (rz0 + rz1) / 2 + 0.2, math.atan2(-inward, 0), base, solid=False, coarse=False)
        else:
            self.piece('lounge-chair', far_x + toward_door * 0.6, rz1 - 1.2, math.pi * 0.75 * toward_door, base, coarse='int-fabric-green')
        # a small wall washbasin by the door, as every room of a residential hotel has
        self.piece('basin', door_wall_x - toward_door * 0.1, rz0 + 0.9, math.atan2(-toward_door, 0), base, solid=False, coarse=False)
        self.hang('ceiling-lamp', cx, cz, top, base)
        # curtains on every window of this room
        for slot in self.window_slots:
            mx, mz = lerp2(slot['a'], slot['b'], 0.5)
            if not (base < slot['sill'] < top + 0.3):
                continue
            if rx0 - 0.6 < mx < rx1 + 0.6 and rz0 - 0.6 < mz < rz1 + 0.6:
                self.curtains(slot, base, k)

    def _office(self, x0, x1, z0, z1, pz1, base, top, k):
        st = self.stair
        # desk pairs with generous aisles, a shelf wall, a sofa corner, plants
        desks = []
        for zc in frange(pz1 + 2.2, z1 - 2.0, 4.0):
            for xc in frange(x0 + 2.0, x1 - 2.0, 3.6):
                if st['x0'] - 1.4 < xc < st['x1'] + 1.4 and zc < st['z1'] + 2.5:
                    continue
                desks.append((xc, zc))
        desks = desks[:6]
        for i, (xc, zc) in enumerate(desks):
            self.piece('office-desk-real', xc, zc, 0.0, base, fit=(1.6, None, 0.8))
            self.piece('office-chair-real', xc, zc + 0.75, math.pi, base, solid=False, coarse=False, fit=(0.62, None, 0.72))
            if i % 2 == 0:
                self.piece('laptop', xc - 0.2, zc - 0.05, math.pi, base, y=base + 0.79, solid=False, coarse=False)
            self.hang('pendant', xc, zc, top, base, drop=1.1)
        # shelving + books along the party side
        shelf_x = x0 + 0.3 if st['side'] > 0 else x1 - 0.3
        facing_yaw = math.pi / 2 if st['side'] > 0 else -math.pi / 2
        for zc in frange(pz1 + 1.5, min(z1 - 1.0, pz1 + 6.0), 1.1):
            self.piece('shelving', shelf_x, zc, facing_yaw, base)
            self.book_rows(shelf_x, zc, facing_yaw, base, k, width=0.85)
        # lounge corner near the front windows
        sx = x1 - 2.2 if st['side'] < 0 else x0 + 2.2
        self.piece('sofa', sx, z1 - 1.0, math.pi, base, coarse='int-fabric-blue')
        self.piece('coffee-table', sx, z1 - 2.1, math.pi / 2, base)
        self.rug(sx, z1 - 1.9, 2.6, 2.0, 0.0, base, k)
        self.piece('plant-medium', sx + 1.5, z1 - 0.6, 0.0, base, solid=False, coarse=False)
        self.piece('plant-medium', x0 + 0.6 if st['side'] < 0 else x1 - 0.6, pz1 + 0.8, 0.0, base, solid=False, coarse=False)
        mid = (x0 + x1) / 2
        self.add_light(mid, top - 0.4, (pz1 + z1) / 2 - 2.0, k, intensity=22.0, distance=11.0, priority=3)
        self.add_light(sx, top - 0.4, z1 - 1.8, k, intensity=14.0, distance=7.0, priority=2)

    # ---------------------------------------------------------- output

    def export(self, out_dir):
        os.makedirs(out_dir, exist_ok=True)
        results = {}
        exterior_only = getattr(self, 'scope', 'full') == 'exterior'
        if exterior_only:
            exterior_scope_glazing(self)
        fine = sorted((c for c in bpy.data.collections if c.name.startswith('fine-s')), key=lambda c: int(c.name[6:]))
        parts = [('exterior', self.exterior)] if exterior_only else [('exterior', self.exterior), ('interior', self.interior)] + [(c.name, c) for c in fine]
        if exterior_only:
            fine = []
        for name, col in parts:
            for o in bpy.context.scene.objects:
                o.select_set(False)
            layer = bpy.context.view_layer.layer_collection.children[col.name]
            bpy.context.view_layer.active_layer_collection = layer
            path = os.path.join(out_dir, f'{name}.glb')
            bpy.ops.export_scene.gltf(
                filepath=path,
                export_format='GLB',
                use_active_collection=True,
                use_visible=False,
                export_yup=True,
                export_apply=True,
                export_materials='EXPORT',
                export_image_format='NONE',
                export_jpeg_quality=80,
                export_texcoords=name != 'exterior',
                export_normals=True,
                export_tangents=False,
                export_gpu_instances=True,
                export_extras=False,
                export_cameras=False,
                export_lights=False,
            )
            tris = 0
            for o in col.objects:
                if o.type == 'MESH':
                    o.data.calc_loop_triangles()
                    tris += len(o.data.loop_triangles)
            lib_tris = getattr(self, 'library_triangles', {}).get(name, 0)
            results[name] = {'bytes': os.path.getsize(path), 'triangles': tris + lib_tris, 'libraryTriangles': lib_tris,
                             'objects': len(col.objects)}
        with open(os.path.join(out_dir, 'physics.json'), 'w') as fh:
            json.dump(self.physics, fh, indent=1)
        # at most six real lights per storey, strongest priority first
        lights = []
        for k in sorted({light['storey'] for light in self.lights}):
            mine = sorted((l for l in self.lights if l['storey'] == k), key=lambda l: -l['priority'])[:6]
            lights += mine
        report = {'files': results, 'parts': self.stats, 'stair': self.stair, 'lights': lights,
                  'storeys': self.storeys, 'fine': [c.name for c in fine],
                  'levels': self.levels, 'roofLevel': self.roof_level,
                  'ground': self.ground_programme(), 'upper': self.upper_programme()}
        if exterior_only:
            report['scope'] = 'exterior'
            report['stair'] = None   # planned for the openings only; nothing is built
        with open(os.path.join(out_dir, 'report.json'), 'w') as fh:
            json.dump(report, fh, indent=1)
        return report


# ---------------------------------------------------------------- utilities


def clip_ring_to_rect(ring, rect):
    """Sutherland-Hodgman: a (possibly concave) ring clipped to an axis-aligned
    rectangle (x0, x1, z0, z1). Returns the clipped ring (may be empty)."""
    x0, x1, z0, z1 = rect
    out = list(ring)
    for axis, bound, keep_greater in ((0, x0, True), (0, x1, False), (1, z0, True), (1, z1, False)):
        if not out:
            break
        src, out = out, []

        def inside(p):
            return p[axis] >= bound - 1e-9 if keep_greater else p[axis] <= bound + 1e-9

        for i in range(len(src)):
            p, q = src[i - 1], src[i]
            pin, qin = inside(p), inside(q)
            if qin:
                if not pin:
                    t = (bound - p[axis]) / ((q[axis] - p[axis]) or 1e-12)
                    out.append(lerp2(p, q, t))
                out.append(q)
            elif pin:
                t = (bound - p[axis]) / ((q[axis] - p[axis]) or 1e-12)
                out.append(lerp2(p, q, t))
    return out


def mesh_size(data):
    """(width x, height, depth z) of a mesh in frame terms (Blender x, z, y)."""
    if not data or not data.vertices:
        return (0.5, 0.5, 0.5)
    xs = [v.co.x for v in data.vertices]
    ys = [v.co.y for v in data.vertices]
    zs = [v.co.z for v in data.vertices]
    return (max(xs) - min(xs) or 0.5, max(zs) - min(zs) or 0.5, max(ys) - min(ys) or 0.5)


def at_edge(a, d, nout, sv, off):
    return add(add(a, mul(d, sv)), mul(nout, off))


def frange(a, b, step):
    out = []
    v = a
    while v <= b + 1e-6:
        out.append(v)
        v += step
    return out


def seg_distance(p, a, b):
    d = sub(b, a)
    l2 = dot(d, d) or 1e-9
    t = max(0.0, min(1.0, dot(sub(p, a), d) / l2))
    q = add(a, mul(d, t))
    return length(sub(p, q))


def rect_minus(r, hole):
    if not hole:
        return [r]
    x0, x1, z0, z1 = r
    hx0, hx1, hz0, hz1 = max(x0, hole[0]), min(x1, hole[1]), max(z0, hole[2]), min(z1, hole[3])
    out = []
    if hz0 - z0 > 0.05:
        out.append((x0, x1, z0, hz0))
    if z1 - hz1 > 0.05:
        out.append((x0, x1, hz1, z1))
    if hx0 - x0 > 0.05:
        out.append((x0, hx0, hz0, hz1))
    if x1 - hx1 > 0.05:
        out.append((hx1, x1, hz0, hz1))
    return out


# ---------------------------------------------------------------- furniture

# footprint (x, height, z) for collision; pieces are authored at the origin,
# standing on y = 0, facing +z.
FURNITURE_FOOTPRINT = {
    'table-4': (0.8, 0.75, 0.8),
    'bar-counter': (1.2, 1.1, 0.65),
    'back-bar': (1.8, 2.2, 0.45),
    'host-stand': (0.6, 1.1, 0.45),
    'kitchen-counter': (1.4, 0.92, 0.7),
    'range': (1.2, 0.92, 0.8),
    'fridge': (0.8, 2.0, 0.75),
    'shelf': (1.8, 1.6, 0.6),
    'shop-counter': (1.8, 1.0, 0.6),
    'bed-double': (1.45, 0.55, 2.05),
    'nightstand': (0.45, 0.55, 0.4),
    'wardrobe': (1.2, 2.0, 0.6),
    'desk': (1.2, 0.75, 0.6),
    'box-stack': (0.6, 0.9, 0.45),
    'office-desk': (1.4, 0.75, 0.7),
}


def f_table4(mb):
    mb.box((0, 0.73, 0), (0.8, 0.04, 0.8), 'int-wood')
    mb.box((0, 0.36, 0), (0.08, 0.7, 0.08), 'int-dark-metal')
    mb.box((0, 0.02, 0), (0.5, 0.04, 0.5), 'int-dark-metal')


def f_chair(mb):
    mb.box((0, 0.45, 0), (0.42, 0.05, 0.42), 'int-wood')
    mb.box((0, 0.7, -0.19), (0.42, 0.45, 0.04), 'int-wood')
    for sx in (-0.18, 0.18):
        for sz in (-0.18, 0.18):
            mb.box((sx, 0.22, sz), (0.035, 0.44, 0.035), 'int-dark-metal')


def f_bar_counter(mb):
    mb.box((0, 0.52, 0), (1.2, 1.04, 0.55), 'int-wood', mats={'top': 'int-wood'})
    mb.box((0, 1.07, 0.03), (1.2, 0.05, 0.65), 'int-wood-light')
    mb.box((0, 0.18, 0.3), (1.2, 0.04, 0.04), 'int-steel')


def f_bar_stool(mb):
    mb.box((0, 0.76, 0), (0.36, 0.05, 0.36), 'int-leather')
    mb.box((0, 0.38, 0), (0.05, 0.76, 0.05), 'int-steel')
    mb.box((0, 0.02, 0), (0.36, 0.03, 0.36), 'int-steel')


def f_back_bar(mb):
    mb.box((0, 0.45, 0), (1.8, 0.9, 0.45), 'int-wood')
    for y in (1.25, 1.65, 2.05):
        mb.box((0, y, -0.05), (1.8, 0.03, 0.3), 'int-wood-light')
        for i in range(8):
            mb.box((-0.8 + i * 0.22, y + 0.14, -0.05), (0.07, 0.26, 0.07), 'int-bottle')
    mb.box((0, 1.6, -0.2), (1.8, 1.4, 0.03), 'int-dark-metal')


def f_host(mb):
    mb.box((0, 0.55, 0), (0.6, 1.1, 0.45), 'int-wood')


def f_kitchen_counter(mb):
    mb.box((0, 0.45, 0), (1.4, 0.9, 0.7), 'int-steel')
    mb.box((0, 1.6, -0.2), (1.4, 0.04, 0.3), 'int-steel')


def f_range(mb):
    mb.box((0, 0.45, 0), (1.2, 0.9, 0.8), 'int-dark-metal')
    mb.box((0, 0.93, 0), (1.2, 0.03, 0.8), 'int-steel')
    mb.box((0, 2.1, -0.1), (1.3, 0.5, 0.9), 'int-steel')


def f_fridge(mb):
    mb.box((0, 1.0, 0), (0.8, 2.0, 0.75), 'int-steel')


def f_shelf(mb):
    mb.box((0, 0.8, 0), (1.8, 1.6, 0.08), 'int-steel')
    for y in (0.15, 0.55, 0.95, 1.35):
        mb.box((0, y, 0), (1.8, 0.03, 0.6), 'int-steel')
        mb.box((0, y + 0.13, 0.12), (1.6, 0.22, 0.22), 'int-fabric-red')


def f_shop_counter(mb):
    mb.box((0, 0.5, 0), (1.8, 1.0, 0.6), 'int-wood-light')
    mb.box((0.4, 1.15, 0), (0.4, 0.3, 0.05), 'int-screen')


def f_bed(mb):
    mb.box((0, 0.2, 0), (1.45, 0.3, 2.0), 'int-wood')
    mb.box((0, 0.44, 0.02), (1.4, 0.2, 1.95), 'int-fabric-bed')
    mb.box((0, 0.56, 0.35), (1.42, 0.06, 1.3), 'int-fabric-blue')
    mb.box((-0.33, 0.6, -0.78), (0.6, 0.12, 0.35), 'int-fabric-bed')
    mb.box((0.33, 0.6, -0.78), (0.6, 0.12, 0.35), 'int-fabric-bed')
    mb.box((0, 0.6, -1.0), (1.5, 1.2, 0.06), 'int-wood')


def f_nightstand(mb):
    mb.box((0, 0.27, 0), (0.45, 0.55, 0.4), 'int-wood')
    mb.box((0, 0.65, 0), (0.16, 0.2, 0.16), 'int-light')


def f_wardrobe(mb):
    mb.box((0, 1.0, 0), (1.2, 2.0, 0.6), 'int-wood-light')
    mb.box((0, 1.0, 0.301), (0.01, 1.9, 0.005), 'int-wood')


def f_desk(mb):
    mb.box((0, 0.73, 0), (1.2, 0.04, 0.6), 'int-wood-light')
    for sx in (-0.56, 0.56):
        mb.box((sx, 0.36, 0), (0.04, 0.72, 0.56), 'int-wood-light')
    # small lamp: dark stem, a shade that glows only a little
    mb.box((0.42, 0.9, -0.15), (0.03, 0.3, 0.03), 'int-dark-metal')
    mb.box((0.42, 1.07, -0.15), (0.16, 0.1, 0.16), 'int-light')


def f_office_desk(mb):
    mb.box((0, 0.73, 0), (1.4, 0.04, 0.7), 'int-wood-light')
    for sx in (-0.66, 0.66):
        mb.box((sx, 0.36, 0), (0.04, 0.72, 0.66), 'int-dark-metal')
    mb.box((0, 0.76, -0.22), (0.2, 0.02, 0.15), 'int-dark-metal')
    mb.box((0, 0.98, -0.25), (0.56, 0.34, 0.03), 'int-screen')
    mb.box((0, 0.84, -0.25), (0.04, 0.12, 0.03), 'int-dark-metal')
    mb.box((0, 0.755, 0.05), (0.45, 0.015, 0.15), 'int-dark-metal')


def f_office_chair(mb):
    mb.box((0, 0.48, 0), (0.48, 0.08, 0.46), 'int-fabric-blue')
    mb.box((0, 0.8, -0.21), (0.46, 0.55, 0.06), 'int-fabric-blue')
    mb.box((0, 0.24, 0), (0.05, 0.44, 0.05), 'int-dark-metal')
    mb.box((0, 0.03, 0), (0.55, 0.04, 0.55), 'int-dark-metal')


def f_rug(mb):
    mb.box((0, 0.012, 0), (1.6, 0.012, 1.1), 'int-rug')


def f_armchair(mb):
    mb.box((0, 0.22, 0), (0.8, 0.44, 0.75), 'int-fabric-green')
    mb.box((0, 0.65, -0.3), (0.8, 0.5, 0.16), 'int-fabric-green')
    for sx in (-0.33, 0.33):
        mb.box((sx, 0.55, 0), (0.14, 0.24, 0.75), 'int-fabric-green')


def f_ceiling_light(mb):
    mb.box((0, -0.03, 0), (0.5, 0.05, 0.5), 'int-light')


def f_plant(mb):
    mb.box((0, 0.2, 0), (0.4, 0.4, 0.4), 'int-ceramic')
    mb.box((0, 0.8, 0), (0.6, 0.8, 0.6), 'int-plant')


def f_stair_lamp(mb):
    mb.box((0, 2.4, 0), (0.3, 0.1, 0.3), 'int-light')


def f_box_stack(mb):
    mb.box((0, 0.25, 0), (0.6, 0.5, 0.45), 'int-wood-light')
    mb.box((0.05, 0.7, 0.02), (0.5, 0.4, 0.4), 'int-wood-light')


FALLBACK = {
    'cafe-table': 'table-4', 'cafe-chair': 'chair', 'bar-stool': 'bar-stool', 'pendant': 'ceiling-light',
    'ceiling-lamp': 'ceiling-light', 'plant-tall': 'plant', 'plant-small': None, 'plant-medium': 'plant',
    'bed-frame': 'bed-double', 'nightstand': 'nightstand', 'dresser': 'wardrobe', 'lounge-chair': 'armchair',
    'side-table': 'nightstand', 'writing-desk': 'desk', 'office-desk-real': 'office-desk',
    'office-chair-real': 'office-chair', 'shelving': 'shelf', 'sofa': 'armchair', 'coffee-table': 'table-4',
    'cardboard-box': 'box-stack', 'crate': 'box-stack', 'basin': None,
    # procedural pieces keep their own builders
    'bar-counter': 'bar-counter', 'back-bar': 'back-bar', 'host-stand': 'host-stand',
    'kitchen-counter': 'kitchen-counter', 'range': 'range', 'fridge': 'fridge', 'shelf': 'shelf',
    'shop-counter': 'shop-counter',
}
FALLBACK_SIZE = {
    'bar-counter': (1.2, 1.1, 0.65), 'back-bar': (1.8, 2.2, 0.45), 'host-stand': (0.6, 1.1, 0.45),
    'kitchen-counter': (1.4, 0.92, 0.7), 'range': (1.2, 0.92, 0.8), 'fridge': (0.8, 2.0, 0.75),
    'shelf': (1.8, 1.6, 0.6), 'shop-counter': (1.8, 1.0, 0.6),
}
# coarse tier: one box per large piece, drawn on every storey but the active one
COARSE_MATERIAL = {
    'cafe-table': 'int-wood', 'bed-frame': 'int-duvet', 'nightstand': 'int-wood', 'dresser': 'int-wood-light',
    'writing-desk': 'int-wood', 'office-desk-real': 'int-wood-light', 'shelving': 'int-dark-metal',
    'coffee-table': 'int-wood', 'bar-counter': 'int-wood', 'back-bar': 'int-wood', 'kitchen-counter': 'int-steel',
    'range': 'int-dark-metal', 'fridge': 'int-steel', 'host-stand': 'int-wood', 'shelf': 'int-steel',
    'shop-counter': 'int-wood-light', 'cardboard-box': 'int-wood-light', 'crate': 'int-wood',
}

FURNITURE_BUILDERS = {
    'box-stack': f_box_stack,
    'office-desk': f_office_desk,
    'table-4': f_table4, 'chair': f_chair, 'bar-counter': f_bar_counter, 'bar-stool': f_bar_stool,
    'back-bar': f_back_bar, 'host-stand': f_host, 'kitchen-counter': f_kitchen_counter, 'range': f_range,
    'fridge': f_fridge, 'shelf': f_shelf, 'shop-counter': f_shop_counter, 'bed-double': f_bed,
    'nightstand': f_nightstand, 'wardrobe': f_wardrobe, 'desk': f_desk, 'office-chair': f_office_chair,
    'rug': f_rug, 'armchair': f_armchair, 'ceiling-light': f_ceiling_light, 'plant': f_plant,
    'stair-lamp': f_stair_lamp,
}


# ---------------------------------------------------------------- preview render


def render_previews(b, out_dir):
    """Eevee stills for the builder's own look (NOT evidence; the game capture is)."""
    os.makedirs(out_dir, exist_ok=True)
    scene = bpy.context.scene
    scene.render.engine = 'BLENDER_EEVEE_NEXT' if 'BLENDER_EEVEE_NEXT' in [e.identifier for e in bpy.types.RenderSettings.bl_rna.properties['engine'].enum_items] else 'BLENDER_EEVEE'
    scene.render.resolution_x = 1280
    scene.render.resolution_y = 800
    world = bpy.data.worlds.new('sky')
    world.use_nodes = True
    world.node_tree.nodes['Background'].inputs['Color'].default_value = (0.55, 0.65, 0.8, 1)
    world.node_tree.nodes['Background'].inputs['Strength'].default_value = 1.0
    scene.world = world
    sun = bpy.data.objects.new('sun', bpy.data.lights.new('sun', 'SUN'))
    sun.data.energy = 3.5
    sun.rotation_euler = (math.radians(50), 0, math.radians(35))
    scene.collection.objects.link(sun)
    ground = MeshBuilder('ground')
    ground.box((0, -0.05, 6), (60, 0.1, 60), 'concrete-precast')
    ground.build(scene.collection)
    cam = bpy.data.objects.new('cam', bpy.data.cameras.new('cam'))
    scene.collection.objects.link(cam)
    scene.camera = cam
    fr = b.spec['frame']
    shots = {
        'front-34': (B(fr['width'] * 0.9, 1.7, fr['depth'] / 2 + 12), B(0, b.height * 0.45, fr['depth'] / 2)),
        'front': (B(0, 1.7, fr['depth'] / 2 + 16), B(0, b.height * 0.5, fr['depth'] / 2)),
        'ground-inside': (B(b.rect[1] - 1.0, 1.6, b.rect[3] - 1.0), B(b.rect[0] + 2, 1.0, b.rect[2] + 3)),
    }
    if b.storeys > 1:
        lvl = b.levels[1]
        shots['upper-corridor'] = (B((b.rect[0] + b.rect[1]) / 2 + 0.3, lvl + 1.6, b.rect[3] - 0.8), B((b.rect[0] + b.rect[1]) / 2, lvl + 1.2, b.rect[2] + 2))
    # BB_PREVIEW_SHOTS='{"name": [[x, y, z], [x, y, z], lensMm]}' adds stills
    # from given spec-frame eyes (e.g. a game pose converted to the frame)
    for name, v in json.loads(os.environ.get('BB_PREVIEW_SHOTS') or '{}').items():
        shots[name] = (B(*v[0]), B(*v[1]))
    for name, (eye, target) in shots.items():
        cam.location = eye
        cam.rotation_euler = (target - eye).to_track_quat('-Z', 'Y').to_euler()
        cam.data.lens = 18 if 'inside' in name or 'corridor' in name else (23.5 if name.startswith('pose') else 28)
        scene.render.filepath = os.path.join(out_dir, f'{name}.png')
        bpy.ops.render.render(write_still=True)


# ---------------------------------------------------------------- garage-base flats
# (bldg-263347480; every key absent = unchanged builds)
# `storeysSeen: {"count": n, "groundM": g}` - the storey count read off the
# street reference when the Assessor roll leaves a storey out (a garage storey
# under three floors of flats is rolled as 3). The LiDAR roof height and the
# roof allowance stay what the data says; the uppers share the rest equally.
# `ground: "garage"` - a residential garage ground floor: the base stair lobby
# and core, the room behind the doors dressed as parking bays (painted stripes,
# wheel stops), storage shelving and boxes along the walls, strip lamps.
# `garagePanels: {"material", "tint", "rows", "cols"}` - `glazed-garage` infills
# on this building get opaque painted panels in the frame grid instead of glass.
# `bays[].plaques: {"material", "tint", "heightM", "widthFrac"}` - a flat
# glazed-tile plaque on every face of a (classic, unglazed) bay in the spandrel
# between two storeys of windows.
# `bayOnlyUpper: true` - the upper storeys of the street faces open only behind
# the projecting bays (plain wall between them, no punched windows).
_gb_init_base = Building.__init__


def _gb_init(self, spec, style):
    ss = (style or {}).get('storeysSeen') or {}
    n = int(ss.get('count') or 0)
    if n > 1 and n != spec['storeys']:
        f = spec['floors']
        usable = spec['heightM'] - f.get('roofM', 0.6)
        g = float(ss.get('groundM') or usable / n)
        up = (usable - g) / (n - 1)
        if up >= MIN_STOREY_M and g >= MIN_STOREY_M:
            spec = dict(spec, storeys=n, floors=dict(f, groundM=round(g, 3), upperM=round(up, 3)))
    _gb_init_base(self, spec, style)


Building.__init__ = _gb_init
_gb_ground_programme_base = Building.ground_programme
_gb_build_interior_base = Building.build_interior
_gb_vacant_base = Building._vacant
_gb_closed_infill_base = Building._closed_infill


def _gb_ground_programme(self):
    if self.style.get('ground') == 'garage':
        # the interior builder dresses a garage as a bare concrete room first
        return 'vacant' if getattr(self, '_gb_in_interior', False) else 'garage'
    return _gb_ground_programme_base(self)


def _gb_build_interior(self):
    self._gb_in_interior = True
    try:
        return _gb_build_interior_base(self)
    finally:
        self._gb_in_interior = False


def _gb_vacant(self, rx0, rx1, kz, z1, top):
    if self.style.get('ground') != 'garage':
        return _gb_vacant_base(self, rx0, rx1, kz, z1, top)
    mb = self.soft_for(0)
    w, dz = rx1 - rx0, z1 - kz
    bays = max(1, int(w // 2.6))
    bw = w / bays
    stall = min(5.4, dz - 1.0)
    # stripes between the stalls, a wheel stop at the head of each stall
    for i in range(bays + 1):
        x = rx0 + bw * i
        if 0.3 < x - rx0 < w - 0.3:
            mb.box((x, 0.026, z1 - stall / 2), (0.1, 0.006, stall), 'int-baseboard', 0.0)
    for i in range(bays):
        x = rx0 + bw * (i + 0.5)
        mb.box((x, 0.07, z1 - stall + 0.35), (min(1.8, bw - 0.6), 0.12, 0.16), 'int-concrete-floor', 0.0)
    # storage along the back wall, boxes on the floor, strip lamps over the stalls
    back = z1 - stall - 0.1
    if back - kz > 0.9:
        for xc in frange(rx0 + 0.9, rx1 - 0.9, 1.6):
            self.piece('shelving', xc, kz + 0.35, 0.0, 0.0)
        for fx, kind, yaw in ((0.2, 'cardboard-box', 0.2), (0.55, 'crate', 0.0), (0.8, 'box-stack', 1.4)):
            self.piece(kind, rx0 + w * fx, min(back - 0.4, kz + 1.1), yaw, 0.0)
    for i in range(bays):
        x = rx0 + bw * (i + 0.5)
        self.hang('ceiling-lamp', x, z1 - stall / 2, top, 0.0)
    self.add_light((rx0 + rx1) / 2, top - 0.5, z1 - stall / 2, 0, intensity=12.0, distance=10.0, priority=2)


def _gb_closed_infill(self, kind, span, at, s, store, glass, yaw):
    gp = self.style.get('garagePanels')
    if kind != 'glazed-garage' or not gp:
        return _gb_closed_infill_base(self, kind, span, at, s, store, glass, yaw)
    s0, s1, y0, y1, mat, tint = span[:6]
    w, h, cs = s1 - s0, y1 - y0, (s0 + s1) / 2
    back = -0.16
    fm = self.tinted(mat or 'painted-white', tint)
    pm = self.tinted(gp.get('material', 'painted-steel'), gp.get('tint'))
    p = at(cs, back)
    store.box((p[0], (y0 + y1) / 2, p[1]), (w, h, 0.03), pm, yaw)
    cols = int(gp.get('cols') or max(2, int(round(w / 0.75))))
    rows = int(gp.get('rows') or max(3, int(round(h / 0.6))))
    # `railM` - stile/rail width (chunky painted rails), drawn on
    # both faces of the door; `surroundM` - a painted casing round the
    # opening on the facade, jambs down to the pavement
    rw = float(gp.get('railM', 0.07))
    for off in ((back + 0.03, back - 0.03) if gp.get('railM') else (back + 0.03,)):
        for i in range(cols + 1):
            q = at(s0 + rw / 2 + (w - rw) * i / cols, off)
            store.box((q[0], (y0 + y1) / 2, q[1]), (rw, h, 0.06), fm, yaw)
        for r in range(rows + 1):
            q = at(cs, off)
            store.box((q[0], y0 + rw / 2 + (h - rw) * r / rows, q[1]), (w, rw, 0.06), fm, yaw)
    sw = float(gp.get('surroundM', 0.0))
    if sw > 0:
        tm = self.tinted(gp.get('surroundMaterial', 'painted-white'), gp.get('surroundTint'))
        for sv in (s0 - sw / 2 + 0.02, s1 + sw / 2 - 0.02):
            q = at(sv, 0.03)
            store.box((q[0], (y1 + 0.02) / 2, q[1]), (sw, y1 + 0.02, 0.08), tm, yaw)
        q = at(cs, 0.03)
        store.box((q[0], y1 + sw / 2 - 0.02, q[1]), (w + 2 * sw - 0.04, sw, 0.08), tm, yaw)


_gb_openings_for_edge_base = Building.openings_for_edge
def _gb_openings_for_edge(self, e):
    ops, windows, doors = _gb_openings_for_edge_base(self, e)
    if self.edge_style(e).get('bayOnlyUpper') and self.is_street(e):
        punched = [o for kind, o in windows if kind == 'punched']
        windows = [w for w in windows if w[0] != 'punched']
        ops = [o for o in ops if not any(o is q for q in punched)]
    return ops, windows, doors


Building.openings_for_edge = _gb_openings_for_edge
# Every key below is new; absent = unchanged builds.
# `bays[].sash: {footM, fillFrac, maxWidthM, sillM, headGapM, lightM, upperRows,
# upperCols, muntinM, meetingFrac}` - a classic (3-sided, unglazed) bay whose faces are nearly
# filled by white double-hung sashes: a wide face splits into lights of about
# `lightM`, each with a one-light lower sash, a meeting rail and an upper sash
# divided `upperRows` x `upperCols` by muntins, in a casing with a projecting
# sill (`maxWidthM` caps one window per face; `muntinM`, the upper-sash pane
# width, replaces `upperCols`). The glass takes `glassBacking` (kind "bay").
# `bays[].plaques.ornament: {material, tint}` - a relief border and a centre
# medallion on every plaque; with `sash` the plaques fill the spandrel between
# the plate and the sills of the storey above.
# `stringCourse: {heightM, projectionM, material, tint}` - a thin continuous
# band at the first-floor line that also runs round the foot of every bay.
_gb_bays_base = Building._bays


def _gb_is_sash(sp):
    return bool(sp.get('sash')) and not sp.get('glazed') and not sp.get('roundCorners')


# `bays[].sash.sideRunM` - the run along the wall of each angled side
# face (default the projection, a 45-degree bay); a longer run gives shallower
# side faces wide enough for a full sash.
def _gb_ring(at, spec, c, w, p):
    q = p
    if _gb_is_sash(spec) and spec['sash'].get('sideRunM'):
        q = min(float(spec['sash']['sideRunM']), w / 2 - 0.2)
    return [at(c - w / 2, 0), at(c - w / 2 + q, p), at(c + w / 2 - q, p), at(c + w / 2, 0)]


def _gb_fbox(mb, q, fd, fn, u, v, off, size, mat, fy):
    # a box on a face frame: u along the face, v height, off out of the face
    p = add(add(q, mul(fd, u)), mul(fn, off))
    mb.box((p[0], v, p[1]), size, mat, fy)


def _gb_scroll(mb, q, fd, fn, u, v, r, off, mat, fy, sense=1, n=7):
    # a gilt volute: a tightening spiral of small blocks, raised off the face
    for i in range(n):
        t = i / n
        ang = sense * (0.6 + t * 4.6)
        rr = r * (1.0 - 0.72 * t)
        s = max(0.028, r * 0.32 * (1.0 - 0.5 * t))
        _gb_fbox(mb, q, fd, fn, u + rr * math.cos(ang), v + rr * math.sin(ang), off, (s, s, 0.03), mat, fy)


def _gb_figure(mb, q, fd, fn, u, v, hgt, off, mat, fy):
    # a stylised relief figure: stacked torso blocks, head, spread arms
    rows = ((0.0, 0.34), (0.16, 0.46), (0.32, 0.5), (0.48, 0.4), (0.62, 0.26))
    wid = hgt * 0.42
    for y, fw in rows:
        _gb_fbox(mb, q, fd, fn, u, v - hgt / 2 + hgt * (y + 0.08), off, (wid * fw, hgt * 0.16, 0.04), mat, fy)
    _gb_fbox(mb, q, fd, fn, u, v + hgt * 0.36, off + 0.01, (hgt * 0.17, hgt * 0.17, 0.045), mat, fy)
    for side in (-1, 1):
        _gb_fbox(mb, q, fd, fn, u + side * wid * 0.34, v + hgt * 0.14, off, (wid * 0.26, hgt * 0.07, 0.035), mat, fy)
        _gb_fbox(mb, q, fd, fn, u + side * wid * 0.5, v + hgt * 0.24, off, (wid * 0.1, hgt * 0.16, 0.035), mat, fy)


def _gb_bays(self, e, a, b):
    allspecs = self.style.get('bays') or []
    sash = [sp for sp in allspecs if _gb_is_sash(sp)]
    if sash:
        self.style['bays'] = [sp for sp in allspecs if not _gb_is_sash(sp)]
        try:
            _gb_bays_base(self, e, a, b)
        finally:
            self.style['bays'] = allspecs
        _gb_sash_bays(self, e, a, b, sash)
    else:
        _gb_bays_base(self, e, a, b)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    specs = [sp for sp in allspecs if sp.get('plaques') and not sp.get('glazed') and not sp.get('roundCorners')]
    mb = MeshBuilder('bay-plaques')
    for spec in specs:
        pq = spec['plaques']
        m = self.tinted(pq.get('material', 'terracotta'), pq.get('tint'))
        orn = pq.get('ornament')
        om = self.tinted(orn.get('material', 'painted-white'), orn.get('tint')) if orn else None
        c, w, p = self.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.6)
        ring = _gb_ring(at, spec, c, w, p)
        centre = at(c, p * 0.4)
        k0 = spec.get('fromStorey', 1)
        k1 = min(self.storeys, spec.get('toStorey', self.storeys - 1) + 1)
        h = float(pq.get('heightM', 0.5))
        sh = spec.get('sash') if _gb_is_sash(spec) else None
        fm = self.tinted(pq.get('material', 'terracotta'), pq['fieldTint']) if pq.get('fieldTint') else None
        for k in range(k0 + 1, k1):
            if sh and pq.get('fullSpandrel'):
                # between the head casing of the storey below and the sill above
                lo = self.levels[k] - 0.12 - float(sh.get('headGapM', 0.2)) + 0.07
                hi = self.levels[k] + 0.05 + float(sh.get('sillM', 0.6)) - 0.1
                hk = min(h, hi - lo)
                yc = (lo + hi) / 2
            elif sh:
                lo = self.levels[k] + (0.12 if k > k0 else 0.05) + 0.04
                hi = self.levels[k] + 0.05 + float(sh.get('sillM', 0.6)) - 0.08
                hk = min(h, hi - lo)
                yc = (lo + hi) / 2
            else:
                hk = h
                yc = self.levels[k] + 0.12 + (0.05 + 0.75 - 0.12) / 2
            if hk < 0.12:
                continue
            for fa, fb in zip(ring[:-1], ring[1:]):
                fl = length(sub(fb, fa))
                fd = norm(sub(fb, fa))
                fn = (-fd[1], fd[0])
                if dot(sub(lerp2(fa, fb, 0.5), centre), fn) < 0:
                    fn = (fd[1], -fd[0])
                pw = fl * float(pq.get('widthFrac', 0.7))
                q = add(lerp2(fa, fb, 0.5), mul(fn, 0.015))
                fy = box_yaw(fd)
                mb.box((q[0], yc, q[1]), (pw, hk, 0.03), m, fy)
                if not om:
                    continue
                if orn.get('relief') == 'scroll-figure':
                    # sculpted tile panel: a sunk darker field inside a gilt
                    # moulded border, a relief figure at the centre and
                    # volutes either side (one on a narrow face)
                    if fm:
                        _gb_fbox(mb, q, fd, fn, 0, yc, 0.018, (pw - 0.1, hk - 0.1, 0.01), fm, fy)
                    for yy in (yc - hk / 2 + 0.03, yc + hk / 2 - 0.03):
                        _gb_fbox(mb, q, fd, fn, 0, yy, 0.025, (pw, 0.05, 0.035), om, fy)
                    for side in (-1, 1):
                        _gb_fbox(mb, q, fd, fn, side * (pw / 2 - 0.025), yc, 0.025, (0.05, hk, 0.035), om, fy)
                    r = min(hk * 0.3, pw * 0.13)
                    if pw > 0.75:
                        _gb_figure(mb, q, fd, fn, 0, yc, hk * 0.8, 0.035, om, fy)
                        for side in (-1, 1):
                            _gb_scroll(mb, q, fd, fn, side * pw * 0.27, yc, r, 0.035, om, fy, sense=side)
                            _gb_fbox(mb, q, fd, fn, side * pw * 0.13, yc - hk * 0.22, 0.03, (pw * 0.12, 0.035, 0.03), om, fy)
                    else:
                        _gb_figure(mb, q, fd, fn, 0, yc, hk * 0.78, 0.035, om, fy)
                    continue
                # relief: a raised border and a medallion with a cross bar
                qo = add(q, mul(fn, 0.02))
                for yy in (yc - hk / 2 + 0.025, yc + hk / 2 - 0.025):
                    mb.box((qo[0], yy, qo[1]), (pw, 0.04, 0.025), om, fy)
                for side in (-1, 1):
                    qs = add(qo, mul(fd, side * (pw / 2 - 0.02)))
                    mb.box((qs[0], yc, qs[1]), (0.04, hk, 0.025), om, fy)
                qm = add(q, mul(fn, 0.03))
                mb.box((qm[0], yc, qm[1]), (min(0.2, pw * 0.3), min(0.2, hk * 0.6), 0.03), om, fy)
                if pw > 0.9:
                    mb.box((qo[0], yc, qo[1]), (pw * 0.7, 0.035, 0.02), om, fy)
                    for side in (-1, 1):
                        qs = add(qm, mul(fd, side * pw * 0.3))
                        mb.box((qs[0], yc, qs[1]), (0.1, 0.1, 0.03), om, fy)
    mb.build(self.exterior)
    # thin continuous string course over the ground storey, round every bay foot
    sc = self.style.get('stringCourse')
    if sc and self.storeys > 1 and e['role'] == 'front':
        hh, pr = float(sc.get('heightM', 0.12)), float(sc.get('projectionM', 0.05))
        m = self.tinted(sc.get('material', 'painted-white'), sc.get('tint'))
        y = self.levels[1] - hh / 2
        tb = MeshBuilder('string-course')
        q = at(L / 2, pr / 2)
        tb.box((q[0], y, q[1]), (L, hh, pr + 0.02), m, box_yaw(d))
        for spec in allspecs:
            if spec.get('fromStorey', 1) != 1 or spec.get('glazed'):
                continue
            c, w, p = self.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.6)
            ring = _gb_ring(at, spec, c, w, p)
            centre = at(c, p * 0.4)
            for fa, fb in zip(ring[:-1], ring[1:]):
                fl = length(sub(fb, fa))
                fd = norm(sub(fb, fa))
                fn = (-fd[1], fd[0])
                if dot(sub(lerp2(fa, fb, 0.5), centre), fn) < 0:
                    fn = (fd[1], -fd[0])
                q = add(lerp2(fa, fb, 0.5), mul(fn, pr / 2))
                tb.box((q[0], y, q[1]), (fl + 0.06, hh, pr + 0.02), m, box_yaw(fd))
        tb.build(self.exterior)


def _gb_sash_bays(self, e, a, b, specs):
    s = self.style
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    mb = MeshBuilder('bay-windows-sash')
    glass = MeshBuilder('bay-glazing-sash')
    for spec in specs:
        sh = spec['sash']
        c, w, p = self.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.6)
        mat = self.tinted(spec.get('material', 'painted-stucco'), spec.get('tint'))
        fmat = self.tinted(spec.get('frame', s['frame']), spec.get('frameTint'))
        k0 = spec.get('fromStorey', 1)
        k1 = min(self.storeys, spec.get('toStorey', self.storeys - 1) + 1)
        y_bottom = self.levels[k0]
        y_top = self.levels[k1] - 0.3 if k1 < self.storeys else self.roof_level

        def at(sv, off):
            return add(add(a, mul(d, sv)), mul(nout, off))

        ring = _gb_ring(at, spec, c, w, p)
        centre = at(c, p * 0.4)
        self._prism(mb, ring, y_bottom - float(sh.get('footM', 0.45)), y_bottom + 0.05, mat)
        for k in range(k0 + 1, k1):
            self._prism(mb, ring, self.levels[k] - 0.12, self.levels[k] + 0.12, mat)
        if s.get('dentilCornice') and k1 >= self.storeys:
            # the bay rises into the frieze of the dentil cornice
            self._prism(mb, ring, y_top - 0.05, self.height - 0.2, mat)
        else:
            self._prism(mb, [ring[0]] + [add(q, mul(nout, 0.12)) for q in ring[1:-1]] + [ring[-1]], y_top - 0.05, y_top + 0.35, mat)
        fill = float(sh.get('fillFrac', 0.86))
        sill_h, gap = float(sh.get('sillM', 0.6)), float(sh.get('headGapM', 0.2))
        ur, uc = int(sh.get('upperRows', 2)), int(sh.get('upperCols', 3))
        meet, light = float(sh.get('meetingFrac', 0.5)), float(sh.get('lightM', 1.1))
        for k in range(k0, k1):
            y0 = self.levels[k] + 0.05
            y1 = (self.levels[k + 1] if k + 1 < self.storeys else self.roof_level) - 0.12
            sill, head = y0 + sill_h, y1 - gap
            if head - sill < 0.6:
                continue
            ym = sill + (head - sill) * meet
            for fa, fb in zip(ring[:-1], ring[1:]):
                fl = length(sub(fb, fa))
                fd = norm(sub(fb, fa))
                fn = (-fd[1], fd[0])
                if dot(sub(lerp2(fa, fb, 0.5), centre), fn) < 0:
                    fn = (fd[1], -fd[0])
                ia, ib = sub(fa, mul(fn, 0.1)), sub(fb, mul(fn, 0.1))
                ww = fl * fill if fl > 1.2 else max(0.3, fl - 0.14)
                if sh.get('maxWidthM'):
                    ww = min(ww, float(sh['maxWidthM']))
                wall_grid(mb, fa, fb, ia, ib, y0, y1, [((fl - ww) / 2, (fl + ww) / 2, sill, head)],
                          mat, 'int-plaster', mat, cap_top=False)
                gc = add(lerp2(fa, fb, 0.5), mul(fn, -0.05))
                fyaw = box_yaw(fd)
                glass.box((gc[0], (sill + head) / 2, gc[1]), (ww, head - sill, 0.02), s['glass'], fyaw)
                self._glass_backing(glass, s, 'bay', gc, sill, head, ww, fyaw, mul(fn, -1))
                out = add(gc, mul(fn, 0.025))
                # projecting sill, head casing
                q = add(gc, mul(fn, 0.07))
                glass.box((q[0], sill - 0.035, q[1]), (ww + 0.14, 0.07, 0.18), fmat, fyaw)
                glass.box((out[0], head - 0.045, out[1]), (ww, 0.09, 0.07), fmat, fyaw)
                n = max(1, int(round(ww / light)))
                lw = ww / n
                for j in range(n + 1):
                    q2 = add(out, mul(fd, -ww / 2 + lw * j))
                    pw = 0.08 if j in (0, n) else 0.1
                    q2 = add(q2, mul(fd, pw / 2 if j == 0 else (-pw / 2 if j == n else 0)))
                    glass.box((q2[0], (sill + head) / 2, q2[1]), (pw, head - sill, 0.07), fmat, fyaw)
                for j in range(n):
                    lc = add(out, mul(fd, -ww / 2 + lw * (j + 0.5)))
                    # bottom rail of the lower sash, meeting rail
                    glass.box((lc[0], sill + 0.05, lc[1]), (lw, 0.08, 0.06), fmat, fyaw)
                    glass.box((lc[0], ym, lc[1]), (lw, 0.07, 0.07), fmat, fyaw)
                    ucl = max(1, int(round(lw / float(sh['muntinM'])))) if sh.get('muntinM') else uc
                    for i in range(1, ucl):
                        q3 = add(lc, mul(fd, -lw / 2 + lw * i / ucl))
                        glass.box((q3[0], (ym + head) / 2, q3[1]), (0.025, head - ym, 0.04), fmat, fyaw)
                    for r in range(1, ur):
                        yy = ym + (head - ym) * r / ur
                        glass.box((lc[0], yy, lc[1]), (lw, 0.025, 0.04), fmat, fyaw)
    mb.build(self.exterior)
    glass.build(self.exterior)


Building._bays = _gb_bays

# `fireEscape.ladder: true` (+ `standoffM`) - a platform-and-ladder escape
# hung `standoffM` off the wall (the face of a bay): slatted platforms with a
# lattice underside and railings, outriggers back to the wall, near-vertical
# ladders between the platforms and a drop ladder under the lowest one.
_gb_fire_escape_base = Building._fire_escape


def _gb_fire_escape(self, e, a, b):
    spec = self.style.get('fireEscape')
    if not spec or not spec.get('ladder') or self.storeys < 2:
        return _gb_fire_escape_base(self, e, a, b)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    c, w, depth = self.f2s(spec['at'], L, e), spec.get('widthM', 2.6), spec.get('depthM', 0.9)
    so = float(spec.get('standoffM', 0.0))
    mat = self.tinted(spec.get('material', 'painted-steel'), spec.get('tint'))
    mb = MeshBuilder('fire-escape')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    k_last = min(self.storeys - 1, spec.get('toStorey', self.storeys - 1))
    # `levels` - only these storeys get a platform; `dropLadder`
    # (default true) - the drop ladder under the lowest platform; `roofLadder`
    # - a ladder from the top platform over the cornice; `braces` - diagonal
    # struts from the platform's outer edge back down to the wall
    ks = [k for k in (spec.get('levels') or range(1, k_last + 1)) if 1 <= k < self.storeys]
    k_last = max(ks)
    for k in ks:
        y = self.levels[k] - 0.02
        if spec.get('braces'):
            for side in (-1, 1):
                sv = c + side * (w / 2 - 0.1)
                for i in range(9):
                    t = (i + 0.5) / 9
                    q = at(sv, so + depth * (1 - t))
                    mb.box((q[0], y - 0.1 - 1.0 * t, q[1]), (0.04, 0.13, 0.13), mat, yaw)
        if spec.get('roofLadder') and k == k_last:
            lx = c + (w / 2 - 0.45)
            s0, s1 = y, self.height + float(spec.get('roofLadderAboveM', 0.9))
            for side in (-1, 1):
                q = at(lx + side * 0.2, so + depth * 0.35)
                mb.box((q[0], (s0 + s1) / 2, q[1]), (0.045, s1 - s0, 0.05), mat, yaw)
            for i in range(1, int((s1 - s0) / 0.3) + 1):
                q = at(lx, so + depth * 0.35)
                mb.box((q[0], s0 + i * 0.3, q[1]), (0.4, 0.03, 0.03), mat, yaw)
    for k in ks:
        y = self.levels[k] - 0.02
        p = at(c, so + depth / 2)
        # frame, slats across (the lattice seen from below), cross bearers
        for off in (so + 0.03, so + depth - 0.03):
            q = at(c, off)
            mb.box((q[0], y - 0.04, q[1]), (w, 0.1, 0.05), mat, yaw)
        for i in range(int(w / 0.09) + 1):
            q = at(c - w / 2 + 0.03 + i * (w - 0.06) / int(w / 0.09), so + depth / 2)
            mb.box((q[0], y, q[1]), (0.025, 0.03, depth), mat, yaw)
        for off in (so + depth * 0.33, so + depth * 0.66):
            q = at(c, off)
            mb.box((q[0], y - 0.03, q[1]), (w, 0.02, 0.02), mat, yaw)
        # outriggers back to the wall at both ends
        for side in (-1, 1):
            q = at(c + side * (w / 2 - 0.03), (so + depth) / 2)
            mb.box((q[0], y - 0.06, q[1]), (0.05, 0.1, so + depth), mat, yaw)
        # railing: top and mid rails, posts, balusters
        for yy in (y + 1.0, y + 0.5):
            q = at(c, so + depth)
            mb.box((q[0], yy, q[1]), (w, 0.04, 0.04), mat, yaw)
            for side in (-1, 1):
                q2 = at(c + side * w / 2, so + depth / 2)
                mb.box((q2[0], yy, q2[1]), (0.04, 0.04, depth), mat, yaw)
        for i in range(int(w / 0.12) + 1):
            q = at(c - w / 2 + i * w / int(w / 0.12), so + depth)
            mb.box((q[0], y + 0.5, q[1]), (0.018, 1.0, 0.018), mat, yaw)
        # ladder: up to the next platform, or the drop ladder below the lowest
        lx = c + (w / 2 - 0.4) * (1 if k % 2 else -1)
        ya, yb = (y, self.levels[k + 1] - 0.02) if (k + 1) in ks else (None, None)
        spans = []
        if ya is not None:
            spans.append((lx, ya, yb))
        if k == min(ks) and spec.get('dropLadder', True):
            spans.append((c - (w / 2 - 0.4), max(1.3, y - 2.2), y))
        for sx, s0, s1 in spans:
            for side in (-1, 1):
                q = at(sx + side * 0.2, so + depth * 0.5)
                mb.box((q[0], (s0 + s1) / 2, q[1]), (0.04, s1 - s0, 0.05), mat, yaw)
            for i in range(1, int((s1 - s0) / 0.3) + 1):
                q = at(sx, so + depth * 0.5)
                mb.box((q[0], s0 + i * 0.3, q[1]), (0.4, 0.025, 0.025), mat, yaw)
    mb.build(self.exterior)


Building._fire_escape = _gb_fire_escape

# storefront.layout[] item keys (front edge):
# `leafClosed: {panelRows, panelCols, knobTint}` - the door drawn as a closed
# panelled leaf (the item's `leaf` material) in a casing of the style's trim,
# with a transom light over it. The physics opening is unchanged (walkable).
# `entry: {recessM, surround, surroundTint, gilt, gate, gateTint, floor,
# floorTint, vestibuleTint, archRiseM, innerLeaf}` - a recessed entrance vestibule: side walls,
# ceiling and tiled floor, an inner doorway with its leaves standing open, an
# arched head with a keystone, pilasters with gilt capitals and an entablature
# on the facade, and a two-leaf iron gate folded open against the vestibule
# walls. The vestibule walls are physics solids; the passage stays walkable.
_gb_dress_base = Building._dress_openings
_GB_KINDS = ('gb-door', 'gb-entry')


def _gb_retag(self, e, windows):
    if not (self.is_street(e) and e['role'] == 'front'):
        return windows
    sf = self.edge_style(e).get('storefront') or {}
    L = e['lengthM']
    tags = []
    for item in sf.get('layout') or []:
        if item.get('leafClosed') or item.get('entry'):
            s0, s1 = sorted((self.f2s(item['to'], L, e), self.f2s(item['from'], L, e)))
            tags.append((s0, s1, item))
    if not tags:
        return windows
    out = []
    for kind, sp in windows:
        if kind == 'storefront-door':
            hit = next((t for t in tags if abs(t[0] - sp[0]) < 1e-6 and abs(t[1] - sp[1]) < 1e-6), None)
            if hit:
                out.append(('gb-entry' if hit[2].get('entry') else 'gb-door', tuple(sp) + (hit[2],)))
                continue
        out.append((kind, sp))
    return out


def _gb_dress(self, e, a, b, windows, trim, glass, frames, store, es=None):
    mine = [w for w in windows if w[0] in _GB_KINDS]
    _gb_dress_base(self, e, a, b, [w for w in windows if w[0] not in _GB_KINDS], trim, glass, frames, store, es)
    if not mine:
        return
    s = es or self.style
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    sf = s['storefront']
    for kind, sp in mine:
        s0, s1, head, leaf, item = sp[0], sp[1], sp[3], sp[6], sp[-1]
        door_h = min(2.45, head - sf['transomM'] * 0.6)
        w, cs = s1 - s0, (s0 + s1) / 2
        if kind == 'gb-door':
            lc = item['leafClosed'] if isinstance(item['leafClosed'], dict) else {}
            p = at(cs, -0.14)
            store.box((p[0], door_h / 2, p[1]), (w - 0.06, door_h, 0.05), leaf, yaw)
            pr, pc = int(lc.get('panelRows', 3)), int(lc.get('panelCols', 2))
            pw, ph = (w - 0.06 - 0.12 * (pc + 1)) / pc, (door_h - 0.14 * (pr + 1)) / pr
            # `panelTint` - raised panels in their own tint inside a
            # darker bolection moulding, on both faces; `grille` - the top
            # row is a glazed light behind an iron grille
            ptm = self.tinted(leaf.split('@')[0], lc['panelTint']) if lc.get('panelTint') else leaf
            mould = self.tinted(leaf.split('@')[0], lc['mouldTint']) if lc.get('mouldTint') else leaf
            iron = self.tinted('painted-steel', lc.get('grilleTint'))
            if pw > 0.05 and ph > 0.1:
                for i in range(pc):
                    for r in range(pr):
                        u = s0 + 0.03 + 0.12 * (i + 1) + pw * (i + 0.5)
                        yy = 0.14 * (r + 1) + ph * (r + 0.5)
                        if lc.get('grille') and r == pr - 1:
                            continue
                        for off in ((-0.11, -0.17) if lc.get('panelTint') else (-0.11,)):
                            q = at(u, off)
                            store.box((q[0], yy, q[1]), (pw, ph, 0.02), ptm, yaw)
                            if lc.get('mouldTint'):
                                for dy in (-1, 1):
                                    store.box((q[0], yy + dy * (ph / 2 + 0.015), q[1]), (pw + 0.06, 0.03, 0.03), mould, yaw)
                                for dx in (-1, 1):
                                    q2 = at(u + dx * (pw / 2 + 0.015), off)
                                    store.box((q2[0], yy, q2[1]), (0.03, ph + 0.06, 0.03), mould, yaw)
                if lc.get('grille'):
                    gw, gh = w - 0.06 - 0.24, ph
                    gy = 0.14 * pr + ph * (pr - 0.5)
                    g = at(cs, -0.115)
                    glass.box((g[0], gy, g[1]), (gw, gh, 0.02), s['glass'], yaw)
                    dark = at(cs, -0.16)
                    store.box((dark[0], gy, dark[1]), (gw, gh, 0.01), self.tinted('painted-steel', '#141414'), yaw)
                    nb = max(3, int(gw / 0.07))
                    for j in range(1, nb):
                        q = at(cs - gw / 2 + gw * j / nb, -0.09)
                        store.box((q[0], gy, q[1]), (0.018, gh, 0.02), iron, yaw)
                    for yy in (gy - gh / 2 + 0.02, gy, gy + gh / 2 - 0.02):
                        q = at(cs, -0.085)
                        store.box((q[0], yy, q[1]), (gw, 0.025, 0.02), iron, yaw)
            q = at(s0 + 0.12 if lc.get('hinge') == 'right' else s1 - 0.12, -0.09)
            store.box((q[0], 1.0, q[1]), (0.06, 0.06, 0.06), self.tinted('painted-white', lc.get('knobTint', '#c9a24a')), yaw)
            # transom light, head rail, casing
            if head - door_h > 0.12:
                g = at(cs, -0.14)
                glass.box((g[0], (door_h + head) / 2, g[1]), (w - 0.06, head - door_h, 0.02), s['glass'], yaw)
                store.box((g[0], door_h + 0.03, g[1]), (w, 0.06, 0.08), s['trim'], yaw)
            for sv in (s0 - 0.04, s1 + 0.04):
                q = at(sv, 0.02)
                store.box((q[0], head / 2, q[1]), (0.14, head, 0.06), s['trim'], yaw)
            q = at(cs, 0.02)
            store.box((q[0], head + 0.06, q[1]), (w + 0.28, 0.12, 0.07), s['trim'], yaw)
            continue
        en = item['entry']
        if en.get('style') == 'shouldered':
            _gb_entry_shouldered(self, en, s, s0, s1, head, door_h, at, d, nout, yaw, store, glass)
            continue
        R = float(en.get('recessM', 1.4))
        cream = self.tinted(en.get('surround', 'cast-stone-trim'), en.get('surroundTint'))
        gilt = self.tinted('painted-white', en.get('gilt', '#c9a24a'))
        iron = self.tinted(en.get('gate', 'painted-steel'), en.get('gateTint'))
        floor = self.tinted(en.get('floor', 'terracotta'), en.get('floorTint'))
        D = WALL_T + R
        vest = self.tinted(en.get('surround', 'cast-stone-trim'), en.get('vestibuleTint', en.get('surroundTint')))
        # vestibule: side walls, ceiling, floor
        for i, sv in enumerate((s0 + 0.05, s1 - 0.05)):
            p = at(sv, -D / 2)
            store.box((p[0], head / 2, p[1]), (0.1, head, D), vest, yaw)
            self.physics['solids'].append({'id': f'vestibule-{i}', 'center': {'x': p[0], 'y': head / 2, 'z': p[1]},
                                           'size': {'x': 0.1, 'y': head, 'z': D}, 'yaw': yaw})
        p = at(cs, -D / 2)
        store.box((p[0], head - 0.04, p[1]), (w, 0.08, D), vest, yaw)
        store.box((p[0], 0.015, p[1]), (w - 0.1, 0.03, D), floor, yaw)
        # back wall with the inner doorway, its leaves standing open inward
        iw = min(1.3, w - 0.5)
        idh = min(door_h, head - 0.35)
        for p0, p1 in ((s0 + 0.1, cs - iw / 2), (cs + iw / 2, s1 - 0.1)):
            q = at((p0 + p1) / 2, -D + 0.05)
            store.box((q[0], head / 2, q[1]), (p1 - p0, head, 0.1), vest, yaw)
        q = at(cs, -D + 0.05)
        store.box((q[0], (idh + head) / 2, q[1]), (iw, head - idh, 0.1), vest, yaw)
        inner = en.get('innerLeaf', 'wood-siding@7a5236')
        for hs, sgn in ((cs - iw / 2 + 0.03, 1), (cs + iw / 2 - 0.03, -1)):
            lw = iw / 2 - 0.04
            hinge = at(hs, -D)
            c = add(hinge, mul(nout, -lw / 2))
            store.box((c[0], idh / 2, c[1]), (0.05, idh - 0.02, lw), inner, yaw)
            g2 = add(c, mul(d, 0.03 * sgn))
            glass.box((g2[0], idh * 0.62, g2[1]), (0.02, idh * 0.45, lw * 0.65), s['glass'], yaw)
        # arched head inside the opening: spandrels, archivolt, keystone
        rise = min(w / 2, float(en.get('archRiseM', 0.6)))
        spring = head - rise
        n = 20
        for i in range(n):
            u0, u1 = -1 + 2 * i / n, -1 + 2 * (i + 1) / n
            um = (u0 + u1) / 2
            ya = spring + rise * math.sqrt(max(0.0, 1 - um * um))
            if head - ya > 0.02:
                q = at(cs + um * w / 2, -0.12)
                store.box((q[0], (ya + head) / 2, q[1]), (w / n + 0.01, head - ya, 0.24), cream, yaw)
            q = at(cs + um * w / 2, 0.01)
            store.box((q[0], ya - 0.02, q[1]), (w / n + 0.02, 0.07, 0.05), gilt, yaw)
        q = at(cs, 0.04)
        store.box((q[0], head - 0.08, q[1]), (0.24, 0.32, 0.1), gilt, yaw)
        # pilasters with bases and gilt capitals, entablature, cornice cap
        for sv in (s0 - 0.15, s1 + 0.15):
            q = at(sv, 0.05)
            store.box((q[0], head / 2, q[1]), (0.3, head, 0.1), cream, yaw)
            store.box((q[0], 0.16, q[1]), (0.36, 0.32, 0.14), cream, yaw)
            store.box((q[0], head - 0.1, q[1]), (0.38, 0.16, 0.15), gilt, yaw)
            for k in (-1, 1):
                q2 = add(q, mul(d, k * 0.08))
                store.box((q2[0], head * 0.55, q2[1]), (0.03, head * 0.7, 0.12), gilt, yaw)
        q = at(cs, 0.06)
        store.box((q[0], head + 0.17, q[1]), (w + 0.7, 0.34, 0.12), cream, yaw)
        store.box((q[0], head + 0.3, q[1]), (w + 0.74, 0.05, 0.15), gilt, yaw)
        store.box((q[0], head + 0.06, q[1]), (w + 0.7, 0.04, 0.14), gilt, yaw)
        q = at(cs, 0.1)
        store.box((q[0], head + 0.4, q[1]), (w + 0.84, 0.12, 0.2), cream, yaw)
        # iron gate, two leaves folded open against the vestibule walls
        gl = min(w / 2 - 0.06, D - 0.2)
        gh = max(1.8, spring)
        for sv in (s0 + 0.13, s1 - 0.13):
            for yy in (0.12, gh * 0.5, gh - 0.04):
                q = at(sv, -(0.1 + gl / 2))
                store.box((q[0], yy, q[1]), (0.03, 0.04, gl), iron, yaw)
            for j in range(int(gl / 0.11) + 1):
                q = at(sv, -(0.1 + j * gl / int(gl / 0.11)))
                store.box((q[0], gh / 2, q[1]), (0.022, gh, 0.022), iron, yaw)
                if j % 2 == 0:
                    store.box((q[0], gh + 0.06, q[1]), (0.03, 0.12, 0.03), iron, yaw)


# `entry.style: "shouldered"` (+ `field`, `fieldTint`, `backTint`,
# `friezeM`) - the entrance as the facade's showpiece: a dark recessed
# vestibule (walls, ceiling, floor in `vestibuleTint` / `floorTint`), a closed
# ornamental iron gate across its back in front of dark closed inner leaves,
# a flat head with stepped shoulders in the top corners, polychrome
# pilasters (cream stiles, a coloured sunk field with a gilt bead and
# rosettes, gilt capitals) and a frieze of gilt volutes and a centre relief
# over the head under a cream cap. The passage stays walkable: the gate is
# drawn, not a physics solid (the runtime has no openable door).
def _gb_entry_shouldered(self, en, s, s0, s1, head, door_h, at, d, nout, yaw, store, glass):
    R = float(en.get('recessM', 1.4))
    w, cs = s1 - s0, (s0 + s1) / 2
    cream = self.tinted(en.get('surround', 'cast-stone-trim'), en.get('surroundTint'))
    gilt = self.tinted('painted-white', en.get('gilt', '#c9a24a'))
    iron = self.tinted(en.get('gate', 'painted-steel'), en.get('gateTint'))
    field = self.tinted(en.get('field', 'painted-white'), en.get('fieldTint', '#8e2a22'))
    vest = self.tinted('painted-white', en.get('vestibuleTint', '#3a322c'))
    floor = self.tinted(en.get('floor', 'terracotta'), en.get('floorTint'))
    back_leaf = self.tinted('painted-white', en.get('backTint', '#1e1915'))
    D = WALL_T + R
    for i, sv in enumerate((s0 + 0.05, s1 - 0.05)):
        p = at(sv, -D / 2)
        store.box((p[0], head / 2, p[1]), (0.1, head, D), vest, yaw)
        self.physics['solids'].append({'id': f'vestibule-{i}', 'center': {'x': p[0], 'y': head / 2, 'z': p[1]},
                                       'size': {'x': 0.1, 'y': head, 'z': D}, 'yaw': yaw})
    p = at(cs, -D / 2)
    store.box((p[0], head - 0.04, p[1]), (w, 0.08, D), vest, yaw)
    store.box((p[0], 0.015, p[1]), (w - 0.1, 0.03, D), floor, yaw)
    # back wall with the inner doorway, closed dark leaves in it
    iw = min(1.3, w - 0.5)
    idh = min(door_h, head - 0.3)
    for p0, p1 in ((s0 + 0.1, cs - iw / 2), (cs + iw / 2, s1 - 0.1)):
        q = at((p0 + p1) / 2, -D + 0.05)
        store.box((q[0], head / 2, q[1]), (p1 - p0, head, 0.1), vest, yaw)
    q = at(cs, -D + 0.05)
    store.box((q[0], (idh + head) / 2, q[1]), (iw, head - idh, 0.1), vest, yaw)
    q = at(cs, -D + 0.03)
    store.box((q[0], idh / 2, q[1]), (iw, idh, 0.04), back_leaf, yaw)
    for sgn in (-1, 1):
        q = at(cs + sgn * iw / 4, -D + 0.06)
        glass.box((q[0], idh * 0.64, q[1]), (iw / 2 - 0.2, idh * 0.42, 0.02), s['glass'], yaw)
    # closed ornamental iron gate across the vestibule, 0.35 m before the back
    gz = -D + 0.4
    gw, gh = w - 0.2, head - 0.12
    for yy in (0.08, gh * 0.33, gh * 0.66, gh - 0.03):
        q = at(cs, gz)
        store.box((q[0], yy, q[1]), (gw, 0.04, 0.03), iron, yaw)
    n = max(4, int(gw / 0.1))
    for j in range(n + 1):
        q = at(cs - gw / 2 + gw * j / n, gz)
        store.box((q[0], gh / 2, q[1]), (0.022, gh, 0.022), iron, yaw)
    q = at(cs, gz)
    store.box((q[0], gh / 2, q[1]), (0.05, gh, 0.035), iron, yaw)
    # scrollwork band between the rails: rings of small bars
    for j in range(n):
        uc = cs - gw / 2 + gw * (j + 0.5) / n
        for ang in range(8):
            a2 = ang * math.pi / 4
            q = at(uc + 0.035 * math.cos(a2), gz)
            store.box((q[0], gh * 0.495 + 0.035 * math.sin(a2), q[1]), (0.018, 0.018, 0.018), iron, yaw)
    # flat head with stepped shoulders in both top corners
    for sgn in (-1, 1):
        for k2, (sw, sh2) in enumerate(((0.26, 0.1), (0.14, 0.12))):
            u = (s0 + 0.1 + sw / 2) if sgn < 0 else (s1 - 0.1 - sw / 2)
            q = at(u, -0.1)
            store.box((q[0], head - 0.05 - sh2 / 2 - 0.1 * k2, q[1]), (sw, sh2, 0.22), cream, yaw)
            q = at(u, 0.0)
            store.box((q[0], head - 0.05 - 0.1 * k2 - sh2, q[1]), (sw, 0.025, 0.03), gilt, yaw)
    q = at(cs, -0.1)
    store.box((q[0], head - 0.03, q[1]), (w, 0.08, 0.24), cream, yaw)
    # polychrome pilasters
    pwid = 0.36
    for sv in (s0 - pwid / 2 + 0.02, s1 + pwid / 2 - 0.02):
        q = at(sv, 0.05)
        store.box((q[0], head / 2, q[1]), (pwid, head, 0.1), cream, yaw)
        store.box((q[0], 0.18, q[1]), (pwid + 0.06, 0.36, 0.14), cream, yaw)
        q2 = at(sv, 0.105)
        store.box((q2[0], (0.42 + head - 0.3) / 2, q2[1]), (pwid - 0.14, head - 0.72, 0.02), field, yaw)
        store.box((q2[0], (0.42 + head - 0.3) / 2, q2[1]), (0.03, head - 0.76, 0.035), gilt, yaw)
        yy = 0.55
        while yy < head - 0.35:
            q3 = at(sv, 0.12)
            store.box((q3[0], yy, q3[1]), (0.13, 0.13, 0.03), gilt, yaw)
            for k2 in (-1, 1):
                q4 = at(sv + k2 * 0.07, 0.12)
                store.box((q4[0], yy + 0.1, q4[1]), (0.04, 0.06, 0.025), gilt, yaw)
            yy += 0.3
        for k2 in (-1, 1):
            q4 = at(sv + k2 * (pwid / 2 - 0.045), 0.11)
            store.box((q4[0], head / 2 + 0.1, q4[1]), (0.025, head - 0.6, 0.025), gilt, yaw)
        q = at(sv, 0.07)
        store.box((q[0], head - 0.12, q[1]), (pwid + 0.08, 0.2, 0.16), gilt, yaw)
    # frieze over the head: cream frame, coloured field, gilt volutes and relief
    fh = float(en.get('friezeM', 0.46))
    fw = w + 2 * pwid + 0.04
    y0 = head + 0.02
    q = at(cs, 0.05)
    store.box((q[0], y0 + fh / 2, q[1]), (fw, fh, 0.1), cream, yaw)
    qf = at(cs, 0.0)
    fd = d
    fn = nout
    _gb_fbox(store, qf, fd, fn, 0, y0 + fh / 2, 0.105, (fw - 0.16, fh - 0.12, 0.02), field, yaw)
    for yy in (y0 + 0.05, y0 + fh - 0.05):
        _gb_fbox(store, qf, fd, fn, 0, yy, 0.11, (fw - 0.1, 0.04, 0.035), gilt, yaw)
    _gb_figure(store, qf, fd, fn, 0, y0 + fh / 2, fh * 0.8, 0.12, gilt, yaw)
    nv = max(2, int((fw - 0.5) / 0.42))
    for j in range(nv):
        u = -fw / 2 + 0.3 + (fw - 0.6) * (j + 0.5) / nv
        if abs(u) < 0.25:
            continue
        _gb_scroll(store, qf, fd, fn, u, y0 + fh / 2, min(0.13, fh * 0.28), 0.12, gilt, yaw, sense=1 if u < 0 else -1)
    q = at(cs, 0.1)
    store.box((q[0], y0 + fh + 0.06, q[1]), (fw + 0.16, 0.12, 0.2), cream, yaw)
    store.box((q[0], y0 + fh + 0.005, q[1]), (fw + 0.1, 0.03, 0.18), gilt, yaw)


_gb_openings_r2_base = Building.openings_for_edge


def _gb_openings_r2(self, e):
    ops, windows, doors = _gb_openings_r2_base(self, e)
    return ops, _gb_retag(self, e, windows), doors


Building.openings_for_edge = _gb_openings_r2
Building._dress_openings = _gb_dress

# `edgeWalls: {"<edge index>" | "*": {material, tint}}` - the wall finish on
# non-street edges (a stucco rear, a clapboard party return standing above a
# lower neighbour); a listed index wins over "*".
_gb_edge_style_base = Building.edge_style


def _gb_edge_style(self, e):
    es = _gb_edge_style_base(self, e)
    ew = self.style.get('edgeWalls')
    if not ew or self.is_street(e):
        return es
    w = ew.get(str(e['index'])) or ew.get('*')
    if not w:
        return es
    return dict(es, wall=w.get('material', es['wall']), wallTint=w.get('tint'))


Building.edge_style = _gb_edge_style
Building.ground_programme = _gb_ground_programme
Building.build_interior = _gb_build_interior
Building._vacant = _gb_vacant
Building._closed_infill = _gb_closed_infill


# Every key below is new; absent = unchanged builds.
# `dentilCornice: {material, tint, projectionM, friezeM, dentilM}` - a deep
# painted cornice across the whole front: a flat frieze, a bed mould, a
# dentil course, a corona and a cap (the bays rise into the frieze).
# `plinth: {heightM, projectionM, material, tint}` - a painted base band along
# the front at the pavement, broken at the doorways.
_gb_edge_trim_base = Building._edge_trim


def _gb_edge_trim(self, e, a, b, trim):
    _gb_edge_trim_base(self, e, a, b, trim)
    s = self.style
    if e['role'] != 'front' or not self.is_street(e):
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    dc = s.get('dentilCornice')
    if dc:
        m = self.tinted(dc.get('material', 'painted-white'), dc.get('tint'))
        top = self.height
        proj = float(dc.get('projectionM', 0.45))
        fh = float(dc.get('friezeM', 0.6))
        mb = MeshBuilder('dentil-cornice')
        for y, hgt, off in ((top - 0.42 - fh, fh, 0.08), (top - 0.42, 0.08, proj * 0.6),
                            (top - 0.2, 0.18, proj), (top - 0.02, 0.07, proj * 0.85)):
            p = at(L / 2, off / 2)
            mb.box((p[0], y + hgt / 2, p[1]), (L, hgt, off + 0.02), m, yaw)
        step = float(dc.get('dentilM', 0.22))
        n = int(L / step)
        for i in range(n):
            p = at((L - n * step) / 2 + step * (i + 0.5), proj * 0.42)
            mb.box((p[0], top - 0.27, p[1]), (step * 0.55, 0.14, proj * 0.84), m, yaw)
        mb.build(self.exterior)
    pl = s.get('plinth')
    if pl:
        m = self.tinted(pl.get('material', 'painted-white'), pl.get('tint'))
        hh, pr = float(pl.get('heightM', 0.3)), float(pl.get('projectionM', 0.05))
        cuts = []
        for item in (s.get('storefront') or {}).get('layout') or []:
            if item['kind'] != 'window':
                cuts.append(tuple(sorted((self.f2s(item['to'], L, e), self.f2s(item['from'], L, e)))))
        cuts.sort()
        mb = MeshBuilder('plinth')
        cur = 0.0
        for c0, c1 in cuts + [(L, L)]:
            if c0 - cur > 0.05:
                p = at((cur + c0) / 2, pr / 2)
                mb.box((p[0], hh / 2, p[1]), (c0 - cur, hh, pr + 0.02), m, yaw)
            cur = max(cur, c1)
        mb.build(self.exterior)


Building._edge_trim = _gb_edge_trim

# `interiorDoors: {material, tint}` - upper-floor room doorways get a
# panelled leaf standing open against the room side of the partition and a
# casing on both faces (drawn only; the doorways stay walkable).
# `noSconces: true` - no corridor wall sconces (the corridor keeps its
# ceiling lamps and light).
_gb_partition_base = partition
_GB_DOORS = {'mat': None}


def _gb_partition(mb, a, b, base, top, doors, mat='int-plaster', t=PART_T):
    out = _gb_partition_base(mb, a, b, base, top, doors, mat, t)
    dm = _GB_DOORS['mat']
    if not dm or not doors:
        return out
    L = length(sub(b, a))
    d = norm(sub(b, a))
    perp = (-d[1], d[0])
    h = min(top - 0.05, base + DOOR_H) - base
    for sv in doors:
        # the leaf swung right round, flat against the wall beside the opening
        if sv - DOOR_W / 2 - 0.95 < 0:
            continue
        c = add(lerp2(a, b, (sv - DOOR_W / 2 - 0.52) / L), mul(perp, t / 2 + 0.045))
        mb.box((c[0], base + h / 2, c[1]), (0.84, h - 0.02, 0.04), dm, box_yaw(d))
        for yy in (base + h * 0.3, base + h * 0.72):
            c2 = add(c, mul(perp, 0.02))
            mb.box((c2[0], yy, c2[1]), (0.6, h * 0.3, 0.02), dm, box_yaw(d))
        for side in (-1, 1):
            for jam in (sv - DOOR_W / 2 - 0.04, sv + DOOR_W / 2 + 0.04):
                q = add(lerp2(a, b, jam / L), mul(perp, side * (t / 2 + 0.01)))
                mb.box((q[0], base + h / 2, q[1]), (0.08, h, 0.02), 'int-wood-light', box_yaw(d))
            q = add(lerp2(a, b, sv / L), mul(perp, side * (t / 2 + 0.01)))
            mb.box((q[0], base + h + 0.04, q[1]), (DOOR_W + 0.16, 0.08, 0.02), 'int-wood-light', box_yaw(d))
    return out


partition = _gb_partition
_gb_upper_base = Building._upper
_gb_piece_base = Building.piece


def _gb_upper(self, walls, k):
    idoor = self.style.get('interiorDoors')
    _GB_DOORS['mat'] = self.tinted(idoor.get('material', 'painted-white'), idoor.get('tint')) if idoor else None
    self._gb_no_sconce = bool(self.style.get('noSconces'))
    try:
        return _gb_upper_base(self, walls, k)
    finally:
        _GB_DOORS['mat'] = None
        self._gb_no_sconce = False


def _gb_piece(self, kind, *args, **kw):
    if kind == 'wall-sconce' and getattr(self, '_gb_no_sconce', False):
        return None
    return _gb_piece_base(self, kind, *args, **kw)


Building._upper = _gb_upper
Building.piece = _gb_piece


# ---------------------------------------------------------------- main


# `glassModel` (style or a streetEdges override; absent = unchanged): the
# Reflection-dominant glazing. A daylit window seen from the street
# is the sky and the street mirrored over a dim room, never a lit white box.
#   "reflective"  or  {upper, street, backing, bays}
#   upper    kit glass for every upper window, frame-grid bay and door light
#            (default 'glass-reflective'; opaque, so nothing is drawn behind)
#   street   kit glass for shopfront / lobby / transom glazing (default
#            'glass-storefront'; blended, a seventh of the shop shows through)
#   backing  keep `glassBacking` room-tone planes (default false: an opaque
#            reflective pane hides them, and they were the lit-box read)
#   bays     glazed projecting bays take `upper` too (default true)
# Either value may carry a `@rrggbb` tint, which scales reflectance and body
# together (darker = less sky, more road).
def apply_glass_model(style):
    gm = style.get('glassModel')
    if not gm:
        return style
    if not isinstance(gm, dict):
        gm = {}
    style['glass'] = gm.get('upper', 'glass-reflective')
    style['storefrontGlass'] = gm.get('street', 'glass-storefront')
    if not gm.get('backing', False):
        style.pop('glassBacking', None)
    if gm.get('bays', True) and style.get('bays'):
        style['bays'] = [{**b, 'glass': style['glass']} if b.get('glazed') else b for b in style['bays']]
    return style


# `upperFit` (style; absent/false = unchanged) - bldg-222862957, 2026-09-28.
# A shallow plan (about 8 m deep) under a tall ground storey: the stair core's
# long flight fills most of the depth, so the corridor strip beside it starts
# at or past the back wall and `_upper` lays a room and the landing partition
# outside the shell (build REJECT: a bed frame leaves the footprint AABB).
# With `upperFit: true`, inside `_upper` a room shorter than
# `upperFitMinRoomM` (default 2.4 m) or not inside the plan rectangle, and a
# partition with an end outside it, are left out. Other buildings unchanged.
_uf_upper_base = Building._upper
def _uf_upper(self, walls, k):
    if not self.style.get('upperFit'):
        return _uf_upper_base(self, walls, k)
    x0, x1, z0, z1 = self.rect
    tol = 0.05
    min_room = float(self.style.get('upperFitMinRoomM', 2.4))
    g = globals()
    part_base = g['partition']
    room_base = self._room

    def inside(p):
        return x0 - tol <= p[0] <= x1 + tol and z0 - tol <= p[1] <= z1 + tol

    def fitted_partition(mb, a, b, *args, **kw):
        if not (inside(a) and inside(b)):
            return []
        return part_base(mb, a, b, *args, **kw)

    def fitted_room(r, *args, **kw):
        rx0, rx1, rz0, rz1 = r
        if rz1 - rz0 < min_room or not (inside((rx0, rz0)) and inside((rx1, rz1))):
            return None
        return room_base(r, *args, **kw)

    g['partition'] = fitted_partition
    self._room = fitted_room
    try:
        return _uf_upper_base(self, walls, k)
    finally:
        g['partition'] = part_base
        del self._room


Building._upper = _uf_upper




# `rotunda.round` (absent = unchanged): the corner
# rotunda's pilotis as true round columns and the underside of the floors
# that overhang it.
#   {"columns": n, "diameterM": d, "sides": 16, "material": kit@tint,
#    "soffit": kit@tint, "fascia": kit@tint, "fasciaM": h, "at": [t, ...]}
# Columns stand on the old corner line between the curve and the footprint
# corner (at the listed fractions of the sweep, default evenly spaced), from
# the pavement to the rotunda head; `soffit` closes the overhang's underside
# at the rotunda head (a fan from the corner to the curve) and `fascia` runs
# a band `fasciaM` tall around its lip. Set `rotunda.columns` to 0 when this
# is used, so the kit's square columns do not draw as well.
def _g2_rotunda_round(self, inset=None):
    spec = self.style.get('rotunda') or {}
    rs = spec.get('round')
    if not rs:
        return
    n = len(self.poly)
    v = int(spec['vertex']) % n
    r = spec.get('radiusM', 3.0)
    V = self.poly[v]
    u_in = norm(sub(self.poly[(v - 1) % n], V))
    u_out = norm(sub(self.poly[(v + 1) % n], V))
    O = add(V, add(mul(u_in, r), mul(u_out, r)))
    A, C = add(V, mul(u_in, r)), add(V, mul(u_out, r))
    a0 = math.atan2(A[1] - O[1], A[0] - O[0])
    a1 = math.atan2(C[1] - O[1], C[0] - O[0])
    da = (a1 - a0 + math.pi) % (2 * math.pi) - math.pi
    top = self._rotunda_cut({'index': v, 'lengthM': 1.0})[2]
    mb = MeshBuilder('rotunda-pilotis')
    cols = int(rs.get('columns', 0))
    d = float(rs.get('diameterM', 0.6))
    sides = max(8, int(rs.get('sides', 16)))
    cm = rs.get('material', 'concrete-precast')
    ats = rs.get('at') or [0.2 + 0.6 * i / max(1, cols - 1) for i in range(cols)]
    for t in ats[:max(0, cols)]:
        ang = a0 + da * t
        edge_pt = add(O, (r * math.cos(ang), r * math.sin(ang)))
        c = lerp2(edge_pt, V, float(rs.get('towardCorner', 0.55)))
        if not self._inside_outer(c, d / 2 + 0.05):
            continue
        ring = [(c[0] + d / 2 * math.cos(2 * math.pi * k / sides), c[1] + d / 2 * math.sin(2 * math.pi * k / sides)) for k in range(sides)]
        self._prism(mb, ring, 0.0, top + 0.02, cm)
    segs = max(6, int(spec.get('segments', 12)))
    pts = [add(O, (r * math.cos(a0 + da * i / segs), r * math.sin(a0 + da * i / segs))) for i in range(segs + 1)]
    if inset:
        # the soffit runs from the wall lines' ends over the set-back drum
        rr = inset[2]
        pts = [A] + [add(O, (rr * math.cos(a0 + da * i / segs), rr * math.sin(a0 + da * i / segs))) for i in range(segs + 1)] + [C]
        segs = len(pts) - 1
    if rs.get('soffit'):
        down = B(0, -1, 0)
        for i in range(segs):
            p, q = pts[i], pts[i + 1]
            mb.quad([B(V[0], top, V[1]), B(p[0], top, p[1]), B(q[0], top, q[1])], rs['soffit'], down)
    if rs.get('fascia'):
        fh = float(rs.get('fasciaM', 0.35))
        # the lip of the overhang: the two footprint edges from A to V to C
        for p, q in ((A, V), (V, C)):
            mid = lerp2(p, q, 0.5)
            out = B(mid[0] - O[0], 0, mid[1] - O[1])
            mb.quad([B(p[0], top - fh, p[1]), B(q[0], top - fh, q[1]), B(q[0], top + 0.01, q[1]), B(p[0], top + 0.01, p[1])], rs['fascia'], out)
    if not mb.empty():
        mb.build(self.exterior)


_g2_rotunda_base = Building._rotunda


# `rotunda.round.insetM` (absent or 0 = unchanged): the
# glass drum stands `insetM` behind both street wall lines instead of meeting
# them, so the floors above overhang it all round (a deep shadowed soffit, as
# a 1980s corner office's glass drum reads), with short return walls where the
# wall lines stop. `litBand` {glass, fromM, toM} lays a lit-interior pane band
# just outside the drum glass (e.g. a warm lobby seen through the lower panes).
def _g2_rotunda_inset(self, spec, rs):
    n = len(self.poly)
    v = int(spec['vertex']) % n
    r = spec.get('radiusM', 3.0)
    ins = min(float(rs.get('insetM', 0)), r * 0.6)
    V = self.poly[v]
    u_in = norm(sub(self.poly[(v - 1) % n], V))
    u_out = norm(sub(self.poly[(v + 1) % n], V))
    O = add(V, add(mul(u_in, r), mul(u_out, r)))
    A, C = add(V, mul(u_in, r)), add(V, mul(u_out, r))
    a0 = math.atan2(A[1] - O[1], A[0] - O[0])
    a1 = math.atan2(C[1] - O[1], C[0] - O[0])
    da = (a1 - a0 + math.pi) % (2 * math.pi) - math.pi
    top = self._rotunda_cut({'index': v, 'lengthM': 1.0})[2]
    segs = max(6, int(spec.get('segments', 12)))
    rr = r - ins
    pts = [add(O, (rr * math.cos(a0 + da * i / segs), rr * math.sin(a0 + da * i / segs))) for i in range(segs + 1)]
    glass = MeshBuilder('rotunda-glazing')
    frames = MeshBuilder('rotunda-frames')
    ret = MeshBuilder('rotunda-returns')
    gmat, fmat = spec.get('glass', self.style['storefrontGlass']), spec.get('frame', self.style['storefrontFrame'])
    levels = [0.0] + [y for y in self.levels[1:] if y < top - 0.2] + [top]
    pitch = spec.get('transomM', 0)
    if pitch and pitch > 0.3:
        extra = []
        for y0, y1 in zip(levels[:-1], levels[1:]):
            k = max(1, int(round((y1 - y0) / pitch)))
            extra += [y0 + (y1 - y0) * j / k for j in range(1, k)]
        levels = sorted(levels + extra)
    lit = rs.get('litBand')
    for i in range(segs):
        p, q = pts[i], pts[i + 1]
        mid = lerp2(p, q, 0.5)
        out = B(mid[0] - O[0], 0, mid[1] - O[1])
        glass.quad([B(p[0], 0.0, p[1]), B(q[0], 0.0, q[1]), B(q[0], top, q[1]), B(p[0], top, p[1])], gmat, out)
        if lit:
            o = norm((mid[0] - O[0], mid[1] - O[1]))
            p2, q2 = add(p, mul(o, 0.015)), add(q, mul(o, 0.015))
            y0, y1 = float(lit.get('fromM', 2.2)), min(top, float(lit.get('toM', 3.6)))
            glass.quad([B(p2[0], y0, p2[1]), B(q2[0], y0, q2[1]), B(q2[0], y1, q2[1]), B(p2[0], y1, p2[1])], lit.get('glass', 'glass-door'), out)
        yaw = box_yaw(norm(sub(q, p)))
        for y in levels:
            frames.box((mid[0], y, mid[1]), (length(sub(q, p)) + 0.02, 0.08, 0.1), fmat, yaw, skip=('+x', '-x'))
    for p in pts:
        frames.box((p[0], top / 2, p[1]), (0.07, top, 0.1), fmat, 0.0, skip=('top', 'bottom'))
    # return walls from where each wall line stops to the drum's ends
    wm = self.wall_material
    for W, D, nrm in ((A, pts[0], mul(u_in, -1)), (C, pts[-1], mul(u_out, -1))):
        ret.quad([B(W[0], 0.0, W[1]), B(D[0], 0.0, D[1]), B(D[0], top, D[1]), B(W[0], top, W[1])], wm, B(nrm[0], 0, nrm[1]))
        ret.quad([B(D[0], 0.0, D[1]), B(W[0], 0.0, W[1]), B(W[0], top, W[1]), B(D[0], top, D[1])], wm, B(-nrm[0], 0, -nrm[1]))
    glass.build(self.exterior)
    frames.build(self.exterior)
    ret.build(self.exterior)
    return O, r, rr, a0, da, top, segs, A, C, V


# `bladeSigns[].flagDeg` (absent = unchanged): the banner
# is a flag on a short steel arm, its face turned `flagDeg` from square-on to
# the wall (towards +along the edge; negative turns it the other way), so it
# shows its face to a camera across the street instead of its thin edge.
# `widthM` sets the flag's width (default 0.8). Other blade keys as before.
_g2_blade_sign_base = Building._blade_sign


def _g2_blade_sign(self, e, a, b):
    keys = ('bladeSigns', 'bladeSign')
    saved = {k: self.style.get(k) for k in keys}
    flags = [sp for sp in (self.style.get('bladeSigns') or []) if sp.get('flagDeg') is not None]
    if flags:
        self.style['bladeSigns'] = [sp for sp in self.style['bladeSigns'] if sp.get('flagDeg') is None]
    try:
        _g2_blade_sign_base(self, e, a, b)
    finally:
        for k, v in saved.items():
            if v is not None:
                self.style[k] = v
    if not flags or self.storeys < 2:
        return
    front_e, front_a, front_b = e, a, b
    mb = MeshBuilder('blade-flags')
    for spec in flags:
        e, a, b = front_e, front_a, front_b
        if spec.get('edge') is not None:
            e = next(x for x in self.edges if x['index'] == int(spec['edge']))
            a, b = self.poly[e['index']], self.poly[(e['index'] + 1) % len(self.poly)]
        L = e['lengthM']
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        th = math.radians(float(spec['flagDeg']))
        fdir = add(mul(nout, math.cos(th)), mul(d, math.sin(th)))
        xdir = add(mul(d, math.cos(th)), mul(nout, -math.sin(th)))
        y0 = self.levels[1] + spec.get('bottomM', 0.6)
        h = spec.get('heightM', 2.0)
        w = spec.get('widthM', 0.8)
        root = at_edge(a, d, nout, self.f2s(spec['at'], L, e), 0.12)
        c = add(root, mul(fdir, 0.12 + w / 2))
        face = spec.get('face', 'painted-white')
        mb.box((c[0], y0 + h / 2, c[1]), (0.03, h, w), 'signage', box_yaw(xdir), mats={'+x': face, '-x': face})
        for yy in (y0 + h + 0.04, y0 - 0.04):   # the arm along the top, a tie at the foot
            q = add(root, mul(fdir, (0.12 + w) / 2 + 0.03))
            mb.box((q[0], yy, q[1]), (0.04, 0.04, w + 0.18), 'painted-steel', box_yaw(xdir))
    mb.build(self.exterior)


Building._blade_sign = _g2_blade_sign


# `floorsM` (absent = unchanged): {groundM?, upperM?}
# overrides the spec's derived storey heights, e.g. a low two-storey row whose
# LiDAR height includes a pitched roof: lower plates leave the roof blocks'
# hip room under the same height allowance.
_g2_init_base = Building.__init__


def _g2_init(self, spec, style):
    fo = (style or {}).get('floorsM')
    if fo:
        spec = dict(spec, floors={**spec['floors'], **fo})
    _g2_init_base(self, spec, style)
    # `wallTopM` (absent = unchanged): the street walls and
    # their coping stop here instead of at the LiDAR height; `roofBlocks` keep
    # the LiDAR allowance, so a pitched roof shows above a low eave line
    wt = (style or {}).get('wallTopM')
    if wt and self.roof_level < float(wt) < self.height:
        self._g2_lidar = self.height
        self.height = float(wt)


Building.__init__ = _g2_init


# `awningSpans[].valanceM` / `valance` (absent = unchanged):
# a taller front valance (a shop's sign strip on the awning's lip) in `valance`
# (kit@tint, default the awning material), `valanceM` tall, hanging from the
# awning's outer edge, so a canvas awning reads as a band from across a
# junction instead of a line.
_g2_facade_relief_base = Building._facade_relief


def _g2_arch_heads(self, e, a, b, s):
    # `archHeads` (absent = unchanged): a segmental arched
    # fanlight over an upper window span - [{from, to, storey, riseM, ring,
    # glass, frame}]: a glazed arc segment with radial glazing bars sitting on
    # the window head, framed by a soldier-course ring (`ring`, default the
    # wall material), drawn on the wall face (the rectangular window below
    # stays as built). The rise is capped under the parapet.
    specs = s.get('archHeads') or []
    if not specs:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    mb = MeshBuilder(f'arch-heads-{e["index"]}')
    for sp in specs:
        s0, s1 = sorted((self.f2s(sp['from'], L, e), self.f2s(sp['to'], L, e)))
        k = int(sp.get('storey', 1))
        if k >= len(self.levels):
            continue
        head = self.levels[k] + min(s.get('upperHeadM', s['windowHeadM']), self.storey_height(k) - 0.2)
        rise = max(0.2, min(float(sp.get('riseM', 0.6)), self.height - 0.35 - head))
        # `dropM`: spring the arch this far below the window head, so
        # the arch eats the top of the rectangular window; the two corners
        # between the arc and the old head are filled with the ring/wall
        # material, and the window reads round-headed (not a flat grid)
        drop = max(0.0, float(sp.get('dropM', 0.0)))
        if drop > 0:
            spring = head - drop
            half0 = (s1 - s0) / 2
            rr = drop + rise
            rad0 = (half0 * half0 + rr * rr) / (2 * rr)
            yc0 = spring + rr - rad0
            cs0 = (s0 + s1) / 2
            ring0 = sp.get('ring') or self.wall_material
            out0 = B(nout[0], 0, nout[1])
            for side in (-1, 1):
                edge_s = cs0 + side * half0
                pts2 = [(edge_s, spring)]
                m = 16
                for i in range(1, m + 1):
                    x = edge_s - side * half0 * i / m
                    y = yc0 + math.sqrt(max(0.0, rad0 * rad0 - (x - cs0) ** 2))
                    if y >= head:
                        # where the arc crosses the old head line
                        xh = cs0 + side * math.sqrt(max(0.0, rad0 * rad0 - (head - yc0) ** 2))
                        pts2.append((xh, head))
                        break
                    pts2.append((x, y))
                pts2.append((edge_s, head))
                if len(pts2) >= 3:
                    mb.poly([B(at(sv, 0.05)[0], y, at(sv, 0.05)[1]) for sv, y in pts2], ring0, out0)
            head, rise = spring, rr
        half = (s1 - s0) / 2
        rad = (half * half + rise * rise) / (2 * rise)
        cs = (s0 + s1) / 2
        yc = head + rise - rad
        th = math.asin(min(1.0, half / rad))
        n = max(12, int(sp.get('segments', 12)) // 4 * 4)   # Finer arc = smoother ring
        arc = [(cs + rad * math.sin(-th + 2 * th * i / n), yc + rad * math.cos(-th + 2 * th * i / n)) for i in range(n + 1)]
        out = B(nout[0], 0, nout[1])
        gm = sp.get('glass', s['glass'])
        fm = sp.get('frame', s.get('sashMaterial', 'painted-steel'))
        ring = sp.get('ring') or self.wall_material
        pts = [B(at(sv, 0.03)[0], y, at(sv, 0.03)[1]) for sv, y in arc]
        mb.poly([B(at(s0, 0.03)[0], head, at(s0, 0.03)[1])] + pts[1:-1] + [B(at(s1, 0.03)[0], head, at(s1, 0.03)[1])], gm, out)
        for i in range(1, 4):   # radial glazing bars
            sv, y = arc[i * n // 4]
            p0, p1 = (cs, head), (sv, y)
            ln = math.hypot(p1[0] - p0[0], p1[1] - p0[1])
            # a box cannot roll in the wall plane, so the bar is short
            # segments stacked along the ray
            for j in range(6):
                t = (j + 0.5) / 6
                q = at(p0[0] + (p1[0] - p0[0]) * t, 0.045)
                mb.box((q[0], p0[1] + (p1[1] - p0[1]) * t, q[1]), (0.05, ln / 6 + 0.01, 0.03), fm, yaw)
        q = at(cs, 0.045)
        mb.box((q[0], head + 0.03, q[1]), (s1 - s0, 0.06, 0.04), fm, yaw)
        for i in range(n):   # soldier-course ring
            (sa, ya), (sb, yb) = arc[i], arc[i + 1]
            ma = ((sa + sb) / 2, (ya + yb) / 2)
            q = at(ma[0], 0.06)
            mb.box((q[0], ma[1] + 0.12, q[1]), (abs(sb - sa) + 0.05, 0.26, 0.06), ring, yaw)
    if not mb.empty():
        mb.build(self.exterior)


def _g2_facade_relief(self, e, a, b, s):
    _g2_arch_heads(self, e, a, b, s)
    lidar = getattr(self, '_g2_lidar', None)
    if lidar and s.get('roofBlocks'):
        _g2_facade_relief_base(self, e, a, b, {**s, 'roofBlocks': []})
        only = {k: s[k] for k in ('wall', 'wallTint', 'roofBlocks') if k in s}
        low, self.height = self.height, lidar
        try:
            _g2_facade_relief_base(self, e, a, b, only)
        finally:
            self.height = low
    else:
        _g2_facade_relief_base(self, e, a, b, s)
    spans = [sp for sp in (s.get('awningSpans') or []) if sp.get('valanceM')]
    if not spans:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    g_head = self.spec['floors']['groundM'] - s['storefront'].get('headDropM', 0.35)
    mb = MeshBuilder(f'awning-valance-{e["index"]}')
    for sp in spans:
        s0, s1 = sorted((self.f2s(sp['from'], L, e), self.f2s(sp['to'], L, e)))
        depth = sp.get('depthM', 1.1)
        vh = float(sp['valanceM'])
        q = add(add(a, mul(d, (s0 + s1) / 2)), mul(nout, depth + 0.02))
        y_top = g_head + 0.15 + 0.04
        mb.box((q[0], y_top - vh / 2, q[1]), (s1 - s0, vh, 0.04), sp.get('valance', s['awning']), yaw)
    mb.build(self.exterior)


Building._facade_relief = _g2_facade_relief


# storefront `garage` items with `material: "carriage"` (
# any other material = unchanged): a pair of hinged timber carriage doors -
# solid lower leaves in `tint` (default near-black) with a glazed top row of
# small panes (`paneRowM` tall is not configurable: 28% of the door, max 1 m),
# a centre meeting stile and frame. Old industrial fronts; not a roll-up.
_g2_closed_infill_base = Building._closed_infill


def _g2_louvre(self, span, at, store, yaw):
    # `material: "louvre"`: a fixed louvred vent panel -
    # a dark void behind deep horizontal blades in a steel frame, `tint` the
    # blade colour (default rust brown)
    s0, s1, y0, y1, _mat, tint = span[:6]
    w, h, cs = s1 - s0, y1 - y0, (s0 + s1) / 2
    back = -0.16
    blade = self.tinted('painted-steel-light', tint or '#7a4a36')
    p = at(cs, back - 0.12)
    store.box((p[0], (y0 + y1) / 2, p[1]), (w, h, 0.02), 'painted-steel', yaw)
    n = max(4, int(h / 0.14))
    for i in range(n):
        q = at(cs, back - 0.02)
        store.box((q[0], y0 + 0.08 + (h - 0.16) * (i + 0.5) / n, q[1]), (w - 0.1, 0.05, 0.16), blade, yaw)
    for sv in (s0 + 0.04, s1 - 0.04):
        q = at(sv, back)
        store.box((q[0], (y0 + y1) / 2, q[1]), (0.08, h, 0.2), blade, yaw)
    for yy in (y0 + 0.04, y1 - 0.04):
        q = at(cs, back)
        store.box((q[0], yy, q[1]), (w, 0.08, 0.2), blade, yaw)


def _g2_closed_infill(self, kind, span, at, s, store, glass, yaw):
    if kind == 'garage' and len(span) >= 6 and span[4] == 'louvre':
        return _g2_louvre(self, span, at, store, yaw)
    if kind != 'garage' or len(span) < 6 or span[4] != 'carriage':
        return _g2_closed_infill_base(self, kind, span, at, s, store, glass, yaw)
    s0, s1, y0, y1, _mat, tint = span[:6]
    w, h, cs = s1 - s0, y1 - y0, (s0 + s1) / 2
    back = -0.16
    leaf = self.tinted('painted-steel', tint or '#26282a')
    gh = min(1.0, h * 0.28)
    p = at(cs, back)
    store.box((p[0], y0 + (h - gh) / 2, p[1]), (w, h - gh, 0.05), leaf, yaw)
    glass.box((p[0], y1 - gh / 2, p[1]), (w, gh, 0.02), 'glass-reflective@7d868c', yaw)
    cols = max(2, int(round(w / 0.45)))
    for i in range(cols + 1):
        q = at(s0 + 0.03 + (w - 0.06) * i / cols, back + 0.03)
        store.box((q[0], y1 - gh / 2, q[1]), (0.06 if i not in (0, cols, cols // 2) else 0.12, gh, 0.06), leaf, yaw)
    for yy in (y1 - gh, y1 - 0.03, y1 - gh / 2):
        q = at(cs, back + 0.03)
        store.box((q[0], yy, q[1]), (w, 0.06, 0.06), leaf, yaw)
    # lower leaves: meeting stile and a rail with two raised panels a leaf
    q = at(cs, back + 0.04)
    store.box((q[0], y0 + (h - gh) / 2, q[1]), (0.1, h - gh, 0.06), leaf, yaw)
    for f in (0.25, 0.75):
        q = at(s0 + w * f, back + 0.035)
        store.box((q[0], y0 + (h - gh) * 0.55, q[1]), (w * 0.36, (h - gh) * 0.6, 0.03), leaf, yaw)


Building._closed_infill = _g2_closed_infill


_g2_inner_rect_base = Building._inner_rect


def _g2_inner_rect(self):
    # With a set-back drum the rooms (rectangle) stop behind it: the two rect
    # sides at the rotunda corner move in until that rect corner lies inside
    # the drum, so no slab, partition or furniture stands under the overhang.
    x0, x1, z0, z1 = _g2_inner_rect_base(self)
    spec = (getattr(self, 'style', None) or {}).get('rotunda') or {}
    rs = spec.get('round') or {}
    if not rs.get('insetM'):
        return (x0, x1, z0, z1)
    r = spec.get('radiusM', 3.0)
    rr = r - min(float(rs['insetM']), r * 0.6)
    d = r - rr / math.sqrt(2) + 0.15
    V = self.poly[int(spec['vertex']) % len(self.poly)]
    if abs(V[0] - x0) < abs(V[0] - x1):
        x0 = min(x0 + d, x1 - 4.0)
    else:
        x1 = max(x1 - d, x0 + 4.0)
    if abs(V[1] - z0) < abs(V[1] - z1):
        z0 = min(z0 + d, z1 - 4.0)
    else:
        z1 = max(z1 - d, z0 + 4.0)
    return (x0, x1, z0, z1)


Building._inner_rect = _g2_inner_rect


def _g2_rotunda(self):
    spec = self.style.get('rotunda') or {}
    rs = spec.get('round') or {}
    if rs.get('insetM'):
        g = _g2_rotunda_inset(self, spec, rs)
        _g2_rotunda_round(self, g)
        return
    _g2_rotunda_base(self)
    _g2_rotunda_round(self)


Building._rotunda = _g2_rotunda


# ----------------------------------------------------------------------------------
# Louvred residential frame (855 Folsom). Every key is absent by default, so
# buildings that do not name them build unchanged.
#   frameGrid.panelSlide "leaf"   screen bays close one or all of `panelLeaves`
#                                 equal leaves (share `panelClosedShare` all
#                                 closed), full bay height, in ONE plane
#   frameGrid.panelOutM m         leaf face offset out from the wall plane
#                                 (capped inside the frame's own projection)
#   frameGrid.rail "solid"        opaque dark guard plate (`railMaterial`,
#                                 `railTint`) for balcony and screen bays
#   pattern kind "loggia"         glazed bay: glass `loggiaRecessM` behind the
#                                 wall line in `loggiaFrame` mullions every
#                                 `loggiaMullionM`, and a balcony deck
#                                 `loggiaOutM` out from it with a dark glass
#                                 guard (`loggiaRailGlass`) and planters. For a
#                                 recessed court face: the decks stay inside
#                                 the footprint's own notch.
#   edge style garageRibs "vertical"   corrugated gate ribs on garage infills
#   edge style wallLamps [{at, y, d}]  round wall lamps (fractions from the
#                                 viewer's left, centre height, diameter)

def _g1_panel_leaf(self, grid, kind, s0, s1, y0, y1, at, store, frames, yaw, pm):
    if grid.get('panelSlide') != 'leaf':
        return False
    w, h = s1 - s0, y1 - y0
    leaves = max(1, int(grid.get('panelLeaves', 2)))
    lw = (w - 0.04) / leaves
    closed = leaves if self.rng.random() < grid.get('panelClosedShare', 0.55) else max(1, leaves - 1)
    left = self.rng.random() < 0.5
    d = norm(sub(at(1.0, 0.0), at(0.0, 0.0)))
    pr = max(0.02, min(grid.get('projectionM', 0.12), self._projection_cap(d)))
    out = max(-0.3, min(grid.get('panelOutM', 0.0), pr - 0.05))
    fm = grid.get('panelFrame', 'painted-white')
    for j in range(closed):
        idx = j if left else leaves - 1 - j
        a0 = s0 + 0.02 + lw * idx
        sv = a0 + lw / 2
        p = at(sv, out - 0.02)
        store.box((p[0], (y0 + y1) / 2, p[1]), (lw - 0.03, h - 0.04, 0.03), pm, yaw, skip=('top', 'bottom', '+x', '-x'))
        for ev in (a0 + 0.025, a0 + lw - 0.025):
            q = at(ev, out)
            frames.box((q[0], (y0 + y1) / 2, q[1]), (0.045, h - 0.02, 0.05), fm, yaw, skip=('top', 'bottom'))
        if grid.get('panelFrameRails', False):   # head and sill bars too
            for yy in (y0 + 0.035, y1 - 0.035):
                q = at(sv, out)
                frames.box((q[0], yy, q[1]), (lw - 0.07, 0.045, 0.05), fm, yaw, skip=('+x', '-x'))
    return True


def _g1_loggia(self, grid, s0, s1, y0, y1, at, store, glass, yaw):
    outm = float(grid.get('loggiaOutM', 0.0))
    if outm <= 0.05:
        return
    w, cs = s1 - s0, (s0 + s1) / 2
    ext = w + grid.get('columnW', 0.4)
    dm = self.tinted(grid.get('material', 'concrete-precast'), grid.get('tint'))
    p = at(cs, outm / 2)
    store.box((p[0], y0 - 0.12, p[1]), (ext, 0.24, outm), dm, yaw, skip=('+x', '-x'))
    q = at(cs, outm - 0.05)
    glass.box((q[0], y0 + 0.52, q[1]), (ext - 0.04, 0.95, 0.02), grid.get('loggiaRailGlass', grid.get('railGlass', 'glass-storefront@6f7880')), yaw,
              skip=('top', 'bottom', '+x', '-x'))
    rm = self.tinted(grid.get('railMaterial', 'painted-steel'), grid.get('railTint'))
    store.box((q[0], y0 + 1.02, q[1]), (ext, 0.05, 0.06), rm, yaw, skip=('+x', '-x', 'bottom'))
    if grid.get('balconyPlanters'):
        for f in (0.22, 0.7):
            if self.rng.random() < 0.3:
                continue
            pl = at(s0 + w * f, outm - 0.4)
            store.box((pl[0], y0 + 0.22, pl[1]), (0.9, 0.44, 0.4), 'painted-steel-mid', yaw, skip=('bottom',))
            store.box((pl[0], y0 + 0.66, pl[1]), (0.8, 0.44, 0.36), 'int-plant', yaw, skip=('bottom',))


def _g1_balcony_lip(self, grid, kind, s0, s1, y0, at, store, bmat, yaw):
    """Deck slab from the wall line out to `balconyLipM`, as far as the
    footprint's world AABB allowance lets it project at this bay (the kit's
    fixed cap assumes the worst corner; a diagonal facade has room mid-face)."""
    want = float(grid.get('balconyLipM', 0.0))
    if want <= 0:
        return 0.0
    fr = self.spec['frame']
    c, sn = math.cos(fr['heading']), math.sin(fr['heading'])

    def world(p):
        return (p[0] * c + p[1] * sn, -p[0] * sn + p[1] * c)
    pts = [world(p) for p in self.poly]
    x0, x1 = min(p[0] for p in pts) - 0.42, max(p[0] for p in pts) + 0.42
    z0, z1 = min(p[1] for p in pts) - 0.42, max(p[1] for p in pts) + 0.42
    lip = want
    while lip > 0.05:
        corners = [world(at(sv, lip)) for sv in (s0 - 0.1, s1 + 0.1)]
        if all(x0 <= x <= x1 and z0 <= z <= z1 for x, z in corners):
            break
        lip -= 0.05
    if lip <= 0.05:
        return 0.0
    w = s1 - s0
    p = at((s0 + s1) / 2, lip / 2)
    store.box((p[0], y0 - 0.02, p[1]), (w + 0.2, 0.2, lip), bmat, yaw, skip=('+x', '-x'))
    return lip


def _g1_garage_recess(self, span, store, glass, at, cs, s0, s1, y0, y1, back, yaw):
    """A garage mouth: the shutter stands `recessM` behind the wall line, `doorM`
    tall, under a dark slatted screen up to the opening head, between dark
    reveals and a soffit. Returns (back, door top, door height)."""
    rec = span[7] if len(span) > 7 and span[7] else 0.0
    if rec <= 0:
        return back, y1, y1 - y0
    w = s1 - s0
    back = back - rec
    top = min(y1, y0 + float(span[8])) if len(span) > 8 and span[8] else y1
    dark = 'painted-steel-mid@8c9094'
    mid = back / 2
    for sv in (s0 + 0.05, s1 - 0.05):
        p = at(sv, mid)
        store.box((p[0], (y0 + y1) / 2, p[1]), (0.1, y1 - y0, -back), dark, yaw, skip=('top', 'bottom'))
    p = at(cs, mid)
    store.box((p[0], y1 - 0.05, p[1]), (w, 0.1, -back), dark, yaw, skip=('top', '+x', '-x'))
    if top < y1 - 0.05:
        g = at(cs, back + 0.01)
        glass.box((g[0], (top + y1) / 2, g[1]), (w - 0.1, y1 - top, 0.02), 'glass-dark', yaw, skip=('top', 'bottom', '+x', '-x'))
    return back, top, top - y0


def _g1_vertical_ribs(s, store, at, cs, back, s0, w, y0, y1, h, m, yaw):
    if s.get('garageRibs') != 'vertical':
        return False
    n = max(2, int(w / 0.16))
    for i in range(1, n):
        q = at(s0 + w * i / n, back + 0.02)
        store.box((q[0], (y0 + y1) / 2, q[1]), (0.025, h - 0.02, 0.02), m, yaw, skip=('top', 'bottom'))
    return True


def _g1_wall_lamps(self, s, L, e, at, mb, nout):
    lamps = s.get('wallLamps') or []
    out = B(nout[0], 0, nout[1])
    for lp in lamps:
        c = at(self.f2s(lp['at'], L, e), 0.0)
        r = lp.get('d', 0.36) / 2
        d = norm(sub(at(1.0, 0.0), at(0.0, 0.0)))
        for rr, off, mat in ((r * 1.18, 0.03, 'painted-steel-mid'), (r, 0.075, lp.get('face', 'painted-white@fbf8ee'))):
            ring = []
            for k in range(12):
                a = 2 * math.pi * k / 12
                q = add(add(c, mul(d, rr * math.cos(a))), mul(nout, off))
                ring.append(B(q[0], lp['y'] + rr * math.sin(a), q[1]))
            mb.quad(ring, mat, out)


Building._g1_panel_leaf = _g1_panel_leaf
Building._g1_loggia = _g1_loggia


# `lowParts` (style; absent = unchanged): a lower wing measured
# from LiDAR, e.g. [{"zMax": -12.0, "heightM": 12.07, "toleranceM": 0.5,
# "source": "..."}]. The part is the footprint where the spec-frame z is below
# `zMax` (the rear, away from the front edge). Everything drawn there above
# `heightM` is cut away (exterior, interior, fine storeys, lights, physics);
# a step wall with punched windows closes the tall block at `zMax`, and the
# part gets its own roof. Rooms and the stair core are planned in the tall
# block only. The build FAILS unless the drawn part tops out within
# `toleranceM` of `heightM` and the step stays inside the footprint.

def _g1_low_parts(self):
    return [p for p in (self.style.get('lowParts') or []) if p.get('heightM') is not None]


_g1_orig_plan_stair = Building.plan_stair


def _g1_plan_stair(self):
    for p in _g1_low_parts(self):
        x0, x1, z0, z1 = self.rect
        self.rect = (x0, x1, max(z0, p['zMax'] + WALL_T + 0.05), z1)
    return _g1_orig_plan_stair(self)


def _g1_ring_below(ring, zmax, below=True):
    """A ring clipped to the half plane z <= zmax (or >= when below=False)."""
    out = []
    for i in range(len(ring)):
        p, q = ring[i - 1], ring[i]
        pin = p[1] <= zmax if below else p[1] >= zmax
        qin = q[1] <= zmax if below else q[1] >= zmax
        if qin != pin:
            t = (zmax - p[1]) / ((q[1] - p[1]) or 1e-12)
            out.append(lerp2(p, q, t))
        if qin:
            out.append(q)
    return out


def _g1_cut_objects(self, zmax, h):
    from mathutils import Matrix
    ycut = -zmax   # spec z -> Blender -Y
    cols = [self.exterior, self.interior] + [c for c in bpy.data.collections if c.name.startswith('fine-s')]
    for col in cols:
        for obj in list(col.objects):
            if obj.type != 'MESH':
                continue
            if obj.matrix_world != Matrix.Identity(4) or obj.data.users > 1:
                # placed library pieces: drop the whole piece when it stands in the cut
                c = obj.matrix_world @ (sum((Vector(b) for b in obj.bound_box), Vector()) / 8)
                if c.y > ycut and c.z > h:
                    bpy.data.objects.remove(obj, do_unlink=True)
                continue
            bm = bmesh.new()
            bm.from_mesh(obj.data)
            f1 = [f for f in bm.faces if any(v.co.z > h + 1e-4 for v in f.verts)
                  and min(v.co.y for v in f.verts) < ycut - 1e-4 < max(v.co.y for v in f.verts)]
            if f1:
                bmesh.ops.bisect_plane(bm, geom=list({e for f in f1 for e in f.edges} | set(f1) | {v for f in f1 for v in f.verts}),
                                       plane_co=(0, ycut, 0), plane_no=(0, 1, 0))
            f2 = [f for f in bm.faces if any(v.co.y > ycut + 1e-4 for v in f.verts)
                  and min(v.co.z for v in f.verts) < h - 1e-4 < max(v.co.z for v in f.verts)]
            if f2:
                bmesh.ops.bisect_plane(bm, geom=list({e for f in f2 for e in f.edges} | set(f2) | {v for f in f2 for v in f.verts}),
                                       plane_co=(0, 0, h), plane_no=(0, 0, 1))
            dead = [f for f in bm.faces if f.calc_center_median().y > ycut + 1e-4 and f.calc_center_median().z > h + 1e-4]
            if dead:
                bmesh.ops.delete(bm, geom=dead, context='FACES')
            bm.to_mesh(obj.data)
            bm.free()
            obj.data.update()
            if not obj.data.polygons:
                bpy.data.objects.remove(obj, do_unlink=True)


def _g1_step_geometry(self, zmax, h):
    s = self.style
    top = self.height
    # the step face: the footprint's span at zmax, outer face at zmax facing -z
    xs = []
    n = len(self.poly)
    for i in range(n):
        p, q = self.poly[i], self.poly[(i + 1) % n]
        if (p[1] - zmax) * (q[1] - zmax) < 0:
            t = (zmax - p[1]) / (q[1] - p[1])
            xs.append(p[0] + (q[0] - p[0]) * t)
    if len(xs) < 2:
        raise RuntimeError(f'lowParts: zMax {zmax} does not cross the footprint')
    xa, xb = min(xs) + WALL_T, max(xs) - WALL_T
    wm = self.tinted(s['wall'], s.get('returnWallTint') or s.get('wallTint'))
    mb = MeshBuilder('shell-step')
    w = xb - xa
    mb.box(((xa + xb) / 2, (h + top) / 2, zmax + WALL_T / 2), (w, top - h, WALL_T), wm, skip=('+x', '-x'))
    glass = MeshBuilder('glazing-step')
    for k in range(self.storeys):
        y0 = self.levels[k]
        if y0 < h - 0.05:
            continue
        sh = self.storey_height(k)
        nwin = max(1, int(w / 3.0))
        for i in range(nwin):
            cx = xa + w * (i + 0.5) / nwin
            glass.box((cx, y0 + 0.85 + (sh - 1.3) / 2, zmax - 0.012), (1.5, sh - 1.3, 0.02), s['glass'], skip=('top', 'bottom', '+x', '-x'))
    # the lower part's own roof, from the inset ring clipped to the part
    roof = MeshBuilder('roof-low')
    ring = _g1_ring_below(self.inner, zmax)
    if len(ring) >= 3:
        roof.poly([B(p[0], h + 0.02, p[1]) for p in ring], s['roof'], B(0, 1, 0))
    proxy = MeshBuilder('shadow-proxy-step')
    pr = _g1_ring_below(self._inset(WALL_T + 0.05), zmax)
    if len(pr) >= 3:
        proxy.poly([B(p[0], h, p[1]) for p in pr], 'int-shadow-proxy', B(0, 1, 0))
    proxy.quad([B(xa, h, zmax + 0.4), B(xb, h, zmax + 0.4), B(xb, self.roof_level, zmax + 0.4), B(xa, self.roof_level, zmax + 0.4)],
               'int-shadow-proxy', B(0, 0, -1))
    for m in (mb, glass, roof, proxy):
        m.build(self.exterior)
    # physics: the step wall above the part; walls wholly inside the part drop to its height
    self.physics['walls'].append({'id': 'wall-step', 'a': {'x': xa, 'z': zmax + WALL_T / 2}, 'b': {'x': xb, 'z': zmax + WALL_T / 2},
                                  'height': top - h, 'thickness': WALL_T, 'base': h, 'openings': []})
    for wall in self.physics['walls']:
        if wall['id'] != 'wall-step' and max(wall['a']['z'], wall['b']['z']) < zmax:
            wall['height'] = h
    self.physics['solids'] = [sd for sd in self.physics['solids']
                              if not (sd['center']['z'] < zmax and sd['center']['y'] > h)]
    self.lights = [lt for lt in self.lights if not (lt['z'] < zmax and lt['y'] > h)]
    return xa, xb


def _g1_check_low_part(self, part, xa, xb):
    """Fails the build: drawn height over the part vs its LiDAR height, and
    the step inside the footprint."""
    zmax, h, tol = part['zMax'], part['heightM'], part.get('toleranceM', 0.5)
    ycut = -zmax
    top = -1e9
    for obj in self.exterior.objects:
        if obj.type != 'MESH':
            continue
        for v in obj.data.vertices:
            co = obj.matrix_world @ v.co
            if co.y > ycut + 0.3:   # well inside the part
                top = max(top, co.z)
    ok_h = abs(top - h) <= tol
    inside = self._inside_outer((xa, zmax + 0.01)) and self._inside_outer((xb, zmax + 0.01))
    rec = {'zMax': zmax, 'heightM': h, 'drawnTopM': round(top, 3), 'toleranceM': tol, 'heightOk': ok_h, 'stepInside': bool(inside)}
    self.stats.setdefault('lowParts', []).append(rec)
    if not ok_h or not inside:
        raise RuntimeError(f'lowParts check failed: {rec}')


_g1_orig_export = Building.export


def _g1_export(self, out_dir):
    for part in _g1_low_parts(self):
        _g1_cut_objects(self, part['zMax'], part['heightM'])
        xa, xb = _g1_step_geometry(self, part['zMax'], part['heightM'])
        _g1_check_low_part(self, part, xa, xb)
    return _g1_orig_export(self, out_dir)


Building.plan_stair = _g1_plan_stair
Building.export = _g1_export


# `upperWindowStoreys` (style, top level or inside a `streetEdges` entry;
# bldg-939458463; absent = unchanged): the upper storey indices (1 = first
# storey above the ground floor) that keep their punched / ribbon windows on
# that edge. The other upper storeys stay blank wall, e.g. a tall mural wall
# with a clerestory row only under the roof. Projecting bays are untouched.

_b939_orig_openings_for_edge = Building.openings_for_edge


def _b939_openings_for_edge(self, e):
    ops, windows, doors = _b939_orig_openings_for_edge(self, e)
    keep = self.edge_style(e).get('upperWindowStoreys')
    if keep is None or self.storeys < 2:
        return ops, windows, doors
    keep = {int(k) for k in keep}

    def storey_of(y):
        k = 0
        for i, lvl in enumerate(self.levels):
            if y >= lvl - 1e-6:
                k = i
        return k

    drop = set()
    for w in windows:
        if w[0] in ('ribbon', 'punched'):
            o = tuple(w[1][:4])
            k = storey_of(o[2])
            if k >= 1 and k not in keep:
                drop.add(o)
    if not drop:
        return ops, windows, doors
    ops = [o for o in ops if tuple(o[:4]) not in drop]
    windows = [w for w in windows if not (w[0] in ('ribbon', 'punched') and tuple(w[1][:4]) in drop)]
    return ops, windows, doors


Building.openings_for_edge = _b939_openings_for_edge




# ---------------------------------------------------------------------------
# bldg-939458463 block. Self-contained, appended; every key
# below is NEW and absent = unchanged for every other building.
#
# `upperSetback` (style, list): [{fromM, depthM, edges: [i..], ledge?, deck?,
#   railing?, lid?}] - above fromM the listed street faces step back depthM
#   (a set-back clerestory over a tall shop storey; a rooftop box behind a
#   railed deck). Exterior geometry near each face plane is cut at fromM and
#   moved inward, so the footprint corners and the shell top stay the data.
#   ledge: {heightM, projectionM, material, tint} a cornice at the step;
#   deck: {material, tint} a deck over the step; railing: {heightM, material,
#   postM} a pipe rail on the step edge; lid: {material, tint} a flat top over
#   the stepped-back volume.
# `awningSpans[].profile` "barrel" | "box" (+ topM, riseM, ribM): a rounded
#   ribbed canvas awning or a squared box awning instead of the stepped one.
# `storefront.layout[].lattice` true: a folding scissor (diamond lattice) gate
#   over that opening (use with grille false); `latticeMaterial` on the edge.
# `pierPanels` (edge style): [{from, to, fromY?, topM, material?, tint?}]
#   applied pier / frame boards on the face.
# `fireEscapeLanding` (edge style): {at, yM, widthM, depthM, material,
#   ladderToM} a single landing with rails and a stowed drop ladder.
# `appliedWindows` (edge style): [{from, to, y0, y1, bars}] a small barred
#   window set on a blank / painted wall.
# `wallStreaks` (edge style): {count, tint, fromM, lengthM, widthM, seed}
#   weathering streaks under a ledge.
# `mural.layout` (edge style mural): [{kind: rect|band|lines, color, ...}] a
#   laid-out abstract composition in mural fractions instead of the seeded one.
# `shopFit` (style): {wallShelving, lamps, boxes, tables} extra shop fittings.
# ---------------------------------------------------------------------------

def _b5_at(a, d, nout):
    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))
    return at


def _b5_ribbon(mb, p0, p1, y0, y1, width, mat, nout, off):
    """A flat strip in the facade plane from (p0, y0) to (p1, y1), both sides."""
    dx, dy = p1[0] - p0[0], p1[1] - p0[1]
    ln = math.hypot(dx, dy)
    dv = y1 - y0
    L = math.hypot(math.hypot(dx, dy), dv) or 1.0
    # perpendicular in plane: along-edge component and vertical component
    ph, pv = -dv / L * width / 2, ln / L * width / 2
    ux, uz = (dx / ln, dy / ln) if ln > 1e-6 else (0.0, 0.0)
    o = (nout[0] * off, nout[1] * off)
    q0 = B(p0[0] + ux * ph + o[0], y0 + pv, p0[1] + uz * ph + o[1])
    q1 = B(p1[0] + ux * ph + o[0], y1 + pv, p1[1] + uz * ph + o[1])
    q2 = B(p1[0] - ux * ph + o[0], y1 - pv, p1[1] - uz * ph + o[1])
    q3 = B(p0[0] - ux * ph + o[0], y0 - pv, p0[1] - uz * ph + o[1])
    out = B(nout[0], 0, nout[1])
    mb.quad([q0, q1, q2, q3], mat, out)
    mb.quad([q0, q1, q2, q3], mat, B(-nout[0], 0, -nout[1]))


_b5_orig_facade_relief = Building._facade_relief


def _b5_facade_relief(self, e, a, b, s):
    spans = s.get('awningSpans') or []
    special = [sp for sp in spans if sp.get('profile') in ('barrel', 'box')]
    if special:
        s = {**s, 'awningSpans': [sp for sp in spans if sp not in special]}
    _b5_orig_facade_relief(self, e, a, b, s)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    at = _b5_at(a, d, nout)
    g_head = self.spec['floors']['groundM'] - s['storefront'].get('headDropM', 0.35)
    if special:
        _b5_awnings(self, e, L, at, d, nout, yaw, s, special, g_head)
    lay = [it for it in (s['storefront'].get('layout') or []) if it.get('lattice')]
    if lay:
        _b5_lattice(self, e, L, at, nout, yaw, s, lay, g_head)
    mb = MeshBuilder(f'b5-dress-{e["index"]}')
    for pp in s.get('pierPanels') or []:
        s0, s1 = sorted((self.f2s(pp['from'], L, e), self.f2s(pp['to'], L, e)))
        y0, y1 = pp.get('fromY', 0.0), pp.get('topM', g_head)
        p = at((s0 + s1) / 2, pp.get('projectionM', 0.03) / 2)
        mb.box((p[0], (y0 + y1) / 2, p[1]), (s1 - s0, y1 - y0, pp.get('projectionM', 0.03) + 0.01),
               self.tinted(pp.get('material', 'painted-white'), pp.get('tint')), yaw, skip=('-z',))
    for w in s.get('appliedWindows') or []:
        s0, s1 = sorted((self.f2s(w['from'], L, e), self.f2s(w['to'], L, e)))
        y0, y1 = w['y0'], w['y1']
        cs, ww, hh = (s0 + s1) / 2, s1 - s0, y1 - y0
        off = w.get('offsetM', 0.09)
        p = at(cs, off)
        mb.box((p[0], (y0 + y1) / 2, p[1]), (ww, hh, 0.02), 'glass-dark', yaw)
        fm = w.get('frame', 'painted-steel')
        for yy in (y0, y1):
            q = at(cs, off + 0.02)
            mb.box((q[0], yy, q[1]), (ww + 0.1, 0.06, 0.05), fm, yaw)
        for sv in (s0, s1):
            q = at(sv, off + 0.02)
            mb.box((q[0], (y0 + y1) / 2, q[1]), (0.06, hh + 0.1, 0.05), fm, yaw)
        if w.get('bars'):
            n = max(2, int(ww / 0.11))
            for i in range(1, n):
                q = at(s0 + ww * i / n, off + 0.05)
                mb.box((q[0], (y0 + y1) / 2, q[1]), (0.018, hh, 0.018), fm, yaw)
    ws = s.get('wallStreaks')
    if ws:
        rng = random.Random(ws.get('seed', 5))
        mat = self.tinted('painted-white', ws.get('tint', '#8f9295'))
        top = ws.get('fromM', self.height - 0.3)
        for _ in range(int(ws.get('count', 10))):
            sv = rng.uniform(0.3, L - 0.3)
            ln = ws.get('lengthM', 1.8) * rng.uniform(0.5, 1.2)
            wd = ws.get('widthM', 0.2) * rng.uniform(0.5, 1.3)
            q0, q1 = at(sv - wd / 2, 0.006), at(sv + wd / 2, 0.006)
            mb.quad([B(q0[0], top - ln, q0[1]), B(q1[0], top - ln * rng.uniform(0.6, 1.0), q1[1]),
                     B(q1[0], top, q1[1]), B(q0[0], top, q0[1])], mat, B(nout[0], 0, nout[1]))
    mb.build(self.exterior)
    fe = s.get('fireEscapeLanding')
    if fe:
        _b5_landing(self, e, L, at, nout, yaw, fe)


def _b5_awnings(self, e, L, at, d, nout, yaw, s, spans, g_head):
    for sp in spans:
        s0, s1 = sorted((self.f2s(sp['from'], L, e), self.f2s(sp['to'], L, e)))
        depth = sp.get('depthM', 1.1)
        top = sp.get('topM', g_head + 0.75)
        rise = sp.get('riseM', 0.55)
        vh = sp.get('valanceM', 0.3)
        mat = sp.get('material', s['awning'])
        am = MeshBuilder(f'awning-{sp["profile"]}-{e["index"]}')
        if sp['profile'] == 'box':
            p = at((s0 + s1) / 2, depth / 2)
            am.box((p[0], top - vh / 2, p[1]), (s1 - s0, vh, depth), mat, yaw, skip=('-z',))
            am.build(self.exterior)
            continue
        # barrel: quarter-ellipse from the wall (top) to the lip (top - rise)
        n = 8
        prof = [(depth * math.sin(math.pi / 2 * k / n), top - rise + rise * math.cos(math.pi / 2 * k / n)) for k in range(n + 1)]
        prof += [(depth, top - rise - vh)]
        for k in range(len(prof) - 1):
            (o0, y0), (o1, y1) = prof[k], prof[k + 1]
            pa0, pb0, pa1, pb1 = at(s0, o0), at(s1, o0), at(s0, o1), at(s1, o1)
            ring = [B(pa0[0], y0, pa0[1]), B(pb0[0], y0, pb0[1]), B(pb1[0], y1, pb1[1]), B(pa1[0], y1, pa1[1])]
            om = (o0 + o1) / 2
            out = B(nout[0] * 0.5, 0, nout[1] * 0.5) + Vector((0, 0, 1.0 if k < n else 0.0))
            am.quad(ring, mat, out)
            am.quad(ring, mat, -out)
        # end panels (fan from the wall foot of the profile)
        for sv in (s0, s1):
            base = at(sv, 0.0)
            c = B(base[0], top - rise - vh, base[1])
            for k in range(len(prof) - 1):
                (o0, y0), (o1, y1) = prof[k], prof[k + 1]
                q0, q1 = at(sv, o0), at(sv, o1)
                tri = [c, B(q0[0], y0, q0[1]), B(q1[0], y1, q1[1])]
                am.quad(tri, mat, B(d[0], 0, d[1]))
                am.quad(tri, mat, B(-d[0], 0, -d[1]))
        # ribs over the barrel
        rib = sp.get('ribM', 0.75)
        cnt = max(1, int((s1 - s0) / rib))
        for i in range(1, cnt):
            sv = s0 + (s1 - s0) * i / cnt
            for k in range(n):
                (o0, y0), (o1, y1) = prof[k], prof[k + 1]
                q0, q1 = at(sv - 0.025, o0 + 0.012), at(sv + 0.025, o1 + 0.012)
                r0, r1 = at(sv + 0.025, o0 + 0.012), at(sv - 0.025, o1 + 0.012)
                am.quad([B(q0[0], y0 + 0.012, q0[1]), B(r0[0], y0 + 0.012, r0[1]),
                         B(q1[0], y1 + 0.012, q1[1]), B(r1[0], y1 + 0.012, r1[1])], mat,
                        B(nout[0], 0, nout[1]) + Vector((0, 0, 1.0)))
        am.build(self.exterior)


def _b5_lattice(self, e, L, at, nout, yaw, s, items, g_head):
    gm = s.get('latticeMaterial', 'painted-white@c9cdd1')
    mb = MeshBuilder(f'lattice-gates-{e["index"]}')
    for it in items:
        s0, s1 = sorted((self.f2s(it['from'], L, e), self.f2s(it['to'], L, e)))
        y0, y1 = 0.03, it.get('latticeTopM', g_head - 0.05)
        pitch, rise = it.get('latticePitchM', 0.17), it.get('latticeRiseM', 0.34)
        cols = max(2, int(round((s1 - s0) / pitch)))
        xs = [s0 + (s1 - s0) * i / cols for i in range(cols + 1)]
        # channel stiles
        for x in xs:
            p = at(x, 0.05)
            mb.box((p[0], (y0 + y1) / 2, p[1]), (0.022, y1 - y0, 0.03), gm, yaw)
        # top and bottom track
        for yy in (y0 + 0.03, y1 - 0.03):
            p = at((s0 + s1) / 2, 0.05)
            mb.box((p[0], yy, p[1]), (s1 - s0, 0.06, 0.05), gm, yaw)
        rows = max(1, int((y1 - y0 - 0.1) / rise))
        for i in range(cols):
            for j in range(rows):
                ya = y0 + 0.05 + (y1 - y0 - 0.1) * j / rows
                yb = y0 + 0.05 + (y1 - y0 - 0.1) * (j + 1) / rows
                _b5_ribbon(mb, at(xs[i], 0), at(xs[i + 1], 0), ya, yb, 0.02, gm, nout, 0.055)
                _b5_ribbon(mb, at(xs[i], 0), at(xs[i + 1], 0), yb, ya, 0.02, gm, nout, 0.06)
    mb.build(self.exterior)


def _b5_landing(self, e, L, at, nout, yaw, fe):
    mat = fe.get('material', 'painted-steel')
    c, w, dep, y = self.f2s(fe['at'], L, e), fe.get('widthM', 2.4), fe.get('depthM', 0.85), fe.get('yM', 4.8)
    mb = MeshBuilder(f'fire-escape-landing-{e["index"]}')
    p = at(c, dep / 2 + 0.02)
    mb.box((p[0], y, p[1]), (w, 0.05, dep), mat, yaw)
    # slatted deck lines
    for i in range(1, 8):
        q = at(c - w / 2 + w * i / 8, dep / 2 + 0.02)
        mb.box((q[0], y + 0.03, q[1]), (0.02, 0.02, dep), mat, yaw)
    for yy in (y + 1.0, y + 0.5):
        q = at(c, dep)
        mb.box((q[0], yy, q[1]), (w, 0.04, 0.04), mat, yaw)
        for side in (-1, 1):
            q2 = at(c + side * w / 2, dep / 2)
            mb.box((q2[0], yy, q2[1]), (0.04, 0.04, dep), mat, yaw)
    for i in range(int(w / 0.3) + 1):
        q = at(c - w / 2 + i * w / int(w / 0.3), dep)
        mb.box((q[0], y + 0.5, q[1]), (0.02, 1.0, 0.02), mat, yaw)
    # brackets to the wall
    for side in (-1, 1):
        q = at(c + side * (w / 2 - 0.1), dep / 2)
        mb.box((q[0], y - 0.35, q[1]), (0.05, 0.05, dep * 0.9), mat, yaw)
    # stowed drop ladder at the outer end
    lx = c + w / 2 - 0.45
    lo = fe.get('ladderToM', y - 2.2)
    for sd in (-0.2, 0.2):
        q = at(lx + sd, dep - 0.08)
        mb.box((q[0], (lo + y + 1.0) / 2, q[1]), (0.035, y + 1.0 - lo, 0.035), mat, yaw)
    for k in range(int((y + 1.0 - lo) / 0.3)):
        q = at(lx, dep - 0.08)
        mb.box((q[0], lo + 0.15 + 0.3 * k, q[1]), (0.4, 0.025, 0.025), mat, yaw)
    mb.build(self.exterior)


Building._facade_relief = _b5_facade_relief


# mural.layout: a laid-out composition (fractions of the mural rect, x from
# the edge's `from` end). kinds: rect {x0,x1,y0,y1}, band {x0,x1 at y0, x2,x3
# at y1}, lines {x0,x1,y0,y1,n,color} short text-like bars.
_b5_orig_mural = Building._mural


def _b5_mural(self, e, a, b, spec):
    lay = spec.get('layout')
    if not lay:
        return _b5_orig_mural(self, e, a, b, spec)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    f0, f1 = self.f2s(spec.get('from', 0.05), L, e), self.f2s(spec.get('to', 0.95), L, e)
    y0 = spec.get('bottomM', 3.0)
    y1 = min(self.height - 0.4, spec.get('topM', self.height - 1.0))
    mb = MeshBuilder('mural')
    out = B(nout[0], 0, nout[1])

    def P(fx, fy, layer):
        sv = f0 + (f1 - f0) * fx
        p = add(add(a, mul(d, sv)), mul(nout, 0.012 + 0.004 * layer))
        return B(p[0], y0 + (y1 - y0) * fy, p[1])

    def M(c):
        return f'painted-white@{c.lstrip("#").lower()}'
    mb.quad([P(0, 0, 0), P(1, 0, 0), P(1, 1, 0), P(0, 1, 0)], M(spec.get('ground', '#2c2d31')), out)
    for i, sh in enumerate(lay, start=1):
        k = sh.get('kind', 'rect')
        if k == 'rect':
            mb.quad([P(sh['x0'], sh['y0'], i), P(sh['x1'], sh['y0'], i), P(sh['x1'], sh['y1'], i), P(sh['x0'], sh['y1'], i)], M(sh['color']), out)
        elif k == 'band':
            mb.quad([P(sh['x0'], sh['y0'], i), P(sh['x1'], sh['y0'], i), P(sh['x3'], sh['y1'], i), P(sh['x2'], sh['y1'], i)], M(sh['color']), out)
        elif k == 'lines':
            n = int(sh.get('n', 4))
            rng = random.Random(sh.get('seed', 3))
            hh = (sh['y1'] - sh['y0']) / (2 * n)
            for j in range(n):
                ya = sh['y1'] - (2 * j + 1) * hh
                xb = sh['x0'] + (sh['x1'] - sh['x0']) * rng.uniform(0.55, 1.0)
                mb.quad([P(sh['x0'], ya, i), P(xb, ya, i), P(xb, ya + hh, i), P(sh['x0'], ya + hh, i)], M(sh['color']), out)
    mb.build(self.exterior)


Building._mural = _b5_mural


# upperSetback: cut + move exterior geometry near each face plane
def _b5_setback_apply(self):
    from mathutils import Matrix
    cols = [self.exterior] + [c for c in bpy.data.collections if c.name.startswith('fine-s')]
    cum = {}
    for sb in self.style.get('upperSetback') or []:
        y0, dep, gap = float(sb['fromM']), float(sb['depthM']), 0.02
        cum0 = dict(cum)
        n_e = len(self.poly)
        for ei in sb.get('edges', []):
            e = self.edges[int(ei)]
            i = e['index']
            a, b = self.poly[i], self.poly[(i + 1) % len(self.poly)]
            nout = mul(self.inward(a, b), -1)
            dvec = norm(sub(b, a))
            L = e['lengthM']
            c0 = cum.get(i, 0.0)
            lo, hi = -(c0 + WALL_T + 0.1), 1.2 - c0

            def dist(co):
                # Blender (X, Y, Z) -> frame (x = X, z = -Y)
                return (co.x - a[0]) * nout[0] + (-co.y - a[1]) * nout[1]

            def along(co):
                return (co.x - a[0]) * dvec[0] + (-co.y - a[1]) * dvec[1]

            def inband(co):
                return lo <= dist(co) <= hi and -0.8 <= along(co) <= L + 0.8
            mvx, mvy = -nout[0] * dep, nout[1] * dep   # frame move -nout*dep -> Blender (x, -z)
            for col in cols:
                for obj in list(col.objects):
                    if obj.type != 'MESH':
                        continue
                    if obj.matrix_world != Matrix.Identity(4) or obj.data.users > 1:
                        cc = obj.matrix_world @ (sum((Vector(q) for q in obj.bound_box), Vector()) / 8)
                        if cc.z > y0 + gap and inband(cc):
                            obj.location.x += mvx
                            obj.location.y += mvy
                        continue
                    bm = bmesh.new()
                    bm.from_mesh(obj.data)
                    for yc in (y0, y0 + gap):
                        fs = [f for f in bm.faces if min(v.co.z for v in f.verts) < yc - 1e-4 < max(v.co.z for v in f.verts)
                              and any(inband(v.co) for v in f.verts)]
                        if fs:
                            bmesh.ops.bisect_plane(bm, geom=list({ed for f in fs for ed in f.edges} | set(fs) | {v for f in fs for v in f.verts}),
                                                   plane_co=(0, 0, yc), plane_no=(0, 0, 1))
                    moved = set()
                    for v in bm.verts:
                        if v.co.z > y0 + gap / 2 and inband(v.co):
                            v.co.x += mvx
                            v.co.y += mvy
                            moved.add(v)
                    dead = [f for f in bm.faces if all(y0 - 1e-4 <= v.co.z <= y0 + gap + 1e-4 for v in f.verts)
                            and any(v in moved for v in f.verts) and any(v not in moved for v in f.verts)]
                    if dead:
                        bmesh.ops.delete(bm, geom=dead, context='FACES')
                    bm.to_mesh(obj.data)
                    bm.free()
                    obj.data.update()
            cum[i] = c0 + dep
            # dressing on the step
            yaw = box_yaw(dvec)
            at = _b5_at(a, dvec, nout)
            mb = MeshBuilder(f'setback-{i}-{int(y0 * 100)}')
            led = sb.get('ledge')
            if led:
                pr = min(0.4, led.get('projectionM', 0.15))
                hh = led.get('heightM', 0.22)
                p = at(L / 2, ((pr - c0) - (c0 + dep + WALL_T)) / 2)
                mb.box((p[0], y0 + gap - hh / 2 + 0.02, p[1]), (L, hh, pr + dep + WALL_T),
                       self.tinted(led.get('material', 'painted-stucco'), led.get('tint')), yaw)
            # deck and rail stop at a neighbouring face that already stepped back
            ta, tb = cum0.get((i - 1) % n_e, 0.0), cum0.get((i + 1) % n_e, 0.0)
            dk = sb.get('deck')
            if dk:
                p = at((ta + L - tb) / 2, -c0 - (dep + WALL_T) / 2)
                mb.box((p[0], y0 + 0.01 + 0.003 * i, p[1]), (L - ta - tb, 0.06, dep + WALL_T),
                       self.tinted(dk.get('material', 'concrete-precast'), dk.get('tint')), yaw)
            rl = sb.get('railing')
            if rl:
                rm = rl.get('material', 'painted-steel')
                rh = rl.get('heightM', 1.0)
                off = -c0 - 0.12
                ra, rb_ = ta + 0.1, L - tb - 0.1
                for yy in (y0 + rh, y0 + rh / 2):
                    p = at((ra + rb_) / 2, off)
                    mb.box((p[0], yy, p[1]), (rb_ - ra, 0.045, 0.045), rm, yaw)
                cnt = max(1, int((rb_ - ra) / rl.get('postM', 1.6)))
                for k in range(cnt + 1):
                    p = at(ra + (rb_ - ra) * k / cnt, off)
                    mb.box((p[0], y0 + rh / 2, p[1]), (0.045, rh, 0.045), rm, yaw)
            mb.build(self.exterior)
        lid = sb.get('lid')
        if lid:
            # the footprint with the stepped faces offset inward, at the top
            n = len(self.poly)
            lines = []
            for i in range(n):
                a, b = self.poly[i], self.poly[(i + 1) % n]
                nin = self.inward(a, b)
                off = cum.get(i, 0.0) + (WALL_T * 0.5 if i in cum else WALL_T * 0.5)
                lines.append((add(a, mul(nin, off)), norm(sub(b, a))))
            pts = []
            for i in range(n):
                (p1, d1), (p2, d2) = lines[i - 1], lines[i]
                den = d1[0] * d2[1] - d1[1] * d2[0]
                if abs(den) < 1e-9:
                    pts.append(p2)
                    continue
                t = ((p2[0] - p1[0]) * d2[1] - (p2[1] - p1[1]) * d2[0]) / den
                pts.append(add(p1, mul(d1, t)))
            mb = MeshBuilder('rooftop-box-lid')
            ytop = self.height - 0.01
            mb.poly([B(p[0], ytop, p[1]) for p in pts], self.tinted(lid.get('material', 'painted-stucco'), lid.get('tint')), B(0, 1, 0))
            mb.build(self.exterior)


# ---- storeyWindowSpans (bldg-260361488): one upper storey drawn with its own
# ribbon lights instead of the building-wide upper window pattern.
#   storeyWindowSpans {"<k>": {"spans": [[f0, f1], ...], "sillM": m, "headM": m}}
#   k = storey index (1 = first storey above ground); f = fraction along the
#   street edge (viewer's left to right). Street edges only. Absent = unchanged.
_swb_orig_openings = Building.openings_for_edge


def _swb_openings(self, e):
    res = _swb_orig_openings(self, e)
    sw = self.style.get('storeyWindowSpans') or {}
    if not sw or e['role'] == 'party' or not self.is_street(e):
        return res
    ops, windows = list(res[0]), list(res[1])
    L = e['lengthM']
    for key, cfg in sw.items():
        k = int(key)
        if k < 1 or k >= self.storeys:
            continue
        lo = self.levels[k]
        hi = self.levels[k + 1] if k + 1 < self.storeys else self.roof_level
        drop = [o for kind, o in windows if kind in ('punched', 'ribbon') and lo <= o[2] < hi]
        windows = [(kind, o) for kind, o in windows if not any(o is d for d in drop)]
        ops = [o for o in ops if not any(o is d for d in drop)]
        sill = lo + cfg.get('sillM', self.style['windowSillM'])
        head = lo + min(cfg.get('headM', self.style['windowHeadM']), self.storey_height(k) - 0.2)
        for f0, f1 in cfg.get('spans', []):
            s0, s1 = sorted((self.f2s(f0, L, e), self.f2s(f1, L, e)))
            o = (s0, s1, sill, head)
            ops.append(o)
            windows.append(('ribbon', o))
    return (ops, windows) + tuple(res[2:])


# ---------------------------------------------------------------- balcony canopies + many fire escapes
# (bldg-256729086; every key absent = unchanged builds)
# `fireEscapes: [fireEscape, ...]` - more than one front fire escape; each item
# takes the `fireEscape` keys plus `toStorey`, `dropLadder: true` (a hung
# ladder under the lowest platform) and `stairUp: {from, to, upRight}` (a flight
# from the top platform up to the storey above, e.g. onto a balcony). The single `fireEscape` key still works.
# `balconyCanopies: [{at, widthM, storey, depthM, slab, rail, soffit, rafters,
# fascia, roof, rafterCount, canopyDepthM, bracketCount, bracketDropM}]` - a painted balcony on one storey
# of the front: slab, a fretwork balustrade (rails, posts, a lattice of bars),
# and a pent canopy above it sloping out from the wall, with a painted soffit,
# exposed rafters and a fascia board. Each of slab/rail/soffit/rafters/fascia/
# roof is {"material", "tint"}.
# Placement options: `slabRail: false` draws the canopy only (a crown on the
# parapet); `canopy: "below"` hangs the canopy under the balcony slab as a
# painted eave over the storey beneath; `canopyTopM` sets the canopy's wall-side
# top above the storey floor; `canopyAt`/`canopyWidthM` centre and size the
# canopy independently of the balcony (fractions like `at`).
# `signBands: [{from, to, material, tint, heightM, bottomM, projectionM}]` -
# blank shop sign boards on the front over the storefronts (fractions like the
# storefront layout; bottom defaults to the storefront head). No lettering.
_bc_fire_escape_base = Building._fire_escape


def _bc_mat(self, spec, key, default):
    m = spec.get(key) or {}
    return self.tinted(m.get('material', default), m.get('tint'))


def _bc_fire_escape(self, e, a, b):
    items = list(self.style.get('fireEscapes') or [])
    if not items:
        _bc_fire_escape_base(self, e, a, b)
    saved = self.style
    for item in items:
        self.style = dict(saved, fireEscape=item)
        try:
            _bc_fire_escape_base(self, e, a, b)
        finally:
            self.style = saved
        _bc_fe_grating(self, e, a, b, item)
        su = item.get('stairUp')
        if su and self.storeys > 2:
            # a flight from the top platform to the storey above (a balcony there)
            L = e['lengthM']
            d = norm(sub(b, a))
            nout = mul(self.inward(a, b), -1)
            yaw = box_yaw(d)
            mat = item.get('material', 'painted-steel')
            k = min(self.storeys - 2, item.get('toStorey', self.storeys - 1))
            y = self.levels[k] - 0.02
            rise = self.levels[k + 1] - self.levels[k]
            s0, s1 = sorted((self.f2s(su['from'], L, e), self.f2s(su['to'], L, e)))
            depth = item.get('depthM', 0.9)
            steps = 14
            run = (s1 - s0) / steps
            up_right = su.get('upRight', True)
            mb = MeshBuilder('fire-escape-stair')
            for i in range(steps):
                sv = s0 + (i + 0.5) * run if up_right else s1 - (i + 0.5) * run
                q = add(add(a, mul(d, sv)), mul(nout, depth * 0.55))
                mb.box((q[0], y + rise * (i + 1) / (steps + 1), q[1]), (run + 0.02, 0.03, 0.62), mat, yaw)
            # solid channel stringers either side of the treads and sloped
            # handrails on posts: one continuous flight, not floating treads
            lo, hi = (s0, s1) if up_right else (s1, s0)
            for off in (depth * 0.55 - 0.33, depth * 0.55 + 0.33):
                p0 = _bc_p3(at_edge(a, d, nout, lo, off), y + 0.02)
                p1 = _bc_p3(at_edge(a, d, nout, hi, off), y + rise)
                _bc_obox(mb, p0, p1, _bc_p3(nout, 0), 0.05, 0.2, mat)
                h0 = _bc_p3(at_edge(a, d, nout, lo, off), y + 0.95)
                h1 = _bc_p3(at_edge(a, d, nout, hi, off), y + rise + 0.95)
                _bc_obox(mb, h0, h1, _bc_p3(nout, 0), 0.04, 0.04, mat)
                for f in (0.0, 0.5, 1.0):
                    sv = lo + (hi - lo) * f
                    q = at_edge(a, d, nout, sv, off)
                    yb = y + rise * f
                    mb.box((q[0], yb + 0.5, q[1]), (0.035, 0.95, 0.035), mat, yaw)
            mb.build(self.exterior)
        if item.get('dropLadder') and self.storeys > 1:
            L = e['lengthM']
            d = norm(sub(b, a))
            nout = mul(self.inward(a, b), -1)
            yaw = box_yaw(d)
            c, w, depth = self.f2s(item['at'], L, e), item.get('widthM', 3.4), item.get('depthM', 0.9)
            mat = item.get('material', 'painted-steel')
            mb = MeshBuilder('fire-escape-ladder')
            y1 = self.levels[1] - 0.02
            y0 = max(2.3, y1 - 2.6)
            x = c + w / 2 - 0.45
            if item.get('dropLadderAt') is not None:
                # counterweighted drop stair, stowed: a steep flight hung from
                # the platform edge, its foot held clear of the shopfronts
                x = self.f2s(item['dropLadderAt'], L, e)
                y0 = y1 - float(item.get('dropLadderM', 1.7))
                run = float(item.get('dropLadderRunM', 0.9))
                off = depth - 0.12
                for side in (-0.3, 0.3):
                    p0 = _bc_p3(at_edge(a, d, nout, x + side, off), y0)
                    p1 = _bc_p3(at_edge(a, d, nout, x + side, off - run), y1)
                    _bc_obox(mb, p0, p1, _bc_p3(d, 0), 0.05, 0.14, mat)
                n = max(3, int((y1 - y0) / 0.24))
                for i in range(n):
                    f = (i + 0.5) / n
                    q = at_edge(a, d, nout, x, off - run * f)
                    mb.box((q[0], y0 + (y1 - y0) * f, q[1]), (0.62, 0.025, 0.2), mat, yaw)
                # counterweight arm and weight box above the platform rail
                q = at_edge(a, d, nout, x + 0.42, depth - 0.05)
                mb.box((q[0], y1 + 0.55, q[1]), (0.08, 1.1, 0.08), mat, yaw)
                mb.box((q[0], y1 + 1.0, q[1]), (0.18, 0.3, 0.14), mat, yaw)
                mb.build(self.exterior)
                continue
            for off in (-0.22, 0.22):
                p = add(add(a, mul(d, x + off)), mul(nout, depth - 0.05))
                mb.box((p[0], (y0 + y1) / 2, p[1]), (0.04, y1 - y0, 0.04), mat, yaw)
            n = int((y1 - y0) / 0.3)
            for i in range(n + 1):
                p = add(add(a, mul(d, x)), mul(nout, depth - 0.05))
                mb.box((p[0], y0 + i * (y1 - y0) / max(1, n), p[1]), (0.44, 0.03, 0.03), mat, yaw)
            mb.build(self.exterior)
    _bc_balcony_canopies(self, e, a, b)
    _bc_sign_bands(self, e, a, b)
    _bc_extras(self, e, a, b)
    _bc_r10(self, e, a, b)


def _bc_balcony_canopies(self, e, a, b):
    specs = self.style.get('balconyCanopies') or []
    if not specs or self.storeys < 2:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    mb = MeshBuilder('balcony-canopy')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    for spec in specs:
        k = max(1, min(self.storeys - 1, int(spec.get('storey', 1))))
        c, w = self.f2s(spec['at'], L, e), spec.get('widthM', 4.0)
        depth = spec.get('depthM', 0.9)
        slab = _bc_mat(self, spec, 'slab', 'cast-stone')
        rail = _bc_mat(self, spec, 'rail', 'painted-steel')
        soffit = _bc_mat(self, spec, 'soffit', 'painted-white')
        rafter = _bc_mat(self, spec, 'rafters', 'painted-steel')
        fascia = _bc_mat(self, spec, 'fascia', 'painted-steel')
        roofm = _bc_mat(self, spec, 'roof', 'glazed-tile')
        y = self.levels[k]
        slab_rail = spec.get('slabRail', True)
        # slab with a thin edge
        p = at(c, depth / 2)
        if slab_rail:
            mb.box((p[0], y - 0.08, p[1]), (w, 0.16, depth), slab, yaw)
        # balustrade: top and bottom rails, end posts, fretwork lattice
        rh = spec.get('railM', 1.05)
        for yy, t in () if not slab_rail else ((y + rh, 0.07), (y + 0.1, 0.05), (y + rh * 0.62, 0.035), (y + rh * 0.3, 0.035)):
            q = at(c, depth - 0.03)
            mb.box((q[0], yy, q[1]), (w, t, 0.05), rail, yaw)
            for side in (-1, 1):
                q2 = at(c + side * (w / 2 - 0.03), depth / 2)
                mb.box((q2[0], yy, q2[1]), (0.05, t, depth), rail, yaw)
        n = max(2, int(w / 0.16))
        for i in range(n + 1 if slab_rail else 0):
            sv = c - w / 2 + i * w / n
            q = at(sv, depth - 0.03)
            tall = i % 4 == 0
            h = rh if tall else rh * 0.62 - 0.1
            y0 = y + 0.1 if tall else y + 0.1
            mb.box((q[0], y0 + h / 2, q[1]), (0.05 if tall else 0.025, h, 0.04), rail, yaw)
        if spec.get('canopy') == 'skirt':
            _bc_skirt(self, mb, spec, at, d, nout, yaw, L, e, y, c, w, depth)
            continue
        # pent canopy: from the wall under the next storey (or roof) out past the rail
        top = (self.levels[k + 1] if k + 1 < self.storeys else self.roof_level) - spec.get('dropM', 0.15)
        if spec.get('canopy') == 'below':
            top = y - 0.18
        if spec.get('canopyTopM') is not None:
            top = y + float(spec['canopyTopM'])
        if spec.get('canopyAt') is not None:
            c = self.f2s(spec['canopyAt'], L, e)
        w = spec.get('canopyWidthM', w)
        cd = spec.get('canopyDepthM', depth + 0.35)
        low = top - spec.get('fallM', 0.75)
        w2 = w / 2 + spec.get('overhangM', 0.3)
        t = 0.08

        def P(sv, off, yy):
            q = at(sv, off)
            return B(q[0], yy, q[1])
        up = B(0, 1, 0)
        down = B(0, -1, 0)
        # roof skin (top) and painted soffit (underside), then the two gable ends
        mb.poly([P(c - w2, 0.02, top + t), P(c + w2, 0.02, top + t), P(c + w2, cd, low + t), P(c - w2, cd, low + t)], roofm, up)
        mb.poly([P(c - w2, 0.02, top), P(c + w2, 0.02, top), P(c + w2, cd, low), P(c - w2, cd, low)], soffit, down)
        for side in (-1, 1):
            sv = c + side * w2
            out = B(d[0] * side, 0, d[1] * side)
            mb.poly([P(sv, 0.02, top), P(sv, cd, low), P(sv, cd, low + t), P(sv, 0.02, top + t)], fascia, out)
        # fascia board along the low edge
        q = at(c, cd + 0.03)
        mb.box((q[0], low - 0.06 + t / 2, q[1]), (2 * w2, 0.22, 0.06), fascia, yaw)
        # rafters under the soffit: short stepped segments following the slope
        nr = int(spec.get('rafterCount', max(3, round(w / 1.0))))
        seg = 4
        for i in range(nr + 1):
            sv = c - w2 + 0.1 + i * (2 * w2 - 0.2) / nr
            for j in range(seg):
                f = (j + 0.5) / seg
                off = 0.02 + f * (cd - 0.02)
                yy = top - f * (top - low) - 0.07
                q = at(sv, off)
                mb.box((q[0], yy, q[1]), (0.1, 0.12, (cd - 0.02) / seg + 0.02), rafter, yaw)
        # triangular brackets at the ends and every other rafter bay, wall to fascia
        nb = int(spec.get('bracketCount', 3))
        for i in range(nb):
            sv = c - w / 2 + 0.15 + i * (w - 0.3) / max(1, nb - 1)
            yb = low - spec.get('bracketDropM', 0.3)
            side = B(d[0], 0, d[1])
            bm = rafter if (i % 2 == 0 or not spec.get('rafters2')) else _bc_mat(self, spec, 'rafters2', 'glazed-tile')
            if spec.get('bracketStruts'):
                # A diagonal strut wall->fascia plus a short post, not a flat triangle
                _bc_obox(mb, _bc_p3(at(sv, 0.03), yb), _bc_p3(at(sv, cd - 0.12), low - 0.04), _bc_p3(d, 0), 0.09, 0.09, bm)
                _bc_obox(mb, _bc_p3(at(sv, 0.03), yb), _bc_p3(at(sv, 0.03), top - 0.05), _bc_p3(d, 0), 0.09, 0.09, bm)
                continue
            for off_s in (-0.04, 0.04):
                mb.poly([P(sv + off_s, 0.02, top - 0.1), P(sv + off_s, 0.02, yb), P(sv + off_s, cd - 0.1, low - 0.05)], bm, side if off_s > 0 else -side)
        up = spec.get('upturnM') if spec.get('canopy') == 'below' else None
        if up:
            for side in (-1, 1):
                _bc_obox(mb, _bc_p3(at(c + side * (w2 - 0.25), cd), low + t), _bc_p3(at(c + side * (w2 + 0.2), cd + 0.15), low + t + up), _bc_p3(nout, 0), 0.1, 0.12, _bc_mat(self, spec, 'edge', 'glazed-tile'))
    mb.build(self.exterior)



def _bc_sign_bands(self, e, a, b):
    specs = self.style.get('signBands') or []
    if not specs:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    sf = self.style.get('storefront') or {}
    head = self.spec['floors']['groundM'] - sf.get('headDropM', 0.35)
    mb = MeshBuilder('sign-bands')
    for spec in specs:
        s0, s1 = sorted((self.f2s(spec['from'], L, e), self.f2s(spec['to'], L, e)))
        h = spec.get('heightM', 0.7)
        y0 = spec.get('bottomM', head + 0.05)
        pr = spec.get('projectionM', 0.08)
        q = add(add(a, mul(d, (s0 + s1) / 2)), mul(nout, pr / 2))
        mb.box((q[0], y0 + h / 2, q[1]), (s1 - s0, h, pr), self.tinted(spec.get('material', 'signage'), spec.get('tint')), yaw)
        bd = spec.get('border')
        if bd:
            # a raised moulded frame round the board face (blank; no lettering)
            bm = self.tinted(bd.get('material', 'painted-white'), bd.get('tint'))
            bw = bd.get('widthM', 0.05)
            q = add(add(a, mul(d, (s0 + s1) / 2)), mul(nout, pr + 0.01))
            for yy in (y0 + bw / 2, y0 + h - bw / 2):
                mb.box((q[0], yy, q[1]), (s1 - s0, bw, 0.03), bm, yaw)
            for sv in (s0 + bw / 2, s1 - bw / 2):
                q2 = add(add(a, mul(d, sv)), mul(nout, pr + 0.01))
                mb.box((q2[0], y0 + h / 2, q2[1]), (bw, h, 0.03), bm, yaw)
        gl = spec.get('glyphs')
        if gl:
            _bc_glyphs(self, mb, gl, a, d, nout, yaw, s0, s1, y0, h, pr + 0.012)
    mb.build(self.exterior)


# `glyphs: {count, material, tint, seed, fill}` on a sign band / crown
# nameboard - procedural, illegible brush-stroke marks in `count` character
# cells (reads as lettering at street distance; spells nothing).
def _bc_glyphs(self, mb, gl, a, d, nout, yaw, s0, s1, y0, h, off):
    import random as _r
    rng = _r.Random(gl.get('seed', 5))
    n = max(1, int(gl.get('count', 4)))
    gm = self.tinted(gl.get('material', 'signage'), gl.get('tint'))
    W = (s1 - s0) * gl.get('fill', 0.8)
    ch = min(h * 0.72, W / n * 0.86)
    gap = (W - n * ch) / max(1, n)
    x = (s0 + s1) / 2 - W / 2 + gap / 2
    sw = ch * 0.13
    yc = y0 + h / 2
    for i in range(n):
        cx0 = x + i * (ch + gap)
        for _ in range(rng.randint(4, 7)):
            kind = rng.random()
            if kind < 0.45:
                ln = ch * rng.uniform(0.4, 1.0)
                cx = cx0 + rng.uniform(ln / 2, ch - ln / 2)
                cy = yc + rng.uniform(-ch / 2 + sw, ch / 2 - sw)
                q = add(add(a, mul(d, cx)), mul(nout, off))
                mb.box((q[0], cy, q[1]), (ln, sw, 0.012), gm, yaw)
            elif kind < 0.85:
                ln = ch * rng.uniform(0.35, 0.95)
                cx = cx0 + rng.uniform(sw, ch - sw)
                cy = yc + rng.uniform(-ch / 2 + ln / 2, ch / 2 - ln / 2)
                q = add(add(a, mul(d, cx)), mul(nout, off))
                mb.box((q[0], cy, q[1]), (sw, ln, 0.012), gm, yaw)
            else:
                cx = cx0 + rng.uniform(0.25, 0.75) * ch
                cy = yc + rng.uniform(-0.25, 0.25) * ch
                dx = rng.choice((-1, 1)) * ch * 0.3
                p0 = _bc_p3(add(add(a, mul(d, cx - dx)), mul(nout, off)), cy - ch * 0.3)
                p1 = _bc_p3(add(add(a, mul(d, cx + dx)), mul(nout, off)), cy + ch * 0.3)
                _bc_obox(mb, p0, p1, _bc_p3(nout, 0), sw, 0.012, gm)


# ---- extras (bldg-256729086): every key below is NEW and optional;
# absent = unchanged builds. Exterior-only; materials are kit names or `int-`
# finishes (the exterior contract allows both).
#   balconyCanopies[].canopy = "skirt": a painted eave hung under a balcony
#       slab, sloping from the slab edge down and back toward the wall so its
#       painted soffit faces the street: `canopyDepthM` (top edge out),
#       `skirtInM` (lower edge out), `fallM`, `skirtTopDropM`, `rafters` and
#       `rafters2` (alternating), `edge` (lower tile edge), `upturnM`.
#   fireEscapes[].grating: true - slatted deck under every platform + a
#       perforated tread plate; `dropLadderAt`/`dropLadderM`/`dropLadderRunM`
#       hang a stowed counterweighted drop stair instead of the hung ladder.
#   parapetCrown: {at, widthM, depthM, backM, eaveM, ridgeM, roof, soffit,
#       fascia, rafters, upturnM} - a small hipped tiled crown over the parapet
#       with upturned front corners (heights above the roof level).
#   fretBands: [{from, to, yM, heightM, material, tint, reliefM}] - a raised
#       key (meander) band in relief on the front.
#   stoneVeins: {material, count (per m2), seed, widthM} - pale veins on the
#       ground band's piers and fascia (between storefront layout items).
#   railPanels: [{at, widthM, storey, heightM, depthM, panel, bars}] - a
#       lattice panel set into a balcony railing.
#   shopDisplays: [{from, to, backM, heightM, tiers, seed, palette}] - stocked
#       shelving and a display table behind storefront windows.
#   lanterns: [{at, offM, yM, radiusM, heightM, material, cap}]
#   acUnits: [{at, storey, yM, widthM}]; downpipes: [{at, fromM, toM}]
#   bladeBoards: [{at, bottomM, heightM, widthM, material, tint, border}]
INTERIOR.setdefault('int-ext-stone', (0.028, 0.05, 0.042, 0.1, 0.0))
INTERIOR.setdefault('int-ext-vein', (0.34, 0.4, 0.36, 0.14, 0.0))


def _bc_p3(q, y):
    return (q[0], y, q[1])


def _bc_v(a, b, s=1.0):
    return (a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s)


def _bc_n3(v):
    m = math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) or 1.0
    return (v[0] / m, v[1] / m, v[2] / m)


def _bc_cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _bc_obox(mb, p0, p1, side, w, t, mat):
    """Oriented box from p0 to p1 (frame 3-tuples), width w along `side`, thickness t."""
    u = _bc_n3(_bc_v(p1, p0, -1))
    dp = side[0] * u[0] + side[1] * u[1] + side[2] * u[2]
    s = _bc_n3(_bc_v(side, u, -dp))
    n = _bc_cross(u, s)

    def C(p, i, j):
        return B(*_bc_v(_bc_v(p, s, i * w / 2), n, j * t / 2))

    def H(v, k=1):
        return B(v[0] * k, v[1] * k, v[2] * k)
    mb.quad([C(p0, 1, -1), C(p1, 1, -1), C(p1, 1, 1), C(p0, 1, 1)], mat, H(s))
    mb.quad([C(p0, -1, -1), C(p1, -1, -1), C(p1, -1, 1), C(p0, -1, 1)], mat, H(s, -1))
    mb.quad([C(p0, -1, 1), C(p1, -1, 1), C(p1, 1, 1), C(p0, 1, 1)], mat, H(n))
    mb.quad([C(p0, -1, -1), C(p1, -1, -1), C(p1, 1, -1), C(p0, 1, -1)], mat, H(n, -1))
    mb.quad([C(p0, -1, -1), C(p0, 1, -1), C(p0, 1, 1), C(p0, -1, 1)], mat, H(u, -1))
    mb.quad([C(p1, -1, -1), C(p1, 1, -1), C(p1, 1, 1), C(p1, -1, 1)], mat, H(u))


def _bc_prism(mb, c, r, h, sides, mat, cap=None, bulge=1.0):
    """Upright n-sided prism (lanterns): centre c (frame 3-tuple) at mid height."""
    ring = []
    for i in range(sides):
        t = 2 * math.pi * i / sides
        ring.append((math.cos(t), math.sin(t)))
    lv = [(-h / 2, 0.72), (-h / 4, 1.0 * bulge), (h / 4, 1.0 * bulge), (h / 2, 0.72)]
    for (y0, f0), (y1, f1) in zip(lv, lv[1:]):
        for i in range(sides):
            a0, a1 = ring[i], ring[(i + 1) % sides]
            pts = [B(c[0] + a0[0] * r * f0, c[1] + y0, c[2] + a0[1] * r * f0), B(c[0] + a1[0] * r * f0, c[1] + y0, c[2] + a1[1] * r * f0),
                   B(c[0] + a1[0] * r * f1, c[1] + y1, c[2] + a1[1] * r * f1), B(c[0] + a0[0] * r * f1, c[1] + y1, c[2] + a0[1] * r * f1)]
            mid = ((a0[0] + a1[0]) / 2, (a0[1] + a1[1]) / 2)
            mb.quad(pts, mat, B(mid[0], 0, mid[1]))
    cm = cap or mat
    for y, f, sgn in ((-h / 2, 0.72, -1), (h / 2, 0.72, 1)):
        mb.quad([B(c[0] + a[0] * r * f, c[1] + y, c[2] + a[1] * r * f) for a in ring], cm, B(0, sgn, 0))
        mb.box((c[0], c[1] + y + sgn * 0.03, c[2]), (r * 1.1, 0.06, r * 1.1), cm)


def _bc_skirt(self, mb, spec, at, d, nout, yaw, L, e, y, c, w, depth):
    soffit = _bc_mat(self, spec, 'soffit', 'painted-white')
    raf = _bc_mat(self, spec, 'rafters', 'signal-red')
    raf2 = _bc_mat(self, spec, 'rafters2', 'glazed-tile') if spec.get('rafters2') else raf
    edge = _bc_mat(self, spec, 'edge', 'glazed-tile')
    fascia = _bc_mat(self, spec, 'fascia', 'signal-red')
    roofm = _bc_mat(self, spec, 'roof', 'glazed-tile')
    if spec.get('canopyAt') is not None:
        c = self.f2s(spec['canopyAt'], L, e)
    w = spec.get('canopyWidthM', w)
    w2 = w / 2
    yt = y - spec.get('skirtTopDropM', 0.2)
    ot = spec.get('canopyDepthM', 1.25)
    yb = yt - spec.get('fallM', 0.85)
    ob = spec.get('skirtInM', 0.5)

    def P(sv, off, yy):
        q = at(sv, off)
        return B(q[0], yy, q[1])
    n3 = _bc_v(_bc_p3(nout, 0), (0, 1, 0), 0)
    face = _bc_n3((n3[0] * (yt - yb), ob - ot, n3[2] * (yt - yb)))
    fB = B(*face)
    mb.quad([P(c - w2, ot, yt), P(c + w2, ot, yt), P(c + w2, ob, yb), P(c - w2, ob, yb)], soffit, fB)
    mb.quad([P(c - w2, ot, yt), P(c + w2, ot, yt), P(c + w2, ob, yb), P(c - w2, ob, yb)], soffit, -fB)
    mb.quad([P(c - w2, 0.02, yb), P(c + w2, 0.02, yb), P(c + w2, ob, yb), P(c - w2, ob, yb)], soffit, B(0, -1, 0))
    mb.quad([P(c - w2, 0.02, yt), P(c + w2, 0.02, yt), P(c + w2, ot, yt), P(c - w2, ot, yt)], roofm, B(0, 1, 0))
    for side in (-1, 1):
        sv = c + side * w2
        mb.poly([P(sv, 0.02, yt), P(sv, ot, yt), P(sv, ob, yb), P(sv, 0.02, yb)], soffit, B(d[0] * side, 0, d[1] * side))
    # top beam along the slab edge, tile edge along the lower rim
    q = at(c, ot + 0.04)
    mb.box((q[0], yt - 0.08, q[1]), (w + 0.12, 0.2, 0.09), fascia, yaw)
    q = at(c, ob + 0.03)
    mb.box((q[0], yb - 0.04, q[1]), (w + 0.06, 0.12, 0.14), edge, yaw)
    # slim alternating rafters on the painted face, top beam to tile edge
    nr = int(spec.get('rafterCount', max(4, round(w / 0.9))))
    for i in range(nr + 1):
        sv = c - w2 + 0.08 + i * (w - 0.16) / nr
        p0 = _bc_v(_bc_p3(at(sv, ot), yt - 0.02), face, 0.04)
        p1 = _bc_v(_bc_p3(at(sv, ob), yb + 0.02), face, 0.04)
        _bc_obox(mb, p0, p1, _bc_p3(d, 0), spec.get('rafterW', 0.09), 0.07, raf if i % 2 == 0 else raf2)
    # end brackets from the wall up to the lower rim, and upturned corners
    for side in (-1, 1):
        sv = c + side * (w2 - 0.1)
        _bc_obox(mb, _bc_p3(at(sv, 0.03), yb - spec.get('bracketDropM', 0.55)), _bc_p3(at(sv, ob), yb - 0.02), _bc_p3(d, 0), 0.09, 0.09, raf)
        up = spec.get('upturnM', 0.22)
        if up:
            _bc_obox(mb, _bc_p3(at(c + side * (w2 - 0.05), ot + 0.04), yt - 0.08), _bc_p3(at(c + side * (w2 + 0.3), ot + 0.16), yt + up), _bc_p3(nout, 0), 0.09, 0.12, fascia)


def _bc_fe_grating(self, e, a, b, item):
    if not item.get('grating') or self.storeys < 2:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    c, w, depth = self.f2s(item['at'], L, e), item.get('widthM', 3.4), item.get('depthM', 0.9)
    mat = item.get('material', 'painted-steel')
    mb = MeshBuilder('fire-escape-grating')
    k_last = min(self.storeys - 1, item.get('toStorey', self.storeys - 1))
    for k in range(1, k_last + 1):
        y = self.levels[k] - 0.02
        q = at_edge(a, d, nout, c, depth / 2 + 0.02)
        mb.box((q[0], y + 0.033, q[1]), (w - 0.04, 0.012, depth - 0.04), item.get('deckMaterial', 'perforated-metal'), yaw)
        n = max(2, int(w / item.get('slatM', 0.14)))
        for i in range(n + 1):
            q = at_edge(a, d, nout, c - w / 2 + 0.03 + (w - 0.06) * i / n, depth / 2 + 0.02)
            mb.box((q[0], y - 0.06, q[1]), (0.025, 0.07, depth), mat, yaw, skip=('top',))
        for off in (0.05, depth - 0.02):
            q = at_edge(a, d, nout, c, off)
            mb.box((q[0], y - 0.07, q[1]), (w, 0.14, 0.05), mat, yaw)
        # diagonal knee braces back to the wall
        for sv in (c - w / 2 + 0.2, c + w / 2 - 0.2):
            _bc_obox(mb, _bc_p3(at_edge(a, d, nout, sv, 0.03), y - 0.75), _bc_p3(at_edge(a, d, nout, sv, depth - 0.1), y - 0.08), _bc_p3(d, 0), 0.05, 0.05, mat)
    mb.build(self.exterior)


def _bc_extras(self, e, a, b):
    s = self.style
    keys = ('parapetCrown', 'fretBands', 'stoneVeins', 'railPanels', 'shopDisplays', 'lanterns', 'acUnits', 'downpipes', 'bladeBoards')
    if not any(s.get(k) for k in keys):
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    d3, n3 = _bc_p3(d, 0), _bc_p3(nout, 0)

    def at(sv, off):
        return at_edge(a, d, nout, sv, off)

    def P(sv, off, yy):
        q = at(sv, off)
        return B(q[0], yy, q[1])

    # -- tiled crown over the parapet
    cr = s.get('parapetCrown')
    if cr:
        mb = MeshBuilder('balcony-canopy-crown')
        c, w2 = self.f2s(cr['at'], L, e), cr.get('widthM', 6.0) / 2
        dep, back = cr.get('depthM', 0.9), cr.get('backM', 0.5)
        ye, yr = self.roof_level + cr.get('eaveM', 0.55), self.roof_level + cr.get('ridgeM', 1.1)
        wr = max(0.3, w2 - cr.get('hipM', 0.9))
        roofm = _bc_mat(self, cr, 'roof', 'glazed-tile')
        soff = _bc_mat(self, cr, 'soffit', 'painted-white')
        fas = _bc_mat(self, cr, 'fascia', 'glazed-tile')
        raf = _bc_mat(self, cr, 'rafters', 'signal-red')
        mid = (dep - back) / 2
        mb.quad([P(c - w2, dep, ye), P(c + w2, dep, ye), P(c + wr, mid, yr), P(c - wr, mid, yr)], roofm, B(*_bc_v(n3, (0, 1, 0))))
        mb.quad([P(c - w2, -back, ye), P(c + w2, -back, ye), P(c + wr, mid, yr), P(c - wr, mid, yr)], roofm, B(*_bc_v(_bc_v((0, 0, 0), n3, -1), (0, 1, 0))))
        for side in (-1, 1):
            mb.poly([P(c + side * w2, dep, ye), P(c + side * wr, mid, yr), P(c + side * w2, -back, ye)], roofm, B(*_bc_v(_bc_v((0, 0, 0), d3, side), (0, 1, 0))))
        mb.quad([P(c - w2, dep, ye - 0.02), P(c + w2, dep, ye - 0.02), P(c + w2, -back, ye - 0.02), P(c - w2, -back, ye - 0.02)], soff, B(0, -1, 0))
        # ridge with end finials
        q = at(c, mid)
        mb.box((q[0], yr + 0.04, q[1]), (2 * wr + 0.1, 0.1, 0.14), fas, yaw)
        # teal fascia board along the front eave, rafter tails under it
        q = at(c, dep + 0.03)
        mb.box((q[0], ye - 0.1, q[1]), (2 * w2 + 0.04, 0.2, 0.06), fas, yaw)
        nr = int(cr.get('rafterCount', max(6, round(2 * w2 / 0.45))))
        for i in range(nr + 1):
            q = at(c - w2 + 0.1 + i * (2 * w2 - 0.2) / nr, dep - 0.3)
            mb.box((q[0], ye - 0.08, q[1]), (0.07, 0.09, 0.58), raf, yaw)
        # upturned front corners (flying eaves)
        up = cr.get('upturnM', 0.35)
        for side in (-1, 1):
            _bc_obox(mb, _bc_p3(at(c + side * (w2 - 0.2), dep), ye + 0.02), _bc_p3(at(c + side * (w2 + 0.3), dep + 0.18), min(ye + up, self.roof_level + cr.get('capM', 1.08))), n3, 0.12, 0.1, roofm)
        if cr.get('finialM'):
            for side in (-1, 1):
                _bc_obox(mb, _bc_p3(at(c + side * (wr - 0.05), mid), yr), _bc_p3(at(c + side * (wr + 0.22), mid), min(yr + cr['finialM'], self.roof_level + cr.get('capM', 1.08))), n3, 0.1, 0.12, fas)
        nb = cr.get('nameboard')
        if nb:
            # A cream board under the crown with a dark border and glyph marks
            nw, nbot, ntop = nb.get('widthM', 2 * w2 - 1.0), self.roof_level + nb.get('bottomM', -0.55), ye - nb.get('topDropM', 0.18)
            nm = self.tinted(nb.get('material', 'painted-white'), nb.get('tint'))
            q = at(c, 0.07)
            mb.box((q[0], (nbot + ntop) / 2, q[1]), (nw, ntop - nbot, 0.08), nm, yaw)
            bm = self.tinted(nb.get('borderMaterial', 'signage'), nb.get('borderTint'))
            q = at(c, 0.12)
            for yy in (nbot + 0.03, ntop - 0.03):
                mb.box((q[0], yy, q[1]), (nw, 0.06, 0.03), bm, yaw)
            for side in (-1, 1):
                q2 = at(c + side * (nw / 2 - 0.03), 0.12)
                mb.box((q2[0], (nbot + ntop) / 2, q2[1]), (0.06, ntop - nbot, 0.03), bm, yaw)
            if nb.get('glyphs'):
                _bc_glyphs(self, mb, nb['glyphs'], a, d, nout, yaw, c - nw / 2 + 0.1, c + nw / 2 - 0.1, nbot + 0.06, ntop - nbot - 0.12, 0.125)
        # frieze under the crown on the parapet face, posts at its ends
        q = at(c, 0.05)
        pt = self.roof_level + cr.get('friezeBottomM', 0.05)
        mb.box((q[0], (pt + ye - 0.2) / 2, q[1]), (2 * w2 - 0.4, max(0.05, ye - 0.2 - pt), 0.06), fas, yaw)
        for side in (-1, 1):
            q = at(c + side * (w2 - 0.25), dep - 0.2)
            mb.box((q[0], (pt + ye) / 2, q[1]), (0.12, ye - pt, 0.12), raf, yaw)
        mb.build(self.exterior)

    # -- raised key (meander) bands
    fb = s.get('fretBands') or []
    if fb:
        mb = MeshBuilder('facade-fret')
        segs = (((0.5, 0), (0.5, 4)), ((0.5, 4), (4, 4)), ((4, 4), (4, 1.5)), ((4, 1.5), (2, 1.5)))
        for spec in fb:
            s0, s1 = sorted((self.f2s(spec['from'], L, e), self.f2s(spec['to'], L, e)))
            h = spec.get('heightM', 0.4)
            y0 = spec['yM']
            u = h / 5.0
            t = u * 0.7
            rl = spec.get('reliefM', 0.035)
            m = self.tinted(spec.get('material', 'cast-stone-trim'), spec.get('tint'))
            for yy in (y0 + t / 2, y0 + h - t / 2):
                q = at((s0 + s1) / 2, rl / 2)
                mb.box((q[0], yy, q[1]), (s1 - s0, t, rl), m, yaw)
            period = 5 * u
            n = max(1, int((s1 - s0) / period))
            x0 = s0 + ((s1 - s0) - n * period) / 2
            for i in range(n):
                for (ax, ay), (bx, by) in segs:
                    sx0, sx1 = sorted((x0 + i * period + ax * u, x0 + i * period + bx * u))
                    sy0, sy1 = sorted((y0 + ay * u, y0 + by * u))
                    q = at((sx0 + sx1) / 2, rl / 2)
                    mb.box((q[0], (sy0 + sy1) / 2, q[1]), (sx1 - sx0 + t, sy1 - sy0 + t, rl), m, yaw)
        mb.build(self.exterior)

    # -- veins on the polished stone piers and fascia
    sv_spec = s.get('stoneVeins')
    band_top = self.band_top
    if sv_spec and band_top:
        import random as _r
        rng = _r.Random(sv_spec.get('seed', 7))
        sf = s.get('storefront') or {}
        head = self.spec['floors']['groundM'] - sf.get('headDropM', 0.35)
        spans = sorted(tuple(sorted((self.f2s(it['from'], L, e), self.f2s(it['to'], L, e)))) for it in sf.get('layout') or [])
        panels, cur = [], 0.0
        for s0, s1 in spans:
            if s0 - cur > 0.08:
                panels.append((cur, s0, 0.0, head))
            cur = max(cur, s1)
        if L - cur > 0.08:
            panels.append((cur, L, 0.0, head))
        panels.append((0.0, L, head, band_top))
        vm = self.tinted(sv_spec.get('material', 'int-ext-vein'), sv_spec.get('tint'))
        mb = MeshBuilder('stone-veins')
        dens = sv_spec.get('count', 1.3)
        for (p0, p1, q0, q1) in panels:
            area = (p1 - p0) * (q1 - q0)
            for _ in range(max(1, int(round(area * dens)))):
                x, yv = rng.uniform(p0, p1), rng.uniform(q0, q1)
                ang = rng.choice((-1, 1)) * rng.uniform(0.35, 1.2)
                wv = sv_spec.get('widthM', 0.012) * rng.uniform(0.5, 1.4)
                for _k in range(rng.randint(3, 8)):
                    ang += rng.uniform(-0.45, 0.45)
                    ln = rng.uniform(0.08, 0.3)
                    nx, ny = x + math.cos(ang) * ln, yv + math.sin(ang) * ln
                    if not (p0 + 0.02 < nx < p1 - 0.02 and q0 + 0.02 < ny < q1 - 0.02):
                        break
                    # thin quad from (x, yv) to (nx, ny) on the outer face
                    ox, oy = -math.sin(ang) * wv / 2, math.cos(ang) * wv / 2
                    mb.quad([P(x - ox, 0.004, yv - oy), P(nx - ox, 0.004, ny - oy), P(nx + ox, 0.004, ny + oy), P(x + ox, 0.004, yv + oy)], vm, B(*n3))
                    x, yv = nx, ny
        # polished slab joints: a faint lighter line between stone panels on the piers
        for (p0, p1, q0, q1) in panels[:-1]:
            for yy in (q0 + 1.3, q0 + 2.6):
                if yy < q1 - 0.1:
                    q = at((p0 + p1) / 2, 0.003)
                    mb.quad([P(p0, 0.003, yy - 0.004), P(p1, 0.003, yy - 0.004), P(p1, 0.003, yy + 0.004), P(p0, 0.003, yy + 0.004)], vm, B(*n3))
        if not mb.empty():
            mb.build(self.exterior)

    # -- lattice panels in balcony railings
    rp = s.get('railPanels') or []
    if rp:
        mb = MeshBuilder('balcony-canopy-panel')
        for spec in rp:
            k = max(1, min(self.storeys - 1, int(spec.get('storey', 2))))
            y = self.levels[k]
            c, w = self.f2s(spec['at'], L, e), spec.get('widthM', 2.0)
            h = spec.get('heightM', 0.75)
            off = spec.get('depthM', 1.0) - 0.05
            pm = _bc_mat(self, spec, 'panel', 'glazed-tile')
            bm = _bc_mat(self, spec, 'bars', 'painted-steel')
            q = at(c, off)
            mb.box((q[0], y + 0.15 + h / 2, q[1]), (w, h, 0.02), pm, yaw)
            nb = max(2, int(w / 0.25))
            for i in range(nb + 1):
                q = at(c - w / 2 + i * w / nb, off + 0.02)
                mb.box((q[0], y + 0.15 + h / 2, q[1]), (0.035, h, 0.03), bm, yaw)
            for f in (0.0, 0.33, 0.66, 1.0):
                q = at(c, off + 0.02)
                mb.box((q[0], y + 0.15 + h * f, q[1]), (w, 0.035, 0.03), bm, yaw)
            # a round medallion at the centre
            _bc_prism(mb, _bc_p3(at(c, off + 0.04), y + 0.15 + h / 2), h * 0.3, 0.04, 10, bm)
        mb.build(self.exterior)

    # -- stocked shop windows
    sd = s.get('shopDisplays') or []
    if sd:
        import random as _r
        mb = MeshBuilder('shop-displays')
        for spec in sd:
            rng = _r.Random(spec.get('seed', 3))
            s0, s1 = sorted((self.f2s(spec['from'], L, e), self.f2s(spec['to'], L, e)))
            w = s1 - s0
            back = spec.get('backM', 1.2)
            hh = spec.get('heightM', 2.5)
            wood = spec.get('shelf', 'int-wood')
            pal = spec.get('palette') or ['int-fabric-red', 'int-fabric-green', 'int-ceramic', 'int-wood-light', 'int-fabric-blue', 'signal-red', 'int-bottle', 'int-leather']
            q = at((s0 + s1) / 2, -back)
            mb.box((q[0], hh / 2, q[1]), (w - 0.04, hh, 0.04), wood, yaw)
            tiers = spec.get('tiers', 4)
            for t in range(tiers):
                yy = 0.45 + t * (hh - 0.6) / max(1, tiers - 1)
                q = at((s0 + s1) / 2, -back + 0.2)
                mb.box((q[0], yy, q[1]), (w - 0.08, 0.03, 0.38), wood, yaw)
                x = s0 + 0.06
                while x < s1 - 0.12:
                    iw = rng.uniform(0.07, 0.24)
                    ih = rng.uniform(0.1, min(0.42, (hh - 0.6) / max(1, tiers - 1) - 0.06))
                    if x + iw > s1 - 0.06:
                        break
                    q = at(x + iw / 2, -back + 0.2 + rng.uniform(-0.08, 0.08))
                    mat = rng.choice(pal)
                    if rng.random() < 0.3:
                        _bc_prism(mb, (q[0], yy + 0.015 + ih / 2, q[1]), iw / 2, ih, 6, mat)
                    else:
                        mb.box((q[0], yy + 0.015 + ih / 2, q[1]), (iw, ih, rng.uniform(0.1, 0.26)), mat, yaw)
                    x += iw + rng.uniform(0.01, 0.05)
            # low display table near the glass with goods on it
            tq = at((s0 + s1) / 2, -0.55)
            mb.box((tq[0], 0.38, tq[1]), (w - 0.3, 0.76, 0.42), wood, yaw)
            x = s0 + 0.2
            while x < s1 - 0.3:
                iw = rng.uniform(0.12, 0.3)
                q = at(x + iw / 2, -0.55)
                ih = rng.uniform(0.08, 0.35)
                mb.box((q[0], 0.76 + ih / 2, q[1]), (iw, ih, rng.uniform(0.12, 0.3)), rng.choice(pal), yaw)
                x += iw + 0.05
            # hanging goods from the ceiling (lanterns, strings)
            for i in range(spec.get('hanging', 3)):
                q = at(s0 + (i + 0.5) * w / max(1, spec.get('hanging', 3)), -back * 0.6)
                _bc_prism(mb, (q[0], hh + 0.35, q[1]), 0.16, 0.3, 8, 'signal-red', 'painted-white@c9a24a')
        mb.build(self.exterior)


    # -- lanterns, window A/C units, downpipes, projecting boards
    ln = s.get('lanterns') or []
    fx = MeshBuilder('canopy-lanterns')
    for spec in ln:
        q = at(self.f2s(spec['at'], L, e), spec.get('offM', 0.8))
        yy = spec['yM']
        _bc_prism(fx, (q[0], yy, q[1]), spec.get('radiusM', 0.22), spec.get('heightM', 0.42), 10,
                  self.tinted(spec.get('material', 'signal-red'), spec.get('tint')), spec.get('cap', 'painted-white@c9a24a'))
        fx.box((q[0], yy + spec.get('heightM', 0.42) / 2 + 0.2, q[1]), (0.015, 0.4, 0.015), 'painted-steel')
        fx.box((q[0], yy - spec.get('heightM', 0.42) / 2 - 0.12, q[1]), (0.03, 0.2, 0.03), spec.get('tassel', 'signal-red'))
    if not fx.empty():
        fx.build(self.exterior)
    mb = MeshBuilder('facade-fixtures')
    for spec in s.get('acUnits') or []:
        k = max(0, min(self.storeys - 1, int(spec.get('storey', 1))))
        yy = self.levels[k] + spec.get('yM', 0.9)
        wv = spec.get('widthM', 0.62)
        q = at(self.f2s(spec['at'], L, e), 0.18)
        mb.box((q[0], yy, q[1]), (wv, 0.42, 0.4), 'painted-white@d8d6cf', yaw)
        q2 = at(self.f2s(spec['at'], L, e), 0.385)
        for i in range(6):
            mb.box((q2[0], yy - 0.15 + i * 0.06, q2[1]), (wv - 0.08, 0.015, 0.012), 'painted-steel-mid', yaw)
        for side in (-1, 1):
            q3 = at(self.f2s(spec['at'], L, e) + side * (wv / 2 - 0.05), 0.2)
            _bc_obox(mb, _bc_p3(at(self.f2s(spec['at'], L, e) + side * (wv / 2 - 0.05), 0.02), yy - 0.55), _bc_p3(q3, yy - 0.22), d3, 0.03, 0.03, 'painted-steel-mid')
    for spec in s.get('downpipes') or []:
        sv = self.f2s(spec['at'], L, e)
        y0, y1 = spec.get('fromM', 0.3), spec.get('toM', self.roof_level)
        q = at(sv, 0.08)
        mb.box((q[0], (y0 + y1) / 2, q[1]), (0.09, y1 - y0, 0.09), spec.get('material', 'painted-steel-light'), yaw)
        yy = y0 + 0.4
        while yy < y1:
            q2 = at(sv, 0.04)
            mb.box((q2[0], yy, q2[1]), (0.14, 0.04, 0.08), 'painted-steel-mid', yaw)
            yy += 1.6
        q = at(sv, 0.2)
        mb.box((q[0], y1 + 0.05, q[1]), (0.2, 0.18, 0.25), spec.get('material', 'painted-steel-light'), yaw)
    if not mb.empty():
        mb.build(self.exterior)
    bb = s.get('bladeBoards') or []
    if bb:
        mb = MeshBuilder('sign-blades')
        for spec in bb:
            sv = self.f2s(spec['at'], L, e)
            y0, h, wv = spec.get('bottomM', 3.0), spec.get('heightM', 1.0), spec.get('widthM', 0.55)
            face = self.tinted(spec.get('material', 'painted-white'), spec.get('tint'))
            q = at(sv, 0.12 + wv / 2)
            mb.box((q[0], y0 + h / 2, q[1]), (0.08, h, wv), face, yaw)
            bd = spec.get('border')
            if bd:
                bm = self.tinted(bd.get('material', 'painted-white'), bd.get('tint'))
                for yy in (y0 + 0.025, y0 + h - 0.025):
                    mb.box((q[0], yy, q[1]), (0.1, 0.05, wv + 0.02), bm, yaw)
                for off in (0.12, 0.12 + wv):
                    q2 = at(sv, off)
                    mb.box((q2[0], y0 + h / 2, q2[1]), (0.1, h, 0.05), bm, yaw)
            for yy in (y0 + 0.15, y0 + h - 0.15):
                q2 = at(sv, 0.07)
                mb.box((q2[0], yy, q2[1]), (0.04, 0.04, 0.16), 'painted-steel', yaw)
        mb.build(self.exterior)



# --- bldg-222862957 block 3. Self-contained; every key is
# new and absent from DEFAULT_STYLE, so other buildings build unchanged.
#   floorStack      {groundM, upperM}: overrides the spec's civic floor stack
#                   (make-spec's tallGround stack gives a 6 m shop storey where
#                   a street photo shows ~4.5 m); the LiDAR height still caps
#                   the wall, the gap above the roof level becomes parapet.
#   tileEave        {edges:[i..], projectionM, dropM, material, soffit,
#                   bracket, bracketEveryM, ribEveryM}: a Mission-revival
#                   clay-tile pent eave hung off the parapet top along the
#                   listed street edges (and around `cornerBay`).
#   cornerBay       {edge, extendM, sagittaM, segments, material, tint,
#                   archWindows, fromM, toM}: a bowed bay across a chamfer
#                   edge with one tall arch-headed window per facet.
#   storefrontPiers {material, tint, widthM, depthM}: pilasters between and
#                   at the ends of every storefront layout item, from the
#                   pavement to the storefront head (piers in their own paint
#                   instead of the wall colour).
#   upperFlats      {count}: shallow-plan upper storeys as small flats along
#                   the street wall, off a rear corridor that wraps the stair.
#   groundUnits     {splitAt:[layout fractions], kinds:[...]}: the ground
#                   storey as several small units (shop / cafe / bar) with
#                   partitions at the listed front-layout fractions.

_r3_init_base = Building.__init__


def _r3_init(self, spec, style):
    fs = style.get('floorStack')
    if fs:
        f = dict(spec['floors'])
        f['groundM'] = float(fs.get('groundM', f['groundM']))
        f['upperM'] = float(fs.get('upperM', f['upperM']))
        top = f['groundM'] + (spec['storeys'] - 1) * f['upperM']
        f['roofM'] = round(max(0.3, spec['heightM'] - top), 3)
        spec = dict(spec, floors=f)
    _r3_init_base(self, spec, style)


Building.__init__ = _r3_init


def _r3_outward(self, a, b):
    return mul(self.inward(a, b), -1)


def _r3_eave_path(self, mb, pts, normals, y_w, cfg, closed=False):
    """A sloped clay-tile pent roof along a polyline at the wall face.
    pts: 2-D points; normals: outward 2-D unit normals per point (mitred)."""
    proj = float(cfg.get('projectionM', 0.75))
    drop = float(cfg.get('dropM', 0.38))
    tile = cfg.get('material', 'terracotta@c0603f')
    soffit = cfg.get('soffit', 'wood-siding@6b4a30')
    rib_every = float(cfg.get('ribEveryM', 0.2))
    up = B(0, 1, 0)
    for i in range(len(pts) - 1):
        p, q = pts[i], pts[i + 1]
        np_, nq = normals[i], normals[i + 1]
        pl, ql = add(p, mul(np_, proj)), add(q, mul(nq, proj))
        # slope (tiles), soffit, fascia lip
        mb.quad([B(p[0], y_w, p[1]), B(q[0], y_w, q[1]), B(ql[0], y_w - drop, ql[1]), B(pl[0], y_w - drop, pl[1])], tile, up)
        mb.quad([B(p[0], y_w - 0.14, p[1]), B(q[0], y_w - 0.14, q[1]), B(ql[0], y_w - drop - 0.1, ql[1]), B(pl[0], y_w - drop - 0.1, pl[1])], soffit, B(0, -1, 0))
        seg_n = norm(add(np_, nq))
        mb.quad([B(pl[0], y_w - drop - 0.1, pl[1]), B(ql[0], y_w - drop - 0.1, ql[1]), B(ql[0], y_w - drop + 0.01, ql[1]), B(pl[0], y_w - drop + 0.01, pl[1])], tile, B(seg_n[0], 0, seg_n[1]))
        # barrel courses: a ridge down the slope every `ribEveryM`
        L = length(sub(q, p))
        if L < 0.05:
            continue
        d = norm(sub(q, p))
        n = int(L // rib_every)
        for j in range(n):
            t = (j + 0.5) / n
            w0 = lerp2(p, q, t)
            w1 = lerp2(pl, ql, t)
            hw = mul(d, rib_every * 0.32)
            A0, A1 = B(w0[0], y_w + 0.005, w0[1]), B(w1[0], y_w - drop + 0.005, w1[1])
            R0, R1 = B(w0[0], y_w + 0.06, w0[1]), B(w1[0], y_w - drop + 0.06, w1[1])
            L0, L1 = B(w0[0] - hw[0], y_w + 0.005, w0[1] - hw[1]), B(w1[0] - hw[0], y_w - drop + 0.005, w1[1] - hw[1])
            G0, G1 = B(w0[0] + hw[0], y_w + 0.005, w0[1] + hw[1]), B(w1[0] + hw[0], y_w - drop + 0.005, w1[1] + hw[1])
            mb.quad([L0, L1, R1, R0], tile, B(-d[0], 1, -d[1]))
            mb.quad([R0, R1, G1, G0], tile, B(d[0], 1, d[1]))
            # the round barrel end at the lip
            mb.quad([L1, G1, B(w1[0] + hw[0], y_w - drop - 0.05, w1[1] + hw[1]), B(w1[0] - hw[0], y_w - drop - 0.05, w1[1] - hw[1])], tile, B(seg_n[0], 0, seg_n[1]))
    # brackets under the soffit
    br = cfg.get('bracket')
    if br:
        every = float(cfg.get('bracketEveryM', 0.9))
        for i in range(len(pts) - 1):
            p, q = pts[i], pts[i + 1]
            L = length(sub(q, p))
            nb = int(L // every)
            if nb < 1:
                continue
            nn = norm(add(normals[i], normals[i + 1]))
            yaw = box_yaw(norm(sub(q, p)))
            for j in range(nb):
                w = lerp2(p, q, (j + 0.5) / nb)
                c = add(w, mul(nn, proj * 0.35))
                mb.box((c[0], y_w - 0.3, c[1]), (0.1, 0.26, proj * 0.7), br, yaw)


def _r3_bay_ring(self, cb):
    e = next(x for x in self.edges if x['index'] == int(cb['edge']))
    a, b = v2(e['a']), v2(e['b'])
    n = len(self.poly)
    ext = float(cb.get('extendM', 0.5))
    # extend along the neighbouring edges (away from the chamfer)
    ia = e['index']
    prev_a = self.poly[(ia - 1) % n]
    next_b = self.poly[(ia + 2) % n]
    a2 = add(a, mul(norm(sub(prev_a, a)), ext))
    b2 = add(b, mul(norm(sub(next_b, b)), ext))
    nout = _r3_outward(self, a, b)
    chord = length(sub(b2, a2))
    sag = float(cb.get('sagittaM', 0.8))
    R = (chord * chord / 4 + sag * sag) / (2 * sag)
    mid = lerp2(a2, b2, 0.5)
    O = add(mid, mul(nout, sag - R))
    a0 = math.atan2(a2[1] - O[1], a2[0] - O[0])
    a1 = math.atan2(b2[1] - O[1], b2[0] - O[0])
    da = (a1 - a0 + math.pi) % (2 * math.pi) - math.pi
    segs = max(2, int(cb.get('segments', 3)))
    ring = [add(O, (R * math.cos(a0 + da * i / segs), R * math.sin(a0 + da * i / segs))) for i in range(segs + 1)]
    ring[0], ring[-1] = a2, b2
    return ring, O


def _r3_corner_bay(self):
    cb = self.style.get('cornerBay')
    if not cb:
        return
    s = self.style
    ring, O = _r3_bay_ring(self, cb)
    mat = self.tinted(cb.get('material', s['wall']), cb.get('tint', s.get('wallTint')))
    frame = self.tinted(cb.get('frame', s['frame']), cb.get('frameTint'))
    y0 = self.levels[1] - float(cb.get('corbelM', 0.45)) if self.storeys > 1 else 3.0
    y_floor = self.levels[1] if self.storeys > 1 else 3.2
    y1 = self.height - float(cb.get('belowTopM', 0.05))
    mb = MeshBuilder('corner-bay')
    glass = MeshBuilder('corner-bay-glazing')
    # corbel and head plates (a closed prism back along the chord)
    self._prism(mb, ring, y0, y_floor + 0.05, mat)
    self._prism(mb, ring, y1 - 0.2, y1, mat)
    win_w = float(cb.get('windowW', 0.85))
    sill = y_floor + float(cb.get('sillM', 0.7))
    head = min(y1 - 0.45, y_floor + float(cb.get('headM', 3.0)))
    # `archWindows: false`: plain square-headed sashes (no arch head)
    arch = cb.get('archWindows', True) is not False
    for fa, fb in zip(ring[:-1], ring[1:]):
        fl = length(sub(fb, fa))
        fd = norm(sub(fb, fa))
        mid = lerp2(fa, fb, 0.5)
        fn = norm(sub(mid, O))
        ia_, ib_ = sub(fa, mul(fn, 0.18)), sub(fb, mul(fn, 0.18))
        ww = max(0.4, min(fl - 0.35, win_w))
        r = ww / 2
        spring = head - r if arch else head
        wall_grid(mb, fa, fb, ia_, ib_, y_floor + 0.05, y1 - 0.2, [((fl - ww) / 2, (fl + ww) / 2, sill, head)],
                  mat, 'int-plaster', mat, cap_top=False)
        yaw = box_yaw(fd)
        gc = add(mid, mul(fn, -0.08))
        outv = B(fn[0], 0, fn[1])
        # spandrels: close the rectangle's head corners down to the arch
        segs = 8
        for side in ((-1, 1) if arch else ()):
            corner = add(mid, mul(fd, side * r))
            pts = [B(corner[0] + fn[0] * 0.002, head, corner[1] + fn[1] * 0.002)]
            for i in range(segs + 1):
                ang = math.pi / 2 * i / segs
                u = side * r * math.cos(ang)
                v = r * math.sin(ang)
                q = add(mid, mul(fd, u))
                pts.append(B(q[0] + fn[0] * 0.002, spring + v, q[1] + fn[1] * 0.002))
            mb.poly(pts, mat, outv)
            # arch reveal soffit (spandrel underside into the wall)
            for i in range(segs):
                a0_, a1_ = math.pi / 2 * i / segs, math.pi / 2 * (i + 1) / segs
                qa = add(mid, mul(fd, side * r * math.cos(a0_)))
                qb = add(mid, mul(fd, side * r * math.cos(a1_)))
                ya, yb = spring + r * math.sin(a0_), spring + r * math.sin(a1_)
                ra, rb = sub(qa, mul(fn, 0.18)), sub(qb, mul(fn, 0.18))
                mb.quad([B(qa[0], ya, qa[1]), B(qb[0], yb, qb[1]), B(rb[0], yb, rb[1]), B(ra[0], ya, ra[1])], mat, B(-fd[0] * side, -1, -fd[1] * side))
        # glass: rectangle to the springing line plus the half-round head
        glass.box((gc[0], (sill + spring) / 2, gc[1]), (ww, spring - sill, 0.02), s['glass'], yaw)
        fan = [B(gc[0], spring, gc[1])]
        for i in range(13):
            ang = math.pi * i / 12
            q = add(gc, mul(fd, r * math.cos(ang)))
            fan.append(B(q[0], spring + r * math.sin(ang), q[1]))
        if arch:
            glass.poly(fan, s['glass'], outv)
        self._glass_backing(glass, s, 'bay', gc, sill, head, ww, yaw, mul(fn, -1))
        # frame: jambs, sill, meeting rail, arch ring
        for side in (-1, 1):
            q = add(gc, mul(fd, side * (r - 0.03)))
            glass.box((q[0], (sill + spring) / 2, q[1]), (0.06, spring - sill, 0.08), frame, yaw)
        glass.box((gc[0], sill + 0.03, gc[1]), (ww + 0.1, 0.07, 0.14), frame, yaw)
        glass.box((gc[0], sill + (spring - sill) * 0.55, gc[1]), (ww, 0.05, 0.07), frame, yaw)
        glass.box((gc[0], spring, gc[1]), (ww, 0.05, 0.07), frame, yaw)
        for i in (range(12) if arch else ()):
            a0_, a1_ = math.pi * i / 12, math.pi * (i + 1) / 12
            for (ro, ri) in ((r, r - 0.06),):
                qa_o = add(gc, mul(fd, ro * math.cos(a0_)))
                qb_o = add(gc, mul(fd, ro * math.cos(a1_)))
                qa_i = add(gc, mul(fd, ri * math.cos(a0_)))
                qb_i = add(gc, mul(fd, ri * math.cos(a1_)))
                off = mul(fn, 0.03)
                P = lambda q, y: B(q[0] + off[0], y, q[1] + off[1])
                glass.quad([P(qa_o, spring + ro * math.sin(a0_)), P(qb_o, spring + ro * math.sin(a1_)),
                            P(qb_i, spring + ri * math.sin(a1_)), P(qa_i, spring + ri * math.sin(a0_))], frame, outv)
    # trim band at the floor line and under the head plate
    trim = s.get('trim')
    if trim:
        self._prism(mb, [add(q, mul(norm(sub(q, O)), 0.04)) for q in ring], y_floor + 0.05, y_floor + 0.2, trim)
        self._prism(mb, [add(q, mul(norm(sub(q, O)), 0.04)) for q in ring], y1 - 0.45, y1 - 0.3, trim)
    mb.build(self.exterior)
    glass.build(self.exterior)


# -- storeysSeen (bldg-263347489) ----------------------
# `storeysSeen` (absent = unchanged): {"count", "groundM", "upperM"} - the
# storeys counted at the street when the roll leaves one out (a garage base
# under three flats rolled as 3 storeys). The LiDAR roof stays the height;
# the stack must fit under it at MIN_STOREY_M or the build stops.
_ss_init_base = Building.__init__


def _ss_init(self, spec, style):
    seen = style.get('storeysSeen') or {}
    n = int(seen.get('count') or 0)
    rolled = spec.get('storeys')
    if n > int(rolled or 0) or (n and n == int(rolled or 0) and spec.get('storeysRoll')):
        g = float(seen.get('groundM') or 0)
        u = float(seen.get('upperM') or 0)
        top = g + (n - 1) * u
        if min(g, u) < MIN_STOREY_M or top > spec['heightM'] - 0.05:
            raise SystemExit(f'storeysSeen {n}x ({g}, {u}) does not fit under the LiDAR height {spec["heightM"]}')
        spec = dict(spec, storeys=n, floors=dict(spec['floors'], groundM=g, upperM=u,
                                                 roofM=round(spec['heightM'] - top, 3)))
        _ss_init_base(self, spec, style)
        self.spec = dict(self.spec, storeysSource=f'street count (storeysSeen {n}); roll {rolled}')
        return
    _ss_init_base(self, spec, style)


Building.__init__ = _ss_init


# -- boxBays + side-street projections (bldg-263347489) -------------
# `bays[].box: true` (absent = unchanged): a flat-fronted rectangular bay
# (the mid-century stucco-box oriel) instead of the canted 3-sided bay: one
# wide punched window on the front face (`windowW`, `lights` sash lights,
# `sillM`, `headM` above the storey line), blank or narrow-windowed returns
# (`sideWindowW`), cladding `material`/`tint` (e.g. ribbed metal), no storey
# plates unless `plates: true`, a flush soffit and a thin cap.
# `streetEdges.<i>.bays` / `.fireEscape` (absent = unchanged): the front-only
# projections drawn on another street face too (a corner building's side street).
_bb_bays_base = Building._bays


def _bb_box_bay(self, spec, e, a, b, mb, glass, s):
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    c, w, p = self.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.5)
    mat = self.tinted(spec.get('material', 'painted-stucco'), spec.get('tint'))
    fmat = self.tinted(spec.get('frame', s['frame']), spec.get('frameTint'))
    k0 = spec.get('fromStorey', 1)
    k1 = min(self.storeys, spec.get('toStorey', self.storeys - 1) + 1)
    y_bottom = self.levels[k0]
    y_top = self.levels[k1] - 0.3 if k1 < self.storeys else self.roof_level

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    ring = [at(c - w / 2, 0), at(c - w / 2, p), at(c + w / 2, p), at(c + w / 2, 0)]
    centre = at(c, p * 0.4)
    self._prism(mb, ring, y_bottom - 0.15, y_bottom + 0.05, mat)
    if spec.get('plates'):
        for k in range(k0 + 1, k1):
            self._prism(mb, ring, self.levels[k] - 0.08, self.levels[k] + 0.08, mat)
    cap = float(spec.get('capM', 0.25))
    self._prism(mb, [ring[0], add(ring[1], mul(nout, 0.04)), add(ring[2], mul(nout, 0.04)), ring[3]], y_top - 0.02, y_top + cap, mat)
    lights = max(1, int(spec.get('lights', 3)))
    for k in range(k0, k1):
        y0 = self.levels[k] + (0.05 if k == k0 else 0.0)
        y1 = self.levels[k + 1] if k + 1 < k1 else y_top
        for fi, (fa, fb) in enumerate(zip(ring[:-1], ring[1:])):
            fl = length(sub(fb, fa))
            fd = norm(sub(fb, fa))
            fn = (-fd[1], fd[0])
            if dot(sub(lerp2(fa, fb, 0.5), centre), fn) < 0:
                fn = (fd[1], -fd[0])
            ia, ib = sub(fa, mul(fn, 0.1)), sub(fb, mul(fn, 0.1))
            front_face = fi == 1
            ww = min(fl - 0.3, float(spec.get('windowW', 2.2))) if front_face else min(fl - 0.12, float(spec.get('sideWindowW', 0) or 0))
            sill = self.levels[k] + float(spec.get('sillM', 0.85))
            head = self.levels[k] + float(spec.get('headM', 2.3))
            ops = [((fl - ww) / 2, (fl + ww) / 2, sill, head)] if ww > 0.3 else []
            wall_grid(mb, fa, fb, ia, ib, y0, y1, ops, mat, 'int-plaster', mat, cap_top=False)
            if not ops:
                continue
            gc = add(lerp2(fa, fb, 0.5), mul(fn, -0.05))
            fyaw = box_yaw(fd)
            glass.box((gc[0], (sill + head) / 2, gc[1]), (ww, head - sill, 0.02), s['glass'], fyaw)
            self._glass_backing(glass, s, 'bay', gc, sill, head, ww, fyaw, mul(fn, -1))
            for yy in (sill + 0.03, head - 0.03):
                glass.box((gc[0], yy, gc[1]), (ww, 0.06, 0.08), fmat, fyaw)
            n_l = lights if front_face else 1
            for i in range(n_l + 1):
                q = add(gc, mul(fd, -ww / 2 + 0.03 + (ww - 0.06) * i / n_l))
                glass.box((q[0], (sill + head) / 2, q[1]), (0.06, head - sill, 0.08), fmat, fyaw)


def _bb_bays(self, e, a, b):
    s = self.style
    specs = s.get('bays') or []
    boxes = [q for q in specs if q.get('box')]
    if not boxes:
        return _bb_bays_base(self, e, a, b)
    rest = [q for q in specs if not q.get('box')]
    if rest:
        self.style = dict(s, bays=rest)
        try:
            _bb_bays_base(self, e, a, b)
        finally:
            self.style = s
    mb = MeshBuilder('bay-windows')
    glass = MeshBuilder('bay-glazing')
    for spec in boxes:
        _bb_box_bay(self, spec, e, a, b, mb, glass, s)
    mb.build(self.exterior)
    glass.build(self.exterior)


Building._bays = _bb_bays
_bb_build_exterior_base = Building.build_exterior


def _bb_build_exterior(self):
    _bb_build_exterior_base(self)
    extra = self.style.get('streetEdges') or {}
    n = len(self.poly)
    for e in self.edges:
        x = extra.get(str(e['index'])) or {}
        if e['role'] == 'front' or not (x.get('bays') or x.get('fireEscape')):
            continue
        i = e['index']
        a, b = self.poly[i], self.poly[(i + 1) % n]
        saved = self.style
        self.style = self.edge_style(e)
        try:
            self._bays(e, a, b)
            self._fire_escape(e, a, b)
        finally:
            self.style = saved


Building.build_exterior = _bb_build_exterior

# `groundGarage` (with "ground": "vacant"; absent = unchanged): the ground
# storey is a residents' garage - painted stall lines and wheel stops behind
# each street garage door run, storage shelving and boxes along the back,
# strip lights. {stallW, stallD, storage: bool}
_bb_vacant_base = Building._vacant


def _bb_vacant(self, rx0, rx1, kz, z1, top):
    g = self.style.get('groundGarage')
    if not g:
        return _bb_vacant_base(self, rx0, rx1, kz, z1, top)
    sw = float(g.get('stallW', 2.7))
    sd = min(float(g.get('stallD', 5.2)), max(2.5, z1 - kz - 1.2))
    paint = self.coarse_for(0)
    n = max(1, int((rx1 - rx0 - 0.4) // sw))
    x_start = (rx0 + rx1) / 2 - n * sw / 2
    for i in range(n + 1):
        x = x_start + i * sw
        paint.box((x, 0.006, z1 - 0.3 - sd / 2), (0.1, 0.012, sd), 'int-ceramic', 0.0)
    for i in range(n):
        xc = x_start + (i + 0.5) * sw
        zs = z1 - 0.3 - sd + 0.35
        paint.box((xc, 0.07, zs), (1.6, 0.14, 0.18), 'int-concrete-floor', 0.0)
        self.physics['solids'].append({'id': f'f-wheel-stop-{i}', 'center': {'x': xc, 'y': 0.07, 'z': zs},
                                       'size': {'x': 1.6, 'y': 0.14, 'z': 0.18}, 'yaw': 0.0})
    if g.get('storage', True) and z1 - sd - kz > 1.0:
        for xc in frange(rx0 + 0.7, rx1 - 0.6, 1.4):
            self.piece('shelving', xc, kz + 0.35, 0.0, 0.0)
        self.piece('cardboard-box', rx0 + 0.9, kz + 1.0, 0.3, 0.0)
        self.piece('crate', rx1 - 0.9, kz + 1.0, 0.8, 0.0)
    for xc in frange(rx0 + sw / 2, rx1 - sw / 2 + 0.01, sw * 2):
        self.hang('ceiling-lamp', xc, (kz + z1) / 2, top, 0.0)
    self.add_light((rx0 + rx1) / 2, top - 0.5, (kz + z1) / 2, 0, intensity=12.0, distance=10.0, priority=2)


Building._vacant = _bb_vacant


# -- mid-century box facade detail (bldg-263347489) --------
# Every key below is absent by default, so buildings that do not name them
# build unchanged.
#   bays[].ribs     {pitchM, widthM, depthM, tint}: vertical ribbed-metal
#                   cladding battens on a box bay's faces, in the spandrels
#                   under and over each storey's window and on blank returns.
#   bays[].soffit   {tint, depthM}: a dark underside plate under the bay's
#                   cantilever so the bay reads as projecting over the base.
#   windowStrip     {from, to, material, tint, ribs: {pitchM, widthM, depthM},
#                   revealTint, louvre: {widthM, heightM, tint}} (front or a
#                   streetEdges face): a vertical ribbed-panel strip from the
#                   first floor line to the parapet between `from` and `to`
#                   (fractions from the viewer's left); panels fill the gaps
#                   between the windows inside it and the spandrels above and
#                   below them, with a louvred vent centred in each spandrel
#                   and dark reveals at both sides; `louvre.centre: true` puts
#                   one pair of small vents on the strip's centre line in each
#                   spandrel between storeys instead.
#   windowSills     false: punched windows get no projecting sill (and no
#                   hood); a flush aluminium frame only.
#   garagePanel     {doorM, cols, rows, tint, panelTint}: `garage` layout
#                   items draw as runs of flat-panelled swing/sectional leaves
#                   (raised panels, narrow meeting joints, no shutter guides)
#                   instead of a roll-up shutter; one span holds
#                   round(width / doorM) leaves butting together, or exactly
#                   `leaves` per span when that key is given.
#   entryGate.closed true: the grille leaf hangs shut in the opening plane
#                   (bars flush with the wall) instead of standing open.
#   entryGate.mailboxes true: a steel mailbox bank on the lobby wall just
#                   inside the gate, plus a lobby floor/back tone.
#   entryGate       {tint, number, numberTint, numberM}: front storefront-door
#                   items draw as a black steel security gate (standing open
#                   inward so the doorway stays walkable) with a barred
#                   transom and the street number mounted over the head in
#                   plain seven-segment block digits.
#   fireEscape.tint / .heavy: tinted steel; heavy adds channel fascias on each
#                   landing, hangers and a drop ladder under the lowest one.
#   doorLeaves      {material}: every interior partition doorway gets
#                   a leaf swung fully open, flat to the wall beside it so
#                   the passage stays clear) and a head casing.
#   groundGarage.columns {spacingM, sizeM}: a row of concrete columns down
#                   the garage between the stalls and the storage wall.

def _mc_face_ribs(mb, fa, fb, fn, y0, y1, s0, s1, rib, mat):
    """Vertical battens along the face fa->fb (outward normal fn) between
    along-face offsets s0..s1 and heights y0..y1."""
    if y1 - y0 < 0.08 or s1 - s0 < 0.08:
        return
    fd = norm(sub(fb, fa))
    yaw = box_yaw(fd)
    pitch = float(rib.get('pitchM', 0.3))
    rw = float(rib.get('widthM', 0.05))
    rd = float(rib.get('depthM', 0.03))
    n = max(1, int((s1 - s0) // pitch))
    off = (s1 - s0 - n * pitch) / 2 + pitch / 2
    for i in range(n):
        sv = s0 + off + i * pitch
        if sv > s1 - rw / 2:
            break
        q = add(add(fa, mul(fd, sv)), mul(fn, rd / 2 + 0.005))
        mb.box((q[0], (y0 + y1) / 2, q[1]), (rw, y1 - y0, rd), mat, yaw)


_mc_box_bay_base = _bb_box_bay


def _mc_box_bay(self, spec, e, a, b, mb, glass, s):
    _mc_box_bay_base(self, spec, e, a, b, mb, glass, s)
    rib = spec.get('ribs')
    soffit = spec.get('soffit')
    if not rib and not soffit:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    c, w, p = self.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.5)
    k0 = spec.get('fromStorey', 1)
    k1 = min(self.storeys, spec.get('toStorey', self.storeys - 1) + 1)
    y_bottom = self.levels[k0]
    y_top = self.levels[k1] - 0.3 if k1 < self.storeys else self.roof_level

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    ring = [at(c - w / 2, 0), at(c - w / 2, p), at(c + w / 2, p), at(c + w / 2, 0)]
    centre = at(c, p * 0.4)
    if soffit:
        sm = self.tinted('painted-stucco', soffit.get('tint', '#3b3d3f'))
        dz = float(soffit.get('depthM', 0.03))
        self._prism(mb, ring, y_bottom - 0.15 - dz, y_bottom - 0.15, sm)
    if not rib:
        return
    rmat = self.tinted(spec.get('material', 'corrugated-metal'), rib.get('tint', spec.get('tint')))
    for k in range(k0, k1):
        y0 = self.levels[k] + (0.05 if k == k0 else 0.0)
        y1 = self.levels[k + 1] if k + 1 < k1 else y_top
        for fi, (fa, fb) in enumerate(zip(ring[:-1], ring[1:])):
            fl = length(sub(fb, fa))
            fd = norm(sub(fb, fa))
            fn = (-fd[1], fd[0])
            if dot(sub(lerp2(fa, fb, 0.5), centre), fn) < 0:
                fn = (fd[1], -fd[0])
            front_face = fi == 1
            ww = min(fl - 0.3, float(spec.get('windowW', 2.2))) if front_face else min(fl - 0.12, float(spec.get('sideWindowW', 0) or 0))
            sill = self.levels[k] + float(spec.get('sillM', 0.85))
            head = self.levels[k] + float(spec.get('headM', 2.3))
            if ww > 0.3:
                o0, o1 = (fl - ww) / 2, (fl + ww) / 2
                _mc_face_ribs(mb, fa, fb, fn, y0, sill - 0.04, 0.02, fl - 0.02, rib, rmat)
                _mc_face_ribs(mb, fa, fb, fn, head + 0.04, y1, 0.02, fl - 0.02, rib, rmat)
                _mc_face_ribs(mb, fa, fb, fn, sill, head, 0.02, o0 - 0.04, rib, rmat)
                _mc_face_ribs(mb, fa, fb, fn, sill, head, o1 + 0.04, fl - 0.02, rib, rmat)
            else:
                _mc_face_ribs(mb, fa, fb, fn, y0, y1, 0.02, fl - 0.02, rib, rmat)


_bb_box_bay = _mc_box_bay


def _mc_window_strip(self, e, a, b, es):
    ws = es.get('windowStrip')
    if not ws or self.storeys < 2:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    s0, s1 = sorted((self.f2s(ws['from'], L, e), self.f2s(ws['to'], L, e)))
    pm = self.tinted(ws.get('material', 'corrugated-metal'), ws.get('tint', '#b8bcbd'))
    rm = self.tinted('painted-stucco', ws.get('revealTint', '#55595c'))
    rib = ws.get('ribs') or {'pitchM': 0.2, 'widthM': 0.04, 'depthM': 0.025}
    lv = ws.get('louvre') or {}
    lm = self.tinted('painted-steel', lv.get('tint', '#8c9194'))
    mb = MeshBuilder('window-strip')
    ww = float(es.get('windowW', 1.6))
    wins = []
    for f in es.get('upperWindows') or []:
        cs = self.f2s(f, L, e)
        if s0 - 0.01 <= cs - ww / 2 and cs + ww / 2 <= s1 + 0.01:
            wins.append((cs - ww / 2, cs + ww / 2))
    wins.sort()

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    def panel(p0, p1, y0, y1):
        if p1 - p0 < 0.05 or y1 - y0 < 0.05:
            return
        q = at((p0 + p1) / 2, 0.012)
        mb.box((q[0], (y0 + y1) / 2, q[1]), (p1 - p0, y1 - y0, 0.024), pm, yaw)
        _mc_face_ribs(mb, at(p0, 0.024), at(p1, 0.024), nout, y0, y1, 0.0, p1 - p0, rib, pm)

    y_lo, y_hi = self.levels[1], self.roof_level - 0.02
    # gaps between (and beside) the windows: full height
    cursor = s0
    for w0, w1 in wins + [(s1, s1)]:
        panel(cursor, w0 - 0.02, y_lo, y_hi)
        cursor = w1 + 0.02
    # spandrels over and under each window, a vent in each
    sill_m, head_m = float(es.get('upperSillM', es.get('windowSillM', 0.85))), float(es.get('windowHeadM', 2.25))
    lw_, lh_ = float(lv.get('widthM', 0.45)), float(lv.get('heightM', 0.3))
    for k in range(1, self.storeys):
        base = self.levels[k]
        zones = [(base + head_m + 0.02, (self.levels[k + 1] + sill_m - 0.02) if k + 1 < self.storeys else y_hi)]
        if k == 1:
            zones.append((y_lo, base + sill_m - 0.02))
        centre = bool(lv.get('centre'))
        if centre and wins and k + 1 < self.storeys:
            vy = (zones[0][0] + zones[0][1]) / 2
            for sv in ((s0 + s1) / 2 - lw_ * 0.3, (s0 + s1) / 2 + lw_ * 0.3):
                q = at(sv, 0.05)
                mb.box((q[0], vy, q[1]), (lw_ * 0.5, lh_, 0.03), lm, yaw)
                for i in range(3):
                    q2 = at(sv, 0.075)
                    mb.box((q2[0], vy - lh_ / 2 + lh_ * (i + 0.5) / 3, q2[1]), (lw_ * 0.5 - 0.03, 0.02, 0.03), lm, yaw)
        for w0, w1 in wins:
            for z0, z1 in zones:
                panel(w0 - 0.02, w1 + 0.02, z0, z1)
                if not centre and w1 - w0 > 0.5 and z1 - z0 > lh_ + 0.2:
                    vy = (z0 + z1) / 2
                    q = at((w0 + w1) / 2, 0.05)
                    mb.box((q[0], vy, q[1]), (lw_, lh_, 0.03), lm, yaw)
                    for i in range(4):
                        q2 = at((w0 + w1) / 2, 0.075)
                        mb.box((q2[0], vy - lh_ / 2 + lh_ * (i + 0.5) / 4, q2[1]), (lw_ - 0.04, 0.025, 0.03), lm, yaw)
    # dark reveals down both sides of the strip (it reads set back)
    for sv in (s0 - 0.03, s1 + 0.03):
        q = at(sv, 0.03)
        mb.box((q[0], (y_lo + y_hi) / 2, q[1]), (0.06, y_hi - y_lo, 0.06), rm, yaw)
    mb.build(self.exterior)


class _NoBoxes:
    def box(self, *args, **kw):
        return None


_mc_dress_base = Building._dress_openings


def _mc_dress_openings(self, e, a, b, windows, trim, glass, frames, store, es=None):
    s = es or self.style
    gate = s.get('entryGate') if e['role'] == 'front' else None
    rest = [w for w in windows if not (gate and w[0] == 'storefront-door')]
    doors = [w for w in windows if gate and w[0] == 'storefront-door']
    t = _NoBoxes() if s.get('windowSills') is False else trim
    _mc_dress_base(self, e, a, b, rest, t, glass, frames, store, es)
    for _, span in doors:
        _mc_entry_gate(self, e, a, b, span, gate, s, store, glass)
    own = (self.style.get('streetEdges') or {}).get(str(e['index'])) or {}
    if (e['role'] == 'front' and self.style.get('windowStrip')) or own.get('windowStrip'):
        _mc_window_strip(self, e, a, b, dict(s, windowStrip=own.get('windowStrip') or self.style.get('windowStrip')))


def _mc_digit_boxes(ch):
    segs = {'0': 'abcdef', '1': 'bc', '2': 'abged', '3': 'abgcd', '4': 'fgbc', '5': 'afgcd',
            '6': 'afgedc', '7': 'abc', '8': 'abcdefg', '9': 'abcdfg'}
    return segs.get(ch, '')


def _mc_entry_gate(self, e, a, b, span, gate, s, store, glass):
    s0, s1, y0, y1, d0, d1 = span[:6]
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    gm = self.tinted('painted-steel', gate.get('tint', '#16171a'))
    w = s1 - s0
    cs = (s0 + s1) / 2
    dh = min(2.45, y1 - 0.1)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    # dim lobby tone behind the barred transom only (the doorway stays open)
    if y1 - dh > 0.15:
        q = at(cs, -WALL_T + 0.02)
        store.box((q[0], (dh + y1) / 2, q[1]), (w, y1 - dh, 0.02), self.tinted('painted-stucco', '#2c2d2f'), yaw)
    # steel frame: jambs + head at the outer face
    for sv in (s0 + 0.04, s1 - 0.04):
        q = at(sv, -0.06)
        store.box((q[0], y1 / 2, q[1]), (0.08, y1, 0.1), gm, yaw)
    q = at(cs, -0.06)
    store.box((q[0], dh + 0.04, q[1]), (w, 0.08, 0.1), gm, yaw)
    store.box((q[0], y1 - 0.04, q[1]), (w, 0.08, 0.1), gm, yaw)
    # barred transom over the gate
    nb = max(2, int(w / 0.12))
    for i in range(1, nb):
        q = at(s0 + w * i / nb, -0.06)
        store.box((q[0], (dh + y1) / 2, q[1]), (0.025, y1 - dh - 0.08, 0.025), gm, yaw)
    leaf_w = d1 - d0 - 0.06
    if gate.get('mailboxes'):
        # lobby: dark floor tone, pale back wall, a steel mailbox bank on the side wall
        q = at(cs, -1.35)
        store.box((q[0], 0.012, q[1]), (w + 0.4, 0.02, 2.5), self.tinted('painted-white', '#5a5652'), yaw)
        mb_m = self.tinted('painted-white', '#8d9194')
        md_m = self.tinted('painted-white', '#6f7376')
        side = s1 - 0.05 if (s1 - d1) < (d0 - s0) else s0 + 0.05
        sgn = -1 if side > cs else 1
        for k in range(4):
            q = at(side + sgn * 0.09, -0.5 - k * 0.3)
            store.box((q[0], 1.25, q[1]), (0.16, 0.9, 0.28), mb_m, yaw)
            for r in range(4):
                q2 = at(side + sgn * 0.175, -0.5 - k * 0.3)
                store.box((q2[0], 0.9 + r * 0.22, q2[1]), (0.012, 0.16, 0.24), md_m, yaw)
    if gate.get('closed'):
        # grille leaf hung shut in the opening plane, bars flush with the wall
        g0, g1 = d0 + 0.03, d1 - 0.03
        q = at((g0 + g1) / 2, -0.08)
        for yy in (0.1, dh * 0.45, dh - 0.1):
            store.box((q[0], yy, q[1]), (g1 - g0, 0.05, 0.04), gm, yaw)
        for sv in (g0, g1):
            q2 = at(sv, -0.08)
            store.box((q2[0], dh / 2, q2[1]), (0.05, dh, 0.05), gm, yaw)
        nbar = max(4, int((g1 - g0) / 0.1))
        for i in range(1, nbar):
            q2 = at(g0 + (g1 - g0) * i / nbar, -0.08)
            store.box((q2[0], dh / 2, q2[1]), (0.018, dh - 0.1, 0.018), gm, yaw)
        # lock box + pull at the latch side
        q2 = at(g1 - 0.12, -0.03)
        store.box((q2[0], 1.05, q2[1]), (0.12, 0.2, 0.06), gm, yaw)
    else:
        # gate leaf standing open inward against the jamb (the doorway stays clear)
        hinge = at(d0 + 0.05, -0.12)
        inward = mul(nout, -1)
        for yy in (0.08, dh / 2, dh - 0.08):
            c = add(hinge, mul(inward, leaf_w / 2))
            store.box((c[0], yy, c[1]), (0.04, 0.05, leaf_w), gm, yaw)
        nbar = max(3, int(leaf_w / 0.11))
        for i in range(nbar + 1):
            c = add(hinge, mul(inward, 0.02 + (leaf_w - 0.04) * i / nbar))
            store.box((c[0], dh / 2, c[1]), (0.02, dh - 0.04, 0.02), gm, yaw)
    # street number over the head: seven-segment block digits on a flat plate
    num = str(gate.get('number') or '')
    if not num:
        return
    nm = self.tinted('painted-steel', gate.get('numberTint', '#1b1c1e'))
    hgt = float(gate.get('numberM', 0.2))
    cw, t = hgt * 0.55, hgt * 0.12
    y_mid = y1 + 0.18 + hgt / 2
    total = len(num) * cw + (len(num) - 1) * hgt * 0.25
    x = cs - total / 2
    for ch in num:
        cx = x + cw / 2
        pos = {'a': (0, hgt / 2, cw, t), 'g': (0, 0, cw, t), 'd': (0, -hgt / 2, cw, t),
               'f': (-cw / 2, hgt / 4, t, hgt / 2), 'b': (cw / 2, hgt / 4, t, hgt / 2),
               'e': (-cw / 2, -hgt / 4, t, hgt / 2), 'c': (cw / 2, -hgt / 4, t, hgt / 2)}
        for sg in _mc_digit_boxes(ch):
            ox, oy, sw, sh = pos[sg]
            if ch == '1' and gate.get('centreOne'):
                ox = 0.0  # a '1' is one centred stroke, not the right half of an 8
            q = at(cx + ox, 0.015)
            store.box((q[0], y_mid + oy, q[1]), (sw, sh, 0.02), nm, yaw)
        x += cw + hgt * 0.25


Building._dress_openings = _mc_dress_openings

_mc_closed_infill_base = Building._closed_infill


def _mc_closed_infill(self, kind, span, at, s, store, glass, yaw):
    gp = s.get('garagePanel')
    if kind != 'garage' or not gp:
        return _mc_closed_infill_base(self, kind, span, at, s, store, glass, yaw)
    s0, s1, y0, y1, mat, tint = span[:6]
    w, h = s1 - s0, y1 - y0
    m = self.tinted(mat or 'painted-steel', tint or gp.get('tint', '#2e3033'))
    pmat = self.tinted(mat or 'painted-steel', gp.get('panelTint', tint or gp.get('tint', '#2e3033')))
    jm = self.tinted('painted-steel', '#141516')
    n = max(1, int(round(w / float(gp.get('doorM', 2.4)))))
    if gp.get('leaves'):
        n = max(1, int(gp['leaves']))
    cols, rows = int(gp.get('cols', 2)), int(gp.get('rows', 4))
    back = -0.14
    lw = w / n
    for i in range(n):
        l0 = s0 + i * lw
        lc = l0 + lw / 2
        p = at(lc, back)
        store.box((p[0], (y0 + y1) / 2, p[1]), (lw - 0.02, h, 0.04), m, yaw)
        mx, my = 0.12, 0.12
        pw = (lw - 2 * mx - (cols - 1) * 0.08) / cols
        ph = (h - 2 * my - (rows - 1) * 0.08) / rows
        for ci in range(cols):
            for ri in range(rows):
                sv = l0 + mx + pw / 2 + ci * (pw + 0.08)
                yy = y0 + my + ph / 2 + ri * (ph + 0.08)
                q = at(sv, back + 0.03)
                store.box((q[0], yy, q[1]), (pw, ph, 0.025), pmat, yaw)
        if i:
            q = at(l0, back + 0.01)
            store.box((q[0], (y0 + y1) / 2, q[1]), (0.025, h, 0.05), jm, yaw)
    # steel head angle right under the lintel
    q = at((s0 + s1) / 2, -0.03)
    store.box((q[0], y1 - 0.03, q[1]), (w, 0.06, 0.06), jm, yaw)


Building._closed_infill = _mc_closed_infill

_mc_fire_escape_base = Building._fire_escape


def _mc_fire_escape(self, e, a, b):
    spec = self.style.get('fireEscape')
    if not spec or self.storeys < 2 or not (spec.get('tint') or spec.get('heavy')):
        return _mc_fire_escape_base(self, e, a, b)
    mat = self.tinted(spec.get('material', 'painted-steel'), spec.get('tint'))
    saved = self.style
    self.style = dict(saved, fireEscape=dict(spec, material=mat))
    try:
        _mc_fire_escape_base(self, e, a, b)
    finally:
        self.style = saved
    if not spec.get('heavy'):
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    c, w, depth = self.f2s(spec['at'], L, e), spec.get('widthM', 3.4), spec.get('depthM', 0.9)
    mb = MeshBuilder('fire-escape-heavy')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    k_last = min(self.storeys - 1, spec.get('toStorey', self.storeys - 1))
    for k in range(1, k_last + 1):
        y = self.levels[k] - 0.02
        q = at(c, depth + 0.02)
        mb.box((q[0], y - 0.06, q[1]), (w + 0.06, 0.18, 0.05), mat, yaw)
        for side in (-1, 1):
            q2 = at(c + side * (w / 2 + 0.02), depth / 2)
            mb.box((q2[0], y - 0.06, q2[1]), (0.05, 0.18, depth), mat, yaw)
            # diagonal-free hanger rods back to the wall above
            q3 = at(c + side * (w / 2 - 0.1), depth - 0.05)
            mb.box((q3[0], y + 0.9, q3[1]), (0.03, 1.8, 0.03), mat, yaw)
    # drop ladder hung under the lowest landing, stopping ~2.5 m above the pavement
    y_hi = self.levels[1] - 0.1
    y_lo = max(2.3, y_hi - 1.6)
    lx = c + w / 2 - 0.45
    for off in (-0.22, 0.22):
        q = at(lx + off, depth - 0.1)
        mb.box((q[0], (y_lo + y_hi) / 2, q[1]), (0.04, y_hi - y_lo, 0.04), mat, yaw)
    r = y_lo + 0.1
    while r < y_hi:
        q = at(lx, depth - 0.1)
        mb.box((q[0], r, q[1]), (0.44, 0.025, 0.025), mat, yaw)
        r += 0.3
    mb.build(self.exterior)


Building._fire_escape = _mc_fire_escape

_MC_DOOR_LEAVES = {}
_mc_partition_base = partition


def partition(mb, a, b, base, top, doors, mat='int-plaster', t=PART_T):
    solids = _mc_partition_base(mb, a, b, base, top, doors, mat, t)
    dl = _MC_DOOR_LEAVES.get('spec')
    if not dl or not doors:
        return solids
    L = length(sub(b, a))
    if L < 0.2:
        return solids
    d = norm(sub(b, a))
    perp = (-d[1], d[0])
    yaw = box_yaw(d)
    lm = dl.get('material', 'int-door')
    dh = min(top - 0.05, base + DOOR_H) - base
    for sv in doors:
        # leaf swung right round, flat against the wall face beside the
        # opening, so neither the doorway nor the corridor is obstructed
        # the leaf folds back onto whichever side of the opening has wall
        # enough for it (never past the partition's own ends)
        lw = DOOR_W - 0.1
        if sv - DOOR_W / 2 - 0.02 - lw >= 0.1:
            lc = sv - DOOR_W / 2 - 0.02 - lw / 2
        elif sv + DOOR_W / 2 + 0.02 + lw <= L - 0.1:
            lc = sv + DOOR_W / 2 + 0.02 + lw / 2
        else:
            lc = None
        if lc is not None:
            c = add(add(a, mul(d, lc)), mul(perp, t / 2 + 0.03))
            mb.box((c[0], base + dh / 2, c[1]), (lw, dh - 0.02, 0.04), lm, yaw)
        q = add(a, mul(d, sv))
        mb.box((q[0], base + dh + 0.04, q[1]), (DOOR_W + 0.14, 0.08, t + 0.04), 'int-wood-light', yaw)
        for side in (-1, 1):
            q2 = add(a, mul(d, sv + side * (DOOR_W / 2 + 0.035)))
            mb.box((q2[0], base + dh / 2, q2[1]), (0.07, dh, t + 0.04), 'int-wood-light', yaw)
    return solids


_mc_init_base = Building.__init__


def _mc_init(self, spec, style):
    _MC_DOOR_LEAVES['spec'] = style.get('doorLeaves')
    _mc_init_base(self, spec, style)


Building.__init__ = _mc_init

_mc_vacant_base = Building._vacant


def _mc_vacant(self, rx0, rx1, kz, z1, top):
    _mc_vacant_base(self, rx0, rx1, kz, z1, top)
    g = self.style.get('groundGarage') or {}
    col = g.get('columns')
    if not col:
        return
    sz = float(col.get('sizeM', 0.35))
    sd = min(float(g.get('stallD', 5.2)), max(2.5, z1 - kz - 1.2))
    zc = z1 - 0.3 - sd - 0.35
    if zc - sz / 2 < kz + 0.4:
        return
    mb = self.coarse_for(0)
    for xc in frange(rx0 + float(col.get('spacingM', 5.4)) / 2, rx1 - 0.3, float(col.get('spacingM', 5.4))):
        mb.box((xc, top / 2, zc), (sz, top, sz), 'int-plaster', 0.0)
        self.physics['solids'].append({'id': f'f-garage-col-{len(self.physics["solids"])}',
                                       'center': {'x': xc, 'y': top / 2, 'z': zc},
                                       'size': {'x': sz, 'y': top, 'z': sz}, 'yaw': 0.0})


Building._vacant = _mc_vacant

# -- zig-zag steel fire escape (bldg-263347489) -------
# `fireEscape.zigzag: true` (front or a streetEdges face; absent = unchanged):
# one continuous steel escape instead of loose treads - a solid landing deck
# with a deep channel fascia on each storey, a railing (top and mid rail,
# posts at 0.3 m), and between every pair of landings one straight flight
# whose two channel stringers run unbroken from deck to deck, treads spanning
# between them and a sloped handrail on the open side. Flights alternate
# direction (true zig-zag). No wall brackets, hanger rods or drop ladder.
# Keys: at, widthM, depthM, toStorey, material, tint, flightW (0.6).


def _zz_beam(mb, a, d, nout, s0, y0, s1, y1, off, half_n, h, mat):
    """A straight beam from (s0, y0) to (s1, y1) along d at `off` out from the
    wall line, half_n thick across the face and h deep (vertical)."""
    def P(sv, yy, o):
        q = add(add(a, mul(d, sv)), mul(nout, o))
        return B(q[0], yy, q[1])
    lo = [P(s0, y0 - h, off - half_n), P(s1, y1 - h, off - half_n), P(s1, y1 - h, off + half_n), P(s0, y0 - h, off + half_n)]
    hi = [P(s0, y0, off - half_n), P(s1, y1, off - half_n), P(s1, y1, off + half_n), P(s0, y0, off + half_n)]
    up = B(0, 1, 0)
    mb.quad(hi, mat, up)
    mb.quad(lo, mat, B(0, -1, 0))
    n3 = B(nout[0], 0, nout[1])
    mb.quad([lo[2], lo[3], hi[3], hi[2]], mat, n3)
    mb.quad([lo[0], lo[1], hi[1], hi[0]], mat, B(-nout[0], 0, -nout[1]))
    dv = B(d[0], 0, d[1])
    mb.quad([lo[1], lo[2], hi[2], hi[1]], mat, dv)
    mb.quad([lo[0], lo[3], hi[3], hi[0]], mat, B(-d[0], 0, -d[1]))


_zz_fire_escape_base = Building._fire_escape


def _zz_fire_escape(self, e, a, b):
    spec = self.style.get('fireEscape')
    if not spec or not spec.get('zigzag') or self.storeys < 2:
        return _zz_fire_escape_base(self, e, a, b)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    c, w, depth = self.f2s(spec['at'], L, e), spec.get('widthM', 3.4), spec.get('depthM', 0.9)
    mat = self.tinted(spec.get('material', 'painted-steel'), spec.get('tint'))
    fw = float(spec.get('flightW', 0.6))
    mb = MeshBuilder('fire-escape')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    k_last = min(self.storeys - 1, spec.get('toStorey', self.storeys - 1))
    for k in range(1, k_last + 1):
        y = self.levels[k] - 0.02
        p = at(c, depth / 2 + 0.02)
        mb.box((p[0], y - 0.03, p[1]), (w, 0.06, depth), mat, yaw)
        q = at(c, depth + 0.03)
        mb.box((q[0], y - 0.08, q[1]), (w + 0.06, 0.2, 0.05), mat, yaw)
        for side in (-1, 1):
            q2 = at(c + side * (w / 2 + 0.02), depth / 2 + 0.02)
            mb.box((q2[0], y - 0.08, q2[1]), (0.05, 0.2, depth), mat, yaw)
        for yy in (y + 1.0, y + 0.5):
            q = at(c, depth)
            mb.box((q[0], yy, q[1]), (w, 0.04, 0.04), mat, yaw)
            for side in (-1, 1):
                q2 = at(c + side * w / 2, depth / 2 + 0.02)
                mb.box((q2[0], yy, q2[1]), (0.04, 0.04, depth), mat, yaw)
        n = max(2, int(w / 0.3))
        for i in range(n + 1):
            q = at(c - w / 2 + i * w / n, depth)
            mb.box((q[0], y + 0.5, q[1]), (0.025, 1.0, 0.025), mat, yaw)
        if k + 1 > k_last:
            continue
        # flight from this deck to the next, alternating direction
        rise = self.levels[k + 1] - self.levels[k]
        run = min(w - 0.5, rise / 0.75)
        sgn = 1 if k % 2 else -1
        s_lo = c - sgn * run / 2
        s_hi = c + sgn * run / 2
        off = depth * 0.5
        for so in (off - fw / 2, off + fw / 2):
            _zz_beam(mb, a, d, nout, s_lo, y, s_hi, y + rise, so, 0.025, 0.18, mat)
        steps = max(6, int(round(rise / 0.2)))
        for i in range(1, steps):
            t = i / steps
            sv = s_lo + (s_hi - s_lo) * t
            q = at(sv, off)
            mb.box((q[0], y + rise * t - 0.06, q[1]), (abs(s_hi - s_lo) / steps + 0.02, 0.03, fw), mat, yaw)
        _zz_beam(mb, a, d, nout, s_lo, y + 0.9, s_hi, y + rise + 0.9, off + fw / 2 + 0.02, 0.02, 0.04, mat)
    mb.build(self.exterior)


Building._fire_escape = _zz_fire_escape


# ---------------------------------------------------------------- bldg-263347489 block 3 (flats + entry lamp)
# New style keys only:
#   flatRooms true      : upper 'apartment' rooms alternate bedroom / living-kitchen
#                         with the phase shifted per storey, so no two storeys repeat
#                         (1960s walk-up flat: sofa, low table, kitchen run, small
#                         dining table, shelving) instead of every room a hotel room.
#   entryGate.lamp true : a wall lantern over the front gate head.
_fr_room_base = Building._room


def _fr_room(self, r, base, top, k, door_wall_x, window):
    if not self.style.get('flatRooms'):
        return _fr_room_base(self, r, base, top, k, door_wall_x, window)
    rx0, rx1, rz0, rz1 = r
    idx = int(round((rz0 + rx0) * 7.0)) + k + (1 if (self.style.get('flatRoomsShift') and k >= 3) else 0)
    if idx % 2 == 0:
        return _fr_room_base(self, r, base, top, k, door_wall_x, window)
    cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
    w = rx1 - rx0
    toward_door = 1 if door_wall_x > cx else -1
    far_x = rx1 - 0.05 if toward_door > 0 else rx0 + 0.05
    flip = 1 if k % 2 else -1
    # kitchen run on one partition: counter, range, fridge
    kz = rz0 + 0.36 if flip > 0 else rz1 - 0.36
    kyaw = 0.0 if flip > 0 else math.pi
    run0 = cx - min(1.5, w * 0.3)
    self.piece('kitchen-counter', run0, kz, kyaw, base)
    self.piece('range', run0 + 1.3, kz, kyaw, base)
    self.piece('fridge', run0 + 2.35 if run0 + 2.8 < rx1 else run0 - 1.1, kz, kyaw, base)
    # small dining table with two chairs mid room
    dz = cz - flip * 0.2
    dx = cx + toward_door * min(0.8, w * 0.15)
    self.piece('cafe-table', dx, dz, 0.0, base)
    self.piece('cafe-chair', dx - 0.6, dz, math.pi / 2, base, solid=False, coarse=False)
    self.piece('cafe-chair', dx + 0.6, dz, -math.pi / 2, base, solid=False, coarse=False)
    # living corner at the other end: sofa facing the kitchen side, low table, rug
    sz = rz1 - 0.55 if flip > 0 else rz0 + 0.55
    syaw = math.pi if flip > 0 else 0.0
    sx = cx - toward_door * min(0.7, w * 0.12)
    self.piece('sofa', sx, sz, syaw, base, coarse=(('int-fabric-blue', 'int-fabric-green', 'int-fabric-red', 'int-leather')[k % 4] if self.style.get('flatRoomsShift') else 'int-fabric-green'))
    self.piece('coffee-table', sx, sz - flip * 0.95, 0.0, base)
    self.rug(sx, sz - flip * 0.9, 2.0, 1.4, 0.0, base, k)
    self.piece('shelving', far_x - (0.25 if toward_door > 0 else -0.25), cz, math.pi / 2 * (1 if toward_door > 0 else -1), base)
    if k % 3 == 0:
        self.piece('plant-medium', far_x - toward_door * 0.4, sz, 0.0, base, solid=False, coarse=False)
    self.wall_frame(sx, sz + flip * 0.5, (0, -flip), base, k, height=1.6)
    self.hang('ceiling-lamp', cx, cz, top, base)
    for slot in self.window_slots:
        mx, mz = lerp2(slot['a'], slot['b'], 0.5)
        if not (base < slot['sill'] < top + 0.3):
            continue
        if rx0 - 0.6 < mx < rx1 + 0.6 and rz0 - 0.6 < mz < rz1 + 0.6:
            self.curtains(slot, base, k)


Building._room = _fr_room

_fr_entry_gate_base = _mc_entry_gate


def _fr_entry_gate(self, e, a, b, span, gate, s, store, glass):
    _fr_entry_gate_base(self, e, a, b, span, gate, s, store, glass)
    if not gate.get('lamp'):
        return
    s0, s1, y0, y1 = span[:4]
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    cs = (s0 + s1) / 2
    hy = min(y1 + 0.45, self.levels[1] - 0.25) if len(self.levels) > 1 else y1 + 0.45
    house = self.tinted('painted-steel', '#16171a')
    lens = self.tinted('painted-white', '#f4eedc')
    q = add(add(a, mul(d, cs)), mul(nout, 0.08))
    store.box((q[0], hy + 0.13, q[1]), (0.2, 0.05, 0.16), house, yaw)
    store.box((q[0], hy, q[1]), (0.16, 0.22, 0.12), lens, yaw)
    store.box((q[0], hy - 0.13, q[1]), (0.18, 0.04, 0.14), house, yaw)


_mc_entry_gate = _fr_entry_gate




# ---------------------------------------------------------------- bldg-263347489 block 4 (grated escape + gate backdrop)
# New style keys only; absent = unchanged:
#   fireEscape.grate true   (with zigzag): landings are open steel grating - two
#                           thin edge angles and cross bars at 0.1 m instead of a
#                           solid 60 mm deck, so the underside reads light, not as
#                           a dark slab; plus a straight wall ladder from the top
#                           landing to the parapet (stiles stop under the roof top).
#   entryGate.backdrop hex  : an opaque lobby wall of that tone `backdropM`
#                           (0.9) behind the gate, so the bars read against a
#                           dark lobby instead of the stair behind it.
_g4_fire_escape_base = Building._fire_escape


def _g4_fire_escape(self, e, a, b):
    spec = self.style.get('fireEscape')
    if not spec or not spec.get('zigzag') or not spec.get('grate') or self.storeys < 2:
        return _g4_fire_escape_base(self, e, a, b)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    c, w, depth = self.f2s(spec['at'], L, e), spec.get('widthM', 3.4), spec.get('depthM', 0.9)
    mat = self.tinted(spec.get('material', 'painted-steel'), spec.get('tint'))
    fw = float(spec.get('flightW', 0.6))
    mb = MeshBuilder('fire-escape')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    k_last = min(self.storeys - 1, spec.get('toStorey', self.storeys - 1))
    for k in range(1, k_last + 1):
        y = self.levels[k] - 0.02
        # grating: edge angles along the wall and the outer edge, bars across
        for off in (0.05, depth - 0.01):
            q = at(c, off)
            mb.box((q[0], y - 0.04, q[1]), (w, 0.08, 0.04), mat, yaw)
        for side in (-1, 1):
            q2 = at(c + side * (w / 2 - 0.02), depth / 2 + 0.02)
            mb.box((q2[0], y - 0.04, q2[1]), (0.04, 0.08, depth), mat, yaw)
        nb = max(4, int(w / 0.1))
        for i in range(1, nb):
            q = at(c - w / 2 + i * w / nb, depth / 2 + 0.02)
            mb.box((q[0], y - 0.012, q[1]), (0.012, 0.025, depth - 0.06), mat, yaw)
        for yy in (y + 1.0, y + 0.5):
            q = at(c, depth)
            mb.box((q[0], yy, q[1]), (w, 0.035, 0.035), mat, yaw)
            for side in (-1, 1):
                q2 = at(c + side * w / 2, depth / 2 + 0.02)
                mb.box((q2[0], yy, q2[1]), (0.035, 0.035, depth), mat, yaw)
        n = max(2, int(w / 0.3))
        for i in range(n + 1):
            q = at(c - w / 2 + i * w / n, depth)
            mb.box((q[0], y + 0.5, q[1]), (0.02, 1.0, 0.02), mat, yaw)
        if k + 1 > k_last:
            # wall ladder from the top landing to the parapet, gooseneck over it
            top = self.height
            lx = c + (w / 2 - 0.45) * (1 if k % 2 else -1)
            h = top - 0.03 - y  # stiles stop under the LiDAR top (no gooseneck above it)
            for side in (-0.22, 0.22):
                q = at(lx + side, 0.18)
                mb.box((q[0], y + h / 2, q[1]), (0.03, h, 0.03), mat, yaw)
            for r in frange(y + 0.3, top - 0.05, 0.3):
                q = at(lx, 0.18)
                mb.box((q[0], r, q[1]), (0.44, 0.022, 0.022), mat, yaw)
            continue
        rise = self.levels[k + 1] - self.levels[k]
        run = min(w - 0.5, float(spec.get('flightRunM') or rise / 0.75))
        sgn = 1 if k % 2 else -1
        s_lo = c - sgn * run / 2
        s_hi = c + sgn * run / 2
        off = depth * 0.5
        for so in (off - fw / 2, off + fw / 2):
            _zz_beam(mb, a, d, nout, s_lo, y, s_hi, y + rise, so, 0.02, 0.14, mat)
        steps = max(6, int(round(rise / 0.2)))
        for i in range(1, steps):
            t = i / steps
            sv = s_lo + (s_hi - s_lo) * t
            q = at(sv, off)
            mb.box((q[0], y + rise * t - 0.06, q[1]), (abs(s_hi - s_lo) / steps + 0.02, 0.025, fw), mat, yaw)
        _zz_beam(mb, a, d, nout, s_lo, y + 0.9, s_hi, y + rise + 0.9, off + fw / 2 + 0.02, 0.018, 0.035, mat)
    mb.build(self.exterior)


Building._fire_escape = _g4_fire_escape

_g4_entry_gate_base = _fr_entry_gate


def _g4_entry_gate(self, e, a, b, span, gate, s, store, glass):
    _g4_entry_gate_base(self, e, a, b, span, gate, s, store, glass)
    tone = gate.get('backdrop')
    if not tone:
        return
    s0, s1, y0, y1 = span[:4]
    d = norm(sub(b, a))
    yaw = box_yaw(d)
    inward = self.inward(a, b)
    q = add(add(a, mul(d, (s0 + s1) / 2)), mul(inward, float(gate.get('backdropM', 0.9))))
    store.box((q[0], (y0 + y1) / 2, q[1]), (s1 - s0 + 0.1, y1 - y0, 0.04), self.tinted('painted-white', tone), yaw)


_mc_entry_gate = _g4_entry_gate



# ---------------------------------------------------------------- bldg-263347489 block 5 (door panels + overhang soffit)
# New style keys only; absent = unchanged:
#   garagePanel.panelW m : narrow tall panels instead of the cols x rows grid -
#                          `cols` panels of that width centred in equal slots
#                          across each leaf, `rows` high, each a darker inset
#                          face inside a raised moulding frame (`frameTint`)
#                          so every leaf reads as its own outlined door.
#   groundBand.soffit    {depthM, heightM, tint}: the upper storeys oversail
#                          the garage base - a slab `depthM` proud of every
#                          street face (front + streetEdges keys) at the band
#                          top, casting a real shadow line over the base.
_r5_closed_infill_base = Building._closed_infill


def _r5_closed_infill(self, kind, span, at, s, store, glass, yaw):
    gp = s.get('garagePanel')
    if kind != 'garage' or not gp or not gp.get('panelW'):
        return _r5_closed_infill_base(self, kind, span, at, s, store, glass, yaw)
    s0, s1, y0, y1, mat, tint = span[:6]
    w, h = s1 - s0, y1 - y0
    m = self.tinted('painted-white', gp.get('tint', '#3a3735'))
    pm = self.tinted('painted-white', gp.get('panelTint', '#2b2927'))
    fm = self.tinted('painted-white', gp.get('frameTint', '#4a4643'))
    jm = self.tinted('painted-steel', '#141516')
    n = max(1, int(gp.get('leaves') or round(w / float(gp.get('doorM', 2.4)))))
    cols, rows = int(gp.get('cols', 3)), int(gp.get('rows', 1))
    pw = float(gp['panelW'])
    back = -0.14
    lw = w / n
    for i in range(n):
        l0 = s0 + i * lw
        p = at(l0 + lw / 2, back)
        store.box((p[0], (y0 + y1) / 2, p[1]), (lw - 0.03, h, 0.04), m, yaw)
        my = 0.22
        ph = (h - 2 * my - (rows - 1) * 0.12) / rows
        slot = (lw - 0.2) / cols
        for ci in range(cols):
            sc = l0 + 0.1 + slot * (ci + 0.5)
            for ri in range(rows):
                yy = y0 + my + ph / 2 + ri * (ph + 0.12)
                q = at(sc, back + 0.025)
                store.box((q[0], yy, q[1]), (pw, ph, 0.012), pm, yaw)
                for dx in (-1, 1):
                    q2 = at(sc + dx * (pw / 2 + 0.015), back + 0.03)
                    store.box((q2[0], yy, q2[1]), (0.03, ph + 0.06, 0.022), fm, yaw)
                for dy in (-1, 1):
                    q2 = at(sc, back + 0.03)
                    store.box((q2[0], yy + dy * (ph / 2 + 0.015), q2[1]), (pw + 0.06, 0.03, 0.022), fm, yaw)
        if i:
            q = at(l0, back + 0.01)
            store.box((q[0], (y0 + y1) / 2, q[1]), (0.03, h, 0.05), jm, yaw)
    q = at((s0 + s1) / 2, -0.03)
    store.box((q[0], y1 - 0.03, q[1]), (w, 0.06, 0.06), jm, yaw)


Building._closed_infill = _r5_closed_infill

_bf5_build_exterior_base = Building.build_exterior


def _bf5_build_exterior(self):
    _bf5_build_exterior_base(self)
    band = self.style.get('groundBand') or {}
    sof = band.get('soffit')
    if not sof or len(self.levels) < 2:
        return
    keys = set((self.style.get('streetEdges') or {}).keys())
    n = len(self.poly)
    dep, hh = float(sof.get('depthM', 0.25)), float(sof.get('heightM', 0.14))
    top = float(band.get('topM', self.levels[1]))
    m = self.tinted(band.get('material', 'painted-white'), sof.get('tint', '#aab1b6'))
    mb = MeshBuilder('ground-soffit')
    for e in self.edges:
        if not (e['role'] == 'front' or str(e['index']) in keys):
            continue
        i = e['index']
        a, b = self.poly[i], self.poly[(i + 1) % n]
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        L = e['lengthM']
        q = add(add(a, mul(d, L / 2)), mul(nout, dep / 2))
        mb.box((q[0], top - hh / 2, q[1]), (L, hh, dep), m, box_yaw(d))
    mb.build(self.exterior)


Building.build_exterior = _bf5_build_exterior



# ---------------------------------------------------------------- bldg-263347489 block 5b (garage fit-out)
#   groundGarage.fitout true : concrete columns on the stall line behind every
#                              second stall, fluorescent strip fittings over the
#                              stalls and the aisle, and yellow-painted wheel-stop
#                              caps, so the garage seen through its doors reads
#                              as a working residents' garage.
_r5_vacant_base = Building._vacant


def _r5_vacant(self, rx0, rx1, kz, z1, top):
    _r5_vacant_base(self, rx0, rx1, kz, z1, top)
    g = self.style.get('groundGarage') or {}
    if not g.get('fitout'):
        return
    sw = float(g.get('stallW', 2.7))
    sd = min(float(g.get('stallD', 5.2)), max(2.5, z1 - kz - 1.2))
    paint = self.coarse_for(0)
    n = max(1, int((rx1 - rx0 - 0.4) // sw))
    x_start = (rx0 + rx1) / 2 - n * sw / 2
    zc = z1 - 0.3 - sd - 0.3
    if zc - 0.2 > kz + 0.3:
        for i in range(0, n + 1, 2):
            x = x_start + i * sw
            if rx0 + 0.3 < x < rx1 - 0.3:
                paint.box((x, top / 2, zc), (0.35, top, 0.35), 'int-concrete-floor', 0.0)
                self.physics['solids'].append({'id': f'f-garage-col-{i}', 'center': {'x': x, 'y': top / 2, 'z': zc},
                                               'size': {'x': 0.35, 'y': top, 'z': 0.35}, 'yaw': 0.0})
    for i in range(n):
        xc = x_start + (i + 0.5) * sw
        zs = z1 - 0.3 - sd + 0.35
        paint.box((xc, 0.145, zs), (1.6, 0.012, 0.18), 'int-fabric-yellow' if 'int-fabric-yellow' in INTERIOR else 'int-wood-light', 0.0)
        paint.box((xc, top - 0.04, z1 - 0.3 - sd / 2), (0.12, 0.05, 1.3), 'int-light', 0.0)
    for xc in frange(rx0 + 1.5, rx1 - 1.0, 3.0):
        paint.box((xc, top - 0.04, (kz + z1 - sd) / 2), (0.12, 0.05, 1.3), 'int-light', 0.0)


Building._vacant = _r5_vacant


def main():
    args = parse_args()
    with open(args.spec) as fh:
        spec = json.load(fh)
    style = dict(DEFAULT_STYLE)
    if args.style and os.path.exists(args.style):
        with open(args.style) as fh:
            user = json.load(fh)
        for k, v in user.items():
            if isinstance(v, dict) and isinstance(style.get(k), dict):
                style[k] = {**style[k], **v}
            else:
                style[k] = v
    apply_glass_model(style)
    if args.library and os.path.exists(args.library):
        # Placement needs only each piece's size (library.json, tracked); the
        # binaries are referenced, so a clean clone builds the same GLBs.
        with open(args.library) as fh:
            lib = json.load(fh)
        LIBRARY['dir'] = args.furniture_lib
        LIBRARY['pieces'] = dict(lib.get('pieces', {}))
    if args.textures and os.path.isdir(args.textures):
        LIBRARY['textures'] = args.textures
    # clean scene
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    b = Building(spec, style)
    b.plan_stair()
    b.build_exterior()
    b.build_interior()
    report = b.export(args.out)
    if args.render:
        render_previews(b, args.render)
    print('BLENDER_BUILDING_REPORT ' + json.dumps(report['files']))





# `shopBays` (bldg-264356957; absent = unchanged): per-shop dressing
# on a street face, so a row of small shops under one roof reads as distinct
# shopfronts instead of one repeated module. A list of
#   {edge, from, to,                       edge index + fractions along it
#    fascia:   {fromM, toM, tint, material?, projectionM?, lipTint?}
#    panel:    {fromM, toM, tint, material?, projectionM?, from?, to?}
#    cornice:  {topM?, heightM?, projectionM?, tint, material?, bracketM?, dentils?}
#    frieze:   {fromM, toM, tint, accent, moduleM?}
#    pilasters:{tint, widthM?, projectionM?, toM?, material?, at?: [fractions]}
#    windows:  [{from, to, sillM, headM, cols, rows, frameTint?, lit?}]
#    lattice:  [{from, to, fromM?, toM, tint?, cellM?, frameTint?}]
#    stringLights: [{atM, from?, to?, sagM?, spacingM?, offM?, tint?}]}
# Fractions inside a bay are of the whole edge (same as storefront.layout).
# fascia/panel/cornice/frieze are flat or stepped boxes on the wall face,
# windows are face-mounted multipane lights (dark or warm-lit backing, glass,
# muntin grid), lattice is a folding scissor gate (diagonal crossing bars and
# pickets) set just inside an opening. Projections stay under the same
# world-AABB allowance as the cornice.
_bb957_facade_relief_base = Building._facade_relief


def _bb957_strip(mb, P, out, u0, y0, u1, y1, t, mat):
    du, dy = u1 - u0, y1 - y0
    ln = math.hypot(du, dy) or 1.0
    pu, py = -dy / ln * t / 2, du / ln * t / 2
    mb.quad([P(u0 - pu, y0 - py), P(u1 - pu, y1 - py), P(u1 + pu, y1 + py), P(u0 + pu, y0 + py)], mat, out)


def _bb957_facade_relief(self, e, a, b, s):
    _bb957_facade_relief_base(self, e, a, b, s)
    bays = [sp for sp in (self.style.get('shopBays') or []) if int(sp.get('edge', -1)) == e['index']]
    if not bays:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    cap = self._projection_cap(d)
    out = B(nout[0], 0, nout[1])
    mb = MeshBuilder(f'shopbays-{e["index"]}')
    glass_mb = MeshBuilder(f'shopbays-glass-{e["index"]}')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    def span(f0, f1):
        return sorted((self.f2s(f0, L, e), self.f2s(f1, L, e)))

    def band(s0, s1, y0, y1, proj, mat):
        p = at((s0 + s1) / 2, proj / 2)
        mb.box((p[0], (y0 + y1) / 2, p[1]), (s1 - s0, y1 - y0, proj + 0.01), mat, yaw)

    for bay in bays:
        s0, s1 = span(bay.get('from', 0.0), bay.get('to', 1.0))
        fa = bay.get('fascia')
        if fa:
            proj = max(0.02, min(fa.get('projectionM', 0.1), cap))
            mat = self.tinted(fa.get('material', s['wall']), fa.get('tint'))
            y0, y1 = fa['fromM'], fa['toM']
            band(s0, s1, y0, y1, proj, mat)
            lip = self.tinted(fa.get('material', s['wall']), fa.get('lipTint') or fa.get('tint'))
            for yy in (y0 + 0.04, y1 - 0.04):
                band(s0, s1, yy - 0.04, yy + 0.04, min(proj + 0.05, cap), lip)
        for pn in (bay.get('panels') or ([bay['panel']] if bay.get('panel') else [])):
            p0, p1 = span(pn['from'], pn['to']) if 'from' in pn else (s0, s1)
            band(p0, p1, pn['fromM'], pn['toM'], max(0.01, min(pn.get('projectionM', 0.04), cap)),
                 self.tinted(pn.get('material', s['wall']), pn.get('tint')))
        fr = bay.get('frieze')
        if fr:
            y0, y1 = fr['fromM'], fr['toM']
            band(s0, s1, y0, y1, 0.05, self.tinted(fr.get('material', s['wall']), fr.get('tint')))
            acc = self.tinted(fr.get('material', s['wall']), fr.get('accent'))
            m = fr.get('moduleM', 0.32)
            n = max(1, int((s1 - s0) / m))
            h = y1 - y0
            for i in range(n):
                c = s0 + (s1 - s0) * (i + 0.5) / n
                # a running key: tall and short blocks alternating
                hh = h * (0.62 if i % 2 == 0 else 0.3)
                yc = y0 + h / 2 if i % 2 == 0 else (y0 + h * 0.3 if (i // 2) % 2 == 0 else y1 - h * 0.3)
                p = at(c, 0.07)
                mb.box((p[0], yc, p[1]), (m * 0.42, hh, 0.04), acc, yaw)
            band(s0, s1, y0 - 0.03, y0 + 0.03, 0.08, acc)
            band(s0, s1, y1 - 0.03, y1 + 0.03, 0.08, acc)
        pil = bay.get('pilasters')
        if pil:
            pw = pil.get('widthM', 0.35)
            pp = max(0.02, min(pil.get('projectionM', 0.12), cap))
            mat = self.tinted(pil.get('material', s['wall']), pil.get('tint'))
            y1 = pil.get('toM', self.height)
            pos = [self.f2s(f, L, e) for f in pil['at']] if pil.get('at') else [s0 + pw / 2, s1 - pw / 2]
            for sv in pos:
                sv = min(max(sv, pw / 2), L - pw / 2)
                p = at(sv, pp / 2)
                mb.box((p[0], y1 / 2, p[1]), (pw, y1, pp + 0.01), mat, yaw)
                q = at(sv, min(pp + 0.04, cap) / 2)
                mb.box((q[0], 0.15, q[1]), (pw + 0.06, 0.3, min(pp + 0.04, cap) + 0.01), mat, yaw)
        co = bay.get('cornice')
        if co:
            top = co.get('topM', self.height)
            hgt = co.get('heightM', 0.6)
            proj = max(0.04, min(co.get('projectionM', 0.3), cap))
            mat = self.tinted(co.get('material', s['wall']), co.get('tint'))
            # frieze board, bed mould, corona, cap: a real shadow line
            for y, hh, off in ((top - hgt, hgt * 0.45, 0.04), (top - hgt * 0.55, hgt * 0.14, proj * 0.45),
                               (top - hgt * 0.41, hgt * 0.3, proj), (top - hgt * 0.11, hgt * 0.11, proj * 0.65)):
                band(s0, s1, y, y + hh, off, mat)
            if co.get('bracketM'):
                n = max(2, int((s1 - s0) / co['bracketM']))
                for i in range(n + 1):
                    c = s0 + 0.12 + (s1 - s0 - 0.24) * i / n
                    p = at(c, proj * 0.42)
                    mb.box((p[0], top - hgt * 0.62, p[1]), (0.12, hgt * 0.42, proj * 0.84), mat, yaw)
            if co.get('dentils'):
                n = max(2, int((s1 - s0) / 0.16))
                for i in range(n):
                    c = s0 + (s1 - s0) * (i + 0.5) / n
                    p = at(c, proj * 0.3)
                    mb.box((p[0], top - hgt * 0.5, p[1]), (0.07, hgt * 0.1, proj * 0.6), mat, yaw)
        for w in bay.get('windows') or []:
            w0, w1 = span(w['from'], w['to'])
            y0, y1 = w['sillM'], w['headM']
            fm = self.tinted('painted-steel', w.get('frameTint'))
            back = self.tinted('painted-white', w['lit']) if w.get('lit') else 'painted-steel@0b0c0f'
            p = at((w0 + w1) / 2, 0.012)
            mb.box((p[0], (y0 + y1) / 2, p[1]), (w1 - w0, y1 - y0, 0.02), back, yaw)
            p = at((w0 + w1) / 2, 0.03)
            glass_mb.box((p[0], (y0 + y1) / 2, p[1]), (w1 - w0 - 0.06, y1 - y0 - 0.06, 0.01), 'glass-storefront', yaw)
            cols, rows = max(1, int(w.get('cols', 3))), max(1, int(w.get('rows', 2)))
            for i in range(cols + 1):
                t = 0.07 if i in (0, cols) else 0.035
                q = at(w0 + t / 2 + (w1 - w0 - t) * i / cols, 0.045)
                mb.box((q[0], (y0 + y1) / 2, q[1]), (t, y1 - y0, 0.04), fm, yaw)
            for r in range(rows + 1):
                t = 0.07 if r in (0, rows) else 0.035
                q = at((w0 + w1) / 2, 0.045)
                mb.box((q[0], y0 + t / 2 + (y1 - y0 - t) * r / rows, q[1]), (w1 - w0, t, 0.04), fm, yaw)
            q = at((w0 + w1) / 2, min(0.1, cap) / 2)
            mb.box((q[0], y0 - 0.04, q[1]), (w1 - w0 + 0.1, 0.08, min(0.1, cap)), fm, yaw)
        for lt in bay.get('lattice') or []:
            g0, g1 = span(lt['from'], lt['to'])
            y0, y1 = lt.get('fromM', 0.05), lt['toM']
            mat = self.tinted('painted-steel', lt.get('tint'))
            off = -0.06

            def P(u, y, off=off):
                p = at(u, off)
                return B(p[0], y, p[1])
            cell = lt.get('cellM', 0.34)
            cols = max(1, int(round((g1 - g0) / cell)))
            cw = (g1 - g0) / cols
            rows = max(1, int(round((y1 - y0) / (cw * 1.9))))
            ch = (y1 - y0) / rows
            for i in range(cols):
                u0, u1 = g0 + cw * i, g0 + cw * (i + 1)
                for r in range(rows):
                    v0, v1 = y0 + ch * r, y0 + ch * (r + 1)
                    _bb957_strip(mb, P, out, u0, v0, u1, v1, 0.028, mat)
                    _bb957_strip(mb, P, out, u0, v1, u1, v0, 0.028, mat)
            fmat = self.tinted('painted-steel', lt.get('frameTint')) if lt.get('frameTint') else mat
            for i in range(cols + 1):
                q = at(g0 + cw * i, off + 0.01)
                mb.box((q[0], (y0 + y1) / 2, q[1]), (0.03, y1 - y0, 0.03), mat, yaw)
            for yy in (y0 + 0.03, y1 - 0.04):
                q = at((g0 + g1) / 2, off + 0.012)
                mb.box((q[0], yy, q[1]), (g1 - g0, 0.07, 0.035), fmat, yaw)
            if lt.get('frameTint'):
                for uu in (g0 + 0.05, g1 - 0.05):
                    q = at(uu, off + 0.02)
                    mb.box((q[0], (y0 + y1) / 2, q[1]), (0.1, y1 - y0, 0.06), fmat, yaw)
        for sl in bay.get('stringLights') or []:
            l0, l1 = span(sl.get('from', bay.get('from', 0.0)), sl.get('to', bay.get('to', 1.0)))
            yl, sag = sl['atM'], sl.get('sagM', 0.25)
            bulb = self.tinted('painted-white', sl.get('tint', '#ffd89a'))
            wire = 'painted-steel@0b0c0f'
            n = max(2, int((l1 - l0) / sl.get('spacingM', 0.45)))
            for i in range(n + 1):
                t = i / n
                q = at(l0 + (l1 - l0) * t, min(sl.get('offM', 0.2), cap - 0.04))
                y = yl - sag * 4 * t * (1 - t)
                mb.box((q[0], y - 0.06, q[1]), (0.07, 0.09, 0.07), bulb, yaw)
                if i < n:
                    q2 = at(l0 + (l1 - l0) * (t + 0.5 / n), min(sl.get('offM', 0.2), cap - 0.04))
                    mb.box((q2[0], y, q2[1]), ((l1 - l0) / n, 0.012, 0.012), wire, yaw)
    mb.build(self.exterior)
    glass_mb.build(self.exterior)


Building._facade_relief = _bb957_facade_relief


# `shopStock` (bldg-264356957; absent = unchanged): a ground-floor
# shop at retail density - {palette?: [hex], racks?: n}. Goods on both faces
# of every shop shelf (folded stock and boxed wares from the interior fabric
# and painted materials), a line of garment rails behind the display glass,
# and a stocked wall run along each side wall.
_bb957_ground_base = Building._ground


def _bb957_ground(self, walls):
    _bb957_ground_base(self, walls)
    ss = self.style.get('shopStock')
    if not ss or self.ground_programme() != 'shop':
        return
    rng = random.Random(self.style.get('seed', 1) * 7919 + 13)
    pal = ss.get('palette') or ['#c8a24a', '#6d8f5a', '#b5553b', '#3d5f8c', '#e3dccb', '#8a4f7d', '#2f2f33', '#d98c3a']
    mats = ['int-fabric-red', 'int-fabric-blue', 'int-fabric-green', 'int-leather'] + [f'painted-white@{c.lstrip("#").lower()}' for c in pal]
    mb = self.soft_for(0)

    def goods_row(x, z, yaw, width, shelves, depth=0.34):
        c, s_ = math.cos(yaw), math.sin(yaw)
        for sy in shelves:
            u = -width / 2 + 0.05
            while u < width / 2 - 0.12:
                gw = rng.uniform(0.08, 0.26)
                gh = rng.uniform(0.1, 0.3)
                px, pz = x + u * c, z - u * s_
                mb.box((px, sy + gh / 2, pz), (gw, gh, depth * rng.uniform(0.6, 1.0)), rng.choice(mats), yaw)
                u += gw + rng.uniform(0.01, 0.05)
                if rng.random() < 0.08:
                    u += rng.uniform(0.1, 0.2)

    shelves = [f for f in self.furn_objects if f['kind'] == 'shelf' and f['storey'] == 0]
    for f in shelves:
        yaw = f['yaw']
        nx, nz = math.sin(yaw), math.cos(yaw)
        for side in (-1, 1):
            goods_row(f['x'] + nx * 0.36 * side, f['z'] + nz * 0.36 * side, yaw, 1.7, (0.12, 0.52, 0.92, 1.32, 1.62), depth=0.15)
    x0, x1, z0, z1 = self.rect
    st = self.stair
    kz = z0 + min(6.0, (z1 - z0) * 0.3)
    # garment rails a step behind the display glass (the front is +z = z1)
    n = int(ss.get('racks', 4))
    rx0, rx1 = x0 + 1.2, x1 - 1.2
    wx = st['x0'] + 1.45 if st['side'] < 0 else st['x1'] - 1.45
    if st['side'] < 0:
        rx0 = max(rx0, wx + 0.8)
    else:
        rx1 = min(rx1, wx - 0.8)
    for i in range(n):
        if rx1 - rx0 < 1.4:
            break
        cx = rx0 + (rx1 - rx0) * (i + 0.5) / n
        cz = z1 - 1.6
        w = min(1.3, (rx1 - rx0) / n - 0.3)
        mb.box((cx, 1.38, cz), (w, 0.03, 0.03), 'int-dark-metal')
        for sx in (cx - w / 2, cx + w / 2):
            mb.box((sx, 0.69, cz), (0.03, 1.38, 0.03), 'int-dark-metal')
            mb.box((sx, 0.01, cz), (0.05, 0.02, 0.45), 'int-dark-metal')
        u = cx - w / 2 + 0.06
        while u < cx + w / 2 - 0.06:
            gw = rng.uniform(0.035, 0.07)
            gl = rng.uniform(0.55, 0.95)
            mb.box((u, 1.34 - gl / 2, cz), (gw, gl, 0.42), rng.choice(mats), 0.0)
            u += gw + rng.uniform(0.005, 0.02)
        self.physics['solids'].append({'id': f'f-rail-{i}', 'center': {'x': cx, 'y': 0.7, 'z': cz},
                                       'size': {'x': max(0.1, w), 'y': 1.4, 'z': 0.45}, 'yaw': 0.0})
    # stocked wall run along both side walls, front half of the sales floor
    for wxl, yaw in ((x0 + 0.22, math.pi / 2), (x1 - 0.22, -math.pi / 2)):
        if st['side'] < 0 and wxl < st['x1'] + 1.6:
            continue
        if st['side'] > 0 and wxl > st['x0'] - 1.6:
            continue
        zc0, zc1 = kz + 1.0, z1 - 0.8
        if zc1 - zc0 < 1.0:
            continue
        mb.box((wxl, 1.1, (zc0 + zc1) / 2), (0.04, 2.2, zc1 - zc0), 'int-wood')
        for sy in (0.3, 0.75, 1.2, 1.65):
            mb.box((wxl + (0.14 if yaw > 0 else -0.14), sy - 0.02, (zc0 + zc1) / 2), (0.28, 0.03, zc1 - zc0), 'int-wood')
        goods_row(wxl + (0.15 if yaw > 0 else -0.15), (zc0 + zc1) / 2, yaw, zc1 - zc0, (0.3, 0.75, 1.2, 1.65), depth=0.22)


Building._ground = _bb957_ground



# `shopFitout` (bldg-264356957; absent = unchanged): splits one
# ground-floor sales hall into per-shop units and dresses them as retail.
#   {partitions: [{edge, at}],   shop boundaries as edge fractions (same as
#                                 storefront.layout); each becomes a party
#                                 partition running inward from the glass
#    rearPassageM?,               open strip kept at the back (reachability)
#    tints?: [...],               per-unit wall lining tints (cycled)
#    heightM?, stock?: {palette?, stacks?}}
# Partitions are cut wherever a placed solid (shelf, rail, stair) stands in
# their way, so they never overlap furniture; each unit's partition faces get
# a mid-tone lining so the display glass shows a coloured room, not a white
# hall. `stock.stacks` adds folded-garment stacks (thin layered slabs from the
# interior fabric materials) on low display tables behind the glass.
_bb957_ground_r4 = Building._ground


def _bb957_fitout(self, walls):
    _bb957_ground_r4(self, walls)
    fo = self.style.get('shopFitout')
    if not fo or self.ground_programme() != 'shop':
        return
    rng = random.Random(self.style.get('seed', 1) * 104729 + 7)
    x0, x1, z0, z1 = self.rect
    mb = self.soft_for(0)
    hgt = fo.get('heightM', min(3.6, self.height - 1.2))
    tints = fo.get('tints') or ['#8f7a63', '#6f7f86', '#9a6b58', '#7d7560', '#5f6a78']
    rear = fo.get('rearPassageM', 2.6)
    solids = [s_ for s_ in self.physics['solids'] if s_['center']['y'] < 2.5 and not str(s_.get('id', '')).startswith(('guard', 'landing'))]
    st = self.stair
    xs = []
    for p in fo.get('partitions') or []:
        e = next((x for x in self.edges if x['index'] == int(p['edge'])), None)
        if not e:
            continue
        a, b = (e['a']['x'], e['a']['z']), (e['b']['x'], e['b']['z'])
        L = e['lengthM']
        d = norm(sub(b, a))
        pt = add(a, mul(d, self.f2s(p['at'], L, e)))
        xs.append(min(x1 - 0.3, max(x0 + 0.3, pt[0])))
    xs.sort()
    T = 0.12
    zt, zb = z1 - 0.25, z0 + rear
    n_part = 0
    for i, px in enumerate(xs):
        if st['x0'] - 0.3 < px < st['x1'] + 0.3:
            continue
        blocks = []
        for s_ in solids:
            r = max(s_['size']['x'], s_['size']['z']) / 2 + 0.15
            if abs(s_['center']['x'] - px) < r + T / 2:
                blocks.append((s_['center']['z'] - r, s_['center']['z'] + r))
        segs, cur = [], zt
        for bz0, bz1 in sorted(blocks, key=lambda q: -q[1]):
            if bz1 < zb or bz0 > cur:
                continue
            if cur - bz1 > 1.2:
                segs.append((bz1, cur))
            cur = min(cur, bz0)
        if cur - zb > 1.2:
            segs.append((zb, cur))
        for sz0, sz1 in segs:
            n_part += 1
            zc, ln = (sz0 + sz1) / 2, sz1 - sz0
            mb.box((px, hgt / 2, zc), (T, hgt, ln), self.tinted('int-plaster', tints[i % len(tints)]))
            for side in (-1, 1):
                mb.box((px + side * (T / 2 + 0.006), hgt / 2, zc), (0.01, hgt - 0.05, ln - 0.02),
                       self.tinted('int-plaster', tints[(i + (1 if side > 0 else 0)) % len(tints)]))
                mb.box((px + side * (T / 2 + 0.012), 0.06, zc), (0.012, 0.12, ln), 'int-baseboard')
            self.physics['solids'].append({'id': f'f-partition-{n_part}', 'center': {'x': px, 'y': hgt / 2, 'z': zc},
                                           'size': {'x': T, 'y': hgt, 'z': ln}, 'yaw': 0.0})
    # folded stacks on low display tables a step behind the glass, one table per unit
    sk = (fo.get('stock') or {})
    if not sk.get('stacks'):
        return
    pal = sk.get('palette') or ['#3b4a5e', '#8c7b62', '#6e2f33', '#c9c1ae', '#2e3b2f', '#a8643c', '#50505a', '#d8cfbd']
    mats = ['int-fabric-red', 'int-fabric-blue', 'int-fabric-green', 'int-rug'] + [self.tinted('int-fabric-bed', c) for c in pal]
    edges_x = [x0 + 0.4] + xs + [x1 - 0.4]
    for i in range(len(edges_x) - 1):
        ux0, ux1 = edges_x[i] + 0.9, edges_x[i + 1] - 0.9
        if ux1 - ux0 < 1.0:
            continue
        cx, cz, tw, td = (ux0 + ux1) / 2, z1 - 3.0, min(1.8, ux1 - ux0), 0.8
        if any(abs(s_['center']['x'] - cx) < (s_['size']['x'] + tw) / 2 and abs(s_['center']['z'] - cz) < (s_['size']['z'] + td) / 2
               for s_ in self.physics['solids'] if s_['center']['y'] < 2.5):
            continue
        mb.box((cx, 0.74, cz), (tw, 0.04, td), 'int-wood')
        for lx in (cx - tw / 2 + 0.06, cx + tw / 2 - 0.06):
            for lz in (cz - td / 2 + 0.06, cz + td / 2 - 0.06):
                mb.box((lx, 0.36, lz), (0.04, 0.72, 0.04), 'int-dark-metal')
        u = cx - tw / 2 + 0.12
        while u < cx + tw / 2 - 0.2:
            for row in (-0.2, 0.2):
                y = 0.76
                for _ in range(rng.randint(3, 7)):
                    th = rng.uniform(0.025, 0.045)
                    mb.box((u + rng.uniform(-0.01, 0.01), y + th / 2, cz + row), (0.3, th, 0.26), rng.choice(mats), rng.uniform(-0.05, 0.05))
                    y += th
            u += 0.36
        self.physics['solids'].append({'id': f'f-stacktable-{i}', 'center': {'x': cx, 'y': 0.5, 'z': cz},
                                       'size': {'x': tw, 'y': 1.0, 'z': td}, 'yaw': 0.0})


# ---------------------------------------------------------------- bldg-264356936: brick flats dressing (_hb_)
Building._ground = _bb957_fitout


# bldg-264356957 (absent = unchanged): new keys inside a `shopBays`
# entry, all fractions of the whole edge as elsewhere in this block.
#   displays:   [{from, to, fromM?, toM?, frontM?, depthM?, backTint?,
#                 palette?, shelfM?}]  a packed window display a step behind
#               the storefront glass: dark back board, shelf tiers stocked edge
#               to edge with coloured goods, a lit valance over the top, so
#               the glass reads as merchandise, not an empty pale room
#   signBoards: [{from, to, fromM, toM, tint, faceTint?, frameTint?,
#                 projectionM?, glyphTint?, glyphRows?, lampsM?}]  a boxed sign
#               cabinet with a raised frame and inset face; glyphs are abstract
#               letter-like stroke blocks (no readable text), lamps are
#               gooseneck lights over the board
#   brackets:   {topM, heightM, projectionM?, spacingM?, tint}  modelled scroll
#               consoles (stepped plate, scroll foot, cap block) with
#               modillion blocks between them, real depth under the cornice
#   catenary:   [{atM, from?, to?, loopM?, sagM?, spacingM?, offM?}]  festoon
#               string lights: a sagging wire between hooks with emissive
#               bulbs hanging from it
# top-level `shopFixtures`: {ceilingM?, spacingM?, rowsM?: [inward m]}
#               pendant shop lights (rod, shade, emissive lamp) over the
#               ground-floor sales floor.
_bb957_relief_r5 = Building._facade_relief


def _bb957_relief_r6(self, e, a, b, s):
    _bb957_relief_r5(self, e, a, b, s)
    bays = [sp for sp in (self.style.get('shopBays') or []) if int(sp.get('edge', -1)) == e['index']]
    bays = [bp for bp in bays if any(bp.get(k) for k in ('displays', 'signBoards', 'brackets', 'catenary'))]
    if not bays:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    cap = self._projection_cap(d)
    out = B(nout[0], 0, nout[1])
    mb = MeshBuilder(f'shopbays6-{e["index"]}')
    rng = random.Random(self.style.get('seed', 1) * 31337 + e['index'])

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    def span(f0, f1):
        return sorted((self.f2s(f0, L, e), self.f2s(f1, L, e)))

    def bx(u, off, y, su, sy, sd, mat):
        p = at(u, off)
        mb.box((p[0], y, p[1]), (su, sy, sd), mat, yaw)

    for bay in bays:
        for dp in bay.get('displays') or []:
            u0, u1 = span(dp['from'], dp['to'])
            u0, u1 = u0 + 0.08, u1 - 0.08
            y0, y1 = dp.get('fromM', 0.45), dp.get('toM', 2.6)
            fr, dep = dp.get('frontM', 0.5), dp.get('depthM', 0.6)
            back = self.tinted('painted-steel', dp.get('backTint', '#141416'))
            shelf = self.tinted('painted-steel', dp.get('shelfTint', '#2a2622'))
            pal = dp.get('palette') or ['#c23b2e', '#e0a62a', '#2f6fb0', '#3f8f4e', '#7b3f8c', '#e8e0cc', '#d9642b', '#1f3f6e', '#b8325f', '#f0cf4a']
            mats = [self.tinted('painted-white', c) for c in pal] + ['int-fabric-red', 'int-fabric-blue', 'int-fabric-green', 'int-leather']
            uc, ul = (u0 + u1) / 2, u1 - u0
            bx(uc, -(fr + dep), (y0 + y1) / 2 + 0.2, ul, y1 - y0 + 0.4, 0.04, back)
            tiers = max(2, int((y1 - y0) / dp.get('shelfM', 0.42)))
            for t in range(tiers):
                sy = y0 + (y1 - y0) * t / tiers
                dd = dep * (1.0 - 0.45 * t / tiers)  # stepped: deeper low tiers
                bx(uc, -(fr + dep - dd / 2), sy, ul, 0.03, dd, shelf)
                u = u0 + 0.03
                while u < u1 - 0.1:
                    gw = rng.uniform(0.07, 0.24)
                    gh = rng.uniform(0.1, (y1 - y0) / tiers * 0.85)
                    gd = rng.uniform(0.12, max(0.14, dd * 0.8))
                    bx(u + gw / 2, -(fr + dep - gd / 2 - rng.uniform(0.0, dd - gd)), sy + 0.015 + gh / 2, gw, gh, gd, rng.choice(mats))
                    u += gw + rng.uniform(0.0, 0.03)
            bx(uc, -(fr + dep / 2), y1 + 0.32, ul, 0.1, dep, shelf)
            bx(uc, -(fr + dep * 0.55), y1 + 0.26, ul - 0.1, 0.025, 0.05, 'int-light')
            p = at(uc, -(fr + dep / 2))
            self.physics['solids'].append({'id': f'f-display-{e["index"]}-{len(self.physics["solids"])}',
                                           'center': {'x': p[0], 'y': (y1 + 0.4) / 2, 'z': p[1]},
                                           'size': {'x': ul, 'y': y1 + 0.4, 'z': dep}, 'yaw': yaw})
        for sb in bay.get('signBoards') or []:
            u0, u1 = span(sb['from'], sb['to'])
            y0, y1 = sb['fromM'], sb['toM']
            pj = max(0.08, min(sb.get('projectionM', 0.22), cap - 0.02))
            box_m = self.tinted('painted-steel', sb.get('tint'))
            face = self.tinted('painted-white', sb.get('faceTint', sb.get('tint')))
            frame = self.tinted('painted-steel', sb.get('frameTint', sb.get('tint')))
            uc, ul, h = (u0 + u1) / 2, u1 - u0, y1 - y0
            bx(uc, pj / 2, (y0 + y1) / 2, ul, h, pj, box_m)
            fw = min(0.1, h * 0.12)
            for yy in (y0 + fw / 2, y1 - fw / 2):
                bx(uc, (pj + 0.05) / 2, yy, ul + 0.04, fw, pj + 0.05, frame)
            for uu in (u0 + fw / 2, u1 - fw / 2):
                bx(uu, (pj + 0.05) / 2, (y0 + y1) / 2, fw, h, pj + 0.05, frame)
            bx(uc, pj + 0.004, (y0 + y1) / 2, ul - 2 * fw, h - 2 * fw, 0.01, face)
            gt = sb.get('glyphTint')
            if gt:
                gmat = self.tinted('painted-white', gt)
                rows = max(1, int(sb.get('glyphRows', 1)))
                ih = (h - 2 * fw) * 0.78
                rh = ih / rows
                for r in range(rows):
                    lh = rh * (0.72 if r == 0 else 0.5)
                    yc = y1 - fw - (h - 2 * fw - ih) / 2 - rh * (r + 0.5)
                    lw = lh * 0.62
                    avail = ul - 2 * fw - 0.3
                    nl = max(3, min(int(avail / (lw * 1.25)), 14 if r == 0 else 22))
                    tot = nl * lw * 1.25
                    u = uc - tot / 2
                    for i in range(nl):
                        if rng.random() < 0.14 and 0 < i < nl - 1:
                            u += lw * 1.25
                            continue
                        st = lw * 0.22
                        # stroke letterform: one or two stems plus bars
                        bx(u + st / 2, pj + 0.025, yc, st, lh, 0.03, gmat)
                        k = rng.randint(0, 3)
                        if k in (0, 1):
                            bx(u + lw - st / 2, pj + 0.025, yc, st, lh, 0.03, gmat)
                        for bar in ((1.0, 0.0), (0.5, 0.5), (0.0, 1.0))[: (1 + k) % 3 + 1]:
                            yb = yc - lh / 2 + st / 2 + (lh - st) * bar[0]
                            bx(u + lw / 2, pj + 0.025, yb, lw, st, 0.03, gmat)
                        u += lw * 1.25
            lm = sb.get('lampsM')
            if lm:
                n = max(1, int(ul / lm))
                for i in range(n):
                    uu = u0 + ul * (i + 0.5) / n
                    lp = min(pj + 0.3, cap - 0.03)
                    bx(uu, lp / 2, y1 + 0.12, 0.03, 0.03, lp, 'int-dark-metal')
                    bx(uu, lp - 0.04, y1 + 0.05, 0.16, 0.08, 0.12, 'int-dark-metal')
                    bx(uu, lp - 0.04, y1 + 0.005, 0.12, 0.012, 0.08, 'int-light')
        br = bay.get('brackets')
        if br:
            s0, s1 = span(bay.get('from', 0.0), bay.get('to', 1.0))
            top, hg = br['topM'], br.get('heightM', 0.8)
            pj = max(0.08, min(br.get('projectionM', 0.34), cap - 0.02))
            mat = self.tinted(br.get('material', s['wall']), br.get('tint'))
            sp = br.get('spacingM', 0.9)
            n = max(2, int((s1 - s0 - 0.3) / sp))
            for i in range(n + 1):
                c = s0 + 0.15 + (s1 - s0 - 0.3) * i / n
                bx(c, pj * 0.45 / 2, top - hg * 0.55, 0.16, hg * 0.8, pj * 0.45, mat)          # plate
                bx(c, pj * 0.7 / 2, top - hg * 0.3, 0.16, hg * 0.3, pj * 0.7, mat)             # upper scroll
                bx(c, pj * 0.3 / 2, top - hg * 0.92, 0.18, hg * 0.16, pj * 0.3, mat)           # foot
                bx(c, pj / 2, top - hg * 0.08, 0.22, hg * 0.16, pj, mat)                        # cap block
                if i < n:
                    c2 = c + (s1 - s0 - 0.3) / n / 2
                    for dx in (-0.22, 0.0, 0.22):
                        bx(c2 + dx, pj * 0.55 / 2, top - hg * 0.2, 0.1, hg * 0.12, pj * 0.55, mat)
        for cn in bay.get('catenary') or []:
            l0, l1 = span(cn.get('from', bay.get('from', 0.0)), cn.get('to', bay.get('to', 1.0)))
            yl, sag = cn['atM'], cn.get('sagM', 0.28)
            off = min(cn.get('offM', 0.22), cap - 0.04)
            loop = cn.get('loopM', 2.2)
            nl = max(1, int(round((l1 - l0) / loop)))
            lw = (l1 - l0) / nl

            def P(u, y, off=off):
                p = at(u, off)
                return B(p[0], y, p[1])
            wire = 'painted-steel@0b0c0f'
            spc = cn.get('spacingM', 0.3)
            for k in range(nl):
                a0 = l0 + lw * k
                bx(a0, off / 2, yl + 0.03, 0.03, 0.06, off, 'int-dark-metal')
                seg = 10
                for j in range(seg):
                    t0, t1 = j / seg, (j + 1) / seg
                    _bb957_strip(mb, P, out, a0 + lw * t0, yl - sag * 4 * t0 * (1 - t0),
                                 a0 + lw * t1, yl - sag * 4 * t1 * (1 - t1), 0.014, wire)
                nb = max(2, int(lw / spc))
                for j in range(nb):
                    t = (j + 0.5) / nb
                    y = yl - sag * 4 * t * (1 - t)
                    bx(a0 + lw * t, off, y - 0.04, 0.025, 0.05, 0.025, 'int-dark-metal')
                    bx(a0 + lw * t, off, y - 0.1, 0.075, 0.09, 0.075, 'int-light')
            bx(l1, off / 2, yl + 0.03, 0.03, 0.06, off, 'int-dark-metal')
    mb.build(self.exterior)


Building._facade_relief = _bb957_relief_r6

_bb957_ground_r5 = Building._ground


def _bb957_fixtures(self, walls):
    _bb957_ground_r5(self, walls)
    fx = self.style.get('shopFixtures')
    if not fx or self.ground_programme() != 'shop':
        return
    x0, x1, z0, z1 = self.rect
    mb = self.soft_for(0)
    ceil = fx.get('ceilingM', 4.4)
    sp = fx.get('spacingM', 2.4)
    st = self.stair
    for rz in fx.get('rowsM') or [2.2, 4.8, 7.4]:
        z = z1 - rz
        if z < z0 + 0.8:
            continue
        n = max(1, int((x1 - x0 - 1.0) / sp))
        for i in range(n):
            x = x0 + 0.5 + (x1 - x0 - 1.0) * (i + 0.5) / n
            if st['x0'] - 0.4 < x < st['x1'] + 0.4 and st.get('z0', z0) - 0.4 < z < st.get('z1', z1) + 0.4:
                continue
            drop = 0.9
            mb.box((x, ceil - drop / 2, z), (0.015, drop, 0.015), 'int-dark-metal')
            mb.box((x, ceil - drop - 0.09, z), (0.34, 0.18, 0.34), 'int-dark-metal')
            mb.box((x, ceil - drop - 0.185, z), (0.26, 0.012, 0.26), 'int-light')


Building._ground = _bb957_fixtures

# Appended, self-contained. NEW style keys only; absent = unchanged builds.
#   hbBayCrown: {dropM, hipRiseM, hipMaterial, hipTint, corbelTiers, corbelStepM,
#              panels, panelTint}  bays stop dropM below the roof line (so the
#              main cornice runs over them), get a hipped cap tucked under the
#              cornice, a stepped panelled corbel base, and raised apron panels.
#   fireEscape.hbStandoffM (+ hbSupportsAt: [f, ...])  platforms stand in front of
#              the bays instead of starting at the wall; steel outriggers carry them.
#   hbCorniceDentils: {spacingM, sizeM, tint}  dentil course under the corona.
#   hbSignBands: [{from, to, bottomM, topM, material, tint}]  shop sign boards.
#   hbArchDoors: [{from, to, material, tint, ringTint}]  round-arched head:
#              brick spandrels close the opening's top corners, voussoir ring.

def _hb_ring(bld, a, b, e, spec):
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(bld.inward(a, b), -1)
    c, w, p = bld.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.6)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))
    return d, nout, c, w, p, at


_hb_orig_bays = Building._bays


def _hb_bays(self, e, a, b):
    s = self.style
    crown = s.get('hbBayCrown')
    if not crown or not (s.get('bays') or []):
        return _hb_orig_bays(self, e, a, b)
    drop = float(crown.get('dropM', 0.35))
    saved = self.roof_level
    self.roof_level = saved - drop
    try:
        _hb_orig_bays(self, e, a, b)
    finally:
        self.roof_level = saved
    mb = MeshBuilder('bay-windows-crown')
    hip = self.tinted(crown.get('hipMaterial', 'painted-white'), crown.get('hipTint', '#2c4234'))
    rise = float(crown.get('hipRiseM', 0.35))
    for spec in s.get('bays') or []:
        if int(spec.get('roundCorners', 0) or 0) > 0 or spec.get('glazed'):
            continue
        d, nout, c, w, p, at = _hb_ring(self, a, b, e, spec)
        mat = self.tinted(spec.get('material', 'painted-stucco'), spec.get('tint'))
        k0 = spec.get('fromStorey', 1)
        k1 = min(self.storeys, spec.get('toStorey', self.storeys - 1) + 1)
        y_bottom = self.levels[k0]
        y_top = self.levels[k1] - 0.3 if k1 < self.storeys else self.roof_level - drop
        # hipped cap from the cap slab's outer edge back up to the wall
        pe = p + 0.12
        o = [at(c - w / 2, 0.0), at(c - w / 2 + p, pe), at(c + w / 2 - p, pe), at(c + w / 2, 0.0)]
        w1, w2 = at(c - w / 2 + p, 0.0), at(c + w / 2 - p, 0.0)
        y0, y1 = y_top + 0.35, y_top + 0.35 + rise
        up = B(0, 1, 0)
        mb.poly([B(o[0][0], y0, o[0][1]), B(o[1][0], y0, o[1][1]), B(w1[0], y1, w1[1])], hip, up)
        mb.poly([B(o[1][0], y0, o[1][1]), B(o[2][0], y0, o[2][1]), B(w2[0], y1, w2[1]), B(w1[0], y1, w1[1])], hip, up)
        mb.poly([B(o[2][0], y0, o[2][1]), B(o[3][0], y0, o[3][1]), B(w2[0], y1, w2[1])], hip, up)
        # stepped corbel base below the existing one (it ends at y_bottom - 0.45)
        tiers = int(crown.get('corbelTiers', 2))
        step = float(crown.get('corbelStepM', 0.22))
        yb = y_bottom - 0.45
        for t in range(1, tiers + 1):
            f = 1.0 - t / (tiers + 1.0)
            pp = p * f
            ring = [at(c - w / 2 + (p - pp), 0), at(c - w / 2 + p, pp), at(c + w / 2 - p, pp), at(c + w / 2 - (p - pp), 0)]
            self._prism(mb, ring, yb - step, yb, mat)
            yb -= step
        # raised apron panels under each facet's window, every storey
        if crown.get('panels', True):
            pmat = self.tinted(spec.get('material', 'painted-stucco'), crown.get('panelTint', spec.get('tint')))
            ring = [at(c - w / 2, 0), at(c - w / 2 + p, p), at(c + w / 2 - p, p), at(c + w / 2, 0)]
            ctr = at(c, p * 0.4)
            for k in range(k0, k1):
                ya = self.levels[k] + 0.05
                for fa, fb in zip(ring[:-1], ring[1:]):
                    fl = length(sub(fb, fa))
                    fd = norm(sub(fb, fa))
                    fn = (-fd[1], fd[0])
                    if dot(sub(lerp2(fa, fb, 0.5), ctr), fn) < 0:
                        fn = (fd[1], -fd[0])
                    ww = min(fl - 0.3, 1.05)
                    q = add(lerp2(fa, fb, 0.5), mul(fn, 0.02))
                    mb.box((q[0], ya + 0.4, q[1]), (ww, 0.5, 0.05), pmat, box_yaw(fd))
                    mb.box((q[0], ya + 0.68, q[1]), (ww + 0.12, 0.06, 0.09), pmat, box_yaw(fd))
            # panel faces on the lowest corbel tier's front
            fd = d
            q = at(c, p * (1.0 - 1.0 / (tiers + 1.0)) * 0.9 + 0.02)
            mb.box((q[0], y_bottom - 0.45 - step * 0.5, q[1]), (w - 2 * p, step * 0.6, 0.04), pmat, box_yaw(fd))
    mb.build(self.exterior)


Building._bays = _hb_bays

_hb_orig_fire_escape = Building._fire_escape


def _hb_fire_escape(self, e, a, b):
    spec = self.style.get('fireEscape')
    so = float((spec or {}).get('hbStandoffM', 0) or 0)
    if not spec or so <= 0 or self.storeys < 2:
        return _hb_orig_fire_escape(self, e, a, b)
    nout = mul(self.inward(a, b), -1)
    _hb_orig_fire_escape(self, e, add(a, mul(nout, so)), add(b, mul(nout, so)))
    L = e['lengthM']
    d = norm(sub(b, a))
    yaw = box_yaw(d)
    mat = spec.get('material', 'painted-steel')
    depth = spec.get('depthM', 0.9)
    mb = MeshBuilder('fire-escape-outriggers')
    k_last = min(self.storeys - 1, spec.get('toStorey', self.storeys - 1))
    c, w = self.f2s(spec['at'], L, e), spec.get('widthM', 3.4)
    sup = spec.get('hbSupportsAt')
    xs = [self.f2s(f, L, e) for f in sup] if sup else [c - w / 2 + 0.1, c + w / 2 - 0.1]
    for k in range(1, k_last + 1):
        y = self.levels[k] - 0.02
        for sv in xs:
            p = add(add(a, mul(d, sv)), mul(nout, (so + depth * 0.5) / 2))
            mb.box((p[0], y - 0.08, p[1]), (0.07, 0.1, so + depth * 0.5), mat, yaw)
            # knee brace from the wall below the platform out to the standoff
            n = 5
            for i in range(n):
                t = (i + 0.5) / n
                q = add(add(a, mul(d, sv)), mul(nout, so * t))
                mb.box((q[0], y - 0.9 * (1 - t) - 0.12, q[1]), (0.05, 0.2, so / n + 0.02), mat, yaw)
    mb.build(self.exterior)


Building._fire_escape = _hb_fire_escape

_hb_orig_edge_trim = Building._edge_trim


def _hb_edge_trim(self, e, a, b, trim):
    _hb_orig_edge_trim(self, e, a, b, trim)
    s = self.style
    if e['role'] != 'front':
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))
    mb = MeshBuilder('front-dressing')
    den = s.get('hbCorniceDentils')
    if den and s.get('cornice'):
        h = self.spec['frame']['heading']
        dw = (d[0] * math.cos(h) + d[1] * math.sin(h), -d[0] * math.sin(h) + d[1] * math.cos(h))
        cap = 0.43 / (abs(dw[0]) + abs(dw[1])) - 0.03
        proj = min(s['corniceProjectionM'], cap)
        sz = float(den.get('sizeM', 0.09))
        sp = float(den.get('spacingM', 0.2))
        dm = self.tinted(den.get('material', 'painted-white'), den.get('tint')) if den.get('tint') else s['trim']
        n = int((L - 0.2) / sp)
        top = self.height
        for i in range(n + 1):
            p = at(0.1 + i * sp, proj * 0.45 + sz / 2)
            mb.box((p[0], top - 0.41 - sz * 0.6, p[1]), (sz, sz * 1.2, sz), dm, yaw)
    for sb in s.get('hbSignBands') or []:
        s0, s1 = sorted((self.f2s(sb['from'], L, e), self.f2s(sb['to'], L, e)))
        p = at((s0 + s1) / 2, 0.06)
        y0, y1 = sb['bottomM'], sb['topM']
        mb.box((p[0], (y0 + y1) / 2, p[1]), (s1 - s0, y1 - y0, 0.1), self.tinted(sb.get('material', 'signage'), sb.get('tint')), yaw)
    for ad in s.get('hbArchDoors') or []:
        s0, s1 = sorted((self.f2s(ad['from'], L, e), self.f2s(ad['to'], L, e)))
        g = self.spec['floors']['groundM']
        head = g - s['storefront'].get('headDropM', 0.35)
        r = (s1 - s0) / 2
        cx = (s0 + s1) / 2
        spring = head - r
        wm = self.tinted(ad.get('material', s['wall']), ad.get('tint', s.get('wallTint')))
        rm = self.tinted(ad.get('material', s['wall']), ad.get('ringTint', ad.get('tint', s.get('wallTint'))))
        seg = 12
        off = 0.035
        for i in range(seg):
            t0, t1 = math.pi * i / seg, math.pi * (i + 1) / seg

            def arc(t, rr):
                return (cx + rr * math.cos(t), spring + rr * math.sin(t))

            def box_pt(t):
                cs, sn = math.cos(t), math.sin(t)
                return (cx + r * max(-1.0, min(1.0, cs * 1.5)), spring + r * max(0.0, min(1.0, sn * 1.5)))
            pts = [arc(t0, r), arc(t1, r), box_pt(t1), box_pt(t0)]
            ws = [at(u, off) for u, _ in pts]
            mb.poly([B(ws[k][0], pts[k][1], ws[k][1]) for k in range(4)], wm, B(nout[0], 0, nout[1]))
            # intrados strip back into the reveal
            ia, ib = arc(t0, r), arc(t1, r)
            fa, fb = at(ia[0], off), at(ib[0], off)
            ba, bb = at(ia[0], -WALL_T * 0.8), at(ib[0], -WALL_T * 0.8)
            mb.poly([B(fa[0], ia[1], fa[1]), B(fb[0], ib[1], fb[1]), B(bb[0], ib[1], bb[1]), B(ba[0], ia[1], ba[1])], wm, B(0, -1, 0))
            # voussoir ring
            tm = (t0 + t1) / 2
            vq = arc(tm, r + 0.12)
            vp = at(vq[0], off + 0.02)
            mb.box((vp[0], vq[1], vp[1]), (0.24 * abs(math.sin(tm)) + 0.06, 0.24 * abs(math.cos(tm)) + 0.06, 0.05), rm, yaw)
    mb.build(self.exterior)


Building._edge_trim = _hb_edge_trim



# ---------------------------------------------------------------- bldg-264356936 block 4: cornice brackets, frieze, mural dots, arch fill (_hb_)
# Appended, self-contained. NEW style keys only; absent = unchanged builds.
#   hbFrieze: {heightM, material, tint}  flat frieze board under the cornice,
#              full length of the front edge.
#   hbCorniceBrackets: {spacingM, heightM, depthM, widthM, material, tint}
#              scroll-bracket stand-ins (deep top block + tapered drop) under
#              the corona, evenly spaced along the front edge.
#   hbMuralDots: {count, rMinM, rMaxM, seed, palette}  round dots/rings over
#              the style `mural` field (same from/to/bottomM/topM); abstract.
#   hbArchDoors[].fillTint  dark infill of the arch head (the tympanum),
#              recessed into the reveal, so the head reads as a recessed door
#              instead of a lit pane.

_hb4_orig_edge_trim = Building._edge_trim


def _hb4_edge_trim(self, e, a, b, trim):
    _hb4_orig_edge_trim(self, e, a, b, trim)
    s = self.style
    if e['role'] != 'front':
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    out = B(nout[0], 0, nout[1])

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))
    mb = MeshBuilder('front-dressing-r4')
    top = self.height
    corona = top - 0.41
    fr = s.get('hbFrieze')
    fh = 0.0
    if fr and s.get('cornice'):
        fh = float(fr.get('heightM', 0.5))
        fm = self.tinted(fr.get('material', 'painted-white'), fr.get('tint')) if fr.get('tint') else s['trim']
        p = at(L / 2, 0.03)
        mb.box((p[0], corona - 0.11 - fh / 2, p[1]), (L - 0.1, fh, 0.06), fm, yaw)
    br = s.get('hbCorniceBrackets')
    if br and s.get('cornice'):
        sp = float(br.get('spacingM', 0.9))
        bh = float(br.get('heightM', 0.55))
        bd = float(br.get('depthM', 0.35))
        bw = float(br.get('widthM', 0.14))
        bm = self.tinted(br.get('material', 'painted-white'), br.get('tint')) if br.get('tint') else s['trim']
        n = max(1, int((L - 0.3) / sp))
        y1 = corona - 0.11
        for i in range(n + 1):
            sv = 0.15 + (L - 0.3) * i / n
            q = at(sv, 0.06 + bd / 2)
            mb.box((q[0], y1 - 0.09, q[1]), (bw, 0.18, bd), bm, yaw)
            q = at(sv, 0.06 + bd * 0.3)
            mb.box((q[0], y1 - 0.18 - (bh - 0.18) / 2, q[1]), (bw * 0.85, bh - 0.18, bd * 0.6), bm, yaw)
            q = at(sv, 0.06 + bd * 0.12)
            mb.box((q[0], y1 - bh - 0.05, q[1]), (bw * 0.7, 0.1, bd * 0.24), bm, yaw)
    md = s.get('hbMuralDots')
    mu = s.get('mural')
    if md and mu:
        s0, s1 = sorted((self.f2s(mu.get('from', 0.05), L, e), self.f2s(mu.get('to', 0.95), L, e)))
        y0 = mu.get('bottomM', 3.0)
        y1 = min(self.height - 0.4, mu.get('topM', self.height - 1.0))
        base = 0.012 + 0.004 * (int(mu.get('shapes', 36)) + 1)
        rng = random.Random(md.get('seed', 5))
        pal = md.get('palette') or ['#f2efe6', '#f0b531', '#d2553a', '#2f6fa8', '#3f8f5e', '#141414']
        rmin, rmax = float(md.get('rMinM', 0.08)), float(md.get('rMaxM', 0.35))
        seg = 10
        for i in range(int(md.get('count', 40))):
            r = rng.uniform(rmin, rmax)
            cx = rng.uniform(s0 + r, s1 - r)
            cy = rng.uniform(y0 + r, y1 - r)
            mat = f'painted-white@{rng.choice(pal).lstrip("#")}'
            off = base + 0.0015 * (i % 4)
            pts = []
            for k in range(seg):
                t = 2 * math.pi * k / seg
                p = at(cx + r * math.cos(t), off)
                pts.append(B(p[0], cy + r * math.sin(t), p[1]))
            mb.poly(pts, mat, out)
    for ad in s.get('hbArchDoors') or []:
        if not ad.get('fillTint'):
            continue
        s0, s1 = sorted((self.f2s(ad['from'], L, e), self.f2s(ad['to'], L, e)))
        g = self.spec['floors']['groundM']
        head = g - s['storefront'].get('headDropM', 0.35)
        r = (s1 - s0) / 2
        cx = (s0 + s1) / 2
        spring = head - r
        fm = self.tinted(ad.get('fillMaterial', 'painted-white'), ad['fillTint'])
        seg = 12
        off = -float(ad.get('fillDepthM', 0.12))
        pts = []
        for k in range(seg + 1):
            t = math.pi * k / seg
            p = at(cx + r * math.cos(t), off)
            pts.append(B(p[0], spring + r * math.sin(t), p[1]))
        mb.poly(pts, fm, out)
        dy = float(ad.get('fillDownM', 0.0))
        if dy > 0:
            p = at(cx, off)
            mb.box((p[0], spring - dy / 2, p[1]), (2 * r, dy, 0.03), fm, yaw)
    mb.build(self.exterior)


Building._edge_trim = _hb4_edge_trim


# ---------------------------------------------------------------- bldg-264356936 block 6: fire-escape stringers (_hb6_)
#   fireEscape.hbStringers: {widthM, depthM}  two stringers under each stair
#   flight (stepped bars from platform to platform) so the treads read as one
#   connected iron stair, not floating slats. Absent = unchanged.
_hb6_orig_fire_escape = Building._fire_escape


def _hb6_fire_escape(self, e, a, b):
    _hb6_orig_fire_escape(self, e, a, b)
    spec = self.style.get('fireEscape') or {}
    st = spec.get('hbStringers')
    if not st or self.storeys < 3:
        return
    so = float(spec.get('hbStandoffM', 0) or 0)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    c, w, depth = self.f2s(spec['at'], L, e), spec.get('widthM', 3.4), spec.get('depthM', 0.9)
    mat = spec.get('material', 'painted-steel')
    sw, sd = float(st.get('widthM', 0.05)), float(st.get('depthM', 0.14))
    mb = MeshBuilder('fire-escape-stringers')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off + so))

    k_last = min(self.storeys - 1, spec.get('toStorey', self.storeys - 1))
    steps = 14
    run = (w - 0.9) / steps
    for k in range(1, k_last):
        y = self.levels[k] - 0.02
        rise = self.levels[k + 1] - self.levels[k]
        for side_off in (0.28, 0.82):
            for i in range(steps + 1):
                q = at(c - w / 2 + 0.3 + i * run, depth * side_off)
                yy = y + rise * i / (steps + 1)
                mb.box((q[0], yy + rise / (steps + 1) / 2 - sd / 2, q[1]), (run + 0.01, rise / (steps + 1) + sd, sw), mat, yaw)
    mb.build(self.exterior)


Building._fire_escape = _hb6_fire_escape


# ---------------------------------------------------------------- bldg-264356936 block 6: street-measured floor split (_hb6_)
#   hbFloors: {groundM, upperM}  re-splits the spec's floor stack (e.g. a
#   shop storey read lower on the street than the civic default) without
#   changing the stack top: honoured only when ground + (n-1)*upper equals
#   the spec's stack within 5 cm, so the LiDAR height and roof stay put.
_hb6_orig_init = Building.__init__


def _hb6_init(self, spec, style):
    hf = (style or {}).get('hbFloors')
    f = (spec or {}).get('floors') or {}
    n = int((spec or {}).get('storeys') or 1)
    if hf and n > 1 and f.get('groundM') and f.get('upperM'):
        g, u = float(hf.get('groundM', 0)), float(hf.get('upperM', 0))
        top = f['groundM'] + (n - 1) * f['upperM']
        if g > 2.4 and u > 2.4 and abs(g + (n - 1) * u - top) <= 0.05:
            spec = dict(spec, floors=dict(f, groundM=g, upperM=u))
    _hb6_orig_init(self, spec, style)


Building.__init__ = _hb6_init



def _r3_tile_eaves(self):
    cfg = self.style.get('tileEave')
    if not cfg:
        return
    mb = MeshBuilder('tile-eave')
    y_w = self.height - float(cfg.get('belowTopM', 0.0))
    edges = [int(i) for i in cfg.get('edges', [])]
    for e in self.edges:
        if e['index'] not in edges:
            continue
        a, b = v2(e['a']), v2(e['b'])
        n = _r3_outward(self, a, b)
        _r3_eave_path(self, mb, [a, b], [n, n], y_w, cfg)
    cb = self.style.get('cornerBay')
    if cb and cfg.get('aroundBay', True):
        ring, O = _r3_bay_ring(self, cb)
        normals = [norm(sub(q, O)) for q in ring]
        _r3_eave_path(self, mb, ring, normals, y_w + float(cfg.get('bayRiseM', 0.0)), dict(cfg, bracketEveryM=cfg.get('bracketEveryM', 0.9)))
    mb.build(self.exterior)


def _r3_piers(self):
    cfg = self.style.get('storefrontPiers')
    if not cfg:
        return
    mat = self.tinted(cfg.get('material', 'painted-steel'), cfg.get('tint'))
    wmin = float(cfg.get('widthM', 0.32))
    dep = float(cfg.get('depthM', 0.08))
    mb = MeshBuilder('storefront-piers')
    g = self.spec['floors']['groundM']
    for e in self.edges:
        if not self.is_street(e):
            continue
        s = self.edge_style(e)
        sf = s['storefront']
        layout = sf.get('layout') or []
        if not layout:
            continue
        L = e['lengthM']
        a, b = v2(e['a']), v2(e['b'])
        d = norm(sub(b, a))
        nout = _r3_outward(self, a, b)
        yaw = box_yaw(d)
        head = g - sf.get('headDropM', 0.35)
        top = min(head + float(cfg.get('aboveHeadM', 0.0)), g - 0.05)
        spans = sorted(tuple(sorted((self.f2s(it['from'], L, e), self.f2s(it['to'], L, e)))) for it in layout)
        cuts = [(0.0, spans[0][0])] + [(spans[i][1], spans[i + 1][0]) for i in range(len(spans) - 1)] + [(spans[-1][1], L)]
        for s0, s1 in cuts:
            w = max(wmin, s1 - s0)
            c = (s0 + s1) / 2
            c = min(max(c, w / 2), L - w / 2)
            if s1 - s0 < 0.02 and 0.05 < c < L - 0.05:
                w = wmin * 0.6
            p = add(add(a, mul(d, c)), mul(nout, dep / 2))
            mb.box((p[0], top / 2, p[1]), (w, top, dep), mat, yaw)
            # a plinth at the foot
            p2 = add(add(a, mul(d, c)), mul(nout, dep + 0.02))
            mb.box((p2[0], 0.2, p2[1]), (w + 0.04, 0.4, 0.04), mat, yaw)
    mb.build(self.exterior)


_r3_build_exterior_base = Building.build_exterior


def _r3_build_exterior(self):
    _r3_build_exterior_base(self)
    _r3_corner_bay(self)
    _r3_tile_eaves(self)
    _r3_piers(self)


Building.build_exterior = _r3_build_exterior


# ---------------------------------------------------------------- interiors

def _r3_frame_x(self, u):
    return u if self.stair['side'] < 0 else -u


def _r3_flat(self, walls, fu0, fu1, zc, z1, base, top, k, idx):
    X = lambda u: _r3_frame_x(self, u)
    face_pu = math.atan2(X(1) - X(0), 0)   # facing +u
    face_mu = math.atan2(X(0) - X(1), 0)   # facing -u
    # bed against the rear partition at the far end of the flat
    bed_w, bed_l = 1.5, 2.05
    bu = fu1 - 0.45 - bed_w / 2 - 0.4
    bz = zc + 0.06 + bed_l / 2
    self.piece('bed-frame', X(bu), bz, 0.0, base, fit=(bed_w + 0.1, None, bed_l + 0.05))
    self.bedding(X(bu), bz, 0.0, base, k, bed_w, bed_l)
    for side in (-1, 1):
        self.piece('nightstand', X(bu + side * (bed_w / 2 + 0.33)), zc + 0.3, 0.0, base)
    self.bedside_lamp(X(bu - (bed_w / 2 + 0.33)), zc + 0.3, base + 0.62, k)
    self.wall_frame(X(bu), zc + 0.07, (0, 1), base, k, height=1.75)
    # living corner by the street window: sofa on the near partition, a low
    # table, an armchair facing it, a rug, a plant in the window corner
    sz = z1 - 1.75
    self.piece('sofa', X(fu0 + 0.45), sz, face_pu, base)
    self.piece('coffee-table', X(fu0 + 1.45), sz, face_pu, base)
    self.piece('lounge-chair', X(fu0 + 2.45), sz, face_mu, base, coarse='int-fabric-green')
    self.rug(X(fu0 + 1.45), sz, 2.2, 1.6, 0.0, base, k)
    self.piece('plant-medium', X(fu1 - 0.35), z1 - 0.4, 0.0, base, solid=False, coarse=False)
    self.wall_frame(X(fu0 + 0.07), sz, (X(1) - X(0), 0), base, k, height=1.6)
    # a dresser on the far partition, shelves beside the door
    self.piece('dresser', X(fu1 - 0.3), zc + 3.0, face_mu, base)
    if (idx + k) % 2 == 0:
        self.piece('shelving', X(fu0 + 1.7), zc + 0.3, 0.0, base)
    else:
        self.piece('writing-desk', X(fu0 + 1.7), zc + 0.35, 0.0, base)
    self.hang('ceiling-lamp', X((fu0 + fu1) / 2), (zc + z1) / 2, top, base)
    for slot in self.window_slots:
        mx, mz = lerp2(slot['a'], slot['b'], 0.5)
        if not (base < slot['sill'] < top + 0.3):
            continue
        lo, hi = sorted((X(fu0), X(fu1)))
        if lo - 0.3 < mx < hi + 0.3 and zc - 0.3 < mz < z1 + 0.8:
            self.curtains(slot, base, k)
    if idx % 2 == 0:
        self.add_light(X((fu0 + fu1) / 2), top - 0.4, (zc + z1) / 2, k, intensity=14.0, distance=7.0, priority=1)


_r3_upper_base = Building._upper


def _r3_upper(self, walls, k):
    cfg = self.style.get('upperFlats')
    if not cfg:
        return _r3_upper_base(self, walls, k)
    x0, x1, z0, z1 = self.rect
    st = self.stair
    base = self.levels[k]
    top = self.storey_top(k)
    X = lambda u: _r3_frame_x(self, u)
    if st['side'] < 0:
        u0, u1, su0, su1 = x0, x1, st['x0'], st['x1']
    else:
        u0, u1, su0, su1 = -x1, -x0, -st['x1'], -st['x0']
    cw = float(cfg.get('corridorM', 1.3))
    zc = z0 + cw                       # rear corridor's front wall
    cu1 = su1 + cw                     # side corridor beside the stair
    count = max(1, int(cfg.get('count', 4)))
    width = (u1 - cu1) / count
    # stair front partition with the arrival door (the stair lands at its z1)
    arrival_u = su1 - FLIGHT_W / 2
    self.physics['solids'] += partition(walls, (X(su0), st['z1']), (X(su1), st['z1']), base, top,
                                        [abs(X(arrival_u) - X(su0))] if st['side'] < 0 else [abs(X(arrival_u) - X(su0))])
    # flats' side wall along the side corridor
    self.physics['solids'] += partition(walls, (X(cu1), zc), (X(cu1), z1), base, top, [])
    # rear corridor wall with a door into every flat
    doors = []
    for i in range(count):
        fu0 = cu1 + i * width
        doors.append(abs(X(fu0 + PART_T + 0.2 + 0.45) - X(cu1)))
    self.physics['solids'] += partition(walls, (X(cu1), zc), (X(u1), zc), base, top, doors)
    for i in range(1, count):
        uu = cu1 + i * width
        self.physics['solids'] += partition(walls, (X(uu), zc), (X(uu), z1), base, top, [])
    for i in range(count):
        fu0 = cu1 + i * width + PART_T / 2 + 0.05
        fu1 = cu1 + (i + 1) * width - PART_T / 2 - 0.05
        _r3_flat(self, walls, fu0, fu1, zc + PART_T / 2 + 0.05, z1 - 0.05, base, top, k, i)
    # corridor: runner, lamps, sconces; the hall in front of the stair
    self.rug(X((cu1 + u1) / 2), z0 + cw / 2, u1 - cu1 - 1.0, cw - 0.5, 0.0, base, k)
    self.rug(X(su1 + cw / 2), (z0 + z1) / 2, cw - 0.5, z1 - z0 - 1.0, 0.0, base, k)
    for uu in frange(cu1 + 2.0, u1 - 1.0, 4.0):
        self.hang('ceiling-lamp', X(uu), z0 + cw / 2, top, base)
    self.hang('ceiling-lamp', X((su0 + su1) / 2), (st['z1'] + z1) / 2, top, base)
    self.piece('plant-tall', X(su0 + 0.4), z1 - 0.4, 0.0, base, solid=False, coarse=False)
    self.wall_frame(X(su0 + 0.07), (st['z1'] + z1) / 2, (X(1) - X(0), 0), base, k, height=1.6)
    self.add_light(X(su1), top - 0.3, (z0 + z1) / 2, k, intensity=12.0, distance=9.0, priority=2)


Building._upper = _r3_upper


_r3_ground_base = Building._ground


def _r3_ground(self, walls):
    cfg = self.style.get('groundUnits')
    if not cfg:
        return _r3_ground_base(self, walls)
    x0, x1, z0, z1 = self.rect
    st = self.stair
    top = self.storey_top(0)
    X = lambda u: _r3_frame_x(self, u)
    U = lambda x: x if st['side'] < 0 else -x
    if st['side'] < 0:
        u0, u1, su0, su1 = x0, x1, st['x0'], st['x1']
    else:
        u0, u1, su0, su1 = -x1, -x0, -st['x1'], -st['x0']
    # stair lobby from the street door to the core
    wu = su0 + 1.45
    door_x = getattr(self, 'stair_door_x', None)
    if door_x is not None:
        wu = max(wu, U(door_x) + 0.7)
    wu = max(wu, su1 + 0.05)
    self.physics['solids'] += partition(walls, (X(wu), z1), (X(wu), z0), 0.0, top, [])
    self.physics['solids'] += partition(walls, (X(su0), st['z1']), (X(su1), st['z1']), 0.0, top, [0.7])
    self.rug(X((su0 + wu) / 2), (st['z1'] + z1) / 2, min(1.0, wu - su0 - 0.4), max(1.0, z1 - st['z1'] - 1.0), 0.0, 0.0, 0)
    self.hang('ceiling-lamp', X((su0 + wu) / 2), (st['z1'] + z1) / 2, top, 0.0)
    if wu - su1 > 0.3:
        self.physics['solids'] += partition(walls, (X(su1), st['z1']), (X(wu), st['z1']), 0.0, top, [])
    # unit boundaries at front-layout fractions
    front = next(e for e in self.edges if e['role'] == 'front')
    a, b = v2(front['a']), v2(front['b'])
    d = norm(sub(b, a))
    cuts = sorted(U(a[0] + d[0] * self.f2s(f, front['lengthM'], front)) for f in cfg.get('splitAt', []))
    cuts = [c for c in cuts if wu + 2.0 < c < u1 - 2.0]
    bounds = [wu + PART_T] + cuts + [u1]
    kinds = list(cfg.get('kinds') or [])
    # kinds are listed from the lobby end toward the far end
    for i in range(1, len(bounds) - 1):
        self.physics['solids'] += partition(walls, (X(bounds[i]), z0), (X(bounds[i]), z1), 0.0, top, [])
    for i in range(len(bounds) - 1):
        ua, ub = bounds[i] + 0.1, bounds[i + 1] - 0.1
        kind = kinds[i] if i < len(kinds) else 'shop'
        _r3_unit(self, kind, ua, ub, z0 + 0.05, z1 - 0.05, top, i)
def _r3_unit(self, kind, ua, ub, za, zb, top, i):
    X = lambda u: _r3_frame_x(self, u)
    cu = (ua + ub) / 2
    w = ub - ua
    if kind == 'bar':
        # bar along the rear wall, back bar and bottles behind, stools, tables by the glass
        for uu in frange(ua + 0.9, ub - 0.6, 1.2):
            self.piece('bar-counter', X(uu), za + 1.55, 0.0, 0.0)
            self.piece('bar-stool', X(uu), za + 2.35, math.pi, 0.0, solid=False, coarse=False)
        self.piece('back-bar', X(cu), za + 0.3, 0.0, 0.0, fit=(max(1.2, w - 1.2), None, None))
        for j, uu in enumerate(frange(ua + 0.8, ub - 0.8, 1.3)):
            self.piece('wine-bottles', X(uu), za + 0.3, 0.0, 0.0, y=1.25 + 0.4 * (j % 2), solid=False, coarse=False)
        for uu in frange(ua + 0.8, ub - 0.8, 1.9):
            self.hang('pendant', X(uu), za + 1.55, top, 0.0)
        for uu in frange(ua + 1.0, ub - 0.9, 2.2):
            self.piece('cafe-table', X(uu), zb - 1.1, 0.0, 0.0)
            self.piece('cafe-chair', X(uu - 0.6), zb - 1.1, math.atan2(X(1) - X(0), 0), 0.0, solid=False, coarse=False)
            self.piece('cafe-chair', X(uu + 0.6), zb - 1.1, math.atan2(X(0) - X(1), 0), 0.0, solid=False, coarse=False)
        self.piece('plant-medium', X(ub - 0.4), zb - 0.4, 0.0, 0.0, solid=False, coarse=False)
        self.add_light(X(cu), top - 0.5, (za + zb) / 2, 0, intensity=24.0, distance=9.0, priority=5)
    elif kind == 'cafe':
        self.piece('bar-counter', X(cu), za + 1.3, 0.0, 0.0)
        self.piece('back-bar', X(cu), za + 0.3, 0.0, 0.0)
        for uu in frange(ua + 0.9, ub - 0.8, 1.9):
            self.piece('cafe-table', X(uu), zb - 1.6, 0.0, 0.0)
            self.piece('cafe-chair', X(uu), zb - 1.0, math.pi, 0.0, solid=False, coarse=False)
            self.piece('cafe-chair', X(uu), zb - 2.2, 0.0, 0.0, solid=False, coarse=False)
            self.hang('pendant', X(uu), zb - 1.6, top, 0.0, drop=1.2)
        self.piece('plant-small', X(ua + 0.4), zb - 0.4, 0.0, 0.0, solid=False, coarse=False)
        self.wall_frame(X(ua + 0.02), (za + zb) / 2, (X(1) - X(0), 0), 0.0, 0, height=1.65)
        self.add_light(X(cu), top - 0.5, (za + zb) / 2, 0, intensity=20.0, distance=8.0, priority=4)
    else:
        # a small shop: shelving on the rear and side walls, a counter, stock boxes
        for uu in frange(ua + 0.7, ub - 0.6, 1.1):
            self.piece('shelving', X(uu), za + 0.3, 0.0, 0.0)
        for zz in frange(za + 1.6, zb - 1.6, 1.2):
            self.piece('shelving', X(ub - 0.3), zz, math.atan2(X(0) - X(1), 0), 0.0)
        self.piece('shop-counter', X(ua + 0.8), zb - 2.0, math.atan2(X(1) - X(0), 0), 0.0)
        self.piece('cardboard-box', X(cu), za + 1.3, 0.4 + i, 0.0)
        self.piece('plant-small', X(ua + 0.8), zb - 2.0, 0.0, 0.0, y=1.0, solid=False, coarse=False)
        self.hang('ceiling-lamp', X(cu), (za + zb) / 2, top, 0.0)
        self.add_light(X(cu), top - 0.4, (za + zb) / 2, 0, intensity=18.0, distance=8.0, priority=3)


Building._ground = _r3_ground


# --- bldg-222862957 block 5. Self-contained; every key is
# new and absent from DEFAULT_STYLE, so other buildings build unchanged.
# Needs block 3 above (reuses _r3_eave_path, _r3_bay_ring, _r3_unit).
#   tileRoof        {sections:[{edge, from, to, lipM, riseM}], bay:{lipM, riseM},
#                   projectionM, material, courseMaterial, courseEveryM,
#                   ribEveryM, bracket, bracketEveryM}: a tall sloped
#                   clay-tile band (a hip-roof face seen from the street) from
#                   an absolute lip height up to the wall head (+riseM, inside
#                   the LiDAR allowance), in stepped sections with closed ends,
#                   a ridge roll and a back slope; also around `cornerBay`.
#   upperOpenings   [{at, w, kind: single|triple|balcony, sillM?, headM?}]
#                   (style or a streetEdges override; null = unchanged):
#                   replaces the evenly spaced upper punched windows with an
#                   irregular list; `balcony` adds a wrought-iron railing.
#   windowSurrounds {material?, widthM, projM, headM, headProjM}: raised
#                   trim surrounds with a moulded head on those openings.
#   corniceBand     {edges, belowLipM, heightM, projM, dentilEveryM, material?}
#                   a projecting cornice/frieze band under the tile lip.
#   shopUnits       [{from, to, awning?, awningDepthM?, fascia?, blade?}]:
#                   per-unit sign boards (fascia tints), canvas awnings and
#                   blank blade signs, a gooseneck lamp over every board.
#   streetDetail    {plaqueAt, conduitAt, meterAt}: a house-number plaque, a
#                   surface conduit run and a utility box on the front.
#   groundUnits kind 'fastfood': order counter, back bar, menu boards.

def _r5_edge_geom(self, e):
    a, b = v2(e['a']), v2(e['b'])
    d = norm(sub(b, a))
    nout = _r3_outward(self, a, b)
    return a, b, d, nout, box_yaw(d)


_r5_openings_base = Building.openings_for_edge


def _r5_openings(self, e):
    ops, windows, doors = _r5_openings_base(self, e)
    if e['role'] == 'party' or not self.is_street(e) or self.storeys < 2:
        return ops, windows, doors
    s = self.edge_style(e)
    items = s.get('upperOpenings')
    if items is None:
        return ops, windows, doors
    lvl1 = self.levels[1] - 0.01
    drop = [o for k, o in windows if k == 'punched' and o[2] >= lvl1]
    windows = [(k, o) for k, o in windows if not (k == 'punched' and o[2] >= lvl1)]
    ops = [o for o in ops if o not in drop]
    L = e['lengthM']
    for k in range(1, self.storeys):
        lvl = self.levels[k]
        top = lvl + self.storey_height(k) - 0.35
        for it in items:
            c = self.f2s(it['at'], L, e)
            w = float(it.get('w', s['windowW']))
            kind = it.get('kind', 'single')
            sill = lvl + float(it.get('sillM', 0.15 if kind == 'balcony' else s['windowSillM']))
            head = min(top, lvl + float(it.get('headM', s['windowHeadM'])))
            if kind == 'triple':
                mull = 0.14
                lw = (w - 2 * mull) / 3
                for j in range(3):
                    x0 = c - w / 2 + j * (lw + mull)
                    o = (x0, x0 + lw, sill, head)
                    ops.append(o)
                    windows.append(('punched', o))
            else:
                o = (c - w / 2, c + w / 2, sill, head)
                ops.append(o)
                windows.append(('punched', o))
    return ops, windows, doors


Building.openings_for_edge = _r5_openings


def _r5_surrounds_and_balconies(self):
    s = self.style
    ws = s.get('windowSurrounds')
    mb = MeshBuilder('window-surrounds')
    rail = MeshBuilder('balcony-rail')
    iron = s.get('railMaterial', 'painted-steel@1c1c1c')
    any_rail = False
    for e in self.edges:
        if e['role'] == 'party' or not self.is_street(e) or self.storeys < 2:
            continue
        es = self.edge_style(e)
        items = es.get('upperOpenings')
        if not items:
            continue
        a, b, d, nout, yaw = _r5_edge_geom(self, e)
        L = e['lengthM']
        at = lambda sv, off: add(add(a, mul(d, sv)), mul(nout, off))
        mat = self.tinted(ws.get('material', 'painted-wood'), ws.get('tint')) if ws and ws.get('tint') else (s['trim'] if ws else None)
        for k in range(1, self.storeys):
            lvl = self.levels[k]
            top = lvl + self.storey_height(k) - 0.35
            for it in items:
                c = self.f2s(it['at'], L, e)
                w = float(it.get('w', es['windowW']))
                kind = it.get('kind', 'single')
                sill = lvl + float(it.get('sillM', 0.15 if kind == 'balcony' else es['windowSillM']))
                head = min(top, lvl + float(it.get('headM', es['windowHeadM'])))
                if ws:
                    tw = float(ws.get('widthM', 0.13))
                    tp = float(ws.get('projM', 0.05))
                    hh = float(ws.get('headM', 0.26))
                    hp = float(ws.get('headProjM', 0.12))
                    for side in (-1, 1):
                        p = at(c + side * (w / 2 + tw / 2), tp / 2)
                        mb.box((p[0], (sill + head) / 2, p[1]), (tw, head - sill, tp), mat, yaw)
                    # moulded head: frieze plate plus a projecting cap
                    p = at(c, tp / 2)
                    mb.box((p[0], head + hh / 2, p[1]), (w + 2 * tw, hh, tp), mat, yaw)
                    p = at(c, hp / 2)
                    mb.box((p[0], head + hh + 0.04, p[1]), (w + 2 * tw + 0.16, 0.08, hp), mat, yaw)
                    if kind != 'balcony':
                        p = at(c, 0.07)
                        mb.box((p[0], sill - 0.06, p[1]), (w + 2 * tw + 0.1, 0.1, 0.14), mat, yaw)
                        # apron under the sill
                        p = at(c, tp / 2)
                        mb.box((p[0], sill - 0.3, p[1]), (w * 0.8, 0.22, tp), mat, yaw)
                    if kind == 'triple':
                        lw = (w - 0.28) / 3
                        for j in (1, 2):
                            p = at(c - w / 2 + j * (lw + 0.14) - 0.07, tp / 2)
                            mb.box((p[0], (sill + head) / 2, p[1]), (0.14, head - sill, tp), mat, yaw)
                if kind == 'balcony':
                    any_rail = True
                    dep = float(it.get('depthM', 0.36))
                    bw = w + 0.4
                    y0 = sill - 0.02
                    p = at(c, dep / 2)
                    rail.box((p[0], y0 - 0.06, p[1]), (bw, 0.12, dep), s.get('balconyDeck', 'painted-steel@2a2a2a'), yaw)
                    for side in (-1, 1):   # console brackets
                        p = at(c + side * (bw / 2 - 0.1), dep / 2)
                        rail.box((p[0], y0 - 0.3, p[1]), (0.05, 0.36, dep * 0.8), iron, yaw)
                    yr = y0 + 0.95
                    p = at(c, dep - 0.02)
                    rail.box((p[0], yr, p[1]), (bw, 0.05, 0.05), iron, yaw)
                    rail.box((p[0], y0 + 0.12, p[1]), (bw, 0.03, 0.03), iron, yaw)
                    nb = max(4, int(bw / 0.11))
                    for j in range(nb + 1):
                        p = at(c - bw / 2 + bw * j / nb, dep - 0.02)
                        rail.box((p[0], (y0 + yr) / 2, p[1]), (0.018, yr - y0, 0.018), iron, yaw)
                    for side in (-1, 1):
                        p = at(c + side * bw / 2, dep / 2)
                        rail.box((p[0], yr, p[1]), (0.05, 0.05, dep), iron, yaw)
                        for j in range(1, 4):
                            q = at(c + side * bw / 2, dep * j / 4)
                            rail.box((q[0], (y0 + yr) / 2, q[1]), (0.018, yr - y0, 0.018), iron, yaw)
    if ws:
        mb.build(self.exterior)
    if any_rail:
        rail.build(self.exterior)


def _r5_band(self, mb, pts, normals, y_top, lip, cfg, closed_ends=True):
    proj = float(cfg.get('projectionM', 0.42))
    tile = cfg.get('material', 'terracotta@a65a3e')
    course = cfg.get('courseMaterial', tile)
    band = dict(cfg, projectionM=proj, dropM=max(0.3, y_top - lip), bracket=None)
    _r3_eave_path(self, mb, pts, normals, y_top, band)
    drop = y_top - lip
    # horizontal tile courses: the butt edge of every row down the slope
    every = float(cfg.get('courseEveryM', 0.34))
    slope_len = math.hypot(drop, proj)
    n = max(2, int(slope_len / every))
    for i in range(len(pts) - 1):
        p, q = pts[i], pts[i + 1]
        np_, nq = normals[i], normals[i + 1]
        sn = norm(add(np_, nq))
        for j in range(1, n):
            t = j / n
            if t > 0.86:
                continue
            y = y_top - drop * t
            pa, qa = add(p, mul(np_, proj * t + 0.035)), add(q, mul(nq, proj * t + 0.035))
            mb.quad([B(pa[0], y - 0.045, pa[1]), B(qa[0], y - 0.045, qa[1]), B(qa[0], y + 0.02, qa[1]), B(pa[0], y + 0.02, pa[1])],
                    course, B(sn[0], -0.3, sn[1]))
    # ridge roll along the wall head, and a back slope onto the roof deck
    for i in range(len(pts) - 1):
        p, q = pts[i], pts[i + 1]
        seg = sub(q, p)
        Ls = length(seg)
        if Ls < 0.05:
            continue
        dd = norm(seg)
        c = lerp2(p, q, 0.5)
        mb.box((c[0], y_top + 0.05, c[1]), (Ls + 0.02, 0.12, 0.2), tile, box_yaw(dd))
        sn = norm(add(normals[i], normals[i + 1]))
        back = float(cfg.get('backM', 0.9))
        pb, qb = sub(p, mul(normals[i], back)), sub(q, mul(normals[i + 1], back))
        yb = max(self.roof_level + 0.05, self.height - 0.3)
        mb.quad([B(p[0], y_top, p[1]), B(q[0], y_top, q[1]), B(qb[0], yb, qb[1]), B(pb[0], yb, pb[1])], tile, B(-sn[0], 1, -sn[1]))
    if closed_ends:
        for idx, sgn in ((0, -1), (len(pts) - 1, 1)):
            p = pts[idx]
            nn = normals[idx]
            dd = norm(sub(pts[1], pts[0])) if idx == 0 else norm(sub(pts[-1], pts[-2]))
            lp = add(p, mul(nn, proj))
            mb.poly([B(p[0], y_top + 0.11, p[1]), B(p[0], lip - 0.1, p[1]), B(lp[0], lip - 0.1, lp[1])], tile, B(dd[0] * sgn, 0, dd[1] * sgn))
    # brackets (rafter tails) under the lip
    br = cfg.get('bracket')
    if br:
        every_b = float(cfg.get('bracketEveryM', 0.85))
        for i in range(len(pts) - 1):
            p, q = pts[i], pts[i + 1]
            Ls = length(sub(q, p))
            nb = int(Ls // every_b)
            if nb < 1:
                continue
            nn = norm(add(normals[i], normals[i + 1]))
            yaw = box_yaw(norm(sub(q, p)))
            for j in range(nb):
                w = lerp2(p, q, (j + 0.5) / nb)
                c = add(w, mul(nn, 0.17))
                mb.box((c[0], lip - 0.26, c[1]), (0.09, 0.22, 0.34), br, yaw)
def _r5_tile_roof(self):
    cfg = self.style.get('tileRoof')
    if not cfg:
        return
    mb = MeshBuilder('tile-roof')
    for sec in cfg.get('sections', []):
        e = next(x for x in self.edges if x['index'] == int(sec['edge']))
        a, b, d, nout, yaw = _r5_edge_geom(self, e)
        L = e['lengthM']
        s0, s1 = sorted((self.f2s(sec.get('from', 0.0), L, e), self.f2s(sec.get('to', 1.0), L, e)))
        pts = [add(a, mul(d, s0)), add(a, mul(d, s1))]
        y_top = self.height + float(sec.get('riseM', 0.0))
        _r5_band(self, mb, pts, [nout, nout], y_top, float(sec['lipM']), cfg)
    cb = self.style.get('cornerBay')
    bay = cfg.get('bay')
    if cb and bay:
        ring, O = _r3_bay_ring(self, cb)
        normals = [norm(sub(q, O)) for q in ring]
        _r5_band(self, mb, ring, normals, self.height + float(bay.get('riseM', 0.0)), float(bay['lipM']), cfg, closed_ends=False)
    mb.build(self.exterior)


def _r5_cornice(self):
    cfg = self.style.get('corniceBand')
    tr = self.style.get('tileRoof')
    if not cfg or not tr:
        return
    mat = self.tinted(cfg['material'], cfg.get('tint')) if cfg.get('material') else self.style['trim']
    mb = MeshBuilder('cornice-band')
    hgt = float(cfg.get('heightM', 0.3))
    pr = float(cfg.get('projM', 0.16))
    every = float(cfg.get('dentilEveryM', 0.36))
    for sec in tr.get('sections', []):
        e = next(x for x in self.edges if x['index'] == int(sec['edge']))
        a, b, d, nout, yaw = _r5_edge_geom(self, e)
        L = e['lengthM']
        s0, s1 = sorted((self.f2s(sec.get('from', 0.0), L, e), self.f2s(sec.get('to', 1.0), L, e)))
        y1 = float(sec['lipM']) - float(cfg.get('belowLipM', 0.4))
        c = (s0 + s1) / 2
        p = add(add(a, mul(d, c)), mul(nout, pr / 2))
        mb.box((p[0], y1 - 0.05, p[1]), (s1 - s0, 0.1, pr), mat, yaw)
        p = add(add(a, mul(d, c)), mul(nout, 0.03))
        mb.box((p[0], y1 - hgt / 2, p[1]), (s1 - s0, hgt, 0.06), mat, yaw)
        p = add(add(a, mul(d, c)), mul(nout, pr * 0.6 / 2))
        mb.box((p[0], y1 - hgt - 0.04, p[1]), (s1 - s0, 0.08, pr * 0.6), mat, yaw)
        n = int((s1 - s0) / every)
        for j in range(n):
            q = add(add(a, mul(d, s0 + (j + 0.5) * (s1 - s0) / n)), mul(nout, 0.09))
            mb.box((q[0], y1 - 0.16, q[1]), (0.1, 0.12, 0.12), mat, yaw)
    mb.build(self.exterior)


def _r5_shop_units(self):
    units = self.style.get('shopUnits')
    if not units:
        return
    s = self.style
    e = next(x for x in self.edges if x['role'] == 'front')
    a, b, d, nout, yaw = _r5_edge_geom(self, e)
    L = e['lengthM']
    at = lambda sv, off: add(add(a, mul(d, sv)), mul(nout, off))
    g = self.spec['floors']['groundM']
    head = g - s['storefront'].get('headDropM', 0.35)
    signs = MeshBuilder('sign-fascia')
    lamps = MeshBuilder('sign-lamps')
    awn = MeshBuilder('awning-units')
    blade = MeshBuilder('blade-sign-units')
    have_awn = have_blade = False
    y0 = head + 0.06
    y1 = min(g - 0.12, head + 0.66)
    for u in units:
        s0, s1 = sorted((self.f2s(u['from'], L, e), self.f2s(u['to'], L, e)))
        s0 += 0.2
        s1 -= 0.2
        if s1 - s0 < 0.4:
            continue
        c = (s0 + s1) / 2
        fm = self.tinted(u.get('fasciaMaterial', 'painted-steel'), u.get('fascia', '#1c1c1c'))
        p = at(c, 0.05)
        signs.box((p[0], (y0 + y1) / 2, p[1]), (s1 - s0, y1 - y0, 0.07), fm, yaw)
        # raised blank panel on the board (the lettering field) and a trim edge
        if u.get('panel'):
            p = at(c, 0.095)
            signs.box((p[0], (y0 + y1) / 2, p[1]), ((s1 - s0) * 0.72, (y1 - y0) * 0.56, 0.02), self.tinted('painted-steel', u['panel']), yaw)
        p = at(c, 0.1)
        signs.box((p[0], y1 + 0.02, p[1]), (s1 - s0 + 0.06, 0.05, 0.1), fm, yaw)
        # gooseneck lamps over the board
        n = max(1, int((s1 - s0) / 1.8))
        for j in range(n):
            q = at(s0 + (j + 0.5) * (s1 - s0) / n, 0.2)
            lamps.box((q[0], y1 + 0.16, q[1]), (0.03, 0.03, 0.34), 'painted-steel@1f2a22', yaw)
            q = at(s0 + (j + 0.5) * (s1 - s0) / n, 0.36)
            lamps.box((q[0], y1 + 0.08, q[1]), (0.16, 0.08, 0.12), 'painted-steel@1f2a22', yaw)
        if u.get('awning'):
            have_awn = True
            depth = float(u.get('awningDepthM', 1.0))
            ya = head + 0.05
            for k in range(4):
                t = (k + 0.5) / 4
                q = at(c, depth * t)
                awn.box((q[0], ya + 0.62 - 0.52 * t, q[1]), (s1 - s0, 0.03, depth / 4 + 0.02), u['awning'], yaw)
            q = at(c, depth)
            awn.box((q[0], ya - 0.02, q[1]), (s1 - s0, 0.22, 0.03), u['awning'], yaw)
            for side in (-1, 1):
                sv_ = c + side * (s1 - s0) / 2
                w0, w1 = at(sv_, 0.01), at(sv_, depth)
                awn.poly([B(w0[0], ya + 0.62, w0[1]), B(w1[0], ya + 0.1, w1[1]), B(w1[0], ya - 0.13, w1[1])],
                         u['awning'], B(d[0] * side, 0, d[1] * side))
        bl = u.get('blade')
        if bl:
            have_blade = True
            sv = self.f2s(bl.get('at', u['from']), L, e)
            bh = float(bl.get('heightM', 0.95))
            yb = float(bl.get('bottomM', g + 0.25))
            face = self.tinted('painted-steel', bl.get('face', '#e8e0cc'))
            q = at(sv, 0.62)
            blade.box((q[0], yb + bh / 2, q[1]), (0.07, bh, 0.62), face, yaw)
            q = at(sv, 0.62)
            blade.box((q[0], yb + bh / 2, q[1]), (0.09, bh + 0.06, 0.66), self.tinted('painted-steel', bl.get('edge', '#2a2a2a')), yaw, skip=('+x', '-x'))
            for yy in (yb + 0.15, yb + bh - 0.15):
                q = at(sv, 0.16)
                blade.box((q[0], yy, q[1]), (0.04, 0.04, 0.32), 'painted-steel@1c1c1c', yaw)
    signs.build(self.exterior)
    lamps.build(self.exterior)
    if have_awn:
        awn.build(self.exterior)
    if have_blade:
        blade.build(self.exterior)


def _r5_street_detail(self):
    cfg = self.style.get('streetDetail')
    if not cfg:
        return
    e = next(x for x in self.edges if x['role'] == 'front')
    a, b, d, nout, yaw = _r5_edge_geom(self, e)
    L = e['lengthM']
    at = lambda sv, off: add(add(a, mul(d, sv)), mul(nout, off))
    mb = MeshBuilder('street-detail')
    g = self.spec['floors']['groundM']
    if cfg.get('plaqueAt') is not None:
        q = at(self.f2s(cfg['plaqueAt'], L, e), 0.02)
        mb.box((q[0], 2.75, q[1]), (0.42, 0.16, 0.03), 'painted-white', yaw)
        q = at(self.f2s(cfg['plaqueAt'], L, e), 0.035)
        mb.box((q[0], 2.75, q[1]), (0.3, 0.08, 0.01), 'painted-steel@2a2a2a', yaw)
        # a wall lamp over the stair door
        q = at(self.f2s(cfg['plaqueAt'], L, e), 0.1)
        mb.box((q[0], 3.05, q[1]), (0.16, 0.24, 0.16), 'painted-steel@1f2a22', yaw)
    if cfg.get('conduitAt') is not None:
        q = at(self.f2s(cfg['conduitAt'], L, e), 0.03)
        mb.box((q[0], (0.4 + g) / 2, q[1]), (0.04, g - 0.4, 0.04), 'painted-steel@6b6b62', yaw)
    if cfg.get('meterAt') is not None:
        q = at(self.f2s(cfg['meterAt'], L, e), 0.08)
        mb.box((q[0], 1.1, q[1]), (0.45, 0.6, 0.16), 'painted-steel@5d6259', yaw)
    mb.build(self.exterior)


_r5_build_exterior_base = Building.build_exterior


def _r5_build_exterior(self):
    _r5_build_exterior_base(self)
    _r5_tile_roof(self)
    _r5_cornice(self)
    _r5_surrounds_and_balconies(self)
    _r5_shop_units(self)
    _r5_street_detail(self)


Building.build_exterior = _r5_build_exterior


_r5_unit_base = _r3_unit


def _r5_unit(self, kind, ua, ub, za, zb, top, i):
    if kind != 'fastfood':
        return _r5_unit_base(self, kind, ua, ub, za, zb, top, i)
    X = lambda u: _r3_frame_x(self, u)
    cu = (ua + ub) / 2
    # order counter across the unit, back bar behind, menu boards over it
    self.piece('bar-counter', X(cu), za + 1.5, 0.0, 0.0, fit=(max(1.2, ub - ua - 1.6), None, None))
    self.piece('back-bar', X(cu), za + 0.3, 0.0, 0.0, fit=(max(1.2, ub - ua - 1.0), None, None))
    for uu in (cu - 0.9, cu, cu + 0.9):
        self.wall_frame(X(uu), za + 0.03, (0, 1), 0.0, 0, height=2.3)
    for uu in frange(ua + 0.9, ub - 0.8, 1.7):
        self.piece('cafe-table', X(uu), zb - 1.3, 0.0, 0.0)
        self.piece('cafe-chair', X(uu), zb - 0.75, math.pi, 0.0, solid=False, coarse=False)
        self.piece('cafe-chair', X(uu), zb - 1.85, 0.0, 0.0, solid=False, coarse=False)
        self.hang('pendant', X(uu), zb - 1.3, top, 0.0, drop=1.1)
    self.piece('plant-small', X(ub - 0.4), zb - 0.4, 0.0, 0.0, solid=False, coarse=False)
    self.add_light(X(cu), top - 0.5, (za + zb) / 2, 0, intensity=22.0, distance=8.0, priority=4)


_r3_unit = _r5_unit


# --- bldg-222862957 block 9. Self-contained; every key is
# new and absent from DEFAULT_STYLE, so other buildings build unchanged.
#   partCornice   [{edge, from, to, heightM, projM, bracketEveryM, material?,
#                 closedEnds?}]: a flat bracketed Italianate cornice (frieze,
#                 dentils, corona, cap, scroll brackets) over PART of an edge,
#                 so one footprint can carry two frontages (a tile pent over
#                 one part, a bracketed cornice over the other). Projection is
#                 capped by the same world-AABB 0.45 m rule as `cornice`.
#   returnWall    {edge, material, tint, flashingM, downpipeAt?}: an exposed
#                 party/return wall gets its own weathered render skin, a
#                 metal flashing cap and an optional downpipe, instead of a
#                 bare slab in the facade paint.

def _r9_part_cornice(self):
    secs = self.style.get('partCornice')
    if not secs:
        return
    mb = MeshBuilder('part-cornice')
    h = self.spec['frame']['heading']
    top = self.height
    for sec in secs:
        e = next(x for x in self.edges if x['index'] == int(sec['edge']))
        a, b, d, nout, yaw = _r5_edge_geom(self, e)
        L = e['lengthM']
        s0, s1 = sorted((self.f2s(sec.get('from', 0.0), L, e), self.f2s(sec.get('to', 1.0), L, e)))
        span = s1 - s0
        if span < 0.2:
            continue
        mat = self.tinted(sec['material'], sec.get('tint')) if sec.get('material') else self.style['trim']
        # `bandMaterial`: bed mould, dentils, corona, cap and brackets in a
        # contrasting trim paint over the wall-coloured frieze panel
        band = sec.get('bandMaterial') or mat
        dw = (d[0] * math.cos(h) + d[1] * math.sin(h), -d[0] * math.sin(h) + d[1] * math.cos(h))
        cap = 0.43 / (abs(dw[0]) + abs(dw[1])) - 0.03
        proj = min(float(sec.get('projM', 0.4)), cap)
        hgt = float(sec.get('heightM', 1.0))
        c = (s0 + s1) / 2
        at = lambda sv, off: add(add(a, mul(d, sv)), mul(nout, off))
        # frieze panel, bed mould, dentil course, corona, cap
        rows = ((top - hgt, hgt * 0.62, 0.05), (top - hgt * 0.38, 0.1, proj * 0.35),
                (top - 0.38, 0.24, proj), (top - 0.14, 0.1, proj * 0.75))
        for ri, (y, hh, off) in enumerate(rows):
            p = at(c, off / 2)
            mb.box((p[0], y + hh / 2, p[1]), (span, hh, off + 0.02), mat if ri == 0 else band, yaw)
        # `dentils: false` drops the small dentil teeth (read as crenellation)
        n = max(2, int(span / 0.3)) if sec.get('dentils', True) is not False else 0
        for j in range(n):
            q = at(s0 + (j + 0.5) * span / n, proj * 0.3)
            mb.box((q[0], top - 0.47, q[1]), (0.09, 0.1, proj * 0.6), band, yaw)
        # scroll brackets from the frieze to the corona soffit
        every = float(sec.get('bracketEveryM', 1.2))
        nb = max(2, int(round(span / every)))
        for j in range(nb + 1):
            sv = s0 + 0.12 + (span - 0.24) * j / nb
            # `bracketW`/`bracketH`: heavier scroll brackets under the corona
            bw, bh = float(sec.get('bracketW', 0.16)), float(sec.get('bracketH', 0.5))
            q = at(sv, proj * 0.42)
            mb.box((q[0], top - 0.37 - bh / 2, q[1]), (bw, bh, proj * 0.84), band, yaw)
            q = at(sv, 0.1)
            mb.box((q[0], top - hgt * 0.8, q[1]), (0.14, 0.2, 0.2), band, yaw)
        if sec.get('closedEnds', True):
            for sv in (s0, s1):
                q = at(sv, proj / 2)
                mb.box((q[0], top - 0.3, q[1]), (0.05, 0.6, proj), mat, yaw)
    mb.build(self.exterior)


def _r9_return_wall(self):
    cfg = self.style.get('returnWall')
    if not cfg:
        return
    e = next(x for x in self.edges if x['index'] == int(cfg['edge']))
    a, b, d, nout, yaw = _r5_edge_geom(self, e)
    L = e['lengthM']
    at = lambda sv, off: add(add(a, mul(d, sv)), mul(nout, off))
    mb = MeshBuilder('return-wall')
    skin = self.tinted(cfg.get('material', 'painted-stucco'), cfg.get('tint'))
    y0, y1 = float(cfg.get('fromM', 0.0)), self.height - float(cfg.get('belowTopM', 0.0))
    p = at(L / 2, 0.015)
    mb.box((p[0], (y0 + y1) / 2, p[1]), (L - 0.04, y1 - y0, 0.02), skin, yaw)
    fl = float(cfg.get('flashingM', 0.18))
    metal = cfg.get('flashing', 'painted-steel@5d5f58')
    p = at(L / 2, 0.04)
    mb.box((p[0], self.height - fl / 2 + 0.02, p[1]), (L, fl, 0.08), metal, yaw)
    # a darker weather band at the foot of the wall (splash-back)
    if cfg.get('plinthTint'):
        p = at(L / 2, 0.03)
        mb.box((p[0], 0.35, p[1]), (L - 0.04, 0.7, 0.03), self.tinted('painted-stucco', cfg['plinthTint']), yaw)
    if cfg.get('downpipeAt') is not None:
        q = at(self.f2s(cfg['downpipeAt'], L, e), 0.07)
        mb.box((q[0], self.height / 2, q[1]), (0.09, self.height - 0.1, 0.09), metal, yaw)
    mb.build(self.exterior)


_r9_build_exterior_base = Building.build_exterior


def _r9_build_exterior(self):
    _r9_build_exterior_base(self)
    _r9_part_cornice(self)
    _r9_return_wall(self)


Building.build_exterior = _r9_build_exterior


# --- bldg-222862957 block 12. Self-contained; every key is
# new and absent from DEFAULT_STYLE, so other buildings build unchanged.
# Needs blocks 3/5/9 above (_r3_frame_x, _r3_unit, _r3_flat,
# _r5_edge_geom). Also adds `archWindows` to cornerBay and `dentils`,
# `bracketW`, `bracketH` to partCornice sections (edited in place above).
#   wallWeather  {edges:[i..], seed, streakTint, sootTint, sootTopM, sootM,
#                patchTints:[...], patchesPerGap}: weathering on the upper
#                street walls as thin offset skins in the wall paint: dirt
#                streaks under every upper opening's sill, a soot band under
#                the cornice/eave, and irregular faded repaint patches between
#                openings (a flat single-colour wall reads as a toy).
#   tileBarrels  {everyM, heightM, widthM, material, noseMaterial}: raised
#                barrel-tile rows down every `tileRoof` slope (straight
#                sections and the corner-bay ring), so the pent eave has
#                relief and shadow at street distance.
#   unitDress    [{wallTint, wainscotTint, railTint, kind?}] per ground unit
#                (same order as groundUnits.kinds): painted wall liners, a
#                wainscot and a picture rail per unit; `kind: salon` adds
#                styling chairs, mirrors and a waiting sofa, `market` adds
#                produce crates in front of the shelving.
#   flatDress    [{wallTint, extra}] per upper flat: painted liners and a
#                dresser / desk / armchair so the flats read lived in.
INTERIOR.update({
    'int-r12-oxblood': (0.36, 0.1, 0.08, 0.85, 0.0),
    'int-r12-green': (0.16, 0.26, 0.2, 0.85, 0.0),
    'int-r12-cream': (0.86, 0.8, 0.66, 0.9, 0.0),
    'int-r12-terracotta': (0.66, 0.36, 0.24, 0.9, 0.0),
    'int-r12-sage': (0.55, 0.6, 0.5, 0.9, 0.0),
    'int-r12-blue': (0.42, 0.5, 0.6, 0.9, 0.0),
    'int-r12-rose': (0.74, 0.58, 0.54, 0.9, 0.0),
    'int-r12-walnut': (0.24, 0.14, 0.08, 0.55, 0.0),
    'int-r12-black': (0.05, 0.05, 0.05, 0.5, 0.0),
    'int-r12-mirror': (0.75, 0.78, 0.8, 0.05, 1.0),
})


def _r12_wall_weather(self):
    cfg = self.style.get('wallWeather')
    if not cfg or self.storeys < 2:
        return
    rng = random.Random(int(cfg.get('seed', 12)))
    wall = self.style['wall']
    streak = self.tinted(wall, cfg.get('streakTint', '#7a6a50'))
    soot = self.tinted(wall, cfg.get('sootTint', '#6e604a'))
    patches = [self.tinted(wall, t) for t in cfg.get('patchTints', ['#a08a66', '#94805e'])]
    lvl1 = self.levels[1]
    soot_top = float(cfg.get('sootTopM', self.height - 1.3))
    soot_h = float(cfg.get('sootM', 0.45))
    mb = MeshBuilder('wall-weather')
    for ei in cfg.get('edges', []):
        e = next((x for x in self.edges if x['index'] == int(ei)), None)
        if e is None:
            continue
        a, b, d, nout, yaw = _r5_edge_geom(self, e)
        L = e['lengthM']
        at = lambda sv, off: add(add(a, mul(d, sv)), mul(nout, off))
        ops, windows, doors = self.openings_for_edge(e)
        ups = sorted(o for o in ops if o[2] >= lvl1 - 0.01)
        head_max = max([o[3] for o in ups], default=lvl1 + 1.0)
        # soot band under the cornice / eave, two steps (dark, then fading)
        s_top = max(soot_top, head_max + 0.35)
        for (y0, y1, off, m) in ((s_top - soot_h * 0.45, s_top, 0.014, soot), (s_top - soot_h, s_top - soot_h * 0.45, 0.012, streak)):
            p = at(L / 2, off)
            mb.box((p[0], (y0 + y1) / 2, p[1]), (L - 0.1, y1 - y0, 0.01), m, yaw)
        # dirt streaks under every upper opening's sill
        for (s0, s1, sill, head) in ups:
            w = s1 - s0
            for (c, sw, ln) in (((s0 + s1) / 2, w * 0.7, rng.uniform(0.6, 1.1)),
                                (s0 + 0.08, 0.1, rng.uniform(0.9, 1.5)),
                                (s1 - 0.08, 0.1, rng.uniform(0.8, 1.4))):
                y1 = sill - 0.06
                y0 = max(lvl1 + 0.12, y1 - ln)
                if y1 - y0 < 0.15:
                    continue
                p = at(c, 0.016)
                mb.box((p[0], (y0 + y1) / 2, p[1]), (sw, y1 - y0, 0.01), streak, yaw)
        # faded repaint patches between the openings (never over one)
        cuts = [0.05] + [x for o in ups for x in (o[0] - 0.08, o[1] + 0.08)] + [L - 0.05]
        gaps = [(cuts[i], cuts[i + 1]) for i in range(0, len(cuts) - 1, 2)]
        for (g0, g1) in gaps:
            if g1 - g0 < 0.35:
                continue
            for _ in range(int(cfg.get('patchesPerGap', 2))):
                pw = rng.uniform(0.35, 1.0) * (g1 - g0)
                pc = rng.uniform(g0 + pw / 2, g1 - pw / 2)
                ph = rng.uniform(0.6, 2.2)
                py = rng.uniform(lvl1 + 0.2 + ph / 2, max(lvl1 + 0.25 + ph / 2, s_top - soot_h - 0.05 - ph / 2))
                p = at(pc, 0.008)
                mb.box((p[0], py, p[1]), (pw, ph, 0.01), rng.choice(patches), yaw)
    mb.build(self.exterior)


def _r12_barrel_strip(self, mb, p, q, np_, nq, y_top, lip, proj, cfg):
    every = float(cfg.get('everyM', 0.22))
    hgt = float(cfg.get('heightM', 0.07))
    half = float(cfg.get('widthM', 0.12)) / 2
    mat = cfg.get('material', 'terracotta@8a5646')
    nose = cfg.get('noseMaterial', mat)
    drop = y_top - lip
    seg = sub(q, p)
    Ls = length(seg)
    if Ls < 0.2:
        return
    dd = norm(seg)
    sn = norm(add(np_, nq))
    # slope normal (outward-up) in the plane of sn and y
    ln = math.hypot(drop, proj)
    nh, ny = drop / ln, proj / ln
    n = max(1, int(Ls / every))
    for j in range(n):
        c = add(p, mul(dd, (j + 0.5) * Ls / n))

        def S(off_s, t, raise_):
            w = add(add(c, mul(dd, off_s)), mul(sn, proj * t + 0.035 + raise_ * nh))
            return B(w[0], y_top - drop * t + raise_ * ny, w[1])
        # stop short of the lip (`lipT`) so the barrels stay inside the 0.45 m AABB rule
        t0, t1 = 0.04, float(cfg.get('lipT', 0.86))
        L0, R0, M0 = S(-half, t0, 0), S(half, t0, 0), S(0, t0, hgt)
        L1, R1, M1 = S(-half, t1, 0), S(half, t1, 0), S(0, t1, hgt)
        up = B(sn[0] * nh, ny, sn[1] * nh)
        mb.quad([L0, M0, M1, L1], mat, up)
        mb.quad([M0, R0, R1, M1], mat, up)
        # the round nose of the barrel at the lip
        mb.poly([L1, M1, R1], nose, B(sn[0], -0.2, sn[1]))


def _r12_tile_barrels(self):
    cfg = self.style.get('tileBarrels')
    roof = self.style.get('tileRoof')
    if not cfg or not roof:
        return
    mb = MeshBuilder('tile-barrels')
    proj = float(roof.get('projectionM', 0.42))
    for sec in roof.get('sections', []):
        e = next(x for x in self.edges if x['index'] == int(sec['edge']))
        a, b, d, nout, yaw = _r5_edge_geom(self, e)
        L = e['lengthM']
        s0, s1 = sorted((self.f2s(sec.get('from', 0.0), L, e), self.f2s(sec.get('to', 1.0), L, e)))
        y_top = self.height + float(sec.get('riseM', 0.0))
        _r12_barrel_strip(self, mb, add(a, mul(d, s0)), add(a, mul(d, s1)), nout, nout, y_top, float(sec['lipM']), proj, cfg)
    cb = self.style.get('cornerBay')
    bay = roof.get('bay')
    if cb and bay:
        ring, O = _r3_bay_ring(self, cb)
        normals = [norm(sub(q, O)) for q in ring]
        y_top = self.height + float(bay.get('riseM', 0.0))
        for i in range(len(ring) - 1):
            _r12_barrel_strip(self, mb, ring[i], ring[i + 1], normals[i], normals[i + 1], y_top, float(bay['lipM']), proj, cfg)
    mb.build(self.exterior)


_r12_build_exterior_base = Building.build_exterior


def _r12_build_exterior(self):
    _r12_build_exterior_base(self)
    _r12_wall_weather(self)
    _r12_tile_barrels(self)


Building.build_exterior = _r12_build_exterior


def _r12_liners(self, mb, X, ua, ub, za, zb, base, top, d, rear=True):
    wall = d.get('wallMat', 'int-r12-cream')
    wains = d.get('wainscotMat')
    rail = d.get('railMat', 'int-r12-walnut')
    h = top - base - 0.03
    cu, w, zc = (ua + ub) / 2, ub - ua, (za + zb) / 2
    faces = [((X(ua - 0.04), zc), (0.02, zb - za - 0.1)), ((X(ub + 0.04), zc), (0.02, zb - za - 0.1))]
    if rear:
        faces.append(((X(cu), za - 0.03), (w + 0.06, 0.02)))
    for (x, z), (sx, sz) in faces:
        mb.box((x, base + h / 2, z), (sx, h, sz), wall)
        if wains:
            mb.box((x, base + 0.5, z), (sx + (0.02 if sx < 0.05 else 0), 1.0, sz + (0.02 if sz < 0.05 else 0)), wains)
        mb.box((x, base + (1.02 if wains else 2.3), z), (sx + (0.03 if sx < 0.05 else 0), 0.05, sz + (0.03 if sz < 0.05 else 0)), rail)


_r12_unit_base = _r3_unit


def _r12_unit(self, kind, ua, ub, za, zb, top, i):
    dress = self.style.get('unitDress') or []
    d = dress[i] if i < len(dress) else {}
    X = lambda u: _r3_frame_x(self, u)
    cu = (ua + ub) / 2
    k2 = d.get('kind')
    if k2 == 'salon':
        # styling stations along the rear wall: mirror, chair; waiting sofa by the glass
        for j, uu in enumerate(frange(ua + 0.8, ub - 0.7, 1.35)):
            self.wall_frame(X(uu), za + 0.03, (0, 1), 0.0, 0, height=1.35)
            self.piece('side-table', X(uu), za + 0.35, 0.0, 0.0, solid=False, coarse=False)
            self.piece('office-chair-real', X(uu), za + 1.05, 0.0, 0.0, solid=False, coarse=False)
        self.piece('sofa', X(cu), zb - 1.0, math.pi, 0.0)
        self.piece('plant-tall', X(ua + 0.4), zb - 0.5, 0.0, 0.0, solid=False, coarse=False)
        self.piece('shop-counter', X(ub - 0.8), zb - 2.2, math.atan2(X(0) - X(1), 0), 0.0)
        self.hang('ceiling-lamp', X(cu), (za + zb) / 2, top, 0.0)
        self.add_light(X(cu), top - 0.4, (za + zb) / 2, 0, intensity=20.0, distance=8.0, priority=3)
    elif k2 == 'market':
        _r12_unit_base(self, 'shop', ua, ub, za, zb, top, i)
        for j, uu in enumerate(frange(ua + 0.9, ub - 0.9, 0.9)):
            self.piece('crate', X(uu), zb - 0.55, 0.3 * (j % 3), 0.0, solid=False, coarse=False)
    else:
        _r12_unit_base(self, kind, ua, ub, za, zb, top, i)
    if d:
        mb = MeshBuilder(f'unit-dress-{i}')
        _r12_liners(self, mb, X, ua, ub, za, zb, 0.0, top, d)
        mb.build(self.interior)


_r3_unit = _r12_unit

_r12_flat_base = _r3_flat


def _r12_flat(self, walls, fu0, fu1, zc, z1, base, top, k, idx):
    _r12_flat_base(self, walls, fu0, fu1, zc, z1, base, top, k, idx)
    dress = self.style.get('flatDress') or []
    if not dress:
        return
    d = dress[idx % len(dress)]
    X = lambda u: _r3_frame_x(self, u)
    extra = d.get('extra')
    face_mu = math.atan2(X(0) - X(1), 0)
    if extra == 'desk':
        self.piece('writing-desk', X(fu0 + 0.6), z1 - 1.2, math.atan2(X(1) - X(0), 0), base)
        self.piece('office-chair-real', X(fu0 + 1.1), z1 - 1.2, face_mu, base, solid=False, coarse=False)
    elif extra == 'lounge':
        self.piece('lounge-chair', X(fu0 + 0.7), z1 - 0.9, 0.0, base, solid=False, coarse=False)
        self.piece('side-table', X(fu0 + 1.3), z1 - 0.7, 0.0, base, solid=False, coarse=False)
    elif extra == 'dresser':
        self.piece('dresser', X(fu0 + 0.5), zc + 0.7, math.atan2(X(1) - X(0), 0), base)
    self.piece('plant-small', X(fu0 + 0.3), z1 - 0.3, 0.0, base, solid=False, coarse=False)
    mb = MeshBuilder(f'flat-dress-{k}-{idx}')
    _r12_liners(self, mb, X, fu0 + 0.1, fu1 - 0.1, zc + 0.05, z1 - 0.05, base, top, d, rear=False)
    mb.build(self.interior)


_r3_flat = _r12_flat


# --- bldg-222862957 block 13. Self-contained; every key is
# new and absent from DEFAULT_STYLE, so other buildings build unchanged.
#   bayRoofDropM   (style, number): the upper projecting bays stop this far
#                  below the roof level, so a low bay sits under a deep plain
#                  frieze instead of running up into the cornice.
#   partCornice[].pairGapM: scroll brackets set in close pairs (this gap
#                  between the two) with no drop blocks on the frieze; the
#                  frieze stays a deep plain wall-coloured panel with one thin
#                  moulding (`mouldM`) at its foot and a slim corona.
#   shopUnits[].boardM / awningUnder: a taller sign board (boardM high, from
#                  the storefront head) and the awning hung BELOW the board
#                  over the transom, so the board still reads.
#   interiorFloor  {unitMat, flatMat, ceilingMat?}: a finished floor (and
#                  optional ceiling) liner in every ground unit and flat.
#   flatDress[].more: extra library pieces per flat (nightstand, lamp,
#                  frames, a rug-sized coffee table), so a flat is not one bed.
#   unitDress[].stools: a row of bar stools along the counter front.
INTERIOR.update({
    'int-r13-oak': (0.46, 0.3, 0.17, 0.45, 0.0),
    'int-r13-tile': (0.52, 0.48, 0.42, 0.35, 0.0),
    'int-r13-ceiling': (0.9, 0.88, 0.84, 0.9, 0.0),
})

_r13_bays_base = Building._bays


def _r13_bays(self, e, a, b):
    drop = float(self.style.get('bayRoofDropM', 0) or 0)
    if drop <= 0:
        return _r13_bays_base(self, e, a, b)
    keep = self.roof_level
    self.roof_level = keep - drop
    try:
        return _r13_bays_base(self, e, a, b)
    finally:
        self.roof_level = keep


Building._bays = _r13_bays

_r13_part_cornice_base = _r9_part_cornice


def _r13_part_cornice(self):
    secs = self.style.get('partCornice') or []
    if not any('pairGapM' in s for s in secs):
        return _r13_part_cornice_base(self)
    mb = MeshBuilder('part-cornice')
    h = self.spec['frame']['heading']
    top = self.height
    for sec in secs:
        e = next(x for x in self.edges if x['index'] == int(sec['edge']))
        a, b, d, nout, yaw = _r5_edge_geom(self, e)
        L = e['lengthM']
        s0, s1 = sorted((self.f2s(sec.get('from', 0.0), L, e), self.f2s(sec.get('to', 1.0), L, e)))
        span = s1 - s0
        if span < 0.2:
            continue
        mat = self.tinted(sec['material'], sec.get('tint')) if sec.get('material') else self.style['trim']
        band = sec.get('bandMaterial') or mat
        dw = (d[0] * math.cos(h) + d[1] * math.sin(h), -d[0] * math.sin(h) + d[1] * math.cos(h))
        cap = 0.43 / (abs(dw[0]) + abs(dw[1])) - 0.03
        proj = min(float(sec.get('projM', 0.4)), cap)
        hgt = float(sec.get('heightM', 1.0))
        mould = float(sec.get('mouldM', 0.07))
        c = (s0 + s1) / 2
        at = lambda sv, off: add(add(a, mul(d, sv)), mul(nout, off))
        # plain frieze panel (wall paint), thin moulding at its foot, a slim
        # bed mould, the corona and a cap
        rows = ((top - hgt, hgt - 0.3, 0.04, mat), (top - hgt - 0.02, mould, 0.1, band),
                (top - 0.34, 0.06, proj * 0.4, band), (top - 0.28, 0.16, proj, band),
                (top - 0.12, 0.08, proj * 0.8, mat))
        for y, hh, off, m in rows:
            p = at(c, off / 2)
            mb.box((p[0], y + hh / 2, p[1]), (span, hh, off + 0.02), m, yaw)
        gap = float(sec['pairGapM'])
        every = float(sec.get('bracketEveryM', 1.2))
        bw, bh = float(sec.get('bracketW', 0.1)), float(sec.get('bracketH', 0.32))
        nb = max(1, int(round(span / every)))
        for j in range(nb + 1):
            sv = s0 + 0.2 + (span - 0.4) * j / nb
            for side in (-0.5, 0.5):
                q = at(sv + side * gap, proj * 0.4)
                mb.box((q[0], top - 0.28 - bh / 2, q[1]), (bw, bh, proj * 0.8), band, yaw)
        if sec.get('closedEnds', True):
            for sv in (s0, s1):
                q = at(sv, proj / 2)
                mb.box((q[0], top - 0.2, q[1]), (0.05, 0.4, proj), mat, yaw)
    mb.build(self.exterior)


_r9_part_cornice = _r13_part_cornice

_r13_shop_units_base = _r5_shop_units


def _r13_shop_units(self):
    units = self.style.get('shopUnits') or []
    if not any(u.get('boardM') for u in units):
        return _r13_shop_units_base(self)
    # the base draws boards, lamps and blades; awnings are drawn here under
    # the taller board
    plain = [{k: v for k, v in u.items() if k != 'awning'} for u in units]
    keep = self.style['shopUnits']
    self.style['shopUnits'] = plain
    try:
        _r13_shop_units_base(self)
    finally:
        self.style['shopUnits'] = keep
    s = self.style
    e = next(x for x in self.edges if x['role'] == 'front')
    a, b, d, nout, yaw = _r5_edge_geom(self, e)
    L = e['lengthM']
    at = lambda sv, off: add(add(a, mul(d, sv)), mul(nout, off))
    g = self.spec['floors']['groundM']
    head = g - s['storefront'].get('headDropM', 0.35)
    boards = MeshBuilder('sign-board-tall')
    awn = MeshBuilder('awning-under-board')
    have_awn = False
    for u in units:
        s0, s1 = sorted((self.f2s(u['from'], L, e), self.f2s(u['to'], L, e)))
        s0 += 0.2
        s1 -= 0.2
        if s1 - s0 < 0.4:
            continue
        c = (s0 + s1) / 2
        bm = float(u.get('boardM', 0) or 0)
        if bm > 0.66:
            fm = self.tinted(u.get('fasciaMaterial', 'painted-steel'), u.get('fascia', '#1c1c1c'))
            y0, y1 = head + 0.06, min(g - 0.1, head + bm)
            p = at(c, 0.045)
            boards.box((p[0], (y0 + y1) / 2, p[1]), (s1 - s0, y1 - y0, 0.06), fm, yaw)
            p = at(c, 0.09)
            boards.box((p[0], y1 + 0.02, p[1]), (s1 - s0 + 0.06, 0.05, 0.1), fm, yaw)
        if u.get('awning'):
            have_awn = True
            depth = float(u.get('awningDepthM', 1.0))
            ya = head - float(u.get('awningDropM', 0.05))
            rise = 0.55
            for k in range(4):
                t = (k + 0.5) / 4
                q = at(c, depth * t)
                awn.box((q[0], ya - rise * t, q[1]), (s1 - s0, 0.03, depth / 4 + 0.02), u['awning'], yaw)
            q = at(c, depth)
            awn.box((q[0], ya - rise - 0.12, q[1]), (s1 - s0, 0.22, 0.03), u['awning'], yaw)
            for side in (-1, 1):
                sv_ = c + side * (s1 - s0) / 2
                w0, w1 = at(sv_, 0.01), at(sv_, depth)
                awn.poly([B(w0[0], ya, w0[1]), B(w1[0], ya - rise, w1[1]), B(w1[0], ya - rise - 0.23, w1[1])],
                         u['awning'], B(d[0] * side, 0, d[1] * side))
    boards.build(self.exterior)
    if have_awn:
        awn.build(self.exterior)


_r5_shop_units = _r13_shop_units


def _r13_floor(self, name, X, ua, ub, za, zb, base, top, mat, ceil):
    mb = MeshBuilder(name)
    xa, xb = X(ua + 0.02), X(ub - 0.02)
    cx, w = (xa + xb) / 2, abs(xb - xa)
    mb.box((cx, base + 0.012, (za + zb) / 2), (w, 0.02, zb - za - 0.04), mat)
    if ceil:
        mb.box((cx, top - 0.03, (za + zb) / 2), (w, 0.02, zb - za - 0.04), ceil)
    mb.build(self.interior)


_r13_unit_base = _r3_unit


def _r13_unit(self, kind, ua, ub, za, zb, top, i):
    _r13_unit_base(self, kind, ua, ub, za, zb, top, i)
    X = lambda u: _r3_frame_x(self, u)
    fl = self.style.get('interiorFloor')
    dress = self.style.get('unitDress') or []
    d = dress[i] if i < len(dress) else {}
    if fl:
        _r13_floor(self, f'unit-floor-{i}', X, ua, ub, za, zb, 0.0, top,
                   d.get('floorMat', fl.get('unitMat', 'int-r13-oak')), fl.get('ceilingMat'))
    if d.get('stools'):
        for uu in frange(ua + 0.9, ub - 0.6, 0.75):
            self.piece('bar-stool', X(uu), za + 2.3, math.pi, 0.0, solid=False, coarse=False)
    for j, kind2 in enumerate(d.get('more') or []):
        uu = ua + 0.6 + (ub - ua - 1.2) * (j + 0.5) / max(1, len(d['more']))
        self.piece(kind2, X(uu), (za + zb) / 2 + 0.4, 0.3 * j, 0.0, solid=False, coarse=False)


_r3_unit = _r13_unit

_r13_flat_base = _r3_flat


def _r13_flat(self, walls, fu0, fu1, zc, z1, base, top, k, idx):
    _r13_flat_base(self, walls, fu0, fu1, zc, z1, base, top, k, idx)
    X = lambda u: _r3_frame_x(self, u)
    fl = self.style.get('interiorFloor')
    if fl:
        _r13_floor(self, f'flat-floor-{k}-{idx}', X, fu0 + 0.1, fu1 - 0.1, zc + 0.05, z1 - 0.05, base, top,
                   fl.get('flatMat', 'int-r13-oak'), fl.get('ceilingMat'))
    dress = self.style.get('flatDress') or []
    if not dress:
        return
    more = dress[idx % len(dress)].get('more') or []
    for j, kind2 in enumerate(more):
        uu = fu0 + 0.5 + (fu1 - fu0 - 1.0) * (j + 0.5) / max(1, len(more))
        zz = zc + 0.45 if j % 2 == 0 else (zc + z1) / 2
        self.piece(kind2, X(uu), zz, math.pi * (j % 2), base, solid=False, coarse=False)


_r3_flat = _r13_flat


# --- bldg-222862957 block 14. Self-contained; the key is
# new and absent from DEFAULT_STYLE, so other buildings build unchanged.
#   bedDivan  {material, heightM, insetM}: a solid divan base from the floor
#             up under every made bed (`bedding`), so the mattress sits on a
#             closed box instead of reading as lifted off the floor when the
#             library frame is an open or low platform.
_r14_bedding_base = Building.bedding


def _r14_bedding(self, x, z, yaw, level, k, width, length):
    cfg = self.style.get('bedDivan')
    if cfg:
        h = float(cfg.get('heightM', 0.34))
        ins = float(cfg.get('insetM', 0.04))
        self.soft_for(k).box((x, level + h / 2, z), (width - 2 * ins, h, length - 2 * ins),
                             cfg.get('material', 'int-wood'), yaw)
    return _r14_bedding_base(self, x, z, yaw, level, k, width, length)


Building.bedding = _r14_bedding

# --- bldg-222862957 block 15. Self-contained; every key is
# new and absent from DEFAULT_STYLE, so other buildings build unchanged.
#   embedFurniture  true: library furniture is written into the fine storeys
#                   as real meshes instead of lib:<kind> reference empties
#                   (the runtime does not resolve those yet, so the storey the
#                   camera stands on otherwise shows no furniture at all).
#   sashLights      {upperCols, upperRows, lowerCols, barM, mullionMinW,
#                   mullionM, curtains: {tints, widthFrac}}: divided lights on
#                   every punched window (glazing bars in the upper and lower
#                   sash), a wide central mullion on windows at least
#                   mullionMinW wide (a sash pair), and drawn curtain panels at
#                   both jambs just behind the glass.
#   entryRecess     {depthM, floor, returnBulkhead?}: every storefront door is
#                   set back depthM in a recessed entry: tiled vestibule floor,
#                   glazed side returns on bulkheads, a soffit, the transom kept
#                   on the front plane and the door frame + open leaf at the
#                   back of the recess. The display panes either side stay on
#                   the front plane.
#   bulkheadTile    {everyM, rowsM, grout}: grout lines over every storefront
#                   bulkhead (and the entry returns), so the base panel reads
#                   as a tiled bulkhead instead of a flat slab.
#   flatRug         {material, marginM}: a rug on the floor of every flat.
_r15_build_interior_base = Building.build_interior


def _r15_build_interior(self):
    if self.style.get('embedFurniture'):
        LIBRARY['by_reference'] = False
    return _r15_build_interior_base(self)


Building.build_interior = _r15_build_interior

_r15_dress_openings_base = Building._dress_openings


def _r15_tile_grid(self, mb, at, s0, s1, off, y0, y1, cfg, yaw):
    every = float(cfg.get('everyM', 0.3))
    rows = cfg.get('rowsM') or []
    g = cfg.get('grout', 'painted-steel@3a322a')
    w = s1 - s0
    if w <= 0.05:
        return
    n = max(1, int(round(w / every)))
    for i in range(1, n):
        p = at(s0 + w * i / n, off)
        mb.box((p[0], (y0 + y1) / 2, p[1]), (0.012, y1 - y0, 0.012), g, yaw)
    for yy in rows:
        if y0 < yy < y1:
            p = at((s0 + s1) / 2, off)
            mb.box((p[0], yy, p[1]), (w, 0.012, 0.012), g, yaw)


def _r15_entry(self, s, at, nout, yaw, span, store, glass, cfg):
    s0, s1, y0, y1 = span[:4]
    d0, d1 = span[4], span[5]
    leaf_mat = span[6] if len(span) > 6 else s['storefrontFrame']
    sf = s['storefront']
    bulk, transom = sf['bulkheadM'], sf['transomM']
    gh1 = y1 - transom
    dh = min(2.45, gh1)
    R = float(cfg.get('depthM', 1.0))
    f0 = -0.12
    back = f0 - R
    ft = 0.06
    cm = (d0 + d1) / 2
    fr = s['storefrontFrame']
    # vestibule floor, flush with the pavement
    p = at(cm, (f0 + back) / 2)
    store.box((p[0], 0.012, p[1]), (d1 - d0, 0.024, R), cfg.get('floor', 'cast-stone@8a8378'), yaw)
    # glazed side returns on bulkheads, a post at each front and back corner
    for u in (d0 + 0.04, d1 - 0.04):
        p = at(u, (f0 + back) / 2)
        store.box((p[0], bulk / 2, p[1]), (0.08, bulk, R), cfg.get('returnBulkhead', s['bulkhead']), yaw)
        glass.box((p[0], (bulk + gh1) / 2, p[1]), (0.02, gh1 - bulk, R), s['storefrontGlass'], yaw)
        store.box((p[0], bulk, p[1]), (0.07, ft, R), fr, yaw)
        for off in (f0, back):
            q = at(u, off)
            store.box((q[0], gh1 / 2, q[1]), (ft, gh1, ft), fr, yaw)
    # soffit at the transom bar, transom glass kept on the front plane
    p = at(cm, (f0 + back) / 2)
    store.box((p[0], gh1 + 0.03, p[1]), (d1 - d0, 0.06, R), fr, yaw)
    g = at(cm, f0)
    glass.box((g[0], (gh1 + y1) / 2, g[1]), (d1 - d0, transom, 0.02), s['storefrontGlass'], yaw)
    store.box((g[0], gh1, g[1]), (d1 - d0, ft, 0.09), fr, yaw)
    store.box((g[0], y1 - ft / 2, g[1]), (d1 - d0, ft, 0.09), fr, yaw)
    # door frame at the back of the recess: jambs, head, a panel over the head
    ia, ib = d0 + 0.1, d1 - 0.1
    for u in (ia, ib):
        q = at(u, back)
        store.box((q[0], dh / 2, q[1]), (ft, dh, 0.1), fr, yaw)
    q = at(cm, back)
    store.box((q[0], dh + 0.03, q[1]), (ib - ia, 0.06, 0.1), fr, yaw)
    if gh1 - dh > 0.08:
        store.box((q[0], (dh + gh1) / 2, q[1]), (ib - ia, gh1 - dh, 0.05), fr, yaw)
    # the leaf standing open inward against a jamb (the doorway stays walkable)
    leaf_w = ib - ia - 0.06
    hinge = at(ia + 0.03, back - 0.02)
    c = add(hinge, mul(nout, -leaf_w / 2))
    store.box((c[0], dh / 2, c[1]), (0.05, dh - 0.02, leaf_w), leaf_mat, yaw)
    glass.box((c[0], dh * 0.62, c[1]), (0.06, dh * 0.5, leaf_w * 0.7), s['glass'], yaw)


def _r15_dress_openings(self, e, a, b, windows, trim, glass, frames, store, es=None):
    s = es or self.style
    er = self.style.get('entryRecess')
    bt = self.style.get('bulkheadTile')
    sl = self.style.get('sashLights')
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    entries = []
    if er and self.is_street(e):
        out = []
        for kind, span in windows:
            if kind == 'storefront-door' and len(span) >= 6 and not (len(span) > 7 and span[7] == 'double-door'):
                s0, s1, y0, y1, d0, d1 = span[:6]
                if s1 - s0 > 0.1:
                    if d0 - s0 >= 0.15:
                        out.append(('storefront', (s0, d0, y0, y1)))
                    if s1 - d1 >= 0.15:
                        out.append(('storefront', (d1, s1, y0, y1)))
                    entries.append(span)
                    continue
            out.append((kind, span))
        windows = out
    res = _r15_dress_openings_base(self, e, a, b, windows, trim, glass, frames, store, es)
    for span in entries:
        _r15_entry(self, s, at, nout, yaw, span, store, glass, er)
    if bt and self.is_street(e) and s.get('storefront'):
        bulk = s['storefront']['bulkheadM']
        for kind, span in windows:
            if kind == 'storefront':
                _r15_tile_grid(self, store, at, span[0] + 0.06, span[1] - 0.06, -0.12 + 0.045, 0.02, bulk - 0.03, bt, yaw)
            elif kind == 'storefront-door':
                s0, s1, y0, y1, d0, d1 = span[:6]
                for p0, p1 in ((s0, d0), (d1, s1)):
                    if p1 - p0 >= 0.15:
                        _r15_tile_grid(self, store, at, p0 + 0.06, p1 - 0.06, -0.12 + 0.045, 0.02, bulk - 0.03, bt, yaw)
    if sl:
        depth_glass = WALL_T * 0.35
        fm = s['frame']
        bar = float(sl.get('barM', 0.03))
        cur = sl.get('curtains') or {}
        tints = cur.get('tints') or []
        for kind, span in windows:
            if kind != 'punched':
                continue
            s0, s1, y0, y1 = span[:4]
            w, h = s1 - s0, y1 - y0
            cs = (s0 + s1) / 2
            mid = (y0 + y1) / 2 + 0.05
            off = -depth_glass + 0.02
            lights = [(s0, s1)]
            if w >= float(sl.get('mullionMinW', 99)):
                mw = float(sl.get('mullionM', 0.14))
                p = at(cs, -depth_glass + 0.03)
                frames.box((p[0], (y0 + y1) / 2, p[1]), (mw, h, 0.1), fm, yaw)
                lights = [(s0, cs - mw / 2), (cs + mw / 2, s1)]
            for la, lb in lights:
                lw = lb - la
                for cols, ya, yb in ((int(sl.get('upperCols', 1)), mid, y1), (int(sl.get('lowerCols', 1)), y0, mid)):
                    for c in range(1, cols):
                        p = at(la + lw * c / cols, off)
                        frames.box((p[0], (ya + yb) / 2, p[1]), (bar, yb - ya, 0.05), fm, yaw)
                rows = int(sl.get('upperRows', 1))
                for r in range(1, rows):
                    p = at((la + lb) / 2, off)
                    frames.box((p[0], mid + (y1 - mid) * r / rows, p[1]), (lw, bar, 0.05), fm, yaw)
            if tints:
                hsh = int(abs(cs * 7.31 + y0 * 5.03 + len(tints)) * 97)
                if hsh % 5 == 0:
                    continue   # some windows bare
                t = tints[hsh % len(tints)]
                cw = w * float(cur.get('widthFrac', 0.25))
                for u in (s0 + 0.07 + cw / 2, s1 - 0.07 - cw / 2):
                    p = at(u, -depth_glass - 0.06)
                    glass.box((p[0], (y0 + y1) / 2 + 0.02, p[1]), (cw, h - 0.14, 0.03),
                              self.tinted('painted-white', t), yaw)
    return res


Building._dress_openings = _r15_dress_openings

_r15_flat_base = _r3_flat


def _r15_flat(self, walls, fu0, fu1, zc, z1, base, top, k, idx):
    _r15_flat_base(self, walls, fu0, fu1, zc, z1, base, top, k, idx)
    rug = self.style.get('flatRug')
    if not rug:
        return
    X = lambda u: _r3_frame_x(self, u)
    m = float(rug.get('marginM', 0.9))
    w = (fu1 - fu0) - 2 * m
    dz = (z1 - zc) - 2 * m
    if w < 0.8 or dz < 0.8:
        return
    mb = MeshBuilder(f'flat-rug-{k}-{idx}')
    mb.box((X((fu0 + fu1) / 2), base + 0.03, (zc + z1) / 2), (w, 0.012, dz), rug.get('material', 'int-rug-border'))
    mb.build(self.interior)


_r3_flat = _r15_flat



# ---- more extras (bldg-256729086); every key NEW, absent = unchanged.
#   scissorGrilles: [{from, to, bottomM, topM, material, pitchM}] - a folding
#       lattice gate (diagonal bars + verticals) just outside a shopfront.
#   roofLadders: [{at, fromM, toM, widthM, offM, material}] - a wall ladder with
#       rails that rise past the parapet (goosenecked over it).
#   flagpoles: [{at, offM, heightM, lean, material}] - poles on the parapet.
#   wallVents: [{at, yM, widthM, heightM}] - small louvred wall vents.
#   windowCurtains: {tints: [...], every, widthFrac} - part-drawn side curtains
#       behind upper punched windows (from window_slots).
#   sashFrames: {material, stileM} - heavier sash stiles + meeting rail in front
#       of the glass of upper punched windows.
def _bc_r10(self, e, a, b):
    s = self.style
    keys = ('scissorGrilles', 'roofLadders', 'flagpoles', 'wallVents', 'windowCurtains', 'sashFrames')
    if not any(s.get(k) for k in keys):
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    d3, n3 = _bc_p3(d, 0), _bc_p3(nout, 0)

    def at(sv, off):
        return at_edge(a, d, nout, sv, off)
    mb = MeshBuilder('facade-r10')
    for g in s.get('scissorGrilles') or []:
        s0, s1 = sorted((self.f2s(g['from'], L, e), self.f2s(g['to'], L, e)))
        y0, y1 = g.get('bottomM', 0.05), g.get('topM', 3.0)
        gm = self.tinted(g.get('material', 'painted-steel'), g.get('tint'))
        pitch = g.get('pitchM', 0.3)
        n = max(2, int(round((s1 - s0) / pitch)))
        pitch = (s1 - s0) / n
        off = g.get('offM', 0.06)
        for i in range(n + 1):
            q = at(s0 + i * pitch, off)
            mb.box((q[0], (y0 + y1) / 2, q[1]), (0.03, y1 - y0, 0.03), gm, yaw)
        rise = pitch * g.get('slope', 1.4)
        yy = y0 + 0.15
        while yy + rise < y1 - 0.1:
            for i in range(n):
                xa, xb = s0 + i * pitch, s0 + (i + 1) * pitch
                for (x0, x1) in ((xa, xb), (xb, xa)):
                    _bc_obox(mb, _bc_p3(at(x0, off + 0.02), yy), _bc_p3(at(x1, off + 0.02), yy + rise), n3, 0.018, 0.015, gm)
            yy += rise
        for yy in (y0 + 0.06, y1 - 0.06):
            q = at((s0 + s1) / 2, off)
            mb.box((q[0], yy, q[1]), (s1 - s0, 0.06, 0.06), gm, yaw)
    for r in s.get('roofLadders') or []:
        sv = self.f2s(r['at'], L, e)
        y0, y1 = r.get('fromM', self.levels[-1] + 1.0), r.get('toM', self.roof_level + 1.1)
        wv, off = r.get('widthM', 0.45), r.get('offM', 0.14)
        m = r.get('material', 'painted-steel-light')
        for side in (-1, 1):
            q = at(sv + side * wv / 2, off)
            mb.box((q[0], (y0 + y1) / 2, q[1]), (0.05, y1 - y0, 0.05), m, yaw)
            # gooseneck over the parapet
            _bc_obox(mb, _bc_p3(q, y1), _bc_p3(at(sv + side * wv / 2, -0.35), y1 + 0.25), d3, 0.05, 0.05, m)
            q2 = at(sv + side * wv / 2, off / 2)
            yb = y0 + 0.3
            while yb < y1 - 0.2:
                mb.box((q2[0], yb, q2[1]), (0.04, 0.04, off), m, yaw)
                yb += 1.2
        yy = y0 + 0.15
        while yy < y1:
            q = at(sv, off)
            mb.box((q[0], yy, q[1]), (wv, 0.03, 0.03), m, yaw)
            yy += 0.3
    for f in s.get('flagpoles') or []:
        sv = self.f2s(f['at'], L, e)
        h = f.get('heightM', 3.0)
        base = self.roof_level + f.get('baseM', 0.6)
        lean = f.get('lean', 0.35)
        p0 = _bc_p3(at(sv, f.get('offM', 0.1)), base)
        p1 = _bc_p3(at(sv, f.get('offM', 0.1) + lean * h), base + h * (1 - lean * lean) ** 0.5)
        m = f.get('material', 'painted-white@efeee9')
        _bc_obox(mb, p0, p1, d3, 0.045, 0.045, m)
        _bc_prism(mb, (p1[0], p1[1] + 0.04, p1[2]), 0.05, 0.08, 6, 'painted-white@c9a24a')
        q = at(sv, 0.05)
        mb.box((q[0], base + 0.1, q[1]), (0.12, 0.25, 0.1), 'painted-steel-mid', yaw)
    for v in s.get('wallVents') or []:
        sv = self.f2s(v['at'], L, e)
        wv, hv = v.get('widthM', 0.3), v.get('heightM', 0.22)
        q = at(sv, 0.02)
        mb.box((q[0], v['yM'], q[1]), (wv, hv, 0.04), 'painted-white@d9d3c4', yaw)
        for i in range(4):
            q2 = at(sv, 0.045)
            mb.box((q2[0], v['yM'] - hv / 2 + (i + 0.7) * hv / 5, q2[1]), (wv - 0.05, 0.018, 0.02), 'painted-steel-mid', yaw)
    slots = [w for w in getattr(self, 'window_slots', []) if w.get('kind') == 'punched']
    front = []
    for w in slots:
        # keep only this edge's windows (their inward normal matches)
        if abs(w['nin'][0] + nout[0]) < 1e-3 and abs(w['nin'][1] + nout[1]) < 1e-3 and w['sill'] > self.levels[1] - 0.1:
            front.append(w)
    wc = s.get('windowCurtains')
    if wc:
        tints = wc.get('tints') or ['#d8c7a0']
        every = wc.get('every', 1)
        dg = WALL_T * 0.35
        for i, w in enumerate(front):
            if i % every:
                continue
            m = self.tinted(wc.get('material', 'int-curtain'), tints[i % len(tints)])
            pa, pb = w['a'], w['b']
            ww = math.hypot(pb[0] - pa[0], pb[1] - pa[1])
            h = w['head'] - w['sill']
            cw = ww * wc.get('widthFrac', 0.26) * (1 + 0.4 * ((i * 7) % 3) / 2)
            for side in (0, 1):
                if (i + side) % 3 == 2:
                    continue
                t = (cw / 2) / ww if side == 0 else 1 - (cw / 2) / ww
                cx = pa[0] + (pb[0] - pa[0]) * t
                cz = pa[1] + (pb[1] - pa[1]) * t
                q = (cx + nout[0] * (WALL_T - dg - 0.07), cz + nout[1] * (WALL_T - dg - 0.07))
                mb.box((q[0], w['sill'] + h / 2 + 0.02, q[1]), (cw, h - 0.06, 0.03), m, yaw)
    sf = s.get('sashFrames')
    if sf:
        m = sf.get('material', s.get('frame', 'painted-white'))
        st = sf.get('stileM', 0.1)
        dg = WALL_T * 0.35
        for w in front:
            pa, pb = w['a'], w['b']
            ww = math.hypot(pb[0] - pa[0], pb[1] - pa[1])
            h = w['head'] - w['sill']
            def P2(t, o):
                return (pa[0] + (pb[0] - pa[0]) * t + nout[0] * (WALL_T - dg + o), pa[1] + (pb[1] - pa[1]) * t + nout[1] * (WALL_T - dg + o))
            for t in (st / 2 / ww, 1 - st / 2 / ww):
                q = P2(t, 0.02)
                mb.box((q[0], w['sill'] + h / 2, q[1]), (st, h, 0.05), m, yaw)
            q = P2(0.5, 0.045)
            mb.box((q[0], w['sill'] + h / 2 + 0.05, q[1]), (ww, st * 1.1, 0.06), m, yaw)
            for yy in (w['sill'] + st / 2, w['head'] - st / 2):
                q = P2(0.5, 0.02)
                mb.box((q[0], yy, q[1]), (ww, st, 0.05), m, yaw)
    if not mb.empty():
        mb.build(self.exterior)


Building._fire_escape = _bc_fire_escape


Building.openings_for_edge = _swb_openings


# ---- fireEscapeDrop + shopFascia (bldg-260361488): a single-balcony fire
# escape whose stair drops sideways to a small landing one storey down, and a
# deep sign fascia band over the shopfront. Absent keys = unchanged output.
#   fireEscapeDrop {"at": f, "widthM": m, "depthM": m, "storey": k,
#                   "landingAt": f, "landingW": m, "material": kit}
#     balcony centred at fraction f on storey k's floor line; the stair runs
#     from the balcony toward `landingAt` (viewer's left-to-right fractions)
#     down to a landing on storey k-1. Front edge only; visual, not climbable.
#   shopFascia {"heightM": m, "projectionM": m, "material": kit, "tint": hex,
#               "topM": m below the first upper floor line, "lamps": n,
#               "lampMaterial": kit}
_fed_orig_fire_escape = Building._fire_escape
_fed_orig_edge_trim = Building._edge_trim


def _fed_rail(self, mb, at, c, w, y, depth, mat, yaw):
    for yy in (y + 1.0, y + 0.5):
        q = at(c, depth)
        mb.box((q[0], yy, q[1]), (w, 0.04, 0.04), mat, yaw)
        for side in (-1, 1):
            q2 = at(c + side * w / 2, depth / 2)
            mb.box((q2[0], yy, q2[1]), (0.04, 0.04, depth), mat, yaw)
    n = max(1, int(w / 0.25))
    for i in range(n + 1):
        q = at(c - w / 2 + i * w / n, depth)
        mb.box((q[0], y + 0.5, q[1]), (0.02, 1.0, 0.02), mat, yaw)


def _fed_fire_escape(self, e, a, b):
    _fed_orig_fire_escape(self, e, a, b)
    spec = self.style.get('fireEscapeDrop')
    if not spec or self.storeys < 3:
        return
    k = int(spec.get('storey', self.storeys - 1))
    if k < 2 or k >= self.storeys:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    mat = spec.get('material', 'painted-steel')
    w, depth = spec.get('widthM', 2.2), spec.get('depthM', 0.75)
    lw = spec.get('landingW', 1.2)
    c = self.f2s(spec['at'], L, e)
    lc = self.f2s(spec.get('landingAt', min(0.95, spec['at'] + 0.3)), L, e)
    sgn = 1.0 if lc >= c else -1.0

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    mb = MeshBuilder('fire-escape-drop')
    y_top, y_bot = self.levels[k] - 0.02, self.levels[k - 1] - 0.02
    lad = spec.get('ladder')
    if lad:
        # `ladder` {"at": -1..1 across the balcony, "toM": m above storey
        # k-1's floor, "runM": m of lean along the facade, "widthM": m}: one
        # balcony and a steep drop ladder (continuous stringers, rungs) in a
        # plane parallel to the wall at the deck's outer third; no landing
        p = at(c, depth / 2 + 0.02)
        mb.box((p[0], y_top, p[1]), (w, 0.05, depth), mat, yaw)
        for side in (-1, 1):   # brackets under the deck back to the wall
            q = at(c + side * (w / 2 - 0.1), depth / 2)
            mb.box((q[0], y_top - 0.18, q[1]), (0.05, 0.3, depth), mat, yaw)
        _fed_rail(self, mb, at, c, w, y_top, depth, mat, yaw)
        la = float(lad.get('at', 0.6))
        lx = c + la * (w / 2 - 0.35)
        run = float(lad.get('runM', 0.45)) * (1 if la >= 0 else -1)
        y_end = self.levels[k - 1] + float(lad.get('toM', 0.6))
        lwid = float(lad.get('widthM', 0.45))
        off = depth * 0.7
        n = 40
        for i in range(n):
            t0, t1 = i / n, (i + 1) / n
            ya, yb = y_top - (y_top - y_end) * t0, y_top - (y_top - y_end) * t1
            for sd in (-1, 1):
                q = at(lx + run * (t0 + t1) / 2 + sd * lwid / 2, off)
                mb.box((q[0], (ya + yb) / 2, q[1]), (abs(run) / n + 0.045, ya - yb + 0.02, 0.05), mat, yaw)
        rungs = max(3, int((y_top - y_end) / 0.3))
        for i in range(1, rungs):
            t = i / rungs
            q = at(lx + run * t, off)
            mb.box((q[0], y_top - (y_top - y_end) * t, q[1]), (lwid, 0.035, 0.035), mat, yaw)
        mb.build(self.exterior)
        return
    for cc, ww, yy in ((c, w, y_top), (lc, lw, y_bot)):
        p = at(cc, depth / 2 + 0.02)
        mb.box((p[0], yy, p[1]), (ww, 0.05, depth), mat, yaw)
        _fed_rail(self, mb, at, cc, ww, yy, depth, mat, yaw)
    # stair: from inside the balcony (hatch) down to the landing's near end
    s_top = c + sgn * w * 0.15
    s_bot = lc - sgn * lw / 2
    rise = y_top - y_bot
    steps = 16
    run = (s_bot - s_top) / steps
    for i in range(steps):
        sv = s_top + (i + 0.5) * run
        y = y_top - rise * (i + 1) / (steps + 1)
        q = at(sv, depth * 0.55)
        mb.box((q[0], y, q[1]), (abs(run) + 0.02, 0.03, 0.55), mat, yaw)
        for off in (0.28, 0.82):
            q = at(sv, depth * off)
            mb.box((q[0], y + 0.9, q[1]), (abs(run) + 0.03, 0.035, 0.035), mat, yaw)
    # stringers
    for off in (0.26, 0.84):
        for i in range(steps):
            sv = s_top + (i + 0.5) * run
            q = at(sv, depth * off)
            mb.box((q[0], y_top - rise * (i + 0.5) / (steps + 1) - 0.06, q[1]), (abs(run) + 0.03, 0.12, 0.03), mat, yaw)
    mb.build(self.exterior)


def _fed_edge_trim(self, e, a, b, trim):
    _fed_orig_edge_trim(self, e, a, b, trim)
    sf = self.style.get('shopFascia')
    if not sf or e['role'] != 'front' or self.storeys < 2:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    h = sf.get('heightM', 0.7)
    proj = min(sf.get('projectionM', 0.1), self._projection_cap(d))
    top = self.levels[1] - sf.get('topM', 0.05)
    m = self.tinted(sf.get('material', 'painted-stucco'), sf.get('tint'))
    p = at(L / 2, proj / 2)
    trim.box((p[0], top - h / 2, p[1]), (L, h, proj + 0.02), m, yaw)
    # thin cap and drip mould on the band
    for y, hh, off in ((top + 0.03, 0.06, proj + 0.05), (top - h - 0.03, 0.06, proj + 0.03)):
        q = at(L / 2, off / 2)
        trim.box((q[0], y, q[1]), (L, hh, off + 0.02), m, yaw)
    n = int(sf.get('lamps', 0))
    lm = sf.get('lampMaterial', 'painted-steel')
    for i in range(n):
        sv = L * (i + 1) / (n + 1)
        q = at(sv, proj + 0.12)
        trim.box((q[0], top + 0.12, q[1]), (0.03, 0.03, 0.24), lm, yaw)
        q = at(sv, proj + 0.24)
        trim.box((q[0], top + 0.04, q[1]), (0.16, 0.08, 0.12), lm, yaw)
    _fed_sill_drips(self, e, L, at, yaw, trim)


# ---- sillDrips (bldg-260361488): weathering streaks under every upper
# window sill on the front edge, as thin stacked wall-tint panels that fade
# downward (darker and narrower near the sill). Absent = unchanged.
#   sillDrips {"tint": hex (strongest), "fade": hex (lightest), "lengthM": m,
#              "material": kit, "steps": n, "widthShare": 0..1,
#              "kinds": [punched|ribbon], "startM": m below the sill}
def _fed_sill_drips(self, e, L, at, yaw, trim):
    sd = self.style.get('sillDrips')
    if not sd or e['role'] != 'front':
        return
    res = self.openings_for_edge(e)
    kinds = sd.get('kinds', ['punched', 'ribbon'])
    wins = [o for kind, o in res[1] if kind in kinds]
    st = float(sd.get('startM', 0.12))
    mat = sd.get('material', self.style['wall'])
    c0, c1 = sd.get('tint', '#cbbd98'), sd.get('fade', '#dcd0ae')

    def mix(t):
        a_, b_ = int(c0.lstrip('#'), 16), int(c1.lstrip('#'), 16)
        ch = [round(((a_ >> s) & 255) * (1 - t) + ((b_ >> s) & 255) * t) for s in (16, 8, 0)]
        return '#%02x%02x%02x' % tuple(ch)
    n = max(1, int(sd.get('steps', 3)))
    ln = float(sd.get('lengthM', 0.9))
    for o in wins:
        s0, s1, sill = o[0], o[1], o[2]
        w = (s1 - s0) * float(sd.get('widthShare', 0.85))
        for i in range(n):
            t = i / n
            hh = ln / n
            q = at((s0 + s1) / 2, 0.008 + 0.002 * i)
            trim.box((q[0], sill - st - hh * (i + 0.5), q[1]), (w * (1 - 0.18 * t), hh + 0.01, 0.012),
                     self.tinted(mat, mix(t)), yaw)


Building._fire_escape = _fed_fire_escape
Building._edge_trim = _fed_edge_trim


_b5_orig_export = Building.export


def _b5_export(self, out_dir):
    if self.style.get('upperSetback'):
        _b5_setback_apply(self)
    return _b5_orig_export(self, out_dir)


Building.export = _b5_export


# shopFit: extra fittings in a ground-floor shop (wall shelving runs along
# the side walls, stock boxes, a lamp grid, a second counter)
_b5_orig_ground = Building._ground


def _b5_ground(self, walls):
    _b5_orig_ground(self, walls)
    sf = self.style.get('shopFit')
    if not sf or self.ground_programme() in ('restaurant', 'vacant'):
        return
    x0, x1, z0, z1 = self.rect
    st = self.stair
    top = self.storey_top(0)
    wx = st['x0'] + 1.45 if st['side'] < 0 else st['x1'] - 1.45
    door_x = getattr(self, 'stair_door_x', None)
    if door_x is not None:
        wx = max(wx, door_x + 0.7) if st['side'] < 0 else min(wx, door_x - 0.7)
    kz = z0 + min(6.0, (z1 - z0) * 0.3)
    rx0 = wx + PART_T if st['side'] < 0 else x0
    rx1 = x1 if st['side'] < 0 else wx - PART_T
    far = rx1 - 0.3 if st['side'] < 0 else rx0 + 0.3
    face = -math.pi / 2 if st['side'] < 0 else math.pi / 2
    if sf.get('wallShelving', True):
        for zc in frange(kz + 0.9, z1 - 2.2, 1.3):
            self.piece('shelving', far, zc, face, 0.0)
    if sf.get('boxes', True):
        rng = random.Random(9)
        for zc in frange(kz + 0.9, z1 - 2.2, 2.6):
            self.piece('cardboard-box', far + (-0.75 if st['side'] < 0 else 0.75), zc + 0.6, rng.uniform(-0.4, 0.4), 0.0)
    if sf.get('lamps', True):
        for xc in frange(rx0 + 1.5, rx1 - 1.0, 2.6):
            for zc in frange(kz + 1.5, z1 - 1.0, 3.0):
                self.hang('ceiling-lamp', xc, zc, top, 0.0)


Building._ground = _b5_ground


# ---------------------------------------------------------------------------
# bldg-939458463 block 2. Self-contained, appended; every key
# below is NEW and absent = unchanged for every other building.
#
# `upperSetback[].bandM` (default the wall thickness): how deep behind the face plane the
#   stepped geometry is moved; a deep rooftop box (depthM > 1.2) needs a band
#   deeper than its step so upper-storey fittings move with the face; when any
#   step sets it, the interior collection (slabs, partitions) moves too.
# `awningSpans[].letters` (box profile): {color, heightM, fill, seed, sides}
#   abstract white glyph strokes on the valance front (no real text).
# `awningSpans[].signBox` (box profile): {end: from|to, widthM, heightM,
#   depthM, material} a raised sign box standing on the awning at one end.
# `mural.layout` kind `disc`: {cx, cy, rM, color} a filled circle (centre in
#   mural fractions, radius in metres).
# ---------------------------------------------------------------------------

def _b6_setback_apply(self):
    from mathutils import Matrix
    # Freshly placed pieces (linked furniture duplicates above all) still carry
    # a stale identity matrix_world until the depsgraph updates: without this
    # every one tested its bounding box at the origin, missed the band and was
    # left standing in front of the stepped-back face (259965512).
    bpy.context.view_layer.update()
    cols = [self.exterior] + [c for c in bpy.data.collections if c.name.startswith('fine-s')]
    if any('bandM' in sb for sb in self.style.get('upperSetback') or []):
        cols.append(self.interior)   # slabs, partitions and ceilings step back with the faces
        # bake object transforms so every vertex (not a whole object by its
        # centre) is tested against the band
        for col in cols:
            for obj in col.objects:
                if obj.type == 'MESH' and obj.data.users == 1 and obj.matrix_world != Matrix.Identity(4):
                    obj.data.transform(obj.matrix_world)
                    obj.matrix_world = Matrix.Identity(4)
    cum = {}
    for sb in self.style.get('upperSetback') or []:
        y0, dep, gap = float(sb['fromM']), float(sb['depthM']), 0.02
        cum0 = dict(cum)
        n_e = len(self.poly)
        for ei in sb.get('edges', []):
            e = self.edges[int(ei)]
            i = e['index']
            a, b = self.poly[i], self.poly[(i + 1) % len(self.poly)]
            nout = mul(self.inward(a, b), -1)
            dvec = norm(sub(b, a))
            L = e['lengthM']
            c0 = cum.get(i, 0.0)
            lo, hi = -(c0 + max(WALL_T + 0.1, float(sb.get('bandM', 0.0)))), 1.2 - c0

            def dist(co):
                # Blender (X, Y, Z) -> frame (x = X, z = -Y)
                return (co.x - a[0]) * nout[0] + (-co.y - a[1]) * nout[1]

            def along(co):
                return (co.x - a[0]) * dvec[0] + (-co.y - a[1]) * dvec[1]

            def inband(co):
                return lo <= dist(co) <= hi and -0.8 <= along(co) <= L + 0.8
            mvx, mvy = -nout[0] * dep, nout[1] * dep   # frame move -nout*dep -> Blender (x, -z)
            for col in cols:
                for obj in list(col.objects):
                    if obj.type != 'MESH':
                        continue
                    if obj.matrix_world != Matrix.Identity(4) or obj.data.users > 1:
                        cc = obj.matrix_world @ (sum((Vector(q) for q in obj.bound_box), Vector()) / 8)
                        if cc.z > y0 + gap and inband(cc):
                            obj.location.x += mvx
                            obj.location.y += mvy
                        continue
                    bm = bmesh.new()
                    bm.from_mesh(obj.data)
                    for yc in (y0, y0 + gap):
                        fs = [f for f in bm.faces if min(v.co.z for v in f.verts) < yc - 1e-4 < max(v.co.z for v in f.verts)
                              and any(inband(v.co) for v in f.verts)]
                        if fs:
                            bmesh.ops.bisect_plane(bm, geom=list({ed for f in fs for ed in f.edges} | set(fs) | {v for f in fs for v in f.verts}),
                                                   plane_co=(0, 0, yc), plane_no=(0, 0, 1))
                    moved = set()
                    for v in bm.verts:
                        if v.co.z > y0 + gap / 2 and inband(v.co):
                            v.co.x += mvx
                            v.co.y += mvy
                            moved.add(v)
                    dead = [f for f in bm.faces if all(y0 - 1e-4 <= v.co.z <= y0 + gap + 1e-4 for v in f.verts)
                            and any(v in moved for v in f.verts) and any(v not in moved for v in f.verts)]
                    if dead:
                        bmesh.ops.delete(bm, geom=dead, context='FACES')
                    bm.to_mesh(obj.data)
                    bm.free()
                    obj.data.update()
            cum[i] = c0 + dep
            # dressing on the step
            yaw = box_yaw(dvec)
            at = _b5_at(a, dvec, nout)
            mb = MeshBuilder(f'setback-{i}-{int(y0 * 100)}')
            led = sb.get('ledge')
            if led:
                pr = min(0.4, led.get('projectionM', 0.15))
                hh = led.get('heightM', 0.22)
                p = at(L / 2, ((pr - c0) - (c0 + dep + WALL_T)) / 2)
                mb.box((p[0], y0 + gap - hh / 2 + 0.02, p[1]), (L, hh, pr + dep + WALL_T),
                       self.tinted(led.get('material', 'painted-stucco'), led.get('tint')), yaw)
            # deck and rail stop at a neighbouring face that already stepped back
            ta, tb = cum0.get((i - 1) % n_e, 0.0), cum0.get((i + 1) % n_e, 0.0)
            dk = sb.get('deck')
            if dk:
                p = at((ta + L - tb) / 2, -c0 - (dep + WALL_T) / 2)
                mb.box((p[0], y0 + 0.01 + 0.003 * i, p[1]), (L - ta - tb, 0.06, dep + WALL_T),
                       self.tinted(dk.get('material', 'concrete-precast'), dk.get('tint')), yaw)
            rl = sb.get('railing')
            if rl:
                rm = rl.get('material', 'painted-steel')
                rh = rl.get('heightM', 1.0)
                off = -c0 - 0.12
                ra, rb_ = ta + 0.1, L - tb - 0.1
                for yy in (y0 + rh, y0 + rh / 2):
                    p = at((ra + rb_) / 2, off)
                    mb.box((p[0], yy, p[1]), (rb_ - ra, 0.045, 0.045), rm, yaw)
                cnt = max(1, int((rb_ - ra) / rl.get('postM', 1.6)))
                for k in range(cnt + 1):
                    p = at(ra + (rb_ - ra) * k / cnt, off)
                    mb.box((p[0], y0 + rh / 2, p[1]), (0.045, rh, 0.045), rm, yaw)
            mb.build(self.exterior)
        lid = sb.get('lid')
        if lid:
            # the footprint with the stepped faces offset inward, at the top
            n = len(self.poly)
            lines = []
            for i in range(n):
                a, b = self.poly[i], self.poly[(i + 1) % n]
                nin = self.inward(a, b)
                off = cum.get(i, 0.0) + (WALL_T * 0.5 if i in cum else WALL_T * 0.5)
                lines.append((add(a, mul(nin, off)), norm(sub(b, a))))
            pts = []
            for i in range(n):
                (p1, d1), (p2, d2) = lines[i - 1], lines[i]
                den = d1[0] * d2[1] - d1[1] * d2[0]
                if abs(den) < 1e-9:
                    pts.append(p2)
                    continue
                t = ((p2[0] - p1[0]) * d2[1] - (p2[1] - p1[1]) * d2[0]) / den
                pts.append(add(p1, mul(d1, t)))
            mb = MeshBuilder('rooftop-box-lid')
            ytop = self.height - 0.01
            mb.poly([B(p[0], ytop, p[1]) for p in pts], self.tinted(lid.get('material', 'painted-stucco'), lid.get('tint')), B(0, 1, 0))
            mb.build(self.exterior)


_b5_setback_apply = _b6_setback_apply


def _b6_glyphs(mb, at, s0, s1, off, ya, yb, col, seed, fill, yaw):
    """Abstract glyph strokes across [s0, s1] between ya and yb at offset off."""
    rng = random.Random(seed)
    h = yb - ya
    gw, gap = h * 0.62, h * 0.2
    span = (s1 - s0) * fill
    n = max(1, int(span / (gw + gap)))
    x = s0 + ((s1 - s0) - n * (gw + gap)) / 2
    t = h * 0.16
    for k in range(n):
        if rng.random() < 0.12 and 0 < k < n - 1:
            x += gw + gap
            continue
        strokes = [(0.0, 0.0, t, h)]
        pick = rng.randrange(6)
        if pick in (0, 1, 4):
            strokes.append((0.0, h - t, gw, t))
        if pick in (0, 2, 4):
            strokes.append((0.0, 0.0, gw, t))
        if pick in (1, 2, 3, 5):
            strokes.append((gw - t, 0.0, t, h))
        if pick in (3, 4, 5):
            strokes.append((0.0, h / 2 - t / 2, gw, t))
        for (dx, dy, w, hh) in strokes:
            p = at(x + dx + w / 2, off)
            mb.box((p[0], ya + dy + hh / 2, p[1]), (w, hh, 0.012), col, yaw)
        x += gw + gap


_b6_orig_facade_relief = Building._facade_relief


def _b6_facade_relief(self, e, a, b, s):
    _b6_orig_facade_relief(self, e, a, b, s)
    spans = [sp for sp in (s.get('awningSpans') or []) if sp.get('profile') == 'box' and (sp.get('letters') or sp.get('signBox'))]
    if not spans:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    at = _b5_at(a, d, nout)
    g_head = self.spec['floors']['groundM'] - s['storefront'].get('headDropM', 0.35)
    mb = MeshBuilder(f'b6-awning-dress-{e["index"]}')
    for sp in spans:
        s0, s1 = sorted((self.f2s(sp['from'], L, e), self.f2s(sp['to'], L, e)))
        depth = sp.get('depthM', 1.1)
        top = sp.get('topM', g_head + 0.75)
        vh = sp.get('valanceM', 0.3)
        mat = sp.get('material', s['awning'])
        lt = sp.get('letters')
        if lt:
            col = self.tinted('painted-white', lt.get('color', '#f2f2ee'))
            hh = min(vh * 0.7, lt.get('heightM', vh * 0.5))
            ya = top - vh / 2 - hh / 2
            _b6_glyphs(mb, at, s0 + 0.3, s1 - 0.3, depth + 0.006, ya, ya + hh, col, lt.get('seed', 7), lt.get('fill', 0.8), yaw)
        sb = sp.get('signBox')
        if sb:
            w, h, dp = sb.get('widthM', 1.6), sb.get('heightM', 0.8), sb.get('depthM', 0.4)
            c = s1 - w / 2 if sb.get('end', 'to') == 'to' else s0 + w / 2
            p = at(c, depth - dp / 2)
            mb.box((p[0], top + h / 2, p[1]), (w, h, dp), sb.get('material', mat), yaw)
            if lt:
                col = self.tinted('painted-white', lt.get('color', '#f2f2ee'))
                _b6_glyphs(mb, at, c - w / 2 + 0.12, c + w / 2 - 0.12, depth + 0.006, top + h * 0.28, top + h * 0.72, col, lt.get('seed', 7) + 1, 0.9, yaw)
    mb.build(self.exterior)


Building._facade_relief = _b6_facade_relief


_b6_orig_mural = Building._mural


def _b6_mural(self, e, a, b, spec):
    lay = spec.get('layout') or []
    discs = [sh for sh in lay if sh.get('kind') == 'disc']
    if not discs:
        return _b6_orig_mural(self, e, a, b, spec)
    _b6_orig_mural(self, e, a, b, dict(spec, layout=[sh for sh in lay if sh.get('kind') != 'disc']))
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    f0, f1 = self.f2s(spec.get('from', 0.05), L, e), self.f2s(spec.get('to', 0.95), L, e)
    y0 = spec.get('bottomM', 3.0)
    y1 = min(self.height - 0.4, spec.get('topM', self.height - 1.0))
    mb = MeshBuilder('mural-discs')
    out = B(nout[0], 0, nout[1])
    for i, sh in enumerate(discs):
        sv = f0 + (f1 - f0) * sh['cx']
        cy = y0 + (y1 - y0) * sh['cy']
        r = sh.get('rM', 1.0)
        off = 0.012 + 0.004 * (len(lay) + 1 + i)
        pts = []
        for k in range(32):
            t = 2 * math.pi * k / 32
            p = add(add(a, mul(d, sv + r * math.cos(t))), mul(nout, off))
            pts.append(B(p[0], cy + r * math.sin(t), p[1]))
        mb.poly(pts, f'painted-white@{sh["color"].lstrip("#").lower()}', out)
    mb.build(self.exterior)


Building._mural = _b6_mural




# ---------------------------------------------------------------- _hyA street-oriented stills
# Not a placement change. The Blender model is built in the AUTHORING frame
# (z = north), which is the left/right mirror image of the street; the game
# draws it under authored/reflection.js (scale.z = -1), which restores the
# street. Style fractions ("from the viewer's left") land correctly IN GAME
# (checked: local +x is the street viewer's left there), but the raw eevee
# stills (front.png ...) are viewed inside the authoring frame and so read
# mirrored against a street photo.
#   `streetStills: true` (style): after the normal stills, also write
#   `<name>-street.png` next to each, flipped horizontally, so they read like
#   the photo. Extra files only; no geometry, GLB or spec output changes.
_hyA_orig_render_previews = render_previews


def _hyA_flip_png(src, dst):
    import numpy as np
    img = bpy.data.images.load(src)
    try:
        w, h = img.size
        px = np.array(img.pixels[:], dtype=np.float32).reshape(h, w, img.channels)
        img.pixels[:] = px[:, ::-1, :].reshape(-1).tolist()
        img.filepath_raw = dst
        img.file_format = 'PNG'
        img.save()
    finally:
        bpy.data.images.remove(img)


def _hyA_render_previews(b, out_dir):
    _hyA_orig_render_previews(b, out_dir)
    if not (getattr(b, 'style', None) or {}).get('streetStills'):
        return
    for name in sorted(os.listdir(out_dir)):
        if name.endswith('.png') and not name.endswith('-street.png'):
            _hyA_flip_png(os.path.join(out_dir, name), os.path.join(out_dir, name[:-4] + '-street.png'))


render_previews = _hyA_render_previews




# ---------------------------------------------------------------------------
# _hyB: San Francisco bay windows (swell / cant / box). Appended, self-contained;
# every key is NEW and a style without `bays[].hyB` builds byte-identical.
#
# `bays[]` already carries fromStorey / toStorey (the bay starts at that storey
# over a flat wall below) and roundCorners / glazed / sash / box (other blocks).
# A bay that names `hyB` is pulled out of the list the older blocks see and is
# built here instead; every other bay goes through the previous chain untouched.
#
#   bays[].hyB = {
#     plan:  "swell" (round plan: curved glazing) | "cant" (default, 3-sided) | "box"
#     arc:   swell only: "segmental" (default, rise = projectionM) | "half" (semicircle, rise = widthM/2)
#     panes: swell only, 2..6 (default 3): curved sash panes across the curve
#     facetsPerPane: swell only (default 2): flat facets that approximate one curved pane
#     sashes: {front: 2|3, sides: 1}   cant / box only: double-hung sashes per facet
#     sideRunM: cant only: run along the wall of each angled side face
#     sillM (0.7) headGapM (0.2) meetingFrac (0.5) marginM (0.12 cant front) fillFrac
#     muntins: {upperRows, upperCols, lowerRows, lowerCols}  bars per sash facet
#     sashDepthM: how far the lower sash stands proud of the upper (0.025)
#     cornice: {projectionM (0.12), dentils (bool), roofRiseM (0.22)}  follows the plan
#     corbel:  {tiers (2), stepM (0.15), brackets (count, 0 = none), dropM (0.5),
#               beltBrackets (bool: small brackets under every storey belt)}
#   }
# ---------------------------------------------------------------------------

def _hyB_perp_out(ring):
    cx = sum(q[0] for q in ring) / len(ring)
    cz = sum(q[1] for q in ring) / len(ring)
    fns = []
    for i in range(len(ring) - 1):
        t = norm(sub(ring[i + 1], ring[i]))
        n = (-t[1], t[0])
        if dot(sub(lerp2(ring[i], ring[i + 1], 0.5), (cx, cz)), n) < 0:
            n = (t[1], -t[0])
        fns.append(n)
    return fns


def _hyB_off(ring, dist):
    # the ring pushed out by dist (mitred); its two wall ends slide along the wall
    fns = _hyB_perp_out(ring)
    out = []
    for i, q in enumerate(ring):
        if i == 0:
            out.append(add(q, mul(norm(sub(ring[0], ring[-1])), dist)))
        elif i == len(ring) - 1:
            out.append(add(q, mul(norm(sub(ring[-1], ring[0])), dist)))
        else:
            nn = norm(add(fns[i - 1], fns[i]))
            out.append(add(q, mul(nn, dist / max(0.5, dot(nn, fns[i])))))
    return out


def _hyB_scale(ring, o, f):
    return [add(o, mul(sub(q, o), f)) for q in ring]


def _hyB_sample(ring, count):
    # `count` evenly spaced (point, outward normal, tangent) along the ring
    fns = _hyB_perp_out(ring)
    segs = [length(sub(ring[i + 1], ring[i])) for i in range(len(ring) - 1)]
    tot = sum(segs)
    out = []
    for j in range(count):
        t = tot * (j / (count - 1) if count > 1 else 0.5)
        i = 0
        while i < len(segs) - 1 and t > segs[i]:
            t -= segs[i]
            i += 1
        f = min(1.0, t / max(1e-6, segs[i]))
        out.append((lerp2(ring[i], ring[i + 1], f), fns[i], norm(sub(ring[i + 1], ring[i]))))
    return out


def _hyB_arc(at, c, w, p, panes, per, arc):
    if arc == 'half':
        p = w / 2
    p = max(0.15, min(p, w / 2))
    R = (w * w / 4 + p * p) / (2 * p)
    th = math.atan2(w / 2, R - p)
    n = panes * per
    ring = []
    for i in range(n + 1):
        phi = -th + 2 * th * i / n
        ring.append(at(c + R * math.sin(phi), p - R + R * math.cos(phi)))
    return ring


def _hyB_sash(self, glass, pl, sill, head, cfg, first, last, fmat, gmat):
    # one double-hung sash across the polyline `pl` (one or more facets)
    s = self.style
    meet = float(cfg.get('meetingFrac', 0.5))
    ym = sill + (head - sill) * meet
    mun = cfg.get('muntins') or {}
    ur, uc = int(mun.get('upperRows', 1)), int(mun.get('upperCols', 1))
    lr, lc = int(mun.get('lowerRows', 1)), int(mun.get('lowerCols', 1))
    proud = float(cfg.get('sashDepthM', 0.025))
    fns = _hyB_perp_out(pl)
    for fa, fb, fn in zip(pl[:-1], pl[1:], fns):
        fl = length(sub(fb, fa))
        fd = norm(sub(fb, fa))
        fy = box_yaw(fd)
        gc = add(lerp2(fa, fb, 0.5), mul(fn, -0.06))
        glass.box((gc[0], (sill + head) / 2, gc[1]), (fl, head - sill, 0.02), gmat, fy)
        self._glass_backing(glass, s, 'bay', gc, sill, head, fl, fy, mul(fn, -1))
        up = add(gc, mul(fn, 0.03))
        lo = add(gc, mul(fn, 0.03 + proud))
        glass.box((lo[0], sill + 0.05, lo[1]), (fl, 0.09, 0.06), fmat, fy)
        glass.box((lo[0], ym - 0.02, lo[1]), (fl, 0.06, 0.06), fmat, fy)
        glass.box((up[0], ym + 0.03, up[1]), (fl, 0.06, 0.05), fmat, fy)
        glass.box((up[0], head - 0.045, up[1]), (fl, 0.09, 0.05), fmat, fy)
        for r in range(1, ur):
            yy = ym + (head - ym) * r / ur
            glass.box((up[0], yy, up[1]), (fl, 0.022, 0.035), fmat, fy)
        for r in range(1, lr):
            yy = sill + (ym - sill) * r / lr
            glass.box((lo[0], yy, lo[1]), (fl, 0.022, 0.035), fmat, fy)
        for i in range(1, uc):
            q = add(up, mul(fd, -fl / 2 + fl * i / uc))
            glass.box((q[0], (ym + head) / 2, q[1]), (0.022, head - ym, 0.035), fmat, fy, skip=('top', 'bottom'))
        for i in range(1, lc):
            q = add(lo, mul(fd, -fl / 2 + fl * i / lc))
            glass.box((q[0], (sill + ym) / 2, q[1]), (0.022, ym - sill, 0.035), fmat, fy, skip=('top', 'bottom'))
        # projecting sill and a head casing
        sq = add(gc, mul(fn, 0.11))
        glass.box((sq[0], sill - 0.03, sq[1]), (fl + 0.06, 0.06, 0.16), fmat, fy)
        hq = add(gc, mul(fn, 0.06))
        glass.box((hq[0], head + 0.03, hq[1]), (fl + 0.06, 0.06, 0.1), fmat, fy)
    # stiles at every polyline vertex; the shared end post belongs to the left sash
    for i, q in enumerate(pl):
        if i == len(pl) - 1 and not last:
            continue
        j0, j1 = max(0, i - 1), min(len(pl) - 1, i + 1)
        yaw = box_yaw(norm(sub(pl[j1], pl[j0])))
        nn = fns[min(i, len(fns) - 1)] if i in (0, len(pl) - 1) else norm(add(fns[i - 1], fns[i]))
        qq = add(q, mul(nn, -0.02))
        wd = 0.09 if (i == 0 or i == len(pl) - 1) else 0.075
        glass.box((qq[0], (sill + head) / 2, qq[1]), (wd, head - sill, 0.08), fmat, yaw, skip=('top', 'bottom'))


def _hyB_bay(self, spec, e, a, b, mb, glass):
    s = self.style
    cfg = spec['hyB']
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    c, w, p = self.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.6)
    mat = self.tinted(spec.get('material', 'painted-stucco'), spec.get('tint'))
    fmat = self.tinted(spec.get('frame', s['frame']), spec.get('frameTint'))
    gmat = spec.get('glass', s['glass'])
    k0 = spec.get('fromStorey', 1)
    k1 = min(self.storeys, spec.get('toStorey', self.storeys - 1) + 1)
    if k0 >= k1:
        return
    drop = float((s.get('hbBayCrown') or {}).get('dropM', 0) or 0) + float(s.get('bayRoofDropM', 0) or 0)
    y_bottom = self.levels[k0]
    y_top = self.levels[k1] - 0.3 if k1 < self.storeys else self.roof_level - drop

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    plan = cfg.get('plan', 'cant')
    sh = cfg.get('sashes') or {}
    runs = []  # (polyline, first, last) per sash
    if plan == 'swell':
        panes = max(2, min(6, int(cfg.get('panes', 3))))
        per = max(1, int(cfg.get('facetsPerPane', 2)))
        ring = _hyB_arc(at, c, w, p, panes, per, cfg.get('arc', 'segmental'))
        for i in range(panes):
            runs.append((ring[i * per:(i + 1) * per + 1], i == 0, i == panes - 1, i))
    else:
        if plan == 'box':
            ring = [at(c - w / 2, 0), at(c - w / 2, p), at(c + w / 2, p), at(c + w / 2, 0)]
        else:
            q = min(float(cfg.get('sideRunM') or p), w / 2 - 0.2)
            ring = [at(c - w / 2, 0), at(c - w / 2 + q, p), at(c + w / 2 - q, p), at(c + w / 2, 0)]
        for fi in range(3):
            fa, fb = ring[fi], ring[fi + 1]
            fl = length(sub(fb, fa))
            fd = norm(sub(fb, fa))
            n = max(1, int(sh.get('front', 3) if fi == 1 else sh.get('sides', 1)))
            m = float(cfg.get('marginM', 0.12)) if fi == 1 else 0.07
            if fl - 2 * m < 0.35 * n:
                m = 0.0
            ga, gb = add(fa, mul(fd, m)), add(fb, mul(fd, -m))
            for j in range(n):
                runs.append(([lerp2(ga, gb, j / n), lerp2(ga, gb, (j + 1) / n)], j == 0, j == n - 1, fi))
    centre_o = at(c, 0)
    # base: full-width slab, stepped corbel tiers and per-bay brackets
    cb = cfg.get('corbel') or {}
    tiers = int(cb.get('tiers', 2))
    step = float(cb.get('stepM', 0.15))
    self._prism(mb, _hyB_off(ring, 0.05), y_bottom - 0.1, y_bottom + 0.05, mat)
    yb = y_bottom - 0.1
    for t in range(1, tiers + 1):
        f = 1.0 - 0.5 * t / (tiers + 1.0)
        self._prism(mb, _hyB_scale(ring, centre_o, f), yb - step, yb, mat)
        yb -= step
    nbr = int(cb.get('brackets', 0))
    if nbr > 0:
        drop_m = float(cb.get('dropM', 0.5))
        y_hi = y_bottom - 0.1
        for pt, nn, tt in _hyB_sample(ring, nbr):
            yaw = box_yaw(tt)
            for j, (hh, dep) in enumerate(((0.22, 0.26), (0.16, 0.19), (0.12, 0.12))):
                hh = hh * drop_m / 0.5
                q = add(pt, mul(nn, -dep / 2 + 0.04))
                mb.box((q[0], y_hi - sum((0.22, 0.16, 0.12)[:j]) * drop_m / 0.5 - hh / 2, q[1]), (0.1, hh, dep), mat, yaw)
            q = add(pt, mul(nn, 0.05))
            mb.box((q[0], y_hi - drop_m * 0.94 - 0.02, q[1]), (0.12, 0.05, 0.06), mat, yaw)
    # belts at every inner storey line (they follow the plan), optional small brackets
    belt = _hyB_off(ring, 0.05)
    for k in range(k0 + 1, k1):
        self._prism(mb, belt, self.levels[k] - 0.12, self.levels[k] + 0.12, mat)
        if cb.get('beltBrackets'):
            cnt = max(3, int(round(sum(length(sub(ring[i + 1], ring[i])) for i in range(len(ring) - 1)) / 0.4)))
            for pt, nn, tt in _hyB_sample(ring, cnt)[1:-1]:
                q = add(pt, mul(nn, 0.055))
                mb.box((q[0], self.levels[k] - 0.18, q[1]), (0.07, 0.12, 0.09), mat, box_yaw(tt))
    # cornice + low roof, following the plan
    co = cfg.get('cornice') or {}
    cp = float(co.get('projectionM', 0.12))
    self._prism(mb, _hyB_off(ring, 0.03), y_top - 0.3, y_top - 0.06, mat)
    self._prism(mb, _hyB_off(ring, cp), y_top - 0.06, y_top + 0.06, mat)
    if co.get('dentils'):
        cnt = max(4, int(round(sum(length(sub(ring[i + 1], ring[i])) for i in range(len(ring) - 1)) / 0.14)))
        for pt, nn, tt in _hyB_sample(ring, cnt)[1:-1]:
            q = add(pt, mul(nn, 0.06))
            mb.box((q[0], y_top - 0.12, q[1]), (0.07, 0.09, 0.08), mat, box_yaw(tt))
    slab = _hyB_off(ring, cp + 0.04)
    self._prism(mb, slab, y_top + 0.06, y_top + 0.16, mat)
    rise = float(co.get('roofRiseM', 0.22))
    if rise > 0:
        top = _hyB_scale(slab, centre_o, 0.3)
        up = B(0, 1, 0)
        sfn = _hyB_perp_out(slab)
        for i in range(len(slab) - 1):
            mb.quad([B(slab[i][0], y_top + 0.16, slab[i][1]), B(slab[i + 1][0], y_top + 0.16, slab[i + 1][1]),
                     B(top[i + 1][0], y_top + 0.16 + rise, top[i + 1][1]), B(top[i][0], y_top + 0.16 + rise, top[i][1])],
                    mat, up + B(sfn[i][0], 0, sfn[i][1]))
        mb.poly([B(q[0], y_top + 0.16 + rise, q[1]) for q in top], mat, up)
    # storeys: apron and head panels per facet, piers, then the sashes
    sill_h, gap = float(cfg.get('sillM', 0.7)), float(cfg.get('headGapM', 0.2))
    for k in range(k0, k1):
        y0 = self.levels[k] + (0.05 if k == k0 else 0.12)
        y1 = (self.levels[k + 1] - 0.12) if k + 1 < k1 else (y_top - 0.3)
        sill, head = self.levels[k] + sill_h, y1 - gap
        if head - sill < 0.7:
            continue
        for fa, fb, fn in zip(ring[:-1], ring[1:], _hyB_perp_out(ring)):
            ob = B(fn[0], 0, fn[1])
            mb.quad([B(fa[0], y0, fa[1]), B(fb[0], y0, fb[1]), B(fb[0], sill, fb[1]), B(fa[0], sill, fa[1])], mat, ob)
            mb.quad([B(fa[0], head, fa[1]), B(fb[0], head, fb[1]), B(fb[0], y1, fb[1]), B(fa[0], y1, fa[1])], mat, ob)
        # piers between the facet edge and the first / last sash of a cant or box facet
        if plan != 'swell':
            fns = _hyB_perp_out(ring)
            for fi in range(3):
                fa, fb = ring[fi], ring[fi + 1]
                fr = [r for r in runs if r[3] == fi]
                if not fr:
                    continue
                ga, gb = fr[0][0][0], fr[-1][0][-1]
                ob = B(fns[fi][0], 0, fns[fi][1])
                for p0, p1 in ((fa, ga), (gb, fb)):
                    if length(sub(p1, p0)) > 0.01:
                        mb.quad([B(p0[0], sill, p0[1]), B(p1[0], sill, p1[1]), B(p1[0], head, p1[1]), B(p0[0], head, p0[1])], mat, ob)
        for pl, first, last, _fi in runs:
            _hyB_sash(self, glass, pl, sill, head, cfg, first, last, fmat, gmat)


_hyB_orig_bays = Building._bays


def _hyB_bays(self, e, a, b):
    specs = self.style.get('bays') or []
    mine = [q for q in specs if q.get('hyB')]
    if not mine:
        return _hyB_orig_bays(self, e, a, b)
    saved = self.style
    self.style = dict(saved, bays=[q for q in specs if not q.get('hyB')])
    try:
        _hyB_orig_bays(self, e, a, b)
    finally:
        self.style = saved
    mb = MeshBuilder('bay-windows-hyb')
    glass = MeshBuilder('bay-glazing-hyb')
    for spec in mine:
        _hyB_bay(self, spec, e, a, b, mb, glass)
    mb.build(self.exterior)
    glass.build(self.exterior)


Building._bays = _hyB_bays




# ---------------------------------------------------------------- arched openings (_hyC_)
# Appended, self-contained. NEW style keys only; absent = unchanged builds.
#   upperWindowArch: {type, riseM, cornerM, surround, keystone, fan, spandrel,
#                     reveal, storeys: [k, ...]}   (style, or a streetEdges override)
#       arched heads on the upper punched / ribbon windows (all upper storeys, or
#       only the listed storey indices).
#   storefront.layout[].arch: {..same.., headM, porch}
#       the same on one ground-floor layout item (window, door, double-door,
#       garage, blank, glazed-garage, gate): entries, garages, shop surrounds.
#   arch fields
#     type      "round" (semicircle; riseM below half the width makes it a
#               circular segment) | "segmental" (default rise 0.2 x width) |
#               "basket" (flat top, quarter-ellipse corners: riseM = corner
#               height, cornerM = corner width)
#     riseM     arch rise; the crown is the opening head, so the opening is not
#               made taller. Capped to leave 0.5 m of straight jamb.
#     surround  true | metres | {widthM, projectionM, material, tint, jambs}: a
#               projecting archivolt band following the arch, run down the jambs
#     keystone  true | {widthM, upM, downM, material, tint}: a block at the crown
#     fan       true | {rays, rings, barM, frameM, material, tint}: an arched
#               glazing frame, a spring bar, radial ribs from the spring centre
#               and concentric rings (a fanlight)
#     spandrel  {material, tint}: the filler in the head corners (default: wall)
#     reveal    {material, tint}: the arch soffit lining (default: spandrel)
#     revealM   soffit depth (default 0.16)
#     porch     true | {steps, riseM, treadM, material, tint}: a landing across
#               the doorway and a stair of `steps` risers out to the pavement
# The wall itself is still cut as a rectangle; the arch is closed with spandrel
# plates on the outer face, so the corners read filled from outside.

_hyC_orig_openings = Building.openings_for_edge


def _hyC_openings(self, e):
    res = _hyC_orig_openings(self, e)
    if self.edge_style(e).get('upperWindowArch'):
        if not hasattr(self, '_hyC_win'):
            self._hyC_win = {}
        self._hyC_win[e['index']] = res[1]
    return res


Building.openings_for_edge = _hyC_openings


def _hyC_kind(t):
    t = str(t or 'round').lower()
    if t in ('basket', 'basket-handle', 'basket-handled', 'flat'):
        return 'basket'
    if t in ('segmental', 'segment'):
        return 'segmental'
    return 'round'


def _hyC_rise(kind, arch, w, avail):
    h = w / 2
    r = arch.get('riseM')
    if r is None:
        r = min(0.45, w * 0.18) if kind == 'basket' else (w * 0.2 if kind == 'segmental' else h)
    r = min(float(r), avail)
    if kind != 'basket':
        r = min(r, h)
    return max(0.08, r)


def _hyC_profile(kind, s0, s1, y1, rise, corner, n=18):
    """Head outline from the left spring to the right spring, (s, y) pairs."""
    w = s1 - s0
    cs = (s0 + s1) / 2
    spring = y1 - rise
    pts = []
    if kind == 'basket':
        rx = max(0.05, min(corner, w * 0.45))
        m = max(4, n // 2)
        for i in range(m + 1):
            t = math.pi / 2 * i / m
            pts.append((s0 + rx - rx * math.cos(t), spring + rise * math.sin(t)))
        for i in range(m, -1, -1):
            t = math.pi / 2 * i / m
            pts.append((s1 - rx + rx * math.cos(t), spring + rise * math.sin(t)))
        return pts
    h = w / 2
    R = (h * h + rise * rise) / (2 * rise)
    yc = y1 - R
    th0 = math.asin(min(1.0, h / R))
    for i in range(n + 1):
        t = -th0 + 2 * th0 * i / n
        pts.append((cs + R * math.sin(t), yc + R * math.cos(t)))
    pts[0] = (s0, spring)
    pts[-1] = (s1, spring)
    return pts


def _hyC_mat(bld, spec, default):
    if isinstance(spec, dict) and (spec.get('material') or spec.get('tint')):
        return bld.tinted(spec.get('material') or 'painted-white', spec.get('tint'))
    return default


def _hyC_arch(self, mb, a, d, nout, yaw, es, arch, s0, s1, y0, y1, frame_mat):
    w = s1 - s0
    if w < 0.5 or y1 - y0 < 1.0:
        return
    kind = _hyC_kind(arch.get('type'))
    rise = _hyC_rise(kind, arch, w, (y1 - y0) - 0.5)
    spring = y1 - rise
    cs = (s0 + s1) / 2
    prof = _hyC_profile(kind, s0, s1, y1, rise, float(arch.get('cornerM', rise * 2.0)))
    out = B(nout[0], 0, nout[1])
    depth_glass = min(WALL_T - 0.03, es.get('windowDepthM', WALL_T * 0.35))

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    def P(sv, y, off):
        q = at(sv, off)
        return B(q[0], y, q[1])

    def dv(ns, ny):
        return B(d[0] * ns, ny, d[1] * ns)

    def nrm(i):
        i0, i1 = max(0, i - 1), min(len(prof) - 1, i + 1)
        tx, ty = prof[i1][0] - prof[i0][0], prof[i1][1] - prof[i0][1]
        ln = math.hypot(tx, ty) or 1.0
        return (-ty / ln, tx / ln)

    wall_d = self.wall_material_at(spring, y1) if es is self.style else self.tinted(es['wall'], es.get('wallTint'))
    smat = _hyC_mat(self, arch.get('spandrel'), wall_d)
    so = 0.02
    # spandrels: the head corners between the rectangle and the arch
    for (sa, ya), (sb, yb) in zip(prof[:-1], prof[1:]):
        if abs(sb - sa) < 1e-6 or (y1 - ya < 1e-4 and y1 - yb < 1e-4):
            continue
        if y1 - yb < 1e-4:
            pts = [P(sa, ya, so), P(sb, yb, so), P(sa, y1, so)]
        elif y1 - ya < 1e-4:
            pts = [P(sa, ya, so), P(sb, yb, so), P(sb, y1, so)]
        else:
            pts = [P(sa, ya, so), P(sb, yb, so), P(sb, y1, so), P(sa, y1, so)]
        mb.poly(pts, smat, out)
    # soffit lining along the arch
    rev = min(float(arch.get('revealM', 0.16)), WALL_T - 0.05)
    rmat = _hyC_mat(self, arch.get('reveal'), smat)
    for i, ((sa, ya), (sb, yb)) in enumerate(zip(prof[:-1], prof[1:])):
        n0, n1 = nrm(i), nrm(i + 1)
        hint = dv(-(n0[0] + n1[0]), -(n0[1] + n1[1]))
        mb.poly([P(sa, ya, so), P(sb, yb, so), P(sb, yb, -rev), P(sa, ya, -rev)], rmat, hint)
    # projecting archivolt / surround
    sur = arch.get('surround')
    sw, pj = 0.0, 0.0
    if sur:
        if sur is True:
            sur = {}
        elif not isinstance(sur, dict):
            sur = {'widthM': float(sur)}
        sw = float(sur.get('widthM', 0.15))
        pj = float(sur.get('projectionM', 0.05))
        mat_s = _hyC_mat(self, sur, es['trim'])
        ext = []
        for i, (sv, yy) in enumerate(prof):
            nx, ny = nrm(i)
            ext.append((sv + nx * sw, yy + ny * sw))
        for i in range(len(prof) - 1):
            (sa, ya), (sb, yb) = prof[i], prof[i + 1]
            (ea, fa), (eb, fb) = ext[i], ext[i + 1]
            mb.poly([P(sa, ya, pj), P(sb, yb, pj), P(eb, fb, pj), P(ea, fa, pj)], mat_s, out)
            n0, n1 = nrm(i), nrm(i + 1)
            mb.poly([P(ea, fa, so), P(eb, fb, so), P(eb, fb, pj), P(ea, fa, pj)], mat_s,
                    dv(n0[0] + n1[0], n0[1] + n1[1]))
            mb.poly([P(sa, ya, so), P(sb, yb, so), P(sb, yb, pj), P(sa, ya, pj)], mat_s,
                    dv(-(n0[0] + n1[0]), -(n0[1] + n1[1])))
        if sur.get('jambs', True):
            for sgn, k in ((-1, 0), (1, len(prof) - 1)):
                ex, ey = ext[k]
                sj = prof[k][0]
                mb.poly([P(sj, y0, pj), P(ex, y0, pj), P(ex, ey, pj), P(sj, spring, pj)], mat_s, out)
                mb.poly([P(ex, y0, so), P(ex, ey, so), P(ex, ey, pj), P(ex, y0, pj)], mat_s, dv(sgn, 0))
    # keystone
    ks = arch.get('keystone')
    if ks:
        if ks is True:
            ks = {}
        kw = float(ks.get('widthM', min(0.28, max(0.14, w * 0.08))))
        top = y1 + sw + float(ks.get('upM', 0.06))
        bot = y1 - float(ks.get('downM', 0.14))
        dep = max(pj, 0.02) + 0.05
        kmat = _hyC_mat(self, ks, _hyC_mat(self, sur if isinstance(sur, dict) else None, es['trim']))
        q = at(cs, dep / 2)
        mb.box((q[0], (top + bot) / 2, q[1]), (kw, top - bot, dep), kmat, yaw)
    # arched glazing: frame ring, spring bar, radial ribs, concentric rings
    fan = arch.get('fan')
    if fan:
        if fan is True:
            fan = {}
        fmat = _hyC_mat(self, fan, frame_mat)
        bar = float(fan.get('barM', 0.035))
        fw = float(fan.get('frameM', 0.06))
        goff = -depth_glass + 0.03

        def strip(p0, p1, width):
            ux, uy = p1[0] - p0[0], p1[1] - p0[1]
            ln = math.hypot(ux, uy)
            if ln < 1e-4:
                return
            px, py = -uy / ln * width / 2, ux / ln * width / 2
            mb.poly([P(p0[0] + px, p0[1] + py, goff), P(p1[0] + px, p1[1] + py, goff),
                     P(p1[0] - px, p1[1] - py, goff), P(p0[0] - px, p0[1] - py, goff)], fmat, out)
        for i in range(len(prof) - 1):
            n0, n1 = nrm(i), nrm(i + 1)
            (sa, ya), (sb, yb) = prof[i], prof[i + 1]
            mb.poly([P(sa, ya, goff), P(sb, yb, goff), P(sb - n1[0] * fw, yb - n1[1] * fw, goff),
                     P(sa - n0[0] * fw, ya - n0[1] * fw, goff)], fmat, out)
        strip((s0, spring), (s1, spring), bar * 1.4)
        rays = int(fan.get('rays', 7))
        c = (cs, spring)
        for j in range(1, rays + 1):
            tip = prof[int(round(j / (rays + 1.0) * (len(prof) - 1)))]
            strip(c, (tip[0], tip[1]), bar)
        rings = int(fan.get('rings', 1))
        for r in range(1, rings + 1):
            f = r / (rings + 1.0)
            pp = [(c[0] + (p[0] - c[0]) * f, c[1] + (p[1] - c[1]) * f) for p in prof]
            for p0, p1 in zip(pp[:-1], pp[1:]):
                strip(p0, p1, bar)
    # recessed porch: landing across the doorway and a stair out to the pavement
    pc = arch.get('porch')
    if pc:
        if pc is True:
            pc = {}
        n = max(1, int(pc.get('steps', 2)))
        rs = float(pc.get('riseM', 0.13))
        tr = float(pc.get('treadM', 0.28))
        pmat = _hyC_mat(self, pc, es['trim'])
        q = at(cs, -0.05)
        mb.box((q[0], y0 + n * rs / 2, q[1]), (w - 0.04, n * rs, 0.14), pmat, yaw)
        for j in range(n):
            top = y0 + (n - 1 - j) * rs
            if top - y0 < 1e-4:
                top = y0 + 0.03
            q = at(cs, so + tr * (j + 0.5))
            mb.box((q[0], (y0 + top) / 2, q[1]), (w + 0.2, top - y0, tr), pmat, yaw)


_hyC_orig_relief = Building._facade_relief


def _hyC_facade_relief(self, e, a, b, s):
    _hyC_orig_relief(self, e, a, b, s)
    up = s.get('upperWindowArch')
    sf = s.get('storefront') or {}
    lay = [it for it in (sf.get('layout') or []) if isinstance(it, dict) and it.get('arch')]
    if not up and not lay:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    mb = MeshBuilder(f'hyC-arches-{e["index"]}')
    if up:
        only = up.get('storeys')
        fm = s.get('sashMaterial', s['frame'])
        for kind, sp in (getattr(self, '_hyC_win', {}).get(e['index']) or []):
            if kind not in ('punched', 'ribbon'):
                continue
            k = max(i for i in range(len(self.levels)) if self.levels[i] <= sp[2] + 0.01)
            if only is not None and k not in only:
                continue
            _hyC_arch(self, mb, a, d, nout, yaw, s, up, sp[0], sp[1], sp[2], sp[3], fm)
    if lay:
        head = self.spec['floors']['groundM'] - sf.get('headDropM', 0.35)
        for it in lay:
            s0, s1 = sorted((self.f2s(it['to'], L, e), self.f2s(it['from'], L, e)))
            ih = min(head, it['headM']) if it.get('headM') else head
            if it['arch'].get('headM'):
                ih = float(it['arch']['headM'])
            _hyC_arch(self, mb, a, d, nout, yaw, s, it['arch'], s0, s1, 0.0, ih, s['storefrontFrame'])
    if not mb.empty():
        mb.build(self.exterior)


Building._facade_relief = _hyC_facade_relief




# ---------------------------------------------------------------------------
# _hyD_: wall-cladding relief (lap siding, wood shingle, rubble stone base).
# Self-contained, appended; every key below is NEW and absent = unchanged.
# Geometry-light: thin wedge ledges and joint quads merged into ONE mesh per
# street face, drawn with the wall's own authored-kit material (darker tint
# for shadow lines). No new materials, no textures. Street faces only (this
# runs from _facade_relief, which is never called for party or rear walls);
# openings, rotunda cuts and projecting bays are left clear.
#   hyDLap     {pitchM 0.15, fromM, toM, projM 0.018, heightM 0.045,
#               shade 0.62, marginM 0.12, maxTris 14000}
#              horizontal lap/clapboard: a sloped nose + dark underside every
#              pitchM. fromM defaults to the ground band top (or 0.3 m), toM
#              to the wall head less 0.7 m.
#   hyDShingle {kind "square"|"fish", rowM 0.16, shingleM 0.14, fromM, toM,
#               projM 0.014, shade 0.55, marginM 0.12, maxTris 16000}
#              shingle courses: a butt-line ledge per row plus staggered dark
#              joints (square) or scallop chevrons (fish).
#   hyDRubble  {toM, courseM 0.30, blockM 0.6, projM 0.03, shade 0.55,
#               marginM 0.08, maxTris 6000, seed 3}
#              rough rubble/rusticated base: irregular courses (ledge line)
#              and staggered block joints on the base storey (default toM =
#              ground band top, else the first level). Material = groundBand
#              material/tint, else the wall.
# ---------------------------------------------------------------------------
_hyD_orig_facade_relief = Building._facade_relief


def _hyD_shade(tint, k):
    t = (tint or '#a9a292').lstrip('#')
    if len(t) != 6:
        t = 'a9a292'
    c = [max(0, min(255, int(int(t[i:i + 2], 16) * k))) for i in (0, 2, 4)]
    return '#%02x%02x%02x' % tuple(c)


def _hyD_hash(*v):
    h = 2166136261
    for x in v:
        h = ((h ^ (int(x) & 0xffffffff)) * 16777619) & 0xffffffff
    return (h ^ (h >> 15)) / 4294967295.0


def _hyD_free(ops, L, y0, y1, margin, vmargin, m0=0.06):
    """Free (s0, s1) intervals along the face for the height band y0..y1."""
    cut = sorted((o[0] - margin, o[1] + margin) for o in ops if o[3] + vmargin > y0 and o[2] - vmargin < y1)
    out, cur = [], m0
    for c0, c1 in cut:
        if c0 > cur:
            out.append((cur, min(c0, L - m0)))
        cur = max(cur, c1)
    if cur < L - m0:
        out.append((cur, L - m0))
    return [(a, b) for a, b in out if b - a > 0.08]


def _hyD_facade_relief(self, e, a, b, s):
    _hyD_orig_facade_relief(self, e, a, b, s)
    lap, shg, rub = s.get('hyDLap'), s.get('hyDShingle'), s.get('hyDRubble')
    if not (lap or shg or rub):
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    cap = self._projection_cap(d)
    ops = list(self.openings_for_edge(e)[0])
    cut = self._rotunda_cut(e)
    if cut:
        ops.append((cut[0], cut[1], 0.0, cut[2]))
    n3 = B(nout[0], 0, nout[1])
    up = B(0, 1, 0)

    def P(sv, y, off):
        p = add(add(a, mul(d, sv)), mul(nout, off))
        return B(p[0], y, p[1])

    def ledge(mb, s0, s1, y, proj, h, m_nose, m_under):
        # nose slopes back to the wall from (y, proj) up to (y + h, 0); the
        # underside is the dark shadow line beneath the board
        mb.quad([P(s0, y, 0.0), P(s1, y, 0.0), P(s1, y, proj), P(s0, y, proj)], m_under, B(0, -1, 0))
        mb.quad([P(s0, y, proj), P(s1, y, proj), P(s1, y + h, 0.0), P(s0, y + h, 0.0)], m_nose, n3 + up)

    def joint(mb, sv, y0, y1, w, off, m):
        mb.quad([P(sv - w / 2, y0, off), P(sv + w / 2, y0, off), P(sv + w / 2, y1, off), P(sv - w / 2, y1, off)], m, n3)

    def strip(mb, p0, p1, w, off, m):
        dy, ds = p1[1] - p0[1], p1[0] - p0[0]
        ln = math.hypot(ds, dy) or 1.0
        qs, qy = -dy / ln * w / 2, ds / ln * w / 2
        mb.quad([P(p0[0] - qs, p0[1] - qy, off), P(p1[0] - qs, p1[1] - qy, off),
                 P(p1[0] + qs, p1[1] + qy, off), P(p0[0] + qs, p0[1] + qy, off)], m, n3)

    def mats(name, tint, shade):
        return (self.tinted(name, tint), self.tinted(name, _hyD_shade(tint, shade)))

    def fit(tag, gen, k, max_tris):
        # coarsen the pitch until the face fits its triangle allowance
        for _ in range(6):
            mb = MeshBuilder(f'hyD-{tag}-{e["index"]}')
            gen(mb, k)
            if len(mb.faces) * 2 <= max_tris:
                break
            k *= 1.25
        mb.build(self.exterior)

    seed = int(s.get('seed', 5)) * 131 + e['index']
    wall_name = s.get('wall', self.style['wall'])
    wall_tint = s.get('wallTint') or self.style.get('wallTint')

    if lap:
        y0 = lap.get('fromM', self.band_top or 0.3)
        y1 = lap.get('toM', self.height - 0.7)
        proj = max(0.006, min(lap.get('projM', 0.018), cap))
        m_nose, m_under = mats(lap.get('material', wall_name), lap.get('tint', wall_tint), lap.get('shade', 0.62))

        def gen_lap(mb, pitch):
            h = min(lap.get('heightM', 0.045), pitch * 0.6)
            y = y0 + pitch
            while y < y1:
                for f0, f1 in _hyD_free(ops, L, y - 0.02, y + h, lap.get('marginM', 0.12), 0.1):
                    ledge(mb, f0, f1, y, proj, h, m_nose, m_under)
                y += pitch
        fit('lap', gen_lap, float(lap.get('pitchM', 0.15)), lap.get('maxTris', 14000))
    if shg:
        y0 = shg.get('fromM', self.band_top or 0.3)
        y1 = shg.get('toM', self.height - 0.7)
        proj = max(0.006, min(shg.get('projM', 0.014), cap))
        fish = shg.get('kind', 'square') == 'fish'
        m_nose, m_dark = mats(shg.get('material', wall_name), shg.get('tint', wall_tint), shg.get('shade', 0.55))
        wr = float(shg.get('shingleM', 0.14)) / float(shg.get('rowM', 0.16))

        def gen_shg(mb, row):
            wsh = row * wr
            r, y = 0, y0 + row
            while y < y1:
                for f0, f1 in _hyD_free(ops, L, y - row, y + 0.03, shg.get('marginM', 0.12), 0.1):
                    ledge(mb, f0, f1, y, proj, min(0.03, row * 0.3), m_nose, m_dark)
                    k = 0
                    x = f0 + (0.5 if r % 2 else 0.0) * wsh + wsh * (0.6 + 0.3 * _hyD_hash(seed, r, k))
                    while x < f1 - 0.03:
                        if fish:
                            hw = wsh / 2
                            strip(mb, (x - hw, y - row * 0.55), (x, y - row * 0.95), 0.012, 0.007, m_dark)
                            strip(mb, (x, y - row * 0.95), (x + hw, y - row * 0.55), 0.012, 0.007, m_dark)
                        else:
                            joint(mb, x, y - row + 0.03, y - 0.005, 0.006, 0.007, m_dark)
                        k += 1
                        x += wsh * (0.85 + 0.3 * _hyD_hash(seed, r, k))
                y += row
                r += 1
        fit('shingle', gen_shg, float(shg.get('rowM', 0.16)), shg.get('maxTris', 16000))
    if rub:
        bnd = self.style.get('groundBand') or {}
        top = rub.get('toM') or self.band_top or (self.levels[1] if self.storeys > 1 else 3.0)
        proj = max(0.006, min(rub.get('projM', 0.03), cap))
        m_nose, m_dark = mats(rub.get('material', bnd.get('material', wall_name)), rub.get('tint', bnd.get('tint', wall_tint)),
                              rub.get('shade', 0.55))
        rseed = int(rub.get('seed', 3)) * 977 + e['index']
        ratio = float(rub.get('blockM', 0.6)) / float(rub.get('courseM', 0.30))
        mg = rub.get('marginM', 0.08)

        def gen_rub(mb, ch):
            bw = ch * ratio
            y, r = 0.0, 0
            while y < top - 0.05:
                c = min(ch * (0.75 + 0.55 * _hyD_hash(rseed, r)), top - y)
                y1c = y + c
                if y > 0.1:
                    for f0, f1 in _hyD_free(ops, L, y - 0.03, y + 0.05, mg, 0.05):
                        ledge(mb, f0, f1, y, proj, min(0.04, c * 0.35), m_nose, m_dark)
                for f0, f1 in _hyD_free(ops, L, y - 0.02, y1c + 0.02, mg, 0.05):
                    k = 0
                    x = f0 + bw * (0.3 + 0.7 * _hyD_hash(rseed, r, 99))
                    while x < f1 - 0.15:
                        joint(mb, x, y + 0.04, y1c - 0.005, 0.02, 0.008, m_dark)
                        k += 1
                        x += bw * (0.6 + 0.9 * _hyD_hash(rseed, r, k))
                y = y1c
                r += 1
        fit('rubble', gen_rub, float(rub.get('courseM', 0.30)), rub.get('maxTris', 6000))


Building._facade_relief = _hyD_facade_relief


def main():
    args = parse_args()
    with open(args.spec) as fh:
        spec = json.load(fh)
    style = dict(DEFAULT_STYLE)
    if args.style and os.path.exists(args.style):
        with open(args.style) as fh:
            user = json.load(fh)
        for k, v in user.items():
            if isinstance(v, dict) and isinstance(style.get(k), dict):
                style[k] = {**style[k], **v}
            else:
                style[k] = v
    apply_glass_model(style)
    if args.library and os.path.exists(args.library):
        # Placement needs only each piece's size (library.json, tracked); the
        # binaries are referenced, so a clean clone builds the same GLBs.
        with open(args.library) as fh:
            lib = json.load(fh)
        LIBRARY['dir'] = args.furniture_lib
        LIBRARY['pieces'] = dict(lib.get('pieces', {}))
    if args.textures and os.path.isdir(args.textures):
        LIBRARY['textures'] = args.textures
    # clean scene
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    b = Building(spec, style)
    b.scope = args.scope
    b.plan_stair()
    b.build_exterior()
    if args.scope == 'exterior':
        EXTERIOR_SCOPE['seethrough'] = {g for g in args.seethrough_glass.split(',') if g}
        EXTERIOR_SCOPE['glass'] = args.exterior_glass
        exterior_scope_close_doors(b)
    else:
        b.build_interior()
    report = b.export(args.out)
    if args.render:
        render_previews(b, args.render)
    print('BLENDER_BUILDING_REPORT ' + json.dumps(report['files']))


# ---------------------------------------------------------------- bldg-264356936 block 9: ladder fire escape (_hb9_)
#   hbLadderEscape: {at, widthM, depthM, material, ladderOffM, dropM, gooseneckM}
#   The common San Francisco front escape: one shallow railed balcony per upper
#   storey, a vertical ladder between balconies through a hatch (ladderOffM
#   along the facade from the balcony centre), a drop ladder hung under the
#   lowest balcony (dropM long) and a gooseneck ladder from the top balcony up
#   over the parapet (gooseneckM above the roof line). Replaces `fireEscape`
#   on the front when present; absent = unchanged builds.
_hb9_orig_fire_escape = Building._fire_escape


def _hb9_fire_escape(self, e, a, b):
    spec = self.style.get('hbLadderEscape')
    if not spec or self.storeys < 2:
        return _hb9_orig_fire_escape(self, e, a, b)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    c = self.f2s(spec['at'], L, e)
    w, depth = float(spec.get('widthM', 2.0)), float(spec.get('depthM', 0.55))
    mat = spec.get('material', 'painted-steel')
    lo = float(spec.get('ladderOffM', 0.55))
    mb = MeshBuilder('fire-escape-ladder')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    def ladder(sv, off, y0, y1, gap=0.42):
        for side in (-1, 1):
            q = at(sv + side * gap / 2, off)
            mb.box((q[0], (y0 + y1) / 2, q[1]), (0.035, y1 - y0, 0.05), mat, yaw)
        n = max(1, int((y1 - y0) / 0.3))
        for i in range(1, n):
            q = at(sv, off)
            mb.box((q[0], y0 + (y1 - y0) * i / n, q[1]), (gap, 0.022, 0.022), mat, yaw)

    k_last = self.storeys - 1
    for k in range(1, k_last + 1):
        y = self.levels[k] - 0.02
        # slatted platform on two wall brackets
        p = at(c, depth / 2 + 0.02)
        mb.box((p[0], y, p[1]), (w, 0.04, depth), mat, yaw)
        for sv in (c - w / 2 + 0.12, c + w / 2 - 0.12):
            q = at(sv, depth / 2)
            mb.box((q[0], y - 0.07, q[1]), (0.05, 0.1, depth), mat, yaw)
            for i in range(4):
                t = (i + 0.5) / 4
                q = at(sv, depth * t)
                mb.box((q[0], y - 0.55 * (1 - t) - 0.1, q[1]), (0.04, 0.14, depth / 4 + 0.02), mat, yaw)
        # railing: top and mid rails on the front and ends, fine balusters
        for yy in (y + 0.95, y + 0.08):
            q = at(c, depth)
            mb.box((q[0], yy, q[1]), (w, 0.035, 0.035), mat, yaw)
            for side in (-1, 1):
                q2 = at(c + side * w / 2, depth / 2)
                mb.box((q2[0], yy, q2[1]), (0.035, 0.035, depth), mat, yaw)
        nb = int(w / 0.12)
        for i in range(nb + 1):
            q = at(c - w / 2 + i * w / nb, depth)
            mb.box((q[0], y + 0.5, q[1]), (0.014, 0.9, 0.014), mat, yaw)
        for side in (-1, 1):
            for j in range(1, 4):
                q = at(c + side * w / 2, depth * j / 4)
                mb.box((q[0], y + 0.5, q[1]), (0.014, 0.9, 0.014), mat, yaw)
        # vertical ladder up to the next balcony
        if k < k_last:
            ladder(c + lo, depth * 0.55, y + 0.04, self.levels[k + 1] - 0.02)
    # drop ladder hung under the lowest balcony
    y1 = self.levels[1] - 0.02
    ladder(c - lo, depth * 0.55, y1 - float(spec.get('dropM', 1.6)), y1)
    # gooseneck: straight up past the roof line, then over to the parapet
    yt = self.levels[k_last] - 0.02
    top = self.roof_level + float(spec.get('gooseneckM', 1.0))
    ladder(c - lo, depth * 0.55, yt + 0.04, top)
    for side in (-1, 1):
        for i in range(4):
            t = (i + 0.5) / 4
            q = at(c - lo + side * 0.21, depth * 0.55 * (1 - t))
            mb.box((q[0], top - 0.12 * t * t, q[1]), (0.035, 0.05, depth * 0.55 / 4 + 0.02), mat, yaw)
    mb.build(self.exterior)


Building._fire_escape = _hb9_fire_escape


# ---------------------------------------------------------------- bldg-264356936 block 11: bay sash glazing (_hb11_)
#   hbBayGlazing: {marginM, maxW, sillM, headM, glass, frame, frameTint,
#                  sashM, revealM, meetingRail}
#   Each facet of a classic 3-sided bay carries one big double-hung sash: the
#   opening fills the facet up to marginM of wall (maxW cap), sillM above the
#   storey plate and headM below the next one, glass set revealM into the
#   facet, painted stiles/rails/meeting rail, and the `glassBacking` room tone
#   behind it (kind 'bay'). Glazed / round-cornered bays and styles without
#   the key are unchanged.
#   hbBayGlazing.sashes / bays[].sashes: {front: 1-3, sides: 1-3} (2026-10-10)
#   double-hung sashes side by side on the front facet / each side facet,
#   split by a mullion pier of hbBayGlazing.mullionM (0.12) wall; each sash
#   keeps the maxW cap and at least 0.3 m. The bay's own `sashes` wins over
#   the building default. Absent (or 1) = the single centred sash as before.
_hb11_orig_base_bays = _hb_orig_bays


def _hb11_sash_spans(fl, margin, maxw, n, mull):
    """[(s0, s1)] facet-local spans for n side-by-side sashes on a facet fl long."""
    if n <= 1:
        ww = max(0.3, min(fl - margin, maxw))
        return [((fl - ww) / 2, (fl + ww) / 2)]
    while n > 1 and (fl - margin - (n - 1) * mull) / n < 0.3:
        n -= 1
    if n <= 1:
        return _hb11_sash_spans(fl, margin, maxw, 1, mull)
    ww = min(maxw, (fl - margin - (n - 1) * mull) / n)
    g = n * ww + (n - 1) * mull
    s0 = (fl - g) / 2
    return [(s0 + i * (ww + mull), s0 + i * (ww + mull) + ww) for i in range(n)]


def _hb11_base_bays(self, e, a, b):
    s = self.style
    bg = s.get('hbBayGlazing')
    specs = s.get('bays') or []
    if not bg or not specs or any(sp.get('glazed') or int(sp.get('roundCorners', 0) or 0) > 0 for sp in specs):
        return _hb11_orig_base_bays(self, e, a, b)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    mb = MeshBuilder('bay-windows')
    glass = MeshBuilder('bay-glazing')
    margin = float(bg.get('marginM', 0.16))
    maxw = float(bg.get('maxW', 1.3))
    sill_off = float(bg.get('sillM', 0.55))
    head_off = float(bg.get('headM', 0.22))
    gmat = bg.get('glass', s['glass'])
    fmat = self.tinted(bg.get('frame', s['frame']), bg.get('frameTint'))
    sash = float(bg.get('sashM', 0.07))
    rev = float(bg.get('revealM', 0.1))
    mull = float(bg.get('mullionM', 0.12))
    for spec in specs:
        c, w, p = self.f2s(spec['at'], L, e), spec['widthM'], spec.get('projectionM', 0.6)
        mat = self.tinted(spec.get('material', 'painted-stucco'), spec.get('tint'))
        sashes = spec.get('sashes') or bg.get('sashes')
        k0 = spec.get('fromStorey', 1)
        k1 = min(self.storeys, spec.get('toStorey', self.storeys - 1) + 1)
        y_bottom = self.levels[k0]
        y_top = self.levels[k1] - 0.3 if k1 < self.storeys else self.roof_level

        def at(sv, off):
            return add(add(a, mul(d, sv)), mul(nout, off))

        ring = [at(c - w / 2, 0), at(c - w / 2 + p, p), at(c + w / 2 - p, p), at(c + w / 2, 0)]
        centre = at(c, p * 0.4)
        self._prism(mb, ring, y_bottom - 0.45, y_bottom + 0.05, mat)
        for k in range(k0 + 1, k1):
            self._prism(mb, ring, self.levels[k] - 0.12, self.levels[k] + 0.12, mat)
        self._prism(mb, [ring[0]] + [add(q, mul(nout, 0.12)) for q in ring[1:-1]] + [ring[-1]], y_top - 0.05, y_top + 0.35, mat)
        for k in range(k0, k1):
            y0 = self.levels[k] + 0.05
            y1 = (self.levels[k + 1] if k + 1 < self.storeys else self.roof_level) - 0.12
            for fi, (fa, fb) in enumerate(zip(ring[:-1], ring[1:])):
                fl = length(sub(fb, fa))
                fd = norm(sub(fb, fa))
                fn = (-fd[1], fd[0])
                if dot(sub(lerp2(fa, fb, 0.5), centre), fn) < 0:
                    fn = (fd[1], -fd[0])
                ia, ib = sub(fa, mul(fn, 0.1)), sub(fb, mul(fn, 0.1))
                nsash = int((sashes or {}).get('front' if fi == 1 else 'sides', 1) or 1)
                spans = _hb11_sash_spans(fl, margin, maxw, max(1, min(3, nsash)), mull)
                sill, head = y0 + sill_off, y1 - head_off
                wall_grid(mb, fa, fb, ia, ib, y0, y1, [(s0_, s1_, sill, head) for s0_, s1_ in spans],
                          mat, 'int-plaster', mat, cap_top=False)
                fyaw = box_yaw(fd)
                for s0_, s1_ in spans:
                    ww = s1_ - s0_
                    # one sash stays centred exactly as before (byte-identical builds)
                    oc = lerp2(fa, fb, 0.5) if len(spans) == 1 else add(fa, mul(fd, (s0_ + s1_) / 2))
                    gc = add(oc, mul(fn, -rev))
                    glass.box((gc[0], (sill + head) / 2, gc[1]), (ww, head - sill, 0.02), gmat, fyaw)
                    self._glass_backing(glass, s, 'bay', add(gc, mul(fn, -0.02)), sill, head, ww, fyaw, mul(fn, -1))
                    fc = add(gc, mul(fn, 0.02))
                    mid = (sill + head) / 2 + 0.04
                    rails = [(sill + sash / 2, sash), (head - sash / 2, sash)]
                    if bg.get('meetingRail', True):
                        rails.append((mid, sash * 1.2))
                    for yy, hh in rails:
                        glass.box((fc[0], yy, fc[1]), (ww, hh, 0.06), fmat, fyaw)
                    for side in (-1, 1):
                        q = add(fc, mul(fd, side * (ww / 2 - sash / 2)))
                        glass.box((q[0], (sill + head) / 2, q[1]), (sash, head - sill, 0.06), fmat, fyaw)
                    # stone-coloured sill ledge under each sash
                    q = add(oc, mul(fn, 0.03))
                    mb.box((q[0], sill - 0.04, q[1]), (ww + 0.14, 0.07, 0.12), mat, fyaw)
    mb.build(self.exterior)
    glass.build(self.exterior)


_hb_orig_bays = _hb11_base_bays


# ---------------------------------------------------------------- bldg-264356936 block 12: steep fire-escape stair (_hb12_)
#   fireEscape.hbSteepStair: {runM, treadWM, alongF, riserM, stringerM}
#   The front escape keeps its railed platforms (standoff hbStandoffM, outriggers
#   at hbSupportsAt) but each flight becomes one steep narrow stair: runM along
#   the facade, centred at alongF (facade fraction), treadWM wide, treads every
#   riserM of rise, a stepped stringer each side and a sloped handrail. Replaces
#   the full-width shallow flight; absent = unchanged builds.
_hb12_orig_fire_escape = Building._fire_escape


def _hb12_fire_escape(self, e, a, b):
    spec = self.style.get('fireEscape')
    st = (spec or {}).get('hbSteepStair')
    if not spec or not st or self.storeys < 2 or self.style.get('hbLadderEscape'):
        return _hb12_orig_fire_escape(self, e, a, b)
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    so = float(spec.get('hbStandoffM', 0) or 0)
    c, w, depth = self.f2s(spec['at'], L, e), spec.get('widthM', 3.4), spec.get('depthM', 0.9)
    mat = spec.get('material', 'painted-steel')
    mb = MeshBuilder('fire-escape')

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off + so))

    def wall_at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    k_last = min(self.storeys - 1, spec.get('toStorey', self.storeys - 1))
    sup = spec.get('hbSupportsAt')
    xs = [self.f2s(f, L, e) for f in sup] if sup else [c - w / 2 + 0.1, c + w / 2 - 0.1]
    for k in range(1, k_last + 1):
        y = self.levels[k] - 0.02
        p = at(c, depth / 2 + 0.02)
        mb.box((p[0], y, p[1]), (w, 0.05, depth), mat, yaw)
        for yy in (y + 1.0, y + 0.5):
            q = at(c, depth)
            mb.box((q[0], yy, q[1]), (w, 0.04, 0.04), mat, yaw)
            for side in (-1, 1):
                q2 = at(c + side * w / 2, depth / 2)
                mb.box((q2[0], yy, q2[1]), (0.04, 0.04, depth), mat, yaw)
        n_posts = int(w / 0.25)
        for i in range(n_posts + 1):
            q = at(c - w / 2 + i * w / n_posts, depth)
            mb.box((q[0], y + 0.5, q[1]), (0.02, 1.0, 0.02), mat, yaw)
        if so > 0:
            for sv in xs:
                q = add(wall_at(sv, 0), mul(nout, (so + depth * 0.5) / 2))
                mb.box((q[0], y - 0.08, q[1]), (0.07, 0.1, so + depth * 0.5), mat, yaw)
                nb = 5
                for i in range(nb):
                    t = (i + 0.5) / nb
                    q = wall_at(sv, so * t)
                    mb.box((q[0], y - 0.9 * (1 - t) - 0.12, q[1]), (0.05, 0.2, so / nb + 0.02), mat, yaw)
        if k + 1 <= k_last:
            rise = self.levels[k + 1] - self.levels[k]
            run = float(st.get('runM', 1.6))
            tw = float(st.get('treadWM', 0.55))
            sc = self.f2s(st.get('alongF', spec['at']), L, e)
            n = max(4, int(round(rise / float(st.get('riserM', 0.21)))))
            tr = run / n
            off = depth * 0.5
            x0 = sc - run / 2
            for i in range(n - 1):
                q = at(x0 + (i + 0.5) * tr, off)
                mb.box((q[0], y + rise * (i + 1) / n, q[1]), (tr + 0.03, 0.03, tw), mat, yaw)
            sm = float(st.get('stringerM', 0.1))
            for side in (-1, 1):
                for i in range(n):
                    q = at(x0 + (i + 0.5) * tr, off + side * tw / 2)
                    yy = y + rise * (i + 0.5) / n
                    mb.box((q[0], yy, q[1]), (tr + 0.02, rise / n + sm, 0.035), mat, yaw)
                    mb.box((q[0], yy + 0.9, q[1]), (tr + 0.02, rise / n + 0.03, 0.03), mat, yaw)
    mb.build(self.exterior)


Building._fire_escape = _hb12_fire_escape


# ---------------------------------------------------------------- bldg-264356936 block 13: patchwork mural bands, downspouts (_hb13_)
# Appended, self-contained. NEW style keys only; absent = unchanged builds.
#   hbMuralBands: {count, slope, waveM, waveLenM, seed, palette, dotEvery,
#              dotSpacingM, dotRM, dotPalette}
#              abstract patchwork over the style `mural` field: wavy diagonal
#              colour bands (built as column slices clamped to the field) and
#              regular dot fields on every `dotEvery`-th band. Generic pattern,
#              copies no artwork.
#   hbDownspouts: {at: [fractions of the front edge], widthM, material, tint,
#              topDropM}  square rainwater leaders down the front face.
_hb13_orig_edge_trim = Building._edge_trim


def _hb13_edge_trim(self, e, a, b, trim):
    _hb13_orig_edge_trim(self, e, a, b, trim)
    s = self.style
    if e['role'] != 'front':
        return
    mbx = s.get('hbMuralBands')
    ds = s.get('hbDownspouts')
    if not mbx and not ds:
        return
    L = e['lengthM']
    d = norm(sub(b, a))
    nout = mul(self.inward(a, b), -1)
    yaw = box_yaw(d)
    out = B(nout[0], 0, nout[1])

    def at(sv, off):
        return add(add(a, mul(d, sv)), mul(nout, off))

    def P(sv, y, off):
        p = at(sv, off)
        return B(p[0], y, p[1])
    mb = MeshBuilder('front-dressing-r13')
    mu = s.get('mural')
    if mbx and mu:
        s0, s1 = sorted((self.f2s(mu.get('from', 0.05), L, e), self.f2s(mu.get('to', 0.95), L, e)))
        y0 = mu.get('bottomM', 3.0)
        y1 = min(self.height - 0.4, mu.get('topM', self.height - 1.0))
        W, H = s1 - s0, y1 - y0
        rng = random.Random(mbx.get('seed', 3))
        pal = mbx.get('palette') or ['#2f6fa8', '#3f8f5e', '#f0b531', '#e58fb0', '#6fb7d8']
        n = int(mbx.get('count', 7))
        slope = float(mbx.get('slope', 0.55))
        amp = float(mbx.get('waveM', 0.25))
        lam = float(mbx.get('waveLenM', 2.6))
        # boundaries span the whole field after the diagonal shift
        lo = -slope * W - amp
        hi = H + amp
        step = (hi - lo) / n
        phase = [rng.uniform(0, 2 * math.pi) for _ in range(n + 1)]

        def bnd(i, x):
            if i <= 0:
                return -1e9
            if i >= n:
                return 1e9
            return lo + i * step + slope * (x - s0) + amp * math.sin(2 * math.pi * (x - s0) / lam + phase[i])
        cols = max(4, int(W / 0.2))
        cw = W / cols
        off = 0.012 + 0.004 * (int(mu.get('shapes', 36)) + 1) + 0.002
        cols_of = []
        for i in range(n):
            cols_of.append(pal[i % len(pal)] if i % len(pal) != (i - 1) % len(pal) else rng.choice(pal))
        for i in range(n):
            mat = f'painted-white@{cols_of[i].lstrip("#")}'
            for j in range(cols):
                xa, xb = s0 + j * cw, s0 + (j + 1) * cw
                la = min(max(y0 + bnd(i, xa), y0), y1)
                lb = min(max(y0 + bnd(i, xb), y0), y1)
                ha = min(max(y0 + bnd(i + 1, xa), y0), y1)
                hb = min(max(y0 + bnd(i + 1, xb), y0), y1)
                if ha - la < 0.01 and hb - lb < 0.01:
                    continue
                mb.quad([P(xa, la, off), P(xb, lb, off), P(xb, hb, off), P(xa, ha, off)], mat, out)
        de = int(mbx.get('dotEvery', 2))
        if de > 0:
            sp = float(mbx.get('dotSpacingM', 0.2))
            r = float(mbx.get('dotRM', 0.035))
            dpal = mbx.get('dotPalette') or ['#f2efe6', '#141414']
            seg = 6
            for i in range(0, n, de):
                dm = f'painted-white@{dpal[(i // de) % len(dpal)].lstrip("#")}'
                x = s0 + sp / 2
                row = 0
                while x < s1 - r:
                    y = y0 + sp / 2 + (sp / 2 if row % 2 else 0)
                    while y < y1 - r:
                        if y0 + bnd(i, x) + r < y < y0 + bnd(i + 1, x) - r and y > y0 + r:
                            pts = []
                            for k in range(seg):
                                t = 2 * math.pi * k / seg
                                pts.append(P(x + r * math.cos(t), y + r * math.sin(t), off + 0.003))
                            mb.poly(pts, dm, out)
                        y += sp
                    x += sp * 0.866
                    row += 1
    if ds:
        w = float(ds.get('widthM', 0.09))
        dm = self.tinted(ds.get('material', 'painted-steel'), ds.get('tint')) if ds.get('tint') else s['trim']
        ytop = self.height - float(ds.get('topDropM', 0.7))
        for f in ds.get('at') or []:
            sv = self.f2s(f, L, e)
            q = at(sv, 0.03 + w / 2)
            mb.box((q[0], (0.15 + ytop) / 2, q[1]), (w, ytop - 0.15, w), dm, yaw)
            mb.box((q[0], ytop + 0.12, q[1]), (w * 2.2, 0.24, w * 1.6), dm, yaw)
            q = at(sv, 0.03 + w)
            mb.box((q[0], 0.2, q[1]), (w, 0.1, w * 2), dm, yaw)
    mb.build(self.exterior)


Building._edge_trim = _hb13_edge_trim


# ---------------------------------------------------------------- interior clip (_clip_)
#   `clipInteriorToFootprint: true` (style; absent = unchanged): just before
#   export (after furniture and lamps are emitted), faces of the building's own interior meshes that lie
#   outside the footprint are deleted, and a shared library piece (instanced
#   mesh data) with any vertex outside is dropped whole. For narrow, notched or
#   diagonal footprints where the room planner puts a lamp, rug or stair edge
#   past the outer wall and the containment gate rejects every build.
_clip_orig_export = Building.export


def _clip_export(self, out_dir):
    if self.style.get('clipInteriorToFootprint'):
        _clip_interior(self)
    return _clip_orig_export(self, out_dir)


def _clip_interior(self):
    cols = [self.interior] + [c for c in bpy.data.collections if c.name.startswith('fine-s')]
    bpy.context.view_layer.update()   # fresh pieces still carry a stale matrix_world
    inside = lambda v: self._inside_outer((v.x, -v.y), 0.01)
    dropped = clipped = 0
    for col in cols:
        for o in list(col.all_objects):
            if o.type != 'MESH':
                continue
            mw = o.matrix_world
            if o.data.users > 1:
                if any(not inside(mw @ v.co) for v in o.data.vertices):
                    bpy.data.objects.remove(o, do_unlink=True)
                    dropped += 1
                continue
            bm = bmesh.new()
            bm.from_mesh(o.data)
            out = [f for f in bm.faces if any(not inside(mw @ v.co) for v in f.verts)]
            if out:
                bmesh.ops.delete(bm, geom=out, context='FACES')
                loose = [v for v in bm.verts if not v.link_faces]
                if loose:
                    bmesh.ops.delete(bm, geom=loose, context='VERTS')
                bm.to_mesh(o.data)
                o.data.update()
                clipped += 1
            bm.free()
    # interior physics (stair guards, flights) planned outside the footprint goes too
    def _centre_in(e):
        pts = []
        c, sz = e.get('center'), e.get('size') or {}
        if c:
            hx, hz = sz.get('x', 0) / 2, sz.get('z', 0) / 2
            pts += [(c['x'] + dx, c['z'] + dz) for dx in (-hx, hx) for dz in (-hz, hz)]
        for k in ('from', 'to'):
            if e.get(k):
                pts.append((e[k]['x'], e[k]['z']))
        return all(self._inside_outer(p, 0.0) for p in pts)
    for key in ('solids', 'steps'):
        kept = [e for e in self.physics.get(key, []) if not str(e.get('id', '')).startswith(('guard-', 'flight-')) or _centre_in(e)]
        dropped += len(self.physics.get(key, [])) - len(kept)
        if key in self.physics:
            self.physics[key] = kept
    print(f'INTERIOR_CLIP dropped {dropped} pieces, clipped {clipped} meshes')
    if os.environ.get('EO_CLIP_LOG'):
        with open(os.environ['EO_CLIP_LOG'], 'a') as fh:
            fh.write(f"{self.spec.get('id')} dropped={dropped} clipped={clipped} cols={[c.name for c in cols]} lamps={[(o.name, o.type, getattr(o.data, 'users', None)) for c in cols for o in c.all_objects if 'lamp' in o.name][:6]}\n")


Building.export = _clip_export


# ---------------------------------------------------------------- hand ornament (_hand_)
#   artifacts/buildings/<id>/hand.py, beside style.json, runs after the
#   generated exterior with this module's names in scope plus `b` (the
#   Building), `front` (its front edge) and `fa`/`fb`/`fd`/`fout` (front edge
#   ends, unit direction and outward normal in frame xz). It adds hand-modelled
#   ornament (portals, cornice brackets, friezes, stoops) to b.exterior, so the
#   ornament is rebuilt with the building. Absent = unchanged builds.
class HandKit:
    """Facade-relative modelling for hand.py: positions are (fraction along a
    street edge as the viewer reads it left to right, height y above the frame
    ground, offset out of the wall). Everything lands in one 'hand' mesh."""

    def __init__(self, b, edge=None):
        self.b = b
        self.mb = MeshBuilder('hand-ornament')
        self.edge(edge)

    def edge(self, index=None):
        b = self.b
        self.e = next(x for x in b.edges if (x['index'] == index if index is not None else x['role'] == 'front'))
        self.a, b2 = v2(self.e['a']), v2(self.e['b'])
        self.d = norm(sub(b2, self.a))
        self.out = mul(b.inward(self.a, b2), -1)
        self.L = self.e['lengthM']
        self.yaw = box_yaw(self.d)
        # the viewer's left-to-right direction along the wall
        self.r = self.d if b.left_to_right(self.e) else mul(self.d, -1)
        return self

    def p(self, f, off=0.0):
        sv = self.b.f2s(f, self.L, self.e)
        return add(add(self.a, mul(self.d, sv)), mul(self.out, off))

    def P(self, f, y, off=0.0, dr=0.0):
        """Blender-space point; dr nudges along the viewer's right in metres."""
        q = add(self.p(f, off), mul(self.r, dr))
        return B(q[0], y, q[1])

    def box(self, f, y, off, w, h, d, mat, dr=0.0):
        """Box centred at fraction f, centre height y, its back face `off` out of the wall."""
        q = add(self.p(f, off + d / 2), mul(self.r, dr))
        self.mb.box((q[0], y, q[1]), (w, h, d), mat, self.yaw)

    def gable(self, f, y, w, depth, rise, roof, face, soffit=None, eave=0.12, off=0.0, thick=0.08):
        """A projecting gabled hood/porch roof: eaves at height y, ridge y+rise,
        `depth` out of the wall, with a pediment face and a flat soffit."""
        hw = w / 2 + eave
        o = off + depth
        L, R, T = (-hw, y), (hw, y), (0.0, y + rise)
        P = lambda u, yy, oo: self.P(f, yy, oo, dr=u)
        up = B(0, 1, 0)
        outv = B(self.out[0], 0, self.out[1])
        for side in (L, R):
            pts = [P(side[0], side[1], off), P(T[0], T[1], off), P(T[0], T[1], o + eave), P(side[0], side[1], o + eave)]
            self.mb.quad(pts, roof, up)
            top = [P(side[0], side[1] + thick, off), P(T[0], T[1] + thick, off), P(T[0], T[1] + thick, o + eave), P(side[0], side[1] + thick, o + eave)]
            self.mb.quad(top, roof, up)
            self.mb.quad([pts[3], pts[2], top[2], top[3]], roof, outv)
        self.mb.quad([P(-w / 2, y, o), P(w / 2, y, o), P(0, y + rise * (w / 2) / hw, o)], face, outv)
        self.mb.quad([P(-w / 2, y, off), P(w / 2, y, off), P(w / 2, y, o), P(-w / 2, y, o)], soffit or face, B(0, -1, 0))

    def bracket(self, f, y, off, w, drop, depth, mat, dr=0.0):
        """A tapered scroll-bracket stand-in hung under y: deep at the top, `drop` long."""
        self.box(f, y - 0.08, off, w, 0.16, depth, mat, dr)
        self.box(f, y - drop / 2, off, w, drop, depth * 0.45, mat, dr)
        self.box(f, y - drop + 0.06, off, w * 1.2, 0.12, depth * 0.3, mat, dr)

    def cylinder(self, f, y0, y1, off, r, mat, dr=0.0, segs=12, r1=None):
        """Upright round shaft centred r out of the wall (plus `off`); r1 tapers the top."""
        q = add(self.p(f, off + r), mul(self.r, dr))
        r1 = r if r1 is None else r1
        ring = lambda y, rr: [B(q[0] + rr * math.cos(2 * math.pi * i / segs), y, q[1] + rr * math.sin(2 * math.pi * i / segs)) for i in range(segs)]
        lo, hi = ring(y0, r), ring(y1, r1)
        for i in range(segs):
            j = (i + 1) % segs
            mid = (lo[i] + lo[j]) / 2
            self.mb.quad([lo[i], lo[j], hi[j], hi[i]], mat, mid - B(q[0], y0, q[1]))
        self.mb.quad(hi, mat, B(0, 1, 0))

    def column(self, f, y0, y1, off, r, mat, dr=0.0):
        """Tuscan column: square plinth, torus band, tapered shaft, necking and abacus."""
        self.box(f, y0 + 0.09, off, r * 2.6, 0.18, r * 2.6, mat, dr)
        self.cylinder(f, y0 + 0.18, y0 + 0.28, off + 0.15 * r, r * 1.15, mat, dr)
        self.cylinder(f, y0 + 0.28, y1 - 0.3, off + 0.3 * r, r, mat, dr, r1=r * 0.86)
        self.cylinder(f, y1 - 0.3, y1 - 0.2, off + 0.2 * r, r * 1.05, mat, dr)
        self.box(f, y1 - 0.12, off, r * 2.5, 0.16, r * 2.5, mat, dr)

    def _arc(self, span, rise, n, grow=0.0):
        """Points (u, v) on a segmental arch springing at v=0 across `span`, offset outward by grow."""
        R = (span * span / 4 + rise * rise) / (2 * rise)
        cy = rise - R
        t0 = math.atan2(-cy, -span / 2)
        t1 = math.atan2(-cy, span / 2)
        return [((R + grow) * math.cos(t0 + (t1 - t0) * i / n), cy + (R + grow) * math.sin(t0 + (t1 - t0) * i / n)) for i in range(n + 1)]

    def arch_ring(self, f, y, span, rise, width, depth, mat, off=0.0, n=13, keystone=None):
        """Voussoir ring over an opening: springs at height y, `width` deep in
        the facade plane, `depth` proud of the wall; optional keystone material."""
        inner, outer = self._arc(span, rise, n), self._arc(span, rise, n, width)
        P = lambda uv, o: self.P(f, y + uv[1], o, dr=uv[0])
        outv = B(self.out[0], 0, self.out[1])
        for i in range(n):
            m = keystone if keystone and i == n // 2 else mat
            o = off + depth + (0.04 if m is keystone and keystone else 0.0)
            self.mb.quad([P(inner[i], o), P(inner[i + 1], o), P(outer[i + 1], o), P(outer[i], o)], m, outv)
            self.mb.quad([P(inner[i], off), P(inner[i + 1], off), P(inner[i + 1], o), P(inner[i], o)], m, B(0, -1, 0))
            self.mb.quad([P(outer[i], off), P(outer[i + 1], off), P(outer[i + 1], o), P(outer[i], o)], m, B(0, 1, 0))
            # joint lines between voussoirs read through the shading of a small step
            if i % 2 == 0:
                self.mb.quad([P(inner[i], o + 0.012), P(inner[i + 1], o + 0.012), P(outer[i + 1], o + 0.012), P(outer[i], o + 0.012)], m, outv)

    def arch_fill(self, f, y, span, rise, mat, off=0.0, n=13, bars=None, bar_mat=None):
        """Flat tympanum (glass or panel) filling the arch above the spring line, with radial bars."""
        pts = self._arc(span, rise, n)
        c = self.P(f, y, off)
        outv = B(self.out[0], 0, self.out[1])
        for i in range(n):
            self.mb.quad([c, self.P(f, y + pts[i][1], off, dr=pts[i][0]), self.P(f, y + pts[i + 1][1], off, dr=pts[i + 1][0])], mat, outv)
        for k in range(1, (bars or 0) + 1):
            i = round(n * k / (bars + 1))
            u, v = pts[i]
            ln = math.hypot(u, v) or 1
            nu, nv = -v / ln * 0.025, u / ln * 0.025
            o = off + 0.03
            self.mb.quad([self.P(f, y + nv, o, dr=nu), self.P(f, y - nv, o, dr=-nu), self.P(f, y + v - nv, o, dr=u - nu), self.P(f, y + v + nv, o, dr=u + nu)], bar_mat or mat, outv)

    def pyramid(self, center_xz, y, half, rise, mat):
        cx, cz = center_xz
        apex = B(cx, y + rise, cz)
        c = [B(cx - half, y, cz - half), B(cx + half, y, cz - half), B(cx + half, y, cz + half), B(cx - half, y, cz + half)]
        for i in range(4):
            mid = (c[i] + c[(i + 1) % 4]) / 2
            self.mb.quad([c[i], c[(i + 1) % 4], apex], mat, mid - B(cx, y, cz))

    def build(self):
        self.mb.build(self.b.exterior)


_hand_orig_build_exterior = Building.build_exterior


def _hand_build_exterior(self):
    _hand_orig_build_exterior(self)
    if '--style' not in sys.argv:
        return
    path = os.path.join(os.path.dirname(sys.argv[sys.argv.index('--style') + 1]), 'hand.py')
    if not os.path.exists(path):
        return
    front = next(x for x in self.edges if x['role'] == 'front')
    fa, fb = v2(front['a']), v2(front['b'])
    ns = dict(globals(), b=self, kit=HandKit(self), front=front, fa=fa, fb=fb, fd=norm(sub(fb, fa)), fout=mul(self.inward(fa, fb), -1))
    with open(path) as fh:
        exec(compile(fh.read(), path, 'exec'), ns)
    ns['kit'].build()
    print(f'HAND_ORNAMENT {path}')


Building.build_exterior = _hand_build_exterior


# ---------------------------------------------------------------- exterior scope
#   `--scope exterior` (build.mjs passes BUILDING_SCOPE from scope.mjs; user
#   decision 2026-10-09: exteriors only until the whole city is built). The
#   shell is built exactly as in a full build; then
#   - build_interior() never runs: no slabs, stair, partitions, programmes,
#     furniture, soft furnishings, lights, coarse boxes or fine-s<k> storeys,
#     and export writes exterior.glb only;
#   - every door opening is closed by an opaque panel in its reveal and the
#     physics walls lose their openings (a solid shell nobody walks into);
#   - every see-through kit pane becomes the opaque hollow-shell pane
#     (EXTERIOR_SCOPE['glass'], tint kept) and glassBacking planes are not
#     drawn, so no window, transom, door light or shopfront shows the empty
#     shell behind it.
EXTERIOR_SCOPE = {'seethrough': set(), 'glass': 'glass-shell'}
EXTERIOR_DOOR_DEPTH_M = 0.2   # closing panel, from the outer wall face


def exterior_scope_close_doors(b):
    mb = MeshBuilder('door-closures')
    for wall in b.physics['walls']:
        openings = wall.get('openings') or []
        if not openings:
            continue
        pa, pb = (wall['a']['x'], wall['a']['z']), (wall['b']['x'], wall['b']['z'])
        L = length(sub(pb, pa))
        if L < 1e-6:
            continue
        d = norm(sub(pb, pa))
        nout = mul(b.inward(pa, pb), -1)
        off = wall.get('thickness', WALL_T) / 2 - min(EXTERIOR_DOOR_DEPTH_M, wall.get('thickness', WALL_T) * 0.65)
        for o in openings:
            s0, s1 = o['t0'] * L, o['t1'] * L
            y0, y1 = o.get('sill', 0.0), o['head']
            c = add(add(pa, mul(d, (s0 + s1) / 2)), mul(nout, off))
            mb.box((c[0], (y0 + y1) / 2, c[1]), (s1 - s0, y1 - y0, 0.04), EXTERIOR_SCOPE['glass'], box_yaw(d))
        wall['openings'] = []
    mb.build(b.exterior)


def exterior_scope_glazing(b):
    swap = {}
    for o in b.exterior.all_objects:
        if o.type != 'MESH':
            continue
        for i, mat in enumerate(o.data.materials):
            if mat is None:
                continue
            base, _, tint = mat.name.partition('@')
            if base not in EXTERIOR_SCOPE['seethrough']:
                continue
            name = EXTERIOR_SCOPE['glass'] + ('@' + tint if tint else '')
            swap[mat.name] = name
            o.data.materials[i] = MATS.get(name)
    b.stats['exteriorScopeGlass'] = dict(sorted(swap.items()))


main()
