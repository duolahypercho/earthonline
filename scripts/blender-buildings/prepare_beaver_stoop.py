"""Build registered pigment, relief and roughness pages for the hand-set stair tiles."""

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

FILES = {
    "diff": "dd1adc1d35618db8fd58f2d22b51242bc570dc83c23518a1100d3570dfe05559",
    "nor_gl": "7378375dd7d7ec01a33bce4fc679f8a10fad4085746c7ad698724e8ca1113807",
    "arm": "6a44f0a0cec7caab1f7fa0753a958dfd087d8a4dc7003b6fe394aa12b21040eb",
}
CELLS = ((0, 1), (3, 0), (6, 1), (4, 3), (2, 4), (7, 6), (5, 7), (1, 7))
SIZE = 248  # Four pixels of padding at every edge of a half-page.


def digest(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def noise(rng, frequency):
    small = rng.random((frequency, frequency), dtype=np.float32)
    field = np.asarray(
        Image.fromarray(small).resize((SIZE, SIZE), Image.Resampling.BICUBIC)
    )
    return (field - field.mean()) / max(field.std(), 1e-6)


def face(originals, col, row, index):
    rng = np.random.default_rng(21741 + index)
    crop = tuple(
        round(v * 2048 / 9) for v in (col + 0.07, row + 0.07, col + 0.93, row + 0.93)
    )
    photo, normal, orm = [
        np.asarray(im.crop(crop).resize((SIZE, SIZE), Image.Resampling.LANCZOS)).astype(
            float
        )
        / 255
        for im in originals
    ]
    # Retain the spatial variation in eight different pieces of fired clay.
    # The ochre slip wears to a lighter clay body along the rounded front cap.
    level = np.mean(photo, axis=(0, 1))
    kiln = np.array((150, 84, 53)) / 255 + rng.uniform(-0.025, 0.025)
    body = kiln * (photo / level) ** 0.86
    v, u = (np.arange(SIZE) + 0.5) / SIZE, (np.arange(SIZE) + 0.5) / SIZE
    edge = np.minimum(u, 1 - u)[None, :]
    fine = noise(rng, 92)
    medium = noise(rng, 24)
    coarse = noise(rng, 7)
    rolled_edge = np.exp(-(((v[:, None] - 0.89) / 0.055) ** 2))
    sand = 0.41 * coarse + 0.33 * medium + 0.24 * fine
    # Mineral bloom follows the grout and local pores. The broad foot contact
    # exposes the clay slip. Keep the point light and cast shadows out of the color.
    joints = np.exp(-edge / 0.095) + 0.65 * np.exp(-v[:, None] / 0.07)
    salts = np.clip((sand + 0.48 * joints - 0.48) * 0.63, 0, 0.78)
    worn = np.clip((0.65 + 0.22 * medium + 0.13 * fine) * rolled_edge, 0, 1)
    clay = body * (1 + 0.035 * coarse[:, :, None])
    clay += worn[:, :, None] * np.array((0.085, 0.069, 0.044))
    color = clay * (1 - salts[:, :, None]) + salts[:, :, None] * (
        np.array((184, 155, 124)) / 255
    )
    n = normal * 2 - 1
    gy, gx = np.gradient(0.10 * medium + 0.2 * salts + 0.018 * fine)
    n[:, :, 0] = 0.83 * n[:, :, 0] - gx
    n[:, :, 1] = 0.83 * n[:, :, 1] + gy
    n /= np.maximum(np.linalg.norm(n, axis=2, keepdims=True), 1e-6)
    rough = np.clip(
        0.64
        + (orm[:, :, 1] - np.mean(orm[:, :, 1])) * 0.85
        + 0.18 * salts
        - 0.20 * worn,
        0.40,
        0.92,
    )
    orm = np.stack((orm[:, :, 0], rough, np.zeros_like(rough)), axis=2)
    return [
        np.pad(
            np.uint8(np.rint(np.clip(z, 0, 1) * 255)),
            ((4, 4), (4, 4), (0, 0)),
            mode="edge",
        )
        for z in (color, 0.5 + 0.5 * n, orm)
    ]


def build(inputs, dest):
    dest.mkdir(parents=True, exist_ok=True)
    originals = []
    for role, expected in FILES.items():
        src = inputs / f"terracotta_floor_tiles_{role}_2k.jpg"
        if digest(src) != expected:
            raise ValueError(f"Unregistered input {src.name}")
        with Image.open(src) as im:
            originals.append(im.convert("RGB"))
    files = []
    for group in range(2):
        pages = [np.empty((512, 512, 3), dtype=np.uint8) for _ in range(3)]
        for cell in range(4):
            index = 4 * group + cell
            y, x = divmod(cell, 2)
            for page, sample in zip(pages, face(originals, *CELLS[index], index)):
                page[256 * y : 256 * (y + 1), 256 * x : 256 * (x + 1)] = sample
        for role, page in zip(("color", "normal", "orm"), pages):
            path = dest / f"kiln-{group + 1}-{role}.png"
            Image.fromarray(page).save(path, compress_level=9)
            files.append({"file": path.name, "sha256": digest(path)})
    record = {
        "files": files,
        "inputs": FILES,
        "scan": "Terracotta Floor Tiles",
        "artist": "Dimitrios Savva",
        "license": "CC0-1.0",
        "url": "https://polyhaven.com/a/terracotta_floor_tiles",
        "authored": "Kiln tones, mineral bloom and worn edges composed for the Beaver Street house.",
        "build": "scripts/blender-buildings/prepare_beaver_stoop.py",
    }
    (dest / "surfaces.json").write_text(json.dumps(record, indent=2) + "\n")


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("inputs", type=Path)
    cli.add_argument("destination", type=Path)
    args = cli.parse_args()
    build(args.inputs, args.destination)
