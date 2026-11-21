"""Relate the metric room and its six separately placed chair seats to the game lot."""

import gzip
import json

from beaver_blender_scene import site_basis
from beaver_street_scene import load_surface
from mathutils import Matrix, Vector

# The Blender glTF exporter maps XYZ to X, Z, -Y.
CHANGE_UP = Matrix(((1, 0, 0, 0), (0, 0, 1, 0), (0, -1, 0, 0), (0, 0, 0, 1)))


def rows(matrix):
    return [list(r) for r in matrix]


def site_and_chairs(scene, repo, chairs, chair_pivot):
    with gzip.open(repo / "public/data/sf/sf-city.json.gz", "rt") as f:
        parcel = next(
            h
            for h in json.load(f)["detailBuildings"]
            if h["addr"] == "21 Beaver Street"
        )
    footprint = [Vector(p) for p in zip(parcel["points"][::2], parcel["points"][1::2])]
    scale = load_surface(repo, parcel["centroid"])["plan_scale"]
    site = site_basis(footprint, scale)[3]
    parcel_from_gltf = CHANGE_UP @ site.inverted() @ CHANGE_UP.inverted()
    frames = {
        chair: next(
            o.matrix_world.copy()
            for o in scene.objects[chair].children
            if o.name.endswith("hand-wrapped rush")
        )
        for chair in chairs
    }
    pivot = Matrix.Translation(Vector(chair_pivot))
    poses = {}
    for chair, frame in frames.items():
        relative = frame @ frames[chairs[0]].inverted()
        pose = CHANGE_UP @ relative @ pivot @ CHANGE_UP.inverted()
        poses[chair] = {
            "gltf_from_standalone": rows(pose),
            "parcel_from_standalone": rows(parcel_from_gltf @ pose),
        }
    return {
        "site_address": parcel["addr"],
        "units": "Row-major affine matrices, multiplying homogeneous column vectors. Y is up, lengths in metres. The game maps parcel coordinates to the surveyed site.",
        "parcel_from_assembled_gltf": rows(parcel_from_gltf),
        "prototype": "rush-chair.glb",
        "chairs": poses,
        "prototype_alignment": "Aligned on the six shared, physically wrapped seat meshes. The full assembly retains each original chair frame.",
    }
