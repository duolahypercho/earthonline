"""Make registered pigment, relief and roughness maps for the private front bed."""

import argparse
import json
from pathlib import Path

import numpy as np
from cook_beaver_fitted_room import PARENT, digest
from PIL import Image, ImageDraw, ImageFilter

SIZE = 512
PLANT = "Agave wax, fibres and seed"
CURB = "Weathered planter render"
SOIL = "Granite mulch with fines"
MATERIALS = (PLANT, CURB, SOIL)


def foliage():
    """Four living leaf faces and a band of dry leaf, straw, seed and spine surfaces."""
    rng = np.random.default_rng(210821)
    y, x = np.mgrid[:SIZE, :SIZE]
    col = x // 128
    u = (x % 128) / 127
    t = np.clip(1 - (y - 3) / 374, 0, 1)
    v = 2 * u - 1
    phase = col * 1.93
    noise = rng.normal(0, 1, x.shape)
    dust = np.sin(7 * v + 9 * t + phase) * np.cos(16 * t - v) + 0.5 * np.sin(
        22 * v + 8 * t
    )
    fibre = np.sin(131 * v + 2.5 * np.sin(5 * t + phase) + 3 * t * v)
    # Wax is interrupted by impressions from neighboring leaves in the unopened bud.
    scar = np.exp(
        -(((np.abs(v) - (0.48 + 0.30 * np.sin(6.1 * t + phase))) / 0.021) ** 2)
    )
    scar *= np.clip((t - 0.09) * 5, 0, 1) * np.clip((0.91 - t) * 6, 0, 1)
    ribs = np.exp(-((v / 0.095) ** 2))
    edge = np.clip((abs(v) - 0.897) * 12, 0, 1)
    colors = np.array(
        [[119, 139, 127], [143, 160, 150], [130, 144, 121], [99, 124, 104]]
    )
    color = colors[col].astype(float)
    color += (20 * t + 8.5 * dust + 1.8 * fibre + 1.5 * noise + 24 * scar + 4 * ribs)[
        ..., None
    ]
    base = np.clip((0.27 - t) * 3, 0, 1)[..., None]
    color = color * (1 - 0.43 * base) + np.array([128, 128, 80]) * 0.43 * base
    color = (
        color * (1 - edge[..., None] * 0.70)
        + np.array([166, 147, 91]) * edge[..., None] * 0.70
    )
    rust = np.clip((t - 0.88) * 7.8, 0, 1) ** 2
    color = color * (1 - rust[..., None]) + np.array([69, 48, 37]) * rust[..., None]
    rough = np.clip(163 + 10 * dust + 21 * scar + 24 * edge + 2.5 * noise, 80, 245)
    relief = 0.015 * fibre + 0.048 * scar + 0.035 * ribs + 0.008 * noise
    grass = y >= 384
    band = x // 64
    along = np.clip((511 - y) / 127, 0, 1)
    palette = np.array(
        [
            [81, 90, 53],
            [128, 126, 77],
            [177, 157, 103],
            [190, 169, 121],
            [131, 111, 71],
            [108, 86, 57],
            [87, 69, 48],
            [59, 44, 35],
        ]
    )
    strands = np.sin((x % 64) * 1.30 + 1.3 * np.sin(along * 5 + band))
    dry = (
        palette[band] + (12 * along + 2.6 * strands + 6 * dust + 2.4 * noise)[..., None]
    )
    color[grass] = dry[grass]
    rough[grass] = (204 + 8 * strands + 2 * noise)[grass]
    relief[grass] = (0.042 * strands + 0.009 * noise)[grass]
    dy, dx = np.gradient(relief)
    vec = np.stack((-2.8 * dx, 2.8 * dy, np.ones_like(dx)), axis=-1)
    vec /= np.linalg.norm(vec, axis=-1, keepdims=True)
    return (
        color,
        (vec * 0.5 + 0.5) * 255,
        np.stack((np.full_like(rough, 255), rough, np.zeros_like(rough)), axis=-1),
    )


def survey(inputs, recipe, asset=None):
    src = json.loads(recipe.read_text())
    if asset:
        src["inputs"] = [record for record in src["inputs"] if record["asset"] == asset]
        src["source"] = [url for url in src["source"] if url.endswith("/" + asset)]
        if not src["inputs"]:
            raise ValueError(f"Missing material survey for {asset}")
    maps = {}
    for record in src["inputs"]:
        path = inputs / record["url"].rsplit("/", 1)[-1]
        if digest(path) != record["sha256"]:
            raise ValueError(f"Changed survey input {path.name}")
        with Image.open(path) as image:
            maps[record["role"]] = np.asarray(
                image.convert("RGB").resize((SIZE, SIZE), Image.Resampling.LANCZOS),
                float,
            )
    return maps, src


