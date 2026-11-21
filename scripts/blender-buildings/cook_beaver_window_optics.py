"""Pack the measured exterior triangles and Cycles light into a stackless ray tree."""

import argparse
import json
from hashlib import sha256
from pathlib import Path

import numpy as np
from PIL import Image

DEST = (
    Path(__file__).resolve().parents[2]
    / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"
)
MAGIC = int.from_bytes(b"BWO1", "little")
HEADER = 7
LEAF = 1 << 31


def digest(f):
    return sha256(f.read_bytes()).hexdigest()


def rgb9e5(values):
    rgb = np.maximum(np.asarray(values, dtype=np.float64), 0)
    peak = rgb.max(axis=-1)
    exponent = np.clip(np.floor(np.log2(np.maximum(peak, 2.0**-24))) + 16, 0, 31)
    scale = np.exp2(exponent - 24)
    exponent = np.minimum(exponent + (np.rint(peak / scale) >= 512), 31)
    mant = np.clip(np.rint(rgb / np.exp2(exponent - 24)[..., None]), 0, 511).astype(
        np.uint32
    )
    return (
        mant[..., 0]
        | mant[..., 1] << 9
        | mant[..., 2] << 18
        | exponent.astype(np.uint32) << 27
    )


def vertices(source, xyz, meta):
    # A shared corner can have a distinct reflection on either side of a frame.
    # Keep sharp faces and object boundaries separate when averaging bake noise.
    edges = np.cross(xyz[:, 1] - xyz[:, 0], xyz[:, 2] - xyz[:, 0])
    axis = np.argmax(np.abs(edges), axis=1)
    sign = edges[np.arange(len(axis)), axis] < 0
    object_id = np.repeat(
        np.arange(len(meta["groups"])), [n for _, n in meta["groups"]]
    )
    tag = 6 * object_id + 2 * axis + sign
    coordinates = np.ascontiguousarray(xyz.reshape(-1, 3), np.float32)
    keys = np.column_stack(
        (np.rint(coordinates * 10000).astype(np.int32), np.repeat(tag, 3))
    )
    _, first, back = np.unique(keys, axis=0, return_index=True, return_inverse=True)
    back = back.reshape(-1)
    pts = coordinates[first]
    count = np.bincount(back)
    vertex = np.zeros((len(pts), 8), dtype="<u4")
    vertex[:, :3] = pts.view(np.uint32)
    for sample in range(5):
        light = np.load(source / f"light-{sample}.npy", mmap_mode="r")
        if light.shape != (len(xyz), 3, 3) or not np.isfinite(light).all():
            raise ValueError("Invalid vertex lighting")
        flat = light.reshape(-1, 3)
        colors = np.column_stack(
            [np.bincount(back, weights=flat[:, c]) / count for c in range(3)]
        )
        gain = meta["canonical_source_to_game"] if sample < 4 else 1
        vertex[:, 3 + sample] = rgb9e5(colors * gain)
    return vertex, back.reshape(-1, 3).astype(np.uint32)


def bounds_cost(extent):
    x, y, z = extent.T
    return x * y + x * z + y * z


