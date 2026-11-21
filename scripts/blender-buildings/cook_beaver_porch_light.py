"""Pack the measured porch visibility in a linearly filterable, two-channel field."""

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

URL = "/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/porch-light.png"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def srgb(linear):
    return np.where(
        linear <= 0.0031308, 12.92 * linear, 1.055 * linear ** (1 / 2.4) - 0.055
    )


def filtered_fixture(direct, indirect, pigment):
    """Average the diffuse bounce on each receiver without moving the direct shadow."""
    result = direct.astype(np.float64).copy()
    for left in range(0, pigment.shape[1], 128):
        part = np.s_[:, left : left + 128]
        albedo = pigment[part]
        mask = (albedo[:, :, 3] > 0.5) & (albedo[:, :, :3].max(2) > 0.0001)
        bounce = indirect[part]
        weighted = np.dstack((bounce * mask[:, :, None], mask))
        for axis in (0, 1):
            # One standard deviation is one chart texel. Normalize at door
            # cutouts and chart edges so the empty region cannot darken the wall.
            span = weighted.shape[axis]
            pad = [(0, 0), (0, 0), (0, 0)]
            pad[axis] = (2, 2)
            padded = np.moveaxis(np.pad(weighted, pad), axis, 0)
            averaged = (
                sum(k * padded[i : i + span] for i, k in enumerate((1, 4, 6, 4, 1)))
                / 16
            )
            weighted = np.moveaxis(averaged, 0, axis)
        filtered = np.divide(
            weighted[:, :, :3],
            weighted[:, :, 3:],
            out=bounce.astype(np.float64).copy(),
            where=weighted[:, :, 3:] > 0,
        )
        result[part] += np.where(mask[:, :, None], filtered, bounce)
    return result


def surface_pages(source, field):
    meta = json.loads((source / "porch-surfaces.provenance.json").read_text())
    raw = source / "porch-surfaces.npz"
    if (
        digest(raw) != meta["raw_sha256"]
        or field["source_sha256"] != meta["source_sha256"]
        or meta["hours"] != [8.5, 12, 17, 22]
        or meta["resolution"] != 128
        or meta["sky_strength"][-1] != 0
    ):
        raise ValueError("Invalid porch surface survey")
    with np.load(raw, allow_pickle=False) as archive:
        light = archive["radiance"].astype(np.float64)
        pigment = archive["pigment"].astype(np.float64)
        light[5] = filtered_fixture(
            archive["fixture_direct"], archive["fixture_indirect"], pigment
        )
    if (
        light.shape != (6, 128, 640, 3)
        or pigment.shape != (128, 640, 4)
        or not np.isfinite(light).all()
        or not np.isfinite(pigment).all()
        or light.min() < 0
    ):
        raise ValueError("Invalid surface measurements")
    light[[0, 1, 2, 3, 5]] *= field["calibration"]
    linear_page = np.zeros((512, 512, 4), np.uint8)
    color_page = np.zeros_like(linear_page)
    for h, row in enumerate(light):
        for chart in range(5):
            index = 5 * h + chart
            y, x = divmod(index % 16, 4)
            radiance = row[:, 128 * chart : 128 * (chart + 1)]
            encoded = np.sqrt(radiance / (1 + radiance))
            page = linear_page if index < 16 else color_page
            if index >= 16:
                encoded = srgb(encoded)
            page[128 * y : 128 * (y + 1), 128 * x : 128 * (x + 1), :3] = np.rint(
                255 * encoded
            )
    for chart in range(5):
        original = pigment[:, 128 * chart : 128 * (chart + 1)]
        valid = (original[:, :, 3] > 0.5) & (original[:, :, :3].max(2) > 0.0001)
        sample = np.dstack(
            (np.clip(original[:, :, :3], 0, 1) * valid[:, :, None], valid)
        )
        sample = sample.reshape(64, 2, 64, 2, 4).mean((1, 3))
        sample[:, :, :3] = srgb(sample[:, :, :3])
        y, x = divmod(chart, 4)
        color_page[384 + 64 * y : 448 + 64 * y, 256 + 64 * x : 320 + 64 * x] = np.rint(
            255 * sample
        )
    return color_page, linear_page, meta


def cook(source, destination):
    raw = source / "porch-transfer.npy"
    meta = json.loads((source / "porch-transfer.provenance.json").read_text())
    if digest(raw) != meta["npy_sha256"]:
        raise ValueError("Porch transfer differs from the measured source")
    values = np.load(raw, allow_pickle=False)
    if meta["grid"] != [48, 64, 48] or values.shape != (48, 64, 48, 2):
        raise ValueError("Expected the registered porch grid")
    if not np.isfinite(values).all() or values.min() < 0:
        raise ValueError("Invalid porch irradiance")
    scales = values.max(axis=(0, 1, 2)).astype(np.float64)
    if min(scales) <= 0:
        raise ValueError("Missing area or point light field")
    quantized = np.rint(values / scales * 65535).astype(np.uint16)
    # Hardware bilinear filtering of high and low bytes reconstructs a linear
    # weighted sum in each horizontal slice. The shader interpolates in height.
    atlas = np.zeros((512, 512, 4), dtype=np.uint8)
    for y in range(64):
        plane = quantized[:, y].transpose(1, 0, 2)
        row, column = divmod(y, 8)
        tile = atlas[64 * row : 64 * row + 48, 64 * column : 64 * column + 48]
        tile[:, :, ::2] = plane >> 8
        tile[:, :, 1::2] = plane & 255
    destination.mkdir(parents=True, exist_ok=True)
    png = destination / "porch-light.png"
    color_page, linear_page, surface = surface_pages(source.parent / "surfaces", meta)
    Image.fromarray(np.concatenate((color_page, linear_page, atlas))).save(
        png, compress_level=9
    )
    study = json.loads((source.parent / "porch-light.provenance.json").read_text())
    if meta["source_sha256"] != study["sha256"]:
        raise ValueError("Camera and light field must come from the same study")
    eye = next(
        v[1] for v in study["cameras"] if v[0] == "Entrance from the design reference"
    )
    camera_distance = np.linalg.norm(np.array(eye) - study["fixture"]["centre"])
    result = {
        **meta,
        "url": URL,
        "sha256": digest(png),
        "bytes": png.stat().st_size,
        "scales": scales.tolist(),
        "reference_camera_distance_m": float(camera_distance),
        "encoding": "porch-radiance-and-visibility-v1",
        "surface_sha256": surface["raw_sha256"],
        "surface_build": surface["build"],
        "fixture_direct_samples": surface["fixture_direct_samples"],
        "fixture_indirect_samples": surface["fixture_indirect_samples"],
        "charts": surface["charts"],
        "hours": surface["hours"],
        "source": "Area and visibility of the fitted opal porch shade and its previous point light in the authored Blender scene.",
        "license": "Project-authored for Earth Online. No third-party content in this light field.",
        "build": "scripts/blender-buildings/cook_beaver_porch_light.py",
    }
    (destination / "porch-light.provenance.json").write_text(
        json.dumps(result, indent=2) + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    cook(args.source, args.destination)
