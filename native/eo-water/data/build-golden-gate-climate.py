#!/usr/bin/env python3
"""Build golden-gate-climate.json (and its NOTICE) from NOAA text data.

Inputs (plain-text JSON from the NOAA CO-OPS API, saved into SRC_DIR):

  wind-9414290-2022.json, -2023.json, -2024.json
      https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=wind
      &station=9414290&begin_date=<Y>0101&end_date=<Y>1231&time_zone=lst
      &units=metric&interval=h&format=json
  harcon-SFB1203-bin18.json
      https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/SFB1203/harcon.json?units=metric&bin=18

Usage: python3 build-golden-gate-climate.py SRC_DIR
Writes golden-gate-climate.json and golden-gate-climate.NOTICE.txt beside this script.
Standard library only.
"""
import hashlib, json, math, os, sys, collections, datetime

HERE = os.path.dirname(os.path.abspath(__file__))
YEARS = (2022, 2023, 2024)
# constituents kept for the current (amplitude at least 2 cm/s at bin 18)
KEEP = ("M2", "S2", "N2", "K2", "L2", "K1", "O1", "P1", "Q1", "J1", "OO1", "M4", "MS4", "MN4", "M6", "MK3", "2MK3")


def sha(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()


def main(src):
    sources = []
    speed = collections.defaultdict(list)
    ue = collections.defaultdict(float)
    un = collections.defaultdict(float)
    for y in YEARS:
        p = os.path.join(src, f"wind-9414290-{y}.json")
        sources.append((os.path.basename(p), sha(p)))
        for r in json.load(open(p))["data"]:
            try:
                s, d = float(r["s"]), float(r["d"])
            except (KeyError, ValueError):
                continue
            mo, hr = int(r["t"][5:7]), int(r["t"][11:13])
            speed[(mo, hr)].append(s)
            ue[(mo, hr)] += s * math.sin(math.radians(d))
            un[(mo, hr)] += s * math.cos(math.radians(d))
    table = [[round(sum(speed[(m, h)]) / len(speed[(m, h)]), 2) for h in range(24)] for m in range(1, 13)]
    count = [[len(speed[(m, h)]) for h in range(24)] for m in range(1, 13)]
    dirs = [[round(math.degrees(math.atan2(ue[(m, h)], un[(m, h)])) % 360.0, 1) for h in range(24)] for m in range(1, 13)]
    hp = os.path.join(src, "harcon-SFB1203-bin18.json")
    sources.append((os.path.basename(hp), sha(hp)))
    hc = json.load(open(hp))["HarmonicConstituents"]
    cons = []
    for c in hc:
        if c["constituentName"] in KEEP:
            cons.append({"name": c["constituentName"], "amp_cm_s": c["majorAmplitude"], "phase_gmt_deg": c["majorPhaseGMT"]})
    out = {
        "wind": {
            "station": "NOAA CO-OPS 9414290 San Francisco, CA (37.8063 N, 122.4659 W)",
            "years": list(YEARS),
            "time": "local standard time (UTC-8), hour of the day 0..23",
            "unit": "m/s, mean of the hourly records",
            "mean_speed": table,
            "records": count,
            "vector_mean_from_deg": dirs,
        },
        "current": {
            "station": "NOAA CO-OPS SFB1203 Golden Gate Bridge, 0.46 nm E of (37.82007 N, 122.473 W), bin 18",
            "bin_depth_m": hc[0]["binDepth"],
            "major_axis_azimuth_deg": hc[0]["azi"],
            "mean_cm_s": hc[0]["majorMeanSpeed"],
            "unit": "cm/s along the major axis (flood toward the azimuth), Greenwich phase lags",
            "constituents": cons,
        },
    }
    jp = os.path.join(HERE, "golden-gate-climate.json")
    with open(jp, "w") as f:
        json.dump(out, f, indent=1)
        f.write("\n")
    today = datetime.date.today().isoformat()
    lines = [
        "golden-gate-climate.json - Golden Gate wind climatology and tidal-current constants",
        "",
        "Source: NOAA National Ocean Service, Center for Operational Oceanographic Products",
        "and Services (CO-OPS): station 9414290 San Francisco, CA, hourly wind (2022-2024),",
        "and current station SFB1203 Golden Gate Bridge, harmonic constants of bin 18 (9.1 m).",
        "https://tidesandcurrents.noaa.gov/ , API https://api.tidesandcurrents.noaa.gov/",
        "Licence: public domain (a work of the United States Government, 17 U.S.C. 105).",
        f"Retrieved: {today}",
    ]
    for name, d in sources:
        lines.append(f"Source sha256 {name}: {d}")
    lines += [
        "Produced by:   native/eo-water/data/build-golden-gate-climate.py (version 1)",
        f"Output sha256: {sha(jp)}",
        "",
        "Kept: the mean hourly wind speed and its vector-mean direction per month and hour",
        "of the day (local standard time) over the three years; the current's major-axis",
        "amplitude and Greenwich phase for the constituents of at least 2 cm/s, and its",
        "mean (residual) speed along the major axis.",
        "The wind is measured at the shore station, not over the open strait.",
    ]
    with open(os.path.join(HERE, "golden-gate-climate.NOTICE.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main(sys.argv[1])
