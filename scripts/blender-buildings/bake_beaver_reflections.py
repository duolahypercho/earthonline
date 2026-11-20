"""Bake the kitchen reflection atlas from the shipped, lightmapped house.
Run with blender --background --factory-startup --python this_file.
The four octahedral tiles contain incident daylight and fixture radiance at
two appliance positions. Day alpha is visibility to the outdoor environment.
"""
import hashlib
import json
import math
import sys
import struct
import zlib
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

OUT = Path(__file__).resolve().parents[2] / "public/assets/buildings/beaver-terrace/finishes"
options = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
if "--house-furniture" in options:
    OUT = OUT / "patina/furniture"
elif "--house-patina" in options:
    OUT = OUT / "patina"
SOURCE = OUT / "terrace.glb"
SIZE = 256
# Across, height, depth, in metres. Each volume surrounds one appliance and
# its fittings. The sample is in the air just beyond its hardware.
PROBES = [
    {"at": [1.414, 0.89, 11.68], "min": [1.16, 0, 11.57], "max": [2.24, 2.17, 12.85]},
    {"at": [1.19, 1.11, 9.80], "min": [0.06, 0, 9.04], "max": [1.24, 1.37, 10.48]},
]

if "--house-furniture" in options:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from beaver_dimensions import furniture_point
    for probe, name in zip(PROBES, ("enamel-refrigerator", "oven-range")):
        for key in probe:
            probe[key] = furniture_point(name, probe[key])

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=str(SOURCE))
vertices, triangles, transfer, colors, emission = [], [], [], [], []
for obj in bpy.context.scene.objects:
    if obj.type != "MESH":
        continue
    mesh = obj.data
    mat = mesh.materials[0]
    if mat.name.startswith("glass"):
        continue
    shader = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    albedo = np.array(shader.inputs["Base Color"].default_value[:3])
    albedo *= 1 - shader.inputs["Metallic"].default_value
    lamp = np.array(shader.inputs["Emission Color"].default_value[:3]) * shader.inputs["Emission Strength"].default_value
    uv = mesh.uv_layers.active.data
    mesh.calc_loop_triangles()
    first = len(vertices)
    vertices.extend(obj.matrix_world @ v.co for v in mesh.vertices)
    for tri in mesh.loop_triangles:
        triangles.append(tuple(first + i for i in tri.vertices))
        transfer.append([(uv[i].uv.x, 1 - uv[i].uv.y) for i in tri.loops])
        colors.append(albedo)
        emission.append(lamp)

tree = BVHTree.FromPolygons(vertices, triangles, all_triangles=True)
points = np.asarray(vertices)[np.asarray(triangles)]
transfer, colors, emission = map(np.asarray, (transfer, colors, emission))
# A tangent basis for recovering interpolated corner lighting at the exact hit.
a = points[:, 1] - points[:, 0]
b = points[:, 2] - points[:, 0]
cross = np.cross(a, b)
area2 = np.maximum(np.sum(cross * cross, axis=1), 1e-24)[:, None]
grad1 = np.cross(b, cross) / area2
grad2 = np.cross(cross, a) / area2

def srgb(value):
    v = np.clip(value, 0, 1)
    return np.rint(255 * np.where(v <= 0.0031308, 12.92 * v, 1.055 * v ** (1/2.4) - 0.055)).astype(np.uint8)

def hemisphere_directions(dx, dy):
    u, v = np.meshgrid((np.arange(SIZE) + dx) * (2 / SIZE) - 1,
                       (np.arange(SIZE) + dy) * (2 / SIZE) - 1)
    h = 1 - abs(u) - abs(v)
    x = np.where(h < 0, (1 - abs(v)) * np.sign(u), u)
    z = np.where(h < 0, (1 - abs(u)) * np.sign(v), v)
    # Model (x, height, depth) to Blender.
    direction = np.stack([x, -z, h], axis=-1).reshape(-1, 3)
    return direction / np.linalg.norm(direction, axis=1)[:, None]

