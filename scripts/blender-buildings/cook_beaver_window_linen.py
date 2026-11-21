"""Cook the two-sided Roman blind's measured light and installed geometry."""

import argparse
import json
import struct
from pathlib import Path

import numpy as np
from beaver_floor_gloss import SOURCES
from check_interior_containment import accessor, local_matrix, read_glb
from cook_beaver_fitted_finishes import encode
from cook_beaver_fitted_room import DEST, PUBLIC, digest
from PIL import Image, ImageFilter

FLAX = "Sun-filtering plainweave flax"
MATERIALS = (
    FLAX,
    "Dark woven cotton lift tape",
    "Unbleached lift cord",
    "Rubbed brass lift rings",
)


def geometry(source, parcel, path):
    doc, blob = read_glb(source)
    meshes = [[] for _ in MATERIALS]
    count = 0
    for node in doc["nodes"]:
        if "mesh" not in node:
            continue
        if node.get("children"):
            raise ValueError("Expected an unparented installed part")
        matrix = parcel @ local_matrix(node)
        transform = matrix[:3, :3]
        normal_basis = np.linalg.inv(transform).T
        for p in doc["meshes"][node["mesh"]]["primitives"]:
            material = doc["materials"][p["material"]]["name"]
            group = MATERIALS.index(material)
            attrs = p["attributes"]
            pos = accessor(doc, blob, attrs["POSITION"]).astype(float)
            normal = accessor(doc, blob, attrs["NORMAL"]).astype(float)
            positions = np.einsum("ij,kj->ki", transform, pos) + matrix[:3, 3]
            normals = np.einsum("ij,kj->ki", normal_basis, normal)
            normals /= np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-10)
            uv = (
                accessor(doc, blob, attrs["TEXCOORD_1"])
                if material == FLAX
                else np.zeros((len(pos), 2))
            )
            indices = accessor(doc, blob, p["indices"]).ravel()
            if np.linalg.det(transform) < 0:
                indices = indices.reshape(-1, 3)[:, [0, 2, 1]].ravel()
            if (
                any(not np.isfinite(v).all() for v in (positions, normals, uv))
                or np.min(uv) < -0.001
                or np.max(uv) > 1.001
            ):
                raise ValueError("Invalid installed garment coordinates")
            meshes[group].append((positions, normals, uv, indices))
            count += len(indices) // 3
    data = bytearray()
    views = []
    access = []
    primitives = []

    def field(raw, kind):
        raw = np.ascontiguousarray(raw)
        data.extend(b"\0" * (-len(data) % 4))
        view = {"buffer": 0, "byteOffset": len(data), "byteLength": raw.nbytes}
        views.append(view)
        data.extend(raw.tobytes())
        rec = {
            "bufferView": len(views) - 1,
            "count": len(raw),
            "componentType": 5125 if kind == "SCALAR" else 5126,
            "type": kind,
        }
        if kind == "VEC3":
            rec.update(min=raw.min(axis=0).tolist(), max=raw.max(axis=0).tolist())
        access.append(rec)
        return len(access) - 1

    materials = []
    for group, parts in enumerate(meshes):
        if not parts:
            raise ValueError(f"Missing {MATERIALS[group]}")
        pos, nor, uv, indices = zip(*parts)
        offsets = np.cumsum([0, *map(len, pos[:-1])])
        attrs = {
            name: field(np.concatenate(rows).astype("<f4"), kind)
            for name, rows, kind in (
                ("POSITION", pos, "VEC3"),
                ("NORMAL", nor, "VEC3"),
                ("TEXCOORD_0", uv, "VEC2"),
            )
        }
        ind = field(
            np.concatenate(
                [idx.astype(np.uint32) + off for idx, off in zip(indices, offsets)]
            ).astype("<u4"),
            "SCALAR",
        )
        primitives.append({"attributes": attrs, "indices": ind, "material": group})
        mat = next(m for m in doc["materials"] if m["name"] == MATERIALS[group])
        pbr = {
            k: v
            for k, v in mat["pbrMetallicRoughness"].items()
            if not k.endswith("Texture")
        }
        materials.append(
            {"name": MATERIALS[group], "doubleSided": True, "pbrMetallicRoughness": pbr}
        )
    model = {
        "asset": {"version": "2.0", "generator": "Earth Online rear window cooker"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "name": "Installed Roman blind"}],
        "meshes": [{"primitives": primitives}],
        "materials": materials,
        "accessors": access,
        "bufferViews": views,
        "buffers": [{"byteLength": len(data)}],
    }
    header = json.dumps(model, separators=(",", ":")).encode()
    header += b" " * (-len(header) % 4)
    data.extend(b"\0" * (-len(data) % 4))
    path.write_bytes(
        struct.pack(
            "<4sIII4s", b"glTF", 2, 28 + len(header) + len(data), len(header), b"JSON"
        )
        + header
        + struct.pack("<I4s", len(data), b"BIN\0")
        + data
    )
    return count


