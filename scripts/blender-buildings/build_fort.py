"""Headless Blender builder for a masonry casemate fort (style.json `"builder": "fort"`).

    blender -b --factory-startup --python scripts/blender-buildings/build_fort.py -- \
        --spec artifacts/buildings/<id>/spec.json --style artifacts/buildings/<id>/style.json \
        --out <dir> [--render <dir>] --scope exterior

The same contract as build_building.py (spec, frame, kit materials, physics.json,
report.json, exterior.glb), for the one building type that builder cannot
express: a closed ring of vaulted gun casemates around an open parade ground.

  - the SCARP: the outer wall stands exactly on the surveyed footprint, from the
    floor datum to the LiDAR height; brick, a granite plinth course and a
    granite coping; small embrasures in rows, one row per casemate tier, on
    every face but the landward one (`front`, the gorge);
  - the GORGE (the front edge): the barracks face, rows of windows with granite
    lintels and sills and one arched sally port in granite;
  - the PARADE: an open court cut out of the footprint. Its ring is the convex
    hull of the footprint pushed in by the casemate depth (a half-plane
    intersection, so short jogs of the survey outline never fold it), and its
    faces are arcades: per tier a row of round-arched casemate openings, the
    gorge side rows of windows;
  - the BARBETTE: the roof terrace on top of the casemates behind a parapet,
    the parapet top at the LiDAR height;
  - optional corner TURRETS (stair towers) up to `turretRiseM` over the parapet
    (the envelope allows 0.6 m).

Openings are recesses (exterior scope: nothing is drawn behind them): reveal
faces into the wall and a dark back panel. Every exterior material is an
authored-kit name (`kit@rrggbb` tints), so the runtime swaps in the kit's node
materials. The shell (outer wall faces, named `shell`) is what the verifier
measures against the footprint and the height.

It executes build_building.py's kit (MeshBuilder, MATS, B, geometry helpers,
render_previews) without its trailing main(), the way build_house.py does; the
pass id covers both scripts (build.mjs).
"""

from types import SimpleNamespace
import json
import math
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_BASE = os.path.join(_HERE, 'build_building.py')
with open(_BASE) as _fh:
    _src = _fh.read()
_cut = _src.rstrip().rfind('\nmain()')
if _cut < 0:
    raise SystemExit('build_fort.py: build_building.py no longer ends with main(); update the import shim')
exec(compile(_src[:_cut], _BASE, 'exec'), globals())  # noqa: S102 - our own sibling script

FORT_DEFAULTS = {
    'brick': 'brick-common-bond',
    'granite': 'limestone-ashlar@b4b8bc',
    'deck': 'concrete-precast@a49e94',
    'parade': 'concrete-precast@8e897f',
    'recess': 'painted-block@3a332e',
    'door': 'wood-siding@5a4434',
    'iron': 'painted-steel',
    'parapetM': 1.3,            # parapet height above the barbette deck
    'parapetThickM': 0.9,
    'plinthM': 1.1,             # granite base course on the scarp
    'copingM': 0.35,            # granite band at the top of the scarp
    'casemateDepthM': 9.0,      # parade ring inset on the casemate fronts
    'gorgeDepthM': 8.0,         # ... on the gorge (barracks) front
    'tiers': [0.0, 4.9, 9.4],   # casemate tier floors (m above the datum)
    'embrasure': {'widthM': 0.9, 'heightM': 1.3, 'sillM': 1.3, 'spacingM': 5.4, 'minEdgeM': 5.0, 'surroundM': 0.22},
    'arcade': {'bayM': 5.2, 'widthFrac': 0.6, 'springFrac': 0.55, 'depthM': 0.9, 'segments': 8},
    'gorgeWindows': {'rows': [1.1, 4.3, 7.5, 10.7], 'widthM': 1.0, 'heightM': 1.8, 'spacingM': 3.6, 'depthM': 0.5},
    'sallyPort': {'widthM': 3.2, 'heightM': 4.4, 'springM': 3.0, 'surroundM': 0.5},
    'turrets': [],              # [{vertex, sizeM}] polygon vertex indices that get a stair tower
    'turretRiseM': 0.5,
    'gallery': True,            # iron railings along the upper arcade tiers
}

RECESS_DEPTH = 0.6


# ---------------------------------------------------------------- 2-D helpers (frame x/z)

def signed_area(ring):
    return sum(ring[i][0] * ring[(i + 1) % len(ring)][1] - ring[(i + 1) % len(ring)][0] * ring[i][1] for i in range(len(ring))) / 2


def point_in(ring, p):
    x, z = p
    inside = False
    for i in range(len(ring)):
        a, b = ring[i - 1], ring[i]
        if (b[1] > z) != (a[1] > z) and x < (a[0] - b[0]) * (z - b[1]) / ((a[1] - b[1]) or 1e-12) + b[0]:
            inside = not inside
    return inside


def outward(ring, a, b):
    """Unit normal of edge a->b pointing out of `ring`."""
    d = norm(sub(b, a))
    n = (d[1], -d[0])
    mid = lerp2(a, b, 0.5)
    return mul(n, -1) if point_in(ring, add(mid, mul(n, 0.05))) else n


def convex_hull(points):
    pts = sorted(set(points))
    if len(pts) < 3:
        return pts

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    lower, upper = [], []
    for p in pts:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(pts):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    return lower[:-1] + upper[:-1]


def clip_half_plane(ring, p, n, d):
    """Keep the part of convex `ring` where dot(x - p, n) >= d (n points inward)."""
    out = []
    f = [dot(sub(q, p), n) - d for q in ring]
    for i in range(len(ring)):
        a, b, fa, fb = ring[i - 1], ring[i], f[i - 1], f[i]
        if fb >= 0:
            if fa < 0:
                out.append(lerp2(a, b, fa / (fa - fb)))
            out.append(b)
        elif fa >= 0:
            out.append(lerp2(a, b, fa / (fa - fb)))
    return out


def inset_ring(ring, distance, limit=3.0):
    """Mitred inward offset of a simple ring (inward = into the ring); mitres capped at `limit` x distance."""
    n = len(ring)
    out = []
    for i in range(n):
        p0, p1, p2 = ring[i - 1], ring[i], ring[(i + 1) % n]
        n0 = mul(outward(ring, p0, p1), -1)
        n1 = mul(outward(ring, p1, p2), -1)
        bis = add(n0, n1)
        if length(bis) < 1e-6:
            bis = n0
        bis = norm(bis)
        cos_half = max(dot(bis, n0), 1.0 / limit)
        out.append(add(p1, mul(bis, distance / cos_half)))
    return out


def dedupe(ring, eps=0.05):
    out = []
    for p in ring:
        if not out or length(sub(p, out[-1])) > eps:
            out.append(p)
    if len(out) > 2 and length(sub(out[0], out[-1])) <= eps:
        out.pop()
    return out


# ---------------------------------------------------------------- the fort


