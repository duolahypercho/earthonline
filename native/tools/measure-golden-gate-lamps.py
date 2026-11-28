#!/usr/bin/env python3
"""Measure the mapped roadway lamp heads in the USGS 3DEP 2023 point cloud.

Run with numpy and laspy[lazrs]. The inventory consists of search windows
identified in a side elevation of the returns. A window is never an emitted
fixture without a sampled horizontal head and classified roadway below it.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import gzip
import hashlib
import io
import json
from pathlib import Path
from urllib.request import urlopen

import laspy
import numpy as np

ROOT = Path(__file__).resolve().parents[2]
SOURCE = "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/CA_SanFrancisco_1_B23"
OUT = ROOT / "native/eo-landmarks/data/golden-gate-lamps.json"


def windows():
    # Search centres in metres from the south tower, increasing toward Marin.
    regular = [-418.7, -381.4, *(-296.7 + 45.45*np.arange(7)),
               *(23.3 + 45.48*np.arange(28)), *(1298.6 + 45.46*np.arange(7))]
    north = [
        [1648.2, 1683.7, 1724.8, 1749.3, 1785.5, 1817.6, 1853.8, 1883.1, 1916.2, 1944.6, 1975.6, 2001.6, 2025.9],
        [1648.1, 1683.7, 1724.8, 1749.3, 1785.5, 1817.6, 1853.8, 1886.3, 1922.4, 1953.5, 1987.3, 2016.1],
    ]
    south = [[-630.7, -592., -547.8, -501.5], [-655.9, -620.4, -585.9, -543.1, -499.8]]
    for i, side in enumerate((-1, 1)):
        for station in sorted(regular + north[i] + south[i]):
            yield station, side, station in south[i]


def sample(cache, *, stations=None, lateral_limit=23, search_radius=32):
    cache.mkdir(parents=True, exist_ok=True)
    hashes = {}

    def read(key):
        path = cache / key.replace("/", "_")
        if not path.exists():
            with urlopen(f"{SOURCE}/{key}", timeout=60) as f:
                data = f.read()
            path.write_bytes(data)
        data = path.read_bytes()
        hashes[key] = hashlib.sha256(data).hexdigest()
        return data

    with gzip.open(ROOT / "public/data/sf/sf-city.json.gz") as f:
        meta = json.load(f)["meta"]
    origin, proj = meta["center"], meta["projection"]
    gate = json.loads((ROOT / "native/eo-landmarks/data/landmarks-js.json").read_text())["goldenGate"]
    south, north = [np.mean(list(gate["towerLegs"][side].values()), axis=0) for side in ("south", "north")]
    axis = (north - south) / np.linalg.norm(north - south)
    across = np.array([-axis[1], axis[0]])
    curves = []
    for way in gate["carriageways"]:
        points = np.array(way).reshape((-1, 2)) - south
        order = np.argsort(points @ axis)
        curves.append(((points @ axis)[order], (points @ across)[order]))

    def offset(t):
        return np.mean([np.interp(t, c, a) for c, a in curves], axis=0)

    if stations is None:
        stations = np.array([w[0] for w in windows()])
    stations = np.asarray(stations)
    plan = south + stations[:, None] * axis + offset(stations)[:, None] * across
    radius = 6378137
    mx = radius * np.radians(origin["lon"] + plan[:, 0] / proj["metersPerDegreeLon"])
    my = radius * np.arcsinh(np.tan(np.radians(origin["lat"] - plan[:, 1] / proj["metersPerDegreeLat"])))
    bounds = json.loads(read("ept.json"))["bounds"]
    keys = set()

    def visit(root):
        for key, count in json.loads(read(f"ept-hierarchy/{root}.json")).items():
            level, i, j, k = map(int, key.split("-"))
            size = (bounds[3] - bounds[0]) / 2**level
            x, y, h = (bounds[n] + v*size for n, v in enumerate((i, j, k)))
            overlap = (mx+search_radius >= x) & (mx-search_radius <= x+size) & (my+search_radius >= y) & (my-search_radius <= y+size)
            if h > 115 or h+size < 30 or not overlap.any():
                continue
            if count == -1 and key != root:
                visit(key)
            elif count > 0:
                keys.add(key)
    visit("0-0-0-0")

    def read_points(key):
        tile = laspy.read(io.BytesIO(read(f"ept-data/{key}.laz")))
        x = (np.degrees(np.asarray(tile.x)/radius) - origin["lon"])*proj["metersPerDegreeLon"] - south[0]
        z = (origin["lat"] - np.degrees(np.arctan(np.sinh(np.asarray(tile.y)/radius))))*proj["metersPerDegreeLat"] - south[1]
        t = x*axis[0] + z*axis[1]
        a = x*across[0] + z*across[1] - offset(t)
        h = np.asarray(tile.z)
        keep = (abs(a)<lateral_limit) & (h>30) & (h<115)
        return np.column_stack((t[keep], a[keep], h[keep], np.asarray(tile.classification)[keep]))

    with ThreadPoolExecutor(4) as pool:
        points = np.concatenate(list(pool.map(read_points, sorted(keys))))
    return points, hashes


def measure(points):
    points = points[np.argsort(points[:, 0])]
    rows = []
    for seed, side, tall in windows():
        i, j = np.searchsorted(points[:, 0], [seed-2.2, seed+2.2])
        station, lateral, elevation, classification = points[i:j].T
        lateral = lateral * side
        road = elevation[(abs(lateral) < 8) & (classification == 17)]
        if len(road) < 8:
            raise ValueError(f"Unmeasured deck under {seed}, side {side}")
        h = elevation - np.median(road)
        floor, ceiling = (10.55, 11.9) if tall and side < 0 else (9.6, 10.8) if tall else (7.4, 8.55)
        head = (classification == 1) & (lateral > 6) & (lateral < 23) & (h > floor) & (h < ceiling)
        if 475 < seed < 800:
            # At midspan the main cable crosses the height of the lamp heads.
            # Isolate the inboard housing in the cross section.
            along = 10.0 if side < 0 else 12.6
            head &= abs(lateral - along) < .85
        xe, ye = np.arange(seed-2.25, seed+2.51, .5), np.arange(6, 23.51, .5)
        count = np.histogram2d(station[head], lateral[head], bins=(xe, ye))[0]
        ti, ai = np.unravel_index(count.argmax(), count.shape)
        face = head & (abs(lateral - (ye[ai]+.25)) < .8) & (abs(station - (xe[ti]+.25)) < .75)
        if face.sum() < 7:
            raise ValueError(f"Unmeasured head at {seed}, side {side}")
        rows.append([round(float(np.median(station[face])), 2), side,
                     round(float(np.percentile(h[face], 90)), 2), int(tall)])
    return sorted(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=ROOT/"external/survey/bridge-roads")
    points, hashes = sample(parser.parse_args().cache)
    rows = measure(points)
    OUT.write_text('[\n' + ',\n'.join('  '+json.dumps(row) for row in rows) + '\n]\n')
    notice = ["Golden Gate roadway luminaires, USGS 3D Elevation Program, CA_SanFrancisco_1_B23.",
              "Public domain, United States Geological Survey. Dataset: " + SOURCE + "/ept.json",
              "Columns: station from south tower (m), side (-1 west / +1 east), crown above roadway (m), tall approach type (0/1).",
              "Rebuild: python3 native/tools/measure-golden-gate-lamps.py",
              "Output SHA-256: " + hashlib.sha256(OUT.read_bytes()).hexdigest(), "", "Source SHA-256 and path:"]
    OUT.with_suffix(".NOTICE.txt").write_text("\n".join(notice + [f"{v}  {k}" for k,v in sorted(hashes.items())]) + "\n")
    print(f"{len(rows)} measured fixtures, {len(hashes)} source files")


if __name__ == "__main__":
    main()
