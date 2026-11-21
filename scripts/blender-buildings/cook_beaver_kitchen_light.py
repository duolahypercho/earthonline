"""Cook the fitted shade and kitchen light measurements for the house texture arrays."""

import argparse
import json
from collections import defaultdict
from pathlib import Path

import numpy as np
from beaver_floor_gloss import SOURCES
from check_interior_containment import accessor, local_matrix, read_glb
from cook_beaver_fitted_finishes import (
    ROUGHNESS,
    denoise_transfer,
    encode,
    pack_reflections,
)
from cook_beaver_fitted_room import DEST, PUBLIC, digest
from PIL import Image


def in_run(name):
    return name.startswith(
        (
            "chopping-board - ",
            "oven-range - ",
            "range-hood - ",
            "base-cabinets - ",
            "enamel-stockpot - ",
            "enamel-refrigerator - ",
            "Patinated brass ",
            "Painted kitchen ",
            "Door stile ",
            "Door rail ",
            "Recessed panel ",
            "Drawer front ",
            "Three-bay kitchen ",
        )
    )


def registrations(portable):
    doc, _ = read_glb(portable)
    parts = defaultdict(set)
    for node in doc["nodes"]:
        if "mesh" in node:
            part = node["extras"]["source_part"]
            for primitive in doc["meshes"][node["mesh"]]["primitives"]:
                parts[doc["materials"][primitive["material"]]["name"]].add(part)
    registrations = {}
    for name, uses in parts.items():
        if name == "Black stove enamel":
            group = "shade"
        elif name == "Cream vitreous reflector":
            group = "reflector"
        elif name in (
            "Braided brown flex",
            "Darkened brass fittings",
            "Warm opal globe",
        ):
            group = "fitting"
        elif uses == {"Lime grout bed"}:
            group = "grout"
        elif uses == {"Hand-set running bond tile"}:
            group = "tile"
        elif name == "porcelain@e4dfce":
            group = "worktop"
        elif all(in_run(s) for s in uses):
            group = "kitchen"
        elif any(in_run(s) for s in uses):
            group = "shared"
        else:
            continue
        registrations[name] = group
    if set(registrations.values()) != {
        "shade",
        "reflector",
        "fitting",
        "tile",
        "worktop",
        "shared",
        "grout",
        "kitchen",
    }:
        raise ValueError("Incomplete kitchen materials")
    return registrations


def shared_bounds(path, groups):
    doc, blob = read_glb(path)
    lows, highs = [], []
    for node in doc["nodes"]:
        if not in_run(node["name"]):
            continue
        for prim in doc["meshes"][node["mesh"]]["primitives"]:
            name = doc["materials"][prim["material"]]["name"]
            if groups.get(name) == "shared":
                xyz = accessor(doc, blob, prim["attributes"]["POSITION"])
                matrix = local_matrix(node)
                placed = (
                    np.einsum("ij,kj->ik", xyz, matrix[:3, :3].astype(np.float64))
                    + matrix[:3, 3]
                )
                if not np.isfinite(placed).all():
                    raise ValueError(f"Non-finite kitchen position in {node['name']}")
                lows.append(placed.min(0))
                highs.append(placed.max(0))
    # Allow for the world-to-parcel float transform at city-scale coordinates.
    return [
        (np.min(lows, axis=0) - 0.002).tolist(),
        (np.max(highs, axis=0) + 0.002).tolist(),
    ]


def pack_board(source, room, output):
    path = source.with_name("kitchen-board-survey.npz")
    info = json.loads(path.with_suffix(".provenance.json").read_text())
    if (
        info["sha256"] != digest(path)
        or info["source_scene_sha256"] != room["source_scene_sha256"]
        or info["source_glb_sha256"] != room["source_glb_sha256"]
        or info["camera_glare"] != 0
    ):
        raise ValueError("The board and camera must match the fitted kitchen")
    with np.load(path, allow_pickle=False) as raw:
        color, normal = raw["pigment"], raw["normals"]
    if (
        any(
            m.shape != (512, 512, 3) or not np.isfinite(m).all()
            for m in (color, normal)
        )
        or color.max() <= 0
    ):
        raise ValueError("Invalid baked oak material")
    srgb = np.where(
        color <= 0.0031308,
        12.92 * color,
        1.055 * np.maximum(0, color) ** (1 / 2.4) - 0.055,
    )
    pages = np.full((3, 512, 512, 4), 255, np.uint8)
    pages[0, :, :, :3] = np.rint(np.clip(srgb, 0, 1) * 255).astype(np.uint8)
    pages[1, :, :, :3] = np.rint(np.clip(normal, 0, 1) * 255).astype(np.uint8)
    # Keep source roughness/metallic factors; these two channels are multipliers.
    name = output / "kitchen-board.png"
    Image.fromarray(pages.reshape(1536, 512, 4)).save(name, compress_level=9)
    recipe = {
        k: info[k]
        for k in (
            "material",
            "source_part",
            "local_from_parcel",
            "min",
            "size",
            "widths",
            "roughness",
            "license",
            "attribution",
            "source_scene_sha256",
            "camera_glare",
        )
    }
    doc, _ = read_glb(DEST / "fitted-room.glb")
    node = next(n for n in doc["nodes"] if n["name"] == info["source_part"])
    prims = doc["meshes"][node["mesh"]]["primitives"]
    if any(doc["materials"][p["material"]]["name"] != info["material"] for p in prims):
        raise ValueError("The oak board must be a single finish")
    recipe["triangles"] = sum(
        doc["accessors"][p["indices"]]["count"] // 3 for p in prims
    )
    recipe.update(
        url="/" + (DEST / name.name).relative_to(PUBLIC).as_posix(),
        bytes=name.stat().st_size,
        sha256=digest(name),
        source="The actual chopping board's six oak faces, procedural pigment and tangent-space bump from the fitted source.",
        survey={"file": path.name, "sha256": info["sha256"]},
        fitted_geometry_sha256=room["sha256"],
        build="scripts/blender-buildings/cook_beaver_kitchen_light.py",
    )
    name.with_suffix(".provenance.json").write_text(json.dumps(recipe, indent=2) + "\n")
    return recipe


