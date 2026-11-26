"""Bake the city's bedrock map (rock type per mapped geologic unit) from the USGS
geologic map databases, for the hill rock and the planting (Docs/HILLS-PLAN.md H7).

Inputs (U.S. Geological Survey, public domain), read from their archives as
downloaded, under external/geology/ (or EO_GEOLOGY_DIR), each folder with a
PROVENANCE.json:

  mf2337/mf2337c.tgz     Blake, Graymer and Jones, 2000, MF-2337 (1:62,500): the
                         San Francisco North quadrangle and Marin, mageo/ma-geol.e00
  of98-354/sfs_data.tar.gz  Bonilla, 1998, OF 98-354 (1:24,000): the San Francisco
                         South quadrangle and part of Hunters Point, sfs-geol.e00
  mf2342/mf2342c.tgz     Graymer, 2000, MF-2342 (1:50,000): the Oakland quadrangles;
                         within the city's box only Yerba Buena Island,
                         oakdb/ow-geol.e00

Each is an ARC/INFO export (.e00) coverage: arcs (ARC), their left and right
polygons, and the polygon attribute table (IFO ...PAT) whose PTYPE is the map
unit. OF 98-354's export is compressed (EXP 1, the export's own run-length and
numeric packing); it is unpacked here. Coordinates are Lambert conformal conic
on Clarke 1866 (NAD27): California State Plane zone III for OF 98-354 and
MF-2342 (false easting 2,000,000 US survey feet), the same cone with no false
easting for MF-2337 (each file's PRJ section and FGDC metadata). They are taken
to NAD27 longitude and latitude, shifted to WGS84 with the NIMA TR8350.2
three-parameter shift for NAD27 western United States (dX -8, dY 159, dZ 175 m,
about 5 m), and laid into the game's surveyed plan metres (x east, z south of
37.778 N, 122.4194 W; 110,574 m a degree of latitude, 87,986.247 m of
longitude: sf-lots-parks.json.gz meta.projection).

Each unit code maps to one class (UNITS below, with the unit's description from
the map's pamphlet and metadata): chert, sandstone (greywacke, shale, the
melange's sheared matrix), serpentinite, greenstone (basalt, volcanic rock),
surficial (Quaternary deposits, artificial fill, dune sand, landslides), other
bedrock, water. Polygons are clipped to the city's box (lon -122.52..-122.35,
lat 37.70..37.82) and their rings simplified (Douglas-Peucker, SIMPLIFY_M,
well inside the maps' own line accuracy at 1:24,000-1:62,500).

Output: native/assets/sf-lithology-v1.json.gz and its sf-lithology-v1.NOTICE.txt (a
derived table beside the NAIP-derived ones). It lists the sources in paint order: a reader rasterises them in that
order, so where two maps overlap the later (larger-scale) one wins.

Usage: python3 scripts/world-data/build-sf-lithology.py [--list]
  --list  print the unit codes inside the box with their areas and stop.
"""
import gzip
import hashlib
import io
import json
import math
import os
import re
from pathlib import Path
import sys
import tarfile

root = Path(__file__).resolve().parents[2]
out = root / "native/assets"
geology = Path(os.environ.get("EO_GEOLOGY_DIR") or (root / "external/geology"))

VERSION = "sf-lithology-v1"
TARGET = VERSION + ".json.gz"
NOTICE = VERSION + ".NOTICE.txt"
BOX = (-122.52, 37.70, -122.35, 37.82)  # lon west, lat south, lon east, lat north
CENTER_LAT, CENTER_LON = 37.778, -122.4194
M_PER_DEG_LAT, M_PER_DEG_LON = 110574.0, 87986.24747640654
SIMPLIFY_M = 2.0
# MF-2337 and OF 98-354 meet on the quadrangles' seam, 37 45' N (NAD27); each digitised
# its edge a metre or two off it, which leaves slivers between the maps. A vertex within
# this of the seam is set on it (degrees: about 3 m).
SEAM_LAT = 37.75
SEAM_SNAP_DEG = 3.0e-5
# NIMA TR8350.2 (3rd ed., 2000), Table B-1: NAD27 to WGS84, western United States.
NAD27_TO_WGS84 = (-8.0, 159.0, 175.0)

