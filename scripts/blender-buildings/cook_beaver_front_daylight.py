"""Project measured incident daylight onto the three diffuse spherical-harmonic bands."""

import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def basis(n):
    x, y, z = n.T
    return np.column_stack(
        (np.ones_like(z), x, y, z, x * y, y * z, 3 * z * z - 1, x * z, x * x - y * y)
    )


def cook(source, output):
    record = json.loads((source / "front-daylight.provenance.json").read_text())
    raw = source / record["panoramas"]
    parent = json.loads((DEST / "street-garden.provenance.json").read_text())
    if (
        digest(raw) != record["panoramas_sha256"]
        or record["parent_scene_sha256"] != parent["source_scene_sha256"]
        or digest(source / record["scene"]) != record["scene_sha256"]
    ):
        raise ValueError("The incident survey or source house has changed")
    L = np.load(raw)["radiance"].astype(np.float64)
    if L.shape != (7, 6, 64, 128, 3) or not np.isfinite(L).all() or np.any(L < 0):
        raise ValueError("Invalid scene-linear panorama")
    h, w = L.shape[2:4]
    az = (np.arange(w) + 0.5) * math.tau / w - math.pi
    edges = np.linspace(-math.pi / 2, math.pi / 2, h + 1)
    el = 0.5 * (edges[:-1] + edges[1:])
    a, e = np.meshgrid(az, el)
    n = np.stack(
        [np.sin(a) * np.cos(e), np.cos(a) * np.cos(e), np.sin(e)], axis=-1
    ).reshape(-1, 3)
    omega = np.repeat(np.diff(np.sin(edges)) * math.tau / w, w)
    # Orthonormal real SH, l <= 2. Convolution of incoming radiance with the
    # normalized clamped cosine multiplies these bands by 1, 2/3 and 1/4.
    C = np.array(
        [
            0.28209479177387814,
            *([0.4886025119029199] * 3),
            1.0925484305920792,
            1.0925484305920792,
            0.31539156525252005,
            1.0925484305920792,
            0.5462742152960396,
        ]
    )
    A = np.array([1, *([2 / 3] * 3), *([0.25] * 5)])
    kernel = omega[:, None] * basis(n) * (C * C * A)
    coeff = np.einsum(
        "pnc,nk->pkc", L.reshape(-1, h * w, 3), kernel, optimize=True
    ).reshape(7, 6, 9, 3)
    # Check the integration against a uniform full sphere and independent normals.
    flat = np.array([[0, 0, 1.0], [0, 0, -1.0], [1.0, 0, 0]])
    if np.max(np.abs(basis(flat) @ kernel.sum(0) - 1)) > 0.001:
        raise ValueError("Full-sphere integration is not energy-normalized")
    i = np.arange(128) + 0.5
    z = 1 - 2 * i / 128
    angle = i * math.pi * (3 - math.sqrt(5))
    test = np.column_stack(
        (np.sqrt(1 - z * z) * np.cos(angle), np.sqrt(1 - z * z) * np.sin(angle), z)
    )
    cosine = np.maximum(0, np.einsum("ic,jc->ij", test, n)) * omega / math.pi
    quadrature = np.einsum("ij,pjc->pic", cosine, L.reshape(42, h * w, 3))
    predicted = np.einsum("ik,pkc->pic", basis(test), coeff.reshape(42, 9, 3))
    relative = abs(predicted - quadrature) / (
        quadrature.mean(1, keepdims=True) + 1.0e-6
    )
    coeff[:-1] *= record["canonical_source_to_game"]
    output.mkdir(parents=True, exist_ok=True)
    binary = output / "front-daylight.bin"
    binary.write_bytes(coeff.astype("<f4").tobytes())
    proof = {
        "url": f"/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/{binary.name}",
        "sha256": digest(binary),
        "bytes": binary.stat().st_size,
        "garden_sha256": parent["sha256"],
        "source_scene_sha256": record["scene_sha256"],
        "panoramas_sha256": record["panoramas_sha256"],
        "solar_table_sha256": record["solar_table_sha256"],
        "hours": record["source_hours"],
        "across": record["probe_across"],
        "depth": record["probe_depth"],
        "height_above_grade_m": record["height_above_grade_m"],
        "coefficient_layout": "7 skies, 6 probes, 9 polynomial terms, RGB f32 little endian",
        "relative_quadrature_error_p95": float(np.percentile(relative, 95)),
        "attribution": "Earth Online. Scene-linear measurements of the assembled 21 Beaver house in Blender Cycles.",
        "license": "MIT for the project-authored measurements. The fitted garden uses CC0 1.0 photographs credited in street-garden.provenance.json.",
        "build": "scripts/blender-buildings/cook_beaver_front_daylight.py",
    }
    (output / "front-daylight.provenance.json").write_text(
        json.dumps(proof, indent=2) + "\n"
    )


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("source", type=Path)
    cli.add_argument("--output", type=Path, default=DEST)
    args = cli.parse_args()
    cook(args.source, args.output)
