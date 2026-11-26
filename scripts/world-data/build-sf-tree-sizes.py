"""Build the street trees' trunk sizes and the allometry that sizes them.

Writes, from two public sources:

* public/data/sf/sf-tree-sizes.json.gz: every located record of the DataSF
  Street Tree List (tkzw-k3nq, PDDL 1.0) that is a tree: its position in the
  city's local metre frame (the sf-city.json projection: x east, z south,
  before the game's ground-plan scale), full species, trunk diameter (DBH,
  inches), plot size, site and planting date;
* public/data/sf/sf-tree-allometry.json: the USDA Forest Service Urban Tree
  Database equations (McPherson, van Doorn and Peper 2016, PSW-GTR-253;
  data RDS-2016-0005) that predict crown diameter and tree height from DBH,
  for the species the list's trees are matched to (species in the Northern
  California Coast region, else the species in another region, else the
  genus, else a stated family or broadleaf fallback), with each list species'
  match;
* a NOTICE beside each, with the sources, retrieval, digests and licences.

Python standard library only. Deterministic for the same upstream data:
rows sorted by tree id, gzip with mtime 0.

  python3 scripts/world-data/build-sf-tree-sizes.py [--raw DIR] [--from DIR]

--raw DIR keeps the downloaded responses (DIR must be empty or new);
--from DIR rebuilds from responses kept that way, without the network.
"""
import argparse
import csv
import datetime
import gzip
import hashlib
import io
import json
import math
import time
import zipfile
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "public/data/sf"
VERSION = "sf-tree-sizes-v1"
DATASET = "tkzw-k3nq"
PORTAL = "https://data.sf.gov"
META_URL = f"{PORTAL}/api/views/{DATASET}"
ROWS_URL = f"{PORTAL}/resource/{DATASET}.json"
FIELDS = "treeid,species,mapdbh,plotsize,planteddate,latitude,longitude,siteinfo,data_as_of"
PAGE = 50000
UTD_URL = "https://www.fs.usda.gov/rds/archive/products/RDS-2016-0005/RDS-2016-0005.zip"
UTD_SHA256 = "c0c888e2531ca58b673b7c18a30a01cbd33e90bd8b041fb3483e394bd8664faf"
UTD_CITATION = (
    "McPherson, E. Gregory; van Doorn, Natalie S.; Peper, Paula J. 2016. Urban tree database. "
    "Fort Collins, CO: Forest Service Research Data Archive. Updated 21 January 2020. "
    "https://doi.org/10.2737/RDS-2016-0005"
)
UTD_REPORT = (
    "McPherson, E. Gregory; van Doorn, Natalie S.; Peper, Paula J. 2016. Urban tree database and "
    "allometric equations. Gen. Tech. Rep. PSW-GTR-253. Albany, CA: U.S. Department of Agriculture, "
    "Forest Service, Pacific Southwest Research Station. https://doi.org/10.2737/PSW-GTR-253"
)
UTD_USE = (
    "These data were collected using funding from the U.S. Government and can be used without "
    "additional permissions or fees. (RDS-2016-0005 metadata, Use_Constraints.)"
)
USER_AGENT = "earth-online-data-build (python urllib)"
# The sf-city.json projection (meta.center, meta.projection).
CENTER_LAT, CENTER_LON = 37.778, -122.4194
M_PER_DEG_LAT, M_PER_DEG_LON = 110574.0, 87986.24747640654
# San Francisco's reference region in the database: Northern California Coast (Berkeley, CA).
HOME = "NoCalC"
# Other regions, nearest climate first (coastal California, then the inland and Pacific ones).
REGION_ORDER = [HOME, "SoCalC", "PacfNW", "InlVal", "SacVal", "InlEmp", "SWDsrt", "Tropic", "InterW",
                "TpIntW", "NMtnPr", "MidWst", "LoMidW", "NoEast", "Piedmt", "GulfCo", "CenFla"]
