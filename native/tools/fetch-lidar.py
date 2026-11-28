#!/usr/bin/env python3
"""Fetch USGS 3DEP lidar points for the city's hill parks from the public Entwine (EPT) copy.

Source: USGS 3D Elevation Program, project CA_SanFrancisco_B23 (the 2023
survey the game's 1 m ground model comes from), work unit
CA_SanFrancisco_1_B23, as published by USGS on the AWS Open Data bucket
`usgs-lidar-public` (Entwine Point Tiles: an octree of LAZ files, EPSG:3857
plan, NAVD88 metres as delivered). Public domain (U.S. Government work).
The USGS download pages answer automated clients with an anti-bot page; this
reads only the public bucket.

An area is one of `AREAS` below: the city's own mapped green polygons
(`public/data/sf/sf-lots-parks.json.gz`, by OSM id), optionally clipped to a
lon/lat box. The script reads the octree's hierarchy and downloads every node
whose plan box meets an area polygon grown by `--margin` metres, down to
`--max-depth` (the cloud's points are spread over the depths, each deeper one
denser: through depth 8 about 14 points a square metre, through 9 about 75;
the whole city at full density is about 75 GB). Each file is checked as it
lands and listed with its URL, retrieval time, bytes and SHA-256.

    python3 -I native/tools/fetch-lidar.py --area mclaren --area bernal   # or --all
    python3 -I native/tools/fetch-lidar.py --all --dry-run                # node counts and bytes only

`--root` defaults to `EO_LIDAR_DIR`, else `<repo>/external/lidar` (git-ignored).
Files land in <root>/<area>/ept-data/; <root>/<area>/PROVENANCE.json lists
them. The hill builder's tree cook (`cargo run --release -p eo-hills --example
cook -- trees`) reads those folders.
"""
import argparse, concurrent.futures, datetime, gzip, hashlib, json, math, os, sys, time, urllib.request

BASE = "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/CA_SanFrancisco_1_B23"
R = 6378137.0
AGENT = "earth-online-lidar-fetch/2"
REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
GREENS = os.path.join(REPO, "public", "data", "sf", "sf-lots-parks.json.gz")

# The hill parks (Docs/HILLS-PLAN.md H6c): the mapped greens (OSM way `w` or
# relation `r` ids in sf-lots-parks.json.gz) and an optional clip box
# (lon0, lat0, lon1, lat1).
AREAS = {
    "mclaren": {"greens": ["w28716696"], "label": "John McLaren Park"},
    "bernal": {"greens": ["w16752188"], "label": "Bernal Heights Park"},
    "twin-peaks": {"greens": ["w894478070"], "label": "Twin Peaks"},
    "mount-davidson": {"greens": ["r16800880", "r10373519"], "label": "Mount Davidson Park and its wood"},
    "glen-canyon": {"greens": ["w35800082"], "label": "Glen Canyon Park"},
    "corona-heights": {"greens": ["w225182880"], "label": "Corona Heights Park"},
    "bayview-hill": {"greens": ["w82480966"], "label": "Bayview Hill Park"},
    "lands-end": {"greens": ["r2142026"], "label": "Lands End and Lincoln Park"},
    # Golden Gate Park west of 122.49 W (the Chain of Lakes, the windmills, the bison paddock and the west woods).
    "ggp-west": {"greens": ["w158602261"], "clip": [-122.5200, 37.7600, -122.4900, 37.7800], "label": "Golden Gate Park, west end"},
}


