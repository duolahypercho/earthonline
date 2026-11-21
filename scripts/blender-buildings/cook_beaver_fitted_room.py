"""Cook the fitted Blender dining export into native house geometry and material pages."""

import argparse
import hashlib
import io
import json
import shutil
import struct
from pathlib import Path

import numpy as np
from check_interior_containment import accessor, local_matrix, read_glb
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
PUBLIC = ROOT / "public"
PARENT = PUBLIC / "assets/buildings/beaver-terrace/finishes/patina/furniture/photo"
DEST = PARENT / "fitted-dining"
SIZE = 512


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def reference(path):
    return {
        "url": "/" + path.relative_to(PUBLIC).as_posix(),
        "bytes": path.stat().st_size,
        "sha256": digest(path),
    }


def cook(portable, lighting=None):
    recipe = json.loads((portable / "portable-dining.provenance.json").read_text())
    room = portable / recipe["parts"]["fitted-room"]["file"]
    if digest(room) != recipe["parts"]["fitted-room"]["sha256"]:
        raise ValueError("Furnished source does not match its recorded SHA-256")
    layout = json.loads(
        (Path(__file__).with_name("beaver_dining_layout.json")).read_text()
    )
    if recipe["design_image_sha256"] != layout["design_image_sha256"]:
        raise ValueError("Unexpected source design")
    target = json.loads((PARENT / "terrace.provenance.json").read_text())
    surveys = (
        lighting_surveys(lighting, recipe["source_scene_sha256"], target["sha256"])
        if lighting
        else []
    )
    gltf, blob = read_glb(room)
    parcel = np.array(recipe["placement"]["parcel_from_assembled_gltf"])
    DEST.mkdir(parents=True, exist_ok=True)
    textures = gltf["textures"]
    channels = []
    surfaces = []
    primitives = set()
    lights = []
    boxes = []
    nodes = []
    meshes = {}

    def inside(p):
        return (parcel @ np.array([*p, 1]))[:3]

    # The original ground-floor boards already own their runtime reflection UVs
    # and the floor substrate. Keep them, and replace only the furnishing nodes.
    for src in gltf["nodes"]:
        transform = local_matrix(src)
        srcname = src.get("extras", {}).get("source_part", "")
        if src.get("children"):
            raise ValueError("Expected evaluated, parent-free source parts")
        if "mesh" not in src:
            if "KHR_lights_punctual" in src.get("extensions", {}):
                light = gltf["extensions"]["KHR_lights_punctual"]["lights"][
                    src["extensions"]["KHR_lights_punctual"]["light"]
                ]
                lights.append(
                    {
                        "name": srcname,
                        "parcel_xyz": inside(transform[:3, 3]).tolist(),
                        "type": light["type"],
                        "color": light["color"],
                        "intensity": light["intensity"],
                    }
                )
            continue
        if srcname.startswith("ground-storey - "):
            continue
        source_mesh = src["mesh"]
        if source_mesh not in meshes:
            meshes[source_mesh] = len(meshes)
        primitives.update(
            pr["material"] for pr in gltf["meshes"][source_mesh]["primitives"]
        )
        nodes.append(
            {
                "name": srcname,
                "mesh": meshes[source_mesh],
                "matrix": (parcel @ transform).T.ravel().tolist(),
            }
        )

    def image_id(texture):
        if not texture:
            return None
        if texture.get("texCoord", 0) != 0 or texture.get("extensions"):
            raise ValueError("The selected material needs additional UV coordinates")
        return textures[texture["index"]]["source"]

    mat_indices = {}
    cooked_mats = []
    for old in sorted(primitives):
        m = gltf["materials"][old]
        mat_indices[old] = len(cooked_mats)
        p = m["pbrMetallicRoughness"]
        slots = tuple(
            image_id(t)
            for t in [
                p.get("baseColorTexture"),
                m.get("normalTexture"),
                p.get("metallicRoughnessTexture"),
            ]
        )
        layer = None
        if any(i is not None for i in slots):
            if slots not in channels:
                channels.append(slots)
            layer = channels.index(slots)
        surfaces.append(
            {
                "name": m["name"],
                "layer": layer,
                "normal_strength": m.get("normalTexture", {}).get("scale", 1),
                "garden": m["name"].startswith("Garden "),
                "thin": (
                    "Roman blind" in m["name"] or m["name"].startswith("Garden leaf")
                ),
                "coat": m.get("extensions", {})
                .get("KHR_materials_clearcoat", {})
                .get("clearcoatFactor", 0),
            }
        )
        # Only the channels baked into native pages are removed. PBR factors
        # and emissive intensity still pass through the ordinary Rust cooker.
        material = {
            k: m[k]
            for k in (
                "name",
                "doubleSided",
                "emissiveFactor",
                "alphaMode",
                "alphaCutoff",
            )
            if k in m
        }
        material["pbrMetallicRoughness"] = {
            k: v for k, v in p.items() if not k.endswith("Texture")
        }
        if "KHR_materials_emissive_strength" in m.get("extensions", {}):
            material["extensions"] = {
                "KHR_materials_emissive_strength": m["extensions"][
                    "KHR_materials_emissive_strength"
                ]
            }
        cooked_mats.append(material)

    # The geometry of the ground boards belongs to the common shell, but
    # these three fitted photographs use the revised, registered wood finish.
    floor = next(
        m for m in gltf["materials"] if m["name"].startswith("Worn oiled floor")
    )
    floor_pbr = floor["pbrMetallicRoughness"]
    channels.append(
        tuple(
            image_id(t)
            for t in (
                floor_pbr["baseColorTexture"],
                floor["normalTexture"],
                floor_pbr["metallicRoughnessTexture"],
            )
        )
    )
    accessors = []
    old_indices = {}
    views = []
    view_indices = {}
    data = bytearray()

    def access(index):
        if index in old_indices:
            return old_indices[index]
        src = gltf["accessors"][index].copy()
        vi = src["bufferView"]
        if vi not in view_indices:
            view = gltf["bufferViews"][vi].copy()
            start = view.get("byteOffset", 0)
            data.extend(b"\0" * (-len(data) % 4))
            view["byteOffset"] = len(data)
            data.extend(blob[start : start + view["byteLength"]])
            view_indices[vi] = len(views)
            views.append(view)
        src["bufferView"] = view_indices[vi]
        old_indices[index] = len(accessors)
        accessors.append(src)
        return old_indices[index]

    packed_meshes = []
    for old in meshes:
        prims = []
        for pr in gltf["meshes"][old]["primitives"]:
            attrs = {
                k: access(pr["attributes"][k])
                for k in ("POSITION", "NORMAL", "TEXCOORD_0")
            }
            prims.append(
                {
                    "attributes": attrs,
                    "indices": access(pr["indices"]),
                    "material": mat_indices[pr["material"]],
                }
            )
        packed_meshes.append({"primitives": prims})
    doc = {
        "asset": {"version": "2.0", "generator": "Earth Online fitted room cooker"},
        "extensionsUsed": ["KHR_materials_emissive_strength"],
        "scene": 0,
        "scenes": [{"nodes": list(range(len(nodes)))}],
        "nodes": nodes,
        "meshes": packed_meshes,
        "materials": cooked_mats,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": len(data)}],
    }
    header = json.dumps(doc, separators=(",", ":"), ensure_ascii=False).encode()
    header += b" " * (-len(header) % 4)
    data.extend(b"\0" * (-len(data) % 4))
    path = DEST / "fitted-room.glb"
    path.write_bytes(
        struct.pack(
            "<4sIII4s", b"glTF", 2, 28 + len(header) + len(data), len(header), b"JSON"
        )
        + header
        + struct.pack("<I4s", len(data), b"BIN\0")
        + data
    )

    def tile(i, default):
        if i is None:
            return Image.new("RGBA", (SIZE, SIZE), default)
        v = gltf["bufferViews"][gltf["images"][i]["bufferView"]]
        start = v.get("byteOffset", 0)
        src = Image.open(io.BytesIO(blob[start : start + v["byteLength"]])).convert(
            "RGBA"
        )
        return src.resize((SIZE, SIZE), Image.Resampling.LANCZOS)

    maps = []
    for i, (name, default) in enumerate(
        [
            ("colour", (255, 255, 255, 255)),
            ("normal", (128, 128, 255, 255)),
            ("rough-metal", (255, 255, 255, 255)),
        ]
    ):
        page = Image.new("RGBA", (SIZE, SIZE * len(channels)))
        for row, ch in enumerate(channels):
            page.paste(tile(ch[i], default), (0, row * SIZE))
        dst = DEST / (name + ".png")
        page.save(dst, optimize=True)
        maps.append(reference(dst))

    def translation(delta):
        m = np.eye(4)
        m[:3, 3] = delta
        return m

    def anchor(p):
        x, z = p
        return (x, 0, -z)

    ang = layout["table_rotation_rad"]
    co, si = np.cos(ang), np.sin(ang)
    rot = np.array([[co, 0, si, 0], [0, 1, 0, 0], [-si, 0, co, 0], [0, 0, 0, 1]])
    move = (
        translation(anchor(layout["table_center"]))
        @ rot
        @ translation(-np.array(anchor(layout["previous_table_center"])))
    )
    inv = np.linalg.inv(parcel)

    def corners(a, b):
        return np.array(
            [
                [x, y, z, 1]
                for x, y, z in [
                    (a[0], a[1], a[2]),
                    (b[0], a[1], a[2]),
                    (b[0], b[1], a[2]),
                    (a[0], b[1], a[2]),
                    (a[0], a[1], b[2]),
                    (b[0], a[1], b[2]),
                    (b[0], b[1], b[2]),
                    (a[0], b[1], b[2]),
                ]
            ]
        )

    removed = []
    for group in target["metric_assemblies"]:
        centre = np.array(group["anchor"])
        metric = translation((inv @ [*centre, 1])[:3] - centre)
        for i in group["boxes"]:
            pts = corners(*target["boxes"][i])
            if "oak-dining-table" in group["components"]:
                fix = move
            elif "oak-sideboard" in group["components"]:
                fix = translation((0, 0, -0.722))
            elif "oven-range" in group["components"] and (
                150 <= i <= 152 or 158 <= i <= 160
            ):
                fix = translation((0, 0, -1.5))
            else:
                continue
            boxes.append((pts @ (parcel @ fix @ metric).T)[:, :3].tolist())
            removed.append(i)
    for name in (
        "Three-bay kitchen plinth",
        "Painted kitchen carcass",
        "Honed worktop with drip edge",
        "Turned starting newel",
    ):
        node = next(
            n for n in gltf["nodes"] if n.get("extras", {}).get("source_part") == name
        )
        bounds = [
            gltf["accessors"][pr["attributes"]["POSITION"]]
            for pr in gltf["meshes"][node["mesh"]]["primitives"]
        ]
        pts = corners(
            np.min([v["min"] for v in bounds], axis=0),
            np.max([v["max"] for v in bounds], axis=0),
        )
        boxes.append((pts @ (parcel @ local_matrix(node)).T)[:, :3].tolist())
    # Preserve the controller's ramp and rise. Widen its lower stations to
    # the surveyed nosings of the seven flared steps in the fitted scene.
    tread = next(
        n
        for n in gltf["nodes"]
        if n.get("extras", {}).get("source_part") == "stair-flight-1 - oiled-oak@ae906f"
    )
    geometry = gltf["meshes"][tread["mesh"]]["primitives"][0]["attributes"]
    pos = accessor(gltf, blob, geometry["POSITION"])
    tm = local_matrix(tread)
    pos = np.einsum("ij,kj->ki", tm[:3, :3], pos) + tm[:3, 3]
    if not np.isfinite(pos).all():
        raise ValueError("Non-finite stair vertex")
    edge = lambda i: np.min(pos[abs(pos[:, 1] - (i / 6 + 0.002)) < 0.0001, 2])
    foot = edge(1)
    length = (edge(2) - foot) * 7
    ramps = []
    for index in (0, 1):
        a, b, c, d = (inv @ np.column_stack((target["ramps"][index], np.ones(4))).T).T[
            :, :3
        ]

        def rail(p, far, t, inner):
            at = p + (far - p) * t
            if inner:
                at[0] -= 0.405 * (1 - np.clip((at[2] - foot) / length, 0, 1)) ** 2
            return inside(at).tolist()

        for t in np.linspace(0, 1, 21)[:-1]:
            ramps.append(
                [
                    rail(a, b, t, True),
                    rail(a, b, t + 0.05, True),
                    rail(d, c, t + 0.05, index == 1),
                    rail(d, c, t, index == 1),
                ]
            )
    info = dict(
        source="Fitted dining scene and the surveyed 21 Beaver lot",
        attribution=recipe["attribution"],
        license=recipe["license"],
        credits=recipe["credits"],
        source_scene_sha256=recipe["source_scene_sha256"],
        source_glb_sha256=digest(room),
        house_sha256=target["sha256"],
        build="scripts/blender-buildings/cook_beaver_fitted_room.py",
        **reference(path),
        maps=maps,
        layers=len(channels),
        surfaces=surfaces,
        replaced_boxes=removed,
        boxes=boxes,
        replaced_ramps=[0, 1],
        ramps=ramps,
        lights=lights,
    )
    path.with_suffix(".provenance.json").write_text(json.dumps(info, indent=2) + "\n")
    for field in maps:
        src = PUBLIC / field["url"].lstrip("/")
        src.with_suffix(".provenance.json").write_text(
            json.dumps(
                dict(
                    **field,
                    source=info["source"],
                    source_scene_sha256=recipe["source_scene_sha256"],
                    source_glb_sha256=digest(room),
                    build=info["build"],
                    attribution=info["attribution"],
                    license=info["license"],
                    credits=info["credits"],
                ),
                indent=2,
            )
            + "\n"
        )
    for src, meta in surveys:
        out = DEST / src.name
        shutil.copyfile(src, out)
        meta.update(
            reference(out),
            fitted_geometry_sha256=info["sha256"],
            install="scripts/blender-buildings/cook_beaver_fitted_room.py --lighting",
        )
        out.with_suffix(".provenance.json").write_text(
            json.dumps(meta, indent=2) + "\n"
        )
    print(
        f"{len(nodes)} mesh instances, {len(channels)} map layers, {len(boxes)} collision boxes"
    )