def cloth_transport(field):
    # Adjacent columns follow the same yarn course around the fold. Average
    # primarily across those columns, with a shorter radius across the pleats.
    # A local median guides the weights without transferring a bright window
    # rim across a shadow or into the separate header chart.
    padded = np.pad(field, ((1, 1), (3, 3), (0, 0)), mode="edge")
    h, w = field.shape[:2]
    median = np.median(
        [padded[y : y + h, x : x + w] for y in range(3) for x in range(0, 7, 2)], axis=0
    )
    field = np.minimum(field, np.maximum(4 * median, 0.001))
    guide = np.log1p(150 * np.mean(median, axis=2))
    guard = np.pad(guide, ((2, 2), (7, 7)), mode="edge")
    rgba = np.pad(field, ((2, 2), (7, 7), (0, 0)), mode="edge")
    result = np.zeros_like(field)
    total = np.zeros((h, w), np.float32)
    for y in range(-2, 3):
        for x in range(-7, 8):
            dy, dx = y + 2, x + 7
            delta = guard[dy : dy + h, dx : dx + w] - guide
            weight = np.exp(
                -0.5 * ((y / 0.84) ** 2 + (x / 3.4) ** 2 + (delta / 0.36) ** 2)
            )
            total += weight
            result += rgba[dy : dy + h, dx : dx + w] * weight[:, :, None]
    return result / total[:, :, None]


def weave_coordinates(source, page):
    # Store the source's arc length along the sewn cloth, measured independently
    # at every column. The hem turns back on itself five times. Projecting the
    # weave down the wall would stretch and reverse it at each return.
    doc, blob = read_glb(source)
    charts = []
    sample = (np.arange(512) + 0.5) / 512
    for name in ("Linen-covered headrail", "Five pleats in unbleached flax"):
        node = next(n for n in doc["nodes"] if n["name"] == name)
        attr = doc["meshes"][node["mesh"]]["primitives"][0]["attributes"]
        tex = accessor(doc, blob, attr["TEXCOORD_0"])
        uv = accessor(doc, blob, attr["TEXCOORD_1"])
        y = np.unique(uv[:, 1])
        rows = []
        for v in y:
            chosen = uv[:, 1] == v
            u = uv[chosen, 0]
            order = np.argsort(u)
            rows.append(np.interp(sample, u[order], tex[chosen, 1][order]))
        charts.append((y, np.asarray(rows)))
    y = np.concatenate([c[0] for c in charts])
    rows = np.vstack([c[1] for c in charts])
    if not np.all(np.diff(y) > 0):
        raise ValueError("Overlapping cloth and headrail charts")
    mapping = np.column_stack([np.interp(sample, y, column) for column in rows.T])
    lo, hi = float(mapping.min()), float(mapping.max())
    fixed = np.rint((mapping - lo) / (hi - lo) * 65535).astype(np.uint16)
    page[:, :, 1] = fixed >> 8
    page[:, :, 2] = fixed & 255
    u0, u1 = uv[:, 0].min(), uv[:, 0].max()
    x0, x1 = tex[:, 0].min(), tex[:, 0].max()
    scale = (x1 - x0) / (u1 - u0)
    return [float(scale), float(x0 - scale * u0), lo, hi]