def get(url, tries=4):
    for k in range(tries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": AGENT})
            with urllib.request.urlopen(req, timeout=120) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            # a refusal is reported, never worked around
            if e.code in (403, 429) or k == tries - 1:
                raise
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            if k == tries - 1:
                raise
        time.sleep(2.0 * (k + 1))


def merc(lat, lon):
    return R * math.radians(lon), R * math.log(math.tan(math.pi / 4 + math.radians(lat) / 2))


def lonlat(mx, my):
    return math.degrees(mx / R), math.degrees(2 * math.atan(math.exp(my / R)) - math.pi / 2)


def load_greens():
    d = json.load(gzip.open(GREENS))
    p = d["meta"]["projection"]
    c, ml, mo = p["center"], p["metersPerDegLat"], p["metersPerDegLon"]
    out = {}
    for g in d["greens"]:
        pg = g["polygon"]
        out[g["id"]] = [(c["lon"] + pg[i] / mo, c["lat"] - pg[i + 1] / ml) for i in range(0, len(pg), 2)]
    return out


def clip_ring(ring, box):
    """Sutherland-Hodgman: the ring (lon, lat) clipped to box (lon0, lat0, lon1, lat1)."""
    def cut(pts, inside, inter):
        out = []
        for i, p in enumerate(pts):
            q = pts[i - 1]
            if inside(p):
                if not inside(q):
                    out.append(inter(q, p))
                out.append(p)
            elif inside(q):
                out.append(inter(q, p))
        return out
    lon0, lat0, lon1, lat1 = box
    def at_x(x):
        return lambda a, b: (x, a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0]))
    def at_y(y):
        return lambda a, b: (a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]), y)
    pts = list(ring)
    for inside, inter in ((lambda p: p[0] >= lon0, at_x(lon0)), (lambda p: p[0] <= lon1, at_x(lon1)),
                          (lambda p: p[1] >= lat0, at_y(lat0)), (lambda p: p[1] <= lat1, at_y(lat1))):
        if not pts:
            break
        pts = cut(pts, inside, inter)
    return pts