# (archive under external/geology, member, short id, citation, scale, url)
SOURCES = [
    {
        "id": "mf2337",
        "archive": "mf2337/mf2337c.tgz",
        "member": "mageo/ma-geol.e00",
        "false_easting_m": 0.0,
        "seam": True,
        "citation": "Blake, M.C., Jr., Graymer, R.W., and Jones, D.L., 2000, Geologic map and map database of parts of Marin, San Francisco, Alameda, Contra Costa, and Sonoma Counties, California: U.S. Geological Survey Miscellaneous Field Studies Map MF-2337, scale 1:62,500 (digital database by A. Soule and R.W. Graymer)",
        "scale": "1:62,500",
        "url": "https://pubs.usgs.gov/mf/2000/2337/",
    },
    {
        "id": "mf2342",
        "archive": "mf2342/mf2342c.tgz",
        "member": "oakdb/ow-geol.e00",
        "false_easting_m": 2000000.0 * 1200.0 / 3937.0,
        "citation": "Graymer, R.W., 2000, Geologic map and map database of the Oakland metropolitan area, Alameda, Contra Costa, and San Francisco Counties, California: U.S. Geological Survey Miscellaneous Field Studies Map MF-2342, scale 1:50,000",
        "scale": "1:50,000",
        "url": "https://pubs.usgs.gov/mf/2000/2342/",
    },
    {
        "id": "of98-354",
        "archive": "of98-354/sfs_data.tar.gz",
        "member": "sfs-geol.e00",
        "false_easting_m": 2000000.0 * 1200.0 / 3937.0,
        "seam": True,
        "citation": "Bonilla, M.G., 1998, Preliminary geologic map of the San Francisco South 7.5' quadrangle and part of the Hunters Point 7.5' quadrangle, San Francisco Bay area, California: a digital database: U.S. Geological Survey Open-File Report 98-354, scale 1:24,000",
        "scale": "1:24,000",
        "url": "https://pubs.usgs.gov/of/1998/of98-354/",
    },
]