atlas = np.full((4*SIZE, 2*SIZE, 4), 255, dtype=np.uint8)
# The second page carries diffuse reflectance and hit normals, in linear
# unorm. The game can light these with its current ambient and sky field.
face_normals = cross / np.maximum(np.linalg.norm(cross, axis=1)[:, None], 1e-12)
for row, probe in enumerate(PROBES):
    x, h, z = probe["at"]
    origin = Vector((x, -z, h))
    daylight, fixture = [np.zeros((SIZE*SIZE, 3)) for _ in range(2)]
    outside = np.zeros(SIZE*SIZE)
    diffuse, normals = np.zeros_like(daylight), np.zeros_like(daylight)
    for dx, dy in [(0.25,0.25), (0.75,0.25), (0.25,0.75), (0.75,0.75)]:
        faces = np.full(SIZE*SIZE, -1)
        hits = np.zeros_like(daylight)
        directions = hemisphere_directions(dx, dy)
        for i, direction in enumerate(directions):
            p, _, face, _ = tree.ray_cast(origin, Vector(direction), 50)
            if face is not None:
                faces[i], hits[i] = face, p
        mask = faces >= 0
        outside += ~mask
        f = faces[mask]
        diffuse[mask] += colors[f]
        hit_normals = face_normals[f].copy()
        hit_normals[np.sum(hit_normals * directions[mask], axis=1) > 0] *= -1
        normals[mask] += hit_normals[:, [0, 2, 1]] * (1, 1, -1)
        delta = hits[mask] - points[f, 0]
        w1 = np.sum(delta * grad1[f], axis=1)
        w2 = np.sum(delta * grad2[f], axis=1)
        weights = np.stack([1 - w1 - w2, w1, w2], axis=1)
        light = np.clip(np.einsum("ij,ijk->ik", weights, transfer[f]), 0, 1) ** 2
        daylight[mask] += colors[f] * light[:, :1]
        fixture[mask] += colors[f] * light[:, 1:] * (1.55, 1.05, 0.56) + emission[f]
    tile = atlas[row*SIZE:(row+1)*SIZE]
    tile[:, :SIZE, :3] = srgb(daylight.reshape(SIZE, SIZE, 3) / 4)
    tile[:, :SIZE, 3] = np.rint(outside.reshape(SIZE, SIZE) * 255/4).astype(np.uint8)
    # Allow bright reflections of the bulb up to four times unit radiance.
    tile[:, SIZE:, :3] = srgb(fixture.reshape(SIZE, SIZE, 3) / 16)

    surface = atlas[(row+2)*SIZE:(row+3)*SIZE]
    surface[:, :SIZE, :3] = np.rint(255*np.clip(diffuse.reshape(SIZE,SIZE,3)/4,0,1)).astype(np.uint8)
    normal = normals / np.maximum(np.linalg.norm(normals, axis=1)[:, None], 1e-8)
    surface[:, SIZE:, :3] = np.rint((normal.reshape(SIZE,SIZE,3)*0.5 + 0.5)*255).astype(np.uint8)

def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))

path = OUT / "kitchen-reflections.png"
raw = b"".join(b"\0" + row.tobytes() for row in atlas)
header = struct.pack(">IIBBBBB", 2*SIZE, 4*SIZE, 8, 6, 0, 0, 0)
content = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
path.write_bytes(content)
(OUT / "kitchen-reflections.provenance.json").write_text(json.dumps({
    "source": "scripts/blender-buildings/bake_beaver_reflections.py, ray cast against terrace.glb",
    "house_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
    "license": "MIT", "attribution": "Earth Online contributors",
    "url": "/" + path.relative_to(Path(__file__).resolve().parents[2] / "public").as_posix(),
    "sha256": hashlib.sha256(content).hexdigest(), "bytes": len(content), "probes": PROBES
}, indent=2) + "\n")
