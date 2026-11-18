"""Vehicle v3 body engine: a feature-line loft measured from orthographic traces.

Blender frame (same as fleet-v1/v2): X right, -Y nose, Z up, ground z = 0.

A body is described by longitudinal curves (control points, PCHIP-interpolated so
nothing overshoots) instead of the superellipse envelope of fleet-v1:

  top(y)        centre-line silhouette: bumper, bonnet, windscreen, roof, tailgate
  bottom(y)     sill / bumper underside (wheel arches are added analytically)
  crease(y)     the shoulder character line, drawn as a real crease
  belt(y)       glass line (side-window sill)
  rail(y)       drop from top(y) to the roof rail / bonnet edge
  halfW(y)      max body half-width (fender bulge over the arches included)
  beltW(y)      half-width at the glass line
  railW(y)      half-width at the roof rail / bonnet edge

Each station's cross-section is a Hermite spline through fixed key points, so every
station has the same ring count and the rings line up into clean quads. The crease
key point uses one-sided tangents, which gives a hard character line under smooth
shading.  Plan-view rounding of the nose and tail scales the section's width.
"""
import math

import bmesh
import bpy
from mathutils import Vector


# ---------------------------------------------------------------- curves
class Curve:
    """Monotone piecewise-cubic (PCHIP) through (x, y) control points."""

    def __init__(self, pts):
        pts = sorted(pts)
        self.x = [p[0] for p in pts]
        self.y = [p[1] for p in pts]
        n = len(pts)
        h = [self.x[i + 1] - self.x[i] for i in range(n - 1)]
        d = [(self.y[i + 1] - self.y[i]) / h[i] for i in range(n - 1)]
        m = [0.0] * n
        if n == 2:
            m = [d[0], d[0]]
        else:
            m[0], m[-1] = d[0], d[-1]
            for i in range(1, n - 1):
                if d[i - 1] * d[i] <= 0:
                    m[i] = 0.0
                else:
                    w1, w2 = 2 * h[i] + h[i - 1], h[i] + 2 * h[i - 1]
                    m[i] = (w1 + w2) / (w1 / d[i - 1] + w2 / d[i])
        self.h, self.m = h, m

    def __call__(self, x):
        xs = self.x
        if x <= xs[0]:
            return self.y[0] + self.m[0] * (x - xs[0])
        if x >= xs[-1]:
            return self.y[-1] + self.m[-1] * (x - xs[-1])
        lo, hi = 0, len(xs) - 1
        while hi - lo > 1:
            mid = (lo + hi) // 2
            if xs[mid] <= x:
                lo = mid
            else:
                hi = mid
        h = self.h[lo]
        t = (x - xs[lo]) / h
        t2, t3 = t * t, t * t * t
        return ((2 * t3 - 3 * t2 + 1) * self.y[lo] + (t3 - 2 * t2 + t) * h * self.m[lo]
                + (-2 * t3 + 3 * t2) * self.y[lo + 1] + (t3 - t2) * h * self.m[lo + 1])


def hermite(p0, p1, m0, m1, t):
    t2, t3 = t * t, t * t * t
    return (p0 * (2 * t3 - 3 * t2 + 1) + m0 * (t3 - 2 * t2 + t)
            + p1 * (-2 * t3 + 3 * t2) + m1 * (t3 - t2))