# ---------------------------------------------------------------- the unit table
# (source, code) -> (class, description). The descriptions are the maps' own, shortened
# (MF-2337: "Description of Map Units" in its pamphlet mamf.txt / mf2337j.pdf; OF 98-354:
# "Geologic unit descriptions" on its map sheet sfsouth.pdf; MF-2342: its pamphlet
# oakmf.txt / mf2342j.pdf). The class is this script's reading of the description:
# chert; sandstone = Franciscan sandstone, greywacke and shale, and the melange's sheared
# shale-and-sandstone matrix; serpentinite; greenstone = altered basalt (pillow lava);
# surficial = Quaternary deposits (fill, dune and beach sand, slope debris, landslides,
# alluvium, the Colma Formation's friable sand); other = other bedrock (the Merced
# Formation's soft Plio-Pleistocene sediments, fault rocks, metamorphic rocks); water.
CLASSES = ["chert", "sandstone", "serpentinite", "greenstone", "surficial", "other", "water"]
UNITS = {
    # MF-2337 (San Francisco North quadrangle)
    ("mf2337", "water"): ("water", "Water"),
    ("mf2337", "Qaf"): ("surficial", "Artificial fill (Quaternary): clay, silt, sand, rock fragments, organic material and man-made debris"),
    ("mf2337", "Qs"): ("surficial", "Beach sand (Quaternary): well to moderately sorted; loose to soft"),
    ("mf2337", "Qd"): ("surficial", "Dune sand (Quaternary): well sorted; loose to soft"),
    ("mf2337", "Qls"): ("surficial", "Landslide deposits (Quaternary): largely bedrock debris"),
    ("mf2337", "Qal"): ("surficial", "Alluvium (Quaternary): sand, gravel, silt and clay; loose to soft and friable"),
    ("mf2337", "Qsr"): ("surficial", "Slope debris and ravine fill (Quaternary): angular rock fragments in sand, silt and clay matrix; light yellow to reddish-brown"),
    ("mf2337", "Qu"): ("surficial", "Undifferentiated surficial deposits (Quaternary): beach sand, marine deposits, artificial fill, alluvium, landslides"),
    ("mf2337", "Qob"): ("surficial", "Older beach deposits (Quaternary): well-sorted medium- to coarse-grained gray sand"),
    ("mf2337", "Qc"): ("surficial", "Colma Formation (Quaternary): fine- to medium-grained sand, well sorted, soft and friable"),
    ("mf2337", "Kfs"): ("sandstone", "Franciscan Complex sandstone and shale (Cretaceous): sandstone light gray where fresh, weathering to buff; shale dark gray; typically forms resistant topography"),
    ("mf2337", "Kfss"): ("sandstone", "Franciscan Complex massive sandstone (Cretaceous): thick-bedded and massive graywacke sandstone with thin fissile shale"),
    ("mf2337", "Kfsh"): ("sandstone", "Franciscan Complex thin-bedded sandstone and shale (Cretaceous): interbedded and laminated shale and fine-grained sandstone, beds 5-13 cm"),
    ("mf2337", "Kfgwy"): ("sandstone", "Franciscan Complex graywacke (Cretaceous): graywacke, shale and some metagraywacke; overlies chert and greenstone in parts of San Francisco"),
    ("mf2337", "fsr"): ("sandstone", "Franciscan Complex melange: variably sheared shale and sandstone containing hard tectonic inclusions of greenstone, chert, graywacke and serpentinite"),
    ("mf2337", "KJfch"): ("chert", "Franciscan Complex chert (Cretaceous and Jurassic): thin-bedded, closely fractured radiolarian chert with shale interbeds; up to 100 m thick over pillow lava in parts of San Francisco"),
    ("mf2337", "Jfg"): ("greenstone", "Franciscan Complex greenstone (Jurassic): pillow lava, tuff, breccia and intrusive basalt, diabase"),
    ("mf2337", "Jfgs"): ("greenstone", "Franciscan Complex greenstone (Jurassic): well-bedded pillow lavas and minor diabase; larger masses closely fractured, softened by weathering, with distinctive red soil"),
    ("mf2337", "Jspm"): ("serpentinite", "Massive serpentinite (Jurassic): hard, massive serpentine with some partly serpentinized peridotite; mapped only in San Francisco"),
    ("mf2337", "sp"): ("serpentinite", "Serpentinite (Jurassic): including relatively fresh ultramafic rock; lenses and irregular masses within and along melange; prominent shear fabric"),
    # MF-2342 (Oakland West quadrangle: Yerba Buena and Treasure Islands)
    ("mf2342", "H2O"): ("water", "Water"),
    ("mf2342", "af"): ("surficial", "Artificial fill (Historic): man-made deposit of various materials and ages"),
    ("mf2342", "Kfa"): ("sandstone", "Sandstone of the Alcatraz terrane (Cretaceous): coarse-grained lithic wacke, massive with thin shale partings; dark greenish-gray fresh, weathers yellowish-brown"),
    # OF 98-354 (San Francisco South quadrangle, part of Hunters Point)
    ("of98-354", "water"): ("water", "Water"),
    ("of98-354", "Qaf"): ("surficial", "Artificial fill: clay, silt, sand, rock fragments, organic matter and man-made debris"),
    ("of98-354", "Qaf/tf"): ("surficial", "Artificial fill over tidal flat"),
    ("of98-354", "Qafs"): ("surficial", "Artificial fill, Native American shellmound: dark silty to sandy soil with shell fragments"),
    ("of98-354", "Ql"): ("surficial", "Landslide deposits: composition depends on the formation involved"),
    ("of98-354", "Ql(?)"): ("surficial", "Landslide deposits (queried)"),
    ("of98-354", "Qal"): ("surficial", "Alluvium: mostly sand and silt, locally clay, gravel or boulders; gray to brown"),
    ("of98-354", "Qb"): ("surficial", "Beach deposits: well sorted medium-grained loose gray sand; locally gravel and cobbles"),
    ("of98-354", "Qm"): ("surficial", "Bay mud: soft to firm clay and silt"),
    ("of98-354", "Qd"): ("surficial", "Dune sand: well sorted fine-grained sand, gray and loose in most places"),
    ("of98-354", "Qt"): ("surficial", "Marine terrace deposits: friable well sorted fine-grained yellowish-orange to gray sand"),
    ("of98-354", "Qsr"): ("surficial", "Slope debris and ravine fill: stony silty to sandy clay; yellowish-orange to medium gray"),
    ("of98-354", "Qc"): ("surficial", "Colma Formation: friable well sorted fine to medium sand; sandy clay and silty sand in the southeast; yellowish orange to gray"),
    ("of98-354", "Qu"): ("surficial", "Sedimentary deposits, undifferentiated"),
    ("of98-354", "fr"): ("other", "Fault rocks: gouge, breccia, fractured and sheared rock along the San Andreas fault"),
    ("of98-354", "QTm"): ("other", "Merced Formation (Pleistocene and Pliocene): friable to firm sand, silt and clay; medium gray to yellowish orange"),
    ("of98-354", "KJs"): ("sandstone", "Franciscan Complex sandstone and shale: hard where fresh; medium dark gray fresh, olive gray to yellowish brown moderately weathered"),
    ("of98-354", "KJs?"): ("sandstone", "Franciscan Complex sandstone and shale (queried)"),
    ("of98-354", "KJsk"): ("sandstone", "Franciscan Complex sandstone and shale, sandstone generally with more than two percent potassium feldspar"),
    ("of98-354", "KJc"): ("chert", "Franciscan Complex chert: hard chert interbedded with firm shale, chert layers 2-3 inches; generally grayish red"),
    ("of98-354", "KJg"): ("greenstone", "Franciscan Complex greenstone: altered volcanic rocks, mostly basalt; grayish olive to moderate olive gray moderately weathered"),
    ("of98-354", "KJm"): ("other", "Metamorphic rocks: schistose, gneissose or granulose; dark gray, dark greenish gray or dark bluish gray"),
    ("of98-354", "sp"): ("serpentinite", "Serpentine: hard to soft, generally greenish gray; small bodies of gabbro and diabase"),
    ("of98-354", "KJu"): ("sandstone", "Sheared rocks: hard rock fragments in a matrix of sheared rock derived mostly from Franciscan shale and sandstone and serpentine; dark gray fresh, yellowish brown weathered"),
}