class Fort:
    def __init__(self, spec, style):
        self.spec = spec
        self.style = style
        self.f = {**FORT_DEFAULTS, **(style.get('fort') or {})}
        for key in ('embrasure', 'arcade', 'gorgeWindows', 'sallyPort'):
            self.f[key] = {**FORT_DEFAULTS[key], **((style.get('fort') or {}).get(key) or {})}
        self.ring = [v2(p) for p in spec['polygon']]
        self.height = float(spec['heightM'])
        self.front = int(spec['frame']['edgeIndex'])
        self.exterior = bpy.data.collections.new('exterior')
        bpy.context.scene.collection.children.link(self.exterior)
        self.stats = {'exterior': {}}
        self.scope = 'exterior'
        self.deck_y = self.height - self.f['parapetM']
        self.tiers = [t for t in self.f['tiers'] if t < self.deck_y - 2.5]
        self.storeys = len(self.tiers)
        self.levels = [round(t, 3) for t in self.tiers]
        self.parade = self._parade_ring()
        self.passage = self._passage()

    # -- parade ring ------------------------------------------------------

    def _parade_ring(self):
        hull = convex_hull([tuple(p) for p in self.ring])
        if signed_area(hull) < 0:
            hull = list(reversed(hull))
        front_n = outward(self.ring, self.ring[self.front], self.ring[(self.front + 1) % len(self.ring)])
        ring = list(hull)
        gorge_edges = []
        for i in range(len(hull)):
            a, b = hull[i], hull[(i + 1) % len(hull)]
            nin = mul(outward(hull, a, b), -1)
            gorge = dot(mul(nin, -1), front_n) > 0.9
            depth = self.f['gorgeDepthM'] if gorge else self.f['casemateDepthM']
            ring = clip_half_plane(ring, a, nin, depth)
            if gorge:
                gorge_edges.append((a, b))
        ring = dedupe(ring, 0.5)
        if len(ring) < 3:
            raise SystemExit('build_fort.py: the casemate depth leaves no parade inside the footprint')
        # every parade corner stands inside the footprint with the casemate mass around it
        for p in ring:
            if not point_in(self.ring, p):
                raise SystemExit(f'build_fort.py: parade corner {p} falls outside the footprint')
        self.gorge_dir = front_n
        return ring

    def _passage(self):
        """The sally port runs straight through the gorge range into the parade: from the
        middle of the front edge, along its inward normal, to the first parade face it meets."""
        a, b = self.ring[self.front], self.ring[(self.front + 1) % len(self.ring)]
        d = norm(sub(b, a))
        L = length(sub(b, a))
        mid = add(a, mul(d, L / 2))
        inward = mul(outward(self.ring, a, b), -1)
        best = None
        for j in range(len(self.parade)):
            p, q = self.parade[j], self.parade[(j + 1) % len(self.parade)]
            e = sub(q, p)
            den = inward[0] * e[1] - inward[1] * e[0]
            if abs(den) < 1e-9:
                continue
            w = sub(p, mid)
            t = (w[0] * e[1] - w[1] * e[0]) / den
            u = (w[0] * inward[1] - w[1] * inward[0]) / den
            if t > 0.5 and 0.0 <= u <= 1.0 and (best is None or t < best['depth']):
                best = {'edge': j, 'depth': t, 's': u * length(e)}
        if best is None:
            raise SystemExit('build_fort.py: the sally port axis meets no parade face')
        sp = self.f['sallyPort']
        Lp = length(sub(self.parade[(best['edge'] + 1) % len(self.parade)], self.parade[best['edge']]))
        if best['s'] - sp['widthM'] / 2 < 0.5 or best['s'] + sp['widthM'] / 2 > Lp - 0.5:
            raise SystemExit('build_fort.py: the sally port passage meets a parade corner')
        best.update({'mid': mid, 'inward': inward, 'along': d})
        return best

    # -- facade pieces ----------------------------------------------------

    @staticmethod
    def _outline(s0, s1, y0, y1, arch_spring=None, segments=8):
        """An opening outline [(s, y)]: sill corners first, then up the right jamb, over the
        head (a semicircle springing at arch_spring, or flat at y1) and down the left."""
        if arch_spring is None:
            return [(s0, y0), (s1, y0), (s1, y1), (s0, y1)]
        r = (s1 - s0) / 2
        c = (s0 + s1) / 2
        outline = [(s0, y0), (s1, y0), (s1, arch_spring)]
        for k in range(1, segments):
            t = math.pi * k / segments
            outline.append((c + r * math.cos(t), arch_spring + r * math.sin(t)))
        outline.append((s0, arch_spring))
        return outline

    def recess(self, mb, a, d, nout, s0, s1, y0, y1, wall, reveal, back, depth=RECESS_DEPTH, arch_spring=None, segments=8):
        """An opening s0..s1 x y0..y1 along a->d on a wall face whose outward normal is nout:
        reveals `depth` into the wall and a back panel. With arch_spring, the head is a
        semicircle springing at arch_spring (y1 is then ignored). Returns the outline
        [(s, y)] of the opening (for the wall face to leave open)."""
        def P(s, y, off=0.0):
            q = add(add(a, mul(d, s)), mul(nout, off))
            return B(q[0], y, q[1])
        outline = self._outline(s0, s1, y0, y1, arch_spring, segments)
        n3 = B(nout[0], 0, nout[1])
        # back panel, facing out (none for a through-passage)
        if back is not None:
            mb.quad([P(s, y, -depth) for s, y in outline], back, n3)
        # reveals: one quad per outline segment, facing into the opening
        cx = sum(s for s, _ in outline) / len(outline)
        cy = sum(y for _, y in outline) / len(outline)
        for i in range(len(outline)):
            (sa, ya), (sb, yb) = outline[i], outline[(i + 1) % len(outline)]
            mid_s, mid_y = (sa + sb) / 2, (ya + yb) / 2
            inward = sub((cx, cy), (mid_s, mid_y))
            hint_v = add(mul(d, inward[0]), (0, 0))
            hint = B(hint_v[0], inward[1], hint_v[1])
            mb.quad([P(sa, ya), P(sb, yb), P(sb, yb, -depth), P(sa, ya, -depth)], reveal, hint)
        return outline

    def wall_face(self, mb, a, d, nout, s0, s1, yb, yt, holes, mat_for):
        """The face of a wall strip s0..s1 x yb..yt with openings `holes` (outlines from recess()).
        Openings must lie inside the strip. Openings whose spans along the wall overlap form a
        column and are stacked; columns must not overlap. Piers between columns are quads; in a
        column, panels fill below, between and above the openings, each edge following the
        opening outline (an arch head included)."""
        def P(s, y):
            q = add(a, mul(d, s))
            return B(q[0], y, q[1])
        n3 = B(nout[0], 0, nout[1])

        def span(o):
            return min(s for s, _ in o), max(s for s, _ in o)
        columns = []
        for o in sorted(holes, key=lambda o: span(o)[0]):
            h0, h1 = span(o)
            if columns and h0 < columns[-1][1] - 1e-6:
                columns[-1][1] = max(columns[-1][1], h1)
                columns[-1][2].append(o)
            else:
                columns.append([h0, h1, [o]])
        cursor = s0
        for c0, c1, stack in columns:
            if c0 - cursor > 1e-4:
                mb.quad([P(cursor, yb), P(c0, yb), P(c0, yt), P(cursor, yt)], mat_for(yb, yt), n3)
            stack.sort(key=lambda o: min(y for _, y in o))
            floor_y = yb
            for k, outline in enumerate(stack):
                h0, h1 = span(outline)
                ylo = min(y for _, y in outline)
                # outline runs (s0,y0) (s1,y0) (s1,top) ... (s0,top): the upper chain drops the sill points
                chain = outline[2:]
                ytop = max(y for _, y in chain)
                if ylo - floor_y > 1e-4:
                    mb.quad([P(c0, floor_y), P(c1, floor_y), P(c1, ylo), P(c0, ylo)], mat_for(floor_y, ylo), n3)
                # side strips where this opening is narrower than its column
                if h0 - c0 > 1e-4:
                    mb.quad([P(c0, ylo), P(h0, ylo), P(h0, ytop), P(c0, ytop)], mat_for(ylo, ytop), n3)
                if c1 - h1 > 1e-4:
                    mb.quad([P(h1, ylo), P(c1, ylo), P(c1, ytop), P(h1, ytop)], mat_for(ylo, ytop), n3)
                ceil_y = min(y for _, y in stack[k + 1]) if k + 1 < len(stack) else yt
                pts = [P(h0, ceil_y), P(h1, ceil_y)] + [P(s, y) for s, y in chain]
                mb.quad(pts, mat_for(ytop, ceil_y), n3)
                if c1 - h1 > 1e-4 or h0 - c0 > 1e-4:
                    # the head panel spans the opening only: close the corners over the side strips
                    if h0 - c0 > 1e-4:
                        mb.quad([P(c0, ytop), P(h0, ytop), P(h0, ceil_y), P(c0, ceil_y)], mat_for(ytop, ceil_y), n3)
                    if c1 - h1 > 1e-4:
                        mb.quad([P(h1, ytop), P(c1, ytop), P(c1, ceil_y), P(h1, ceil_y)], mat_for(ytop, ceil_y), n3)
                floor_y = ceil_y
            cursor = c1
        if s1 - cursor > 1e-4:
            mb.quad([P(cursor, yb), P(s1, yb), P(s1, yt), P(cursor, yt)], mat_for(yb, yt), n3)

    # -- the scarp (outer walls) -------------------------------------------

    def build_scarp(self):
        f = self.f
        shell = MeshBuilder('shell')
        trim = MeshBuilder('trim-granite')
        recesses = MeshBuilder('openings')
        H = self.height
        brick, granite = f['brick'], f['granite']

        def band_mat(y0, y1):
            mid = (y0 + y1) / 2
            if mid < f['plinthM'] or mid > H - f['copingM']:
                return granite
            return brick
        emb = f['embrasure']
        n = len(self.ring)
        for i in range(n):
            a, b = self.ring[i], self.ring[(i + 1) % n]
            L = length(sub(b, a))
            if L < 1e-3:
                continue
            d = norm(sub(b, a))
            nout = outward(self.ring, a, b)
            holes = []   # (outline, band index)
            if i == self.front:
                holes += self._gorge_openings(recesses, trim, a, d, nout, L)
            elif L >= emb['minEdgeM']:
                count = max(1, int((L - 2.0) // emb['spacingM']))
                step = L / count
                for t in self.tiers:
                    for k in range(count):
                        c = step * (k + 0.5)
                        s0, s1 = c - emb['widthM'] / 2, c + emb['widthM'] / 2
                        y0 = t + emb['sillM']
                        y1 = y0 + emb['heightM']
                        if y1 > H - f['copingM'] - 0.3:
                            continue
                        holes.append(self.recess(recesses, a, d, nout, s0, s1, y0, y1, brick, granite, f['recess'], depth=0.7))
                        self._surround(trim, a, d, nout, s0, s1, y0, y1, emb['surroundM'], granite)
            # the face, band by band: the material zones (plinth, coping), each split
            # dropped where it would cut through an opening on this face
            splits = [y for y in (f['plinthM'], H - f['copingM'])
                      if not any(min(yy for _, yy in o) < y < max(yy for _, yy in o) for o in holes)]
            bands = sorted({0.0, H} | set(splits))
            for j in range(len(bands) - 1):
                y0, y1 = bands[j], bands[j + 1]
                inside = [o for o in holes if min(y for _, y in o) >= y0 - 1e-6 and max(y for _, y in o) <= y1 + 1e-6]
                self.wall_face(shell, a, d, nout, 0.0, L, y0, y1, inside, band_mat)
            if f['plinthM'] not in splits and f['plinthM'] > 0:
                self._plinth_strip(trim, a, d, nout, L, holes, granite)
        self.parts(shell, 'shell')
        self.parts(trim, 'trim')
        self.parts(recesses, 'openings')
        shell.build(self.exterior)
        trim.build(self.exterior)
        recesses.build(self.exterior)

    def _plinth_strip(self, mb, a, d, nout, L, holes, mat, proj=0.04):
        """The granite base course as a strip proud of the face, broken at openings that reach the ground."""
        yaw = box_yaw(d)
        h = self.f['plinthM']
        cuts = sorted((min(s for s, _ in o), max(s for s, _ in o)) for o in holes if min(y for _, y in o) < h)
        cursor = 0.0
        for c0, c1 in cuts + [(L, L)]:
            if c0 - cursor > 0.05:
                q = add(add(a, mul(d, (cursor + c0) / 2)), mul(nout, proj / 2))
                mb.box((q[0], h / 2, q[1]), (c0 - cursor, h, proj), mat, yaw)
            cursor = max(cursor, c1)

    def _surround(self, mb, a, d, nout, s0, s1, y0, y1, w, mat, proj=0.07, arch_spring=None):
        """A granite frame proud of the wall face around a rectangular opening."""
        yaw = box_yaw(d)

        def at(s, off):
            return add(add(a, mul(d, s)), mul(nout, off))
        c = at((s0 + s1) / 2, proj / 2)
        mb.box((c[0], y0 - w / 2, c[1]), (s1 - s0 + 2 * w, w, proj), mat, yaw)
        mb.box((c[0], y1 + w / 2, c[1]), (s1 - s0 + 2 * w, w, proj), mat, yaw)
        for s in (s0 - w / 2, s1 + w / 2):
            q = at(s, proj / 2)
            mb.box((q[0], (y0 + y1) / 2, q[1]), (w, y1 - y0, proj), mat, yaw)

    def _gorge_openings(self, recesses, trim, a, d, nout, L):
        f = self.f
        gw = f['gorgeWindows']
        sp = f['sallyPort']
        holes = []
        mid = L / 2
        port0, port1 = mid - sp['widthM'] / 2, mid + sp['widthM'] / 2
        # a through-passage into the parade (no back panel): its reveals are the
        # passage walls, vault and floor
        holes.append(self.recess(recesses, a, d, nout, port0, port1, 0.0, None, f['brick'], f['brick'], None,
                                 depth=self.passage['depth'], arch_spring=sp['springM'], segments=10))
        # the port's granite surround: jambs and a flat arch block over the head
        yaw = box_yaw(d)
        w = sp['surroundM']
        for s in (port0 - w / 2, port1 + w / 2):
            q = add(add(a, mul(d, s)), mul(nout, 0.05))
            trim.box((q[0], sp['springM'] / 2, q[1]), (w, sp['springM'], 0.1), f['granite'], yaw)
        q = add(add(a, mul(d, mid)), mul(nout, 0.05))
        crown = sp['springM'] + sp['widthM'] / 2
        trim.box((q[0], crown + 0.25, q[1]), (sp['widthM'] + 2 * w, 0.5, 0.1), f['granite'], yaw)
        count = max(1, int((L - 2.0) // gw['spacingM']))
        step = L / count
        for y0 in gw['rows']:
            y1 = y0 + gw['heightM']
            if y1 > self.height - f['copingM'] - 0.3:
                continue
            for k in range(count):
                c = step * (k + 0.5)
                s0, s1 = c - gw['widthM'] / 2, c + gw['widthM'] / 2
                # the port bay is a solid pier for the full height: no window over the arch
                if s1 > port0 - w - 0.4 and s0 < port1 + w + 0.4:
                    continue
                holes.append(self.recess(recesses, a, d, nout, s0, s1, y0, y1, f['brick'], f['granite'], f['recess'], depth=gw['depthM']))
                # granite lintel and sill
                for yy, hh in ((y1 + 0.15, 0.3), (y0 - 0.08, 0.16)):
                    q = add(add(a, mul(d, c)), mul(nout, 0.05))
                    trim.box((q[0], yy, q[1]), (gw['widthM'] + 0.3, hh, 0.1), f['granite'], yaw)
        return holes

    # -- parade (courtyard) faces ------------------------------------------

    def build_parade(self):
        f = self.f
        arc = f['arcade']
        gw = f['gorgeWindows']
        faces = MeshBuilder('parade-walls')
        recesses = MeshBuilder('parade-openings')
        rails = MeshBuilder('parade-galleries')
        ring = self.parade
        top = self.deck_y
        bands = sorted({0.0, top} | {t for t in self.tiers if 0 < t < top})
        for i in range(len(ring)):
            a, b = ring[i], ring[(i + 1) % len(ring)]
            L = length(sub(b, a))
            if L < 0.5:
                continue
            d = norm(sub(b, a))
            # the parade faces point INTO the court: away from the casemate mass
            nout = mul(outward(ring, a, b), -1)
            gorge = dot(nout, self.gorge_dir) < -0.9
            holes = []
            # where the sally port passage comes out into the parade: the same arch,
            # left open (the passage's reveals were drawn from the outer face)
            pass_span = None
            if i == self.passage['edge']:
                sp = f['sallyPort']
                # s runs a -> b here as on the parade ring; the passage met it at passage['s']
                c = self.passage['s']
                pass_span = (c - sp['widthM'] / 2, c + sp['widthM'] / 2)
                holes.append(self._outline(pass_span[0], pass_span[1], 0.0, None, sp['springM'], 10))
            if gorge:
                count = max(1, int((L - 1.5) // gw['spacingM']))
                step = L / count
                for y0 in gw['rows']:
                    y1 = y0 + gw['heightM']
                    if y1 > top - 0.4:
                        continue
                    for k in range(count):
                        c = step * (k + 0.5)
                        if pass_span and c + gw['widthM'] / 2 > pass_span[0] - 0.6 and c - gw['widthM'] / 2 < pass_span[1] + 0.6:
                            continue
                        holes.append(self.recess(recesses, a, d, nout, c - gw['widthM'] / 2, c + gw['widthM'] / 2, y0, y1,
                                                 f['brick'], f['brick'], f['recess'], depth=gw['depthM']))
            else:
                count = max(1, int(L // arc['bayM']))
                step = L / count
                for j, t in enumerate(self.tiers):
                    t1 = bands[bands.index(t) + 1]
                    tier_h = t1 - t
                    w = step * arc['widthFrac']
                    spring = t + tier_h * arc['springFrac']
                    if spring + w / 2 > t1 - 0.35:
                        w = max(1.0, 2 * (t1 - 0.35 - spring))
                    for k in range(count):
                        c = step * (k + 0.5)
                        holes.append(self.recess(recesses, a, d, nout, c - w / 2, c + w / 2, t, None,
                                                 f['brick'], f['brick'], f['recess'], depth=arc['depthM'],
                                                 arch_spring=spring, segments=arc['segments']))
                    if f['gallery'] and j > 0:
                        self._railing(rails, a, d, nout, L, t)
            for j in range(len(bands) - 1):
                y0, y1 = bands[j], bands[j + 1]
                inside = [o for o in holes if min(y for _, y in o) >= y0 - 1e-6 and max(y for _, y in o) <= y1 + 1e-6]
                self.wall_face(faces, a, d, nout, 0.0, L, y0, y1, inside, lambda y0, y1: f['brick'])
            # a granite string course at each tier floor and under the parapet
            yaw = box_yaw(d)
            for t in [t for t in self.tiers if t > 0] + [top - 0.25]:
                q = add(add(a, mul(d, L / 2)), mul(nout, 0.06))
                faces.box((q[0], t + 0.1, q[1]), (L, 0.22, 0.12), f['granite'], yaw)
        # the parade floor
        floor = MeshBuilder('parade-floor')
        floor.quad([B(p[0], 0.03, p[1]) for p in ring], f['parade'], B(0, 1, 0))
        self.parts(faces, 'parade')
        self.parts(recesses, 'parade-openings')
        self.parts(rails, 'galleries')
        self.parts(floor, 'parade-floor')
        for mb in (faces, recesses, rails, floor):
            mb.build(self.exterior)

    def _railing(self, mb, a, d, nout, L, y):
        """An iron gallery rail along a parade face at tier floor y: top rail and posts."""
        yaw = box_yaw(d)
        iron = self.f['iron']
        q = add(add(a, mul(d, L / 2)), mul(nout, 0.35))
        mb.box((q[0], y + 1.05, q[1]), (L - 0.4, 0.05, 0.05), iron, yaw)
        mb.box((q[0], y + 0.55, q[1]), (L - 0.4, 0.03, 0.03), iron, yaw)
        # walkway slab the rail stands on
        q2 = add(add(a, mul(d, L / 2)), mul(nout, 0.2))
        mb.box((q2[0], y + 0.06, q2[1]), (L - 0.2, 0.12, 0.4), iron, yaw)
        steps = max(2, int(L / 2.6))
        for k in range(steps + 1):
            s = 0.2 + (L - 0.4) * k / steps
            p = add(add(a, mul(d, s)), mul(nout, 0.35))
            mb.box((p[0], y + 0.55, p[1]), (0.05, 1.0, 0.05), iron, yaw)

    # -- barbette (roof) -----------------------------------------------------

    def _fill(self, name, loops, y, mat, up=True):
        """A horizontal face at height y over the region bounded by `loops` (outer ring first,
        then holes), triangulated by Blender's scan fill."""
        import bmesh
        mesh = bpy.data.meshes.new(name)
        bm = bmesh.new()
        edges = []
        for loop in loops:
            vs = [bm.verts.new(B(p[0], y, p[1])) for p in loop]
            for k in range(len(vs)):
                edges.append(bm.edges.new((vs[k], vs[(k + 1) % len(vs)])))
        bmesh.ops.triangle_fill(bm, use_beauty=True, use_dissolve=False, edges=edges, normal=B(0, 1, 0) if up else B(0, -1, 0))
        for face in bm.faces:
            face.normal_update()
            if (face.normal.z > 0) != up:
                face.normal_flip()
        bm.to_mesh(mesh)
        bm.free()
        mesh.materials.append(MATS.get(mat))
        obj = bpy.data.objects.new(name, mesh)
        self.exterior.objects.link(obj)
        tris = sum(len(p.vertices) - 2 for p in mesh.polygons)
        self.stats['exterior'][name] = tris
        return obj

    def build_barbette(self):
        f = self.f
        H = self.height
        inner = inset_ring(self.ring, f['parapetThickM'])
        # deck between the parapet's inner face and the parade
        self._fill('roof-deck', [inner, self.parade], self.deck_y, f['deck'])
        # parapet: coping on top, inner face down to the deck
        self._fill('shell-coping', [self.ring, inner], H, f['granite'])
        par = MeshBuilder('parapet-inner')
        for i in range(len(inner)):
            a, b = inner[i], inner[(i + 1) % len(inner)]
            if length(sub(b, a)) < 1e-3:
                continue
            n_in = mul(outward(inner, a, b), -1)
            par.quad([B(a[0], self.deck_y, a[1]), B(b[0], self.deck_y, b[1]), B(b[0], H, b[1]), B(a[0], H, a[1])], f['brick'], B(n_in[0], 0, n_in[1]))
        # the parade-side parapet (low wall at the deck's court edge)
        for i in range(len(self.parade)):
            a, b = self.parade[i], self.parade[(i + 1) % len(self.parade)]
            L = length(sub(b, a))
            if L < 0.5:
                continue
            d = norm(sub(b, a))
            nin = outward(self.parade, a, b)   # toward the deck
            q = add(lerp2(a, b, 0.5), mul(nin, 0.25))
            par.box((q[0], self.deck_y + 0.5, q[1]), (L, 1.0, 0.5), f['granite'], box_yaw(d))
        self.parts(par, 'parapet')
        par.build(self.exterior)
        self._gun_platforms()
        self._turrets()

    def _gun_platforms(self):
        """Barbette gun platforms: low granite traverse circles on the deck along the
        casemate (non-gorge) ranges, midway between the parapet and the parade."""
        f = self.f
        spacing = float(f.get('gunPlatformM', 9.0))
        radius = float(f.get('gunPlatformRadiusM', 2.2))
        if spacing <= 0:
            return
        mb = MeshBuilder('barbette-platforms')
        inner = inset_ring(self.ring, f['parapetThickM'])
        seg = 14
        for i in range(len(self.parade)):
            a, b = self.parade[i], self.parade[(i + 1) % len(self.parade)]
            L = length(sub(b, a))
            nin = outward(self.parade, a, b)       # from the court onto the deck
            if dot(nin, self.gorge_dir) > 0.9 or L < spacing:
                continue
            d = norm(sub(b, a))
            depth = f['casemateDepthM'] - f['parapetThickM']
            count = int(L // spacing)
            for k in range(count):
                c = add(add(a, mul(d, L * (k + 0.5) / count)), mul(nin, depth / 2 + 0.3))
                if not point_in(inner, c) or any(not point_in(inner, add(c, (radius * math.cos(t), radius * math.sin(t))))
                                                for t in (0, math.pi / 2, math.pi, 1.5 * math.pi)):
                    continue
                y0, y1 = self.deck_y, self.deck_y + 0.12
                ring = [add(c, (radius * math.cos(2 * math.pi * j / seg), radius * math.sin(2 * math.pi * j / seg))) for j in range(seg)]
                mb.quad([B(p[0], y1, p[1]) for p in ring], f['granite'], B(0, 1, 0))
                for j in range(seg):
                    p, q = ring[j], ring[(j + 1) % seg]
                    o = norm(sub(lerp2(p, q, 0.5), c))
                    mb.quad([B(p[0], y0, p[1]), B(q[0], y0, q[1]), B(q[0], y1, q[1]), B(p[0], y1, p[1])], f['granite'], B(o[0], 0, o[1]))
                # the pivot block
                mb.box((c[0], y1 + 0.15, c[1]), (0.6, 0.3, 0.6), f['granite'])
        if not mb.empty():
            self.parts(mb, 'platforms')
            mb.build(self.exterior)

    def _turrets(self):
        f = self.f
        H = self.height
        mb = MeshBuilder('turrets')
        for t in f['turrets']:
            i = int(t['vertex'])
            size = float(t.get('sizeM', 4.0))
            p = self.ring[i]
            # the tower sits inside the corner: along the bisector, clear of both faces
            prev, nxt = self.ring[i - 1], self.ring[(i + 1) % len(self.ring)]
            n0 = mul(outward(self.ring, prev, p), -1)
            n1 = mul(outward(self.ring, p, nxt), -1)
            c = add(p, mul(add(n0, n1), size / 2 + 0.05))
            if not point_in(self.ring, c):
                continue
            d = norm(sub(nxt, p))
            top = H + min(0.55, f['turretRiseM'])
            mb.box((c[0], (self.deck_y + top) / 2, c[1]), (size, top - self.deck_y, size), f['brick'], box_yaw(d),
                   mats={'top': f['granite']})
            mb.box((c[0], top - 0.15, c[1]), (size + 0.2, 0.3, size + 0.2), f['granite'], box_yaw(d))
        if not mb.empty():
            self.parts(mb, 'turrets')
            mb.build(self.exterior)

    # -- bookkeeping ----------------------------------------------------------

    def parts(self, mb, key):
        self.stats['exterior'][key] = self.stats['exterior'].get(key, 0) + sum(len(fc) - 2 for fc in mb.faces)

    def _shell_walls(self, t, openings=None):
        """One wall per footprint edge, inset by half its thickness, full height;
        `openings` maps an edge index to its openings."""
        walls = []
        for i in range(len(self.ring)):
            a, b = self.ring[i], self.ring[(i + 1) % len(self.ring)]
            if length(sub(b, a)) < 0.05:
                continue
            nin = mul(outward(self.ring, a, b), -1)
            aa, bb = add(a, mul(nin, t / 2)), add(b, mul(nin, t / 2))
            walls.append({'id': f'wall-{i}', 'a': {'x': aa[0], 'z': aa[1]}, 'b': {'x': bb[0], 'z': bb[1]},
                          'height': self.height, 'thickness': t, 'base': 0, 'openings': (openings or {}).get(i, [])})
        return walls

    def _floor(self):
        floor = {'top': 0}
        # the slab is the frame rectangle; keep it inside the footprint AABB as the kit does
        inset = Building._floor_inset(SimpleNamespace(spec=self.spec, poly=self.ring))
        if inset:
            floor['inset'] = inset
        return floor

    def physics(self):
        """The casemate mass is solid to the player and the parade is open: the outer
        walls with the sally port cut, the parade walls (set back into the casemates by
        half their thickness) with the passage's inner mouth cut, and the two passage
        walls between. Both cuts are `court` passages into the declared open court, not
        doors: there is no interior behind them. The grade floor as before."""
        t = 0.34
        sp = self.f['sallyPort']
        P = self.passage
        a, b = self.ring[self.front], self.ring[(self.front + 1) % len(self.ring)]
        L = length(sub(b, a))
        half = sp['widthM'] / 2
        head = min(sp['heightM'], self.height - 1.0)
        walls = self._shell_walls(t, {self.front: [{'id': 'sally-port', 't0': (L / 2 - half) / L, 't1': (L / 2 + half) / L,
                                                    'sill': 0, 'head': head, 'court': 'parade'}]})
        n = len(self.parade)
        for j in range(n):
            p, q = self.parade[j], self.parade[(j + 1) % n]
            Lp = length(sub(q, p))
            if Lp < 0.05:
                continue
            off = mul(outward(self.parade, p, q), t / 2)   # into the casemate mass
            pp, qq = add(p, off), add(q, off)
            openings = []
            if j == P['edge']:
                openings.append({'id': 'sally-port-inner', 't0': (P['s'] - half) / Lp, 't1': (P['s'] + half) / Lp,
                                 'sill': 0, 'head': head, 'court': 'parade'})
            walls.append({'id': f'parade-{j}', 'a': {'x': pp[0], 'z': pp[1]}, 'b': {'x': qq[0], 'z': qq[1]},
                          'height': self.deck_y, 'thickness': t, 'base': 0, 'openings': openings})
        for side in (-1, 1):
            o = mul(P['along'], side * (half + t / 2))
            s0 = add(add(P['mid'], o), mul(P['inward'], t))
            s1 = add(add(P['mid'], o), mul(P['inward'], P['depth'] - t))
            walls.append({'id': f'passage-{"l" if side < 0 else "r"}', 'a': {'x': s0[0], 'z': s0[1]}, 'b': {'x': s1[0], 'z': s1[1]},
                          'height': head, 'thickness': t, 'base': 0, 'openings': []})
        courts = [{'id': 'parade', 'polygon': [{'x': round(p[0], 4), 'z': round(p[1], 4)} for p in self.parade]}]
        return {'walls': walls, 'solids': [], 'steps': [], 'floor': self._floor(), 'courts': courts}

    def report_extra(self):
        return {'ground': 'casemates', 'upper': 'casemates',
                'fort': {'parade': [[round(p[0], 3), round(p[1], 3)] for p in self.parade],
                         'deckM': round(self.deck_y, 3), 'tiers': self.levels}}

    def export(self, out_dir):
        os.makedirs(out_dir, exist_ok=True)
        for o in bpy.context.scene.objects:
            o.select_set(False)
        layer = bpy.context.view_layer.layer_collection.children[self.exterior.name]
        bpy.context.view_layer.active_layer_collection = layer
        path = os.path.join(out_dir, 'exterior.glb')
        bpy.ops.export_scene.gltf(
            filepath=path, export_format='GLB', use_active_collection=True, use_visible=False,
            export_yup=True, export_apply=True, export_materials='EXPORT', export_image_format='NONE',
            export_texcoords=False, export_normals=True, export_tangents=False, export_gpu_instances=True,
            export_extras=False, export_cameras=False, export_lights=False)
        tris = 0
        for o in self.exterior.objects:
            if o.type == 'MESH':
                o.data.calc_loop_triangles()
                tris += len(o.data.loop_triangles)
        files = {'exterior': {'bytes': os.path.getsize(path), 'triangles': tris, 'libraryTriangles': 0,
                              'objects': len(self.exterior.objects)}}
        with open(os.path.join(out_dir, 'physics.json'), 'w') as fh:
            json.dump(self.physics(), fh, indent=1)
        report = {'files': files, 'parts': self.stats, 'stair': None, 'lights': [], 'storeys': self.storeys,
                  'fine': [], 'levels': self.levels, 'roofLevel': round(self.deck_y, 3),
                  'scope': 'exterior', **self.report_extra()}
        with open(os.path.join(out_dir, 'report.json'), 'w') as fh:
            json.dump(report, fh, indent=1)
        return report


# ---------------------------------------------------------------- the pavilion

PAVILION_DEFAULTS = {
    'wall': 'painted-stucco@ecd3a2',
    'eave': 'signal-red@8c4a40',
    'frame': 'signal-red@8c4a40',
    'glass': 'glass-dark',
    'door': 'glass-door',
    'roof': 'membrane-roof',
    'segmentM': 1.6,          # longest side of the drum along each surveyed arc (shaded smooth)
    'segments': 32,           # sides of the round upper drum and its eave
    'lowerM': 3.9,            # eave soffit over the ground storey
    'eaveThickM': 0.6,
    'eaveM': 2.2,             # lower eave reach past the drum (a canopy part)
    'upperInsetM': 3.2,       # the upper drum's setback from the lower drum
    'upperEaveM': 0.9,
    'upperEaveThickM': 0.5,
    'ribbon': {'sillM': 0.9, 'headM': 3.1, 'transomM': 2.35, 'mullionM': 1.4, 'endM': 0.12, 'depthM': 0.18, 'facing': 0.2},
    'doorway': {'widthM': 1.8, 'headM': 2.6},
    'portholes': {'count': 6, 'diameterM': 0.75, 'frameM': 0.12},
}


def fit_circle(ring):
    """Least-squares circle through the ring's vertices (algebraic fit): centre, radius."""
    import numpy as np
    A = np.array([[p[0], p[1], 1.0] for p in ring])
    b = np.array([p[0] * p[0] + p[1] * p[1] for p in ring])
    D, E, F = np.linalg.lstsq(A, b, rcond=None)[0]
    c = (D / 2, E / 2)
    return c, math.sqrt(F + c[0] ** 2 + c[1] ** 2)


def arc_ring(ring, radius, segment_m):
    """The round wall a curved survey describes: through every vertex, each edge an arc of
    the fitted `radius` bulging out of the ring (scripts/blender-buildings/curved-footprint.mjs:
    the overflow past each edge is exactly its sagitta). Returns the sampled ring, vertices
    included, in the ring's own order."""
    out = []
    n = len(ring)
    for i in range(n):
        a, b = ring[i], ring[(i + 1) % n]
        L = length(sub(b, a))
        half = L / 2
        k = max(1, int(math.ceil(L / segment_m)))
        out.append(a)
        if half >= radius or k == 1:
            for j in range(1, k):
                out.append(lerp2(a, b, j / k))
            continue
        nout = outward(ring, a, b)
        mid = lerp2(a, b, 0.5)
        centre = sub(mid, mul(nout, math.sqrt(radius * radius - half * half)))
        t0 = math.atan2(a[1] - centre[1], a[0] - centre[0])
        t1 = math.atan2(b[1] - centre[1], b[0] - centre[0])
        dt = (t1 - t0 + math.pi) % (2 * math.pi) - math.pi
        for j in range(1, k):
            t = t0 + dt * j / k
            out.append((centre[0] + radius * math.cos(t), centre[1] + radius * math.sin(t)))
    return out


def radial(ring, centre, delta):
    """`ring` pushed `delta` metres away from `centre` (inward when negative)."""
    out = []
    for p in ring:
        d = sub(p, centre)
        r = length(d)
        out.append(add(centre, mul(d, (r + delta) / r)))
    return out


def smooth(obj):
    """Shade every face smooth: a round drum's quads share their joint normals."""
    if obj is not None:
        for poly in obj.data.polygons:
            poly.use_smooth = True


class Pavilion(Fort):
    """A round two-drum pavilion on a footprint its style flags curved (`footprintCurved`).
    The ground-storey drum is the round wall the survey describes: through every surveyed
    vertex, each edge an arc of the vertices' fitted circle. It carries a ribbon of framed
    windows on its front half and a broad flat eave ring; a smaller round upper drum is set
    back from it with round windows and its own eave."""

    def __init__(self, spec, style):
        self.spec = spec
        self.style = style
        cfg = style.get('pavilion') or {}
        self.f = {**PAVILION_DEFAULTS, **cfg}
        for key in ('ribbon', 'doorway', 'portholes'):
            self.f[key] = {**PAVILION_DEFAULTS[key], **(cfg.get(key) or {})}
        self.ring = [v2(p) for p in spec['polygon']]
        self.height = float(spec['heightM'])
        self.front = int(spec['frame']['edgeIndex'])
        self.exterior = bpy.data.collections.new('exterior')
        bpy.context.scene.collection.children.link(self.exterior)
        self.stats = {'exterior': {}}
        self.scope = 'exterior'
        self.deck_y = self.height
        self.storeys = 2
        self.levels = [0.0, round(self.f['lowerM'] + self.f['eaveThickM'], 3)]
        if self.f['lowerM'] + self.f['eaveThickM'] > self.height - 1.5:
            raise SystemExit(f"build_fort.py: pavilion height {self.height} leaves no upper drum over a {self.f['lowerM']} m storey")
        if not style.get('footprintCurved'):
            raise SystemExit('build_fort.py: the pavilion builder needs a footprint its style flags curved (footprintCurved)')
        self.centre, self.drum_r = fit_circle(self.ring)
        self.drum = arc_ring(self.ring, self.drum_r, self.f['segmentM'])
        if self.drum_r - self.f['upperInsetM'] - self.f['upperEaveM'] < 1.0:
            raise SystemExit('build_fort.py: the upper drum and its eave do not fit inside the lower drum')
        # the drum turns with the ring's winding; side 0 is centred on the front edge's normal
        a, b = self.ring[self.front], self.ring[(self.front + 1) % len(self.ring)]
        self.front_n = outward(self.ring, a, b)
        self.winding = 1.0 if signed_area(self.ring) > 0 else -1.0

    def circle(self, radius):
        n = int(self.f['segments'])
        theta_f = math.atan2(self.front_n[1], self.front_n[0])
        t0 = theta_f - self.winding * math.pi / n
        return [(self.centre[0] + radius * math.cos(t0 + self.winding * 2 * math.pi * k / n),
                 self.centre[1] + radius * math.sin(t0 + self.winding * 2 * math.pi * k / n)) for k in range(n)]

    def physics(self, t=0.34, envelope_m=0.35):
        """The closed shell (exterior scope, no interior) along the drawn round wall: one wall
        per drum side, its outer face on the wall. The compiler keeps every part inside the
        footprint's bounds + 0.5 m, so where the arcs bulge further (the four extremes, up to
        ~0.6 m) the outer face is held at the bounds + `envelope_m`."""
        xs = [p[0] for p in self.ring]
        zs = [p[1] for p in self.ring]
        lo = (min(xs) - envelope_m, min(zs) - envelope_m)
        hi = (max(xs) + envelope_m, max(zs) + envelope_m)
        face = [(min(max(p[0], lo[0]), hi[0]), min(max(p[1], lo[1]), hi[1])) for p in self.drum]
        line = radial(face, self.centre, -t / 2)
        walls = []
        n = len(line)
        for i in range(n):
            a, b = line[i], line[(i + 1) % n]
            if length(sub(b, a)) < 0.05:
                continue
            walls.append({'id': f'drum-{i}', 'a': {'x': a[0], 'z': a[1]}, 'b': {'x': b[0], 'z': b[1]},
                          'height': self.height, 'thickness': t, 'base': 0, 'openings': []})
        return {'walls': walls, 'solids': [], 'steps': [], 'floor': self._floor()}

    def report_extra(self):
        return {'ground': 'pavilion', 'upper': 'pavilion',
                'pavilion': {'lowerM': self.f['lowerM'], 'upperInsetM': self.f['upperInsetM'],
                             'drumRadiusM': round(self.drum_r, 3), 'drumSides': len(self.drum),
                             'upperSegments': int(self.f['segments'])}}

    def build(self):
        f = self.f
        H = self.height
        rb = f['ribbon']
        y_base = 0.0
        y_top = f['lowerM']
        drum = self.drum
        n = len(drum)
        # the door: the drum side that faces the front edge most squarely
        door_i = max(range(n), key=lambda i: dot(outward(drum, drum[i], drum[(i + 1) % n]), self.front_n))
        shell = MeshBuilder('shell')
        trim = MeshBuilder('trim-frames')
        recesses = MeshBuilder('openings')
        for i in range(n):
            a, b = drum[i], drum[(i + 1) % n]
            L = length(sub(b, a))
            if L < 1e-3:
                continue
            d = norm(sub(b, a))
            nout = outward(drum, a, b)
            yaw = box_yaw(d)
            holes = []
            if i == door_i:
                dw = f['doorway']
                half = min(dw['widthM'], L - 2 * rb['endM']) / 2
                s0, s1 = L / 2 - half, L / 2 + half
                holes.append(self.recess(recesses, a, d, nout, s0, s1, y_base, dw['headM'], f['wall'], f['frame'], f['door'], depth=rb['depthM']))
                # a glazed transom over the door
                holes.append(self.recess(recesses, a, d, nout, s0, s1, dw['headM'] + 0.15, rb['headM'], f['wall'], f['frame'], f['glass'], depth=rb['depthM']))
            elif dot(nout, self.front_n) > rb['facing']:
                s0, s1 = rb['endM'], L - rb['endM']
                holes.append(self.recess(recesses, a, d, nout, s0, s1, rb['sillM'], rb['headM'], f['wall'], f['frame'], f['glass'], depth=rb['depthM']))
                count = max(1, int(round((s1 - s0) / rb['mullionM'])))
                for k in range(1, count):
                    q = add(add(a, mul(d, s0 + (s1 - s0) * k / count)), mul(nout, -rb['depthM'] / 2))
                    trim.box((q[0], (rb['sillM'] + rb['headM']) / 2, q[1]), (0.09, rb['headM'] - rb['sillM'], rb['depthM']), f['frame'], yaw)
                q = add(add(a, mul(d, (s0 + s1) / 2)), mul(nout, -rb['depthM'] / 2))
                trim.box((q[0], rb['transomM'], q[1]), (s1 - s0, 0.08, rb['depthM']), f['frame'], yaw)
            self.wall_face(shell, a, d, nout, 0.0, L, y_base, y_top, holes, lambda y0, y1: f['wall'])
        self.parts(shell, 'shell')
        self.parts(trim, 'frames')
        self.parts(recesses, 'openings')
        smooth(shell.build(self.exterior))
        for mb in (trim, recesses):
            mb.build(self.exterior)
        # the lower eave ring: soffit, fascia and top, out past the drum (a canopy part)
        y0, y1 = y_top, y_top + f['eaveThickM']
        outer = radial(drum, self.centre, f['eaveM'])
        upper = self.circle(self.drum_r - f['upperInsetM'])
        self._fill('canopy-eave-soffit', [outer, drum], y0, f['eave'], up=False)
        self._fill('canopy-eave-top', [outer, upper], y1, f['eave'])
        fascia = MeshBuilder('canopy-eave-fascia')
        self._band(fascia, outer, y0, y1, f['eave'])
        self.parts(fascia, 'canopy-eave-fascia')
        smooth(fascia.build(self.exterior))
        # the upper drum
        up = MeshBuilder('shell-upper')
        self._band(up, upper, y1, H, f['wall'])
        self.parts(up, 'shell-upper')
        smooth(up.build(self.exterior))
        # round windows on the upper drum, proud of its face
        ph = f['portholes']
        holes_mb = MeshBuilder('trim-portholes')
        count = max(0, int(ph['count']))
        ymid = (y1 + H - f['upperEaveThickM']) / 2
        for k in range(count):
            # spread round the ring by perimeter
            t = (k + 0.5) / count
            p, nrm, dd = self._ring_at(upper, t)
            for rad, mat, off in ((ph['diameterM'] / 2 + ph['frameM'], f['frame'], 0.02), (ph['diameterM'] / 2, f['glass'], 0.035)):
                pts = []
                for j in range(14):
                    ang = 2 * math.pi * j / 14
                    q = add(add(p, mul(dd, rad * math.cos(ang))), mul(nrm, off))
                    pts.append(B(q[0], ymid + rad * math.sin(ang), q[1]))
                holes_mb.quad(pts, mat, B(nrm[0], 0, nrm[1]))
        self.parts(holes_mb, 'portholes')
        holes_mb.build(self.exterior)
        # the upper eave and the roof
        uouter = self.circle(self.drum_r - f['upperInsetM'] + f['upperEaveM'])
        yu0 = H - f['upperEaveThickM']
        self._fill('trim-upper-soffit', [uouter, upper], yu0, f['eave'], up=False)
        self._fill('roof-upper', [uouter], H, f['roof'])
        ufascia = MeshBuilder('trim-upper-fascia')
        self._band(ufascia, uouter, yu0, H, f['eave'])
        self.parts(ufascia, 'upper-fascia')
        smooth(ufascia.build(self.exterior))
        for p in uouter:
            if not point_in(self.ring, p):
                raise SystemExit('build_fort.py: the upper eave leaves the footprint')

    def _band(self, mb, ring, y0, y1, mat):
        for i in range(len(ring)):
            a, b = ring[i], ring[(i + 1) % len(ring)]
            o = outward(ring, a, b)
            mb.quad([B(a[0], y0, a[1]), B(b[0], y0, b[1]), B(b[0], y1, b[1]), B(a[0], y1, a[1])], mat, B(o[0], 0, o[1]))

    def _ring_at(self, ring, t):
        lens = [length(sub(ring[(i + 1) % len(ring)], ring[i])) for i in range(len(ring))]
        target = t * sum(lens)
        for i, L in enumerate(lens):
            if target <= L or i == len(lens) - 1:
                a, b = ring[i], ring[(i + 1) % len(ring)]
                d = norm(sub(b, a))
                return add(a, mul(d, min(target, L))), outward(ring, a, b), d
            target -= L


def fort_main():
    args = parse_args()
    if args.scope != 'exterior':
        raise SystemExit('build_fort.py builds the exterior scope only (no casemate interiors yet)')
    with open(args.spec) as fh:
        spec = json.load(fh)
    style = {}
    if args.style and os.path.exists(args.style):
        with open(args.style) as fh:
            style = json.load(fh)
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    if style.get('builder') == 'pavilion':
        fort = Pavilion(spec, style)
        fort.build()
    else:
        fort = Fort(spec, style)
        fort.build_scarp()
        fort.build_parade()
        fort.build_barbette()
    report = fort.export(args.out)
    if args.render:
        render_fort_previews(fort, args.render)
    print('BLENDER_BUILDING_REPORT ' + json.dumps(report['files']))


def render_fort_previews(fort, out_dir):
    """Eevee stills for the builder's own look (NOT evidence; the game capture is)."""
    os.makedirs(out_dir, exist_ok=True)
    scene = bpy.context.scene
    engines = [e.identifier for e in bpy.types.RenderSettings.bl_rna.properties['engine'].enum_items]
    scene.render.engine = 'BLENDER_EEVEE_NEXT' if 'BLENDER_EEVEE_NEXT' in engines else 'BLENDER_EEVEE'
    scene.render.resolution_x = 1280
    scene.render.resolution_y = 800
    world = bpy.data.worlds.new('sky')
    world.use_nodes = True
    world.node_tree.nodes['Background'].inputs['Color'].default_value = (0.55, 0.65, 0.8, 1)
    scene.world = world
    sun = bpy.data.objects.new('sun', bpy.data.lights.new('sun', 'SUN'))
    sun.data.energy = 3.5
    sun.rotation_euler = (math.radians(50), 0, math.radians(35))
    scene.collection.objects.link(sun)
    cam = bpy.data.objects.new('cam', bpy.data.cameras.new('cam'))
    scene.collection.objects.link(cam)
    scene.camera = cam
    fr = fort.spec['frame']
    span = max(fr['width'], fr['depth'])
    H = fort.height
    shots = {
        'aerial-front': (B(0, span * 0.9, fr['depth'] / 2 + span * 0.9), B(0, 0, 0)),
        'aerial-back': (B(span * 0.3, span * 0.9, -fr['depth'] / 2 - span * 0.9), B(0, 0, 0)),
        'aerial-side': (B(span * 1.1, span * 0.7, 0), B(0, 0, 0)),
        'front-street': (B(fr['width'] * 0.3, 1.7, fr['depth'] / 2 + 28), B(0, H * 0.45, fr['depth'] / 2)),
        'parade': (B(0, 1.7, 0), B(0, H * 0.4, -fr['depth'] / 2)),
    }
    for name, (eye, target) in shots.items():
        cam.location = eye
        cam.rotation_euler = (target - eye).to_track_quat('-Z', 'Y').to_euler()
        cam.data.lens = 18 if name == 'parade' else 28
        scene.render.filepath = os.path.join(out_dir, f'{name}.png')
        bpy.ops.render.render(write_still=True)


fort_main()
