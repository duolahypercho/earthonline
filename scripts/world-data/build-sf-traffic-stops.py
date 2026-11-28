"""Retain OSM stop scope and signed approaches for the packed San Francisco map."""

import argparse
import collections
import gzip
import hashlib
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET


OSM_SHA256 = "a2b744fbbb8e5246ba827482a12d452e630e248a5cef2919b69c4e23f7b4260b"
CITY_SHA256 = "f1f41c398c24f6636c593a1aefb14e8e4d016b0a800d8f81d0fe7ad522639f1d"
STREET_CLASSES = {
    "primary", "secondary", "tertiary", "unclassified", "residential", "service",
    "living_street", "primary_link", "secondary_link", "tertiary_link", "residential_link",
}


def verify(path, expected):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    if digest.hexdigest() != expected:
        raise ValueError(f"{path} does not match its recorded SHA-256")


def extract(osm, city):
    layer = city["streetPoints"]
    frame, origin = city["meta"]["projection"], city["meta"]["center"]
    bounds = layer["bbox"]
    ways = {str(way["id"]) for way in city["roads"] if way["highway"] in STREET_CLASSES}
    positions, stops = {}, {}
    parents = collections.defaultdict(list)
    with gzip.open(osm) as stream:
        events = iter(ET.iterparse(stream, events=("start", "end")))
        _, root = next(events)
        for event, entity in events:
            if event != "end":
                continue
            if entity.tag == "node":
                lat, lon = float(entity.attrib["lat"]), float(entity.attrib["lon"])
                if bounds["minLat"] <= lat <= bounds["maxLat"] and bounds["minLon"] <= lon <= bounds["maxLon"]:
                    node = entity.attrib["id"]
                    positions[node] = (
                        (lon - origin["lon"]) * frame["metersPerDegreeLon"],
                        (origin["lat"] - lat) * frame["metersPerDegreeLat"],
                    )
                    tags = {tag.attrib["k"]: tag.attrib["v"] for tag in entity.iterfind("tag")}
                    if tags.get("highway") == "stop":
                        stops[node] = tags.get("stop"), tags.get("stop:direction") or tags.get("direction")
            elif entity.tag == "way" and entity.attrib["id"] in ways:
                refs = [part.attrib["ref"] for part in entity.iterfind("nd")]
                for i, node in enumerate(refs):
                    if node in stops:
                        before = positions.get(refs[i - 1]) if i else None
                        after = positions.get(refs[i + 1]) if i + 1 < len(refs) else None
                        parents[node].append((entity.attrib["id"], before, after))
            if entity.tag in {"node", "way", "relation"}:
                root.clear()

    cells = collections.defaultdict(list)
    for node in stops:
        x, z = positions[node]
        cells[(math.floor(x), math.floor(z))].append(node)
    packed = layer["features"]["stopSign"]
    controls, matches = {}, 0
    if len(packed["xz"]) != 2 * len(packed["attr"]):
        raise ValueError("Invalid packed stop coordinates and direction categories")
    for x, z, code in zip(packed["xz"][::2], packed["xz"][1::2], packed["attr"]):
        candidates = [
            n for i in range(math.floor(x) - 1, math.floor(x) + 2)
            for j in range(math.floor(z) - 1, math.floor(z) + 2)
            for n in cells.get((i, j), [])
            if abs(positions[n][0] - x) <= 0.05 and abs(positions[n][1] - z) <= 0.05
        ]
        if len(candidates) != 1:
            continue
        node = candidates[0]
        scope, direction = stops[node]
        if code != ("b" if direction == "both" else "d" if direction else "-"):
            continue
        matches += 1
        ways = parents[node]
        if scope == "all" and direction in (None, "both"):
            # A junction has at least three legs, including split and unsplit ways.
            rays = {p for _, before, after in ways for p in (before, after) if p not in (None, positions[node])}
            if len(rays) >= 3:
                controls[node] = {"junction": sorted({way for way, _, _ in ways}, key=int)}
        elif scope in (None, "all", "minor") and direction in ("forward", "backward"):
            inbound = [(w, before if direction == "forward" else after) for w, before, after in ways]
            inbound = [(w, p) for w, p in inbound if p is not None and p != positions[node]]
            if len(inbound) == 1:
                way, prior = inbound[0]
                dx, dz = (b - a for a, b in zip(prior, positions[node]))
                length = math.hypot(dx, dz)
                controls[node] = {
                    "way": way, "toward": [round(dx / length, 8), round(dz / length, 8)], "all_way": scope == "all",
                }
    print(f"{matches} matched source records, {len(controls)} controls with junction or approach evidence")
    return [{"id": int(n), "position": [round(x, 3) for x in positions[n]], **controls[n]}
            for n in sorted(controls, key=int)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("osm", type=Path, help="Pinned BBBike San Francisco OSM XML gzip")
    parser.add_argument("city", type=Path, help="Shipped sf-city.json.gz")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    verify(args.osm, OSM_SHA256)
    verify(args.city, CITY_SHA256)
    with gzip.open(args.city, "rt") as stream:
        stops = extract(args.osm, json.load(stream))
    rows = ",\n".join(json.dumps(stop, separators=(",", ":")) for stop in stops)
    encoded = ('{"version":1,"stops":[\n' + rows + "\n]}\n").encode("utf-8")
    args.output.write_bytes(encoded)
    print(f"{hashlib.sha256(encoded).hexdigest()}  {args.output}")


if __name__ == "__main__":
    main()
