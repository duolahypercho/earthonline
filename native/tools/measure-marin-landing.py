#!/usr/bin/env python3
"""Bake the Marin abutment's ground colours from the pinned USDA NAIP tiles.

Run with Python 3 and Pillow. Coordinates are the unscaled city survey plan.
The colour field dresses existing terrain; the raised deck has no image on
the ground beneath it. All roads and collisions retain their existing extent.
"""
import gzip
import hashlib
import json
import math
import os
import struct
from urllib.request import urlopen
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

NATIVE = Path(__file__).resolve().parents[1]
REPO = NATIVE.parent
SOURCE = Path(os.environ.get("EO_SURVEY_ROOT") or REPO / "external/survey")
OUT = NATIVE / "assets/marin-landing.bin"
WEST, NORTH, EAST, SOUTH, STEP = -5936, -6208, -5136, -5632, 2


def bake_images(grid, origin, proj, load_tile, entries, gate, calibrated, source):
    """Two registered strips in one surface map, a headland overview and a landing inset."""
    config = json.loads((NATIVE / "assets/marin-image-footprints.json").read_text())
    width = config["width"]
    rows = width // len(config["bands"])
    tile_size = grid["tile"]
    atlas = Image.new("RGBA", (width, width))
    used = set()
    south = gate["wayEnds"]["south"]
    far = gate["towerLegs"]["north"]
    carriageways = [list(zip(way[::2], way[1::2])) for way in gate["carriageways"]]
    for band, spec in enumerate(config["bands"]):
        west, north, step = spec["west"], spec["north"], spec["metresPerTexel"]
        # A sample belongs to its pixel centre in the city frame. The service
        # supplies angularly square pixels, cropped separately in every row.
        cols = []
        for i in range(width):
            lon = origin["lon"] + (west + (i + 0.5) * step) / proj["metersPerDegreeLon"]
            col = math.floor((lon - grid["west"]) / grid["dlon"])
            cols.append(divmod(col, tile_size))
        strip = Image.new("RGB", (width, rows))
        for j in range(rows):
            lat = origin["lat"] - (north + (j + 0.5) * step) / proj["metersPerDegreeLat"]
            row = math.floor((grid["north"] - lat) / (tile_size * grid["dlat"]))
            centre = grid["north"] - (row + 0.5) * tile_size * grid["dlat"]
            py = math.floor(tile_size / 2 + (centre - lat) / grid["dlon"])
            line = bytearray()
            sources = {}
            for c in {col for col, _ in cols}:
                path = f"A1/rgb/r{row:03d}_c{c:03d}.jpg"
                used.add(path)
                sources[c] = load_tile(path).crop((0, py, tile_size, py + 1)).tobytes()
            for col, px in cols:
                line.extend(sources[col][px * 3:px * 3 + 3])
            strip.paste(Image.frombytes("RGB", (width, 1), bytes(line)), (0, j))
        strip = strip.point(sum(calibrated, []))
        # No aerial image of the suspended deck or its approaches on the
        # terrain under the real mesh. End the cut perpendicular to the road.
        mask = Image.new("L", (width, rows), 255)
        draw = ImageDraw.Draw(mask)
        def pixel(x, z):
            return ((x - west) / step, (z - north) / step)
        for way in carriageways:
            draw.line([pixel(x,z) for x,z in way], 0,
                      width=math.ceil(gate["dimensions"]["widthM"] / step))
        mask = mask.filter(ImageFilter.GaussianBlur(2 / step))
        # Feather the image border. Keep the coverage north of the strait.
        mx = [min(1, (min(i + 0.5, width - i - 0.5) * step) / 24) for i in range(width)]
        alpha = bytearray(mask.tobytes())
        mid_z = (south[1] + (far["east"][1] + far["west"][1]) / 2) / 2
        for j in range(rows):
            z = north + (j + 0.5) * step
            mz = min(1, min(j + 0.5, rows - j - 0.5) * step / 24, max(0, (mid_z - z)/24))
            for i, wx in enumerate(mx):
                k = j * width + i
                alpha[k] = round(alpha[k] * min(wx, mz))
        strip.putalpha(Image.frombytes("L", (width, rows), bytes(alpha)))
        atlas.paste(strip, (0, band * rows))
    out = NATIVE / "assets/marin-headlands.png"
    atlas.save(out, compress_level=9)
    notice = ["Marin Headlands registered ground reflectance", source["attribution"], source["licence"],
              "Coordinates: marin-image-footprints.json, native city survey metres.",
              "Rebuild: python3 native/tools/measure-marin-landing.py",
              "Output SHA-256: " + hashlib.sha256(out.read_bytes()).hexdigest(), "", "Verified source tiles:"]
    for path in sorted(used):
        notice.extend((path + "  " + entries[path]["sha256"], entries[path]["url"]))
    out.with_suffix(".NOTICE.txt").write_text("\n".join(notice) + "\n")


