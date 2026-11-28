"""Bake each street segment's pavement condition and last resurfacing date.

Sources (San Francisco Public Works, DataSF, both PDDL 1.0):
  * Streets Data - Pavement Condition Index (PCI) Scores (5aye-4rtt): one row
    per street segment (CNN) with its current score, the date of that score,
    whether it was a survey or a resurfacing ("Treatment") and its line.
  * Streets Data - Historical Pavement Condition Index (PCI) Scores (78va-8dhi):
    every survey and resurfacing since 1992; only each segment's latest
    resurfacing date is kept.

Writes public/data/sf/sf-pavement.json.gz and its NOTICE. Positions are in the
city file's local metre frame (sf-city.json.gz `meta`: x east, z south), so the
game reads them as it reads the city's own points.

  python3 scripts/world-data/build-sf-pavement.py [--raw DIR]

With --raw DIR the original responses are written there (or read from there
when present), so a build can be repeated from an archived snapshot.
"""
import gzip
import hashlib
import io
import json
from pathlib import Path
import sys
from urllib.parse import urlencode
from urllib.request import Request, urlopen

VERSION = "sf-pavement-v1"
root = Path(__file__).resolve().parents[2]
out_dir = root / "public/data/sf"
portal = "https://data.sf.gov"
current_id, history_id = "5aye-4rtt", "78va-8dhi"
current_url = f"{portal}/resource/{current_id}.json?" + urlencode({
    "$select": "cnn,pci_score,pci_change_date,treatment_or_survey,functional_class,line,point",
    "$order": "cnn",
    "$limit": "50000",
})
history_url = f"{portal}/resource/{history_id}.json?" + urlencode({
    "$select": "cnn,max(pci_change_date) as last_treatment",
    "$where": "treatment_or_survey='Treatment'",
    "$group": "cnn",
    "$order": "cnn",
    "$limit": "50000",
})
meta_urls = {k: f"{portal}/api/views/{k}" for k in (current_id, history_id)}

raw_dir = None
if "--raw" in sys.argv:
    raw_dir = Path(sys.argv[sys.argv.index("--raw") + 1])
    raw_dir.mkdir(parents=True, exist_ok=True)


def fetch(name, url):
    """The response body (archived under --raw when given)."""
    if raw_dir is not None and (raw_dir / name).exists():
        return (raw_dir / name).read_bytes()
    req = Request(url, headers={"User-Agent": "earth-online-data-build/1.0"})
    body = urlopen(req, timeout=300).read()
    if raw_dir is not None:
        (raw_dir / name).write_bytes(body)
    return body


raw = {
    "current": fetch("pci-current.json", current_url),
    "history": fetch("pci-history-treatments.json", history_url),
    "meta-current": fetch("pci-current-meta.json", meta_urls[current_id]),
    "meta-history": fetch("pci-history-meta.json", meta_urls[history_id]),
}
meta = {k: json.loads(raw[f"meta-{k}"]) for k in ("current", "history")}
for k, m in meta.items():
    lic = (m.get("license") or {}).get("name", "")
    assert "Public Domain Dedication" in lic, f"{k}: licence changed: {lic!r}; review before shipping"

city = json.load(gzip.open(root / "public/data/sf/sf-city.json.gz"))["meta"]
c_lat, c_lon = city["center"]["lat"], city["center"]["lon"]
m_lat = city["projection"]["metersPerDegreeLat"]
m_lon = city["projection"]["metersPerDegreeLon"]


def local(lon, lat):
    return [round((lon - c_lon) * m_lon, 1), round((c_lat - lat) * m_lat, 1)]


def day(s):
    return s[:10] if s else None


