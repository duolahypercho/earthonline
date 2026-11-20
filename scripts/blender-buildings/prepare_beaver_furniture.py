"""Prepare portable material pages for the sitting-room rug and oiled oak furniture."""

import argparse
import hashlib
import io
import json
import urllib.request
from pathlib import Path

import numpy as np
from PIL import Image

root = Path(__file__).resolve().parents[2]
out = root / "public/assets/buildings/beaver-terrace/finishes/patina/furniture"
parser = argparse.ArgumentParser()
parser.add_argument("source_cache", type=Path)
cache = parser.parse_args().source_cache.resolve()
if cache.is_relative_to(root):
    raise ValueError("Photographic inputs belong outside the repository.")
out.mkdir(parents=True, exist_ok=True)
cache.mkdir(parents=True, exist_ok=True)


def read(url, digest):
    file = cache / url.rsplit("/", 1)[1]
    data = file.read_bytes() if file.is_file() else urllib.request.urlopen(url, timeout=50).read()
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError(f"Photographic source changed: {url}")
    if not file.exists():
        file.write_bytes(data)
    return Image.open(io.BytesIO(data)).convert("RGB")


def publish(name, pages, metadata):
    atlas = Image.new("RGBA", (512, 1536))
    for i, page in enumerate(pages):
        atlas.paste(page.convert("RGBA"), (0, 512 * i))
    path = out / (name + ".png")
    atlas.save(path, compress_level=9)
    raw = path.read_bytes()
    metadata.update(
        url="/" + path.relative_to(root / "public").as_posix(),
        sha256=hashlib.sha256(raw).hexdigest(),
        bytes=len(raw),
        build="scripts/blender-buildings/prepare_beaver_furniture.py",
    )
    path.with_suffix(".provenance.json").write_text(json.dumps(metadata, indent=2) + "\n")


url = "https://images.metmuseum.org/CRDImages/is/original/DP220897.jpg"
digest = "2dd2e6ed8622458e0c6cae79055eb8d5c2ace7c6f20af708cc9852e6430f7859"
plate = read(url, digest)
# Trace the inner edge of the four bound corners so no museum backing is sampled.
corners = (175, 135, 86, 3768, 2686, 3750, 2691, 279)
rug = plate.transform((768, 1080), Image.Transform.QUAD, corners, Image.Resampling.BICUBIC).resize(
    (512, 512), Image.Resampling.LANCZOS
)
publish(
    "wool-carpet",
    [rug, Image.new("RGB", (512, 512), (128, 128, 255)), Image.new("RGB", (512, 512), (252, 240, 0))],
    dict(
        source="https://www.metmuseum.org/art/collection/search/452588",
        license="CC0-1.0",
        license_url="https://www.metmuseum.org/about-the-met/policies-and-documents/open-access",
        attribution="The Metropolitan Museum of Art. Carpet, attributed to Turkey, first half of the 19th century. Bequest of Joseph V. McMullan, 1973, 1974.149.36.",
        inputs=[dict(role="recto photograph", url=url, sha256=digest)],
        families=["woven-carpet"],
        metres_per_tile=1.0,
        normal_strength=0.0,
        paint_mix=0.0,
    ),
)
wood = json.loads((out.parent / "painted-wood.provenance.json").read_text())
pages = []
inputs = []
for role in ("albedo", "normal", "rough"):
    entry = next(f for f in wood["inputs"] if f["role"] == role)
    inputs.append({k: entry[k] for k in ("role", "url", "sha256")})
    im = read(entry["url"], entry["sha256"])
    # Sample one board. Flip its tangent vectors on reflection and keep the timber's pores.
    a = np.asarray(im.crop((112, 279, 905, 317)).resize((256, 256), Image.Resampling.LANCZOS)).copy()
    if role == "albedo":
        l = np.asarray(Image.fromarray(a).convert("L"), dtype=float)
        a = np.uint8(np.clip(222 + (l - np.median(l)) * 0.73, 0, 255))[:, :, None].repeat(3, 2)
    if role == "rough":
        a = np.stack(
            [np.full(a.shape[:2], 250), np.uint8(118 + 0.23 * a[:, :, 0]), np.zeros(a.shape[:2])],
            axis=-1,
        ).astype("uint8")
    b = a[:, ::-1].copy()
    if role == "normal":
        b[:, :, 0] = 255 - b[:, :, 0]
    row = np.concatenate([a, b], axis=1)
    other = row[::-1].copy()
    if role == "normal":
        other[:, :, 1] = 255 - other[:, :, 1]
    pages.append(Image.fromarray(np.concatenate([row, other], axis=0)))
publish(
    "joinery",
    pages,
    dict(
        source=wood["source"],
        license="CC0-1.0",
        license_url="https://polyhaven.com/license",
        attribution=wood["attribution"],
        inputs=inputs,
        families=["furniture-oak"],
        metres_per_tile=1.15,
        normal_strength=0.29,
        paint_mix=0.88,
    ),
)