# ---------------------------------------------------------------- body spec
class BodySpec:
    def __init__(self, cfg):
        self.cfg = cfg
        if 'shellSpan' in cfg:
            # the lofted shell covers only part of the vehicle (a truck cab)
            self.y0, self.y1 = cfg['shellSpan']
            self.L = self.y1 - self.y0
        else:
            self.L = cfg['L']
            self.y0, self.y1 = -self.L / 2, self.L / 2
        self.c = {k: Curve(v) for k, v in cfg['curves'].items()}
        self.arches = cfg['arches']  # [(y, zc, r)]
        self.nose_r = cfg.get('noseRound', 0.34)
        self.tail_r = cfg.get('tailRound', 0.28)
        self.nose_min = cfg.get('noseMin', 0.0)
        self.tail_min = cfg.get('tailMin', 0.0)
        # plan-view corner superellipse exponents (2 = elliptical, higher = squarer)
        self.nose_p = cfg.get('nosePower', 2.0)
        self.tail_p = cfg.get('tailPower', 2.0)

    def bottom(self, y):
        z = self.c['bottom'](y)
        for ay, zc, r in self.arches:
            dy = y - ay
            if abs(dy) < r:
                z = max(z, zc + math.sqrt(r * r - dy * dy))
        return z

    def plan(self, y):
        """Plan-view width factor: rounds the nose and tail corners."""
        if y < self.y0 + self.nose_r:
            t = min(1.0, (self.y0 + self.nose_r - y) / self.nose_r)
            p = self.nose_p
            return self.nose_min + (1 - self.nose_min) * max(0.0, 1 - t ** p) ** (1 / p)
        if y > self.y1 - self.tail_r:
            t = min(1.0, (y - (self.y1 - self.tail_r)) / self.tail_r)
            p = self.tail_p
            return self.tail_min + (1 - self.tail_min) * max(0.0, 1 - t ** p) ** (1 / p)
        return 1.0

    def keys(self, y):
        """Key points (x, z) bottom-centre -> top-centre, and crease flags."""
        c = self.c
        top = c['top'](y)
        zb = self.bottom(y)
        f = self.plan(y)
        hw = c['halfW'](y) * f
        bw = min(c['beltW'](y) * f, hw - 0.005)
        rw = min(c['railW'](y) * f, bw - 0.004)
        rail = top - c['rail'](y)
        belt = min(c['belt'](y), rail - 0.01)
        crease = min(c['crease'](y), belt - 0.02)
        # lower body: keep the ordering even over an arch where the section is short
        zc = max(crease, zb + 0.10)
        zmax = max(zc - c['bulgeDrop'](y), zb + 0.06)
        zr = zb + min(0.07, (zmax - zb) * 0.4)
        tuck = c['tuck'](y) if 'tuck' in c else 0.05
        roofEdge = top - c['crown'](y)
        boxy = bool(self.cfg.get('boxySections'))
        hard = lambda index: boxy and index in (1, 2, 4, 5, 6, 7)
        pts = [
            (0.0, zb, False),                       # 0 bottom centre
            (hw - 0.17 * f, zb, hard(1)),           # 1 floor edge
            (hw - tuck, zr, hard(2)),                # 2 rocker turn (tuck-under)
            (hw - 0.004, zmax, False),              # 3 lower door
            # 4 shoulder crease (widest). It is only a crease where the section
            # still has a shoulder above it: at the tail the top falls down the
            # rear face and a sharp crease there folds across the bumper
            (hw, zc, hard(4) or (belt - zc >= 0.10 and zc - zb >= 0.15)),
            (bw, belt, hard(5)),                    # 5 glass line / fender top
            (rw, rail, hard(6)),                    # 6 roof rail / bonnet edge
            (rw * 0.80, roofEdge, hard(7)),         # 7 roof shoulder
            (0.0, top, False),                      # 8 top centre
        ]
        return pts


# per-span sample counts (between key i and i+1) for each LOD
SPAN = {0: [5, 3, 5, 3, 5, 7, 4, 7], 1: [3, 2, 3, 2, 3, 4, 2, 4], 2: [2, 1, 2, 1, 1, 2, 1, 2]}
# explicit parameter samples for spans that need thin edge rows (glass band:
# a thin belt row and a thin top-frame row so the glass fills the opening)
SPAN_T = {0: {5: [0.0, 0.03, 0.2, 0.4, 0.6, 0.8, 0.955]},
          1: {5: [0.0, 0.05, 0.5, 0.94]},
          2: {5: [0.0, 0.08, 0.92]}}
KEY_RING = {}


def section(spec, y, lod):
    ks = spec.keys(y)
    P = [Vector((k[0], k[1])) for k in ks]
    n = len(P)
    # chord-scaled Catmull-Rom: tangent directions from the neighbours, magnitudes
    # from the span's own chord, so short spans never loop or overshoot
    dirs = []
    for i in range(n):
        if i == 0:
            d = Vector((1.0, 0.0))
        elif i == n - 1:
            d = Vector((-1.0, 0.0))
        else:
            d = (P[i + 1] - P[i - 1])
            d = d.normalized() if d.length > 1e-9 else (P[i + 1] - P[i]).normalized()
        dirs.append(d)
    out = []
    spans = SPAN[lod]
    ring = 0
    for i in range(n - 1):
        a, b = P[i], P[i + 1]
        c = (b - a).length
        ma, mb = dirs[i] * c, dirs[i + 1] * c
        if ks[i][2]:
            ma = (b - a)
        if ks[i + 1][2]:
            mb = (b - a)
        KEY_RING[i] = ring
        ts = SPAN_T.get(lod, {}).get(i)
        if ts is None:
            ts = [s_ / spans[i] for s_ in range(spans[i])]
        for t in ts:
            out.append(hermite(a, b, ma, mb, t))
            ring += 1
    KEY_RING[n - 1] = ring
    out.append(P[-1])
    return out


