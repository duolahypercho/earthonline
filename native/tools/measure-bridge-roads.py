#!/usr/bin/env python3
"""Regenerate bridge deck and approach profiles from USGS 3DEP bridge and ground returns.

Requires laspy[lazrs] and NumPy. The cache holds the original Entwine tiles.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import gzip
import hashlib
import io
import json
import math
from pathlib import Path
from urllib.request import urlopen

import laspy
import numpy as np

ROOT = Path(__file__).resolve().parents[2]
SOURCE = "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/CA_SanFrancisco_1_B23"
EAST = {"236348361": "EAST_WESTBOUND", "237731428": "EAST_EASTBOUND"}
RINCON = {"50690291": "RINCON_LOWER", "739661211": "RINCON_MAIN", "179235237": "RINCON_RAMP"}


PLAZA = {way: "PLAZA_" + way for way in ("26968357", "920648684", "920648685", "920648686", "920648689")}
FEEDERS = {way: "FEEDER_" + way for way in ("123741452", "468201421", "27772070", "397094807", "839972871")}
PAVED = {"1212176124": "ISLAND_LANDING", "60319196": "RINCON_UNDERPASS", "1034687991": "ISLAND_UNDERPASS", **PLAZA, **FEEDERS}
DRIVEWAYS = {"8917011": "SAINT_CHARLES", "459121915": "SAINT_CHARLES_DRIVE"}
MARIPOSA = {way: "MARIPOSA_STREET" for way in ("396965550", "544431982", "544431983", "623399444")}
MARIPOSA.update({way: "MARIPOSA_RAMP" for way in ("8915249", "1111444346", "945101895")})
APRON_X = range(2284, 2311, 2)
APRON_Z = range(1542, 1567, 2)


def mariposa_returns(name, x, z, deck, ground, other):
    # Observed lower-road bands at exposed stations, in source metres and
    # NAVD88. These identify the correct cluster among returns on the rail
    # bed, ramp and freeway. Only actual returns enter the output profile.
    if name == "MARIPOSA_STREET":
        center = np.interp(x, [2256.2, 2294.5, 2313.7, 2356.1, 2367.9],
                           [12.0, 12.9, 13.0, 12.2, 11.4])
    else:
        center = np.interp(z, [1540.3, 1558.4, 1599.4, 1653.4, 1691.3, 1760.7],
                           [13.0, 12.9, 15.2, 18.3, 19.5, 20.6])
    for group, kind in ((deck, 17), (ground, 2), (other, 1)):
        measured = [v for v in group if abs(v - center) <= 0.65]
        if len(measured) >= 5:
            return measured, kind
    return [], None


def sample_line(points, step):
    samples = []
    for a, b in zip(points, points[1:]):
        n = math.ceil(math.dist(a, b) / step)
        samples.extend(tuple(u + (v - u) * i / n for u, v in zip(a, b)) for i in range(n))
    return samples + [points[-1]]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=ROOT / "external/survey/bridge-roads")
    cache = parser.parse_args().cache
    cache.mkdir(parents=True, exist_ok=True)
    hashes = {}

    def read(key):
        path = cache / key.replace("/", "_")
        if not path.exists():
            with urlopen(f"{SOURCE}/{key}", timeout=60) as f:
                data = f.read()
            path.write_bytes(data)
        data = path.read_bytes()
        hashes[key] = digest(data)
        return data

    with gzip.open(ROOT / "public/data/sf/sf-city.json.gz") as f:
        city = json.load(f)
    landmarks = json.loads((ROOT / "native/eo-landmarks/data/landmarks-js.json").read_bytes())
    center = city["meta"]["center"]
    projection = city["meta"]["projection"]
    ends = landmarks["bayBridgeWest"]["deckEnds"]
    gate = landmarks["goldenGate"]["wayEnds"]
    locations = {"BAY_SF": [(ends["sf"]["x"], ends["sf"]["z"])],
                 "BAY_ISLAND": [(ends["island"]["x"], ends["island"]["z"])],
                 "GATE_SOUTH": [gate["south"]], "GATE_NORTH": [gate["north"]]}
    for road in city["roads"]:
        way = str(road["id"])
        if way in MARIPOSA:
            points = list(zip(road["points"][::2], road["points"][1::2]))
            # Stop on the exposed straight after the freeway diverges.
            if way == "945101895":
                points = points[:3]
            locations.setdefault(MARIPOSA[way], []).extend(sample_line(points, 6))
        elif way in EAST or way in RINCON or way in PAVED or way in DRIVEWAYS:
            points = list(zip(road["points"][::2], road["points"][1::2]))
            step = 3 if way in DRIVEWAYS else 60 if way in EAST else 18
            locations[(EAST | RINCON | PAVED | DRIVEWAYS)[way]] = points if way == "179235237" else sample_line(points, step)
    assert set(EAST.values()) | set(RINCON.values()) | set(PAVED.values()) <= locations.keys()
    for name in set(MARIPOSA.values()):
        locations[name] = sorted(set(locations[name]))
    locations["MARIPOSA_APRON"] = [(x, z) for z in APRON_Z for x in APRON_X]

    # Sample both hinges and the centre of each closed Islais Creek leaf.
    spans = json.loads((ROOT / "native/eo-data/data/movable-road-spans.json").read_text())
    for span in spans:
        points = list(zip(span["points"][::2], span["points"][1::2]))
        locations[f"ISLAIS_{span['id']}"] = [points[0], tuple((a+b)/2 for a,b in zip(points[0],points[-1])), points[-1]]

    records = []
    for name, positions in locations.items():
        for x, z in positions:
            lon = math.radians(center["lon"] + x / projection["metersPerDegreeLon"])
            lat = math.radians(center["lat"] - z / projection["metersPerDegreeLat"])
            radius = 4 if name in MARIPOSA.values() else 1 if name in DRIVEWAYS.values() or name == "MARIPOSA_APRON" else 2 if name == "ISLAND_UNDERPASS" else 5
            records.append((name, x, z, 6378137 * lon, 6378137 * math.asinh(math.tan(lat)), radius / math.cos(lat)))
    extent = json.loads(read("ept.json"))["bounds"]
    keys = {}

    def visit(root):
        for key, count in json.loads(read(f"ept-hierarchy/{root}.json")).items():
            level, i, j, k = map(int, key.split("-"))
            if level > 8:
                continue
            size = (extent[3] - extent[0]) / 2**level
            a, b, h = (extent[n] + idx * size for n, idx in enumerate((i, j, k)))
            if h > 100 or h + size < 0:
                continue
            sites = [n for n, (_, _, _, mx, my, r) in enumerate(records)
                     if a-r <= mx <= a+size+r and b-r <= my <= b+size+r]
            if sites and count == -1 and key != root:
                visit(key)
            elif sites and count > 0:
                # The island underpass is narrow and partly hidden. Entwine
                # partitions the returns among levels, so include its parents.
                selected = [i for i in sites if level == 8 or records[i][0] in ("ISLAND_UNDERPASS", "MARIPOSA_APRON") or records[i][0] in DRIVEWAYS.values() or records[i][0] in MARIPOSA.values()]
                if selected:
                    keys[key] = selected
    visit("0-0-0-0")

    def sample(key):
        tile = laspy.read(io.BytesIO(read(f"ept-data/{key}.laz")))
        classes = np.asarray(tile.classification)
        x, y, z = (np.asarray(v) for v in (tile.x, tile.y, tile.z))
        result = []
        for i in keys[key]:
            _, _, _, mx, my, radius = records[i]
            disc = (x-mx)**2 + (y-my)**2 <= radius**2
            other = z[disc & (classes == 1)] if records[i][0] in MARIPOSA.values() else ()
            result.append((i, z[disc & (classes == 17)], z[disc & (classes == 2)], other))
        return result

    heights = [[] for _ in records]
    ground = [[] for _ in records]
    other = [[] for _ in records]
    with ThreadPoolExecutor(6) as pool:
        for result in pool.map(sample, sorted(keys)):
            for i, deck_values, ground_values, other_values in result:
                heights[i].extend(deck_values)
                ground[i].extend(ground_values)
                other[i].extend(other_values)
    # YBITS carries the eastbound roadway under the westbound roadway. The
    # first two unobscured eastbound centreline discs are east of their fork.
    # Continue their measured longitudinal grade through the short overlap;
    # the overhead laser returns in the overlap belong to the westbound road.
    exposed = sorted((x, np.median(heights[i])) for i, (name, x, *_) in enumerate(records)
                     if name == "EAST_EASTBOUND" and x >= 5000)
    (x0, h0), (x1, h1) = exposed[:2]
    tables = {}
    counts = []
    apron = {}
    for (name, x, z, *_), values, soil, unclassified in zip(records, heights, ground, other):
        classification = 17
        if name == "MARIPOSA_APRON":
            # Only build complete cells on the lower deck or its continuous
            # abutment. Omit occluded cells and the track bed to the south.
            band = [v for v in (*values, *soil) if 12.0 <= v <= 13.4]
            if len(band) >= 5:
                apron[x, z] = round(float(np.median(band)) * 100)
            continue
        if name in MARIPOSA.values():
            values, classification = mariposa_returns(name, x, z, values, soil, unclassified)
            if not values:
                if (x, z) in (locations[name][0], locations[name][-1]):
                    raise ValueError(f"No pavement at survey boundary of {name} ({x}, {z})")
                counts.append(f"{name} ({x:.2f}, {z:.2f}): occluded; interpolate between visible stations")
                continue
        if name in DRIVEWAYS.values():
            if soil:
                values, classification = soil, 2
            elif name != "SAINT_CHARLES" or (x, z) != locations[name][-1]:
                raise ValueError(f"No exposed street returns at {name} ({x}, {z})")
        if name.endswith("_UNDERPASS"):
            # Ground under the stacked decks. Interpolate between the visible
            # ground returns on either side where the decks occlude the laser.
            if not soil:
                continue
            if name == "ISLAND_UNDERPASS" and values:
                # A few returns on the overhead slab carry the ground class.
                # Keep ground only when the two surfaces are distinguishable.
                soil = [v for v in soil if v < min(values) - 1.0]
                if not soil:
                    continue
            values, classification = soil, 2
        if name in PAVED.values() and not values:
            values, classification = soil, 2
        if not values:
            raise ValueError(f"No classified deck at {name} ({x}, {z})")
        h = float(np.median(values))
        if name in {"FEEDER_123741452", "FEEDER_468201421", "FEEDER_27772070"} and classification == 17:
            h -= landmarks["bayBridgeWest"]["deckStepM"]
        if name in RINCON.values():
            # Lower ramp is directly visible beside the upper carriageway at its
            # west end. Elsewhere the laser sees the upper deck over the lower.
            if name == "RINCON_RAMP" and (x, z) == locations[name][0]:
                h = float(np.median([v for v in values if v < (min(values) + max(values)) / 2]))
            else:
                h -= landmarks["bayBridgeWest"]["deckStepM"]
        inferred = name == "ISLAND_LANDING" or (name == "EAST_EASTBOUND" and x < x0)
        if inferred:
            h = h0 + (h1 - h0) * (x - x0) / (x1 - x0)
        tables.setdefault(name, []).append((x, z, h))
        counts.append(f"{name} ({x:.2f}, {z:.2f}): {len(values)} class {classification} returns, adopted {h:.2f} m" + (" (inferred beneath upper roadway)" if inferred else ""))
    code = ["//! Surveyed in source plan metres and NAVD88 metres.",
            "//! Generated by native/tools/measure-bridge-roads.py.", ""]
    for name, table in tables.items():
        if len(table) == 1:
            code.append(f"pub const {name}_NAVD88_M: f64 = {table[0][2]:.2f};")
        else:
            code.append(f"pub const {name}: &[[f64; 3]] = &[")
            axis = 1 if name.endswith("_UNDERPASS") or name.startswith("ISLAIS_") or name in ("SAINT_CHARLES", "MARIPOSA_RAMP") else 0
            ordered = sorted(table, key=lambda p: p[axis])
            if any(round(a[axis], 2) >= round(b[axis], 2) for a, b in zip(ordered, ordered[1:])):
                raise ValueError(f"Repeated interpolation station on {name}")
            for x, z, h in ordered:
                code.append(f"    [{x:.2f}, {z:.2f}, {h:.2f}],")
            code.append("];")
    code.extend([
        "/// Lower apron grid in NAVD88 centimetres. Zero marks an unsurveyed vertex.",
        f"pub const APRON_ORIGIN: [f64; 2] = [{APRON_X.start}.0, {APRON_Z.start}.0];",
        f"pub const APRON_SPACING: f64 = {APRON_X.step}.0;",
        f"pub const MARIPOSA_APRON: &[[i16; {len(APRON_X)}]] = &[",
        *["    [" + ", ".join(str(apron.get((x, z), 0)) for x in APRON_X) + "]," for z in APRON_Z],
        "];",
    ])
    out = ROOT / "native/eo-data/src/bridge_readings.rs"
    source = "\n".join(code) + "\n"
    if not out.exists() or out.read_text() != source:
        out.write_text(source)
    notice = ["USGS 3D Elevation Program, CA_SanFrancisco_1_B23. Public domain (United States Government work).",
              f"Source root: {SOURCE}/", "XY EPSG:3857, Z NAVD88 metres. Level 8 LAS bridge deck within 5 m; class 2 on exposed pavement.",
              "East span pier chainages digitized from Caltrans, SFOBB Self-Anchored Suspension Bridge,",
              "Evaluation of the ASTM A354 Grade BD Rods (2014), fig. 1.1-1 and pp. 1-2 to 1-3.",
              "https://files.mtc.ca.gov/library/pub/28860.pdf",
              "Skyway box depth (midspan): https://rosap.ntl.bts.gov/view/dot/27621/dot_27621_DS1.pdf",
              "SAS main cable, approximately 2.6 ft diameter: SJSU College of Engineering, Spring 2012, p. 6,",
              "https://www.sjsu.edu/engineering/docs/magazines/spring-2012.pdf",
              "Transition structures: https://files.mtc.ca.gov/library/pub/30903.pdf",
              "Plan coordinates: city road ways (OpenStreetMap contributors, ODbL) and landmarks-js.json endpoints.",
              "Adopted deck elevations are medians. At Rincon the covered lower road is inferred from the upper",
              "deck minus the west span register's 6.42 m separation, except the exposed west end of the on-ramp.",
              "Treasure Island Road uses visible class 2 inside a 2 m disc, levels 0-8; pavement hidden",
              "by bridge decks is interpolated between exposed returns.",
              "Saint Charles Avenue and its service drive use class 2 medians in 1 m discs at up to 3 m spacing,",
              "levels 0-8. The southernmost street sample is class 17 on the deck, continuous with the exposed pavement.",
              "Mariposa Street over the Caltrain tracks and its southbound on-ramp use levels 0-8 in 4 m discs.",
              "Follow the lower roadway band between exposed stations; prefer class 17, then 2, then 1.",
              "Use at least five returns in the band; interpolate covered stations between measured ends.",
              "Behind the southwest curb use a 2 m grid of class 2 and 17 medians in 1 m discs, levels 0-8.",
              "Retain five or more returns in the observed lower apron band (12.0-13.4 m NAVD88). Omit missing cells.",
              "Crossing identification: Caltrain bridge inventory, https://www.caltrain.com/media/36811",
              "At Yerba Buena Island the covered eastbound centreline continues the grade between its first",
              "two exposed 3DEP deck discs. The island street blends into its existing tunnel approach.",
              f"Output src/bridge_readings.rs SHA-256 {digest(out.read_bytes())}", "", *counts, "",
              *(f"{v}  {k}" for k, v in sorted(hashes.items()))]
    (ROOT / "native/eo-data/data").mkdir(exist_ok=True)
    (ROOT / "native/eo-data/data/bridge-roads.NOTICE.txt").write_text("\n".join(notice) + "\n")


if __name__ == "__main__":
    main()
