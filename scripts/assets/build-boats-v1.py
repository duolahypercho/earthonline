"""Build the moored-boat fleet: four types, two trims each, three LODs each.

Run headless from the repository root:

  blender -b --factory-startup --python scripts/assets/build-boats-v1.py -- \
      [--output public/assets/boats] [--bodies sailYacht-a,jetSki-b] [--no-provenance]

Everything comes from the parameters in this file: hull lines, deck and
superstructure, rig and gear, and one shared texture atlas painted from seeded
noise. No .blend, mesh or image is read; the reference photos listed in
REFERENCE_PHOTOS were looked at for shape and colour and are never loaded.

Frame (glTF): +Y up, metres, bow toward +Z, port toward +X. The origin is on
the design waterline (y = 0 is the static waterline) at the middle of the
waterline length, on the centreline, so a body placed at sea level floats at
its design draft. Authoring frame in this file: x is the distance aft of the
stem head, b is the offset to port, z is the height above the waterline. A
point (x, b, z) goes to Blender (b, x - xm, z), xm being the midpoint of the
waterline, and the exporter's Y-up conversion maps Blender -Y (the bow) onto
glTF +Z.

Hull lines: stations of a lofted section (cubic bezier from the keel to the
sheer for round-bilge hulls; keel, chine and flared topsides for planing
hulls). The underwater body of the sailing yacht and the motor yacht is
scaled so that the hull displaces the published displacement at the published
waterline; the jet ski's draft is solved from its published dry weight plus a
full tank. Dimensions and their sources are in DIMENSION_SOURCES and are
written to the provenance file.

Materials: one material per body, `BoatAtlas`: base colour and
metallic-roughness from one 1024 px atlas (JPEG). The atlas holds a painted
hull-side panel per body (waterline, antifouling, boot and sheer stripes,
grime and rust streaks, paint wear, all placed in metres along the hull) and
shared tiles for deck, canvas, glass, metal, rubber, rope and paint. Every GLB
embeds the same atlas bytes, so the eight bodies and their LODs can share one
texture. LOD1 and LOD2 are rebuilt from the same parameters at lower sampling
(LOD2 keeps the silhouette only); nothing is decimated.

Outputs: <type>-<variant>-lod{0,1,2}.glb and boats-v1.provenance.json in the
output folder. Re-run this script to regenerate digests; never edit them.
"""
import argparse
import hashlib
import json
import math
import os
import shutil
import struct
import sys
import tempfile
from pathlib import Path

import bmesh
import bpy
import numpy as np

ROOT = Path(__file__).resolve().parents[2]
GENERATOR = 'scripts/assets/build-boats-v1.py'
DEFAULT_OUT = ROOT / 'public' / 'assets' / 'boats'
SEAWATER = 1025.0          # kg/m3
ATLAS_PX = 1024
MR_PX = 512
JPEG_QUALITY = 88
SHARP_DEG = 35.0
TYPES = ('sailYacht', 'motorYacht', 'jetSki', 'fishingBoat')

# ---------------------------------------------------------------- sources ---
# Each dimension the hulls use, where it came from and what the page gave.
DIMENSION_SOURCES = [
    {'id': 'sail-c34', 'subject': 'sailYacht: 34 ft fin-keel masthead sloop (Catalina 34 Mk I class)',
     'url': 'https://en.wikipedia.org/wiki/Catalina_34',
     'gave': 'LOA 10.52 m, LWL 9.09 m, beam 3.58 m, draft 1.70 m (standard fin keel), displacement 5,420 kg, '
             'ballast 2,268 kg, I 13.41 m, J 4.11 m, P 11.73 m, E 3.58 m; internally mounted spade rudder; '
             'specs cited there from sailboatdata.com and sailboat.guide',
     'used': 'hull length, waterline length, beam, keel draft, displacement for the canoe-body depth, '
             'mast position (J aft of the forestay), mast height (I above the sheer), boom height (P), boom length (E)'},
    {'id': 'sail-c34-tallrig', 'subject': 'sailYacht: same class, tall-rig variant (cross-check)',
     'url': 'https://boat-specs.com/catalina-yachts/catalina-34-mkii-tall-rig',
     'gave': 'hull length 10.52 m, waterline 9.09 m, beam 3.58 m, draft 1.7 m, I 14.02 m, J 4.11 m, P 12.34 m, E 3.66 m',
     'used': 'cross-check of hull figures; air draft not published there'},
    {'id': 'sail-airdraft', 'subject': 'sailYacht: air draft',
     'url': 'https://c34.org/bbs/index.php?msg=80740',
     'gave': 'owners estimate about 50 ft (15.2 m) air draft for the tall rig, mast base about 1.5 ft below the sheer '
             'estimated; method: I plus deck height at the mast plus antenna',
     'used': 'masthead at sheer + I + 0.22 m (14.7 m above the waterline), VHF antenna above it; standard rig'},
    {'id': 'motor-c42ss', 'subject': 'motorYacht: 42 ft flybridge sport sedan (Carver 42 Super Sport class)',
     'url': 'https://www.denisonyachtsales.com/used-carver-yachts-for-sale-old/boat-reviews-%E2%80%93-carver-yachts/boat-review-42-carver-super-sport-2007/',
     'gave': "LOA 43'7\" (13.28 m) with platform, beam 13'11\" (4.24 m), draft 46\" (1.17 m), bridge clearance "
             "19'5\" (5.92 m) with arch, dry weight 33,650 lb (15,263 kg) with fuel and water, headroom 6'6\"; "
             'flybridge, hardtop over the cockpit',
     'used': 'overall length incl. platform and bow pulpit, beam, running-gear draft, arch and radar top, '
             'displacement for the hull depth'},
    {'id': 'motor-sedanbridge', 'subject': 'motorYacht: class cross-check (44 ft sedan bridge)',
     'url': 'https://www.yachtbuyer.com/en-us/sea-ray/new/44-sedan-bridge',
     'gave': "LOA 45'5\", beam 14', draft (full load) 3'8\", dry weight 28,499 lb, flybridge",
     'used': 'cross-check of beam and draft for the class'},
    {'id': 'jetski-vx', 'subject': 'jetSki: three-seat personal watercraft (Yamaha VX class)',
     'url': 'https://global.yamaha-motor.com/business/waverunner/products/vx/spec.html',
     'gave': 'length 3.37 m, width 1.24 m, height 1.20 m, dry weight 326 kg, fuel 70 l, 1-3 riders',
     'used': 'length incl. nozzle, beam, height keel to mirror top; draft solved for 326 kg + 52 kg fuel '
             '(70 l x 0.74 kg/l) in seawater'},
    {'id': 'fish-co26-003', 'subject': 'fishingBoat: 40 ft fiberglass combination troll/crab vessel',
     'url': 'https://dockstreetbrokers.com/vessels/co26-003',
     'gave': "40'x12'x6'8\" fiberglass combination vessel (1975), rigged for troll and crab, sliding troll block "
             'davits, articulating davit with crab block, gurdies, 4 fish holds',
     'used': 'hull length 12.2 m, gear list (trolling poles, crab davit and block, gurdies)'},
    {'id': 'fish-cr26-008', 'subject': 'fishingBoat: 43 ft fiberglass crabber (cross-check)',
     'url': 'https://dockstreetbrokers.com/vessels/cr26-008',
     'gave': "43'x14.5'x6.5' fiberglass crabber (1989), crab block, trolling poles, seine boom, radar",
     'used': 'beam and draft range for the class (beam 3.8 m and keel draft 1.75 m chosen between the two)'},
    {'id': 'fish-hullform', 'subject': 'fishingBoat: troller hull form',
     'url': 'https://blog.tadroberts.ca/2014/11/troller-hull-lines/',
     'gave': "40' troller 10'3\" beam, 4'6\" draft to the design waterline, 20,700 lb design displacement; "
             'drag to the keel, long parallel midbody, vertical topsides amidships, hard bilge well under water, '
             'considerable deadrise, flare under the sheer forward, reverse in the sections aft',
     'used': 'section shapes, keel drag, flare and deadrise tables'},
    {'id': 'crab-pot', 'subject': 'fishingBoat: Dungeness crab pots',
     'url': 'https://discoverwcseafood.msi.ucsb.edu/how-its-harvested/wild-caught/pots',
     'gave': 'pots measure three to four feet across, rubber-wrapped steel frame with wire mesh, hauled with a '
             'hydraulic crab block',
     'used': 'pot diameter 0.95 m, rubber-wrapped rims, mesh walls, crab block on the davit; height 0.36 m '
             '(14 in, NOAA Fishery Leaflet 419 as summarised in search; that PDF did not render here)'},
]

REFERENCE_PHOTOS = [
    {'page': 'https://commons.wikimedia.org/wiki/File:Catalina_34_Mark_II_sailboat_Dream_Catcher_2503.jpg',
     'author': 'Ahunt', 'licence': 'CC0-1.0', 'bytes': 518752,
     'sha256': 'd43e052d7bf2aea3333413334877ee6fe2af3fbc5d3f60bf1a16f83864d16cec',
     'use': 'sailYacht moored at a dock, port bow quarter: sheer, trunk, ports, stripes, sail cover, lifelines'},
    {'page': 'https://commons.wikimedia.org/wiki/File:Catalina_34_Mark_II_sailboat_Waterparke_1905.jpg',
     'author': 'Ahunt', 'licence': 'CC0-1.0', 'bytes': 1375504,
     'sha256': '8fb47dc2fd8a5c698bfa2a9409544b61eb7b9cabc330784fa1d93e7c5a5f1ada',
     'use': 'sailYacht stern quarter: sail cover and mast collar, dodger, pushpit, fenders over the side'},
    {'page': 'https://commons.wikimedia.org/wiki/File:Catalina_34_Mark_II_sailboat_Waterparke_0252.jpg',
     'author': 'Ahunt', 'licence': 'CC0-1.0', 'bytes': 1371788,
     'sha256': '8488b3fc2aa43fca5cff591d908f7e68364ca8adeee187d1c5edd264fca89968',
     'use': 'sailYacht broadside under sail: mast height against hull length'},
    {'page': 'https://commons.wikimedia.org/wiki/File:Flybridge_motor_yacht_St_Katharine_Docks_River_Thames_London_England.jpg',
     'author': 'Acabashi', 'licence': 'CC-BY-SA-4.0', 'bytes': 10618310,
     'sha256': 'e568e2fb852df8affb75c8b79760ad88e96721bdffa82ae1e92ea599cf5da778',
     'use': 'motorYacht broadside: flybridge, window band, hull windows, swim platform, fenders'},
    {'page': "https://commons.wikimedia.org/wiki/File:Boats_moored_at_Fisherman%27s_Wharf,_San_Francisco_17.jpg",
     'author': 'Slyronit', 'licence': 'CC-BY-SA-4.0', 'bytes': 783616,
     'sha256': '731624c774fa011a2fbee5aeb18db9656e674b646417e7852641fb7f4991a7fc',
     'use': "local context at Fisherman's Wharf: a moored flybridge motor yacht, sloops with blue sail covers"},
    {'page': 'https://commons.wikimedia.org/wiki/File:Yamaha_Waverunner_VX_-_Perissa_-_Santorini_-_Greece_-_02.jpg',
     'author': 'Norbert Nagel', 'licence': 'CC-BY-SA-3.0', 'bytes': 3746822,
     'sha256': '11abff919198efcfcc1723afc664e06c7abe46a9d558c392044d26d93fcac016',
     'use': 'jetSki broadside afloat: hull, seat, hood, handlebar, waterline'},
    {'page': 'https://commons.wikimedia.org/wiki/File:Yamaha_Waverunner_VX_-_Perissa_-_Santorini_-_Greece_-_01.jpg',
     'author': 'Norbert Nagel', 'licence': 'CC-BY-SA-3.0', 'bytes': 9521135,
     'sha256': '1e13b671d159e5c172dbd57a52fe4e57934ee564ce50b251aaa98ecd293f6568',
     'use': 'jetSki three-quarter view afloat'},
    {'page': "https://commons.wikimedia.org/wiki/File:SF_Fisherman%27s_Wharf_-_%27Pale_Horse%27,_%27Myka_Lynn%27_%26_%27Golden_Eagle%27.jpg",
     'author': 'Joe Mabel', 'licence': 'CC-BY-SA-4.0', 'bytes': 11408864,
     'sha256': 'b029f6d7f29a20a82a71167de49ce2386e10d87fa9eb2e3bd3c85fe401bc9d7a',
     'use': "fishingBoat at Fisherman's Wharf: wheelhouse forward, raised trolling poles, mast, radar, blue trim, "
            'dark-hull variant'},
    {'page': 'https://commons.wikimedia.org/wiki/File:California-05930_-_Kathy_II_(20611457676).jpg',
     'author': 'Dennis G. Jarvis', 'licence': 'CC-BY-SA-2.0', 'bytes': 1967250,
     'sha256': '9a0dd19b6d61a5f3055a43975178b64de20c15a739864b9c3baef0ea3d528200',
     'use': "fishingBoat at Fisherman's Wharf: wooden hulls, trim stripes, rub rails, weathering"},
    {'page': "https://commons.wikimedia.org/wiki/File:SF_Fisherman%27s_Wharf_-_%27Ace_Hi%27.jpg",
     'author': 'Joe Mabel', 'licence': 'CC-BY-SA-4.0', 'bytes': 10099004,
     'sha256': '22ec514e68df6a9e07105491450e8b967560ddc4775f8a30fcf661b36aeb0d75',
     'use': "fishingBoat at Fisherman's Wharf: wooden troller, dark sheer band, two masts"},
]

REFERENCE_AERIAL = {
    'source': 'USDA Farm Service Agency NAIP, via the USGS National Map image service USGSNAIPImagery',
    'service': 'https://imagery.nationalmap.gov/arcgis/rest/services/USGSNAIPImagery/ImageServer',
    'licence': 'Public domain (https://www.usa.gov/publicdomain/label/1.0/)',
    'resolutionM': 0.6,
    'tiles': [
        {'tile': 'rgb06/r001_c006.jpg', 'sha256': '374a9340913f4755632c71d20c581a1d6e5fed412f5fe7f12f6067cb270b4b13',
         'bbox': [-122.433869, 37.811395, -122.420231, 37.822248]},
        {'tile': 'rgb06/r001_c007.jpg', 'sha256': 'ba9c5247b19ef8e42a00faf0bd7f5180144e2b136703989fc2cc4496608bc667',
         'bbox': [-122.420231, 37.811395, -122.406592, 37.822248]},
        {'tile': 'rgb06/r002_c006.jpg', 'sha256': '1a1fe453a086b0ebcc53100ce36aae5843a7205a205146d3adbdb2e2c080454e',
         'bbox': [-122.433869, 37.800543, -122.420231, 37.811395]},
        {'tile': 'rgb06/r002_c007.jpg', 'sha256': '0beab8a52e5f30290ed72b019ac53ee970dd66a5de059d019a440765739be984',
         'bbox': [-122.420231, 37.800543, -122.406592, 37.811395]},
    ],
    'use': "Hyde Street Harbor and the Fisherman's Wharf lagoons seen from above: mostly white hulls and decks, "
           'some blue canvas, boats of about 7 to 15 m; colour and size check only',
}


# ------------------------------------------------------------ small maths ---
def clamp(v, a, b):
    return a if v < a else b if v > b else v


def lerp(a, b, t):
    return a + (b - a) * t


def smoothstep(t):
    t = clamp(t, 0.0, 1.0)
    return t * t * (3 - 2 * t)


def v3(p):
    return np.array(p, dtype=float)


class Curve:
    """Monotone cubic (PCHIP) through (t, value) pairs; flat outside the table."""

    def __init__(self, table):
        pts = sorted(table)
        self.x = np.array([p[0] for p in pts], float)
        self.y = np.array([p[1] for p in pts], float)
        n = len(self.x)
        self.m = np.zeros(n)
        if n < 2:
            return
        h = np.diff(self.x)
        d = np.diff(self.y) / h
        m = np.zeros(n)
        m[0], m[-1] = d[0], d[-1]
        for i in range(1, n - 1):
            if d[i - 1] * d[i] <= 0:
                m[i] = 0.0
            else:
                w1, w2 = 2 * h[i] + h[i - 1], h[i] + 2 * h[i - 1]
                m[i] = (w1 + w2) / (w1 / d[i - 1] + w2 / d[i])
        self.m = m

    def __call__(self, t):
        x, y = self.x, self.y
        if len(x) == 1:
            return float(y[0])
        t = clamp(t, x[0], x[-1])
        i = int(np.searchsorted(x, t, side='right') - 1)
        i = min(max(i, 0), len(x) - 2)
        h = x[i + 1] - x[i]
        s = (t - x[i]) / h
        s2, s3 = s * s, s * s * s
        return float((2 * s3 - 3 * s2 + 1) * y[i] + (s3 - 2 * s2 + s) * h * self.m[i]
                     + (-2 * s3 + 3 * s2) * y[i + 1] + (s3 - s2) * h * self.m[i + 1])


def bezier(points, k):
    t = np.linspace(0.0, 1.0, k)[:, None]
    P = [v3(p) for p in points]
    if len(P) == 3:
        return (1 - t) ** 2 * P[0] + 2 * (1 - t) * t * P[1] + t * t * P[2]
    return ((1 - t) ** 3) * P[0] + 3 * ((1 - t) ** 2) * t * P[1] + 3 * (1 - t) * t * t * P[2] + t ** 3 * P[3]


def resample(poly, n):
    """n + 1 points evenly spaced by arc length; the end points are kept exactly."""
    poly = np.asarray(poly, float)
    seg = np.linalg.norm(np.diff(poly, axis=0), axis=1)
    s = np.concatenate([[0.0], np.cumsum(seg)])
    if s[-1] < 1e-9:
        return [tuple(map(float, poly[0]))] * (n + 1)
    q = np.linspace(0.0, s[-1], n + 1)
    out = np.stack([np.interp(q, s, poly[:, 0]), np.interp(q, s, poly[:, 1])], 1)
    out[0], out[-1] = poly[0], poly[-1]
    return [tuple(map(float, p)) for p in out]