def cook(folder, portable, linen, dest):
    survey = folder / "window-light.npz"
    meta = json.loads(survey.with_suffix(".provenance.json").read_text())
    base = json.loads((DEST / "fitted-room.provenance.json").read_text())
    recipe = json.loads((portable / "portable-dining.provenance.json").read_text())
    if (
        meta["sha256"] != digest(survey)
        or meta["geometry_sha256"] != digest(folder / meta["geometry"])
        or meta["parent_scene_sha256"] != recipe["source_scene_sha256"]
        or recipe["parts"]["fitted-room"]["sha256"] != base["source_glb_sha256"]
        or meta["house_sha256"] != base["house_sha256"]
        or meta["sources"] != [*SOURCES, "site-noon-sun"]
    ):
        raise ValueError("Geometry, house and calibrated sources must match the survey")
    with np.load(survey, allow_pickle=False) as source:
        light = source["light"]
    if (
        light.shape != (8, 2, 112, 512, 3)
        or not np.isfinite(light).all()
        or light.min() < 0
    ):
        raise ValueError("Invalid blind radiance")
    pages = np.full((3, 512, 512, 4), 255, np.uint8)
    for row in range(8):
        for side in range(2):
            # Average Monte Carlo noise in the unrolled cloth coordinates.
            # Reducing height in linear light retains the narrow fold shadows.
            bands = [
                np.asarray(
                    Image.fromarray(channel).resize((512, 64), Image.Resampling.BOX)
                )
                for channel in np.moveaxis(cloth_transport(light[row, side]), -1, 0)
            ]
            rgb = np.stack(bands, -1)
            pages[side, row * 64 : (row + 1) * 64, :, :3] = encode(rgb, srgb=side == 0)
    img = next(r for r in meta["credits"] if r["role"] == "albedo")
    if digest(linen) != img["sha256"]:
        raise ValueError("Incorrect flax photograph")
    scan = Image.open(linen).convert("L").resize((512, 512), Image.Resampling.LANCZOS)
    # The optical measurements already contain the broad fabric colour.
    # A high-frequency yarn ratio supplies the missing close-range detail.
    flat = np.asarray(scan.filter(ImageFilter.GaussianBlur(5)), float)
    yarn = np.asarray(scan, float) / np.maximum(flat, 1)
    pages[2, :, :, :3] = np.uint8(np.rint(np.clip(yarn * 0.5, 0, 1) * 255))[:, :, None]
    weave = weave_coordinates(folder / meta["geometry"], pages[2])
    dest.mkdir(exist_ok=True, parents=True)
    atlas = dest / "window-linen.png"
    model = dest / "window-linen.glb"
    Image.fromarray(pages.reshape(1536, 512, 4)).save(atlas, compress_level=9)
    triangles = geometry(
        folder / meta["geometry"],
        np.asarray(recipe["placement"]["parcel_from_assembled_gltf"]),
        model,
    )

    def asset(p):
        return {
            "url": "/" + (DEST.relative_to(PUBLIC) / p.name).as_posix(),
            "sha256": digest(p),
            "bytes": p.stat().st_size,
        }

    record = dict(
        **asset(model),
        maps=asset(atlas),
        house_sha256=base["house_sha256"],
        fitted_geometry_sha256=base["sha256"],
        source_scene_sha256=meta["source_scene_sha256"],
        source_glb_sha256=meta["geometry_sha256"],
        survey_sha256=meta["sha256"],
        design_image_sha256=meta["design_sha256"],
        source_sun=meta["site_sun_blender_xyz"],
        weave=weave,
        triangles=triangles,
        materials=list(MATERIALS),
        credits=meta["credits"],
        attribution=meta["attribution"],
        license=meta["license"],
        source_url=meta["source_url"],
        sources=meta["sources"],
        build="scripts/blender-buildings/cook_beaver_window_linen.py",
    )
    model.with_suffix(".provenance.json").write_text(
        json.dumps(record, indent=2) + "\n"
    )
    print(
        "Window triangles",
        triangles,
        "geometry",
        model.stat().st_size,
        "lighting",
        atlas.stat().st_size,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("survey_directory", "portable_directory", "linen_diffuse"):
        parser.add_argument(field, type=Path)
    parser.add_argument("--output", type=Path, default=DEST)
    args = parser.parse_args()
    cook(
        args.survey_directory, args.portable_directory, args.linen_diffuse, args.output
    )
