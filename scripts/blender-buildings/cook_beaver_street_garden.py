"""Cook the registered garden parts without shipping the Blender library twice."""

import argparse
import json
import struct
from pathlib import Path

import numpy as np
from check_interior_containment import read_glb
from cook_beaver_fitted_room import PARENT, SIZE, digest
from PIL import Image


def portable_geometry(path, transform, dest, *, label="Installed street garden"):
    doc, blob = read_glb(path)
    mats = doc["materials"]
    material_names = [m["name"] for m in mats]
    doc["materials"] = [
        {
            "name": m["name"],
            "doubleSided": True,
            "pbrMetallicRoughness": {
                "baseColorFactor": [1, 1, 1, 1],
                "metallicFactor": 1,
                "roughnessFactor": 1,
            },
        }
        for m in mats
    ]
    used = set()
    triangles = 0
    for m in doc["meshes"]:
        for p in m["primitives"]:
            p["attributes"] = {
                k: p["attributes"][k]
                for k in ("POSITION", "NORMAL", "TEXCOORD_0", "TEXCOORD_1")
            }
            used.update(p["attributes"].values())
            used.add(p["indices"])
            triangles += doc["accessors"][p["indices"]]["count"] // 3
    # Geometry views may be interleaved with packed pictures in an exported GLB.
    access_order = sorted(used)
    remap = {a: i for i, a in enumerate(access_order)}
    accessors = [doc["accessors"][i] for i in access_order]
    indices = sorted({a["bufferView"] for a in accessors})
    views = []
    data = bytearray()
    for vi in indices:
        old = doc["bufferViews"][vi]
        offset = old.get("byteOffset", 0)
        data.extend(b"\0" * (-len(data) % 4))
        views.append(dict(old, byteOffset=len(data)))
        data.extend(blob[offset : offset + old["byteLength"]])
    for acc in accessors:
        acc["bufferView"] = indices.index(acc["bufferView"])
    for m in doc["meshes"]:
        for p in m["primitives"]:
            p["attributes"] = {k: remap[v] for k, v in p["attributes"].items()}
            p["indices"] = remap[p["indices"]]
    root_node = len(doc["nodes"])
    old_scene = doc["scenes"][doc.get("scene", 0)]
    doc["nodes"].append(
        {
            "name": label,
            "children": old_scene["nodes"],
            "matrix": np.asarray(transform).T.ravel().tolist(),
        }
    )
    result = {k: doc[k] for k in ("asset", "nodes", "materials", "meshes")}
    result.update(
        scenes=[{"nodes": [root_node]}],
        scene=0,
        bufferViews=views,
        accessors=accessors,
        buffers=[{"byteLength": len(data)}],
    )
    header = json.dumps(result, ensure_ascii=False, separators=(",", ":")).encode()
    header += b" " * (-len(header) % 4)
    data.extend(b"\0" * (-len(data) % 4))
    dest.write_bytes(
        struct.pack(
            "<4sIII4s", b"glTF", 2, 28 + len(header) + len(data), len(header), b"JSON"
        )
        + header
        + struct.pack("<I4s", len(data), b"BIN\0")
        + data
    )
    return material_names, triangles


def cook(src, out):
    meta = json.loads((src / "street-garden.provenance.json").read_text())
    source = src / meta["portable"]["file"]
    if (
        digest(source) != meta["portable"]["sha256"]
        or digest(src / meta["file"]) != meta["sha256"]
    ):
        raise ValueError("The garden library or fitted scene has changed")
    out.mkdir(parents=True, exist_ok=True)
    model = out / "street-garden.glb"

    def ref(p):
        return {
            "url": "/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/"
            + p.name,
            "sha256": digest(p),
            "bytes": p.stat().st_size,
        }

    materials, count = portable_geometry(
        source, meta["parcel_from_assembled_gltf"], model
    )
    materials_by_name = {m["name"]: m for m in meta["materials"]["materials"]}
    if set(materials) != set(materials_by_name):
        raise ValueError("Registered material list differs from the GLB")
    pages = []
    for name in materials:
        page = Image.new("RGBA", (SIZE, SIZE * 3))
        for i, role in enumerate(("pigment", "normal", "orm")):
            info = materials_by_name[name]["maps"][role]
            img = src / "surfaces" / info["file"]
            if digest(img) != info["sha256"]:
                raise ValueError(img)
            with Image.open(img) as im:
                if im.size != (SIZE, SIZE):
                    raise ValueError(img)
                page.paste(im.convert("RGBA"), (0, SIZE * i))
        p = out / ("street-garden-" + name.split()[0].lower() + ".png")
        page.save(p, optimize=True)
        pages.append(ref(p))
    recipe = dict(
        **ref(model),
        maps=pages,
        materials=materials,
        triangles=count,
        house_sha256=json.loads((PARENT / "terrace.provenance.json").read_text())[
            "sha256"
        ],
        source_scene_sha256=meta["sha256"],
        portable_sha256=meta["portable"]["sha256"],
        replaced=meta["replaced"],
        credits=meta["materials"],
        construction_references=meta["sources"],
        build="scripts/blender-buildings/cook_beaver_street_garden.py",
    )
    (out / "street-garden.provenance.json").write_text(
        json.dumps(recipe, indent=2) + "\n"
    )


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("source", type=Path)
    cli.add_argument("--output", type=Path, default=PARENT)
    a = cli.parse_args()
    cook(a.source, a.output)