def bake_overview(origin, projection, lut, gate):
    """Project the wider Mercator export onto the same city grid at 11 m."""
    spec = json.loads((NATIVE / "assets/marin-overview-source.json").read_text())
    path = SOURCE / "marin-headlands/overview.jpg"
    if not path.is_file():
        path.parent.mkdir(parents=True, exist_ok=True)
        with urlopen(spec["url"], timeout=60) as stream:
            raw = stream.read()
        if hashlib.sha256(raw).hexdigest() != spec["sha256"]:
            raise ValueError("USGS overview differs from the pinned photograph")
        path.write_bytes(raw)
    if hashlib.sha256(path.read_bytes()).hexdigest() != spec["sha256"]:
        raise ValueError("Unverified Marin overview")
    width, height = spec["width"] // 2, spec["height"] // 2
    step = 2 * spec["metresPerTexel"]
    ext = spec["extent3857"]
    with Image.open(path) as src:
        assert src.size == (spec["width"], spec["height"])
        src = src.convert("RGB")
        # Longitude is linear in Mercator. Reproject latitude at every row
        # boundary, averaging the source footprint to remove subpixel traffic.
        out = Image.new("RGB", (width, height))
        for row in range(height):
            def image_row(j):
                lat = origin["lat"] - (spec["north"] + j * step) / projection["metersPerDegreeLat"]
                y = 6378137 * math.asinh(math.tan(math.radians(lat)))
                return max(0., min(src.height, src.height * (ext["ymax"] - y) / (ext["ymax"] - ext["ymin"])))
            strip = src.resize((width, 1), Image.Resampling.BOX,
                               box=(0., image_row(row), float(src.width), image_row(row + 1)))
            out.paste(strip, (0, row))
    out = out.point(sum(lut, []))
    mask = Image.new("L", (width, height), 255)
    draw = ImageDraw.Draw(mask)
    for way in gate["carriageways"]:
        draw.line([((x - spec["west"])/step, (z - spec["north"])/step)
                   for x,z in zip(way[::2], way[1::2])], fill=0,
                  width=math.ceil(gate["dimensions"]["widthM"]/step))
    mask = mask.filter(ImageFilter.GaussianBlur(0.6))
    # Feather twelve texels into the neighbouring survey or regional ground.
    weights = bytearray(mask.tobytes())
    for y in range(height):
        for x in range(width):
            i = y * width + x
            weights[i] = round(weights[i] * min(1, (min(x, y, width-1-x, height-1-y) + .5)/12))
    out.putalpha(Image.frombytes("L", (width, height), bytes(weights)))
    out_path = NATIVE / "assets/marin-overview.png"
    out.save(out_path, compress_level=9)
    notice = ["Marin Headlands distant terrain colour", spec["attribution"], spec["licence"],
              spec["url"], "Source SHA-256: " + spec["sha256"],
              "Image boundary and resolution: marin-overview-source.json, two source pixels per output pixel.",
              "Rebuild: python3 native/tools/measure-marin-landing.py",
              "Output SHA-256: " + hashlib.sha256(out_path.read_bytes()).hexdigest()]
    out_path.with_suffix(".NOTICE.txt").write_text("\n".join(notice) + "\n")


