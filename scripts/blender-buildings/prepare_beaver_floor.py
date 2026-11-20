"""Make a board-aligned floor material from three CC0 photographic maps.
Pass a source cache directory outside the checkout. Each download is pinned.
"""
import argparse
import hashlib
import io
import json
import urllib.request
from pathlib import Path

from PIL import Image, ImageOps

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "public/assets/buildings/beaver-terrace/finishes"
parser = argparse.ArgumentParser()
parser.add_argument("source_cache", type=Path)
cache = parser.parse_args().source_cache.resolve()
if cache.is_relative_to(ROOT):
    raise ValueError("Original texture plates belong outside the checkout.")
cache.mkdir(parents=True, exist_ok=True)
OUT.mkdir(parents=True, exist_ok=True)
hashes = {
    "diff": "db386853009b92b9edd7255b35c9f7b0d4b7de16837a7a9da8422148754bc758",
    "nor_gl": "f428496e5d6c726d17d9ecea29c0acd8660348630acf791cea7bdd7f3c6e262b",
    "rough": "9d7492db1197f1386466dfd7dd43f692446166c1433ddeebf66c10e8b226a6d0",
}
# Select eight single boards inside the scan, excluding their joints. Geometry
# supplies the real floor seams. All three maps get identical cuts and orientation.
strips = [(44,108),(187,251),(278,342),(368,432),(441,505),(601,665),(737,801),(836,900)]
atlas = Image.new("RGBA", (512,1536))
sources = []
for page, (kind, digest) in enumerate(hashes.items()):
    name = f"wood_floor_worn_{kind}_1k.jpg"
    path = cache / name
    url = "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/wood_floor_worn/" + name
    raw = path.read_bytes() if path.exists() else urllib.request.urlopen(
        urllib.request.Request(url,headers={"User-Agent":"EarthOnline asset pipeline"}),timeout=60).read()
    if hashlib.sha256(raw).hexdigest() != digest:
        raise ValueError(f"Source checksum mismatch for {name}")
    if not path.exists():
        path.write_bytes(raw)
    image = Image.open(io.BytesIO(raw)).convert("RGB")
    for row,(y0,y1) in enumerate(strips):
        board = image.crop((0,y0,1024,y1)).resize((512,64),Image.Resampling.LANCZOS)
        if kind == "diff":
            # A pale oil finish preserves the photographed pores and rays,
            # while lifting knots and reducing the orange varnish cast.
            wash = ImageOps.colorize(ImageOps.grayscale(board), "#947e66", "#d4c1a4",
                                     blackpoint=42, whitepoint=207)
            board = Image.blend(board, wash, 0.77)
        if kind == "rough":
            board = Image.merge("RGB",(Image.new("L",board.size,255),board.getchannel("R"),Image.new("L",board.size,0)))
        atlas.paste(board,(0,page*512+row*64))
    sources.append({"url":url,"sha256":digest})
path = OUT / "floorboards.png"
atlas.save(path,compress_level=9)
raw = path.read_bytes()
(OUT/"floorboards.provenance.json").write_text(json.dumps({
    "source":"https://polyhaven.com/a/wood_floor_worn","attribution":"Dimitrios Savva / Poly Haven",
    "license":"CC0-1.0","license_url":"https://polyhaven.com/license","inputs":sources,
    "build":"scripts/blender-buildings/prepare_beaver_floor.py",
    "treatment":"Grain strips with a pale oil colour treatment and the original normal and roughness",
    "url":"/"+path.relative_to(ROOT/"public").as_posix(),"sha256":hashlib.sha256(raw).hexdigest(),"bytes":len(raw)
},indent=2)+"\n")
