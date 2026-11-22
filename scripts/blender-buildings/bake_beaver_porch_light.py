"""Measure direct light from the fitted opal and the old point across the entrance."""

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

GRID = (48, 64, 48)
MINIMUM = (4.30, -0.80, -1.85)
MAXIMUM = (7.12, 3.18, 0.56)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def surfaces(scene, minimum, maximum):
    corners = []
    radiating = []
    graph = bpy.context.evaluated_depsgraph_get()
    for original in scene.objects:
        if (
            original.type != "MESH"
            or original.hide_render
            or not original.visible_shadow
        ):
            continue
        # The swinging leaf receives the field in its current orientation.
        # The enclosing static plaster and metalwork delimit the exterior field.
        if original.name.startswith("front-door - ") or "glass-door@" in original.name:
            continue
        ob = original.evaluated_get(graph)
        bounds = np.array([list(ob.matrix_world @ Vector(p)) for p in ob.bound_box])
        if (bounds.max(0) < minimum).any() or (bounds.min(0) > maximum).any():
            continue
        mesh = ob.to_mesh()
        try:
            mesh.calc_loop_triangles()
            co = np.empty((len(mesh.vertices), 3), np.float32)
            mesh.vertices.foreach_get("co", co.ravel())
            matrix = np.asarray(ob.matrix_world)
            co = np.einsum("ij,nj->ni", matrix[:3, :3], co) + matrix[:3, 3]
            indices = np.empty((len(mesh.loop_triangles), 3), np.int32)
            mesh.loop_triangles.foreach_get("vertices", indices.ravel())
            tris = co[indices]
            use = (tris.min(1) <= maximum).all(1) & (tris.max(1) >= minimum).all(1)
            if not use.any():
                continue
            corners.append(tris[use])
            for i, slot in enumerate(original.material_slots):
                if slot.material and slot.material.name.startswith("porch-opal@"):
                    mats = np.empty(len(tris), np.int32)
                    mesh.loop_triangles.foreach_get("material_index", mats)
                    radiating.append(tris[(mats == i) & use])
        finally:
            ob.to_mesh_clear()
    if not corners or not any(len(tris) for tris in radiating):
        raise ValueError("The porch and its opal shade must be present")
    scene_tris = np.concatenate(corners).reshape(-1, 3)
    cells = np.arange(len(scene_tris), dtype=np.int32).reshape(-1, 3)
    tree = BVHTree.FromPolygons(scene_tris.tolist(), cells.tolist(), all_triangles=True)
    return tree, np.concatenate(radiating)


def sample_emitter(triangles, count):
    crosses = np.cross(
        triangles[:, 1] - triangles[:, 0], triangles[:, 2] - triangles[:, 0]
    )
    lengths = np.linalg.norm(crosses, axis=1)
    valid = lengths > 1e-12
    triangles, crosses, lengths = triangles[valid], crosses[valid], lengths[valid]
    cdf = np.cumsum(0.5 * lengths)
    area = float(cdf[-1])
    rng = np.random.default_rng(2119)
    for i in range(count):
        face = np.searchsorted(cdf, (i + 0.5) * area / count)
        u, v = rng.random(2)
        root = math.sqrt(u)
        point = np.array([1 - root, root * (1 - v), root * v]) @ triangles[face]
        yield point, crosses[face] / lengths[face], area / count


def visible(tree, point, directions, distances, indices, bias):
    origin = Vector(point)
    # A boundary receiver occupies part of a cell. Stop slightly short of it
    # so a wall does not shadow its own outward-facing sample.
    return np.fromiter(
        (
            tree.ray_cast(origin, Vector(directions[i]), max(0, distances[i] - bias))[0]
            is None
            for i in indices
        ),
        dtype=bool,
        count=len(indices),
    )


