#!/usr/bin/env python3
"""Recover the two median light standards of each east-span Skyway bay from USGS 3DEP.

Reads laspy[lazrs] and NumPy, the bridge profile register and the city projection.
Original EPT tiles are cached beside the inputs of measure-bridge-roads.py.
"""
import argparse
import gzip
import hashlib
import io
import json
import re
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.request import urlopen

import laspy
import numpy as np

ROOT = Path(__file__).resolve().parents[2]
SOURCE = "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/CA_SanFrancisco_1_B23"
TOWER = np.array([5361.05, -4122.52])  # east_span.rs, from the 3DEP tower returns


def sample(profiles, meta, cache):
    hashes = {}

    def read(key):
        path = cache / key.replace("/", "_")
        if not path.is_file():
            with urlopen(f"{SOURCE}/{key}", timeout=60) as source:
                data = source.read()
            path.write_bytes(data)
        data = path.read_bytes()
        hashes[key] = hashlib.sha256(data).hexdigest()
        return data

    bounds = json.loads(read("ept.json"))["bounds"]
    origin, proj = meta["center"], meta["projection"]
    lon, lat = origin["lon"], origin["lat"]
    dx, dz = proj["metersPerDegreeLon"], proj["metersPerDegreeLat"]
    radius = 6378137
    sites = []
    starts = []
    for a in profiles:
        v = np.diff(a[:, :2], axis=0)
        length = np.linalg.norm(v, axis=1)
        ds = np.r_[0, np.cumsum(length)]
        t = np.clip(np.sum((TOWER - a[:-1, :2]) * v, axis=1) / length**2, 0, 1)
        j = np.argmin(np.linalg.norm(a[:-1, :2] + t[:, None]*v - TOWER, axis=1))
        # Caltrans fig. 1.1-1 gives 385 m from the SAS tower to pier E2.
        start = np.interp(ds[j] + t[j]*length[j] + 385, ds, a[:, 0])
        starts.append(start)
        near = a[a[:, 0] >= start-65]
        sites.append((radius*np.radians(lon + near[:, 0]/dx),
                      radius*np.arcsinh(np.tan(np.radians(lat - near[:, 1]/dz)))))
    keys = set()

    def visit(root):
        for key, value in json.loads(read(f"ept-hierarchy/{root}.json")).items():
            lod, i, j, k = map(int, key.split("-"))
            if lod > 9:
                continue
            size = (bounds[3] - bounds[0]) / 2**lod
            x, y, z = (bounds[n]+v*size for n, v in enumerate((i, j, k)))
            if z > 95 or z+size < 0:
                continue
            if not any(((xx >= x-95) & (xx <= x+size+95) &
                        (yy >= y-95) & (yy <= y+size+95)).any() for xx, yy in sites):
                continue
            if value == -1 and key != root:
                visit(key)
            elif value > 0:
                keys.add(key)
    visit("0-0-0-0")

    def points(key):
        tile = laspy.read(io.BytesIO(read(f"ept-data/{key}.laz")))
        x = (np.degrees(np.asarray(tile.x)/radius)-lon)*dx
        z = (lat-np.degrees(np.arctan(np.sinh(np.asarray(tile.y)/radius))))*dz
        y = np.asarray(tile.z)
        out = []
        for a, side, start in zip(profiles, (1,-1), starts):
            lateral = (z - np.interp(x, a[:, 0], a[:, 1])) * side
            height = y - np.interp(x, a[:, 0], a[:, 2])
            # A buffer about the observed inner shoulder. Heights are relative
            # to the supporting deck to follow its grade through the touchdown.
            take = (x >= start) & (x <= a[-1,0]+3) & (lateral>8) & (lateral<20) & (height>3) & (height<25)
            out.append(np.column_stack([x[take],z[take],height[take]]))
        return out

    with ThreadPoolExecutor(4) as pool:
        samples = list(pool.map(points, sorted(keys)))
    return [np.vstack(side) for side in zip(*samples)], hashes


