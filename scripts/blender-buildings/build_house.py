"""Headless Blender builder for one house or small flat block (style.json `"builder": "house"`).

    blender -b --factory-startup --python scripts/blender-buildings/build_house.py -- \
        --spec artifacts/buildings/<id>/spec.json --style artifacts/buildings/<id>/style.json \
        --out <dir> [--render <dir>] [--furniture-lib <dir> --library <library.json>]
        [--scope full|exterior]   (exterior: the shell only, as build_building.py)

The same contract as build_building.py (same spec, frame, kit materials,
physics.json, report.json, exterior / interior / fine-s<k> GLBs), for the
residential frontages that builder cannot express: a garage storey at the
sidewalk with an entry door or side gate beside it, living storeys over it,
box or angled bay windows, a front gable over a flat rear roof (the common
Outer Sunset section), a flat roof with a parapet or a projecting eave, rear
windows, fences and gates, and a dwelling programme on every storey (garage,
living, dining, kitchen, bedrooms, bath; or two flats a floor).

It reuses build_building.py's kit (MeshBuilder, wall_grid, partition, slabs,
stairs, furniture routing, export) by executing that file without its trailing
`main()` call, so neither builder's output changes when the other is used. The
pass id of a house covers both scripts (build.mjs).

Height contract: the shell's top is the LiDAR median height (the verifier's
rule); nothing may rise more than 0.6 m above it. A front gable's ridge is
therefore drawn at min(style ridge, height + 0.55) and the report records how
far that sits below the measured LiDAR peak.
"""

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
    raise SystemExit('build_house.py: build_building.py no longer ends with main(); update the import shim')
exec(compile(_src[:_cut], _BASE, 'exec'), globals())  # noqa: S102 - our own sibling script

# ---------------------------------------------------------------- preview colours

KIT_PREVIEW.update({
    'painted-clapboard': (0.9, 0.89, 0.86, 0.76, 0.0),
    'cedar-shingle': (0.42, 0.34, 0.27, 0.82, 0.0),
    'painted-shingle': (0.9, 0.89, 0.87, 0.74, 0.0),
})
INTERIOR.update({
    'int-tile-bath': (0.88, 0.88, 0.86, 0.3, 0.0),
    'int-counter': (0.82, 0.8, 0.76, 0.25, 0.0),
    'int-cabinet': (0.9, 0.89, 0.86, 0.45, 0.0),
    'int-appliance': (0.9, 0.9, 0.9, 0.3, 0.2),
    'int-carpet': (0.55, 0.52, 0.47, 1.0, 0.0),
    'int-garage-floor': (0.46, 0.45, 0.43, 0.85, 0.0),
    'int-copper': (0.6, 0.38, 0.25, 0.4, 0.8),
})
TEXTURED.update({'int-carpet': ('poly_wool_herringbone', 0.9), 'int-tile-bath': ('herringbone_parquet', 0.7)})

# ---------------------------------------------------------------- procedural fixtures
# Tier 5 (Blender procedural, last): the project's furniture library and the CC0
# sets it was drawn from hold no bathroom fixtures, laundry machines, water
# heaters or kitchen wall cabinets; these are simple fitted boxes, never street
# furniture. Every other piece routes to the library first (FALLBACK only when
# the off-repo library is absent).


def f_toilet(mb):
    mb.box((0, 0.2, -0.05), (0.38, 0.4, 0.5), 'int-ceramic')
    mb.box((0, 0.42, -0.02), (0.4, 0.05, 0.46), 'int-ceramic')
    mb.box((0, 0.6, -0.26), (0.42, 0.36, 0.18), 'int-ceramic')


def f_bathtub(mb):
    mb.box((0, 0.27, 0), (0.76, 0.54, 1.62), 'int-ceramic', mats={'top': 'int-ceramic'})
    mb.box((0, 0.52, 0), (0.6, 0.04, 1.46), 'int-tile-bath')


def f_vanity(mb):
    mb.box((0, 0.4, 0), (0.9, 0.8, 0.5), 'int-cabinet')
    mb.box((0, 0.83, 0), (0.94, 0.05, 0.52), 'int-counter')
    mb.box((0, 1.45, -0.23), (0.7, 0.8, 0.03), 'int-screen')


def f_washer(mb):
    mb.box((0, 0.43, 0), (0.6, 0.86, 0.62), 'int-appliance')
    mb.box((0, 0.5, 0.315), (0.34, 0.34, 0.02), 'int-screen')


def f_water_heater(mb):
    mb.box((0, 0.75, 0), (0.55, 1.5, 0.55), 'int-appliance')
    mb.box((0, 1.6, 0), (0.1, 0.2, 0.1), 'int-copper')


def f_kitchen_upper(mb):
    mb.box((0, 0.35, 0), (1.4, 0.7, 0.34), 'int-cabinet')


def f_tv_console(mb):
    mb.box((0, 0.25, 0), (1.6, 0.5, 0.42), 'int-wood')
    mb.box((0, 0.88, -0.05), (1.3, 0.74, 0.05), 'int-screen')


def f_counter_run(mb):
    mb.box((0, 0.44, 0), (1.4, 0.88, 0.62), 'int-cabinet')
    mb.box((0, 0.9, 0), (1.42, 0.04, 0.64), 'int-counter')


FURNITURE_BUILDERS.update({
    'toilet': f_toilet, 'bathtub': f_bathtub, 'vanity': f_vanity, 'washer': f_washer,
    'water-heater': f_water_heater, 'kitchen-upper': f_kitchen_upper, 'tv-console': f_tv_console,
    'counter-run': f_counter_run,
})
FALLBACK.update({k: k for k in ('toilet', 'bathtub', 'vanity', 'washer', 'water-heater', 'kitchen-upper',
                                 'tv-console', 'counter-run')})
FALLBACK_SIZE.update({
    'toilet': (0.42, 0.78, 0.7), 'bathtub': (0.76, 0.56, 1.62), 'vanity': (0.94, 0.85, 0.52),
    'washer': (0.6, 0.86, 0.62), 'water-heater': (0.55, 1.7, 0.55), 'kitchen-upper': (1.4, 0.7, 0.34),
    'tv-console': (1.6, 1.25, 0.42), 'counter-run': (1.42, 0.92, 0.64),
})
COARSE_MATERIAL.update({
    'bathtub': 'int-ceramic', 'vanity': 'int-cabinet', 'washer': 'int-appliance', 'water-heater': 'int-appliance',
    'tv-console': 'int-wood', 'counter-run': 'int-cabinet', 'sofa': 'int-fabric-blue', 'bed-frame': 'int-duvet',
})

GARAGE_DEPTH_M = 6.0
HALL_W = 1.05


# ---------------------------------------------------------------- the house


# House-only style keys. build_building.py has grown wrappers around
# Building.__init__ (and other methods) that read some of the same key names
# with a different schema (`storeysSeen` as {count, groundM, upperM}, `bays`,
# `floors`...). The base constructor therefore sees the style without them;
# House reads its own keys from the full style afterwards.
HOUSE_KEYS = ('builder', 'storeysSeen', 'groundM', 'roofLevelM', 'parapetM', 'roofForm', 'garages',
              'entry', 'windows', 'bays', 'rear', 'edgeRoles', 'rooftop', 'floors', 'groundRooms',
              'rearWall', 'rearTint', 'partyWall', 'partyTint', 'fence', 'fences', 'gate',
              'stoop', 'eave', 'parapet', 'programme', 'flats', 'groundBand', 'belts', 'beltMaterial',
              'beltTint', 'stairFrontM', 'stairSide', 'rake', 'soffit', 'streetEdges', 'edgeItems',
              'galleries', 'panels', 'wings', 'rearCorridor', 'rearDoor', 'garageDepthM', 'groundWindows',
              'windowsPerStorey', 'edgeFront')


