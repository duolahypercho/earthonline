"""Bake the city's kerbside parking and mid-block signalised crossings.

Sources (DataSF, all PDDL 1.0):
  * Parking regulations (except non-metered color curb) (hi6h-neyh), SFMTA:
    one line per regulated stretch of kerb (time limited, permit, paid, no
    parking ...), drawn along its side of the street.
  * Parking Meters (8vzz-qzz9), SFMTA: one point per on-street metered space.
  * Traffic Signals (ybh5-27n2), SFMTA: the signals, beacons and flashers whose
    cross street is MIDBLOCK (a crossing between junctions).

Writes public/data/sf/sf-street-survey.json.gz and its NOTICE, positions in the
city file's local metre frame (sf-city.json.gz `meta`: x east, z south).

  python3 scripts/world-data/build-sf-street-survey.py [--raw DIR]
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

VERSION = "sf-street-survey-v1"
root = Path(__file__).resolve().parents[2]
out_dir = root / "public/data/sf"
portal = "https://data.sf.gov"
SETS = {
    "parking": ("hi6h-neyh", {"$select": "regulation,shape", "$order": "objectid", "$limit": "50000"}),
    "meters": ("8vzz-qzz9", {"$select": "on_offstreet_type,active_meter_flag,cap_color,longitude,latitude", "$where": "on_offstreet_type='ON'", "$order": "objectid", "$limit": "50000"}),
    "signals": ("ybh5-27n2", {"$select": "cnn,street1,street2,type,ped_signal,shape", "$where": "upper(street1) like '%MIDBLOCK%' OR upper(street2) like '%MIDBLOCK%'", "$order": "objectid", "$limit": "5000"}),
}
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


urls, raw, meta = {}, {}, {}
for key, (dataset, q) in SETS.items():
    urls[key] = f"{portal}/resource/{dataset}.json?" + urlencode(q)
    raw[key] = fetch(f"{key}.json", urls[key])
    m = json.loads(fetch(f"{key}-meta.json", f"{portal}/api/views/{dataset}"))
    lic = (m.get("license") or {}).get("name", "")
    assert "Public Domain Dedication" in lic, f"{dataset}: licence changed: {lic!r}; review before shipping"
    meta[key] = (dataset, m.get("name"), dt.datetime.fromtimestamp(m["rowsUpdatedAt"], dt.timezone.utc).strftime("%Y-%m-%d") if m.get("rowsUpdatedAt") else None)

city = json.load(gzip.open(root / "public/data/sf/sf-city.json.gz"))["meta"]
c_lat, c_lon = city["center"]["lat"], city["center"]["lon"]
m_lat = city["projection"]["metersPerDegreeLat"]
m_lon = city["projection"]["metersPerDegreeLon"]


def local(lon, lat):
    return [round((lon - c_lon) * m_lon, 2), round((c_lat - lat) * m_lat, 2)]


# Parking: whether the stretch lets a car park (1) or not (0).
ALLOWED = ("time limited", "pay or permit", "paid + permit", "government permit", "no overnight parking", "no oversized vehicles")
BANNED = ("no parking any time", "no parking anytime", "no stopping")
parking, p_skip = [], 0
for r in json.loads(raw["parking"]):
    reg = (r.get("regulation") or "").strip().lower()
    allowed = 1 if reg in ALLOWED else 0 if reg in BANNED else None
    shape = r.get("shape") or {}
    coords = shape.get("coordinates")
    if allowed is None or not coords:
        p_skip += 1
        continue
    if shape.get("type") == "MultiLineString":
        coords = [p for part in coords for p in part]
    line = [v for lon, lat in coords for v in local(lon, lat)]
    if len(line) >= 4:
        parking.append([allowed, line])
    else:
        p_skip += 1

meters, m_skip = [], 0
for r in json.loads(raw["meters"]):
    try:
        lon, lat = float(r["longitude"]), float(r["latitude"])
    except (KeyError, TypeError, ValueError):
        m_skip += 1
        continue
    if (r.get("active_meter_flag") or "") not in ("M", "T", "L", "U"):
        m_skip += 1
        continue
    meters += local(lon, lat)

# Mid-block crossings: signals, beacons and flashers (not radar speed or message signs, not future ones).
KINDS = {"SIGNAL": 1, "PENDING SIGNAL": 1, "BEACON": 2, "BEACON - RRFB": 2, "PENDING BEACON": 2, "PENDING SIGNAL - FLASHER": 3, "FLASHER": 3, "LIGHTED CROSSWALK": 3}
midblock, s_skip = [], 0
for r in json.loads(raw["signals"]):
    kind = KINDS.get((r.get("type") or "").strip().upper())
    pt = (r.get("shape") or {}).get("coordinates")
    if not kind or not pt:
        s_skip += 1
        continue
    midblock.append([int(float(r.get("cnn") or 0)), kind] + local(pt[0], pt[1]))

doc = {
    "version": VERSION,
    "asOf": {k: v[2] for k, v in meta.items()},
    "parking": parking,
    "meters": meters,
    "midblock": midblock,
    "fields": {"parking": ["allowed", "line"], "meters": "flat x z pairs", "midblock": ["cnn", "kind 1 signal / 2 beacon / 3 flasher", "x", "z"]},
}
bio = io.BytesIO()
with gzip.GzipFile(fileobj=bio, mode="wb", mtime=0) as gz:
    gz.write(json.dumps(doc, separators=(",", ":")).encode())
target = out_dir / "sf-street-survey.json.gz"
target.write_bytes(bio.getvalue())
digest = lambda b: hashlib.sha256(b).hexdigest()
lines = "".join(f"  {name} ({ds}), rows updated {asof}\n" for ds, name, asof in meta.values())
(out_dir / "sf-street-survey.NOTICE.txt").write_text(
    "sf-street-survey.json.gz - San Francisco kerbside parking and mid-block signalised crossings\n\n"
    f"Source: DataSF, SFMTA:\n{lines}"
    "License: Open Data Commons Public Domain Dedication and License (PDDL) 1.0\n"
    "         http://opendatacommons.org/licenses/pddl/1.0/\n"
    "Attribution (courtesy): City and County of San Francisco, SFMTA, via DataSF.\n"
    "Retrieved from:\n" + "".join(f"  {u}\n" for u in urls.values()) +
    "Source SHA-256:\n" + "".join(f"  {meta[k][0]} {digest(raw[k])}\n" for k in raw) +
    f"Produced by:   scripts/world-data/build-sf-street-survey.py ({VERSION})\n"
    f"Output SHA-256: {digest(target.read_bytes())} (sf-street-survey.json.gz)\n\n"
    f"Kept: {len(parking)} parking regulation stretches (allowed or banned; {p_skip} others skipped), "
    f"{len(meters) // 2} on-street meters ({m_skip} skipped), {len(midblock)} mid-block signals, beacons and flashers ({s_skip} other devices skipped).\n"
    "Carried: whether a stretch lets a car park and its line, each meter's point, and each mid-block device's\n"
    "kind and point, in the city file's local metre frame (sf-city.json.gz meta: x east, z south).\n"
)
print(len(parking), "parking", len(meters) // 2, "meters", len(midblock), "midblock", target.stat().st_size, "bytes")