# Names the list and the database spell differently.
SYNONYMS = {
    "Platanus x hispanica": "Platanus x acerifolia",
    "Platanus acerifolia": "Platanus x acerifolia",
    "Platanus hispanica": "Platanus x acerifolia",
    # Brisbane box: the database keeps it under its older name.
    "Lophostemon confertus": "Tristaniopsis conferta",
    "Tristania conferta": "Tristaniopsis conferta",
}
# A database record whose name puts it in another genus than the tree's own: not a genus match
# for that name's genus (Tristaniopsis conferta is Lophostemon confertus, not a water gum).
TRUE_GENUS = {"Tristaniopsis conferta": "Lophostemon"}
# Myrtaceae genera with no record of their own (Tristaniopsis, Lophostemon, ...): the
# database's coastal broadleaf-evergreen Myrtaceae street tree of the same upright habit.
MYRTACEAE = {"Tristaniopsis", "Lophostemon", "Leptospermum", "Agonis", "Syzygium", "Eugenia",
             "Acca", "Feijoa", "Luma", "Ugni", "Corymbia", "Angophora", "Backhousia", "Kunzea"}
MYRTACEAE_FALLBACK = ("SoCalC", "Melaleuca quinquenervia")
# Palms and palm-like monocots: the game sizes a palm by its height only; not sized here.
PALMS = {"Phoenix", "Washingtonia", "Syagrus", "Trachycarpus", "Archontophoenix", "Butia", "Brahea",
         "Chamaerops", "Livistona", "Jubaea", "Howea", "Rhopalostylis", "Arecastrum", "Sabal",
         "Cordyline", "Dracaena", "Yucca"}
# Conifer genera with no record of their own: the database's coastal conifer in San Francisco's region.
CONIFERS = {"Cupressus", "Hesperocyparis", "Cupressocyparis", "Xanthocyparis", "Cedrus", "Araucaria", "Thuja",
            "Chamaecyparis", "Cryptomeria", "Callitris", "Abies", "Picea", "Sequoiadendron", "Metasequoia",
            "Taxodium", "Pseudotsuga", "Tsuga", "Calocedrus"}
CONIFER_FALLBACK = ("NoCalC", "Pinus radiata")
# Records that are not a standing tree (as the game's own tree export skips them).
NOT_TREES = ("planting site", "potential site", "stump", "paved over", "vacant")


def fetch(url, raw_dir, name, from_dir):
    if from_dir:
        return (from_dir / name).read_bytes()
    data = urlopen(Request(url, headers={"User-Agent": USER_AGENT}), timeout=180).read()
    if raw_dir:
        (raw_dir / name).write_bytes(data)
    return data


def scientific(species):
    """'Ulmus parvifolia 'True Green' :: Chinese Elm' -> ('Ulmus', 'Ulmus parvifolia')."""
    sci = species.split("::")[0].strip()
    words = []
    for w in sci.replace("‘", "'").replace("’", "'").split():
        if w.startswith("'") or w.startswith('"') or w.lower() in ("var.", "ssp.", "subsp.", "cv.", "spp.", "sp."):
            break
        words.append(w)
    if not words:
        return "", ""
    genus = words[0].capitalize()
    if len(words) >= 3 and words[1].lower() in ("x", "×"):
        name = f"{genus} x {words[2].lower()}"
    elif len(words) >= 2:
        name = f"{genus} {words[1].lower()}"
    else:
        name = genus
    return genus, SYNONYMS.get(name, name)


def num(v):
    try:
        f = float(v)
    except (TypeError, ValueError):
        return None
    return f if math.isfinite(f) else None


