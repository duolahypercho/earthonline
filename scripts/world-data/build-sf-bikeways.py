"""Bake San Francisco's bikeway network: each street segment's bike facility.

Source: DataSF "MTA Bike Network Linear Features" (ygmz-vaxd), maintained by
the SFMTA, PDDL 1.0. One row per facility and direction on a street segment
(CNN): its class (I path, II lane, III route, IV separated bikeway), the
direction it carries, whether it runs against the street's traffic
(contraflow), its painted buffer, raised bikeway, barrier (concrete, safe-hit
posts, parking, ...) and surface treatment (green paint), with its line.

Writes public/data/sf/sf-bikeways.json.gz and its NOTICE. Positions are in the
city file's local metre frame (sf-city.json.gz `meta`: x east, z south).

  python3 scripts/world-data/build-sf-bikeways.py [--raw DIR]
"""
import datetime as dt
import gzip
import hashlib
import io
import json
from pathlib import Path
import sys
from urllib.parse import urlencode
from urllib.request import Request, urlopen

VERSION = "sf-bikeways-v1"
root = Path(__file__).resolve().parents[2]
out_dir = root / "public/data/sf"
portal = "https://data.sf.gov"
dataset = "ygmz-vaxd"
rows_url = f"{portal}/resource/{dataset}.json?" + urlencode({
    "$select": "cnn,facility_t,dir,contraflow,buffered,raised,barrier,surface_tr,double,update_yr,shape",
    "$order": "objectid",
    "$limit": "50000",
})
meta_url = f"{portal}/api/views/{dataset}"

raw_dir = None
if "--raw" in sys.argv:
    raw_dir = Path(sys.argv[sys.argv.index("--raw") + 1])
    raw_dir.mkdir(parents=True, exist_ok=True)


def fetch(name, url):
    if raw_dir is not None and (raw_dir / name).exists():
        return (raw_dir / name).read_bytes()
    body = urlopen(Request(url, headers={"User-Agent": "earth-online-data-build/1.0"}), timeout=300).read()
    if raw_dir is not None:
        (raw_dir / name).write_bytes(body)
    return body


raw_rows = fetch("bikeways.json", rows_url)
raw_meta = fetch("bikeways-meta.json", meta_url)
meta = json.loads(raw_meta)
lic = (meta.get("license") or {}).get("name", "")
assert "Public Domain Dedication" in lic, f"licence changed: {lic!r}; review before shipping"

city = json.load(gzip.open(root / "public/data/sf/sf-city.json.gz"))["meta"]
c_lat, c_lon = city["center"]["lat"], city["center"]["lon"]
m_lat = city["projection"]["metersPerDegreeLat"]
m_lon = city["projection"]["metersPerDegreeLon"]


def local(lon, lat):
    return [round((lon - c_lon) * m_lon, 1), round((c_lat - lat) * m_lat, 1)]


CLASSES = {"CLASS I": 1, "CLASS II": 2, "CLASS III": 3, "CLASS IV": 4}
# barrier words -> bits (sf-bikeways.json `barrierBits`)
BARRIER = [("CONCRETE", 1), ("ISLAND", 1), ("CURB", 1), ("PLANTER", 1), ("K-RAIL", 1), ("SAFE", 2), ("POST", 2), ("PARKING", 4), ("SIDEWALK LEVEL", 8)]
rows = json.loads(raw_rows)
assert 0 < len(rows) < 50000
records, skipped = [], 0
for r in rows:
    cls = CLASSES.get((r.get("facility_t") or "").strip())
    shape = r.get("shape") or {}
    coords = shape.get("coordinates")
    if not cls or not coords or not r.get("cnn"):
        skipped += 1
        continue
    if shape.get("type") == "MultiLineString":
        coords = [p for part in coords for p in part]
    if len(coords) < 2:
        skipped += 1
        continue
    words = (r.get("barrier") or "").upper()
    barrier = 0
    for w, bit in BARRIER:
        if w in words:
            barrier |= bit
    surface = (r.get("surface_tr") or "").upper()
    flags = (1 if r.get("contraflow") == "YES" else 0) | (2 if r.get("buffered") == "YES" else 0) | (4 if r.get("raised") == "YES" else 0) | (8 if "GREEN PAINT" in surface else 0) | (16 if r.get("double") == "1" else 0)
    line = []
    for lon, lat in coords:
        line += local(lon, lat)
    records.append([int(float(r["cnn"])), cls, (r.get("dir") or "").strip(), flags, barrier, line])

as_of = dt.datetime.fromtimestamp(meta["rowsUpdatedAt"], dt.timezone.utc).strftime("%Y-%m-%d") if meta.get("rowsUpdatedAt") else None
doc = {
    "version": VERSION,
    "asOf": as_of,
    "fields": ["cnn", "class", "dir", "flags", "barrierBits", "line"],
    "flags": {"contraflow": 1, "buffered": 2, "raised": 4, "greenPaint": 8, "otherSide": 16},
    "barrierBits": {"concreteOrIsland": 1, "posts": 2, "parking": 4, "sidewalkLevel": 8},
    "records": records,
}
bio = io.BytesIO()
with gzip.GzipFile(fileobj=bio, mode="wb", mtime=0) as gz:
    gz.write(json.dumps(doc, separators=(",", ":")).encode())
target = out_dir / "sf-bikeways.json.gz"
target.write_bytes(bio.getvalue())
digest = lambda b: hashlib.sha256(b).hexdigest()
by = {}
for rec in records:
    by[rec[1]] = by.get(rec[1], 0) + 1
(out_dir / "sf-bikeways.NOTICE.txt").write_text(
    "sf-bikeways.json.gz - San Francisco bikeway network\n\n"
    f"Source: DataSF, SFMTA: MTA Bike Network Linear Features ({dataset}), rows updated {as_of}\n"
    "License: Open Data Commons Public Domain Dedication and License (PDDL) 1.0\n"
    "         http://opendatacommons.org/licenses/pddl/1.0/\n"
    "Attribution (courtesy): City and County of San Francisco, SFMTA, via DataSF.\n"
    f"Retrieved from: {rows_url}\n"
    f"Source SHA-256: {digest(raw_rows)}\n"
    f"Produced by:   scripts/world-data/build-sf-bikeways.py ({VERSION})\n"
    f"Output SHA-256: {digest(target.read_bytes())} (sf-bikeways.json.gz)\n\n"
    f"Kept: {len(records)} facility records (class I {by.get(1, 0)}, II {by.get(2, 0)}, III {by.get(3, 0)}, IV {by.get(4, 0)}); {skipped} rows without a class or line.\n"
    "Carried per record: CNN, class, direction, flags (contraflow, buffered, raised, green paint, other side),\n"
    "barrier bits (concrete or island, safe-hit posts, parking, sidewalk level) and the line in the city file's\n"
    "local metre frame (sf-city.json.gz meta: x east, z south).\n"
)
print(len(records), "records", skipped, "skipped", by, target.stat().st_size, "bytes")