rows = json.loads(raw["current"])
assert 0 < len(rows) < 50000, f"{len(rows)} rows: the query limit may have cut the table"
last = {r["cnn"]: day(r.get("last_treatment")) for r in json.loads(raw["history"])}
classes = {"Arterial": "A", "Collector": "C", "Residential": "R"}
segments, points, skipped = [], [], 0
for r in rows:
    try:
        cnn = int(r["cnn"])
        pci = int(round(float(r["pci_score"])))
    except (KeyError, ValueError):
        skipped += 1
        continue
    kind = "T" if r.get("treatment_or_survey") == "Treatment" else "S"
    date = day(r.get("pci_change_date"))
    treated = last.get(r["cnn"])
    if kind == "T" and date and (treated is None or date > treated):
        treated = date
    rec = [cnn, max(0, min(100, pci)), date, kind, classes.get(r.get("functional_class") or "", ""), treated]
    line = (r.get("line") or {}).get("coordinates")
    if line and len(line) >= 2:
        pts = []
        for lon, lat in line:
            pts += local(lon, lat)
        segments.append(rec + [pts])
    elif r.get("point"):
        lon, lat = r["point"]["coordinates"]
        points.append(rec + local(lon, lat))
    else:
        skipped += 1

as_of = meta["current"].get("rowsUpdatedAt")
import datetime as _dt

as_of_day = _dt.datetime.fromtimestamp(as_of, _dt.timezone.utc).strftime("%Y-%m-%d") if as_of else None
doc = {
    "version": VERSION,
    "asOf": as_of_day,
    "frame": {"centerLat": c_lat, "centerLon": c_lon, "metersPerDegreeLat": m_lat, "metersPerDegreeLon": m_lon},
    "fields": ["cnn", "pci", "pciDate", "treatmentOrSurvey", "class", "lastTreatment", "geometry"],
    "segments": segments,
    "points": points,
}
bio = io.BytesIO()
with gzip.GzipFile(fileobj=bio, mode="wb", mtime=0) as gz:
    gz.write(json.dumps(doc, separators=(",", ":")).encode())
target = out_dir / "sf-pavement.json.gz"
target.write_bytes(bio.getvalue())

with_date = sum(1 for s in segments if s[5])
digest = lambda b: hashlib.sha256(b).hexdigest()
(out_dir / "sf-pavement.NOTICE.txt").write_text(
    "sf-pavement.json.gz - San Francisco street pavement condition and last resurfacing\n\n"
    "Source: DataSF, San Francisco Public Works:\n"
    f"  Streets Data - Pavement Condition Index (PCI) Scores ({current_id}), rows updated {as_of_day}\n"
    f"  Streets Data - Historical Pavement Condition Index (PCI) Scores ({history_id}), latest resurfacing per segment\n"
    "License: Open Data Commons Public Domain Dedication and License (PDDL) 1.0\n"
    "         http://opendatacommons.org/licenses/pddl/1.0/\n"
    "Attribution (courtesy): City and County of San Francisco, San Francisco Public Works, via DataSF.\n"
    f"Retrieved from:\n  {current_url}\n  {history_url}\n"
    f"Source SHA-256: {current_id} {digest(raw['current'])}\n"
    f"                {history_id} {digest(raw['history'])}\n"
    f"Produced by:   scripts/world-data/build-sf-pavement.py ({VERSION})\n"
    f"Output SHA-256: {digest(target.read_bytes())} (sf-pavement.json.gz)\n\n"
    f"Kept: {len(segments)} street segments with a line ({with_date} with a recorded resurfacing date), "
    f"{len(points)} intersection points; {skipped} rows without a usable score or position.\n"
    "Carried per record: CNN, PCI score (0-100), the score's date, survey or treatment, functional class,\n"
    "the latest resurfacing date, and the segment line (or intersection point) in the city file's local\n"
    "metre frame (sf-city.json.gz meta: x east, z south). The game reads the age of the pavement from\n"
    "the resurfacing date against the snapshot date above, not against the clock.\n"
)
print(len(segments), "segments", len(points), "points", skipped, "skipped", with_date, "with a resurfacing date", target.stat().st_size, "bytes")
