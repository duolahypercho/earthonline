#!/usr/bin/env python3
"""Fit the terraces of the eight concrete shafts above the bridge sidewalks.

Run with numpy and laspy[lazrs]. A crown is the connected group of returns
above a measured horizontal ledge. Fit its plan in strips so the steps and
recesses survive without triangulating the individual laser returns.
"""
import argparse
from collections import deque
import hashlib
import json
from pathlib import Path
import runpy

import numpy as np

lamp_survey = runpy.run_path(str(Path(__file__).with_name("measure-golden-gate-lamps.py")))
ROOT = lamp_survey["ROOT"]
OUT = ROOT / "native/eo-landmarks/data/golden-gate-pylons.json"
# Search windows in the registered 3DEP frame, from the south tower. The
# windows bracket the shafts; the output comes from occupied cells inside.
SEARCH = [("S2", -454.5, 13.6, 16.0), ("S1", -345.3, 13.6, 16.0),
          ("N1", 1620.4, 11.8, 15.1), ("N2", 1719.5, 18.5, 22.2)]
CELL = 0.25


def connected(mask):
    remaining = set(map(tuple, np.argwhere(mask)))
    largest = []
    while remaining:
        item = remaining.pop()
        group = [item]
        queue = deque(group)
        while queue:
            i, j = queue.popleft()
            for other in ((i-1, j), (i+1, j), (i, j-1), (i, j+1)):
                if other in remaining:
                    remaining.remove(other)
                    group.append(other)
                    queue.append(other)
        if len(group) > len(largest):
            largest = group
    return np.asarray(largest)


def simplify(rows, tolerance):
    # Keep the cross stations needed to fit both plan edges to one grid cell.
    a = np.asarray(rows)
    t = (a[:, 0] - a[0, 0]) / (a[-1, 0] - a[0, 0])
    error = np.max(abs(a[:, 1:] - (a[0, 1:] + t[:, None]*(a[-1, 1:]-a[0, 1:]))), axis=1)
    i = int(error.argmax())
    if error[i] <= tolerance:
        return [rows[0], rows[-1]]
    return simplify(rows[:i+1], tolerance)[:-1] + simplify(rows[i:], tolerance)


def footprint(points, station, lateral, deck, level, top):
    p = points[(points[:, 2] >= deck+level-.16) & (points[:, 2] <= deck+top+.2)]
    along = station + np.arange(-7, 7.1, CELL)
    cross = lateral + np.arange(-6, 6.1, CELL)
    counts = np.histogram2d(p[:, 1], p[:, 0], bins=(cross, along))[0]
    # Isolated cable and equipment returns do not support a concrete face.
    padded = np.pad(counts > 0, 1)
    neighbours = sum(padded[i:i+counts.shape[0],j:j+counts.shape[1]].astype(int)
                     for i in range(3) for j in range(3))
    cells = connected(neighbours >= 7)
    if len(cells) < 70:
        raise ValueError(f"Undersampled crown at {station}, {lateral}, level {level}")
    rows = []
    for i in sorted(set(cells[:, 0])):
        columns = cells[cells[:, 0] == i, 1]
        if len(columns) >= 4:
            rows.append([cross[i]+CELL/2, along[columns.min()], along[columns.max()+1]])
    # Boundary stations lie on the outer face of the two end cell rows.
    rows[0][0] -= CELL/2
    rows[-1][0] += CELL/2
    return [[round(float(x), 2) for x in row] for row in simplify(rows, CELL)]


def measure(points):
    records = []
    for name, station, west, east in SEARCH:
        local = points[abs(points[:, 0]-station)<7]
        road = local[(abs(local[:, 1])<7) & (local[:, 3] == 17) & (abs(local[:, 0]-station)<2)]
        if len(road) < 50:
            raise ValueError(f"Missing 3DEP road returns at {name}")
        deck = np.median(road[:, 2])
        for centre in (-west, east):
            samples = local[abs(local[:, 1]-centre)<6]
            levels = []
            for lower in ([5.1, 7.1, 8.95] if name.startswith("S") else [5.1, 7.1]):
                edges = np.arange(lower, lower+1.5, .05)
                counts = np.histogram(samples[:, 2]-deck, bins=edges)[0]
                peak = edges[counts.argmax()]+.025
                plane = samples[abs(samples[:, 2]-deck-peak)<.12, 2]
                if len(plane) < 100:
                    raise ValueError(f"Missing terrace at {name}, {centre}, {lower}")
                levels.append(round(float(np.median(plane)-deck), 2))
            terraces = [{"height": level, "sections": footprint(samples, station, centre, deck, level, levels[-1])}
                        for level in levels]
            records.append({"pylon": name, "side": "west" if centre<0 else "east", "terraces": terraces})
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=ROOT/"external/survey/bridge-roads")
    args = parser.parse_args()
    points, hashes = lamp_survey["sample"](args.cache, stations=[r[1] for r in SEARCH],
                                            lateral_limit=32, search_radius=58)
    rows = measure(points)
    OUT.write_text(json.dumps(rows, separators=(",", ":"), allow_nan=False)+"\n")
    note = ["Golden Gate concrete pylon crowns, USGS 3DEP CA_SanFrancisco_1_B23 (2023).",
            "Public domain, United States Geological Survey. " + lamp_survey["SOURCE"] + "/ept.json",
            "Heights above classified roadway at the pylon, metres. Plan sections are [offset from the mapped carriageway centre, south and north station from the south tower], metres.",
            "Rebuild: python3 native/tools/measure-golden-gate-pylons.py",
            "Output SHA-256: "+hashlib.sha256(OUT.read_bytes()).hexdigest(), "", "Source SHA-256 and path:"]
    OUT.with_suffix(".NOTICE.txt").write_text("\n".join(note+[f"{v}  {k}" for k,v in sorted(hashes.items())])+"\n")
    print(f"{len(rows)} shafts, {sum(len(r['terraces']) for r in rows)} terraces, {len(hashes)} source files")


if __name__ == "__main__":
    main()