class House(Building):
    def __init__(self, spec, style):
        super().__init__(spec, {k: v for k, v in style.items() if k not in HOUSE_KEYS})
        self.style = style
        s = style
        h = self.height
        roof = s.get('roofForm') or {}
        self.roof_form = roof.get('form', 'flat')
        # storey stack from the look (the roll often counts the garage storey
        # as a basement) under the LiDAR height. roofLevelM = top of the top
        # storey (the deck of a flat roof, the wall head under a gable's eave).
        seen = int(s.get('storeysSeen') or spec['storeys'])
        top = float(s.get('roofLevelM') or (h - float(s.get('parapetM', 0.45))))
        top = min(top, h - 0.05)
        ground = float(s.get('groundM', 2.65))
        upper = (top - ground) / (seen - 1) if seen > 1 else top
        if seen > 1 and upper < MIN_STOREY_M:
            raise SystemExit(f'house: {seen} storeys do not fit under roofLevelM {top} (upper {upper:.2f} m)')
        self.storeys = seen
        self.levels = [0.0] + [ground + k * upper for k in range(seen - 1)]
        self.roof_level = top
        self.spec = dict(self.spec, storeys=seen, floors={'groundM': ground, 'upperM': round(upper, 3), 'roofM': round(h - top, 3)})
        self.front = next(e for e in self.edges if e['role'] == 'front')
        fa, fb = v2(self.front['a']), v2(self.front['b'])
        self.front_z = max(fa[1], fb[1])
        self.rect = self._clean_rect(self.rect)
        self.holes = self._holes()
        self.door_frame_x = None
        self.report_notes = []

    # -- geometry helpers ------------------------------------------------

    def _inner_rect(self):
        """The rooms' rectangle. build_building shrinks the AABB evenly until
        its corners are inside, which on a house with a slanted side wall
        throws away a metre of the other side too. Here each side moves in on
        its own until at least 85 % of it lies inside the inset footprint; a
        short lightwell notch stays inside the rect and is routed around
        (_holes). `rect` in the style overrides it (frame metres) for an L."""
        forced = self.style.get('rect')
        if forced:
            return tuple(float(v) for v in forced)
        x0, x1, z0, z1 = self.ix0, self.ix1, self.iz0, self.iz1

        def frac(axis, value, lo, hi):
            n = 60
            hits = 0
            for i in range(n):
                t = lo + (hi - lo) * (i + 0.5) / n
                p = (value, t) if axis == 'x' else (t, value)
                hits += 1 if self._inside(p) else 0
            return hits / n
        for _ in range(400):
            fr = {'x0': frac('x', x0 + 0.02, z0, z1), 'x1': frac('x', x1 - 0.02, z0, z1),
                  'z0': frac('z', z0 + 0.02, x0, x1), 'z1': frac('z', z1 - 0.02, x0, x1)}
            side, worst = min(fr.items(), key=lambda kv: kv[1])
            if worst >= 0.85:
                break
            if side == 'x0':
                x0 += 0.05
            elif side == 'x1':
                x1 -= 0.05
            elif side == 'z0':
                z0 += 0.05
            else:
                z1 -= 0.05
        return (x0, x1, z0, z1)

    def _clean_rect(self, rect):
        return rect

    def _holes(self):
        """Axis-aligned boxes of the rect that lie OUTSIDE the inset footprint
        (lightwell notches, jogs): no room, partition or piece is placed there."""
        x0, x1, z0, z1 = self.rect
        step = 0.2
        cells = []
        zz = z0 + step / 2
        while zz < z1:
            xx = x0 + step / 2
            while xx < x1:
                if not self._inside((xx, zz)):
                    cells.append((xx, zz))
                xx += step
            zz += step
        if not cells:
            return []
        # cluster by z-contiguity (notches are few and small)
        cells.sort(key=lambda c: (c[1], c[0]))
        groups = []
        for c in cells:
            for g in groups:
                if abs(g['z1'] - c[1]) <= step * 1.5 and (g['x0'] - step * 1.5 <= c[0] <= g['x1'] + step * 1.5):
                    g['x0'], g['x1'] = min(g['x0'], c[0]), max(g['x1'], c[0])
                    g['z0'], g['z1'] = min(g['z0'], c[1]), max(g['z1'], c[1])
                    break
            else:
                groups.append({'x0': c[0], 'x1': c[0], 'z0': c[1], 'z1': c[1]})
        pad = step / 2 + PART_T
        return [(g['x0'] - pad, g['x1'] + pad, g['z0'] - pad, g['z1'] + pad) for g in groups]

    def part(self, walls, a, b, base, top, doors, mat='int-plaster'):
        """partition(), cut to the runs that lie inside the inset footprint (a
        slanted side wall or a lightwell notch would otherwise take it through
        the shell), with door centres kept where they were."""
        L = length(sub(b, a))
        if L < 0.3:
            return []
        d = norm(sub(b, a))
        step = 0.05
        runs, cur = [], None
        n = int(L / step) + 1
        for i in range(n + 1):
            sv = min(L, i * step)
            ok = self._inside(add(a, mul(d, sv)))
            if ok and cur is None:
                cur = sv
            if (not ok or i == n) and cur is not None:
                end = sv if ok else sv - step
                if end - cur >= 0.3:
                    runs.append((cur, end))
                cur = None
        solids = []
        for s0, s1 in runs:
            ds = [c - s0 for c in doors if s0 + DOOR_W / 2 <= c <= s1 - DOOR_W / 2]
            solids += partition(walls, add(a, mul(d, s0)), add(a, mul(d, s1)), base, top, ds, mat)
        return solids

    def front_span(self):
        """Frame x-range of the street face: the front edge plus any street
        edges lying on the front line (a jog). Roof parts over the front stay
        inside it, so a footprint that widens toward the rear never pushes an
        eave past the front corner."""
        xs = []
        streets = [str(k) for k in (self.style.get('streetEdges') or {})]
        for e in self.edges:
            if e['role'] == 'front' or str(e['index']) in streets:
                a, b = v2(e['a']), v2(e['b'])
                if min(a[1], b[1]) > self.front_z - 0.6:
                    xs += [a[0], b[0]]
        return min(xs), max(xs)

    def free(self, x, z, r=0.3):
        if not self._inside((x, z)):
            return False
        for hx0, hx1, hz0, hz1 in self.holes:
            if hx0 - r < x < hx1 + r and hz0 - r < z < hz1 + r:
                return False
        return True

    def at_edge(self, e, sv, off=0.0):
        a, b = v2(e['a']), v2(e['b'])
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        return add(add(a, mul(d, sv)), mul(nout, off))

    def frame_x_of(self, e, f):
        return self.at_edge(e, self.f2s(f, e['lengthM'], e))[0]

    # -- programme / stair -------------------------------------------------

    def ground_programme(self):
        return 'vacant'  # concrete garage floor; rooms lay their own finish

    def upper_programme(self):
        return self.style.get('programme', 'house')

    def plan_stair(self):
        """The stair core sits right behind the garage, on the entry side, and
        runs through every storey (build_building's two-flight U stair)."""
        x0, x1, z0, z1 = self.rect
        s = self.style
        entry = (s.get('entry') or s.get('gate') or {})
        if entry:
            ex = self.frame_x_of(self.front, (entry['from'] + entry['to']) / 2)
            self.door_frame_x = ex
            side = -1 if ex < (x0 + x1) / 2 else 1
        else:
            side = -1
        if s.get('stairSide') in (-1, 1):
            side = s['stairSide']
        width = 2 * FLIGHT_W + STAIR_GAP
        rise_max = max(self.storey_height(k) for k in range(self.storeys))
        steps = math.ceil(rise_max / RISER)
        per_flight = math.ceil(steps / 2)
        run = per_flight * TREAD
        depth = LANDING_D + run
        garage = float(s.get('garageDepthM', GARAGE_DEPTH_M))
        # a shallow house keeps the core inside: the garage gives way to it
        garage = max(2.5, min(garage, (z1 - z0) - depth - PART_T - 0.2))
        front = z1 - garage - PART_T
        if s.get('stairFrontM') is not None:
            front = z1 - float(s['stairFrontM'])
        # keep clear of lightwell notches: slide the core rearward past them
        sx0 = x0 if side < 0 else x1 - width
        for _ in range(60):
            clear = all(self.free(px, pz, 0.05) for px in (sx0 + 0.1, sx0 + width - 0.1)
                        for pz in frange(front - depth + 0.1, front - 0.05, 0.4))
            if clear:
                break
            if front - 0.25 - depth < z0 + 0.05:  # never slide past the rear wall
                front = z0 + 0.05 + depth
                break
            front -= 0.25
        self.stair = {'side': side, 'x0': sx0, 'x1': sx0 + width, 'z0': front - depth, 'z1': front,
                      'run': run, 'perFlight': per_flight}
        return self.stair

    # -- exterior -----------------------------------------------------------

    def edge_tops(self, e):
        """(s0, s1, top) segments along a -> b. A front gable holds the side
        walls at the eave for its depth; everywhere else the wall rises to the
        LiDAR height (a parapet around the flat roof)."""
        L = e['lengthM']
        if self.roof_form not in ('front-gable', 'front-hip', 'front-gambrel'):
            return [(0.0, L, self.height)]
        if e['role'] == 'front':
            return [(0.0, L, self.roof_level)]
        depth = float((self.style.get('roofForm') or {}).get('depthM', 5.0))
        zc = self.front_z - depth
        a, b = v2(e['a']), v2(e['b'])
        ina, inb = a[1] > zc + 1e-3, b[1] > zc + 1e-3
        if ina and inb:
            return [(0.0, L, self.roof_level)]
        if not ina and not inb:
            return [(0.0, L, self.height)]
        t = (zc - a[1]) / ((b[1] - a[1]) or 1e-9)
        sc = max(0.0, min(L, t * L))
        return [(0.0, sc, self.roof_level if ina else self.height), (sc, L, self.height if ina else self.roof_level)]

    def wall_mat_for(self, e):
        s = self.style
        if e['role'] == 'party':
            return self.tinted(s.get('partyWall', 'painted-stucco'), s.get('partyTint'))
        if e['role'] == 'front' or str(e['index']) in (s.get('streetEdges') or {}):
            return self.wall_material_at
        return self.tinted(s.get('rearWall', s['wall']), s.get('rearTint', s.get('wallTint')))

    def front_items(self):
        """Openings on the front: (kind, s0, s1, y0, y1, spec) in metres along a -> b."""
        s = self.style
        e = self.front
        L = e['lengthM']
        items = []

        def span(f0, f1):
            p, q = sorted((self.f2s(f0, L, e), self.f2s(f1, L, e)))
            return p, q
        for g in s.get('garages') or []:
            p, q = span(g['from'], g['to'])
            items.append(('garage', p, q, 0.0, float(g.get('headM', 2.3)), g))
        for key in ('entry', 'gate'):
            d = s.get(key)
            if d:
                p, q = span(d['from'], d['to'])
                items.append((key, p, q, float(d.get('sillM', 0.0)), float(d.get('headM', 2.25)), d))
        for w in s.get('windows') or []:
            p, q = span(w['from'], w['to'])
            k = int(w.get('storey', 1))
            base = self.levels[k] if k < self.storeys else self.roof_level
            items.append(('window', p, q, base + float(w.get('sill', 0.85)), base + float(w.get('head', 2.25)), w))
        # a bay opens the wall behind it on every storey it spans
        for bay in s.get('bays') or []:
            p, q = span(bay['from'], bay['to'])
            for k in range(int(bay.get('fromStorey', 1)), int(bay.get('toStorey', self.storeys - 1)) + 1):
                y0 = self.levels[k] + 0.06
                y1 = (self.levels[k + 1] if k + 1 < self.storeys else self.roof_level) - 0.3
                items.append(('bay-opening', p + 0.12, q - 0.12, y0, y1, bay))
        return items

    def side_items(self, e):
        """Rear and open-flank windows: evenly spaced per storey (party walls stay blind)."""
        if e['role'] in ('party', 'front'):
            return []
        s = self.style.get('rear') or {}
        L = e['lengthM']
        if L < 1.6:
            return []
        per = s.get('windowsPerStorey')
        ww = float(s.get('windowW', 1.1))
        n = int(per) if per is not None else max(1, int(L // 3.2))
        n = min(n, max(1, int((L - 0.6) // (ww + 0.6))))
        items = []
        for k in range(self.storeys):
            if k == 0 and not s.get('groundWindows', True):
                continue
            base = self.levels[k]
            top = (self.levels[k + 1] if k + 1 < self.storeys else self.roof_level)
            for i in range(n):
                c = L * (i + 0.5) / n
                if k == 0 and i == 0 and s.get('rearDoor', True) and e['role'] == 'rear' and L > 3.0:
                    items.append(('rear-door', c - 0.45, c + 0.45, 0.0, 2.1, {}))
                    continue
                items.append(('window', c - ww / 2, c + ww / 2, base + 0.9, min(top - 0.35, base + 2.2),
                              {'panes': s.get('panes', [1, 2])}))
        return items

    def x_range_at(self, z):
        """Frame x-extent of the footprint along the line z (slanted party
        walls make the house narrower toward the rear)."""
        xs = []
        P = self.poly
        for i in range(len(P)):
            a, b = P[i], P[(i + 1) % len(P)]
            if (a[1] - z) * (b[1] - z) <= 0 and abs(b[1] - a[1]) > 1e-9:
                t = (z - a[1]) / (b[1] - a[1])
                xs.append(a[0] + (b[0] - a[0]) * t)
        return (min(xs), max(xs)) if len(xs) >= 2 else None

    def front_like_items(self, e, sub):
        """A second street face on a jogged front (style `edgeFront`: {edge:
        {garages, entry, gate, windows, bays}}), dressed exactly like the front
        with fractions from the viewer's left facing that face. The survey
        often splits one street wall into two edges a few decimetres apart."""
        saved = (self.style, self.front)
        self.style = {**self.style, 'garages': sub.get('garages') or [], 'entry': sub.get('entry'),
                      'gate': sub.get('gate'), 'windows': sub.get('windows') or [], 'bays': sub.get('bays') or []}
        self.front = e
        try:
            return self.front_items()
        finally:
            self.style, self.front = saved

    def edge_items(self, e):
        """Explicit openings on a non-front edge (style `edgeItems`: {edge: [..]}),
        fractions from the viewer's left facing that face."""
        L = e['lengthM']
        items = []
        for it in (self.style.get('edgeItems') or {}).get(str(e['index']), []):
            p, q = sorted((self.f2s(it['from'], L, e), self.f2s(it['to'], L, e)))
            k = int(it.get('storey', 0))
            base = self.levels[k] if k < self.storeys else self.roof_level
            if it.get('kind') == 'door':
                kind = 'rear-door' if k == 0 else 'unit-door'
                items.append((kind, p, q, base, base + float(it.get('head', 2.1)), it))
            else:
                items.append(('window', p, q, base + float(it.get('sill', 0.9)), base + float(it.get('head', 2.2)), it))
        return items

    def build_exterior(self):
        s = self.style
        shell = MeshBuilder('shell')
        trim = MeshBuilder('trim')
        glass = MeshBuilder('glazing')
        frames = MeshBuilder('frames')
        doors = MeshBuilder('doors')
        n = len(self.poly)
        for i, e in enumerate(self.edges):
            a_out, b_out = self.poly[i], self.poly[(i + 1) % n]
            a_in, b_in = self.inner[i], self.inner[(i + 1) % n]
            L = e['lengthM']
            if e['role'] == 'front':
                items = self.front_items()
            elif str(e['index']) in (s.get('edgeFront') or {}):
                items = self.front_like_items(e, s['edgeFront'][str(e['index'])])
            elif str(e['index']) in (s.get('edgeItems') or {}):
                items = self.edge_items(e)
            else:
                items = self.side_items(e)
            ops = [(it[1], it[2], it[3], it[4]) for it in items]
            wall_mat = self.wall_mat_for(e)
            d = norm(sub(b_out, a_out))
            perp_in = self.inward(a_out, b_out)
            for s0, s1, top in self.edge_tops(e):
                ao = add(a_out, mul(d, s0))
                bo = add(a_out, mul(d, s1))
                ai = a_in if s0 <= 1e-6 else add(ao, mul(perp_in, WALL_T))
                bi = b_in if s1 >= L - 1e-6 else add(bo, mul(perp_in, WALL_T))
                local = [(p - s0, q - s0, y0, y1) for p, q, y0, y1 in ops if q > s0 + 1e-3 and p < s1 - 1e-3]
                wall_grid(shell, ao, bo, ai, bi, 0.0, top, local, wall_mat, 'int-plaster', wall_mat,
                          mat_top=s.get('coping', s['trim']), splits=[self.band_top] if self.band_top else ())
            self._dress(e, a_out, b_out, items, trim, glass, frames, doors)
            nin = perp_in
            pa = add(a_out, mul(nin, WALL_T / 2))
            pb = add(b_out, mul(nin, WALL_T / 2))
            door_ops = [it for it in items if it[0] in ('entry', 'gate', 'rear-door')]
            self.physics['walls'].append({
                'id': f'wall-{i}', 'a': {'x': pa[0], 'z': pa[1]}, 'b': {'x': pb[0], 'z': pb[1]},
                'height': self.height, 'thickness': WALL_T, 'base': 0,
                'openings': [{'id': f'{it[0]}-{i}-{j}', 't0': it[1] / L, 't1': it[2] / L, 'sill': 0, 'head': it[4]}
                             for j, it in enumerate(sorted(door_ops, key=lambda q: q[1]))],
            })
        self._house_bays(trim)
        self._galleries()
        self._roof(trim)
        self._fences()
        self._band_trim(trim)
        self._panels(trim)
        self._front_extras(trim)
        # shadow proxy (as build_building): drawn into the shadow map only
        proxy = MeshBuilder('shadow-proxy')
        ring = self._inset(WALL_T + 0.05)
        top = self.roof_level
        proxy.poly([B(p[0], top, p[1]) for p in ring], 'int-shadow-proxy', B(0, 1, 0))
        for i in range(len(ring)):
            p, q = ring[i], ring[(i + 1) % len(ring)]
            nout = mul(self.inward(self.poly[i], self.poly[(i + 1) % len(self.poly)]), -1)
            proxy.quad([B(p[0], 0.05, p[1]), B(q[0], 0.05, q[1]), B(q[0], top, q[1]), B(p[0], top, p[1])],
                       'int-shadow-proxy', B(nout[0], 0, nout[1]))
        proxy.build(self.exterior)
        self.stats['exterior'] = {}
        for m in (shell, trim, glass, frames, doors):
            obj = m.build(self.exterior)
            if obj:
                self.stats['exterior'][m.name] = len(m.faces)

    # -- openings --------------------------------------------------------------

    def _window(self, glass, frames, trim, at, yaw, s0, s1, y0, y1, spec, depth=WALL_T * 0.4, sill_trim=True):
        s = self.style
        w, h = s1 - s0, y1 - y0
        cs = (s0 + s1) / 2
        frame_mat = self.tinted(spec.get('frame', s['frame']), spec.get('frameTint'))
        g = at(cs, -depth)
        glass.box((g[0], (y0 + y1) / 2, g[1]), (w, h, 0.02), s['glass'], yaw)
        ft = 0.065
        cols, rows = (spec.get('panes') or [1, 1])[:2]
        # outer frame
        for sv in (s0 + ft / 2, s1 - ft / 2):
            p = at(sv, -depth + 0.03)
            frames.box((p[0], (y0 + y1) / 2, p[1]), (ft, h, 0.1), frame_mat, yaw)
        for yy in (y0 + ft / 2, y1 - ft / 2):
            p = at(cs, -depth + 0.03)
            frames.box((p[0], yy, p[1]), (w, ft, 0.1), frame_mat, yaw)
        # sashes / muntins
        for c in range(1, int(cols)):
            p = at(s0 + w * c / cols, -depth + 0.03)
            frames.box((p[0], (y0 + y1) / 2, p[1]), (0.05, h, 0.08), frame_mat, yaw)
        for r in range(1, int(rows)):
            p = at(cs, -depth + 0.03)
            frames.box((p[0], y0 + h * r / rows, p[1]), (w, 0.05, 0.08), frame_mat, yaw)
        # casing and sill
        if spec.get('casing', True):
            cw = 0.09
            for sv in (s0 - cw / 2, s1 + cw / 2):
                p = at(sv, 0.02)
                trim.box((p[0], (y0 + y1) / 2, p[1]), (cw, h + 2 * cw, 0.04), frame_mat, yaw)
            p = at(cs, 0.02)
            trim.box((p[0], y1 + cw / 2, p[1]), (w + 2 * cw, cw, 0.04), frame_mat, yaw)
        if sill_trim:
            p = at(cs, 0.04)
            trim.box((p[0], y0 - 0.03, p[1]), (w + 0.2, 0.06, 0.1), frame_mat, yaw)

    def _dress(self, e, a, b, items, trim, glass, frames, doors):
        s = self.style
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        yaw = box_yaw(d)

        def at(sv, off):
            return add(add(a, mul(d, sv)), mul(nout, off))

        for kind, s0, s1, y0, y1, spec in items:
            w = s1 - s0
            cs = (s0 + s1) / 2
            if kind == 'window':
                self._window(glass, frames, trim, at, yaw, s0, s1, y0, y1, spec)
                self.window_slots.append({'a': at(s0, -WALL_T), 'b': at(s1, -WALL_T), 'nin': mul(nout, -1),
                                          'sill': y0, 'head': y1, 'kind': 'punched'})
            elif kind == 'garage':
                mat = self.tinted(spec.get('material', 'painted-white'), spec.get('tint'))
                rec = 0.14
                p = at(cs, -rec)
                doors.box((p[0], (y0 + y1) / 2, p[1]), (w, y1 - y0, 0.05), mat, yaw)
                # jamb and head trim around the opening
                for sv in (s0 - 0.05, s0 + w + 0.05):
                    q = at(sv, 0.01)
                    doors.box((q[0], (y0 + y1) / 2, q[1]), (0.1, y1 - y0, 0.04), s['trim'], yaw)
                q = at(cs, 0.01)
                doors.box((q[0], y1 + 0.05, q[1]), (w + 0.2, 0.1, 0.04), s['trim'], yaw)
                pattern = spec.get('pattern', 'sectional')
                if pattern in ('sectional', 'panel'):
                    rows = 4
                    cols = 4 if pattern == 'panel' else 1
                    for r in range(1, rows):
                        q = at(cs, -rec + 0.03)
                        doors.box((q[0], y0 + (y1 - y0) * r / rows, q[1]), (w - 0.04, 0.035, 0.02), mat, yaw)
                    if pattern == 'panel':
                        for r in range(rows):
                            for c in range(cols):
                                q = at(s0 + w * (c + 0.5) / cols, -rec + 0.035)
                                doors.box((q[0], y0 + (y1 - y0) * (r + 0.5) / rows, q[1]),
                                          (w / cols - 0.16, (y1 - y0) / rows - 0.14, 0.02), mat, yaw)
                    # dark section joints so the door reads at street distance
                    # (a same-colour 2 cm relief vanishes in flat daylight)
                    for r in range(1, rows):
                        q = at(cs, -rec + 0.027)
                        doors.box((q[0], y0 + (y1 - y0) * r / rows - 0.022, q[1]), (w - 0.03, 0.012, 0.012), 'painted-steel-mid', yaw)
                    lites = spec.get('lites')  # a row of small glazed lites in the top section
                    if lites:
                        n = int(lites)
                        for c in range(n):
                            q = at(s0 + w * (c + 0.5) / n, -rec + 0.04)
                            doors.box((q[0], y1 - (y1 - y0) / rows * 0.5, q[1]), (w / n - 0.12, (y1 - y0) / rows * 0.45, 0.012), s['glass'], yaw)
                elif pattern == 'slat':
                    slats = max(6, int((y1 - y0) / 0.11))
                    for r in range(slats):
                        q = at(cs, -rec + 0.035)
                        doors.box((q[0], y0 + (y1 - y0) * (r + 0.5) / slats, q[1]), (w - 0.02, (y1 - y0) / slats - 0.012, 0.02),
                                  self.tinted(spec.get('material', 'painted-clapboard'), spec.get('slatTints', [spec.get('tint')])[r % len(spec.get('slatTints', [spec.get('tint')]))]), yaw)
                elif pattern == 'mural':
                    # a painted door: a few broad colour fields (described by kind, not copied)
                    fields = spec.get('fields') or []
                    for j, fld in enumerate(fields):
                        f0, f1, g0, g1, tint = fld
                        q = at(s0 + w * (f0 + f1) / 2, -rec + 0.03)
                        doors.box((q[0], y0 + (y1 - y0) * (g0 + g1) / 2, q[1]), (w * (f1 - f0), (y1 - y0) * (g1 - g0), 0.012),
                                  self.tinted('painted-white', tint), yaw)
                # head trim / lintel band over the garage
                lintel = spec.get('lintel')
                if lintel:
                    q = at(cs, 0.03)
                    trim.box((q[0], y1 + lintel.get('heightM', 0.3) / 2, q[1]), (w + 0.1, lintel.get('heightM', 0.3), 0.08),
                             self.tinted(lintel.get('material', 'brick-running-bond'), lintel.get('tint')), yaw)
            elif kind in ('entry', 'rear-door'):
                rec = float(spec.get('recessM', 0.12)) if kind == 'entry' else 0.1
                mat = self.tinted(spec.get('material', 'painted-white'), spec.get('tint'))
                # the leaf stands open against the reveal: the doorway is a real passage
                hinge = at(s0 + 0.05, -rec - 0.05)
                inward = mul(nout, -1)
                leaf_w = min(0.9, w - 0.1)
                c = add(hinge, mul(inward, leaf_w / 2))
                doors.box((c[0], (y0 + y1) / 2, c[1]), (0.045, y1 - y0 - 0.02, leaf_w), mat, yaw)
                if spec.get('glass', kind == 'entry'):
                    doors.box((c[0] + nout[0] * 0.0, y0 + (y1 - y0) * 0.62, c[1]), (0.05, (y1 - y0) * 0.45, leaf_w * 0.6), s['glass'], yaw)
                # casing
                cw = 0.1
                for sv in (s0 - cw / 2, s1 + cw / 2):
                    p = at(sv, 0.02)
                    trim.box((p[0], (y0 + y1) / 2, p[1]), (cw, y1 - y0 + cw, 0.05), self.tinted(s['frame'], None), yaw)
                p = at(cs, 0.02)
                trim.box((p[0], y1 + cw / 2, p[1]), (w + 2 * cw, cw, 0.05), s['frame'], yaw)
                # threshold slab in the doorway (the entries here are at grade)
                p = at(cs, -rec / 2)
                doors.box((p[0], 0.015, p[1]), (w, 0.03, max(0.1, rec)), self.tinted(spec.get('stepMaterial', 'cast-stone'), spec.get('stepTint')), yaw)
                if kind == 'entry':
                    # transom light over a recessed door
                    if spec.get('transom'):
                        g = at(cs, -rec + 0.02)
                        glass.box((g[0], y1 - 0.2, g[1]), (w - 0.1, 0.3, 0.02), s['glass'], yaw)
                    # house number plaque by the door (digits are signage kind, drawn as a plain plate)
                    p = at(s1 + 0.25, 0.03)
                    trim.box((p[0], 1.7, p[1]), (0.24, 0.1, 0.02), 'painted-steel', yaw)
                    if rec > 0.3:
                        # recessed vestibule: floor, side and head reveals in the wall colour
                        pass
            elif kind == 'unit-door':
                mat = self.tinted(spec.get('material', 'painted-white'), spec.get('tint'))
                p = at(cs, -0.08)
                doors.box((p[0], (y0 + y1) / 2, p[1]), (w, y1 - y0, 0.05), mat, yaw)
                cw = 0.08
                for sv in (s0 - cw / 2, s1 + cw / 2):
                    q = at(sv, 0.02)
                    trim.box((q[0], (y0 + y1) / 2, q[1]), (cw, y1 - y0 + cw, 0.04), s['frame'], yaw)
            elif kind == 'gate':
                gm = self.tinted(spec.get('material', 'painted-steel'), spec.get('tint'))
                bars = max(4, int(w / 0.11))
                for i in range(bars + 1):
                    p = at(s0 + 0.03 + (w - 0.06) * i / bars, -0.06)
                    doors.box((p[0], (y0 + y1) / 2, p[1]), (0.018, y1 - y0, 0.018), gm, yaw)
                for yy in (y0 + 0.1, y1 - 0.08, (y0 + y1) / 2):
                    p = at(cs, -0.06)
                    doors.box((p[0], yy, p[1]), (w, 0.04, 0.03), gm, yaw)
            elif kind == 'bay-opening':
                pass

    # -- bays (their own meshes: projectionExempt at runtime) -------------------

    def _house_bays(self, trim):
        s = self.style
        specs = s.get('bays') or []
        if not specs:
            return
        e = self.front
        a, b = v2(e['a']), v2(e['b'])
        L = e['lengthM']
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        mb = MeshBuilder('bay-windows')
        glass = MeshBuilder('bay-glazing')
        for bi, spec in enumerate(specs):
            p0, p1 = sorted((self.f2s(spec['from'], L, e), self.f2s(spec['to'], L, e)))
            w = p1 - p0
            c = (p0 + p1) / 2
            p = float(spec.get('projectionM', 0.6))
            shape = spec.get('shape', 'box')
            mat = self.tinted(spec.get('material', s['wall']), spec.get('tint', s.get('wallTint')))
            tmat = self.tinted(spec.get('trim', s['trim']), spec.get('trimTint'))
            k0 = int(spec.get('fromStorey', 1))
            k1 = min(self.storeys - 1, int(spec.get('toStorey', self.storeys - 1)))

            def at(sv, off):
                return add(add(a, mul(d, sv)), mul(nout, off))
            side = min(p, w / 3) if shape == 'angled' else 0.0
            ring = [at(c - w / 2, 0), at(c - w / 2 + side, p), at(c + w / 2 - side, p), at(c + w / 2, 0)]
            centre = at(c, p * 0.4)
            y_bottom = self.levels[k0]
            y_top = (self.levels[k1 + 1] if k1 + 1 < self.storeys else self.roof_level) - 0.02
            if spec.get('topM') is not None:
                y_top = float(spec['topM'])
            # skirt / corbel under the bay, a plate at each storey line, a cap on top
            self._prism(mb, ring, y_bottom - float(spec.get('skirtM', 0.3)), y_bottom + 0.06, mat)
            mb.poly([B(q[0], y_bottom + 0.061, q[1]) for q in ring], 'int-wood-floor', B(0, 1, 0))
            for k in range(k0 + 1, k1 + 1):
                self._prism(mb, ring, self.levels[k] - 0.1, self.levels[k] + 0.1, mat)
            cap = [ring[0], add(ring[1], mul(nout, 0.08)), add(ring[2], mul(nout, 0.08)), ring[3]]
            self._prism(mb, cap, y_top - 0.02, y_top + float(spec.get('capM', 0.18)), tmat)
            mb.poly([B(q[0], y_top - 0.03, q[1]) for q in ring], 'int-ceiling', B(0, -1, 0))
            for k in range(k0, k1 + 1):
                y0 = self.levels[k] + 0.1
                y1 = (self.levels[k + 1] - 0.1) if k + 1 <= k1 else y_top - 0.02
                wins = spec.get('windows') or {}
                for fi, (fa, fb) in enumerate(((ring[0], ring[1]), (ring[1], ring[2]), (ring[2], ring[3]))):
                    fl = length(sub(fb, fa))
                    if fl < 0.15:
                        continue
                    fd = norm(sub(fb, fa))
                    fn = (-fd[1], fd[0])
                    if dot(sub(lerp2(fa, fb, 0.5), centre), fn) < 0:
                        fn = (fd[1], -fd[0])
                    ia, ib = sub(fa, mul(fn, 0.1)), sub(fb, mul(fn, 0.1))
                    face = ('side', 'front', 'side')[fi]
                    count = int(wins.get(face, 1 if face == 'front' else (1 if fl > 0.45 else 0)))
                    sill = y0 + float(spec.get('sillM', 0.7))
                    head = y1 - float(spec.get('headGapM', 0.3))
                    ops = []
                    if count:
                        pier = float(spec.get('pierM', 0.12))
                        ww = (fl - pier * (count + 1)) / count
                        if ww > 0.2:
                            for j in range(count):
                                o0 = pier + j * (ww + pier)
                                ops.append((o0, o0 + ww, sill, head))
                    wall_grid(mb, fa, fb, ia, ib, y0, y1, ops, mat, 'int-plaster', mat, cap_top=False)
                    fyaw = box_yaw(fd)

                    def fat(sv, off, fa=fa, fd=fd, fn=fn):
                        return add(add(fa, mul(fd, sv)), mul(fn, off))
                    for o0, o1, sy, hy in ops:
                        self._window(glass, glass, mb, fat, fyaw, o0, o1, sy, hy,
                                     {'panes': spec.get('panes', [1, 1]), 'frame': spec.get('frameMaterial', s['frame']),
                                      'frameTint': spec.get('frameTint'), 'casing': False}, depth=0.04, sill_trim=True)
                        self.window_slots.append({'a': fat(o0, -0.1), 'b': fat(o1, -0.1), 'nin': (-fn[0], -fn[1]),
                                                  'sill': sy, 'head': hy, 'kind': 'bay'})
        mb.build(self.exterior)
        glass.build(self.exterior)

    # -- roof ---------------------------------------------------------------------

    def _galleries(self):
        """Open access galleries on court faces, kept inside the 0.45 m
        projection envelope: a deck edge, a solid balustrade and posts at each
        upper storey (the real galleries are deeper; see the report)."""
        specs = self.style.get('galleries') or []
        if not specs:
            return
        mb = MeshBuilder('gallery')
        for g in specs:
            e = self.edges[int(g['edge'])]
            a, b = v2(e['a']), v2(e['b'])
            d = norm(sub(b, a))
            nout = mul(self.inward(a, b), -1)
            yaw = box_yaw(d)
            L = e['lengthM']
            pj = min(0.42, float(g.get('projectionM', 0.42)))
            bal = self.tinted(g.get('material', 'painted-white'), g.get('tint'))
            post = self.tinted(g.get('postMaterial', 'painted-steel'), g.get('postTint'))
            deck = self.tinted(g.get('deckMaterial', 'concrete-precast'), None)
            mid = lerp2(a, b, 0.5)
            for k in range(int(g.get('fromStorey', 1)), self.storeys):
                y = self.levels[k]
                c = add(mid, mul(nout, pj / 2))
                mb.box((c[0], y - 0.1, c[1]), (L - 0.1, 0.2, pj), deck, yaw)
                c2 = add(mid, mul(nout, pj - 0.04))
                mb.box((c2[0], y + 0.5, c2[1]), (L - 0.1, 1.0, 0.06), bal, yaw)
                mb.box((c2[0], y + 1.03, c2[1]), (L - 0.06, 0.06, 0.1), post, yaw)
            for sv in frange(0.3, L - 0.2, float(g.get('postSpacingM', 3.0))):
                p = add(add(a, mul(d, sv)), mul(nout, pj - 0.04))
                y0 = self.levels[int(g.get('fromStorey', 1))] - 0.2
                mb.box((p[0], (y0 + self.height - 0.05) / 2, p[1]), (0.08, self.height - 0.05 - y0, 0.08), post, yaw)
        mb.build(self.exterior)

    def _roof(self, trim):
        s = self.style
        rf = s.get('roofForm') or {}
        roof = MeshBuilder('roof')
        rmat = self.tinted(rf.get('material', s.get('roof', 'asphalt-roof')), rf.get('tint'))
        top = self.roof_level
        # flat deck over the whole footprint (behind a gable it is the rear roof)
        roof.poly([B(p[0], top + 0.02, p[1]) for p in self.inner], rmat if rf.get('form') not in ('front-gable', 'front-hip', 'front-gambrel') else 'asphalt-roof', B(0, 1, 0))
        form = rf.get('form', 'flat')
        e = self.front
        a, b = v2(e['a']), v2(e['b'])
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        eave_ov = float(rf.get('eaveM', 0.3))
        if form == 'front-gable':
            depth = float(rf.get('depthM', 5.0))
            ridge_want = float(rf.get('ridgeM', self.height + 0.55))
            ridge = min(ridge_want, self.height + 0.55)
            peak = self.spec.get('peakHeightM')
            self.report_notes.append({'roof': 'front-gable', 'ridgeDrawnM': round(ridge, 2), 'ridgeWantedM': ridge_want,
                                      'cappedBy': 'LiDAR median + 0.6 m (authored contract)' if ridge < ridge_want else None})
            # the gable spans the street face
            xa, xb = self.front_span()
            side_ov = float(rf.get('sideOverhangM', 0.05))
            xa -= side_ov
            xb += side_ov
            zf = self.front_z + eave_ov
            zb = self.front_z - depth
            ye = top
            xm = (xa + xb) / 2
            wall = self.wall_material
            gmat = self.tinted(rf.get('gableMaterial', s['wall']), rf.get('gableTint', s.get('wallTint')))
            slope_t = 0.12
            # two slopes
            for x_edge in (xa, xb):
                roof.quad([B(x_edge, ye, zf), B(x_edge, ye, zb), B(xm, ridge, zb), B(xm, ridge, zf)], rmat, B(0, 1, 0))
                roof.quad([B(x_edge, ye - slope_t, zf), B(x_edge, ye - slope_t, zb), B(xm, ridge - slope_t, zb), B(xm, ridge - slope_t, zf)],
                          s.get('soffit', 'painted-white'), B(0, -1, 0))
                roof.quad([B(x_edge, ye - slope_t, zf), B(x_edge, ye, zf), B(x_edge, ye, zb), B(x_edge, ye - slope_t, zb)], s['trim'],
                          B(-1 if x_edge < xm else 1, 0, 0))
            # fascia / rake boards on the front
            for x_edge in (xa, xb):
                roof.quad([B(x_edge, ye - slope_t, zf), B(xm, ridge - slope_t, zf), B(xm, ridge, zf), B(x_edge, ye, zf)],
                          s.get('rake', s['trim']), B(0, 0, 1))
            # front gable triangle (wall face) and the rear one where it meets the flat roof
            fz = self.front_z
            roof_t = MeshBuilder('gable')
            inset = 0.0
            roof_t.quad([B(xa + side_ov, ye, fz), B(xb - side_ov, ye, fz), B(xm, ridge - slope_t, fz)], gmat, B(0, 0, 1))
            roof_t.quad([B(xa + side_ov, ye, zb), B(xb - side_ov, ye, zb), B(xm, ridge - slope_t, zb)], gmat, B(0, 0, -1))
            # pediment return: a boxed cornice across the eave line
            if rf.get('pediment', True):
                p = ((xa + xb) / 2, fz + 0.12)
                roof_t.box((p[0], ye + 0.06, p[1]), (xb - xa, 0.14, 0.26), s['trim'])
            # a vent or small window in the gable
            gw = rf.get('gableWindow')
            if gw:
                wy0 = ye + 0.25
                wy1 = min(ridge - 0.45, wy0 + float(gw.get('heightM', 0.7)))
                ww = float(gw.get('widthM', 0.9))
                if wy1 - wy0 > 0.25:
                    roof_t.box((xm, (wy0 + wy1) / 2, fz + 0.01), (ww, wy1 - wy0, 0.02), s['glass'])
                    roof_t.box((xm, (wy0 + wy1) / 2, fz + 0.02), (ww + 0.12, wy1 - wy0 + 0.12, 0.01), s['frame'])
            roof_t.build(self.exterior)
        elif form == 'front-gambrel':
            # A street-facing gambrel (barn) roof: a steep lower pitch from the
            # eave to a knee, a shallow upper pitch to the ridge, the gambrel end
            # walls front and rear. `kneeInsetF` (0-0.5 of the half width) and
            # `kneeRiseF` (share of the rise reached at the knee) shape it.
            # Ridge capped at LiDAR median + 0.55 m like the other forms.
            depth = float(rf.get('depthM', 6.0))
            ridge_want = float(rf.get('ridgeM', self.height + 0.55))
            ridge = min(ridge_want, self.height + 0.55)
            self.report_notes.append({'roof': 'front-gambrel', 'ridgeDrawnM': round(ridge, 2), 'ridgeWantedM': ridge_want,
                                      'cappedBy': 'LiDAR median + 0.6 m (authored contract)' if ridge < ridge_want else None})
            xa, xb = self.front_span()
            side_ov = float(rf.get('sideOverhangM', 0.12))
            zf = self.front_z + eave_ov
            zb = self.front_z - depth
            rx = self.x_range_at(zb + 0.01) or (xa, xb)
            ye = top
            xm = (xa + xb) / 2
            half = (xb - xa) / 2
            ki = float(rf.get('kneeInsetF', 0.22))
            kr = float(rf.get('kneeRiseF', 0.7))
            yk = ye + (ridge - ye) * kr
            slope_t = 0.12
            soffit = s.get('soffit', 'painted-white')
            for sgn in (-1, 1):
                xe = xm + sgn * (half + side_ov)
                xer = min(max(xe, rx[0]), rx[1]) if sgn > 0 else max(min(xe, rx[1]), rx[0])
                xk = xm + sgn * half * (1 - ki)
                roof.quad([B(xe, ye, zf), B(xer, ye, zb), B(xk, yk, zb), B(xk, yk, zf)], rmat, B(0, 1, 0))
                roof.quad([B(xk, yk, zf), B(xk, yk, zb), B(xm, ridge, zb), B(xm, ridge, zf)], rmat, B(0, 1, 0))
                roof.quad([B(xe, ye - slope_t, zf), B(xer, ye - slope_t, zb), B(xk, yk - slope_t, zb), B(xk, yk - slope_t, zf)], soffit, B(0, -1, 0))
                roof.quad([B(xe, ye - slope_t, zf), B(xe, ye, zf), B(xer, ye, zb), B(xer, ye - slope_t, zb)], s['trim'], B(sgn, 0, 0))
                # rake boards on the front gambrel end
                roof.quad([B(xe, ye - slope_t, zf), B(xk, yk - slope_t, zf), B(xk, yk, zf), B(xe, ye, zf)], s.get('rake', s['trim']), B(0, 0, 1))
                roof.quad([B(xk, yk - slope_t, zf), B(xm, ridge - slope_t, zf), B(xm, ridge, zf), B(xk, yk, zf)], s.get('rake', s['trim']), B(0, 0, 1))
            roof_t = MeshBuilder('gable')
            gmat = self.tinted(rf.get('gableMaterial', s['wall']), rf.get('gableTint', s.get('wallTint')))
            fz = self.front_z
            ka, kb = xm - half * (1 - ki), xm + half * (1 - ki)
            for z, nz, (x0, x1) in ((fz, 1, (xa, xb)), (zb, -1, (max(xa, rx[0]), min(xb, rx[1])))):
                pts = [B(x0, ye, z), B(x1, ye, z), B(min(kb, x1), yk - slope_t, z), B(xm, ridge - slope_t, z), B(max(ka, x0), yk - slope_t, z)]
                roof_t.quad([pts[0], pts[1], pts[2]], gmat, B(0, 0, nz))
                roof_t.quad([pts[0], pts[2], pts[3]], gmat, B(0, 0, nz))
                roof_t.quad([pts[0], pts[3], pts[4]], gmat, B(0, 0, nz))
            gw = rf.get('gableWindows') or []
            for w in gw:  # windows in the front gambrel end: [fromF, toF, sillAboveEaveM, headAboveEaveM]
                f0, f1, y0, y1 = w
                p0, p1 = xa + (xb - xa) * f0, xa + (xb - xa) * f1
                roof_t.box(((p0 + p1) / 2, ye + (y0 + y1) / 2, fz + 0.015), (p1 - p0, y1 - y0, 0.02), s['glass'])
                roof_t.box(((p0 + p1) / 2, ye + (y0 + y1) / 2, fz + 0.025), (p1 - p0 + 0.12, y1 - y0 + 0.12, 0.01), s['frame'])
            roof_t.build(self.exterior)
        elif form == 'front-hip':
            # A hipped roof over the front of the house (the common Outer Sunset
            # tract roof): equal-pitch slopes down to the street and both sides,
            # a short ridge running back, and the rear gable end standing on the
            # flat rear roof. Ridge capped like the gable (LiDAR median + 0.55 m).
            depth = float(rf.get('depthM', 5.0))
            ridge_want = float(rf.get('ridgeM', self.height + 0.55))
            ridge = min(ridge_want, self.height + 0.55)
            self.report_notes.append({'roof': 'front-hip', 'ridgeDrawnM': round(ridge, 2), 'ridgeWantedM': ridge_want,
                                      'cappedBy': 'LiDAR median + 0.6 m (authored contract)' if ridge < ridge_want else None})
            xa, xb = self.front_span()
            side_ov = float(rf.get('sideOverhangM', eave_ov))
            xa -= side_ov
            xb += side_ov
            zf = self.front_z + eave_ov
            zb = self.front_z - depth
            ye = top
            xm = (xa + xb) / 2
            half = (xb - xa) / 2
            run = min(half, max(0.3, zf - zb - 0.3))
            if run < half:  # a short roof: keep the pitch, lower the ridge
                ridge = ye + (ridge - ye) * run / half
            zr = zf - run
            slope_t = 0.12
            soffit = s.get('soffit', 'painted-white')
            # rear ends follow a slanted party wall inward (never past the footprint)
            rx = self.x_range_at(zb + 0.01) or (xa, xb)
            xra, xrb = max(xa, rx[0]), min(xb, rx[1])
            for x_edge, x_rear in ((xa, xra), (xb, xrb)):
                sgn = -1 if x_edge < xm else 1
                roof.quad([B(x_edge, ye, zf), B(x_rear, ye, zb), B(xm, ridge, zb), B(xm, ridge, zr)], rmat, B(0, 1, 0))
                roof.quad([B(x_edge, ye - slope_t, zf), B(x_rear, ye - slope_t, zb), B(xm, ridge - slope_t, zb), B(xm, ridge - slope_t, zr)],
                          soffit, B(0, -1, 0))
                roof.quad([B(x_edge, ye - slope_t, zf), B(x_edge, ye, zf), B(x_rear, ye, zb), B(x_rear, ye - slope_t, zb)], s['trim'],
                          B(sgn, 0, 0))
            roof.quad([B(xa, ye, zf), B(xb, ye, zf), B(xm, ridge, zr)], rmat, B(0, 1, 0))
            roof.quad([B(xa, ye - slope_t, zf), B(xb, ye - slope_t, zf), B(xm, ridge - slope_t, zr)], soffit, B(0, -1, 0))
            # front fascia board along the eave
            roof.quad([B(xa, ye - slope_t, zf), B(xb, ye - slope_t, zf), B(xb, ye, zf), B(xa, ye, zf)], s['trim'], B(0, 0, 1))
            # the rear gable end over the flat roof behind
            roof_t = MeshBuilder('gable')
            gmat = self.tinted(rf.get('gableMaterial', s['wall']), rf.get('gableTint', s.get('wallTint')))
            roof_t.quad([B(max(xa + side_ov, rx[0]), ye, zb), B(min(xb - side_ov, rx[1]), ye, zb), B(xm, ridge - slope_t, zb)], gmat, B(0, 0, -1))
            roof_t.build(self.exterior)
        elif form == 'flat':
            # a projecting eave slab (wide flat soffit) or just the parapet coping
            ov = float(rf.get('eaveM', 0.0))
            if ov > 0:
                fz = self.front_z
                xa, xb = self.front_span()
                y = self.height
                t = float(rf.get('fasciaM', 0.25))
                sov = float(rf.get('sideOverhangM', ov))
                roof.box(((xa + xb) / 2, y - t / 2, fz + ov / 2), (xb - xa + 2 * sov, t, ov), rf.get('fascia', s['trim']),
                         mats={'bottom': s.get('soffit', 'painted-white')})
                for edge_idx in rf.get('eaveEdges') or []:
                    ee = self.edges[int(edge_idx)]
                    pa, pb = v2(ee['a']), v2(ee['b'])
                    no = mul(self.inward(pa, pb), -1)
                    mid = add(lerp2(pa, pb, 0.5), mul(no, ov / 2))
                    dd = norm(sub(pb, pa))
                    roof.box((mid[0], y - t / 2, mid[1]), (ee['lengthM'] + 2 * sov, t, ov), rf.get('fascia', s['trim']), box_yaw(dd),
                             mats={'bottom': s.get('soffit', 'painted-white')})
        # rooftop bits: a vent stack or two and a skylight, inside the parapet
        rt = s.get('rooftop') or {}
        rb = MeshBuilder('rooftop')
        x0, x1, z0, z1 = self.rect
        for j, (fx, fz) in enumerate(rt.get('vents', [(0.3, 0.3)])):
            px, pz = x0 + (x1 - x0) * fx, z0 + (z1 - z0) * fz
            if self.free(px, pz):
                rb.box((px, top + 0.35, pz), (0.12, 0.7, 0.12), 'painted-steel-mid')
        for fx, fz, sw, sd in rt.get('skylights', []):
            px, pz = x0 + (x1 - x0) * fx, z0 + (z1 - z0) * fz
            if self.free(px, pz):
                rb.box((px, top + 0.12, pz), (sw, 0.2, sd), 'painted-white')
                rb.box((px, top + 0.23, pz), (sw - 0.1, 0.02, sd - 0.1), 'glass-dark')
        rb.build(self.exterior)
        roof.build(self.exterior)

    def _panels(self, trim):
        """Flat coloured panels on the front (spandrels between stacked windows)."""
        e = self.front
        a, b = v2(e['a']), v2(e['b'])
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        yaw = box_yaw(d)
        L = e['lengthM']
        for pnl in self.style.get('panels') or []:
            p0, p1 = sorted((self.f2s(pnl['from'], L, e), self.f2s(pnl['to'], L, e)))
            k = int(pnl.get('storey', 1))
            base = self.levels[k] if k < self.storeys else self.roof_level
            y0, y1 = base + float(pnl['y0']), base + float(pnl['y1'])
            c = add(add(a, mul(d, (p0 + p1) / 2)), mul(nout, 0.025))
            trim.box((c[0], (y0 + y1) / 2, c[1]), (p1 - p0, y1 - y0, 0.03), self.tinted(pnl.get('material', 'painted-white'), pnl.get('tint')), yaw)

    def _front_extras(self, trim):
        """Shutters beside front windows (`windows[].shutters`: {"tint", "widthM"}),
        small fabric awnings (`awnings`: [{from, to, y, depthM, tint}] over the
        entry storey) and a chimney flue (`chimney`: {fx, fz, heightM, dM} on the
        roof rect). Absent keys add nothing."""
        s = self.style
        e = self.front
        a, b = v2(e['a']), v2(e['b'])
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        yaw = box_yaw(d)
        L = e['lengthM']
        for w in s.get('windows') or []:
            sh = w.get('shutters')
            if not sh:
                continue
            p0, p1 = sorted((self.f2s(w['from'], L, e), self.f2s(w['to'], L, e)))
            k = int(w.get('storey', 1))
            base = self.levels[k] if k < self.storeys else self.roof_level
            y0, y1 = base + float(w.get('sill', 0.85)), base + float(w.get('head', 2.25))
            sw = float(sh.get('widthM', min(0.45, (p1 - p0) * 0.35)))
            mat = self.tinted(sh.get('material', 'painted-white'), sh.get('tint', '#2e2e2e'))
            for sc in (p0 - 0.06 - sw / 2, p1 + 0.06 + sw / 2):
                if 0.05 < sc - sw / 2 and sc + sw / 2 < L - 0.05:
                    c = add(add(a, mul(d, sc)), mul(nout, 0.04))
                    trim.box((c[0], (y0 + y1) / 2, c[1]), (sw, y1 - y0, 0.04), mat, yaw)
                    for f in (0.3, 0.7):  # louvre rails read at street distance
                        trim.box((c[0], y0 + (y1 - y0) * f, c[1] + 0.0), (sw * 0.9, 0.03, 0.05), mat, yaw)
        awn = MeshBuilder('awning')  # canopy part: its own projection allowance at runtime
        for aw in s.get('awnings') or []:
            p0, p1 = sorted((self.f2s(aw['from'], L, e), self.f2s(aw['to'], L, e)))
            y = float(aw.get('y', 2.35))
            dep = float(aw.get('depthM', 0.7))
            mat = self.tinted(aw.get('material', 'painted-white'), aw.get('tint', '#b03028'))
            ao, bo = add(a, mul(d, p0)), add(a, mul(d, p1))
            af, bf = add(ao, mul(nout, dep)), add(bo, mul(nout, dep))
            awn.quad([B(ao[0], y + 0.45, ao[1]), B(bo[0], y + 0.45, bo[1]), B(bf[0], y, bf[1]), B(af[0], y, af[1])], mat, B(0, 1, 0))
            awn.quad([B(af[0], y, af[1]), B(bf[0], y, bf[1]), B(bf[0], y - 0.22, bf[1]), B(af[0], y - 0.22, af[1])], mat,
                      B(nout[0], 0, nout[1]))
        awn.build(self.exterior)
        ch = s.get('chimney')
        if ch:
            x0, x1, z0, z1 = self.rect
            px, pz = x0 + (x1 - x0) * float(ch.get('fx', 0.2)), z0 + (z1 - z0) * float(ch.get('fz', 0.5))
            dm = float(ch.get('dM', 0.28))
            # the authored height contract: nothing above LiDAR median + 0.6 m
            hgt = min(float(ch.get('heightM', 1.4)), self.height + 0.55 - 0.4 - self.roof_level)
            if self.free(px, pz, 0.1):
                trim.box((px, self.roof_level + hgt / 2, pz), (dm, hgt, dm), self.tinted(ch.get('material', s['wall']), ch.get('tint', s.get('wallTint'))))
                trim.box((px, self.roof_level + hgt + 0.2, pz), (dm * 0.5, 0.4, dm * 0.5), 'painted-steel-mid')

    def _band_trim(self, trim):
        """Storey-line trim on the front (a painted belt at each floor line) when asked."""
        s = self.style
        if not s.get('belts'):
            return
        e = self.front
        a, b = v2(e['a']), v2(e['b'])
        d = norm(sub(b, a))
        nout = mul(self.inward(a, b), -1)
        yaw = box_yaw(d)
        L = e['lengthM']
        for k in s['belts']:
            y = self.levels[int(k)] if int(k) < self.storeys else self.roof_level
            p = add(lerp2(a, b, 0.5), mul(nout, 0.03))
            trim.box((p[0], y, p[1]), (L, 0.12, 0.06), self.tinted(s.get('beltMaterial', s['trim']), s.get('beltTint')), yaw)

    def _fences(self):
        """Fences and gates along named edges (drawn on the footprint line, inside the envelope)."""
        specs = self.style.get('fences') or []
        if not specs:
            return
        mb = MeshBuilder('fence')
        for f in specs:
            e = self.edges[int(f['edge'])]
            a, b = v2(e['a']), v2(e['b'])
            d = norm(sub(b, a))
            nout = mul(self.inward(a, b), -1)
            yaw = box_yaw(d)
            L = e['lengthM']
            s0, s1 = sorted((self.f2s(f.get('from', 0), L, e), self.f2s(f.get('to', 1), L, e)))
            h = float(f.get('heightM', 1.8))
            mat = self.tinted(f.get('material', 'wood-siding'), f.get('tint'))
            off = float(f.get('offsetM', 0.2))
            if f.get('kind', 'boards') == 'boards':
                boards = max(2, int((s1 - s0) / 0.15))
                for i in range(boards):
                    sv = s0 + (s1 - s0) * (i + 0.5) / boards
                    p = add(add(a, mul(d, sv)), mul(nout, off))
                    mb.box((p[0], h / 2, p[1]), ((s1 - s0) / boards - float(f.get('gapM', 0.012)), h, 0.022), mat, yaw)
                for yy in (0.3, h - 0.3):
                    p = add(add(a, mul(d, (s0 + s1) / 2)), mul(nout, off - 0.03))
                    mb.box((p[0], yy, p[1]), (s1 - s0, 0.09, 0.04), mat, yaw)
            else:
                bars = max(3, int((s1 - s0) / 0.12))
                for i in range(bars + 1):
                    p = add(add(a, mul(d, s0 + (s1 - s0) * i / bars)), mul(nout, off))
                    mb.box((p[0], h / 2, p[1]), (0.02, h, 0.02), mat, yaw)
                for yy in (0.1, h - 0.06):
                    p = add(add(a, mul(d, (s0 + s1) / 2)), mul(nout, off))
                    mb.box((p[0], yy, p[1]), (s1 - s0, 0.04, 0.04), mat, yaw)
        mb.build(self.exterior)

    # -- interior: a dwelling on every storey --------------------------------------

    def _bands(self, k):
        """Front band (street), middle band (beside the stair) and rear band."""
        x0, x1, z0, z1 = self.rect
        st = self.stair
        return {'front': (z0 if False else st['z1'] + PART_T / 2, z1), 'middle': (st['z0'], st['z1']), 'rear': (z0, st['z0'] - PART_T / 2)}

    def _ground(self, walls):
        x0, x1, z0, z1 = self.rect
        st = self.stair
        top = self.storey_top(0)
        s = self.style
        side = st['side']
        # entry hall from the door (or gate) to the stair, along the stair side
        dx = self.door_frame_x if self.door_frame_x is not None else (st['x0'] + st['x1']) / 2
        entry = s.get('entry') or s.get('gate') or {}
        ew = abs(self.frame_x_of(self.front, entry.get('to', 0.3)) - self.frame_x_of(self.front, entry.get('from', 0.2))) if entry else 1.0
        half = max(HALL_W / 2, ew / 2 + 0.2)
        hx0, hx1 = dx - half, dx + half
        if side < 0:
            hx0 = x0
        else:
            hx1 = x1
        # garage: the rest of the front, back to the stair core
        gz0 = st['z1'] + PART_T / 2
        gx0, gx1 = (hx1 + PART_T / 2, x1) if side < 0 else (x0, hx0 - PART_T / 2)
        wall_x = hx1 if side < 0 else hx0
        # hall | garage partition with a door into the garage, garage back wall
        self.physics['solids'] += self.part(walls, (wall_x, z1), (wall_x, gz0), 0.0, top, [max(0.6, (z1 - gz0) - 1.0)])
        # the garage's back wall has a door onto the hall beside the stair (the
        # only way from the street to the ground-floor rear rooms but the stair)
        ga, gb = (gx0, gx1) if side < 0 else (gx1, gx0)
        hall_mid = (st['x1'] + HALL_W / 2) if side < 0 else (st['x0'] - HALL_W / 2)
        back_door = [abs(hall_mid - ga)] if min(gx0, gx1) + 0.5 < hall_mid < max(gx0, gx1) - 0.5 else []
        self.physics['solids'] += self.part(walls, (ga, gz0), (gb, gz0), 0.0, top, back_door)
        # garage furnishing: shelving on the far wall, boxes, water heater and laundry at the back
        self.floor_finish(0, (hx0, hx1, gz0, z1), 'int-tile-floor')
        far_x = gx1 - 0.3 if side < 0 else gx0 + 0.3
        facing = -math.pi / 2 if side < 0 else math.pi / 2
        for zz in frange(gz0 + 1.2, z1 - 1.8, 1.1):
            if self.free(far_x, zz):
                self.piece('shelving', far_x, zz, facing, 0.0)
        for j, (fx, fz, kind) in enumerate(((0.8, 0.35, 'cardboard-box'), (0.78, 0.45, 'cardboard-box'), (0.72, 0.3, 'crate'))):
            px = gx0 + (gx1 - gx0) * (fx if side < 0 else 1 - fx)
            pz = gz0 + (z1 - gz0) * fz
            if self.free(px, pz):
                self.piece(kind, px, pz, 0.3 * j, 0.0)
        heater_x = gx1 - 0.45 if side < 0 else gx0 + 0.45
        self.P('water-heater', heater_x, gz0 + 0.45, math.pi, 0)
        self.hang('ceiling-lamp', (gx0 + gx1) / 2, (gz0 + z1) / 2, top, 0.0)
        self.add_light((gx0 + gx1) / 2, top - 0.4, (gz0 + z1) / 2, 0, intensity=8.0, distance=8.0, priority=1)
        # hall: runner and a lamp
        self.rug((hx0 + hx1) / 2, (gz0 + z1) / 2, max(0.7, hx1 - hx0 - 0.5), max(0.8, z1 - gz0 - 1.2), 0.0, 0.0, 0)
        self.hang('ceiling-lamp', (hx0 + hx1) / 2, (gz0 + z1) / 2, top, 0.0)
        self.add_light((hx0 + hx1) / 2, top - 0.3, (gz0 + z1) / 2, 0, intensity=10.0, distance=7.0, priority=3)
        # beside the stair and behind it: the ground-floor rooms of the style
        self._rear_rooms(walls, 0, s.get('groundRooms', ['laundry', 'family']))
        self._wings(walls)

    def floor_finish(self, k, r, mat):
        rx0, rx1, rz0, rz1 = r
        mb = self.soft_for(k)
        base = self.levels[k]
        ring = clip_ring_to_rect(self.inner, (rx0, rx1, rz0, rz1))
        if len(ring) >= 3:
            mb.poly([B(p[0], base + 0.025, p[1]) for p in ring], mat, B(0, 1, 0))

    def _rear_rooms(self, walls, k, rooms):
        """rooms: [middle, rear...]: the room beside the stair, then the rear band split across its width."""
        x0, x1, z0, z1 = self.rect
        st = self.stair
        base = self.levels[k]
        top = self.storey_top(k)
        side = st['side']
        # hall strip beside the stair, joining the front band to the rear band
        hx0, hx1 = (st['x1'], st['x1'] + HALL_W) if side < 0 else (st['x0'] - HALL_W, st['x0'])
        mx0, mx1 = (hx1 + PART_T / 2, x1) if side < 0 else (x0, hx0 - PART_T / 2)
        mz0, mz1 = st['z0'], st['z1']
        rz1 = st['z0'] - PART_T / 2
        # partitions: hall | middle room (door), middle room front and back
        mid_wall_x = hx1 if side < 0 else hx0
        self.physics['solids'] += self.part(walls, (mid_wall_x, mz0), (mid_wall_x, mz1), base, top, [(mz1 - mz0) / 2])
        front_door = [] if k == 0 else []
        self.physics['solids'] += self.part(walls, (mx0, mz1), (mx1, mz1), base, top, front_door)
        # the rear band's front wall, across the house, a door off the hall
        door_s = (hx0 + hx1) / 2 - x0
        self.physics['solids'] += self.part(walls, (x0, rz1), (x1, rz1), base, top, [door_s])
        # stair side wall on the hall (open at the arrival end)
        middle = rooms[0] if rooms else 'closet'
        self.room(middle, (mx0, mx1, mz0, mz1), k, door=(mid_wall_x, (mz0 + mz1) / 2))
        rear = rooms[1:] or ['bedroom']
        n = len(rear)
        if self.style.get('rearCorridor') and n > 1:
            # a long rear band (flats): the hall runs on as a corridor to the
            # back wall and the rooms line up along it, one door each
            cz = [rz1 - (rz1 - z0) * i / n for i in range(n + 1)]
            doors = [rz1 - (cz[i] + cz[i + 1]) / 2 for i in range(n)]
            for wx_ in ((hx1, hx0) if side < 0 else (hx0, hx1)):
                self.physics['solids'] += self.part(walls, (wx_, rz1), (wx_, z0), base, top, doors)
            ox0, ox1 = (x0, hx0 - PART_T / 2) if side < 0 else (hx1 + PART_T / 2, x1)
            for i, kind in enumerate(rear):
                za, zb = cz[i + 1] + (PART_T / 2 if i < n - 1 else 0), cz[i] - (PART_T / 2 if i else 0)
                if i:
                    self.physics['solids'] += self.part(walls, (mx0, cz[i]), (mx1, cz[i]), base, top, [])
                    if ox1 - ox0 > 0.5:
                        self.physics['solids'] += self.part(walls, (ox0, cz[i]), (ox1, cz[i]), base, top, [])
                self.room(kind, (mx0, mx1, za, zb), k, door=(mid_wall_x, (za + zb) / 2))
                if ox1 - ox0 > 1.3:
                    self.room('closet', (ox0, ox1, za, zb), k, door=(hx0 if side < 0 else hx1, (za + zb) / 2))
            self.floor_finish(k, (hx0, hx1, z0, rz1), 'int-wood-floor')
            self.hang('ceiling-lamp', (hx0 + hx1) / 2, (z0 + rz1) / 2, top, base)
            return
        # split the rear band across its width; each part keeps a rear window,
        # and each partition between them has a door near the front, so the
        # parts beyond the hall are reached through their neighbour
        cuts = [x0 + (x1 - x0) * i / n for i in range(n + 1)]
        for i, kind in enumerate(rear):
            rx0, rx1 = cuts[i] + (PART_T / 2 if i else 0), cuts[i + 1] - (PART_T / 2 if i < n - 1 else 0)
            if i:
                self.physics['solids'] += self.part(walls, (cuts[i], z0), (cuts[i], rz1), base, top, [max(0.6, rz1 - z0 - 0.9)])
            door_x = min(max((hx0 + hx1) / 2, rx0 + 0.6), rx1 - 0.6)
            self.room(kind, (rx0, rx1, z0, rz1 - PART_T / 2), k, door=(door_x, rz1))

    def _wings(self, walls):
        """Extra rectangles of an L or U footprint (style `wings`, frame metres):
        flats side by side along the wing's long axis on every storey."""
        for wi, w in enumerate(self.style.get('wings') or []):
            x0, x1, z0, z1 = (float(v) for v in w['rect'])
            units = int(w.get('units', 2))
            along_x = (x1 - x0) >= (z1 - z0)
            for k in range(self.storeys):
                base = self.levels[k]
                top = self.storey_top(k)
                if along_x:
                    cuts = [x0 + (x1 - x0) * i / units for i in range(units + 1)]
                    for i in range(1, units):
                        self.physics['solids'] += self.part(walls, (cuts[i], z0), (cuts[i], z1), base, top, [])
                    for i in range(units):
                        r = (cuts[i] + (PART_T / 2 if i else 0), cuts[i + 1] - (PART_T / 2 if i < units - 1 else 0), z0, z1)
                        self.room(w.get('room', 'flat'), r, k, door=((r[0] + r[1]) / 2, z1))
                else:
                    cuts = [z0 + (z1 - z0) * i / units for i in range(units + 1)]
                    for i in range(1, units):
                        self.physics['solids'] += self.part(walls, (x0, cuts[i]), (x1, cuts[i]), base, top, [])
                    for i in range(units):
                        r = (x0, x1, cuts[i] + (PART_T / 2 if i else 0), cuts[i + 1] - (PART_T / 2 if i < units - 1 else 0))
                        self.room(w.get('room', 'flat'), r, k, door=(x1, (r[2] + r[3]) / 2))

    def _upper(self, walls, k):
        s = self.style
        x0, x1, z0, z1 = self.rect
        st = self.stair
        base = self.levels[k]
        top = self.storey_top(k)
        plan = (s.get('floors') or {}).get(str(k)) or {}
        front = plan.get('front', 'living')
        rooms = plan.get('rooms', ['bath', 'kitchen'])
        # front band: the stair arrives into it (open), one room or two across
        fz0 = st['z1'] + PART_T / 2
        fronts = front if isinstance(front, list) else [front]
        n = len(fronts)
        cuts = [x0 + (x1 - x0) * i / n for i in range(n + 1)]
        for i, kind in enumerate(fronts):
            fx0, fx1 = cuts[i] + (PART_T / 2 if i else 0), cuts[i + 1] - (PART_T / 2 if i < n - 1 else 0)
            if i:
                self.physics['solids'] += self.part(walls, (cuts[i], fz0 + 1.4), (cuts[i], z1), base, top, [])
            self.room(kind, (fx0, fx1, fz0 + (1.4 if n > 1 else 0.0), z1), k, door=((st['x0'] + st['x1']) / 2, fz0))
        if n > 1:
            # a landing hall across the front of the stair
            self.physics['solids'] += self.part(walls, (x0, fz0 + 1.4), (x1, fz0 + 1.4), base, top,
                                                [((cuts[i] + cuts[i + 1]) / 2) - x0 for i in range(n)])
        self._rear_rooms(walls, k, rooms)

    # -- rooms ------------------------------------------------------------------

    def room(self, kind, r, k, door=None):
        rx0, rx1, rz0, rz1 = r
        if rx1 - rx0 < 1.2 or rz1 - rz0 < 1.2:
            return
        base = self.levels[k]
        top = self.storey_top(k)
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        w, dpt = rx1 - rx0, rz1 - rz0
        fn = getattr(self, f'_room_{kind.replace("-", "_")}', None)
        if fn is None:
            raise SystemExit(f'house: unknown room kind {kind}')
        fn(r, k, base, top, door)
        # curtains on this room's windows
        for slot in self.window_slots:
            mx, mz = lerp2(slot['a'], slot['b'], 0.5)
            if not (base < slot['sill'] < top + 0.3) or kind in ('garage', 'bath', 'laundry', 'closet', 'kitchen', 'hall'):
                continue
            # a bay's panes stand outside the footprint: curtains there would
            # leave the building's plan (the interior containment gate)
            ends = [add(add(slot[e], mul(slot['nin'], 0.2)), mul(norm(sub(slot['b'], slot['a'])), sg * 0.3))
                    for e, sg in (('a', -1), ('b', 1))]
            if not all(self._inside(q, self.poly) for q in ends):
                continue
            if rx0 - 0.9 < mx < rx1 + 0.9 and rz0 - 0.9 < mz < rz1 + 0.9:
                self.curtains(slot, base, k)

    def P(self, kind, x, z, yaw, k, **kw):
        """Place a piece only where the floor exists (not in a lightwell notch)."""
        if not self.free(x, z, 0.15):
            return False
        self.piece(kind, x, z, yaw, self.levels[k], **kw)
        return True

    def _toward(self, r, door):
        """Which rect side the door is on, to keep the swing clear."""
        return door

    def _room_living(self, r, k, base, top, door):
        rx0, rx1, rz0, rz1 = r
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        self.floor_finish(k, r, 'int-wood-floor')
        # sofa facing the street windows, back to the room; coffee table, chair, console, rug
        sz = rz1 - min(3.0, (rz1 - rz0) * 0.55)
        self.P('sofa', cx, sz, math.pi, k)
        self.P('pillows', cx, sz + 0.05, math.pi, k, y=base + 0.42, solid=False, coarse=False)
        self.P('coffee-table', cx, sz + 1.05, math.pi / 2, k)
        self.rug(cx, sz + 0.95, min(2.6, rx1 - rx0 - 0.6), 1.9, 0.0, base, k)
        self.P('lounge-chair', rx0 + 0.7, rz1 - 1.0, math.pi * 0.8, k)
        self.P('side-table', rx0 + 0.55, rz1 - 1.8, 0.0, k)
        self.P('tv-console', rx1 - 0.3, sz + 1.0, -math.pi / 2, k)
        self.P('plant-medium', rx1 - 0.5, rz1 - 0.5, 0.0, k, solid=False, coarse=False)
        self.P('shelving', rx0 + 0.3, max(rz0 + 0.8, sz - 0.2), math.pi / 2, k)
        self.book_rows(rx0 + 0.3, max(rz0 + 0.8, sz - 0.2), math.pi / 2, base, k, width=0.85, shelves=(0.85, 1.25))
        self.wall_frame(cx, rz0 + 0.07, (0, 1), base, k, height=1.6)
        self.hang('ceiling-lamp', cx, cz, top, base)
        self.add_light(cx, top - 0.4, cz, k, intensity=16.0, distance=8.0, priority=3)

    def _room_family(self, r, k, base, top, door):
        rx0, rx1, rz0, rz1 = r
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        self.floor_finish(k, r, 'int-carpet')
        self.P('sofa', cx, rz0 + 0.55, 0.0, k)
        self.P('coffee-table', cx, rz0 + 1.6, math.pi / 2, k)
        self.P('tv-console', cx, rz1 - 0.3, math.pi, k)
        self.P('writing-desk', rx1 - 0.6, cz, -math.pi / 2, k)
        self.P('cafe-chair', rx1 - 1.1, cz, math.pi / 2, k, solid=False, coarse=False)
        self.P('desk-lamp', rx1 - 0.55, cz + 0.25, 0.0, k, y=base + 0.54, solid=False, coarse=False)
        self.rug(cx, rz0 + 1.5, min(2.4, rx1 - rx0 - 0.6), 1.8, 0.0, base, k)
        self.hang('ceiling-lamp', cx, cz, top, base)
        self.add_light(cx, top - 0.4, cz, k, intensity=12.0, distance=7.0, priority=2)

    def _room_dining(self, r, k, base, top, door):
        rx0, rx1, rz0, rz1 = r
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        self.floor_finish(k, r, 'int-wood-floor')
        self.P('cafe-table', cx, cz, 0.0, k)
        for ang in (0, math.pi / 2, math.pi, -math.pi / 2):
            self.P('cafe-chair', cx + 0.62 * math.sin(ang), cz + 0.62 * math.cos(ang), ang + math.pi, k, solid=False, coarse=False)
        self.P('dresser', rx0 + 0.35, cz, math.pi / 2, k)
        self.P('plant-small', rx0 + 0.35, cz + 0.3, 0.0, k, y=base + 1.18, solid=False, coarse=False)
        self.hang('pendant', cx, cz, top, base, drop=1.0)
        self.add_light(cx, top - 0.6, cz, k, intensity=12.0, distance=6.0, priority=2)

    def _room_kitchen(self, r, k, base, top, door):
        rx0, rx1, rz0, rz1 = r
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        self.floor_finish(k, r, 'int-tile-floor')
        # an L of base cabinets on the wall away from the door, range in the run, fridge at the end
        run_x = rx1 - 0.34 if (door and door[0] < cx) else rx0 + 0.34
        yaw = -math.pi / 2 if run_x > cx else math.pi / 2
        zz = rz0 + 0.8
        placed = 0
        while zz < rz1 - 0.8:
            kind = 'range' if placed == 1 else 'counter-run'
            if kind == 'range':
                self.P('range', run_x - (0.06 if run_x > cx else -0.06), zz, yaw, k)
            else:
                self.P('counter-run', run_x, zz, yaw, k, fit=(1.42, None, None))
            self.P('kitchen-upper', run_x + (0.14 if run_x > cx else -0.14), zz, yaw, k, y=base + 1.5, solid=False)
            zz += 1.45
            placed += 1
        self.P('fridge', run_x - (0.05 if run_x > cx else -0.05), rz1 - 0.5, yaw, k)
        if rx1 - rx0 > 3.0:
            self.P('cafe-table', cx + (-0.4 if run_x > cx else 0.4), cz, 0.0, k)
            self.P('cafe-chair', cx + (-1.0 if run_x > cx else 1.0), cz, math.pi / 2 if run_x > cx else -math.pi / 2, k, solid=False, coarse=False)
        self.hang('ceiling-lamp', cx, cz, top, base)
        self.add_light(cx, top - 0.35, cz, k, intensity=14.0, distance=6.0, priority=2)

    def _room_bedroom(self, r, k, base, top, door):
        rx0, rx1, rz0, rz1 = r
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        self.floor_finish(k, r, 'int-carpet')
        w, dpt = rx1 - rx0, rz1 - rz0
        bed_w = 1.5 if w > 3.0 else 1.0
        bed_l = 2.05
        # headboard on the side wall away from the door, bed across the room
        head_left = not (door and door[0] < cx)
        bx = (rx0 + 0.06 + bed_l / 2) if head_left else (rx1 - 0.06 - bed_l / 2)
        yaw = -math.pi / 2 if head_left else math.pi / 2
        if w < bed_l + 1.0:
            # narrow room: head on the inner wall instead
            bx, yaw = cx, 0.0
            bz = rz0 + 0.06 + bed_l / 2
            self.P('bed-frame', bx, bz, yaw, k, fit=(bed_w + 0.1, None, bed_l + 0.05))
            self.bedding(bx, bz, yaw, base, k, bed_w, bed_l)
            ns = [(bx - bed_w / 2 - 0.33, rz0 + 0.3), (bx + bed_w / 2 + 0.33, rz0 + 0.3)]
        else:
            bz = cz
            self.P('bed-frame', bx, bz, yaw, k, fit=(bed_w + 0.1, None, bed_l + 0.05))
            self.bedding(bx, bz, yaw, base, k, bed_w, bed_l)
            hx = rx0 + 0.3 if head_left else rx1 - 0.3
            ns = [(hx, bz - bed_w / 2 - 0.33), (hx, bz + bed_w / 2 + 0.33)]
        for nx, nz in ns:
            if rx0 + 0.2 < nx < rx1 - 0.2 and rz0 + 0.2 < nz < rz1 - 0.2:
                self.P('nightstand', nx, nz, yaw, k)
        self.bedside_lamp(ns[0][0], ns[0][1], base + 0.62, k)
        self.P('dresser', cx, rz1 - 0.34 if dpt > 3.2 else rz0 + 0.34, math.pi if dpt > 3.2 else 0.0, k)
        self.rug(bx, bz, 1.6, 2.2, yaw, base, k)
        self.wall_frame(cx, rz0 + 0.07, (0, 1), base, k, height=1.7)
        self.hang('ceiling-lamp', cx, cz, top, base)
        self.add_light(cx, top - 0.4, cz, k, intensity=10.0, distance=6.0, priority=1)

    def _room_bath(self, r, k, base, top, door):
        rx0, rx1, rz0, rz1 = r
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        self.floor_finish(k, r, 'int-tile-bath')
        far_x = rx1 - 0.4 if (door and door[0] < cx) else rx0 + 0.4
        yaw = -math.pi / 2 if far_x > cx else math.pi / 2
        self.P('bathtub', far_x, cz, 0.0, k)
        self.P('toilet', cx, rz0 + 0.38, 0.0, k)
        self.P('vanity', cx, rz1 - 0.3, math.pi, k)
        self.hang('ceiling-lamp', cx, cz, top, base)
        self.add_light(cx, top - 0.3, cz, k, intensity=6.0, distance=4.0, priority=0)

    def _room_laundry(self, r, k, base, top, door):
        rx0, rx1, rz0, rz1 = r
        cx, cz = (rx0 + rx1) / 2, (rz0 + rz1) / 2
        self.floor_finish(k, r, 'int-tile-floor')
        self.P('washer', cx - 0.35, rz0 + 0.4, 0.0, k)
        self.P('washer', cx + 0.3, rz0 + 0.4, 0.0, k)
        self.P('shelving', cx, rz1 - 0.3, math.pi, k)
        self.P('cardboard-box', rx0 + 0.4, rz1 - 0.5, 0.2, k)
        self.hang('ceiling-lamp', cx, cz, top, base)

    def _room_closet(self, r, k, base, top, door):
        rx0, rx1, rz0, rz1 = r
        self.floor_finish(k, r, 'int-wood-floor')
        self.P('shelving', (rx0 + rx1) / 2, rz0 + 0.3, 0.0, k)
        self.P('cardboard-box', rx0 + 0.4, rz1 - 0.4, 0.4, k)

    def _room_flat(self, r, k, base, top, door):
        """A whole small flat in one band: living at the window end, a kitchen
        run on one side wall, a bedroom behind a partition at the inner end."""
        rx0, rx1, rz0, rz1 = r
        dpt = rz1 - rz0
        walls = MeshBuilder(f'flat-walls-{k}-{int(rz0 * 10)}')
        street_end = rz1 > (self.rect[2] + self.rect[3]) / 2
        if dpt > 7.0:
            split = rz0 + dpt * 0.42 if street_end else rz1 - dpt * 0.42
            door_s = (rx1 - rx0) * 0.25
            self.physics['solids'] += self.part(walls, (rx0, split), (rx1, split), base, top, [door_s])
            walls.build(self.interior)
            if street_end:
                self._room_living((rx0, (rx0 + rx1) / 2 + 0.6, split + PART_T / 2, rz1), k, base, top, door)
                self._room_kitchen(((rx0 + rx1) / 2 + 0.6, rx1, split + PART_T / 2, rz1), k, base, top, (rx0, split))
                self._room_bedroom((rx0, rx1, rz0, split - PART_T / 2), k, base, top, (rx0 + door_s, split))
            else:
                self._room_living((rx0, (rx0 + rx1) / 2 + 0.6, rz0, split - PART_T / 2), k, base, top, door)
                self._room_kitchen(((rx0 + rx1) / 2 + 0.6, rx1, rz0, split - PART_T / 2), k, base, top, (rx0, split))
                self._room_bedroom((rx0, rx1, split + PART_T / 2, rz1), k, base, top, (rx0 + door_s, split))
        else:
            self._room_living(r, k, base, top, door)

    # -- output -------------------------------------------------------------------

    def export(self, out_dir):
        report = super().export(out_dir)
        report['house'] = {'roofForm': self.roof_form, 'notes': self.report_notes, 'holes': self.holes,
                           'storeysSeen': self.storeys, 'roofLevelM': self.roof_level}
        with open(os.path.join(out_dir, 'report.json'), 'w') as fh:
            json.dump(report, fh, indent=1)
        return report


# ---------------------------------------------------------------- main


def house_main():
    args = parse_args()
    with open(args.spec) as fh:
        spec = json.load(fh)
    style = dict(DEFAULT_STYLE)
    style.update({'cornice': False, 'beltCourse': False, 'windowHoods': False, 'trim': 'painted-white',
                  'frame': 'painted-white', 'wall': 'painted-clapboard'})
    if args.style and os.path.exists(args.style):
        with open(args.style) as fh:
            user = json.load(fh)
        for k, v in user.items():
            if isinstance(v, dict) and isinstance(style.get(k), dict):
                style[k] = {**style[k], **v}
            else:
                style[k] = v
    if args.furniture_lib and args.library and os.path.exists(args.library):
        with open(args.library) as fh:
            lib = json.load(fh)
        LIBRARY['dir'] = args.furniture_lib
        LIBRARY['pieces'] = {k: v for k, v in lib.get('pieces', {}).items()
                             if os.path.exists(os.path.join(args.furniture_lib, v['file']))}
    if args.textures and os.path.isdir(args.textures):
        LIBRARY['textures'] = args.textures
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    h = House(spec, style)
    h.scope = args.scope
    h.plan_stair()
    h.build_exterior()
    if args.scope == 'exterior':
        # the same exterior-scope path as build_building.main(): no interior,
        # closed doors, opaque glass-shell panes (export() swaps the glazing)
        EXTERIOR_SCOPE['seethrough'] = {g for g in args.seethrough_glass.split(',') if g}
        EXTERIOR_SCOPE['glass'] = args.exterior_glass
        exterior_scope_close_doors(h)
    else:
        h.build_interior()
    report = h.export(args.out)
    if args.render:
        render_previews(h, args.render)
    print('BLENDER_BUILDING_REPORT ' + json.dumps(report['files']))


house_main()