def bake(out):
    src = Path(bpy.data.filepath)
    meta = json.loads((src.parent / "porch-light.provenance.json").read_text())
    if digest(src) != meta["sha256"] or out.resolve().is_relative_to(
        Path(__file__).resolve().parents[2]
    ):
        raise ValueError("Use the fitted porch source and an external output directory")
    optical = src.parent.parent / "window-optics/optics-source.provenance.json"
    optical_meta = json.loads(optical.read_text())
    if optical_meta["source_sha256"] != meta["source_scene_sha256"]:
        raise ValueError("Inconsistent parcel registration")
    matrix = np.asarray(optical_meta["parcel_from_assembled_gltf"]) @ np.array(
        [[1, 0, 0, 0], [0, 0, 1, 0], [0, -1, 0, 0], [0, 0, 0, 1]]
    )
    grid = np.stack(
        np.meshgrid(
            *(np.linspace(a, b, n) for a, b, n in zip(MINIMUM, MAXIMUM, GRID)),
            indexing="ij",
        ),
        axis=-1,
    )
    inverse = np.linalg.inv(matrix)
    points = (
        np.einsum("ij,nj->ni", inverse[:3, :3], grid.reshape(-1, 3)) + inverse[:3, 3]
    )
    lamp = meta["fixture"]
    for centre in (lamp["centre"], lamp["previous_light_origin"]):
        parcel = (matrix @ [*centre, 1])[:3]
        if (parcel < MINIMUM).any() or (parcel > MAXIMUM).any():
            raise ValueError("The surveyed field must contain both lamp origins")
    tree, emitter = surfaces(
        bpy.context.scene, points.min(0) - 0.05, points.max(0) + 0.05
    )
    edges = np.cross(emitter[:, 1] - emitter[:, 0], emitter[:, 2] - emitter[:, 0])
    area = 0.5 * np.linalg.norm(edges, axis=1).sum()
    if not math.isclose(area, lamp["area_m2"], rel_tol=1e-4):
        raise ValueError("The surveyed bounds truncate the opal shade")
    field = np.zeros((len(points), 2))
    samples = 48
    for start, normal, area in sample_emitter(emitter, samples):
        difference = points - start
        distance = np.maximum(0.007, np.linalg.norm(difference, axis=1))
        direction = difference / distance[:, None]
        cosine = np.einsum("ni,i->n", direction, normal)
        indices = np.flatnonzero(cosine > 0)
        hit = indices[
            visible(tree, start + normal * 0.0015, direction, distance, indices, 0.035)
        ]
        field[hit, 0] += area * cosine[hit] / (distance[hit] ** 2)
    old = np.array(lamp["previous_light_origin"])
    delta = points - old
    distance = np.maximum(0.053, np.linalg.norm(delta, axis=1))
    hit = visible(
        tree, old, delta / distance[:, None], distance, np.arange(len(points)), 0.035
    )
    field[hit, 1] = 1 / (4 * math.pi * distance[hit] ** 2)
    if not np.isfinite(field).all() or np.any(field.max(0) <= 0):
        raise ValueError("Empty fixture transfer")
    out.mkdir(exist_ok=True, parents=True)
    np.save(out / "porch-transfer.npy", field.reshape((*GRID, 2)).astype(np.float32))
    rec = {
        "source_sha256": meta["sha256"],
        "parent_scene_sha256": meta["source_scene_sha256"],
        "house_sha256": optical_meta["house_sha256"],
        "calibration": optical_meta["canonical_source_to_game"],
        "new_centre": (matrix @ [*lamp["centre"], 1])[:3].tolist(),
        "old_centre": (matrix @ [*old, 1])[:3].tolist(),
        "grid": list(GRID),
        "min": list(MINIMUM),
        "max": list(MAXIMUM),
        "area_m2": lamp["area_m2"],
        "power_w": lamp["watts"],
        "radiance": lamp["radiance"],
        "colour": lamp["colour"],
        "samples": samples,
        "npy_sha256": digest(out / "porch-transfer.npy"),
    }
    (out / "porch-transfer.provenance.json").write_text(
        json.dumps(rec, indent=2) + "\n"
    )


if __name__ == "__main__":
    args = argparse.ArgumentParser()
    args.add_argument("out", type=Path)
    bake(args.parse_args(sys.argv[sys.argv.index("--") + 1 :]).out)