def load_utd(zbytes):
    z = zipfile.ZipFile(io.BytesIO(zbytes))
    coef_name = "Data/TS6_Growth_coefficients.csv"
    species_name = "Data/TS2_Regional_species_and_counts.csv"
    region_name = "Data/TS1_Regional_information.csv"
    coef_raw, species_raw, region_raw = (z.read(n) for n in (coef_name, species_name, region_name))
    regions = {r["Region Code"].strip(): r for r in csv.DictReader(io.StringIO(region_raw.decode("utf-8-sig")))}
    counts = {}
    for r in csv.DictReader(io.StringIO(species_raw.decode("utf-8-sig"))):
        key = (r["Region"].strip(), r["Scientific Name"].strip())
        counts[key] = (int(num(r.get("Grand Total")) or 0), r["Tree Type"].strip(), r["Common Name"].strip())
    eqs = {}
    for r in csv.DictReader(io.StringIO(coef_raw.decode("utf-8-sig"))):
        r = {k.strip(): (v.strip() if isinstance(v, str) else v) for k, v in r.items()}
        key = (r["Region"], r["Scientific Name"])
        comp = (r["Independent variable"], r["Predicts component"])
        e = eqs.setdefault(key, {"spCode": r["SpCode"]})
        term = {"eq": r["EqName"]}
        for c in "abcde":
            v = num(r[c])
            if v is not None:
                term[c] = v
        term["min"], term["max"] = num(r["Apps min"]), num(r["Apps max"])
        if comp == ("dbh", "crown dia"):
            e["crown"] = term
        elif comp == ("dbh", "tree ht"):
            e["height"] = term
        elif comp == ("age", "dbh"):
            e["ageDbh"] = term
            e["dbhCm"] = [num(r["Data min (from raw data)"]), num(r["Data max (from raw data)"])]
    usable = {k: v for k, v in eqs.items() if "crown" in v and "height" in v and "dbhCm" in v}
    digests = {n: hashlib.sha256(b).hexdigest() for n, b in ((coef_name, coef_raw), (species_name, species_raw), (region_name, region_raw))}
    return usable, counts, regions, digests


