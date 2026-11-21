"""Filter the Cycles measurements for the portable furniture's dielectric materials."""

import argparse
import json
import math
from pathlib import Path

import numpy as np
from beaver_floor_gloss import SOURCES
from check_interior_containment import read_glb
from cook_beaver_fitted_room import DEST, PUBLIC, digest
from PIL import Image

ROUGHNESS = (0.035, 0.20, 0.38, 0.70)


def kernel(rough):
    # Split-sum GGX importance sampling about the reflected direction, over the
    # entire sphere. Both hemispheres matter on a rounded rail or ceramic rim.
    width, height = 128, 64
    az, el = np.meshgrid(
        ((np.arange(62) + 0.5) / 62 - 0.5) * 2 * np.pi,
        ((np.arange(30) + 0.5) / 30 - 0.5) * np.pi,
    )
    mirror = np.stack(
        [np.sin(az) * np.cos(el), np.cos(az) * np.cos(el), np.sin(el)], axis=-1
    ).reshape(-1, 3)
    t = np.stack([np.cos(az), -np.sin(az), az * 0], axis=-1).reshape(-1, 3)
    b = np.cross(mirror, t)
    idx = []
    weights = []
    for i in range(128):
        u = (i + 0.5) / 128
        h_cos = math.sqrt((1 - u) / (1 + (rough**4 - 1) * u))
        phi = 2 * np.pi * int(f"{i:07b}"[::-1], 2) / 128
        half = mirror * h_cos + (t * math.cos(phi) + b * math.sin(phi)) * math.sqrt(
            1 - h_cos * h_cos
        )
        l = 2 * h_cos * half - mirror
        weight = max(0, 2 * h_cos * h_cos - 1)
        if weight <= 0:
            continue
        x = (np.arctan2(l[:, 0], l[:, 1]) / (2 * np.pi) + 0.5) * width - 0.5
        y = np.clip(
            (np.arcsin(l[:, 2].clip(-1, 1)) / np.pi + 0.5) * height - 0.5, 0, height - 1
        )
        left = np.floor(x).astype(int)
        bottom = np.floor(y).astype(int)
        for i_x, wx in ((0, 1 - (x - left)), (1, x - left)):
            for i_y, wy in ((0, 1 - (y - bottom)), (1, y - bottom)):
                idx.append(
                    np.minimum(bottom + i_y, height - 1) * width + (left + i_x) % width
                )
                weights.append(wx * wy * weight)
    weights = np.stack(weights, axis=1)
    return np.stack(idx, axis=1), (weights / weights.sum(axis=1, keepdims=True)).astype(
        np.float32
    )


def encode(linear, srgb=False):
    q = np.sqrt(np.maximum(0, linear) / (1 + np.maximum(0, linear)))
    if srgb:
        q = np.where(q <= 0.0031308, 12.92 * q, 1.055 * q ** (1 / 2.4) - 0.055)
    return np.rint(255 * q).astype(np.uint8)


def denoise_transfer(light, widths=(128, 128, 128)):
    """Filter indirect Monte Carlo noise across each continuous UV chart.

    Direct fixture visibility guides the filter at the bowl rim, cloth folds
    and contact shadows. The measured direct component remains unfiltered.
    """
    result = light.copy()
    for x, width in zip(np.cumsum((0, *widths[:-1])), widths):
        height = light.shape[1]
        region = np.s_[:, :, x : x + width, :]
        chart = light[region].copy()
        direct = chart[7]
        chart[0] = np.maximum(0, chart[0] - direct)
        guide = np.log(0.025 + direct @ np.array([0.2126, 0.7152, 0.0722]))
        value = chart[:7]
        for step in (1, 2, 4):
            padded = np.pad(
                value,
                ((0, 0), (2 * step, 2 * step), (2 * step, 2 * step), (0, 0)),
                mode="edge",
            )
            guide_pad = np.pad(guide, 2 * step, mode="edge")
            total = np.zeros_like(value)
            weights = np.zeros_like(guide)
            for i, a in enumerate((1, 4, 6, 4, 1)):
                for j, b in enumerate((1, 4, 6, 4, 1)):
                    yy = slice(i * step, i * step + height)
                    xx = slice(j * step, j * step + width)
                    weight = (
                        a * b * np.exp(-0.5 * ((guide - guide_pad[yy, xx]) / 0.20) ** 2)
                    )
                    total += padded[:, yy, xx, :] * weight[None, :, :, None]
                    weights += weight
            value = (total / weights[None, :, :, None]).astype(np.float32)
        result[:7, :, x : x + width, :] = value
        result[0, :, x : x + width, :] += direct
    return result


