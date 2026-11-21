"""Cook the measured wall irradiance into one linear texture-array page."""

import argparse
import json
from hashlib import sha256
from pathlib import Path

import numpy as np
from PIL import Image

DEST = (
    Path(__file__).resolve().parents[2]
    / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"
)


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def cook(source, output):
    meta = json.loads((source / "plaster-irradiance.provenance.json").read_text())
    lighting = json.loads((DEST / "front-daylight.provenance.json").read_text())
    house = json.loads((DEST / "kitchen-reflections.provenance.json").read_text())
    raw = source / meta["radiance"]
    if (
        meta["parent_scene_sha256"] != lighting["source_scene_sha256"]
        or digest(source / meta["scene"]) != meta["scene_sha256"]
        or digest(raw) != meta["radiance_sha256"]
    ):
        raise ValueError("The wall survey and source house differ")
    with np.load(raw) as archive:
        light = archive["radiance"].astype(np.float64)
        pigment = archive["pigment"].astype(np.float64)
    if pigment.shape != (128, 2048, 4) or not np.isfinite(pigment).all():
        raise ValueError("Invalid source reflectance")
    # Sixteen 128-square charts are packed four across. Keep gaps transparent.
    pigment = (
        pigment.reshape(128, 4, 4, 128, 4).transpose(1, 0, 2, 3, 4).reshape(512, 512, 4)
    )
    valid = (pigment[:, :, :3].max(2) > 0.0001) & (pigment[:, :, 3] > 0.5)
    if valid.mean() < 0.15:
        raise ValueError("Insufficient plaster coverage")
    linear = np.clip(pigment[:, :, :3], 0, 1)
    srgb = np.where(
        linear <= 0.0031308, 12.92 * linear, 1.055 * np.power(linear, 1 / 2.4) - 0.055
    )
    color_page = np.zeros_like(pigment, dtype=np.uint8)
    color_page[:, :, :3] = np.rint(255 * srgb * valid[:, :, None]).astype(np.uint8)
    color_page[:, :, 3] = valid.astype(np.uint8) * 255
    if (
        light.shape != (8, 64, 1024, 3)
        or not np.isfinite(light).all()
        or np.any(light < 0)
    ):
        raise ValueError("Invalid wall irradiance")
    light[:6] *= meta["canonical_source_to_game"]
    light[7] *= meta["canonical_source_to_game"]
    # Row six describes outgoing radiance under a unit diffuse sky.
    # All other rows are scene-linear light in the calibrated runtime units.
    light = np.concatenate(
        [
            light[:, :, :512].reshape(512, 512, 3),
            light[:, :, 512:].reshape(512, 512, 3),
        ],
        axis=0,
    )
    encoded = np.rint(255 * np.sqrt(light / (1 + light))).astype(np.uint8)
    pixels = np.full((1024, 512, 4), 255, dtype=np.uint8)
    pixels[:, :, :3] = encoded
    output.mkdir(exist_ok=True, parents=True)
    path = output / "wall-daylight.png"
    Image.fromarray(np.concatenate([pixels, color_page], axis=0)).save(
        path, compress_level=9
    )
    s = (encoded.astype(np.float64) / 255) ** 2
    decoded = s / (1 - s)
    err = np.abs(decoded - light) / (0.01 + light)
    photo = json.loads(
        (DEST / "fitted-dining/floor-reflections.provenance.json").read_text()
    )["dining_camera"]
    display_gain = photo["display_meter_gain"]
    evidence = {
        "url": lighting["url"].rsplit("/", 1)[0] + "/" + path.name,
        "sha256": digest(path),
        "bytes": path.stat().st_size,
        "house_sha256": house["house_sha256"],
        "source_scene_sha256": meta["scene_sha256"],
        "parent_scene_sha256": meta["parent_scene_sha256"],
        "raw_sha256": meta["radiance_sha256"],
        "hours": meta["hours"],
        "charts": meta["charts"],
        "cell_size": 64,
        "pigment_cell_size": 128,
        "pigment_coverage": float(valid.mean()),
        "source_exposure_ev": meta["source_exposure_ev"],
        "display_meter_gain": display_gain,
        "linear_game_units_per_source_unit": meta["canonical_source_to_game"],
        "encoding": "sqrt(L/(1+L))",
        "relative_encoding_error_p95": float(np.percentile(err, 95)),
        "attribution": "Earth Online. Cycles irradiance measurements on the plaster envelope of 21 Beaver Street. Source plaster and garden photographs are credited in limewash.provenance.json and street-garden.provenance.json.",
        "license": "Project-authored lighting measurements, MIT. Existing photographed material inputs, CC0 1.0.",
        "build": "scripts/blender-buildings/cook_beaver_wall_daylight.py",
    }
    (output / "wall-daylight.provenance.json").write_text(
        json.dumps(evidence, indent=2) + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--output", type=Path, default=DEST)
    args = parser.parse_args()
    cook(args.source, args.output)