def match(genus, name, utd, counts):
    """The database record a list species is sized with: (key, match level) or (None, reason)."""
    if not genus:
        return None, "no-name"
    if genus in PALMS:
        return None, "palm"
    for reg in REGION_ORDER:
        if (reg, name) in utd:
            return (reg, name), f"species/{reg}"
    for reg in REGION_ORDER:
        same = sorted((k for k in utd if k[0] == reg and TRUE_GENUS.get(k[1], k[1].split()[0]) == genus), key=lambda k: (-counts.get(k, (0,))[0], k[1]))
        if same:
            return same[0], f"genus/{reg}"
    if genus in MYRTACEAE and MYRTACEAE_FALLBACK in utd:
        return MYRTACEAE_FALLBACK, "family-myrtaceae"
    if genus in CONIFERS and CONIFER_FALLBACK in utd:
        return CONIFER_FALLBACK, "conifer"
    return ("*", "broadleaf"), "broadleaf-median"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--raw", type=Path, help="keep the downloaded responses here (a new or empty folder)")
    ap.add_argument("--from", dest="from_dir", type=Path, help="rebuild from responses kept with --raw")
    a = ap.parse_args()
    if a.raw:
        a.raw.mkdir(parents=True, exist_ok=True)
        if any(a.raw.iterdir()):
            raise SystemExit(f"{a.raw} is not empty")
    retrieved = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    if a.from_dir:
        retrieved = (a.from_dir / "retrieved.txt").read_text().strip()
    elif a.raw:
        (a.raw / "retrieved.txt").write_text(retrieved + "\n")

    # --- DataSF street tree list -------------------------------------------------
    meta_raw = fetch(META_URL, a.raw, "tkzw-k3nq-metadata.json", a.from_dir)
    meta = json.loads(meta_raw)
    lic = meta.get("license") or {}
    assert meta.get("licenseId") == "PDDL", f"tkzw-k3nq licence changed: {meta.get('licenseId')} {lic}"
    rows, pages, page_digests = [], 0, hashlib.sha256()
    while True:
        q = urlencode({"$select": FIELDS, "$order": "treeid", "$limit": PAGE, "$offset": pages * PAGE})
        body = fetch(f"{ROWS_URL}?{q}", a.raw, f"tkzw-k3nq-rows-{pages:03d}.json", a.from_dir)
        page_digests.update(body)
        page = json.loads(body)
        rows.extend(page)
        pages += 1
        if len(page) < PAGE:
            break
        if not a.from_dir:
            time.sleep(1.0)
    as_of = sorted({r.get("data_as_of", "")[:10] for r in rows if r.get("data_as_of")})

    # --- USDA Urban Tree Database ----------------------------------------------------
    zbytes = fetch(UTD_URL, a.raw, "RDS-2016-0005.zip", a.from_dir)
    zsha = hashlib.sha256(zbytes).hexdigest()
    assert zsha == UTD_SHA256, f"RDS-2016-0005.zip digest {zsha} is not the archive's {UTD_SHA256}"
    utd, counts, regions, utd_digests = load_utd(zbytes)

    # --- the trees ------------------------------------------------------------------------
    kept, skipped = [], {"no location": 0, "not a tree": 0, "duplicate id": 0}
    seen = set()
    for r in rows:
        tid = int(num(r.get("treeid")) or -1)
        lat, lon = num(r.get("latitude")), num(r.get("longitude"))
        sp = (r.get("species") or "").strip()
        if lat is None or lon is None or not (37.6 < lat < 37.9 and -122.6 < lon < -122.3):
            skipped["no location"] += 1
            continue
        if any(sp.lower().startswith(p) for p in NOT_TREES):
            skipped["not a tree"] += 1
            continue
        if tid in seen:
            skipped["duplicate id"] += 1
            continue
        seen.add(tid)
        x = (lon - CENTER_LON) * M_PER_DEG_LON
        z = (CENTER_LAT - lat) * M_PER_DEG_LAT
        dbh = num(r.get("mapdbh"))
        dbh = dbh if dbh is not None and 0 < dbh <= 150 else None
        planted = (r.get("planteddate") or "")[:10]
        try:
            planted_i = int(datetime.date.fromisoformat(planted).strftime("%Y%m%d")) if planted else 0
        except ValueError:
            planted_i = 0
        kept.append((tid, round(x * 10), round(z * 10), sp, dbh, (r.get("plotsize") or "").strip(), (r.get("siteinfo") or "").strip(), planted_i))
    kept.sort()
    species = sorted({k[3] for k in kept})
    plots = sorted({k[5] for k in kept if k[5]})
    sites = sorted({k[6] for k in kept if k[6]})
    sp_ix = {s: i for i, s in enumerate(species)}
    plot_ix = {s: i for i, s in enumerate(plots)}
    site_ix = {s: i for i, s in enumerate(sites)}

    # --- species -> allometry ----------------------------------------------------------
    used, eq_ix, sp_eq, sp_match = [], {}, [], []
    broadleaf = sorted(k for k in utd if k[0] == HOME and counts.get(k, (0, ""))[1][:1] == "B")
    for s in species:
        genus, name = scientific(s)
        key, level = match(genus, name, utd, counts)
        if key is None:
            sp_eq.append(-1)
        elif key == ("*", "broadleaf"):
            sp_eq.append(-2)
        else:
            if key not in eq_ix:
                eq_ix[key] = len(used)
                used.append(key)
            sp_eq.append(eq_ix[key])
        sp_match.append(level)
    for key in broadleaf:
        if key not in eq_ix:
            eq_ix[key] = len(used)
            used.append(key)
    equations = []
    for key in used:
        reg, name = key
        e = utd[key]
        n, ttype, common = counts.get(key, (0, "", ""))
        city = regions.get(reg, {})
        equations.append({
            "key": f"{reg}/{name}", "region": reg, "city": f"{city.get('City', '')}, {city.get('State', '')}".strip(", "),
            "species": name, "common": common, "spCode": e["spCode"], "treeType": ttype, "n": n,
            "crown": e["crown"], "height": e["height"], "ageDbh": e.get("ageDbh"), "dbhCm": e["dbhCm"],
        })
    allometry = {
        "meta": {
            "version": "sf-tree-allometry-v1",
            "generator": "scripts/world-data/build-sf-tree-sizes.py",
            "source": "USDA Forest Service Urban Tree Database (RDS-2016-0005), Data/TS6_Growth_coefficients.csv",
            "citation": UTD_CITATION,
            "report": UTD_REPORT,
            "use": UTD_USE,
            "archive": {"url": UTD_URL, "sha256": zsha, "files": utd_digests},
            "units": "DBH in centimetres; crown diameter (crown) and total tree height (height) in metres; ageDbh: DBH (cm) from years since planting",
            "equations": "TS4_Growth_eqn_forms.csv: lin a+bx; quad a+bx+cx^2; cub a+bx+cx^2+dx^3; quart +ex^4; "
                         "loglogw1..4 exp(a+b ln(ln(x+1)) + w mse/2) and expow1..4 exp(a+bx + w mse/2) with mse = c "
                         "and w = 1, sqrt(x), x, x^2; prediction clamped to [min, max] (the equation's application range) "
                         "and DBH to dbhCm (the species' measured range)",
            "rule": f"A list species is sized with the database record of the same species in {HOME} (Berkeley, CA: "
                    "San Francisco's region), else the same species in the nearest other region (order: regionOrder), "
                    "else the genus (its most-measured species) in that order, else for a Myrtaceae genus "
                    f"{MYRTACEAE_FALLBACK[1]} ({MYRTACEAE_FALLBACK[0]}), for a conifer genus {CONIFER_FALLBACK[1]} "
                    f"({CONIFER_FALLBACK[0]}), else the median over the {HOME} broadleaf species (broadleaf); palms "
                    "and palm-like monocots are not sized.",
            "regionOrder": REGION_ORDER,
            "synonyms": SYNONYMS,
            "trueGenus": TRUE_GENUS,
        },
        "equations": equations,
        "broadleaf": [eq_ix[k] for k in broadleaf],
    }
    sizes = {
        "meta": {
            "version": VERSION,
            "generator": "scripts/world-data/build-sf-tree-sizes.py",
            "frame": {"centerLat": CENTER_LAT, "centerLon": CENTER_LON, "metersPerDegreeLat": M_PER_DEG_LAT,
                      "metersPerDegreeLon": M_PER_DEG_LON, "axes": "x east, z south, decimetres; the sf-city.json frame, before the ground-plan scale"},
            "source": {"dataset": DATASET, "name": meta.get("name"), "url": f"{PORTAL}/d/{DATASET}", "licence": "PDDL-1.0",
                       "dataAsOf": as_of, "rowsUpdatedAt": meta.get("rowsUpdatedAt"), "attribution": meta.get("attribution")},
            "counts": {"rows": len(rows), "kept": len(kept), **{f"skipped {k}": v for k, v in skipped.items()},
                       "withDbh": sum(1 for k in kept if k[4] is not None), "withPlanted": sum(1 for k in kept if k[7])},
            "columns": "treeid; x, z (dm); sp (species index); dbh (inches x10, -1 none); plot (index, -1 none); site (index, -1 none); planted (yyyymmdd, 0 none)",
            "allometry": "sf-tree-allometry.json: speciesAllometry is the equation index (-1 not sized, -2 the broadleaf median)",
        },
        "species": species,
        "speciesAllometry": sp_eq,
        "speciesMatch": sp_match,
        "plots": plots,
        "sites": sites,
        "treeid": [k[0] for k in kept],
        "x": [k[1] for k in kept],
        "z": [k[2] for k in kept],
        "sp": [sp_ix[k[3]] for k in kept],
        "dbh": [round(k[4] * 10) if k[4] is not None else -1 for k in kept],
        "plot": [plot_ix[k[5]] if k[5] else -1 for k in kept],
        "site": [site_ix[k[6]] if k[6] else -1 for k in kept],
        "planted": [k[7] for k in kept],
    }
    bio = io.BytesIO()
    with gzip.GzipFile(fileobj=bio, mode="wb", mtime=0, compresslevel=9) as g:
        g.write(json.dumps(sizes, separators=(",", ":"), ensure_ascii=False).encode())
    sizes_path = OUT / "sf-tree-sizes.json.gz"
    sizes_path.write_bytes(bio.getvalue())
    allo_path = OUT / "sf-tree-allometry.json"
    allo_path.write_text(json.dumps(allometry, indent=1, ensure_ascii=False) + "\n")

    # --- notices ---------------------------------------------------------------------------
    by_level = {}
    for s, lvl in zip(species, sp_match):
        by_level.setdefault(lvl.split("/")[0], [0, 0])[0] += 1
    trees_by_level = {}
    for k in kept:
        lvl = sp_match[sp_ix[k[3]]].split("/")[0]
        trees_by_level[lvl] = trees_by_level.get(lvl, 0) + 1
    q = f"$select={FIELDS}&$order=treeid&$limit={PAGE}"
    (OUT / "sf-tree-sizes.NOTICE.txt").write_text(
        "sf-tree-sizes.json.gz - San Francisco street trees: position, species, trunk diameter, plot, site, planting date\n\n"
        f"Source: DataSF \"{meta.get('name')}\" ({DATASET}), {meta.get('attribution')}.\n"
        f"  {PORTAL}/d/{DATASET}\n"
        "License: Open Data Commons Public Domain Dedication and License (PDDL) 1.0\n"
        "         http://opendatacommons.org/licenses/pddl/1.0/\n"
        f"Retrieved: {retrieved} from {ROWS_URL}?{q}&$offset=0..{(pages - 1) * PAGE} ({pages} pages)\n"
        f"           and the metadata {META_URL} (rowsUpdatedAt {meta.get('rowsUpdatedAt')}).\n"
        f"Upstream data as of: {as_of[0]} to {as_of[-1]} (the rows' data_as_of)\n"
        f"Retrieved rows SHA-256 (the pages in order): {page_digests.hexdigest()}\n"
        f"Metadata SHA-256: {hashlib.sha256(meta_raw).hexdigest()}\n"
        "Produced by:   scripts/world-data/build-sf-tree-sizes.py (" + VERSION + ")\n"
        f"Output sha256: {hashlib.sha256(sizes_path.read_bytes()).hexdigest()} (sf-tree-sizes.json.gz)\n\n"
        f"Kept {len(kept)} of {len(rows)} rows: " + ", ".join(f"{v} {k}" for k, v in skipped.items() if v) + " skipped.\n"
        f"With a DBH: {sizes['meta']['counts']['withDbh']}; with a planting date: {sizes['meta']['counts']['withPlanted']}.\n"
        "Positions: the list's latitude and longitude in the sf-city.json frame (centre 37.778, -122.4194; 110574 m per\n"
        "degree of latitude, 87986.24747640654 m per degree of longitude; x east, z south), in decimetres.\n"
        "No caretaker, planter or address fields are carried. The DBH is as the list records it, which for\n"
        "many trees is not a recent measurement (a tree planted in 2007 may still read 3 in).\n")
    (OUT / "sf-tree-allometry.NOTICE.txt").write_text(
        "sf-tree-allometry.json - urban tree allometry: crown diameter and tree height from trunk diameter (DBH)\n\n"
        f"Source: {UTD_CITATION}\n"
        f"Report: {UTD_REPORT}\n"
        f"Use: {UTD_USE}\n"
        "      U.S. Government work; credit the citation above.\n"
        f"Retrieved: {retrieved} from {UTD_URL}\n"
        f"Archive SHA-256: {zsha} (as the archive's catalogue page states)\n"
        + "".join(f"  {n} SHA-256 {d}\n" for n, d in sorted(utd_digests.items()))
        + "Produced by:   scripts/world-data/build-sf-tree-sizes.py\n"
        f"Output sha256: {hashlib.sha256(allo_path.read_bytes()).hexdigest()} (sf-tree-allometry.json)\n\n"
        f"Equations kept: {len(equations)} (the records the list's species match, and the {HOME} broadleaf species for the median).\n"
        f"Rule: {allometry['meta']['rule']}\n"
        "List species by match: " + ", ".join(f"{k} {v[0]}" for k, v in sorted(by_level.items())) + "\n"
        "Trees by match: " + ", ".join(f"{k} {v}" for k, v in sorted(trees_by_level.items())) + "\n")
    print(f"kept {len(kept)} trees of {len(rows)} rows; {len(species)} species; {len(equations)} equations")
    print("trees by match:", dict(sorted(trees_by_level.items())))


if __name__ == "__main__":
    main()