def cook(source, portable, output):
    survey = json.loads(source.with_suffix(".provenance.json").read_text())
    room = json.loads((DEST / "fitted-room.provenance.json").read_text())
    if (
        digest(source) != survey["sha256"]
        or survey["sources"] != list(SOURCES)
        or digest(portable) != room["source_glb_sha256"]
        or survey["geometry_sha256"] != room["source_glb_sha256"]
        or survey["house_sha256"] != room["house_sha256"]
        or survey["source_scene"]["sha256"] != room["source_scene_sha256"]
    ):
        raise ValueError(
            "Kitchen light and room geometry must have the same registered source"
        )
    with np.load(source, allow_pickle=False) as data:
        field = data["transfer"]
        panos = data["incident"]
    if (
        field.shape != (8, 64, 640, 3)
        or panos.shape != (7, 8, 64, 128, 3)
        or not all(np.isfinite(a).all() and a.min() >= 0 for a in (field, panos))
    ):
        raise ValueError("Invalid incident light")
    pages = np.zeros((3, 512, 512, 4), np.uint8)
    pages[:, :, :, 3] = 255
    # The eighth row is the direct contribution of the two real downlights.
    main = denoise_transfer(field[:, :, :512], (128, 256, 128))
    pages[0, :, :, :3] = encode(main.reshape(512, 512, 3), srgb=True)
    pack_reflections(pages, panos)
    # The two strips below the directional panoramas carry the board and the
    # two directions of exposed mortar. Keep each physical receiver independent.
    for page, block in [(1, field[:, :, 512:576]), (2, field[:, :, 576:])]:
        if page == 1:
            filtered = denoise_transfer(block, (64,))
        else:
            filtered = np.concatenate(
                [denoise_transfer(block[:, j : j + 32], (64,)) for j in (0, 32)], axis=1
            )
        for row, tile in enumerate(filtered):
            pages[page, 448:, row * 64 : (row + 1) * 64, :3] = encode(tile)
    output.mkdir(parents=True, exist_ok=True)
    path = output / "kitchen-light.png"
    Image.fromarray(pages.reshape(1536, 512, 4)).save(path, compress_level=9)
    recipe = {
        k: survey[k]
        for k in (
            "license",
            "attribution",
            "source_scene",
            "house_sha256",
            "sources",
            "probe_blender_xyz",
            "spot_blender_xyz",
            "kitchen_frame",
            "samples",
        )
    }
    materials = registrations(portable)
    board = pack_board(source, room, output)
    recipe.update(
        source="Cycles surface transfer on the enamel shade, backsplash and slab in the furnished kitchen; full-sphere incident light above and below the pendant and at six stations along the kitchen run.",
        survey={"file": source.name, "sha256": survey["sha256"]},
        fitted_geometry_sha256=room["sha256"],
        materials=materials,
        shared_bounds=shared_bounds(DEST / "fitted-room.glb", materials),
        board=board,
        roughness=ROUGHNESS,
        surface_only_source="range-hood-direct",
        pages=[
            "sRGB diffuse transfer (shade, tile, worktop)",
            "linear GGX radiance, two narrow lobes; board irradiance",
            "linear GGX radiance, two broad lobes; recessed mortar irradiance",
        ],
        transfer_encoding="sqrt(L/(1+L))",
        build="scripts/blender-buildings/cook_beaver_kitchen_light.py",
        url="/" + (DEST / path.name).relative_to(PUBLIC).as_posix(),
        bytes=path.stat().st_size,
        sha256=digest(path),
    )
    path.with_suffix(".provenance.json").write_text(json.dumps(recipe, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("survey", type=Path)
    parser.add_argument("portable", type=Path)
    parser.add_argument("--output", type=Path, default=DEST)
    args = parser.parse_args()
    cook(args.survey, args.portable, args.output)
