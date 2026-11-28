#!/usr/bin/env python3
"""Restore the road centreline over the movable leaves at Islais Creek."""
import gzip
import hashlib
import json
from pathlib import Path
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[2]
CACHE = ROOT / "external/survey/movable-road-spans"
OUTPUT = ROOT / "native/eo-data/data/movable-road-spans.json"
WAYS = "667768321v4,667768323v4,675892536v7"
NODES = "6252543988v1,6252543989v1,6252543990v1,6252543991v1,6252543992v2,11733512894v1"
# Fixed road ways meeting the first and last nodes in each leaf's point order.
APPROACHES = {
    667768321: (28846364, 667768322),
    667768323: (28846361, 667768320),
    675892536: (675892535, 675892538),
}


def source(kind, refs, notice):
    url = f"https://api.openstreetmap.org/api/0.6/{kind}.json?{kind}={refs}"
    cache = CACHE / f"{hashlib.sha256(url.encode()).hexdigest()}.json"
    if not cache.exists():
        cache.write_bytes(urlopen(url, timeout=30).read())
    raw = cache.read_bytes()
    elements = json.loads(raw)["elements"]
    assert set(refs.split(",")) == {f"{e['id']}v{e['version']}" for e in elements}
    notice.extend([url, "SHA-256 " + hashlib.sha256(raw).hexdigest()])
    return elements


def main():
    CACHE.mkdir(parents=True, exist_ok=True)
    notice = ["© OpenStreetMap contributors, ODbL 1.0.", "https://www.openstreetmap.org/copyright",
              "Movable road leaves, with their original directions and tags. Coordinates use the shipped SF projection and 0.1 m precision."]
    with gzip.open(ROOT / "public/data/sf/sf-city.json.gz") as f:
        city = json.load(f)
    center, projection = city["meta"]["center"], city["meta"]["projection"]
    ways = source("ways", WAYS, notice)
    nodes = {p["id"]: p for p in source("nodes", NODES, notice)}
    spans = []
    for way in sorted(ways, key=lambda w: w["id"]):
        tags, points = way["tags"], []
        assert tags["bridge"] == "movable"
        for node in way["nodes"]:
            p = nodes[node]
            points += [round((p["lon"] - center["lon"]) * projection["metersPerDegreeLon"], 1),
                       round((center["lat"] - p["lat"]) * projection["metersPerDegreeLat"], 1)]
        fixed = APPROACHES[way["id"]]
        for approach, hinge in zip(fixed, [points[:2], points[-2:]]):
            ref = next(r for r in city["roads"] if r["id"] == approach)
            assert ref["bridge"] and hinge in [ref["points"][:2], ref["points"][-2:]]
        spans.append(dict(id=way["id"], approach=str(fixed[0]), name=tags["name"], highway=tags["highway"],
                          lanes=int(tags["lanes"]), oneway=tags.get("oneway") == "yes", maxspeed=tags.get("maxspeed", ""),
                          sidewalk=tags.get("sidewalk", ""), surface=tags.get("surface", ""), bridge=True,
                          tunnel=False, layer=tags["layer"], points=points))
    OUTPUT.write_text(json.dumps(spans, indent=2, ensure_ascii=False) + "\n")
    notice.append("Output SHA-256 " + hashlib.sha256(OUTPUT.read_bytes()).hexdigest())
    OUTPUT.with_suffix(".NOTICE.txt").write_text("\n".join(notice) + "\n")


if __name__ == "__main__":
    main()
