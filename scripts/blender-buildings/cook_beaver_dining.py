"""Encode the Blender floor measurements in the two existing linear panorama slots."""

import hashlib
import json
from pathlib import Path

import numpy as np
from cook_beaver_mission_roof import png

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"
SIZE = 512
LUMA = np.array([0.2126, 0.7152, 0.0722])


def cook(measurements, output, source):
    def filtered(name):
        values = np.load(measurements / (name + ".npy"))
        # Suppress isolated bright samples through the opal glass and varnish.
        # The three-texel neighborhood preserves the resolved legs and rails.
        neighbors = np.lib.stride_tricks.sliding_window_view(
            np.pad(values, ((1, 1), (1, 1), (0, 0)), mode="edge"), (3, 3), axis=(0, 1)
        )
        return np.median(neighbors, axis=(-2, -1))

    def encode(values):
        return np.rint(255 * np.sqrt(np.clip(values, 0, 1))).astype(np.uint8)

    # Match the two engines' incident clear-noon key on a white card. The lamp
    # uses this one fixed unit conversion for all camera angles, hours and weather.
    reference_key = np.asarray(source["reference_game_key_rgb"])
    source_key = np.asarray(source["source_key_rgb"])
    lamp_units = (reference_key @ LUMA) / (source_key @ LUMA)
    lamp = filtered("pendant") * lamp_units
    direct = filtered("pendant-direct")[:, :, 0] * lamp_units
    sky = filtered("clear-sky") / source_key
    uniform = np.sum(filtered("uniform-sky") * LUMA, axis=-1)
    half = SIZE // 2
    quarter = SIZE // 4
    other_light = np.zeros((SIZE, SIZE))
    other_light[half:] = uniform.reshape(half, 2, SIZE).mean(axis=1)
    for row, side in enumerate(("front", "rear")):
        for col, angle in enumerate((25, 57)):
            field = np.sum(filtered(f"{side}-{angle}") * LUMA, axis=-1)
            other_light[
                row * quarter : (row + 1) * quarter, col * half : (col + 1) * half
            ] = field.reshape(quarter, 2, half).mean(axis=1)
    lamp_page = np.dstack((encode(lamp / 2), encode(direct / 2)))
    sky_page = np.dstack((encode(sky), encode(other_light)))
    path = output / "dining-floor-light.png"
    png(path, np.concatenate((lamp_page, sky_page)))
    meta = dict(
        source,
        url="/" + (DEST / path.name).relative_to(ROOT / "public").as_posix(),
        bytes=path.stat().st_size,
        sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
        fixture_units_per_source_unit=float(lamp_units),
        pages=[
            "Lamp RGB in native linear units (divided by two); direct red lamp radiance in alpha at the same scale.",
            "Clear sky RGB relative to the source key; alpha packs four 256 by 128 indirect solar fields above a 512 by 256 diffuse sky field.",
        ],
        encoding="Square root of diffuse exitant radiance on a neutral white Lambertian receiver.",
    )
    path.with_suffix(".provenance.json").write_text(json.dumps(meta, indent=2) + "\n")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("measurements", type=Path)
    parser.add_argument("--install", action="store_true")
    args = parser.parse_args()
    folder = args.measurements.resolve()
    if folder.is_relative_to(ROOT):
        parser.error("Keep original measurements outside the checkout.")
    cook(
        folder,
        DEST if args.install else folder,
        json.loads((folder / "source.json").read_text()),
    )