def ray_tree(xyz, indices, material_class):
    low = xyz.min(1)
    high = xyz.max(1)
    center = (low + high) * 0.5
    base = low.min(0) - 0.002
    step = (high.max(0) + 0.002 - base) / 65535.0
    chunks = []
    size = HEADER

    def build(ids):
        nonlocal size
        lo, hi = low[ids].min(0), high[ids].max(0)
        node = np.zeros((1, 4), np.uint32)
        qlo = np.clip(np.floor((lo - base) / step) - 1, 0, 65535).astype(np.uint32)
        qhi = np.clip(np.ceil((hi - base) / step) + 1, 0, 65535).astype(np.uint32)
        node[0, :3] = qlo | (qhi << 16)
        chunks.append(node)
        size += 1
        if len(ids) <= 8:
            chunks.append(
                np.column_stack(
                    (indices[ids], ids | (material_class[ids] << 27))
                ).astype(np.uint32)
            )
            size += len(ids)
            node[0, 3] = LEAF | size
            return
        points = center[ids]
        # Search binned surface-area splits on all three axes for large nodes.
        axis = int(np.argmax(np.ptp(points, axis=0)))
        cut = None
        if len(ids) > 48:
            best = float("inf")
            for dim in range(3):
                x = points[:, dim]
                first = x.min()
                span = x.max() - first
                if span < 1.0e-6:
                    continue
                cell = np.minimum(11, ((x - first) * (12.0 / span)).astype(np.int32))
                lower = np.full((12, 3), np.inf)
                upper = np.full((12, 3), -np.inf)
                np.minimum.at(lower, cell, low[ids])
                np.maximum.at(upper, cell, high[ids])
                left = np.maximum.accumulate(upper, axis=0) - np.minimum.accumulate(
                    lower, axis=0
                )
                right = (
                    np.maximum.accumulate(upper[::-1], axis=0)
                    - np.minimum.accumulate(lower[::-1], axis=0)
                )[::-1]
                n = np.cumsum(np.bincount(cell, minlength=12))[:-1]
                cost = n * bounds_cost(np.maximum(left[:-1], 0)) + (
                    len(ids) - n
                ) * bounds_cost(np.maximum(right[1:], 0))
                cost[(n == 0) | (n == len(ids))] = np.inf
                k = int(np.argmin(cost))
                if cost[k] < best:
                    best = cost[k]
                    cut = cell <= k
        if cut is None:
            mid = len(ids) // 2
            order = np.argpartition(points[:, axis], mid)
            groups = [ids[order[:mid]], ids[order[mid:]]]
        else:
            groups = [ids[cut], ids[~cut]]

        # The same two elevations receive every ray. Visit the nearer box first.
        def distance(group):
            lo = low[group].min(0)
            hi = high[group].max(0)
            eyes = np.array([[3.5, 5, 0], [3.5, 5, 13]])
            return np.min(np.sum((np.clip(eyes, lo, hi) - eyes) ** 2, axis=1))

        if distance(groups[1]) < distance(groups[0]):
            groups.reverse()
        for group in groups:
            build(group)
        node[0, 3] = size

    build(np.arange(len(indices), dtype=np.uint32))
    return np.concatenate(chunks), base.astype("<f4"), step.astype("<f4")


