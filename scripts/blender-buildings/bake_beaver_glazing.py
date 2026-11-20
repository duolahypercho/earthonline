"""Capture the mapped block at the front sash and pack its day/night reflection maps."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina"
parser = argparse.ArgumentParser()
parser.add_argument("source_cache", type=Path)
parser.add_argument(
    "--capture",
    action="store_true",
    help="Generate the six native game views for each time before packing",
)
args = parser.parse_args()
cache = args.source_cache.resolve()
if cache.is_relative_to(ROOT):
    raise ValueError("The native reflection source views belong outside the checkout.")
cache.mkdir(parents=True, exist_ok=True)
# Surveyed world coordinates of the front garden opposite the middle sash.
eye = [-1616.52, 65.75, 1810.34]
views = [
    ("north", 0.0, 0.0),
    ("east", math.pi / 2, 0.0),
    ("south", math.pi, 0.0),
    ("west", -math.pi / 2, 0.0),
    ("up", 0.0, math.pi / 2 - 0.00001),
    ("down", 0.0, -math.pi / 2 + 0.00001),
]
poses = [
    dict(name=name, eye=eye, yaw=yaw, pitch=pitch, fov_y_deg=90, near=0.05, far=4200.0, aspect=1.0)
    for name, yaw, pitch in views
]
pose_file = cache / "front-sash-poses.json"
pose_file.write_text(json.dumps(poses, indent=2) + "\n")
coords = (np.arange(512) + 0.5) / 512
u, v = np.meshgrid(coords, coords)
yaw = (u - 0.5) * 2 * math.pi
elev = (0.5 - v) * math.pi
ray = np.stack((np.cos(elev) * np.sin(yaw), np.sin(elev), -np.cos(elev) * np.cos(yaw)), axis=-1)
axes = []
for _, a, b in views:
    forward = np.array((math.cos(b) * math.sin(a), math.sin(b), -math.cos(b) * math.cos(a)))
    right = np.array((math.cos(a), 0, math.sin(a)))
    axes.append((forward, right, np.cross(right, forward)))
owner = np.argmax(
    np.stack([np.einsum("...i,i->...", ray, forward) for forward, _, _ in axes]), axis=0
)
atlas = Image.new("RGBA", (512, 1024))
inputs = []
for page, (time, hour) in enumerate((("day", 12), ("night", 22))):
    label = "sash-" + time
    if args.capture:
        cmd = [
            "./target/release/eo-app",
            "--bench",
            "--segments",
            "kit_street",
            "--beaver-house",
            "--house-finishes",
            "--no-house-patina",
            "--no-agents",
            "--no-upload-ring",
            "--upload-budget-mb",
            "32",
            "--width",
            "512",
            "--height",
            "512",
            "--time-of-day",
            str(hour),
            "--label",
            label,
            "--out",
            str(cache),
        ]
        with (cache / (label + ".log")).open("w") as log:
            subprocess.run(
                cmd,
                cwd=ROOT / "native",
                env=dict(os.environ, EO_CAPTURE_POSES=str(pose_file), EO_HLOD="0"),
                stdout=log,
                stderr=subprocess.STDOUT,
                check=True,
            )
    panorama = np.zeros((512, 512, 3), dtype=np.uint8)
    for i, ((name, _, _), (forward, right, up)) in enumerate(zip(views, axes)):
        path = cache / f"capture-{label}-{name}.png"
        inputs.append({"view": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
        image = np.asarray(Image.open(path).convert("RGB"))
        mask = owner == i
        dirs = ray[mask]
        z = np.einsum("ij,j->i", dirs, forward)
        x = np.clip(
            (np.einsum("ij,j->i", dirs, right) / z + 0.99999999) * 0.5 * image.shape[1] - 0.5,
            0,
            image.shape[1] - 1,
        )
        y = np.clip(
            (1.0 - np.einsum("ij,j->i", dirs, up) / z) * 0.5 * image.shape[0] - 0.5,
            0,
            image.shape[0] - 1,
        )
        x0 = x.astype(int)
        y0 = y.astype(int)
        x1 = np.minimum(x0 + 1, image.shape[1] - 1)
        y1 = np.minimum(y0 + 1, image.shape[0] - 1)
        fx = (x - x0)[:, None]
        fy = (y - y0)[:, None]
        panorama[mask] = np.rint(
            ((1 - fx) * image[y0, x0] + fx * image[y0, x1]) * (1 - fy)
            + ((1 - fx) * image[y1, x0] + fx * image[y1, x1]) * fy
        )
    atlas.paste(Image.fromarray(panorama).convert("RGBA"), (0, 512 * page))
out = OUT / "street-reflections.png"
atlas.save(out, compress_level=9)
out.with_suffix(".provenance.json").write_text(
    json.dumps(
        {
            "source": "Native Earth Online release captures of the Beaver Street block, with the closed-door courtyard loaded",
            "attribution": "Earth Online contributors. City data sources and licences are recorded in public/data/sf/*.NOTICE.txt.",
            "license": "MIT (game rendering); underlying city data retains its stated licences",
            "build": "scripts/blender-buildings/bake_beaver_glazing.py",
            "eye_world_m": eye,
            "poses": poses,
            "inputs": inputs,
            "url": "/" + out.relative_to(ROOT / "public").as_posix(),
            "sha256": hashlib.sha256(out.read_bytes()).hexdigest(),
            "bytes": out.stat().st_size,
        },
        indent=2,
    )
    + "\n"
)