def material_finishes(portable):
    doc, _ = read_glb(portable)
    materials = {}
    for node in doc["nodes"]:
        if "mesh" not in node:
            continue
        part = node.get("extras", {}).get("source_part", "")
        for primitive in doc["meshes"][node["mesh"]]["primitives"]:
            m = doc["materials"][primitive["material"]]
            coat = m.get("extensions", {}).get("KHR_materials_clearcoat", {})
            if m["name"] == "Waxed walnut stair handrail":
                group = "rail"
            elif part == "Hand-thrown mixing bowl":
                group = "bowl"
            elif part == "Folded striped kitchen linen":
                group = "linen"
            elif part.startswith("oak-dining-table - dining-board-"):
                group = "boards"
            elif part.startswith("dining-chair") and coat:
                group = "chairs"
            else:
                continue
            if any("Texture" in k for k in coat):
                raise ValueError("The fitted finish needs a textured coat")
            materials[m["name"]] = {
                "group": group,
                "coat": coat.get("clearcoatFactor", 0),
                "roughness": coat.get("clearcoatRoughnessFactor", 0),
            }
    if {v["group"] for v in materials.values()} != {
        "rail",
        "bowl",
        "boards",
        "chairs",
        "linen",
    }:
        raise ValueError("Incomplete material registration")
    return materials


def pack_reflections(pages, panos):
    for k, rough in enumerate(ROUGHNESS):
        index, weight = kernel(rough)
        for row, probes in enumerate(panos):
            for column, panorama in enumerate(probes):
                rad = np.einsum(
                    "ijk,ij->ik", panorama.reshape(-1, 3)[index], weight
                ).reshape(30, 62, 3)
                tile = np.pad(encode(rad), ((0, 0), (1, 1), (0, 0)), mode="wrap")
                tile = np.pad(tile, ((1, 1), (0, 0), (0, 0)), mode="edge")
                x = column * 64
                y = (row + 7 * (k % 2)) * 32
                pages[1 + k // 2, y : y + 32, x : x + 64, :3] = tile


def cook(source, portable, output):
    info = json.loads(source.with_suffix(".provenance.json").read_text())
    room = json.loads((DEST / "fitted-room.provenance.json").read_text())
    if (
        digest(source) != info["sha256"]
        or digest(portable) != room["source_glb_sha256"]
        or info["source_scene"]["sha256"] != room["source_scene_sha256"]
        or info["geometry_sha256"] != room["source_glb_sha256"]
        or info["house_sha256"] != room["house_sha256"]
        or info["sources"] != list(SOURCES)
    ):
        raise ValueError(
            "Lighting and geometry are from different fitted house sources"
        )
    with np.load(source, allow_pickle=False) as data:
        light = data["transfer"]
        panos = data["incident"]
    if (
        light.shape != (8, 128, 384, 3)
        or panos.shape != (7, 8, 64, 128, 3)
        or not all(np.isfinite(a).all() and a.min() >= 0 for a in (light, panos))
    ):
        raise ValueError("Invalid directional or diffuse lighting measurements")
    pages = np.zeros((3, 512, 512, 4), np.uint8)
    pages[:, :, :, 3] = 255
    for row, field in enumerate(denoise_transfer(light)):
        y = (row // 2) * 128
        x = (row % 2) * 256
        pages[0, y : y + 128, x : x + 256, :3] = encode(field[:, :256], srgb=True)
        # The last two panorama rows are unused. A sixty-four texel chart
        # follows the continuous folded cloth through its original UVs.
        linen = field[:, 256:].reshape(64, 2, 64, 2, 3).mean(axis=(1, 3))
        pages[1, 448:, row * 64 : (row + 1) * 64, :3] = encode(linen)
    pack_reflections(pages, panos)
    output.mkdir(parents=True, exist_ok=True)
    path = output / "fitted-finish-light.png"
    Image.fromarray(pages.reshape(1536, 512, 4)).save(path, compress_level=9)
    data = {
        "source": "Measured incident light at the walnut volute, stair flight, bowl and four dining-table positions, and surface irradiance baked on the registered ceramic, folded flax and boards.",
        "license": info["license"],
        "attribution": info["attribution"],
        "source_scene": info["source_scene"],
        "house_sha256": room["house_sha256"],
        "fitted_geometry_sha256": room["sha256"],
        "survey": {"file": source.name, "sha256": info["sha256"]},
        "probe_blender_xyz": info["probe_blender_xyz"],
        "table_parcel_bounds": info["table_parcel_bounds"],
        "roughness": ROUGHNESS,
        "materials": material_finishes(portable),
        "sources": SOURCES,
        "surface_only_source": "pendant-direct",
        "pages": [
            "sRGB-encoded diffuse transfer, 8 sources",
            "linear encoded GGX radiance, two narrow lobes and folded-linen transfer",
            "linear encoded GGX radiance, two broad lobes",
        ],
        "transfer_encoding": "sqrt(L/(1+L))",
        "samples": info["samples"],
        "build": "scripts/blender-buildings/cook_beaver_fitted_finishes.py",
        "url": "/" + (DEST / path.name).relative_to(PUBLIC).as_posix(),
        "bytes": path.stat().st_size,
        "sha256": digest(path),
    }
    path.with_suffix(".provenance.json").write_text(json.dumps(data, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("survey", type=Path)
    parser.add_argument("portable", type=Path, help="Registered fitted-room.glb")
    parser.add_argument("--output", type=Path, default=DEST)
    args = parser.parse_args()
    cook(args.survey, args.portable, args.output)