def cook(src, out, exterior_only=False):
    meta = json.loads((src / "optics-source.provenance.json").read_text())
    wall = json.loads((DEST / "wall-daylight.provenance.json").read_text())
    if (
        meta["source_sha256"] != wall["source_scene_sha256"]
        or meta["house_sha256"] != wall["house_sha256"]
        or meta["hours"] != [8.5, 12, 17, 22]
    ):
        raise ValueError("Window optics and house source differ")
    matrix = np.asarray(meta["parcel_from_assembled_gltf"]) @ np.array(
        [[1, 0, 0, 0], [0, 0, 1, 0], [0, -1, 0, 0], [0, 0, 0, 1]]
    )
    regions = [(src, meta)]
    room_record = None
    if not exterior_only:
        room_record = src / "rooms/optics-source.provenance.json"
        room = json.loads(room_record.read_text())
        for key in (
            "source_sha256",
            "house_sha256",
            "parcel_from_assembled_gltf",
            "hours",
            "canonical_source_to_game",
        ):
            if room[key] != meta[key]:
                raise ValueError("Room and street surveys differ on " + key)
        if room["region"] != "rooms":
            raise ValueError("Expected the indoor optical survey")
        regions.append((src / "rooms", room))
    # The upper four bits let the visible frame recover its own Cycles
    # diffuse return without accepting a neighbouring surface. Bit 27 identifies the moving leaf.
    finishes = {
        "oil-rubbed-wood": 1,
        "sash-putty": 2,
        "wrought-iron": 3,
        "front-door-paint": 4,
        "front-door-moulding": 5,
    }
    material_classes = []
    points = []
    corners = []
    lights = []
    offset = 0
    for directory, region in regions:
        for name, value in region["files"].items():
            if digest(directory / name) != value:
                raise ValueError("Changed optical survey " + str(directory / name))
        positions = np.load(directory / "triangles.npy")
        positions = (positions @ matrix[:3, :3].T + matrix[:3, 3]).astype(np.float32)
        if not np.isfinite(positions).all():
            raise ValueError("Invalid optical geometry")
        material_classes.append(
            np.repeat(
                [
                    (finishes.get(name.split(" - ")[-1].split("@")[0], 0) << 1)
                    | int(name.startswith("front-door - "))
                    for name, n in region["groups"]
                ],
                [n for name, n in region["groups"]],
            ).astype(np.uint32)
        )
        lit, ids = vertices(directory, positions, region)
        points.append(positions)
        corners.append(ids + offset)
        lights.append(lit)
        offset += len(lit)
    xyz = np.concatenate(points)
    indices = np.concatenate(corners)
    vertex = np.concatenate(lights)
    nodes, base, step = ray_tree(xyz, indices, np.concatenate(material_classes))
    skies = np.load(src / "sky.npy")
    if skies.shape != (5, 80, 160, 3) or not np.isfinite(skies).all():
        raise ValueError("Invalid incident sky")
    skies[:4] *= meta["canonical_source_to_game"]
    environment = rgb9e5(skies).ravel()
    vstart = HEADER + len(nodes)
    sstart = vstart + 2 * len(vertex)
    head = np.zeros((HEADER, 4), dtype="<u4")
    head[0] = [MAGIC, vstart, sstart, len(vertex)]
    head[1, :3] = base.view(np.uint32)
    head[2, :3] = step.view(np.uint32)
    head[1, 3] = 160
    head[2, 3] = 80
    for i, row in enumerate(np.linalg.inv(matrix)[:3]):
        head[3 + i] = row.astype("<f4").view("<u4")
    head[6] = [
        HEADER,
        len(indices),
        5,
        len(environment) | (0 if exterior_only else LEAF),
    ]
    blocks = np.concatenate(
        [head, nodes, vertex.reshape(-1, 4), environment.reshape(-1, 4)]
    ).astype("<u4")
    payload = blocks.tobytes()
    # PNG is lossless byte storage for the GPU records, independent of host compression libraries at load.
    width = 1024
    height = (len(payload) + 4 * width - 1) // (4 * width)
    raw = np.zeros(4 * width * height, np.uint8)
    raw[: len(payload)] = np.frombuffer(payload, np.uint8)
    out.mkdir(exist_ok=True, parents=True)
    f = out / "window-optics.png"
    Image.fromarray(raw.reshape(height, width, 4)).save(f, compress_level=6)
    record = {
        "url": "/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/"
        + f.name,
        "sha256": digest(f),
        "bytes": f.stat().st_size,
        "house_sha256": meta["house_sha256"],
        "source_scene_sha256": meta["source_sha256"],
        "survey_sha256": digest(src / "optics-source.provenance.json"),
        "height": height,
        "width": width,
        "payload_bytes": len(payload),
        "triangles": len(indices),
        "vertices": len(vertex),
        "hours": meta["hours"],
        "sky_dimensions": [160, 80],
        "includes_rooms": not exterior_only,
        "room_survey_sha256": digest(room_record) if room_record else None,
        "source_notices": [
            "public/assets/buildings/beaver-terrace/terrace.provenance.json",
            "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/street-garden.provenance.json",
            "public/data/sf/sf-footprints.NOTICE.txt",
            "public/data/sf/sf-elevation.NOTICE.txt",
            "scripts/blender-buildings/sources/beaver-parcel.NOTICE.txt",
            "THIRD_PARTY_NOTICES.md",
        ],
        "encoding": "BWO1",
        "attribution": "Earth Online contributors. Source mesh and lighting of the furnished 21 Beaver Street model. Sources for street and neighbouring geometry are recorded in the street context and garden provenance.",
        "license": "Project-authored geometry and light measurements, MIT. Existing surveyed city inputs retain their recorded licences.",
        "build": "scripts/blender-buildings/cook_beaver_window_optics.py",
    }
    (out / "window-optics.provenance.json").write_text(
        json.dumps(record, indent=2) + "\n"
    )
    print(
        "Cooked",
        len(indices),
        "triangles,",
        len(vertex),
        "lit vertices,",
        len(payload),
        "GPU bytes",
        flush=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("--output", type=Path, default=DEST)
    parser.add_argument(
        "--exterior-only",
        action="store_true",
        help="Cook the street and courtyard measurements on their own",
    )
    args = parser.parse_args()
    cook(args.source, args.output, args.exterior_only)
