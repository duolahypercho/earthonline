"""Pack board-aligned original colour, roughness and normal maps into the study."""

import hashlib
import json

import bpy
import numpy as np
import OpenImageIO as oiio
from beaver_dining_parts import material


def read_plate(path):
    inp = oiio.ImageInput.open(str(path))
    rgb = np.asarray(inp.read_image(format=oiio.UINT8))
    inp.close()
    return rgb


def pack(name, atlas, path, space):
    height, width, channels = atlas.shape
    out = oiio.ImageOutput.create(str(path))
    out.open(str(path), oiio.ImageSpec(width, height, channels, oiio.UINT8))
    out.write_image(np.ascontiguousarray(atlas))
    out.close()
    im = bpy.data.images.load(str(path))
    im.name = name
    im.colorspace_settings.name = space
    im.pack()
    return im


def install_oak(scene, source, dest, records):
    strips = [
        (44, 108),
        (187, 251),
        (278, 342),
        (368, 432),
        (441, 505),
        (601, 665),
        (737, 801),
        (836, 900),
    ]
    dest.mkdir(exist_ok=True, parents=True)
    images = {}
    packed = []
    for kind in ("Diffuse", "Rough", "nor_gl"):
        rec = next(r for r in records if r["map"] == kind)
        path = source / rec["file"]
        if hashlib.sha256(path.read_bytes()).hexdigest() != rec["sha256"]:
            raise ValueError(f"Photographic source changed: {path.name}")
        raw = read_plate(path)
        if raw.ndim == 2:
            raw = raw[:, :, None]
        if raw.shape[:2] != (4096, 4096):
            raise ValueError("Expected the pinned 4k scan")
        # Two-by-two area filtering retains exactly the same eight board cuts
        # and orientations that the editable house assigns to its UVs.
        planks = [
            raw[4 * y0 : 4 * y1].reshape(128, 2, 2048, 2, -1).mean(axis=(1, 3))
            for y0, y1 in strips
        ]
        atlas = np.concatenate(planks, axis=0)
        if kind == "nor_gl":
            normal = atlas[:, :, :3] / 127.5 - 1
            atlas = (
                normal
                / np.maximum(np.linalg.norm(normal, axis=-1, keepdims=True), 1e-6)
                + 1
            ) * 127.5
        atlas = np.uint8(np.clip(np.rint(atlas), 0, 255))
        png = dest / f"worn-oak-{kind.lower()}.png"
        images[kind] = pack(
            f"Dining oak - {kind}",
            atlas,
            png,
            "sRGB" if kind == "Diffuse" else "Non-Color",
        )
        packed.append(
            {"file": png.name, "sha256": hashlib.sha256(png.read_bytes()).hexdigest()}
        )
    floor = material(
        "Worn oiled floor - eight photographed boards", (150, 121, 87), 0.49
    )
    tree = floor.node_tree
    bs = tree.nodes["Principled BSDF"]
    uv = tree.nodes.new("ShaderNodeUVMap")
    uv.uv_map = "Board surface"
    for kind, socket in [
        ("Diffuse", "Base Color"),
        ("Rough", "Roughness"),
        ("nor_gl", "Normal"),
    ]:
        tex = tree.nodes.new("ShaderNodeTexImage")
        tex.image = images[kind]
        tree.links.new(uv.outputs["UV"], tex.inputs["Vector"])
        signal = tex.outputs["Color"]
        if kind == "nor_gl":
            normal = tree.nodes.new("ShaderNodeNormalMap")
            normal.uv_map = uv.uv_map
            normal.inputs["Strength"].default_value = 0.72
            tree.links.new(signal, normal.inputs["Color"])
            signal = normal.outputs["Normal"]
        tree.links.new(signal, bs.inputs[socket])
    bs.inputs["IOR"].default_value = 1.46
    count = 0
    for o in scene.objects["ground-storey"].children:
        if o.data.uv_layers.get("Board surface") and o.data.materials[
            0
        ].name.startswith("oiled-oak@"):
            o.data.materials[0] = floor
            count += 1
    if count != 4:
        raise ValueError(f"Expected four surveyed oak batches, got {count}")
    meta = {
        "source": "https://polyhaven.com/a/wood_floor_worn",
        "license": "CC0-1.0",
        "license_url": "https://polyhaven.com/license",
        "attribution": "Dimitrios Savva / Poly Haven",
        "inputs": records,
        "board_strips_at_1024": strips,
        "packed_maps": packed,
    }
    (dest / "provenance.json").write_text(json.dumps(meta, indent=2) + "\n")