# ---------------------------------------------------------------- .e00 reading
def e00_lines(raw):
    """The export's lines; a compressed export (EXP 1) unpacked."""
    text = raw.decode("latin-1").replace("\r", "")
    first, _, rest = text.partition("\n")
    if not first.startswith("EXP  1"):
        return text.split("\n")
    head, body = first[:80], first[80:] + rest.replace("\n", "")
    lines, buf, i, n, numeric = [head], [], 0, len(body), False
    while i < n:
        c = body[i]
        i += 1
        if c != "~":
            buf.append(c)
            numeric = False
            continue
        c = body[i]
        i += 1
        if c == " ":  # "~ " and a count: that many spaces
            buf.append(" " * (ord(body[i]) - 32))
            i += 1
            numeric = False
        elif c == "}":  # end of line
            lines.append("".join(buf))
            buf = []
            numeric = False
        elif numeric:  # a "~" that only ended the number before it
            buf.append(c)
            numeric = False
        elif c in "~-":
            buf.append(c)
        else:  # a packed number: two digits a character
            m = ord(c) - 33
            point, odd, exp = m % 15, m // 45, (m // 15) % 3
            digits, count = [], 0
            while i < n and body[i] not in " ~":
                v = ord(body[i]) - 33
                i += 1
                if v == 92:
                    v = 92 + ord(body[i]) - 33
                    i += 1
                for d in (chr(48 + v // 10), chr(48 + v % 10)):
                    digits.append(d)
                    count += 1
                    if count == point:
                        digits.append(".")
            numeric = i < n
            if odd:
                digits.pop()
            if exp:
                digits[-2:-2] = list("E+" if exp == 1 else "E-")
            buf.append("".join(digits))
    if buf:
        lines.append("".join(buf))
    return lines


def ints(line, k=10):
    return [int(line[j : j + k]) for j in range(0, len(line.rstrip()), k)]


def floats(line, k):
    return [float(line[j : j + k]) for j in range(0, len(line.rstrip()), k)]


def read_coverage(lines):
    """(arcs {id: (fnode, tnode, lpoly, rpoly, points)}, polygon units [code per polygon], prj lines)."""
    at = {l[:3]: k for k, l in enumerate(lines) if l[:3] in ("ARC", "PAL", "PRJ", "IFO") and l[3:5] == "  "}
    k = at["ARC"]
    double = lines[k][5] == "3"
    k += 1
    arcs = {}
    while True:
        h = ints(lines[k])
        k += 1
        if h[0] == -1:
            break
        cov, _id, fnode, tnode, lpoly, rpoly, npts = h
        pts = []
        while len(pts) < npts:
            v = floats(lines[k], 21 if double else 14)
            k += 1
            pts.extend(zip(v[0::2], v[1::2]))
        arcs[cov] = (fnode, tnode, lpoly, rpoly, pts[:npts])
    # the polygon attribute table (the IFO section's "<cover>.PAT"): one record per
    # polygon, in polygon order
    k = next(j for j in range(at["IFO"], len(lines)) if re.match(r"\S+\.PAT +XX", lines[j]))
    nitems, nrec = int(lines[k][34:38]), int(lines[k][46:56])
    items = []
    for j in range(nitems):
        d = lines[k + 1 + j]
        size, typ = int(d[16:19]), int(d[34:37])
        width = {10: 8, 20: size, 30: size, 40: 14, 50: 6 if size == 2 else 11, 60: 14 if size == 4 else 24}[typ]
        items.append((d[:16].strip(), width))
    k += 1 + nitems
    # a record is its fields' export widths end to end, broken into 80-character lines
    nlines = max(1, -(-sum(w for _, w in items) // 80))
    units = []
    for _ in range(nrec):
        s = "".join(lines[k + j].ljust(80) for j in range(nlines))
        k += nlines
        p = 0
        for nm, w in items:
            if nm == "PTYPE":
                units.append(s[p : p + w].strip())
            p += w
    units = units if any(nm == "PTYPE" for nm, _ in items) else None
    assert units is not None, "no polygon attribute table with PTYPE"
    k = at["PRJ"] + 1
    prj = []
    while not lines[k].startswith("EOP"):
        if lines[k] != "~":
            prj.append(lines[k])
        k += 1
    return arcs, units, prj


def rings_of(arcs, poly):
    """The closed rings of polygon `poly` (its arcs, the polygon on their right walked
    forward, on their left backward, chained by node)."""
    out_from = {}
    for a, (fn, tn, lp, rp, pts) in sorted(arcs.items()):
        if lp == rp:
            continue
        if rp == poly:
            out_from.setdefault(fn, []).append((tn, pts))
        elif lp == poly:
            out_from.setdefault(tn, []).append((fn, pts[::-1]))
    rings = []
    while out_from:
        start = min(out_from)
        node, ring = start, []
        while True:
            nxt = out_from[node]
            to, pts = nxt.pop(0)
            if not nxt:
                del out_from[node]
            ring.extend(pts if not ring else pts[1:])
            node = to
            if node == start:
                break
            assert node in out_from, f"polygon {poly}: ring does not close at node {node}"
        rings.append(ring)
    return rings


# ---------------------------------------------------------------- projection
CLARKE_A, CLARKE_B = 6378206.4, 6356583.8
WGS_A, WGS_F = 6378137.0, 1.0 / 298.257223563


def lcc_inverse(false_easting):
    """Inverse Lambert conformal conic, California zone III's cone on Clarke 1866
    (standard parallels 37 04' and 38 26', origin 36 30' N, 120 30' W; Snyder 1987, 15)."""
    a = CLARKE_A
    e2 = 1.0 - (CLARKE_B / CLARKE_A) ** 2
    e = math.sqrt(e2)
    p1, p2, p0, l0 = map(math.radians, (37 + 4 / 60, 38 + 26 / 60, 36.5, -120.5))
    m = lambda p: math.cos(p) / math.sqrt(1 - e2 * math.sin(p) ** 2)
    t = lambda p: math.tan(math.pi / 4 - p / 2) / ((1 - e * math.sin(p)) / (1 + e * math.sin(p))) ** (e / 2)
    n = (math.log(m(p1)) - math.log(m(p2))) / (math.log(t(p1)) - math.log(t(p2)))
    F = m(p1) / (n * t(p1) ** n)
    r0 = a * F * t(p0) ** n

    def inv(x, y):
        x -= false_easting
        r = math.copysign(math.hypot(x, r0 - y), n)
        th = math.atan2(x, r0 - y)
        tt = (r / (a * F)) ** (1 / n)
        p = math.pi / 2 - 2 * math.atan(tt)
        for _ in range(8):
            p = math.pi / 2 - 2 * math.atan(tt * ((1 - e * math.sin(p)) / (1 + e * math.sin(p))) ** (e / 2))
        return math.degrees(th / n + l0), math.degrees(p)

    return inv


def nad27_to_wgs84(lon, lat):
    """Geodetic NAD27 (Clarke 1866) to WGS84 by the three-parameter geocentric shift."""
    e2c = 1.0 - (CLARKE_B / CLARKE_A) ** 2
    p, l = math.radians(lat), math.radians(lon)
    nn = CLARKE_A / math.sqrt(1 - e2c * math.sin(p) ** 2)
    X = nn * math.cos(p) * math.cos(l) + NAD27_TO_WGS84[0]
    Y = nn * math.cos(p) * math.sin(l) + NAD27_TO_WGS84[1]
    Z = nn * (1 - e2c) * math.sin(p) + NAD27_TO_WGS84[2]
    e2 = WGS_F * (2 - WGS_F)
    lon2 = math.atan2(Y, X)
    q = math.hypot(X, Y)
    lat2 = math.atan2(Z, q * (1 - e2))
    for _ in range(8):
        nw = WGS_A / math.sqrt(1 - e2 * math.sin(lat2) ** 2)
        h = q / math.cos(lat2) - nw
        lat2 = math.atan2(Z, q * (1 - e2 * nw / (nw + h)))
    return math.degrees(lon2), math.degrees(lat2)


def plan(lon, lat):
    return ((lon - CENTER_LON) * M_PER_DEG_LON, (CENTER_LAT - lat) * M_PER_DEG_LAT)


# ---------------------------------------------------------------- geometry
def ring_area(r):
    return 0.5 * sum(r[i - 1][0] * r[i][1] - r[i][0] * r[i - 1][1] for i in range(len(r)))


def clip_ring(r, x0, z0, x1, z1):
    """Sutherland-Hodgman against the box (a ring that leaves and re-enters keeps the
    box edge between, which fills the same)."""
    for axis, lim, keep_lo in ((0, x0, False), (0, x1, True), (1, z0, False), (1, z1, True)):
        if not r:
            return r
        inside = (lambda p: p[axis] <= lim) if keep_lo else (lambda p: p[axis] >= lim)
        res = []
        for i in range(len(r)):
            a, b = r[i - 1], r[i]
            ia, ib = inside(a), inside(b)
            if ia != ib:
                t = (lim - a[axis]) / (b[axis] - a[axis])
                res.append((a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])))
            if ib:
                res.append(b)
        r = res
    return r


def simplify(pts, tol):
    """Douglas-Peucker on a closed ring (its first point kept, the ring's farthest point
    from it the second anchor)."""
    if len(pts) < 8:
        return pts
    far = max(range(len(pts)), key=lambda i: (pts[i][0] - pts[0][0]) ** 2 + (pts[i][1] - pts[0][1]) ** 2)

    def dp(seq):
        keep = [False] * len(seq)
        keep[0] = keep[-1] = True
        stack = [(0, len(seq) - 1)]
        while stack:
            i, j = stack.pop()
            (ax, az), (bx, bz) = seq[i], seq[j]
            dx, dz = bx - ax, bz - az
            ll = dx * dx + dz * dz
            best, bi = -1.0, -1
            for k in range(i + 1, j):
                px, pz = seq[k]
                if ll == 0:
                    d = (px - ax) ** 2 + (pz - az) ** 2
                else:
                    d = (dx * (az - pz) - dz * (ax - px)) ** 2 / ll
                if d > best:
                    best, bi = d, k
            if bi > 0 and best > tol * tol:
                keep[bi] = True
                stack += [(i, bi), (bi, j)]
        return [p for p, kk in zip(seq, keep) if kk]

    a = dp(pts[: far + 1])
    b = dp(pts[far:] + pts[:1])
    return a[:-1] + b[:-1]


def load(src):
    path = geology / src["archive"]
    with tarfile.open(path) as tf:
        raw = tf.extractfile(src["member"]).read()
    lines = e00_lines(raw)
    arcs, units, prj = read_coverage(lines)
    lcc = lcc_inverse(src["false_easting_m"])

    def inv(x, y):
        lon, lat = lcc(x, y)
        if src.get("seam") and abs(lat - SEAM_LAT) < SEAM_SNAP_DEG:
            lat = SEAM_LAT
        return lon, lat

    lon0, lat0, lon1, lat1 = BOX
    x0, z1 = plan(lon0, lat0)
    x1, z0 = plan(lon1, lat1)
    # an unlabelled polygon (a digitising sliver, no PTYPE) takes the unit of the labelled
    # neighbour it shares the most boundary with
    shared = {}
    for fn, tn, lp, rp, pts in arcs.values():
        length = sum(math.dist(pts[i], pts[i + 1]) for i in range(len(pts) - 1))
        for a, b in ((lp, rp), (rp, lp)):
            if a >= 2 and b >= 2 and a != b and not units[a - 1] and units[b - 1]:
                shared.setdefault(a, {}).setdefault(b, 0.0)
                shared[a][b] += length
    joined = 0
    polys = []
    for p in range(2, len(units) + 1):  # polygon 1 is the universe
        code = units[p - 1]
        if not code and p in shared:
            code = units[max(sorted(shared[p]), key=lambda q: shared[p][q]) - 1]
            joined += 1
        rings = []
        for r in rings_of(arcs, p):
            pr = [plan(*nad27_to_wgs84(*inv(x, y))) for x, y in r]
            pr = clip_ring(pr, x0, z0, x1, z1)
            if len(pr) >= 3 and abs(ring_area(pr)) > 1.0:
                rings.append(pr)
        if rings:
            polys.append((code, rings))
    if joined:
        print(f"{src['id']}: {joined} unlabelled polygons joined to their longest neighbour", file=sys.stderr)
    return raw, prj, polys


def provenance(src):
    """The archive's PROVENANCE.json record (url, retrieval, bytes, sha256, licence)."""
    folder = (geology / src["archive"]).parent
    rec = json.loads((folder / "PROVENANCE.json").read_text())
    name = Path(src["archive"]).name
    f = next(f for f in rec["files"] if f["path"] == name)
    data = (folder / name).read_bytes()
    assert hashlib.sha256(data).hexdigest() == f["sha256"] and len(data) == f["bytes"], f"{name}: digest differs from {folder}/PROVENANCE.json"
    return rec, f


def main():
    listing = "--list" in sys.argv
    lon0, lat0, lon1, lat1 = BOX
    sources, polygons, units, unlabelled = [], [], {}, 0
    for si, src in enumerate(SOURCES):
        raw, prj, polys = load(src)
        if listing:
            area = {}
            for code, rings in polys:
                area[code] = area.get(code, 0.0) + abs(sum(ring_area(r) for r in rings))
            print(src["id"], " ".join(prj))
            for code, a in sorted(area.items(), key=lambda kv: -kv[1]):
                print(f"  {code:8s} {a / 1e6:8.3f} km2")
            continue
        rec, f = provenance(src)
        sources.append({
            "id": src["id"],
            "citation": src["citation"],
            "scale": src["scale"],
            "url": src["url"],
            "archive": f["url"],
            "archiveSha256": f["sha256"],
            "member": src["member"],
            "memberSha256": hashlib.sha256(raw).hexdigest(),
            "retrieved": f["retrieved"],
            "licence": rec["licence"],
            "projection": " ".join(" ".join(l.split()) for l in prj),
        })
        for code, rings in polys:
            if not code:
                unlabelled += 1
                continue
            cls, desc = UNITS[(src["id"], code)]
            rr = []
            for r in rings:
                r = simplify(r, SIMPLIFY_M)
                if len(r) >= 3 and abs(ring_area(r)) > 1.0:
                    rr.append([round(v, 1) for p in r for v in p])
            if not rr:
                continue
            u = units.setdefault((src["id"], code), {"source": src["id"], "code": code, "class": cls, "description": desc, "areaKm2": 0.0, "polygons": 0})
            u["areaKm2"] += abs(sum(ring_area(r) for r in rings)) / 1e6
            u["polygons"] += 1
            polygons.append({"s": si, "u": code, "c": CLASSES.index(cls), "rings": rr})
    if listing:
        return
    for u in units.values():
        u["areaKm2"] = round(u["areaKm2"], 4)
    by_class = {}
    for u in units.values():
        by_class[u["class"]] = round(by_class.get(u["class"], 0.0) + u["areaKm2"], 4)
    doc = {
        "meta": {
            "version": VERSION,
            "producedBy": "scripts/world-data/build-sf-lithology.py",
            "projection": {"center": {"lat": CENTER_LAT, "lon": CENTER_LON}, "metersPerDegLat": M_PER_DEG_LAT, "metersPerDegLon": M_PER_DEG_LON, "axes": "x east, z south, metres (surveyed plan; the game scales by eo_data::groundscale)"},
            "bbox": {"minLon": lon0, "minLat": lat0, "maxLon": lon1, "maxLat": lat1},
            "datum": "source NAD27 (Clarke 1866) to WGS84 by the NIMA TR8350.2 three-parameter shift for NAD27 western United States, dX -8 dY 159 dZ 175 m",
            "simplifyM": SIMPLIFY_M,
            "classes": CLASSES,
            "paintOrder": "sources in order; a later source's polygon wins where two overlap",
            "sources": sources,
            "units": sorted(units.values(), key=lambda u: (u["source"], u["class"], u["code"])),
            "areaKm2ByClass": by_class,
            "unlabelledPolygonsDropped": unlabelled,
        },
        "polygons": polygons,
    }
    bio = io.BytesIO()
    with gzip.GzipFile(fileobj=bio, mode="wb", mtime=0, compresslevel=9) as stream:
        stream.write(json.dumps(doc, separators=(",", ":")).encode())
    target = out / TARGET
    target.write_bytes(bio.getvalue())
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    lines = [
        TARGET + " - San Francisco bedrock: mapped geologic units as rock-type classes",
        "",
        "Sources (U.S. Geological Survey; public domain, U.S. Government work):",
    ]
    for s in sources:
        lines += [
            f"  {s['citation']}.",
            f"    {s['url']}",
            f"    Archive: {s['archive']} (sha256 {s['archiveSha256']}, retrieved {s['retrieved']})",
            f"    Read: {s['member']} (sha256 {s['memberSha256']})",
            f"    Licence: {s['licence']}",
        ]
    lines += [
        "",
        "Credit: U.S. Geological Survey. USGS does not endorse this product; the maps' use",
        "constraints apply: do not use the data at a scale larger than its source maps",
        "(1:24,000 to 1:62,500); boundaries are good to tens of metres, not to a site.",
        "",
        "Method: each map unit (PTYPE) is read to one class from the maps' own unit",
        "descriptions (meta.units lists every code with its description and class);",
        "polygons are clipped to lon -122.52..-122.35, lat 37.70..37.82, taken from",
        "Lambert conformal conic (NAD27) to WGS84 (NIMA TR8350.2 NAD27 western US shift)",
        "and to the game's surveyed plan metres, and simplified by " + f"{SIMPLIFY_M:g} m.",
        "An unlabelled sliver polygon takes the unit of the neighbour it shares the most",
        "boundary with.",
        "Where MF-2337 and OF 98-354 meet (37 45' N, NAD27) vertices within 3 m of the",
        "seam are set on it, so the two maps close without a sliver.",
        "MF-2342 covers the city only at Yerba Buena and Treasure Islands (its west",
        "edge is 122 22' 30\" W); the San Francisco peninsula comes from MF-2337 (north of",
        "37 45' N) and OF 98-354 (south of it).",
        "",
        "Area by class (km2, inside the box): " + ", ".join(f"{k} {v:g}" for k, v in sorted(by_class.items())),
        "",
        "Produced by:   scripts/world-data/build-sf-lithology.py (" + VERSION + ")",
        f"Output sha256: {digest} ({TARGET})",
    ]
    (out / NOTICE).write_text("\n".join(lines) + "\n")
    print(target, target.stat().st_size, "bytes,", len(polygons), "polygons;", by_class)


if __name__ == "__main__":
    main()
