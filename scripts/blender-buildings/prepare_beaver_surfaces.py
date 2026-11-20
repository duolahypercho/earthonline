"""Build the house's portable, scan-derived diffuse, tangent normal and ORM pages."""

import argparse
import hashlib
import io
import json
import urllib.request
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina"
parser = argparse.ArgumentParser()
parser.add_argument("source_cache", type=Path)
cache = parser.parse_args().source_cache.resolve()
if cache.is_relative_to(ROOT):
    raise ValueError("Keep original photographic plates outside the checkout.")
cache.mkdir(parents=True, exist_ok=True)
OUT.mkdir(parents=True, exist_ok=True)
manifest = json.loads((ROOT / "native/assets/cc0-manifest.json").read_text())
source = {a["id"]: a for a in manifest["assets"]}
source.update(
    {
        "white_stucco_02": {
            "authors": ["Dimitrios Savva"],
            "page": "https://polyhaven.com/a/white_stucco_02",
            "files": [
                {
                    "role": "albedo",
                    "url": "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/white_stucco_02/white_stucco_02_diff_2k.jpg",
                    "sha256": "794a309cd768446958e7344d058a35626f507fe132fe9160bb4d50434e2e892c",
                },
                {
                    "role": "normal",
                    "url": "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/white_stucco_02/white_stucco_02_nor_gl_2k.jpg",
                    "sha256": "9d7f80ba4d49da2b6d6a66953e597ff14489d8c50a626cb9d95006878e719c06",
                },
                {
                    "role": "arm",
                    "url": "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/white_stucco_02/white_stucco_02_arm_2k.jpg",
                    "sha256": "a11ab0778948ddb8592d3326fdac1f5efae8a3430a937282793d10ab2a5eaaab",
                },
            ],
        },
        "plaster_grey_04": {
            "authors": ["Rob Tuytel"],
            "page": "https://polyhaven.com/a/plaster_grey_04",
            "files": [
                {
                    "role": "albedo",
                    "url": "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/plaster_grey_04/plaster_grey_04_diff_2k.jpg",
                    "sha256": "f8fd4a4263dc257060f34021bd537d968cc04b38b332f29ddd516f1fdd5ec201",
                },
                {
                    "role": "normal",
                    "url": "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/plaster_grey_04/plaster_grey_04_nor_gl_2k.jpg",
                    "sha256": "25a501bd78e1b55d62bebe2f9fd5ff77ed796a66e092b2efbd5339e4bee15f0a",
                },
                {
                    "role": "arm",
                    "url": "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/plaster_grey_04/plaster_grey_04_arm_2k.jpg",
                    "sha256": "7bed7ffac681c134600cb87507446cd3974a05c672a1315ac597788c660432b6",
                },
            ],
        },
    }
)
floor = json.loads((OUT.parent / "floorboards.provenance.json").read_text())
linen = {
    "diff": "04897d7611d7390ddf5b2c6f7273f448dfc48341fdbd1f4f207fa30b331d7f57",
    "nor_gl": "e2402166d1a489f7414814e69e1836ab672aa24e30bec2d41fea356f81687e53",
    "arm": "e1ca6b51ed62834063253e08f8f2e23836aa00d72675b960ddd8e608d9f0c7e6",
}
base_url = "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/rough_linen/rough_linen_"
source["rough_linen"] = {
    "authors": ["colormass", "Rico Cilliers"],
    "page": "https://polyhaven.com/a/rough_linen",
    "files": [
        {"role": role, "url": base_url + k + "_1k.jpg", "sha256": linen[k]}
        for role, k in (("albedo", "diff"), ("normal", "nor_gl"), ("arm", "arm"))
    ],
}
source["wood_floor_worn"] = {
    "authors": ["Dimitrios Savva"],
    "page": floor["source"],
    "files": [dict(f, role=r) for r, f in zip(("albedo", "normal", "rough"), floor["inputs"])],
}


def get(asset, role, inputs):
    entry = next(f for f in source[asset]["files"] if f["role"] == role)
    path = cache / entry["url"].rsplit("/", 1)[1]
    raw = (
        path.read_bytes()
        if path.exists()
        else urllib.request.urlopen(
            urllib.request.Request(
                entry["url"], headers={"User-Agent": "EarthOnline material pipeline"}
            ),
            timeout=60,
        ).read()
    )
    if hashlib.sha256(raw).hexdigest() != entry["sha256"]:
        raise ValueError(f"Input digest mismatch for {path.name}")
    if not path.exists():
        path.write_bytes(raw)
    inputs.append({"asset": asset, "role": role, "url": entry["url"], "sha256": entry["sha256"]})
    return Image.open(io.BytesIO(raw)).convert("RGB")


def size(im):
    return im.resize((512, 512), Image.Resampling.LANCZOS)


def mirrored_patch(im, rect, role):
    # Reflect the patch's tangent directions too, so the lighting crosses
    # its joins continuously. The clay tile's curvature lives in geometry.
    patch = np.array(im.crop(rect).resize((256, 256), Image.Resampling.LANCZOS))
    x = patch[:, ::-1].copy()
    if role == "normal":
        x[:, :, 0] = 255 - x[:, :, 0]
    row = np.concatenate([patch, x], axis=1)
    y = row[::-1, :].copy()
    if role == "normal":
        y[:, :, 1] = 255 - y[:, :, 1]
    return Image.fromarray(np.concatenate([row, y], axis=0))