def in_ring(ring, x, y):
    ins, j = False, len(ring) - 1
    for i in range(len(ring)):
        (xi, yi), (xj, yj) = ring[i], ring[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
            ins = not ins
        j = i
    return ins


def seg_meets_box(a, b, box):
    """Liang-Barsky: whether segment a-b meets the box (x0, y0, x1, y1)."""
    x0, y0, x1, y1 = box
    t0, t1, dx, dy = 0.0, 1.0, b[0] - a[0], b[1] - a[1]
    for p, q in ((-dx, a[0] - x0), (dx, x1 - a[0]), (-dy, a[1] - y0), (dy, y1 - a[1])):
        if p == 0:
            if q < 0:
                return False
        else:
            t = q / p
            if p < 0:
                t0 = max(t0, t)
            else:
                t1 = min(t1, t)
            if t0 > t1:
                return False
    return True


def ring_meets_box(ring, box):
    x0, y0, x1, y1 = box
    if any(x0 <= p[0] <= x1 and y0 <= p[1] <= y1 for p in ring):
        return True
    if in_ring(ring, (x0 + x1) / 2, (y0 + y1) / 2):
        return True
    return any(seg_meets_box(ring[i - 1], ring[i], box) for i in range(len(ring)))


def area_rings(name, greens):
    a = AREAS[name]
    rings = []
    for gid in a["greens"]:
        if gid not in greens:
            sys.exit(f"{name}: green {gid} is not in {GREENS}")
        r = greens[gid]
        if "clip" in a:
            r = clip_ring(r, a["clip"])
        if len(r) >= 3:
            rings.append(r)
    return rings


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", default=os.environ.get("EO_LIDAR_DIR") or os.path.join(REPO, "external", "lidar"))
    ap.add_argument("--area", action="append", default=[], choices=sorted(AREAS))
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--max-depth", type=int, default=8, help="deepest octree level fetched (default 8)")
    ap.add_argument("--margin", type=float, default=30.0, help="ground metres round each area polygon (default 30)")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    names = sorted(AREAS) if a.all else a.area
    if not names:
        sys.exit("name an --area or pass --all")

    greens = load_greens()
    ept = json.loads(get(f"{BASE}/ept.json"))
    x0, y0, _, x1, _, _ = ept["bounds"]
    hier = {}

    def hierarchy(key):
        if key not in hier:
            hier[key] = json.loads(get(f"{BASE}/ept-hierarchy/{key}.json"))
        return hier[key]

    def node_box(key):
        d, x, y, _ = (int(v) for v in key.split("-"))
        s = (x1 - x0) / (1 << d)
        return x0 + x * s, y0 + y * s, x0 + (x + 1) * s, y0 + (y + 1) * s

    grand = [0, 0, 0]
    for name in names:
        rings = area_rings(name, greens)
        lat_c = sum(p[1] for r in rings for p in r) / sum(len(r) for r in rings)
        grow = a.margin / (R * math.cos(math.radians(lat_c))) * (180 / math.pi)  # metres -> degrees of longitude
        grow_lat = a.margin / R * (180 / math.pi)

        def meets(key):
            bx0, by0, bx1, by1 = node_box(key)
            lo0, la0 = lonlat(bx0, by0)
            lo1, la1 = lonlat(bx1, by1)
            box = (lo0 - grow, la0 - grow_lat, lo1 + grow, la1 + grow_lat)
            return any(ring_meets_box(r, box) for r in rings)

        picked, pending = {}, ["0-0-0-0"]
        while pending:
            for key, n in hierarchy(pending.pop()).items():
                if int(key.split("-")[0]) > a.max_depth or not meets(key):
                    continue
                if n == -1:
                    pending.append(key)
                elif n > 0:
                    picked[key] = n
        by_depth = {}
        for k, n in picked.items():
            d = int(k.split("-")[0])
            by_depth.setdefault(d, [0, 0])
            by_depth[d][0] += 1
            by_depth[d][1] += n
        pts = sum(picked.values())
        print(f"{name}: {len(picked)} nodes, {pts:,} points through depth {a.max_depth}; per depth {dict(sorted(by_depth.items()))}")
        if a.dry_run:
            grand[0] += len(picked)
            grand[1] += pts
            continue

        out = os.path.join(a.root, name)
        os.makedirs(os.path.join(out, "ept-data"), exist_ok=True)

        def fetch(key):
            p = os.path.join(out, "ept-data", f"{key}.laz")
            url = f"{BASE}/ept-data/{key}.laz"
            retrieved = None
            if not os.path.exists(p):
                data = get(url)
                retrieved = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
                with open(p + ".part", "wb") as f:
                    f.write(data)
                os.replace(p + ".part", p)
            with open(p, "rb") as f:
                data = f.read()
            return key, url, retrieved, len(data), hashlib.sha256(data).hexdigest()

        old = {}
        prov_path = os.path.join(out, "PROVENANCE.json")
        if os.path.exists(prov_path):
            old = {f["path"]: f for f in json.load(open(prov_path)).get("files", [])}
        files = []
        with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, a.jobs)) as ex:
            for i, (key, url, retrieved, nbytes, sha) in enumerate(ex.map(fetch, sorted(picked))):
                path = f"ept-data/{key}.laz"
                prev = old.get(path)
                if retrieved is None:
                    if prev and prev.get("sha256") == sha:
                        retrieved = prev.get("retrieved")
                    else:
                        retrieved = "unknown (file present before this run)"
                files.append({"path": path, "url": url, "points": picked[key], "bytes": nbytes, "sha256": sha, "retrieved": retrieved})
                if (i + 1) % 100 == 0:
                    print(f"  {i + 1}/{len(picked)}")
        prov = {
            "source": "USGS 3D Elevation Program, CA_SanFrancisco_B23 (work unit CA_SanFrancisco_1_B23), Entwine Point Tiles on the AWS Open Data bucket usgs-lidar-public",
            "url": BASE,
            "licence": "Public domain (U.S. Government work)",
            "attribution": "U.S. Geological Survey, 3D Elevation Program",
            "crs": "EPSG:3857 plan; heights NAVD88 metres as delivered",
            "generator": "native/tools/fetch-lidar.py",
            "area": {"name": name, "label": AREAS[name]["label"], "greens": AREAS[name]["greens"], "clip_lonlat": AREAS[name].get("clip"),
                     "greens_file": "public/data/sf/sf-lots-parks.json.gz", "margin_m": a.margin, "max_depth": a.max_depth,
                     "rings_lonlat": [[[round(p[0], 7), round(p[1], 7)] for p in r] for r in rings]},
            "ept_bounds": ept["bounds"],
            "ept_scale_offset": {s["name"]: [s.get("scale"), s.get("offset")] for s in ept["schema"] if s["name"] in ("X", "Y", "Z")},
            "files": files,
        }
        with open(prov_path + ".part", "w") as f:
            json.dump(prov, f, indent=1)
        os.replace(prov_path + ".part", prov_path)
        mb = sum(f["bytes"] for f in files) / 1e6
        grand[0] += len(files)
        grand[1] += pts
        grand[2] += sum(f["bytes"] for f in files)
        print(f"  wrote {len(files)} files, {mb:.1f} MB, {prov_path}")
    print(f"total: {grand[0]} nodes, {grand[1]:,} points" + ("" if a.dry_run else f", {grand[2] / 1e6:.1f} MB"))


if __name__ == "__main__":
    main()