def bake():
    with gzip.open(REPO / "public/data/sf/sf-city.json.gz") as stream:
        meta = json.load(stream)["meta"]
    origin, proj = meta["center"], meta["projection"]
    manifest = json.loads((NATIVE / "assets/survey-manifest.json").read_text())
    source = next(s for s in manifest["sets"] if s["group"] == "A1")
    grid = source["grid"]
    entries = {f["path"]: f for f in source["files"]}
    n = grid["tile"]
    images = {}

    raw_tiles = {}
    def load_tile(path):
        if path not in raw_tiles:
            raw = (SOURCE / path).read_bytes()
            if hashlib.sha256(raw).hexdigest() != entries[path]["sha256"]:
                raise ValueError(f"Unverified NAIP tile {path}")
            with Image.open(SOURCE / path) as im:
                raw_tiles[path] = im.convert("RGB")
        return raw_tiles[path]

    def sample(x, z):
        lon = origin["lon"] + x / proj["metersPerDegreeLon"]
        lat = origin["lat"] - z / proj["metersPerDegreeLat"]
        col = math.floor((lon - grid["west"]) / grid["dlon"])
        row = math.floor((grid["north"] - lat) / (n * grid["dlat"]))
        # The service delivered square *angular* pixels about each requested
        # box centre. See eo_farshore::survey::naip::Grid::pixel.
        centre = grid["north"] - (row + 0.5) * n * grid["dlat"]
        py = math.floor(n / 2 + (centre - lat) / grid["dlon"])
        path = f"A1/rgb/r{row:03d}_c{col // n:03d}.jpg"
        if path not in images:
            images[path] = load_tile(path).filter(ImageFilter.MedianFilter(5)).filter(ImageFilter.BoxBlur(1))
        return images[path].getpixel((col % n, py))

    gate = json.loads((NATIVE / "eo-landmarks/data/landmarks-js.json").read_text())["goldenGate"]
    way = gate["carriageways"][1]
    pts = list(zip(way[::2], way[1::2]))
    end = gate["wayEnds"]["north"]
    def tower_centre(legs):
        return tuple((a + b) / 2 for a, b in zip(legs["west"], legs["east"]))

    south = tower_centre(gate["towerLegs"]["south"])
    north = tower_centre(gate["towerLegs"]["north"])
    span = math.dist(south, north)
    ax, az = ((b - a) / span for a, b in zip(south, north))
    segments = [
        (a, b) for a, b in zip(pts, pts[1:]) if min(a[1], b[1]) < SOUTH + 50
    ]
    half_width = gate["dimensions"]["widthM"] / 2 + min(
        math.dist(end, p) for p in (pts[0], pts[-1])
    )

    def distance(x, z, a, b):
        dx, dz = b[0] - a[0], b[1] - a[1]
        t = ((x - a[0]) * dx + (z - a[1]) * dz) / (dx * dx + dz * dz)
        t = max(0, min(1, t))
        return math.hypot(x - a[0] - t * dx, z - a[1] - t * dz)

    width = (EAST - WEST) // STEP + 1
    height = (SOUTH - NORTH) // STEP + 1
    buf = bytearray(struct.pack("<4siiIII", b"MRN1", WEST, NORTH, STEP, width, height))
    # A neutral patch of clear US 101 pavement, registered in the same aerial
    # tile, sets its exposure against the city's road material (eo-render
    # cc0.rs, ASPHALT_ALBEDO). Store reflectance colours for the game's light.
    reference = [sample(WEST + x, NORTH + z) for x in range(362,371,2) for z in range(76,83,2)]
    channel_mean = [sum(pixel[i] for pixel in reference) / len(reference) for i in range(3)]
    linear = lambda x: ((x / 255 + 0.055) / 1.055) ** 2.4 if x > 10 else x / 3294.6
    def encode(x):
        return round(255 * max(0, min(1, 1.055*x**(1/2.4)-0.055 if x>0.0031308 else 12.92*x)))
    calibrated = [[encode(linear(v)*0.10/linear(mean)) for v in range(256)] for mean in channel_mean]
    for j in range(height):
        z = NORTH + j * STEP
        for i in range(width):
            x = WEST + i * STEP
            edge = min(x - WEST, EAST - x, z - NORTH, SOUTH - z) / 32
            under_bridge = (x - end[0]) * ax + (z - end[1]) * az < 0
            weight = edge
            if under_bridge:
                corridor = min(distance(x, z, a, b) for a, b in segments)
                weight = min(edge, (corridor - half_width) / 12)
            rgb = sample(x, z)
            buf.extend(calibrated[i][v] for i,v in enumerate(rgb))
            buf.append(round(255 * max(0, min(1, weight))))
    OUT.write_bytes(buf)
    notice = ["Marin landing ground-colour survey", source["attribution"], source["licence"],
              "Source grid and individual tile bounds: native/assets/survey-manifest.json, group A1.",
              "Rebuild: python3 native/tools/measure-marin-landing.py",
              "Output SHA-256: " + hashlib.sha256(buf).hexdigest(), "", "Source tiles:"]
    for path in sorted(images):
        notice.extend((path + "  " + entries[path]["sha256"], entries[path]["url"]))
    OUT.with_suffix(".NOTICE.txt").write_text("\n".join(notice) + "\n")
    bake_overview(origin, proj, calibrated, gate)
    bake_images(grid, origin, proj, load_tile, entries, gate, calibrated, source)


if __name__ == "__main__":
    bake()