def stems(points):
    # A connected section at 4.5–6.5 m isolates the shaft beneath its optics.
    section = points[(points[:, 2] >= 4.5) & (points[:, 2] < 6.5)]
    occupied = {}
    for i, cell in enumerate(np.floor(section[:, :2]/0.7).astype(int)):
        occupied.setdefault(tuple(cell), []).append(i)
    found = []
    while occupied:
        pending = [next(iter(occupied))]
        indices = []
        while pending:
            x, z = pending.pop()
            if (x,z) not in occupied:
                continue
            indices.extend(occupied.pop((x,z)))
            pending.extend((x+i,z+j) for i in (-1,0,1) for j in (-1,0,1) if (x+i,z+j) in occupied)
        shaft = section[indices]
        if len(shaft)<7 or np.any(np.ptp(shaft[:, :2],axis=0)>1.7):
            continue
        foot = np.median(shaft[:, :2], axis=0)
        pole = points[np.sum((points[:,:2]-foot)**2, axis=1)<1.65**2]
        top = np.quantile(pole[:,2], 0.995)
        if top >= 10.5 and np.count_nonzero(pole[:,2] > top-1) >= 5:
            found.append((foot, top, pole, shaft))
    return sorted(found, key=lambda p: p[0][0])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=ROOT/"external/survey/bridge-roads")
    cache = parser.parse_args().cache
    cache.mkdir(exist_ok=True, parents=True)
    register = (ROOT/"native/eo-data/src/bridge_readings.rs").read_text()
    profiles = []
    for name in ("EAST_WESTBOUND","EAST_EASTBOUND"):
        literal = re.search(name+r": .*? = &(\[.*?\]);", register, re.S).group(1)
        a = np.array(json.loads(literal.replace(",\n]", "\n]")))
        profiles.append(a[np.argsort(a[:,0])])
    with gzip.open(ROOT/"public/data/sf/sf-city.json.gz") as f:
        meta = json.load(f)["meta"]
    points, hashes = sample(profiles, meta, cache)
    roads = [stems(p) for p in points]
    # Five height populations separated by gaps in the surveyed cap elevations.
    def group(h):
        return int(np.searchsorted([12.,14.5,17.5,20.],h))
    families = []
    for g in range(5):
        offsets = []
        widths = []
        for sign, rows in zip((-1,1), roads):
            for foot, top, pole, shaft in rows:
                if group(top) == g:
                    # Only protruding forward-facing optics, clear of the shaft.
                    offsets.extend(top-pole[(pole[:,0]-foot[0])*sign>0.63,2])
                    widths.append(np.ptp(shaft[:,:2],axis=0))
        deltas = np.asarray(offsets)
        drops = []
        for j in range(1,10):
            band = deltas[(deltas>j+0.1)&(deltas<j+0.9)]
            if len(band)<10:
                break
            drops.append(np.median(band))
        if not 3 <= len(drops) <= 7:
            raise ValueError(f"unresolved optical rows for family {g}: {drops}")
        families.append((len(drops),drops[0],np.median(np.diff(drops)),*np.median(widths,axis=0)))
    output = ["//! Generated by native/tools/measure-skyway-lamps.py, from USGS 3DEP.",
              "//! Dimensions are source metres. See data/skyway-lamps.NOTICE.txt.",
              "pub const LAMP_FAMILIES: &[(usize, f64, f64, f64, f64)] = &["]
    for count, first, pitch, width, depth in families:
        output.append(f"    ({count}, {first:.3f}, {pitch:.3f}, {width:.3f}, {depth:.3f}),")
    output.append("];")
    for name,rows in zip(("WESTBOUND","EASTBOUND"),roads):
        output.extend(["",f"pub const {name}: &[([f64; 2], f64, usize)] = &["])
        for (x,z),top,_,_ in rows:
            output.append(f"    ([{x:.2f}, {z:.2f}], {top:.2f}, {group(top)}),")
        output.append("];")
    data = ("\n".join(output)+"\n").encode()
    (ROOT/"native/eo-data/src/skyway_readings.rs").write_bytes(data)
    inventory = "\n".join(f"{k} {v}" for k,v in sorted(hashes.items())).encode()
    note = f"""USGS 3D Elevation Program, CA_SanFrancisco_1_B23. Public domain, United States Government work.
{SOURCE}/
Horizontal EPSG:3857, vertical NAVD88 metres. Game plan from OpenStreetMap contributors (ODbL).
Generated by native/tools/measure-skyway-lamps.py from EPT levels 0 through 9 and the registered deck profiles.
From pier E2 to the end of each mapped carriageway, {len(roads[0])} westbound and {len(roads[1])} eastbound shafts
were measured on the median shoulders. Recorded cap height is above the local roadway, including foundation.
Shaft centres at 4.5–6.5 m and near-maximum cap heights are individually measured. Each of the five pole
sizes uses the median first optical row and row spacing from forward bracket returns in its own size group.
The two horizontal shaft spreads are the median source X/Z bounds across the measured stems in each group.
Piers: Caltrans 2014, SFOBB Evaluation of ASTM A354 Grade BD Rods, fig. 1.1-1, https://files.mtc.ca.gov/library/pub/28860.pdf
Housing finish, pentagonal profile and direction of optics: Bay Area Toll Authority / Caltrans, Lighting the East Span,
pp. 1–2, https://files.mtc.ca.gov/library/pub/non/47654_12.pdf
The runtime fits the mount to its own inner parapet. Housing proportions and aiming angles are approximations.
sha256 for sorted source entries (each line is path, space, SHA-256): {hashlib.sha256(inventory).hexdigest()} ({len(hashes)} files)
sha256 for skyway_readings.rs: {hashlib.sha256(data).hexdigest()}
"""
    (ROOT/"native/eo-data/data/skyway-lamps.NOTICE.txt").write_text(note)
    print([len(r) for r in roads], families)

if __name__ == "__main__":
    main()