def tint(im, base, gain):
    lum = np.asarray(im.convert("L"), dtype=float)
    return Image.fromarray(
        np.clip(np.rint(np.asarray(base) + (lum - np.median(lum))[:, :, None] * gain), 0, 255).astype(
            "uint8"
        )
    )


def surface(name, assets, families, metres, bump, paint_mix):
    pages = []
    inputs = []
    for role in ("albedo", "normal", "arm"):
        if name == "limewash":
            # Both source tiles are seamless. The sand finish supplies the
            # close relief and the float marks introduce soft, broad variation.
            sand = size(get(assets[0], role, inputs))
            float_marks = size(get(assets[1], role, inputs))
            if role == "albedo":
                sand = tint(sand, (235, 224, 204), 0.86)
                float_marks = tint(float_marks, (229, 215, 191), 0.83)
            page = Image.blend(sand, float_marks, 0.39 if role == "albedo" else 0.32)
        elif name == "aggregate":
            # Crop to the size of the actual pictured fines in the private bed.
            page = size(get(assets[0], role, inputs))
            if role == "albedo":
                page = tint(page, (173, 154, 130), 1.53)
        elif name == "fired-earth":
            page = mirrored_patch(get(assets[0], role, inputs), (182, 195, 241, 300), role)
            if role == "albedo":
                # Only the clay body belongs on other shapes. Remove the
                # broad curvature shadow and retain the mineral flecks.
                gray = np.asarray(page.convert("L"), dtype=float)
                local = np.asarray(
                    page.convert("L").filter(ImageFilter.GaussianBlur(22)), dtype=float
                )
                grain = (gray - local) * 0.97 + (local - np.median(local)) * 0.12
                page = Image.fromarray(
                    np.uint8(np.clip(np.array((195, 146, 117)) + grain[:, :, None], 0, 255))
                )
            if role == "arm":
                orm = np.asarray(page, dtype=float).copy()
                orm[:, :, 0] = 250  # the destination tile and grout supply their own contact shadows
                orm[:, :, 1] = 218 + 0.30 * (orm[:, :, 1] - np.median(orm[:, :, 1]))
                orm[:, :, 2] = 0
                page = Image.fromarray(np.uint8(np.clip(orm, 0, 255)))
            if role == "normal":
                # Remove the large tilt of this convex piece of the scan.
                n = np.asarray(page, dtype=float).copy()
                low = np.asarray(page.filter(ImageFilter.GaussianBlur(11)), dtype=float)
                n[:, :, :2] = 128 + (n[:, :, :2] - low[:, :, :2]) * 0.68
                n[:, :, 2] = 253
                page = Image.fromarray(np.uint8(np.clip(n, 0, 255)))
        elif name == "painted-wood":
            r = "rough" if role == "arm" else role
            page = size(get(assets[0], r, inputs).crop((98, 282, 903, 318)))
            if role == "albedo":
                page = tint(page, (221, 220, 210), 0.45)
            if role == "arm":
                page = Image.merge(
                    "RGB",
                    (
                        Image.new("L", page.size, 255),
                        page.getchannel("R"),
                        Image.new("L", page.size, 0),
                    ),
                )
        else:
            page = size(get(assets[0], role, inputs))
            if role == "albedo":
                page = tint(page, (225, 221, 211), 1.4)
        pages.append(page.convert("RGBA"))
    result = Image.new("RGBA", (512, 1536))
    for i, p in enumerate(pages):
        result.paste(p, (0, i * 512))
    out = OUT / (name + ".png")
    result.save(out, compress_level=9)
    raw = out.read_bytes()
    data = {
        "source": [source[k]["page"] for k in assets],
        "license": "CC0-1.0",
        "license_url": "https://polyhaven.com/license",
        "attribution": list(dict.fromkeys(author for k in assets for author in source[k]["authors"])),
        "inputs": inputs,
        "build": "scripts/blender-buildings/prepare_beaver_surfaces.py",
        "url": "/" + out.relative_to(ROOT / "public").as_posix(),
        "sha256": hashlib.sha256(raw).hexdigest(),
        "bytes": len(raw),
        "families": families,
        "metres_per_tile": metres,
        "normal_strength": bump,
        "paint_mix": paint_mix,
    }
    out.with_suffix(".provenance.json").write_text(json.dumps(data, indent=2) + "\n")


surface(
    "limewash",
    ["white_stucco_02", "plaster_grey_04"],
    ["painted-stucco", "float-finish-stucco"],
    1.80,
    0.84,
    0.13,
)
surface("aggregate", ["gravel_floor_03"], ["decomposed-granite", "garden-stone"], 2.50, 1.15, 0.22)
surface("fired-earth", ["clay_roof_tiles_02"], ["fired-clay", "terracotta"], 0.61, 0.42, 0.17)
surface(
    "painted-wood", ["wood_floor_worn"], ["front-door-paint", "front-door-moulding"], 1.0, 0.30, 0.92
)
surface(
    "linen",
    ["rough_linen"],
    [
        "canvas-plain",
        "flax-linen",
        "curtain-linen",
        "cotton-coverlet",
        "cotton-turnback",
        "cotton-binding",
    ],
    0.50,
    0.68,
    0.95,
)
