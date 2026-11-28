#!/usr/bin/env python3
"""Fetch San Francisco's active parcel outlines for one small area from DataSF.

Source: DataSF "Parcels - Active and Retired" (dataset acdm-wktn), recorded
parcel geography from San Francisco Public Works' basemap with the Assessor's
block and lot numbers. Licence: Open Data Commons Public Domain Dedication and
License (PDDL 1.0, http://opendatacommons.org/licenses/pddl/1.0/), as the
dataset's metadata states (`licenseId: PDDL`). Attribution: City and County of
San Francisco.

One request through the Socrata (SODA) API: the active parcels whose outline
meets a box round (--lat, --lon), --half metres each way, rounded out to
1e-4 degrees, only the columns the yard survey reads. The box uses the game's
own plan frame (metres per degree at 37.778 N: 110574 north, 87986.25 east),
so it is the same square the lidar and the captures use. Files land in a
folder OUTSIDE the repository:

    python3 -I native/tools/fetch-parcels.py --root <folder> --name liberty-hill \
        --lat 37.7570 --lon -122.4250 --half 150
    python3 -I native/tools/fetch-parcels.py ... --count     # how many parcels, nothing written

`--root` defaults to `EO_PARCELS_DIR`. Writes <root>/<name>/parcels.geojson and
<root>/<name>/manifest.json (the query, its URL, the licence, the date, the
bytes and their sha-256). The yard survey
(`cargo run --release -p eo-kit --example yard_survey -- <name>`) reads them.
"""
import argparse, datetime, hashlib, json, math, os, sys, urllib.parse, urllib.request

# data.sfgov.org answers with a permanent redirect to this host.
BASE = "https://data.sf.gov/resource/acdm-wktn"
DATASET = "https://data.sfgov.org/d/acdm-wktn"
AGENT = "earth-online-parcels-fetch/1"
M_PER_DEG_LAT = 110574.0
M_PER_DEG_LON = 87986.25
COLUMNS = "blklot,mapblklot,block_num,lot_num,from_address_num,to_address_num,street_name,street_type,active,date_map_add,date_map_alt,shape"


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": AGENT})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=os.environ.get("EO_PARCELS_DIR"))
    ap.add_argument("--name", required=True)
    ap.add_argument("--lat", type=float, required=True)
    ap.add_argument("--lon", type=float, required=True)
    ap.add_argument("--half", type=float, default=150.0, help="half side of the area, ground metres")
    ap.add_argument("--count", action="store_true", help="count the parcels only")
    a = ap.parse_args()
    if not a.root and not a.count:
        sys.exit("set --root or EO_PARCELS_DIR")

    dlat, dlon = a.half / M_PER_DEG_LAT, a.half / M_PER_DEG_LON
    south, north = math.floor((a.lat - dlat) * 1e4) / 1e4, math.ceil((a.lat + dlat) * 1e4) / 1e4
    west, east = math.floor((a.lon - dlon) * 1e4) / 1e4, math.ceil((a.lon + dlon) * 1e4) / 1e4
    ring = f"{west} {south}, {east} {south}, {east} {north}, {west} {north}, {west} {south}"
    where = f"intersects(shape, 'POLYGON(({ring}))') AND active = true"
    if a.count:
        url = f"{BASE}.json?" + urllib.parse.urlencode({"$select": "count(*)", "$where": where})
        print(json.loads(get(url))[0]["count"], "active parcels meet", [west, south, east, north])
        return
    params = {"$select": COLUMNS, "$where": where, "$order": "blklot", "$limit": "5000"}
    url = f"{BASE}.geojson?" + urllib.parse.urlencode(params)
    data = get(url)
    feats = json.loads(data).get("features", [])
    if len(feats) >= 5000:
        sys.exit("5000 features: the box is larger than this tool is meant for")

    out = os.path.join(a.root, a.name)
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "parcels.geojson.part"), "wb") as f:
        f.write(data)
    os.replace(os.path.join(out, "parcels.geojson.part"), os.path.join(out, "parcels.geojson"))
    manifest = {
        "source": "DataSF, Parcels - Active and Retired (acdm-wktn): recorded parcel geography, San Francisco Public Works basemap, Assessor block and lot",
        "dataset": DATASET,
        "url": url,
        "query": params,
        "license": "Open Data Commons Public Domain Dedication and License (PDDL 1.0), http://opendatacommons.org/licenses/pddl/1.0/",
        "attribution": "City and County of San Francisco (DataSF)",
        "fetched": datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat(),
        "area": {"lat": a.lat, "lon": a.lon, "half_m": a.half, "box_lonlat": [west, south, east, north]},
        "files": [{"file": "parcels.geojson", "features": len(feats), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}],
    }
    with open(os.path.join(out, "manifest.json"), "w") as f:
        json.dump(manifest, f, indent=1)
    print(f"wrote {len(feats)} parcels, {len(data):,} bytes, manifest {os.path.join(out, 'manifest.json')}")


if __name__ == "__main__":
    main()