def lighting_surveys(directory, scene_hash, house_hash):
    meter = json.loads((directory / "room-meter.json").read_text())
    original = json.loads((PARENT / "downstairs-daylight.provenance.json").read_text())
    if (
        meter["interior"]["sha256"] != scene_hash
        or meter["exterior"]["sha256"] != original["source_scene"]["sha256"]
        or not 0 < meter["exposure_range_ev"] < 8
        or not meter["exterior_exposed_beam"] > 0
    ):
        raise ValueError(
            "Camera meter does not describe the registered interior and exterior"
        )
    surveys = []
    for name in ("dining-floor-light", "downstairs-daylight", "floor-reflections"):
        src = directory / (name + ".png")
        meta = json.loads(src.with_suffix(".provenance.json").read_text())
        if (
            digest(src) != meta["sha256"]
            or meta["house_sha256"] != house_hash
            or meta["source_scene"]["sha256"] != scene_hash
        ):
            raise ValueError(f"Light survey {name} does not describe the fitted source")
        if name == "downstairs-daylight":
            meta.update(
                exposure_range_ev=meter["exposure_range_ev"],
                exterior_exposed_beam=meter["exterior_exposed_beam"],
                camera_measurements=meter,
            )
        surveys.append((src, meta))
    return surveys


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("portable", type=Path)
    cli.add_argument(
        "--lighting",
        type=Path,
        help="Install verified Blender light measurements for this arrangement",
    )
    args = cli.parse_args()
    cook(args.portable, args.lighting)