def prepare(inputs, out):
    out.mkdir(parents=True, exist_ok=True)
    concrete, c = survey(inputs, PARENT / "apron-concrete.provenance.json")
    gravel, g = survey(inputs, PARENT.parent.parent / "aggregate.provenance.json")
    topsoil, d = survey(inputs, PARENT / "yard-earth.provenance.json", "park_dirt")
    albedo = concrete["albedo"]
    mineral = gravel["albedo"]
    y, x = np.mgrid[:SIZE, :SIZE] / SIZE
    streak = (
        0.5 + 0.5 * np.sin(2 * np.pi * x * 7 + 1.3 * np.sin(2 * np.pi * x * 2))
    ) ** 7
    below = (
        0.56 + 0.050 * np.sin(2 * np.pi * x * 2) + 0.012 * np.sin(2 * np.pi * x * 13)
    )
    # UV zero is the cap. The upper damp line varies along the face.
    penetration = np.clip((y - below) * 6, 0, 1) * np.clip((0.985 - y) * 11, 0, 1)
    tide = penetration * (0.60 + 0.4 * streak)
    brightness = albedo.mean(2) - np.median(albedo)
    grit = mineral.mean(2) - np.median(mineral)
    cast = (
        np.array([169, 159, 140])
        + (0.88 * brightness + 0.36 * grit - 21 * tide - 5 * streak * y)[..., None]
    )
    # Pockets left by the coarse aggregate show chipped rims on the exposed cement.
    rng = np.random.default_rng(412)
    pits = Image.new("L", (SIZE, SIZE))
    pen = ImageDraw.Draw(pits)
    for i in range(950):
        cx, cy = rng.uniform(0, SIZE, 2)
        radius = rng.uniform(0.65, 2.1) if i > 150 else rng.uniform(2.0, 6.8)
        angle = rng.uniform(0, 2 * np.pi)
        ring = []
        for j in range(7):
            a = angle + j * (2 * np.pi / 7)
            r = radius * rng.uniform(0.46, 1.18)
            ring.append((cx + np.cos(a) * r, cy + 1.4 * np.sin(a) * r))
        pen.polygon(ring, fill=int(rng.uniform(77, 245)))
    cavity = np.asarray(pits.filter(ImageFilter.GaussianBlur(0.48)), float) / 255
    dy, dx = np.gradient(cavity)
    cast += (39 * (dy - dx) - 55 * cavity)[..., None]
    cast += np.array([17, 17, 19])
    cast -= tide[..., None] * np.array([6, 5, 1])
    normal = concrete["normal"] / 127.5 - 1
    normal[..., 0] += dx * 1.9
    normal[..., 1] -= dy * 2.2
    normal /= np.maximum(0.0001, np.linalg.norm(normal, axis=-1, keepdims=True))
    cast_normal = (0.5 + 0.5 * normal) * 255
    earth = topsoil["albedo"]
    earth_l = earth.mean(2) - np.median(earth)
    fines = (
        np.array([116, 100, 78])
        + (1.17 * grit + 0.68 * earth_l)[..., None]
        + 0.26 * (earth - earth.mean(2)[..., None])
    )
    results = [
        foliage(),
        (cast, cast_normal, concrete["arm"]),
        (fines, gravel["normal"], gravel["arm"]),
    ]
    rec = []
    for name, maps in zip(MATERIALS, results):
        files = {}
        for channel, val in zip(("pigment", "normal", "orm"), maps):
            path = out / f"{name} - {channel}.png"
            Image.fromarray(np.rint(np.clip(val, 0, 255)).astype("uint8")).save(path)
            files[channel] = {"file": path.name, "sha256": digest(path)}
        rec.append({"name": name, "maps": files})
    credits = [*c["inputs"], *g["inputs"], *d["inputs"]]
    (out / "garden-surfaces.json").write_text(
        json.dumps(
            {
                "materials": rec,
                "inputs": credits,
                "attribution": sorted(
                    set(c["attribution"] + g["attribution"] + d["attribution"])
                ),
                "source": sorted(set(c["source"] + g["source"] + d["source"])),
                "license": "Project-authored pigment, leaf normals and geometry. Mineral and earth photographs CC0-1.0.",
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("inputs", type=Path)
    cli.add_argument("out", type=Path)
    args = cli.parse_args()
    prepare(args.inputs, args.out)