def polygon_area(poly):
    a = 0.0
    for i in range(len(poly)):
        x0, y0 = poly[i]
        x1, y1 = poly[(i + 1) % len(poly)]
        a += x0 * y1 - x1 * y0
    return abs(a) * 0.5


def clip_below(poly, zc):
    out = []
    n = len(poly)
    for i in range(n):
        a, b = poly[i], poly[(i + 1) % n]
        ina, inb = a[1] <= zc, b[1] <= zc
        if ina:
            out.append(a)
        if ina != inb:
            t = (zc - a[1]) / (b[1] - a[1])
            out.append((a[0] + t * (b[0] - a[0]), zc))
    return out


# ------------------------------------------------------------------- hull ---
class Hull:
    """Hull lines from tables over t = x / L (0 at the stem head, 1 at the transom).

    sheer, keel and half give the deck-edge height, the centreline bottom and
    the half-breadth at the sheer. Round-bilge sections take deadrise (beta),
    topside flare and bilge fullness; planing sections take a chine table.
    rake tilts the transom (degrees, positive = top aft) over rake_len metres.
    """

    def __init__(self, L, sheer, keel, half, beta=None, flare=None, full=None, chine=None,
                 rake=0.0, rake_len=0.8, camber=0.05):
        self.L = L
        self._sz, self._kz, self._hb = Curve(sheer), Curve(keel), Curve(half)
        self._beta = Curve(beta or [(0, 40), (1, 10)])
        self._flare = Curve(flare or [(0, 10), (1, 0)])
        self._full = Curve(full or [(0, .5), (1, .6)])
        self.chine = None
        if chine:
            self.chine = dict(half=Curve(chine['half']), z=Curve(chine['z']), flat=chine.get('flat', .05))
        self.rake = math.radians(rake)
        self.rake_len = rake_len
        self.camber = camber
        xs = np.linspace(0, L, 400)
        self.hbmax = max(self.hb(x) for x in xs)
        self.hbt = self.hb(L)

    def t(self, x):
        return clamp(x / self.L, 0.0, 1.0)

    def sz(self, x):
        return self._sz(self.t(x))

    def kz(self, x):
        return min(self._kz(self.t(x)), self.sz(x))

    def hb(self, x):
        return max(0.0, self._hb(self.t(x)))

    def shear(self, x, z):
        w = clamp((x - (self.L - self.rake_len)) / self.rake_len, 0.0, 1.0)
        return w * math.tan(self.rake) * (z - self.kz(self.L))

    def section(self, x, n):
        """n + 1 points (b, z) from the keel (b = 0) up to the sheer."""
        t = self.t(x)
        kz, sz, hb = self.kz(x), self.sz(x), self.hb(x)
        if hb < 1e-4 or sz - kz < 1e-4:
            return [(0.0, sz)] * (n + 1)
        if self.chine is None:
            H = sz - kz
            beta, phi, f = math.radians(self._beta(t)), math.radians(self._flare(t)), self._full(t)
            P0, P3 = (0.0, kz), (hb, sz)
            P1 = (f * hb * math.cos(beta), kz + f * hb * math.sin(beta))
            P2 = (hb - f * H * math.sin(phi), sz - f * H * math.cos(phi))
            return resample(bezier((P0, P1, P2, P3), 120), n)
        c = self.chine
        hc = min(c['half'](t), hb)
        cz = clamp(c['z'](t), kz, sz)
        fl = min(c['flat'], max(hb - hc, 0.0) * 0.5)
        C2 = (hc + fl, min(cz + fl * math.tan(math.radians(4)), sz))
        nb = max(1, int(round(n * 0.42)))
        ns = max(1, n - nb - 1)
        bottom = [(hc * i / nb, lerp(kz, cz, i / nb)) for i in range(nb + 1)]
        q = (C2[0] + (hb - C2[0]) * 0.15, C2[1] + (sz - C2[1]) * 0.65)
        side = resample(bezier((C2, q, (hb, sz)), 60), ns)
        pts = bottom + side
        return pts[:n + 1] if len(pts) >= n + 1 else pts + [pts[-1]] * (n + 1 - len(pts))

    def halfwidth_at(self, x, z):
        sec = self.section(x, 48)
        if z <= sec[0][1]:
            return 0.0
        for (b0, z0), (b1, z1) in zip(sec, sec[1:]):
            if z0 <= z <= z1 and z1 > z0:
                return lerp(b0, b1, (z - z0) / (z1 - z0))
        return sec[-1][0]

    def volume(self, z_wl=0.0, n=260):
        xs = np.linspace(0.0, self.L, n)
        areas = []
        for x in xs:
            sec = self.section(x, 48)
            poly = [(0.0, sec[0][1])] + sec[1:] + [(0.0, sec[-1][1])]
            clipped = clip_below(poly, z_wl)
            areas.append(2.0 * polygon_area(clipped) if len(clipped) > 2 else 0.0)
        areas = np.array(areas)
        return float(np.sum((areas[1:] + areas[:-1]) * 0.5 * np.diff(xs)))

    def waterline_ends(self):
        xs = np.linspace(0.0, self.L, 4001)
        kz = np.array([self.kz(x) for x in xs])
        below = np.where(kz <= 0.0)[0]

        def cross(a, b):
            for _ in range(60):
                m = (a + b) / 2
                if (self.kz(a) <= 0) == (self.kz(m) <= 0):
                    a = m
                else:
                    b = m
            return (a + b) / 2

        i0, i1 = below[0], below[-1]
        xf = xs[0] if i0 == 0 else cross(xs[i0 - 1], xs[i0])
        xa = self.L + self.shear(self.L, 0.0) if i1 == len(xs) - 1 else cross(xs[i1], xs[i1 + 1])
        return xf, xa


def scale_below(table, k):
    return [(t, z * k if z < 0 else z) for t, z in table]


def solve_depth(make, target, lo=0.6, hi=1.6):
    """Scale factor on the underwater depth tables so the hull displaces `target` m3."""
    for _ in range(40):
        mid = (lo + hi) / 2
        if make(mid).volume() < target:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2


# ----------------------------------------------------------------- meshes ---
class Part:
    """One node's polygons in the authoring frame, with a UV spec per face."""

    def __init__(self, name):
        self.name = name
        self.V, self.F, self.S, self.specs = [], [], [], []
        self._keys = {}

    def spec(self, tile, mode='box', sharp=SHARP_DEG, key=None):
        k = (tile, mode, sharp, key)
        if key is not None and k in self._keys:
            return self._keys[k]
        self.specs.append(dict(tile=tile, mode=mode, sharp=sharp))
        i = len(self.specs) - 1
        if key is not None:
            self._keys[k] = i
        return i

    def verts(self, pts):
        i = len(self.V)
        self.V.extend(tuple(float(c) for c in p) for p in pts)
        return i

    def face(self, idx, sp):
        self.F.append(tuple(idx))
        self.S.append(sp)


def grid(part, rows, sp, close=False):
    nr, nc = len(rows), len(rows[0])
    base = part.verts([p for r in rows for p in r])
    for i in range(nr - 1):
        for j in range(nc if close else nc - 1):
            j2 = (j + 1) % nc
            part.face((base + i * nc + j, base + (i + 1) * nc + j, base + (i + 1) * nc + j2, base + i * nc + j2), sp)
    return base


def ngon(part, pts, sp):
    clean = []
    for p in pts:
        if not clean or np.linalg.norm(v3(p) - v3(clean[-1])) > 1e-6:
            clean.append(p)
    if len(clean) > 2 and np.linalg.norm(v3(clean[0]) - v3(clean[-1])) < 1e-6:
        clean.pop()
    if len(clean) < 3:
        return
    base = part.verts(clean)
    part.face(tuple(range(base, base + len(clean))), sp)


def loft(part, rings, sp, cap0=True, cap1=True):
    grid(part, rings, sp, close=True)
    if cap0:
        ngon(part, rings[0][::-1], sp)
    if cap1:
        ngon(part, rings[-1], sp)


def frames(pts, up=(0.0, 0.0, 1.0)):
    P = [v3(p) for p in pts]
    n = len(P)
    seg = [P[i + 1] - P[i] for i in range(n - 1)]
    seg = [s / np.linalg.norm(s) if np.linalg.norm(s) > 1e-9 else None for s in seg]
    for i in range(len(seg)):
        if seg[i] is None:
            seg[i] = next((s for s in seg[i:] + seg[:i][::-1] if s is not None), v3((0, 0, 1)))
    T = []
    for i in range(n):
        t = seg[0] if i == 0 else seg[-1] if i == n - 1 else seg[i - 1] + seg[i]
        if np.linalg.norm(t) < 1e-9:
            t = seg[min(i, n - 2)]
        T.append(t / np.linalg.norm(t))
    u = v3(up)
    if abs(np.dot(u, T[0])) > 0.95:
        u = v3((1.0, 0.0, 0.0)) if abs(T[0][0]) < 0.9 else v3((0.0, 1.0, 0.0))
    N = np.cross(u, T[0])
    N /= np.linalg.norm(N)
    out = [(N, np.cross(T[0], N))]
    for i in range(1, n):
        N = N - np.dot(N, T[i]) * T[i]
        if np.linalg.norm(N) < 1e-9:
            N = out[-1][0]
        N = N / np.linalg.norm(N)
        out.append((N, np.cross(T[i], N)))
    return P, out


def tube(part, pts, r, sides, sp, caps=True, radii=None, ry=None, up=(0, 0, 1), squash=1.0):
    """Sweep a circle (or an ellipse rx = r, ry) along a polyline."""
    if sides < 3 or len(pts) < 2:
        return
    P, fr = frames(pts, up)
    rings = []
    for i, (p, (N, B)) in enumerate(zip(P, fr)):
        rx = radii[i] if radii is not None else r
        rv = (ry[i] if isinstance(ry, (list, tuple)) else ry) if ry is not None else rx * squash
        ring = []
        for k in range(sides):
            a = 2 * math.pi * (k + 0.5) / sides
            ring.append(tuple(p + N * math.cos(a) * rx + B * math.sin(a) * rv))
        rings.append(ring)
    loft(part, rings, sp, caps, caps)


def axes(yaw=0.0, pitch=0.0, roll=0.0):
    cy, sy = math.cos(yaw), math.sin(yaw)
    cp, sp_ = math.cos(pitch), math.sin(pitch)
    cr, sr = math.cos(roll), math.sin(roll)
    Rz = np.array([[cy, -sy, 0], [sy, cy, 0], [0, 0, 1]])
    Ry = np.array([[cp, 0, sp_], [0, 1, 0], [-sp_, 0, cp]])
    Rx = np.array([[1, 0, 0], [0, cr, -sr], [0, sr, cr]])
    R = Rz @ Ry @ Rx
    return R[:, 0], R[:, 1], R[:, 2]


def box(part, c, size, sp, ax=None):
    ex, ey, ez = ax or (v3((1, 0, 0)), v3((0, 1, 0)), v3((0, 0, 1)))
    c = v3(c)
    hx, hy, hz = size[0] / 2, size[1] / 2, size[2] / 2
    pts = [c + sx * hx * ex + sy * hy * ey + sz * hz * ez
           for sx in (-1, 1) for sy in (-1, 1) for sz in (-1, 1)]
    base = part.verts(pts)
    for f in ((0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)):
        part.face(tuple(base + i for i in f), sp)


def revolve(part, p0, axis, profile, sides, sp):
    """Surface of revolution: profile is (s along axis, radius) pairs."""
    axis = v3(axis) / np.linalg.norm(axis)
    u = np.cross(axis, (0, 0, 1) if abs(axis[2]) < 0.9 else (1, 0, 0))
    u /= np.linalg.norm(u)
    w = np.cross(axis, u)
    rings = []
    for s, r in profile:
        c = v3(p0) + axis * s
        rings.append([tuple(c + r * (math.cos(a) * u + math.sin(a) * w))
                      for a in (2 * math.pi * (k + .5) / sides for k in range(sides))])
    loft(part, rings, sp)


def torus(part, c, axis, R, r, nM, nm, sp):
    axis = v3(axis) / np.linalg.norm(axis)
    u = np.cross(axis, (0, 0, 1) if abs(axis[2]) < 0.9 else (1, 0, 0))
    u /= np.linalg.norm(u)
    w = np.cross(axis, u)
    rows = []
    for i in range(nM + 1):
        a = 2 * math.pi * i / nM
        radial = math.cos(a) * u + math.sin(a) * w
        cen = v3(c) + R * radial
        rows.append([tuple(cen + r * (math.cos(b) * radial + math.sin(b) * axis))
                     for b in (2 * math.pi * (k + .5) / nm for k in range(nm))])
    grid(part, rows, sp, close=True)


def naca(chord, thick, n):
    xs = (1 - np.cos(np.linspace(0, math.pi, n))) / 2
    yt = 5 * thick * (0.2969 * np.sqrt(xs) - 0.1260 * xs - 0.3516 * xs ** 2 + 0.2843 * xs ** 3 - 0.1036 * xs ** 4)
    upper = [(x * chord, y * chord) for x, y in zip(xs, yt)]
    lower = [(x * chord, -y * chord) for x, y in zip(xs[-2:0:-1], yt[-2:0:-1])]
    return upper + lower


def foil(part, sections, n, sp, b0=0.0):
    """Vertical foil (keel, rudder): sections are (x of leading edge, z, chord, thickness ratio)."""
    rings = [[(xle + px, b0 + py, z) for px, py in naca(c, th, n)] for xle, z, c, th in sections]
    loft(part, rings, sp)


def foil_volume(sections, b0=0.0):
    v = 0.0
    for (x0, z0, c0, t0), (x1, z1, c1, t1) in zip(sections, sections[1:]):
        a0, a1 = 0.685 * t0 * c0 * c0, 0.685 * t1 * c1 * c1
        v += abs(z1 - z0) * (a0 + a1) / 2
    return v


def sweep_rect(part, pts, w, h, sp, up=(0, 0, 1), offset=(0.0, 0.0)):
    """Rectangle w (sideways) x h (up) swept along a polyline; offset shifts it (side, up)."""
    P, fr = frames(pts, up)
    rings = []
    for p, (N, B) in zip(P, fr):
        c = p + N * offset[0] + B * offset[1]
        rings.append([tuple(c + N * sx * w / 2 + B * sy * h / 2)
                      for sx, sy in ((1, -1), (1, 1), (-1, 1), (-1, -1))])
    loft(part, rings, sp)


def wall(part, path, zb, zt, thick, sp):
    """A thin wall standing on a plan path (x, b): bottom heights zb, top heights zt per point."""
    P = [v3((x, b, 0.0)) for x, b in path]
    n = len(P)
    rings = []
    for i in range(n):
        t = (P[min(i + 1, n - 1)] - P[max(i - 1, 0)])
        t /= max(np.linalg.norm(t), 1e-9)
        nrm = v3((-t[1], t[0], 0.0)) * thick / 2
        x, b = path[i]
        rings.append([(x + nrm[0], b + nrm[1], zb[i]), (x + nrm[0], b + nrm[1], zt[i]),
                      (x - nrm[0], b - nrm[1], zt[i]), (x - nrm[0], b - nrm[1], zb[i])])
    loft(part, rings, sp)


def outline(xf, xa, hw, nose, n_side, n_nose, n_aft=2, corner=0.05):
    """Closed plan outline (x, b): rounded nose at xf, square aft end at xa, half-width hw(x)."""
    pts = []
    xs = [lerp(xa, xf + nose, i / n_side) for i in range(n_side + 1)]
    for x in xs[:-1]:
        pts.append((x, hw(x)))
    hn = hw(xf + nose)
    for i in range(n_nose + 1):
        a = math.pi * i / n_nose
        pts.append((xf + nose * (1 - math.sin(a)), hn * math.cos(a)))
    for x in xs[-2::-1]:
        pts.append((x, -hw(x)))
    for i in range(1, n_aft):
        pts.append((xa, lerp(-hw(xa), hw(xa), i / n_aft)))
    return pts


def plan_loft(part, levels, sps, cap_sp=None, crown=None):
    """Stack of horizontal plan rings: levels [(z, [(x, b)...])]; sps: one spec per band."""
    rings = [[(x, b, z) for x, b in ring] for z, ring in levels]
    for k in range(len(rings) - 1):
        grid(part, [rings[k], rings[k + 1]], sps[k], close=True)
    ngon(part, rings[0][::-1], sps[0])
    top = rings[-1]
    if crown:
        zc, inset = crown
        cx = sum(p[0] for p in top) / len(top)
        cb = sum(p[1] for p in top) / len(top)
        inner = [(lerp(p[0], cx, inset), lerp(p[1], cb, inset), zc) for p in top]
        grid(part, [top, inner], cap_sp or sps[-1], close=True)
        top = inner
    ngon(part, top, cap_sp or sps[-1])


def stations(L, n, breaks=(), eps=0.012):
    i = np.arange(n) / (n - 1)
    t = 0.55 * i + 0.45 * (0.5 - 0.5 * np.cos(np.pi * i))
    xs = [float(v) * L for v in t]
    for b in breaks:
        xs = [x for x in xs if abs(x - b) > 0.06]
    for b in breaks:
        xs += [b - eps, b + eps]
    return sorted(set(round(x, 6) for x in xs))


