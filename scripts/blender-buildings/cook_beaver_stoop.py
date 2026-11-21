"""Cook the fitted clay courses into reusable, grade-following parts for the game."""

import argparse
import json
from pathlib import Path

import numpy as np
from cook_beaver_fitted_room import PARENT, digest
from cook_beaver_street_garden import portable_geometry
from PIL import Image

URL = "/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/stoop/"


def record(path, metadata):
    ref = {"url": URL + path.name, "bytes": path.stat().st_size, "sha256": digest(path)}
    path.with_suffix(".provenance.json").write_text(
        json.dumps(dict(**ref, **metadata), indent=2) + "\n"
    )
    return ref


def lightmap(source, dest, model):
    raw = source / "light/irradiance.npy"
    spec = json.loads((raw.parent / "irradiance.provenance.json").read_text())
    if (
        digest(raw) != spec["sha256"]
        or model["sha256"] != spec["source_sha256"]
        or spec["hours"] != [8.5, 12, 17, 22]
        or spec["sky_strength"][-1] != 0
    ):
        raise ValueError("Incompatible course irradiance")
    light = np.load(raw, allow_pickle=False).astype(float)
    if (
        light.shape != (6, 32 * len(model["courses"]), 128, 3)
        or not np.isfinite(light).all()
        or light.min() < 0
        or light[3].max() > 0.0001
    ):
        raise ValueError("Invalid stair survey")
    field = json.loads(
        (
            source.parent / "opal-porch/transfer/porch-transfer.provenance.json"
        ).read_text()
    )
    if field["source_sha256"] != model["parent_sha256"]:
        raise ValueError("Inconsistent radiometric calibration")
    light[[0, 1, 2, 3, 5]] *= field["calibration"]
    page = np.zeros((1536, 512, 4), np.uint8)
    for row, signal in enumerate(light):
        y, x = divmod(row, 4)
        encoded = np.uint8(np.rint(np.sqrt(signal / (1 + signal)) * 255))
        page[
            1024 + 256 * y : 1024 + 256 * y + len(signal), 128 * x : 128 * (x + 1), :3
        ] = encoded
    target = dest / "stair-light.png"
    Image.fromarray(page).save(target, compress_level=9)
    return record(
        target,
        {
            "source": spec,
            "calibration": field["calibration"],
            "build": "scripts/blender-buildings/cook_beaver_stoop.py",
            "license": "Project-authored Earth Online light measurements.",
        },
    )


def cook(source, dest):
    meta = json.loads((source / "stoop.provenance.json").read_text())
    materials = json.loads((source / "surfaces/surfaces.json").read_text())
    if digest(source / meta["file"]) != meta["sha256"] or meta["tiles"] != 5 * len(
        meta["courses"]
    ):
        raise ValueError("Invalid fitted clay scene or module count")
    inputs = {s["file"]: s["sha256"] for s in materials["files"]}
    dest.mkdir(exist_ok=True, parents=True)
    recipe = {
        "house_sha256": meta["house_sha256"],
        "blender_sha256": meta["sha256"],
        "tiles": meta["tiles"],
        "light": lightmap(source, dest, meta),
        "courses": [],
        "maps": [],
        "pier_bases": meta["pier_bases"],
    }
    build = "scripts/blender-buildings/cook_beaver_stoop.py"
    for group, name in enumerate(
        ("Stoop clay · ochre kiln", "Stoop clay · umber kiln")
    ):
        page = Image.new("RGBA", (512, 1536))
        for i, role in enumerate(("color", "normal", "orm")):
            path = source / "surfaces" / f"kiln-{group + 1}-{role}.png"
            if digest(path) != inputs[path.name]:
                raise ValueError(path)
            with Image.open(path) as im:
                if im.size != (512, 512):
                    raise ValueError(path)
                page.paste(im.convert("RGBA"), (0, 512 * i))
        packed = dest / f"kiln-{group + 1}.png"
        page.save(packed, compress_level=9)
        recipe["maps"].append(
            {
                "material": name,
                "asset": record(packed, {"source": materials, "build": build}),
            }
        )
    for i, course in enumerate(meta["courses"]):
        raw = source / course["file"]
        if digest(raw) != course["sha256"]:
            raise ValueError(raw)
        target = dest / raw.name
        transform = np.array(meta["parcel_from_assembled_gltf"])
        transform[1, 3] -= course["height"]
        transform[2, 3] -= course["depth"]
        names, count = portable_geometry(
            raw,
            transform,
            target,
            label="Hand-set landing" if i == 0 else "Hand-set stair course",
        )
        if set(names) != {m["material"] for m in recipe["maps"]}:
            raise ValueError("Unexpected fired clay")
        recipe["courses"].append(
            {
                "asset": record(
                    target,
                    {
                        "source_sha256": course["sha256"],
                        "blender_sha256": meta["sha256"],
                        "license": "Project-authored Earth Online geometry.",
                        "build": build,
                    },
                ),
                "triangles": count,
            }
        )
    (dest / "stoop.provenance.json").write_text(json.dumps(recipe, indent=2) + "\n")


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("source", type=Path)
    cli.add_argument("--output", type=Path, default=PARENT / "stoop")
    args = cli.parse_args()
    cook(args.source, args.output)