def stations(spec, step, forced=(), ends=20):
    ys = []
    n = max(2, int(round(spec.L / step)))
    for i in range(n + 1):
        ys.append(spec.y0 + spec.L * i / n)
    # denser at the rounded ends
    # denser toward each tip, where the plan superellipse turns the corner
    for k in range(1, ends):
        u = (k / ends) ** 2
        ys.append(spec.y0 + spec.nose_r * u)
        ys.append(spec.y1 - spec.tail_r * u)
    # stations placed by plan-width fraction, so a squared-off nose or tail (high
    # superellipse power) still gets its corner and end face sampled evenly
    fs = [0.995, 0.985, 0.97, 0.95, 0.92, 0.88, 0.82, 0.74, 0.64, 0.52, 0.40, 0.28, 0.16]
    if ends < 20:
        fs = [0.97, 0.82, 0.52, 0.16] if ends >= 10 else [0.90, 0.40]
    for lo_, hi_, tip in ((spec.y0, spec.y0 + spec.nose_r, spec.y0), (spec.y1 - spec.tail_r, spec.y1, spec.y1)):
        for ftarget in fs:
            a_, b_ = lo_, hi_
            for _ in range(40):
                m_ = (a_ + b_) / 2
                inside = (spec.plan(m_) > ftarget)
                # plan() grows away from the tip
                if (tip == spec.y0) == inside:
                    b_ = m_
                else:
                    a_ = m_
            ys.append((a_ + b_) / 2)
    ys += list(forced)
    ys = sorted(set(round(y, 5) for y in ys if spec.y0 <= y <= spec.y1))
    return ys


def build_shell(spec, name, material, lod=0, step=0.025, forced=()):
    """Closed, mirrored shell. Returns (object, ring index map)."""
    ys = stations(spec, step, forced, ends={0: 20, 1: 10, 2: 5}.get(lod, 20))
    bm = bmesh.new()
    ring_l = bm.verts.layers.int.new('ring')
    rows = []
    for y in ys:
        sec = section(spec, y, lod)
        right = [bm.verts.new((p.x, y, p.y)) for p in sec]
        # left side mirrors rings 1..N-1 (centre points are shared)
        left = [bm.verts.new((-p.x, y, p.y)) for p in sec[1:-1]]
        for j, v in enumerate(right):
            v[ring_l] = j
        for j, v in enumerate(left):
            v[ring_l] = j + 1
        ring = right + list(reversed(left))  # bottom centre -> right -> top centre -> left
        rows.append(ring)
    m = len(rows[0])
    for a, b in zip(rows, rows[1:]):
        for j in range(m):
            j2 = (j + 1) % m
            bm.faces.new((a[j], b[j], b[j2], a[j2]))
    # caps (flat) at nose and tail, then weld degenerate points
    for f in bm.faces:
        f.smooth = True
    # domed end caps: concentric rings stepping outward along -y (nose) / +y (tail)
    for ring, sign, dome in ((rows[0], -1, spec.cfg.get('noseDome', 0.035)), (rows[-1], 1, spec.cfg.get('tailDome', 0.03))):
        cx = sum(v.co.x for v in ring) / len(ring)
        cz = sum(v.co.z for v in ring) / len(ring)
        prev = ring
        steps = 4
        for k in range(1, steps + 1):
            s_ = 1 - k / (steps + 0.6)
            dy = sign * dome * (1 - s_ * s_)
            cur = [bm.verts.new((cx + (v.co.x - cx) * s_, v.co.y + dy, cz + (v.co.z - cz) * s_)) for v in ring]
            for v2, v1 in zip(cur, ring):
                v2[ring_l] = v1[ring_l]
            m_ = len(ring)
            for j in range(m_):
                j2 = (j + 1) % m_
                quad = (prev[j], prev[j2], cur[j2], cur[j]) if sign > 0 else (prev[j], cur[j], cur[j2], prev[j2])
                try:
                    f = bm.faces.new(quad)
                    # a flat end panel (dome 0) is planar: shading its radial
                    # spokes smooth draws creases converging on the centre
                    f.smooth = abs(dome) > 1e-4
                except ValueError:
                    pass
            prev = cur
        try:
            f = bm.faces.new(prev if sign > 0 else list(reversed(prev)))
            f.smooth = abs(dome) > 1e-4
        except ValueError:
            pass
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=0.0008)
    bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=0.0005)
    # the last stations step down to a bumper-only section, so the upper rows
    # crowd into the bumper top and twist; relax them (volume kept, the
    # silhouette extremes pinned) instead of drawing that crumple
    relax = spec.cfg.get('endRelax', 0.30)
    if relax > 0:
        y_lo, y_hi = min(ys), max(ys)
        zs_ = [v.co.z for v in bm.verts]
        z_lo, z_hi = min(zs_), max(zs_)
        vs = [v for v in bm.verts if (v.co.y > y_hi - relax or v.co.y < y_lo + relax)
              and abs(v.co.y - y_hi) > 1e-4 and abs(v.co.y - y_lo) > 1e-4 and not v.is_boundary]
        nbrs = {v: [e.other_vert(v) for e in v.link_edges] for v in vs}
        # Taubin (lambda/mu) smoothing: removes the wrinkle without shrinking the body
        for _ in range(spec.cfg.get('endRelaxIters', 6)):
            for fac in (0.5, -0.53):
                new_ = {v: v.co + (sum((n.co for n in nb), Vector()) / len(nb) - v.co) * fac for v, nb in nbrs.items() if nb}
                for v, co in new_.items():
                    v.co = co
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    ob = bpy.data.objects.new(name, me)
    me.materials.append(material)
    return ob, dict(KEY_RING), ys