# ------------------------------------------------------------------- atlas ---
def _tiles():
    tiles = {}
    hull_keys = [f'{t}-{v}' for t in TYPES for v in 'ab']
    for i, k in enumerate(hull_keys):
        tiles['hull:' + k] = ((i % 2) * 512, ATLAS_PX - 144 * (i // 2 + 1), 512, 144)
    generic = ['deck_white', 'deck_cream', 'gel_white', 'gel_cream', 'gel_navy', 'teak', 'canvas_navy', 'canvas_tan',
               'af_red', 'af_black', 'paint_white', 'paint_cream', 'paint_blue', 'paint_red', 'paint_green', 'deck_grey',
               'rust_steel', 'wood', 'vinyl_grey', 'vinyl_black', 'crab_mesh', 'ski_blue', 'ski_red', 'cushion_white']
    for i, k in enumerate(generic):
        tiles[k] = ((i % 8) * 128, 64 + (i // 8) * 128, 128, 128)
    small = ['glass', 'stainless', 'alu', 'rubber', 'orange', 'rope', 'fender_navy', 'fender_white']
    for i, k in enumerate(small):
        tiles[k] = (i * 128, 0, 128, 64)
    return tiles


TILE = _tiles()
UV_MARGIN = 2.5


def tile_uv(name, s, t):
    x, y, w, h = TILE[name]
    s, t = clamp(s, 0.0, 1.0), clamp(t, 0.0, 1.0)
    return ((x + UV_MARGIN + s * (w - 2 * UV_MARGIN)) / ATLAS_PX,
            (y + UV_MARGIN + t * (h - 2 * UV_MARGIN)) / ATLAS_PX)


def seeded(name):
    return np.random.default_rng(int.from_bytes(hashlib.sha256(name.encode()).digest()[:8], 'little'))


def vnoise(h, w, cy, cx, rng):
    cy, cx = max(1, min(int(cy), h)), max(1, min(int(cx), w))
    g = rng.random((cy + 1, cx + 1))
    g[-1, :] = g[0, :]
    g[:, -1] = g[:, 0]
    ys = (np.arange(h) + 0.5) / h * cy
    xs = (np.arange(w) + 0.5) / w * cx
    y0, x0 = np.floor(ys).astype(int), np.floor(xs).astype(int)
    fy, fx = ys - y0, xs - x0
    fy, fx = fy * fy * (3 - 2 * fy), fx * fx * (3 - 2 * fx)
    a, b = g[np.ix_(y0, x0)], g[np.ix_(y0, x0 + 1)]
    c, d = g[np.ix_(y0 + 1, x0)], g[np.ix_(y0 + 1, x0 + 1)]
    return (a * (1 - fx) + b * fx) * (1 - fy)[:, None] + (c * (1 - fx) + d * fx) * fy[:, None]


def fbm(h, w, cells, octaves, rng, gain=0.5):
    cy, cx = cells
    tot = np.zeros((h, w))
    amp, norm = 1.0, 0.0
    for _ in range(octaves):
        tot += amp * vnoise(h, w, cy, cx, rng)
        norm += amp
        amp *= gain
        cy, cx = cy * 2, cx * 2
    return tot / norm


def hexc(s):
    s = s.lstrip('#')
    return np.array([int(s[i:i + 2], 16) / 255.0 for i in (0, 2, 4)])


def mix(rgb, col, t):
    t = np.clip(t, 0.0, 1.0)[..., None]
    return rgb * (1 - t) + np.asarray(col) * t


def streak_mask(h, w, rng, count, width, length, start=(0.85, 1.0), strength=(0.4, 1.0)):
    """Vertical run marks in pixel space (row 0 at the bottom), stronger at their top."""
    m = np.zeros((h, w))
    X = np.arange(w)[None, :]
    Y = np.arange(h)[:, None]
    for k in range(count):
        x0 = rng.uniform(0, w)
        wd = rng.uniform(*width)
        L = rng.uniform(*length) * h
        top = rng.uniform(*start) * h
        s = rng.uniform(*strength)
        dy = top - Y
        prof = np.clip(1 - dy / L, 0, 1) ** 0.8 * (dy >= 0)
        xc = x0 + 0.4 * np.sin(Y / rng.uniform(5, 11) + k)
        m = np.maximum(m, s * prof * np.exp(-((X - xc) / wd) ** 2))
    return m


def paint_generic(name, w, h):
    rng = seeded('tile:' + name)
    n1 = fbm(h, w, (4, 4), 5, rng)
    n2 = fbm(h, w, (16, 16), 3, rng)
    rough = np.full((h, w), 0.85)
    metal = np.zeros((h, w))

    def base(col, var=0.08):
        rgb = np.empty((h, w, 3))
        rgb[:] = hexc(col)
        return rgb * (1 + var * (n1 - 0.5) * 2 + 0.3 * var * (n2 - 0.5))[..., None]

    grime = hexc('5a5246')
    if name in ('deck_white', 'deck_cream'):
        rgb = base('bdb9ae' if name == 'deck_white' else 'c2b498', 0.05)
        dots = ((np.arange(h)[:, None] % 4 < 2) & (np.arange(w)[None, :] % 4 < 2)).astype(float)
        rgb *= (1 - 0.05 * dots)[..., None]
        rgb = mix(rgb, grime, np.clip((fbm(h, w, (2, 2), 4, rng) - 0.4) * 1.0, 0, 0.3))
        # green-grey mildew in the non-skid where the deck stays damp
        rgb = mix(rgb, hexc('74745e'), np.clip((fbm(h, w, (6, 6), 3, rng) - 0.6) * 2.0, 0, 0.25))
        rough[:] = 0.88
    elif name in ('gel_white', 'gel_cream', 'gel_navy'):
        col = {'gel_white': 'c8c6bc', 'gel_cream': 'cbbf9f', 'gel_navy': '23293a'}[name]
        rgb = base(col, 0.05)
        if name == 'gel_navy':
            rgb = mix(rgb, hexc('4d5363'), np.clip((n1 - 0.4) * 1.8, 0, 0.55))
        if name != 'gel_navy':
            # oxidised gelcoat: chalky, yellowed blotches
            rgb = mix(rgb, hexc('aca794'), np.clip((fbm(h, w, (2, 3), 4, rng) - 0.4) * 1.6, 0, 0.35))
        rgb = mix(rgb, grime, 0.45 * streak_mask(h, w, rng, 18, (2.0, 5.0), (0.2, 0.8)))
        rough[:] = 0.8 + 0.06 * n2
    elif name == 'teak':
        rgb = base('857a69', 0.06)
        seam = (np.arange(h)[:, None] % 16 < 1.5).astype(float) * np.ones((1, w))
        grain = vnoise(h, w, 64, 6, rng)
        rgb *= (0.9 + 0.12 * grain)[..., None]
        rgb = mix(rgb, hexc('3a342c'), seam * 0.8)
        rough[:] = 0.9
    elif name in ('canvas_navy', 'canvas_tan'):
        rgb = base('2b3446' if name == 'canvas_navy' else '9d8b6c', 0.07)
        fade = np.clip((fbm(h, w, (2, 3), 4, rng) - 0.35) * 1.5, 0, 1)
        rgb = mix(rgb, hexc('556075' if name == 'canvas_navy' else 'b8aa8c'), fade * 0.6)
        weave = ((np.arange(h)[:, None] + np.arange(w)[None, :]) % 3 == 0).astype(float)
        rgb *= (1 - 0.03 * weave)[..., None]
        seams = (np.arange(w)[None, :] % 42 < 1.5) * np.ones((h, 1))
        rgb = mix(rgb, hexc('1d222c' if name == 'canvas_navy' else '6e604a'), seams * 0.5)
        rgb = mix(rgb, grime, 0.3 * streak_mask(h, w, rng, 10, (2, 6), (0.2, 0.6)))
        rgb = mix(rgb, hexc('3f4034'), (fbm(h, w, (24, 24), 2, rng) > 0.7) * 0.3)  # mildew spots
        rough[:] = 0.95
    elif name in ('af_red', 'af_black'):
        rgb = base('5c1e17' if name == 'af_red' else '1c1e22', 0.1)
        rgb = mix(rgb, hexc('7d4a3e' if name == 'af_red' else '3a3b3c'), np.clip((n2 - 0.55) * 2.5, 0, 0.6))
        rough[:] = 0.93
    elif name in ('paint_white', 'paint_cream'):
        rgb = base('c6c4ba' if name == 'paint_white' else 'c8bb98', 0.05)
        rust = streak_mask(h, w, rng, 6, (1, 2.5), (0.1, 0.4), strength=(0.4, 0.9))
        rgb = mix(rgb, hexc('7a4a28'), rust * 0.3)
        rgb = mix(rgb, grime, 0.35 * streak_mask(h, w, rng, 16, (2, 5), (0.3, 0.9)))
        chips = (fbm(h, w, (28, 28), 2, rng) > 0.8).astype(float)
        rgb = mix(rgb, hexc('7a766c'), chips * 0.3)
        rough[:] = 0.8 + 0.1 * rust
    elif name in ('paint_blue', 'paint_red', 'paint_green'):
        col = {'paint_blue': '1d3e6b', 'paint_red': '6e1e16', 'paint_green': '1f3628'}[name]
        rgb = base(col, 0.08)
        rgb = mix(rgb, hexc('a9a9a0'), np.clip((n2 - 0.5) * 1.2, 0, 0.25))
        chips = (fbm(h, w, (20, 20), 3, rng) > 0.66).astype(float)
        rgb = mix(rgb, hexc('6b4a32'), chips * 0.8)
        rgb = mix(rgb, hexc('7a4a28'), streak_mask(h, w, rng, 8, (1, 3), (0.2, 0.6)) * 0.6)
        rough[:] = 0.82
    elif name == 'deck_grey':
        rgb = base('6e706a', 0.08)
        rgb = mix(rgb, hexc('3d3a33'), np.clip((n1 - 0.45) * 1.6, 0, 0.6))
        rgb = mix(rgb, hexc('6a3f22'), (fbm(h, w, (12, 12), 3, rng) > 0.72) * 0.3)
        rough[:] = 0.9
    elif name == 'rust_steel':
        rgb = base('5a3220', 0.12)
        rgb = mix(rgb, hexc('7c4a24'), np.clip((n2 - 0.4) * 1.5, 0, 1) * 0.7)
        rgb = mix(rgb, hexc('585a58'), (fbm(h, w, (6, 6), 3, rng) > 0.6) * 0.6)
        rough[:] = 0.95
    elif name == 'wood':
        rgb = base('6a4a30', 0.08)
        grain = vnoise(h, w, 80, 5, rng)
        rgb *= (0.85 + 0.25 * grain)[..., None]
        rgb = mix(rgb, hexc('8b8170'), np.clip((n1 - 0.5) * 1.5, 0, 0.5))
        rough[:] = 0.85
    elif name in ('vinyl_grey', 'vinyl_black', 'cushion_white'):
        col = {'vinyl_grey': '7d7d7a', 'vinyl_black': '1b1b1c', 'cushion_white': 'bfbcb2'}[name]
        rgb = base(col, 0.06)
        rgb = mix(rgb, grime, np.clip((n1 - 0.5), 0, 0.2))
        rough[:] = 0.65
    elif name == 'crab_mesh':
        rgb = np.empty((h, w, 3))
        rgb[:] = hexc('202020')
        yy, xx = np.arange(h)[:, None], np.arange(w)[None, :]
        wire = (((yy + xx) % 9 < 1.6) | ((yy - xx) % 9 < 1.6)).astype(float)
        rgb = mix(rgb, hexc('8a8c88'), wire * (0.7 + 0.3 * n2))
        rgb = mix(rgb, hexc('5c3a22'), np.clip((n1 - 0.55) * 2, 0, 0.5))
        rough[:] = 0.75
        metal[:] = 0.5 * wire
    elif name in ('ski_blue', 'ski_red'):
        rgb = base('1f3a7a' if name == 'ski_blue' else '7c1c14', 0.06)
        rgb = mix(rgb, hexc('8a8f99' if name == 'ski_blue' else 'a07a70'), np.clip((n1 - 0.45) * 1.2, 0, 0.3))
        rough[:] = 0.6
    elif name == 'glass':
        rgb = base('15191c', 0.1)
        rgb = mix(rgb, hexc('4c4f50'), np.clip((n1 - 0.5) * 1.2, 0, 0.35))
        rough[:] = 0.2
    elif name in ('stainless', 'alu'):
        rgb = base('9c9c98' if name == 'stainless' else '8f9294', 0.06)
        rough[:] = 0.38 if name == 'stainless' else 0.5
        metal[:] = 1.0 if name == 'stainless' else 0.85
    elif name == 'rubber':
        rgb = base('121213', 0.15)
        rough[:] = 0.92
    elif name == 'orange':
        rgb = base('c8501e', 0.08)
        rgb = mix(rgb, hexc('8a5a3c'), np.clip((n1 - 0.45) * 1.4, 0, 0.5))
        rough[:] = 0.7
    elif name == 'rope':
        rgb = base('8f8670', 0.1)
        rough[:] = 0.95
    elif name in ('fender_navy', 'fender_white'):
        rgb = base('1f2a44' if name == 'fender_navy' else 'bdbcb4', 0.06)
        rgb = mix(rgb, grime, np.clip((n1 - 0.35) * 1.3, 0, 0.55))
        rough[:] = 0.62
    else:
        raise KeyError(name)
    return np.clip(rgb, 0, 1), rough, metal


def paint_hull(body, w, h):
    """The hull-side panel of one body, painted in metres along x and z."""
    rng = seeded('hull:' + body.key)
    H, c = body.hull, body.paint
    (x0, x1), (z0, z1) = body.hx, body.hz
    xs = x0 + (np.arange(w) + 0.5) / w * (x1 - x0)
    zs = z0 + (np.arange(h) + 0.5) / h * (z1 - z0)
    X, Z = np.meshgrid(xs, zs)
    sheer = np.array([H.sz(clamp(x, 0.0, H.L)) for x in xs])[None, :]
    rel = sheer - Z
    pz = (z1 - z0) / h
    px = (x1 - x0) / w

    def band(lo, hi, v):
        return np.clip((v - lo) / pz + 0.5, 0, 1) * np.clip((hi - v) / pz + 0.5, 0, 1)

    rgb = np.empty((h, w, 3))
    rgb[:] = hexc(c['top'])
    rough = np.full((h, w), c.get('rough', 0.72))
    metal = np.zeros((h, w))
    mott = fbm(h, w, (3, 26), 5, rng)
    rgb *= (1 + c.get('mottle', 0.06) * (mott - 0.5) * 2)[..., None]
    if c.get('chalk'):
        blot = np.clip((fbm(h, w, (2, 12), 4, rng) - 0.38) * 2.0, 0, 1)
        rgb = mix(rgb, hexc(c.get('chalk_col', '5c6170')), blot * c['chalk'])
    if c.get('planks'):
        seam = (np.mod(rel, c['planks']) < pz * 1.3) & (Z > 0.02)
        rgb = mix(rgb, rgb * 0.62, seam.astype(float) * 0.8)
    for lo, hi, col in c.get('bands', []):
        rgb = mix(rgb, hexc(col) * (1 + 0.05 * (mott - .5))[..., None], band(lo, hi, rel))
    if c.get('swoosh'):
        x0s, slope, zb0, col = c['swoosh']
        edge = np.clip((x0s + slope * (Z - zb0) - X) / px + 0.5, 0, 1) * np.clip((Z - zb0) / pz + 0.5, 0, 1)
        rgb = mix(rgb, hexc(col) * (1 + 0.05 * (mott - .5))[..., None], edge)
    for xa, xb, da, db, col in c.get('windows', []):
        r = (db - da) / 2
        cz = (da + db) / 2
        dx = np.maximum(np.maximum(xa + r - X, X - (xb - r)), 0)
        inside = (dx / r) ** 2 + ((rel - cz) / r) ** 2 <= 1.0
        rgb = mix(rgb, hexc(col), inside.astype(float))
        rough = np.where(inside, 0.2, rough)
    if c.get('wear'):
        wv = c['wear']
        scrape = vnoise(h, w, 40, 6, rng) * fbm(h, w, (30, 30), 3, rng)
        near = np.exp(-((rel - wv.get('line', 0.3)) / 0.25) ** 2) + 0.6 * np.exp(-(X / 1.5) ** 2)
        chip = (scrape * (0.6 + near) > wv.get('thresh', 0.42)) & (Z > 0.05)
        rgb = mix(rgb, hexc(wv['under']), chip.astype(float) * 0.85)
        rough = np.where(chip, 0.88, rough)
        scuff = (vnoise(h, w, 5, 140, rng) > 0.72) & (Z > 0.25) & (rel > 0.15)
        rgb = mix(rgb, hexc('6b6a64'), scuff.astype(float) * 0.35)
    dark = np.zeros((h, w))
    n_streaks = int(c.get('streaks', 5) * (x1 - x0))
    sources = list(c.get('sources', [])) + list(rng.uniform(x0 + 0.4, x1 - 0.3, n_streaks))
    for k, xs0 in enumerate(sources):
        broad = k % c.get('broad_every', 4) == 0
        wd = rng.uniform(0.10, 0.25) if broad else rng.uniform(0.008, 0.03)
        L = rng.uniform(0.25, 1.0) * max(float(np.max(sheer)), 0.4)
        top = c.get('streak_top', 0.02)
        s = rng.uniform(0.18, 0.32) if broad else rng.uniform(0.3, 0.75)
        d = rel - top
        prof = np.clip(1 - d / L, 0, 1) ** 0.9 * (d >= 0)
        xc = xs0 + 0.003 * np.sin(Z * 9 + k)
        dark = np.maximum(dark, s * prof * np.exp(-((X - xc) / wd) ** 2))
    rgb = mix(rgb, hexc(c.get('grime', '625a4b')), dark * c.get('grime_amt', 0.5))
    if c.get('rust'):
        rr = c['rust']
        rm = np.zeros((h, w))
        for k, (xs0, d0) in enumerate(rr['sources']):
            wd = rng.uniform(*rr.get('width', (0.03, 0.09)))
            L = rng.uniform(*rr.get('len', (0.35, 1.1)))
            d = rel - d0
            prof = np.clip(1 - d / L, 0, 1) ** 0.6 * (d >= -0.01)
            xc = xs0 + 0.012 * np.sin(Z * 17 + k)
            rm = np.maximum(rm, rng.uniform(0.6, 1.0) * prof * np.exp(-((X - xc) / wd) ** 2))
        rgb = mix(rgb, hexc('6e3b1c'), rm * rr.get('amount', 0.8))
        rough = np.maximum(rough, 0.9 * (rm > 0.3))
    lowg = np.clip(1 - Z / np.maximum(0.42 * sheer, 0.15), 0, 1) ** 1.6 * (Z > 0)
    rgb = mix(rgb, hexc(c.get('low_col', '5e5a4a')), lowg * c.get('lowgrime', 0.3))
    if c.get('boot'):
        lo, hi, col = c['boot']
        rgb = mix(rgb, hexc(col) * (1 + 0.08 * (mott - .5))[..., None], band(lo, hi, Z))
    af_top = c.get('af_top', 0.03)
    if c.get('stain'):
        # the brown tide stain above the waterline, over the boot stripe, heaviest at the bow
        st = c['stain']
        up = np.clip(1 - (Z - af_top) / st.get('height', 0.22), 0, 1) * (Z >= af_top - 0.01)
        along = st.get('base', 0.3) + (1 - st.get('base', 0.3)) * np.exp(-(X / st.get('reach', 2.4)) ** 2)
        tex = 0.55 + 0.45 * fbm(h, w, (3, 40), 4, rng)
        rgb = mix(rgb, hexc(st.get('col', '7a6342')), up ** 1.5 * along * tex * st['amount'])
    if c.get('af'):
        af = np.empty((h, w, 3))
        af[:] = hexc(c['af'])
        af *= (1 + 0.16 * (fbm(h, w, (6, 50), 4, rng) - 0.5))[..., None]
        fadeaf = np.clip(1 - (af_top - Z) / 0.3, 0, 1) * 0.35
        af = mix(af, hexc('8a7d66'), fadeaf * (fbm(h, w, (4, 40), 3, rng) > 0.45))
        below = np.clip((af_top - Z) / pz + 0.5, 0, 1)
        rgb = mix(rgb, af, below)
        rough = np.where(Z < af_top, 0.93, rough)
    edge = af_top + 0.035 + 0.035 * fbm(h, w, (2, 60), 3, rng)
    scum = np.clip((edge - Z) / 0.03, 0, 1) * (Z > -0.25)
    slime = np.clip(1 - np.abs(Z + 0.06) / 0.2, 0, 1)
    rgb = mix(rgb, hexc('5b5a3a'), scum * c.get('scum', 0.55) * (Z >= af_top - 0.005))
    rgb = mix(rgb, hexc('3f4630'), slime * c.get('slime', 0.35) * (Z < af_top))
    rough = np.where(scum > 0.3, np.maximum(rough, 0.88), rough)
    return np.clip(rgb, 0, 1), rough, metal


def make_atlas(bodies, workdir):
    base = np.zeros((ATLAS_PX, ATLAS_PX, 3))
    rough = np.ones((ATLAS_PX, ATLAS_PX))
    metal = np.zeros((ATLAS_PX, ATLAS_PX))
    for name, (x, y, w, h) in TILE.items():
        if name.startswith('hull:'):
            body = bodies.get(name[5:])
            if body is None:
                continue
            rgb, r, m = paint_hull(body, w, h)
        else:
            rgb, r, m = paint_generic(name, w, h)
        base[y:y + h, x:x + w] = rgb
        rough[y:y + h, x:x + w] = r
        metal[y:y + h, x:x + w] = m
    mr = np.ones((MR_PX, MR_PX, 3))
    k = ATLAS_PX // MR_PX
    mr[..., 1] = rough.reshape(MR_PX, k, MR_PX, k).mean(axis=(1, 3))
    mr[..., 2] = metal.reshape(MR_PX, k, MR_PX, k).mean(axis=(1, 3))
    paths = []
    for name, arr, colour in (('boats-v1-albedo', base, 'sRGB'), ('boats-v1-mr', mr, 'Non-Color')):
        hgt, wid = arr.shape[:2]
        img = bpy.data.images.new(name, wid, hgt, alpha=False)
        img.colorspace_settings.name = colour
        rgba = np.ones((hgt, wid, 4), dtype=np.float32)
        rgba[..., :3] = np.round(np.clip(arr, 0, 1) * 255) / 255
        img.pixels.foreach_set(rgba.ravel())
        img.file_format = 'JPEG'
        path = os.path.join(workdir, name + '.jpg')
        img.save(filepath=path, quality=JPEG_QUALITY)
        bpy.data.images.remove(img)
        paths.append(path)
    return paths


def make_material(albedo_path, mr_path):
    a = bpy.data.images.load(albedo_path)
    a.colorspace_settings.name = 'sRGB'
    m = bpy.data.images.load(mr_path)
    m.colorspace_settings.name = 'Non-Color'
    mat = bpy.data.materials.new('BoatAtlas')
    mat.use_nodes = True
    mat.use_backface_culling = True
    nt = mat.node_tree
    bsdf = nt.nodes['Principled BSDF']
    t1 = nt.nodes.new('ShaderNodeTexImage')
    t1.image = a
    t2 = nt.nodes.new('ShaderNodeTexImage')
    t2.image = m
    sep = nt.nodes.new('ShaderNodeSeparateColor')
    nt.links.new(t1.outputs['Color'], bsdf.inputs['Base Color'])
    nt.links.new(t2.outputs['Color'], sep.inputs['Color'])
    nt.links.new(sep.outputs['Green'], bsdf.inputs['Roughness'])
    nt.links.new(sep.outputs['Blue'], bsdf.inputs['Metallic'])
    return mat


# ------------------------------------------------------------------ bodies ---
class Detail:
    """Sampling per LOD: 0 near, 1 mid (about a quarter), 2 silhouette only."""

    def __init__(self, lod):
        self.lod = lod

    def pick(self, *v):
        return v[min(self.lod, len(v) - 1)]

    @property
    def near(self):
        return self.lod == 0

    @property
    def far(self):
        return self.lod == 2


class Body:
    def __init__(self, key, label, hull, paint, budget, build, meta):
        self.key, self.label, self.hull, self.paint = key, label, hull, paint
        self.kind, self.variant = key.split('-')
        self.budget, self._build, self.meta = budget, build, meta
        self.x_fwd, self.x_aft = hull.waterline_ends()
        self.xm = (self.x_fwd + self.x_aft) / 2
        self.lwl = self.x_aft - self.x_fwd
        L = hull.L
        shear_top = hull.shear(L, hull.sz(L))
        self.hx = (-0.03, L + max(0.0, shear_top) + 0.03)
        xs = np.linspace(0, L, 300)
        top = max(max(hull.sz(x) for x in xs), meta.get('tileTopZ', -1e9))
        self.hz = (min(hull.kz(x) for x in xs) - 0.03, top + 0.03)

    def blender(self, p):
        return (p[1], p[0] - self.xm, p[2])

    def build(self, lod):
        return self._build(self, Detail(lod))


def hull_shell(part, body, D, deck_fn, breaks=(), n_st=(44, 14, 7), n_sec=(22, 9, 4)):
    """Hull sides, deck and transom as one closed shell."""
    H = body.hull
    hs = part.spec('hull:' + body.key, 'hull', key='hull')
    ts = part.spec('hull:' + body.key, 'transom', key='transom')
    xs = stations(H.L, D.pick(*n_st), breaks)
    nsec = D.pick(*n_sec)
    port, deck = [], []
    for x in xs:
        port.append([(x + H.shear(x, z), b, z) for b, z in H.section(x, nsec)])
        pts, _ = deck_fn(x, H.hb(x), H.sz(x))
        deck.append([(x + H.shear(x, z), b, z) for b, z in pts])
    stbd = [[(p[0], -p[1], p[2]) for p in row] for row in port]
    grid(part, port, hs)
    grid(part, stbd, hs)
    rows = [r + [(p[0], -p[1], p[2]) for p in r[-2::-1]] for r in deck]
    W = len(rows[0])
    base = part.verts([p for r in rows for p in r])
    for i in range(len(xs) - 1):
        xm = (xs[i] + xs[i + 1]) / 2
        _, tiles = deck_fn(xm, H.hb(xm), H.sz(xm))
        M = len(tiles)
        for k in range(W - 1):
            tile, mode = tiles[k] if k < M else tiles[2 * M - 1 - k]
            sp = part.spec(tile, mode, key='deck')
            part.face((base + i * W + k, base + (i + 1) * W + k, base + (i + 1) * W + k + 1, base + i * W + k + 1), sp)
    transom = port[-1] + rows[-1][1:-1] + stbd[-1][::-1]
    ngon(part, transom, ts)
    return xs


def strip_along(part, H, x0, x1, n, at, w, h, sp, sides=(1, -1)):
    """A rail or rubbing strip along the hull: at(x) -> (b, z) of its centre on the port side."""
    for sgn in sides:
        pts = []
        for i in range(n + 1):
            x = lerp(x0, x1, i / n)
            b, z = at(x)
            pts.append((x + H.shear(x, z), sgn * b, z))
        sweep_rect(part, pts, w, h, sp)


def fender(part, D, x, b, zc, length, r, sp, line_sp, tie):
    if D.far:
        return
    sides = D.pick(12, 7)
    L2 = length / 2
    prof = [(-L2, 0.0), (-L2 + 0.01, r * 0.45), (-L2 + 0.04, r * 0.85), (-L2 + 0.09, r),
            (L2 - 0.09, r), (L2 - 0.04, r * 0.85), (L2 - 0.01, r * 0.45), (L2, 0.0)]
    if not D.near:
        prof = [prof[0], prof[2], prof[3], prof[4], prof[5], prof[7]]
    revolve(part, (x, b, zc), (0, 0, 1), prof, sides, sp)
    if D.near:
        tube(part, [(x, b, zc + L2), tie], 0.006, 4, line_sp)


# ---------------------------------------------------------- sailing yacht ---
def sail_tables(k):
    keel = [(0, 1.40), (.025, .62), (.055, .20), (.082, 0.0), (.12, -.22), (.2, -.40), (.35, -.52), (.5, -.53),
            (.65, -.45), (.8, -.27), (.9, -.10), (.945, 0.0), (1.0, .16)]
    return Hull(10.52,
                sheer=[(0, 1.40), (.1, 1.31), (.3, 1.18), (.55, 1.10), (.8, 1.07), (1, 1.09)],
                keel=scale_below(keel, k),
                half=[(0, 0), (.05, .45), (.15, 1.02), (.3, 1.52), (.45, 1.75), (.56, 1.77), (.7, 1.71),
                      (.85, 1.58), (1, 1.44)],
                beta=[(0, 55), (.2, 30), (.4, 18), (.6, 12), (.8, 7), (1, 4)],
                flare=[(0, 30), (.2, 18), (.45, 8), (.7, 4), (1, 2)],
                full=[(0, .35), (.3, .55), (.6, .6), (1, .62)],
                rake=-14.0, rake_len=0.9, camber=0.06)


SAIL_KEEL = [(3.86, -0.30, 1.78, 0.13), (4.02, -0.55, 1.72, 0.125), (4.38, -1.62, 1.30, 0.105),
             (4.46, -1.70, 1.12, 0.06)]
SAIL_RUDDER = [(8.30, -0.15, 0.58, 0.10), (8.36, -0.45, 0.56, 0.10), (8.52, -1.36, 0.40, 0.09),
               (8.56, -1.41, 0.33, 0.05)]
SAIL_MAST_X = 4.18          # J = 4.11 m aft of the forestay tack at x = 0.07
SAIL_COCKPIT = (6.55, 9.95)


def sail_build(body, D):
    v = body.variant
    H = body.hull
    hull, sup, rig, gear = Part('Hull'), Part('Superstructure'), Part('Rig'), Part('Gear')
    deck_t = 'deck_white' if v == 'a' else 'deck_cream'
    gel = 'gel_white' if v == 'a' else 'gel_cream'
    canvas = 'canvas_navy' if v == 'a' else 'canvas_tan'
    ck0, ck1 = SAIL_COCKPIT

    def zc(x, b):
        hb = max(H.hb(x), 1e-4)
        return H.sz(x) + H.camber * min(1.0, hb / H.hbmax) * (1 - min(1.0, (b / hb) ** 2))

    def deck_fn(x, hb, sz):
        if ck0 <= x <= ck1 and hb > 1.3:
            bo, bi, hc, d = 1.10, 1.04, 0.24, 0.44
            z = zc(x, bo)
            pts = [(hb, sz), (bo, z), (bo, z + hc), (bi, z + hc), (bi, z - d), (0.0, z - d)]
            tiles = [(deck_t, 'top'), (gel, 'side'), ('teak', 'top'), (gel, 'side'), ('teak', 'top')]
        else:
            bo, bi = hb * 0.62, hb * 0.59
            pts = [(hb, sz), (bo, zc(x, bo)), (bo, zc(x, bo)), (bi, zc(x, bi)), (bi, zc(x, bi)), (0.0, zc(x, 0))]
            tiles = [(deck_t, 'top')] * 5
        return pts, tiles

    hull_shell(hull, body, D, deck_fn, breaks=(ck0, ck1))
    sole = zc(8.0, 1.1) - 0.44
    af = 'af_black' if v == 'a' else 'af_red'
    foil(hull, SAIL_KEEL, D.pick(14, 8, 5), hull.spec(af, 'side'))
    foil(hull, SAIL_RUDDER, D.pick(10, 6, 4), hull.spec(af, 'side'))
    if D.near:
        sh = hull.spec('stainless', 'flat')
        tube(hull, [(7.25, 0, -0.42), (7.95, 0, -0.52)], 0.0125, 6, sh)
        box(hull, (7.90, 0, -0.46), (0.06, 0.03, 0.16), sh)
        for k in range(3):
            a = 2 * math.pi * k / 3
            box(hull, (7.98, 0.09 * math.cos(a), -0.52 + 0.09 * math.sin(a)), (0.03, 0.06, 0.16), sh,
                axes(0, 0, a))

    # Toe rail and rub rail.
    toe = 'alu' if v == 'a' else 'teak'
    strip_along(hull, H, 0.30, H.L - 0.02, D.pick(24, 8, 4), lambda x: (H.hb(x) - 0.02, H.sz(x) + 0.02),
                0.035, 0.04, hull.spec(toe, 'flat' if v == 'a' else 'box'))
    if not D.far:
        strip_along(hull, H, 0.12, H.L - 0.01, D.pick(24, 8),
                    lambda x: (H.halfwidth_at(x, H.sz(x) - 0.025) + 0.012, H.sz(x) - 0.025),
                    0.025, 0.05, hull.spec('fender_white' if v == 'a' else 'rubber', 'flat'))

    # Cabin trunk with ports.
    def trunk_hw(x):
        return min(1.04, 0.70 + (x - 2.55) * 0.30, H.hb(x) - 0.42)
    xf, xa = 2.55, ck0 + 0.012
    ns, nn = D.pick(10, 5, 3), D.pick(8, 4, 2)
    z_deck = min(zc(x, 0.6) for x in np.linspace(xf, xa, 20)) - 0.04
    ztop = zc(4.2, 0) + 0.46
    lv = [(z_deck, outline(xf, xa, trunk_hw, 0.55, ns, nn)),
          (ztop - 0.30, outline(xf + 0.02, xa, trunk_hw, 0.55, ns, nn)),
          (ztop - 0.08, outline(xf + 0.16, xa, lambda x: trunk_hw(x) - 0.035, 0.55, ns, nn)),
          (ztop, outline(xf + 0.30, xa, lambda x: trunk_hw(x) - 0.07, 0.5, ns, nn))]
    g = sup.spec(gel, 'side', key='trunk')
    plan_loft(sup, lv, [g, g, g], cap_sp=sup.spec(deck_t, 'top', key='trunktop'), crown=(ztop + 0.04, 0.35))
    if not D.far:
        gl = sup.spec('glass', 'flat')
        lean = math.atan2(0.035, 0.22)
        for sgn in (1, -1):
            for xo in (3.15, 3.65, 4.15):
                bw = trunk_hw(xo) - 0.03
                tube(sup, [(xo, sgn * (bw - 0.01), ztop - 0.19), (xo, sgn * (bw + 0.012), ztop - 0.19)], 0.11, 8, gl,
                     ry=0.045)
            box(sup, (5.55, sgn * (trunk_hw(5.55) - 0.02 + 0.006), ztop - 0.19), (1.25, 0.012, 0.13), gl,
                axes(0, 0, -sgn * lean))
        hatch = sup.spec(gel, 'box')
        box(sup, (1.95, 0, zc(1.95, 0) + 0.04), (0.56, 0.56, 0.08), hatch)
        box(sup, (1.95, 0, zc(1.95, 0) + 0.085), (0.48, 0.48, 0.012), gl)
        box(sup, (3.05, 0, ztop + 0.05), (0.52, 0.52, 0.07), hatch)
        box(sup, (3.05, 0, ztop + 0.09), (0.44, 0.44, 0.012), gl)
        box(sup, (6.20, 0, ztop + 0.05), (0.55, 0.62, 0.08), sup.spec(gel, 'box'))
    # Cockpit seats and wheel.
    seat = sup.spec(gel, 'box', key='seat')
    if not D.far:
        for sgn in (1, -1):
            box(sup, (8.25, sgn * 0.79, sole + 0.21), (2.7, 0.50, 0.42), seat)
            box(sup, (8.25, sgn * 0.79, sole + 0.425), (2.6, 0.46, 0.012), sup.spec('teak', 'box', key='seat-top'))
    if D.near:
        st = gear.spec('stainless', 'flat', sharp=100)
        tube(gear, [(9.15, 0, sole), (9.15, 0, 1.05)], 0.06, 8, st)
        revolve(gear, (9.12, 0, 1.05), (0, 0, 1), [(0, 0), (0.01, 0.1), (0.08, 0.12), (0.15, 0.08), (0.17, 0)], 8,
                gear.spec('vinyl_black', 'flat'))
        torus(gear, (9.30, 0, 1.30), (1, 0, 0), 0.42, 0.016, 28, 5, st)
        for k in range(5):
            a = 2 * math.pi * k / 5 + 0.3
            tube(gear, [(9.30, 0, 1.30), (9.30, 0.42 * math.cos(a), 1.30 + 0.42 * math.sin(a))], 0.008, 4, st)
        for sgn in (1, -1):
            for xw, zz in ((8.15, zc(8.15, 1.1) + 0.24), (6.35, ztop)):
                revolve(gear, (xw, sgn * (1.07 if xw > 7 else 0.62), zz), (0, 0, 1),
                        [(0, 0.0), (0.001, 0.085), (0.12, 0.08), (0.13, 0.05), (0.2, 0.045), (0.21, 0.0)], 10, st)
        tube(gear, [(7.62, 0, 2.86), (8.95, 0, sole + 0.45)], 0.018, 5, gear.spec('rope', 'flat'))

    # Dodger (canvas hood over the companionway), variant a only.
    if v == 'a':
        cs = sup.spec(canvas, 'box', key='dodger')
        nr = D.pick(14, 8, 4)
        rings = []
        for x, hgt, zb in ((5.78, 0.06, ztop), (5.98, 0.30, ztop), (6.25, 0.50, ztop), (6.50, 0.58, ztop),
                           (6.74, 0.60, ztop - 0.02)):
            wd = 1.0 if x < 6.5 else 1.04
            outer = [(x, wd * math.cos(math.pi * i / nr), zb + hgt * math.sin(math.pi * i / nr) ** 0.5)
                     for i in range(nr + 1)]
            inner = [(x, (wd - 0.02) * math.cos(math.pi * i / nr), zb + (hgt - 0.02) * math.sin(math.pi * i / nr) ** 0.5)
                     for i in range(nr, -1, -1)]
            rings.append(outer + inner)
        loft(sup, rings, cs)
        if not D.far:
            gl = sup.spec('glass', 'flat')
            for sgn in (1, -1):
                box(sup, (6.07, sgn * 0.40, ztop + 0.30), (0.012, 0.52, 0.20), gl, axes(0, math.radians(-38), 0))

    # Mast, spreaders, boom and sail cover.
    al = rig.spec('alu', 'flat', sharp=100)
    mtop = H.sz(SAIL_MAST_X) + 13.41 + 0.22
    zb_mast = ztop - 0.02
    mx = SAIL_MAST_X
    tube(rig, [(mx, 0, zb_mast), (mx, 0, mtop)], 0.058, D.pick(12, 8, 4), al, ry=0.095)
    if not D.far:
        box(rig, (mx + 0.05, 0, mtop + 0.03), (0.32, 0.10, 0.06), al)
        zs = (zb_mast + mtop) / 2 + 0.2
        for sgn in (1, -1):
            tube(rig, [(mx, sgn * 0.05, zs), (mx + 0.08, sgn * 0.86, zs + 0.05)], 0.022, D.pick(6, 4), al, ry=0.012)
    if D.near:
        tube(rig, [(mx + 0.10, 0, mtop + 0.06), (mx + 0.10, 0, mtop + 0.98)], 0.006, 4, rig.spec('vinyl_black', 'flat'))
        tube(rig, [(mx - 0.05, 0, mtop + 0.06), (mx - 0.05, 0, mtop + 0.40)], 0.004, 3, rig.spec('stainless', 'flat'))
    zg = mtop - 0.20 - 11.73
    gx0, gx1 = mx + 0.12, mx + 3.58 + 0.25
    zg1 = zg + 0.18
    tube(rig, [(gx0, 0, zg), (gx1, 0, zg1)], 0.045, D.pick(8, 6, 4), al, ry=0.068)
    cov = rig.spec(canvas, 'box', key='cover', sharp=70)
    nseg, nsd = D.pick(26, 10, 4), D.pick(14, 8, 5)
    rings = []
    cx1 = gx1 - 0.40
    for i in range(nseg + 1):
        s = i / nseg
        x = lerp(mx + 0.02, cx1, s)
        zcen = lerp(zg, zg1, (x - gx0) / (gx1 - gx0))
        wd = lerp(0.36, 0.20, s) / 2
        ht = lerp(0.46, 0.25, s)
        tie = 0.9 if (D.near and i % 3 == 0 and 0 < i < nseg) else 1.0
        zc0 = zcen - 0.07 + ht / 2
        ring = []
        for k in range(nsd):
            a = 2 * math.pi * k / nsd
            cs_, sn = math.cos(a), math.sin(a)
            ring.append((x, wd * tie * math.copysign(abs(cs_) ** 0.8, cs_), zc0 + ht / 2 * tie * math.copysign(abs(sn) ** 0.75, sn)))
        rings.append(ring)
    loft(rig, rings, cov)
    if not D.far:
        collar = []
        for z, rx, rb in ((zg + 0.05, 0.20, 0.16), (zg + 0.40, 0.16, 0.13), (zg + 0.62, 0.12, 0.10)):
            collar.append([(mx + 0.06 + rx * math.cos(2 * math.pi * k / nsd), rb * math.sin(2 * math.pi * k / nsd), z)
                           for k in range(nsd)])
        loft(rig, collar, cov)

    # Standing rigging, furled jib, running rigging.
    hounds = (mx - 0.07, 0, mtop - 0.22)
    tack = (0.07, 0, H.sz(0.07) + 0.06)
    stern_head = (H.L + H.shear(H.L, H.sz(H.L)) - 0.06, 0, H.sz(H.L) + 0.08)
    wire = rig.spec('stainless', 'flat', sharp=100)
    wsides = D.pick(4, 3, 0)
    zsp = (zb_mast + mtop) / 2 + 0.25
    if wsides:
        tube(rig, [tack, hounds], 0.005, wsides, wire, caps=False)
        tube(rig, [(mx + 0.10, 0, mtop + 0.02), stern_head], 0.005, wsides, wire, caps=False)
        for sgn in (1, -1):
            cp = (mx + 0.08, sgn * (H.hb(mx + 0.08) - 0.07), H.sz(mx + 0.08) + 0.03)
            tube(rig, [(mx, sgn * 0.06, mtop - 0.3), (mx + 0.08, sgn * 0.85, zsp + 0.05), cp], 0.0045, wsides, wire,
                 caps=False)
            for dx in (-0.42, 0.45):
                lp = (mx + dx, sgn * (H.hb(mx + dx) - 0.08), H.sz(mx + dx) + 0.03)
                tube(rig, [(mx, sgn * 0.06, zsp - 0.1), lp], 0.004, wsides, wire, caps=False)
    furl = rig.spec(canvas, 'box', key='furl', sharp=70)
    drum = v3(tack) + v3((0.02, 0, 0.18))
    head = v3(tack) + (v3(hounds) - v3(tack)) * 0.955
    npts = D.pick(18, 8, 4)
    pts, radii = [], []
    for i in range(npts + 1):
        s = i / npts
        pts.append(tuple(drum + (head - drum) * s))
        radii.append(Curve([(0, .05), (.12, .085), (.35, .078), (.7, .05), (1, .022)])(s))
    tube(rig, pts, 0, D.pick(8, 6, 4), furl, radii=radii)
    if D.near:
        revolve(rig, tuple(drum - v3((0, 0, .17))), (0, 0, 1), [(0, 0), (0.0, .07), (.12, .07), (.13, 0.02), (.17, 0.02)],
                10, rig.spec('alu', 'flat'))
        rope = rig.spec('rope', 'flat')
        clew = v3(drum) + (head - drum) * 0.12
        for sgn in (1, -1):
            tube(rig, [tuple(clew), (5.2, sgn * 1.25, H.sz(5.2) + 0.1), (8.15, sgn * 1.07, zc(8.15, 1.1) + 0.40)], 0.006, 4,
                 rope, caps=False)
        tube(rig, [(mx + 0.15, 0, mtop - 0.05), (gx1 - 0.05, 0, zg1 + 0.05)], 0.004, 3, rope, caps=False)

    # Pulpit, pushpit, stanchions, lifelines.
    if not D.far:
        rs = gear.spec('stainless', 'flat', sharp=100)
        rsd = D.pick(6, 4)
        hgt = 0.62

        def edge(x, inset=0.07):
            return H.hb(x) - inset

        pul = []
        for sgn in (1, -1):
            pul.append([(1.15, sgn * edge(1.15), H.sz(1.15) + hgt), (0.62, sgn * edge(0.62, 0.09), H.sz(0.62) + hgt),
                        (0.30, sgn * 0.20, H.sz(0.3) + hgt)])
        path = pul[0][::-1][::-1] + [(0.16, 0.0, H.sz(0.16) + hgt)] + pul[1][::-1]
        path = pul[0] + [(0.16, 0.0, H.sz(0.16) + hgt)] + pul[1][::-1]
        tube(gear, [(1.15, edge(1.15), H.sz(1.15))] + path + [(1.15, -edge(1.15), H.sz(1.15))], 0.0125, rsd, rs)
        tube(gear, [(0.30, 0.20, H.sz(0.3) + hgt), (0.34, 0.24, H.sz(0.34))], 0.0125, rsd, rs)
        tube(gear, [(0.30, -0.20, H.sz(0.3) + hgt), (0.34, -0.24, H.sz(0.34))], 0.0125, rsd, rs)
        xp = 9.35
        xe = H.L + H.shear(H.L, H.sz(H.L)) - 0.10
        ph = 0.72
        push = [(xp, edge(xp), H.sz(xp)), (xp, edge(xp), H.sz(xp) + ph), (xe - 0.1, edge(xe) - 0.02, H.sz(xe) + ph),
                (xe, 0.8, H.sz(xe) + ph), (xe, -0.8, H.sz(xe) + ph), (xe - 0.1, -edge(xe) + 0.02, H.sz(xe) + ph),
                (xp, -edge(xp), H.sz(xp) + ph), (xp, -edge(xp), H.sz(xp))]
        tube(gear, push, 0.0125, rsd, rs)
        for sgn in (1, -1):
            tube(gear, [(xe - 0.1, sgn * (edge(xe) - 0.02), H.sz(xe) + ph), (xe - 0.05, sgn * (edge(xe) - 0.02), H.sz(xe))],
                 0.0125, rsd, rs)
        st_x = [1.65, 3.05, 4.45, 5.85, 7.25, 8.3]
        for sgn in (1, -1):
            tops = []
            for x in st_x:
                b = sgn * edge(x)
                tube(gear, [(x, b, H.sz(x)), (x, b, H.sz(x) + 0.61)], 0.0125, rsd, rs)
                tops.append((x, b, H.sz(x) + 0.61))
            if D.near:
                for frac in (1.0, 0.55):
                    line = [(1.15, sgn * edge(1.15), H.sz(1.15) + hgt * frac)]
                    line += [(x, b, H.sz(x) + 0.61 * frac) for x, b, _ in tops]
                    line += [(xp, sgn * edge(xp), H.sz(xp) + ph * frac)]
                    tube(gear, line, 0.003, 4, rs, caps=False)

    # Fenders over both sides.
    if not D.far:
        fs = gear.spec('fender_white' if v == 'a' else 'fender_navy', 'flat', sharp=100)
        ls = gear.spec('rope', 'flat')
        for x in (3.2, 5.2, 7.2):
            hw = max(H.halfwidth_at(x, z) for z in np.linspace(0.30, 0.80, 6))
            for sgn in (1, -1):
                fender(gear, D, x, sgn * (hw + 0.105), 0.56, 0.56, 0.10, fs, ls,
                       (x, sgn * (H.hb(x) - 0.03), H.sz(x) + 0.03))
    return [hull, sup, rig, gear]


def sail_body(variant):
    target = 5420.0 / SEAWATER - foil_volume(SAIL_KEEL) - foil_volume(SAIL_RUDDER)
    k = solve_depth(sail_tables, target)
    H = sail_tables(k)
    if variant == 'a':
        paint = dict(top='c9c7bd', mottle=0.09, chalk=0.45, chalk_col='a59f8b', rough=0.82, boot=(0.03, 0.12, '1e2b4a'),
                     af='1e2026', bands=[(0.10, 0.17, '1e2b4a'), (0.20, 0.215, '1e2b4a')], streaks=11, grime_amt=0.62,
                     lowgrime=0.45, sources=[3.0, 4.2, 4.6, 5.2, 7.2],
                     rust=dict(sources=[(x, 0.02) for x in (1.0, 2.8, 4.1, 4.3, 6.4, 8.2)], amount=0.35,
                               len=(0.08, 0.3), width=(0.01, 0.03)),
                     stain=dict(amount=0.9, col='6b5434', height=0.3, reach=2.6, base=0.35), broad_every=2)
    else:
        paint = dict(top='202638', mottle=0.07, chalk=0.6, chalk_col='4f5566', rough=0.85, boot=(0.03, 0.09, 'c7c3b5'),
                     af='5a1d16', bands=[(0.10, 0.125, 'b9b3a0')], streaks=7, grime='8c8878', grime_amt=0.4,
                     lowgrime=0.3, low_col='6c6a5c', sources=[3.0, 4.2, 4.6, 7.2],
                     rust=dict(sources=[(x, 0.02) for x in (1.0, 2.8, 4.1, 4.3, 6.4, 8.2)], amount=0.3,
                               len=(0.08, 0.3), width=(0.01, 0.03)),
                     stain=dict(amount=0.4, col='6e5a3e', height=0.2, reach=2.4, base=0.3), broad_every=2)
    meta = dict(cls='34 ft fin-keel masthead sloop, moored', sources=['sail-c34', 'sail-c34-tallrig', 'sail-airdraft'],
                published=dict(loaM=10.52, lwlM=9.09, beamM=3.58, draftM=1.70, displacementKg=5420),
                underwaterDepthScale=round(k, 4))
    label = 'Sailing yacht, 10.5 m sloop, ' + ('white hull, navy canvas, dodger' if variant == 'a'
                                                else 'navy hull, tan canvas')
    return Body('sailYacht-' + variant, label, H, paint, 20000, sail_build, meta)


# ------------------------------------------------------------ motor yacht ---
def motor_tables(k):
    keel = [(0, 2.00), (.03, 1.30), (.07, .48), (.105, 0.0), (.18, -.38), (.3, -.62), (.5, -.72), (.75, -.72),
            (1, -.68)]
    chz = [(0, 1.70), (.05, 1.05), (.15, .32), (.3, .02), (.5, -.10), (1, -.12)]
    return Hull(12.15,
                sheer=[(0, 2.00), (.08, 1.90), (.25, 1.74), (.5, 1.57), (.75, 1.43), (1, 1.34)],
                keel=scale_below(keel, k),
                half=[(0, 0), (.05, .58), (.15, 1.36), (.3, 1.89), (.5, 2.10), (.75, 2.10), (1, 2.03)],
                chine=dict(half=[(0, 0), (.05, .30), (.15, 1.10), (.3, 1.68), (.5, 1.90), (.75, 1.93), (1, 1.88)],
                           z=scale_below(chz, k), flat=0.07),
                rake=-8.0, rake_len=0.9, camber=0.08)


MOTOR_COCKPIT = 9.70
MOTOR_PLATFORM = 1.03
MOTOR_PULPIT = 0.25


def motor_build(body, D):
    v = body.variant
    H = body.hull
    hull, sup, rig, gear = Part('Hull'), Part('Superstructure'), Part('Rig'), Part('Gear')
    gel = 'gel_white' if v == 'a' else 'gel_cream'
    deck_t = 'deck_white' if v == 'a' else 'deck_cream'
    canvas = 'canvas_navy' if v == 'a' else 'canvas_tan'
    ck = MOTOR_COCKPIT
    d_ck = 0.52

    def zc(x, b):
        hb = max(H.hb(x), 1e-4)
        return H.sz(x) + H.camber * min(1.0, hb / H.hbmax) * (1 - min(1.0, (b / hb) ** 2))

    def deck_fn(x, hb, sz):
        th = min(0.07, hb * 0.3)
        if x >= ck:
            pts = [(hb, sz), (hb - th, sz), (hb - th, sz - d_ck), (0.0, sz - d_ck)]
            tiles = [(gel, 'top'), (gel, 'side'), (deck_t, 'top')]
        else:
            pts = [(hb, sz), (hb - th, zc(x, hb - th)), (hb - th, zc(x, hb - th)), (0.0, zc(x, 0))]
            tiles = [(deck_t, 'top')] * 3
        return pts, tiles

    hull_shell(hull, body, D, deck_fn, breaks=(ck,), n_st=(46, 14, 7), n_sec=(24, 8, 4))
    sole = H.sz(H.L) - d_ck
    xt = H.L + H.shear(H.L, 0.36)
    # Swim platform and bow pulpit.
    box(hull, (xt + MOTOR_PLATFORM / 2 - 0.05, 0, 0.36), (MOTOR_PLATFORM + 0.1, 3.5, 0.12), hull.spec(gel, 'box'))
    box(hull, (xt + MOTOR_PLATFORM / 2 - 0.03, 0, 0.43), (MOTOR_PLATFORM, 3.4, 0.025), hull.spec('teak', 'box'))
    box(hull, (0.05, 0, H.sz(0.05) + 0.04), (0.6, 0.42, 0.08), hull.spec(gel, 'box'))
    if not D.far:
        box(hull, (-MOTOR_PULPIT + 0.06, 0, H.sz(0) - 0.03), (0.12, 0.10, 0.16), hull.spec('stainless', 'flat'))
        strip_along(hull, H, 0.15, H.L - 0.01, D.pick(24, 8),
                    lambda x: (H.halfwidth_at(x, H.sz(x) - 0.04) + 0.015, H.sz(x) - 0.04), 0.03, 0.06,
                    hull.spec('stainless' if v == 'a' else 'rubber', 'flat'))
    # Running gear: shafts, struts, propellers, rudders (draft 1.17 m at the rudders).
    af = 'af_black' if v == 'a' else 'af_red'
    rg = hull.spec('stainless', 'flat')
    for sgn in (1, -1):
        b = sgn * 0.62
        tube(hull, [(6.6, b, -0.55), (10.45, b, -0.86)], 0.035, D.pick(8, 5, 3), rg)
        if not D.far:
            box(hull, (9.9, b, -0.70), (0.10, 0.04, 0.32), rg)
            revolve(hull, (10.45, b, -0.86), (1, 0, -0.08), [(0, 0.0), (0.01, 0.08), (0.14, 0.06), (0.16, 0)], 8, rg)
            for k in range(D.pick(4, 3)):
                a = 2 * math.pi * k / D.pick(4, 3)
                box(hull, (10.48, b + 0.16 * math.cos(a), -0.86 + 0.16 * math.sin(a)), (0.05, 0.10, 0.26), rg,
                    axes(0, 0, a))
        box(hull, (10.95, b, -0.86), (0.42, 0.035, 0.62), hull.spec(af, 'box'))
    if D.near:
        tube(hull, [(6.0, 0, -0.80), (9.8, 0, -0.80)], 0.04, 4, hull.spec(af, 'flat'))

    # Salon with window band, flybridge, arch.
    def side_hw(x):
        return max(0.4, H.hb(x) - 0.34)
    ns, nn = D.pick(12, 6, 3), D.pick(10, 5, 2)
    z_s = H.sz(6.0)
    z_base = min(H.sz(x) for x in np.linspace(3.4, ck, 12)) - 0.08
    lv = [(z_base, outline(3.40, ck + 0.012, side_hw, 1.3, ns, nn)),
          (z_s + 0.40, outline(3.48, ck + 0.012, side_hw, 1.3, ns, nn)),
          (z_s + 1.02, outline(4.45, ck + 0.012, lambda x: side_hw(x) - 0.07, 1.0, ns, nn)),
          (z_s + 1.16, outline(4.80, ck + 0.012, lambda x: side_hw(x) - 0.09, 0.9, ns, nn))]
    g = sup.spec(gel, 'side', key='salon')
    gl = sup.spec('glass', 'flat', key='glass')
    plan_loft(sup, lv, [g, gl, g], cap_sp=sup.spec(gel, 'top', key='roof'))
    z_roof = z_s + 1.18
    if not D.far:
        fr = sup.spec(gel, 'box', key='mullion')
        for xm in (6.0, 8.0):
            for sgn in (1, -1):
                tube(sup, [(xm, sgn * (side_hw(xm) - 0.02), z_s + 0.40), (xm, sgn * (side_hw(xm) - 0.09), z_s + 1.02)],
                     0.035, 4, fr)
        for sgn in (1, -1):
            tube(sup, [(3.62, sgn * 0.78, z_s + 0.40), (4.55, sgn * 0.60, z_s + 1.02)], 0.035, 4, fr)
            tube(sup, [(ck, sgn * 0.55, z_s + 0.40), (ck, sgn * 0.55, z_s + 1.02)], 0.04, 4, fr)
    # Flybridge: deck slab over the salon and cockpit, coaming, visor screen, helm, seats.
    xb0, xb1 = 4.95, H.L - 0.25

    def bridge_hw(x):
        return min(H.hb(x) - 0.42, 1.85)
    nb = D.pick(14, 7, 3)
    zb0, zb1 = z_roof - 0.02, z_roof + 0.24
    lv = [(zb0, outline(xb0 + 0.55, xb1, lambda x: bridge_hw(x) - 0.12, 1.2, nb, nn)),
          (zb1, outline(xb0, xb1, bridge_hw, 1.4, nb, nn))]
    plan_loft(sup, lv, [g], cap_sp=sup.spec(deck_t, 'top', key='bridge-deck'))
    path = outline(xb0, 7.6, lambda x: bridge_hw(x) - 0.03, 1.4, D.pick(8, 4, 2), nn, n_aft=1)
    path = path[:len(path) - 0]
    front = [p for p in path if p[0] < 7.59]
    zt = [zb1 + lerp(0.55, 0.42, clamp((x - xb0) / 3.0, 0, 1)) for x, _ in front]
    wall(sup, front, [zb1 - 0.05] * len(front), zt, 0.06, sup.spec(gel, 'side', key='coaming'))
    if not D.far:
        scr = [p for p in front if p[0] < xb0 + 1.6]
        wall(sup, scr, [zt[i] for i in range(len(scr))], [zt[i] + 0.16 for i in range(len(scr))], 0.012, gl)
    cover = sup.spec(canvas, 'box', key='helmcover')
    box(sup, (xb0 + 1.15, -0.55, zb1 + 0.45), (0.65, 0.95, 0.9), cover)
    if not D.far:
        cush = sup.spec('cushion_white', 'box', key='cushion')
        box(sup, (xb0 + 2.4, 1.15, zb1 + 0.25), (1.8, 0.55, 0.5), cush)
        box(sup, (xb0 + 2.4, 1.38, zb1 + 0.55), (1.8, 0.12, 0.45), cush)
        box(sup, (xb0 + 2.1, -0.55, zb1 + 0.30), (0.6, 0.6, 0.6), cover)
        box(sup, (xb0 + 3.3, 0.0, zb1 + 0.25), (0.9, 0.6, 0.45), sup.spec('teak', 'box'))
    # Radar arch at the aft end of the bridge, radar dome on it, antennas lowered.
    xa_, top = 10.65, 5.92 - 0.37 - 0.10
    ars = sup.spec(gel, 'box', key='arch')
    for sgn in (1, -1):
        tube(sup, [(xa_ - 1.10, sgn * 1.62, zb1 - 0.02), (xa_ - 0.40, sgn * 1.50, (zb1 + top) / 2),
                   (xa_ + 0.2, sgn * 1.30, top)], 0.05, D.pick(6, 4, 4), ars, ry=0.14)
    box(sup, (xa_ + 0.2, 0, top + 0.02), (0.42, 2.75, 0.10), ars)
    revolve(sup, (xa_ + 0.2, 0, top + 0.08), (0, 0, 1), [(0, 0), (0.0, 0.30), (0.25, 0.30), (0.33, 0.18), (0.37, 0)],
            D.pick(14, 8, 6), sup.spec('cushion_white', 'flat'))
    if not D.far:
        for sgn in (1, -1):
            tube(sup, [(xa_ + 0.1, sgn * 1.05, top + 0.1), (xa_ + 2.3, sgn * 1.05, top + 0.18)], 0.012, 4,
                 sup.spec('fender_white', 'flat'))
    # Bridge posts over the cockpit, ladder, rails.
    rs = gear.spec('stainless', 'flat', sharp=100)
    rsd = D.pick(6, 4, 0)
    if rsd:
        for sgn in (1, -1):
            tube(gear, [(xb1 - 0.25, sgn * (bridge_hw(xb1) - 0.15), H.sz(xb1 - 0.25)), (xb1 - 0.25, sgn * (bridge_hw(xb1) - 0.15), zb0)],
                 0.035, rsd, rs)
            rail = [(x, sgn * (bridge_hw(x) - 0.05), zb1 + 0.75) for x in np.linspace(7.6, xb1 - 0.1, 6)]
            rail += [(xb1 - 0.05, sgn * 0.9, zb1 + 0.75)]
            tube(gear, rail, 0.0125, rsd, rs)
            for x in np.linspace(7.8, xb1 - 0.1, 4):
                tube(gear, [(x, sgn * (bridge_hw(x) - 0.05), zb1), (x, sgn * (bridge_hw(x) - 0.05), zb1 + 0.75)], 0.012,
                     rsd, rs)
            bow = [(x, sgn * (H.hb(x) - 0.06), H.sz(x) + 0.65) for x in (5.2, 4.0, 2.8, 1.7, 0.85)]
            bow += [(0.35, sgn * 0.22, H.sz(0.35) + 0.65), (0.05, 0, H.sz(0.05) + 0.65)]
            tube(gear, bow, 0.0125, rsd, rs)
            for x in (5.2, 4.0, 2.8, 1.7, 0.85):
                tube(gear, [(x, sgn * (H.hb(x) - 0.06), H.sz(x)), (x, sgn * (H.hb(x) - 0.06), H.sz(x) + 0.65)], 0.0125,
                     rsd, rs)
        if D.near:
            for sgn in (1, -1):
                tube(gear, [(ck + 0.55, sgn * 0.45, sole), (ck + 0.20, sgn * 0.45, zb1 + 0.6)], 0.016, 5, rs)
            for k in range(6):
                f = (k + 1) / 7
                z = lerp(sole, zb1, f)
                x = lerp(ck + 0.55, ck + 0.20, f)
                box(gear, (x, 0, z), (0.12, 0.9, 0.03), gear.spec('teak', 'box'))
            box(gear, (H.L - 0.35, 0, sole + 0.22), (0.45, 3.0, 0.44), gear.spec('cushion_white', 'box'))
            box(gear, (0.9, 0, zc(0.9, 0) + 0.12), (0.35, 0.28, 0.24), gear.spec('stainless', 'flat'))
    if v == 'b':
        # Bimini over the forward bridge.
        bs = sup.spec(canvas, 'box', key='bimini')
        nr = D.pick(12, 6, 3)
        rings = []
        for x in (xb0 + 0.45, xb0 + 1.3, xb0 + 2.4, xb0 + 3.0):
            wdt = bridge_hw(x) - 0.1
            outer = [(x, wdt * math.cos(math.pi * i / nr), zb1 + 1.65 + 0.25 * math.sin(math.pi * i / nr) ** 0.7)
                     for i in range(nr + 1)]
            inner = [(x, wdt * math.cos(math.pi * i / nr), zb1 + 1.63 + 0.25 * math.sin(math.pi * i / nr) ** 0.7)
                     for i in range(nr, -1, -1)]
            rings.append(outer + inner)
        loft(sup, rings, bs)
        if rsd:
            for x in (xb0 + 0.5, xb0 + 2.95):
                for sgn in (1, -1):
                    tube(gear, [(x, sgn * (bridge_hw(x) - 0.12), zb1), (x, sgn * (bridge_hw(x) - 0.12), zb1 + 1.66)],
                         0.013, rsd, rs)
    # Fenders.
    if not D.far:
        fs = gear.spec('fender_white' if v == 'a' else 'fender_navy', 'flat', sharp=100)
        ls = gear.spec('rope', 'flat')
        for x in (3.6, 6.5, 9.2):
            hw = max(H.halfwidth_at(x, z) for z in np.linspace(0.35, 1.0, 6))
            for sgn in (1, -1):
                fender(gear, D, x, sgn * (hw + 0.13), 0.70, 0.66, 0.12, fs, ls, (x, sgn * (H.hb(x) - 0.05), H.sz(x) + 0.03))
    return [hull, sup, rig, gear]


def motor_body(variant):
    k = solve_depth(motor_tables, 15263.0 / SEAWATER - 0.06)
    H = motor_tables(k)
    win = [(2.6, 5.8, 0.30, 0.50, '16191c'), (6.4, 6.75, 0.33, 0.48, '16191c'), (7.1, 7.45, 0.33, 0.48, '16191c')]
    if variant == 'a':
        paint = dict(top='cac8be', mottle=0.09, chalk=0.45, chalk_col='a59f8b', rough=0.82, boot=(0.04, 0.14, '1e2b4a'),
                     af='1e2026', windows=win, bands=[(0.12, 0.20, '1e2b4a')], streaks=11, grime_amt=0.62, lowgrime=0.45,
                     sources=[4.0, 6.0, 9.7],
                     rust=dict(sources=[(x, 0.02) for x in (1.2, 3.0, 6.1, 9.6)], amount=0.3, len=(0.08, 0.3),
                               width=(0.01, 0.03)),
                     stain=dict(amount=0.9, col='6b5434', height=0.32, reach=3.0, base=0.35), broad_every=2)
    else:
        paint = dict(top='cbbfa0', mottle=0.09, chalk=0.38, chalk_col='a0957b', rough=0.82, boot=(0.04, 0.12, '2a3a33'),
                     af='5a1d16', windows=win, bands=[(0.12, 0.17, '2a3a33')], streaks=11, grime_amt=0.62, lowgrime=0.45,
                     low_col='5a5040', sources=[4.0, 6.0, 9.7],
                     rust=dict(sources=[(x, 0.02) for x in (1.2, 3.0, 6.1, 9.6)], amount=0.3, len=(0.08, 0.3),
                               width=(0.01, 0.03)),
                     stain=dict(amount=0.85, col='6b5434', height=0.32, reach=3.0, base=0.35), broad_every=2)
    meta = dict(cls='42 ft flybridge sport sedan, moored', sources=['motor-c42ss', 'motor-sedanbridge'],
                published=dict(loaM=13.28, beamM=4.24, draftM=1.17, airDraftM=5.92, displacementKg=15263),
                underwaterDepthScale=round(k, 4), swimPlatformM=MOTOR_PLATFORM, bowPulpitM=MOTOR_PULPIT)
    label = 'Motor yacht, 13.3 m flybridge sedan, ' + ('white, navy canvas' if variant == 'a'
                                                       else 'ivory, tan canvas and bimini')
    return Body('motorYacht-' + variant, label, H, paint, 20000, motor_build, meta)


# ---------------------------------------------------------------- jet ski ---
JET_MASS = 326.0 + 70.0 * 0.74


def jet_tables(d):
    return Hull(3.30,
                sheer=[(0, .47 - d), (.05, .45 - d), (.2, .43 - d), (.5, .42 - d), (.8, .41 - d), (1, .40 - d)],
                keel=[(0, .47 - d), (.04, .27 - d), (.10, .12 - d), (.2, .045 - d), (.32, .012 - d), (.45, -d), (1, -d)],
                half=[(0, 0), (.03, .22), (.10, .45), (.22, .57), (.4, .615), (.7, .61), (.9, .585), (1, .55)],
                chine=dict(half=[(0, 0), (.05, .17), (.15, .36), (.3, .47), (.6, .50), (1, .47)],
                           z=[(0, .45 - d), (.05, .26 - d), (.15, .16 - d), (.35, .15 - d), (1, .16 - d)], flat=0.04),
                rake=0.0, camber=0.0)


JET_WALL = Curve([(0.0, .52), (0.25, .60), (0.7, .66), (1.3, .69), (2.3, .67), (2.70, .64), (2.86, .54), (3.3, .53)])
JET_HOOD = Curve([(0.0, .54), (0.15, .64), (0.5, .77), (0.9, .85), (1.12, .87), (1.32, .80), (1.5, .74)])
JET_PED = Curve([(1.1, .74), (1.5, .74), (2.6, .73), (2.85, .55)])


def jet_build(body, D):
    v = body.variant
    H = body.hull
    d = body.meta['draftM']
    hull, sup, rig, gear = Part('Hull'), Part('Superstructure'), Part('Rig'), Part('Gear')
    deck_t = 'ski_blue' if v == 'a' else 'ski_red'
    side_t = 'hull:' + body.key
    mat_t = 'rubber'

    def Z(z):
        return z - d

    def deck_fn(x, hb, sz):
        zw = Z(JET_WALL(x))
        bw = max(hb - 0.05, 0.0)

        def dome(b):
            return zw + (Z(JET_HOOD(x)) - zw) * max(0.0, 1 - (b / max(bw, 1e-6)) ** 2) ** 0.6
        f = smoothstep((x - 1.05) / 0.40)
        g = smoothstep((x - 2.75) / 0.12)
        zfw = Z(lerp(0.50, 0.52, g))
        zped = lerp(Z(JET_PED(x)), zfw, g)
        b1 = max(hb - 0.012, 0.0)
        b3, b4 = max(bw - 0.05, bw * 0.8), max(bw - 0.07, bw * 0.7)
        b5, b6 = min(0.27, bw * 0.45), min(0.22, bw * 0.38)
        pts = [(hb, sz), (b1, sz + 0.035), (bw, zw), (b3, lerp(dome(b3), zw + 0.01, f)), (b4, lerp(dome(b4), zfw, f)),
               (b5, lerp(dome(b5), zfw, f)), (b6, lerp(dome(b6), zped, f)), (0.0, lerp(dome(0.0), zped + 0.005, f))]
        if f < 0.5:
            tiles = [('rubber', 'flat'), (side_t, 'hull')] + [(deck_t, 'top')] * 5
        elif g < 0.5:
            tiles = [('rubber', 'flat'), (side_t, 'hull'), (side_t, 'hull'), (side_t, 'hull'), (mat_t, 'top'),
                     (side_t, 'hull'), (side_t, 'hull')]
        else:
            tiles = [('rubber', 'flat'), (side_t, 'hull'), (side_t, 'hull'), (side_t, 'hull'), (mat_t, 'top'),
                     (mat_t, 'top'), (mat_t, 'top')]
        return pts, tiles

    hull_shell(hull, body, D, deck_fn, n_st=(48, 14, 6), n_sec=(16, 7, 3))
    if not D.far:
        strip_along(hull, H, 0.06, H.L - 0.01, D.pick(24, 8), lambda x: (H.hb(x) + 0.006, H.sz(x) + 0.01), 0.022,
                    0.045, hull.spec('rubber', 'flat'))
    noz = hull.spec('rubber', 'flat')
    tube(hull, [(H.L - 0.10, 0, Z(0.14)), (3.37, 0, Z(0.14))], 0.075, D.pick(10, 6, 4), noz)
    if D.near:
        box(hull, (H.L - 0.15, 0, Z(0.012)), (0.35, 0.36, 0.02), hull.spec('stainless', 'flat'))
        for sgn in (1, -1):
            box(hull, (H.L - 0.22, sgn * (H.halfwidth_at(H.L - 0.22, Z(0.2)) + 0.02), Z(0.2)), (0.30, 0.03, 0.10),
                hull.spec('rubber', 'flat'))
    # Seat with a rear hump.
    seat = sup.spec('vinyl_grey' if v == 'a' else 'vinyl_black', 'box', key='seat', sharp=70)
    top_z = Curve([(1.40, .86), (1.55, .92), (2.15, .90), (2.45, .935), (2.65, .925), (2.78, .82)])
    half_w = Curve([(1.40, .14), (1.55, .20), (2.5, .20), (2.78, .14)])
    nseg, nsd = D.pick(14, 6, 3), D.pick(12, 8, 4)
    rings = []
    for i in range(nseg + 1):
        x = lerp(1.41, 2.77, i / nseg)
        zt, zb, hw = Z(top_z(x)), Z(JET_PED(x) - 0.02), half_w(x)
        cz, hz = (zt + zb) / 2, (zt - zb) / 2
        ring = []
        for k in range(nsd):
            a = 2 * math.pi * k / nsd
            cs, sn = math.cos(a), math.sin(a)
            ring.append((x, hw * math.copysign(abs(cs) ** 0.4, cs), cz + hz * math.copysign(abs(sn) ** 0.5, sn)))
        rings.append(ring)
    loft(sup, rings, seat)
    # Steering pod, handlebar with its pad, mirrors on the hood.
    pod = sup.spec(deck_t, 'box', key='pod')
    nn = D.pick(6, 3, 2)
    plan_loft(sup, [(Z(0.76), outline(0.98, 1.46, lambda x: 0.23, 0.22, 3, nn)),
                    (Z(0.94), outline(1.05, 1.42, lambda x: 0.18, 0.16, 3, nn)),
                    (Z(1.02), outline(1.14, 1.36, lambda x: 0.10, 0.08, 3, nn))], [pod, pod])
    bar = sup.spec('vinyl_black', 'flat', sharp=100)
    hb_pts = [(1.24, -0.38, Z(1.13)), (1.23, -0.15, Z(1.10)), (1.23, 0.15, Z(1.10)), (1.24, 0.38, Z(1.13))]
    tube(sup, hb_pts, 0.014, D.pick(6, 4, 3), bar)
    tube(sup, [(1.26, 0, Z(1.00)), (1.23, 0, Z(1.10))], 0.03, D.pick(6, 4, 3), bar)
    box(sup, (1.23, 0, Z(1.165)), (0.10, 0.30, 0.07), sup.spec('vinyl_grey' if v == 'a' else 'vinyl_black', 'box'))
    if not D.far:
        for sgn in (1, -1):
            tube(sup, [(1.24, sgn * 0.30, Z(1.125)), (1.24, sgn * 0.41, Z(1.14))], 0.02, 6, bar)
            tube(sup, [(0.98, sgn * 0.33, Z(0.80)), (0.97, sgn * 0.41, Z(0.90))], 0.012, 4, bar)
            box(sup, (0.97, sgn * 0.44, Z(0.93)), (0.03, 0.15, 0.08), sup.spec('vinyl_black', 'box'))
        box(sup, (1.10, 0, Z(0.99)), (0.10, 0.26, 0.10), sup.spec('glass', 'flat'), axes(0, math.radians(-35), 0))
        tube(sup, [(2.70, -0.13, Z(0.84)), (2.82, -0.11, Z(0.90)), (2.82, 0.11, Z(0.90)), (2.70, 0.13, Z(0.84))],
             0.014, 5, sup.spec('vinyl_black', 'flat'))
    return [hull, sup, rig, gear]


def jet_body(variant):
    lo, hi = 0.05, 0.40
    target = JET_MASS / SEAWATER
    for _ in range(40):
        mid = (lo + hi) / 2
        if jet_tables(0.0).volume(z_wl=mid) < target:
            lo = mid
        else:
            hi = mid
    d = (lo + hi) / 2
    H = jet_tables(d)
    zb = H.sz(1.0)
    if variant == 'a':
        paint = dict(top='c9c8c0', mottle=0.04, bands=[(0.05, 0.065, '1f3a7a')], streaks=2,
                     swoosh=(0.55, 1.3, zb + 0.03, '1f3a7a'), af=None, lowgrime=0.2, scum=0.5, slime=0.45, rough=0.6,
                     af_top=0.0)
    else:
        paint = dict(top='3b3d40', mottle=0.06, bands=[(0.05, 0.065, 'a9a49a')], streaks=2,
                     swoosh=(0.55, 1.3, zb + 0.03, '7c1c14'), af=None, lowgrime=0.15, scum=0.5, slime=0.45, rough=0.6,
                     af_top=0.0, grime='8a8578')
    meta = dict(cls='three-seat personal watercraft, afloat', sources=['jetski-vx'],
                published=dict(loaM=3.37, beamM=1.24, heightM=1.20, dryKg=326, fuelL=70),
                draftM=round(d, 4), massKg=JET_MASS, tileTopZ=round(0.74 - d, 3))
    label = 'Personal watercraft, 3.4 m, ' + ('white and blue' if variant == 'a' else 'charcoal and red')
    return Body('jetSki-' + variant, label, H, paint, 6000, jet_build, meta)


# ----------------------------------------------------------- fishing boat ---
def fish_tables():
    return Hull(12.20,
                sheer=[(0, 2.28), (.08, 2.10), (.25, 1.80), (.45, 1.58), (.65, 1.47), (.85, 1.43), (1, 1.45)],
                keel=[(0, 2.28), (.015, 1.30), (.035, .40), (.048, 0.0), (.09, -.45), (.18, -.72), (.4, -.82),
                      (.65, -.80), (.82, -.62), (.93, -.30), (.975, 0.0), (1, .12)],
                half=[(0, 0), (.04, .36), (.12, 1.06), (.25, 1.62), (.4, 1.86), (.6, 1.89), (.8, 1.80), (1, 1.58)],
                beta=[(0, 58), (.15, 38), (.4, 24), (.7, 19), (1, 12)],
                flare=[(0, 38), (.12, 28), (.3, 10), (.5, 2), (.75, 0), (1, -3)],
                full=[(0, .40), (.2, .62), (.45, .74), (.75, .76), (1, .70)],
                rake=12.0, rake_len=1.0, camber=0.09)


FISH_BULWARK = Curve([(0, .45), (.2, .58), (.5, .66), (1, .70)])


def fish_build(body, D):
    v = body.variant
    H = body.hull
    hull, sup, rig, gear = Part('Hull'), Part('Superstructure'), Part('Rig'), Part('Gear')
    house = 'paint_white' if v == 'a' else 'paint_cream'
    cap = 'paint_blue' if v == 'a' else 'wood'

    def bh(x):
        return FISH_BULWARK(H.t(x))

    def deck_z(x, b=0.0):
        hb = max(H.hb(x), 1e-4)
        return H.sz(x) - bh(x) + H.camber * min(1.0, hb / H.hbmax) * (1 - min(1.0, (b / hb) ** 2))

    def deck_fn(x, hb, sz):
        th = min(0.10, hb * 0.4)
        pts = [(hb, sz), (hb - th, sz), (hb - th, sz - bh(x)), (0.0, deck_z(x))]
        tiles = [(cap, 'top'), (house, 'side'), ('deck_grey', 'top')]
        return pts, tiles

    hull_shell(hull, body, D, deck_fn, n_st=(48, 16, 8), n_sec=(24, 9, 4))
    # Keel with drag, prop aperture, skeg shoe and rudder.
    af = 'af_red'
    ks = hull.spec(af, 'side', key='keel')
    nk = D.pick(16, 8, 4)
    rings = []
    for i in range(nk + 1):
        x = lerp(0.95, 10.95, i / nk)
        top = H.kz(x) + 0.12
        bot = lerp(-0.95, -1.75, clamp((x - 0.95) / 9.7, 0, 1))
        th = 0.11 if 0 < i < nk else 0.07
        rings.append([(x, th, top), (x, th, bot), (x, -th, bot), (x, -th, top)])
    loft(hull, rings, ks)
    box(hull, (11.40, 0, -1.71), (0.95, 0.10, 0.08), ks)
    box(hull, (11.52, 0, -0.98), (0.50, 0.08, 1.46), ks)
    if not D.far:
        pr = hull.spec('rust_steel', 'flat')
        revolve(hull, (11.05, 0, -1.18), (1, 0, 0), [(-0.12, 0), (-0.1, 0.08), (0.1, 0.08), (0.12, 0)], 8, pr)
        for k in range(4):
            a = 2 * math.pi * k / 4 + 0.4
            box(hull, (11.05, 0.24 * math.cos(a), -1.18 + 0.24 * math.sin(a)), (0.06, 0.18, 0.44), pr, axes(0, 0, a))
    # Rub rails and stem iron.
    if not D.far:
        rr = hull.spec('wood', 'box', key='rub')
        strip_along(hull, H, 0.25, H.L - 0.05, D.pick(24, 8),
                    lambda x: (H.halfwidth_at(x, H.sz(x) - 0.07) + 0.025, H.sz(x) - 0.07), 0.05, 0.075, rr)
        strip_along(hull, H, 0.9, H.L - 0.1, D.pick(24, 8),
                    lambda x: (H.halfwidth_at(x, deck_z(x) - 0.05) + 0.025, deck_z(x) - 0.05), 0.05, 0.07, rr)
        stem = [(x + 0.0, 0, H.kz(x)) for x in np.linspace(0.0, 0.7, 8)]
        tube(hull, stem, 0.03, 4, hull.spec('rust_steel', 'flat'))
    # Wheelhouse forward.
    x0, x1 = 2.70, 6.10

    def hw(x):
        return min(H.hb(x) - 0.55, 1.30)
    zd = deck_z(4.4)
    ns, nn = D.pick(10, 5, 3), D.pick(8, 4, 2)
    lv = [(zd - 0.12, outline(x0, x1, hw, 0.55, ns, nn)),
          (zd + 1.05, outline(x0, x1, hw, 0.55, ns, nn)),
          (zd + 1.70, outline(x0 - 0.12, x1, hw, 0.55, ns, nn)),
          (zd + 1.76, outline(x0 - 0.18, x1 + 0.08, lambda x: hw(x) + 0.07, 0.55, ns, nn)),
          (zd + 1.98, outline(x0 - 0.20, x1 + 0.10, lambda x: hw(x) + 0.08, 0.55, ns, nn))]
    hs_ = sup.spec(house, 'side', key='house')
    gl = sup.spec('glass', 'flat', key='glass')
    trim = sup.spec('paint_blue' if v == 'a' else 'paint_red', 'side', key='trim')
    plan_loft(sup, lv, [hs_, gl, trim, trim], cap_sp=sup.spec(house, 'top', key='roof'), crown=(zd + 2.04, 0.3))
    z_roof = zd + 2.04
    if not D.far:
        post = sup.spec(house, 'box', key='post')
        bot = outline(x0, x1, hw, 0.55, ns, nn)
        topo = outline(x0 - 0.12, x1, hw, 0.55, ns, nn)
        for i in range(len(bot)):
            if i % 2 == 0 or bot[i][0] > x1 - 0.05:
                p0 = (bot[i][0], bot[i][1], zd + 1.05)
                p1 = (topo[i][0], topo[i][1], zd + 1.70)
                tube(sup, [p0, p1], 0.06, 4, post)
        box(sup, (x1 + 0.005, -0.35, zd + 0.95), (0.04, 0.7, 1.95), post)
        box(sup, (x1 + 0.01, -0.35, zd + 1.45), (0.04, 0.45, 0.40), sup.spec('glass', 'flat'))
        if D.near:
            torus(sup, (x1 + 0.03, 0.55, zd + 1.15), (1, 0, 0), 0.30, 0.045, 14, 5, sup.spec('orange', 'flat'))
    # Mast, crosstree, radar, lights, antennas, stack.
    pole = rig.spec(house, 'flat', sharp=100)
    mx = 5.75
    m_top = 8.40
    tube(rig, [(mx, 0, z_roof - 0.05), (mx, 0, m_top)], 0.0, D.pick(10, 6, 4), pole,
         radii=[0.08, 0.05])
    tube(rig, [(mx, -1.05, 7.0), (mx, 1.05, 7.0)], 0.035, D.pick(6, 4, 3), pole)
    box(rig, (mx - 0.25, 0, 4.62), (0.10, 1.30, 0.14), rig.spec('cushion_white', 'box'))
    box(rig, (mx - 0.10, 0, 4.50), (0.30, 0.20, 0.12), rig.spec('cushion_white', 'box'))
    revolve(rig, (4.0, 0, z_roof), (0, 0, 1), [(0, 0), (0, 0.27), (0.2, 0.27), (0.27, 0.15), (0.30, 0)], D.pick(12, 8, 5),
            rig.spec('cushion_white', 'flat'))
    st = rig.spec('stainless', 'flat', sharp=100)
    if not D.far:
        for sgn in (1, -1):
            tube(rig, [(x1 - 0.1, sgn * 1.15, z_roof - 0.02), (x1 - 0.1, sgn * 1.15, z_roof + 2.4)], 0.012, 4,
                 rig.spec('fender_white', 'flat'))
        tube(rig, [(x1 - 0.3, 0.6, z_roof), (x1 - 0.15, 0.9, z_roof + 2.2)], 0.012, 4, rig.spec('fender_white', 'flat'))
        stk = rig.spec('rubber', 'flat')
        tube(rig, [(x1 + 0.25, 0.95, deck_z(x1 + 0.25, 0.95)), (x1 + 0.25, 0.95, z_roof + 1.0)], 0.08, 8, stk)
        revolve(rig, (2.95, 0.5, z_roof), (0, 0, 1), [(0, 0), (0, 0.06), (0.15, 0.06), (0.15, 0.1), (0.25, 0.1), (0.25, 0)],
                8, rig.spec('stainless', 'flat'))
    wsd = D.pick(4, 3, 0)
    if wsd:
        for sgn in (1, -1):
            tube(rig, [(mx, 0, m_top - 0.1), (mx - 0.2, sgn * (H.hb(mx) - 0.08), H.sz(mx))], 0.006, wsd, st, caps=False)
        tube(rig, [(mx, 0, m_top - 0.1), (0.35, 0, H.sz(0.35) + 0.05)], 0.006, wsd, st, caps=False)
    # Trolling poles, raised and stayed to the mast.
    pole_t = 'alu' if v == 'a' else 'wood'
    ps = rig.spec(pole_t, 'flat', sharp=100)
    for sgn in (1, -1):
        base = (6.30, sgn * (H.hb(6.3) - 0.08), H.sz(6.3) + 0.08)
        tip = (6.95, sgn * 2.40, 12.10)
        n = D.pick(6, 3, 1)
        pts = [tuple(lerp(base[c], tip[c], i / n) for c in range(3)) for i in range(n + 1)]
        tube(rig, pts, 0.0, D.pick(8, 5, 3), ps, radii=[lerp(0.065, 0.025, i / n) for i in range(n + 1)])
        if wsd:
            tube(rig, [tip, (mx, 0, m_top - 0.25)], 0.005, wsd, rig.spec('rope', 'flat'), caps=False)
            tube(rig, [tip, (11.4, sgn * 1.45, H.sz(11.4) + 0.9)], 0.004, wsd, st, caps=False)
            if D.near:
                box(rig, (base[0], base[1], base[2] - 0.04), (0.18, 0.12, 0.12), rig.spec('rust_steel', 'flat'))
    # Boom aft from the mast.
    bs = rig.spec('rust_steel', 'flat', sharp=100)
    tube(rig, [(mx + 0.1, 0, 4.15), (9.6, 0, 4.75)], 0.06, D.pick(8, 5, 3), bs)
    if wsd:
        tube(rig, [(mx, 0, m_top - 0.2), (9.55, 0, 4.78)], 0.005, wsd, rig.spec('rope', 'flat'), caps=False)
        tube(rig, [(9.55, 0, 4.70), (9.55, 0, 3.2)], 0.006, wsd, rig.spec('rope', 'flat'), caps=False)
        if D.near:
            box(rig, (9.55, 0, 3.12), (0.12, 0.08, 0.18), bs)
    # Crab davit with block, starboard side aft of the wheelhouse.
    dv = gear.spec('rust_steel', 'flat', sharp=100)
    xd, bd = 7.10, -(H.hb(7.1) - 0.16)
    zdk = deck_z(xd, abs(bd))
    tube(gear, [(xd, bd, zdk), (xd, bd, zdk + 1.85)], 0.075, D.pick(8, 5, 3), dv)
    tube(gear, [(xd, bd, zdk + 1.82), (xd, bd - 0.62, zdk + 2.02)], 0.055, D.pick(8, 5, 3), dv)
    if not D.far:
        torus(gear, (xd, bd - 0.66, zdk + 1.72), (1, 0, 0), 0.20, 0.07, D.pick(14, 8), D.pick(6, 4), dv)
        box(gear, (xd - 0.14, bd - 0.66, zdk + 1.72), (0.12, 0.22, 0.22), gear.spec('paint_red', 'box'))
    # Crab pots: two stacks of three, two singles, buoys and coiled line.
    def pot(xc, bc, zb):
        r, hgt = 0.475, 0.36
        sides = D.pick(16, 8, 5)
        mesh = gear.spec('crab_mesh', 'side', key='mesh')
        rows = [[(xc + r * math.cos(2 * math.pi * k / sides), bc + r * math.sin(2 * math.pi * k / sides), z)
                 for k in range(sides)] for z in (zb + 0.02, zb + hgt - 0.02)]
        loft(gear, rows, mesh)
        if D.near:
            rim = gear.spec('rubber', 'flat', sharp=100)
            for z in (zb + 0.025, zb + hgt - 0.025):
                torus(gear, (xc, bc, z), (0, 0, 1), r, 0.025, D.pick(16, 10), D.pick(4, 3), rim)
    for xc, bc, n in ((8.55, 0.62, 3), (8.55, -0.62, 3), (9.65, 0.62, 1), (9.65, -0.62, 2)):
        zb = deck_z(xc, abs(bc)) + 0.01
        for k in range(n):
            pot(xc, bc, zb + 0.36 * k)
    if not D.far:
        bs_ = gear.spec('orange', 'flat', sharp=100)
        for xc, bc in ((9.65, 0.62), (10.45, -0.2)):
            zb = deck_z(xc, abs(bc)) + 0.37 + (0.0 if xc < 10 else -0.36)
            revolve(gear, (xc, bc, zb), (0, 0, 1), [(0, 0), (0.02, 0.16), (0.30, 0.16), (0.34, 0.08), (0.36, 0)], 10, bs_)
            tube(gear, [(xc, bc, zb + 0.3), (xc, bc, zb + 1.5)], 0.015, 4, gear.spec('rope', 'flat'))
        for xc, bc in ((10.4, 0.55), (10.9, -0.5)):
            torus(gear, (xc, bc, deck_z(xc, abs(bc)) + 0.06), (0, 0, 1), 0.28, 0.06, D.pick(16, 10), 5,
                  gear.spec('rope', 'flat'))
        hatch = gear.spec('deck_grey', 'box')
        box(gear, (7.6, 0, deck_z(7.6) + 0.1), (0.9, 1.0, 0.22), hatch)
        if D.near:
            for sgn in (1, -1):
                xg = 11.35
                revolve(gear, (xg, sgn * 1.05, deck_z(xg, 1.05) + 0.55), (0, 1, 0),
                        [(-0.15, 0), (-0.15, 0.16), (0.15, 0.16), (0.15, 0)], 10, gear.spec('stainless', 'flat'))
                tube(gear, [(xg, sgn * 1.05, deck_z(xg, 1.05)), (xg, sgn * 1.05, deck_z(xg, 1.05) + 0.4)], 0.05, 6, dv)
            box(gear, (0.10, 0, H.sz(0.1) + 0.02), (0.6, 0.18, 0.10), gear.spec('rust_steel', 'flat'))
            box(gear, (1.4, 0, deck_z(1.4) + 0.15), (0.45, 0.35, 0.3), gear.spec('rust_steel', 'flat'))
    # Tyres hung over both sides as fenders.
    if not D.far:
        ty = gear.spec('rubber', 'flat', sharp=100)
        ls = gear.spec('rope', 'flat')
        for x in (3.4, 5.9, 8.4):
            zc_ = 0.82
            w_ = max(H.halfwidth_at(x, z) for z in np.linspace(0.5, 1.15, 6))
            for sgn in (1, -1):
                torus(gear, (x, sgn * (w_ + 0.11), zc_), (0, 1, 0), 0.27, 0.10, D.pick(14, 8), D.pick(6, 4), ty)
                if D.near:
                    tube(gear, [(x, sgn * (w_ + 0.11), zc_ + 0.37), (x, sgn * (H.hb(x) - 0.05), H.sz(x) + 0.02)], 0.008, 4,
                         ls)
    return [hull, sup, rig, gear]


def fish_body(variant):
    H = fish_tables()
    scup = [(x, FISH_BULWARK(x / H.L) - 0.02) for x in (2.2, 3.6, 5.0, 6.4, 7.8, 9.2, 10.6)]
    chain = [(x, 0.05) for x in (5.55, 6.25, 7.1, 9.4, 11.6)]
    if variant == 'a':
        paint = dict(top='c4c2b8', mottle=0.06, boot=(0.04, 0.30, '1d3e6b'), af='5c1e17',
                     bands=[(0.04, 0.55, '1d3e6b')], streaks=7, grime_amt=0.4, lowgrime=0.35,
                     rust=dict(sources=scup + chain, amount=0.3, len=(0.12, 0.45), width=(0.02, 0.05)),
                     wear=dict(under='8a5a3a', line=0.62, thresh=0.66), streak_top=0.55)
    else:
        paint = dict(top='1c2a20', mottle=0.08, chalk=0.3, chalk_col='3e4a40', boot=(0.04, 0.08, 'b8b19c'),
                     af='5c1e17', bands=[(0.04, 0.50, 'bdb49a')], planks=0.14, streaks=6, grime='8a8573',
                     grime_amt=0.35, lowgrime=0.3, low_col='59584a',
                     rust=dict(sources=scup + chain, amount=0.5, len=(0.2, 0.7), width=(0.025, 0.07)),
                     wear=dict(under='6a4a30', line=0.58, thresh=0.5), streak_top=0.50)
    meta = dict(cls="40 ft troller-crabber, Fisherman's Wharf", sources=['fish-co26-003', 'fish-cr26-008', 'fish-hullform',
                                                                       'crab-pot'],
                published=dict(loaM=12.19, beamM=3.81, draftM=1.75, note='class figures 40x12x6.8 ft and 43x14.5x6.5 ft'))
    label = 'Fishing boat, 12.2 m troller-crabber, ' + ('white fiberglass, blue trim' if variant == 'a'
                                                         else 'green planked wood, cream trim')
    return Body('fishingBoat-' + variant, label, H, paint, 25000, fish_build, meta)


BODY_MAKERS = {'sailYacht': sail_body, 'motorYacht': motor_body, 'jetSki': jet_body, 'fishingBoat': fish_body}


# -------------------------------------------------------------- realising ---
def orient_islands(bm):
    """Make every connected island face outward by its signed volume.

    recalc_face_normals picks the face furthest from an island's centre as its
    reference; on a hull with bulwarks that can be an inner bulwark face, which
    turns the whole shell inside out. A closed island's signed volume is
    positive exactly when its faces point outward.
    """
    bm.faces.index_update()
    seen = set()
    for f0 in bm.faces:
        if f0.index in seen:
            continue
        seen.add(f0.index)
        stack, group = [f0], []
        while stack:
            f = stack.pop()
            group.append(f)
            for e in f.edges:
                for g in e.link_faces:
                    if g.index not in seen:
                        seen.add(g.index)
                        stack.append(g)
        vol = 0.0
        for f in group:
            vs = [l.vert.co for l in f.loops]
            for i in range(1, len(vs) - 1):
                vol += vs[0].dot(vs[i].cross(vs[i + 1]))
        if vol < 0:
            bmesh.ops.reverse_faces(bm, faces=group)


def realize(part, body, mat):
    """Part -> Blender mesh object: weld, consistent normals, UVs from specs, sharp edges by angle."""
    bm = bmesh.new()
    sl = bm.faces.layers.int.new('spec')
    vs = [bm.verts.new(body.blender(p)) for p in part.V]
    bm.verts.ensure_lookup_table()
    for idx, sp in zip(part.F, part.S):
        if len(set(idx)) < 3:
            continue
        try:
            f = bm.faces.new([vs[i] for i in idx])
        except ValueError:
            continue
        f[sl] = sp
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-4)
    bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=1e-5)
    loose = [v for v in bm.verts if not v.link_faces]
    bmesh.ops.delete(bm, geom=loose, context='VERTS')
    tiny = [f for f in bm.faces if f.calc_area() < 1e-9]
    bmesh.ops.delete(bm, geom=tiny, context='FACES')
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    orient_islands(bm)
    bbox = {}
    for f in bm.faces:
        s = f[sl]
        if part.specs[s]['mode'] in ('box', 'side', 'top'):
            lo, hi = bbox.get(s, ([1e9] * 3, [-1e9] * 3))
            for v in f.verts:
                for a in range(3):
                    lo[a], hi[a] = min(lo[a], v.co[a]), max(hi[a], v.co[a])
            bbox[s] = (lo, hi)
    uv = bm.loops.layers.uv.new('UVMap')
    (hx0, hx1), (hz0, hz1) = body.hx, body.hz
    for f in bm.faces:
        sp = part.specs[f[sl]]
        tile, mode = sp['tile'], sp['mode']
        if mode == 'flat':
            for l in f.loops:
                l[uv].uv = tile_uv(tile, 0.5, 0.5)
            continue
        if mode in ('hull', 'transom'):
            for l in f.loops:
                co = l.vert.co
                x = co.y + body.xm
                if mode == 'transom':
                    x = hx1 - body.hull.hbt - 0.02 + co.x
                l[uv].uv = tile_uv(tile, (x - hx0) / (hx1 - hx0), (co.z - hz0) / (hz1 - hz0))
            continue
        lo, hi = bbox[f[sl]]
        ext = [max(hi[a] - lo[a], 1e-6) for a in range(3)]
        n = f.normal
        if mode == 'top':
            axes_ = (1, 0)
        elif mode == 'side':
            axes_ = ((0, 2) if abs(n.y) > abs(n.x) else (1, 2))
        else:
            ax = max(range(3), key=lambda a: abs(n[a]))
            axes_ = {2: (1, 0), 0: (1, 2), 1: (0, 2)}[ax]
        for l in f.loops:
            co = l.vert.co
            l[uv].uv = tile_uv(tile, (co[axes_[0]] - lo[axes_[0]]) / ext[axes_[0]],
                               (co[axes_[1]] - lo[axes_[1]]) / ext[axes_[1]])
    for e in bm.edges:
        lf = e.link_faces
        if len(lf) != 2:
            e.smooth = False
            continue
        lim = max(part.specs[lf[0][sl]]['sharp'], part.specs[lf[1][sl]]['sharp'])
        e.smooth = math.degrees(e.calc_face_angle(0.0)) <= min(lim, 75.0)
    for f in bm.faces:
        f.smooth = True
    me = bpy.data.meshes.new(part.name)
    bm.to_mesh(me)
    bm.free()
    me.materials.append(mat)
    ob = bpy.data.objects.new(part.name, me)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def clear_scene():
    for ob in list(bpy.data.objects):
        bpy.data.objects.remove(ob, do_unlink=True)
    for me in list(bpy.data.meshes):
        bpy.data.meshes.remove(me)


def export_body(body, lod, mat, path):
    clear_scene()
    parts = body.build(lod)
    objs = [realize(p, body, mat) for p in parts if p.F]
    root = bpy.data.objects.new(body.key, None)
    bpy.context.scene.collection.objects.link(root)
    for ob in objs:
        ob.parent = root
    root['boatType'] = body.kind
    root['variant'] = body.variant
    root['lod'] = lod
    root['frame'] = 'glTF +Y up, metres, bow +Z, port +X, origin on the design waterline at mid waterline length'
    root['lwlM'] = round(body.lwl, 3)
    bpy.ops.object.select_all(action='DESELECT')
    for ob in objs + [root]:
        ob.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=str(path), export_format='GLB', use_selection=True, export_apply=True, export_yup=True,
        export_texcoords=True, export_normals=True, export_tangents=False, export_materials='EXPORT',
        export_image_format='AUTO', export_jpeg_quality=JPEG_QUALITY, export_image_quality=JPEG_QUALITY,
        export_vertex_color='NONE', export_animations=False, export_skins=False,
        export_morph=False, export_extras=True, export_cameras=False, export_lights=False)


# --------------------------------------------------------------- reading ---
def read_glb(path):
    """Triangle count, bounds and the hull's waterline extent from a GLB (no Blender)."""
    data = Path(path).read_bytes()
    jl = struct.unpack('<I', data[12:16])[0]
    js = json.loads(data[20:20 + jl])
    bin0 = 20 + jl + 8
    tris = 0
    lo, hi = np.full(3, 1e9), np.full(3, -1e9)
    nodes = {}
    wl = [1e9, -1e9]
    comp = {5126: ('f', 4), 5123: ('H', 2), 5125: ('I', 4), 5121: ('B', 1)}

    def accessor(i):
        a = js['accessors'][i]
        bv = js['bufferViews'][a['bufferView']]
        fmt, size = comp[a['componentType']]
        ncomp = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[a['type']]
        off = bin0 + bv.get('byteOffset', 0) + a.get('byteOffset', 0)
        arr = np.frombuffer(data, dtype=np.dtype(fmt).newbyteorder('<'), count=a['count'] * ncomp, offset=off)
        return arr.reshape(a['count'], ncomp) if ncomp > 1 else arr

    for node in js['nodes']:
        assert 'matrix' not in node and not node.get('rotation') and not node.get('scale'), 'unexpected node transform'
        if any(node.get('translation', [0, 0, 0])):
            raise ValueError('unexpected node translation')
        if 'mesh' not in node:
            continue
        for prim in js['meshes'][node['mesh']]['primitives']:
            pos = accessor(prim['attributes']['POSITION'])
            idx = accessor(prim['indices']).astype(np.int64)
            tris += len(idx) // 3
            lo, hi = np.minimum(lo, pos.min(0)), np.maximum(hi, pos.max(0))
            nlo, nhi = nodes.get(node.get('name'), (np.full(3, 1e9), np.full(3, -1e9)))
            nodes[node.get('name')] = (np.minimum(nlo, pos.min(0)), np.maximum(nhi, pos.max(0)))
            if node.get('name') == 'Hull':
                t = pos[idx.reshape(-1, 3)]
                y = t[:, :, 1]
                for a_, b_ in ((0, 1), (1, 2), (2, 0)):
                    ya, yb = y[:, a_], y[:, b_]
                    m = (ya <= 0) != (yb <= 0)
                    if m.any():
                        s = ya[m] / (ya[m] - yb[m])
                        z = t[m, a_, 2] + s * (t[m, b_, 2] - t[m, a_, 2])
                        wl = [min(wl[0], float(z.min())), max(wl[1], float(z.max()))]
    images = []
    for im in js.get('images', []):
        bv = js['bufferViews'][im['bufferView']]
        blob = data[bin0 + bv.get('byteOffset', 0): bin0 + bv.get('byteOffset', 0) + bv['byteLength']]
        images.append({'name': im.get('name'), 'mimeType': im.get('mimeType'), 'bytes': len(blob),
                       'sha256': hashlib.sha256(blob).hexdigest()})
    return dict(triangles=int(tris), min=lo.tolist(), max=hi.tolist(), waterlineZ=wl, images=images,
                nodes={k: (v[0].tolist(), v[1].tolist()) for k, v in nodes.items()},
                materials=[m.get('name') for m in js.get('materials', [])])


# ------------------------------------------------------------------- main ---
def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument('--output', default=str(DEFAULT_OUT))
    ap.add_argument('--bodies', default=','.join(f'{t}-{v}' for t in TYPES for v in 'ab'))
    ap.add_argument('--no-provenance', action='store_true')
    args = ap.parse_args(argv)
    out = Path(args.output).resolve()
    out.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bodies = {}
    for t in TYPES:
        for v in 'ab':
            bodies[f'{t}-{v}'] = BODY_MAKERS[t](v)
    work = tempfile.mkdtemp(prefix='boats-v1-')
    albedo, mrp = make_atlas(bodies, work)
    mat = make_material(albedo, mrp)
    atlas = {'albedo': {'px': ATLAS_PX, 'format': 'image/jpeg', 'quality': JPEG_QUALITY,
                        'sha256': hashlib.sha256(Path(albedo).read_bytes()).hexdigest(),
                        'bytes': Path(albedo).stat().st_size},
             'metallicRoughness': {'px': MR_PX, 'format': 'image/jpeg', 'channels': 'G roughness, B metallic, R unused (1)',
                                   'sha256': hashlib.sha256(Path(mrp).read_bytes()).hexdigest(),
                                   'bytes': Path(mrp).stat().st_size}}
    report = {}
    for key in args.bodies.split(','):
        body = bodies[key]
        rec = {'label': body.label, 'class': body.meta['cls'], 'lods': []}
        for lod in range(3):
            path = out / f'{key}-lod{lod}.glb'
            export_body(body, lod, mat, path)
            info = read_glb(path)
            data = path.read_bytes()
            lo, hi = info['min'], info['max']
            row = {'file': path.name, 'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data),
                   'triangles': info['triangles'],
                   'boundsM': {'min': [round(v, 4) for v in lo], 'max': [round(v, 4) for v in hi]}}
            if lod == 0:
                hlo, hhi = info['nodes']['Hull']
                rec['dimensionsM'] = {
                    'loaOverall': round(hi[2] - lo[2], 3),
                    'loaHullNode': round(hhi[2] - hlo[2], 3),
                    'lwlDesign': round(body.lwl, 3),
                    'lwlMeasured': round(info['waterlineZ'][1] - info['waterlineZ'][0], 3),
                    'waterlineMidZ': round((info['waterlineZ'][1] + info['waterlineZ'][0]) / 2, 4),
                    'beamHull': round(hhi[0] - hlo[0], 3),
                    'beamOverall': round(hi[0] - lo[0], 3),
                    'heightAboveWaterline': round(hi[1], 3),
                    'draft': round(-lo[1], 3)}
                rec['canoeBodyDisplacementKg'] = round(body.hull.volume() * SEAWATER)
                rec['images'] = info['images']
                rec['materials'] = info['materials']
                rec['budgetTriangles'] = body.budget
            rec['lods'].append(row)
            print('BOATS', key, lod, row['triangles'], row['bytes'], flush=True)
        rec['lodRatios'] = [round(r['triangles'] / rec['lods'][0]['triangles'], 3) for r in rec['lods']]
        rec['meta'] = body.meta
        report[key] = rec
        print('BOATS-DIMS', key, json.dumps(rec['dimensionsM']), rec['lodRatios'], flush=True)
    shutil.rmtree(work, ignore_errors=True)
    if args.no_provenance:
        return
    prov_path = out / 'boats-v1.provenance.json'
    prior = json.loads(prov_path.read_text()) if prov_path.exists() else {}
    bodies_rec = prior.get('bodies', {})
    bodies_rec.update(report)
    script = Path(__file__).resolve()
    prov = {
        'name': 'Moored boats v1',
        'generator': GENERATOR,
        'generatorSha256': hashlib.sha256(script.read_bytes()).hexdigest(),
        'blenderVersion': bpy.app.version_string,
        'licence': 'Original work of this repository, generated by the script; no third-party geometry or pixels',
        'thirdPartyContent': False,
        'sourcePixels': 'none: the atlas is painted from seeded noise by the generator',
        'frame': 'glTF +Y up, metres; bow +Z, port +X; origin on the design waterline (y = 0) at the middle of the '
                 'waterline length, on the centreline',
        'material': 'one material per body, BoatAtlas: baseColorTexture and metallicRoughnessTexture, the same atlas '
                    'bytes in every GLB; no vertex colours; UVs in TEXCOORD_0',
        'nodes': 'Hull (shell, deck, keel, rudder, running gear), Superstructure, Rig, Gear; all share one material',
        'atlas': atlas,
        'lods': 'lod0 full; lod1 rebuilt at about a quarter of the sampling; lod2 silhouette only (no wires, rails, '
                'fenders or small fittings)',
        'dimensionKeys': {
            'loaOverall': 'bow-to-stern extent of every node (glTF z), incl. pulpit, roller, platform, nozzle',
            'loaHullNode': 'extent of the Hull node (shell, keel, rudder, rub rails, platform)',
            'lwlDesign': 'waterline length of the hull lines at y = 0',
            'lwlMeasured': 'extent of the Hull node cut at y = 0 in the exported GLB (includes appendages there)',
            'waterlineMidZ': 'midpoint of that cut; about 0 means the origin is at mid waterline',
            'beamHull': 'athwartship extent of the Hull node, incl. rub rails',
            'beamOverall': 'athwartship extent of every node, incl. fenders, tyres and trolling poles',
            'heightAboveWaterline': 'highest point (masthead antenna, radar, pole tips, mirrors)',
            'draft': 'deepest point below the waterline (keel, rudders, running gear)'},
        'bodies': dict(sorted(bodies_rec.items())),
        'dimensionSources': DIMENSION_SOURCES,
        'referencePhotos': REFERENCE_PHOTOS,
        'referenceAerial': REFERENCE_AERIAL,
        'status': 'placed by native/eo-props/src/boats.rs in the floating docks\' berths (eo-app --no-boats '
                  'leaves them out); the visual gate verdict (REJECT) is unchanged',
    }
    prov_path.write_text(json.dumps(prov, indent=2) + '\n')
    print('BOATS-PROVENANCE', prov_path, flush=True)


if __name__ == '__main__':
    main()
